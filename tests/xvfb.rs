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

impl Drop for Xvfb {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
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
        let _ = self.child.kill();
        let _ = self.child.wait();
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
        .stdout(Stdio::null())
        .spawn()
        .expect("start nirushot");
    let deadline = Instant::now() + Duration::from_secs(20);
    loop {
        let found = Command::new("xdotool")
            .args(["search", "--onlyvisible", "--class", "nirushot"])
            .env("DISPLAY", &x.display)
            .output()
            .expect("xdotool is installed");
        let text = String::from_utf8_lossy(&found.stdout);
        if let Some(window) = text.lines().next() {
            let review = Review {
                child,
                window: window.trim().to_string(),
                display: x.display.clone(),
            };
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
    drop(review);
}
