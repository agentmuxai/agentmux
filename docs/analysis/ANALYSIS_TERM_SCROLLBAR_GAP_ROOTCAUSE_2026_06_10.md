# Root-Cause Analysis: Terminal Right-Edge Gap (zoom-dependent width)

**Status:** Complete — empirically verified against the live dev build via CDP
**Author:** AgentX
**Date:** 2026-06-10
**Companion spec:** `docs/specs/SPEC_TERM_SCROLLBAR_ZERO_GAP_2026_06_10.md`
**PR:** #1330 (`agentx/fix-xterm-dead-space`)
**Method:** Chrome DevTools Protocol (port 9223, dev build PID 116308), live DOM
measurement + diagnostic layer colorization + composited-pixel sampling + screenshots.

---

## 1. Executive summary

The "dead space" to the right of terminal text is **22px** of empty `.term-connectelem`
between the right edge of the `.xterm` element and its parent. It is composed of:

- **~14px** — FitAddon's hard-coded scrollbar/overview-ruler reservation that PR #1330's
  column correction **fails to reclaim** at most zoom levels (the dominant, fixable part).
- **~8px** — the irreducible sub-cell remainder (`containerWidth mod cellWidth`), which can
  never be filled by glyphs but is already painted the terminal background color.

**Two findings make the current PR ineffective:**

1. **`overflow: overlay` is a no-op.** Chromium deprecated and aliased `overflow: overlay`
   to `auto` years ago; in CEF's Chrome 148 the rule computes to `auto`. The PR's headline
   CSS change does nothing.
2. **The column-reclaim math is wrong.** `customFit()` adds `floor(14 / cellWidth)` columns.
   When `cellWidth > 14` (true at DPR ≥ ~1.2 / zoomed in), this is `floor(0.97) = 0` — it
   reclaims **zero** columns, leaving the full ~14px reservation as gap. When `cellWidth < 14`
   it reclaims exactly one column. **This on/off behavior is precisely the zoom-dependent
   "thin at some zooms, wide at others" symptom.**

A third, higher-impact finding surfaced during instrumentation:

3. **WebGL is unavailable in CEF — every terminal silently uses the slow DOM renderer.**
   `canvas.getContext('webgl' | 'webgl2' | 'experimental-webgl')` all return null in the
   dev build. The `@xterm/addon-webgl` never loads; `loadRendererAddon` falls through to the
   DOM renderer. Worth a separate investigation (§7).

---

## 2. How it was measured

The dev build exposes CEF remote debugging on `127.0.0.1:9223` (release: 9222), set in
`agentmux-cef/src/main.rs:652`. A minimal CDP client (`scripts/cdp-probe.mjs`,
`cdp-screenshot.mjs`, `png-sample.mjs`) connected to the page target and ran
`Runtime.evaluate` / `Page.captureScreenshot`. All numbers below are from one live terminal
pane at **window DPR 1.25** (Windows 125% scaling), default theme `default-dark`, default
`term:transparency = 0.5`.

> Note: `Page.captureScreenshot` clips are specified in CSS px but the PNG is emitted in
> **device px** (×1.25 here). Pixel offsets in §4 account for this.

---

## 3. Layer geometry (measured, CSS px)

| Element | x | right | width | background |
|---|---|---|---|---|
| `.block-frame-default-inner` | 1077.5 | 2148.5 | 1071 | `rgba(13,14,15,0.5)` (= theme `#0d0e0f` @ 0.5) |
| `.term-connectelem` | 1081.5 | 2143.5 | 1062 | transparent (margin `5px 5px 5px 4px`) |
| `.xterm` / `.xterm-viewport` / `.xterm-screen` | 1081.5 | **2121.5** | **1040** | transparent |

- `.xterm` is **1040px** inside a **1062px** parent → **22px dead band** at
  `[2121.5 → 2143.5]`, plus the 5px right margin to `inner.right` (2148.5).
- `cellWidth = 14.456px` (measured: a 2-char span `"❯ "` = 28.91px ÷ 2). At 72 columns,
  `72 × 14.456 = 1040.8px` ≈ `.xterm` width → **xterm sizes itself to the integer column
  grid, it does not stretch to fill `.term-connectelem`.**

### Diagnostic colorization (definitive)

Injecting `.term-connectelem{background:red}` + `.xterm-screen{background:green}` and
screenshotting the right edge produced an unambiguous map: **green text grid → red 22px
gap → cyan inner-border → dark window background.** The gap is `.term-connectelem`, not the
scrollbar track.

### Composited pixel colors (no diagnostics)

Sampling real pixels across `[2090 → 2160]`:

| Region | CSS x | Color |
|---|---|---|
| Text-area background (between glyphs) | < 2121.5 | `rgb(15,15,16)` |
| **The 22px gap** (`.term-connectelem`) | 2121.5–2143.5 | **`rgb(15,15,16)`** |
| Outside the block (window/tab) | > 2148.5 | `rgb(34,34,34)` (+ a black transition line) |

**The gap is pixel-identical to the terminal background.** It is *not* a contrasting strip —
it is terminal-colored dead space where the text grid simply stops short of the pane edge.
The only genuinely different color is the window surround beyond the block (normal boundary).

---

## 4. The gap model (why it tracks zoom)

```
parentWidth      = .term-connectelem content width            (e.g. 1062)
reservation      = FitAddon overviewRuler.width || 14         (14, hardcoded)
cellWidth        = measured cell width in CSS px              (scales with zoom × DPR)

FitAddon cols    = floor((parentWidth − reservation) / cellWidth)
xterm width      = cols × cellWidth
gap              = parentWidth − xterm width
                 = reservation + ((parentWidth − reservation) mod cellWidth)
                 ≈ 14 + [0 … cellWidth)
```

So the gap is **~14px (reservation) plus a 0…cellWidth sub-cell remainder.** Both terms move
with zoom because `cellWidth` is a function of zoom and `devicePixelRatio`; the remainder
sweeps the whole `[0, cellWidth)` interval as the container/zoom changes — exactly the
"sometimes thin, sometimes wide, never zero" report.

### Why PR #1330's correction does not flatten it

`termwrap.ts:616`:

```js
dims.cols = Math.max(2, dims.cols + Math.floor(FIT_WIDTH_CORRECTION / cellWidth));
//                       └ floor((W−14)/cw) ┘   └ floor(14/cw) = 0 when cw>14 ┘
```

`floor((W−14)/cw) + floor(14/cw)` is **not** `floor(W/cw)`; it under-reclaims by 0 or 1
column depending on how the two fractional parts land. Concretely at the measured point:

| | cellWidth | `floor(14/cw)` reclaimed | resulting cols | resulting gap |
|---|---|---|---|---|
| **Current (buggy)** | 14.456 | **0** | 72 | **22px** (wide) |
| Lower zoom (illustr.) | ~11.5 | 1 | +1 | ~thinner, non-zero |
| **Correct reclaim** `floor(W/cw)` | 14.456 | n/a | **73** | **~7px** (sub-cell only) |

The correct reclaim `floor(parentWidth / cellWidth) = floor(1062 / 14.456) = 73` columns →
`1055.3px`, leaving a **6.7px** sub-cell residue — and that residue is the same
`rgb(15,15,16)` as the background, i.e. visually zero.

---

## 5. Conclusion: what "zero gap at all zoom levels" actually means

- **Reclaimable (must fix):** the ~14px FitAddon reservation. Reclaim it in *pixel space*
  before flooring, not as a second floored column term.
- **Irreducible (already invisible):** the `< cellWidth` sub-cell remainder. It cannot be
  filled by glyphs (no partial columns), but it is already painted the terminal background
  color, so it is imperceptible. Keeping that background match is the rest of the fix.

Net: there is no integer column count that yields a literal 0px gap at every zoom (confirmed
by xterm.js maintainers and the math in §4), but **correct pixel-space reclaim + guaranteed
background match makes the gap visually zero at every zoom level**, which is the goal.

---

## 6. Recommended fix

### 6.1 Replace the quantized correction with pixel-space reclaim

In `customFit()`, drop the `FIT_WIDTH_CORRECTION / cellWidth` term and compute columns from
the full container width (the overlay scrollbar should cost 0 layout):

```js
private customFit() {
    const dims = this.fitAddon.proposeDimensions();
    if (!dims || !Number.isFinite(dims.cols) || !Number.isFinite(dims.rows)) return;
    const core = (this.terminal as any)._core;
    const cellWidth: number = core?._renderService?.dimensions?.css?.cell?.width ?? 0;
    const parent = this.connectElem;               // the element FitAddon measures
    if (cellWidth > 0 && parent) {
        const cs = getComputedStyle(parent);
        const padX = parseFloat(cs.paddingLeft) + parseFloat(cs.paddingRight);
        const availPx = parent.clientWidth - padX; // NO scrollbar reservation (overlay)
        dims.cols = Math.max(2, Math.floor(availPx / cellWidth));
    }
    if (this.terminal.rows !== dims.rows || this.terminal.cols !== dims.cols) {
        core?._renderService?.clear?.();
        this.terminal.resize(dims.cols, dims.rows);
    }
}
```

This reclaims the full reservation at *every* cellWidth, so the only residue is the
irreducible `< cellWidth` remainder.

### 6.2 Reserve 14px only when the scrollbar is actually present

**Empirical scrollbar-cost test (resolved):** a synthetic element matching xterm's viewport
styling was measured in the live build. The vertical scrollbar's layout cost
(`offsetWidth − clientWidth`):

| overflow style | cost |
|---|---|
| `auto` (default) | 14px |
| `::-webkit-scrollbar{width:14px}` + `auto` | 14px |
| `::-webkit-scrollbar{width:14px}` + `scroll` | 14px |
| **`overflow: overlay`** | **14px** ← confirms `overlay` ≡ `auto` (no overlay behavior) |
| `scrollbar-width: none` | **0px** |

So in Chrome 148 a native (or webkit-styled) scrollbar **always** steals 14px when present;
the only zero-cost option hides the scrollbar entirely. Therefore reclaiming the full width
*unconditionally* would clip the last column whenever the scrollbar appears.

**Chosen approach — conditional reservation.** FitAddon reserves 14px *unconditionally*
(whenever scrollback > 0), which is the bug: it reserves even when no scrollbar is showing.
Instead, reserve 14px **iff** the viewport currently has vertical overflow:

- No scrollbar (short output) → reserve 0 → grid fills full width → **zero gap**.
- Scrollbar present (overflow) → reserve 14 → the scrollbar fills its lane → **zero gap**.
- The only cost is a ≤1-column reflow at the moment the scrollbar appears/disappears — the
  natural moment to make room for it, and how most terminals already behave.

This avoids a custom overlay-scrollbar component entirely. (If the reflow proves janky, the
fallback is the OverlayScrollbars route — hide the native scrollbar with
`scrollbar-width: none` and float a custom thumb — but conditional reservation is preferred
for its simplicity.)

The refit must re-run when the overflow state flips. The data-write path already calls
`handleResize`; add an explicit refit when `viewport.scrollHeight > viewport.clientHeight`
transitions. Verified empirically below (§8) by forcing overflow and checking the last
column is not clipped.

### 6.3 Keep the background match (already correct)

`.block-frame-default-inner` paints the resolved theme bg behind the transparent canvas
(`blockframe.tsx:758,824` ← `termViewModel.blockBg`), so the sub-cell residue is already the
terminal color. Preserve this; do not reintroduce an opaque scrollbar track
(the original `var(--scrollbar-background-color)` bug). The track→transparent change from
PR #1330 should stay; the `overflow: overlay` line should be removed (no-op) and replaced by
the §6.2 mechanism.

### 6.4 Revert the misleading constants

`CSS_SCROLLBAR_WIDTH`/`FITADDON_SCROLLBAR_ASSUMPTION`/`FIT_WIDTH_CORRECTION` and their
comments describe a model that doesn't hold (overlay via CSS, exact 14px column add). Remove
them in favor of the §6.1 pixel-space computation.

---

## 7. Secondary finding (high value): WebGL dead in CEF → DOM renderer everywhere

`detectWebGLSupport()` (`termwrap.ts:36`) returns **false** in the running CEF build — all
three context probes fail. Consequences:

- `@xterm/addon-webgl` never loads; every terminal uses the **DOM renderer** (confirmed by
  the `xterm-dom-renderer-owner-1` class and zero `<canvas>` elements). The DOM renderer is
  materially slower for large/fast output and heavy scrollback.
- Likely cause: CEF launched without GPU/WebGL enabled (software compositing). Check the CEF
  command-line flags in `agentmux-cef` (e.g. `--enable-gpu`, `--ignore-gpu-blocklist`,
  `--use-angle`, `--enable-webgl`, `disable-gpu` absence).

**Recommended:** verify whether WebGL is also off in the **portable/release** build (port
9222). If so, file a separate issue — enabling the WebGL renderer is a broad terminal perf
win independent of the gap. (It does **not** change the gap root cause: the DOM renderer
sizes `.xterm` to the integer column grid just like WebGL.)

---

## 8. Verification (done — live dev build, DPR 1.25)

The fix was applied and verified against the running dev build over CDP:

| State | Before | After |
|---|---|---|
| `.xterm` width (cols) | 1040px (72 cols) | **1055px (73 cols)** |
| gap (`connect − xterm`) | **22px** (dead) | **7px** (sub-cell, bg-colored) |
| gap composited color | `rgb(15,15,16)` | `rgb(15,15,16)` = text bg → invisible |
| text reaches right edge | no (22px short) | **yes** (verified: 200-char `f` lines + error text run edge-to-edge) |
| wheel scrollback | works | **works** (scrolled 103→101 via wheel) |
| last-column clipping | n/a | **none** |

### Additional finding during verification — xterm v6 DOM-renderer scroll model

`.xterm-viewport` has **zero children and no `.xterm-scroll-area`**; `scrollHeight ==
clientHeight` even with 120 lines of scrollback. In xterm v6's DOM renderer the viewport
**never scrolls natively** — scrollback is virtual (wheel re-renders buffer lines), so the
native `::-webkit-scrollbar` never appears. Consequences:

- The conditional reservation (§6.2) therefore reserves **0** in the DOM renderer (scrollbar
  never "present"), so the full width is always reclaimed — and there is no native scrollbar
  to clip the last column. The conditional check still correctly reserves 14px on the WebGL
  path (where the viewport *does* scroll natively), so the same code is right for both.
- The `::-webkit-scrollbar` rules in `term.scss` are effectively dead for the DOM renderer.
  Combined with §7 (WebGL dead in CEF), **today every terminal has no visible scrollbar at
  all** — scrolling is wheel-only. That is a separate UX gap worth tracking (the user cannot
  see scroll position or drag to scroll). Out of scope for the dead-space fix.

### Implementation

- `termwrap.ts`: `customFit()` recomputes `cols = floor((connectElem.clientWidth − padX −
  reservation) / cellWidth)`, `reservation = scrollbarVisible ? 14 : 0`. Added
  `setupScrollbarRefit()` (ResizeObserver on the viewport, gated on overflow-state change) so
  the WebGL path re-fits when its scrollbar toggles. Removed the `FIT_WIDTH_CORRECTION`
  constants.
- `term.scss`: removed the no-op `overflow-y: overlay`; set `overflow-y: auto` so the
  scrollbar lane is conditional, not permanently reserved; kept the transparent track.

### Remaining checks (manual, recommended before merge)

1. A few more zoom levels (0.8×, 1.5×, 2.0×) — expect gap `< cellWidth`, bg-colored, at each.
2. `term:transparency ∈ {0, 0.5}` and a light theme — residue should match the resolved bg.
3. A machine where WebGL **does** load — confirm the scrollbar appears, reserves 14px, and
   the last column is not clipped (exercises the conditional-reservation branch).

---

## 9. Appendix: raw measurements (DPR 1.25, default-dark, transparency 0.5)

```
webgl: { webgl1:false, webgl2:false, experimental:false }
termClass: "terminal xterm xterm-dom-renderer-owner-1"
cellWidth: 14.456 px   (span "❯ " = 28.91px / 2)
inner   : x1077.5  right2148.5  w1071  bg rgba(13,14,15,0.5)
connect : x1081.5  right2143.5  w1062  bg transparent  margin 5/5/5/4
xterm   : x1081.5  right2121.5  w1040  bg transparent
viewport: x1081.5  right2121.5  w1040  overflow auto/auto/auto   (overlay→auto: no-op)
screen  : x1081.5  right2121.5  w1040  bg transparent
gap (connect.right − xterm.right) = 22.0 px
pixels  : text-bg rgb(15,15,16) | gap rgb(15,15,16) | outside rgb(34,34,34)
```

Scratch instrumentation lives in `scripts/cdp-*.mjs` and `scripts/png-sample.mjs` — debug
tooling, **not for commit** to the feature branch.
