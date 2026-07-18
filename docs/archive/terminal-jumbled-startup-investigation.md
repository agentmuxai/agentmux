> **Archived 2026-07-17:** Resolved. The font-load race described here shipped as
> `document.fonts.load()` in `termwrap.ts` (PRs #1030, #1040). Kept for historical
> reference only — do not treat as current behavior.

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

## Follow-up — PR #1030 was insufficient (2026-05-25)

After PR #1030 merged, the jumbled-glyph repro **resurfaced** when opening
multiple terminal panes in quick succession on a cold cache. Diagnosis:

`await document.fonts.ready` is **not** the right primitive. Per the CSS
Font Loading spec, `fonts.ready` returns a Promise that resolves when no
currently-pending font loads remain. It does **not** request specific fonts.
Browsers load web fonts lazily — the WOFF/WOFF2 download for `Hack` is only
kicked off when something actually needs to render a glyph in that family.

In `TermWrap.init()` the sequence was:

1. `terminal.open()` mounts the xterm DOM with `font-family: Hack`. The
   browser sees the rule but **does not request `Hack` yet** — nothing has
   asked it to paint a glyph in that family.
2. `await document.fonts.ready` — the FontFaceSet has zero pending loads
   *at this instant*, so the Promise resolves immediately.
3. `customFit()` → `proposeDimensions()` measures cell width. The
   measurement step is what finally triggers `Hack` to be requested, but
   the measurement returns synchronously using fallback metrics.
4. Wrong `cols` → `sendTermSize()` informs PTY of wrong size →
   `resyncController("init")` spawns shell → Ink emits cursor sequences
   sized to the wrong cols → jumbled.

The rAF re-fit a frame later would catch this *if* the `Hack` request
completed within that frame, but on cold cache it typically takes
20–300ms. By then Ink has already painted corrupted output that survives
the eventual SIGWINCH redraw (cursor-positioning sequences already issued
against the wrong dimensions cannot be undone by resizing).

When opening multiple terminals in parallel, each `TermWrap.init()` hits
the same race independently. After the first successful Hack load, the
browser caches the face and later terminals measure correctly — but the
*first* terminal (and any others racing alongside it) lose.

### Proper fix

Replace `await document.fonts.ready` with
`await document.fonts.load(`${fontSize}px "${fontFamily}"`)`. The `load()`
method **actively requests** the named face and resolves only when that
specific face is ready. Subsequent calls in parallel terminals coalesce
on the browser's font cache, so opening N panes pays the network cost
once.

We also load `bold` and `italic` variants in parallel, because xterm.js
renders bold/italic cells with separate font faces. Without those
pre-loads, the first bold or italic glyph triggers another lazy fetch
and the same race recurs at smaller scale.

The bounded timeout (1s) and the rAF re-fit afterwards are kept as
belt-and-suspenders.

```ts
const FIT_FONT_TIMEOUT_MS = 1000;
const fontFamily = this.terminal.options.fontFamily ?? "Hack";
const fontSize = this.terminal.options.fontSize ?? 12;
const fontSpec = (variant: string) => `${variant}${fontSize}px "${fontFamily}"`;
try {
    await Promise.race([
        Promise.all([
            document.fonts?.load(fontSpec("")) ?? Promise.resolve(),
            document.fonts?.load(fontSpec("bold ")) ?? Promise.resolve(),
            document.fonts?.load(fontSpec("italic ")) ?? Promise.resolve(),
        ]),
        new Promise<void>((resolve) => setTimeout(resolve, FIT_FONT_TIMEOUT_MS)),
    ]);
} catch (_) { /* font API unavailable or face unknown — fall through */ }
```

### Other xterm mounts not yet touched

`AgentInstallModal.tsx` constructs its own `new Terminal()` with its own
`FitAddon` and calls `fitAddon.fit()` immediately, with no font-load wait.
The install modal renders short ANSI status lines, not full TUIs, so the
practical impact is small — but the same race exists. Fix at the same
time or in a follow-up; the pattern is identical.

### References (additions)

- MDN — `FontFaceSet.load()`:
  <https://developer.mozilla.org/en-US/docs/Web/API/FontFaceSet/load>
  *"Resolves once the requested fonts have all loaded."*
- CSS Font Loading Module Level 3, §3.4 The FontFaceSet Interface:
  <https://drafts.csswg.org/css-font-loading-3/#font-face-set-interface>
- xterm.js #4830 — multiple users hit this exact pattern with web fonts;
  consensus workaround is `document.fonts.load()` before `fit()`.
