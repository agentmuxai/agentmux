# Spec — Drag and drop a minimized pane, with a ghost the size of its chip

**Date:** 2026-10-20
**Author:** Maricon@charlie
**Type:** Spec (a missing feature and a drag-image fix)
**Status:** proposed
**Decisions applied:** D1 to D4 as recommended (the operator, 2026-10-20: *"go"*).
**Requested by:** the operator: *"we also want to add a missing feature, you should be able to dnd minimized panes. the ghost would be the size of the minimized pane (currently dragging a minimized pane will have a ghost the size of what appears to be the restored pane."*
**Related:** `SPEC_LAYOUT_MINIMIZE_LOCKED_STATE_REDESIGN_2026_07_16.md` (where minimized panes were locked against moves; §8 made minimize a display mode), `SPEC_PANE_CONTROLS_TILED_RESTORE_ICON_AND_MINIMIZE_FROM_MAXIMIZED_2026_10_12.md` (minimize from maximized), `SPEC_DRAG_AND_DROP_CONSOLIDATION_2026_09_27.md`, `SPEC_PANE_DRAG_TO_TAB_2026_07_10.md` (cross-tab drops), `SPEC_NATIVE_POINTER_DRAG_TEAROFF_2026_07_28.md` (tear-off).

## 1. Summary

A minimized pane (a header chip) can be picked up today, but the drop does nothing, and the image you hold is a full-size pane. After this change a chip can be dragged like any pane: to another place in the tab (the pane's edges and the layout's edges), onto another pane to swap with it, to another tab, or out of the window. It stays a chip where it lands, except where it would be the only pane left showing content. The ghost is the chip as it looks.

## 2. What happens today

Code read on main at 2026-10-20.

- **The drag starts.** The whole-pane `draggable()` is registered on the pane header (`frontend/layout/lib/TileLayout.core.tsx`). A chip *is* its header, and `canDrag` only refuses ephemeral and maximized panes, so a chip can be picked up.
- **The drop targets are offered.** `computeMoveNode` (`layoutTree.ts`) has no minimize check, so the drop indicators appear as for any pane.
- **The drop is refused, silently.** `moveNode` rejects a source or destination that is effectively minimized (`layoutTree.ts`, "moveNode rejected: source or destination is minimize-locked", a console warning only), and so do `swapNode`, `resizeNode`, `splitHorizontal` and `splitVertical`. srv refuses the same intents (`crates/srv/src/backend/layout/intent.rs`: `move_node`, `swap_nodes`). The user sees the pane go back to where it was.
- **The ghost is the wrong size.** The tile keeps a rasterised ghost (`toPng` of its hidden `.tile-preview`, sized from the tile's rect by `computeDragPreviewSize`, cached with that size) and redraws it on a resting hover or on a press (`createDragPreviewIntent`). Redrawing is asynchronous (about 170 ms). A pane minimized since its last ghost still holds the full-size image, and a drag that starts before the new one is ready uses it: a ghost the size of the pane before it was minimized. `onGenerateDragPreview` uses whatever image is cached without comparing its size with the tile's size now. The same race exists for an expanded pane that was just resized, but the old and new sizes are usually close enough not to be noticed.

**Why minimized panes were locked.** The lock comes from the July redesign (`SPEC_LAYOUT_MINIMIZE_LOCKED_STATE_REDESIGN_2026_07_16.md`), when minimizing still squeezed a pane's stored size and could dissolve a column into a neighbour: a moved or swapped chip carried a size that meant something else in its new parent. Since that spec's §8 minimize is a display mode: only the `minimized` flag is set, the stored `size` is untouched, and the chip's geometry is derived at render (`layoutGeometry.ts`). Moving a leaf with the flag set moves an ordinary leaf, so those reasons no longer apply to a chip as the thing being moved. They still apply to *splitting* a chip (a full pane would appear inside a header strip) and to *resizing* one.

## 3. The change

### A. The ghost is the chip

1. **Use the chip's own header as the drag image.** For a minimized pane, `onGenerateDragPreview` calls `nativeSetDragImage(headerElement, x, y)` with the chip's live header element (the element the drag was started on), with the cursor offset at the point grabbed. No rasterisation: the image is exactly the chip on screen, at its size and zoom, and is ready at once. (HTML5 drag allows a live element as the drag image; the browser snapshots it at drag start.)
2. **Never use a stale image.** For every pane, `onGenerateDragPreview` uses the cached image only if its size equals `computeDragPreviewSize` of the tile's rect *now*. Otherwise it falls back to the pane's header element, as for a chip, instead of a wrong-sized picture. A minimize or restore also invalidates the cached image (the tile's size changes, so the next hover redraws it).
3. **Native Wayland is unchanged.** `hideDragImageOnNativeWayland` still returns first (a tear-off's release outside the window would otherwise slide the picture back).

### B. A chip can be moved and swapped within the tab

4. **Move.** `moveNode` (frontend) and `move_node` (srv) accept an effectively minimized **source leaf**. The leaf keeps its `minimized` flag, so it lands as a chip. Like any pane moved to another parent it takes the default size there (`DEFAULT_NODE_SIZE`; a move within its parent keeps its size), so it restores later to an even share of its new parent. The **destination** rule changes to match the display-mode model: inserting into a fully-minimized branch (a column of chips) is allowed when the moved leaf is itself minimized (the branch stays all chips) and refused when it is expanded (an expanded pane would be laid out inside a header strip). A minimized *branch* as the source stays refused (only leaves are dragged).
5. **Swap.** Dropping a chip on the centre of an expanded pane swaps them, and each keeps its own flag: the chip takes the expanded pane's slot as a chip, and the expanded pane takes the chip's slot expanded. `swapNode` and `swap_nodes` accept minimized endpoints. A swap never changes how many panes are expanded, so the "one pane always shows content" rule cannot break.
6. **No new geometry.** The chip's position and size in its new slot come from the existing derivation (the header strip of its column, or a chip in its row, with the slip rules).
7. **Indicators.** The drop indicators shown for a chip are the ones that will be accepted: `computeMoveNode` returns no move for a destination the rule in 4 refuses, so the user never sees an indicator that then does nothing. Splitting stays refused, so a chip offers no split targets.

### C. Leaving the tab

8. **To another tab** (`crossTabDrag.ts`, `SPEC_PANE_DRAG_TO_TAB_2026_07_10.md`): the pane arrives as a chip if the target tab has an expanded pane, and **restored** if it would otherwise be the only pane, or if every pane in the target tab is minimized. srv builds a new pane in the target tab (`RedockFloatingPane`, `server/service/tear_off.rs`), so it reads the flag before the source leaf goes, then minimizes the placed pane (`minimize_block_in_layout`, `server/service/layout_intents.rs`); the `minimize` intent's own rule refuses that when the pane would be the only one showing content, which leaves it restored. The window's mirror applies a queued `minimize` action (`layoutPersistence.ts`). Dropped as a tab of another pane, the pane joins that pane's state.
9. **Torn off to a window** (tear-off, `SPEC_NATIVE_POINTER_DRAG_TEAROFF_2026_07_28.md`): a floating window cannot be a chip, so the pane is restored as it leaves. The tear-off already builds a new pane in the new window, without the flag, so this needs no change; the tear-off's own sizing applies.
10. **What stays behind.** Dragging a chip out of a tab never leaves that tab with no expanded pane: a chip is not an expanded pane, so the count does not change.

## 4. Decisions for the operator

| # | Question | Recommended | Alternative |
|---|---|---|---|
| D1 | A chip dropped somewhere in the tab: stays a chip, or restores? | **Stays a chip** (a move doesn't change what the pane shows; a click restores it) | Restore on drop |
| D2 | Chip dropped on an expanded pane's centre | **Swap, each keeps its own state** | Swap and restore the chip; or no swap target for a chip |
| D3 | Drop a chip *onto another chip* (a column of chips as the target) | **Allowed** (it joins the strip, still a chip) | Refused in the first version |
| D4 | Ghost for an expanded pane whose cached image is stale | **The pane's header element** (always right, never late) | Wait for the new image (adds latency to the drag start) |

## 5. Code touched

- `frontend/layout/lib/TileLayout.core.tsx`: `onGenerateDragPreview` (header element for a chip; size check for every pane).
- `frontend/layout/lib/layoutTree.ts`: `moveNode`, `swapNode`, `computeMoveNode` (the destination rule of 4, no indicator for a refused target).
- `crates/srv/src/backend/layout/intent.rs`: `move_node`, `swap_nodes` (the same rules; srv stays the authority, `SPEC_STRONG_REDUCER_AUTHORITY_LAYOUT_2026_06_30.md`).
- `frontend/layout/lib/layoutIntentActions.ts` vectors and their Rust counterpart, so both sides agree case by case.
- `frontend/layout/lib/crossTabDrag.ts` and the tear-off path: restore where §3 C says.
- `SPEC_LAYOUT_MINIMIZE_LOCKED_STATE_REDESIGN_2026_07_16.md` I2: note that a minimized leaf may now be moved and swapped (still not split or resized).

## 6. Tests

Frontend (`frontend/layout/tests/`):
1. `moveNode` of a minimized leaf to another parent: the leaf is there, still `minimized`, with its stored `size` unchanged; the doctor (`validateLayoutInvariants`) reports nothing.
2. `moveNode` of an expanded leaf into a column of chips is refused, and `computeMoveNode` offers no move for it; a minimized leaf into the same column is accepted.
3. `swapNode` of a chip and an expanded pane: positions exchanged, flags kept, expanded count unchanged.
4. The random layout stress test (local, `layoutStress.test.ts`) gains "move" and swap-with-a-chip steps; every step keeps the doctor clean and at least one pane expanded.
5. Ghost: for a minimized tile `onGenerateDragPreview` passes the header element; for an expanded tile with a cached image of another size it passes the header element too, and with a matching size the cached image.

srv (`crates/srv/src/backend/layout/`):
6. `move_node` and `swap_nodes` with minimized sources, the same cases as 1 to 3, and the shared intent vectors pass on both sides.

Cross-tab and tear-off:
7. A chip dragged to a tab whose panes are all minimized arrives restored; to a tab with an expanded pane, as a chip.
8. A torn-off chip opens as a normal window, not minimized.

## 7. Manual checks

1. Three panes in a tab, minimize one. Drag the chip by its header to the other column's edge: the ghost is the chip's size; on release it is a chip in the new column, and the column's other pane is unchanged. Click it: it restores to a normal size there.
2. Drag the chip onto the centre of an expanded pane: they swap, the chip stays a chip.
3. Minimize a pane and drag it within a quarter of a second: the ghost is the chip, never the old full-size picture.
4. Resize a pane and drag it at once: the ghost has the new shape (or is the header), never the old one.
5. Drag a chip to another tab and out of the window, and check §3 C.

## 8. Out of scope

- Splitting or resizing a chip (both stay refused).
- A keyboard way to move a pane (none exists for expanded panes either).
- Changing the ghost of an expanded pane beyond the stale-image fallback.
