# NIRUSHOT

**Part of [NIRUX - A minimal X11 desktop by Nicklas Rudolfsson](https://github.com/nirucon/nirux).**

**Early beta.** On-demand screenshot capture, review and annotation for the NIRUX X11 desktop.
No daemon, upload service or background process.


## Usage

```sh
nirushot --region
nirushot --window
nirushot --full
nirushot --region --delay 3
nirushot --region --save ~/Pictures/shot.png
nirushot --full --copy
```

A capture is always copied to the clipboard. Without `--copy` or `--save`, it
also opens in a review dialog. `--save` writes the chosen file; `--copy` skips
review. Saving or copying an edited review replaces the clipboard with the edited
image. Cancel leaves no file, but the initial capture remains pasteable.

## Review controls

- `c` then drag: crop
- `r` then drag: rectangle
- `a` then drag: arrow
- `m` then drag: opaque black mask
- `t` then click: type one line of text; Enter places it, Escape cancels it.
  S/M/L in the toolbar pick the size; the theme font is used
- `1`–`6` or the swatches: colour for rectangles, arrows and text — the
  theme's focus colour, red (the default), yellow, green, white, black
- `w` or the picker: thin/medium/thick stroke (S/M/L text size with the text
  tool); medium is the stroke 0.4 always used
- `v`: view/pan mode; drag the image or middle-drag to pan
- Mouse wheel or `+`/`-`: zoom; `0`: fit
- Ctrl+Z or the Undo button: undo; Ctrl+Shift+Z or Ctrl+Y: redo
- `s` or Enter: save to `~/Pictures/Screenshot-YYYY-MM-DD_HH-MM-SS.png` and copy
- `y`: copy without saving
- `u`: share with NIRU Share
- The initial capture is already in the clipboard before review opens
- Escape or the close button: discard (Escape first cancels a drag in progress)

## How editing works

The captured image is never changed. Every crop, rectangle, arrow, mask
and text is an operation in an ordered list, in the coordinates of the image it
applies to (a crop starts new coordinates for what follows). What the dialog
shows, and what Save, Copy and Share export, is that list drawn over the
untouched capture. Undo and redo step through earlier lists, so a wrong crop
is one Ctrl+Z away and nothing has to be discarded. The history keeps the
last 100 steps; it stores operations, not image copies.

The mask is always opaque black, never a blur or pixelation, because those
can leak what is underneath.

## Window

The initial region selector uses a thick, opaque amber border so it remains
visible on light and dark wallpapers. The review window is a centred floating
X11 dialog, so NIRUWM does not tile it. It opens on the monitor that holds the
captured region (or the pointer), sized for that monitor, instead of across
the seam of a multi-monitor desktop.

The dialog takes keyboard focus but holds **no keyboard or pointer grab**, so
the screen lock (NIRULOCK, or any X locker) can always take over while a review
is open; the review stays open underneath and its edits are kept. Only `slop`'s
short-lived region selection grabs input.

Large captures stay smooth: the scaled view is cached, a drag preview is drawn
over it without re-scaling, a finished annotation re-scales only the pixels it
changed, and a new zoom is scaled quickly first and refined after a moment.
The toolbar and typography follow the active NIRUX theme and configured
monospace font. Keys follow the active layout, including Shift, AltGr, Caps
Lock and Num Lock.

Saved files use a NIRU notification with Open, Copy and Show in folder when the
installed `notify-send` supports actions; otherwise a plain notification is
shown. `--no-notify` suppresses it. NIRUSHOT does not start a notification
service.

## Runtime

- `maim` capture
- `slop` region selection
- `xprop` active-window lookup
- `xclip` clipboard
- `notify-send` optional notification
- `xdg-open` notification actions

NIRUX installs all of these.

## Build

```sh
cargo test --locked
cargo build --release --locked
sh tests/cli-smoke.sh target/release/nirushot
```

The smoke test uses fake capture and notification providers and does not need a
graphical session.

MIT. Copyright 2026 Nicklas Rudolfsson.

## NIRUX suite

[NIRUX](https://github.com/nirucon/nirux) · [NIRUWM](https://github.com/nirucon/niruwm) · [NIRUBAR](https://github.com/nirucon/nirubar) · [NIRUSCREENSAVER](https://github.com/nirucon/niruscreensaver) · [NIRULOCK](https://github.com/nirucon/nirulock) · [NIRULAUNCHER](https://github.com/nirucon/nirulauncher) · [NIRUNOTIFY](https://github.com/nirucon/nirunotify) · [NIRUSHOT](https://github.com/nirucon/nirushot)
