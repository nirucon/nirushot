//! The non-destructive editing model.
//!
//! The captured image is never changed. Edits are an ordered list of
//! operations in the coordinates of the image they apply to (a crop starts a
//! new coordinate space for the operations after it). The shown and exported
//! image is that list rendered over the original, and undo/redo step through
//! earlier lists.

use image::{Rgba, RgbaImage};

use crate::view::Area;

/// Most undo steps kept. Operations are tiny, so this bounds bookkeeping,
/// not image memory: no image snapshots are stored.
pub const UNDO_LIMIT: usize = 100;

pub const RED: Rgba<u8> = Rgba([220, 70, 70, 255]);
pub const BLACK: Rgba<u8> = Rgba([0, 0, 0, 255]);
/// The stroke 0.4 always used; the medium stroke.
pub const MEDIUM: u32 = 3;

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
}

impl Op {
    fn is_crop(&self) -> bool {
        matches!(self, Op::Crop { .. })
    }

    /// Image areas this operation draws into (unused for a crop).
    pub fn areas(&self) -> Vec<Area> {
        match *self {
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

    fn draw(&self, image: &mut RgbaImage) {
        match *self {
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
pub fn render(original: &RgbaImage, ops: &[Op]) -> RgbaImage {
    let mut image = original.clone();
    for op in ops {
        apply(&mut image, op);
    }
    image
}

fn apply(image: &mut RgbaImage, op: &Op) {
    match *op {
        Op::Crop { x, y, w, h } => {
            *image = image::imageops::crop_imm(image, x, y, w, h).to_image();
        }
        _ => op.draw(image),
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
}

impl Editor {
    pub fn new(original: RgbaImage) -> Self {
        let image = original.clone();
        Self {
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
        apply(&mut self.image, &op);
        if op.is_crop() {
            Change::Full
        } else {
            Change::Areas(op.areas())
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
        self.image = render(&self.original, &self.ops);
        let (short, long) = if before.len() < self.ops.len() {
            (before, self.ops.as_slice())
        } else {
            (self.ops.as_slice(), before)
        };
        // Adding or removing annotations after the last crop keeps the size
        // and touches only their areas.
        if long.starts_with(short) && long[short.len()..].iter().all(|op| !op.is_crop()) {
            Change::Areas(long[short.len()..].iter().flat_map(Op::areas).collect())
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
        let mut editor = Editor::new(original);
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
        assert!(render(&editor.original, editor.ops()) == reference);
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
        let mut editor = Editor::new(pattern(100, 100));
        let op = op_from_drag(b'r', (10, 10), (50, 40), (100, 100), RED, MEDIUM).unwrap();
        assert_eq!(editor.push(op.clone()), Change::Areas(op.areas()));
        assert_eq!(editor.undo(), Some(Change::Areas(op.areas())));
        let crop = op_from_drag(b'c', (10, 10), (50, 40), (100, 100), RED, MEDIUM).unwrap();
        assert_eq!(editor.push(crop), Change::Full);
        assert_eq!(editor.undo(), Some(Change::Full));
    }

    #[test]
    fn history_is_bounded() {
        let mut editor = Editor::new(pattern(20, 20));
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
