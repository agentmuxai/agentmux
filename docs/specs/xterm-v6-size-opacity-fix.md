# xterm v6: Terminal Size & Opacity Regression Fix

## Bug 1 — Terminal Is Slightly Too Small (Width)

### Root Cause

The upstream `@xterm/addon-fit` v0.11.0 `proposeDimensions()` subtracts a fixed right-side
reserve from available width:

```js
// From addon-fit.js source
const t = scrollback === 0 ? 0 : terminal.options.overviewRuler?.width || 14;
availableWidth = parentWidth - termPadding - t;
```

When `scrollback > 0` (our default is 2000) and `overviewRuler` is not configured,
`overviewRuler?.width` is `undefined`, so `undefined || 14 = 14`. **14px is always
subtracted from the available column width.**

The old custom `fitaddon.ts` (deleted in PR #251) read the actual xterm scrollbar width
from `core.viewport.scrollBarWidth`, which matched our CSS scrollbar (6px). Net delta: **8px
per terminal instance**, equivalent to roughly 1 character at typical font sizes.

Our webkit scrollbar is absolutely-positioned and doesn't consume layout space. The correct
reserve is 0 (or 6 to match our CSS scrollbar). FitAddon's hardcoded 14px is wrong for our
setup.

### Fix

Wrap `fitAddon.fit()` in `termwrap.ts` with a corrected `fit()` that calls
`proposeDimensions()` and re-adds the pixel difference:

```ts
// In handleResize() and init(), replace fitAddon.fit() with customFit()
private customFit() {
    const dims = this.fitAddon.proposeDimensions();
    if (!dims) return;
    // FitAddon subtracts 14px for overview ruler / Monaco scrollbar.
    // Our CSS webkit scrollbar is 6px (hidden until hover) and overlaps the content.
    // Add back the 8px discrepancy as extra columns.
    const cellWidth = (this.terminal as any)._core._renderService.dimensions.css.cell.width;
    if (cellWidth > 0) {
        const extraCols = Math.floor(8 / cellWidth);
        dims.cols = Math.max(2, dims.cols + extraCols);
    }
    if (this.terminal.rows !== dims.rows || this.terminal.cols !== dims.cols) {
        (this.terminal as any)._core._renderService.clear?.();
        this.terminal.resize(dims.cols, dims.rows);
    }
}
```

**Callers to update:** `handleResize()` and `init()` — replace both `this.fitAddon.fit()`
calls with `this.customFit()`.

**Alternative (simpler, less precise):** Change the webkit scrollbar CSS from 6px to 14px so
the FitAddon expectation matches reality. Downside: slightly wider scrollbar UI.

---

## Bug 2 — Opacity/Transparency Broken

### Root Cause

Two separate issues combine to make transparency non-functional.

**Issue A — `.xterm-viewport` hardcoded black background (xterm.css)**

In xterm v5, `xterm.js` dynamically set `.xterm-viewport { background-color }` from the
theme via `viewport._viewportElement.style.backgroundColor`. The fallback in `xterm.css` was
`background-color: #000` for the scrollbar track area on macOS.

In xterm v6, background color is set on the **scrollable element** (a different DOM node):
```js
this._scrollableElement.getDomNode().style.backgroundColor = n.colors.background.css
```

This inline style is applied to `.xterm-scrollable-element`, not `.xterm-viewport`. The
`.xterm-viewport` CSS rule (`background-color: #000` in our `xterm.css`) is now the **only**
background applied to that element — inline style no longer overrides it.

Since `.xterm-viewport` is `position: absolute` filling the terminal container and sits
behind the rendering canvas, its solid black background blocks everything behind it,
including the block's `blockBg` (the semi-transparent color from `computeTheme()`).

Even when the xterm WebGL/Canvas renderer renders background cells as transparent
(`allowTransparency: true` → `NULL_COLOR` for background glyphs), the viewport div itself
is still solid black beneath them.

**Issue B — `allowTransparency: true` with WebGL may not clear correctly**

In the WebGL addon, background clearing uses:
```js
alpha || this._clearAll()
```
where `alpha` is derived from `allowTransparency`. If the internal `_alpha` flag is not
being set, background cells won't be cleared to transparent and the WebGL canvas itself
will be opaque.

This needs verification in the running app — check the `[fe]` logs for
`"loaded webgl renderer!"` and confirm `allowTransparency` is being passed through.

### Fix

**Fix A (required):** In `xterm.css`, change the viewport background:

```css
/* Before */
.xterm .xterm-viewport {
    background-color: #000;
    overflow-y: scroll;
    ...
}

/* After */
.xterm .xterm-viewport {
    background-color: transparent;  /* xterm v6 sets background on .xterm-scrollable-element */
    overflow-y: scroll;
    ...
}
```

The scrollbar track color is already handled by our custom `-webkit-scrollbar-track` rule
(`var(--scrollbar-background-color)`), so removing the hardcoded `#000` does not affect
the scrollbar appearance.

**Fix B (verify first):** Confirm `allowTransparency: true` is reaching the WebGL addon
config. In `termwrap.ts`, `loadRendererAddon(useWebGl)` calls:
```ts
const webglAddon = new WebglAddon();
```
The WebglAddon reads `allowTransparency` from the terminal options service directly —
no extra wiring needed. If issue B persists after fix A, add a
`console.log("allowTransparency:", this.terminal.options.allowTransparency)` after
`terminal.open()` to confirm the option is set.

---

## Implementation Order

1. `xterm.css`: `background-color: transparent` on `.xterm-viewport` → test opacity
2. `termwrap.ts`: `customFit()` wrapper replacing `fitAddon.fit()` → test terminal width
3. Verify both in `task dev` before branching/bumping

## Files to Change

| File | Change |
|------|--------|
| `frontend/app/view/term/xterm.css` | `.xterm-viewport { background-color: transparent }` |
| `frontend/app/view/term/termwrap.ts` | Replace `fitAddon.fit()` calls with `customFit()` |
