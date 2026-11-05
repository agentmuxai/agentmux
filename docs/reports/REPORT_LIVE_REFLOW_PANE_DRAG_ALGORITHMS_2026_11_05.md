# Report: live reflow while dragging a pane (algorithms, prior art, plan)

**Status:** implemented in part: phase 1 (the candidate tracker, §4.1 items 2 and 3) in #6502; the transform reflow (phases 2 to 4) is analysis only.

**Date:** 2026-11-05
**Scope:** `frontend/layout/lib/` (futureLayout, placementPreview, TileLayout.core, layoutIntentActions, layoutGeometry, tilelayout.scss), `docs/specs/SPEC_DRAG_AND_DROP_CONSOLIDATION_2026_09_27.md`.
**Question:** the operator wants Chrome-style feedback during a pane drag. A picture follows the cursor, the OTHER panes animate to the layout the drop would produce, and an open slot shows where the pane lands. Is a "smart algorithm" needed, and what is it?

## 0. Short answer

- The layout half is already solved. `futureLayoutOf` runs the drop's own reducer on a copy of the tree and then the real layout pass. So the target rect of every pane is known for any candidate drop.
- Two things are missing. One is applying those rects to the real panes as a cheap visual. The other is a stable way to choose the candidate while the pointer moves.
- The "smart" part is not geometry. It is **input filtering**: a frozen hit-test snapshot, hysteresis around the current candidate, and a dwell/velocity gate. Together they stop panes shuffling while the pointer is only passing over them. Every surveyed system that reflows live (Chrome tabs, sortable lists) is built on this.
- The cost problem is resizing terminals and CEF browser panes, not the algorithm. The recommended answer is to reflow with **compositor transforms only** (no layout, no pane resize). The real resize is committed once, on drop, as today.

## 1. Survey of prior art

Dates are the publication or release date where it could be confirmed; "n/d" means the page showed none. Several entries rest on search or page summaries, not line-by-line reading. That is said where it matters.

### 1.1 Who reflows live, and who only draws an overlay

| System | Live reflow of the other items? | What it shows |
|---|---|---|
| Chrome tab strip | Yes | Dragged tab follows the pointer, the other tabs slide out of the way, tear-off past a vertical threshold |
| niri (Wayland tiling) | Hint rectangle only (reading of the source, unverified) | Insert hint at the insertion point, window follows pointer |
| Sortable lists (dnd-kit, SortableJS, hello-pangea/dnd, Motion Reorder) | Yes | Neighbours shift to open a slot |
| Figma auto layout | Yes (list-like) | Blue insertion indicator |
| VS Code editor groups | No | Translucent overlay over the target group |
| dockview | No | `dropOverlayModel` overlay per target |
| Golden Layout 2 | No | `drop-target-indicator`, drag proxy |
| FlexLayout | No | Outline rectangle for the drop location |
| Hyprland dwindle | No live preview | Cursor's triangle (4 triangles) picks the split side |

The dock-library family (VS Code, dockview, Golden Layout, FlexLayout, rc-dock) stops at an overlay, which is what AgentMux has today. No off-the-shelf library does live reflow of a **tree of splits**. The closest are list-style systems and Chrome's tab strip. We would build the tree version ourselves, reusing the reducer we already have.

### 1.2 Sources and what each teaches

**Chrome tab strip drag**
- `TabDragController`: https://chromium.googlesource.com/chromium/src/+/2accab56152b3890473024585376ab231268c015/chrome/browser/ui/views/tabs/tab_drag_controller.cc (n/d, pinned revision).
  - `kVerticalDetachMagnetism = 15` (mouse) and `kTouchVerticalDetachMagnetism = 50`. The strip's vertical bounds are widened by that many pixels before a tab counts as detached. This is a pixel dead zone.
  - `MoveAttached` caches `attach_index_` and `attach_x_`. The insertion index updates only after the pointer has moved a consistent distance from the cached values. This is the hysteresis against thrash.
  - Escape calls `RevertDrag()`, which returns every tab to its source position and restores selection and focus.
- Mac tab strip design note: https://www.chromium.org/developers/design-documents/tab-strip-mac/ (n/d). The dragged tab follows the mouse. The other tabs "get out of its way if it overlaps them too much". Leaving a boundary above or below the strip means tear-off.
- Third-party clones that copy Chrome's numbers report a 180 to 200 ms ease-out slide and the 15 px magnetism. Treat as a starting range only.
- Igalia, Wayland fallback: https://blogs.igalia.com/max/fallback-tab-dragging/ (n/d). Chrome keeps a hidden window as the drag owner, so the source stays consistent while the item is "in flight".

**niri**
- 0.1.10 (2024-11-09) added interactive move with an insert hint and a start threshold ("windows rubber-band a little before you drag them out"): https://github.com/niri-wm/niri/discussions/793. Hint config: https://niri-wm.github.io/niri/Configuration%3A-Layout.html.
- 25.11 (2025-11): a horizontal title-bar drag of a tiled window now scrolls the view instead of moving it. This resolves the "I only wanted to nudge" ambiguity: https://github.com/niri-wm/niri/discussions/2917.
- Reading of `scrolling.rs`, via an automated summary and not line-checked: the hint is a rectangle at the nearest gap, and the other columns do not shift. Unverified.

**VS Code**
- Overlay-only drop target in `editorDropTarget.ts`. The edge split zone is one third of the group, hard coded. A request to make it configurable was closed "not planned" on 2025-02-02: https://github.com/microsoft/vscode/issues/235090.
- The 5-zone model (4 edges plus centre) is the norm, and users find the edge zone too greedy.

**dockview**
- Overlay model (`size`, `activationSize`, `smallWidthBoundary`), no live reflow: https://dockview.dev/docs/core/dnd/overlay/.
- Smart guides: https://dockview.dev/docs/core/dnd/smartGuides/. Snapping engages at `snapDistance` (default 8 px) and releases only past `snapDistance + releaseDistance` (default 4 px more). This is the clearest published **asymmetric hysteresis**, and the docs say it exists to stop boundary oscillation. There is no dwell or debounce.

**Golden Layout 2 and FlexLayout**
- Golden Layout 2 has `drag-proxy`, `drag-source` and `drop-target-indicator` (docs https://golden-layout.github.io/golden-layout/). Indicator only.
- FlexLayout 0.8.0 moved to HTML5 drag and drop for cross-window dragging and **removed `onTabDrag`**, the hook that let apps override the drop location. `tabDragSpeed` sets the outline transition speed: https://unpkg.com/browse/flexlayout-react@0.8.1/ChangeLog.txt. Native DnD costs per-frame control, the same constraint AgentMux has.

**Sortable-list libraries**
- dnd-kit legacy separates the sorting strategy (how neighbours shift) from collision detection (`closestCenter`, `rectIntersection`, `pointerWithin`). Neighbours move through the `transform` and `transition` the hook returns: https://docs.dndkit.com/presets/sortable.
- dnd-kit `@dnd-kit/dom` has `OptimisticSortingPlugin` as a default. It updates sortable indices **on dragover** so neighbours move aside before the drop: https://dndkit.com/changelog/ (0.3 to 0.5).
- An issue opened 2026-10-27 against 0.5.1 shows its failure mode. It reorders only when the drop target changes, so crossing the middle of an item that stays the target does nothing: https://github.com/clauderic/dnd-kit/issues/2168. Lesson: if the candidate is only re-evaluated on target change, a large target feels dead.
- hello-pangea/dnd computes impact from a **dimension snapshot taken at drag start** (`get-drag-impact`, `get-displaced-by`). Displacement is virtualised to visible items: https://github.com/hello-pangea/dnd and https://medium.com/@alexandereardon/dragging-react-performance-forward-688b30d40a33 (n/d).
- SortableJS has `swapThreshold` (fraction of the target that forms the swap zone), `invertSwap`, `invertedSwapThreshold` (documented as handling "swap glitching") and `animation`: https://github.com/SortableJS/Sortable. A fractional swap zone is a dead zone by another name.
- Atlassian Pragmatic drag and drop, which AgentMux already uses, gives a per-item closest-edge hitbox and an indicator line. Reflow is the app's job: https://atlassian.design/components/pragmatic-drag-and-drop/design-guidelines.
- Motion's `layout` and `Reorder` animate neighbours with FLIP via transforms. They warn that scale animations distort children, and `layout="position"` avoids stretching: https://motion.dev/docs/react-layout-animations.

**FLIP and descendants**
- FLIP (First, Last, Invert, Play), Paul Lewis, 2015-02-11. Measure, apply the end state, invert with a transform, release. Animate only transform and opacity.
- GSAP Flip has `getState`, `from`, `absolute`, `scale` and `nested`. `getState` finishes an in-flight flip so rapid changes are safe. `scale: true` is cheaper than animating width and height but distorts children: https://gsap.com/docs/v3/Plugins/Flip/.
- View Transitions: `match-element` shipped in Chrome 137, with nested groups and scoped transitions in 140+: https://developer.chrome.com/blog/view-transitions-in-2025 (2025-10-08). It is snapshot based and, unscoped, blocks pointer events. That makes it a poor fit for a pointer-driven drag that retargets many times a second.

**Tiling insertion**
- Hyprland dwindle splits the window into four triangles and the cursor's triangle chooses the split: https://wiki.hypr.land/configuring/layouts/dwindle-layout/ and https://github.com/hyprwm/Hyprland/pull/9704. This is the same rule as `determineDropDirection` in `frontend/layout/lib/utils.ts`.

**Hysteresis, dwell, intent**
- A drag-reorder fix resolves the target against slot positions **frozen at drag start**, with a hysteresis of 0.2 of the smaller slot per axis. It was needed because moving the preview shifted the slots under a still pointer: https://github.com/SophiaScherer/DashDetective/pull/137 (2026-10-24). This is exactly the failure to avoid.
- Menu-aim / safe triangle: Ben Kamens, 2013, https://bjk5.com/post/44698559168/breaking-down-amazons-mega-dropdown. Radix has the same idea as its pointer grace area. The transferable idea: do not switch while the pointer is travelling toward something else.
- HTML Standard: drag events fire every 350 ms (plus or minus 200 ms) during a drag, as well as on movement: https://html.spec.whatwg.org/multipage/dnd.html. A dwell timer must be timer-driven, not event-driven.

## 2. Techniques: problem solved and cost

| Technique | Problem it solves | Cost / risk for us |
|---|---|---|
| Frozen snapshot hit test (hello-pangea, DashDetective) | Feedback loop: the preview moves things, the hit test reads them, the target flips | Never hit-test animated geometry. Our overlay nodes follow the real tree, which a visual-only reflow never changes, so it is free if we keep it that way |
| Hysteresis margin (dockview 8+4 px, DashDetective 0.2 of slot, Chrome `attach_x_`) | Flicker at the boundary of two targets | One more state. Too large a margin makes a target feel unreachable (dnd-kit #2168) |
| Start threshold / rubber band (niri, Chrome magnetism) | Accidental drags | A few pixels of lag. Partly present (the own-pane dead zone in `tilelayout-shared.tsx`) |
| Dwell before arming (timer driven) | Panes shuffling as the pointer passes | 100 to 150 ms of latency; too long feels broken |
| Velocity gate (fast means travelling) | Same, without latency when the user is clearly aiming | Needs a sample history; native DnD gives samples only on `dragover` |
| Heading / prediction cone (menu-aim) | Crossing pane B on the way to C arms B first | Complexity, tuning, hard to test. Defer |
| FLIP with transforms | Animates layout changes without layout cost | Scale distorts content. An in-flight animation must retarget from its current visual rect |
| Transform on tile wrappers, panes not resized | Resize storms in terminals and CEF panes | Pane shows its old size scaled or clipped; native browser panes need special care (3.4) |
| Real re-layout during drag | Pixel-true preview | Terminal refit and PTY resize per change, CEF `SetWindowPos` plus page reflow. Rejected for the live phase |
| View Transitions API | Free FLIP for DOM moves | Snapshot based, poor for pointer-driven retargeting |
| Optimistic reorder on dragover (dnd-kit) | Neighbours move before the drop | Reorders only on target change; we reorder through the reducer instead |
| Cancel by restoring (Chrome `RevertDrag`) | Escape must undo | Free for us while the tree is untouched before the drop |

## 3. What exists today, and what is missing

### 3.1 Already in place

- **`futureLayout.ts`**: `futureLayoutOf(model, action)` copies the tree and runs `applyLayoutAction`, the same function the drop commits through. It then runs `computeNodeRects`, the same layout pass as `updateTree`.
  - It returns `rects` by node id and `leafIds`.
  - It handles a foreign node (a pane from another tab, or a pane-tab pill) by inserting a copy.
  - If the reducer refuses, it returns undefined and the caller falls back to the static half-rect ghost.
- **`createPlacementPreview`** puts that in a `createMemo` on `pendingTreeAction.throttledValueAtom()`. It publishes `futureLayout` and `placementWireframe`, which draws every other pane's target rect as a dashed outline. The moved pane's rect is the ghost (`getPlaceholderTransform` in `layoutModel.ts`). Outlines already show post-drop positions; the new feature turns outlines into moving panes.
- **`placementPreview.ts`** is the one place a pointer becomes a `DropDirection` and a `ComputeMove`. The direction comes from `chipOverChipDirection`, else `determineDropDirection` (centre fifth, four diagonal triangles, outer-fifth band).
- **Input pipeline today** (`tilelayout-shared.tsx`):
  - pragmatic `onDrag` on each `OverlayNode`;
  - a 50 ms throttle per node, plus the dead-spot fallback that clamps to the nearest pane;
  - `treeReducer(ComputeMove)`, then `computeMoveNode` into a Move action;
  - `pendingTreeAction` (an `atomWithThrottle` at 10 ms), then the memo.
  There is no hysteresis, dwell or velocity. The candidate flips on every sample that crosses a diagonal.
- **Lifecycle**:
  - `startTileDrag` and `releaseTileDrag` set `activeDrag`.
  - `onDrop` runs `CommitPendingAction`, and `dragleave` runs `ClearPendingAction`.
  - `frontend/app/drag/native-cancel.ts` tells Escape from a swallowed `dragend`.
  - `onSessionEnded` in `tile-drag.ts` is the safety net.
- **The picture following the cursor** already exists. It is the native drag image (a rasterised pane via `html-to-image`), plus the host follow window outside the window (`frontend/app/drag/pane-live-follow.ts`). The dragged tile is blurred in place by `.tile-node.dragging` today.
- **Single writer**: `isLayoutQuiet` (`layoutIntents.ts`) refuses to reconcile with srv while a drag or pending action exists. The tree is not mutated until `CommitPendingAction`, which goes through the intent outbox. The preview is already view-only.
- **Animation precedent**:
  - `.placeholder-sizer` glides with `--duration-quick` and is exempt from reduced motion as "essential spatial motion".
  - Panes were deliberately made to snap: `useDebouncedNodeInnerRect` applies the inner rect immediately, and `frontend/app/platform/pane-anim.ts` records that the pane reflow CSS animation "was removed".
  - `docs/specs/SPEC_PANE_REFLOW_ANIMATION_2026_05_29.md` is the earlier design (a rAF clock with the native browser pane following). It is the best prior record of the browser-pane constraint.

### 3.2 Missing

1. **A per-pane visual offset.** Nothing maps `futureLayout.rects[id]` onto the real `.tile-node`.
2. **A key that survives the move.** `rects` is keyed by node id. `addIntermediateNode` creates an intermediate node that inherits a leaf's id (comment in `updateTree`). A split pane may therefore not keep its node id in the future tree. Key by `data.blockId` and test that every current leaf has exactly one future rect.
3. **A candidate tracker** between raw pointer and `ComputeMove` (section 4).
4. **A source-pane treatment.** The dragged pane must stop occupying its slot visually (hidden, not blurred in place). Its target rect is the open slot.
5. **A cleanup contract.** Clear the reflow on session end, on `activeDrag` false, on tree-version change, and on a watchdog, so a stranded drag cannot leave panes displaced.
6. **A browser-pane rule** (3.4).
7. **Cost measurement.** `futureLayoutOf` does a JSON deep copy, a reducer run and a full layout pass per candidate change. That is probably fine for tens of panes, but no number exists. Measure first, then cache by (target, direction, dragged id, tree version).

### 3.3 Transform or re-layout

| Option | How | Verdict |
|---|---|---|
| A. Transform only | `translate(dx,dy) scale(sx,sy)`, `transform-origin: 0 0`, `transition: transform 180ms ease-out` on each `.tile-node`; DOM size and content unchanged | **Recommended first.** Compositor only. Panes that only move (the common case) are not distorted. Resized panes look scaled for the length of the glide |
| B. Clip resize | Change `.tile-node` width/height to the target; the existing `overflow: clip` crops content that stays at its old pixel size | Refinement if A's scaling reads badly. Content has inline px sizes from `nodeModel.innerRect()` so it should not relayout, but the wrapper and flex header do. Needs a spike |
| C. Real re-layout | Publish the future rects as the real `additionalProps` while dragging | Rejected (3.4). It also breaks the frozen hit-test snapshot |

Notes for A:
- Use `setTransform`'s rounding (floor top/left, ceil width/height) so the end state equals the committed state to the pixel.
- On drop, remove the offsets with `transition: none` in the same batch in which the real layout applies, so there is no second animation.
- Set `will-change: transform` only while armed. Each promoted pane is a compositor layer.
- A transform does not trigger `ResizeObserver`, so terminal fit, PTY size and `pane-size-badge` stay quiet.
- Retarget an in-flight glide from the **current visual rect** (the computed matrix), not the layout rect, or the pane jumps.

### 3.4 Terminals, browser panes, the 60 fps problem

- **Terminals and agent panes (DOM).** With A there is no resize at all. With C each change would run `fitAddon` in `frontend/app/view/term/termwrap.ts` (`customFit`, then a debounced `sendTermSize`) and SIGWINCH the child. That is a storm even when debounced, and the content sits at the old size for a frame or two.
- **Native browser panes (CEF).** A browser pane is a separate child window above the DOM placeholder (see the header of `frontend/app/drag/tearoff-snapshot.ts`), so CSS transforms don't move it. Worse, `frontend/app/view/browser/use-pane-rect-sync.ts` reads the placeholder's `getBoundingClientRect()`, which includes transforms. It sends `browser_pane_resize` per frame while `paneReflowActive()`. A transformed tile would therefore drive `SetWindowPos` and a real page resize every frame. Two rules are needed:
  1. While the reflow is armed, suspend that pane's rect sync (a flag read next to `paneReflowActive`).
  2. Cover the pane instead of moving it. Hide the native window through the existing airspace path (a pane reporting 0x0 is hidden; magnify relies on this). Show its DOM placeholder as a card (title plus the last snapshot, as `tearoff-snapshot.ts` already takes). On end, restore the window at its committed rect with one `browser_pane_resize`.
  This needs confirming with the browser-seat owner.
- **Throttling the pending action.** The 50 ms per-node and 10 ms atom throttles sit on the input side and are cheap. The reflow is driven by the **armed candidate**, which changes a few times a second at most. The animation is CSS, so there is no per-frame JS.
- **Single writer.** The reflow reads `futureLayout` and writes only inline transforms or CSS variables on tile wrappers. It never calls `treeReducer` with a tree-changing action and never pushes to the outbox. The only tree write remains `CommitPendingAction` on drop.
- **Native DnD limits.** During a native drag the page gets `dragover` samples, not `pointermove`, and the cursor image can't be restyled. The "picture following freely" is therefore the OS drag image. That is also what makes cross-window and tear-off work. Don't replace it with a DOM ghost in this work.

## 4. Recommended algorithm

**Frozen-snapshot candidate tracker with hysteresis and dwell, driving a transform-only FLIP reflow.**

### 4.1 Pieces

1. **Snapshot at drag start.** Copy all real pane rects and the tree version. Hit-test only against this snapshot or the real overlay nodes, which never move under a visual-only reflow. Never hit-test transformed geometry.
2. **Raw candidate** `c_raw = (targetId, direction)` from the existing `placementDirection`, evaluated on each `dragover` and from the dead-spot fallback.
3. **Tracker state machine** (pure, no DOM):

```
state: Idle | Armed(c) | Pending(c_new, since)
on sample(p, t):
  v = ema(|p - p_prev| / (t - t_prev))
  c_raw = raw(p)
  if c_raw is none or p is over the dragged pane's own rect:   -> Idle   (panes glide home)
  if state is Armed(c):
      if c_raw == c:                       stay
      else if p inside sticky(c):          stay                  # hysteresis
      else                                 -> Pending(c_raw, t)
  if state is Idle or Pending(c_new):
      if c_raw != c_new:                   -> Pending(c_raw, t)  # restart dwell
      if v < V_SLOW or (t - since) >= DWELL:  -> Armed(c_raw)
timer (not event) fires at since + DWELL and re-runs the check    # dragover can be 350 ms apart
```

   - `sticky(c)` is the snapshot zone of `c` grown by `m = 0.2 * min(zone w, zone h)`. It is applied on the side where the pointer is **leaving**: entering is easy, leaving is deliberate, as in dockview's `snapDistance` vs `snapDistance + releaseDistance`.
   - Starting values: `DWELL` 120 ms, `V_SLOW` 0.35 px/ms, and 200 ms dwell for a centre (swap) candidate. These are tuning starts, not measurements.
   - The first arm needs the pointer at least 8 px outside the dragged pane's own rect. That is the rubber band.
4. **Reflow driver.** On a change to `Armed(c)` or `Idle`:
   - compute `futureLayoutOf` for `c` (memoised);
   - build `{blockId -> (dx, dy, sx, sy)}` from the **current visual rects** to the target rects;
   - write inline transforms in one batch.
   `Idle` writes identity, so panes glide home. The dragged tile is hidden and the existing placeholder shows the open slot.
5. **Commit and cancel.**
   - Drop: `CommitPendingAction` as today, clearing the offsets with `transition: none` in the same batch.
   - Escape, a drop on nothing, the drag leaving the window, or a tree-version change: `ClearPendingAction`, tracker to `Idle`, panes glide home. The tree was never touched, so nothing else needs undoing.
6. **Optional, phase 3: lookahead.** Choose between two candidates inside the sticky margin using `p + v * 60 ms`. Skip menu-aim cones until real traces show a need.

### 4.2 Why this

- It reuses the reducer-based preview, which is already correct for every tree shape. Flat-list libraries can't do that.
- Transform-only reflow plus a snapshot removes the DashDetective feedback loop by construction. Hysteresis then handles the real boundary.
- Dwell is timer-driven because a still pointer may send no `dragover` for up to about 550 ms.
- Cost scales with the number of panes moved, not their content, and never touches a PTY or CEF window.

## 5. Incremental plan

Each phase is shippable, behind a flag (e.g. `layout:livereflow`, default off), and changes no tree-write path.

- **Phase 0, measure.** Time `futureLayoutOf` (copy, reducer, layout) and log the candidate switch rate on scripted sweeps across 2, 4 and 8 panes in a dev build.
- **Phase 1, tracker only.** Add a pure `dropIntentTracker.ts` beside `placementPreview.ts`. Feed the existing ghost and wireframe from the **armed** candidate. This alone cures today's boundary flicker.
  - Tests replay recorded traces:
    - an oscillation of plus or minus 3 px across a diagonal gives at most 1 switch;
    - a fast sweep arms nothing until it slows;
    - a slow aim arms within `DWELL`;
    - a timer-only dwell arms with no further samples.
- **Phase 2, DOM-pane transform reflow.**
  - Per-pane transforms keyed by `blockId`, source tile hidden, 180 ms ease-out glide.
  - The cleanup contract: session end, `activeDrag` false, tree-version change, and a 5 s watchdog.
  - Browser panes keep their place and the dashed outline.
  - Tests: the mapping covers every leaf exactly once for a split, an outer-edge drop and a swap; a no-change candidate gives identity; a retarget in flight starts from the visual rect.
  - Dev counters for terminal `sendTermSize` calls, `browser_pane_resize` calls and `ResizeObserver` callbacks must all be 0 during a scripted drag.
- **Phase 3, browser panes and polish.** Suspend rect sync, cover the native window with a card, restore with one resize. Settle the reduced-motion rule, add optional lookahead, and handle tear-off.
- **Phase 4, cross-tab and cross-window.** Separate spec.

**Verification** (own isolated `task dev` build only, never the operator's or other agents' instance):
1. A CDP-driven pointer path with many samples per second. A Performance trace shows no long task over 16 ms from the preview, and only composite work per frame during a glide.
2. A screenshot sequence at 0, 60, 120 and 180 ms after a change, compared with the dashed outlines for the same candidate. They must coincide at the end.
3. The three counters are 0.
4. Boundary soak: park 1 px either side of a diagonal for 10 s with 0.5 px jitter. Arm switches must be 0.
5. Drop equivalence: for each candidate type, the committed rects equal the pre-drop target rects. This is true by construction through the shared reducer; assert it in a test.

## 6. Risks and open points

- **Resize storms.** Removed for DOM panes by option A. For CEF panes only if rect sync is suspended and the window covered. If the card approach is rejected, browser panes stay frozen with an outline only, which weakens the illusion.
- **Boundary flicker.** Handled by hysteresis, dwell and the snapshot. The residual risk is small panes feeling unreachable, so clamp the margin to 25 percent of the zone and keep the swap dwell separate.
- **Leaf identity.** A split pane may get a new node id in the future tree. Key by `blockId` and confirm the key for stacked leaves. A tab-pill stand-in has no real pane, so only a hole opens.
- **Scale distortion.** Acceptable for 180 ms. If it reads badly, use option B for panes whose size changes.
- **Tree changes mid-drag.** srv or another agent can change the layout during a drag (`processingBackendActions`). Tie the armed state to the tree version, fall back to `Idle`, and recompute.
- **Escape and a swallowed `dragend`.** The page doesn't see Escape during a native drag (`native-cancel.ts`). Cleanup must hang off session end and `activeDrag`, with the watchdog as the last resort.
- **Cross-window.**
  - When the pointer leaves the window, the source could show the tear-off result (pane gone, siblings expanding), as Chrome does.
  - The destination is driven by the cross-window monitor and overlay, not pragmatic drop targets, so it needs its own spec. Until then it keeps the outline.
- **Reduced motion.** Follow the placeholder precedent (essential spatial motion), with a shorter duration.
- **Layer memory.** Limit `will-change` to the armed state.
- **Expectation.** The picture under the cursor is the OS drag image and stays so. The live part is the other panes and the open slot.
