//! Review-window tests against a private Xvfb server.
//!
//! These are ignored by default because they need `Xvfb` and `xdotool`.
//! CI runs them with `cargo test --locked -- --ignored`. Each test starts
//! its own server, so they never touch a running session.
//!
//! `NIRUSHOT_BIN` overrides the binary under test (used to measure the
//! previous release with the same harness), and `NIRUSHOT_REFERENCE_BIN`
//! names an older binary whose export must be pixel-identical.

use std::{
    env, fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

use image::{Rgba, RgbaImage};
use x11rb::{
    connection::Connection,
    protocol::xproto::{ConnectionExt as _, EventMask, GrabMode, GrabStatus},
};

struct Xvfb {
    child: Child,
    display: String,
}

/// SIGTERM, so the server removes its lock file and socket; SIGKILL as a
/// last resort.
fn terminate(child: &mut Child) {
    let _ = Command::new("kill")
        .arg(child.id().to_string())
        .stderr(Stdio::null())
        .status();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Ok(Some(_)) = child.try_wait() {
            return;
        }
        thread::sleep(Duration::from_millis(20));
    }
    let _ = child.kill();
    let _ = child.wait();
}

impl Drop for Xvfb {
    fn drop(&mut self) {
        terminate(&mut self.child);
    }
}

fn start_xvfb(width: u32, height: u32) -> Xvfb {
    for number in 140..240 {
        let lock = format!("/tmp/.X{number}-lock");
        let socket = format!("/tmp/.X11-unix/X{number}");
        if Path::new(&lock).exists() || Path::new(&socket).exists() {
            continue;
        }
        let display = format!(":{number}");
        let child = Command::new("Xvfb")
            .args([
                display.as_str(),
                "-screen",
                "0",
                &format!("{width}x{height}x24"),
                "-nolisten",
                "tcp",
                // Without it the server resets when the readiness probe
                // disconnects, and a client connecting then is refused.
                "-noreset",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("Xvfb is installed");
        let mut server = Xvfb { child, display };
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            if Path::new(&socket).exists() && x11rb::connect(Some(&server.display)).is_ok() {
                return server;
            }
            if let Ok(Some(_)) = server.child.try_wait() {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        let _ = server.child.kill();
    }
    panic!("could not start Xvfb");
}

/// A capture with fine detail, so resampling and annotation differences show.
fn fixture_image(width: u32, height: u32) -> RgbaImage {
    RgbaImage::from_fn(width, height, |x, y| {
        let v = ((x * 7 + y * 13) % 251) as u8;
        Rgba([v, ((x / 16 + y / 16) % 2 * 200) as u8, (y % 256) as u8, 255])
    })
}

struct Fixture {
    dir: tempfile::TempDir,
}

impl Fixture {
    fn new(width: u32, height: u32) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        fs::create_dir_all(&bin).unwrap();
        fs::create_dir_all(dir.path().join("home")).unwrap();
        fixture_image(width, height)
            .save(dir.path().join("capture.png"))
            .unwrap();
        let write = |name: &str, body: &str| {
            let path = bin.join(name);
            fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
            Command::new("chmod").arg("+x").arg(&path).status().unwrap();
        };
        write(
            "maim",
            "for last; do :; done\ncp \"$NIRUSHOT_FIXTURE/capture.png\" \"$last\"",
        );
        write(
            "xclip",
            "cat > \"$NIRUSHOT_FIXTURE/clip-$$.png\"\necho \"$*\" >> \"$NIRUSHOT_FIXTURE/xclip.log\"",
        );
        write("notify-send", "exit 0");
        write("fc-match", "exit 1");
        Self { dir }
    }

    fn path(&self, name: &str) -> PathBuf {
        self.dir.path().join(name)
    }

    fn clipboard_count(&self) -> usize {
        fs::read_to_string(self.path("xclip.log"))
            .map(|s| s.lines().count())
            .unwrap_or(0)
    }
}

fn binary() -> PathBuf {
    env::var_os("NIRUSHOT_BIN")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_BIN_EXE_nirushot")))
}

struct Review {
    child: Child,
    window: String,
    display: String,
}

impl Drop for Review {
    fn drop(&mut self) {
        if let Ok(Some(_)) = self.child.try_wait() {
            return;
        }
        // Escape discards and removes the private capture directory.
        let _ = Command::new("xdotool")
            .args(["key", "Escape"])
            .env("DISPLAY", &self.display)
            .status();
        terminate(&mut self.child);
    }
}

fn open_review(bin: &Path, x: &Xvfb, fixture: &Fixture, out: &Path) -> Review {
    let path = format!(
        "{}:{}",
        fixture.path("bin").display(),
        env::var("PATH").unwrap_or_default()
    );
    let child = Command::new(bin)
        .args(["--full", "--review", "--no-notify", "--save"])
        .arg(out)
        .env("DISPLAY", &x.display)
        .env("PATH", path)
        .env("HOME", fixture.path("home"))
        .env("XDG_CONFIG_HOME", fixture.path("home/.config"))
        .env("NIRUSHOT_FIXTURE", fixture.dir.path())
        .env("TMPDIR", fixture.dir.path())
        .stdout(Stdio::null())
        .spawn()
        .expect("start nirushot");
    let mut review = Review {
        child,
        window: String::new(),
        display: x.display.clone(),
    };
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let found = Command::new("xdotool")
            .args(["search", "--onlyvisible", "--class", "nirushot"])
            .env("DISPLAY", &x.display)
            .output()
            .expect("xdotool is installed");
        let text = String::from_utf8_lossy(&found.stdout);
        if let Some(window) = text.lines().next() {
            review.window = window.trim().to_string();
            // The first frame is drawn after the map; let it settle.
            review.settle();
            return review;
        }
        assert!(Instant::now() < deadline, "review window did not appear");
        thread::sleep(Duration::from_millis(50));
    }
}

impl Review {
    fn xdotool(&self, args: &[String]) {
        let status = Command::new("xdotool")
            .args(args)
            .env("DISPLAY", &self.display)
            .status()
            .unwrap();
        assert!(status.success(), "xdotool {args:?}");
    }

    fn key(&self, key: &str) {
        self.xdotool(&["key".into(), key.into()]);
    }

    fn cpu(&self) -> Duration {
        let stat = fs::read_to_string(format!("/proc/{}/stat", self.child.id())).unwrap();
        let fields: Vec<&str> = stat
            .rsplit_once(')')
            .unwrap()
            .1
            .split_whitespace()
            .collect();
        let ticks: u64 = fields[11].parse::<u64>().unwrap() + fields[12].parse::<u64>().unwrap();
        // Linux reports USER_HZ (100) ticks.
        Duration::from_millis(ticks * 10)
    }

    /// Wait until the review uses no more CPU, or give up after a while.
    fn settle(&self) {
        let deadline = Instant::now() + Duration::from_secs(30);
        let mut last = self.cpu();
        loop {
            thread::sleep(Duration::from_millis(600));
            let now = self.cpu();
            if now == last || Instant::now() > deadline {
                return;
            }
            last = now;
        }
    }

    /// Press at `from`, move in `steps` steps to `to`, and release.
    fn drag(&self, from: (i32, i32), to: (i32, i32), steps: i32, button: u8) {
        let mut args: Vec<String> = vec![
            "mousemove".into(),
            "--window".into(),
            self.window.clone(),
            from.0.to_string(),
            from.1.to_string(),
            "mousedown".into(),
            button.to_string(),
        ];
        for i in 1..=steps {
            let x = from.0 + (to.0 - from.0) * i / steps;
            let y = from.1 + (to.1 - from.1) * i / steps;
            args.extend([
                "sleep".into(),
                "0.016".into(),
                "mousemove".into(),
                "--window".into(),
                self.window.clone(),
                x.to_string(),
                y.to_string(),
            ]);
        }
        args.extend(["mouseup".into(), button.to_string()]);
        self.xdotool(&args);
    }

    fn wait_exit(&mut self) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while Instant::now() < deadline {
            if let Ok(Some(status)) = self.child.try_wait() {
                assert!(status.success(), "nirushot exited with {status}");
                return;
            }
            thread::sleep(Duration::from_millis(50));
        }
        panic!("review did not close");
    }
}

/// Runs the same rectangle drag and save, returning the CPU used by the drag
/// (press, 60 motions, release and the redraw that follows) and the export.
fn rectangle_drag(bin: &Path) -> (Duration, RgbaImage) {
    let x = start_xvfb(3840, 2160);
    let fixture = Fixture::new(3840, 2160);
    let out = fixture.path("out.png");
    let mut review = open_review(bin, &x, &fixture, &out);
    review.key("r");
    review.settle();
    let before = review.cpu();
    review.drag((400, 300), (1600, 1100), 60, 1);
    review.settle();
    let used = review.cpu() - before;
    review.key("s");
    review.wait_exit();
    (used, image::open(&out).unwrap().to_rgba8())
}

#[test]
#[ignore = "needs Xvfb and xdotool"]
fn h2_4k_rectangle_drag_stays_within_cpu_budget() {
    let (used, export) = rectangle_drag(&binary());
    eprintln!("4K 60-step rectangle drag: {} ms CPU", used.as_millis());
    assert!(
        used <= Duration::from_millis(300),
        "drag used {used:?} of CPU"
    );
    let original = fixture_image(3840, 2160);
    assert_eq!(export.dimensions(), original.dimensions());
    assert_ne!(export, original, "the rectangle is in the export");
    if let Some(reference) = env::var_os("NIRUSHOT_REFERENCE_BIN") {
        let (old_cpu, old) = rectangle_drag(Path::new(&reference));
        eprintln!("reference binary: {} ms CPU", old_cpu.as_millis());
        assert!(old == export, "export differs from the reference binary");
    }
}

#[test]
#[ignore = "needs Xvfb and xdotool"]
fn h1_open_review_does_not_block_a_locker_grab() {
    let x = start_xvfb(1280, 800);
    let fixture = Fixture::new(800, 500);
    let out = fixture.path("out.png");
    let review = open_review(&binary(), &x, &fixture, &out);
    let (c, screen) = x11rb::connect(Some(&x.display)).unwrap();
    let root = c.setup().roots[screen].root;
    let pointer = c
        .grab_pointer(
            false,
            root,
            EventMask::BUTTON_PRESS,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
            x11rb::NONE,
            x11rb::NONE,
            x11rb::CURRENT_TIME,
        )
        .unwrap()
        .reply()
        .unwrap()
        .status;
    let keyboard = c
        .grab_keyboard(
            false,
            root,
            x11rb::CURRENT_TIME,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
        )
        .unwrap()
        .reply()
        .unwrap()
        .status;
    assert_eq!(
        pointer,
        GrabStatus::SUCCESS,
        "GrabPointer with a review open"
    );
    assert_eq!(
        keyboard,
        GrabStatus::SUCCESS,
        "GrabKeyboard with a review open"
    );
    // Let the review have its keyboard back so Escape can close it.
    c.ungrab_keyboard(x11rb::CURRENT_TIME).unwrap();
    c.ungrab_pointer(x11rb::CURRENT_TIME).unwrap();
    c.flush().unwrap();
    drop(review);
}

/// Open a 1600x900 review on a 1920x1080 server, run `steps`, save with `s`
/// and return the export together with the original.
fn session(steps: impl FnOnce(&Review)) -> (RgbaImage, RgbaImage) {
    session_with(|r, _| steps(r), "s")
}

fn session_with(steps: impl FnOnce(&Review, &Fixture), save_key: &str) -> (RgbaImage, RgbaImage) {
    let x = start_xvfb(1920, 1080);
    let fixture = Fixture::new(1600, 900);
    let out = fixture.path("out.png");
    let mut review = open_review(&binary(), &x, &fixture, &out);
    steps(&review, &fixture);
    review.settle();
    review.key(save_key);
    review.wait_exit();
    (
        image::open(&out).unwrap().to_rgba8(),
        fixture_image(1600, 900),
    )
}

#[test]
#[ignore = "needs Xvfb and xdotool"]
fn h3_undo_and_redo_from_the_keyboard() {
    let (undone, original) = session(|r| {
        r.key("r");
        r.drag((300, 300), (700, 600), 10, 1);
        r.key("c");
        r.drag((200, 200), (900, 700), 10, 1);
        r.key("ctrl+z");
        r.key("ctrl+z");
    });
    assert!(undone == original, "two undos restore the capture");
    let (redone, original) = session(|r| {
        r.key("r");
        r.drag((300, 300), (700, 600), 10, 1);
        r.key("ctrl+z");
        r.key("ctrl+shift+z");
    });
    assert_eq!(redone.dimensions(), original.dimensions());
    assert!(redone != original, "redo brings the rectangle back");
}

#[test]
#[ignore = "needs Xvfb and xdotool"]
fn e1_text_is_typed_placed_and_cancelled() {
    let (typed, original) = session(|r| {
        r.key("t");
        r.xdotool(&[
            "mousemove".into(),
            "--window".into(),
            r.window.clone(),
            "400".into(),
            "400".into(),
            "click".into(),
            "1".into(),
        ]);
        r.xdotool(&["type".into(), "Hi @ 2".into()]);
        r.key("Return");
    });
    let changed: Vec<(u32, u32)> = typed
        .enumerate_pixels()
        .filter(|(x, y, p)| *p != original.get_pixel(*x, *y))
        .map(|(x, y, _)| (x, y))
        .collect();
    assert!(changed.len() > 50, "the text is in the export");
    let xs = changed.iter().map(|p| p.0);
    let ys = changed.iter().map(|p| p.1);
    let (x0, x1) = (xs.clone().min().unwrap(), xs.max().unwrap());
    let (y0, y1) = (ys.clone().min().unwrap(), ys.max().unwrap());
    assert!(x1 - x0 < 200 && y1 - y0 < 60, "one line of M text");
    let (cancelled, original) = session(|r| {
        r.key("t");
        r.xdotool(&[
            "mousemove".into(),
            "--window".into(),
            r.window.clone(),
            "400".into(),
            "400".into(),
            "click".into(),
            "1".into(),
        ]);
        r.xdotool(&["type".into(), "gone".into()]);
        r.key("Escape");
    });
    assert!(
        cancelled == original,
        "Esc cancels the text, not the review"
    );
}

#[test]
#[ignore = "needs Xvfb and xdotool"]
fn e3_standard_shortcuts() {
    // Ctrl+C copies the edited image (after the initial capture copy), and
    // Ctrl+S saves and closes like `s`.
    let (saved, original) = session_with(
        |r, fixture| {
            assert_eq!(fixture.clipboard_count(), 1, "the capture is copied first");
            r.key("r");
            r.drag((300, 300), (700, 600), 10, 1);
            r.key("ctrl+c");
            r.settle();
            assert_eq!(fixture.clipboard_count(), 2, "Ctrl+C copied the edit");
        },
        "ctrl+s",
    );
    assert!(saved != original, "Ctrl+S saved the edited image");

    // Delete removes the last annotation only.
    let (rect_only, _) = session(|r| {
        r.key("r");
        r.drag((300, 300), (700, 600), 10, 1);
    });
    let (deleted, _) = session(|r| {
        r.key("r");
        r.drag((300, 300), (700, 600), 10, 1);
        r.key("a");
        r.drag((900, 300), (1200, 600), 10, 1);
        r.key("Delete");
    });
    assert!(
        deleted == rect_only,
        "Delete removed the arrow and kept the rectangle"
    );

    // Space-drag pans instead of drawing with the active tool.
    let (panned, original) = session(|r| {
        r.key("r");
        r.xdotool(&["keydown".into(), "space".into()]);
        r.drag((300, 300), (700, 600), 10, 1);
        r.xdotool(&["keyup".into(), "space".into()]);
    });
    assert!(panned == original, "a space-drag drew nothing");
}

// ---------------------------------------------------------------------------
// `nirushot pick`

/// Paints known colours on the screen and keeps them up while alive.
struct Painting {
    c: x11rb::rust_connection::RustConnection,
    windows: Vec<u32>,
}

impl Painting {
    /// Windows as (x, y, w, h, colour).
    fn new(display: &str, parts: &[(i16, i16, u16, u16, u32)]) -> Self {
        use x11rb::{
            protocol::xproto::{CreateWindowAux, WindowClass},
            wrapper::ConnectionExt as _,
        };
        let (c, screen) =
            x11rb::connect(Some(display)).unwrap_or_else(|e| panic!("connect {display}: {e:?}"));
        let root = c.setup().roots[screen].root;
        let mut windows = Vec::new();
        for &(x, y, w, h, colour) in parts {
            let win = c.generate_id().unwrap();
            c.create_window(
                x11rb::COPY_DEPTH_FROM_PARENT,
                win,
                root,
                x,
                y,
                w,
                h,
                0,
                WindowClass::INPUT_OUTPUT,
                x11rb::COPY_FROM_PARENT,
                &CreateWindowAux::new()
                    .background_pixel(colour)
                    .override_redirect(1),
            )
            .unwrap();
            c.map_window(win).unwrap();
            windows.push(win);
        }
        c.sync().unwrap();
        thread::sleep(Duration::from_millis(200));
        Self { c, windows }
    }

    /// Take everything down, so only a frozen frame still shows it.
    fn clear(&self) {
        use x11rb::wrapper::ConnectionExt as _;
        for win in &self.windows {
            self.c.unmap_window(*win).unwrap();
        }
        self.c.sync().unwrap();
    }
}

struct Pick {
    child: Child,
    display: String,
    fixture: Fixture,
}

fn xdo(display: &str, args: &[&str]) {
    let status = Command::new("xdotool")
        .args(args)
        .env("DISPLAY", display)
        .status()
        .unwrap();
    assert!(status.success(), "xdotool {args:?}");
}

fn pick_fixture() -> Fixture {
    let fixture = Fixture::new(4, 4);
    let bin = fixture.path("bin");
    let write = |name: &str, body: &str| {
        let path = bin.join(name);
        fs::write(&path, format!("#!/bin/sh\n{body}\n")).unwrap();
        Command::new("chmod").arg("+x").arg(&path).status().unwrap();
    };
    write(
        "xclip",
        "echo \"$*\" > \"$NIRUSHOT_FIXTURE/xclip.args\"\ncat > \"$NIRUSHOT_FIXTURE/clip.txt\"",
    );
    write(
        "notify-send",
        "printf '%s\\n' \"$@\" > \"$NIRUSHOT_FIXTURE/notify.args\"",
    );
    fixture
}

/// Start `nirushot pick` and wait for its overlay.
fn start_pick(x: &Xvfb, fixture: Fixture, extra: &[&str]) -> Pick {
    let path = format!(
        "{}:{}",
        fixture.path("bin").display(),
        env::var("PATH").unwrap_or_default()
    );
    let child = Command::new(binary())
        .arg("pick")
        .args(extra)
        .env("DISPLAY", &x.display)
        .env("PATH", path)
        .env("HOME", fixture.path("home"))
        .env("XDG_CONFIG_HOME", fixture.path("home/.config"))
        .env("XDG_CACHE_HOME", fixture.path("home/.cache"))
        .env("NIRUSHOT_FIXTURE", fixture.dir.path())
        .env("NIRUSHOT_TIMING", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start nirushot pick");
    let pick = Pick {
        child,
        display: x.display.clone(),
        fixture,
    };
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        let found = Command::new("xdotool")
            .args(["search", "--onlyvisible", "--classname", "^nirushot-pick$"])
            .env("DISPLAY", &x.display)
            .output()
            .unwrap();
        if !found.stdout.is_empty() {
            thread::sleep(Duration::from_millis(150));
            return pick;
        }
        assert!(Instant::now() < deadline, "the picker did not appear");
        thread::sleep(Duration::from_millis(20));
    }
}

impl Pick {
    fn key(&self, key: &str) {
        xdo(&self.display, &["key", "--delay", "40", key]);
        thread::sleep(Duration::from_millis(60));
    }

    /// Exit code, stdout and stderr.
    fn finish(mut self) -> (i32, String, String, Fixture) {
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Ok(Some(_)) = self.child.try_wait() {
                break;
            }
            assert!(Instant::now() < deadline, "the picker did not exit");
            thread::sleep(Duration::from_millis(20));
        }
        let out = self.child.wait_with_output().unwrap();
        let stderr = String::from_utf8_lossy(&out.stderr).to_string();
        eprint!("{stderr}");
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).trim().to_string(),
            stderr,
            self.fixture,
        )
    }
}

fn history(fixture: &Fixture) -> String {
    fs::read_to_string(fixture.path("home/.local/state/nirushot/colors.json")).unwrap_or_default()
}

#[test]
#[ignore = "needs Xvfb and xdotool"]
fn pick_freezes_moves_by_key_cycles_formats_and_records() {
    let x = start_xvfb(1280, 800);
    let paint = Painting::new(
        &x.display,
        &[
            (0, 0, 640, 800, 0x1a2b3c),
            (101, 100, 1, 1, 0xff0080),
            (110, 100, 1, 1, 0x00ff00),
        ],
    );
    xdo(&x.display, &["mousemove", "100", "100"]);
    let pick = start_pick(&x, pick_fixture(), &[]);
    // The live screen changes; the picker keeps showing the frozen frame.
    paint.clear();
    pick.key("Right");
    pick.key("Tab");
    pick.key("Tab");
    pick.key("Return");
    let (code, out, stderr, fixture) = pick.finish();
    assert_eq!(code, 0);
    assert_eq!(out, "hsl(330, 100%, 50%)");
    assert!(stderr.contains("window mapped"), "timing line: {stderr}");
    assert_eq!(
        fs::read_to_string(fixture.path("clip.txt")).unwrap(),
        "hsl(330, 100%, 50%)"
    );
    assert_eq!(
        fs::read_to_string(fixture.path("xclip.args"))
            .unwrap()
            .trim(),
        "-selection clipboard -i"
    );
    let notified = fs::read_to_string(fixture.path("notify.args")).unwrap();
    assert!(notified.contains("Colour #ff0080"), "{notified}");
    assert!(notified.contains("string:image-path:"), "{notified}");
    assert!(notified.contains("swatch-ff0080.png"), "{notified}");
    let first = history(&fixture);
    assert!(
        first.starts_with("[\n  {\"hex\":\"#ff0080\",\"time\":"),
        "{first}"
    );

    // Shift+Right moves ten pixels, Left one back, a click picks; the
    // history keeps the newest first. Same home, so the history carries on.
    let paint = Painting::new(
        &x.display,
        &[(100, 100, 1, 1, 0xffffff), (109, 100, 1, 1, 0x00ff00)],
    );
    xdo(&x.display, &["mousemove", "100", "100"]);
    let pick = start_pick(&x, fixture, &[]);
    pick.key("shift+Right");
    pick.key("Left");
    // A click picks where the pointer is: where the keys put it.
    xdo(&pick.display, &["click", "1"]);
    let (code, out, _, fixture) = pick.finish();
    drop(paint);
    assert_eq!(code, 0);
    assert_eq!(out, "#00ff00");
    let text = history(&fixture);
    let hexes: Vec<&str> = text
        .match_indices("\"hex\":\"")
        .map(|(i, _)| &text[i + 7..i + 14])
        .collect();
    assert_eq!(hexes, ["#00ff00", "#ff0080"], "{text}");
}

#[test]
#[ignore = "needs Xvfb and xdotool"]
fn pick_escape_cancels_without_side_effects() {
    let x = start_xvfb(800, 600);
    let pick = start_pick(&x, pick_fixture(), &[]);
    pick.key("Tab");
    pick.key("Escape");
    let (code, out, _, fixture) = pick.finish();
    assert_eq!(code, 1);
    assert!(out.is_empty());
    assert!(!fixture.path("clip.txt").exists());
    assert!(!fixture.path("notify.args").exists());
    assert!(history(&fixture).is_empty());
    // The grab is gone: another client can take it right away.
    let (c, screen) = x11rb::connect(Some(&x.display)).unwrap();
    let root = c.setup().roots[screen].root;
    let status = c
        .grab_keyboard(
            false,
            root,
            x11rb::CURRENT_TIME,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
        )
        .unwrap()
        .reply()
        .unwrap()
        .status;
    assert_eq!(status, GrabStatus::SUCCESS);
}

#[test]
#[ignore = "needs Xvfb and xdotool"]
fn pick_never_opens_over_a_locker() {
    let x = start_xvfb(800, 600);
    let (c, screen) = x11rb::connect(Some(&x.display)).unwrap();
    let root = c.setup().roots[screen].root;
    let status = c
        .grab_keyboard(
            false,
            root,
            x11rb::CURRENT_TIME,
            GrabMode::ASYNC,
            GrabMode::ASYNC,
        )
        .unwrap()
        .reply()
        .unwrap()
        .status;
    assert_eq!(status, GrabStatus::SUCCESS);
    let fixture = pick_fixture();
    let started = Instant::now();
    let out = Command::new(binary())
        .arg("pick")
        .env("DISPLAY", &x.display)
        .env("HOME", fixture.path("home"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("cannot grab"));
    assert!(started.elapsed() < Duration::from_secs(5));
    let found = Command::new("xdotool")
        .args(["search", "--classname", "^nirushot-pick$"])
        .env("DISPLAY", &x.display)
        .output()
        .unwrap();
    assert!(found.stdout.is_empty(), "no overlay was created");
}
