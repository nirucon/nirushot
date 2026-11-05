//! Drawing the review window cheaply.
//!
//! The finished frame (scaled image and toolbar) lives in a server-side
//! pixmap. Exposes and drag previews only copy from it. The scaled image is
//! cached; an annotation re-scales only the pixels it changed, and a new zoom
//! or window size is scaled with a fast filter first and refined with
//! Lanczos3 once things are quiet.

use std::{
    sync::{mpsc, Arc},
    thread,
    time::Duration,
};

use image::RgbaImage;
use x11rb::{
    connection::Connection,
    protocol::{xproto::*, Event},
    rust_connection::RustConnection,
};

/// Height of the toolbar at the top of the review window.
pub const TOOLBAR: u32 = 52;

/// A fast-filtered view is replaced with a Lanczos3 one after this much quiet.
pub const REFINE_DELAY: Duration = Duration::from_millis(250);

/// Scaled views up to this many pixels are cached whole, so panning is free.
/// Larger zoomed views only keep the visible part.
const FULL_VIEW_PIXELS: u64 = 16 * 1024 * 1024;

/// A rectangle (x, y, width, height).
pub type Area = (i32, i32, i32, i32);

/// Where the image sits in the window.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub scale: f32,
    pub disp_w: u32,
    pub disp_h: u32,
    pub x: f32,
    pub y: f32,
}

pub fn view_layout(img: (u32, u32), win: (u32, u32), zoom: f32, pan: (f32, f32)) -> Layout {
    let avail_w = win.0 as f32;
    let avail_h = win.1.saturating_sub(TOOLBAR) as f32;
    let fit = (avail_w / img.0 as f32)
        .min(avail_h / img.1 as f32)
        .min(2.0);
    let scale = (fit * zoom).clamp(0.03, 8.0);
    let disp_w = (img.0 as f32 * scale).max(1.0) as u32;
    let disp_h = (img.1 as f32 * scale).max(1.0) as u32;
    Layout {
        scale,
        disp_w,
        disp_h,
        x: ((win.0 as f32 - disp_w as f32) / 2.0 + pan.0).round(),
        y: TOOLBAR as f32 + ((avail_h - disp_h as f32) / 2.0 + pan.1).round(),
    }
}

impl Layout {
    /// Window coordinates to image coordinates, clamped to the image.
    pub fn image_point(self, img: (u32, u32), x: i32, y: i32) -> (i32, i32) {
        let ix = ((x as f32 - self.x) / self.scale).round() as i32;
        let iy = ((y as f32 - self.y) / self.scale).round() as i32;
        (ix.clamp(0, img.0 as i32), iy.clamp(0, img.1 as i32))
    }

    /// Image coordinates to window coordinates.
    pub fn window_point(self, x: f32, y: f32) -> (f32, f32) {
        (self.x + x * self.scale, self.y + y * self.scale)
    }

    /// Window position of display pixel (0, 0).
    pub fn origin(self) -> (i32, i32) {
        (self.x as i32, self.y as i32)
    }
}

/// The part of the scaled image (in display pixels) worth keeping.
pub fn view_rect(l: &Layout, win: (u32, u32)) -> Option<Area> {
    if l.disp_w as u64 * l.disp_h as u64 <= FULL_VIEW_PIXELS {
        return Some((0, 0, l.disp_w as i32, l.disp_h as i32));
    }
    let (ox, oy) = l.origin();
    intersect(
        (0, 0, l.disp_w as i32, l.disp_h as i32),
        (
            -ox,
            TOOLBAR as i32 - oy,
            win.0 as i32,
            win.1 as i32 - TOOLBAR as i32,
        ),
    )
}

pub fn intersect(a: Area, b: Area) -> Option<Area> {
    let x0 = a.0.max(b.0);
    let y0 = a.1.max(b.1);
    let x1 = (a.0 + a.2).min(b.0 + b.2);
    let y1 = (a.1 + a.3).min(b.1 + b.3);
    (x1 > x0 && y1 > y0).then_some((x0, y0, x1 - x0, y1 - y0))
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    Triangle,
    Lanczos3,
}

impl Filter {
    fn support(self) -> f32 {
        match self {
            Filter::Triangle => 1.0,
            Filter::Lanczos3 => 3.0,
        }
    }

    fn weight(self, x: f32) -> f32 {
        let x = x.abs();
        match self {
            Filter::Triangle => (1.0 - x).max(0.0),
            Filter::Lanczos3 => {
                if x < 1e-6 {
                    1.0
                } else if x >= 3.0 {
                    0.0
                } else {
                    let px = std::f32::consts::PI * x;
                    3.0 * px.sin() * (px / 3.0).sin() / (px * px)
                }
            }
        }
    }
}

const WEIGHT_BITS: i32 = 14;

/// Fixed-point filter taps for output pixels `from..to` of a `src`→`dst` scale.
struct Taps {
    start: Vec<u32>,
    count: Vec<u32>,
    stride: usize,
    weights: Vec<i32>,
}

fn taps(src: u32, dst: u32, from: u32, to: u32, filter: Filter) -> Taps {
    let ratio = src as f32 / dst as f32;
    let sratio = ratio.max(1.0);
    let support = filter.support() * sratio;
    let stride = (2.0 * support).ceil() as usize + 2;
    let n = (to - from) as usize;
    let mut out = Taps {
        start: Vec::with_capacity(n),
        count: Vec::with_capacity(n),
        stride,
        weights: vec![0; n * stride],
    };
    let mut floats = vec![0f32; stride];
    for (j, o) in (from..to).enumerate() {
        let centre = (o as f32 + 0.5) * ratio;
        let left = ((centre - support).floor().max(0.0) as u32).min(src - 1);
        let right = ((centre + support).ceil() as u32).clamp(left + 1, src);
        let count = ((right - left) as usize).min(stride);
        let mut sum = 0.0;
        for (k, f) in floats.iter_mut().enumerate().take(count) {
            let i = left + k as u32;
            *f = filter.weight((i as f32 + 0.5 - centre) / sratio);
            sum += *f;
        }
        if sum.abs() < 1e-6 {
            // A degenerate window: take the nearest pixel.
            floats[..count].fill(0.0);
            let nearest = ((centre as u32).clamp(left, right - 1) - left) as usize;
            floats[nearest] = 1.0;
            sum = 1.0;
        }
        let mut largest = 0;
        for k in 0..count {
            if floats[k] > floats[largest] {
                largest = k;
            }
        }
        let row = &mut out.weights[j * stride..j * stride + count];
        let mut total = 0;
        for (w, f) in row.iter_mut().zip(&floats) {
            *w = (f / sum * (1 << WEIGHT_BITS) as f32).round() as i32;
            total += *w;
        }
        // Make the taps sum to exactly 1.0 so flat areas stay exact.
        row[largest] += (1 << WEIGHT_BITS) - total;
        out.start.push(left);
        out.count.push(count as u32);
    }
    out
}

/// Scale `src` to `disp` and return the display pixels in `rect`.
///
/// A region comes out exactly as the same pixels of a whole scale would,
/// which is what lets an annotation patch the cached view.
pub fn resample(src: &RgbaImage, disp: (u32, u32), rect: Area, filter: Filter) -> RgbaImage {
    let (rx, ry, rw, rh) = (rect.0 as u32, rect.1 as u32, rect.2 as u32, rect.3 as u32);
    let (sw, sh) = src.dimensions();
    let raw = src.as_raw();
    if disp == (sw, sh) {
        return image::imageops::crop_imm(src, rx, ry, rw, rh).to_image();
    }
    let xt = taps(sw, disp.0, rx, rx + rw, filter);
    let yt = taps(sh, disp.1, ry, ry + rh, filter);
    let r0 = *yt.start.iter().min().unwrap_or(&0);
    let r1 = yt
        .start
        .iter()
        .zip(&yt.count)
        .map(|(s, c)| s + c)
        .max()
        .unwrap_or(0);
    let w = rw as usize;
    // Horizontal pass into a 7-bit fixed-point intermediate.
    let mut mid = vec![0i32; (r1 - r0) as usize * w * 3];
    let mid_shift = WEIGHT_BITS - 7;
    let mid_half = 1 << (mid_shift - 1);
    for (row, out) in (r0..r1).zip(mid.chunks_exact_mut(w * 3)) {
        let line = &raw[row as usize * sw as usize * 4..(row as usize + 1) * sw as usize * 4];
        for (j, px) in out.chunks_exact_mut(3).enumerate() {
            let start = xt.start[j] as usize;
            let count = xt.count[j] as usize;
            let weights = &xt.weights[j * xt.stride..j * xt.stride + count];
            let (mut r, mut g, mut b) = (0i32, 0i32, 0i32);
            for (k, wt) in weights.iter().enumerate() {
                let p = (start + k) * 4;
                r += line[p] as i32 * wt;
                g += line[p + 1] as i32 * wt;
                b += line[p + 2] as i32 * wt;
            }
            px[0] = (r + mid_half) >> mid_shift;
            px[1] = (g + mid_half) >> mid_shift;
            px[2] = (b + mid_half) >> mid_shift;
        }
    }
    // Vertical pass.
    let mut result = RgbaImage::new(rw, rh);
    let mut acc = vec![0i32; w * 3];
    let shift = WEIGHT_BITS + 7;
    let half = 1 << (shift - 1);
    for (i, out) in result.chunks_exact_mut(w * 4).enumerate() {
        acc.fill(0);
        let start = (yt.start[i] - r0) as usize;
        let count = yt.count[i] as usize;
        for k in 0..count {
            let wt = yt.weights[i * yt.stride + k];
            let line = &mid[(start + k) * w * 3..(start + k + 1) * w * 3];
            for (a, v) in acc.iter_mut().zip(line) {
                *a += v * wt;
            }
        }
        for (p, a) in out.chunks_exact_mut(4).zip(acc.chunks_exact(3)) {
            p[0] = ((a[0] + half) >> shift).clamp(0, 255) as u8;
            p[1] = ((a[1] + half) >> shift).clamp(0, 255) as u8;
            p[2] = ((a[2] + half) >> shift).clamp(0, 255) as u8;
            p[3] = 255;
        }
    }
    result
}

/// Display pixels that depend on the image pixels in `area`.
pub fn display_area(img: (u32, u32), disp: (u32, u32), area: Area) -> Option<Area> {
    let axis = |src: u32, dst: u32, a0: i32, len: i32| {
        let ratio = src as f32 / dst as f32;
        let support = Filter::Lanczos3.support() * ratio.max(1.0) + 1.0;
        let d0 = (((a0 as f32 - support) / ratio) - 1.0).floor().max(0.0) as i32;
        let d1 = ((((a0 + len) as f32 + support) / ratio) + 1.0)
            .ceil()
            .min(dst as f32) as i32;
        (d0, d1 - d0)
    };
    let (x, w) = axis(img.0, disp.0, area.0, area.2);
    let (y, h) = axis(img.1, disp.1, area.1, area.3);
    intersect((x, y, w, h), (0, 0, disp.0 as i32, disp.1 as i32))
}

/// Identifies a cached scaled view.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViewKey {
    pub generation: u64,
    pub disp: (u32, u32),
    /// The kept display rectangle.
    pub rect: Area,
}

#[derive(Default)]
pub struct ViewCache {
    pub key: Option<ViewKey>,
    /// Scaled with Lanczos3 rather than the fast filter.
    pub fine: bool,
    pub image: RgbaImage,
}

impl ViewCache {
    /// Make the cache hold `key`. Returns true when a fast-filtered view was
    /// made, so the caller should schedule a refine.
    pub fn update(&mut self, image: &RgbaImage, key: ViewKey, fine: bool) -> bool {
        if self.key == Some(key) && (self.fine || !fine) {
            return false;
        }
        let filter = if fine {
            Filter::Lanczos3
        } else {
            Filter::Triangle
        };
        self.image = resample(image, key.disp, key.rect, filter);
        self.key = Some(key);
        self.fine = fine || key.disp == image.dimensions();
        !self.fine
    }

    /// Re-scale only what `changed` (image areas) touched in the new
    /// generation. Returns the patched display areas, or None when the cache
    /// does not hold the previous generation at this size.
    pub fn patch(
        &mut self,
        image: &RgbaImage,
        generation: u64,
        changed: &[Area],
    ) -> Option<Vec<Area>> {
        let key = self.key.as_mut()?;
        if key.generation + 1 != generation {
            return None;
        }
        let filter = if self.fine {
            Filter::Lanczos3
        } else {
            Filter::Triangle
        };
        let mut patched = Vec::new();
        for area in changed {
            let Some(d) = display_area(image.dimensions(), key.disp, *area)
                .and_then(|d| intersect(d, key.rect))
            else {
                continue;
            };
            let part = resample(image, key.disp, d, filter);
            image::imageops::replace(
                &mut self.image,
                &part,
                (d.0 - key.rect.0) as i64,
                (d.1 - key.rect.1) as i64,
            );
            patched.push(d);
        }
        key.generation = generation;
        Some(patched)
    }
}

/// Converts RGBA pixels to the server's 32-bit ZPixmap layout.
pub fn to_x_bytes(lsb_first: bool, img: &RgbaImage, out: &mut Vec<u8>) {
    out.clear();
    out.reserve(img.as_raw().len());
    for p in img.as_raw().chunks_exact(4) {
        if lsb_first {
            out.extend_from_slice(&[p[2], p[1], p[0], 255]);
        } else {
            out.extend_from_slice(&[p[0], p[1], p[2], 255]);
        }
    }
}

/// Upload ZPixmap bytes in bounded bands so no request exceeds the limit.
#[allow(clippy::too_many_arguments)]
pub fn put_bytes(
    c: &RustConnection,
    drawable: Drawable,
    gc: Gcontext,
    depth: u8,
    x: i16,
    y: i16,
    width: u32,
    height: u32,
    data: &[u8],
) -> Result<(), String> {
    let width = width.max(1);
    let row_bytes = width as usize * 4;
    let max_rows = (250_000 / row_bytes).max(1) as u32;
    let mut row = 0;
    while row < height {
        let rows = max_rows.min(height - row);
        let start = row as usize * row_bytes;
        let end = start + rows as usize * row_bytes;
        c.put_image(
            ImageFormat::Z_PIXMAP,
            drawable,
            gc,
            width as u16,
            rows as u16,
            x,
            y + row as i16,
            0,
            depth,
            &data[start..end],
        )
        .map_err(|e| format!("put_image: {e}"))?;
        row += rows;
    }
    Ok(())
}

/// The live drag shape, in window coordinates.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Preview {
    pub mode: u8,
    pub from: (i32, i32),
    pub to: (i32, i32),
    pub colour: u32,
    pub stroke: u32,
}

/// Window rectangle that a preview may touch.
pub fn preview_bounds(p: &Preview) -> Area {
    let pad = 20 + 2 * p.stroke as i32;
    let x0 = p.from.0.min(p.to.0) - pad;
    let y0 = p.from.1.min(p.to.1) - pad;
    let x1 = p.from.0.max(p.to.0) + pad;
    let y1 = p.from.1.max(p.to.1) + pad;
    (x0, y0, x1 - x0, y1 - y0)
}

fn rect(x: i32, y: i32, w: i32, h: i32) -> Rectangle {
    let clamp = |v: i32| v.clamp(i16::MIN as i32, i16::MAX as i32) as i16;
    Rectangle {
        x: clamp(x),
        y: clamp(y),
        width: w.clamp(0, u16::MAX as i32) as u16,
        height: h.clamp(0, u16::MAX as i32) as u16,
    }
}

/// Inner border rectangles of a box, like the exported rectangle.
fn border(x: i32, y: i32, w: i32, h: i32, t: i32) -> Vec<Rectangle> {
    if w <= 0 || h <= 0 {
        return Vec::new();
    }
    if w <= 2 * t || h <= 2 * t {
        return vec![rect(x, y, w, h)];
    }
    vec![
        rect(x, y, w, t),
        rect(x, y + h - t, w, t),
        rect(x, y + t, t, h - 2 * t),
        rect(x + w - t, y + t, t, h - 2 * t),
    ]
}

/// The two end points of an arrow head at (x1, y1).
pub fn arrow_head(x0: i32, y0: i32, x1: i32, y1: i32, length: f32) -> [(i32, i32); 2] {
    let angle = ((y1 - y0) as f32).atan2((x1 - x0) as f32);
    [0.45_f32, -0.45].map(|offset| {
        (
            (x1 as f32 - (angle + offset).cos() * length).round() as i32,
            (y1 as f32 - (angle + offset).sin() * length).round() as i32,
        )
    })
}

/// Draw the preview straight onto the window with core X requests.
pub fn draw_preview(
    c: &RustConnection,
    win: Window,
    gc: Gcontext,
    p: &Preview,
) -> Result<(), String> {
    let left = p.from.0.min(p.to.0);
    let top = p.from.1.min(p.to.1);
    let width = (p.to.0 - p.from.0).abs();
    let height = (p.to.1 - p.from.1).abs();
    let t = p.stroke.max(1) as i32;
    let e = |e: x11rb::errors::ConnectionError| e.to_string();
    c.change_gc(
        gc,
        &ChangeGCAux::new()
            .foreground(p.colour)
            .line_width(t as u32)
            .cap_style(CapStyle::PROJECTING),
    )
    .map_err(e)?;
    match p.mode {
        b'm' => {
            c.poly_fill_rectangle(win, gc, &[rect(left, top, width, height)])
                .map_err(e)?;
        }
        b'a' => {
            let head = arrow_head(p.from.0, p.from.1, p.to.0, p.to.1, 14.0 * t as f32 / 3.0);
            let segment = |(x0, y0): (i32, i32), (x1, y1): (i32, i32)| Segment {
                x1: x0 as i16,
                y1: y0 as i16,
                x2: x1 as i16,
                y2: y1 as i16,
            };
            c.poly_segment(
                win,
                gc,
                &[
                    segment(p.from, p.to),
                    segment(p.to, head[0]),
                    segment(p.to, head[1]),
                ],
            )
            .map_err(e)?;
        }
        _ => {
            c.poly_fill_rectangle(win, gc, &border(left, top, width, height, t))
                .map_err(e)?;
        }
    }
    Ok(())
}

/// Wait for the next event; a burst of queued MotionNotify becomes the latest one.
pub fn next_event(c: &RustConnection, pending: &mut Option<Event>) -> Result<Event, String> {
    let mut event = match pending.take() {
        Some(event) => event,
        None => c.wait_for_event().map_err(|e| e.to_string())?,
    };
    if matches!(event, Event::MotionNotify(_)) {
        while let Some(next) = c.poll_for_event().map_err(|e| e.to_string())? {
            if matches!(next, Event::MotionNotify(_)) {
                event = next;
            } else {
                *pending = Some(next);
                break;
            }
        }
    }
    Ok(event)
}

/// Sends `atom` to the window once `REFINE_DELAY` has passed without a newer
/// request.
///
/// The thread blocks in a channel receive: it does no work while idle and
/// ends when the timer is dropped.
pub struct RefineTimer {
    tx: Option<mpsc::Sender<u32>>,
    thread: Option<thread::JoinHandle<()>>,
    token: u32,
}

impl RefineTimer {
    pub fn new(c: Arc<RustConnection>, win: Window, atom: Atom) -> Self {
        let (tx, rx) = mpsc::channel::<u32>();
        let thread = thread::spawn(move || {
            while let Ok(mut token) = rx.recv() {
                loop {
                    match rx.recv_timeout(REFINE_DELAY) {
                        Ok(newer) => token = newer,
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            let event = ClientMessageEvent::new(32, win, atom, [token, 0, 0, 0, 0]);
                            let _ = c.send_event(false, win, EventMask::NO_EVENT, event);
                            let _ = c.flush();
                            break;
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => return,
                    }
                }
            }
        });
        Self {
            tx: Some(tx),
            thread: Some(thread),
            token: 0,
        }
    }

    /// Restart the delay.
    pub fn schedule(&mut self) {
        self.token = self.token.wrapping_add(1);
        if let Some(tx) = &self.tx {
            let _ = tx.send(self.token);
        }
    }

    pub fn is_current(&self, token: u32) -> bool {
        token == self.token
    }
}

impl Drop for RefineTimer {
    fn drop(&mut self) {
        self.tx = None;
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::Rgba;

    fn pattern(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_fn(w, h, |x, y| {
            Rgba([
                (x * 7 % 256) as u8,
                (y * 5 % 256) as u8,
                ((x ^ y) % 256) as u8,
                255,
            ])
        })
    }

    #[test]
    fn fit_layout_centres_the_image_below_the_toolbar() {
        let l = view_layout((3840, 2160), (1920, 1132), 1.0, (0.0, 0.0));
        assert_eq!((l.disp_w, l.disp_h), (1920, 1080));
        assert_eq!((l.x, l.y), (0.0, TOOLBAR as f32));
        assert_eq!(l.image_point((3840, 2160), 960, 52 + 540), (1920, 1080));
    }

    #[test]
    fn zoomed_views_only_keep_the_visible_part() {
        let win = (1920, 1132);
        let l = view_layout((3840, 2160), win, 8.0, (0.0, 0.0));
        let (x, y, w, h) = view_rect(&l, win).unwrap();
        assert_eq!((w, h), (1920, 1080));
        assert_eq!(
            (x, y),
            ((l.disp_w as i32 - 1920) / 2, (l.disp_h as i32 - 1080) / 2)
        );
        let fit = view_layout((3840, 2160), win, 1.0, (0.0, 0.0));
        assert_eq!(view_rect(&fit, win), Some((0, 0, 1920, 1080)));
    }

    #[test]
    fn a_region_resample_matches_the_whole_one() {
        let src = pattern(301, 177);
        for filter in [Filter::Triangle, Filter::Lanczos3] {
            for disp in [(97, 57), (450, 265), (301, 177)] {
                let whole = resample(&src, disp, (0, 0, disp.0 as i32, disp.1 as i32), filter);
                let part = resample(&src, disp, (13, 9, 40, 30), filter);
                let expected = image::imageops::crop_imm(&whole, 13, 9, 40, 30).to_image();
                assert!(part == expected, "{filter:?} {disp:?}");
            }
        }
    }

    #[test]
    fn resampling_keeps_flat_colour_exact() {
        let src = RgbaImage::from_pixel(640, 360, Rgba([200, 30, 7, 255]));
        for filter in [Filter::Triangle, Filter::Lanczos3] {
            let out = resample(&src, (311, 175), (0, 0, 311, 175), filter);
            assert!(out.pixels().all(|p| p.0 == [200, 30, 7, 255]), "{filter:?}");
        }
    }

    #[test]
    fn a_patched_view_equals_a_fresh_one() {
        let mut src = pattern(400, 300);
        let l = view_layout((400, 300), (250, 52 + 190), 1.0, (0.0, 0.0));
        let key = ViewKey {
            generation: 0,
            disp: (l.disp_w, l.disp_h),
            rect: view_rect(&l, (250, 242)).unwrap(),
        };
        let mut cache = ViewCache::default();
        cache.update(&src, key, true);
        for y in 100..140 {
            for x in 50..90 {
                src.put_pixel(x, y, Rgba([255, 0, 0, 255]));
            }
        }
        let patched = cache.patch(&src, 1, &[(50, 100, 40, 40)]).unwrap();
        assert_eq!(patched.len(), 1);
        let fresh = resample(&src, key.disp, key.rect, Filter::Lanczos3);
        assert!(cache.image == fresh);
    }

    #[test]
    fn a_fine_cache_satisfies_a_fast_request() {
        let image = pattern(100, 50);
        let l = view_layout((100, 50), (60, 82), 1.0, (0.0, 0.0));
        let key = ViewKey {
            generation: 1,
            disp: (l.disp_w, l.disp_h),
            rect: (0, 0, l.disp_w as i32, l.disp_h as i32),
        };
        let mut cache = ViewCache::default();
        assert!(cache.update(&image, key, false), "fast view needs a refine");
        assert!(!cache.update(&image, key, true));
        assert!(cache.fine);
        assert!(!cache.update(&image, key, false), "kept the fine view");
        assert!(cache.fine);
    }
}
