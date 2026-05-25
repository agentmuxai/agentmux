# Terminal jumbled on startup — investigation

**Symptom:** When loading a terminal pane (especially when starting Claude Code),
text is sometimes rendered jumbled — glyphs land in wrong positions, lines wrap
oddly, box-drawing is broken. Resizing the pane or zooming "clicks" it into
place. Intermittent: doesn't happen every time.

## Diagnosis

Classic xterm.js startup race. The init sequence in
`frontend/app/view/term/termwrap.ts` is:

| Step | Line | Action |
|------|------|--------|
| 1 | termwrap.ts:187 | `terminal.open(connectElem)` mounts xterm to the DOM |
| 2 | termwrap.ts:237 → 437 | `customFit()` calls `fitAddon.proposeDimensions()`, which **measures rendered cell width from the DOM** |
| 3 | termwrap.ts:238 | `sendTermSize()` tells the PTY the cols/rows |
| 4 | termwrap.ts:239 | `resyncController("init")` spawns the PTY |
| 5 | — | PTY emits output sized to whatever cols were computed in step 2 |

If the **`Hack` web font specified at term.tsx:174 isn't loaded yet** when step 2
runs, FitAddon measures using the fallback font's metrics. Result: wrong cell
width → wrong `cols` → PTY receives a stale size. Ink (which Claude Code uses)
emits ANSI cursor-positioning sequences computed against the size it was told,
but those positions don't match where glyphs actually land on screen → jumbled
text, broken box-drawing, mis-wrapped lines.

The reason resize/zoom "clicks it into place":

- The `ResizeObserver` at term.tsx:191 triggers `handleResize_debounced` →
  `customFit()`. By then fonts are loaded, dims are correct, `sendTermSize()`
  issues a SIGWINCH, and Ink redraws against the new size.
- If on the **WebGL renderer** (term.tsx:185 enables it by default when
  `term:disablewebgl` is unset), zoom also forces a texture-atlas regeneration —
  the WebGL atlas built with the fallback font has the wrong glyphs baked in
  until something invalidates it.

Intermittent because it's a font-load race: cold cache → fonts not ready at
first fit → jumbled. Warm cache → fonts ready synchronously → fine.

## Evidence — others hit the same pattern

- xterm.js #4338 — `proposeDimensions()` returns `NaN` when DOM/font isn't
  ready: <https://github.com/xtermjs/xterm.js/issues/4338>
- xterm.js #4830 — fonts render incorrectly, missing bits of glyphs:
  <https://github.com/xtermjs/xterm.js/issues/4830>
- xterm.js #4841 — FitAddon doesn't get full dims on first call; workaround is
  `resize(cols+1, rows+1)`: <https://github.com/xtermjs/xterm.js/issues/4841>
- xterm.js #4113 — devicePixelRatio / zoom mismatch causes wrong fit:
  <https://github.com/xtermjs/xterm.js/issues/4113>
- Hermes IDE #113 — exact same pattern; calls out that the WebGL renderer
  doesn't clear stale-cell ghost overlays until something forces a refresh:
  <https://github.com/hermes-hq/hermes-ide/issues/113>
- Claude Code #59163 — Ink garbling in VS Code's xterm.js, colors and columns
  intact but glyphs wrong:
  <https://github.com/anthropics/claude-code/issues/59163>

## Proposed fix

Three changes in `frontend/app/view/term/termwrap.ts`:

1. **Await fonts before first fit.** Insert `await document.fonts.ready;` (or
   load the Hack font explicitly via the `FontFace` API) between
   `terminal.open()` and `customFit()` around line 236, before the existing
   `customFit()` call at line 237.

2. **Guard NaN in `customFit()`.** At line 438, after `proposeDimensions()`,
   check `Number.isFinite(dims.cols) && Number.isFinite(dims.rows)`. The
   existing `if (!dims) return;` does not catch `{cols: NaN, rows: NaN}` because
   that object is truthy.

3. **One re-fit after first paint.** Schedule a second `customFit()` +
   `sendTermSize()` inside `requestAnimationFrame` after `init()`, so any
   remaining layout shift propagates as a SIGWINCH before the PTY produces
   meaningful output.

Together these eliminate the three known root causes: pre-font-load
measurement, NaN propagation, and post-mount layout shift.
