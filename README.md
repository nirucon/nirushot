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
- `s` or `Enter`: save to `~/Pictures/Screenshot-YYYY-MM-DD_HH-MM-SS.png`
  and copy it to the clipboard
- `y`: copy the current image without saving
- `Esc`: close without keeping a file

The review window is a normal managed X11 window with a themed toolbar and
typography from NIRUWM: Niru Noir, Nord, Satie and Carl Larsson palettes are
honoured, including the configured monospace font. The capture pixels are
unchanged; crop, rectangle, arrow and mask are applied only when you save or copy.
Closing with `Esc` removes the temporary capture.

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
