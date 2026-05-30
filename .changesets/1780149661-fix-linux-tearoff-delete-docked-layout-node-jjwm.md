---
type: patch
---

fix(linux): tear-off — delete docked layout node after IPC succeeds

Phase A left a P1 from reagent's review on PR #1188: the win32 and
darwin tear-off paths both call `treeReducer(DeleteNode)` after a
successful `open_floating_pane_window` so the pane doesn't render
twice (once in the source tab, once in the floater), and the linux
file omitted that step. `TearOffBlock` moves the block server-side
but the source layout's local tree still references the layout node
until SolidJS reconciles a new tree — so the docked pane stays
visible.

Fix: port the same `getLayoutModelForStaticTab` → `DeleteNode` block
from `.darwin.tsx` (lines 226-236) into the linux file's success
branch, and change the error branch to `return` instead of falling
through (matching darwin/win32 — if the IPC failed there is nothing
to clean up locally, and we don't want to delete the layout node when
the user can still see the docked pane).

Also adds the matching imports
(`getLayoutModelForStaticTab`, `LayoutTreeActionType`,
`LayoutTreeDeleteNodeAction`).
