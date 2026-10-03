# SPEC: resize through collapsed panes — every edge of a collapsed run resizes the expanded panes either side

**Date:** 2026-10-03
**Status:** implemented — 2026-10-03, PR #4260 (Phase C keeps a Column's chain across chip slots; `extendResizeHandlesThroughChips` in `layoutGeometry.ts` adds the chip-edge handles after the whole tree is laid out; tests in `layoutModel.test.ts` "resize through collapsed panes"). See §7.
**Author:** AgentY, at the owner's request
**Amends:** `SPEC_LAYOUT_MINIMIZE_LOCKED_STATE_REDESIGN_2026_07_16.md` invariant I3 (§5 below).
**Related:** `SPEC_PANE_MINIMIZE_AND_TOOLCALL_FAILCOLLAPSE_2026_06_21.md`, `SPEC_PANE_MINIMIZE_REFINEMENTS_2026_06_24.md` (docking a collapsed pane onto a neighbour), `docs/analysis/ANALYSIS_PANE_MINIMIZE_ROW_BRANCH_DISTORTIONS_2026_08_30.md` §3 and PR #2855 (the Row-parent version of this problem, fixed), `SPEC_SHIFT_DRAG_GROUP_RESIZE_2026_08_03.md` and its `_DIRECTION_FIX_2026_08_17` follow-up, `SPEC_RESIZE_DEFAULT_FLIP_AND_WINDOW_EDGE_SHIFT_2026_08_26.md`.

## 1. The request (owner, 2026-10-03)

> When there is one or more collapsed panes between 2 expanded panes, all the borders can be used for resize. Currently we have a situation with 3 panes, vertically stacked, with the center collapsed. If I want to resize the top/bottom pane, I need to hover over the bottom border of the top pane. I also want to be able to hover-drag to resize the top of the bottom pane. These two borders are also the top and bottom respectively of the center collapsed pane. If there were multiple collapsed panes in the center, each one of their top and bottom borders could be used to resize the top and bottom of the expanded panes in their column.

Stated as a rule: in a column, for a run of one or more collapsed panes between two expanded panes **A** (above) and **B** (below), **every** horizontal edge in the run (A's bottom/first chip's top, each chip-to-chip edge, last chip's bottom/B's top) is a resize handle, and dragging any of them resizes A against B. The collapsed panes keep their fixed height and move with the drag.

## 2. How it behaves today

A collapsed (minimized) pane is a display-mode flag on its layout leaf, `minimized: true` (`frontend/layout/lib/types.ts:208`); its stored `size` is untouched. Its extent is fixed and recomputed every layout pass (`collapsedExtentPx`, `layoutGeometry.ts:62-73`): a header strip, `HeaderHeightPx` (33) + gap, in a Column parent. In a Row parent it does not keep a slot: it **docks** onto the nearest expanded sibling (right preferred) with zero width, and its header chip is stacked on top of that sibling, pushing the sibling's top down (`resolveRowSlipTargets`, `layoutGeometry.ts:201-224`; Phase B, 430-481).

Resize handles are generated per parent, between consecutive rendered siblings of that parent only, in Phase C of `updateTreeHelper` (`layoutGeometry.ts:483-575`). Collapsed children are treated two ways (`layoutGeometry.ts:506-513`):

- a docked (Row) child has zero extent and is **spanned**: `A | B(collapsed) | C` gets one handle between A and C (`afterIndex` 2, PR #2855);
- a Column child keeps a real chip slot, so it **resets the chain**: no handle on either of its edges, because "a handle spanning it would float on top of the chip".

The drag side refuses any handle that flanks a collapsed node (`layoutResize.ts:315-318`), the reducer rejects resizing one (`layoutTree.ts:435-443`), and group resize leaves collapsed siblings out of its pool (`layoutResize.ts:323-325`). All of this follows invariant I3 of the 07-16 redesign: *"A locked edge presents no affordance: no resize handle, no resize cursor."*

### 2.1 Which layouts produce the owner's symptom

Two trees render as "three stacked panes, the middle one collapsed":

| Tree | Handles today |
|---|---|
| **(a)** `Column[Top, Mid(collapsed), Bottom]` | **none**: Mid resets the chain, so neither of its edges has a handle; Top and Bottom can only be resized from elsewhere. A test pins this: `layoutMinimize.test.ts:722-729` expects `[]`. |
| **(b)** `Column[Top, Row[Mid(collapsed), Bottom]]` | **one**, at Top's bottom edge: Mid docks onto Bottom inside the Row and its chip is drawn full-width above Bottom. The outer Column pairs Top with the Row branch (not collapsed, since Bottom is expanded), so that handle sits at the chip's top and resizes Top against the branch, which is Top against Bottom. The chip's bottom edge lies inside the Row, along the Row's *cross* axis; Phase C only makes main-axis handles and the Row has one rendered child, so that edge has none. |

The owner's report ("the bottom border of the top pane works, the top of the bottom pane does not") matches (b). Tree (b) arises when Mid and Bottom were side by side and Mid was collapsed: `balanceNode` (`layoutNode.ts:216-254`) does not merge that single-rendered-child Row into the Column. Both trees are in scope; which one the owner's window has was not confirmed (open question 1).

## 3. Required behaviour

R1. **Every edge of a collapsed run is a handle.** In a Column, for a maximal run of k ≥ 1 collapsed slots between expanded siblings A and B (rendered order), there are k + 1 handles: A's bottom edge, each edge between two collapsed slots, and B's top edge. This holds whether the collapsed panes are Column children with their own slots (tree a) or chips docked on top of B (tree b), or a mix.

R2. **Each of those handles resizes A against B.** Plain drag (group mode, the default since 2026-08-26) and Shift-drag (direct mode) behave exactly as they would on a handle between A and B with nothing collapsed in between: same pair, same 128 px floor (`MinNodeSizePx`), same group scope (the parent's expanded siblings). The collapsed panes' heights never change; they move with the boundary.

R3. **No jump.** Grabbing any handle in the run and moving the pointer by d moves the boundary by d. (Already true by construction: `onResizeMove` measures from the grabbed handle's own `centerPx`, `layoutResize.ts:332`, so each handle needs only its own correct `centerPx`.)

R4. **A run at the edge of the column has no handles on its own edges.** If the collapsed run has no expanded sibling above (or below), there is nothing to trade against, so those edges stay inert, as today. (A run at the top of a nested column still gets the parent's handles where the parent's rules give them.)

R5. **Collapsing never moves a handle off a pane.** The edge A shared with B before the middle pane was collapsed is still draggable afterwards, at A's bottom.

R6. **The chip stays clickable.** Handles straddle an edge by half the handle size each way (`resizeHandleSizePx / 2`, `layoutGeometry.ts:552`), so on a 33 px chip the top and bottom few pixels become resize zones. The rest of the chip keeps its click (restore) and context-menu behaviour. If the handle size ever makes the two zones meet on a chip, the chip's middle must stay a click target (e.g. clamp each zone to at most a third of the chip).

R7. **Rows unchanged.** In a Row parent collapsed panes already dock with zero width and the run is spanned by one handle (PR #2855); nothing changes there.

## 4. Design

### 4.1 Tree (a): a Column's own collapsed slots

In Phase C, instead of resetting the chain at a collapsed child, remember the run:

1. Walk children in order. On an expanded child, if a run of collapsed slots is pending and there is an expanded `beforeIdx`, emit one handle per boundary of the run, else the ordinary handle.
2. Each emitted handle: `parentIndex = beforeIdx`, `afterIndex = i` (A and B, both expanded, so the drag guard at `layoutResize.ts:315-318` and the reducer accept it unchanged), `centerPx` = that boundary's main-axis position from the final rects (A's bottom; each chip's bottom; B's top), perpendicular span = intersection of A's and B's rects (as today).
3. Ids must be distinct per boundary so `onResizeMove` rebuilds its context per handle (`resizeContext.handleId`): keep `${node.id}-${beforeIdx}` for the first boundary (A's bottom, unchanged) and use `${node.id}-${beforeIdx}-${n}` for the n-th edge below it.
4. A run with no expanded `beforeIdx`, or ending at the last child, emits nothing (R4).

### 4.2 Tree (b): chips docked on top of the after-side pane

Phase B already knows, for each dock target, the stack of chips it draws above the target and how far it pushed the target's top down. In Phase C of a **Column** parent, after emitting the handle between A and the after-side child B (which may be a branch whose top-most rendered descendant carries a chip stack), emit one more handle per docked chip edge at B's top: same `parentIndex`/`afterIndex` as A–B, `centerPx` at each chip's bottom edge, perpendicular span = that chip's width intersected with A's. Requires Phase B to expose, per node, the chip stack drawn at its top edge (offsets and widths), propagated up from the dock target to the branch that is the Column's child. Chips stack only on top of a target, so only the after side needs this.

The same pass covers a mix: Column-slot chips (4.1) followed by chips docked on B (4.2) form one run, every edge a handle on A–B.

### 4.3 Drag, group mode, reducer

No change in `onResizeMove`, `computeGroupResizeSizes` or `resizeNode`: every new handle names the expanded A and B. Group mode's driven node remains `afterNodeId` (B) and collapsed siblings stay out of its pool. The resize dimension overlay (`SPEC_PANE_RESIZE_DIMENSION_OVERLAY_2026_05_26.md`) shows A's and B's sizes as for any A–B handle.

## 5. Amendment to invariant I3

07-16 I3 reads: *"A locked edge presents no affordance: no resize handle, no resize cursor."* Its purpose was that a collapsed pane can never be resized by dragging (bug class 2 of `RESEARCH_PANE_MINIMIZE_BEST_PRACTICES_2026_07_16.md`). Amended:

> **I3 (amended 2026-10-03).** A collapsed pane's own size is never the subject of a resize: no handle resizes it, and the drag guard and reducer refuse any action whose target is collapsed. An edge of a collapsed pane that lies between two expanded panes of the same column **is** a handle for those two panes (this spec); any other collapsed edge presents no affordance.

The drag guard and reducer checks stay as they are; they still protect against a stale handle that names a collapsed node.

## 6. Tests

Through the real `updateTree` (the existing `layoutMinimize.test.ts:669-737` block re-implements the Phase C loop; the new cases must not):

1. Tree (a), one collapsed: 2 handles, both `parentIndex` = Top, `afterIndex` = Bottom, at Top's bottom and Bottom's top, distinct ids. Replaces the `[]` expectation at `layoutMinimize.test.ts:722-729`.
2. Tree (a), three collapsed in a row: 4 handles, `centerPx` at each boundary, strictly increasing, all naming Top and Bottom.
3. Tree (b): 2 handles (chip top, chip bottom), both resizing Top against the Row branch; the second's `centerPx` is the chip's bottom (= Bottom's pushed-down top).
4. A run at the top or bottom of the column: no handles on its edges (R4).
5. Dragging the lowest handle by +40 px gives the same sizes as dragging the topmost by +40 px, in both group and direct mode, and the collapsed panes' rendered heights are unchanged (R2, R3).
6. A stale handle naming a collapsed node is still refused (`layoutResize.ts:315-318`, `layoutTree.ts:435-443`).
7. The chip's click target: with the default handle size, a pointer at the chip's vertical middle is not inside any handle rect (R6).
8. Row parent `A | B(collapsed) | C`: unchanged, one spanning handle (R7).

## 7. As built

- **Phase C** (`layoutGeometry.ts`): in a Column a chip slot no longer resets the pairing chain, so the expanded panes above and below a run get their handle at the upper pane's bottom edge (`parentIndex`/`afterIndex` = those two panes). A Row still resets (R7).
- **`extendResizeHandlesThroughChips`**, called once from `updateTree` after the walk, when every rect is final. That includes chips docked inside a nested Row, which Phase B lays out only after the outer Column's handles exist, so 4.2 is a post-pass rather than a Phase B export. For each vertical-stack handle it walks down through collapsed leaves whose rect starts at the current edge (0.5 px tolerance) and overlaps the handle's span. It adds a handle at each chip's bottom with the same pair, id `${id}-${n}` and span narrowed to the chip. One pass covers §4.1, §4.2, a mix, and the chips of a fully-minimized branch.
- **R6:** an added handle's zone is `min(resizeHandleSizePx / 2, chipHeight / 3)` each side. The original handle at the upper pane's bottom keeps its usual size.
- **No change** to `onResizeMove`, group resize or the reducer.
- **Tests (§6):** 1–5 and 8 are in `layoutModel.test.ts` through the real `updateTree`. 7 checks a 40 px handle (tile gap 20) is clamped on a 53 px chip. 6 is the existing guard and reducer tests, unchanged. The mirrored pairing rule in `layoutMinimize.test.ts` now expects `[[0, 2]]` for a Column chip (was `[]`).
- **Not verified by hand in a running build yet.**

## 8. Open questions

1. Which tree the owner's window has. Both are in scope, so this decides only which case to verify first by hand. A quick way to tell: in tree (b) the collapsed chip is exactly as wide as the bottom pane; in tree (a) it spans the column.
2. Should a run at the edge of a column (R4) instead resize the column against its own parent's neighbour (a cross-level handle)? Not proposed: Phase C never crosses levels today, and the window-edge rules (`SPEC_RESIZE_DEFAULT_FLIP_AND_WINDOW_EDGE_SHIFT_2026_08_26.md`) already cover the outer edges.
