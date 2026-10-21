# NIRUSHOT

Small on-demand screenshot capture helper for the NIRU X11 desktop.

```sh
nirushot --region          # copy by default
nirushot --window
nirushot --full
nirushot --region --delay 3
nirushot --region --copy
nirushot --full --save ~/Pictures/shot.png --no-notify
```

Uses `slop` for region selection, `maim` for capture, `xclip` for the X11
clipboard, `xprop` for the active window and `notify-send`/NIRUNOTIFY for the
completion notification. No daemon, upload, history or background process.

Default behavior copies to the clipboard; `--save PATH` writes only that file
unless `--copy` is also given. The default filename under `~/Pictures` and the
native review surface are next-delivery work. Cancel is reported without
creating a file. Requires NIRUX.

MIT. Copyright 2026 Nicklas Rudolfsson.
