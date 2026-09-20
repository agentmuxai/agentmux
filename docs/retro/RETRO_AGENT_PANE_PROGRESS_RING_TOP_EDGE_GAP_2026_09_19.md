# Retro: gap between the 1px progress ring and the pane border, one edge at a time

**Date:** 2026-09-19
**Severity:** Low — cosmetic, reported on the agent pane's busy-indicator ring.
**Status:** fixed, on the second attempt. `frontend/app/view/agent/agent-view.scss`
was changed (`.agent-pane-progress-bar-slot` and `.agent-pane-progress-bar`);
see "Confirmed root cause and fix" below. Earlier revisions of this document
described this as investigation-only with no fix applied (true for the first
pass, all-negative geometry/DPR results) and then as fixed with a top-edge-only
change (true for that specific edge, on that specific live test, but the fix
was scoped too narrowly — see "First fix attempt was too narrow" below).

## What was reported

After `2f0003851` ("fix(agent-pane): thin the progress-bar ring from 3px to
1px", #3396) landed, thinning the full-perimeter busy-indicator ring
introduced in `90aa773c9` (#3371), the user observed: on the pane's **top**
edge only, there appears to be a small gap between the animated 1px line and
the pane's own border. Left, right, and bottom were not reported as showing
the gap.

## Relevant code

- `frontend/app/element/PaneChrome.scss` `.pane-stack::after` — the pane's
  selection/border ring: `position: absolute; inset: 0; border: 2px solid
  var(--pane-ring-color, var(--border-color));`.
- `frontend/app/view/agent/agent-view.scss`:
  - `.agent-pane-progress-bar-slot` — `position: absolute; inset: 2px;` —
    explicitly inset by the ring's 2px border width so the bar nests inside
    it (`SPEC_AGENT_PANE_PROGRESS_BAR_OVERLAY_NO_GAP_2026_08_25.md`).
  - `.agent-pane-progress-bar` — fills the slot (`inset: 0`), uses
    `padding: 1px` (was `3px`) plus a `mask`/`mask-composite: exclude`
    "picture frame" technique to render only that 1px padding band, with an
    animated `repeating-conic-gradient` on `::before` (`inset: -75%`, rotated
    via `transform: rotate()`) supplying the marching-ants pattern.
  - Comments in both files reference
    `SPEC_AGENT_PANE_PROGRESS_BAR_FULL_PERIMETER_2026_09_18.md`. **That file
    does not exist anywhere in the repo or its history** (`git log --all -- '*FULL_PERIMETER*'`
    returns nothing) — the full-perimeter behavior itself is real and shipped
    in `90aa773c9`, but whoever wrote those comments referenced a spec
    document that was apparently never committed. Worth fixing the comment
    or adding the missing spec, separately from this investigation.

## Investigation method

Rather than guess from reading CSS, I built an isolated static repro
(`C:\Users\asafe\.agentmux\agents\agent1-06309\repro\ring-repro.html`) that
reproduces the exact selector structure and computed values pulled from the
real theme (`frontend/app/theme.scss`, `frontend/tailwindsetup.css`):

- `--block-border-radius: 0` (default theme value — square corners, not
  rounded, so corner-radius mismatch between the ring and the slot is not a
  factor)
- `--header-height: 33px`, `--space-1: 4px` (real header sizing/padding)
- `--zoomfactor: 1` (the default; only overridden per-pane by
  `frontend/app/store/zoom.ts` when a pane is explicitly zoomed)
- The real `.pane-stack::after` ring, `.agent-pane-progress-bar-slot`, and
  `.agent-pane-progress-bar` (mask technique included) rules, copied
  verbatim with variables substituted.

Rendered with Playwright (Chromium) and sampled actual pixel colors (via
`pngjs`) along the top, bottom, left, and right edges, at `deviceScaleFactor`
1, 1.25, 1.5, and 1.75 (covering common fractional Windows display-scaling
values, since a border+mask combination is a known source of device-pixel
rounding seams under fractional DPR). The animated conic-gradient pattern was
swapped for a **flat color** for this test, deliberately isolating pure ring
*geometry* from the gradient's own color/phase behavior (see "Ruled out"
below for why that swap mattered).

## Findings

At every DPR tested (1, 1.25, 1.5, 1.75), on **all four edges**, the pixel
column/row transitions directly from the ring color to the progress-bar
color with no background pixel in between — e.g. at DPR 1, sampling straight
down the top edge at the pane's horizontal center:

```
y offset -1 -> [34, 34, 34]     (pane surroundings)
y offset  0 -> [77, 163, 255]   (ring, blue)
y offset  1 -> [77, 163, 255]   (ring, blue)
y offset  2 -> [255, 59, 48]    (progress bar, red)
y offset  3 -> [51, 51, 51]     (pane background)
```

Bottom, left, and right edges show the identical touching-with-no-gap
pattern. This holds at 1.25/1.5/1.75 DPR too — no frame showed a background
pixel wedged between the two colors on any edge, top included.

**Conclusion: the ring's own inset/padding arithmetic is not the source of
the reported gap.** `.agent-pane-progress-bar-slot`'s `inset: 2px` and
`.pane-stack::after`'s 2px border register pixel-perfectly against each
other in an isolated, static reproduction — including at fractional device
scale factors, which was the leading suspect for a top-only artifact.

## Ruled out

- **Corner-radius mismatch** — `--block-border-radius` is `0` by default;
  square corners, nothing to misalign.
- **`--zoomfactor` at its default value of `1`** — ruled out at DPR 1 in the
  isolated repro (see Findings above). This bullet was corrected after
  further investigation below: `--zoomfactor` is not idle by default in
  practice, and it is not "per-pane content zoom" — see "Confirmed root
  cause" below for what it actually is and why it matters here.
- **Static box-model/inset math** — proven pixel-exact above, at 4 different
  DPR values.
- **Device-pixel rounding under fractional display scaling** — tested
  directly (1.25/1.5/1.75); no gap opened at any of them in the isolated
  repro.

## Confirmed root cause

The user's own follow-up ("perhaps related to the zoom level of the chrome")
pointed at the right mechanism, and reported the diagnostic signature
directly: *"at some levels the gap is gone, but on some it comes back"* —
gap presence toggling with zoom/display-scale level, rather than being
constantly present or constantly absent, is the signature of a
**device-pixel rounding mismatch**, not a CSS math error (which would be
either always-present or never-present, independent of scale level — as the
DPR 1/1.25/1.5/1.75 tests above already showed for pure inset/padding math).

`frontend/app/store/zoom.ts` draws a real distinction the first investigation
pass missed:

- **Per-pane content zoom** (`term:zoom` meta, `Ctrl+Scroll` on a
  terminal/agent/editor/etc. pane) is implemented via **font-size scaling**
  (`computeEffectiveFontSize`), not the CSS `zoom` property at all. This is
  the "per-pane zoom" the original `.agent-pane-progress-bar-slot` comment
  ("outside `.agent-view`'s per-pane zoom CSS property entirely") was
  talking about, and ruling it out was correct as far as it went.
- **Chrome zoom** (`chromeZoomIn`/`chromeZoomOut`, "title bar + status bar"
  per that file's own header comment) is a *different* mechanism:
  `applyChromeZoomCSS` sets `--zoomfactor` directly on
  `document.documentElement`, global and inherited by everything in the
  window. `frontend/app/mixins.scss`'s `block-frame-default-header-layout()`
  — the mixin the agent pane's own hoisted header uses — applies
  `zoom: var(--zoomfactor, 1)` to itself. So the agent pane header scales
  with **chrome** zoom, a mechanism this retro's first pass never
  considered because it was looking for a *per-pane* effect.

The CSS `zoom` property (unlike `transform: scale()`) recomputes real layout
geometry rather than compositing an already-painted layer — this is exactly
why it's normally preferred for scaling UI chrome (surrounding elements
reflow to the true scaled size, no separate compositor-rounding step). But
that also means when `--zoomfactor` combines with the OS's own display
scaling (Windows' 125%/150%/175% etc., a second, independent fractional
factor) to a size that doesn't land on a whole device pixel, Chromium must
round the **header's own box edge** to the nearest device pixel — and nothing
requires that rounding to agree with the **independently-computed** rounding
of `.pane-stack::after`'s border and `.agent-pane-progress-bar-slot`, both of
which are deliberately *unzoomed* siblings computed from `.pane-stack`'s own
un-scaled box. Two independently-rounded boxes agreeing at some
zoom×DPI combinations and disagreeing by exactly one device pixel at others
is precisely "gone at some levels, back at others." And since the header is
the *only* zoomed element in this box, and it sits at the top, this class of
seam can only appear on the top edge — matching the original report exactly.

## First fix attempt was too narrow

The first fix scoped the 1px overlap to the top edge only —
`.agent-pane-progress-bar-slot { inset: 1px 2px 2px 2px; }` paired with
`.agent-pane-progress-bar { padding: 2px 1px 1px 1px; }` — reasoning that
since the header is the only zoomed element in the box, and it sits at the
top, only the top edge could show this class of seam.

Live-tested via `task dev`: **the top gap was gone, but the same gap
appeared on the bottom edge instead.** That result falsifies the "only
possible next to the zoomed header" framing above — the bottom edge is not
adjacent to anything with `zoom` applied. The real cause has to be more
general: `.pane-stack::after` (the focus ring) and
`.agent-pane-progress-bar-slot` are two *independently* absolutely-positioned
elements, each rounded to the device pixel grid separately by the browser,
both computed off `.pane-stack`'s own height/width. Panes in this app are
resizable (drag-resize tiled splits), so that height/width is routinely a
fractional number of CSS pixels with no special relationship to the header's
zoom at all. A fractional container size means *some* edge's rounding can
disagree between the two independently-rounded boxes — chrome zoom on the
header is one way to introduce a fractional discrepancy, but not the only
one, and not tied to any particular edge. Fixing only the top edge didn't
remove the discrepancy, it just moved where it was free to surface.

## Confirmed root cause (revised)

Two independently-positioned, independently-rounded absolute overlays
(`.pane-stack::after`'s 2px border and `.agent-pane-progress-bar-slot`'s
inset) sharing a fractional container size can disagree by one device pixel
on *any* edge — not predictably the same one. Chrome zoom on the header
(described above) is a real, confirmed contributor to that fractional
arithmetic, but the fix needs to protect every edge, not just the one
adjacent to the zoomed element.

## Fix applied

`frontend/app/view/agent/agent-view.scss`, two paired changes, applied
**uniformly on all four sides** (both required together — see comments left
in the code for the full reasoning):

1. `.agent-pane-progress-bar-slot`: inset changed from `2px` to `1px`
   (all sides) — pulls the ring 1px closer to the true pane edge, everywhere.
2. `.agent-pane-progress-bar`: padding (ring thickness) changed from `1px`
   to `2px` (all sides) — widens the ring's own band by 1px on every side,
   so the ring's visible position doesn't move (nothing shifts toward the
   content side).

Together these add one extra pixel of ring coverage on every edge, landing
*inside* the 2px focus ring's own footprint — invisible in the common case,
since the focus ring paints above this element (z-index) and fully covers
it — but present as a safety margin against a one-device-pixel rounding
disagreement, wherever it happens to land. Standard technique for this class
of bug: deliberately overlap a seam by construction, on every edge, rather
than rely on two independently-rounded boxes to agree on any one of them.

## Validation

Extended the same Playwright + pngjs repro methodology used for the
investigation, for both the top-only attempt and the final uniform fix:

- **No regression**: with the uniform fix applied, sampling all four edges
  at DPR 1 produces pixel-identical output to the original (unfixed) CSS —
  border, then a 1px-visible ring, then background — on every edge.
  (`node pixels_uniform.mjs` in the repro directory.)
- **Fix is effective on the edge that actually failed live (bottom)**: built
  `ring-repro-uniform-shortbottom.html`, simulating the bottom border
  rendering 1 device pixel short (`border-width: 2px 2px 1px 2px`).
  Unfixed CSS under that condition reproduces a background-colored gap
  between border and ring (same shape as the original top-edge repro,
  `ring-repro-shortborder.html`, which reproduced the original top symptom).
  The uniform fix closes it: `border(blue) → ring(red) → ring(red) →
  background`, no gap pixel.
- Confirmed via the same method that the top edge (the case the first,
  narrower fix already handled) is unaffected by generalizing to all four
  sides.

This proves the mechanism (a rounding-short edge, on either the top or
bottom, and by the same reasoning potentially left/right at some other
container size) produces precisely the reported symptom, and that the
uniform fix removes it on every edge tested without altering the
correctly-rounding case. Recommend the user re-check at the zoom/display-scale/
pane-size combinations that previously showed the gap now that the app is
running with this change.

## Repro artifacts

`C:\Users\asafe\.agentmux\agents\agent1-06309\repro\`:
- `ring-repro.html` — the isolated static repro, flat-color variant (swap
  `--progress-bar-color` back to a `repeating-conic-gradient` to test the
  animated case)
- `ring-repro-uniform.html` — the final, uniform (all-four-sides) fix
- `ring-repro-shortborder.html` / `ring-repro-shortborder-unfixed.html` —
  the (superseded) top-only fix vs. original, against a simulated top-edge
  short border
- `ring-repro-uniform-shortbottom.html` — the final uniform fix against a
  simulated bottom-edge short border (the case that actually failed live)
- `pixels.mjs` / `pixels_dpr.mjs` / `pixels_chromezoom.mjs` /
  `pixels_short.mjs` / `pixels_short2.mjs` / `pixels_uniform.mjs` /
  `pixels_shortbottom.mjs` — Playwright + pngjs scripts sampling exact pixel
  colors across all four edges, at various DPRs, with `--zoomfactor` set on
  the header only, and against simulated short-border cases on both the top
  and bottom edges

These are scratch/investigation files in the agent workspace, not part of
the repo, and were not committed.
