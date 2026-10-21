# NIRUSHOT

On-demand screenshot capture, review and annotation for the NIRU X11 desktop.
No daemon, upload service or background process.

Part of the [NIRUX suite](https://github.com/nirucon/nirux): [NIRUWM](https://github.com/nirucon/niruwm) · [NIRUBAR](https://github.com/nirucon/nirubar) · [NIRUSCREENSAVER](https://github.com/nirucon/niruscreensaver) · [NIRULOCK](https://github.com/nirucon/nirulock) · [NIRULAUNCHER](https://github.com/nirucon/nirulauncher) · [NIRUNOTIFY](https://github.com/nirucon/nirunotify) · [NIRUSHOT](https://github.com/nirucon/nirushot).

## Usage

```sh
nirushot --region
nirushot --window
nirushot --full
nirushot --region --delay 3
nirushot --region --save ~/Pictures/shot.png
nirushot --full --copy
```

Without `--copy` or `--save`, the capture opens in a review dialog. `--save`
writes the chosen file; `--copy` skips review and copies directly. Cancel leaves
no file.

## Review controls

- `c` then drag: crop
- `r` then drag: red rectangle
- `a` then drag: red arrow
- `m` then drag: opaque black mask
- `v`: view/pan mode; drag the image or middle-drag to pan
- Mouse wheel or `+`/`-`: zoom; `0`: fit
- `s` or Enter: save to `~/Pictures/Screenshot-YYYY-MM-DD_HH-MM-SS.png` and copy
- `y`: copy without saving
- Escape or the close button: discard

The review window is a centred floating X11 dialog, so NIRUWM does not tile it.
The toolbar and typography follow the active NIRUWM theme and configured
monospace font. Pointer and keyboard grabs exist only while the dialog is open.
The original capture is never overwritten; crop, rectangle, arrow and mask are
applied only in the exported result.

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
