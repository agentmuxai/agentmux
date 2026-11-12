# SPEC: Panes glide into place when the layout changes (close first), like the My agents rows

**Status:** implemented. Approved by the operator on 2026-11-12 (D1 to D5 as recommended). PR 1 (the trigger, close and move-away) is #7036; PR 2 (open, split, minimize, restore, swap, the new pane's fade) is the PR that carries this revision. Three points changed in the build, agreed with Agent2 from what the drag's reflow taught (§6.1, §6.2): a pass holds each tile at its old rectangle for one frame, terminals are held for the pass, and a pass never opens over what a drag shows

- **Date:** 2026-11-01, **revised 2026-11-11** against `origin/main` 63848b092 (v0.60.24 plus the day's merges)
- **Author:** Camper@narko
- **Task:** #15458 (spec); the build (#15459) starts after approval
- **Area:** `frontend/layout/` only. No Rust.
- **Builds on:** the live reflow of a pane drag, [`SPEC_LIVE_REFLOW_PANE_DRAG_2026_11_06.md`](SPEC_LIVE_REFLOW_PANE_DRAG_2026_11_06.md) and [`REPORT_LIVE_REFLOW_REAL_SIZES_2026_11_10.md`](../reports/REPORT_LIVE_REFLOW_REAL_SIZES_2026_11_10.md) (Agent2), which is on main and on by default since 2026-11-11. Its §4 names this task: a close, a drop, a minimize and a drag are to move panes the same way, through one helper.
- **Research:** [`REPORT_PANE_CLOSE_ANIMATION_BEST_PRACTICE_2026_11_11.md`](../reports/REPORT_PANE_CLOSE_ANIMATION_BEST_PRACTICE_2026_11_11.md) (#29004): what should move on a close, how fast and with what curve.
- **Motion system:** `frontend/app/motion/` ([`SPEC_MOTION_PRIMITIVES_2026_10_29.md`](SPEC_MOTION_PRIMITIVES_2026_10_29.md)); the Animations setting.

The operator's words (2026-11-01, via AgentY): "when a pane closes and the rest reorganize, its pretty jerky experience. I notice the way the agent pane titles [rows] is really orderly. Can we get something similar for panes?"

---

## 0. Summary, recommendation, and what the operator decides

**What the jerk is.** On a close, every other pane takes its new rectangle in one frame: tiles, content, terminal grids and native browser views all jump together. Nothing eases, and nothing says where the closed pane's space went.

**What changed since the first draft.** The first draft (2026-11-01) recommended laying every pane out at its final size at once and moving only the window it is seen through (a place and a clip). That method is now on main for pane drags: while a pane is dragged, the other panes glide to where the drop would leave them, and when a pane rests over its own slot the layout shows the tear-off, the pane fading and its neighbours closing over the slot. The operator dragged it on 2026-11-11: "this looks good. When I drag a pane, the other move quite well around". The tear-off picture is already the picture of a close.

**Recommendation: a close is a second trigger for the same glide, not a second mechanism.** When the tree changes outside a drag, each pane that changed place or size glides its four edges from the rectangle it shows to its new one, with the timing the drag uses (160 ms, ease-out). The closed pane leaves as a light ghost of its frame that fades over the same time. The final layout, focus and the message to srv all happen at the first frame, as today; only the drawing glides, and a terminal takes its new size once, when the glide ends. With Animations Off it is exactly today's snap.

**Browser panes.** A native browser view is a window the page cannot move with a transform or clip. As in the drag today, a browser pane takes its new rectangle at once while the other panes glide. When the drag's browser phase lands (its phase 3: the native view moves in lockstep), a close gets it too, with no further work here.

### Decisions for the operator (recommendation first)

| # | Question | Recommended | Alternatives |
|---|---|---|---|
| D1 | Scope | **Every change of the tree through the one trigger**, shipped as PR 1 = the trigger plus close (button, keyboard, an agent's close, a pane moved to another window), PR 2 = open and split, minimize and restore, swap | Close only |
| D2 | The feel | **The drag's own: 160 ms, ease-out** (`--duration-base`), scaled by the Animations speed. One motion for drag, drop and close | The first draft's 240 ms decelerate; the rows' 400 ms spring (an overshoot would push a pane past its rectangle, over a neighbour) |
| D3 | How the closed pane leaves | **A ghost of its frame (background, border, header strip, not its content) fades in place over the glide** | Nothing (the neighbours glide over bare background); the rows' "poof" |
| D4 | A tab with browser panes | **The other panes glide and the browser pane jumps**, as in a drag today. For up to 160 ms there can be a bare strip beside the browser pane, or the browser pane over a neighbour's moving edge | Snap the whole change when any visible browser pane changes rectangle (then a tab with a browser pane never glides until the drag's phase 3) |
| D5 | Reduce movement and Off | **As the drag: under Reduce movement it glides, shortened to 100 ms; under Off (speed 0) it is instant.** The operator approved this rule for the drag ("essential spatial motion") | No glide under Reduce movement either (the first draft's D5) |

---

## 1. Request

Make the tile layout's reflow orderly the way the My agents rows are, at least for a close. Say whether open, split, minimize, restore and drop-rearrange are included, how it behaves with animations off, the budget, and the options with a recommendation first.

## 2. What is on main now (63848b092)

| Piece | State |
|---|---|
| A tile's place | `setTransform()` writes `translate3d` + width + height inline on each `.tile-node`; nothing eases it outside a drag. A close, an open and a minimize still snap. |
| The four edges | Every tile carries `--reflow-l/t/r/b` (registered lengths, `tilelayout.scss`) **at rest too**, set from its shown rectangle (`TileLayout.core.tsx` `tileStyle`, `shownPx`). |
| The live reflow | `layout/lib/liveReflow.ts` `createLiveReflow(model)`: while a drag is live (or a tear-off is shown) each DOM tile has an entry `{ rect, box }`. With an entry the tile's transform and `clip-path` are read from the edges, the edges have a transition, and the tile is laid out at `box`: the new size at once where it grows, the larger old size under the closing clip where it shrinks, until the glide is over (`envelope`, `reflowSettleMs`). A shared edge has one target in both neighbours, so it travels as one line. |
| A pane that leaves | `tearOffPreview.ts`: the dragged pane's entry is `leaving`; it fades over the glide and the others close over its slot. Shown before the release and held until the tree lets the pane go, so nothing glides twice. |
| A drop | Commits what is already on screen: the entries go in the same batch as the real rectangles. **So drop-rearrange already glides** and needs nothing from this spec. |
| Terminals | `pane-size-hold.ts`: for the length of a drag they keep their grid and fit once at the end, because a drag arms many candidates. |
| Browser panes | Not in the reflow (`isBrowserBlock` skips them): "left where it is for now". The drag spec's phase 3 moves the native view in lockstep; not built yet. |
| Motion policy | `shouldReduceMotion("transition")` picks `--duration-fast` for the reflow; `--duration-base` is `160ms * --motion-scale`, so speed 0 is no motion. More than 24 panes: no reflow (`REFLOW_MAX_PANES`). |
| The May 2026 animation | Built, then removed on 2026-06-03 (#1246): it resized content every frame. Its stale names remain (`useDebouncedNodeInnerRect`, comments in `layoutModel.ts`). |

## 3. Why it feels jerky, and what the rows have

Rows (`MyAgentsList.tsx`): a keyed `TransitionGroup` with a move class (each row is drawn at its old place and eases to the new one), and a deleted row is taken out of the flow so the others start moving at once.

What makes them orderly, and what a close needs:
1. **One coordinated motion.** All survivors move together, from where they were.
2. **The leaver does not hold the others up.**
3. **The final state is committed first.** The motion is only how it is drawn.
4. **A row keeps its size; a pane does not.** A neighbour of a closed pane grows, so a move alone is not enough. The live reflow's answer is the edges: place and clip from four numbers, the size set once underneath.

## 4. Constraints

- **No per-frame resize of a pane's content.** A terminal refit reallocates its WebGL canvas (`term-resize-policy.ts`); a long transcript relays out. The reflow lays a pane out once per target, never per frame.
- **The final state at the first frame.** Focus moves to the next pane, the PTY size is sent, the tree goes to srv, keys go to the pane under them. None may wait on the motion.
- **The trigger cannot be "the close button".** The tree changes from the user's action, from srv's own actions (an agent's close, close on exit, another window) and from srv's stored tree correcting the frontend. The trigger is a change of the leaves' rectangles when the tree's structure changed.
- **A native browser view cannot be moved by a transform or cut by a clip.**

## 5. Options

### A. A second trigger for the live reflow (recommended)
After a structural change of the tree outside a drag, give every DOM tile whose rectangle changed an entry for the length of the glide: `rect` is its new rectangle, `box` the envelope of the size it was laid out at and the new one. The tile already holds its old edges, so turning the entry on in the same batch as the new rectangle starts the edge transition from the old rectangle with no measuring. When the glide is over the entries go and the plain style, already final, takes over.
- *For:* no new motion code and no new CSS; the look the operator approved; retargeting is the transition's own (a second change during a glide starts from where each edge is drawn); one test bed with the drag.
- *Against:* a pane that grows is uncovered, not slid (the same as in a drag); browser panes jump (D4).

### B. Preview before the commit, as the tear-off does
When this window starts a close, show the layout without the pane at once (`futureLayoutOf` on a delete, the pane `leaving`), and let the tree change land under it.
- *For:* the glide starts on the press, one round trip earlier; the closing pane itself fades, no ghost.
- *Against:* only for closes this window starts, so A is needed anyway for an agent's or srv's close; a close srv refuses or delays (a confirm, an agent pane's shutdown lines) needs a way back. A later refinement at most.

### C. Resize the content every frame, or scale it
The May 2026 build and the first reflow attempt. Rejected by the constraints and by the operator on 2026-11-10 ("the other panes have their aspect ratios squashed").

**Recommendation: A.**

## 6. Design (Option A)

### 6.1 The trigger
`createLiveReflow`'s targets get a second source. Today: the armed drop's future layout, while a drag is live. New: a **settling pass**, opened when `updateTree` commits rectangles that differ from the last committed ones **and** the set or arrangement of leaves changed. The pass holds, per DOM leaf present before and after, its new rectangle; it lasts `reflowSettleMs()` and then ends. While it is open `reflowFor(nodeId)` answers as it does in a drag, so `tileStyle` and `innerRectOf` need no change.

**The start of the glide.** `updateTree` has already written the new rectangles when a pass opens, so a tile's edges at rest would be the new ones and nothing would glide. The pass therefore opens in the same batch as the new rectangles with each entry at the **old** rectangle (kept from the last published layout), and moves the entries to the new one on the first frame after that one which runs on time, at most 400 ms later (changed in PR 2). The frame a pass opens on pays for the layout of every resized pane, and a glide that began in it had its first drawn frame most of the way there; so did one that began in the frame after a slow one, which runs late and gives a transition the time it was due as its start. A pane that came or was restored is not drawn while the pass stands at its start, and its fade starts with the glide. **The arrangement, as built.** It is which panes there are, the order they have in the tree, and which of them are minimized. A pass opens when panes left, panes came, a pane was minimized or restored, or the same panes took another order (a swap). Panes leaving and coming in one change is a layout replaced, and opens none. A change of size alone never opens one, whatever made it: a window or splitter resize, a zoom, a pane resized by a command.

A pass does not open for: the first paint or the reveal gate, a window or splitter resize (`isResizing`), a chrome-zoom relayout, a tab that is not shown or is still being revealed (a new tab's first panes, a tab switched to: the tab reveal gate is held), a magnified layout, a floating pane sized by its own window, more than 24 panes, or the commit of a drop or a tear-off (the panes already show the result). A leaf that moved or resized by less than 1 px gets no entry.

"The panes already show the result" is decided by what is on screen, not by whether a drag is live: the source's release ends the drag before the drop commits the tree, in the same task. So no pass opens while a drag is live, while a tear-off is shown (it is held through the release until the tree lets the pane go), or in the task in which a drag ended. A pane's picture being taken (a press on a pane header) opens no pass and ends an open one, so a fading frame is never in the picture. A layout pass that changes the rectangles under an open pass (a window resize in the middle of a glide) ends it.

### 6.2 Sizes
As in a drag: growth is laid out at once and uncovered by the clip; a shrink keeps the larger size under the closing clip and takes its real size when the pass ends. A close only grows or moves panes, so every pane is at its final size from the first frame. Open and split (PR 2) shrink panes: those take their size when the glide is over, about 200 ms later.

**Terminals are held for the pass** (changed in the build). A pane that grows on one axis and shrinks on the other would fit twice without the hold, and the first fit would run in the glide's first frame, under a clip that is still opening. The reflow's own hold (`holdPaneSizes`, on while any tile has an entry) covers a pass at no cost: a terminal keeps its grid for the glide and fits once when the pass ends, about 200 ms after the close.

### 6.3 The closed pane
Its tile leaves the DOM with the tree change. In the same batch a **ghost** is appended in a layer of the layout's root: an inert, `aria-hidden`, `pointer-events: none` box at the pane's last rectangle with its background, radius and header strip, which fades over the glide's length and removes itself. Not a clone of the pane: a clone of a terminal's canvas is blank and of a long transcript heavy. The neighbours' clips open over it.

### 6.4 A new pane (PR 2)
It appears at its final rectangle and fades in, while its neighbours' clips close to make room. As built: the pass names the panes that came or were restored (`enters`), and each one's tile carries `tile-entering` for the pass: the shared `am-fade-in` keyframe over the glide's length (the shorter one under Reduce movement). A pane that came is never given a glide of its own, and neither is a browser pane (§6.6): it is at its rectangle from the first frame. A pane still fading in when a second pass opens goes on fading; the fade is not started again.

**Minimize and restore.** The header's Minimize merges a pane's tabs into a neighbour where it has one and removes its tile: that is a pane leaving, and it glides as a close does (the ghost fades while the tab pills travel, by the merge's own motion). What follows is the pane minimized to its header strip ("Minimize to header", or a pane with no neighbour to merge into). A minimized pane takes its strip at once and its neighbours glide into the space it gave up; a restored pane takes its rectangle at once and fades in as a new pane does, while its neighbours glide aside. Neither gets a glide of its own: what a minimized tile shows (a strip with its content hidden) is not what it showed before, so there is no one picture to move.

**Swap.** The two tiles glide across each other, each from its own rectangle to the other's. No pane comes or leaves, so nothing fades.

### 6.5 A change during a glide
A second pass replaces the first: a pane already gliding has no start of its own, so each of its edges' transitions starts again from where it is drawn (the reports measured this for a change of candidate), `envelope` keeps a size large enough for both, and the first pass's ghost stays to the end of the second. A drag that starts during a pass takes over the same way.

### 6.6 Browser panes
Not in a pass, as they are not in a drag's reflow: the tile and its native view take the new rectangle at once through today's path. When the drag's phase 3 gives the native view a per-frame move, the pass uses it too.

### 6.7 Which changes

| Change | Result | PR |
|---|---|---|
| Close a pane (button, keyboard, an agent's close, close on exit, `Pane close`) | neighbours glide into the space, the ghost fades | 1 |
| A pane moved to another window, or torn off by other means than a drag | the same | 1 |
| Open or split | neighbours glide aside, the new pane fades in | 2 |
| Minimize and restore | neighbours glide; the minimized pane takes its strip at once, the restored pane fades in | 2 |
| Swap | tiles glide across | 2 |
| Drop-rearrange, tear-off by drag | already glide (live reflow) | done |
| Pane-tab changes inside a pane, magnify | no tile rectangle changes; unchanged | n/a |
| Window or splitter resize, zoom, a pane resized by a command, tab switch, a new tab, first paint | never animated | n/a |

## 7. Animations Off, Reduce movement, the flag

- **Off** (speed 0): the durations are 0, so the pass opens and ends with nothing drawn in between, and no ghost is made. The build checks that the DOM after a close under Off equals today's.
- **Reduce movement:** the glide stays, at 100 ms, as the drag's does (D5).
- **Nothing waits for the motion** but a terminal's fit: the close, focus, rect sync and the disposal of the closed pane's block do not read the pass. The PTY size is sent when the pass ends (§6.2).
- **Hidden window or tab:** no pass.
- **`layout:livereflow` set to false** turns the settling pass off with the drag's reflow: one switch for "panes move", no new setting.

## 8. Budget

| | Budget |
|---|---|
| Glide length | 160 ms at the designed speed, scaled by the Animations speed; 100 ms under Reduce movement |
| Time until usable | Focus and keys at the first frame, as on main. A DOM pane has its final size at the first frame on a close; a terminal fits, and its program is told its size, when the glide ends, about 200 ms later. For a pane that shrinks (PR 2), its real size about 200 ms later |
| Work per pane | One layout per pass, as one frame of a window resize. No script per frame: the glide is CSS |
| Panes at once | 24, shared with the drag; above it the change snaps |

## 9. Build plan, risks, and what this spec did not do

**PR 1:** the settling pass in `liveReflow.ts` (with Agent2, who owns the file; the open follow-up #47354, a tree change during a drag, touches the same lines), the trigger in `updateTree` with the rules of §6.1, the ghost, close and move-away. Tests: targets from before and after rectangles (moved, grown, below the threshold, over the cap, a browser pane left out); the rules that open no pass; a second pass during a glide; under Off no ghost and the same DOM as today; a terminal is told its size once. Look checks on macOS and Linux at 100% and 150%, in a tab with and without a browser pane.

**PR 2 (as built):** open and split, minimize and restore, swap, the new pane's fade. The trigger became the arrangement (§6.1), read from what the model last published against the tree: the leaves, their order (`leafOrder`) and the minimized set. `settlingPass.ts` (Camper's) plans it (`changeOf`, `glides`, `movesOf`) and names the entering panes; `TileLayout.core.tsx` and `tilelayout.scss` give the entering tile its class and fade. `liveReflow.ts` (Agent2's) is used as it is: the pass's shrinking panes go through its `envelope`, its hold keeps the terminals' grids, and one line of its header comment names the wider list of changes. A frame callback that outlived its pass no longer brings it back. Tests: one glide per kind of change, the fade's panes, a pass that must not open (a resize by a command, a container size change, a tab being revealed, panes leaving and coming at once).

**Files (as built, PR 1):** `frontend/layout/lib/settlingPass.ts` (the pass: its plan, its frame and its end; Camper's), `liveReflow.ts` (the second source of targets, about 30 lines; Agent2's file), `layoutGeometry.ts` (one call in `updateTree`), `layoutModel.ts` (the accessor), `tilelayout-shared.tsx`, `TileLayout.core.tsx` and `tilelayout.scss` (the ghost layer, tokens only), `settlingPass.test.ts`.

**Risks.**
- **Not measured for a close.** The mechanism's costs were measured for the drag (the reports above); the first frame of a close with several terminals growing at once is the case to trace before PR 1 merges.
- **A growing pane is uncovered, not slid.** The operator has seen this in the drag and accepted it.
- **Beside a browser pane** (D4): a bare strip or an overlap for the glide's length.
- **A pass opening when it must not** (a resize, a zoom, a tab switch) would be a visible regression; the rules of §6.1 each get a test.
- **Overlap:** Agent2 (live reflow, #47354), Agent1 (motion tokens and the Animations page). The build tells both before touching `liveReflow.ts` and `TileLayout.core.tsx`.
