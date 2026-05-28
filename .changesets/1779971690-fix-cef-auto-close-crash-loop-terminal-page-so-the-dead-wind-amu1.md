---
type: patch
---

fix(cef): auto-close crash-loop terminal page so the dead window releases its instance number

When `on_render_process_terminated` trips the crash budget (added in
the prior PR), the resulting "Window stopped recovering" page sits
indefinitely waiting for the user to click Quit. During that wait
the dead browser remains in the host's window registry, so the
launcher's instance counter still includes it — UI shows "2 windows"
even though only 1 is real, which was one of the user-visible
symptoms of the 2026-05-28 incident.

Add a 30s countdown to the terminal page that calls `window.close()`
when it expires. `window.close()` triggers the existing
`on_before_close` path, which sends `ReportWindowClosed` to the
launcher, which emits `Event::WindowInstanceReleased`, which
decrements the user-visible window count. No new IPC protocol or
host state needed — this just reuses the close-cleanup chain that
already exists for normal window closes.

Any keystroke or mouse-down on the page cancels the auto-close
(hidden via `display: none` on the countdown line), so the message
stays readable as long as the user wants. Single-shot listeners
(`{ once: true }`) avoid leaking handlers if the user interacts
multiple times.

Closes one item of #1117. Depends on the crash-budget PR for the
page to be reachable in the first place.
