---
type: patch
---

fix(layout): resize border no longer triggers pane tear-off

**Root cause:** The resize handle (6 px, centered at the pane boundary) overlaps
1.5 px with the top of the adjacent pane's header. In that overlap zone the
resize handle wins via `z-index: 3` vs the header's `z-index: auto`, but under
WebView2 timing edge cases (layout recomputing during a reactive update,
subpixel rounding at the boundary) the header occasionally received the
`mousedown` instead — starting a pragmatic-dnd drag that set
`_currentDragPayload`, which the `CrossWindowDragMonitor` then interpreted as a
tear-off request.

**Fixes applied (both in `TileLayout.win32.tsx`):**

1. `ResizeHandle.onPointerDown` now calls `event.preventDefault()` before
   `setPointerCapture`. `preventDefault` on `pointerdown` suppresses the
   subsequent `mousedown` event; since HTML5 drag-and-drop requires `mousedown`
   to start, no drag can initiate from a press on the resize handle even if an
   underlying element would otherwise react.

2. `DisplayNode.canDrag` now rejects drags whose initial pointer position
   (`input.clientX/Y`) falls within the ±halfSize zone of any resize handle's
   `centerPx` (converted to display-container-local coordinates). This is
   defense-in-depth for the race: if the pointer barely misses the handle
   element and lands on a header pixel near the border, the drag is cancelled
   before `onDragStart` ever sets `_currentDragPayload`.

Tear-off remains fully functional when dragging from inside the pane header
away from the border zone.
