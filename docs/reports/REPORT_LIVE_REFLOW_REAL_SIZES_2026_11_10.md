# Report: live reflow at real sizes (why the gaps, the design, what it costs)

**Status:** implemented in #6951 (`frontend/layout/lib/liveReflow.ts`, `frontend/app/platform/pane-size-hold.ts`, `frontend/app/drag/pane-capture-gate.ts`).
**Date:** 2026-11-10
**Scope:** `frontend/layout/lib/` (liveReflow, futureLayout, TileLayout.core, tilelayout-shared), `frontend/app/drag/` (tearoff-snapshot, pane-live-follow, pane-capture-gate), `frontend/app/platform/pane-size-hold.ts`, the terminal resize paths (`frontend/app/view/term/term.tsx`, `frontend/app/view/agent/components/AgentShellSubblock.tsx`, `frontend/app/view/agent/hooks/usePtyWidth.ts`). Revises `docs/specs/SPEC_LIVE_REFLOW_PANE_DRAG_2026_11_06.md` §4, §5, §6 and §9.
**Question (operator, 2026-11-10, with a screenshot of a drag):** "notice the long extended blank parts extending to the far right. sounds like it may need a rethink? how well structured is the tree?" Then: "lets deep dive into the most performant, efficient design. Also resolve any open questions, write the analysis to file, and then implement".

## 0. Short answer

- The tree is sound. The blanks came from the preview technique, not from the tree (§1).
- The technique could not work. Phase 2 moved each pane to its future corner and left it at its current size, cropping it where the future rect was smaller. A pane whose future rect is **larger** has nothing to fill it with, so every growing pane left a blank strip. Scaling would fill it, and the operator had already rejected scaling (it squashes the content).
- The rethink: during the drag every pane takes its **real** future position and size. Content lays out as it will after the drop, the layout is covered edge to edge, and the drop commits what is on screen (§3).
- It is cheap because of four rules (§4, §5): a pane is laid out **once per armed candidate**, never per frame; size never animates (a clip does); growth is laid out at once and a shrink only when the glide is over; and terminals hold their grid and their PTY size until the drag ends.
- Five open questions are resolved (§6): the blurred pane in hand, the blank band beside it, the stale preview when the layout changes under a drag (#47354), the inactivity watchdog, and the tree's structure.

## 1. What the screenshot showed

The drag in the screenshot ran in the dev instance from 21:25:04Z to 21:25:16Z (host log). Three separate things are in it.

1. **Blank strips beside panes that were growing.** The second row's two panes were each a third of the width, drawn at the corners of slots that are half the width, with a blank strip after each. That is the phase 2 rule "a larger future rect shows the pane at its own size until the drop fills it". The follow-up of the same day (#6930) covered the dragged pane's own slot with the landing tint; it did nothing for the other panes that grow.
2. **A long dark band running past the window's right edge.** That is the picture in hand (the live-follow window, Windows only). #6930 sized it to the landing rect and padded it with the picture's bottom-right pixel where the landing rect was larger than the pane. Held at the cursor, a picture as wide as a full-width landing rect runs off the window.
3. **A blurred pane.** That is also the picture in hand: it was captured blurred (§6.1).

**The tree.** The dev instance's saved layout at the time (objects.db, read-only):

```
row
  column
    row: pane | pane
    row: pane | column: pane / pane
    pane
    pane
```

A flex tree of alternating rows and columns with proportional sizes, every slot holding a pane. One blemish: the root row has a single child, a wrapper that changes no rect. Sizes are relative within a parent (10, 5, 2.5 appear because a split halves its target's size); only ratios within one parent matter. Nothing in the tree can produce a gap: `computeNodeRects` always divides a branch's whole rect among its children.

## 2. Why crop and scale both fail

A compositor-only preview can change a tile's position, its scale and its clip, and nothing else. For a pane whose future rect differs in size that leaves three choices, all wrong:

| Technique | Shrinking pane | Growing pane |
|---|---|---|
| Scale to the future size (phase 2, first) | content squashed | content stretched |
| Crop to the future size (phase 2, second; #6913) | right and bottom cut off, so a terminal's last lines and an agent's input box are hidden | blank strip |
| Translate only | overlaps its neighbour | blank strip |

The only way to show a pane at another size without distortion is to lay it out at that size. So the question is not whether to resize during the drag but how to make that cheap.

## 3. The design

For each armed candidate, `futureLayoutOf` already gives every pane's rect after the drop (the drop's own reducer on a copy of the tree, then the real layout pass). Each tile takes that rect:

- **Place:** `transform: translate3d(...)` to the rect's corner, with a transition. Compositor only.
- **Size:** `width` and `height` are set at once, with no transition. A size that animated would lay the pane out on every frame.
- **Shown size:** `clip-path: inset(0 calc(100% - Wpx) calc(100% - Hpx) 0)`, with the same transition as the place. Written from the far edges, the clip shows exactly W by H whatever size the tile is laid out at, so a transition between two of them moves the shown edge in a straight line while the laid-out size steps underneath. Neighbours' edges therefore travel together: no gap opens and nothing overlaps during the glide.
- **The envelope:** the tile is laid out at the larger, on each axis, of the size it has and the size it gets. Growth is laid out at once and uncovered by the clip. A shrink keeps the larger size under the closing clip and takes its real size `REFLOW_SETTLE_MS` (the glide's length plus 40 ms) after the candidate armed.
- **The content follows the tile:** a pane's content does not fill its tile by CSS. It is given a width and height computed from the layout's own rect (`innerRect`, `layoutNodeModels.ts`, applied by `block.tsx`). During the reflow that size comes from the tile's laid-out size instead, so the content is laid out with the tile. (The first push of #6951 missed this; see §9.)
- **No candidate armed:** every tile's target is its own layout rect, with the transition still on, so panes glide back.
- **The dragged pane** takes its landing rect like the others, unblurred, under the landing zone's tint. It is the drop's own preview, at its real size.
- **End of the drag:** the entries go, and the plain style (the layout's own rect, no transform transition) takes over in the same batch as the committed tree. After a drop that is the rect the tile already shows. After a cancel it is the rect the drag started from.

The layout's own rects (`additionalProps().rect`) are untouched, so they stay the drag's frozen hit-test snapshot (`placementPreview.ts`), and the commit on drop stays the one tree write.

## 4. What it costs

**Per pointer sample:** nothing new. The candidate tracker runs as before, and, as before, each sample computes its move again and the reducer lays the tree out again, so the action and every rect are new objects many times a second, also while the pointer is held still. The reflow compares by value: the same move on the same layout is one armed drop (`futureLayout.ts`), and targets that send every tile to the same place are the same targets (`liveReflow.ts`). So a held pointer computes no future layout, writes no style and lays nothing out.

**Per armed candidate** (a few per second at most: the tracker arms on a slow pointer or after a 120 ms dwell):

- `futureLayoutOf`: 0.006 to 0.018 ms for 2 to 8 panes (measured in phase 0).
- One style write per tile, and one layout of each pane whose laid-out size changes. That is the same work as **one frame of a window resize or a splitter drag**, which the app already does at the display's rate with the same panes.
- A second, later layout only for panes that shrink on an axis, at settle. A candidate that is replaced inside the glide never reaches settle, so a sweep across several candidates costs the shrinking panes nothing.

**Per frame of the glide:** a transform and a clip-path. No layout, no script.

**What reacts to a pane's size, and what was done about it** (survey of every `ResizeObserver` at pane level):

| Consumer | On a size change | During a drag |
|---|---|---|
| Terminal (`term.tsx`, `termwrap.ts`) | refit of the grid, then `setblocktermsize` to the PTY after 150 ms | **held** (§5) |
| Agent shell drawer (`AgentShellSubblock.tsx`) | same | **held** |
| Agent CLI width (`usePtyWidth.ts`) | `termsize` to the CLI's PTY after 150 ms | **held** |
| Agent transcript (virtual list, composer strip, top band) | row re-measure, DOM only | lays out, once per candidate |
| Sysinfo plots, files list, editor (CodeMirror), image viewer | DOM only, some throttled | lays out, once per candidate |
| Browser pane (`use-pane-rect-sync.ts`) | a native window move and a page resize | not moved yet (§7) |

**Caps, unchanged:** at most 24 panes reflow; more than that and nothing moves until the drop.

## 5. Terminals hold their size

A terminal that followed every candidate would refit its buffer (a reflow of every wrapped line when the column count changes), resize its PTY 150 ms later, and make the program in it redraw: a SIGWINCH to a shell, a full repaint to a TUI. An agent's CLI is told its width the same way and would re-render mid-turn. If the drag is cancelled all of that was for nothing, and the redraws stay in the scrollback.

So for the drag's length (`pane-size-hold.ts`) the three observers above do nothing but remember they fired. The terminal keeps its grid, drawn from its top-left corner inside the resized pane: a pane that grows shows the terminal's own background beyond the grid, and one that shrinks is clipped. When the drag ends they fit once. After a drop that is the committed size: one refit and one PTY resize, as any layout change costs. After a cancel the size is what it was, the fit proposes the grid it has, and nothing is sent.

Rejected: refitting the grid visually while holding only the PTY size. The grid and the program would disagree for the drag's length, and output arriving meanwhile would wrap for the wrong width, permanently in the scrollback.

## 6. Open questions, resolved

### 6.1 The blurred pane in hand

The picture is a capture of the window (`tearoff-snapshot.ts`), started on pointer-down so that it shows the pane before the drag changes anything. The capture runs some 50 to 250 ms later. In the logged drag it took 230 ms from the press, and the host log has the drag's hooks installed well inside that time: a quick drag. By the time the capture ran, the dragged tile already had its blur (`.dragging`), and the picture shows it. The live reflow can lose the same race: a candidate armed inside that window moves the pane out from under its own capture rect.

Fix: `pane-capture-gate.ts`. While a capture is in flight, the drag's own drawing waits: the blur, the reflow and the landing zone. The gate opens when the window has been pictured (before the crop and the encoding), or after 400 ms if the capture stalls.

### 6.2 The blank band beside the pane in hand

#6930 read "the floater should change size with the size of the landing site" as "the picture takes the landing rect's size", and padded it where the landing rect is larger. The pad is a blank band, and at the cursor it can run off the window. Now the picture is only ever **cut**: on each axis it shows the smaller of the pane's own size and the landing rect's. It is never padded and never scaled. The pane at its landing size is on screen anyway, in the landing rect itself, at real size.

No unit error was involved. The picture's size and the landing rect are both CSS px of the same window, and the app's chrome zoom is a CSS `zoom` on chrome elements, not a page zoom, so it is in neither.

Considered and rejected: re-capturing the tile at its landing size and handing that to the picture. It would be the true image, at the cost of a whole-window capture, a crop, an encode and a decode per candidate, 60 ms or more each, for a second copy of what the landing rect already shows.

### 6.3 A layout that changes under the drag (#47354)

The pending move is computed against the tree as it stood. If srv or another window closes a pane mid-drag, or the window is resized, `futureLayoutOf` was not re-run and the tiles kept rects from a layout that was gone. Now the pending action is kept with the shape of the layout it was armed on (pane ids and rects), and there is no future layout while the shape differs. Every pane goes back to its own rect until the drag's next sample arms a candidate on the new layout (the tracker already starts again on a changed shape).

### 6.4 The inactivity watchdog

Phase 2 let the reflow go after 5 s without a new candidate, to cover a drag whose end was lost. It could not tell a lost end from a pointer held still on one candidate, so holding a drop for five seconds, to look at it, would have dropped the preview while the drag was still on. With real sizes that is a whole-layout snap.

Removed. A lost end is already handled by real events, never by inactivity (`drag-session.ts`: a window `dragend`, any `pointerdown`, and on Windows the button poll), and each of them ends the session, which clears `activeDrag` (`tile-drag.ts`) and with it the reflow.

### 6.5 How well structured is the tree

See §1: sound, with one redundant single-child root row in the inspected layout, which the layout's own cleanup (`balanceNode`) had not collapsed. It changes no rect and is left alone here.

## 7. What is still not covered

- **Browser panes** are native windows: a transform does not move them and a clip does not crop them, so they stay where they are during the drag (as before), and a layout with one still shows the old picture around it. That is phase 3, with the browser seat. Real sizes make it simpler than the spec planned: the placeholder tile can take its real future rect, the window is resized once per candidate, and the existing per-frame rect sampling (`paneReflowActive`, `use-pane-rect-sync.ts`) only has to stay open for the glide's length to move the window in lockstep.
- **A terminal that grows** shows its background beyond its grid until the drop (§5). That is the one place a pane is not full of its final content, by choice.
- **Hit-testing stays on the layout the drag started on** (the frozen snapshot of phase 1), while the panes now really move. That was already true of the transform reflow; the simplified zone model (the spec's last PR) is where it is revisited.

## 8. Tests

- `liveReflow.test.ts`: a drop sampled again and again is one candidate and its shrink still settles; a pane's content is sized to the tile the reflow lays out; every pane's rect is the drop's; the areas tile the layout; growth is laid out at once and a shrink at settle; a replaced candidate costs no layout; panes go back with no candidate; nothing moves during a capture; no timer lets go of a held drop; a layout change under the drag clears the preview and the next sample re-arms; sizes are held for the drag's length and released at its end.
- `TileLayout.core.test.tsx`: the tile's transform, size, clip and transitions; no size transition; the dragged tile unblurred; the blur waits for the capture; everything drops in one step at the end.
- `pane-size-hold.test.ts`, `pane-capture-gate.test.ts`, `pane-live-follow.test.ts` (cut, never pad), `placeholder.test.tsx` (the landing zone waits for the capture).

## 9. What the first look check found (Linux, 2026-11-10)

The first push gave every tile its real size and passed its tests, and on screen the content still stood at its old size: a blank strip in a pane that grew, a cut-off edge in one that shrank. Two causes, both invisible to the unit tests, which check the tile's style in a DOM with no layout:

1. **The content is not sized by the tile.** It takes an explicit size from the layout's own rect (§3, "the content follows the tile"). Fixed in `layoutNodeModels.ts`: during the reflow the size is the tile's laid-out size.
2. **The settle never came while the pointer was sampled.** The targets were new objects with every sample, and each new one started the settle timer again, so a tile that shrank kept its larger size for as long as the drag was sampled more often than every 200 ms, which is always. Fixed by comparing by value (§4).

A terminal was therefore never resized by the first push at all, so its hold (§5) passed without being exercised. With the content following the tile it is.

Agent: Agent2@narko
