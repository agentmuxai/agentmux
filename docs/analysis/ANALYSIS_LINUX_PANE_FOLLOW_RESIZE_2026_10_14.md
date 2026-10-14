# Linux browser panes follow a window resize: before and after

**Date:** 2026-10-14
**Author:** Opaz
**Follows:** `ANALYSIS_LINUX_WINDOW_PAINT_2026_10_11.md` (L1, L2), #4690 (macOS
predict-on-resize)

## 1. What changed

A browser pane is a native CEF view laid over a placeholder in the page. On
Linux it moved only when the page had laid out and sent its rect, so during a
window resize its content trailed the page's chrome, by more when the page was
busy. Three changes, all Linux:

1. **Predict on resize.** When the window's size changes
   (`WindowDelegate::on_window_bounds_changed`), the host moves each shown pane
   to where the page will put it (`LayoutBase::predict`, shared with macOS since
   #4690) and lays the window out once. The page's own rect for the new size
   replaces the guess, and a late rect for an older size is mapped onto the
   current one instead of pulling the pane back.
2. **One layout per batch (L1).** Applying a batch used to run a full
   `window.layout()` per pane, and re-show every pane. Now every pane moves,
   visibility changes only when it changes, and the window lays out once.
3. **Reflow nudge (L2).** `TileLayout.linux.tsx` now passes
   `onLeafGeometryChange: notifyPaneReflow`, as Windows does: after a layout
   change each browser pane re-samples its placeholder per frame for a moment,
   so a pane that only moves follows within a frame instead of waiting for the
   200 ms safety poll.

## 2. Method

Isolated `task dev` builds on charlie (GNOME on Wayland, native Wayland, scale
1), with the host's `[pane-bounds]` debug log. The "before" build is `main` plus
the logging alone (window resizes and per-batch timing).

- **Resize:** 4 browser panes on a local test page. The window is resized from
  the page through `windows.setRect` in 8 px steps every 16 ms (40 steps down,
  40 back), like a drag. For every size the host saw, **page lag** is the time
  from `[pane-bounds] window resized` to the first batch the page laid out for
  that size being applied, and **follow lag** the time to `[pane-bounds]
  followed`. Each run: 80 sizes.
- **Busy page:** the same with a 30 ms busy-wait every 75 ms on the page
  (the load Camper used for the same question on Windows, 2026-10-12/13).
- **Swap (L2):** two equal-sized browser panes swapped 10 times, 500 ms apart
  (`treeReducer({ type: "swap" })`), so they only move. Lag is the time from
  the swap to the first batch applied after it.

Scripts: `scratch/trail/{resize.mjs,swap.mjs,analyze.py,swaplag.py}` in the
author's workspace; not committed.

## 3. Results

When the panes reach the window's new size (ms; p50 / p90 / max):

| Run | Before: page lag | After: follow lag | After: page lag (now a correction) |
|---|---|---|---|
| Idle, run 1 | 21.6 / 35.2 / 57.3 | 0.6 / 1.2 / 2.9 | 19.0 / 30.3 / 63.5 |
| Idle, run 2 | 23.7 / 33.6 / 60.8 | | |
| Busy, run 1 | 23.6 / 52.6 / 85.1 | 0.7 / 1.2 / 3.0 | 17.9 / 49.7 / 104.7 |
| Busy, run 2 | 18.2 / 46.0 / 113.0 | 0.7 / 1.0 / 1.9 | 20.6 / 53.0 / 98.5 |

- After, the host moved all 4 panes in every one of the 80 sizes of each run,
  0.6–0.7 ms (p50) after the size changed, whatever the page's load. The page's
  own batches still take as long as before; they now refine a pane that is
  already in place instead of dragging it along.
- Screen frames recorded during a busy resize show each pane's page exactly
  inside its chrome at every width.
- Applying a batch (4 panes): 0.9 ms p50 before, 0.3–0.4 ms after (L1). The
  layout cost was small on this machine to begin with; the lag was the wait
  for the page.

Swap, time to the first batch applied (ms):

| | p50 | max |
|---|---|---|
| Before | 74.7 | 93.9 |
| After (L2) | 8.1 | 12.8 |

Before, a moved-only pane waited for the 200 ms safety poll, so its lag
depended on where the poll was (32–94 ms here); after, it follows within a
frame.

## 4. Limits

- Measured at scale 1 on one machine. The rects, the page's viewport and the
  window's client size are all in device pixels, so other scales use the same
  math, but they weren't measured.
- The resize was driven by the app (`setRect`), not by dragging the window's
  edge; the host sees the same `on_window_bounds_changed` either way.
- Prediction assumes the tile layout keeps its margins from the window's edges
  and scales every pane with the change, as on macOS. A layout that doesn't
  (a pane with a fixed size) is off until the page's rect arrives, as before.
