---
type: patch
---

fix(tabs): commit-on-release tab tear-off + loosen reorder threshold + kill drag circle-slash (Windows)

Three Windows tab drag-and-drop fixes. (1) Reorder now thresholds on each remaining tab's center instead of the gap midpoint, so crossing one neighbour commits the move (was ~1.5–2 tab-widths / grab-position-dependent). (2) A window-level dragover listener paints the copy/"plus" cursor in the tear-off zone instead of the no-drop circle-slash. (3) Dragging a tab down over the window no longer tears mid-drag via SC_MOVE — the tear commits on release, matching the drag-up/away direction and the "don't tear until I release" expectation.
