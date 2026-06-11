# SPEC: Zero-Width Terminal Gap at All Zoom Levels

**Status:** Draft
**Author:** AgentX
**Date:** 2026-06-10
**Related:** PR #1330 (`agentx/fix-xterm-dead-space`), PR #1328 (scrollbar pointer cursor)
**Files:** `frontend/app/view/term/term.scss`, `frontend/app/view/term/termwrap.ts`, `frontend/app/view/term/termutil.ts`, `frontend/app/view/term/termViewModel.ts`, `frontend/app/block/blockframe.tsx`

---

## 1. Problem

A vertical strip of "dead space" appears to the right of the terminal text, between
the last column of glyphs and the right edge of the pane (where the scrollbar lives).

**The defining symptom:** the strip's width **changes with zoom level**. At some zoom
levels it is thin (but never zero); at others it is quite wide. The goal is a gap that
is visually **zero at every zoom level**.

This zoom-dependence is the diagnostic fingerprint of a **cell-quantization remainder**,
not a CSS layout bug.

---

## 2. Root cause

### 2.1 The irreducible remainder (fundamental to all terminal emulators)

A terminal renders text in a grid of fixed-width character cells. The rendered canvas
width is always an exact integer multiple of the cell width:

```
canvasWidth = cols × cellWidth
```

The container the canvas lives in (`.term-connectelem` → `.xterm` → `.xterm-viewport`)
is an arbitrary pixel width set by the flexbox layout. The leftover:

```
gap = containerWidth − (cols × cellWidth)     // always 0 ≤ gap < cellWidth
```

is a **sub-cell remainder that the terminal grid physically cannot fill** — you cannot
draw a partial column. This is confirmed by the xterm.js maintainers: the fit addon
"operates under the assumption that you control the font size yourself and would only
try to fill a given container to fully hold a certain grid size w/o partial row/column
drawing," and "cell fitting gaps can only be leveled out by some CSS adjusting rules,
the terminal/fit addon cannot fill that fully."

### 2.2 Why it varies with zoom

`cellWidth` is derived from the font metrics, which scale with the zoom factor and with
`window.devicePixelRatio`. As zoom changes, `cellWidth` changes to a new fractional
pixel value, so `containerWidth mod cellWidth` lands somewhere different in `[0, cellWidth)`
each time. Hence the strip is thin at some zooms, wide at others, and essentially never
exactly zero. Non-integer `devicePixelRatio` (any zoom ≠ 100%) adds a second layer of
sub-pixel rounding on top of this.

**Consequence: there is no integer column count that makes the pixel gap zero at all
zoom levels.** The +1-column trick (`term.resize(cols + 1, …)`) overshoots and clips
text into the scrollbar; the current correction undershoots and leaves a fractional gap.
Both are quantized and neither can hit zero across the continuous space of zoom levels.

### 2.3 Why PR #1330's current approach is insufficient

PR #1330 does two things:
- `term.scss`: scrollbar track → `transparent`, and `.xterm-viewport` → `overflow-y: overlay`.
- `termwrap.ts`: `CSS_SCROLLBAR_WIDTH = 0` → `FIT_WIDTH_CORRECTION = 14`, which adds
  `floor(14 / cellWidth)` columns back to the canvas.

`floor(14 / cellWidth)` is itself quantized — it adds ~1 whole column (≈ 8–10px), not a
precise 14px. So a fractional remainder always survives, and because `cellWidth` moves
with zoom, the surviving remainder is exactly the zoom-varying strip the user reports.
The column-correction strategy treats a continuous (pixel) problem with a discrete
(column) tool; it can reduce the average gap but never eliminate it across all zooms.

### 2.4 AgentMux-specific rendering architecture (verified)

The remainder strip is only *visible* because something of a contrasting color shows
through it. The relevant layering:

| Layer | Element | Background | Width |
|-------|---------|-----------|-------|
| Block inner | `.block-frame-default-inner` | theme bg via `blockBg` (`blockframe.tsx:758, 824`) | full pane |
| Term wrapper | `.term-connectelem` | none; `margin: 5px` (`term.scss:62`) | full minus margin |
| xterm root | `.xterm` / `.xterm-viewport` | none | 100% of wrapper |
| Canvas grid | `.xterm-screen` (canvas) | **transparent** (`#00000000`, `termutil.ts:70`) | `cols × cellWidth` |

Key facts:
- xterm's own canvas background is forced **transparent** (`termutil.ts:70`); the visible
  terminal background is actually painted by `.block-frame-default-inner` *behind* the
  transparent canvas.
- `term:transparency` **defaults to 0.5** (`termViewModel.ts:213`), so `blockBg` is the
  theme color at 50% alpha — meaning whatever sits *behind* the block (window surface /
  wallpaper / adjacent panes) tints the terminal, and any region where the layering
  differs will read as a different shade.

So the strip becomes visible whenever the remainder region exposes a layer whose
effective color differs from the glyph cells' background — e.g. the canvas paints a
selection/cursor/cell-bg layer the remainder lacks, the `5px` margin region, a border,
or a differently-composited transparency stack at the edge.

---

## 3. Goal (restated precisely)

> **A terminal whose right-edge gap is visually imperceptible at every zoom level
> (0.5×–2.0×) and every `term:transparency` setting.**

Literal zero *pixels* of remainder is unachievable without distorting glyphs (see §2).
Zero *perceptible* gap is achievable and is the correct target — it is what VS Code,
iTerm2, and Windows Terminal all ship.

---

## 4. Proposed solution

### 4.1 Primary: make the remainder region carry the exact glyph-cell background

Guarantee that every pixel from the last glyph column to the pane's right edge is painted
with the **same** effective color as the cell background, so the remainder is invisible
regardless of its width. Because the strip width is irreducible, this is the only
approach that holds across all zoom levels.

Concretely:
1. **Single source of background truth.** The element behind the canvas
   (`.block-frame-default-inner` today) must extend across the full pane width including
   the scrollbar lane and the sub-cell remainder, and must paint the terminal theme bg —
   not a panel/window color. Verify `.term-connectelem`'s `margin: 5px` (`term.scss:62`)
   does not expose a differently-colored parent in the remainder; if it does, move the
   margin to padding on the bg-painted element so the bg reaches the edge.
2. **Consistent transparency compositing.** With `term:transparency = 0.5`, the remainder
   and the cell area must composite over the *same* backdrop. If the canvas contributes
   any opaque/semi-opaque cell-bg layer the remainder lacks, the two regions diverge.
   Confirm the theme bg is applied at exactly one layer and the canvas stays fully
   transparent (`termutil.ts:70`) so cells and remainder share one compositing stack.

### 4.2 Secondary (optional): keep `overflow: overlay` + drop the column fudge

With §4.1 making the remainder invisible, the `FIT_WIDTH_CORRECTION = 14` column fudge in
`termwrap.ts` becomes unnecessary and arguably harmful (it changes the PTY column count
for cosmetic reasons, which can reflow TUIs). Options, in order of preference:

- **Keep `overflow: overlay`, set `FIT_WIDTH_CORRECTION = 0`** (revert `CSS_SCROLLBAR_WIDTH`
  to 14 so the correction nets to 0). The overlay scrollbar takes 0 layout space; the
  remainder is absorbed by §4.1's background. Cleanest: PTY columns match the true
  drawable width and nothing is faked.
- Keep the correction only if measurement shows the overlay genuinely reclaims the full
  14px and the column add lands within 1px — unlikely given §2.2.

### 4.3 Rejected alternatives

- **+1 column** (`resize(cols+1, …)`): overshoots, clips glyphs under the scrollbar. The
  exact regression history warns against this in `addon-fit` (issues #1284, #3867).
- **Snap container width to a multiple of `cellWidth`**: only moves the remainder outside
  the terminal into the parent — still needs §4.1's matching background to hide it there,
  and fights the flex layout. No net benefit over §4.1.
- **`transform: scaleX()` to stretch the canvas to full width**: distorts glyph aspect
  ratio and blurs text. Unacceptable.
- **Larger column fudge / heuristic px**: chases a moving target (§2.2); brittle across
  fonts and DPRs.

---

## 5. Investigation checklist (do before coding the fix)

Run `task dev`, open a terminal, open DevTools (View ▸ Toggle DevTools), and at 3+ zoom
levels (e.g. 0.8×, 1.0×, 1.3×):

1. Inspect the remainder strip. Identify the **topmost painted element** under the cursor
   in that strip (DevTools "inspect element"). Record its computed `background-color` /
   `background` and compare to a glyph cell's effective background.
2. Measure `.xterm-screen` width vs `.xterm-viewport` width vs `.term-connectelem` width.
   Confirm `gap = viewportWidth − screenWidth` and that it tracks `cellWidth`.
3. Read `cellWidth` live: `term._core._renderService.dimensions.css.cell.width` at each
   zoom. Confirm it changes and that `gap < cellWidth`.
4. Toggle `term:transparency` to 0 and re-check: if the strip vanishes at 0 but appears at
   0.5, the cause is compositing-layer divergence (§4.1.2), not a raw color mismatch.
5. Check whether the `5px` margin on `.term-connectelem` exposes `.block-frame-default-inner`
   vs a different parent at the right edge.

The fix in §4.1 should target whichever layer step 1 identifies as the contrasting one.

---

## 6. Test plan

For each zoom level in {0.5×, 0.8×, 1.0×, 1.25×, 1.5×, 2.0×} and each transparency in
{0, 0.5}:

- [ ] No perceptible strip to the right of glyphs when not hovering.
- [ ] On hover, the scrollbar thumb overlays content with no layout shift.
- [ ] Scroll with scrollback present — thumb visible and draggable; no glyphs clipped
      under the thumb.
- [ ] Resize the pane slowly through several widths — no flicker of a contrasting strip.
- [ ] Narrow pane (≤ 30 cols) — no regression.
- [ ] TUI app (e.g. `htop`, `vim`) reflows to the correct column count (verify
      `FIT_WIDTH_CORRECTION` change did not add/remove a column the PTY shouldn't have).
- [ ] Light/non-dark term theme — remainder matches the light bg, not a hardcoded dark.

Capture before/after screenshots at 1.0× and 1.3× (the two most divergent observed widths).

---

## 7. References

- [xterm.js #5299 — Fit addon does not fit exactly to parent element](https://github.com/xtermjs/xterm.js/discussions/5299)
  (maintainer: exact fit is "almost impossible to get done right"; font size defines both
  dimensions; fit only fills whole cells).
- [xterm.js #4853 — FitAddon is calculating wrong dimension](https://github.com/xtermjs/xterm.js/discussions/4853)
  (right gap = scrollbar reservation + sub-cell remainder, "working as intended").
- [xterm.js #3867 — fitAddon.fit() will cover the scroll bar](https://github.com/xtermjs/xterm.js/issues/3867)
  (column calc vs scrollbar reservation interaction).
- [xterm.js #1284 — Fit addon covers scrollbar](https://github.com/xtermjs/xterm.js/issues/1284)
  (history of +1/+2 column overshoot regressions).
- [xterm.js #2662 — Renderer blurry when window zoom changes](https://github.com/xtermjs/xterm.js/issues/2662)
  and [5.0.0 release notes](https://newreleases.io/project/github/xtermjs/xterm.js/release/5.0.0)
  (non-round `devicePixelRatio` sub-pixel rendering; fixed in 5.0.0; WebGL renderer preferred).
- [VS Code terminal appearance](https://code.visualstudio.com/docs/terminal/appearance) +
  community fix: set `terminal.background` == `panel.background` so the right-edge remainder
  is invisible — the canonical "make the gap match the bg" approach this spec adopts.
- [High DPI rendering on HTML5 canvas](https://cmdcolin.github.io/posts/2014-05-22/)
  (sub-pixel gap "fudge factor" for non-integer devicePixelRatio; why it's a hack).

---

## 8. Summary

The right-edge gap is a sub-cell quantization remainder, irreducible in pixels and
zoom-dependent because `cellWidth` scales with zoom/DPR. PR #1330's column correction
fights a continuous problem with a discrete tool and cannot reach zero. The correct,
zoom-stable fix is to make the remainder **invisible** by ensuring every pixel from the
last glyph column to the pane edge carries the exact glyph-cell background (§4.1), then
dropping the now-unneeded column fudge (§4.2). This matches how VS Code and other mature
terminals solve the identical problem.
