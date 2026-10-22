# RCA: panes shrink below their minimum width when the window is resized

**Date:** 2026-10-19
**Author:** Korp@narko, at the operator's request
**Trigger (operator, 2026-10-18):** "the min-width is not respected when panes auto-resize during the window resize ... when the outer window is resized, the panes inside will contract to below the minimum allowed width (like you added for the browser pane)"
**Follow-up:** `docs/specs/SPEC_PANE_MINIMUMS_IN_LAYOUT_GEOMETRY_2026_10_19.md`
**Status:** root cause found by reading the code and by running the allocation function (numbers in §3); not yet reproduced in a live build.

---

## 1. Summary

A pane's minimum size (128px for every pane, and 250px wide for a browser pane since #5153) is enforced in **one** place: the splitter drag handler, which refuses to stage a size below the minimum. Nothing else in the layout knows the minimums exist.

Window resize doesn't go through that handler. It never changes the stored sizes at all: the layout stores each pane's size as a weight (a flex share), and on every container resize the geometry code turns weights into pixels in proportion (`px = weight / total × container px`). Shrink the window by 30% and every pane shrinks by 30%, whatever its minimum. The same holds for any other way the container's pixel size changes (a layout restored into a smaller window, a monitor or DPI change).

The 128px floor has had this hole since it was added. The 250px browser floor makes it easy to see because it is higher.

## 2. What the code does

| Step | Where | What |
|---|---|---|
| 1 | `useOnResize(displayContainerRef, onContainerResize)` (`layoutModelHooks.ts:108`) | The container's `ResizeObserver` fires on every window resize tick. |
| 2 | `LayoutModel.onContainerResize` → `onContainerResize` (`layoutResize.ts`) | Calls `model.updateTree()`. It writes no sizes. |
| 3 | `updateTree` → `updateTreeHelper` (`layoutGeometry.ts:240`, `:340`) | Walks the tree; for each branch calls `computeMainAxisAllocation(children, nodeIsRow, nodePixels, getSize, gapPx, slipChildIds, chromeZoom)`. |
| 4 | `computeMainAxisAllocation` (`layoutGeometry.ts:138`) | Minimized panes get fixed chip sizes (scaled down if they don't fit); the rest of the pixels are split **in proportion to the stored weights**: `pixelToSizeRatio = flexTotal / remainingPx`, `px = weight / pixelToSizeRatio`. No pane has a floor here. |
| 5 | `updateTreeHelper` Phase A | Each child's rect is the allocation. Rects flow to the DOM (`setTransform`) and, for browser panes, to the native page window (`usePaneRectSync`). |

The design is deliberate: `windowEdgeResize.ts` says "Plain window resize stays proportional (zero code — weights untouched, exactly as before)", and the minimize design (`layoutMinimize.ts`) likewise keeps stored sizes untouched and derives geometry at render time. Both are good properties: weights survive a shrink and come back when the window grows again.

The minimum, meanwhile, lives only in `onResizeMove` (`layoutResize.ts`): `if (beforeNodeSize < floor || afterNodeSize < floor) return;` for the direct drag, and per-member floors in `shrinkBlockBy` for the group drag.

The code even says so. `shrinkBlockBy`'s comment: "Already-undersized panes are a real, reachable state here — split, minimize-restore, and window-shrink reflow don't enforce minNodeSize, only this interactive drag path does."

## 3. Evidence

`computeMainAxisAllocation` run on a row of three panes with weights 50 / 25 / 25 (a browser pane, a terminal, a second browser pane), 3px gap, minimum 250px for the two browsers:

| Window (container) width | Pane widths (px) | Second browser pane |
|---|---|---|
| 1600 | 800 · 400 · 400 | ok |
| 1000 | 500 · 250 · 250 | at the minimum |
| 700 | 350 · 175 · 175 | **175, 75px under** |
| 500 | 250 · 125 · 125 | **125** |
| 400 | 200 · 100 · 100 | **100**, and the terminal is under its own 128px floor too |
| 300 | 150 · 75 · 75 | **75** |

The pane never stops at its floor; it keeps shrinking in proportion down to the window's own minimum.

## 4. Root cause

**The minimums are not part of the geometry.** There are two kinds of input that change a pane's size:

1. **Edits to the weights** (splitter drag, Shift+window-edge drag, split, close, restore). Only the splitter drag clamps them.
2. **Changes to the container's pixels** (window resize, a window restored smaller). These leave the weights alone by design, so a clamp on the weights can't catch them.

The minimum was added as a clamp on input kind 1 (a validation at the edit), when it is a constraint on the *output*: the pixels a pane is given. A constraint on the output has to be applied where the output is computed, `computeMainAxisAllocation`, and it isn't.

Contributing causes:

- **No single source for the floor.** `MinNodeSizePx` is read by the splitter drag (`layoutResize.ts`) and by the Shift+window-edge path (`windowEdgeResize.ts`, in pixels); #5153 adds the 250px browser floor in the drag path only. `computeMainAxisAllocation` has no parameter for one.
- **Groups.** A pane's minimum isn't only its own: a column holding a browser pane can't be narrower than 250px either, and a row of three can't be narrower than the sum of its panes' floors. Only a recursive floor (added to the drag path in #5153 as `minExtentPx`) captures that, and the geometry has no counterpart.
- **No policy for "it doesn't fit".** If the floors add up to more than the window, something has to give. Today nothing decides, because nothing checks.

## 5. Who else is affected

Anything that changes pixels or weights without going through the splitter drag can leave a pane under its floor. Each of these is the same cause:

- Window resize and maximize/restore (reported).
- Chrome zoom, if it changes the container's measured px (not checked; the minimum is in the same CSS px as the container rect, so it holds only if that rect changes with zoom).
- A saved layout opened on a smaller monitor or window.
- Splitting a pane (the new pane gets half), closing a neighbour and restoring a minimized pane (the `shrinkBlockBy` comment names split, minimize-restore and window-shrink); tear-off and re-dock are likely too, not checked.
- Shift+window-edge resize (`windowEdgeResize.ts`) keeps other panes' pixels fixed and floors only the panes it shrinks, at the 128px constant, not per pane.

All of them already tolerate under-floor panes: the drag code skips panes already under the floor rather than failing (`shrinkBlockBy`).

## 6. What happens to the page

A browser pane's page is a native window sized from the pane's rect. A pane squeezed to 75px gives the page a 75px-wide window: a mobile layout at best, and the pane's own bar (about 208px of buttons) wraps or clips. That is the user-visible reason for the 250px minimum.

## 7. Options considered (details in the spec)

| Option | Verdict |
|---|---|
| **A. Clamp the weights on every resize tick.** | Rejected: it rewrites stored sizes on every window drag (lossy: growing the window again wouldn't restore proportions, and it persists), and it is the whack-a-mole that left this hole. |
| **B. Make the floors constraints of the geometry.** `computeMainAxisAllocation` resolves per-pane floors the way CSS flexbox resolves `min-width`: a pane whose share is under its floor is frozen at the floor and the rest share what is left. Weights stay untouched. | **Recommended.** Every input kind is covered, with one source for the floor; weights survive a shrink. |
| **C. Refuse the window resize** (a minimum window size from the layout). | Rejected: the minimum depends on the tab's layout, the window is shared by tabs, and the OS drag would feel stuck. |

The spec works out B, including what a drag does when a frozen pane is next to it, and what to do when the floors don't fit (the part that needs a decision).
