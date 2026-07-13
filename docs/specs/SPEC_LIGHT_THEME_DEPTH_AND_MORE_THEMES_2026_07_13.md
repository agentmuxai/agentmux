# Spec: Light Theme — Header/Status-Bar Depth Fixes + 3 New Light Themes

**Date:** 2026-07-13
**Status:** Implemented (this spec documents the change; code is in the same PR)
**Follows:** `docs/specs/SPEC_LIGHT_THEME_AND_DEPTH_FIXES_2026_07_11.md` (the original light theme), the PR #2104 review cycle that fixed `--input-bg-color` and the `*-text-color` triad.

**Trigger:** manual verification of PR #2104 on `task dev` surfaced that the window header (top bar, holding tabs + widgets) and the status bar (bottom) stayed dark under the Light theme, even though every other surface had correctly gone light.

---

## 1. Does the theme system reach the header and status bar? — Answer

**The plumbing does; three specific things didn't wire into it correctly.** Every rule in these areas is written against `var(--token)`, not hardcoded colors, and most tokens resolve correctly via alias (`--widget-icon-color` → `--secondary-text-color`, `--tab-separator-color` → `--border-color`, etc. — both already overridden by `light.scss`). Five concrete gaps were found and fixed, three of them exactly explaining the reported symptom:

| # | Where | Bug | Why it looked "still dark" |
|---|---|---|---|
| 1 | `.window-header` (`window-header.{win32,darwin,linux}.scss`) | `background: var(--tab-strip-bg)` — token exists, but `--tab-strip-bg: rgba(255, 255, 255, 0.03)` at `:root` is a **white** tint (meant to lighten a dark surface). `light.scss` never overrode it. | A white tint over an already-near-white background is invisible; the header rendered at its literal dark-theme value with no light override to fall back to — **this is the top bar staying dark.** |
| 2 | `.status-bar` (`StatusBar.scss`) | `background: rgba(0, 0, 0, 0.35)` — a **hardcoded literal**, not a token at all. | Never participated in the theme system in the first place — **this is the status bar staying dark, on every theme, not just Light.** |
| 3 | `.action-widget-more-btn` hover/open, `.action-widget[.context-active]` (`action-widgets.scss`) | `var(--hoverbg-color, ...)` — missing hyphen; the real token is `--hover-bg-color`. `--hoverbg-color` is never defined anywhere. | Every widget-bar hover, on every theme, silently used the hardcoded white-tint fallback — theme-inert since the day it was written. Same rule also hardcoded `color: white` for the hover/open text color. |
| 4 | Windows caption buttons (min/max hover/press), `window-controls.win32.scss` | Gated on `@media (prefers-color-scheme: light)` — the **OS** setting — not AgentMux's own `data-theme` attribute. | If the OS is in dark mode (common) while AgentMux's own theme is set to Light, the caption buttons kept the dark-tuned tint. Wrong signal entirely, not a missing value. |
| 5 | (adjacent, found during the sweep) `.status-bar-restart-btn`, `.close-btn` hover in `window-controls.win32.scss` | Hardcoded `color: #fff`/`color: white` | **Not bugs** — both are already correctly documented as intentional (white-on-solid-red is universally readable; the Win11 close-button red is an OS brand color, not theme-relative). Left as-is. |
| — | `.config-error-button` in `system-status.scss` | Hardcoded `color: black` | Dead CSS — no component renders this class (only the similarly-named `.config-error-message` is used). Confirmed via `grep`; left untouched, out of scope. |

---

## 2. Fixes

### 2.1 New/fixed tokens (`theme.scss`, `light.scss`)

- **New `--status-bar-bg` token** at `:root` (`theme.scss`), default value = the exact literal it replaces (`rgba(0, 0, 0, 0.35)`) — **zero visual change on any existing dark theme.** `StatusBar.scss` now reads `background: var(--status-bar-bg)`.
- **`light.scss` gains two overrides it was missing**: `--tab-strip-bg: rgba(0, 0, 0, 0.03)` and `--status-bar-bg: rgba(0, 0, 0, 0.06)`. Same polarity-flip reasoning as the PR #2104 `--input-bg-color` fix: black tints darken regardless of what's underneath (no vanish-toward-invisibility risk), so they're the correct direction for every light-family theme's recessed chrome strips — mirrored for both new tokens.

### 2.2 `--hoverbg-color` → `--hover-bg-color` typo fix
`action-widgets.scss`, both call sites (widget hover, "More" button hover/open). Also fixed the hardcoded `color: white` on the same rule → `var(--widget-icon-hover-color)`, matching the equivalent icon-hover treatment three lines above it in the same file.

### 2.3 Caption-button light detection: OS media query → app theme signal
Introduced a generic **light/dark polarity marker**, independent of which specific theme is active:

- `frontend/app/menu/base-menus.ts` — new `LIGHT_THEME_IDS: ReadonlySet<string>`, alongside the existing `THEME_OPTIONS` list (single source of truth for both).
- `app.tsx` — alongside the existing `data-theme="<id>"` attribute, now also sets `data-theme-polarity="light"` on `<html>` when the active theme id is in `LIGHT_THEME_IDS` (removed otherwise).
- `window-controls.win32.scss` — the caption-button hover/press override moves from `@media (prefers-color-scheme: light)` to `[data-theme-polarity="light"] &` (SCSS nesting → `[data-theme-polarity="light"] .window-action-buttons`). Same values, correct gate.

**Why a separate polarity marker instead of one `[data-theme="light"]` check**, given the header/status-bar fixes (§2.1) just needed a plain per-theme override: this specific bug is in a *chrome* file that shouldn't need to know about individual theme ids or be edited every time a new light theme is added — one selector now correctly covers Light **and** the three new themes below, and whatever light theme is added next, for free. (The header/status-bar tokens don't need this: each theme file, dark or light, already declares its own value or inherits a shared default — that pattern doesn't require a polarity concept, only genuinely theme-agnostic *logic* like this one does.)

---

## 3. Three more light themes

Added, matching this codebase's existing "borrow the real palette's well-known colors, apply them through the established structural token pattern" convention (every existing dark theme — Nord, Dracula, Catppuccin, Gruvbox, Tokyo Night — already does this; `light.scss` itself is recognizably GitHub's light palette under a generic name). Selection: two are the official light flavor of an *already-present* dark theme (giving users a light/dark pair within the same family), one is a well-known standalone classic.

| Theme id | Label | Pairs with | Palette source |
|---|---|---|---|
| `catppuccin-latte` | Catppuccin Latte | existing `catppuccin` (Mocha) | Catppuccin's official Latte flavor — Base/Mantle/Text/Subtext + named accents (Blue, Red, Green, Yellow, Peach, Mauve, Teal, Pink) |
| `solarized-light` | Solarized Light | — (standalone classic) | Ethan Schoonover's Solarized — base3/base2/base01/base00 + the 8 accent hues shared with Solarized Dark |
| `gruvbox-light` | Gruvbox Light | existing `gruvbox` (Dark) | Gruvbox's official light variant, using the palette's own "faded" accent set (its documented choice for light backgrounds, vs. the "neutral/bright" set the existing dark Gruvbox theme uses) |

Each new file (`frontend/app/themes/{catppuccin-latte,solarized-light,gruvbox-light}.scss`) follows `light.scss`'s exact token surface and light-polarity conventions (§2.1's black-tint direction for hover/highlight/input-bg/tab-strip/status-bar), but tints hover/highlight with the theme's own accent color rather than plain black — mirroring how the existing dark Catppuccin/Gruvbox themes tint *their* hover/highlight with their own accent instead of plain white. `--error-text-color`/`--success-text-color`/`--warning-text-color` alias to the theme's own `--error-color`/`--success-color`/`--warning-color`, same pattern `light.scss` established in the PR #2104 review (avoids the dark-tuned defaults' contrast failure on a light surface).

**Solarized Light's `--term-*` table is Solarized's own canonical, widely-published 16-color ANSI mapping**, reproduced as-is rather than derived — unlike every other theme file (including this one's own UI tokens), that specific mapping is the palette's unchanged-for-over-a-decade signature. Catppuccin Latte's and Gruvbox Light's `--term-*` tables are derived from each theme's own core semantic colors (same approach `light.scss` itself uses), not a claimed reproduction of any one tool's exact ANSI export.

### 3.1 Registration (4 touchpoints, all existing themes already require these)
- `frontend/app/themes/index.scss` — `@use` the 3 new files.
- `frontend/app/menu/base-menus.ts` — 3 new `THEME_OPTIONS` entries + `LIGHT_THEME_IDS` (§2.3).
- `schema/settings.json` — `window:theme` enum + description gain the 3 new ids.
- `frontend/types/gotypes.d.ts` — no change needed; `window:theme` is typed as plain `string` there, no enum duplication to update.

---

## 4. Verification

- `npx tsc --noEmit` — clean.
- `npm run build` — clean (SCSS compiles, no errors).
- `npx vitest run` — 1688 passed, 2 skipped (pre-existing, unrelated), 0 failures. No test hardcodes the theme count or enumerates `THEME_OPTIONS`, so the 3 additions needed no test updates.
- Not independently re-verified visually beyond the original manual `task dev` check that surfaced the header/status-bar symptom — recommend a follow-up look at all 4 light themes (original + 3 new) once this lands, same as the note on the original light theme spec.

## 5. Files touched

| File | Change |
|---|---|
| `frontend/app/theme.scss` | new `--status-bar-bg` token at `:root` |
| `frontend/app/statusbar/StatusBar.scss` | hardcoded status-bar background → `var(--status-bar-bg)` |
| `frontend/app/window/action-widgets.scss` | `--hoverbg-color` typo → `--hover-bg-color` (×2); hardcoded `color: white` → `var(--widget-icon-hover-color)` |
| `frontend/app/window/window-controls.win32.scss` | `@media (prefers-color-scheme: light)` → `[data-theme-polarity="light"] &` |
| `frontend/app/menu/base-menus.ts` | new `LIGHT_THEME_IDS`; 3 new `THEME_OPTIONS` entries |
| `frontend/app/app.tsx` | sets/clears `data-theme-polarity` alongside `data-theme` |
| `frontend/app/themes/light.scss` | adds `--tab-strip-bg` / `--status-bar-bg` overrides |
| `frontend/app/themes/{catppuccin-latte,solarized-light,gruvbox-light}.scss` | new |
| `frontend/app/themes/index.scss` | `@use` the 3 new files |
| `schema/settings.json` | `window:theme` enum + description |
