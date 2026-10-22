# SPEC: pane minimum sizes are a constraint of the layout geometry, not a check on one input

**Status:** draft — not built. Needs the operator's decision on §6 (when the minimums don't fit).
**Date:** 2026-10-19
**Author:** Korp@narko, at the operator's request
**Trigger (operator, 2026-10-18):** "the min-width is not respected when panes auto-resize during the window resize ... when the outer window is resized, the panes inside will contract to below the minimum allowed width (like you added for the browser pane) ... get this work in, write a spec to file on this issue, sounds like it may need architectural thought"
**Root cause:** `docs/reports/REPORT_PANE_MIN_WIDTH_NOT_ENFORCED_ON_WINDOW_RESIZE_RCA_2026_10_19.md`
**Builds on:** `SPEC_BROWSER_PANE_THIN_BAR_AND_MENUS_IN_PANE_2026_10_18.md` §5.5 (the 250px browser floor in the drag path, #5153), `SPEC_RESIZE_DEFAULT_FLIP_AND_WINDOW_EDGE_SHIFT_2026_08_26.md` (plain window resize stays proportional), the minimize display-mode design (`layoutMinimize.ts`: geometry derived at render time, stored sizes untouched).

---

## 1. Summary

1. **The problem.** The minimum size of a pane (128px; 250px wide for a browser pane) is only a check in the splitter drag. Window resize (and anything else that changes the container's pixels) turns the stored weights into pixels in proportion, with no floor, so panes shrink below their minimum (RCA §3: a browser pane in a 700px window gets 175px).
2. **The fix, in one line.** Make the floors an input of `computeMainAxisAllocation`, the one function that turns weights into pixels, and resolve them the way CSS flexbox resolves `min-width`: a pane whose proportional share is under its floor is held at the floor, and the others share what is left.
3. **Stored weights are not touched.** The floor is applied when geometry is derived, like minimize. Shrink the window and the panes hold their floors; grow it again and the original proportions come back. Nothing is written to the layout and nothing is persisted.
4. **One source for the floors.** A `layoutFloors` module (moved out of #5153's drag code) answers "how small may this node be, along this axis" for a pane, a group, and a stack. The drag path, the geometry and the Shift+window-edge path all call it.
5. **A drag starts from what you see.** If any pane in a group is being held at its floor, the drag first makes the weights equal to the displayed sizes (§5), so the splitter keeps following the pointer.
6. **When the floors don't fit** (more panes than the window can hold at their minimums) is a decision for the operator (§6). Recommended for the first version: shrink the floors in proportion, so geometry is always valid and nothing overflows the window.

## 2. Why not the obvious fixes

| Idea | Why not |
|---|---|
| Clamp the weights on every resize tick (a clamp on each input path). | Rewrites stored sizes during a window drag: lossy (growing the window again can't restore the old proportions), persisted, and it is the same per-path patch that left this hole. Each new input path would need it again. |
| CSS `min-width` on the pane tiles. | Tiles are placed by JS transforms from computed rects (`setTransform`); a sibling's left edge is computed from the previous pane's width, so a tile CSS can't grow without the next one overlapping it. The existing `min-width: 4rem` in `tilelayout.scss` is only a clip guard. |
| A minimum window size derived from the layout. | It depends on the tab's layout, windows hold several tabs, and an OS resize that stops dead is a worse experience than panes holding their minimum. |

## 3. The design

### 3.1 Floors are a function of the tree

`frontend/layout/lib/layoutFloors.ts` (new; `minExtentPx` and `MinBrowserPaneWidthPx` move here from `layoutResize.ts`):

```ts
/** The narrowest (Row axis) or shortest (Column axis) `node` may be, in CSS px. */
export function floorPx(node: LayoutNode, axis: FlexDirection, view: ViewOf): number;
```

- **A pane:** the floor of its view along the axis. Today 128px both ways, except 250px wide for a browser pane. A pane in a tab stack takes the largest floor of any tab in the stack (stable when you switch tabs: a pane never jumps size because a different tab became active).
- **A group** (a branch of the tree): along its own direction, its members' floors added up (minimized members take no room) plus the gaps between them; across it, the largest member's. (Until §3.2 lands, the splitter drag in #5153 can't use the sum: geometry still shares a group's pixels by weight, so it uses the largest `floor × total weight ÷ member weight`. With §3.2 the sum is exact, because a held member keeps its floor whatever the weights.)
- **Per view, declared near the view.** `PANE_FLOORS: Record<view, { w?: number; h?: number }>` in one file, default 128. The browser entry says why: its bar needs about 208px. A later view with its own needs (the agent pane's input row, say) adds a line.

### 3.2 The allocation

`computeMainAxisAllocation` gets `floors: (child) => number` (px along the branch's main axis; absent → 0, which is today's behaviour exactly). After minimized chips are placed (as now), with `P` the pixels left and the expanded, non-slip children `i` having weights `w_i` and floors `f_i`:

1. If `Σ f_i ≤ P` (it fits): start with every child unfrozen. Repeat: give each unfrozen child `px_i = w_i / Σ w_unfrozen × (P − Σ f_frozen)`; freeze every child with `px_i < f_i` at `f_i`; stop when none is under its floor. A frozen child never un-freezes in the same pass: holding a child at its floor takes pixels from the others, which can only push more of them under theirs, so freezing all violators at once reaches the same answer as freezing one at a time. At most `n` rounds.
2. If `Σ f_i > P` (it doesn't fit): §6.

`pixelToSizeRatio` (which the drag math uses) is computed over the unfrozen children only: `Σ w_unfrozen / (P − Σ f_frozen)`. With no frozen child it is today's value, so the normal case is unchanged.

Groups work by recursion: a child group's floor along its parent's axis is `floorPx(group)`; the group is then given its pixels and lays out its own children with their floors. A Column's cross-axis floor (the widest member) is therefore honoured by the Row above it.

### 3.3 Where the floors come from at render time

`updateTreeHelper` already has the `model`; it passes `floors = (child) => floorPx(child, axis, viewOf)` where `viewOf(blockId)` reads the block's `meta.view` (the lookup #5153's drag path already uses). Results are memoized per `(node, axis)` for the pass; the walk is already O(nodes).

## 4. Every input path gets it

| Input | Before | After |
|---|---|---|
| Window resize, maximize/restore | proportional, no floor | floors held (§3.2) |
| A layout opened in a smaller window | same | same fix |
| Chrome zoom, if it changes the container's px | same | same fix |
| Split, close a neighbour, restore a minimized pane | can leave a pane under its floor | derived sizes can't be under a floor; the stored weights may still be (§5 says what a drag does about that) |
| Splitter drag | clamps the weights | unchanged, from the same `floorPx` (the clamp becomes redundant with §3.2 but keeps the drag from staging impossible sizes) |
| Shift+window-edge resize (`windowEdgeResize.ts`) | floors with the constant 128px | uses `floorPx` per pane |

## 5. Drag next to a held pane

During a drag the math runs on weights through `pixelToSizeRatio`. If a pane is held at its floor, its weight is smaller than the pixels it shows, so `weight ↔ pixels` isn't one linear map across the group and the splitter wouldn't follow the pointer.

Rule: **at drag start, if any child of the dragged handle's parent is being held at its floor, rewrite that parent's children's weights to the displayed pixels** (× the ratio), so weights are proportional to what is on screen again; then the existing drag math, with the floors, applies unchanged. It is the same move Shift+window-edge already makes ("flex weights rewritten to preserve pixels") and it only happens when something is held, so an ordinary drag does what it does today. The visible state doesn't change at drag start. The price: that group's original proportions are forgotten once the user drags inside it, which is what dragging a splitter means.

## 6. When the floors don't fit (decision needed)

Four 250px browser panes can't be shown in an 800px window. `Σ f_i > P` needs a rule. Options:

| | What happens | For | Against |
|---|---|---|---|
| **A. Shrink the floors in proportion** (recommended for v1) | `px_i = f_i × P / Σ f_i`: every pane is under its floor by the same fraction, and nothing leaves the window. | Always valid geometry; the same rule minimized chips already use when they don't fit (`computeMainAxisAllocation`'s `scale`); no new state; the panes get their floor back as soon as there is room. | The floor is not absolute. |
| **B. Let the row scroll sideways** | Panes keep their floors; the tab's tile area scrolls horizontally. | The floors are hard. | A new interaction model for a tiling layout: native browser pages are positioned windows that would have to follow a scrolling container; handles, drag-and-drop and tear-off all assume the tab fits the window. |
| **C. Auto-minimize the panes that don't fit** | The lowest-priority panes (the least recently focused) become header chips, using the minimize display mode, and come back when there is room. | Floors stay hard, and the panes that stay are readable. | Panes disappearing as you resize is surprising; needs a separate "minimized by the layout" flag so it undoes itself, and a priority rule. A good second step, not a first. |
| **D. Stop the window from getting that small** | A minimum window size from the layout. | Simple to state. | Rejected in §2. |

Recommendation: **A now, C later if wanted.** A is a few lines in the allocation, valid in every case, and can be replaced by C without changing the rest of this spec.

## 7. Changes

| File | Change |
|---|---|
| `layout/lib/layoutFloors.ts` (new) | `floorPx`, `PANE_FLOORS`, the 128 and 250 constants; `minExtentPx` and `MinBrowserPaneWidthPx` move here. |
| `layout/lib/layoutGeometry.ts` | `computeMainAxisAllocation` takes `floors`; §3.2's resolution and §6's fallback; `updateTreeHelper` passes them. |
| `layout/lib/layoutResize.ts` | The drag reads floors from `layoutFloors`; the materialize step of §5 at drag start. |
| `layout/lib/windowEdgeResize.ts` | Per-pane floors from `layoutFloors`. |
| `layout/lib/layoutInvariants.ts` | A check (logged, never thrown, like the others): a derived pane size under its floor while `Σ floors ≤ P` is a bug. |
| Docs | `SPEC_BROWSER_PANE_THIN_BAR_AND_MENUS_IN_PANE_2026_10_18.md` §5.5 points here for window resize. |

Frontend only. No host, CEF or srv change.

## 8. Tests

### 8.1 Unit (vitest)

- `computeMainAxisAllocation`: with no floors, results are identical to today's (one existing case re-run as a guard, plus the RCA's table); the RCA's 50/25/25 row at 700px with floors 250 · 128 · 250 gives 300 · 150 · 250 (the second browser pane's 175 is held at 250, and the other two share the remaining 450 by weight; today: 350 · 175 · 175); a frozen pane's share is taken from the others in weight proportion; two rounds of freezing (a pane that only falls under its floor after another is held); minimized chips and a slip child alongside floors; `Σ floors > P` gives the §6 result and `Σ px = P`.
- `floorPx`: pane, browser pane, a stack with a browser tab, a Row group (sum plus gaps), a Column group (maximum), a minimized member.
- `pixelToSizeRatio` with and without a held pane.
- The drag: a drag next to a held pane stages the displayed sizes first and then tracks the pointer; a drag with nothing held is byte-for-byte as today.
- `windowEdgeResize` with a browser pane abutting the edge.

### 8.2 Live (own `task dev`, never the operator's instance)

- Three panes (browser, terminal, browser) in a window dragged from 1600px down to 300px: the browser panes stop at 250, the terminal at 128, then the §6 behaviour; dragged back up, the original proportions return.
- A column holding a browser pane beside a terminal: the column doesn't go under 250.
- Maximize and restore; a layout opened after the window was made smaller; chrome zoom 1.0 and 1.5.
- Splitter drag beside a held pane: no jump at drag start, the splitter follows the pointer.
- A browser pane's bar (the thin layout, `SPEC_BROWSER_PANE_THIN_BAR_AND_MENUS_IN_PANE_2026_10_18.md` §3) at exactly 250px.
- Window resize performance with six panes: no slower than today (the resize-lag measurements in `ANALYSIS_WINDOW_RESIZE_REPAINT_LAG_2026_10_06.md` §3).

## 9. Phases

| Phase | What | Shippable alone? |
|---|---|---|
| G1 | `layoutFloors` and the pure allocation with floors and §6, with its unit tests. Nothing calls it with floors yet. | Yes (no behaviour change) |
| G2 | `updateTreeHelper` passes floors; the drag and window-edge paths read from `layoutFloors`. **This fixes the report.** | Yes |
| G3 | §5's materialize at drag start. | With G2: a drag beside a held pane needs it. |
| G4 | The invariant check; the thin-bar spec points here. | Yes |

G1–G3 are one PR; G4 can ride with it.

## 10. Risks

- **Per-tick cost.** `updateTree` runs on every resize tick. The floor lookups are one `meta.view` read per leaf per pass, memoized; measure against the resize-lag baseline (§8.2).
- **Magnified panes and tear-off** are drawn outside the tile geometry and are not constrained; only tiles in the tree are.
- **Gaps.** A tile is drawn its allocation less the tile gap (`layoutNodeModels.ts`: `width - gapSizePx`), so a floor for a *visible* width is the allocation floor plus the gap. #5153's drag floor already adds it for the browser pane (250px + gap, 253px by default); `floorPx` must do the same, with the gap read from the model, and a test at gap 20.
- **A held pane next to a minimized chip.** Chips take their fixed pixels first, then floors apply to what is left; covered by a unit case.
- **Persisted layouts are unchanged**, so a rollback of this change leaves nothing behind.

## 11. Open questions

1. **§6: A, B or C when the floors don't fit?** (Recommended: A now.)
2. **Stacks:** the largest floor among a stack's tabs (stable when switching tabs), or the active tab's (tighter, but the pane changes size when you switch)? (Recommended: the largest.)
3. **Per-view floors:** is the browser the only view with its own, for now? The agent pane, terminal and editor have no stated minimum beyond 128px. Heights: 128px for all, as today.
