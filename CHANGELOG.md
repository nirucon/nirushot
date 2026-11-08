# Changelog

All notable changes to NIRUSHOT. Releases before 0.6.0 are described in
their GitHub release notes.

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
