---
type: patch
---

fix(linux): floating-pane header drag — use compositor IPC instead of polling

After the Phase A chromeless floater landed, the floater appeared at the
drop point but its header could not be dragged. The existing handler in
`floating-pane-workspace.tsx` is polling-based: on `mousedown` it reads
`get_window_position`, then on each `mousemove` it calls
`set_window_position`. That round-trip is correct on Windows
(SetWindowPos in physical px) and on macOS (CEF Views `set_bounds`),
but on Wayland the compositor forbids client-driven top-level
repositioning, so `set_bounds` is a no-op for position. IPCs fire at
~20ms cadence — the window never moves.

Fix: branch on Linux at the top of the header `mousedown` handler and
fire a single `start_window_drag` IPC, mirroring the main window's
`useWindowDrag.linux.ts`. That routes through the patched CEF
`BeginWindowDrag` → `WmMoveResizeHandler::DispatchHostWindowDragMovement`
→ `xdg_toplevel.move`, handing the drag to Mutter until mouseup. No
polling, no `set_bounds`. Same IPC the main title bar already uses — no
new backend surface.

Windows and macOS branches unchanged.
