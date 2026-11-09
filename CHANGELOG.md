# Changelog

All notable changes to NIRUSHOT. Releases before 0.6.0 are described in
their GitHub release notes.

## 0.7.0 (2026-11-09)

### Added

- `nirushot ocr`: copy the text in a screen region.
  - The screen is frozen like `nirushot pick` (one server-side copy of every
    monitor), and the region is selected with the same `slop` selector as a
    region capture. The pixels come from the frozen frame.
  - The region is converted to grey, inverted when the background is dark,
    scaled up three times (less for regions over 24 MP after scaling) and
    padded, then read by `tesseract`. On the fixtures this took character
    accuracy from 98.3 % (unprocessed) to 100 %.
  - The text is printed and copied to the clipboard with `xclip`, like
    `pick`. The notification shows the first line and the character count,
    with **Copy again** and **Undo** (restore the previous clipboard text)
    where the daemon supports actions.
  - Languages default to `swe+eng`, using whichever of them is installed; the
    notification and `nirushot ocr --check` name the missing data and its
    package.
  - tesseract is optional. Without it `nirushot ocr` notifies with the install
    command for Debian, Arch or Void, detected the way `nirux` does. On Void
    only `tesseract-ocr` is run, because Void's `tesseract` is a game.
  - Options: `--check`, `--languages`, `--psm`, `--geometry WxH+X+Y`,
    `--image FILE`, `--no-notify`. Exit status 0 copied, 1 cancelled or no
    text (clipboard unchanged), 2 error.
  - Optional `~/.config/nirushot/config.toml` with `[ocr]` `languages`,
    `psm` and `keep_newlines`.
- Review window: `o` or the **Copy text** button copies the text of the image
  as shown, after crops and masks. The review stays open.
- Tests: unit tests for the config, languages, distro detection, install
  hints, text cleanup and preparation; OCR fixtures of rendered English and
  Swedish text (åäö) in three themes and three sizes, read back by the real
  tesseract; Xvfb tests for selecting a frozen region with `slop`, Undo and
  Copy again, an empty region, Escape, and the review's `o`; smoke tests
  with a fake tesseract. CI installs tesseract with Swedish and English data.

### Fixed

- Escape or a right click in the region selector is a cancel (exit 0 with
  `cancelled`), not "selection failed".
- `install.sh` installs the current release (it still pointed at 0.5.1).

## 0.6.0 (2026-11-08)

### Added

- `nirushot pick`, a freeze-frame colour picker.
  - One server-side copy freezes every monitor, rotated ones included, the
    moment it starts. On othala (2×2560x1440, one inverted) the loupe is up
    about 11–14 ms after launch.
  - A loupe of 11×11 px at 10× follows the pointer, with a crosshair and the
    value. It stays on the pointer's monitor.
  - Arrow keys move one pixel and Shift+arrow ten. Tab and Shift+Tab cycle
    `#rrggbb`, `rgb()` and `hsl()`. Click, Enter or Space picks; Escape or
    right click cancels.
  - The value is printed, copied to the clipboard and shown in a notification
    with a swatch image for daemons that show images.
  - Each pick is added to `~/.local/state/nirushot/colors.json`: a JSON array,
    newest first, of `{"hex":"#rrggbb","time":<unix seconds>}`, at most ten,
    with a repeated colour moved to the top. The file is replaced atomically.
  - Options: `--format`, `--no-copy`, `--no-notify`, `--no-history`. Exit
    status 0 picked, 1 cancelled, 2 error.
  - The input grab exists only while the picker is open. The picker exits
    without showing anything if it cannot grab (for example under a screen
    lock), and it cancels after 60 seconds without input.
- Xvfb tests for the picker: freeze, keyboard moves, formats, clipboard,
  notification, history order, Escape, and refusing to open under a held
  grab.

### Changed

- The Xvfb test server runs with `-noreset`, so a test client connecting
  right after the readiness probe is no longer refused.
