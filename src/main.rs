use std::{env, path::PathBuf, process::Command, thread, time::Duration};

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

fn main() {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.iter().any(|a| a == "--version") {
        println!("NIRUSHOT {}", env!("CARGO_PKG_VERSION"));
        return;
    }
    if args.iter().any(|a| a == "--help") {
        println!("Usage: nirushot [--region|--window|--full] [--delay SECONDS] [--copy|--save PATH] [--no-notify]");
        return;
    }
    let mut mode = "region";
    let mut delay = 0u64;
    let mut copy = false;
    let mut save: Option<PathBuf> = None;
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
            "--no-notify" => notify_enabled = false,
            other => {
                eprintln!("nirushot: unknown option {other}");
                std::process::exit(2);
            }
        }
        i += 1;
    }
    let copy = copy || save.is_none();
    let had_save = save.is_some();
    let target = save.unwrap_or_else(|| {
        PathBuf::from("/tmp").join(format!("nirushot-{}.png", std::process::id()))
    });
    if let Some(parent) = target.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if delay > 0 {
        thread::sleep(Duration::from_secs(delay));
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
    if copy {
        if let Err(e) = run(
            "xclip",
            &[
                "-selection".into(),
                "clipboard".into(),
                "-t".into(),
                "image/png".into(),
                "-i".into(),
                target.to_string_lossy().to_string(),
            ],
        ) {
            eprintln!("nirushot: copy failed: {e}");
            std::process::exit(1);
        }
    }
    let path = target.to_string_lossy().to_string();
    println!("{path}");
    if notify_enabled {
        notify(&path);
    }
    if !had_save {
        let _ = std::fs::remove_file(&target);
    }
}
