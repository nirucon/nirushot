//! `nirushot ocr`: copy the text in a screen region.
//!
//! The screen is frozen the way `nirushot pick` freezes it (one server-side
//! copy of the root window shown in an override-redirect window), the region
//! is selected with `slop` exactly like a region capture, and the pixels come
//! from the frozen copy. The region is converted to grey, turned dark-on-light
//! when the theme is dark, upscaled and padded, then read by `tesseract`.
//!
//! tesseract is an optional runtime dependency. Without it, `nirushot ocr`
//! says how to install it on Debian, Arch and Void.

use std::{
    env, fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use image::{imageops, GrayImage, Luma, RgbaImage};
use x11rb::{connection::Connection, protocol::xproto::*, wrapper::ConnectionExt as _};

use crate::{config_dir, copy_text_to_clipboard, run};

/// Languages tried when nothing is configured.
pub const DEFAULT_LANGUAGES: &str = "swe+eng";
/// Upper bound on the upscaled image, in pixels.
const MAX_PIXELS: u64 = 24_000_000;
/// The upscale factor for screen text (about 10 px cap height at 96 dpi).
const SCALE: u32 = 3;
/// White margin around the region after scaling.
const PAD: u32 = 24;
const TESSERACT_TIMEOUT: Duration = Duration::from_secs(60);

// ---------------------------------------------------------------------------
// Configuration: `[ocr]` in ~/.config/nirushot/config.toml.

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Config {
    pub languages: Option<String>,
    pub psm: Option<u32>,
    /// `None` means the default (true).
    pub keep_newlines: Option<bool>,
}

impl Config {
    pub fn keep_newlines(&self) -> bool {
        self.keep_newlines.unwrap_or(true)
    }
}

pub fn config_path() -> PathBuf {
    config_dir().join("nirushot/config.toml")
}

fn toml_value(raw: &str) -> String {
    let raw = raw.trim();
    for quote in ['"', '\''] {
        if let Some(rest) = raw.strip_prefix(quote) {
            return rest.split(quote).next().unwrap_or("").to_string();
        }
    }
    raw.split('#').next().unwrap_or("").trim().to_string()
}

/// The `[ocr]` table of a config file. Unknown keys and other tables are
/// ignored; bad values are reported and ignored.
pub fn parse_config(text: &str) -> (Config, Vec<String>) {
    let mut config = Config::default();
    let mut problems = Vec::new();
    let mut in_ocr = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            let name = line.trim_start_matches('[').split(']').next().unwrap_or("");
            in_ocr = name.trim() == "ocr";
            continue;
        }
        if !in_ocr {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = toml_value(value);
        match key.trim() {
            "languages" => {
                if valid_languages(&value) {
                    config.languages = Some(value);
                } else {
                    problems.push(format!("[ocr] languages: invalid value {value:?}"));
                }
            }
            "psm" => match value.parse::<u32>() {
                Ok(n) if n <= 13 => config.psm = Some(n),
                _ => problems.push(format!("[ocr] psm: expected 0-13, got {value:?}")),
            },
            "keep_newlines" => match value.as_str() {
                "true" => config.keep_newlines = Some(true),
                "false" => config.keep_newlines = Some(false),
                _ => problems.push(format!(
                    "[ocr] keep_newlines: expected true or false, got {value:?}"
                )),
            },
            _ => {}
        }
    }
    (config, problems)
}

pub fn load_config() -> Config {
    let path = config_path();
    let Ok(text) = fs::read_to_string(&path) else {
        return Config::default();
    };
    let (config, problems) = parse_config(&text);
    for p in problems {
        eprintln!("nirushot ocr: {}: {p}", path.display());
    }
    config
}

/// Language codes such as `swe+eng` or `chi_sim`: never anything that could
/// be read as an option.
fn valid_languages(s: &str) -> bool {
    !s.is_empty()
        && s.split('+').all(|l| {
            !l.is_empty()
                && l.len() <= 32
                && l.bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
                && !l.starts_with('-')
        })
}

// ---------------------------------------------------------------------------
// Distribution and packages, detected the way `nirux` does.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Distro {
    Debian,
    Arch,
    Void,
    Unknown,
}

/// `ID` from os-release first, then the release files NIRUX also checks.
pub fn distro_from(os_release: Option<&str>, exists: impl Fn(&str) -> bool) -> Distro {
    if let Some(text) = os_release {
        let id = text.lines().find_map(|l| {
            let value = l.trim().strip_prefix("ID=")?;
            Some(
                value
                    .trim()
                    .trim_matches('"')
                    .trim_matches('\'')
                    .to_string(),
            )
        });
        match id.as_deref() {
            Some("debian") => return Distro::Debian,
            Some("arch") => return Distro::Arch,
            Some("void") => return Distro::Void,
            _ => {}
        }
    }
    if exists("/etc/void-release") {
        Distro::Void
    } else if exists("/etc/arch-release") {
        Distro::Arch
    } else if exists("/etc/debian_version") {
        Distro::Debian
    } else {
        Distro::Unknown
    }
}

pub fn distro() -> Distro {
    let text = fs::read_to_string("/etc/os-release").ok();
    distro_from(text.as_deref(), |p| Path::new(p).exists())
}

/// The package that holds a language's traineddata.
pub fn language_package(d: Distro, lang: &str) -> String {
    match d {
        Distro::Arch => format!("tesseract-data-{lang}"),
        _ => format!("tesseract-ocr-{lang}"),
    }
}

pub fn engine_package(d: Distro) -> &'static str {
    match d {
        Distro::Arch => "tesseract",
        _ => "tesseract-ocr",
    }
}

/// The command that installs `packages`.
pub fn install_command(d: Distro, packages: &[String]) -> String {
    let list = packages.join(" ");
    match d {
        Distro::Debian => format!("sudo apt install {list}"),
        Distro::Arch => format!("sudo pacman -S {list}"),
        Distro::Void => format!("sudo xbps-install -S {list}"),
        Distro::Unknown => list,
    }
}

/// Install hints for the engine plus the default languages, per distro.
pub fn full_install_hint(d: Distro) -> String {
    let one = |d: Distro| {
        let mut p = vec![engine_package(d).to_string()];
        p.extend(DEFAULT_LANGUAGES.split('+').map(|l| language_package(d, l)));
        install_command(d, &p)
    };
    match d {
        Distro::Unknown => format!(
            "Debian: {}\nArch: {}\nVoid: {}",
            one(Distro::Debian),
            one(Distro::Arch),
            one(Distro::Void)
        ),
        d => one(d),
    }
}

/// Void ships the engine as `tesseract-ocr`, because its `tesseract` is a
/// game; never run that one there.
pub fn engine_names(d: Distro) -> &'static [&'static str] {
    match d {
        Distro::Void => &["tesseract-ocr"],
        _ => &["tesseract", "tesseract-ocr"],
    }
}

fn find_in_path(name: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    env::split_paths(&env::var_os("PATH")?)
        .map(|dir| dir.join(name))
        .find(|p| {
            fs::metadata(p)
                .map(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
                .unwrap_or(false)
        })
}

pub fn find_engine(d: Distro) -> Option<PathBuf> {
    engine_names(d).iter().find_map(|n| find_in_path(n))
}

// ---------------------------------------------------------------------------
// Languages.

/// `tesseract --list-langs` output: a header line, then one code per line.
pub fn parse_language_list(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.contains(' ') && !l.ends_with(':'))
        .map(str::to_string)
        .collect()
}

/// What to pass to `-l`, and which requested languages are not installed.
/// With none of the requested languages installed, English is used if it
/// is there, else the first real language (never `osd` or `equ`).
pub fn choose_languages(
    requested: &str,
    available: &[String],
) -> Result<(Vec<String>, Vec<String>), String> {
    let mut chosen = Vec::new();
    let mut missing = Vec::new();
    for lang in requested.split('+').filter(|l| !l.is_empty()) {
        if available.iter().any(|a| a == lang) {
            if !chosen.iter().any(|c| c == lang) {
                chosen.push(lang.to_string());
            }
        } else if !missing.iter().any(|m| m == lang) {
            missing.push(lang.to_string());
        }
    }
    if chosen.is_empty() {
        let real = |a: &&String| a.as_str() != "osd" && a.as_str() != "equ";
        let fallback = available
            .iter()
            .find(|a| a.as_str() == "eng")
            .or_else(|| available.iter().find(real));
        match fallback {
            Some(l) => chosen.push(l.clone()),
            None => return Err("tesseract has no language data installed".into()),
        }
    }
    Ok((chosen, missing))
}

// ---------------------------------------------------------------------------
// Image preparation and recognition.

/// The factor for a region: three times, less for huge regions.
pub fn scale_for(width: u32, height: u32) -> u32 {
    let mut scale = SCALE;
    while scale > 1 && (width as u64 * height as u64) * (scale as u64).pow(2) > MAX_PIXELS {
        scale -= 1;
    }
    scale
}

/// Grey, dark text on a light background, upscaled and padded with white.
pub fn prepare(image: &RgbaImage) -> (GrayImage, u32) {
    let (w, h) = image.dimensions();
    let mut grey = GrayImage::new(w, h);
    let mut sum = 0u64;
    for (x, y, p) in image.enumerate_pixels() {
        let [r, g, b, _] = p.0;
        let y_ = (0.299 * r as f32 + 0.587 * g as f32 + 0.114 * b as f32).round() as u8;
        sum += y_ as u64;
        grey.put_pixel(x, y, Luma([y_]));
    }
    let pixels = (w as u64 * h as u64).max(1);
    // A dark background (most NIRUX themes): invert so the text is dark.
    if sum / pixels < 128 {
        imageops::invert(&mut grey);
    }
    let scale = scale_for(w, h);
    let scaled = if scale > 1 {
        imageops::resize(
            &grey,
            w * scale,
            h * scale,
            imageops::FilterType::CatmullRom,
        )
    } else {
        grey
    };
    let mut out = GrayImage::from_pixel(
        scaled.width() + 2 * PAD,
        scaled.height() + 2 * PAD,
        Luma([255]),
    );
    imageops::replace(&mut out, &scaled, PAD as i64, PAD as i64);
    (out, scale)
}

/// Tidy tesseract's output: no form feed or trailing spaces, at most one
/// blank line between paragraphs. Without `keep_newlines`, one line.
pub fn clean_text(raw: &str, keep_newlines: bool) -> String {
    let text = raw.replace(['\r', '\x0c'], "");
    if !keep_newlines {
        return text.split_whitespace().collect::<Vec<_>>().join(" ");
    }
    let mut out: Vec<&str> = Vec::new();
    for line in text.lines().map(str::trim_end) {
        if line.trim().is_empty() {
            if out.last().is_some_and(|l| !l.is_empty()) {
                out.push("");
            }
        } else {
            out.push(line);
        }
    }
    while out.last().is_some_and(|l| l.is_empty()) {
        out.pop();
    }
    out.join("\n")
}

fn wait_output(mut child: Child, timeout: Duration) -> Result<(Vec<u8>, Vec<u8>, bool), String> {
    let mut stdout = child.stdout.take();
    let mut stderr = child.stderr.take();
    let out = thread::spawn(move || {
        let mut v = Vec::new();
        if let Some(s) = stdout.as_mut() {
            let _ = s.read_to_end(&mut v);
        }
        v
    });
    let err = thread::spawn(move || {
        let mut v = Vec::new();
        if let Some(s) = stderr.as_mut() {
            let _ = s.read_to_end(&mut v);
        }
        v
    });
    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(10)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(format!("timed out after {} s", timeout.as_secs()));
            }
            Err(e) => return Err(e.to_string()),
        }
    };
    Ok((
        out.join().unwrap_or_default(),
        err.join().unwrap_or_default(),
        status.success(),
    ))
}

pub struct Engine {
    pub path: PathBuf,
    pub languages: Vec<String>,
}

impl Engine {
    pub fn version(&self) -> String {
        Command::new(&self.path)
            .arg("--version")
            .stdin(Stdio::null())
            .output()
            .map(|o| {
                let text = String::from_utf8_lossy(&o.stdout).to_string()
                    + &String::from_utf8_lossy(&o.stderr);
                text.lines().next().unwrap_or("").trim().to_string()
            })
            .unwrap_or_default()
    }

    pub fn open(path: PathBuf) -> Result<Self, String> {
        let child = Command::new(&path)
            .arg("--list-langs")
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot start {}: {e}", path.display()))?;
        let (out, err, _) = wait_output(child, Duration::from_secs(10))?;
        // Old versions print the list on stderr.
        let mut languages = parse_language_list(&String::from_utf8_lossy(&out));
        if languages.is_empty() {
            languages = parse_language_list(&String::from_utf8_lossy(&err));
        }
        Ok(Self { path, languages })
    }

    /// Read the text in `image`.
    pub fn recognise(
        &self,
        image: &RgbaImage,
        languages: &[String],
        psm: Option<u32>,
    ) -> Result<String, String> {
        let (prepared, scale) = prepare(image);
        let dir = tempfile::Builder::new()
            .prefix("nirushot-ocr-")
            .tempdir()
            .map_err(|e| format!("cannot create private temporary directory: {e}"))?;
        let input = dir.path().join("region.png");
        prepared
            .save(&input)
            .map_err(|e| format!("save {}: {e}", input.display()))?;
        let mut cmd = Command::new(&self.path);
        cmd.arg(&input)
            .arg("stdout")
            .arg("-l")
            .arg(languages.join("+"))
            .arg("--dpi")
            .arg((96 * scale).to_string());
        if let Some(psm) = psm {
            cmd.arg("--psm").arg(psm.to_string());
        }
        // OpenMP threads cost more than they win on a screen region.
        if env::var_os("OMP_THREAD_LIMIT").is_none() {
            cmd.env("OMP_THREAD_LIMIT", "1");
        }
        let child = cmd
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot start {}: {e}", self.path.display()))?;
        let (out, err, ok) =
            wait_output(child, TESSERACT_TIMEOUT).map_err(|e| format!("tesseract: {e}"))?;
        if !ok {
            let detail = String::from_utf8_lossy(&err).trim().to_string();
            return Err(format!("tesseract failed: {detail}"));
        }
        Ok(String::from_utf8_lossy(&out).to_string())
    }
}

/// Everything `ocr` needs before it touches the screen.
pub struct Setup {
    pub engine: Engine,
    pub languages: Vec<String>,
    pub missing: Vec<String>,
    pub distro: Distro,
}

pub fn missing_note(d: Distro, missing: &[String]) -> Option<String> {
    if missing.is_empty() {
        return None;
    }
    let packages: Vec<String> = missing.iter().map(|l| language_package(d, l)).collect();
    Some(format!(
        "Missing language data: {}. Install: {}",
        missing.join(", "),
        install_command(d, &packages)
    ))
}

pub fn setup(config: &Config, override_languages: Option<&str>) -> Result<Setup, String> {
    let d = distro();
    let Some(path) = find_engine(d) else {
        return Err(format!(
            "tesseract is not installed. Install it with:\n{}",
            full_install_hint(d)
        ));
    };
    let engine = Engine::open(path)?;
    let requested = override_languages
        .or(config.languages.as_deref())
        .unwrap_or(DEFAULT_LANGUAGES);
    let (languages, missing) = choose_languages(requested, &engine.languages)?;
    Ok(Setup {
        engine,
        languages,
        missing,
        distro: d,
    })
}

// ---------------------------------------------------------------------------
// Clipboard and notification.

/// The clipboard's text, if it holds text, so Undo can put it back.
fn clipboard_text() -> Option<String> {
    let read = |target: &str| -> Option<Vec<u8>> {
        let child = Command::new("xclip")
            .args(["-selection", "clipboard", "-o", "-t", target])
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .ok()?;
        let (out, _, ok) = wait_output(child, Duration::from_millis(800)).ok()?;
        ok.then_some(out)
    };
    let targets = String::from_utf8(read("TARGETS")?).ok()?;
    let target = ["UTF8_STRING", "text/plain;charset=utf-8", "STRING"]
        .into_iter()
        .find(|t| targets.lines().any(|l| l.trim() == *t))?;
    let bytes = read(target)?;
    if bytes.is_empty() || bytes.len() > 4 << 20 {
        return None;
    }
    String::from_utf8(bytes).ok()
}

/// The notification's first line, shortened.
pub fn first_line(text: &str) -> String {
    let line = text
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .trim();
    let max = 80;
    if line.chars().count() > max {
        format!("{}…", line.chars().take(max - 1).collect::<String>())
    } else {
        line.to_string()
    }
}

pub fn summary_and_body(text: &str, note: Option<&str>) -> (String, String) {
    let n = text.chars().count();
    let summary = format!(
        "Text copied ({n} character{})",
        if n == 1 { "" } else { "s" }
    );
    let mut body = first_line(text);
    if let Some(note) = note {
        body.push('\n');
        body.push_str(note);
    }
    (summary, body)
}

fn notify_supports_actions() -> bool {
    Command::new("notify-send")
        .arg("--help")
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).contains("--action"))
        .unwrap_or(false)
}

/// A plain notification; waits a little for the daemon, never long.
pub fn notify_plain(summary: &str, body: &str) {
    let child = Command::new("notify-send")
        .args(["-a", "NIRUSHOT", "-t", "8000", summary, body])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    if let Ok(mut child) = child {
        let deadline = Instant::now() + Duration::from_millis(1500);
        while Instant::now() < deadline {
            if let Ok(Some(_)) = child.try_wait() {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
    }
}

/// `prev` then `text`, for the notification process's stdin.
pub fn encode_payload(prev: Option<&str>, text: &str) -> Vec<u8> {
    let mut out = match prev {
        Some(p) => format!("{}\n{p}", p.len()).into_bytes(),
        None => b"-\n".to_vec(),
    };
    out.extend_from_slice(text.as_bytes());
    out
}

pub fn decode_payload(data: &[u8]) -> Option<(Option<String>, String)> {
    let nl = data.iter().position(|b| *b == b'\n')?;
    let head = std::str::from_utf8(&data[..nl]).ok()?;
    let rest = &data[nl + 1..];
    if head == "-" {
        return Some((None, String::from_utf8(rest.to_vec()).ok()?));
    }
    let len: usize = head.parse().ok()?;
    let prev = String::from_utf8(rest.get(..len)?.to_vec()).ok()?;
    let text = String::from_utf8(rest.get(len..)?.to_vec()).ok()?;
    Some((Some(prev), text))
}

/// Notify after a copy. With action support this hands over to a detached
/// `nirushot ocr --notify-result` that waits for Copy again or Undo, so the
/// caller returns at once.
pub fn notify_copied(text: &str, prev: Option<String>, note: Option<&str>) {
    let (summary, body) = summary_and_body(text, note);
    if !notify_supports_actions() {
        notify_plain(&summary, &body);
        return;
    }
    let exe = env::current_exe().unwrap_or_else(|_| PathBuf::from("nirushot"));
    let child = Command::new(exe)
        .args(["ocr", "--notify-result", &summary, &body])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn();
    match child {
        Ok(mut child) => {
            if let Some(mut stdin) = child.stdin.take() {
                let _ = stdin.write_all(&encode_payload(prev.as_deref(), text));
            }
        }
        Err(_) => notify_plain(&summary, &body),
    }
}

fn notify_result(summary: &str, body: &str) -> i32 {
    let mut data = Vec::new();
    let _ = std::io::stdin().read_to_end(&mut data);
    let Some((prev, text)) = decode_payload(&data) else {
        return 2;
    };
    let mut args = vec![
        "-a".to_string(),
        "NIRUSHOT".into(),
        "-t".into(),
        "10000".into(),
        "-A".into(),
        "again=Copy again".into(),
    ];
    if prev.is_some() {
        args.extend(["-A".into(), "undo=Undo".into()]);
    }
    args.extend([summary.to_string(), body.to_string()]);
    let action = Command::new("notify-send")
        .args(&args)
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default();
    match action.as_str() {
        "again" => {
            let _ = copy_text_to_clipboard(&text);
        }
        "undo" => {
            if let Some(prev) = prev {
                let _ = copy_text_to_clipboard(&prev);
            }
        }
        _ => {}
    }
    0
}

/// Read `image`, copy the text and notify. Used by `ocr` and the review's
/// Copy text key. Returns the text, or None when there was none.
pub fn copy_text_from(
    image: &RgbaImage,
    setup: &Setup,
    config: &Config,
    psm: Option<u32>,
    notify: bool,
) -> Result<Option<String>, String> {
    let raw = setup
        .engine
        .recognise(image, &setup.languages, psm.or(config.psm))?;
    let text = clean_text(&raw, config.keep_newlines());
    let note = missing_note(setup.distro, &setup.missing);
    if text.trim().is_empty() {
        if notify {
            let body = note.unwrap_or_else(|| "The clipboard is unchanged.".into());
            notify_plain("No text found", &body);
        }
        return Ok(None);
    }
    let prev = if notify { clipboard_text() } else { None };
    copy_text_to_clipboard(&text).map_err(|e| format!("copy failed: {e}"))?;
    if notify {
        notify_copied(&text, prev, note.as_deref());
    }
    Ok(Some(text))
}

// ---------------------------------------------------------------------------
// The frozen screen.

fn err(e: impl ToString) -> String {
    e.to_string()
}

/// slop exits unsuccessfully when Escape or a right click cancels it.
pub fn slop_cancelled(stderr: &str) -> bool {
    stderr.contains("cancelled")
}

/// `WxH+X+Y`, as slop prints it and `--geometry` takes it.
pub fn parse_geometry(s: &str) -> Option<(i32, i32, i32, i32)> {
    let (size, rest) = s.trim().split_once('+')?;
    let (w, h) = size.split_once('x')?;
    let (x, y) = rest.split_once('+')?;
    let (w, h, x, y) = (
        w.parse().ok()?,
        h.parse().ok()?,
        x.parse().ok()?,
        y.parse().ok()?,
    );
    (w > 0 && h > 0).then_some((x, y, w, h))
}

enum Selection {
    Region(RgbaImage),
    Cancelled,
}

/// Freeze the screen, let the user select a region (or take `geometry`),
/// and return that region of the frozen frame.
fn freeze_and_select(geometry: Option<&str>) -> Result<Selection, String> {
    let (c, screen) = x11rb::connect(None).map_err(|e| format!("cannot open the display: {e}"))?;
    let setup = c.setup();
    let s = &setup.roots[screen];
    let (root, depth) = (s.root, s.root_depth);
    let (rw, rh) = (s.width_in_pixels as i32, s.height_in_pixels as i32);
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
    let frozen = c.generate_id().map_err(err)?;
    let gc = c.generate_id().map_err(err)?;
    c.create_pixmap(depth, frozen, root, rw as u16, rh as u16)
        .map_err(err)?;
    c.create_gc(
        gc,
        root,
        &CreateGCAux::new()
            .subwindow_mode(SubwindowMode::INCLUDE_INFERIORS)
            .graphics_exposures(0),
    )
    .map_err(err)?;
    c.copy_area(root, frozen, gc, 0, 0, 0, 0, rw as u16, rh as u16)
        .map_err(err)?;
    let win = c.generate_id().map_err(err)?;
    c.create_window(
        x11rb::COPY_DEPTH_FROM_PARENT,
        win,
        root,
        0,
        0,
        rw as u16,
        rh as u16,
        0,
        WindowClass::INPUT_OUTPUT,
        x11rb::COPY_FROM_PARENT,
        &CreateWindowAux::new()
            .background_pixmap(frozen)
            .override_redirect(1)
            .save_under(0),
    )
    .map_err(err)?;
    c.change_property8(
        PropMode::REPLACE,
        win,
        AtomEnum::WM_NAME,
        AtomEnum::STRING,
        b"NIRUSHOT OCR",
    )
    .map_err(err)?;
    c.change_property8(
        PropMode::REPLACE,
        win,
        AtomEnum::WM_CLASS,
        AtomEnum::STRING,
        b"nirushot-ocr\0Nirushot\0",
    )
    .map_err(err)?;
    c.map_window(win).map_err(err)?;
    c.sync().map_err(err)?;

    let picked = match geometry {
        Some(g) => Ok(Some(g.to_string())),
        None => run(
            "slop",
            &[
                "-f".into(),
                "%wx%h+%x+%y".into(),
                "-b".into(),
                "4".into(),
                "-c".into(),
                "1.0,0.72,0.15,1.0".into(),
            ],
        )
        .map(|g| (!g.is_empty()).then_some(g))
        .or_else(|e| {
            if slop_cancelled(&e) {
                Ok(None)
            } else {
                Err(format!("selection failed: {e}"))
            }
        }),
    };
    let result = picked.and_then(|g| {
        let Some(g) = g else {
            return Ok(Selection::Cancelled);
        };
        let area = parse_geometry(&g).ok_or_else(|| format!("bad geometry {g:?}"))?;
        let Some((x, y, w, h)) = crate::view::intersect(area, (0, 0, rw, rh)) else {
            return Err("the region is outside the screen".into());
        };
        let reply = c
            .get_image(
                ImageFormat::Z_PIXMAP,
                frozen,
                x as i16,
                y as i16,
                w as u16,
                h as u16,
                !0,
            )
            .map_err(err)?
            .reply()
            .map_err(err)?;
        let mut img = RgbaImage::new(w as u32, h as u32);
        for (i, p) in img.pixels_mut().enumerate() {
            let Some(b) = reply.data.get(i * 4..i * 4 + 4) else {
                break;
            };
            p.0 = if lsb_first {
                [b[2], b[1], b[0], 255]
            } else {
                [b[1], b[2], b[3], 255]
            };
        }
        Ok(Selection::Region(img))
    });
    let _ = c.destroy_window(win);
    let _ = c.free_pixmap(frozen);
    let _ = c.free_gc(gc);
    let _ = c.sync();
    result
}

// ---------------------------------------------------------------------------

pub fn usage() -> &'static str {
    "Usage: nirushot ocr [--check] [--geometry WxH+X+Y | --image PNG] [--languages L] [--psm N] [--no-notify]"
}

fn check(config: &Config, languages: Option<&str>) -> i32 {
    let d = distro();
    println!("NIRUSHOT OCR check");
    println!("  distribution: {d:?}");
    println!(
        "  config: {}{}",
        config_path().display(),
        if config_path().exists() {
            ""
        } else {
            " (not present, defaults)"
        }
    );
    let Some(path) = find_engine(d) else {
        println!("  tesseract: not installed");
        println!("  install with:");
        for line in full_install_hint(d).lines() {
            println!("    {line}");
        }
        return 2;
    };
    let engine = match Engine::open(path) {
        Ok(e) => e,
        Err(e) => {
            println!("  tesseract: {e}");
            return 2;
        }
    };
    println!(
        "  tesseract: {} ({})",
        engine.path.display(),
        engine.version()
    );
    println!("  installed languages: {}", engine.languages.join(", "));
    let requested = languages
        .or(config.languages.as_deref())
        .unwrap_or(DEFAULT_LANGUAGES);
    println!("  requested languages: {requested}");
    let code = match choose_languages(requested, &engine.languages) {
        Ok((chosen, missing)) => {
            println!("  using: {}", chosen.join("+"));
            match missing_note(d, &missing) {
                Some(note) => {
                    println!("  {note}");
                    1
                }
                None => 0,
            }
        }
        Err(e) => {
            println!("  {e}");
            println!("  install with: {}", full_install_hint(d));
            2
        }
    };
    println!(
        "  psm: {}, keep_newlines: {}",
        config
            .psm
            .map(|p| p.to_string())
            .unwrap_or_else(|| "tesseract default (3)".into()),
        config.keep_newlines()
    );
    if find_in_path("xclip").is_none() {
        println!("  xclip: not installed (needed for the clipboard)");
        return 2;
    }
    code
}

/// `nirushot ocr`: 0 text copied, 1 cancelled or no text, 2 error.
pub fn main(args: &[String]) -> i32 {
    if args.first().map(String::as_str) == Some("--notify-result") {
        let summary = args.get(1).map(String::as_str).unwrap_or("Text copied");
        let body = args.get(2).map(String::as_str).unwrap_or("");
        return notify_result(summary, body);
    }
    let mut geometry = None;
    let mut image_path = None;
    let mut languages = None;
    let mut psm = None;
    let mut notify = true;
    let mut want_check = false;
    let mut i = 0;
    let fail = |m: &str| {
        eprintln!("nirushot ocr: {m}");
        eprintln!("{}", usage());
        2
    };
    while i < args.len() {
        let value = |i: usize| args.get(i + 1).cloned();
        match args[i].as_str() {
            "--check" => want_check = true,
            "--no-notify" => notify = false,
            "--geometry" => match value(i).filter(|g| parse_geometry(g).is_some()) {
                Some(g) => {
                    geometry = Some(g);
                    i += 1;
                }
                None => return fail("--geometry needs WxH+X+Y"),
            },
            "--image" => match value(i).filter(|p| !p.is_empty()) {
                Some(p) => {
                    image_path = Some(PathBuf::from(p));
                    i += 1;
                }
                None => return fail("--image needs a path"),
            },
            "--languages" => match value(i).filter(|l| valid_languages(l)) {
                Some(l) => {
                    languages = Some(l);
                    i += 1;
                }
                None => return fail("--languages needs codes such as swe+eng"),
            },
            "--psm" => match value(i)
                .and_then(|v| v.parse::<u32>().ok())
                .filter(|p| *p <= 13)
            {
                Some(p) => {
                    psm = Some(p);
                    i += 1;
                }
                None => return fail("--psm needs 0-13"),
            },
            "--help" | "-h" => {
                println!("{}", usage());
                return 0;
            }
            other => return fail(&format!("unknown option {other}")),
        }
        i += 1;
    }
    let config = load_config();
    if want_check {
        return check(&config, languages.as_deref());
    }
    // Find the engine before the user selects anything.
    let setup = match setup(&config, languages.as_deref()) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("nirushot ocr: {e}");
            if notify {
                let body = e.split_once('\n').map(|x| x.1).unwrap_or(&e).to_string();
                notify_plain("OCR needs tesseract", &body);
            }
            return 2;
        }
    };
    if let Some(note) = missing_note(setup.distro, &setup.missing) {
        eprintln!("nirushot ocr: {note}");
    }
    let image = match image_path {
        Some(path) => match image::open(&path) {
            Ok(i) => i.to_rgba8(),
            Err(e) => {
                eprintln!("nirushot ocr: cannot read {}: {e}", path.display());
                return 2;
            }
        },
        None => match freeze_and_select(geometry.as_deref()) {
            Ok(Selection::Region(img)) => img,
            Ok(Selection::Cancelled) => {
                println!("cancelled");
                return 1;
            }
            Err(e) => {
                eprintln!("nirushot ocr: {e}");
                return 2;
            }
        },
    };
    match copy_text_from(&image, &setup, &config, psm, notify) {
        Ok(Some(text)) => {
            println!("{text}");
            0
        }
        Ok(None) => {
            eprintln!("nirushot ocr: no text found");
            1
        }
        Err(e) => {
            eprintln!("nirushot ocr: {e}");
            if notify {
                notify_plain("OCR failed", &e);
            }
            2
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    #[test]
    fn config_reads_only_the_ocr_table() {
        let text = r#"
# comment
languages = "deu"
[ocr]
languages = "swe+eng"   # Swedish first
psm = 6
keep_newlines = false
[other]
psm = 9
"#;
        let (c, problems) = parse_config(text);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(c.languages.as_deref(), Some("swe+eng"));
        assert_eq!(c.psm, Some(6));
        assert!(!c.keep_newlines());
        assert!(Config::default().keep_newlines());
    }

    #[test]
    fn config_rejects_bad_values() {
        let (c, problems) =
            parse_config("[ocr]\nlanguages = \"-c foo\"\npsm = 42\nkeep_newlines = yes\n");
        assert_eq!(c, Config::default());
        assert_eq!(problems.len(), 3, "{problems:?}");
        assert!(valid_languages("chi_sim+eng"));
        assert!(!valid_languages("swe++eng"));
        assert!(!valid_languages(""));
    }

    #[test]
    fn distro_detection_follows_nirux() {
        let none = |_: &str| false;
        assert_eq!(
            distro_from(Some("NAME=x\nID=debian\n"), none),
            Distro::Debian
        );
        assert_eq!(distro_from(Some("ID=\"arch\"\n"), none), Distro::Arch);
        assert_eq!(distro_from(Some("ID=void\n"), none), Distro::Void);
        assert_eq!(
            distro_from(Some("ID=ubuntu\n"), |p| p == "/etc/debian_version"),
            Distro::Debian
        );
        assert_eq!(
            distro_from(None, |p| p == "/etc/void-release"),
            Distro::Void
        );
        assert_eq!(distro_from(None, none), Distro::Unknown);
    }

    #[test]
    fn install_hints_per_distro() {
        assert_eq!(
            full_install_hint(Distro::Debian),
            "sudo apt install tesseract-ocr tesseract-ocr-swe tesseract-ocr-eng"
        );
        assert_eq!(
            full_install_hint(Distro::Arch),
            "sudo pacman -S tesseract tesseract-data-swe tesseract-data-eng"
        );
        assert_eq!(
            full_install_hint(Distro::Void),
            "sudo xbps-install -S tesseract-ocr tesseract-ocr-swe tesseract-ocr-eng"
        );
        assert_eq!(full_install_hint(Distro::Unknown).lines().count(), 3);
        assert_eq!(engine_names(Distro::Void), ["tesseract-ocr"]);
        assert_eq!(
            missing_note(Distro::Arch, &["swe".into()]).unwrap(),
            "Missing language data: swe. Install: sudo pacman -S tesseract-data-swe"
        );
        assert_eq!(missing_note(Distro::Debian, &[]), None);
    }

    #[test]
    fn language_choice() {
        let list = parse_language_list(
            "List of available languages in \"/usr/share/tessdata/\" (3):\neng\nosd\nswe\n",
        );
        assert_eq!(list, ["eng", "osd", "swe"]);
        let (c, m) = choose_languages("swe+eng", &list).unwrap();
        assert_eq!((c, m), (vec!["swe".to_string(), "eng".into()], vec![]));
        let only_eng = vec!["eng".to_string(), "osd".into()];
        let (c, m) = choose_languages("swe+eng", &only_eng).unwrap();
        assert_eq!((c, m), (vec!["eng".to_string()], vec!["swe".to_string()]));
        // None of the requested: English, else the first real language.
        let (c, m) = choose_languages("swe", &only_eng).unwrap();
        assert_eq!((c, m), (vec!["eng".to_string()], vec!["swe".to_string()]));
        let deu = vec!["osd".to_string(), "deu".into()];
        assert_eq!(choose_languages("swe+eng", &deu).unwrap().0, ["deu"]);
        assert!(choose_languages("swe", &["osd".to_string()]).is_err());
    }

    #[test]
    fn cleaning_text() {
        let raw = "Hello  world  \n\n\n\nSecond line\nthird\n\n\x0c";
        assert_eq!(clean_text(raw, true), "Hello  world\n\nSecond line\nthird");
        assert_eq!(clean_text(raw, false), "Hello world Second line third");
        assert_eq!(clean_text("\n\n \x0c", true), "");
    }

    #[test]
    fn notification_text() {
        let (s, b) = summary_and_body("\n  First line  \nsecond", None);
        assert_eq!(s, "Text copied (22 characters)");
        assert_eq!(b, "First line");
        let (s, _) = summary_and_body("å", Some("note"));
        assert_eq!(s, "Text copied (1 character)");
        let long = "x".repeat(200);
        assert_eq!(first_line(&long).chars().count(), 80);
    }

    #[test]
    fn payload_round_trips() {
        for (prev, text) in [
            (None, "åäö\ntext"),
            (Some("old\nclip"), "new"),
            (Some(""), ""),
        ] {
            let data = encode_payload(prev, text);
            assert_eq!(
                decode_payload(&data),
                Some((prev.map(str::to_string), text.to_string()))
            );
        }
        assert_eq!(decode_payload(b"garbage"), None);
        assert_eq!(decode_payload(b"99\nshort"), None);
    }

    #[test]
    fn slop_cancel_is_not_an_error() {
        let e = "Failed to detect a compositor, OpenGL hardware-acceleration disabled...\n\
                 Selection was cancelled by keystroke or right-click.";
        assert!(slop_cancelled(e));
        assert!(!slop_cancelled("Failed to grab the mouse!"));
    }

    #[test]
    fn geometry() {
        assert_eq!(parse_geometry("100x50+20+30"), Some((20, 30, 100, 50)));
        assert_eq!(parse_geometry("0x50+20+30"), None);
        assert_eq!(parse_geometry("100x50"), None);
        assert_eq!(parse_geometry("axb+1+2"), None);
    }

    #[test]
    fn preparation_inverts_dark_scales_and_pads() {
        let mut img = RgbaImage::from_pixel(10, 4, Rgba([20, 20, 20, 255]));
        img.put_pixel(0, 0, Rgba([240, 240, 240, 255]));
        let (out, scale) = prepare(&img);
        assert_eq!(scale, 3);
        assert_eq!(out.dimensions(), (30 + 2 * PAD, 12 + 2 * PAD));
        // Dark background became light, the light pixel dark.
        assert!(out.get_pixel(PAD + 15, PAD + 6).0[0] > 200);
        assert!(out.get_pixel(PAD, PAD).0[0] < 60);
        assert_eq!(out.get_pixel(0, 0).0[0], 255);
        // A light image is not inverted.
        let light = RgbaImage::from_pixel(4, 4, Rgba([250, 250, 250, 255]));
        assert!(prepare(&light).0.get_pixel(PAD + 2, PAD + 2).0[0] > 240);
    }

    #[test]
    fn huge_regions_are_scaled_less() {
        assert_eq!(scale_for(800, 200), 3);
        assert_eq!(scale_for(1920, 1080), 3);
        assert_eq!(scale_for(2560, 1440), 2);
        assert_eq!(scale_for(3840, 2160), 1);
    }

    // -----------------------------------------------------------------------
    // Fixtures: rendered text, read back by the real tesseract. Ignored by
    // default; CI installs tesseract and runs them.

    fn levenshtein(a: &str, b: &str) -> usize {
        let a: Vec<char> = a.chars().collect();
        let b: Vec<char> = b.chars().collect();
        let mut row: Vec<usize> = (0..=b.len()).collect();
        for i in 1..=a.len() {
            let mut prev = row[0];
            row[0] = i;
            for j in 1..=b.len() {
                let cur = row[j];
                row[j] = (row[j] + 1)
                    .min(row[j - 1] + 1)
                    .min(prev + usize::from(a[i - 1] != b[j - 1]));
                prev = cur;
            }
        }
        row[b.len()]
    }

    fn render(lines: &[&str], size: f32, fg: [u8; 3], bg: [u8; 3]) -> RgbaImage {
        let font = crate::load_font().expect("a monospace font (fonts-dejavu-core)");
        let width = lines
            .iter()
            .map(|l| crate::text_width(&font, size, l))
            .fold(0.0f32, f32::max)
            .ceil() as u32
            + 24;
        let line_h = (size * 1.5).ceil() as u32;
        let mut img = RgbaImage::from_pixel(
            width,
            line_h * lines.len() as u32 + 16,
            Rgba([bg[0], bg[1], bg[2], 255]),
        );
        for (i, l) in lines.iter().enumerate() {
            crate::draw_text(
                &mut img,
                &font,
                size,
                12.0,
                8.0 + (i as u32 * line_h) as f32,
                l,
                Rgba([fg[0], fg[1], fg[2], 255]),
            );
        }
        img
    }

    pub const ENGLISH: [&str; 3] = [
        "The quick brown fox jumps over the lazy dog.",
        "Copy text from a screen region: 0123456789",
        "Pack my box with five dozen liquor jugs!",
    ];
    pub const SWEDISH: [&str; 3] = [
        "Flygande bäckasiner söka hwila på mjuka tuvor.",
        "Räksmörgås åt jag på Öland i går, ÅÄÖ åäö.",
        "Färgen på skärmen är mörk och texten är ljus.",
    ];

    fn engine() -> Setup {
        setup(&Config::default(), None).expect("tesseract with swe and eng installed")
    }

    /// Character accuracy of each fixture; every case must be exact or
    /// nearly so.
    #[test]
    #[ignore = "needs tesseract with swe and eng"]
    fn fixtures_are_read_back_in_english_and_swedish() {
        let setup = engine();
        assert_eq!(setup.languages, ["swe", "eng"], "both languages installed");
        let themes = [
            ("dark", [0xe5, 0xe5, 0xe5], [0x0f, 0x0f, 0x10]),
            ("light", [0x3b, 0x32, 0x28], [0xf3, 0xec, 0xe0]),
            ("nord", [0xec, 0xef, 0xf4], [0x2e, 0x34, 0x40]),
        ];
        let mut total = 0usize;
        let mut errors = 0usize;
        for (name, lines) in [("English", &ENGLISH), ("Swedish", &SWEDISH)] {
            for size in [13.0f32, 16.0, 22.0] {
                for (theme, fg, bg) in themes {
                    let img = render(lines, size, fg, bg);
                    let raw = setup
                        .engine
                        .recognise(&img, &setup.languages, None)
                        .unwrap();
                    let got = clean_text(&raw, true);
                    let want = lines.join("\n");
                    let e = levenshtein(&got, &want);
                    let rate = 1.0 - e as f64 / want.chars().count() as f64;
                    total += want.chars().count();
                    errors += e;
                    eprintln!(
                        "{name:8} {size:>4} px {theme:5}: {:6.2}% ({e} edits){}",
                        rate * 100.0,
                        if e > 0 {
                            format!("\n    got: {got:?}")
                        } else {
                            String::new()
                        }
                    );
                    assert!(rate >= 0.97, "{name} {size} px {theme}: {got:?}");
                }
            }
        }
        let overall = 100.0 * (1.0 - errors as f64 / total as f64);
        eprintln!("overall character accuracy {overall:.2}% ({errors} edits in {total})");
        assert!(overall >= 99.0, "overall {overall:.2}%");
    }

    /// The Swedish letters survive at the usual UI size.
    #[test]
    #[ignore = "needs tesseract with swe and eng"]
    fn swedish_letters_are_exact_at_ui_size() {
        let setup = engine();
        let img = render(&[SWEDISH[1]], 16.0, [0xe5, 0xe5, 0xe5], [0x0f, 0x0f, 0x10]);
        let raw = setup
            .engine
            .recognise(&img, &setup.languages, None)
            .unwrap();
        assert_eq!(clean_text(&raw, true), SWEDISH[1]);
    }
}
