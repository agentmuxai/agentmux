# RCA: the pane header's "+" sits off centre vertically

**Date:** 2026-10-14 · **Author:** AgentA@Area54
**Status:** analysis — causes measured; fixes 1 and 2 of §5 built in #4933 (the "+" drawn as a 10px icon, tiles padded in whole pixels), fix 3 not built.
**Seen in:** the operator's 0.59.24 window (app zoom 0.85) and a local build at `40623c1ba` (#4888's head, Windows, CEF 154), measured over CDP.
**Code:** [`frontend/app/element/PaneTabStrip.tsx`](../../frontend/app/element/PaneTabStrip.tsx) (the button), [`frontend/app/element/PaneTabStrip.scss`](../../frontend/app/element/PaneTabStrip.scss) (`.pane-tab-strip-add`, `.pane-tab-strip-add-glyph`), [`frontend/app/mixins.scss`](../../frontend/app/mixins.scss) (`block-frame-default-header-layout`), [`frontend/layout/lib/tilelayout.scss`](../../frontend/layout/lib/tilelayout.scss) (`.tile-leaf` padding).

## 1. What the user sees

The "+" at the end of a pane's tab pills looks a little high, sometimes by a fraction of a pixel and sometimes clearly. It differs between panes in the same window and changes with the app zoom and the display's scaling.

## 2. Summary

Three things stack, and none of them is a wrong number in one place:

1. **The "+" is a text glyph** (`<span class="pane-tab-strip-add-glyph">+</span>`, Inter 700 at 15px), centred by its line box, not by its ink. Inter's "+" ink sits about **1.25 device px below** the centre of its line box at zoom 1 and 100% scaling.
2. **A fixed `transform: translateY(-1px)` compensates for that** (added in #2804 on 2026-08-25, measured once, when the strip was a floating overlay in the agent pane). Since #3309 (2026-09-17) the strip lives in the pane header, which is scaled by the app zoom (`zoom: var(--zoomfactor)`), and the nudge scales with zoom and display scaling while the font's error doesn't. At zoom 0.85 it overshoots, and the glyph ends up **high**.
3. **The pane chrome sits on fractional pixels, and boxes and text snap differently.** A tile leaf in a split is padded by half the 3px tile gap, **1.5px**, so every split pane's header starts on a half pixel. The app zoom adds more fractions (the 33px header is 28.05px at 0.85). Chromium paints box backgrounds (the tab pills, the header) snapped to whole device pixels, but places text from the fractional layout position. So the "+" and the pills next to it round in different directions, by up to half a pixel each. Which way they round depends on where the pane is, which is why it's "sometimes".

The DOM is sound: the strip, its Tooltip wrapper and the button all share one 28px height, and the header centres the strip in the space above its border. No caller uses the labelled variant (`addLabel`), so it isn't involved.

## 3. Evidence

### 3.1 The operator's window (0.59.24, app zoom 0.85)

From `UIQuery` and a screenshot of the agent pane:

| Element | top | height | centre |
|---|---|---|---|
| header (`.block-frame-default-header`) | 29.547 | 28.047 | 43.57 |
| strip, tab pill, "+" button | 31.172 | 23.797 | 43.07 |
| glyph line box (`.pane-tab-strip-add-glyph`) | 35.838 | 12.75 | 42.21 |

In the screenshot, the "+" ink spans rows 9–15 with its bar on rows 11–12 (centre ≈ 11.9), while the button box is centred at ≈ 14.4 on the same scale. The "+" is about **2.5px high** against its box, and about 1.5px high against the "AgentA" label beside it.

### 3.2 Where the half pixel comes from

Walking up from the button in the local build (zoom 1, 100% scaling): the button, Tooltip wrapper, strip and header-tabstrip all start at y = 235.5; the header at 233.5; the `.pane-stack` at 233.5; the **`.tile-leaf` at 232 with `padding-top: 1.5px`**. That padding is `calc(var(--gap-size-px) / 2)` with the default gap of 3px (`DefaultGapSizePx`, `layoutResize.ts`). Every tile is also its own composited layer, positioned by `transform: translate3d(x, y, 0)`.

### 3.3 The matrix

A probe over CDP screenshotted each pane's "+" at device resolution and found its ink centre, across app zoom (`--zoomfactor` on the root, which is what `store/zoom.ts` sets) and device scale factor (`Emulation.setDeviceMetricsOverride`). For the reference box, it painted the button solid and found its drawn edges, because that is what the eye compares the glyph with (the pills and header are painted the same way). Three panes, values in **device px, negative = high**:

| Scale | Zoom | Today (glyph + nudge) | Glyph, no nudge | Bars, no glyph | Bars + whole-pixel tile padding |
|---|---|---|---|---|---|
| 1 | 1 | −0.43 … −0.51 | +0.49 … +0.57 | −0.43 … −0.51 | +0.41 … +0.57 |
| 1 | 0.85 | **−1.27 … −1.39** | −0.27 … −0.39 | −0.48 … −0.59 | **−0.01 … +0.09** |
| 1 | 0.9 | −0.43 … −0.51 | +0.49 … +0.57 | −0.14 … −0.22 | −0.41 … −0.51 |
| 1.25 | 0.85 | **−1.04 … −2.22** | −0.04 … −1.27 | −0.50 … −1.19 | −0.79 … +0.79 |
| 1.5 | 0.85 | **−2.56 … −3.04** | −0.68 … −1.20 | −1.66 … −2.09 | **−0.14 … +0.03** |
| 2 | 1 | −0.51 … −1.79 | −0.78 … +0.12 | −0.51 … −2.00 | +0.14 … +0.98 |
| 2 | 0.85 | **−0.31 … −3.67** | −0.31 … −1.78 | −0.31 … −2.24 | **−0.02 … +0.19** |
| 2 | 0.9 | −0.02 … −0.82 | −0.02 … +1.32 | −0.02 … −0.51 | −0.32 … −2.14 |

- **Today's "+" is high in every case,** worst at zoom 0.85 with high display scaling (up to 3.7 device px, about 1.8 CSS px). That matches the operator's window.
- **Removing the nudge** alone makes the "+" sit about half a pixel low at zoom 1 and 0.9, and less high, but still high, at 0.85.
- **"Bars"** (the "+" drawn from two centred 1.5px bars in a 9px box, no font) removes the font's error, but still inherits the box-versus-content rounding.
- **Bars with whole-pixel tile padding** (`1px 2px 2px 1px`, same total gap) is within ±0.2 device px at zoom 0.85 at 1×, 1.5× and 2× scaling, and ±0.8 at 1.25×. At other zooms it is still off by up to about 1px (2px in one pane at 2× / 0.9). There the header's own heights are fractional (33 × 0.9 = 29.7, 28 × 0.9 = 25.2), so rounding still differs between the painted box and its content.

Per-pane differences at the same settings (up to 3.4 device px at 2× / 0.85) come from where each pane's composited tile layer lands: the tiles sit at different `translate3d` offsets, so their content rounds differently.

## 4. Why it wasn't caught

- The nudge was tuned by eye from a screenshot at one zoom and one scaling (#2804), in a context that was replaced three weeks later (#3309). Nothing re-measures it.
- No test can see this: jsdom has no layout, and nothing in CI renders the chrome at more than one zoom or scale.
- The error is small at 100% scaling and zoom 1 (half a pixel), where most checks happen.

## 5. Proposed fix

In order of value; 1 and 2 are small and independent.

1. **Draw the "+" as an icon, not a glyph:** two bars or an inline SVG, sized and centred by the box, with the `translateY(-1px)` nudge removed. This takes the font out of the alignment entirely (columns "Bars" in §3.3). The tab close "×" (`.pane-tab-close`) is a text glyph too and would benefit from the same treatment.
2. **Put tiles on whole pixels:** pad a leaf `floor(gap/2)` on top/left and `ceil(gap/2)` on bottom/right, so the total gap is unchanged and each pane's chrome starts on a pixel at zoom 1. Alternatively, make the default gap even (2 or 4px). With 1, this gives the last column of §3.3.
3. **Snap the header's zoomed sizes** (`--header-height`, the strip and button heights) to device pixels, using the existing `--snap-chrome` token (`theme.scss`, `mixins.snap-chrome()`), which already exists for exactly this inside the zoomed window header. This addresses the remaining error at zooms like 0.9 and 1.1. It touches every pane header, so it needs a visual pass at each zoom step.

**Verification for any fix:** rerun the §3.3 probe (method in §6) at scales 1, 1.25, 1.5 and 2 and zooms 0.85–1.1. The aim is every pane within ±0.5 device px of its painted box.

## 6. Method

Over CDP, on the dev window: for each `.pane-tab-strip-add`, capture `Page.captureScreenshot` with a clip starting at a CSS y whose device position is a whole pixel (a multiple of 4 works for all four scales). Within the button's own device rows, take the ink-weighted centre of rows that differ from the button background. With the button painted solid, take the coverage-weighted centre of the painted box from a column left of the glyph. The offset is ink centre minus painted centre. Zoom is varied through `--zoomfactor` on the root element, display scaling through `Emulation.setDeviceMetricsOverride`, and the nudge and alternatives through an injected stylesheet that is removed afterwards.
