# Agent assignment: NIRUSHOT 0.5.0, a one-shot correctness, performance and editing update

## 0. Quick start

1. Read this file, `README.md`, `src/main.rs` and `tests/cli-smoke.sh`. The audit baseline
   is `c0bba67` (0.4.10, pinned by NIRUX 0.12.199). Revalidate each item against current
   HEAD and skip items that are already fixed, noting that you did.
2. Implement **H1–H7, then E1–E4 in order**. Commit per item. Keep `cargo fmt --check`,
   `clippy -D warnings`, tests and `tests/cli-smoke.sh` green.
3. Pass the gates in §4, release in §5 and report in §6.

This is an **execution** assignment. Do not stop after a plan or a branch. Make sensible
decisions, document them, and continue without routine questions.

## 1. Invariants

- No daemon, no upload and no network. Capture → clipboard → optional review stays the
  flow, and `--copy`/`--save` keep their meaning.
- The original capture is never overwritten. The black mask stays truly opaque: never
  replace it with blur or pixelation, because those can leak content.
- The runtime tools stay `maim`, `slop`, `xprop`, `xclip`, `notify-send` and `xdg-open`.
  Add no new required runtime dependency.
- Keep `rust-version = 1.85`. Locked builds must work on rustc 1.85, and `image` stays
  pinned below 0.25.10 (see the Cargo.toml comment).

## 2. Bugs (verified 2026-11-04 on niruxwork, in a separate Xvfb)

### H1: The open review dialog prevents the screen from locking (high, security)
`review()` (`src/main.rs` ~700) takes an active **keyboard and pointer grab** when the
window maps, and holds it for as long as the dialog is open. With a review open, starting
NIRULOCK failed: `nirulock: cannot grab pointer: ALREADY_GRABBED`, exit 1. A user who
leaves a screenshot open and walks away does not get an idle lock.
**Fix:** remove both grabs from the review dialog. It is a normal focused floating
dialog, and drags inside it already get X's implicit button grab. Keep focus with
`set_input_focus` on map and on click. Only `slop`'s selection phase grabs, and it is
short-lived.
**Done when:** an x11 test opens a review, starts a test locker that grabs pointer and
keyboard, and the grab succeeds. Also verify natively with the real NIRULOCK on an
authorized host.

### H2: Dragging re-scales the whole image on every mouse event (high, lag)
Each loop iteration composes the canvas with `imageops::resize(..., Lanczos3)` of the
full image and uploads it with `put_image`, including on every `MotionNotify`. On a 4K
capture, a 60-step rectangle drag cost **3.1 s of CPU**.
**Fix:**
- Cache the scaled view image per (image generation, zoom, window size).
- Draw the live drag preview (rectangle, arrow, crop frame) over the cached view only.
- Coalesce queued `MotionNotify` events and handle only the latest.
- Use a fast filter (Triangle) while zooming or panning, and Lanczos3 once things are idle.
- Upload only the changed region where practical.

**Done when:** the same 4K 60-step drag uses ≤ 0.3 s CPU, measured under Xvfb with
xdotool (add the test), and the exported image is pixel-identical to the old path for the
same operations.

### H3: No undo
Crop, rectangle, arrow and mask change `image` destructively. A wrong crop can only be
undone by discarding everything with Esc. **Fix:** keep an operation list (non-destructive)
and render it on export, or keep a bounded snapshot stack (≤ 20 steps, memory-capped).
Ctrl+Z undoes and Ctrl+Shift+Z/Ctrl+Y redoes. Show "Undo" in the toolbar.

### H4: The dialog straddles monitors
The window size and position come from the **root** size (`sw`/`sh`). On two side-by-side
monitors it is centered on the seam. That needs confirming on othala (2×2560x1440).
**Fix:** use RandR to center on the monitor that contains the pointer, or the captured
region, and size the window from that monitor.

### H5: One keyboard round trip per key, and only level 0
`key_name()` calls `get_keyboard_mapping` per key and reads only the first keysym, so `+`
on layouts where it is shifted, and Ctrl combinations, do not behave. **Fix:** cache the
map, refresh it on `MappingNotify`, and use the level/Caps helpers from
`nirucon/nirulauncher` `src/main.rs` (`keyboard_level`, `keyboard_symbol`). Handle Ctrl
explicitly for the new shortcuts in E2.

### H6: A generated file is committed
`SHA256SUMS` in the repo root refers to a local build. **Fix:** remove it, ignore it in
`.gitignore`, and confirm that the release workflow generates its own checksums.

### H7: The README contradicts the code
The README says "crop, rectangle, arrow and mask are applied only in the exported result",
but they change the in-session image destructively. Correct the README after H3, and
describe the undo and operation model.

## 3. Editing UX

### E1: Text annotation
A `t` tool: click to place, type a single line (AltGr and Caps via H5), Enter confirms and
Esc cancels. Use the theme font, a size picker (S/M/L), and the current color.

### E2: Color and stroke
A small palette: theme focus color, red, yellow, green, white and black. Cycle it with
`1`–`6` or click the swatches, with a thin/medium/thick stroke. It applies to the rectangle,
arrow and text. The mask is always black.

### E3: Standard shortcuts
Ctrl+C = copy (same as `y`), Ctrl+S = save (same as `s`), Delete = remove the last
annotation, and Space-drag = temporary pan. Update the toolbar hints and the README.

### E4: Highlight tool
An `h` tool dims everything outside the dragged rectangle to 50 %, for pointing at an
area. It is drawn only in the export, the same way as the other operations.

Out of scope: upload or sharing changes, OCR, scrolling capture and video.

## 4. Gates

Automated:
- `cargo fmt --check`, `cargo clippy --locked --all-targets -- -D warnings`,
  `cargo test --locked`, a release build, and `sh tests/cli-smoke.sh`.
- An MSRV job in CI (`dtolnay/rust-toolchain@1.85`, `cargo build --locked`).
- New unit tests: undo/redo of every operation, export equivalence, the operation list
  when cropping after annotating, text rendering bounds, and choosing the monitor for H4
  (pure geometry).
- An Xvfb test, ignored by default and run in CI. Use fake `maim`/`xclip` on PATH, or a
  generated 3840x2160 PNG.
  - Open the review and run the H2 drag budget.
  - The H1 grab test: a tiny test client that tries `GrabPointer` and `GrabKeyboard` must
    get `Success`.
  - The E3 shortcuts.

Native:
- On **niruxwork** (the owner allows local testing), capture a region and the full screen,
  then annotate, undo, save and copy.
- Paste into another app and confirm that it is the edited image.
- Check the notification actions.
- Remove the test screenshots from `~/Pictures` afterwards.
- On **othala** (authorized for full testing): check H4 placement on two monitors, and that
  NIRULOCK locks while a review is open (H1).

## 5. Release

Bump to **0.5.0**. Tag `v0.5.0` through the normal workflow and verify the assets and
checksums. Then pin it in `nirucon/nirux` `manifest`:
- If `nirux/tools/suite-pin` exists, use it.
- Otherwise bump the manifest the usual way. Change only the nirushot line, and rebase if
  other pins land at the same time.

Install through `nirux update` and verify that the running binary comes from the new
generation.

## 6. Final report

For each item H1–H7 and E1–E4: the change, the tests and the evidence. Include CPU for the
4K drag before and after, the result of the lock-while-review test, and the othala
placement. Add release URLs, installed versions and remaining limits. Mark anything
unfinished as **PARTIAL** and give the exact remaining steps.
