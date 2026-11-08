mod edit;
mod keys;
mod monitor;
mod pick;
mod review;
mod sound;
mod view;

use std::{
    env,
    fs::File,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
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

use ab_glyph::FontArc;
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

use edit::{draw_text, text_width};

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
    let child = Command::new("xclip")
        .args(["-selection", "clipboard", "-t", "image/png", "-i"])
        .stdin(Stdio::from(input))
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot start xclip: {e}"))?;
    wait_for_xclip(child)
}

/// Put `text` on the clipboard as UTF-8 text.
fn copy_text_to_clipboard(text: &str) -> Result<(), String> {
    let mut child = Command::new("xclip")
        .args(["-selection", "clipboard", "-i"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("cannot start xclip: {e}"))?;
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| format!("write to xclip: {e}"))?;
    }
    wait_for_xclip(child)
}

fn wait_for_xclip(mut child: std::process::Child) -> Result<(), String> {
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
    if args.first().map(String::as_str) == Some("pick") {
        std::process::exit(pick::main(&args[1..]));
    }
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
        println!("       {}", pick::usage());
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
    // `fail` exits without running destructors: remove the private
    // directory (and the capture in it) first.
    let dir = tempdir.path().to_path_buf();
    let fail_clean = |message: &str| -> ! {
        let _ = std::fs::remove_dir_all(&dir);
        fail(message)
    };
    let temp = tempdir.path().join("capture.png");
    let mut maim = vec!["--quiet".to_string()];
    let mut hint = None;
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
                Err(e) => fail_clean(&format!("selection failed: {e}")),
            };
            hint = monitor::geometry_centre(&geometry);
            maim.extend(["-g".into(), geometry]);
        }
        "window" => {
            let id = match active_window() {
                Ok(id) => id,
                Err(e) => fail_clean(&e),
            };
            maim.extend(["-i".into(), id]);
        }
        _ => {}
    }
    maim.push(temp.to_string_lossy().to_string());
    if let Err(e) = run("maim", &maim) {
        fail_clean(&format!("capture failed: {e}"));
    }
    // The shutter: the picture is taken now, whatever happens next.
    sound::play("screenshot");
    // A capture is always available for paste. Review may replace it with an
    // edited image, but Save or Review must never leave the clipboard empty.
    let copied_initial = copy_to_clipboard(&temp);
    if review_flag || (save.is_none() && !copy) {
        let image = match image::open(&temp) {
            Ok(image) => image.to_rgba8(),
            Err(e) => fail_clean(&format!("cannot read capture: {e}")),
        };
        let output = review::Output {
            temp: &temp,
            save: save.as_deref(),
            notify: notify_enabled,
            hint,
        };
        if let Err(e) = review::review(image, output) {
            fail_clean(&e);
        }
        return;
    }
    if let Some(path) = save.as_deref() {
        let image = match image::open(&temp) {
            Ok(image) => image.to_rgba8(),
            Err(e) => fail_clean(&format!("cannot read capture: {e}")),
        };
        if let Err(e) = atomic_save(&image, path) {
            fail_clean(&e);
        }
        let copied = copy_to_clipboard(path).is_ok();
        println!("{}", path.display());
        if notify_enabled {
            notify(&path.to_string_lossy(), copied);
        }
        return;
    }
    if let Err(error) = copied_initial {
        fail_clean(&format!("copy failed: {error}"));
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
}
