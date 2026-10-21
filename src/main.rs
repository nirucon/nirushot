use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::Duration,
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

fn notify(path: &str) {
    let _ = Command::new("notify-send")
        .args(["-a", "NIRUSHOT", "-t", "6000", "Screenshot", path])
        .status();
}

use image::{Rgba, RgbaImage};
use x11rb::{
    connection::Connection,
    protocol::{xproto::*, Event},
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

const TITLE: &str =
    "NIRUSHOT — c crop · r rectangle · a arrow · m mask · s save · y copy · Esc close";

fn key_name(c: &RustConnection, code: u8) -> u32 {
    c.get_keyboard_mapping(code, 1)
        .ok()
        .and_then(|r| r.reply().ok())
        .and_then(|m| m.keysyms.first().copied())
        .unwrap_or(0)
}

fn set_title(c: &RustConnection, win: Window, text: &str) {
    let _ = c.change_property8(
        PropMode::REPLACE,
        win,
        AtomEnum::WM_NAME,
        AtomEnum::STRING,
        text.as_bytes(),
    );
}

fn to_x_bytes(c: &RustConnection, img: &RgbaImage) -> Vec<u8> {
    let mut data = Vec::with_capacity(img.width() as usize * img.height() as usize * 4);
    let bgr = c.setup().image_byte_order == ImageOrder::LSB_FIRST;
    for p in img.pixels() {
        if bgr {
            data.extend([p[2], p[1], p[0], 255]);
        } else {
            data.extend([p[0], p[1], p[2], 255]);
        }
    }
    data
}

fn put_image(c: &RustConnection, win: Window, gc: Gcontext, img: &RgbaImage) -> Result<(), String> {
    let data = to_x_bytes(c, img);
    c.put_image(
        ImageFormat::Z_PIXMAP,
        win,
        gc,
        img.width() as u16,
        img.height() as u16,
        0,
        0,
        0,
        c.setup().roots[0].root_depth,
        &data,
    )
    .map_err(|e| format!("put_image: {e}"))?;
    c.flush().map_err(|e| e.to_string())
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

fn scale_to(img: &RgbaImage, w: u32, h: u32) -> RgbaImage {
    image::imageops::resize(img, w, h, image::imageops::FilterType::Triangle)
}

fn review(mut image: RgbaImage, temp: &Path, notify_enabled: bool) -> Result<(), String> {
    let (c, screen) = x11rb::connect(None).map_err(|e| e.to_string())?;
    let root = c.setup().roots[screen].root;
    let sw = c.setup().roots[screen].width_in_pixels as u32;
    let sh = c.setup().roots[screen].height_in_pixels as u32;
    let max_w = (sw as f32 * 0.9).max(320.0) as u32;
    let max_h = (sh as f32 * 0.85).max(240.0) as u32;
    let scale = (max_w as f32 / image.width() as f32)
        .min(max_h as f32 / image.height() as f32)
        .min(1.0);
    let win_w = (image.width() as f32 * scale).max(1.0) as u16;
    let win_h = (image.height() as f32 * scale).max(1.0) as u16;
    let win = c.generate_id().map_err(|e| e.to_string())?;
    let gc = c.generate_id().map_err(|e| e.to_string())?;
    let x = ((sw as i16 - win_w as i16) / 2).max(0);
    let y = ((sh as i16 - win_h as i16) / 2).max(0);
    c.create_window(
        0,
        win,
        root,
        x,
        y,
        win_w,
        win_h,
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
                    | EventMask::STRUCTURE_NOTIFY,
            ),
    )
    .map_err(|e| e.to_string())?;
    c.create_gc(gc, win, &CreateGCAux::new())
        .map_err(|e| e.to_string())?;
    set_title(&c, win, TITLE);
    c.map_window(win).map_err(|e| e.to_string())?;
    c.set_input_focus(InputFocus::POINTER_ROOT, win, x11rb::CURRENT_TIME)
        .ok();
    c.flush().map_err(|e| e.to_string())?;
    let mut mode = b'0';
    let mut start: Option<(i32, i32)> = None;
    let mut current: Option<(i32, i32)> = None;
    loop {
        let mut base = scale_to(&image, win_w as u32, win_h as u32);
        if let (Some((x0, y0)), Some((x1, y1))) = (start, current) {
            let ix0 = (x0 as f32 / scale) as i32;
            let iy0 = (y0 as f32 / scale) as i32;
            let ix1 = (x1 as f32 / scale) as i32;
            let iy1 = (y1 as f32 / scale) as i32;
            let sx = (x0 as f32 * scale / scale) as i32;
            let sy0 = (y0 as f32 * scale / scale) as i32;
            let sx1 = (x1 as f32 * scale / scale) as i32;
            let sy1 = (y1 as f32 * scale / scale) as i32;
            match mode {
                b'r' => draw_rect(
                    &mut base,
                    sx.min(sx1),
                    sy0.min(sy1),
                    (sx1 - sx).abs(),
                    (sy1 - sy0).abs(),
                    Rgba([230, 90, 90, 255]),
                ),
                b'm' => {
                    for yy in sy0.min(sy1)..sy0.max(sy1) {
                        for xx in sx.min(sx1)..sx.max(sx1) {
                            if xx >= 0
                                && yy >= 0
                                && (xx as u32) < win_w as u32
                                && (yy as u32) < win_h as u32
                            {
                                base.put_pixel(xx as u32, yy as u32, Rgba([0, 0, 0, 255]));
                            }
                        }
                    }
                }
                b'a' => line(&mut base, sx, sy0, sx1, sy1, Rgba([230, 90, 90, 255])),
                _ => draw_rect(
                    &mut base,
                    sx.min(sx1),
                    sy0.min(sy1),
                    (sx1 - sx).abs(),
                    (sy1 - sy0).abs(),
                    Rgba([220, 200, 140, 255]),
                ),
            }
            let _ = (ix0, iy0, ix1, iy1);
        }
        put_image(&c, win, gc, &base)?;
        let event = c.wait_for_event().map_err(|e| e.to_string())?;
        match event {
            Event::Expose(e) if e.window == win => {
                put_image(&c, win, gc, &base)?;
            }
            Event::ButtonPress(e) if e.event == win && e.detail == 1 => {
                start = Some((e.event_x as i32, e.event_y as i32));
                current = start;
            }
            Event::MotionNotify(e) if start.is_some() => {
                current = Some((e.event_x as i32, e.event_y as i32));
            }
            Event::ButtonRelease(e) if e.event == win && e.detail == 1 => {
                if let (Some((x0, y0)), Some((x1, y1))) = (start.take(), current) {
                    let x = ((x0.min(x1) as f32) / scale).max(0.0) as u32;
                    let y = ((y0.min(y1) as f32) / scale).max(0.0) as u32;
                    let w = ((x0 - x1).abs() as f32 / scale) as u32;
                    let h = ((y0 - y1).abs() as f32 / scale) as u32;
                    if w > 2 && h > 2 {
                        let x = x.min(image.width().saturating_sub(1));
                        let y = y.min(image.height().saturating_sub(1));
                        let w = w.min(image.width() - x);
                        let h = h.min(image.height() - y);
                        match mode {
                            b'c' => {
                                image = image::imageops::crop_imm(&image, x, y, w, h).to_image()
                            }
                            b'r' => draw_rect(
                                &mut image,
                                x as i32,
                                y as i32,
                                w as i32,
                                h as i32,
                                Rgba([220, 70, 70, 255]),
                            ),
                            b'm' => {
                                for yy in y..y + h {
                                    for xx in x..x + w {
                                        image.put_pixel(xx, yy, Rgba([0, 0, 0, 255]));
                                    }
                                }
                            }
                            b'a' => {
                                // The drag direction is already in image space.
                                line(
                                    &mut image,
                                    (x0 as f32 / scale) as i32,
                                    (y0 as f32 / scale) as i32,
                                    (x1 as f32 / scale) as i32,
                                    (y1 as f32 / scale) as i32,
                                    Rgba([220, 70, 70, 255]),
                                );
                            }
                            _ => {}
                        }
                        set_title(&c, win, TITLE);
                    }
                }
            }
            Event::KeyPress(e) if e.event == win => {
                let key = key_name(&c, e.detail);
                match key {
                    0xff1b => {
                        let _ = std::fs::remove_file(temp);
                        let _ = c.destroy_window(win);
                        let _ = c.flush();
                        return Ok(());
                    }
                    0x63 => {
                        mode = b'c';
                        set_title(&c, win, "NIRUSHOT — crop: drag, then release · Esc close");
                    }
                    0x72 => {
                        mode = b'r';
                        set_title(
                            &c,
                            win,
                            "NIRUSHOT — rectangle: drag, then release · Esc close",
                        );
                    }
                    0x61 => {
                        mode = b'a';
                        set_title(&c, win, "NIRUSHOT — arrow: drag, then release · Esc close");
                    }
                    0x6d => {
                        mode = b'm';
                        set_title(&c, win, "NIRUSHOT — mask: drag, then release · Esc close");
                    }
                    0x30 => {
                        mode = b'0';
                        set_title(&c, win, TITLE);
                    }
                    0x73 | 0xff0d => {
                        let path = default_path();
                        if let Some(parent) = path.parent() {
                            let _ = std::fs::create_dir_all(parent);
                        }
                        image
                            .save(&path)
                            .map_err(|e| format!("save {}: {e}", path.display()))?;
                        let _ = copy_to_clipboard(&path);
                        println!("{}", path.display());
                        if notify_enabled {
                            notify(&path.to_string_lossy());
                        }
                        let _ = std::fs::remove_file(temp);
                        let _ = c.destroy_window(win);
                        let _ = c.flush();
                        return Ok(());
                    }
                    0x79 => {
                        let path = temp.with_extension("review.png");
                        image
                            .save(&path)
                            .map_err(|e| format!("save {}: {e}", path.display()))?;
                        copy_to_clipboard(&path)?;
                        println!("{}", path.display());
                        if notify_enabled {
                            notify("copied to clipboard");
                        }
                    }
                    _ => {}
                }
            }
            Event::Error(e) => eprintln!("nirushot: X11 error: {e:?}"),
            _ => {}
        }
    }
}

fn default_path() -> PathBuf {
    let home = std::env::var_os("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("."));
    home.join("Pictures").join(format!(
        "Screenshot-{}.png",
        chrono::Local::now().format("%Y-%m-%d_%H-%M-%S")
    ))
}

fn copy_to_clipboard(path: &Path) -> Result<(), String> {
    run(
        "xclip",
        &[
            "-selection".into(),
            "clipboard".into(),
            "-t".into(),
            "image/png".into(),
            "-i".into(),
            path.to_string_lossy().to_string(),
        ],
    )
    .map(|_| ())
}

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version") {
        println!("NIRUSHOT {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args.iter().any(|a| a == "--help") {
        println!("Usage: nirushot [--region|--window|--full] [--delay SECONDS] [--copy|--save PATH] [--review] [--no-notify]");
        return;
    }
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
                delay = args
                    .get(i)
                    .and_then(|v| v.parse().ok())
                    .ok_or("--delay needs seconds")
                    .unwrap_or(0);
            }
            "--copy" => copy = true,
            "--save" => {
                i += 1;
                save = Some(PathBuf::from(
                    args.get(i)
                        .ok_or("--save needs a path")
                        .unwrap_or(&String::new()),
                ));
            }
            "--review" => review_flag = true,
            "--no-notify" => notify_enabled = false,
            other => {
                eprintln!("nirushot: unknown option {other}");
                std::process::exit(2);
            }
        }
        i += 1;
    }
    if delay > 0 {
        thread::sleep(Duration::from_secs(delay));
    }
    let temp = PathBuf::from("/tmp").join(format!("nirushot-{}.png", std::process::id()));
    let target = save.clone().unwrap_or_else(|| temp.clone());
    if let Some(parent) = target.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let mut maim = vec!["--quiet".to_string()];
    match mode {
        "region" => {
            let geometry = match run(
                "slop",
                &[
                    "-f".into(),
                    "%wx%h+%x+%y".into(),
                    "-b".into(),
                    "1".into(),
                    "-c".into(),
                    "0.5,0.5,0.5,0.5".into(),
                ],
            ) {
                Ok(g) if !g.is_empty() => g,
                Ok(_) => {
                    println!("cancelled");
                    return;
                }
                Err(e) => {
                    eprintln!("nirushot: selection failed: {e}");
                    std::process::exit(1);
                }
            };
            maim.extend(["-g".into(), geometry]);
        }
        "window" => {
            let id = match active_window() {
                Ok(id) => id,
                Err(e) => {
                    eprintln!("nirushot: {e}");
                    std::process::exit(1);
                }
            };
            maim.extend(["-i".into(), id]);
        }
        _ => {}
    }
    maim.push(target.to_string_lossy().to_string());
    if let Err(e) = run("maim", &maim) {
        eprintln!("nirushot: capture failed: {e}");
        std::process::exit(1);
    }
    if review_flag || (save.is_none() && !copy) {
        let image = match image::open(&target) {
            Ok(image) => image.to_rgba8(),
            Err(e) => {
                eprintln!("nirushot: cannot read capture: {e}");
                let _ = std::fs::remove_file(&target);
                std::process::exit(1);
            }
        };
        if let Err(e) = review(image, &target, notify_enabled) {
            eprintln!("nirushot: {e}");
            std::process::exit(1);
        }
        return;
    }
    if copy {
        if let Err(e) = copy_to_clipboard(&target) {
            eprintln!("nirushot: copy failed: {e}");
            std::process::exit(1);
        }
    }
    let path = target.to_string_lossy().to_string();
    println!("{path}");
    if notify_enabled {
        notify(&path);
    }
    if save.is_none() {
        let _ = std::fs::remove_file(&target);
    }
}
