# SPEC: Install-Modal xterm Theme Binding

**Status:** Draft
**Date:** 2026-05-18
**Author:** AgentA
**Related:** [`SPEC_AGENT_INSTALL_STAGE_2026_05_17.md`](./SPEC_AGENT_INSTALL_STAGE_2026_05_17.md) (the modal whose terminal is wrong), `frontend/app/view/term/termutil.ts` (the theme source of truth)

---

## 0. TL;DR

The install-modal terminal renders text in a hardcoded grey (`#cccccc`) on a hardcoded dark background, ignoring the user's configured terminal theme. This makes the install log harder to read than every other terminal in the app and means changing themes (Dracula, Solarized, etc.) has no effect on it.

Root cause: `AgentInstallModal.tsx` constructs its `Terminal` with `theme: { background, foreground }` literals instead of going through `computeTheme()` like every other xterm in the app.

The fix is a four-line redirect through the existing theme pipeline + a small helper for theme resolution outside the block context. After this change there is **one** xterm theme source for the entire renderer.

---

## 1. Problem

### 1.1 What the user sees

Open the install modal for an agent that isn't yet installed. Foreground text reads as low-contrast grey on near-black. Hard to scan; harder than the regular terminal pane next to it. Reported 2026-05-18 (the bug that prompted this spec).

### 1.2 What's actually wrong

```tsx
// AgentInstallModal.tsx (today)
terminal = new Terminal({
    // ...
    theme: {
        background: "#0d0e0f",
        foreground: "#cccccc",   // <-- the grey
    },
});
```

Two problems:

1. The colors are hardcoded literals — they don't respect the user's `term:theme` setting.
2. The theme object is otherwise empty — xterm's ANSI palette (black/red/green/.../brightWhite) falls back to xterm's library defaults, which don't match the rest of the app's palette either.

### 1.3 Why "just change the literal to yellow" is wrong

That was the first fix attempt (commit not landed). It makes the install modal *different* from every other terminal in the app rather than *consistent* with it. The user picked their terminal theme for a reason; the install modal should respect it.

---

## 2. Current architecture (the right way, used by `term.tsx`)

The regular terminal pane resolves its theme through a four-layer override chain plus a live-reactive applicator:

```
backend wconfig.termthemes  ──┐
                              │
block meta "term:theme"       │
connection conf "term:theme"  │ getOverrideConfigAtom(blockId, "term:theme")
settings    "term:theme"      ├─►  ──► resolves to themeName (3-tier)
DefaultTermTheme = "default-dark"  fallback
                              │
                              ▼
            computeTheme(fullConfig, themeName, transparency)
                              │
                              ▼
            <TermThemeUpdater> createEffect → terminal.options.theme = ...
                              │
                              ▼
            xterm.js renders with the resolved palette
```

**Files involved (all in `frontend/app/view/term/`):**

- `termutil.ts:13` — `computeTheme(fullConfig, themeName, transparency)` resolves theme object + bg color, with fallback to `DefaultTermTheme` (`"default-dark"`) and transparency blending.
- `termViewModel.ts:202` — `termThemeNameAtom` memoizes the 3-tier override chain (block-meta → conn-config → settings) via `getOverrideConfigAtom(blockId, "term:theme")`.
- `termtheme.ts:17` — `TermThemeUpdater` component: `createMemo` reads atoms, `createEffect` writes `terminal.options.theme` so theme swaps take effect live without re-creating the xterm.
- `term.tsx:160` — `onMount` snapshots theme + creates `TermWrap` with the initial theme baked in.

**Theme source = `atoms.fullConfigAtom()`** — a SolidJS signal mirroring the backend `wconfig`. Hot-reloads on config change.

---

## 3. Proposed architecture

### 3.1 Goals

1. Install-modal xterm reads from the same theme source as the regular term pane.
2. Theme swaps are reactive (user changes theme → install modal updates live, even if it's currently mid-install).
3. No duplication of theme resolution logic.
4. Modals (which have no `blockId`) get a clean accessor that doesn't require fabricating a fake block.

### 3.2 Non-goals

- Block-meta `term:theme` overrides do **not** apply to the install modal. The modal isn't a block; there's no meaningful blockId to override against.
- Connection-config overrides do **not** apply either, for the same reason.
- The modal does **not** participate in `term:transparency` — modals already sit on opaque panel backgrounds. Hardcode transparency `0`.

So the modal's theme is: `settings["term:theme"] ?? DefaultTermTheme`, with no transparency.

### 3.3 Built-in fallback palette

The backend `wconfig` ships with an **empty** `termthemes` table (see `agentmux-srv/src/backend/wconfig/mod.rs` — only test scaffolding inserts themes). So `computeTheme(fullConfig, "default-dark", 0)` resolves to `theme = {}` today, and xterm.js falls back to its library default palette — which has dim greys and a low-contrast foreground that looks grey-on-black against any dark container.

Fix: add a `FALLBACK_TERM_THEME` constant inside `termutil.ts`, used by `computeTheme` when neither the requested theme nor `DefaultTermTheme` is in `fullConfig.termthemes`. This is **not a parallel source of truth** — `fullConfig.termthemes[name]` still wins when present. The fallback only fires for empty-config installs, which is the steady state today.

Chosen palette: Dracula-style ANSI colors against a near-black background. Foreground `#f8f8f2` — same as Dracula's. High contrast, broadly familiar.

### 3.4 New helper

Add to `termutil.ts`:

```ts
/**
 * Resolve the terminal theme for callers that don't have a blockId
 * (modals, install dialogs, anything outside the pane tree).
 *
 * Respects `settings["term:theme"]` with a fallback to DefaultTermTheme.
 * Does not apply transparency — callers in this position render on
 * opaque panel backgrounds where transparency is meaningless.
 */
export function computeTermThemeFromSettings(
    fullConfig: FullConfigType
): [TermThemeType, string] {
    const themeName = fullConfig?.settings?.["term:theme"] ?? DefaultTermTheme;
    return computeTheme(fullConfig, themeName, 0);
}
```

This is a thin wrapper, not a parallel implementation. All theme-table lookup and fallback logic stays in `computeTheme`.

### 3.4 Install-modal binding

```tsx
// AgentInstallModal.tsx

import { atoms } from "@/app/store/global";
import { computeTermThemeFromSettings } from "@/app/view/term/termutil";

onMount(() => {
    // ...existing setup...
    const [termTheme] = computeTermThemeFromSettings(atoms.fullConfigAtom());

    terminal = new Terminal({
        // ...existing options...
        theme: termTheme,    // <-- no more hardcoded literals
        // (drop the previous {background, foreground} block entirely)
    });

    // ...

    // Live-reactive theme swap. Matches TermThemeUpdater's pattern
    // (see frontend/app/view/term/termtheme.ts).
    const themeStop = createEffect(() => {
        const [t] = computeTermThemeFromSettings(atoms.fullConfigAtom());
        if (terminal) terminal.options.theme = t;
    });

    onCleanup(themeStop);
});
```

### 3.5 Architectural invariant

After this change:

> **There is exactly one place in the renderer that resolves an xterm theme: `computeTheme()` in `termutil.ts`.**

Any new xterm consumer must:
1. Import `computeTheme` (with blockId) or `computeTermThemeFromSettings` (without).
2. Read `atoms.fullConfigAtom()` as the input.
3. Never construct `theme: {...}` literals.

A grep for `new Terminal(` covering the entire `frontend/` tree should return exactly two callers: `termwrap.ts` and `AgentInstallModal.tsx`. Both go through `computeTheme`. CI can enforce this with a lint rule if drift becomes a concern (out of scope for this spec).

---

## 4. Implementation

### 4.1 Edits

1. **`frontend/app/view/term/termutil.ts`** — add `computeTermThemeFromSettings`, export it.
2. **`frontend/app/view/agent/components/AgentInstallModal.tsx`** —
   - Remove the hardcoded `theme: { background, foreground }` block.
   - Compute initial theme via `computeTermThemeFromSettings(atoms.fullConfigAtom())`.
   - Add a `createEffect` that re-applies the theme on `fullConfigAtom` change.
3. **`docs/specs/SPEC_INSTALL_MODAL_TERM_THEME_BINDING_2026_05_18.md`** — this file.

### 4.2 Changeset

```
patch — fix(install-modal): bind xterm to the configured term theme (was hardcoded grey)
```

### 4.3 Tests / smoke

- Open install modal — text foreground should match whatever `term:theme` resolves to (Dracula → `#f8f8f2`, Solarized Dark → `#839496`, default-dark fallback → xterm white defaults).
- Switch terminal theme in settings while install modal is open → install modal updates without remount.
- Install log ANSI codes (npm's greens, the modal's own `\x1b[31m` stderr red) render with the theme's palette, not xterm's defaults.

---

## 5. Open questions

### 5.1 Should the install modal pick up `term:fontfamily` from settings too?

Currently it reads `--termfontfamily` from CSS at runtime (working as intended). Consistent with the rest of the app. No change needed.

### 5.2 Should the install modal pick up `term:fontsize`?

The modal hardcodes `fontSize: 12`. Reasonable — the modal has a fixed-size pane where the user's preferred terminal font size (often 14–16) would overflow. Leave hardcoded.

### 5.3 Future: pull more terminals through this path?

If a future feature surfaces an xterm somewhere else (auth dialog with raw output? subagent quick-view?), it should use `computeTermThemeFromSettings` from day one. The helper exists specifically to make that the easy path.

---

## 6. Acceptance criteria

1. `grep -n 'theme: {' frontend/app/view/agent/components/AgentInstallModal.tsx` returns no hardcoded theme literal.
2. `grep -nE 'new Terminal\(' frontend/` returns two matches, both reaching `computeTheme` (directly or via the new helper).
3. The install modal's foreground text contrast matches the regular term pane's contrast at default theme.
4. Switching terminal themes while the install modal is open updates the modal's colors without reopening.
