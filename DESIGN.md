# nirushot design

On-demand X11 screenshot helper for NIRUX. No daemon, upload, history or
background process. Capture uses maim; region selection uses slop; clipboard
uses xclip; active-window id uses xprop; completion uses NIRUNOTIFY-compatible
notify-send.

First delivery: region/window/full, delay, copy/save, cancel, notification.
Next delivery: native theme-aware review surface with crop, rectangle/arrow
annotation and opaque mask; save edited copy without overwriting the original.
Helpers have deadlines, output caps and no shell strings. Multi-output geometry,
cancel and write failures must be tested. NIRUX pins the checksummed release,
installs maim/slop/xclip, updates bindings after the fallback period and covers
doctor/rollback/uninstall.
