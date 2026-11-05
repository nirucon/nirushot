//! Which monitor the review opens on, and where.
//!
//! The root window spans every monitor, so centring on it put the dialog on
//! the seam between two screens. The review goes to the monitor holding the
//! captured region (or the pointer) and is sized from that monitor.

use x11rb::{
    protocol::{randr::ConnectionExt as _, xproto::*},
    rust_connection::RustConnection,
};

use crate::view::{Area, TOOLBAR};

/// Active monitors from RandR 1.5, or the whole root when there are none.
pub fn monitors(c: &RustConnection, root: Window, root_size: (u32, u32)) -> Vec<Area> {
    let found: Vec<Area> = c
        .randr_get_monitors(root, true)
        .ok()
        .and_then(|r| r.reply().ok())
        .map(|r| {
            r.monitors
                .iter()
                .filter(|m| m.width > 0 && m.height > 0)
                .map(|m| (m.x as i32, m.y as i32, m.width as i32, m.height as i32))
                .collect()
        })
        .unwrap_or_default();
    if found.is_empty() {
        vec![(0, 0, root_size.0 as i32, root_size.1 as i32)]
    } else {
        found
    }
}

pub fn pointer(c: &RustConnection, root: Window) -> Option<(i32, i32)> {
    let reply = c.query_pointer(root).ok()?.reply().ok()?;
    Some((reply.root_x as i32, reply.root_y as i32))
}

/// The monitor containing `point`, else the nearest one.
pub fn choose(monitors: &[Area], point: (i32, i32)) -> Option<Area> {
    let contains =
        |m: &&Area| point.0 >= m.0 && point.0 < m.0 + m.2 && point.1 >= m.1 && point.1 < m.1 + m.3;
    if let Some(m) = monitors.iter().find(contains) {
        return Some(*m);
    }
    let distance = |m: &Area| {
        let dx = (m.0 - point.0).max(point.0 - (m.0 + m.2 - 1)).max(0) as i64;
        let dy = (m.1 - point.1).max(point.1 - (m.1 + m.3 - 1)).max(0) as i64;
        dx * dx + dy * dy
    };
    monitors.iter().min_by_key(|m| distance(m)).copied()
}

/// The centre of a `WxH+X+Y` geometry, as printed by slop.
pub fn geometry_centre(geometry: &str) -> Option<(i32, i32)> {
    let (size, rest) = geometry.trim().split_once('+')?;
    let (w, h) = size.split_once('x')?;
    let (x, y) = rest.split_once('+')?;
    let (w, h): (i32, i32) = (w.parse().ok()?, h.parse().ok()?);
    let (x, y): (i32, i32) = (x.parse().ok()?, y.parse().ok()?);
    Some((x + w / 2, y + h / 2))
}

/// Narrowest review window: wide enough for the toolbar's controls
/// (0.4 used 760 before the text tool, palette and Undo were added).
pub const MIN_WIDTH: u32 = 840;

/// Review window geometry for an image on `monitor`: the 0.4 sizing rules
/// (90 % wide, 84 % high, at least MIN_WIDTH wide, never upscaled), applied to the
/// monitor instead of the whole root, and centred on it.
pub fn placement(monitor: Area, img: (u32, u32)) -> Area {
    let (mx, my, mw, mh) = monitor;
    let (mw, mh) = (mw.max(1) as u32, mh.max(1) as u32);
    let max_w = (mw as f32 * 0.90).max(320.0);
    let max_h = (mh as f32 * 0.84 - TOOLBAR as f32).max(200.0);
    let initial = (max_w / img.0 as f32).min(max_h / img.1 as f32).min(1.0);
    let w = ((img.0 as f32 * initial) as u32)
        .max(MIN_WIDTH)
        .min(mw.saturating_sub(24).max(320));
    let h = TOOLBAR + (img.1 as f32 * initial).max(1.0) as u32;
    let x = mx + ((mw as i32 - w as i32) / 2).max(0);
    let y = my + ((mh as i32 - h as i32) / 2).max(0);
    (x, y, w as i32, h as i32)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// othala: two 2560x1440 monitors stacked vertically.
    const STACKED: [Area; 2] = [(0, 1440, 2560, 1440), (0, 0, 2560, 1440)];
    const SIDE_BY_SIDE: [Area; 2] = [(0, 0, 2560, 1440), (2560, 0, 2560, 1440)];

    fn inside(window: Area, monitor: Area) -> bool {
        window.0 >= monitor.0
            && window.1 >= monitor.1
            && window.0 + window.2 <= monitor.0 + monitor.2
            && window.1 + window.3 <= monitor.1 + monitor.3
    }

    #[test]
    fn picks_the_monitor_under_the_point() {
        assert_eq!(choose(&STACKED, (100, 2000)), Some(STACKED[0]));
        assert_eq!(choose(&STACKED, (100, 100)), Some(STACKED[1]));
        assert_eq!(choose(&SIDE_BY_SIDE, (2560, 10)), Some(SIDE_BY_SIDE[1]));
        assert_eq!(choose(&SIDE_BY_SIDE, (2559, 10)), Some(SIDE_BY_SIDE[0]));
    }

    #[test]
    fn a_point_outside_every_monitor_takes_the_nearest() {
        assert_eq!(choose(&SIDE_BY_SIDE, (6000, 100)), Some(SIDE_BY_SIDE[1]));
        assert_eq!(choose(&STACKED, (-50, 1500)), Some(STACKED[0]));
        assert_eq!(choose(&[], (0, 0)), None);
    }

    #[test]
    fn a_full_capture_of_both_monitors_fits_one_monitor() {
        for (monitors, root) in [(STACKED, (2560, 2880)), (SIDE_BY_SIDE, (5120, 1440))] {
            for monitor in monitors {
                let window = placement(monitor, root);
                assert!(inside(window, monitor), "{window:?} on {monitor:?}");
            }
        }
    }

    #[test]
    fn the_window_is_centred_on_its_monitor() {
        let monitor = SIDE_BY_SIDE[1];
        let (x, y, w, h) = placement(monitor, (1000, 600));
        assert_eq!((w, h), (1000, 600 + TOOLBAR as i32));
        assert_eq!(x + w / 2, 2560 + 1280);
        assert_eq!(y + h / 2, 720 - (TOOLBAR as i32 % 2) / 2);
    }

    #[test]
    fn a_single_root_monitor_keeps_the_old_geometry() {
        // 0.4.10 sized and centred the window on the root.
        let (x, y, w, h) = placement((0, 0, 3840, 2160), (3840, 2160));
        let initial = (3840.0 * 0.9 / 3840.0f32).min((2160.0 * 0.84 - 52.0) / 2160.0);
        assert_eq!(w, (3840.0 * initial) as i32);
        assert_eq!(h, 52 + (2160.0 * initial) as i32);
        assert_eq!((x, y), ((3840 - w) / 2, (2160 - h) / 2));
    }

    #[test]
    fn slop_geometry_centre() {
        assert_eq!(geometry_centre("100x50+20+30"), Some((70, 55)));
        assert_eq!(geometry_centre("2560x1440+0+1440\n"), Some((1280, 2160)));
        assert_eq!(geometry_centre("garbage"), None);
    }
}
