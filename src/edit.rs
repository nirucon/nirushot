//! The non-destructive editing model.
//!
//! The captured image is never changed. Edits are an ordered list of
//! operations in the coordinates of the image they apply to (a crop starts a
//! new coordinate space for the operations after it). The shown and exported
//! image is that list rendered over the original, and undo/redo step through
//! earlier lists.

use ab_glyph::{point, Font, FontArc, PxScale, ScaleFont};
use image::{Rgba, RgbaImage};

use crate::view::Area;

/// Most undo steps kept. Operations are tiny, so this bounds bookkeeping,
/// not image memory: no image snapshots are stored.
pub const UNDO_LIMIT: usize = 100;

pub const RED: Rgba<u8> = Rgba([220, 70, 70, 255]);
pub const BLACK: Rgba<u8> = Rgba([0, 0, 0, 255]);
/// The stroke 0.4 always used; the medium stroke.
pub const MEDIUM: u32 = 3;
/// Thin, medium and thick strokes, in image pixels.
pub const STROKES: [u32; 3] = [2, MEDIUM, 6];

/// The palette after the theme's focus colour: red (0.4's colour), yellow,
/// green, white and black.
pub const PALETTE: [Rgba<u8>; 5] = [
    RED,
    Rgba([240, 196, 40, 255]),
    Rgba([70, 180, 90, 255]),
    Rgba([255, 255, 255, 255]),
    BLACK,
];

#[derive(Clone, Debug, PartialEq)]
pub enum Op {
    Crop {
        x: u32,
        y: u32,
        w: u32,
        h: u32,
    },
    Rect {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        colour: Rgba<u8>,
        stroke: u32,
    },
    Arrow {
        x0: i32,
        y0: i32,
        x1: i32,
        y1: i32,
        colour: Rgba<u8>,
        stroke: u32,
    },
    /// Always opaque black: a blur or pixelation could leak content.
    Mask {
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    },
    /// One line of text; (x, y) is the top-left of its line box.
    Text {
        x: i32,
        y: i32,
        text: String,
        colour: Rgba<u8>,
        size: u32,
    },
}

/// Text sizes for S, M and L, in image pixels.
pub const TEXT_SIZES: [u32; 3] = [20, 32, 48];

impl Op {
    fn is_crop(&self) -> bool {
        matches!(self, Op::Crop { .. })
    }

    /// Image areas this operation draws into (unused for a crop).
    pub fn areas(&self, font: Option<&FontArc>) -> Vec<Area> {
        match *self {
            Op::Text {
                x,
                y,
                ref text,
                size,
                ..
            } => font
                .map(|font| vec![text_area(font, size as f32, x, y, text)])
                .unwrap_or_default(),
            Op::Crop { x, y, w, h } => vec![(x as i32, y as i32, w as i32, h as i32)],
            Op::Rect {
                x, y, w, h, stroke, ..
            } => {
                let t = stroke as i32;
                vec![
                    (x, y, w, t),
                    (x, y + h - t, w, t),
                    (x, y, t, h),
                    (x + w - t, y, t, h),
                ]
            }
            Op::Arrow {
                x0,
                y0,
                x1,
                y1,
                stroke,
                ..
            } => {
                let pad = arrow_head_length(stroke).ceil() as i32 + stroke as i32 + 2;
                vec![(
                    x0.min(x1) - pad,
                    y0.min(y1) - pad,
                    (x1 - x0).abs() + 2 * pad,
                    (y1 - y0).abs() + 2 * pad,
                )]
            }
            Op::Mask { x, y, w, h } => vec![(x, y, w, h)],
        }
    }

    fn draw(&self, image: &mut RgbaImage, font: Option<&FontArc>) {
        match *self {
            Op::Text {
                x,
                y,
                ref text,
                colour,
                size,
            } => {
                if let Some(font) = font {
                    draw_text(image, font, size as f32, x as f32, y as f32, text, colour);
                }
            }
            Op::Crop { .. } => {}
            Op::Rect {
                x,
                y,
                w,
                h,
                colour,
                stroke,
            } => draw_rect(image, x, y, w, h, stroke, colour),
            Op::Arrow {
                x0,
                y0,
                x1,
                y1,
                colour,
                stroke,
            } => arrow(image, x0, y0, x1, y1, stroke, colour),
            Op::Mask { x, y, w, h } => fill_rect(image, x, y, w, h, BLACK),
        }
    }
}

/// Turn a finished drag (image coordinates) into an operation, with the
/// same minimum size and clamping 0.4 applied.
pub fn op_from_drag(
    tool: u8,
    start: (i32, i32),
    end: (i32, i32),
    dims: (u32, u32),
    colour: Rgba<u8>,
    stroke: u32,
) -> Option<Op> {
    let (ix0, iy0) = start;
    let (ix1, iy1) = end;
    let left = ix0.min(ix1) as u32;
    let top = iy0.min(iy1) as u32;
    let width = (ix1 - ix0).unsigned_abs();
    let height = (iy1 - iy0).unsigned_abs();
    let is_arrow = tool == b'a';
    if !((width > 2 && height > 2) || (is_arrow && (width > 2 || height > 2))) {
        return None;
    }
    let width = width.min(dims.0.saturating_sub(left));
    let height = height.min(dims.1.saturating_sub(top));
    let (x, y, w, h) = (left as i32, top as i32, width as i32, height as i32);
    match tool {
        b'c' if width > 0 && height > 0 => Some(Op::Crop {
            x: left,
            y: top,
            w: width,
            h: height,
        }),
        b'r' => Some(Op::Rect {
            x,
            y,
            w,
            h,
            colour,
            stroke,
        }),
        b'm' => Some(Op::Mask { x, y, w, h }),
        b'a' => Some(Op::Arrow {
            x0: ix0,
            y0: iy0,
            x1: ix1,
            y1: iy1,
            colour,
            stroke,
        }),
        _ => None,
    }
}

/// Render `ops` over `original`.
pub fn render(original: &RgbaImage, ops: &[Op], font: Option<&FontArc>) -> RgbaImage {
    let mut image = original.clone();
    for op in ops {
        apply(&mut image, op, font);
    }
    image
}

fn apply(image: &mut RgbaImage, op: &Op, font: Option<&FontArc>) {
    match *op {
        Op::Crop { x, y, w, h } => {
            *image = image::imageops::crop_imm(image, x, y, w, h).to_image();
        }
        _ => op.draw(image, font),
    }
}

/// Image area covered by `text` drawn at (x, y): every glyph outline plus
/// the line box.
pub fn text_area(font: &FontArc, size: f32, x: i32, y: i32, text: &str) -> Area {
    let scale = PxScale::from(size);
    let scaled = font.as_scaled(scale);
    let baseline = y as f32 + scaled.ascent();
    let (mut x0, mut y0) = (x as f32, y as f32);
    let mut x1 = x as f32 + text_width(font, size, text);
    let mut y1 = baseline - scaled.descent();
    let mut caret = x as f32;
    for ch in text.chars() {
        let id = font.glyph_id(ch);
        if let Some(outline) =
            font.outline_glyph(id.with_scale_and_position(scale, point(caret, baseline)))
        {
            let b = outline.px_bounds();
            x0 = x0.min(b.min.x);
            y0 = y0.min(b.min.y);
            x1 = x1.max(b.max.x);
            y1 = y1.max(b.max.y);
        }
        caret += scaled.h_advance(id);
    }
    let (x0, y0) = (x0.floor() as i32 - 1, y0.floor() as i32 - 1);
    let (x1, y1) = (x1.ceil() as i32 + 1, y1.ceil() as i32 + 1);
    (x0, y0, x1 - x0, y1 - y0)
}

pub fn text_width(font: &FontArc, size: f32, text: &str) -> f32 {
    let scaled = font.as_scaled(PxScale::from(size));
    text.chars()
        .map(|ch| scaled.h_advance(font.glyph_id(ch)))
        .sum()
}

/// Anti-aliased text with its line box's top-left at (x, y).
pub fn draw_text(
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

/// What the view has to redo after an edit.
#[derive(Clone, Debug, PartialEq)]
pub enum Change {
    /// Only these image areas changed; the size is the same.
    Areas(Vec<Area>),
    /// The size or everything may have changed.
    Full,
}

pub struct Editor {
    original: RgbaImage,
    ops: Vec<Op>,
    undo: Vec<Vec<Op>>,
    redo: Vec<Vec<Op>>,
    image: RgbaImage,
    generation: u64,
    font: Option<FontArc>,
}

impl Editor {
    pub fn new(original: RgbaImage, font: Option<FontArc>) -> Self {
        let image = original.clone();
        Self {
            font,
            original,
            ops: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            image,
            generation: 0,
        }
    }

    /// The rendered result: what is shown, saved and copied.
    pub fn image(&self) -> &RgbaImage {
        &self.image
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    #[allow(dead_code)]
    pub fn ops(&self) -> &[Op] {
        &self.ops
    }

    pub fn can_undo(&self) -> bool {
        !self.undo.is_empty()
    }

    pub fn push(&mut self, op: Op) -> Change {
        let mut ops = self.ops.clone();
        ops.push(op.clone());
        self.remember();
        self.ops = ops;
        self.redo.clear();
        self.generation += 1;
        // Drawing on top of the current result is the same as rendering
        // the whole list again.
        apply(&mut self.image, &op, self.font.as_ref());
        if op.is_crop() {
            Change::Full
        } else {
            Change::Areas(op.areas(self.font.as_ref()))
        }
    }

    /// Remove the most recent annotation (not a crop), as an undoable step.
    pub fn delete_last(&mut self) -> Option<Change> {
        let index = self.ops.iter().rposition(|op| !op.is_crop())?;
        let mut ops = self.ops.clone();
        let removed = ops.remove(index);
        self.remember();
        self.ops = ops;
        self.redo.clear();
        self.generation += 1;
        self.image = render(&self.original, &self.ops, self.font.as_ref());
        // Behind a crop it lives in other coordinates: redraw everything.
        if self.ops[index..].iter().any(Op::is_crop) {
            Some(Change::Full)
        } else {
            Some(Change::Areas(removed.areas(self.font.as_ref())))
        }
    }

    fn remember(&mut self) {
        self.undo.push(self.ops.clone());
        if self.undo.len() > UNDO_LIMIT {
            self.undo.remove(0);
        }
    }

    pub fn undo(&mut self) -> Option<Change> {
        let previous = self.undo.pop()?;
        let current = std::mem::replace(&mut self.ops, previous);
        let change = self.change_between(&current);
        self.redo.push(current);
        Some(change)
    }

    pub fn redo(&mut self) -> Option<Change> {
        let next = self.redo.pop()?;
        let current = std::mem::replace(&mut self.ops, next);
        let change = self.change_between(&current);
        self.undo.push(current);
        Some(change)
    }

    /// Re-render after `self.ops` replaced `before`, and describe the change.
    fn change_between(&mut self, before: &[Op]) -> Change {
        self.generation += 1;
        self.image = render(&self.original, &self.ops, self.font.as_ref());
        let (short, long) = if before.len() < self.ops.len() {
            (before, self.ops.as_slice())
        } else {
            (self.ops.as_slice(), before)
        };
        // Adding or removing annotations after the last crop keeps the size
        // and touches only their areas.
        if long.starts_with(short) && long[short.len()..].iter().all(|op| !op.is_crop()) {
            let font = self.font.as_ref();
            Change::Areas(
                long[short.len()..]
                    .iter()
                    .flat_map(|op| op.areas(font))
                    .collect(),
            )
        } else {
            Change::Full
        }
    }
}

pub fn fill_rect(image: &mut RgbaImage, x: i32, y: i32, w: i32, h: i32, colour: Rgba<u8>) {
    for yy in y.max(0)..(y + h).min(image.height() as i32) {
        for xx in x.max(0)..(x + w).min(image.width() as i32) {
            image.put_pixel(xx as u32, yy as u32, colour);
        }
    }
}

/// A rectangle outline `stroke` pixels wide, inside the box.
pub fn draw_rect(
    img: &mut RgbaImage,
    x: i32,
    y: i32,
    w: i32,
    h: i32,
    stroke: u32,
    colour: Rgba<u8>,
) {
    if w <= 0 || h <= 0 {
        return;
    }
    let t = stroke;
    let (x0, y0, x1, y1) = (
        x.max(0) as u32,
        y.max(0) as u32,
        (x + w).max(0) as u32,
        (y + h).max(0) as u32,
    );
    for yy in y0..y1.min(img.height()) {
        for xx in x0..x1.min(img.width()) {
            if xx < x0 + t || xx + t >= x1 || yy < y0 + t || yy + t >= y1 {
                img.put_pixel(xx, yy, colour);
            }
        }
    }
}

pub fn arrow_head_length(stroke: u32) -> f32 {
    14.0 * stroke as f32 / MEDIUM as f32
}

pub fn arrow(
    img: &mut RgbaImage,
    x0: i32,
    y0: i32,
    x1: i32,
    y1: i32,
    stroke: u32,
    colour: Rgba<u8>,
) {
    line(img, x0, y0, x1, y1, stroke, colour);
    let angle = ((y1 - y0) as f32).atan2((x1 - x0) as f32);
    let length = arrow_head_length(stroke);
    for offset in [0.45_f32, -0.45] {
        let end_x = x1 as f32 - (angle + offset).cos() * length;
        let end_y = y1 as f32 - (angle + offset).sin() * length;
        line(
            img,
            x1,
            y1,
            end_x.round() as i32,
            end_y.round() as i32,
            stroke,
            colour,
        );
    }
}

/// A Bresenham line drawn with a square brush `stroke` pixels wide.
pub fn line(
    img: &mut RgbaImage,
    mut x0: i32,
    mut y0: i32,
    x1: i32,
    y1: i32,
    stroke: u32,
    colour: Rgba<u8>,
) {
    let lo = -(stroke as i32 / 2);
    let hi = lo + stroke.max(1) as i32 - 1;
    let dx = (x1 - x0).abs();
    let sx = if x0 < x1 { 1 } else { -1 };
    let dy = -(y1 - y0).abs();
    let sy = if y0 < y1 { 1 } else { -1 };
    let mut err = dx + dy;
    loop {
        for ox in lo..=hi {
            for oy in lo..=hi {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn pattern(w: u32, h: u32) -> RgbaImage {
        RgbaImage::from_fn(w, h, |x, y| {
            Rgba([(x * 3 % 256) as u8, (y * 7 % 256) as u8, 90, 255])
        })
    }

    /// 0.4.10's destructive editing, kept verbatim as the reference.
    mod old {
        use image::{Rgba, RgbaImage};

        pub fn fill_rect(image: &mut RgbaImage, x: i32, y: i32, w: i32, h: i32, c: Rgba<u8>) {
            for yy in y.max(0)..(y + h).min(image.height() as i32) {
                for xx in x.max(0)..(x + w).min(image.width() as i32) {
                    image.put_pixel(xx as u32, yy as u32, c);
                }
            }
        }

        pub fn draw_rect(img: &mut RgbaImage, x: i32, y: i32, w: i32, h: i32, c: Rgba<u8>) {
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
                        img.put_pixel(xx, yy, c);
                    }
                }
            }
        }

        pub fn arrow(img: &mut RgbaImage, x0: i32, y0: i32, x1: i32, y1: i32, c: Rgba<u8>) {
            line(img, x0, y0, x1, y1, c);
            let angle = ((y1 - y0) as f32).atan2((x1 - x0) as f32);
            for offset in [0.45_f32, -0.45] {
                let end_x = x1 as f32 - (angle + offset).cos() * 14.0;
                let end_y = y1 as f32 - (angle + offset).sin() * 14.0;
                line(img, x1, y1, end_x.round() as i32, end_y.round() as i32, c);
            }
        }

        fn line(img: &mut RgbaImage, mut x0: i32, mut y0: i32, x1: i32, y1: i32, c: Rgba<u8>) {
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
                        if x >= 0 && y >= 0 && (x as u32) < img.width() && (y as u32) < img.height()
                        {
                            img.put_pixel(x as u32, y as u32, c);
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

        /// The release handler of 0.4.10, image coordinates already mapped.
        pub fn apply(image: &mut RgbaImage, mode: u8, a: (i32, i32), b: (i32, i32)) {
            let ((ix0, iy0), (ix1, iy1)) = (a, b);
            let left = ix0.min(ix1) as u32;
            let top = iy0.min(iy1) as u32;
            let width = (ix1 - ix0).unsigned_abs();
            let height = (iy1 - iy0).unsigned_abs();
            let is_arrow = mode == b'a';
            if (width > 2 && height > 2) || (is_arrow && (width > 2 || height > 2)) {
                let width = width.min(image.width() - left);
                let height = height.min(image.height() - top);
                let red = Rgba([220, 70, 70, 255]);
                match mode {
                    b'c' => {
                        *image =
                            image::imageops::crop_imm(image, left, top, width, height).to_image()
                    }
                    b'r' => draw_rect(
                        image,
                        left as i32,
                        top as i32,
                        width as i32,
                        height as i32,
                        red,
                    ),
                    b'm' => fill_rect(
                        image,
                        left as i32,
                        top as i32,
                        width as i32,
                        height as i32,
                        Rgba([0, 0, 0, 255]),
                    ),
                    b'a' => arrow(image, ix0, iy0, ix1, iy1, red),
                    _ => {}
                }
            }
        }
    }

    impl Editor {
        fn new_test(original: RgbaImage) -> Self {
            Editor::new(original, None)
        }
    }

    fn test_font() -> Option<FontArc> {
        [
            "/usr/share/fonts/truetype/dejavu/DejaVuSansMono.ttf",
            "/usr/share/fonts/TTF/DejaVuSansMono.ttf",
            "/usr/share/fonts/dejavu/DejaVuSansMono.ttf",
            "/usr/share/fonts/truetype/liberation/LiberationMono-Regular.ttf",
        ]
        .iter()
        .find_map(|p| std::fs::read(p).ok())
        .and_then(|data| FontArc::try_from_vec(data).ok())
    }

    #[test]
    fn text_stays_inside_its_bounds() {
        let Some(font) = test_font() else {
            eprintln!("no test font installed; skipped");
            return;
        };
        for (size, text) in [(20, "Hello, världen! @€"), (48, "jgpqy ÅÄÖ |"), (32, "→ x")] {
            let blank = RgbaImage::from_pixel(600, 120, Rgba([255, 255, 255, 255]));
            let mut image = blank.clone();
            let (x, y) = (40, 30);
            draw_text(
                &mut image,
                &font,
                size as f32,
                x as f32,
                y as f32,
                text,
                RED,
            );
            let (ax, ay, aw, ah) = text_area(&font, size as f32, x, y, text);
            let mut inked = 0;
            for (px, py, p) in image.enumerate_pixels() {
                if p != blank.get_pixel(px, py) {
                    inked += 1;
                    let (px, py) = (px as i32, py as i32);
                    assert!(
                        px >= ax && px < ax + aw && py >= ay && py < ay + ah,
                        "{text:?} pixel {px},{py} outside {:?}",
                        (ax, ay, aw, ah)
                    );
                }
            }
            assert!(inked > 20, "{text:?} drew nothing");
            assert!(ah >= size && ah < size * 2);
        }
    }

    #[test]
    fn text_is_an_undoable_operation() {
        let Some(font) = test_font() else {
            return;
        };
        let original = pattern(300, 100);
        let mut editor = Editor::new(original.clone(), Some(font.clone()));
        let op = Op::Text {
            x: 10,
            y: 10,
            text: "Note".into(),
            colour: RED,
            size: TEXT_SIZES[1],
        };
        let change = editor.push(op.clone());
        assert_eq!(change, Change::Areas(op.areas(Some(&font))));
        assert!(*editor.image() != original);
        editor.undo().unwrap();
        assert!(*editor.image() == original);
        editor.redo().unwrap();
        assert!(*editor.image() == render(&original, &[op], Some(&font)));
    }

    type Drag = (u8, (i32, i32), (i32, i32));

    const SESSION: &[Drag] = &[
        (b'r', (10, 12), (120, 90)),
        (b'a', (200, 20), (60, 150)),
        (b'm', (150, 100), (190, 140)),
        (b'r', (5, 5), (6, 6)), // too small: ignored by both
        (b'c', (20, 15), (230, 170)),
        (b'r', (30, 30), (400, 400)), // clamped to the cropped image
        (b'a', (0, 100), (180, 100)),
        (b'm', (100, 0), (140, 20)),
        (b'c', (10, 10), (150, 120)),
        (b'r', (1, 1), (60, 50)),
    ];

    fn run(session: &[Drag]) -> (RgbaImage, Editor) {
        let original = pattern(260, 190);
        let mut reference = original.clone();
        let mut editor = Editor::new(original, None);
        for &(tool, a, b) in session {
            old::apply(&mut reference, tool, a, b);
            if let Some(op) = op_from_drag(tool, a, b, editor.image().dimensions(), RED, MEDIUM) {
                editor.push(op);
            }
        }
        (reference, editor)
    }

    #[test]
    fn export_matches_the_destructive_path() {
        let (reference, editor) = run(SESSION);
        assert!(*editor.image() == reference);
        assert!(render(&editor.original, editor.ops(), None) == reference);
    }

    #[test]
    fn cropping_after_annotating_keeps_the_annotations_in_order() {
        let (_, editor) = run(SESSION);
        let kinds: Vec<_> = editor
            .ops()
            .iter()
            .map(|op| match op {
                Op::Crop { .. } => 'c',
                Op::Rect { .. } => 'r',
                Op::Arrow { .. } => 'a',
                Op::Mask { .. } => 'm',
                Op::Text { .. } => 't',
            })
            .collect();
        assert_eq!(kinds.iter().collect::<String>(), "ramcramcr");
        assert_eq!(editor.image().dimensions(), (140, 110));
        assert_eq!(editor.original.dimensions(), (260, 190));
    }

    #[test]
    fn undo_and_redo_every_operation() {
        for len in 1..=SESSION.len() {
            let (_, mut editor) = run(&SESSION[..len]);
            let (before, _) = run(&SESSION[..len - 1]);
            let after = editor.image().clone();
            let pushed = editor.ops().len();
            if SESSION[len - 1].0 == b'r' && len == 4 {
                // The ignored drag pushed nothing.
                continue;
            }
            editor.undo().expect("undo");
            assert!(*editor.image() == before, "undo step {len}");
            assert_eq!(editor.ops().len(), pushed - 1);
            editor.redo().expect("redo");
            assert!(*editor.image() == after, "redo step {len}");
        }
    }

    #[test]
    fn undo_reaches_the_original_and_a_new_edit_clears_redo() {
        let (_, mut editor) = run(SESSION);
        while editor.undo().is_some() {}
        assert!(*editor.image() == editor.original);
        assert!(!editor.can_undo());
        editor.redo().unwrap();
        editor.push(Op::Mask {
            x: 0,
            y: 0,
            w: 4,
            h: 4,
        });
        assert!(editor.redo().is_none());
    }

    #[test]
    fn annotation_undo_reports_only_its_area() {
        let mut editor = Editor::new_test(pattern(100, 100));
        let op = op_from_drag(b'r', (10, 10), (50, 40), (100, 100), RED, MEDIUM).unwrap();
        assert_eq!(editor.push(op.clone()), Change::Areas(op.areas(None)));
        assert_eq!(editor.undo(), Some(Change::Areas(op.areas(None))));
        let crop = op_from_drag(b'c', (10, 10), (50, 40), (100, 100), RED, MEDIUM).unwrap();
        assert_eq!(editor.push(crop), Change::Full);
        assert_eq!(editor.undo(), Some(Change::Full));
    }

    #[test]
    fn delete_removes_the_last_annotation_and_can_be_undone() {
        let (_, mut editor) = run(SESSION);
        let ops = editor.ops().to_vec();
        let image = editor.image().clone();
        // The session ends with a rectangle after the second crop.
        assert!(matches!(editor.delete_last(), Some(Change::Areas(_))));
        assert_eq!(editor.ops(), &ops[..ops.len() - 1]);
        // Next is the mask before that crop: everything is redrawn.
        assert_eq!(editor.delete_last(), Some(Change::Full));
        assert!(!editor
            .ops()
            .iter()
            .any(|op| matches!(op, Op::Mask { x: 100, .. })));
        assert_eq!(editor.image().dimensions(), image.dimensions());
        editor.undo().unwrap();
        editor.undo().unwrap();
        assert!(*editor.image() == image);
        let mut crops_only = Editor::new_test(pattern(50, 50));
        crops_only.push(Op::Crop {
            x: 1,
            y: 1,
            w: 40,
            h: 40,
        });
        assert_eq!(crops_only.delete_last(), None, "crops are not annotations");
    }

    #[test]
    fn history_is_bounded() {
        let mut editor = Editor::new_test(pattern(20, 20));
        for i in 0..(UNDO_LIMIT + 30) {
            editor.push(Op::Mask {
                x: (i % 20) as i32,
                y: 0,
                w: 1,
                h: 1,
            });
        }
        let mut steps = 0;
        while editor.undo().is_some() {
            steps += 1;
        }
        assert_eq!(steps, UNDO_LIMIT);
    }

    #[test]
    fn colour_and_stroke_apply_to_rectangles_and_arrows_but_not_masks() {
        for colour in PALETTE {
            for stroke in STROKES {
                let blank = RgbaImage::from_pixel(80, 60, Rgba([128, 128, 128, 255]));
                let rect =
                    op_from_drag(b'r', (10, 10), (70, 50), (80, 60), colour, stroke).unwrap();
                let image = render(&blank, &[rect], None);
                // The border is exactly `stroke` pixels wide, in the colour.
                for x in 10..10 + stroke {
                    assert_eq!(*image.get_pixel(x, 30), colour);
                }
                assert_eq!(
                    *image.get_pixel(10 + stroke, 30),
                    Rgba([128, 128, 128, 255])
                );
                let arrow =
                    op_from_drag(b'a', (5, 30), (75, 30), (80, 60), colour, stroke).unwrap();
                let image = render(&blank, &[arrow], None);
                assert_eq!(*image.get_pixel(40, 30), colour);
                let mask =
                    op_from_drag(b'm', (10, 10), (70, 50), (80, 60), colour, stroke).unwrap();
                let image = render(&blank, &[mask], None);
                assert_eq!(*image.get_pixel(40, 30), BLACK, "the mask is always black");
            }
        }
    }

    #[test]
    fn arrow_has_a_head_and_axis_aligned_support() {
        let mut image = RgbaImage::from_pixel(100, 40, Rgba([255, 255, 255, 255]));
        arrow(&mut image, 5, 20, 90, 20, MEDIUM, RED);
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
