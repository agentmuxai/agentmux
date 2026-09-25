# RCA: busy ring's left edge intermittently shows no stripes, or faint ones

**Date:** 2026-09-25
**Severity:** Low: cosmetic.
**Status:** NOT root-caused yet. The isolated geometry is proven clean, and
three live-only candidates remain, ranked below. The next step is a live
capture while the problem is showing.

## What was reported

On an agent pane's busy ring (the marching ants around the pane border), the
top, right and bottom edges look right, with the stripes clearly visible.
Along the **left** edge there are sometimes no stripes, or very short
ones that are hard to see. It happens only sometimes.

Related earlier work:
`RETRO_AGENT_PANE_PROGRESS_RING_TOP_EDGE_GAP_2026_09_19.md` (#3443) fixed a
one-edge rounding gap between this ring and the pane's selection ring by
switching `.agent-pane-progress-bar-slot` from `inset: 2px` to
`inset: 0; border: 2px solid transparent`.

## What was ruled out (2026-09-25)

That retro's reproduction used a **static, flat-color** ring. This pass
re-tested the parts it left out: a running animation, composited layers and
a fractional pane origin.

Setup: an isolated page with the real rules copied verbatim
(`.pane-stack::after` from `PaneChrome.scss`, `.agent-pane-progress-bar-slot`
and `.agent-pane-progress-bar` from `agent-view.scss`), rendered in headless
Chrome. The pane was placed at `left: 316.5px`, the live Manoz pane's actual
x in 0.57.3, and pixels were sampled across all four edges.

| Variant | Left edge (outside → in) | Right edge (in → outside) |
|---|---|---|
| x=316.5, DPR 1, animating, focused | bg · ring · ring · **stripe** · content | content · **stripe** · ring · ring · bg |
| x=316, DPR 1, animating, focused | bg · ring · ring · **stripe** · content | content · **stripe** · ring · ring · bg |
| x=316.5, DPR 1, static, focused | same as above | same as above |
| x=316.5, DPR 1.25, animating, focused | ring ×2 · **stripe** ×1 | **stripe** ×2 · ring ×2 |

All four edges show the stripe band in every variant. The only asymmetry
is at DPR 1.25, where the stripe band is 1 device px on the left and 2 on the
right. That is ordinary 1.25× rounding (1 CSS px = 1.25 device px) and wouldn't
make the stripes disappear.

**Conclusion:** the ring's own CSS (inset, border, padding, mask, rotating
conic gradient, `will-change` layers) renders correctly on the left edge at
fractional x and fractional DPR. The cause must be something in the **live**
pane that the isolated page lacks.

Caveat: headless Chrome composites in software. The live app uses CEF with
GPU raster, which positions composited layers on the device-pixel grid through
a different path.

## Candidate causes, ranked

1. **Another element overlays the left edge band above the ring's z-index.**
   The slot is only `z-index: calc(var(--z-pane-overlay, 4) + 1)` = 5
   (`agent-view.scss:346`). The same `.pane-stack` stacking context contains
   several higher layers:
   - `.pane-stack::after` at 10, or 50 when focused (`PaneChrome.scss:49,55`).
     The isolated page included this layer and rendered cleanly, so it only
     counts here together with candidate 2;
   - `.block-mask` at 10, or 50 when focused (`block.scss:312`), inside
     `.agent-pane-stack-content`, which forms no stacking context of its own;
   - the drop-target overlays and flashes added by #3694 and #3706.

   Outside the pane, the tile layout's resize handle between split panes
   also sits at the left edge.

   Any of these overlapping the leftmost ~3px would erase the stripes when
   opaque, or dim them to "hard to see" when translucent. The unfocused
   selection ring is `rgba(255,255,255,0.16)`. This fits both "no stripes"
   and "faint stripes", and the left-only pattern fits something anchored to
   one side. The resize handle, for example, only exists where the pane has a
   neighbor, and AgentA sits to the left of the observed pane.

2. **GPU-raster layer snapping.** `.pane-stack::after` (`will-change`), the
   animating `::before`, and the masked `.agent-pane-progress-bar` above it
   are separate composited layers. Under GPU raster, each layer's offset is
   snapped to device pixels on its own. At a fractional x, the ring layer and
   the stripe layer can round in opposite directions, so the selection ring's
   2px border covers the 1px stripe band. The overlap falls on one side,
   depending on rounding direction. This wasn't reproducible in headless
   software compositing.

3. **The slot has a horizontal scroll offset.** `.agent-pane-progress-bar-slot`
   is `overflow: hidden`, so it's a scroll container. The rotating
   `::before` at `inset: -75%` gives it right/bottom scrollable overflow. A
   non-zero `scrollLeft` would move the ring left inside the clip: the left
   stripe band gets clipped away and the right one shifts inward, which is
   harder to notice. Nothing is known to scroll it, which keeps this
   lowest-ranked. It's also the cheapest to confirm or rule out.

## Next step: a live capture while it's showing

Once the stack-chrome fix
(`RETRO_AGENT_PANE_BUSY_RING_MISSING_IN_NON_AGENT_FIRST_STACK_2026_09_25.md`)
lands, any agent can inspect its **own** pane while busy. `UIQuery` and
`UIScreenshot` reach one's own pane. When the problem shows on a pane, the
agent in that pane should record:

- `.agent-pane-progress-bar-slot`: `scrollLeft` and its bounding rect
  (tests candidate 3);
- the bounding rects of `.agent-pane-progress-bar`, the `.pane-stack` root,
  and `.block-mask`, plus whether a split neighbor or resize handle sits on the
  left (tests candidate 1);
- the pane's x and the window's device-pixel ratio, plus a `UIScreenshot`
  zoomed on the left edge (tests candidate 2 if 1 and 3 come back
  clean).

Fix direction per candidate:

1. Raise the slot above the selection ring in the stacking order.
2. Put the stripes and the selection ring in the same composited layer, or
   drop `will-change` from `.pane-stack::after` while the ring is animating.
3. Replace the slot's `overflow: hidden` with `overflow: clip` (not
   scrollable) or `contain: paint`.

## Repro artifacts (scratch, not committed)

`C:\Users\area54\.agentmux\agents\manoz-0803a\repro-leftedge\`: `ring.html`
(query params `left`, `ring`, `mode`) and `sample.cjs` (pngjs edge sampler).
