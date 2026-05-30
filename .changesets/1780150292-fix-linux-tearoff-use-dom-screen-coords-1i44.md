---
type: patch
---

fix(linux): tear-off — use DOM screen coords instead of get_cursor_point

reagent CHANGES_REQUESTED on PR #1188 caught a P1: the
`agentmux-cef` host command `get_cursor_point` is a Windows-only
`GetCursorPos` wrapper — on non-Windows builds (Linux, macOS) it
returns `{x:0,y:0}` (drag.rs:211-212). The linux dragend handler was
threading those zeros into `open_floating_pane_window` as the
floater's top-left, so every Linux pane tear-off opened pinned to the
screen's top-left corner instead of the drop point.

The `.darwin.tsx` sibling already solved this by reading
`DragEvent.screenX/screenY` straight out of the DOM event (top-left
origin, CSS px = DIP — exactly what CEF Views positioning expects).
Port that fix verbatim to the linux file:

- `handleDragEnd` captures `dropX = e.screenX`, `dropY = e.screenY`
  before the 50ms settle.
- `handleCrossWindowDragEnd` signature grows `dropX, dropY` params;
  the `get_cursor_point` invoke is gone.
- `cursorPoint` is now `{x: dropX, y: dropY}` — same shape, correct
  source.

Also clears the stale P2 doc-comment at `floating-pane-workspace.tsx`
header: clarified that on Linux the JS-driven drag fires
`start_window_drag` (compositor-driven) rather than the
`get/set_window_position` polling used on Windows/macOS.

After this commit the only remaining `get_cursor_point` invoke is in
the win32 sibling, where it is correct.
