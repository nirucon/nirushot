# nirushot

NIRU screenshot capture and review helper. It is intentionally on-demand: no
daemon, no upload service and no background process.

## Usage

```sh
nirushot --region          # select, review, then save/copy
nirushot --full            # full screen, then review
nirushot --window          # active window, then review
nirushot --region --save ~/Pictures/shot.png --no-notify
nirushot --full --copy     # skip review and copy directly
```

Default interaction:

- `c` then drag: crop
- `r` then drag: red rectangle
- `a` then drag: red arrow
- `m` then drag: opaque black mask
- `v`: view/pan mode; drag the image or middle-drag to pan
- Mouse wheel, `+`/`-`: zoom around the image centre; `0`: fit
- `+`/`-` or mouse wheel: zoom; `0`: fit
- `s` or `Enter`: save to `~/Pictures/Screenshot-YYYY-MM-DD_HH-MM-SS.png`
  and copy it to the clipboard
- `y`: copy the current image without saving
- `Esc`: close without keeping a file

The review window is a centred floating X11 dialog, so NIRUWM does not tile it.
It has a themed toolbar and typography: Niru Noir, Nord, Satie and Carl Larsson
palettes are honoured, including the configured monospace font. The capture
pixels are unchanged; crop, rectangle, arrow and mask are applied only when you
save or copy. The pointer and keyboard are grabbed only for the lifetime of the
dialog, so `s`, `y`, `Esc` and toolbar clicks work regardless of the previous
focus. Closing with `Esc` or the window close button removes the temporary
capture.

`--copy` and `--save PATH` skip the review window. `--review` forces it.
`--delay SECONDS` waits before capture. `--no-notify` suppresses the notification.
Saved screenshots use a NIRU notification with Open, Copy and Show in folder
actions when the installed `notify-send` supports actions; otherwise a plain
notification is shown. No notification service is started by nirushot.

## Runtime

- `maim` for capture
- `slop` for region selection
- `xprop` for active-window lookup
- `xclip` for clipboard
- `notify-send` (optional) for the completion notification

## Build

```sh
cargo test --locked
cargo build --release --locked
sh tests/cli-smoke.sh target/release/nirushot
```

The smoke test uses fake capture/notification providers and does not need a
graphical session.
