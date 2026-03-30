# Spec: CEF Transparency Architecture

**Date:** 2026-03-29
**Status:** Analysis complete, implementation needed

---

## Current Rendering Stack (Tauri)

From innermost to outermost:

```
Layer 7 (top)    xterm text glyphs                 — opaque
Layer 6          xterm theme.background             — set to #00000000 (fully transparent)
Layer 5          .xterm-viewport                    — CSS: background-color: transparent
Layer 4          .xterm-scrollable-element           — xterm v6 inline style (theme bg)
Layer 3          .block-frame-default-inner          — CSS: var(--block-bg-color)
                 OVERRIDDEN by blockBg from          — inline style from computeBgStyleFromMeta()
                 TermViewModel                        (semi-transparent theme bg color)
Layer 2          AppBackground div                   — computeBgStyleFromMeta(tabData?.meta, 0.5)
                                                      (tab wallpaper/gradient/color)
Layer 1          body                                — rgba(34,34,34, var(--window-opacity))
Layer 0          html                                — transparent (when window:transparent=true)
Layer -1         Browser/Window background           — Tauri: transparent via platform API
                                                      CEF: 0xFF000000 (opaque black)
```

---

## How Terminal Transparency Works

### `computeTheme()` — `termutil.ts:13-34`

1. Loads the terminal theme (e.g., `default-dark` with `background: "#1e1e1e"`)
2. If `termTransparency > 0`:
   - Applies alpha to theme bg: `colord("#1e1e1e").alpha(1 - 0.5)` → `"#1e1e1e80"`
   - Applies alpha to selection bg
3. Stores the semi-transparent bg as `bgcolor` (returned separately)
4. Sets `theme.background = "#00000000"` (fully transparent for xterm)
5. Returns `[modifiedTheme, bgcolor]`

### How it's applied

- **xterm terminal**: Gets theme with `background: "#00000000"` → xterm sets this
  on `.xterm-scrollable-element` as inline style → terminal canvas is transparent
- **Block container**: `blockBg` memo in TermViewModel (`termViewModel.ts:217-224`)
  returns `{ bg: bgcolor }` where bgcolor is the semi-transparent color (e.g., `"#1e1e1e80"`)
- **blockframe.tsx**: `computeBgStyleFromMeta(customBg)` converts to inline style
  on `.block-frame-default-inner`

### What shows through

When terminal is transparent, you see through:
1. Terminal → block bg (semi-transparent theme color)
2. Block bg → AppBackground (tab wallpaper/gradient)
3. AppBackground → body bg (`rgba(34,34,34,1)` by default)

---

## What's Different in CEF

### Working correctly
- xterm theme background set to `#00000000` — works (pure CSS/JS)
- Block bg set to semi-transparent color — works (pure CSS/JS)
- AppBackground renders tab wallpaper — works (pure CSS/JS)
- Body bg is opaque `rgba(34,34,34,1)` — works

### Not working

**1. `set_window_transparency` is stubbed**

`app.tsx:141` calls `getApi().setWindowTransparency(isTransparentOrBlur, isBlur, opacity)`
which sends IPC `set_window_transparency` to CEF host. This is in `stubs.rs` —
returns `null` and does nothing.

In Tauri, `set_window_transparency` does:
- Sets `window.set_transparent(true)` on the Tauri window
- On Windows: enables DWM blur/acrylic effects
- Sets the window background to transparent so the desktop shows through

In CEF, none of this happens. The CEF window is always opaque.

**2. `--window-opacity` CSS var never set for non-transparent mode**

`AppSettingsUpdater` in `app.tsx:121-133`:
```typescript
if (isTransparentOrBlur) {
    // Sets --window-opacity on body
    document.body.style.setProperty("--window-opacity", `${opacity}`);
} else {
    // Removes --window-opacity → falls back to CSS default (1)
    document.body.style.removeProperty("--window-opacity");
}
```

If `window:transparent` is false (default), `--window-opacity` stays at the CSS
default of `1`. The body background `rgba(34,34,34,1)` is fully opaque. This is
correct — without window transparency, the body SHOULD be opaque.

**3. CEF browser background is opaque black (0xFF000000)**

Even if we made the body transparent, the CEF browser background would show
through as black. CEF Views framework does not support transparent windows
without `windowless_rendering_enabled` (off-screen rendering), which is a
completely different rendering architecture.

---

## The Real Issue

Terminal per-pane transparency (Layer 3→2 in the stack) should work identically
in CEF and Tauri because it's purely CSS:
- Terminal becomes transparent → block bg shows through
- Block bg is semi-transparent → AppBackground shows through

**If this isn't working, the bug is in one of these layers:**

### Suspect 1: xterm v6 `.xterm-scrollable-element` inline style

xterm v6 sets `background-color` as an inline style on `.xterm-scrollable-element`.
If the theme background `#00000000` is being interpreted as `#000000` (dropping alpha),
the terminal would be opaque black.

**To verify:** Open DevTools in CEF (`toggle_devtools` IPC command) and inspect
the `.xterm-scrollable-element` element's inline style.

### Suspect 2: `blockBg` memo not reactive after meta update

Our earlier `viewType` memo fix (PR #254) should have fixed this. But if the
`termTransparencyAtom` uses `useBlockAtom` (which it does — `termViewModel.ts:210`),
it should be cached in a `createRoot` and survive effect re-runs.

### Suspect 3: The theme bg color isn't being applied to block container

`computeBgStyleFromMeta` in `waveutil.ts` converts `{ bg: "#1e1e1e80" }` to an
inline style. If this function doesn't handle RGBA hex colors correctly, the
block bg could be wrong.

---

## Recommended Investigation Steps

1. **Add DevTools toggle to CEF** — essential for debugging CSS issues.
   The CEF host already has `toggle_devtools` command. Bind it to F12 or add a
   keyboard shortcut.

2. **Inspect the rendering stack in DevTools:**
   - `.xterm-scrollable-element` — does it have `background-color: rgba(0,0,0,0)`?
   - `.block-frame-default-inner` — does it have the semi-transparent bg?
   - Is the AppBackground div visible behind the block?

3. **Test with hardcoded values** — temporarily set block bg to `rgba(255,0,0,0.5)`
   to verify the transparency stack is connected.

---

## Architecture Options

### Option A: Pure CSS transparency (no window transparency)

Terminal transparency shows the app background/wallpaper through the terminal.
The window itself stays opaque. This is the simplest path and should already work
if the CSS layers are correct.

**Changes needed:** Debug why the CSS isn't working. Likely an xterm v6 issue
with `#00000000` being dropped.

### Option B: Platform-split like zoom (`transparency.platform.ts`)

Create `transparency.tauri.ts`, `transparency.cef.ts`, `transparency.platform.ts`.
Each implements the platform-specific window transparency:
- Tauri: `set_window_transparency` → DWM acrylic/blur
- CEF: No window transparency, but ensure CSS transparency works

**Changes needed:** Split `AppSettingsUpdater` logic into platform files. CEF
version skips `setWindowTransparency` IPC call entirely.

### Option C: CEF off-screen rendering for true transparency

Use `windowless_rendering_enabled` in CEF settings to render to a texture,
composite with transparent background. This is a major architecture change.

**Not recommended for Phase 2.**

---

## Immediate Fix (Option A)

The per-pane CSS transparency should work in CEF with no changes. If it doesn't,
the bug is in the xterm v6 theme application or the block bg style.

**Step 1:** Get DevTools working in CEF (F12 or keyboard shortcut)
**Step 2:** Inspect the `.xterm-scrollable-element` background
**Step 3:** If xterm drops the alpha, apply it differently (e.g., CSS opacity on
the xterm container instead of theme background alpha)
