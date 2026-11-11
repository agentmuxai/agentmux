# SPEC: Live reflow while dragging a pane or pane tab (the end state for pane drag and drop)

**Status:** active. Approved by the operator 2026-11-09 with every recommendation ("lets go with your best recommendations", batched as separate PRs, one per phase); spec task #27340, build #27341. Phase 1 (the velocity gate, the frozen snapshot, the preview's cost) ships with this spec.
**Owner:** Agent2@narko. **Layout review:** Camper (panes and layout seat). **Browser panes:** the browser seat (Korp) and QTest (the Browser tally).
**Operator's words (2026-11-06):** "just spoke to agent2 about live reflow for the panes. that's the desired end-state."
**Builds on:** [REPORT_LIVE_REFLOW_PANE_DRAG_ALGORITHMS_2026_11_05.md](../reports/REPORT_LIVE_REFLOW_PANE_DRAG_ALGORITHMS_2026_11_05.md) (prior art, the transform-only recommendation, the plan), `SPEC_MOTION_PRIMITIVES_2026_10_29.md`, `SPEC_MOTION_TIERS_AND_STYLESHEET_DRY_2026_10_15.md`, and the tear-off work (#6457, #6486, #6495, #6498, #22175).

## 1. Summary

Today a drag shows a **ghost**: one rectangle where the dragged pane would land, plus thin outlines of the other panes' future places. The panes themselves do not move. The end state is the one every tiling window manager and Chrome's tab strip has: **the other panes move and resize, live, to open the gap where the dragged pane would land. Dropping commits what is already on screen.**

What makes this feasible here, measured on current `origin/main` (v0.60.17 plus #6495/#6498):

1. **The destination of every pane is already computed.** `futureLayoutOf` (`frontend/layout/lib/futureLayout.ts`, #6396) runs the drop's own reducer on a copy of the tree and the real layout pass, so each pane's future rect is the drop's rect by construction (a test already asserts it).
2. **The candidate is already steadied.** `dropIntentTracker.ts` (hysteresis, dwell, #6502) decides which candidate is armed, so the layout will not shuffle while the pointer passes over zones.
3. **The cost problem is terminals and native browser panes, not the geometry** (§5).

What is new: applying the future rects to the real panes as a visual, a source-pane treatment, a cleanup contract, and a rule for native (CEF) browser panes.

## 2. Which drops reflow

| Drag | Over | Reflow | Commit on drop |
|---|---|---|---|
| A pane (header drag) | another pane's edge band (split left, right, top, bottom) | the target pane gives up half (or the outer-band share); the dragged pane's old slot closes; siblings re-share | `CommitPendingAction`, unchanged |
| A pane | another pane's outer band | the inner half-split of the same side (as the ghost shows today, `clampCrossTabDirection`) | unchanged |
| A pane | the middle of another pane, or its header | **no reflow.** The pane joins the target's tab strip, so no other pane moves. The target's header glows (existing). | unchanged (join) |
| A pane | the strip between panes (a resize seam) | the pane is inserted between the two; both neighbours share the gap | unchanged |
| A pane tab (pill) of a pane with other tabs | another pane's edge band | a **hole opens** (the pill is not a pane yet): the target gives up its share; nothing closes at the source | `splitOutStackMember` |
| A pane's only tab (pill) | another pane's edge band | as a pane drag: the source slot closes, the target splits | `computeMoveNode` |
| A pane | rests over its own slot | a release there tears the pane off (`pane-self-release.ts`), so once the pointer has rested (220 ms within 6 px) the layout **shows the tear-off result**: the pane's tile goes and the other panes close over its slot. It stays through the release until the tree lets the pane go (`tearOffPreview.ts`; the operator, 2026-11-11: "it isnt clear what is happening, it is actually a tear off") | tear-off, unchanged |
| A pane or pill | leaves the window | the source layout **shows the tear-off result** (the dragged pane's slot closes, siblings expand), as Chrome does; the torn window follows the cursor (existing live tear-off) | tear-off, unchanged |
| A pane or pill | another window tab | no reflow in this tab; the tab bar's own highlight (existing) | unchanged |
| A pane or pill | another window (cross-window) | the destination window reflows only once its monitor drives the same preview; **out of scope for phase 1** (§9, Q4) | unchanged |

The rule: **if the drop would change another pane's rect, that change is shown; otherwise nothing moves.** Joins show the existing header glow, not a reflow.

## 3. Choosing the target without jitter

Reflow makes a wrong or flickering candidate far more visible than a ghost does, so input filtering is the main design work. It is mostly in place:

- **Frozen hit-test snapshot.** While a drag is live, zones are tested against the rects the layout had when the drag started (the current rects, not the moving panes'). Otherwise the panes sliding under the pointer change what the pointer is over, and the layout oscillates. This is the one new piece of filtering.
- **Hysteresis and dwell** from `dropIntentTracker.ts`, unchanged: the armed candidate keeps the pointer until it clearly leaves (margin clamped to 25% of the zone); a new candidate arms after a short dwell, or at once from another tab.
- **Velocity gate:** nothing arms while the pointer is fast (a sweep across the window changes nothing until it slows).
- **Lookahead (optional, phase 3):** while a glide runs, the next candidate is not armed until the glide is 80% done, so two glides never fight.

Acceptance: a recorded ±3 px oscillation across a diagonal produces at most one switch; a 10 s park at 1 px either side of a boundary with 0.5 px jitter produces none.

## 4. What the user sees, and the motion

- **Every pane takes its real future rect** (revised 2026-11-10; [REPORT_LIVE_REFLOW_REAL_SIZES_2026_11_10.md](../reports/REPORT_LIVE_REFLOW_REAL_SIZES_2026_11_10.md)). What glides is its four edges, as registered properties its transform and its `clip-path` are both read from; its size is set at once, with no transition, underneath the clip. An edge two neighbours share is one target in both, so it travels as one line and no gap or overlap opens (the report, §3 and §9: a transition on the place and one on the size came apart, across threads and across a change of candidate). Its content lays out at the real size: nothing is cropped, stretched or left empty. (Phase 2 first scaled to the future size; the operator on 2026-11-10: "the other panes have their aspect ratios squashed". It then cropped, which left a blank strip beside every pane that grows; the operator the same day: "notice the long extended blank parts ... sounds like it may need a rethink?".) Ease-out over the motion tokens (`--duration-base`, `--ease-out`), not a new constant.
- **The slot:** the dragged pane's own tile takes its landing rect like the others, at its real size and unblurred, under the landing zone's tint, so the layout never shows an empty slot. (Phase 2 first hid the dragged tile and left the gap as the signal; the operator on 2026-11-10: "should there ever be gaps? they should always cover everything, right? ... the dragged pane can change sizes and move, and the other panes must accommodate to the point where there are no empty spots".) For a pill the slot is empty space with the pill's name on a card (the OS drag image stays under the cursor; §6).
- **The pane in hand (Windows):** the dragged pane is its own live-follow picture under the cursor from the drag's first move, inside the window too (#6909, operator 2026-11-09: "it should be the entire pane as soon as you start dragging"). While a drop is armed the picture is cut to the landing rect on each axis where that is smaller, never padded to a larger one and never scaled (operator 2026-11-10: "the floater should change size with the size of the landing site"; a padded picture was the "long extended blank parts extending to the far right"). The drag's own drawing (the dragged tile's blur, the reflow, the landing zone) waits until the picture has been taken, so the picture is of the pane as it stood (`pane-capture-gate.ts`, `pane-live-follow.ts`, `live_follow.rs`).
- **Retarget in flight:** a new candidate starts each pane's glide from where it is drawn now (a CSS transition retargets from the current value on its own), never from its layout rect, or panes jump.
- **Shared primitives:** the glide is a `move` flight on the motion primitives (`SPEC_MOTION_PRIMITIVES_2026_10_29.md`, Agent1's #8673), with the same preset a pane **closing** uses (#15459, panes glide when one closes), so a close, a drop, a minimize and a drag all move panes the same way. If #15459 has not landed when the build starts, this spec's glide ships first and #15459 reuses it; the two share one `paneGlide` helper and one test.
- **Reduced motion:** the glide is essential spatial motion (it says where the drop lands), as the ghost's glide already is (`tilelayout.scss`, "essential spatial motion"): it stays, shortened to 90 ms, and never becomes a jump cut.
- **Colour:** the dragged pane's slot and the target edge use the theme's accent tokens, checked on every theme at 100% and 150% chrome zoom.

## 5. Performance, including native browser panes

**Terminals and agent panes (DOM).** Revised 2026-11-10: panes are laid out at their real future size, because no compositor-only change can show a pane at another size without distorting or cropping it (the report above, §2). The cost is bounded by four rules. (1) A pane is laid out **once per armed candidate**, never per frame: the same work as one frame of a window resize. (2) Size never animates; a clip does. (3) Growth is laid out at once and a shrink only when the glide is over, so a candidate replaced inside the glide costs the shrinking panes nothing. (4) **Terminals hold their size for the drag's length** (`pane-size-hold.ts`): no refit, no PTY resize, no SIGWINCH and no width sent to an agent's CLI until the drag ends, then one fit, which after a cancel sends nothing. A terminal that grows shows its own background beyond its grid until the drop.

**Per-frame budget.** The layout work (`futureLayoutOf`: a deep copy, the reducer, the layout pass) runs **per armed candidate change**, a few per second at most, never per pointer sample, and is cached by (target, direction, dragged id, tree version). Budget: under 4 ms for 8 panes. **Measured (phase 0, 2026-11-09, `futureLayout.test.ts`, node on narko): 0.006 ms for 2 panes, 0.012 ms for 4, 0.018 ms for 8**, about 200 times under budget, so the cache by candidate is a nicety, not a need. The glide itself is CSS, so there is no per-frame script.

**Native browser panes (CEF).** A browser pane is a child window above the page, so CSS transforms do not move it, and `use-pane-rect-sync.ts` reads the placeholder's transformed `getBoundingClientRect()` and would send `browser_pane_resize` every frame (a real page resize each time). The task asks for these to move **in lockstep** with the DOM panes. Three kinds of change:

1. **Moves without a size change** (the common case: the pane slides over). Move the window every frame with one batched `SetWindowPos` pass (`DeferWindowPos`) driven by the same rAF as the glide. No page resize. This is cheap and exactly in lockstep.
2. **A size change.** The page must not re-layout per frame. During the glide the window is resized **once at the start to the smaller of the old and new rect** and moved per frame; at the end it is set to the final rect with one resize. The user sees the pane slide and then snap to its final size on the last frame; no intermediate page layouts.
3. **If the host cannot move a window per frame within budget** (to be measured on the three platforms: §8), fall back to the card: hide the native window through the existing airspace path (a pane reporting 0x0 is hidden), show its DOM placeholder with the title and the last snapshot (`tearoff-snapshot.ts` already takes one), and restore the window at its final rect with one resize on drop or cancel.

`paneReflowActive()` (`frontend/app/platform/pane-anim.ts`) is a 4 ms settle window today; the glide needs a flag that stays on for its length and is read next to it, so rect sync suspends its own resize work and follows the host's per-frame positions instead.

Cross-checks with #6371 (the Views overlays): an overlay is positioned with the pane's window; the per-frame move must carry it, or the overlay is hidden for the glide. To confirm with the browser seat in phase 0.

**Caps.** At most 24 panes animate at once; beyond that the layout reflows without a glide (snaps at arm time). At most one `futureLayoutOf` per 50 ms.

## 6. Cancel, restore, and correctness

- **Esc** (the page never sees it during a native drag; `native-cancel.ts` already carries it) and **dropping back where the drag started** restore the layout **exactly**: every offset is cleared with `transition: none` in the same batch as the real layout applies, so there is no second animation and nothing is left displaced.
- **Cleanup contract** (the risk that stranded panes could survive a lost `dragend`): the reflow is cleared on drag end and on `activeDrag` false, and it shows no drop while the layout differs from the one the candidate was armed on (srv or another window changed it during the drag, #47354): every pane goes back to its own rect until the next sample arms a candidate on the new layout. A lost end is caught by real events (`drag-session.ts`: a window `dragend`, any `pointerdown`, the button poll on Windows), never by inactivity: the 5 s watchdog of phase 2 could not tell a lost end from a pointer held still on one drop, and is gone.
- **Single writer:** the reflow writes only inline style on tile wrappers (a transform, a size, a clip). It never calls `treeReducer` with a tree-changing action and never pushes to the outbox, and the layout's own rects stay the hit-test snapshot. The only tree write remains the commit on drop.
- **Drop equivalence:** for every candidate kind, the rects after the drop equal the pre-drop target rects. True by construction through the shared reducer; asserted by the existing test, extended to the pill stand-in (which, until #26698's fix, had no size and laid out as a 2 px line).
- **Keys:** future rects are keyed by node id, but a split can give the target a new intermediate node. The reflow keys by `data.blockId` and a test asserts every current leaf has exactly one future rect.

## 7. Keyboard and accessibility

- Pointer drag keeps its keyboard equivalents (the existing pane move commands); live reflow adds no new keyboard path.
- **Escape cancels**, always.
- Announce the drop's result to assistive technology after the commit (the existing pane move announcement); no announcements per candidate.
- **Reduced motion:** §4.
- A pane's hit area follows its final rect, not its in-flight one (the frozen snapshot, §3), so keyboard and pointer targets never move under the pointer.

## 8. What #26698 ships now, and what this replaces

#26698 is the near-term refinement of the **ghost**, made live with the operator. What stays after reflow lands, and what goes:

| #26698 item | After reflow |
|---|---|
| Fix: the pill's ghost was a 2 px line (the stand-in carried no size) | **Stays.** The same stand-in feeds the reflow's hole for a pill. |
| A stronger landing rectangle (accent fill, solid edge, dim outside) | **Replaced** by the live gap for splits. **Stays** for what does not reflow: join (header glow), tear-off, and the fallback when reflow is off. |
| Dashed outlines of every other pane (#6396) | **Replaced** (the panes are the outline). Stays as the fallback for browser panes if §5.3 is chosen and for the cross-window case. |
| Colours on every theme, blur or dim outside the zone | Colours **stay**; blur or dim is dropped for reflow (the gap is the signal). |

So the polish on #26698 is kept small: the stand-in size fix and the visibility of the existing ghost, nothing that reflow will replace.

## 9. Plan, flag, decisions

**Flag:** `layout:livereflow`, default on from 2026-11-11 (phase 2 passed its look checks on macOS and Linux, and the operator dragged it: "this looks good. When I drag a pane, the other move quite well around"); it was off before. The ghost remains the fallback path, so setting the flag to false is a revert to the landing rectangle.

**Phases (each shippable, no tree-write path changes):**

0. **Measure.** `futureLayoutOf` timing on 2, 4 and 8 panes; candidate switch rate on scripted sweeps; per-frame `SetWindowPos` cost on Windows, macOS and Linux (a spike, with Clare and Maricon for the looks). *Gate: the budgets in §5.*
1. **Frozen snapshot and the velocity gate** in the tracker (§3), with replayed-trace tests.
2. **DOM-pane reflow**, with the source in its slot and the cleanup contract. First as transforms over unchanged sizes; from 2026-11-10 at real sizes (§5). Terminal `sendTermSize` and `browser_pane_resize` are 0 during a drag; a DOM pane is laid out once per armed candidate.
3. **Browser panes** (§5): moves in lockstep, one resize at the end, or the card; with the browser seat.
4. **Tear-off result in the source** and the pill hole (§2).
5. **Cross-window** is a separate spec.

**Verification** (own isolated `task dev` only): a CDP-driven pointer path with many samples per second; a Performance trace shows no long task over 16 ms from the preview and only composite work per frame; screenshots at 0, 60, 120 and 180 ms compared with the target rects; the boundary soak of §3. macOS (Clare) and Linux (Maricon) look checks at 100% and 150% before each phase merges.

**Decisions (the operator took every recommendation, 2026-11-09):**

1. **Browser panes during the glide:** move in lockstep and resize once at the end (§5.2); the card (§5.3) is the fallback if the per-frame move misses budget on a platform.
2. **Resized terminals during the glide:** scale. Revised 2026-11-10, three times in a day as the operator dragged: crop, never scale; then the dragged pane into its slot; then **real sizes for every pane**, with terminals holding their grid until the drag ends (§4, §5), because crop left a blank strip beside every pane that grows.
3. **Join drops:** keep the header glow; the target does not shrink.
4. **Cross-window reflow** in the destination: a later spec.
5. **Default:** the flag turns on once phase 2 passes for DOM panes; browser panes stay on the ghost fallback until phase 3.

**Also decided (operator, 2026-11-09):** the landing rectangle and outlines of #6648 are the interim only ("the current scheme ... is no good; if anything the previous was better"); reflow replaces them for every split, and the zone model is simplified with it (§3): the four triangles and the seams, no outer fifth, since the live gap makes the extra zones unnecessary.

**PRs, one per phase, each on muxreview approval:**

| PR | Phase | Content |
|---|---|---|
| 1 | 0 and 1 | this spec; the frozen hit-test snapshot and the velocity gate in `dropIntentTracker.ts`; `futureLayoutOf` timing as a test with a budget; replayed-trace tests |
| 2 | 2 | the DOM-pane transform reflow behind `layout:livereflow`, the source gliding into its slot, the pane in hand sized to the landing, cleanup contract, zero-counters test |
| 2b | 2 | real sizes for every pane (the report of 2026-11-10): one layout per candidate, the clip glide, terminals hold their size, the capture gate, the picture cut and never padded, #47354 |
| 2c | 2, part of 4 | the tear-off result shown when a pane rests over its own slot; flag default on |
| 3 | 3 | browser panes in lockstep (with the browser seat) |
| 4 | 4 | the tear-off result shown in the source when the pointer leaves the window, the pill hole |
| 5 | zones | the simplified zone model (Camper's `determineDropDirection`), with Camper's review |

## 10. For Camper (layout and panes seat)

Please review: the frozen snapshot and the tracker change (§3, `dropIntentTracker.ts`, `placementPreview.ts`); the use of `futureLayoutOf` and the blockId key (§6, `futureLayout.ts`, #6396 is yours); the source's glide into its slot and where the transforms are written (`tilelayout-shared.tsx`, `tilelayout.scss`, `layoutModel.ts`); and that the cleanup contract matches how a drag ends in the layout. The reflow needs no change to `layoutTree.ts` or `layoutIntentActions.ts`.

Agent: Agent2@narko
