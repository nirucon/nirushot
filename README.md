# NIRUSHOT

On-demand screenshot capture, review and annotation, a screen colour picker
and copying text from the screen (OCR), for the NIRUX X11 desktop.

**Status: beta** · Part of the [NIRUX](https://github.com/nirucon/nirux) suite · [MIT License](LICENSE)

![NIRUSHOT](docs/screenshots/main.png)

## About

NIRUSHOT takes a screenshot of a region, a window or the whole screen, puts it
on the clipboard straight away and opens a small review window where you can
crop, annotate, mask and share it before saving. `nirushot pick` freezes the
screen and picks a colour from it with a magnifying loupe, and `nirushot ocr`
freezes it and copies the text in a region you select. There is no daemon, upload
service or background process: it runs when you press the key and exits when
you are done.

## Features

- Region (`slop`), active window or full-screen capture with `maim`, with an
  optional delay.
- The original capture is **always** copied to the clipboard before review
  opens; Cancel leaves no file, but the capture stays pasteable.
- Review and annotation: crop, rectangle, arrow, opaque black mask, highlight
  (everything outside is dimmed to 50 %) and one-line text.
- Non-destructive editing: every edit is an operation over the untouched
  capture, with undo/redo (last 100 steps).
- Six colours (theme focus colour, red, yellow, green, white, black) and
  thin/medium/thick strokes; S/M/L text sizes in the theme font.
- Zoom and pan that stay smooth on large captures.
- Save, copy or share with [NIRUSHARE](https://github.com/nirucon/nirushare):
  to your own devices, or to a Discord channel through a webhook you set up
  (NIRUSHARE 0.4.0 and NIRU Launcher 0.4.0), always after a preview.
- Notification with Open, Copy and Show in folder actions after saving.
- Colour picker (`nirushot pick`): freezes every monitor, shows an 11×11 px
  loupe magnified 10× with a crosshair, moves by pixel from the keyboard,
  copies `#rrggbb`, `rgb()` or `hsl()`, and keeps the last ten picks for the
  NIRU Launcher colours view.
- Copy text from the screen (`nirushot ocr`): freeze, select a region and the
  text is on the clipboard, read by `tesseract` in Swedish and English. The
  notification shows the first line and offers Copy again and Undo. The review
  window has the same thing as `o` (Copy text).
- Plays the NIRUX `screenshot` sound through NIRUNOTIFY when it runs.
- Follows the active NIRUX theme and configured monospace font; keys follow
  the active keyboard layout (Shift, AltGr, Caps Lock, Num Lock).

## Requirements

- X11 (Debian, Arch and Void glibc, x86_64).
- `maim` (capture), `slop` (region selection), `xprop` (active window),
  `xclip` (clipboard).
- Optional: `notify-send` (notifications) and `xdg-open` (notification
  actions).
- Optional, for `nirushot ocr`: `tesseract` with Swedish and English data.

| Distribution | Packages |
|---|---|
| Debian | `tesseract-ocr tesseract-ocr-swe tesseract-ocr-eng` |
| Arch | `tesseract tesseract-data-swe tesseract-data-eng` |
| Void | `tesseract-ocr tesseract-ocr-swe tesseract-ocr-eng` |

NIRUX installs all of these, including tesseract with Swedish and English
data (since NIRUX 0.12.245). Without tesseract, `nirushot ocr` shows the
install command for your distribution.

## Installation

### With NIRUX

NIRUSHOT is a core component: `nirux install` and `nirux update` install and
update it as part of the pinned suite generation. The standalone installer
refuses to touch a NIRUX-managed copy; use `nirux update` instead.

### Standalone

```sh
./install.sh            # download the pinned release binary, verify SHA256SUMS, install to ~/.local/bin
./install.sh --build    # build from this checkout with cargo and install it
```

`NIRUSHOT_PREFIX` changes the prefix (default `~/.local`).

## Usage

```sh
nirushot --region                         # select a region (default)
nirushot --window                         # the active window
nirushot --full                           # all monitors
nirushot --region --delay 3               # wait three seconds first
nirushot --region --save ~/Pictures/shot.png
nirushot --full --copy                    # clipboard only, no review
nirushot --full --save shot.png --review  # save to that path from the review
nirushot --no-notify                      # no notification after saving
nirushot pick                             # pick a colour from the screen
nirushot pick --format hsl                # start in hsl() instead of #rrggbb
nirushot ocr                              # copy the text in a region
nirushot ocr --check                      # is tesseract there, which languages
```

Without `--copy` or `--save` the capture opens in review. `--save PATH` writes
that file, `--copy` skips review, and `--review` opens review anyway. Saving or
copying an edited review replaces the clipboard with the edited image.

Default NIRUWM bindings:

| Key | Action |
|---|---|
| `Print` | `nirushot --region` |
| `Super+Print` | `nirushot --full` |
| `Super+Shift+Print` | `nirushot --window` |
| `Super+Ctrl+C` | `nirushot pick` |
| `Super+Ctrl+O` | `nirushot ocr` |

In NIRUX, `nirux manual screenshots` covers capture, review, the colour
picker and OCR.

### Review controls

| Key | Action |
|---|---|
| `c`, then drag | Crop |
| `r`, then drag | Rectangle |
| `a`, then drag | Arrow |
| `m`, then drag | Opaque black mask |
| `h`, then drag | Highlight (several highlights share one dimming; annotations are never dimmed) |
| `t`, then click | Type one line of text; `Enter` places it, `Escape` cancels it |
| `1` to `6` or the swatches | Colour for rectangles, arrows and text |
| `w` or the picker | Thin/medium/thick stroke (S/M/L size with the text tool) |
| `v` | View/pan mode; drag the image |
| Middle-drag, or hold `Space` and drag | Pan with any tool |
| Mouse wheel, `+`/`=`/`-` | Zoom |
| `0` or `f` | Fit |
| `Ctrl+Z` or Undo | Undo |
| `Ctrl+Shift+Z`, `Ctrl+Y` | Redo |
| `Delete` | Remove the last annotation (undoable) |
| `s`, `Ctrl+S` or `Enter` | Save to `~/Pictures/Screenshot-YYYY-MM-DD_HH-MM-SS.png` and copy |
| `y` or `Ctrl+C` | Copy without saving |
| `o` or Copy text | Copy the text in the image as shown, after crops and masks (OCR); the review stays open |
| `u` | Share with NIRUSHARE: opens the launcher's Share view with this screenshot (devices, and Discord webhooks with a preview first) |
| `Escape` or the close button | Discard (`Escape` first cancels a drag in progress) |

If a file with the default name already exists, `-2`, `-3` and so on are
appended.

### How editing works

The captured image is never changed. Every crop, rectangle, arrow, mask,
highlight and text is an operation in an ordered list, in the coordinates of
the image it applies to (a crop starts new coordinates for what follows). What
the window shows, and what Save, Copy and Share export, is that list drawn over
the untouched capture. Undo and redo step through earlier lists, so a wrong crop
is one `Ctrl+Z` away. The history stores operations, not image copies.

The mask is always opaque black, never a blur or pixelation, because those can
leak what is underneath.

### The review window

The region selector uses a thick, opaque amber border so it stays visible on
light and dark wallpapers. The review window is a centred floating X11 dialog,
so NIRUWM does not tile it. It opens on the monitor that holds the captured
region (or the pointer), sized for that monitor.

The window takes keyboard focus but holds **no keyboard or pointer grab**, so
the screen lock (NIRULOCK, or any X locker) can always take over while a review
is open; the review stays open underneath and keeps its edits. Only `slop`'s
short-lived region selection grabs input.

### Colour picker

`nirushot pick` freezes the screen the moment it starts: what you see while
picking is a still frame of every monitor, so menus, tooltips and video hold
still. A loupe next to the pointer shows the 11×11 pixels around it at 10×,
with a crosshair on the pixel that will be picked and its value below.

| Key | Action |
|---|---|
| Move the mouse | Move the crosshair |
| Arrow keys | Move one pixel |
| `Shift`+arrow keys | Move ten pixels |
| `Tab` / `Shift+Tab` | Cycle the format: `#rrggbb`, `rgb(r, g, b)`, `hsl(h, s%, l%)` |
| Left click, `Enter` or `Space` | Pick |
| `Escape` or right click | Cancel |

The picked value, in the format shown, is printed, copied to the clipboard and
shown in a notification (with a swatch image where the notification daemon
shows images; NIRUNOTIFY shows the text). The colour is added, as hex, to the
history in `~/.local/state/nirushot/colors.json`, which NIRU Launcher shows as
its colours view.

Options: `--format hex|rgb|hsl` (the starting format), `--no-copy`,
`--no-notify` and `--no-history`. The exit status is 0 after a pick, 1 when
cancelled and 2 on an error.

While it is open the picker grabs the keyboard and pointer, like the region
selector. It never opens over a screen lock (it exits if it cannot grab), and
it cancels itself after 60 seconds without input, so it cannot keep the idle
lock out.

### Copy text (OCR)

`nirushot ocr` freezes the screen the way the colour picker does, then you
drag over a region with the same amber selector as a region capture
(`Escape` or a right click cancels). The region is turned grey, made
dark text on a light background if the theme is dark, scaled up three times
(less for very large regions) and read by `tesseract`. The text is printed,
copied to the clipboard and shown in a notification with the first line and
the character count. Where the notification daemon supports actions, **Copy
again** puts the text back on the clipboard and **Undo** restores the text the
clipboard held before.

Languages default to `swe+eng`. If only some of the configured languages are
installed, the rest are used and the notification (and `nirushot ocr --check`)
names the missing data and the package to install. If none is installed,
English or the first installed language is used.

Options: `--languages swe+eng`, `--psm N` (tesseract's page segmentation, 0
to 13), `--no-notify`, `--geometry WxH+X+Y` (a region without selecting, for
scripts) and `--image FILE.png` (read a file, no screen). `--check` prints the
tesseract version, the installed and chosen languages and what is missing.
The exit status is 0 when text was copied, 1 when cancelled or no text was
found (the clipboard is left alone) and 2 on an error, such as tesseract not
installed.

Nothing leaves the machine: the region is written to a private temporary
directory for tesseract and removed straight after.

## Configuration

NIRUSHOT follows the active NIRUX theme and monospace font. The only file of
its own is the optional `~/.config/nirushot/config.toml`, for OCR:

```toml
[ocr]
languages = "swe+eng"   # tesseract language codes joined with +
psm = 3                 # page segmentation; 6 suits one block, 7 one line
keep_newlines = true    # false joins everything into one line
```

All keys are optional. The capture sound is switched in
`~/.config/nirux/sounds.toml` (`nirux sounds toggle screenshot`), and key
bindings live in the NIRUWM configuration.

## Files and paths

| Path | Purpose |
|---|---|
| `~/.local/bin/nirushot` | The program |
| `~/Pictures/Screenshot-*.png` | Default save location |
| Private temporary directory (`nirushot-*`) | The capture while it is reviewed; removed on exit |
| `~/.local/state/nirushot/colors.json` | The last ten picked colours, newest first |
| `~/.config/nirushot/config.toml` | Optional `[ocr]` settings |
| Private temporary directory (`nirushot-ocr-*`) | The prepared OCR region while tesseract reads it |
| `~/.cache/nirushot/swatch-*.png` | Swatch images for pick notifications (only colours still in the history) |

## Troubleshooting

- **No notification actions:** the installed `notify-send` does not support
  `--action`; a plain notification is shown instead.
- **No sound:** the sound comes from NIRUNOTIFY; check that it runs and that
  `screenshot` is on in `nirux sounds status`.
- **`nirushot pick` exits with "cannot grab":** another program (a screen
  lock, or a menu that holds the keyboard) has the input; close it and try
  again.
- **`nirushot ocr` says tesseract is not installed:** install the packages
  in the table under Requirements, then check with `nirushot ocr --check`. On
  Void the program is `tesseract-ocr` (Void's `tesseract` is a game), and
  NIRUSHOT only runs that name there.
- **OCR reads Swedish letters wrong:** `nirushot ocr --check` says whether
  `swe` is installed; without it åäö come out as a, o or worse.
- **OCR misses small or styled text:** select tighter around the text, or set
  `psm = 6` (one block) or `psm = 7` (one line) in the config.
- **Region selection does nothing:** check that `slop` is installed; pressing
  `Escape` in the selector cancels without a capture.

## Uninstall

With NIRUX, NIRUSHOT is removed by `nirux uninstall`. A standalone copy:

```sh
./install.sh --uninstall
```

## Building from source

```sh
cargo test --locked
cargo build --release --locked
sh tests/cli-smoke.sh target/release/nirushot
```

The smoke test uses fake capture and notification providers and needs no
graphical session. CI also runs clippy with `-D warnings`, shellcheck and an
Xvfb review test (`cargo test --release --locked --test xvfb -- --ignored`).
The OCR fixtures (rendered English and Swedish text in three themes and three
sizes, read back by the real tesseract) run with
`cargo test --release --locked --bin nirushot ocr -- --ignored` and need
`tesseract-ocr`, `tesseract-ocr-eng` and `tesseract-ocr-swe`.

## Status, disclaimer and how it's made

**Status: beta.** I use it for every screenshot I take; the annotation tools
are deliberately few.

NIRUX and every niru* project are made by me, Nicklas Rudolfsson
([@nirucon](https://github.com/nirucon)), for me. They are tailored to my own
needs, preferences and workflows, on my own machines. If they fit yours too,
that's a nice bonus.

- **Free and open source.** Anyone is free to use, study, change and share
  it under the [MIT License](LICENSE).
- **No guarantees.** There is no warranty of any kind. Use it at your own
  risk, read what you install and keep backups of anything that matters.
- **How it's made.** Some of these projects I built from scratch by hand. In
  more recent releases I have used Claude, Anthropic's AI, to do much of the
  heavy lifting under my direction: I decide what to build and how it should
  behave, and I test it on my own machines. It saves me a lot of time, and
  the result is quite OK.
- **Alpha, beta, dev, and maybe forever.** Most things here are alpha, beta
  or dev, and may stay that way. Maybe one day there will be a real stable
  release, but given how fast development moves, and how I am as a person,
  maybe never :)

| Label | Meaning |
|---|---|
| dev | Under active construction; may not build, install or work yet. |
| alpha | Works on my machines, but incomplete or rough; expect breaking changes. |
| beta | In daily use on my machines and mostly complete; rough edges remain. |

## License

MIT License, see [LICENSE](LICENSE). Copyright (c) 2026 Nicklas Rudolfsson.
