# Retro: gap between the 1px progress ring and the pane border, one edge at a time

**Date:** 2026-09-19
**Severity:** Low — cosmetic, reported on the agent pane's busy-indicator ring.
**Status:** fixed, on the third attempt. `frontend/app/view/agent/agent-view.scss`
was changed (`.agent-pane-progress-bar-slot`); see "Fix v2" below for what
actually shipped. Earlier revisions of this document described this as
investigation-only with no fix applied (true for the first pass,
all-negative geometry/DPR results), then as fixed with a top-edge-only
overlap (true for that specific edge, on that specific live test, but too
narrowly scoped — see "First fix attempt was too narrow"), then as fixed by
widening the overlap to all four edges (v1: closed the gap on every edge
tested, but ReAgent's PR review caught a real regression in that approach —
see "Fix v1 was caught by review" below — superseded by v2, which removes
the regression and needs no overlap-and-hope at all).

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

## Fix v1 (superseded) — uniform overlap on all four sides

`frontend/app/view/agent/agent-view.scss`, two paired changes, applied
uniformly on all four sides:

1. `.agent-pane-progress-bar-slot`: inset changed from `2px` to `1px`
   (all sides) — pulls the ring 1px closer to the true pane edge, everywhere.
2. `.agent-pane-progress-bar`: padding (ring thickness) changed from `1px`
   to `2px` (all sides) — widens the ring's own band by 1px on every side,
   so the ring's visible position doesn't move (nothing shifts toward the
   content side).

Verified with the same Playwright + pngjs repro methodology: pixel-identical
to the unfixed CSS in the clean-rounding case, and closed a simulated
1-device-pixel-short border on both the top and bottom edges. Pushed as
PR #3443.

### Fix v1 was caught by review

ReAgent's review on PR #3443 ([P1], `agent-view.scss:326`) identified a real
regression this repro methodology missed: v1's "invisible in the common
case" claim depends on the focus ring being opaque enough to fully cover the
extra pixel. It isn't, in two very common states:

- **Default/unfocused**: `--border-color: rgba(255, 255, 255, 0.16)`
  (`theme.scss`) — a 16%-opacity white. The extra pixel shows through,
  alpha-blended, not hidden.
- **Focused-alone** (a single pane, not part of a split — arguably the most
  common way to watch one agent work): `.pane-stack-focused-alone::after {
  border-color: transparent; }` (`PaneChrome.scss`). Nothing covers the
  extra pixel at all.

Both were missed because every repro variant up to this point used
`.pane-stack.focused` for visibility while sampling pixels — the one state
where the border genuinely is opaque. Confirmed the reviewer's claim
directly against the cited lines before responding (`grep` on both files —
exact values matched).

## Fix v2 (shipped) — match the border's own box-model mechanism

Root issue underlying both the original bug and v1's flawed patch: `inset:
2px` (an absolute-position offset) and `.pane-stack::after`'s `border: 2px
solid` (a border-width subtraction) are two *different* CSS box-model
computations that both nominally mean "2px" but are not guaranteed to round
to the same device pixel. v1 tried to paper over an occasional disagreement
between them with an overlap margin; v2 removes the disagreement itself by
having `.agent-pane-progress-bar-slot` ask the layout engine to solve the
exact same problem `.pane-stack::after` does:

```scss
.agent-pane-progress-bar-slot {
    inset: 0;
    border: 2px solid transparent;
    // ...
}
```

Instead of `inset: 2px`. The child `.agent-pane-progress-bar` (`inset: 0`
relative to this element) resolves against the *padding box* per spec —
i.e. automatically inside the transparent border, same as it sat inside the
old `inset: 2px` — so `.agent-pane-progress-bar`'s own rule needed no change
and its `padding: 1px` (ring thickness) reverted to what it was before v1.
Net effect: no extra pixel exists anywhere to hide, so the translucent- and
transparent-border regression v1 introduced cannot occur, by construction —
not by relying on opacity.

## Validation (v2)

- **All three real border states** (opaque/focused, translucent/default,
  transparent/focused-alone): ring stays exactly 1px thick in every case,
  confirmed by sampling `ring-repro.html` (focused), `ring-default.html`
  (default `.pane-stack`, translucent border), and
  `ring-focused-alone.html` (`.pane-stack.focused-alone`, transparent
  border) — no thickening, no color bleed-through in any of them.
- **DPR 1/1.25/1.5/1.75**, all four edges: pixel-identical to the original
  pre-v1 CSS (border, then 1px ring, then background) at every combination
  tested.
- Not independently re-tested against the exact live short-rounding
  scenario from before (v1's `ring-repro-uniform-shortbottom.html` doesn't
  carry over cleanly, since v2's fix works by eliminating the mismatched
  computation rather than by margin — there's no longer a meaningful way to
  "simulate a disagreement" between two rules that now literally ask for
  the same thing). Recommend re-verifying live at whatever zoom/display-scale
  combination previously showed the gap, same as v1's own follow-up ask.

## Repro artifacts

`C:\Users\asafe\.agentmux\agents\agent1-06309\repro\`:
- `ring-repro.html` — isolated static repro, flat-color variant, focused
  state (opaque border); currently reflects the v2 fix
- `ring-default.html` / `ring-focused-alone.html` — v2 fix under the real
  translucent (default) and transparent (focused-alone) border states, the
  cases v1 got wrong
- `ring-repro-uniform.html`, `ring-repro-uniform-shortbottom.html`,
  `ring-repro-shortborder.html` / `-unfixed.html` — v1 (superseded)
  artifacts, kept for the record of what was tried and why it wasn't enough
- `pixels*.mjs` — Playwright + pngjs scripts backing all of the above
  (`pixels_borderstates.mjs` and `pixels_v2_dpr.mjs` are v2's)

These are scratch/investigation files in the agent workspace, not part of
the repo, and were not committed.
