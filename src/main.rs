mod view;

use std::{
    env,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    sync::Arc,
    thread,
    time::{Duration, Instant},
};

fn run(program: &str, args: &[String]) -> Result<String, String> {
    let out = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("cannot start {program}: {e}"))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn active_window() -> Result<String, String> {
    let text = run("xprop", &["-root".into(), "_NET_ACTIVE_WINDOW".into()])?;
    let id = text.split_whitespace().last().ok_or("no active window")?;
    if !id.starts_with("0x") {
        return Err("active window has no usable id".into());
    }
    Ok(id.to_string())
}

use ab_glyph::{point, Font, FontArc, PxScale, ScaleFont};
use image::{Rgba, RgbaImage};

fn notify(path: &str, copied: bool) {
    let exe = env::current_exe().unwrap_or_else(|_| PathBuf::from("nirushot"));
    let mut command = Command::new(exe);
    command.arg("--notify-result").arg(path);
    if copied {
        command.arg("--copied");
    }
    let _ = command.spawn();
}

fn notify_and_handle(path: &Path, copied: bool) {
    let text = path.to_string_lossy().to_string();
    let support = Command::new("notify-send")
        .arg("--help")
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("--action"))
        .unwrap_or(false);
    if support {
        let body = if copied {
            "Saved and copied to the clipboard"
        } else {
            "Saved to Pictures"
        };
        let output = Command::new("notify-send")
            .args([
                "-a",
                "NIRUSHOT",
                "-t",
                "10000",
                "-A",
                "open=Open",
                "-A",
                "copy=Copy",
                "-A",
                "folder=Show in folder",
                "Screenshot",
                body,
                &text,
            ])
            .output();
        let action = output
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        match action.as_str() {
            "open" => {
                let _ = Command::new("xdg-open").arg(path).spawn();
            }
            "copy" => {
                let _ = copy_to_clipboard(path);
            }
            "folder" => {
                if let Some(parent) = path.parent() {
                    let _ = Command::new("xdg-open").arg(parent).spawn();
                }
            }
            _ => {}
        }
    } else {
        let body = if copied {
            "Saved and copied to the clipboard"
        } else {
            "Saved to Pictures"
        };
        let _ = Command::new("notify-send")
            .args(["-a", "NIRUSHOT", "-t", "6000", "Screenshot", body, &text])
            .status();
    }
}

#[derive(Clone, Copy)]
struct Theme {
    background: u32,
    surface: u32,
    foreground: u32,
    muted: u32,
    border: u32,
    accent: u32,
}

impl Default for Theme {
    fn default() -> Self {
        Self {
            background: 0x0f0f10,
            surface: 0x161618,
            foreground: 0xe5e5e5,
            muted: 0xa8a8a8,
            border: 0x2a2a2d,
            accent: 0x5a5a60,
        }
    }
}

fn config_dir() -> PathBuf {
    env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env::var_os("HOME").unwrap_or_default()).join(".config"))
}

fn config_value(key: &str) -> Option<String> {
    let text = std::fs::read_to_string(config_dir().join("niruwm/config.toml")).ok()?;
    text.lines().find_map(|line| {
        let line = line.trim();
        let (name, value) = line.split_once('=')?;
        if name.trim() != key {
            return None;
        }
        Some(value.trim().trim_matches('"').to_string())
    })
}

fn load_theme() -> Theme {
    let name = config_value("theme").unwrap_or_else(|| "niru-noir".into());
    let mut theme = Theme::default();
    match name.as_str() {
        "niru-nord" => {
            theme.background = 0x2e3440;
            theme.foreground = 0xeceff4;
            theme.muted = 0x9aa5b1;
            theme.border = 0x3b4252;
            theme.accent = 0x88c0d0;
        }
        "niru-satie" => {
            theme.background = 0x141414;
            theme.foreground = 0xdedad4;
            theme.muted = 0x8f8b86;
            theme.border = 0x2a2724;
            theme.accent = 0xb08d7d;
        }
        "niru-carl-larsson" => {
            theme.background = 0xf3ece0;
            theme.foreground = 0x3b3228;
            theme.muted = 0x7d6f5f;
            theme.border = 0xd8cdbb;
            theme.accent = 0xa63d2f;
        }
        "niru-forest-vigil" => {
            theme.background = 0x0b100c;
            theme.foreground = 0xd8ded2;
            theme.muted = 0x96a292;
            theme.border = 0x2b362c;
            theme.accent = 0x8aa87e;
        }
        "niru-blood-of-urdr" => {
            theme.background = 0x120808;
            theme.foreground = 0xe8d6cf;
            theme.muted = 0xb09a92;
            theme.border = 0x3a1d1b;
            theme.accent = 0xc96050;
        }
        "niru-root-and-stone" => {
            theme.background = 0x17100b;
            theme.foreground = 0xeadfd2;
            theme.muted = 0xb5a08c;
            theme.border = 0x3a2a1e;
            theme.accent = 0xc19768;
        }
        "niru-blue-horizon" => {
            theme.background = 0x0b1821;
            theme.foreground = 0xdce6eb;
            theme.muted = 0x93a6b2;
            theme.border = 0x243846;
            theme.accent = 0x86aec8;
        }
        "niru-the-norns" => {
            theme.background = 0x111110;
            theme.foreground = 0xece9e4;
            theme.muted = 0xa6a19b;
            theme.border = 0x3b3936;
            theme.accent = 0xc9c2b9;
        }
        "niru-winter-nights" => {
            theme.background = 0x1f232b;
            theme.foreground = 0xead9c3;
            theme.muted = 0xb9aa9a;
            theme.border = 0x4e5157;
            theme.accent = 0xd8c4aa;
        }
        _ => {}
    }
    theme.surface = blend_colour(theme.background, theme.foreground, 0.08);
    theme
}

fn blend_colour(a: u32, b: u32, amount: f32) -> u32 {
    let channel = |shift: u32| {
        let x = ((a >> shift) & 0xff) as f32;
        let y = ((b >> shift) & 0xff) as f32;
        ((x + (y - x) * amount).round() as u32) << shift
    };
    channel(16) | channel(8) | channel(0)
}

fn rgb(value: u32) -> Rgba<u8> {
    Rgba([
        ((value >> 16) & 0xff) as u8,
        ((value >> 8) & 0xff) as u8,
        (value & 0xff) as u8,
        255,
    ])
}

fn load_font() -> Option<FontArc> {
    let candidates = [
        config_value("font_path"),
        Command::new("fc-match")
            .args(["-f", "%{file}", "monospace"])
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| !s.is_empty()),
        Some("/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf".into()),
        Some("/usr/share/fonts/TTF/DejaVuSansMono.ttf".into()),
        Some("/usr/share/fonts/dejavu/DejaVuSansMono.ttf".into()),
        Some("/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf".into()),
    ];
    candidates.into_iter().flatten().find_map(|path| {
        std::fs::read(path)
            .ok()
            .and_then(|data| FontArc::try_from_vec(data).ok())
    })
}

fn text_width(font: &FontArc, size: f32, text: &str) -> f32 {
    let scaled = font.as_scaled(PxScale::from(size));
    text.chars()
        .map(|ch| scaled.h_advance(font.glyph_id(ch)))
        .sum()
}

fn draw_text(
    image: &mut RgbaImage,
    font: &FontArc,
    size: f32,
    x: f32,
    y: f32,
    text: &str,
    colour: Rgba<u8>,
) {
    let scale = PxScale::from(size);
    let scaled = font.as_scaled(scale);
    let baseline = y + scaled.ascent();
    let mut caret = x;
    for ch in text.chars() {
        let id = font.glyph_id(ch);
        let glyph = id.with_scale_and_position(scale, point(caret, baseline));
        if let Some(outline) = font.outline_glyph(glyph) {
            let bounds = outline.px_bounds();
            outline.draw(|gx, gy, coverage| {
                let px = bounds.min.x as i32 + gx as i32;
                let py = bounds.min.y as i32 + gy as i32;
                if px < 0 || py < 0 || px as u32 >= image.width() || py as u32 >= image.height() {
                    return;
                }
                let dst = image.get_pixel_mut(px as u32, py as u32);
                let alpha = coverage;
                for i in 0..3 {
                    dst[i] =
                        (colour[i] as f32 * alpha + dst[i] as f32 * (1.0 - alpha)).round() as u8;
                }
            });
        }
        caret += scaled.h_advance(id);
    }
}

fn fill_rect(image: &mut RgbaImage, x: i32, y: i32, w: i32, h: i32, colour: Rgba<u8>) {
    for yy in y.max(0)..(y + h).min(image.height() as i32) {
        for xx in x.max(0)..(x + w).min(image.width() as i32) {
            image.put_pixel(xx as u32, yy as u32, colour);
        }
    }
}
use view::{
    draw_preview, intersect, next_event, preview_bounds, put_bytes, to_x_bytes, view_layout,
    view_rect, Area, Layout, Preview, RefineTimer, ViewCache, ViewKey, TOOLBAR,
};
use x11rb::{
    connection::Connection,
    protocol::{xproto::*, Event},
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

fn key_name(c: &RustConnection, code: u8) -> u32 {
    c.get_keyboard_mapping(code, 1)
        .ok()
        .and_then(|r| r.reply().ok())
        .and_then(|m| m.keysyms.first().copied())
        .unwrap_or(0)
}

fn draw_rect(img: &mut RgbaImage, x: i32, y: i32, w: i32, h: i32, colour: Rgba<u8>) {
    if w <= 0 || h <= 0 {
        return;
    }
    let (x0, y0, x1, y1) = (
        x.max(0) as u32,
        y.max(0) as u32,
        (x + w).max(0) as u32,
        (y + h).max(0) as u32,
    );
    for yy in y0..y1.min(img.height()) {
        for xx in x0..x1.min(img.width()) {
            if xx < x0 + 3 || xx + 3 >= x1 || yy < y0 + 3 || yy + 3 >= y1 {
                img.put_pixel(xx, yy, colour);
            }
        }
    }
}

fn arrow(img: &mut RgbaImage, x0: i32, y0: i32, x1: i32, y1: i32, colour: Rgba<u8>) {
    line(img, x0, y0, x1, y1, colour);
    let angle = ((y1 - y0) as f32).atan2((x1 - x0) as f32);
    let length = 14.0;
    for offset in [0.45_f32, -0.45] {
        let end_x = x1 as f32 - (angle + offset).cos() * length;
        let end_y = y1 as f32 - (angle + offset).sin() * length;
        line(
            img,
            x1,
            y1,
            end_x.round() as i32,
            end_y.round() as i32,
            colour,
        );
    }
}

fn line(img: &mut RgbaImage, mut x0: i32, mut y0: i32, x1: i32, y1: i32, colour: Rgba<u8>) {
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    loop {
        for ox in -1..=1 {
            for oy in -1..=1 {
                let x = x0 + ox;
                let y = y0 + oy;
                if x >= 0 && y >= 0 && (x as u32) < img.width() && (y as u32) < img.height() {
                    img.put_pixel(x as u32, y as u32, colour);
                }
            }
        }
        if x0 == x1 && y0 == y1 {
            break;
        }
        let e2 = 2 * err;
        if e2 >= dy {
            err += dy;
            x0 += sx;
        }
        if e2 <= dx {
            err += dx;
            y0 += sy;
        }
    }
}

fn toolbar_mode(font: &FontArc, x: i32) -> Option<u8> {
    let mut pos = 118.0;
    for (key, label) in [
        (b'c', "Crop"),
        (b'r', "Rect"),
        (b'a', "Arrow"),
        (b'm', "Mask"),
    ] {
        let width = 28.0 + text_width(font, 12.0, label);
        if (x as f32) >= pos && (x as f32) < pos + width {
            return Some(key);
        }
        pos += width + 8.0;
    }
    None
}

fn intern_atom(c: &RustConnection, name: &str) -> Option<Atom> {
    c.intern_atom(false, name.as_bytes())
        .ok()
        .and_then(|r| r.reply().ok())
        .map(|r| r.atom)
}

fn close_review(c: &RustConnection, win: Window, temp: &Path) {
    let _ = std::fs::remove_file(temp);
    let _ = c.destroy_window(win);
    let _ = c.flush();
}

fn draw_toolbar(
    canvas: &mut RgbaImage,
    theme: &Theme,
    font: Option<&FontArc>,
    mode: u8,
    zoom: f32,
) {
    let win_w = canvas.width();
    fill_rect(
        canvas,
        0,
        0,
        win_w as i32,
        TOOLBAR as i32,
        rgb(theme.surface),
    );
    fill_rect(canvas, 0, 0, win_w as i32, 2, rgb(theme.accent));
    fill_rect(
        canvas,
        0,
        TOOLBAR as i32 - 1,
        win_w as i32,
        1,
        rgb(theme.border),
    );
    let Some(font) = font else {
        return;
    };
    draw_text(
        canvas,
        font,
        14.0,
        16.0,
        10.0,
        "NIRUSHOT",
        rgb(theme.foreground),
    );
    let mut pos = 118.0;
    for (key, label) in [
        (b'c', "Crop"),
        (b'r', "Rect"),
        (b'a', "Arrow"),
        (b'm', "Mask"),
    ] {
        let width = 28.0 + text_width(font, 12.0, label);
        let active = mode == key;
        if active {
            fill_rect(
                canvas,
                pos as i32,
                13,
                width as i32,
                26,
                rgb(theme.foreground),
            );
        }
        let colour = if active {
            rgb(theme.background)
        } else {
            rgb(theme.muted)
        };
        draw_text(canvas, font, 12.0, pos + 9.0, 20.0, label, colour);
        pos += width + 8.0;
    }
    let zoom_label = if (zoom - 1.0).abs() < 0.01 {
        "fit".to_string()
    } else {
        format!("{:.0}%", zoom * 100.0)
    };
    draw_text(
        canvas,
        font,
        12.0,
        pos + 10.0,
        20.0,
        &zoom_label,
        rgb(theme.muted),
    );
    let line1 = "s save   y copy   u share   esc close";
    let line2 = "+/- zoom   0 fit   v view";
    let hint_width = text_width(font, 11.0, line1).max(text_width(font, 11.0, line2));
    let hint_x = win_w as f32 - hint_width - 16.0;
    draw_text(canvas, font, 11.0, hint_x, 7.0, line1, rgb(theme.muted));
    draw_text(canvas, font, 11.0, hint_x, 27.0, line2, rgb(theme.muted));
}

/// The composed frame lives in a server pixmap the size of the window.
struct Frame {
    pixmap: Pixmap,
    bytes: Vec<u8>,
    toolbar: RgbaImage,
}

impl Frame {
    fn new(c: &RustConnection, win: Window, depth: u8, w: u32, h: u32) -> Result<Self, String> {
        let pixmap = c.generate_id().map_err(|e| e.to_string())?;
        c.create_pixmap(depth, pixmap, win, w as u16, h as u16)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            pixmap,
            bytes: Vec::new(),
            toolbar: RgbaImage::new(w, TOOLBAR),
        })
    }
}

/// Everything needed to paint the image area of the frame.
struct Painter<'a> {
    c: &'a RustConnection,
    gc: Gcontext,
    depth: u8,
    lsb_first: bool,
    background: Rgba<u8>,
}

impl Painter<'_> {
    fn pixel(&self, p: &Rgba<u8>) -> [u8; 4] {
        if self.lsb_first {
            [p[2], p[1], p[0], 255]
        } else {
            [p[0], p[1], p[2], 255]
        }
    }

    /// Paint window area `area` (background plus cached view) into the
    /// pixmap and copy it to the window.
    fn area(
        &self,
        frame: &mut Frame,
        win: Window,
        cache: &ViewCache,
        layout: &Layout,
        area: Area,
    ) -> Result<(), String> {
        let (ax, ay, aw, ah) = area;
        if aw <= 0 || ah <= 0 {
            return Ok(());
        }
        let bg = self.pixel(&self.background);
        let bytes = &mut frame.bytes;
        bytes.clear();
        bytes.resize(aw as usize * ah as usize * 4, 0);
        for px in bytes.chunks_exact_mut(4) {
            px.copy_from_slice(&bg);
        }
        if let Some(key) = cache.key {
            let (ox, oy) = layout.origin();
            let placed = (ox + key.rect.0, oy + key.rect.1, key.rect.2, key.rect.3);
            if let Some((x, y, w, h)) = intersect(placed, area) {
                let src = cache.image.as_raw();
                let stride = cache.image.width() as usize * 4;
                for row in 0..h {
                    let sy = (y - placed.1 + row) as usize;
                    let sx = (x - placed.0) as usize;
                    let from = &src[sy * stride + sx * 4..sy * stride + (sx + w as usize) * 4];
                    let dy = (y - ay + row) as usize;
                    let dx = (x - ax) as usize;
                    let to = &mut bytes
                        [(dy * aw as usize + dx) * 4..(dy * aw as usize + dx + w as usize) * 4];
                    for (d, s) in to.chunks_exact_mut(4).zip(from.chunks_exact(4)) {
                        d.copy_from_slice(&self.pixel(&Rgba([s[0], s[1], s[2], 255])));
                    }
                }
            }
        }
        put_bytes(
            self.c,
            frame.pixmap,
            self.gc,
            self.depth,
            ax as i16,
            ay as i16,
            aw as u32,
            ah as u32,
            bytes,
        )?;
        self.c
            .copy_area(
                frame.pixmap,
                win,
                self.gc,
                ax as i16,
                ay as i16,
                ax as i16,
                ay as i16,
                aw as u16,
                ah as u16,
            )
            .map_err(|e| e.to_string())?;
        Ok(())
    }

    fn toolbar(&self, frame: &mut Frame, win: Window) -> Result<(), String> {
        to_x_bytes(self.lsb_first, &frame.toolbar, &mut frame.bytes);
        let (w, h) = frame.toolbar.dimensions();
        put_bytes(
            self.c,
            frame.pixmap,
            self.gc,
            self.depth,
            0,
            0,
            w,
            h,
            &frame.bytes,
        )?;
        self.c
            .copy_area(frame.pixmap, win, self.gc, 0, 0, 0, 0, w as u16, h as u16)
            .map_err(|e| e.to_string())?;
        Ok(())
    }
}

fn review(
    mut image: RgbaImage,
    temp: &Path,
    save: Option<&Path>,
    notify_enabled: bool,
) -> Result<(), String> {
    let (c, screen) = x11rb::connect(None).map_err(|e| e.to_string())?;
    let c = Arc::new(c);
    let root = c.setup().roots[screen].root;
    let depth = c.setup().roots[screen].root_depth;
    let lsb_first = c.setup().image_byte_order == ImageOrder::LSB_FIRST;
    let sw = c.setup().roots[screen].width_in_pixels as u32;
    let sh = c.setup().roots[screen].height_in_pixels as u32;
    let max_w = (sw as f32 * 0.90).max(320.0);
    let max_h = (sh as f32 * 0.84 - TOOLBAR as f32).max(200.0);
    let initial = (max_w / image.width() as f32)
        .min(max_h / image.height() as f32)
        .min(1.0);
    let mut win_w = ((image.width() as f32 * initial) as u32)
        .max(760)
        .min(sw.saturating_sub(24));
    let mut win_h = TOOLBAR + (image.height() as f32 * initial).max(1.0) as u32;
    let win = c.generate_id().map_err(|e| e.to_string())?;
    let gc = c.generate_id().map_err(|e| e.to_string())?;
    let preview_gc = c.generate_id().map_err(|e| e.to_string())?;
    let x = ((sw as i32 - win_w as i32) / 2).max(0) as i16;
    let y = ((sh as i32 - win_h as i32) / 2).max(0) as i16;
    c.create_window(
        0,
        win,
        root,
        x,
        y,
        win_w as u16,
        win_h as u16,
        0,
        WindowClass::INPUT_OUTPUT,
        0,
        &CreateWindowAux::new()
            .background_pixel(0x000000)
            .event_mask(
                EventMask::EXPOSURE
                    | EventMask::BUTTON_PRESS
                    | EventMask::BUTTON_RELEASE
                    | EventMask::POINTER_MOTION
                    | EventMask::KEY_PRESS
                    | EventMask::STRUCTURE_NOTIFY
                    | EventMask::FOCUS_CHANGE,
            ),
    )
    .map_err(|e| e.to_string())?;
    c.create_gc(gc, win, &CreateGCAux::new().graphics_exposures(0))
        .map_err(|e| e.to_string())?;
    c.create_gc(preview_gc, win, &CreateGCAux::new().graphics_exposures(0))
        .map_err(|e| e.to_string())?;
    c.change_property8(
        PropMode::REPLACE,
        win,
        AtomEnum::WM_NAME,
        AtomEnum::STRING,
        b"NIRUSHOT \xe2\x80\x94 screenshot review",
    )
    .ok();
    c.change_property8(
        PropMode::REPLACE,
        win,
        AtomEnum::WM_CLASS,
        AtomEnum::STRING,
        b"nirushot\0Nirushot\0",
    )
    .ok();
    if let (Some(kind), Some(dialog)) = (
        intern_atom(&c, "_NET_WM_WINDOW_TYPE"),
        intern_atom(&c, "_NET_WM_WINDOW_TYPE_DIALOG"),
    ) {
        let _ = c.change_property32(PropMode::REPLACE, win, kind, AtomEnum::ATOM, &[dialog]);
    }
    let wm_protocols = intern_atom(&c, "WM_PROTOCOLS").unwrap_or(0);
    let wm_delete = intern_atom(&c, "WM_DELETE_WINDOW").unwrap_or(0);
    let refine_atom = intern_atom(&c, "_NIRUSHOT_REFINE").unwrap_or(0);
    if wm_protocols != 0 && wm_delete != 0 {
        let _ = c.change_property32(
            PropMode::REPLACE,
            win,
            wm_protocols,
            AtomEnum::ATOM,
            &[wm_delete],
        );
    }
    let mut frame = Frame::new(&c, win, depth, win_w, win_h)?;
    c.map_window(win).map_err(|e| e.to_string())?;
    c.flush().map_err(|e| e.to_string())?;
    let mut refine = RefineTimer::new(Arc::clone(&c), win, refine_atom);
    let theme = load_theme();
    let font = load_font();
    let mut mode = b'0';
    let mut zoom = 1.0f32;
    let mut pan = (0.0f32, 0.0f32);
    let mut drag: Option<(i32, i32)> = None;
    let mut current: Option<(i32, i32)> = None;
    let mut pan_drag: Option<(i32, i32, f32, f32)> = None;
    let mut generation = 0u64;
    let mut cache = ViewCache::default();
    let mut fine = false;
    let mut need_frame = true;
    let mut need_toolbar = true;
    // Image areas changed by the last annotation, re-scaled on their own.
    let mut changed: Vec<Area> = Vec::new();
    let painter = Painter {
        c: &c,
        gc,
        depth,
        lsb_first,
        background: rgb(theme.background),
    };
    let mut need_preview = false;
    let mut drawn: Option<(i32, i32, i32, i32)> = None;
    let mut pending: Option<Event> = None;
    let mut layout = view_layout(image.dimensions(), (win_w, win_h), zoom, pan);
    loop {
        if need_frame || need_toolbar {
            draw_toolbar(&mut frame.toolbar, &theme, font.as_ref(), mode, zoom);
            painter.toolbar(&mut frame, win)?;
            need_toolbar = false;
        }
        if need_frame {
            layout = view_layout(image.dimensions(), (win_w, win_h), zoom, pan);
            if let Some(rect) = view_rect(&layout, (win_w, win_h)) {
                let key = ViewKey {
                    generation,
                    disp: (layout.disp_w, layout.disp_h),
                    rect,
                };
                if cache.update(&image, key, fine) {
                    refine.schedule();
                }
            } else {
                cache.key = None;
            }
            painter.area(&mut frame, win, &cache, &layout, image_area(win_w, win_h))?;
            let clip = Rectangle {
                x: 0,
                y: TOOLBAR as i16,
                width: win_w as u16,
                height: win_h.saturating_sub(TOOLBAR) as u16,
            };
            c.set_clip_rectangles(ClipOrdering::UNSORTED, preview_gc, 0, 0, &[clip])
                .map_err(|e| e.to_string())?;
            changed.clear();
            drawn = None;
            need_frame = false;
            need_preview = true;
        } else if !changed.is_empty() {
            match cache.patch(&image, generation, &changed) {
                Some(areas) => {
                    let (ox, oy) = layout.origin();
                    for (x, y, w, h) in areas {
                        if let Some(area) =
                            intersect((ox + x, oy + y, w, h), image_area(win_w, win_h))
                        {
                            painter.area(&mut frame, win, &cache, &layout, area)?;
                        }
                    }
                    need_preview = true;
                }
                None => need_frame = true,
            }
            changed.clear();
            if need_frame {
                continue;
            }
        }
        if need_preview {
            if let Some((x, y, w, h)) = drawn.take() {
                let x0 = x.clamp(0, win_w as i32);
                let y0 = y.clamp(0, win_h as i32);
                let x1 = (x + w).clamp(0, win_w as i32);
                let y1 = (y + h).clamp(0, win_h as i32);
                if x1 > x0 && y1 > y0 {
                    c.copy_area(
                        frame.pixmap,
                        win,
                        gc,
                        x0 as i16,
                        y0 as i16,
                        x0 as i16,
                        y0 as i16,
                        (x1 - x0) as u16,
                        (y1 - y0) as u16,
                    )
                    .map_err(|e| e.to_string())?;
                }
            }
            if let (Some(from), Some(to)) = (drag, current) {
                let colour = match mode {
                    b'm' => 0x000000,
                    b'c' => 0xdcc88c,
                    _ => 0xdc4646,
                };
                let preview = Preview {
                    mode,
                    from,
                    to,
                    colour,
                    stroke: 3,
                };
                draw_preview(&c, win, preview_gc, &preview)?;
                drawn = Some(preview_bounds(&preview));
            }
            need_preview = false;
        }
        c.flush().map_err(|e| e.to_string())?;
        let event = next_event(&c, &mut pending)?;
        match event {
            Event::Expose(e) if e.window == win => {
                c.copy_area(
                    frame.pixmap,
                    win,
                    gc,
                    e.x as i16,
                    e.y as i16,
                    e.x as i16,
                    e.y as i16,
                    e.width,
                    e.height,
                )
                .map_err(|e| e.to_string())?;
                if drawn.is_some() {
                    need_preview = true;
                }
            }
            Event::MapNotify(e) if e.window == win => {
                // A normal focused dialog: no keyboard or pointer grab. An
                // active grab here would stop NIRULOCK (or any locker) from
                // taking input while a review is left open. Drags still get
                // X's implicit button grab, which ends on release.
                let _ = c.set_input_focus(InputFocus::PARENT, win, x11rb::CURRENT_TIME);
            }
            Event::ClientMessage(e)
                if e.window == win
                    && e.type_ == wm_protocols
                    && e.data.as_data32()[0] == wm_delete =>
            {
                close_review(&c, win, temp);
                return Ok(());
            }
            Event::ClientMessage(e) if e.window == win && e.type_ == refine_atom => {
                if refine.is_current(e.data.as_data32()[0]) && !fine {
                    fine = true;
                    need_frame = true;
                }
            }
            Event::ConfigureNotify(e) if e.window == win => {
                if e.width as u32 != win_w || e.height as u32 != win_h {
                    win_w = e.width.max(320) as u32;
                    win_h = e.height.max(TOOLBAR as u16 + 80) as u32;
                    let _ = c.free_pixmap(frame.pixmap);
                    frame = Frame::new(&c, win, depth, win_w, win_h)?;
                    fine = false;
                    need_frame = true;
                }
            }
            Event::ButtonPress(e) if e.event == win => {
                let _ = c.set_input_focus(InputFocus::PARENT, win, x11rb::CURRENT_TIME);
                if e.detail == 4 || e.detail == 5 {
                    zoom = if e.detail == 4 {
                        (zoom * 1.25).min(8.0)
                    } else {
                        (zoom / 1.25).max(0.1)
                    };
                    pan = (0.0, 0.0);
                    fine = false;
                    need_frame = true;
                    continue;
                }
                if e.detail == 2 {
                    pan_drag = Some((e.event_x as i32, e.event_y as i32, pan.0, pan.1));
                    continue;
                }
                if e.detail != 1 {
                    continue;
                }
                if e.event_y < TOOLBAR as i16 {
                    if let Some(font) = &font {
                        if let Some(selected) = toolbar_mode(font, e.event_x as i32) {
                            mode = selected;
                            need_toolbar = true;
                        }
                    }
                    continue;
                }
                if mode == b'0' {
                    pan_drag = Some((e.event_x as i32, e.event_y as i32, pan.0, pan.1));
                } else {
                    drag = Some((e.event_x as i32, e.event_y as i32));
                    current = drag;
                    need_preview = true;
                }
            }
            Event::MotionNotify(e) => {
                if let Some((sx, sy, px, py)) = pan_drag {
                    pan = (
                        px + (e.event_x as i32 - sx) as f32,
                        py + (e.event_y as i32 - sy) as f32,
                    );
                    fine = false;
                    need_frame = true;
                } else if drag.is_some() {
                    current = Some((e.event_x as i32, e.event_y as i32));
                    need_preview = true;
                }
            }
            Event::ButtonRelease(e) if e.event == win => {
                if e.detail == 2 || pan_drag.is_some() {
                    pan_drag = None;
                }
                if e.detail == 1 {
                    need_preview = true;
                    if let (Some((x0, y0)), Some((x1, y1))) = (drag.take(), current.take()) {
                        let dims = image.dimensions();
                        let (ix0, iy0) = layout.image_point(dims, x0, y0);
                        let (ix1, iy1) = layout.image_point(dims, x1, y1);
                        let left = ix0.min(ix1) as u32;
                        let top = iy0.min(iy1) as u32;
                        let width = (ix1 - ix0).unsigned_abs();
                        let height = (iy1 - iy0).unsigned_abs();
                        let is_arrow = mode == b'a';
                        if (width > 2 && height > 2) || (is_arrow && (width > 2 || height > 2)) {
                            let width = width.min(image.width() - left);
                            let height = height.min(image.height() - top);
                            match mode {
                                b'c' => {
                                    image =
                                        image::imageops::crop_imm(&image, left, top, width, height)
                                            .to_image();
                                    zoom = 1.0;
                                    pan = (0.0, 0.0);
                                    need_frame = true;
                                }
                                b'r' => draw_rect(
                                    &mut image,
                                    left as i32,
                                    top as i32,
                                    width as i32,
                                    height as i32,
                                    Rgba([220, 70, 70, 255]),
                                ),
                                b'm' => fill_rect(
                                    &mut image,
                                    left as i32,
                                    top as i32,
                                    width as i32,
                                    height as i32,
                                    Rgba([0, 0, 0, 255]),
                                ),
                                b'a' => {
                                    arrow(&mut image, ix0, iy0, ix1, iy1, Rgba([220, 70, 70, 255]))
                                }
                                _ => {}
                            }
                            generation += 1;
                            changed =
                                changed_areas(mode, ix0, iy0, ix1, iy1, (left, top, width, height));
                        }
                    }
                }
            }
            Event::KeyPress(e) if e.event == win => {
                let key = key_name(&c, e.detail);
                match key {
                    0xff1b => {
                        close_review(&c, win, temp);
                        return Ok(());
                    }
                    0x63 | 0x72 | 0x61 | 0x6d | 0x76 => {
                        mode = match key {
                            0x63 => b'c',
                            0x72 => b'r',
                            0x61 => b'a',
                            0x6d => b'm',
                            _ => b'0',
                        };
                        need_toolbar = true;
                        continue;
                    }
                    0x2b | 0x3d | 0xffab => {
                        zoom = (zoom * 1.25).min(8.0);
                        pan = (0.0, 0.0);
                        fine = false;
                    }
                    0x2d | 0xffad => {
                        zoom = (zoom / 1.25).max(0.1);
                        pan = (0.0, 0.0);
                        fine = false;
                    }
                    0x30 | 0x66 => {
                        zoom = 1.0;
                        pan = (0.0, 0.0);
                        fine = false;
                    }
                    0x73 | 0xff0d => {
                        let path = save.map(Path::to_path_buf).unwrap_or_else(default_path);
                        atomic_save(&image, &path)?;
                        let copied = copy_to_clipboard(&path).is_ok();
                        println!("{}", path.display());
                        if notify_enabled {
                            notify(&path.to_string_lossy(), copied);
                        }
                        close_review(&c, win, temp);
                        return Ok(());
                    }
                    0x79 => {
                        image
                            .save(temp)
                            .map_err(|e| format!("save {}: {e}", temp.display()))?;
                        copy_to_clipboard(temp)?;
                        println!("{}", temp.display());
                        if notify_enabled {
                            notify(&temp.to_string_lossy(), true);
                        }
                    }
                    0x75 => {
                        let state = env::var_os("XDG_STATE_HOME")
                            .map(PathBuf::from)
                            .unwrap_or_else(|| {
                                env::var_os("HOME")
                                    .map(PathBuf::from)
                                    .unwrap_or_else(|| PathBuf::from("."))
                                    .join(".local/state")
                            })
                            .join("nirushare");
                        std::fs::create_dir_all(&state)
                            .map_err(|e| format!("create {}: {e}", state.display()))?;
                        let path = state.join(format!(
                            "review-{}.png",
                            chrono::Local::now().format("%Y%m%d-%H%M%S-%3f")
                        ));
                        image
                            .save(&path)
                            .map_err(|e| format!("save {}: {e}", path.display()))?;
                        Command::new("nirulauncher")
                            .arg("--share")
                            .env("NIRUSHARE_SEND_FILE", &path)
                            .spawn()
                            .map_err(|e| format!("start NIRU Share: {e}"))?;
                        close_review(&c, win, temp);
                        return Ok(());
                    }
                    _ => continue,
                }
                // Any handled key may change the toolbar or the view.
                need_frame = true;
            }
            Event::Error(e) => eprintln!("nirushot: X11 error: {e:?}"),
            _ => {}
        }
    }
}

fn image_area(win_w: u32, win_h: u32) -> Area {
    (
        0,
        TOOLBAR as i32,
        win_w as i32,
        win_h.saturating_sub(TOOLBAR) as i32,
    )
}

/// Image areas an annotation drew into.
fn changed_areas(
    mode: u8,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    rect: (u32, u32, u32, u32),
) -> Vec<Area> {
    let (l, t, w, h) = (rect.0 as i32, rect.1 as i32, rect.2 as i32, rect.3 as i32);
    match mode {
        b'r' => vec![
            (l, t, w, 3),
            (l, t + h - 3, w, 3),
            (l, t, 3, h),
            (l + w - 3, t, 3, h),
        ],
        b'a' => {
            let pad = 18;
            vec![(
                x0.min(x1) - pad,
                y0.min(y1) - pad,
                (x1 - x0).abs() + 2 * pad,
                (y1 - y0).abs() + 2 * pad,
            )]
        }
        _ => vec![(l, t, w, h)],
    }
}

fn default_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    let directory = home.join("Pictures");
    let stem = format!(
        "Screenshot-{}",
        chrono::Local::now().format("%Y-%m-%d_%H-%M-%S")
    );
    let first = directory.join(format!("{stem}.png"));
    if !first.exists() {
        return first;
    }
    for index in 2..1000 {
        let candidate = directory.join(format!("{stem}-{index}.png"));
        if !candidate.exists() {
            return candidate;
        }
    }
    first
}

fn atomic_save(image: &RgbaImage, path: &Path) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| format!("create {}: {e}", parent.display()))?;
    }
    let name = path
        .file_name()
        .ok_or_else(|| format!("invalid output path: {}", path.display()))?
        .to_string_lossy();
    let temporary = path.with_file_name(format!(".{name}.nirushot-{}.png", std::process::id()));
    image
        .save(&temporary)
        .map_err(|e| format!("save {}: {e}", temporary.display()))?;
    std::fs::rename(&temporary, path).map_err(|e| format!("replace {}: {e}", path.display()))?;
    Ok(())
}

fn copy_to_clipboard(path: &Path) -> Result<(), String> {
    let input = File::open(path).map_err(|e| format!("open {}: {e}", path.display()))?;
    let mut child = Command::new("xclip")
        .args(["-selection", "clipboard", "-t", "image/png", "-i"])
        .stdin(Stdio::from(input))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot start xclip: {e}"))?;
    // xclip usually stays alive while it owns the selection, so waiting for it
    // to exit would hang the screenshot. Give it a short window: an immediate
    // unsuccessful exit is a real failure, while a process still running after
    // that window is the normal clipboard owner.
    let deadline = Instant::now() + Duration::from_millis(250);
    loop {
        match child.try_wait() {
            Ok(Some(status)) if status.success() => return Ok(()),
            Ok(Some(status)) => {
                let mut detail = String::new();
                if let Some(mut stderr) = child.stderr.take() {
                    let _ = stderr.read_to_string(&mut detail);
                }
                let detail = detail.trim();
                return if detail.is_empty() {
                    Err(format!("xclip exited with {status}"))
                } else {
                    Err(format!("xclip exited with {status}: {detail}"))
                };
            }
            Ok(None) => {}
            Err(error) => return Err(format!("could not wait for xclip: {error}")),
        }
        if Instant::now() >= deadline {
            return Ok(());
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if let Some(index) = args.iter().position(|a| a == "--notify-result") {
        if let Some(path) = args.get(index + 1) {
            notify_and_handle(Path::new(path), args.iter().any(|a| a == "--copied"));
        }
        return;
    }
    if args.iter().any(|a| a == "--version") {
        println!("NIRUSHOT {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args.iter().any(|a| a == "--help") {
        println!("Usage: nirushot [--region|--window|--full] [--delay SECONDS] [--copy|--save PATH] [--review] [--no-notify]");
        return;
    }
    let fail = |message: &str| -> ! {
        eprintln!("nirushot: {message}");
        std::process::exit(2);
    };
    let mut mode = "region";
    let mut delay = 0u64;
    let mut copy = false;
    let mut save: Option<PathBuf> = None;
    let mut review_flag = false;
    let mut notify_enabled = true;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--region" => mode = "region",
            "--window" => mode = "window",
            "--full" => mode = "full",
            "--delay" => {
                i += 1;
                let value = args.get(i).unwrap_or_else(|| fail("--delay needs seconds"));
                delay = value
                    .parse()
                    .unwrap_or_else(|_| fail("--delay needs a non-negative number"));
            }
            "--copy" => copy = true,
            "--save" => {
                i += 1;
                let value = args.get(i).unwrap_or_else(|| fail("--save needs a path"));
                if value.is_empty() {
                    fail("--save needs a non-empty path");
                }
                save = Some(PathBuf::from(value));
            }
            "--review" => review_flag = true,
            "--no-notify" => notify_enabled = false,
            other => fail(&format!("unknown option {other}")),
        }
        i += 1;
    }
    if delay > 0 {
        thread::sleep(Duration::from_secs(delay));
    }
    let tempdir = tempfile::Builder::new()
        .prefix("nirushot-")
        .tempdir()
        .unwrap_or_else(|e| fail(&format!("cannot create private temporary directory: {e}")));
    let temp = tempdir.path().join("capture.png");
    let mut maim = vec!["--quiet".to_string()];
    match mode {
        "region" => {
            let geometry = match run(
                "slop",
                &[
                    "-f".into(),
                    "%wx%h+%x+%y".into(),
                    "-b".into(),
                    "4".into(),
                    "-c".into(),
                    "1.0,0.72,0.15,1.0".into(),
                ],
            ) {
                Ok(g) if !g.is_empty() => g,
                Ok(_) => {
                    println!("cancelled");
                    return;
                }
                Err(e) => fail(&format!("selection failed: {e}")),
            };
            maim.extend(["-g".into(), geometry]);
        }
        "window" => {
            let id = match active_window() {
                Ok(id) => id,
                Err(e) => fail(&e),
            };
            maim.extend(["-i".into(), id]);
        }
        _ => {}
    }
    maim.push(temp.to_string_lossy().to_string());
    if let Err(e) = run("maim", &maim) {
        fail(&format!("capture failed: {e}"));
    }
    // A capture is always available for paste. Review may replace it with an
    // edited image, but Save or Review must never leave the clipboard empty.
    let copied_initial = copy_to_clipboard(&temp);
    if review_flag || (save.is_none() && !copy) {
        let image = match image::open(&temp) {
            Ok(image) => image.to_rgba8(),
            Err(e) => fail(&format!("cannot read capture: {e}")),
        };
        if let Err(e) = review(image, &temp, save.as_deref(), notify_enabled) {
            fail(&e);
        }
        return;
    }
    if let Some(path) = save.as_deref() {
        let image = match image::open(&temp) {
            Ok(image) => image.to_rgba8(),
            Err(e) => fail(&format!("cannot read capture: {e}")),
        };
        if let Err(e) = atomic_save(&image, path) {
            fail(&e);
        }
        let copied = copy_to_clipboard(path).is_ok();
        println!("{}", path.display());
        if notify_enabled {
            notify(&path.to_string_lossy(), copied);
        }
        return;
    }
    if let Err(error) = copied_initial {
        fail(&format!("copy failed: {error}"));
    }
    println!("{}", temp.display());
    if notify_enabled {
        let _ = Command::new("notify-send")
            .args([
                "-a",
                "NIRUSHOT",
                "-t",
                "6000",
                "Screenshot",
                "Copied to the clipboard",
                temp.to_string_lossy().as_ref(),
            ])
            .status();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn atomic_save_writes_complete_png() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("shot.png");
        let image = RgbaImage::from_pixel(4, 3, Rgba([10, 20, 30, 255]));
        atomic_save(&image, &path).unwrap();
        let decoded = image::open(&path).unwrap().to_rgba8();
        assert_eq!(decoded.dimensions(), (4, 3));
        let pixel = decoded.get_pixel(0, 0);
        assert_eq!([pixel[0], pixel[1], pixel[2]], [10, 20, 30]);
    }

    #[test]
    fn arrow_has_a_head_and_axis_aligned_support() {
        let mut image = RgbaImage::from_pixel(100, 40, Rgba([255, 255, 255, 255]));
        arrow(&mut image, 5, 20, 90, 20, Rgba([220, 70, 70, 255]));
        let red =
            |x: u32, y: u32| image.get_pixel(x, y)[0] == 220 && image.get_pixel(x, y)[1] == 70;
        assert!(red(5, 20), "shaft start");
        assert!(red(90, 20), "arrow tip");
        assert!(
            red(80, 15) || red(80, 25),
            "arrow head opens around the tip"
        );
    }
}
