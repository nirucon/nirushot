//! `nirushot pick`: a freeze-frame colour picker.
//!
//! The screen is frozen with one server-side copy of the root window (every
//! monitor, rotated or not, in root coordinates) into a pixmap. That pixmap
//! is the background of an override-redirect window covering the root, so
//! the frozen frame costs no client-side decoding and the server repaints it
//! by itself. A small loupe follows the pointer: it fetches the 11x11 pixels
//! around the pointer from the pixmap and draws them magnified.
//!
//! Input is grabbed only while the picker is open (like `slop`), and an idle
//! picker cancels itself so it can never keep a screen locker out for long.
//! If the grab fails (a locker or another grab is active), the picker exits
//! without showing anything.

use std::{
    env, fs,
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::{mpsc, Arc},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use ab_glyph::FontArc;
use image::{Rgba, RgbaImage};
use x11rb::{
    connection::Connection,
    protocol::{xproto::*, Event},
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

use crate::{
    atomic_save, copy_text_to_clipboard, draw_text,
    keys::Keymap,
    load_font, load_theme, monitor, rgb, text_width,
    view::{intersect, next_event, put_bytes, Area},
    Theme,
};

/// Pixels on each side of the picked one: the loupe shows 11x11.
const HALF: i32 = 5;
const CELLS: i32 = 2 * HALF + 1;
/// Magnification.
const CELL: i32 = 10;
const GRID: i32 = CELLS * CELL;
const PAD: i32 = 6;
const LABEL: i32 = 22;
const SWATCH: i32 = 14;
const FONT_SIZE: f32 = 14.0;
/// Distance between the pointer and the loupe.
const OFFSET: i32 = 24;
/// An untouched picker cancels itself after this long.
pub const IDLE: Duration = Duration::from_secs(60);
/// How long to retry the grab while the key that started us is still held.
const GRAB_WAIT: Duration = Duration::from_millis(1000);
const HISTORY_MAX: usize = 10;

const XK_TAB: u32 = 0xff09;
const XK_ISO_LEFT_TAB: u32 = 0xfe20;
const XK_RETURN: u32 = 0xff0d;
const XK_KP_ENTER: u32 = 0xff8d;
const XK_SPACE: u32 = 0x20;
const XK_ESCAPE: u32 = 0xff1b;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Hex,
    Rgb,
    Hsl,
}

impl Format {
    const ALL: [Format; 3] = [Format::Hex, Format::Rgb, Format::Hsl];

    fn parse(name: &str) -> Option<Self> {
        match name {
            "hex" => Some(Format::Hex),
            "rgb" => Some(Format::Rgb),
            "hsl" => Some(Format::Hsl),
            _ => None,
        }
    }

    fn step(self, forward: bool) -> Self {
        let i = Self::ALL.iter().position(|f| *f == self).unwrap_or(0);
        let n = Self::ALL.len();
        Self::ALL[if forward {
            (i + 1) % n
        } else {
            (i + n - 1) % n
        }]
    }
}

pub fn hex(c: [u8; 3]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// Hue in degrees, saturation and lightness in percent, all rounded.
pub fn hsl(c: [u8; 3]) -> (u32, u32, u32) {
    let [r, g, b] = c.map(|v| v as f64 / 255.0);
    let max = r.max(g).max(b);
    let min = r.min(g).min(b);
    let l = (max + min) / 2.0;
    let d = max - min;
    if d == 0.0 {
        return (0, 0, (l * 100.0).round() as u32);
    }
    let s = d / (1.0 - (2.0 * l - 1.0).abs());
    let h = if max == r {
        60.0 * (((g - b) / d).rem_euclid(6.0))
    } else if max == g {
        60.0 * ((b - r) / d + 2.0)
    } else {
        60.0 * ((r - g) / d + 4.0)
    };
    (
        h.round() as u32 % 360,
        (s * 100.0).round() as u32,
        (l * 100.0).round() as u32,
    )
}

pub fn format(c: [u8; 3], f: Format) -> String {
    match f {
        Format::Hex => hex(c),
        Format::Rgb => format!("rgb({}, {}, {})", c[0], c[1], c[2]),
        Format::Hsl => {
            let (h, s, l) = hsl(c);
            format!("hsl({h}, {s}%, {l}%)")
        }
    }
}

/// Black or white, whichever reads better on `c`.
fn contrast(c: [u8; 3]) -> Rgba<u8> {
    let y = 0.299 * c[0] as f32 + 0.587 * c[1] as f32 + 0.114 * c[2] as f32;
    if y > 140.0 {
        Rgba([0, 0, 0, 255])
    } else {
        Rgba([255, 255, 255, 255])
    }
}

/// The 11x11 pixels around the pointer; None outside the screen.
pub type Cells = Vec<Option<[u8; 3]>>;

/// Width of the loupe panel: the grid, or the widest label.
fn panel_width(font: Option<&FontArc>) -> i32 {
    let text = font
        .map(|f| {
            ["#ffffff", "rgb(255, 255, 255)", "hsl(360, 100%, 100%)"]
                .iter()
                .map(|s| text_width(f, FONT_SIZE, s).ceil() as i32)
                .max()
                .unwrap_or(0)
        })
        .unwrap_or(0);
    (GRID + 2 * PAD).max(PAD + SWATCH + 6 + text + PAD)
}

const PANEL_H: i32 = PAD + GRID + PAD + LABEL;

fn fill(img: &mut RgbaImage, (x, y, w, h): Area, colour: Rgba<u8>) {
    let (iw, ih) = (img.width() as i32, img.height() as i32);
    for yy in y.max(0)..(y + h).min(ih) {
        for xx in x.max(0)..(x + w).min(iw) {
            img.put_pixel(xx as u32, yy as u32, colour);
        }
    }
}

fn frame(img: &mut RgbaImage, (x, y, w, h): Area, colour: Rgba<u8>) {
    fill(img, (x, y, w, 1), colour);
    fill(img, (x, y + h - 1, w, 1), colour);
    fill(img, (x, y, 1, h), colour);
    fill(img, (x + w - 1, y, 1, h), colour);
}

/// Draw the loupe: magnified pixels, a crosshair on the picked one, and a
/// swatch with the value in the current format.
pub fn render(
    cells: &Cells,
    f: Format,
    width: i32,
    theme: &Theme,
    font: Option<&FontArc>,
) -> RgbaImage {
    let mut img = RgbaImage::from_pixel(width as u32, PANEL_H as u32, rgb(theme.surface));
    frame(&mut img, (0, 0, width, PANEL_H), rgb(theme.border));
    let gx = (width - GRID) / 2;
    let gy = PAD;
    for (i, cell) in cells.iter().enumerate() {
        let (cx, cy) = (i as i32 % CELLS, i as i32 / CELLS);
        let area = (gx + cx * CELL, gy + cy * CELL, CELL, CELL);
        match cell {
            Some(c) => fill(&mut img, area, Rgba([c[0], c[1], c[2], 255])),
            // Outside every screen: a dim checker.
            None => {
                let shade = if (cx + cy) % 2 == 0 { 0x30 } else { 0x20 };
                fill(&mut img, area, Rgba([shade, shade, shade, 255]));
            }
        }
    }
    let centre = cells[(HALF * CELLS + HALF) as usize].unwrap_or([0, 0, 0]);
    // Crosshair: hairlines along the centre row and column up to the picked
    // pixel, each in the colour that stands out on the cell under it.
    let mid = HALF * CELL + CELL / 2;
    for k in 0..CELLS {
        if k == HALF {
            continue;
        }
        let colour = |c: Option<[u8; 3]>| contrast(c.unwrap_or([0, 0, 0]));
        let col = cells[(k * CELLS + HALF) as usize];
        fill(&mut img, (gx + mid, gy + k * CELL, 1, CELL), colour(col));
        let row = cells[(HALF * CELLS + k) as usize];
        fill(&mut img, (gx + k * CELL, gy + mid, CELL, 1), colour(row));
    }
    let around = contrast(centre);
    let inverse = if around.0[0] == 0 {
        Rgba([255, 255, 255, 255])
    } else {
        Rgba([0, 0, 0, 255])
    };
    let (bx, by) = (gx + HALF * CELL, gy + HALF * CELL);
    frame(&mut img, (bx - 2, by - 2, CELL + 4, CELL + 4), inverse);
    frame(&mut img, (bx - 1, by - 1, CELL + 2, CELL + 2), around);
    frame(
        &mut img,
        (gx - 1, gy - 1, GRID + 2, GRID + 2),
        rgb(theme.border),
    );
    // Label row.
    let ly = gy + GRID + PAD;
    let sy = ly + (LABEL - PAD - SWATCH) / 2;
    fill(
        &mut img,
        (PAD, sy, SWATCH, SWATCH),
        Rgba([centre[0], centre[1], centre[2], 255]),
    );
    frame(&mut img, (PAD, sy, SWATCH, SWATCH), rgb(theme.muted));
    if let Some(font) = font {
        let text = format(centre, f);
        let ty = ly + (LABEL - PAD - FONT_SIZE as i32 - 3) / 2;
        draw_text(
            &mut img,
            font,
            FONT_SIZE,
            (PAD + SWATCH + 6) as f32,
            ty as f32,
            &text,
            rgb(theme.foreground),
        );
    }
    img
}

/// Where the loupe goes for a pointer at `p` on `monitor`: below right,
/// flipped to stay on that monitor.
pub fn place(p: (i32, i32), size: (i32, i32), monitor: Area) -> (i32, i32) {
    let (mx, my, mw, mh) = monitor;
    let mut x = p.0 + OFFSET;
    if x + size.0 > mx + mw {
        x = p.0 - OFFSET - size.0;
    }
    let mut y = p.1 + OFFSET;
    if y + size.1 > my + mh {
        y = p.1 - OFFSET - size.1;
    }
    (
        x.clamp(mx, (mx + mw - size.0).max(mx)),
        y.clamp(my, (my + mh - size.1).max(my)),
    )
}

/// Keep a keyboard-moved point on a monitor (the root may have dead areas).
pub fn clamp_point(p: (i32, i32), monitors: &[Area]) -> (i32, i32) {
    match monitor::choose(monitors, p) {
        Some((x, y, w, h)) => (p.0.clamp(x, x + w - 1), p.1.clamp(y, y + h - 1)),
        None => p,
    }
}

/// Parts of `old` that `new` does not cover.
pub fn subtract(old: Area, new: Area) -> Vec<Area> {
    let Some((ix, iy, iw, ih)) = intersect(old, new) else {
        return vec![old];
    };
    let (x, y, w, h) = old;
    let mut out = Vec::new();
    if iy > y {
        out.push((x, y, w, iy - y));
    }
    if iy + ih < y + h {
        out.push((x, iy + ih, w, y + h - iy - ih));
    }
    if ix > x {
        out.push((x, iy, ix - x, ih));
    }
    if ix + iw < x + w {
        out.push((ix + iw, iy, x + w - ix - iw, ih));
    }
    out
}

// ---------------------------------------------------------------------------
// History: ~/.local/state/nirushot/colors.json, read by the launcher.

pub fn history_path() -> PathBuf {
    PathBuf::from(env::var_os("HOME").unwrap_or_default()).join(".local/state/nirushot/colors.json")
}

fn valid_hex(s: &str) -> bool {
    s.len() == 7
        && s.starts_with('#')
        && s[1..]
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Entries of a colors.json, tolerant of formatting; anything unreadable is
/// skipped.
pub fn parse_history(text: &str) -> Vec<(String, u64)> {
    let mut out = Vec::new();
    for object in text.split('{').skip(1) {
        let object = object.split('}').next().unwrap_or("");
        let field = |name: &str| -> Option<&str> {
            let key = format!("\"{name}\"");
            let rest = &object[object.find(&key)? + key.len()..];
            Some(rest.trim_start().strip_prefix(':')?.trim_start())
        };
        let Some(hex) = field("hex")
            .and_then(|v| v.strip_prefix('"'))
            .and_then(|v| v.split('"').next())
            .map(|v| v.to_ascii_lowercase())
            .filter(|v| valid_hex(v))
        else {
            continue;
        };
        let time = field("time")
            .map(|v| {
                v.chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect::<String>()
            })
            .and_then(|v| v.parse().ok())
            .unwrap_or(0);
        out.push((hex, time));
    }
    out
}

pub fn history_json(entries: &[(String, u64)]) -> String {
    let rows: Vec<String> = entries
        .iter()
        .map(|(hex, time)| format!("  {{\"hex\":\"{hex}\",\"time\":{time}}}"))
        .collect();
    if rows.is_empty() {
        "[]\n".into()
    } else {
        format!("[\n{}\n]\n", rows.join(",\n"))
    }
}

/// Put `hex` first (a repeated colour moves up), keep the newest ten.
pub fn push_history(mut entries: Vec<(String, u64)>, hex: &str, time: u64) -> Vec<(String, u64)> {
    entries.retain(|(h, _)| h != hex);
    entries.insert(0, (hex.to_string(), time));
    entries.truncate(HISTORY_MAX);
    entries
}

/// Write `text` to `path` through a temporary file and a rename.
fn write_atomic(path: &Path, text: &str) -> Result<(), String> {
    let parent = path.parent().ok_or("history path has no parent")?;
    fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    let mut tmp = tempfile::Builder::new()
        .prefix(".colors.json.")
        .tempfile_in(parent)
        .map_err(|e| format!("temporary file in {}: {e}", parent.display()))?;
    tmp.write_all(text.as_bytes())
        .and_then(|_| tmp.as_file().sync_all())
        .map_err(|e| format!("write history: {e}"))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(tmp.path(), fs::Permissions::from_mode(0o644));
    }
    tmp.persist(path)
        .map_err(|e| format!("replace {}: {e}", path.display()))?;
    Ok(())
}

pub fn record(path: &Path, hex: &str, time: u64) -> Result<Vec<(String, u64)>, String> {
    let old = fs::read_to_string(path)
        .map(|t| parse_history(&t))
        .unwrap_or_default();
    let entries = push_history(old, hex, time);
    write_atomic(path, &history_json(&entries))?;
    Ok(entries)
}

// ---------------------------------------------------------------------------
// The notification.

fn cache_dir() -> PathBuf {
    env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env::var_os("HOME").unwrap_or_default()).join(".cache"))
        .join("nirushot")
}

/// A swatch image for notification daemons that show one. Swatches of
/// colours no longer in the history are removed.
fn swatch(c: [u8; 3], keep: &[(String, u64)]) -> Option<PathBuf> {
    let dir = cache_dir();
    let name = format!("swatch-{}.png", &hex(c)[1..]);
    let path = dir.join(&name);
    let mut img = RgbaImage::from_pixel(64, 64, Rgba([c[0], c[1], c[2], 255]));
    frame(&mut img, (0, 0, 64, 64), Rgba([128, 128, 128, 255]));
    atomic_save(&img, &path).ok()?;
    if let Ok(list) = fs::read_dir(&dir) {
        for entry in list.flatten() {
            let file = entry.file_name().to_string_lossy().to_string();
            let Some(code) = file
                .strip_prefix("swatch-")
                .and_then(|f| f.strip_suffix(".png"))
            else {
                continue;
            };
            let wanted = format!("#{code}");
            if file != name && !keep.iter().any(|(h, _)| *h == wanted) {
                let _ = fs::remove_file(entry.path());
            }
        }
    }
    Some(path)
}

/// Whether the value reached the clipboard: None when it was not asked to.
fn notify(c: [u8; 3], value: &str, f: Format, copied: Option<bool>, icon: Option<&Path>) {
    let summary = format!("Colour {}", hex(c));
    let body = match (copied, f) {
        (Some(true), Format::Hex) => "Copied to the clipboard".to_string(),
        (Some(true), _) => format!("{value} copied to the clipboard"),
        (Some(false), _) => format!("{value} (could not copy to the clipboard)"),
        (None, _) => value.to_string(),
    };
    let mut cmd = Command::new("notify-send");
    cmd.args(["-a", "NIRUSHOT", "-t", "6000"]);
    if let Some(icon) = icon {
        let icon = icon.to_string_lossy();
        cmd.args(["-i", &icon, "-h", &format!("string:image-path:{icon}")]);
    }
    cmd.arg(summary)
        .arg(body)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    if let Ok(mut child) = cmd.spawn() {
        // Do not hang on a slow daemon, but let a quick one finish.
        let deadline = Instant::now() + Duration::from_millis(1500);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = child.try_wait() {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}

// ---------------------------------------------------------------------------
// The X side.

struct Pixels {
    lsb_first: bool,
}

impl Pixels {
    fn rgb(&self, p: &[u8]) -> [u8; 3] {
        if self.lsb_first {
            [p[2], p[1], p[0]]
        } else {
            [p[1], p[2], p[3]]
        }
    }
}

struct Picker {
    c: Arc<RustConnection>,
    root: Window,
    win: Window,
    gc: Gcontext,
    frozen: Pixmap,
    depth: u8,
    pixels: Pixels,
    root_size: (i32, i32),
    monitors: Vec<Area>,
    theme: Theme,
    font: Option<FontArc>,
    keymap: Keymap,
    format: Format,
    width: i32,
    pos: (i32, i32),
    cells: Cells,
    drawn: Option<Area>,
    bytes: Vec<u8>,
    idle_atom: Atom,
    idle: mpsc::Sender<()>,
}

enum Outcome {
    Picked([u8; 3], Format),
    Cancelled,
}

fn err(e: impl ToString) -> String {
    e.to_string()
}

/// Sends `atom` to `win` after IDLE without a poke; ends when dropped.
fn idle_timer(c: Arc<RustConnection>, win: Window, atom: Atom) -> mpsc::Sender<()> {
    let (tx, rx) = mpsc::channel::<()>();
    thread::spawn(move || loop {
        match rx.recv_timeout(IDLE) {
            Ok(()) => continue,
            Err(mpsc::RecvTimeoutError::Timeout) => {
                let event = ClientMessageEvent::new(32, win, atom, [0, 0, 0, 0, 0]);
                let _ = c.send_event(false, win, EventMask::NO_EVENT, event);
                let _ = c.flush();
                return;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        }
    });
    tx
}

fn grab(c: &RustConnection, root: Window, cursor: Cursor) -> Result<(), String> {
    let deadline = Instant::now() + GRAB_WAIT;
    let mut pointer = false;
    let mut keyboard = false;
    loop {
        if !pointer {
            pointer = c
                .grab_pointer(
                    false,
                    root,
                    EventMask::BUTTON_PRESS | EventMask::BUTTON_RELEASE | EventMask::POINTER_MOTION,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                    x11rb::NONE,
                    cursor,
                    x11rb::CURRENT_TIME,
                )
                .map_err(err)?
                .reply()
                .map_err(err)?
                .status
                == GrabStatus::SUCCESS;
        }
        if !keyboard {
            keyboard = c
                .grab_keyboard(
                    false,
                    root,
                    x11rb::CURRENT_TIME,
                    GrabMode::ASYNC,
                    GrabMode::ASYNC,
                )
                .map_err(err)?
                .reply()
                .map_err(err)?
                .status
                == GrabStatus::SUCCESS;
        }
        if pointer && keyboard {
            return Ok(());
        }
        if Instant::now() >= deadline {
            let _ = c.ungrab_pointer(x11rb::CURRENT_TIME);
            let _ = c.ungrab_keyboard(x11rb::CURRENT_TIME);
            let _ = c.flush();
            return Err(format!(
                "cannot grab the {} (is the screen locked?)",
                if pointer { "keyboard" } else { "pointer" }
            ));
        }
        thread::sleep(Duration::from_millis(15));
    }
}

fn intern(c: &RustConnection, name: &str) -> Atom {
    c.intern_atom(false, name.as_bytes())
        .ok()
        .and_then(|r| r.reply().ok())
        .map(|r| r.atom)
        .unwrap_or(0)
}

fn open(start: Instant, format: Format, timing: bool) -> Result<Outcome, String> {
    // The font search runs fc-match; keep it off the path to the first frame.
    let font_job = thread::spawn(load_font);
    let (c, screen) = x11rb::connect(None).map_err(|e| format!("cannot open the display: {e}"))?;
    let c = Arc::new(c);
    let setup = c.setup();
    let s = &setup.roots[screen];
    let (root, depth) = (s.root, s.root_depth);
    let root_size = (s.width_in_pixels as i32, s.height_in_pixels as i32);
    let lsb_first = setup.image_byte_order == ImageOrder::LSB_FIRST;
    let bpp = setup
        .pixmap_formats
        .iter()
        .find(|f| f.depth == depth)
        .map(|f| f.bits_per_pixel)
        .unwrap_or(0);
    if bpp != 32 {
        return Err(format!(
            "unsupported display format: depth {depth} at {bpp} bits per pixel"
        ));
    }

    // Freeze first: one server-side copy of everything on every monitor.
    let frozen = c.generate_id().map_err(err)?;
    let gc = c.generate_id().map_err(err)?;
    c.create_pixmap(depth, frozen, root, root_size.0 as u16, root_size.1 as u16)
        .map_err(err)?;
    c.create_gc(
        gc,
        root,
        &CreateGCAux::new()
            .subwindow_mode(SubwindowMode::INCLUDE_INFERIORS)
            .graphics_exposures(0),
    )
    .map_err(err)?;
    c.copy_area(
        root,
        frozen,
        gc,
        0,
        0,
        0,
        0,
        root_size.0 as u16,
        root_size.1 as u16,
    )
    .map_err(err)?;
    c.flush().map_err(err)?;
    let frozen_at = start.elapsed();

    // The crosshair cursor of the core cursor font.
    let cursor_font = c.generate_id().map_err(err)?;
    let cursor = c.generate_id().map_err(err)?;
    c.open_font(cursor_font, b"cursor").map_err(err)?;
    c.create_glyph_cursor(
        cursor,
        cursor_font,
        cursor_font,
        34,
        35,
        0,
        0,
        0,
        0xffff,
        0xffff,
        0xffff,
    )
    .map_err(err)?;
    grab(&c, root, cursor)?;

    let win = c.generate_id().map_err(err)?;
    c.create_window(
        x11rb::COPY_DEPTH_FROM_PARENT,
        win,
        root,
        0,
        0,
        root_size.0 as u16,
        root_size.1 as u16,
        0,
        WindowClass::INPUT_OUTPUT,
        x11rb::COPY_FROM_PARENT,
        &CreateWindowAux::new()
            .background_pixmap(frozen)
            .override_redirect(1)
            .save_under(0)
            .cursor(cursor)
            .event_mask(EventMask::EXPOSURE),
    )
    .map_err(err)?;
    c.change_property8(
        PropMode::REPLACE,
        win,
        AtomEnum::WM_NAME,
        AtomEnum::STRING,
        b"NIRUSHOT colour picker",
    )
    .map_err(err)?;
    c.change_property8(
        PropMode::REPLACE,
        win,
        AtomEnum::WM_CLASS,
        AtomEnum::STRING,
        b"nirushot-pick\0Nirushot\0",
    )
    .map_err(err)?;
    c.map_window(win).map_err(err)?;
    c.flush().map_err(err)?;
    let shown_at = start.elapsed();

    let idle_atom = intern(&c, "_NIRUSHOT_PICK_IDLE");
    let idle = idle_timer(Arc::clone(&c), win, idle_atom);
    let monitors = monitor::monitors(&c, root, (root_size.0 as u32, root_size.1 as u32));
    let pos = monitor::pointer(&c, root).unwrap_or((root_size.0 / 2, root_size.1 / 2));
    let font = font_job.join().ok().flatten();
    let width = panel_width(font.as_ref());
    let keymap = Keymap::load(&c);
    let mut picker = Picker {
        c,
        root,
        win,
        gc,
        frozen,
        depth,
        pixels: Pixels { lsb_first },
        root_size,
        monitors,
        theme: load_theme(),
        font,
        keymap,
        format,
        width,
        pos,
        cells: Vec::new(),
        drawn: None,
        bytes: Vec::new(),
        idle_atom,
        idle,
    };
    picker.sample()?;
    picker.draw()?;
    picker.c.flush().map_err(err)?;
    if timing {
        let unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        eprintln!(
            "nirushot pick: frozen {:.1} ms, window mapped {:.1} ms, loupe drawn {:.1} ms (since main; wall clock {unix} ns)",
            frozen_at.as_secs_f64() * 1e3,
            shown_at.as_secs_f64() * 1e3,
            start.elapsed().as_secs_f64() * 1e3,
        );
    }
    let outcome = picker.run();
    // Hand the screen back before anything slow happens.
    let _ = picker.c.ungrab_pointer(x11rb::CURRENT_TIME);
    let _ = picker.c.ungrab_keyboard(x11rb::CURRENT_TIME);
    let _ = picker.c.destroy_window(picker.win);
    let _ = picker.c.free_pixmap(picker.frozen);
    let _ = picker.c.free_cursor(cursor);
    let _ = picker.c.close_font(cursor_font);
    let _ = picker.c.flush();
    outcome
}

impl Picker {
    /// Fetch the 11x11 pixels around the pointer from the frozen frame.
    fn sample(&mut self) -> Result<(), String> {
        let (px, py) = self.pos;
        let want = (px - HALF, py - HALF, CELLS, CELLS);
        self.cells = vec![None; (CELLS * CELLS) as usize];
        let Some((x, y, w, h)) = intersect(want, (0, 0, self.root_size.0, self.root_size.1)) else {
            return Ok(());
        };
        let reply = self
            .c
            .get_image(
                ImageFormat::Z_PIXMAP,
                self.frozen,
                x as i16,
                y as i16,
                w as u16,
                h as u16,
                !0,
            )
            .map_err(err)?
            .reply()
            .map_err(err)?;
        let stride = w as usize * 4;
        for row in 0..h {
            for col in 0..w {
                let o = row as usize * stride + col as usize * 4;
                let Some(p) = reply.data.get(o..o + 4) else {
                    continue;
                };
                let (cx, cy) = (x + col - want.0, y + row - want.1);
                self.cells[(cy * CELLS + cx) as usize] = Some(self.pixels.rgb(p));
            }
        }
        Ok(())
    }

    fn centre(&self) -> [u8; 3] {
        self.cells
            .get((HALF * CELLS + HALF) as usize)
            .copied()
            .flatten()
            .unwrap_or([0, 0, 0])
    }

    fn draw(&mut self) -> Result<(), String> {
        let img = render(
            &self.cells,
            self.format,
            self.width,
            &self.theme,
            self.font.as_ref(),
        );
        let monitor = monitor::choose(&self.monitors, self.pos).unwrap_or((
            0,
            0,
            self.root_size.0,
            self.root_size.1,
        ));
        let (x, y) = place(self.pos, (self.width, PANEL_H), monitor);
        let area = (x, y, self.width, PANEL_H);
        crate::view::to_x_bytes(self.pixels.lsb_first, &img, &mut self.bytes);
        put_bytes(
            &self.c,
            self.win,
            self.gc,
            self.depth,
            x as i16,
            y as i16,
            self.width as u32,
            PANEL_H as u32,
            &self.bytes,
        )?;
        // Uncover what the old loupe hid, after the new one is up.
        if let Some(old) = self.drawn {
            if old != area {
                for (x, y, w, h) in subtract(old, area) {
                    self.c
                        .clear_area(false, self.win, x as i16, y as i16, w as u16, h as u16)
                        .map_err(err)?;
                }
            }
        }
        self.drawn = Some(area);
        Ok(())
    }

    fn move_to(&mut self, p: (i32, i32)) -> Result<(), String> {
        if p == self.pos {
            return Ok(());
        }
        self.pos = p;
        self.sample()?;
        self.draw()
    }

    fn nudge(&mut self, dx: i32, dy: i32) -> Result<(), String> {
        let p = clamp_point((self.pos.0 + dx, self.pos.1 + dy), &self.monitors);
        self.c
            .warp_pointer(x11rb::NONE, self.root, 0, 0, 0, 0, p.0 as i16, p.1 as i16)
            .map_err(err)?;
        self.move_to(p)
    }

    fn run(&mut self) -> Result<Outcome, String> {
        let mut pending = None;
        loop {
            self.c.flush().map_err(err)?;
            let event = next_event(&self.c, &mut pending)?;
            match event {
                Event::Expose(e) if e.count == 0 => self.draw()?,
                Event::MotionNotify(e) => {
                    let _ = self.idle.send(());
                    self.move_to((e.root_x as i32, e.root_y as i32))?;
                }
                Event::ButtonPress(e) => {
                    let _ = self.idle.send(());
                    match e.detail {
                        1 => {
                            self.move_to((e.root_x as i32, e.root_y as i32))?;
                            return Ok(Outcome::Picked(self.centre(), self.format));
                        }
                        3 => return Ok(Outcome::Cancelled),
                        _ => {}
                    }
                }
                Event::KeyPress(e) => {
                    let _ = self.idle.send(());
                    let sym = self.keymap.symbol(e.detail, e.state);
                    let step = if e.state.contains(KeyButMask::SHIFT) {
                        10
                    } else {
                        1
                    };
                    match sym {
                        XK_ESCAPE => return Ok(Outcome::Cancelled),
                        XK_RETURN | XK_KP_ENTER | XK_SPACE => {
                            return Ok(Outcome::Picked(self.centre(), self.format))
                        }
                        XK_TAB | XK_ISO_LEFT_TAB => {
                            let forward = sym == XK_TAB && !e.state.contains(KeyButMask::SHIFT);
                            self.format = self.format.step(forward);
                            self.draw()?;
                        }
                        // Left, Up, Right, Down, and the keypad ones.
                        0xff51 | 0xff96 => self.nudge(-step, 0)?,
                        0xff52 | 0xff97 => self.nudge(0, -step)?,
                        0xff53 | 0xff98 => self.nudge(step, 0)?,
                        0xff54 | 0xff99 => self.nudge(0, step)?,
                        _ => {}
                    }
                }
                Event::MappingNotify(_) => self.keymap = Keymap::load(&self.c),
                Event::ClientMessage(e) if e.type_ == self.idle_atom && e.window == self.win => {
                    return Ok(Outcome::Cancelled)
                }
                _ => {}
            }
        }
    }
}

// ---------------------------------------------------------------------------

pub fn usage() -> &'static str {
    "Usage: nirushot pick [--format hex|rgb|hsl] [--no-copy] [--no-notify] [--no-history]"
}

/// Runs the picker and returns the exit code: 0 picked, 1 cancelled,
/// 2 failed.
pub fn main(args: &[String]) -> i32 {
    let start = Instant::now();
    let mut format_choice = Format::Hex;
    let mut copy = true;
    let mut notify_enabled = true;
    let mut history = true;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--format" => {
                i += 1;
                match args.get(i).and_then(|v| Format::parse(v)) {
                    Some(f) => format_choice = f,
                    None => {
                        eprintln!("nirushot pick: --format needs hex, rgb or hsl");
                        return 2;
                    }
                }
            }
            "--no-copy" => copy = false,
            "--no-notify" => notify_enabled = false,
            "--no-history" => history = false,
            "--help" | "-h" => {
                println!("{}", usage());
                return 0;
            }
            other => {
                eprintln!("nirushot pick: unknown option {other}");
                eprintln!("{}", usage());
                return 2;
            }
        }
        i += 1;
    }
    let timing = env::var_os("NIRUSHOT_TIMING").is_some_and(|v| !v.is_empty());
    let (colour, chosen) = match open(start, format_choice, timing) {
        Ok(Outcome::Picked(c, f)) => (c, f),
        Ok(Outcome::Cancelled) => return 1,
        Err(e) => {
            eprintln!("nirushot pick: {e}");
            return 2;
        }
    };
    let value = format(colour, chosen);
    println!("{value}");
    let copied = copy.then(|| copy_text_to_clipboard(&value));
    if let Some(Err(e)) = &copied {
        eprintln!("nirushot pick: copy failed: {e}");
    }
    let copied = copied.map(|r| r.is_ok());
    let mut entries = Vec::new();
    if history {
        let time = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        match record(&history_path(), &hex(colour), time) {
            Ok(list) => entries = list,
            Err(e) => eprintln!("nirushot pick: history: {e}"),
        }
    }
    if notify_enabled {
        let icon = swatch(colour, &entries);
        notify(colour, &value, chosen, copied, icon.as_deref());
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        let c = [26, 43, 60];
        assert_eq!(format(c, Format::Hex), "#1a2b3c");
        assert_eq!(format(c, Format::Rgb), "rgb(26, 43, 60)");
        assert_eq!(format(c, Format::Hsl), "hsl(210, 40%, 17%)");
        assert_eq!(format([255, 0, 0], Format::Hsl), "hsl(0, 100%, 50%)");
        assert_eq!(format([0, 255, 0], Format::Hsl), "hsl(120, 100%, 50%)");
        assert_eq!(format([0, 0, 255], Format::Hsl), "hsl(240, 100%, 50%)");
        assert_eq!(format([255, 0, 128], Format::Hsl), "hsl(330, 100%, 50%)");
        assert_eq!(format([255, 255, 255], Format::Hsl), "hsl(0, 0%, 100%)");
        assert_eq!(format([0, 0, 0], Format::Hex), "#000000");
        assert_eq!(format([128, 128, 128], Format::Hsl), "hsl(0, 0%, 50%)");
    }

    #[test]
    fn tab_cycles_formats_both_ways() {
        assert_eq!(Format::Hex.step(true), Format::Rgb);
        assert_eq!(Format::Rgb.step(true), Format::Hsl);
        assert_eq!(Format::Hsl.step(true), Format::Hex);
        assert_eq!(Format::Hex.step(false), Format::Hsl);
    }

    #[test]
    fn history_is_newest_first_unique_and_capped() {
        let mut list = Vec::new();
        for i in 0..12u64 {
            list = push_history(list, &format!("#0000{:02x}", i), 100 + i);
        }
        assert_eq!(list.len(), 10);
        assert_eq!(list[0], ("#00000b".to_string(), 111));
        assert_eq!(list[9].0, "#000002");
        list = push_history(list, "#000005", 200);
        assert_eq!(list.len(), 10);
        assert_eq!(list[0], ("#000005".to_string(), 200));
        assert_eq!(list.iter().filter(|(h, _)| h == "#000005").count(), 1);
    }

    #[test]
    fn history_round_trips_and_tolerates_other_writers() {
        let entries = vec![("#1a2b3c".to_string(), 1762581234), ("#ffffff".into(), 5)];
        let text = history_json(&entries);
        assert_eq!(
            text,
            "[\n  {\"hex\":\"#1a2b3c\",\"time\":1762581234},\n  {\"hex\":\"#ffffff\",\"time\":5}\n]\n"
        );
        assert_eq!(parse_history(&text), entries);
        let other =
            r##"[{"time": 7, "hex": "#ABCDEF"}, {"hex":"nope","time":1}, {"hex":"#000000"}]"##;
        assert_eq!(
            parse_history(other),
            vec![("#abcdef".to_string(), 7), ("#000000".to_string(), 0)]
        );
        assert!(parse_history("garbage").is_empty());
        assert_eq!(history_json(&[]), "[]\n");
    }

    #[test]
    fn record_writes_the_file_atomically() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("state/nirushot/colors.json");
        record(&path, "#112233", 1).unwrap();
        record(&path, "#445566", 2).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert_eq!(
            parse_history(&text),
            vec![("#445566".to_string(), 2), ("#112233".to_string(), 1)]
        );
        let leftovers: Vec<_> = fs::read_dir(path.parent().unwrap())
            .unwrap()
            .flatten()
            .filter(|e| e.file_name() != "colors.json")
            .collect();
        assert!(leftovers.is_empty(), "temporary files left behind");
    }

    /// othala: DisplayPort-5 (inverted) at the top, DisplayPort-4 below.
    const STACKED: [Area; 2] = [(0, 1440, 2560, 1440), (0, 0, 2560, 1440)];

    #[test]
    fn the_loupe_stays_on_the_pointer_monitor() {
        let size = (160, 150);
        // Bottom-right corner of the upper monitor: flipped up and left,
        // never onto the lower monitor.
        let (x, y) = place((2550, 1435), size, STACKED[1]);
        assert!(x + size.0 <= 2560 && y + size.1 <= 1440, "{x},{y}");
        assert_eq!((x, y), (2550 - OFFSET - 160, 1435 - OFFSET - 150));
        // Top of the lower monitor: below the pointer.
        assert_eq!(place((10, 1441), size, STACKED[0]), (34, 1441 + OFFSET));
    }

    #[test]
    fn keyboard_moves_stay_on_a_monitor() {
        let side = [(0, 0, 1920, 1080), (1920, 0, 2560, 1440)];
        // Below the shorter monitor is a dead area.
        assert_eq!(clamp_point((100, 1200), &side), (100, 1079));
        assert_eq!(clamp_point((-3, 5), &STACKED), (0, 5));
        assert_eq!(clamp_point((5, 1439 + 10), &STACKED), (5, 1449));
    }

    #[test]
    fn uncovering_the_old_loupe() {
        assert_eq!(
            subtract((0, 0, 10, 10), (20, 20, 5, 5)),
            vec![(0, 0, 10, 10)]
        );
        let parts = subtract((0, 0, 10, 10), (3, 4, 10, 10));
        let area: i32 = parts.iter().map(|p| p.2 * p.3).sum();
        assert_eq!(area, 100 - 7 * 6);
        assert!(subtract((0, 0, 10, 10), (0, 0, 10, 10)).is_empty());
    }

    #[test]
    fn the_loupe_marks_the_picked_pixel() {
        let mut cells: Cells = vec![Some([10, 200, 30]); (CELLS * CELLS) as usize];
        cells[(HALF * CELLS + HALF) as usize] = Some([250, 1, 2]);
        cells[0] = None;
        let theme = Theme::default();
        let img = render(&cells, Format::Rgb, panel_width(None), &theme, None);
        let gx = (panel_width(None) - GRID) / 2;
        let at = |x: i32, y: i32| img.get_pixel(x as u32, y as u32).0;
        // Inside the centre cell: the picked colour.
        let (cx, cy) = (gx + HALF * CELL + 3, PAD + HALF * CELL + 3);
        assert_eq!(at(cx, cy), [250, 1, 2, 255]);
        // Its outline is drawn just outside it.
        assert_ne!(at(gx + HALF * CELL - 1, cy), [10, 200, 30, 255]);
        // A neighbour keeps its colour, and off-screen cells are a checker.
        assert_eq!(at(gx + 2, PAD + CELL + 2), [10, 200, 30, 255]);
        assert_eq!(at(gx + 2, PAD + 2), [0x30, 0x30, 0x30, 255]);
        // The swatch shows the picked colour.
        let sy = PAD + GRID + PAD + (LABEL - PAD - SWATCH) / 2;
        assert_eq!(at(PAD + 5, sy + 5), [250, 1, 2, 255]);
    }
}
