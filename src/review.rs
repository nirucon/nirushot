//! The review dialog.
//!
//! A normal focused floating dialog: it holds no keyboard or pointer grab, so
//! a screen locker can always take over while a review is open.

use std::{
    env,
    path::{Path, PathBuf},
    process::Command,
    sync::Arc,
};

use ab_glyph::FontArc;
use image::{Rgba, RgbaImage};
use x11rb::{
    connection::Connection,
    properties::{WmSizeHints, WmSizeHintsSpecification},
    protocol::{xproto::*, Event},
    rust_connection::RustConnection,
    wrapper::ConnectionExt as _,
};

use crate::{
    atomic_save, copy_to_clipboard, default_path, draw_text,
    edit::{fill_rect, op_from_drag, text_area, Change, Editor, Op, PALETTE, STROKES, TEXT_SIZES},
    keys::{self, keysym_char, Keymap},
    load_font, load_theme, monitor, notify, ocr, rgb, text_width,
    view::{
        draw_preview, intersect, next_event, preview_bounds, put_bytes, to_x_bytes, view_layout,
        view_rect, Area, Layout, Preview, RefineTimer, ViewCache, ViewKey, TOOLBAR,
    },
    Theme,
};

pub struct Output<'a> {
    pub temp: &'a Path,
    pub save: Option<&'a Path>,
    pub notify: bool,
    /// A root point inside the capture, used to choose the monitor.
    pub hint: Option<(i32, i32)>,
}

/// Something clickable in the toolbar.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Item {
    Tool(u8),
    /// Stroke width, or text size while the text tool is active.
    Size(usize),
    Colour(usize),
    Undo,
    /// Copy the text in the image (OCR).
    Ocr,
}

const TOOLS: [(u8, &str); 6] = [
    (b'c', "Crop"),
    (b'r', "Rect"),
    (b'a', "Arrow"),
    (b'm', "Mask"),
    (b't', "Text"),
    (b'h', "Highlight"),
];

const OCR_LABEL: &str = "Copy text";

const SIZE_LABELS: [&str; 3] = ["S", "M", "L"];
const CELL: f32 = 22.0;
const SWATCH: f32 = 18.0;

/// Toolbar items with their x position and width.
fn toolbar_items(font: &FontArc) -> Vec<(f32, f32, Item)> {
    let mut items = Vec::new();
    let mut pos = 118.0;
    for (key, label) in TOOLS {
        let width = 28.0 + text_width(font, 12.0, label);
        items.push((pos, width, Item::Tool(key)));
        pos += width + 8.0;
    }
    pos += 8.0;
    for i in 0..SIZE_LABELS.len() {
        items.push((pos, CELL, Item::Size(i)));
        pos += CELL + 2.0;
    }
    pos += 12.0;
    for i in 0..=PALETTE.len() {
        items.push((pos, SWATCH, Item::Colour(i)));
        pos += SWATCH + 2.0;
    }
    pos += 12.0;
    let width = 28.0 + text_width(font, 12.0, "Undo");
    items.push((pos, width, Item::Undo));
    pos += width + 8.0;
    let width = 28.0 + text_width(font, 12.0, OCR_LABEL);
    items.push((pos, width, Item::Ocr));
    items
}

fn toolbar_hit(font: &FontArc, x: i32) -> Option<Item> {
    toolbar_items(font)
        .into_iter()
        .find(|(pos, width, _)| (x as f32) >= *pos && (x as f32) < pos + width)
        .map(|(_, _, item)| item)
}

struct ToolbarState {
    tool: u8,
    zoom: f32,
    can_undo: bool,
    /// The picker's selection: text size for the text tool, else stroke.
    size: usize,
    colour: usize,
    colours: [Rgba<u8>; 6],
}

fn outline(canvas: &mut RgbaImage, x: i32, w: i32, colour: Rgba<u8>) {
    fill_rect(canvas, x, 13, w, 1, colour);
    fill_rect(canvas, x, 38, w, 1, colour);
    fill_rect(canvas, x, 13, 1, 26, colour);
    fill_rect(canvas, x + w - 1, 13, 1, 26, colour);
}

fn draw_toolbar(canvas: &mut RgbaImage, theme: &Theme, font: Option<&FontArc>, s: &ToolbarState) {
    let win_w = canvas.width() as i32;
    fill_rect(canvas, 0, 0, win_w, TOOLBAR as i32, rgb(theme.surface));
    fill_rect(canvas, 0, 0, win_w, 2, rgb(theme.accent));
    fill_rect(canvas, 0, TOOLBAR as i32 - 1, win_w, 1, rgb(theme.border));
    let Some(font) = font else {
        return;
    };
    draw_text(
        canvas,
        font,
        14.0,
        16.0,
        10.0,
        "NIRUSHOT",
        rgb(theme.foreground),
    );
    let mut end = 0.0f32;
    for (pos, width, item) in toolbar_items(font) {
        let (x, w) = (pos as i32, width as i32);
        match item {
            Item::Tool(key) => {
                let label = TOOLS
                    .iter()
                    .find(|(k, _)| *k == key)
                    .map(|(_, l)| *l)
                    .unwrap_or("");
                let active = s.tool == key;
                if active {
                    fill_rect(canvas, x, 13, w, 26, rgb(theme.foreground));
                }
                let colour = if active {
                    theme.background
                } else {
                    theme.muted
                };
                draw_text(canvas, font, 12.0, pos + 9.0, 20.0, label, rgb(colour));
            }
            Item::Size(i) => {
                let active = s.size == i;
                if active {
                    fill_rect(canvas, x, 13, w, 26, rgb(theme.foreground));
                }
                let colour = rgb(if active {
                    theme.background
                } else {
                    theme.muted
                });
                if s.tool == b't' {
                    let label = SIZE_LABELS[i];
                    let lx = pos + (CELL - text_width(font, 12.0, label)) / 2.0;
                    draw_text(canvas, font, 12.0, lx, 20.0, label, colour);
                } else {
                    // A bar as thick as the stroke it stands for.
                    let t = [1, 2, 4][i];
                    fill_rect(canvas, x + 5, 26 - t / 2, w - 10, t, colour);
                }
            }
            Item::Colour(i) => {
                if s.colour == i {
                    outline(canvas, x - 1, w + 2, rgb(theme.foreground));
                }
                fill_rect(canvas, x + 3, 19, w - 6, 14, rgb(theme.border));
                fill_rect(canvas, x + 4, 20, w - 8, 12, s.colours[i]);
            }
            Item::Undo => {
                outline(canvas, x, w, rgb(theme.border));
                let colour = if s.can_undo {
                    theme.foreground
                } else {
                    theme.muted
                };
                draw_text(canvas, font, 12.0, pos + 9.0, 20.0, "Undo", rgb(colour));
            }
            Item::Ocr => {
                outline(canvas, x, w, rgb(theme.border));
                draw_text(
                    canvas,
                    font,
                    12.0,
                    pos + 9.0,
                    20.0,
                    OCR_LABEL,
                    rgb(theme.foreground),
                );
            }
        }
        end = pos + width;
    }
    let zoom_label = if (s.zoom - 1.0).abs() < 0.01 {
        "fit".to_string()
    } else {
        format!("{:.0}%", s.zoom * 100.0)
    };
    draw_text(
        canvas,
        font,
        12.0,
        end + 18.0,
        20.0,
        &zoom_label,
        rgb(theme.muted),
    );
    let line1 = "s/^S save   y/^C copy   o text   u share   del remove   esc close";
    let line2 = "^Z undo   1-6 colour   w width   space-drag pan   +/- 0 zoom";
    let hint_width = text_width(font, 11.0, line1).max(text_width(font, 11.0, line2));
    let hint_x = win_w as f32 - hint_width - 16.0;
    // Hints are a convenience; they give way to the controls on a narrow window.
    if hint_x > end + 70.0 {
        draw_text(canvas, font, 11.0, hint_x, 7.0, line1, rgb(theme.muted));
        draw_text(canvas, font, 11.0, hint_x, 27.0, line2, rgb(theme.muted));
    }
}

/// Text being typed, before Enter turns it into an operation.
struct Typing {
    /// Top-left of the line box, in image coordinates.
    at: (i32, i32),
    text: String,
}

/// The composed frame lives in a server pixmap the size of the window.
struct Frame {
    pixmap: Pixmap,
    bytes: Vec<u8>,
    toolbar: RgbaImage,
}

impl Frame {
    fn new(c: &RustConnection, win: Window, depth: u8, w: u32, h: u32) -> Result<Self, String> {
        let pixmap = c.generate_id().map_err(|e| e.to_string())?;
        c.create_pixmap(depth, pixmap, win, w as u16, h as u16)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            pixmap,
            bytes: Vec::new(),
            toolbar: RgbaImage::new(w, TOOLBAR),
        })
    }
}

fn image_area(win_w: u32, win_h: u32) -> Area {
    (
        0,
        TOOLBAR as i32,
        win_w as i32,
        win_h.saturating_sub(TOOLBAR) as i32,
    )
}

fn intern_atom(c: &RustConnection, name: &str) -> Option<Atom> {
    c.intern_atom(false, name.as_bytes())
        .ok()
        .and_then(|r| r.reply().ok())
        .map(|r| r.atom)
}

enum Flow {
    Continue,
    Close,
}

struct Ui<'a> {
    c: Arc<RustConnection>,
    win: Window,
    gc: Gcontext,
    preview_gc: Gcontext,
    depth: u8,
    lsb_first: bool,
    wm_protocols: Atom,
    wm_delete: Atom,
    refine_atom: Atom,
    frame: Frame,
    refine: RefineTimer,
    theme: Theme,
    font: Option<FontArc>,
    out: Output<'a>,
    editor: Editor,
    keymap: Keymap,
    tool: u8,
    zoom: f32,
    pan: (f32, f32),
    win_w: u32,
    win_h: u32,
    /// Start and current point of an annotation drag, in window coordinates.
    drag: Option<((i32, i32), (i32, i32))>,
    typing: Option<Typing>,
    text_size: usize,
    stroke: usize,
    colour: usize,
    pan_drag: Option<(i32, i32, f32, f32)>,
    cache: ViewCache,
    fine: bool,
    layout: Layout,
    need_frame: bool,
    need_toolbar: bool,
    /// Image areas changed by the last edit, re-scaled on their own.
    changed: Vec<Area>,
    need_preview: bool,
    /// An event read ahead while coalescing motion or detecting key repeat.
    pending: Option<Event>,
    /// Space is held: a left drag pans, whatever the tool.
    space_held: bool,
    /// Window areas the drag preview and the text being typed cover.
    drawn: Vec<Area>,
}

pub fn review(image: RgbaImage, out: Output<'_>) -> Result<(), String> {
    let (c, screen) = x11rb::connect(None).map_err(|e| e.to_string())?;
    let c = Arc::new(c);
    let root = c.setup().roots[screen].root;
    let depth = c.setup().roots[screen].root_depth;
    let lsb_first = c.setup().image_byte_order == ImageOrder::LSB_FIRST;
    let sw = c.setup().roots[screen].width_in_pixels as u32;
    let sh = c.setup().roots[screen].height_in_pixels as u32;
    // Open on the monitor holding the captured region (or the pointer), not
    // centred on the root, which straddles the seam between monitors.
    let monitors = monitor::monitors(&c, root, (sw, sh));
    let point = out
        .hint
        .or_else(|| monitor::pointer(&c, root))
        .unwrap_or((sw as i32 / 2, sh as i32 / 2));
    let screen_area = monitor::choose(&monitors, point).unwrap_or((0, 0, sw as i32, sh as i32));
    let (x, y, win_w, win_h) = monitor::placement(screen_area, image.dimensions());
    let (x, y, win_w, win_h) = (x as i16, y as i16, win_w as u32, win_h as u32);
    let win = c.generate_id().map_err(|e| e.to_string())?;
    let gc = c.generate_id().map_err(|e| e.to_string())?;
    let preview_gc = c.generate_id().map_err(|e| e.to_string())?;
    c.create_window(
        0,
        win,
        root,
        x,
        y,
        win_w as u16,
        win_h as u16,
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
                    | EventMask::KEY_RELEASE
                    | EventMask::STRUCTURE_NOTIFY
                    | EventMask::FOCUS_CHANGE,
            ),
    )
    .map_err(|e| e.to_string())?;
    c.create_gc(gc, win, &CreateGCAux::new().graphics_exposures(0))
        .map_err(|e| e.to_string())?;
    c.create_gc(preview_gc, win, &CreateGCAux::new().graphics_exposures(0))
        .map_err(|e| e.to_string())?;
    c.change_property8(
        PropMode::REPLACE,
        win,
        AtomEnum::WM_NAME,
        AtomEnum::STRING,
        b"NIRUSHOT \xe2\x80\x94 screenshot review",
    )
    .ok();
    c.change_property8(
        PropMode::REPLACE,
        win,
        AtomEnum::WM_CLASS,
        AtomEnum::STRING,
        b"nirushot\0Nirushot\0",
    )
    .ok();
    if let (Some(kind), Some(dialog)) = (
        intern_atom(&c, "_NET_WM_WINDOW_TYPE"),
        intern_atom(&c, "_NET_WM_WINDOW_TYPE_DIALOG"),
    ) {
        let _ = c.change_property32(PropMode::REPLACE, win, kind, AtomEnum::ATOM, &[dialog]);
    }
    // Ask the window manager to keep the chosen monitor and position.
    let hints = WmSizeHints {
        position: Some((
            WmSizeHintsSpecification::ProgramSpecified,
            x as i32,
            y as i32,
        )),
        size: Some((
            WmSizeHintsSpecification::ProgramSpecified,
            win_w as i32,
            win_h as i32,
        )),
        ..WmSizeHints::default()
    };
    let _ = hints.set_normal_hints(&*c, win);
    let wm_protocols = intern_atom(&c, "WM_PROTOCOLS").unwrap_or(0);
    let wm_delete = intern_atom(&c, "WM_DELETE_WINDOW").unwrap_or(0);
    let refine_atom = intern_atom(&c, "_NIRUSHOT_REFINE").unwrap_or(0);
    if wm_protocols != 0 && wm_delete != 0 {
        let _ = c.change_property32(
            PropMode::REPLACE,
            win,
            wm_protocols,
            AtomEnum::ATOM,
            &[wm_delete],
        );
    }
    let frame = Frame::new(&c, win, depth, win_w, win_h)?;
    c.map_window(win).map_err(|e| e.to_string())?;
    c.flush().map_err(|e| e.to_string())?;
    let refine = RefineTimer::new(Arc::clone(&c), win, refine_atom);
    let layout = view_layout(image.dimensions(), (win_w, win_h), 1.0, (0.0, 0.0));
    let keymap = Keymap::load(&c);
    let font = load_font();
    let mut ui = Ui {
        c,
        win,
        gc,
        preview_gc,
        depth,
        lsb_first,
        wm_protocols,
        wm_delete,
        refine_atom,
        frame,
        refine,
        theme: load_theme(),
        font: font.clone(),
        out,
        keymap,
        editor: Editor::new(image, font),
        tool: b'0',
        zoom: 1.0,
        pan: (0.0, 0.0),
        win_w,
        win_h,
        drag: None,
        typing: None,
        text_size: 1,
        stroke: 1,
        // Red, as 0.4 always drew.
        colour: 1,
        pan_drag: None,
        cache: ViewCache::default(),
        fine: false,
        layout,
        need_frame: true,
        need_toolbar: true,
        changed: Vec::new(),
        need_preview: false,
        drawn: Vec::new(),
        pending: None,
        space_held: false,
    };
    ui.run()
}

impl Ui<'_> {
    fn run(&mut self) -> Result<(), String> {
        loop {
            self.redraw()?;
            self.c.flush().map_err(|e| e.to_string())?;
            let event = next_event(&self.c, &mut self.pending)?;
            if let Flow::Close = self.handle(event)? {
                let _ = std::fs::remove_file(self.out.temp);
                let _ = self.c.destroy_window(self.win);
                let _ = self.c.flush();
                return Ok(());
            }
        }
    }

    fn err(e: impl ToString) -> String {
        e.to_string()
    }

    fn redraw(&mut self) -> Result<(), String> {
        if self.need_frame || self.need_toolbar {
            let state = ToolbarState {
                tool: self.tool,
                zoom: self.zoom,
                can_undo: self.editor.can_undo(),
                size: self.picker(),
                colour: self.colour,
                colours: self.colours(),
            };
            draw_toolbar(
                &mut self.frame.toolbar,
                &self.theme,
                self.font.as_ref(),
                &state,
            );
            self.paint_toolbar()?;
            self.need_toolbar = false;
        }
        if !self.need_frame && !self.changed.is_empty() {
            let changed = std::mem::take(&mut self.changed);
            let image = self.editor.image();
            match self.cache.patch(image, self.editor.generation(), &changed) {
                Some(areas) => {
                    let (ox, oy) = self.layout.origin();
                    for (x, y, w, h) in areas {
                        let window = image_area(self.win_w, self.win_h);
                        if let Some(area) = intersect((ox + x, oy + y, w, h), window) {
                            self.paint_area(area)?;
                        }
                    }
                    self.need_preview = true;
                }
                None => self.need_frame = true,
            }
        }
        if self.need_frame {
            let image = self.editor.image();
            self.layout = view_layout(
                image.dimensions(),
                (self.win_w, self.win_h),
                self.zoom,
                self.pan,
            );
            if let Some(rect) = view_rect(&self.layout, (self.win_w, self.win_h)) {
                let key = ViewKey {
                    generation: self.editor.generation(),
                    disp: (self.layout.disp_w, self.layout.disp_h),
                    rect,
                };
                if self.cache.update(image, key, self.fine) {
                    self.refine.schedule();
                }
            } else {
                self.cache.key = None;
            }
            self.paint_area(image_area(self.win_w, self.win_h))?;
            let clip = Rectangle {
                x: 0,
                y: TOOLBAR as i16,
                width: self.win_w as u16,
                height: self.win_h.saturating_sub(TOOLBAR) as u16,
            };
            self.c
                .set_clip_rectangles(ClipOrdering::UNSORTED, self.preview_gc, 0, 0, &[clip])
                .map_err(Self::err)?;
            self.changed.clear();
            self.drawn.clear();
            self.need_frame = false;
            self.need_preview = true;
        }
        if self.need_preview {
            let window = (0, 0, self.win_w as i32, self.win_h as i32);
            for area in std::mem::take(&mut self.drawn) {
                if let Some(area) = intersect(area, window) {
                    self.copy_to_window(area)?;
                }
            }
            if let Some((from, to)) = self.drag {
                let c = self.current_colour();
                let colour = match self.tool {
                    b'm' => 0x000000,
                    b'c' => 0xdcc88c,
                    b'h' => 0xffffff,
                    _ => (c[0] as u32) << 16 | (c[1] as u32) << 8 | c[2] as u32,
                };
                let preview = Preview {
                    mode: self.tool,
                    from,
                    to,
                    colour,
                    stroke: STROKES[self.stroke],
                };
                draw_preview(&self.c, self.win, self.preview_gc, &preview)?;
                self.drawn.push(preview_bounds(&preview));
            }
            self.draw_typing()?;
            self.need_preview = false;
        }
        Ok(())
    }

    /// Show the text being typed, with a caret, over the view.
    fn draw_typing(&mut self) -> Result<(), String> {
        let (Some(typing), Some(font)) = (&self.typing, &self.font) else {
            return Ok(());
        };
        let size = TEXT_SIZES[self.text_size] as f32 * self.layout.scale;
        let (wx, wy) = self
            .layout
            .window_point(typing.at.0 as f32, typing.at.1 as f32);
        let (wx, wy) = (wx.round() as i32, wy.round() as i32);
        let (ax, ay, aw, ah) = text_area(font, size, wx, wy, &typing.text);
        let caret = wx + text_width(font, size, &typing.text).round() as i32 + 1;
        let caret_w = (size / 14.0).ceil().max(1.0) as i32;
        let x1 = (ax + aw).max(caret + caret_w + 2);
        let y1 = (ay + ah).max(wy + size.ceil() as i32 + 2);
        let area = (ax.min(wx) - 2, ay.min(wy) - 2, 0, 0);
        let area = (area.0, area.1, x1 - area.0, y1 - area.1);
        let Some(area) = intersect(area, image_area(self.win_w, self.win_h)) else {
            return Ok(());
        };
        let mut canvas = self.area_pixels(area);
        let colour = self.current_colour();
        let (ox, oy) = ((wx - area.0) as f32, (wy - area.1) as f32);
        draw_text(&mut canvas, font, size, ox, oy, &typing.text, colour);
        fill_rect(
            &mut canvas,
            caret - area.0,
            wy - area.1,
            caret_w,
            size.ceil() as i32,
            colour,
        );
        to_x_bytes(self.lsb_first, &canvas, &mut self.frame.bytes);
        put_bytes(
            &self.c,
            self.win,
            self.gc,
            self.depth,
            area.0 as i16,
            area.1 as i16,
            area.2 as u32,
            area.3 as u32,
            &self.frame.bytes,
        )?;
        self.drawn.push(area);
        Ok(())
    }

    /// Turn the text being typed into an operation.
    fn commit_text(&mut self) {
        let Some(typing) = self.typing.take() else {
            return;
        };
        self.need_preview = true;
        if typing.text.trim().is_empty() {
            return;
        }
        let op = Op::Text {
            x: typing.at.0,
            y: typing.at.1,
            text: typing.text,
            colour: self.current_colour(),
            size: TEXT_SIZES[self.text_size],
        };
        self.edit(|editor| Some(editor.push(op)));
    }

    fn copy_to_window(&self, (x, y, w, h): Area) -> Result<(), String> {
        self.c
            .copy_area(
                self.frame.pixmap,
                self.win,
                self.gc,
                x as i16,
                y as i16,
                x as i16,
                y as i16,
                w as u16,
                h as u16,
            )
            .map_err(Self::err)?;
        Ok(())
    }

    /// Window `area` as it looks without previews: background and view.
    fn area_pixels(&self, area: Area) -> RgbaImage {
        let (ax, ay, aw, ah) = area;
        let mut canvas = RgbaImage::from_pixel(aw as u32, ah as u32, rgb(self.theme.background));
        if let Some(key) = self.cache.key {
            let (ox, oy) = self.layout.origin();
            let placed = (ox + key.rect.0, oy + key.rect.1, key.rect.2, key.rect.3);
            if let Some((x, y, w, h)) = intersect(placed, area) {
                let src = self.cache.image.as_raw();
                let src_stride = self.cache.image.width() as usize * 4;
                let dst_stride = aw as usize * 4;
                let dst = &mut *canvas;
                let len = w as usize * 4;
                for row in 0..h {
                    let s0 =
                        (y - placed.1 + row) as usize * src_stride + (x - placed.0) as usize * 4;
                    let d0 = (y - ay + row) as usize * dst_stride + (x - ax) as usize * 4;
                    dst[d0..d0 + len].copy_from_slice(&src[s0..s0 + len]);
                }
            }
        }
        canvas
    }

    /// Paint window `area` into the pixmap and copy it to the window.
    fn paint_area(&mut self, area: Area) -> Result<(), String> {
        let (ax, ay, aw, ah) = area;
        if aw <= 0 || ah <= 0 {
            return Ok(());
        }
        let canvas = self.area_pixels(area);
        to_x_bytes(self.lsb_first, &canvas, &mut self.frame.bytes);
        put_bytes(
            &self.c,
            self.frame.pixmap,
            self.gc,
            self.depth,
            ax as i16,
            ay as i16,
            aw as u32,
            ah as u32,
            &self.frame.bytes,
        )?;
        self.copy_to_window(area)
    }

    fn paint_toolbar(&mut self) -> Result<(), String> {
        to_x_bytes(self.lsb_first, &self.frame.toolbar, &mut self.frame.bytes);
        let (w, h) = self.frame.toolbar.dimensions();
        put_bytes(
            &self.c,
            self.frame.pixmap,
            self.gc,
            self.depth,
            0,
            0,
            w,
            h,
            &self.frame.bytes,
        )?;
        self.copy_to_window((0, 0, w as i32, h as i32))
    }

    /// Run an edit and let the view follow it.
    fn edit(&mut self, f: impl FnOnce(&mut Editor) -> Option<Change>) {
        let before = self.editor.image().dimensions();
        let Some(change) = f(&mut self.editor) else {
            return;
        };
        match change {
            Change::Areas(areas) => self.changed.extend(areas),
            Change::Full => {
                // A crop (or undoing one) starts from a fitted view again.
                if before != self.editor.image().dimensions() {
                    self.zoom = 1.0;
                    self.pan = (0.0, 0.0);
                }
                self.fine = false;
                self.need_frame = true;
            }
        }
        self.need_toolbar = true;
    }

    fn undo(&mut self) {
        self.edit(Editor::undo);
    }

    fn redo(&mut self) {
        self.edit(Editor::redo);
    }

    fn set_zoom(&mut self, zoom: f32) {
        self.zoom = zoom;
        self.pan = (0.0, 0.0);
        self.fine = false;
        self.need_frame = true;
    }

    /// The palette: the theme's focus colour, then the fixed colours.
    fn colours(&self) -> [Rgba<u8>; 6] {
        let mut colours = [rgb(self.theme.accent); 6];
        colours[1..].copy_from_slice(&PALETTE);
        colours
    }

    fn current_colour(&self) -> Rgba<u8> {
        self.colours()[self.colour]
    }

    fn set_colour(&mut self, colour: usize) {
        self.colour = colour;
        self.need_toolbar = true;
        self.need_preview = true;
    }

    /// The picker shows text sizes for the text tool and strokes otherwise.
    fn picker(&self) -> usize {
        if self.tool == b't' {
            self.text_size
        } else {
            self.stroke
        }
    }

    fn set_picker(&mut self, value: usize) {
        if self.tool == b't' {
            self.text_size = value;
        } else {
            self.stroke = value;
        }
        self.need_toolbar = true;
        self.need_preview = true;
    }

    fn set_tool(&mut self, tool: u8) {
        self.commit_text();
        self.tool = tool;
        self.need_toolbar = true;
    }

    fn save(&mut self) -> Result<Flow, String> {
        self.commit_text();
        let path = self
            .out
            .save
            .map(Path::to_path_buf)
            .unwrap_or_else(default_path);
        atomic_save(self.editor.image(), &path)?;
        let copied = copy_to_clipboard(&path).is_ok();
        println!("{}", path.display());
        if self.out.notify {
            notify(&path.to_string_lossy(), copied);
        }
        Ok(Flow::Close)
    }

    fn copy(&mut self) -> Result<Flow, String> {
        self.commit_text();
        let temp = self.out.temp;
        self.editor
            .image()
            .save(temp)
            .map_err(|e| format!("save {}: {e}", temp.display()))?;
        copy_to_clipboard(temp)?;
        println!("{}", temp.display());
        if self.out.notify {
            notify(&temp.to_string_lossy(), true);
        }
        Ok(Flow::Continue)
    }

    /// Copy the text in the image as it is now (crops and masks applied),
    /// with tesseract. The review stays open.
    fn copy_text(&mut self) {
        self.commit_text();
        let config = ocr::load_config();
        let setup = match ocr::setup(&config, None) {
            Ok(setup) => setup,
            Err(e) => {
                eprintln!("nirushot: {e}");
                if self.out.notify {
                    let body = e.split_once('\n').map(|x| x.1).unwrap_or(&e);
                    ocr::notify_plain("OCR needs tesseract", body);
                }
                return;
            }
        };
        match ocr::copy_text_from(self.editor.image(), &setup, &config, None, self.out.notify) {
            Ok(Some(text)) => println!("{text}"),
            Ok(None) => eprintln!("nirushot: no text found"),
            Err(e) => {
                eprintln!("nirushot: {e}");
                if self.out.notify {
                    ocr::notify_plain("OCR failed", &e);
                }
            }
        }
    }

    fn share(&mut self) -> Result<Flow, String> {
        self.commit_text();
        let state = env::var_os("XDG_STATE_HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| {
                env::var_os("HOME")
                    .map(PathBuf::from)
                    .unwrap_or_else(|| PathBuf::from("."))
                    .join(".local/state")
            })
            .join("nirushare");
        std::fs::create_dir_all(&state).map_err(|e| format!("create {}: {e}", state.display()))?;
        let path = state.join(format!(
            "review-{}.png",
            chrono::Local::now().format("%Y%m%d-%H%M%S-%3f")
        ));
        self.editor
            .image()
            .save(&path)
            .map_err(|e| format!("save {}: {e}", path.display()))?;
        Command::new("nirulauncher")
            .arg("--share")
            .env("NIRUSHARE_SEND_FILE", &path)
            .spawn()
            .map_err(|e| format!("start NIRU Share: {e}"))?;
        Ok(Flow::Close)
    }

    fn focus(&self) {
        let _ = self
            .c
            .set_input_focus(InputFocus::PARENT, self.win, x11rb::CURRENT_TIME);
    }

    fn handle(&mut self, event: Event) -> Result<Flow, String> {
        let win = self.win;
        match event {
            Event::Expose(e) if e.window == win => {
                self.copy_to_window((e.x as i32, e.y as i32, e.width as i32, e.height as i32))?;
                if !self.drawn.is_empty() {
                    self.need_preview = true;
                }
            }
            // No keyboard or pointer grab: an active grab would stop NIRULOCK
            // (or any locker) from taking input while a review is left open.
            // Drags still get X's implicit button grab, which ends on release.
            Event::MapNotify(e) if e.window == win => self.focus(),
            Event::ClientMessage(e)
                if e.window == win
                    && e.type_ == self.wm_protocols
                    && e.data.as_data32()[0] == self.wm_delete =>
            {
                return Ok(Flow::Close);
            }
            Event::ClientMessage(e) if e.window == win && e.type_ == self.refine_atom => {
                if self.refine.is_current(e.data.as_data32()[0]) && !self.fine {
                    self.fine = true;
                    self.need_frame = true;
                }
            }
            Event::ConfigureNotify(e) if e.window == win => {
                if e.width as u32 != self.win_w || e.height as u32 != self.win_h {
                    self.win_w = e.width.max(320) as u32;
                    self.win_h = e.height.max(TOOLBAR as u16 + 80) as u32;
                    let _ = self.c.free_pixmap(self.frame.pixmap);
                    self.frame = Frame::new(&self.c, win, self.depth, self.win_w, self.win_h)?;
                    self.fine = false;
                    self.need_frame = true;
                }
            }
            Event::ButtonPress(e) if e.event == win => self.button_press(&e),
            Event::MotionNotify(e) => {
                let at = (e.event_x as i32, e.event_y as i32);
                if let Some((sx, sy, px, py)) = self.pan_drag {
                    self.pan = (px + (at.0 - sx) as f32, py + (at.1 - sy) as f32);
                    self.fine = false;
                    self.need_frame = true;
                } else if let Some((from, _)) = self.drag {
                    self.drag = Some((from, at));
                    self.need_preview = true;
                }
            }
            Event::ButtonRelease(e) if e.event == win => self.button_release(&e),
            Event::KeyPress(e) if e.event == win => return self.key_press(&e),
            Event::KeyRelease(e) if e.event == win => self.key_release(&e)?,
            Event::MappingNotify(_) => self.keymap = Keymap::load(&self.c),
            Event::Error(e) => eprintln!("nirushot: X11 error: {e:?}"),
            _ => {}
        }
        Ok(Flow::Continue)
    }

    fn button_press(&mut self, e: &ButtonPressEvent) {
        self.focus();
        let at = (e.event_x as i32, e.event_y as i32);
        match e.detail {
            4 => return self.set_zoom((self.zoom * 1.25).min(8.0)),
            5 => return self.set_zoom((self.zoom / 1.25).max(0.1)),
            2 => {
                self.pan_drag = Some((at.0, at.1, self.pan.0, self.pan.1));
                return;
            }
            1 => {}
            _ => return,
        }
        if at.1 < TOOLBAR as i32 {
            let hit = self.font.as_ref().and_then(|f| toolbar_hit(f, at.0));
            match hit {
                Some(Item::Tool(tool)) => self.set_tool(tool),
                Some(Item::Size(size)) => self.set_picker(size),
                Some(Item::Colour(colour)) => self.set_colour(colour),
                Some(Item::Undo) => {
                    self.commit_text();
                    self.undo();
                }
                Some(Item::Ocr) => self.copy_text(),
                None => {}
            }
            return;
        }
        if self.space_held {
            self.pan_drag = Some((at.0, at.1, self.pan.0, self.pan.1));
            return;
        }
        if self.tool == b't' {
            self.commit_text();
            if self.font.is_some() {
                let dims = self.editor.image().dimensions();
                let (ix, iy) = self.layout.image_point(dims, at.0, at.1);
                let half = TEXT_SIZES[self.text_size] as i32 / 2;
                self.typing = Some(Typing {
                    at: (ix, iy - half),
                    text: String::new(),
                });
                self.need_preview = true;
            }
            return;
        }
        if self.tool == b'0' {
            self.pan_drag = Some((at.0, at.1, self.pan.0, self.pan.1));
        } else {
            self.drag = Some((at, at));
            self.need_preview = true;
        }
    }

    fn button_release(&mut self, e: &ButtonReleaseEvent) {
        if e.detail == 2 || self.pan_drag.is_some() {
            self.pan_drag = None;
        }
        if e.detail != 1 {
            return;
        }
        let Some((from, to)) = self.drag.take() else {
            return;
        };
        self.need_preview = true;
        let dims = self.editor.image().dimensions();
        let start = self.layout.image_point(dims, from.0, from.1);
        let end = self.layout.image_point(dims, to.0, to.1);
        let (colour, stroke) = (self.current_colour(), STROKES[self.stroke]);
        if let Some(op) = op_from_drag(self.tool, start, end, dims, colour, stroke) {
            self.edit(|editor| Some(editor.push(op)));
        }
    }

    fn key_release(&mut self, e: &KeyReleaseEvent) -> Result<(), String> {
        if keys::shortcut(self.keymap.symbol(e.detail, e.state)) != 0x20 {
            return Ok(());
        }
        // Auto-repeat sends a release immediately followed by a press with
        // the same time; that is still a held key.
        if self.pending.is_none() {
            self.pending = self.c.poll_for_event().map_err(Self::err)?;
        }
        if let Some(Event::KeyPress(next)) = &self.pending {
            if next.detail == e.detail && next.time == e.time {
                self.pending = None;
                return Ok(());
            }
        }
        self.space_held = false;
        Ok(())
    }

    fn key_press(&mut self, e: &KeyPressEvent) -> Result<Flow, String> {
        let sym = self.keymap.symbol(e.detail, e.state);
        let key = keys::shortcut(sym);
        let ctrl = e.state.contains(KeyButMask::CONTROL);
        let shift = e.state.contains(KeyButMask::SHIFT);
        if let Some(typing) = self.typing.as_mut() {
            if !ctrl {
                match sym {
                    0xff1b => self.typing = None,
                    0xff0d | 0xff8d => self.commit_text(),
                    0xff08 => {
                        typing.text.pop();
                    }
                    _ => {
                        if let Some(ch) = keysym_char(sym) {
                            typing.text.push(ch);
                        }
                    }
                }
                self.need_preview = true;
                return Ok(Flow::Continue);
            }
            // Ctrl shortcuts act on the finished text.
            self.commit_text();
        }
        if ctrl {
            match key {
                0x7a if shift => self.redo(),
                0x7a => self.undo(),
                0x79 => self.redo(),
                0x63 => return self.copy(),
                0x73 => return self.save(),
                _ => {}
            }
            return Ok(Flow::Continue);
        }
        match key {
            0xff1b => {
                if self.drag.take().is_some() {
                    self.need_preview = true;
                } else {
                    return Ok(Flow::Close);
                }
            }
            0x63 => self.set_tool(b'c'),
            0x72 => self.set_tool(b'r'),
            0x61 => self.set_tool(b'a'),
            0x6d => self.set_tool(b'm'),
            0x74 if self.font.is_some() => self.set_tool(b't'),
            0x68 => self.set_tool(b'h'),
            0x31..=0x36 => self.set_colour((key - 0x31) as usize),
            0xffff | 0xff9f => self.edit(Editor::delete_last),
            0x20 => self.space_held = true,
            0x77 => self.set_picker((self.picker() + 1) % STROKES.len()),
            0x76 => self.set_tool(b'0'),
            0x2b | 0x3d | 0xffab => self.set_zoom((self.zoom * 1.25).min(8.0)),
            0x2d | 0xffad => self.set_zoom((self.zoom / 1.25).max(0.1)),
            0x30 | 0x66 => self.set_zoom(1.0),
            0x73 | 0xff0d => return self.save(),
            0x79 => return self.copy(),
            0x75 => return self.share(),
            0x6f => self.copy_text(),
            _ => {}
        }
        Ok(Flow::Continue)
    }
}
