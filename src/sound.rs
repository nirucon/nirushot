//! The NIRUX shutter sound, through the running NIRUNOTIFY.
//!
//! NIRUNOTIFY owns the system sounds and their settings
//! (`~/.config/nirux/sounds.toml`, event `screenshot`). Asking it is one line
//! on its control socket: no process is started and nothing new is needed at
//! run time. Without NIRUNOTIFY the capture is simply silent.

use std::{
    io::{Read, Write},
    os::unix::{fs::MetadataExt, net::UnixStream},
    path::PathBuf,
    time::Duration,
};

fn socket_path() -> Option<PathBuf> {
    // The owner of /proc/self is this process's user, so no libc is needed.
    let uid = std::fs::metadata("/proc/self").ok()?.uid();
    let base = std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/tmp"));
    Some(base.join(format!("nirunotify-{uid}.sock")))
}

/// Ask NIRUNOTIFY to play EVENT. Bounded to a fraction of a second and
/// silent on any error.
pub fn play(event: &str) {
    let Some(path) = socket_path() else { return };
    let Ok(mut stream) = UnixStream::connect(path) else {
        return;
    };
    let limit = Some(Duration::from_millis(300));
    let _ = stream.set_write_timeout(limit);
    let _ = stream.set_read_timeout(limit);
    if stream
        .write_all(format!("sound {event}\n").as_bytes())
        .is_ok()
    {
        let mut reply = [0u8; 64];
        let _ = stream.read(&mut reply);
    }
}
