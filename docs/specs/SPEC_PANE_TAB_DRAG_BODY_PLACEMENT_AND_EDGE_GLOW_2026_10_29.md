# SPEC: Tab pill drag, header joins and body places, and an edge-glow drop highlight

**Status:** active. Approved by the operator 2026-10-29 with every recommendation ("approve all recommended": highlight B, centre of the body joins, several selected tabs keep joining anywhere, outer bands clamp to an inner half-split). Spec task #10590; built in task #10591 by Camper-2@narko.
Owner: Camper-2. Written 2026-10-29 against origin/main `fa4a3895f`.

**Changes made while building (2026-10-30).** (1) There is **no new srv command**: the tab leaves its stack with the `pane.removeTab` a close already sends, then a `splithorizontal`/`splitvertical` layout intent (the one `place_block` builds on srv) puts the block in a new leaf, both through the layout outbox so srv applies them in that order. Camper's pane-tab menu ("Open in Its Own Pane") needs the same operation, so there is one function, `splitOutStackMember(model, blockId, { targetNodeId?, side? })` in `layoutStack.ts` (§3.3). (2) One drop target on the pane root reads the pointer's zone (`pillZone`, `layout/lib/paneBodyDrop.ts`) instead of a second target on the header, so there is no nested-target ordering to rely on; a drop on the header and on the middle of the body still end in the unchanged join. (3) The glow reuses the shared `am-blink` and `am-pulse` keyframes (`_keyframes.scss`) instead of new ones. (4) A pane's **only** tab dropped on another pane's body has no stack to leave, so `placePill` (`paneBodyDrop.ts`) moves the whole pane next to the hovered one, the move a header drag makes (`computeMoveNode` and the `Move` it returns); a tab with others behind it goes through `splitOutStackMember`. (5) Checked live in a `task dev` build (2026-10-30): the glow on a header (strips 40 px wide, the identity colour or the accent, blink then breath), the ghost on the left, right and bottom bands and none in the middle, a single-tab pane dropped on a body edge (moves beside it), a tab joined by the header and then split out onto another pane's left edge. One difference from the ghost remains: a pane drag's ghost for an inner direction sits on the seam between two panes, while a tab split out takes half of the hovered pane (as `place_block` and the cross-tab redock do), so the new pane lands inside the hovered one, the same size as the ghost.

## 1. The request

The operator, 2026-10-29 (quoted):

> "beyond that, we want a refinement to tab pane pill tear off mechanics. If the pill is being dragged over the header of a pane, then we get the flashing. (brief digression: lets refine the flashing too, add gradients and the edges and use the bright select color of the pane) But if dragging over the main part of a pane, it acts as if you are draging a pane, with the standard ghost that appears for custom placemenet".

While a pane's tab pill is dragged:

1. **Over a pane's header:** the header drop highlight; dropping joins that pane's tabs.
2. **Over a pane's body:** the same placement ghost and drop zones as dragging a whole pane (split left, right, up, down, or the outer bands); dropping places the tab as its own pane there.
3. **The header highlight is restyled:** gradients at the edges, in the pane's bright selection colour.

And from AgentY, the same day, about the finding in §2.4: settle the "only some headers flash" problem here, with a highlight every header can show within the WCAG limit, and state the trade-off. The operator decides it with this spec.

## 2. What exists today (read in the code, not from older specs' prose)

### 2.1 One drop target, header and body together

`PaneTabStrip.tsx` registers a single `dropTargetForElements` on `foreignDropRootFor(strip)`, which is the **whole pane** (`.pane-stack`, header and body). A pill from another pane dropped anywhere on it appends as a tab; the flash goes on the header (`foreignDropHighlightFor`) wherever the pointer is. This was a deliberate change on 2026-09-24: `SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md` §3.3 records that the **original design was exactly what the operator asks for now** (a drop on another pane's content splits it, with the `OverlayNode`/`ComputeMove` ghost), and that the repo owner dropped it for "join anywhere". This spec reverses that for single pills and amends §3.3.

### 2.2 How a whole-pane drag previews and places

- `tile-drag.ts` `startTileDrag` sets the shared `dragState` (`nodeId`, `layoutModel`, `node`), begins a `"tile"` session and sets `layoutModel.activeDrag`.
- `activeDrag` does two things: the tile **overlay** (`OverlayNodeWrapper` and one `OverlayNode` per pane, `tilelayout-shared.tsx`) takes pointer events, and `layoutModel.overlayTransform` brings the placeholder container on screen (it is parked 100000 px off screen otherwise; `showOverlay` is the second input that does the same).
- Each `OverlayNode` is a `dropTargetForElements` whose `onDrag` (throttled 50 ms) turns the pointer into a `DropDirection` with `determineDropDirection` (centre fifth = Center; four diagonal sectors; the outer fifth of an edge = `Outer*`) and dispatches `treeReducer({ type: ComputeMove, nodeId, nodeToMoveId, direction })`. The `Placeholder` renders the pending action: this is **the standard ghost**.
- `onDrop` commits the pending action (`layoutModel.onDrop()`), or for a node from another tab routes to `redockDraggedPane`.
- **Precedent for previewing a node that is not in this tree:** a pane dragged across tabs (`crossTabDrag.ts`, `SPEC_PANE_DRAG_TO_TAB_2026_07_10.md`) passes the dragged node as `nodeToMove` and `computeMoveNode` (`layoutTree.ts:100`) accepts it when `findNode(rootNode, nodeToMoveId)` finds nothing. Preview only; the commit goes elsewhere (the redock saga).

### 2.3 How a pill moves today

`moveBlockInStack` (`layoutStack.ts`) edits the tree locally (`moveMemberAcrossStacks`, a pane emptied by the move is removed as a move, never a close) and sends `pane.moveTab`; srv applies it (`move_stack_member`) and queues a `stackmove` for other views. There is **no command that takes a tab out of a stack into a new split**. srv has both halves: `remove_stack_member`, and `place_block(Placement::Split { target_block_id, horizontal, position, size_fraction })`, which `RedockFloatingPane` already uses for a block coming from elsewhere (`queue_target_layout_split`, fractions 0.5 inner and 0.2 outer).

### 2.4 Why only some headers flash (PR #5832, #10589)

The header flash is one WCAG 2.3.1 budget for the whole window (`drop-blink-clock.ts`): at most six luminance changes per second, the exit of the pane just left counted against the entry of the next. A header whose area is at or under half of a quarter of a 10-degree field (341 by 256 px, so 10,912 px², about 330 by 33 px) is exempt and always blinks. **Wider headers are not**: a 1000 px header is 33,000 px², so sweeping across wide panes gives the first the blink and the later ones the held negative or the outline alone. Codex found that an earlier draft halved the whole field instead of its quarter; the fix (head `8cf354eb0`) narrows the exemption to what the standard allows. That is the operator's "only some of them show the flashing". It is not a bug in the clock; the flashing **area** (the whole header inverting) is too big to flash every time.

### 2.5 The colour

A pane's bright selection colour is its **identity colour**: `computeBlockIdentityColor` (`blockframe.tsx`) from the pane colour pick or the agent's `frame:activebordercolor`, via `paneRoleColor("identity")` in `pane-color-scheme.ts` (OKLCH L 0.75 / C 0.15 dark, L 0.52 / C 0.17 light, so every hue is equally loud and clears 3:1 against the pill and header tints). `blockframe.tsx` already publishes it on the pane as the CSS variable `--block-identity-color` (it is the focused ring and the active-tab underline). A pane with no identity has none; the drop colour then falls back to `--drop-target-border` (`var(--accent-color)`). No literal colour is added.

## 3. Design

### 3.1 Two zones

The drop target stops being the whole pane for a **single** pill:

| Where the pointer is | Target | Feedback | Drop |
|---|---|---|---|
| a pane's header row | header target (registered on the header element, `foreignDropHighlightFor`) | the edge glow (§3.4), nothing else | joins that pane's tabs (`onReceiveForeignTab`, unchanged) |
| a pane's body, outside the centre fifth | body target (registered on the pane root; the header, being the innermost target, wins where they overlap) | the standard placement ghost for the direction under the pointer | the tab becomes its own pane next to that one (§3.3) |
| a pane's body, centre fifth | body target | no ghost; the **header** glows, as if the pointer were on it | joins that pane's tabs, as today |
| the pane the pill came from | header: as today. Body: ghost and split only if that pane has two or more tabs | | a pane with one tab has no body zone (dragging it would only move it onto itself) |

Centre is "join" rather than the pane drag's "swap" because swapping a tab with a pane has no meaning, and it keeps the old "drop anywhere on the pane joins it" behaviour as the generous middle of the body. Open question 2.

**A multi-tab drag** (Ctrl/Cmd+click selection, `blockIds`) keeps today's behaviour: the whole pane is one join target and there is no body ghost. Placing several tabs as one new pane is a larger change (one new pane with a stack of N, a ghost that stands for it). Open question 3.

### 3.2 The body ghost reuses the pane-drag code, no second path

Nothing in the tile overlay is copied. Three small moves make the pill a participant in the existing machinery:

1. **Extract the direction and preview step out of `OverlayNode.computeDropDirection`** (and the dead-spot fallback's twin) into one function in `layout/lib`: `previewPlacement(model, targetNode, dragged, clientX, clientY)`. It computes the direction (`determineDropDirection`, `chipOverChipDirection` for a minimized chip), dispatches `ComputeMove`, and returns the direction. `OverlayNode` calls it with the tile `dragState`; the pill's body target calls it with a **synthetic dragged leaf**: `{ id: "pill:" + blockId, data: { blockId } }` passed as `nodeToMove`, exactly the cross-tab foreign-node path (§2.2). The ghost, its size (half the target for an inner direction, the outer band for `Outer*`), its enter and exit animation and its motion-policy handling are the existing `Placeholder`'s.
2. **Bring the placeholder on screen without the overlay's pointer capture.** Set `layoutModel.showOverlay` for the length of the pill drag (the same input `overlayTransform` already reads next to `activeDrag`), not `activeDrag`. `activeDrag` would turn on the overlay container's pointer events above every header (so the header target could never be hit) and `disablePointerEvents` on every block (`layoutNodeModels.ts:165`). The body target is a plain `dropTargetForElements` on the pane root, registered next to the header one; pragmatic-dnd's innermost-target rule already gives "header wins over body".
3. **Commit in the pill's own path.** `onDrop` clears the pending action and calls the new `splitBlockOutOfStack` (§3.3). It never calls `layoutModel.onDrop()`: a pending `Move` of a node that is not in the tree must not commit locally, the rule `crossTabDrag.ts` states.

What stays the same: `Placeholder`, `ComputeMove`, `determineDropDirection`, `placeholderTransform`, the throttle, the `ClearPendingAction` on leave, `setCurrentDragPayload(null)` on a drop so the cross-window monitor does not also tear the pill off.

### 3.3 Committing a split: one new command

- **Frontend** `splitOutStackMember(model, blockId, { targetNodeId?, side? })` at the end of `layoutStack.ts` (the target defaults to the member's own pane; the side to the right in a Row, below in a Column; half and half): removes the member from its stack (`removeMemberFromStack`) and sends `pane.removeTab`, then applies a `SplitHorizontal`/`SplitVertical` tree action on the target with a new leaf holding `blockId` (position before for left and up, fraction 0.5, focused), which sends the split as a layout intent. It returns false, touching nothing, when the member is its pane's only tab (§3.1) or is in no pane.
- **srv** needs nothing new. `pane.removeTab` is the close path's own command (the block stays, out of every stack), and srv applies the split intent as given, as it does for any tree edit; `place_block` builds the same intent for a block placed from elsewhere. The outbox sends the two in order, and other views get srv's queued copies as they do for any intent. (An earlier draft added `pane.splitTab`; it would have done the same in one event, at the cost of an RPC type, a reducer handler and queued actions.)
- **Ghost equals result.** The ghost for `Outer*` is a full-width or full-height band at the grandparent level, which a split of one leaf cannot express (the same mismatch `clampCrossTabDirection` documents for cross-tab drops: srv would make a 20% band of the target only). **Recommendation: reuse `clampCrossTabDirection`** so the ghost and the commit agree (an outer direction places a half-split of the hovered pane). Open question 4 says what the alternative costs.

### 3.4 The header highlight: the choice for the operator

The current highlight inverts the **whole header** (a flash of 20,000 to 100,000 px²) with a pulsing 2 px outline, and counter-inverts the tabs so they stay readable. The operator wants gradients at the edges in the pane's bright selection colour. Three ways to build that, with what each does to "every header shows it":

**A. Steady glow, no flash.** Two edge gradients (left and right) in the identity colour fade in over ~150 ms (an opacity transition, not a luminance flip), stay lit while the pointer is over the header, and breathe slowly (a pulse of 1.2 s or slower, i.e. under one change per 0.6 s, well under the limit in both motion tiers). Every header shows it, always, and nothing counts against the flash budget. Cost: no sharp "blink" at the moment of entry; the cue is calmer. Reduced-motion tier: lit, no breathing.

**B. Blink limited to the edge strips (recommended).** The same two gradients, and the entry **blinks the gradients only** (two blinks, as now: on, off, on, off, then held lit), then holds lit with the slow breath. The flashing area is the two strips, not the header. At a strip of `min(25% of the header width, 40 CSS px)` by the header height `H` (33 px at 100% chrome zoom, `Z` times that when zoomed), the area is `2 · 40Z · 33Z = 2,640 Z²` px², which is **at or under the 10,912 px² exemption for any chrome zoom up to 2.03**. So every header, however wide, is exempt from the budget and always blinks: the "only some" problem is gone by construction, and the standard is met by area, not by timing. `flashArea()` in `PaneTabStrip.tsx` then passes the strips' area (`2 · min(W/4, 40Z) · H`) to `beginHover` instead of the header's, so above 2.03 zoom (or if a theme ever widened the strips) the budget applies as a backstop with no new code. The header no longer inverts, which also deletes the counter-invert on the tabs (`drop-target-strobe-exempt` and its steady and plain variants on `.block-frame-default-header-tabstrip`): fewer states to keep in step. Reduced-motion tier: lit, no blink, no breath (the `feedback` category of the motion policy, through `mixins.when-motion-reduced(feedback)`; never a raw `prefers-reduced-motion` query).

**C. Keep the whole-header flash, restyle it.** Gradients added to today's invert. Every header's blink still depends on the budget, so the complaint stays; only narrow headers always blink. Not recommended.

**Recommendation: B**, with A as its reduced-motion treatment. Trade-off in one line: B keeps the "something just happened here" pop the operator likes and makes it reliable for every header, at the price of a smaller flashing area (the strips) instead of the header turning negative; A is calmer and simplest, but has no entry pop.

**Look (A and B).** `::before` and `::after` on `.pane-header--foreign-hover`, `pointer-events: none`, `position: absolute`, inset to the header's top and bottom, `linear-gradient(to right, var(--block-identity-color, var(--drop-target-border)) 0%, transparent 100%)` (mirrored on the right), inside the header's own box so a pane's rounded corners cannot clip it and it cannot reach the neighbouring pane (the property the current negative `outline-offset` has). The 2 px outline is dropped: the strips carry the signal. A gradient is the identity colour at the full `identity` tone over the header's `headerTint`; the contrast table in `pane-color-scheme.test.ts` already guarantees 3:1 against the header tint, so the edge reads in both themes. The colour is a token (`--block-identity-color`, falling back to `--drop-target-border`), never a literal. The blink keyframes live once in `drag/drop-indicators.scss` with the others; the numbers (strip width, blink count, cycle) are written once in `drop-feedback.scss` beside `$drop-blink-cycle` and pinned to the clock's by the existing test.

### 3.5 Edge cases

- **Browser panes.** A page is a native CEF surface over the DOM, so a drag over it may not reach the body target (the known limitation already recorded in §3.3 of the older spec, and true of whole-pane drags). The header always works. Unchanged.
- **Minimized and collapsed panes** have a header only: the header target, the glow, join. No body zone.
- **Dropped outside every pane** (a gutter): no body target, no ghost, the drop does nothing in-window, and a pill released outside the window tears off as today (`pane-tab-tearoff.ts`, untouched).
- **Cancel (Esc, leaving the window).** Pragmatic-dnd's `onDragLeave` and the drag session end clear the pending action and `showOverlay` (the session's `onSessionEnded` hook, as `tile-drag.ts` does for `activeDrag`). A stuck `showOverlay` parks nothing dangerous (it only keeps the placeholder container on screen) but is cleared in the same place anyway.
- **The pane the pill came from** keeps showing no header glow for a single pill, by design (§2.4 of the #10589 PR); with two tabs its body zone is live.
- **Floating (torn-off) panes** are not in the tile tree and are not body targets.
- **Window-tab-bar drops** (the pill over the window tab bar) are unchanged.

## 4. What is reused and what is new

Reused unchanged: `Placeholder`, `ComputeMove`/`computeMoveNode` (foreign-node path), `determineDropDirection`, `clampCrossTabDirection`, `placeholderTransform`, `removeMemberFromStack`, the split reducer actions, srv's `remove_stack_member` and split intents, `source_delete_action`, the drop-blink clock (as a backstop), the motion-policy mixins.

New: `previewPlacement` (extracted, not written twice), `pillZone` and the pill's synthetic dragged leaf (`paneBodyDrop.ts`), the `bodyDrop` prop of `PaneTabStrip`, `splitOutStackMember`, the edge-glow styles.

Deleted by option B: the whole-header invert, the counter-invert on the tabs, and the steady and plain header variants for the pane header (the strip variants for a strip not hosted in a header stay until their one remaining user, if any, is checked).

## 5. Files

`frontend/app/element/PaneTabStrip.tsx` (the zone-reading drop target, flash area), `PaneChrome.tsx` and `PaneHeaderTabStrip.tsx` (the `bodyDrop` handed down), `PaneTabStrip.scss` and `frontend/app/drop-feedback.scss` (glow), `frontend/app/element/drop-blink-clock.ts` (the strips' flash area), `frontend/layout/lib/placementPreview.ts` (new: `previewPlacement`, called by `tilelayout-shared.tsx`), `frontend/layout/lib/paneBodyDrop.ts` (new), `frontend/layout/lib/layoutStack.ts` (`splitOutStackMember`). `showOverlay` is set by the body drop for the length of a placement and cleared when it ends, leaves or drops. No srv change. The amendment to `SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md` §3.3 ships with the build.

## 6. Tests

- Unit: `previewPlacement` for a synthetic leaf gives the same direction and `ComputeMove` as the tile path for the same points; centre fifth is join, not a ghost; a single-tab pane has no body zone; clamped outer directions; `splitBlockOutOfStack` (tab leaves its stack, new leaf at the right side and fraction, a pane with one tab refuses, focus moves).
- Styles: a test that pins the strip numbers to the clock's (as the blink numbers are today) and that `flashArea()` for a 2,000 px header and for 200% chrome zoom is at or under `EXEMPT_AREA_PX2`.
- Live, once `task dev` has a fast profile (#10731; it forces a full-LTO srv build, which narko cannot take): drag a pill over a header, over the four sectors of a body, the centre, the outer bands, its own two-tab pane, a wide header at 100% and 200% zoom; record before and after for the card.

## 7. Open questions (the operator decides)

1. **Highlight: A, B (recommended) or C** (§3.4).
2. **Centre of the body = join the tabs** (recommended) or swap/no-op as a pane drag does.
3. **Several selected tabs dragged together:** keep "join anywhere on the pane" for now (recommended) or place them as one new pane with all of them as its tabs (a follow-up).
4. **Outer bands:** clamp to an inner half-split so the ghost equals the result (recommended), or commit the full-width band, which needs srv to place at the grandparent level (a larger command and a reducer change).
