# Spec: Light theme + theme-system depth fixes

**Date:** 2026-07-11
**Author:** Agent1
**Status:** Phases 1–3 implemented (this PR); Phase 4 (light theme) implemented (this PR); Phases 5–7 not started
**Related:** `frontend/app/theme.scss`, `frontend/app/themes/`, `frontend/app/components/context-menu.scss`, `frontend/app/block/titlebar.scss`, `schema/settings.json`, `frontend/app/menu/base-menus.ts`
**Governing context:** `docs/analysis/ANALYSIS_THEME_SYSTEM_LIGHT_THEME_AND_DEPTH_GAPS_2026_07_07.md` (the research pass this spec turns into concrete work), `SPEC_TERMINAL_THEME_PENETRATION_2026_07_07.md` (terminal penetration — separately proposed and already shipped in PR #2010, 2026-07-08)

---

## 1. Summary

The 2026-07-07 analysis found two things: (1) no light theme exists anywhere in AgentMux — all 9 `window:theme` options, including the one named "high-contrast", are dark-background; (2) several surfaces don't participate in the CSS custom-property theme system at all, including some ("phantom tokens", the context menu, the pane title bar) that would render as visibly broken the moment a light theme shipped.

The terminal-penetration half of the analysis (dimension 2a) was picked up separately and shipped in PR #2010 before this spec was written — it is **not** in scope here. This spec covers the remaining depth-gap items (2b/2c) as prerequisites, then a first light theme (dimension 1).

## 2. Scope

In scope (this PR):
1. Phantom tokens (§2b of the analysis) — 7 CSS custom properties referenced via `var(--token, #fallback)` but never defined anywhere, so every consumer was permanently pinned to its own local fallback literal.
2. Context menu (§2c) — fully hand-painted to literal Catppuccin Mocha hex values.
3. Pane title bar (§2c) — hardcoded `rgba(0,0,0,0.6-0.8)` background that would render as an opaque black bar on a light pane body.
4. A first light theme, `frontend/app/themes/light.scss`, following the existing 8-file override pattern.

Explicitly out of scope (documented here as follow-ups, not started):
5. `tab.scss`'s colored-tab white text (`rgba(255,255,255,...)`, lines 152/157/228) — a real but **separate** contrast issue against user-picked `TAB_COLORS` swatches (already borderline for yellow/lime today, in the all-dark-theme world), independent of `window:theme`/light-vs-dark. Doesn't block the light theme — deferred.
6. Mermaid/Shiki fixed-theme wiring (§2d) — each library's own theming API differs; the analysis recommends its own follow-up investigation.
7. Native/CEF pre-paint flash guards, splash screen (§2e) — needs a persisted-preference read at native process start, before any web content loads; a different kind of work than the CSS-side items here.
8. Scattered bare hex long tail (§2f) — lower severity, broad; fix opportunistically.

## 3. Design

### 3.1 Phantom tokens

Each of the 7 undefined tokens (`--accent-text-color`, `--button-color`, `--error-text-color`, `--success-text-color`, `--warning-text-color`, `--input-bg-color`, `--elevated-bg-color`) is now defined once in `theme.scss`'s `:root`, using the value each token's call sites already, consistently fell back to — so today's rendering is byte-for-byte unchanged across all 9 existing themes, but the token is real. Two of the seven are aliases to pre-existing tokens that already meant the same thing under a different name (`--input-bg-color` → `--form-element-bg-color`, `--elevated-bg-color` → `--modal-bg-color`) rather than new literals, so a future theme only has to get the underlying token right once.

### 3.2 Context menu + pane title bar

- `context-menu.scss`: every literal hex replaced with the corresponding existing token (`--modal-bg-color`, `--border-color`, `--main-text-color`, `--hover-bg-color`, `--error-color`, `--grey-text-color`). No hex fallbacks — every token used is an unconditional `:root` default, so a fallback would only mask a real bug if one were ever missing.
- `titlebar.scss`: the `rgba(0,0,0,0.6/0.7/0.8)` background is now `color-mix(in srgb, var(--block-bg-solid-color) N%, transparent)`. `--block-bg-solid-color` is already defined per-theme in all 8 theme files as each theme's own opaque backing color; mixing from it (rather than a flat literal) makes the title bar a theme-relative "darkened cap on this pane's own surface" instead of an app-wide flat black — and for the default theme, `rgb(0,0,0)` at 60% is numerically identical to the old `rgba(0,0,0,0.6)`, so there's no visual change today. The border switches to `var(--border-color)`. The rename-input's background switches to a `color-mix` off `--hover-bg-color`.

Both fixes generalize to the light theme with zero additional per-theme work, because they derive from tokens the light theme already has to define for other reasons.

### 3.3 Light theme

`frontend/app/themes/light.scss` follows the exact per-theme override pattern of the 8 existing files — same token list, same `[data-theme="light"]` selector shape, same Tailwind `--color-*` sync block. Registered in `themes/index.scss`, `schema/settings.json`'s `window:theme` enum, and `THEME_OPTIONS` in `base-menus.ts`.

Design choice: light theme's `--hover-bg-color`/`--highlight-bg-color`/scrollbar tokens invert polarity from the dark themes (darkening tints instead of lightening ones) — e.g. `rgba(0,0,0,0.06)` instead of `rgba(255,255,255,0.1)` — since a lightening tint on an already-light surface is invisible. This is exactly the per-theme customization point the phantom-token and title-bar fixes above were built to make actually work.

Terminal (`--term-*`) tokens get real light-appropriate values too, since PR #2010's terminal penetration work means a light theme's terminal now actually depends on them (previously wired to nothing).

## 4. Testing

- `npx stylelint frontend/app/theme.scss frontend/app/themes/*.scss frontend/app/components/context-menu.scss frontend/app/block/titlebar.scss` — clean.
- Manual verification: switch `window:theme` to `light` via Settings/hamburger menu and confirm no black-on-black or white-on-white seams in the context menu, pane title bar, and the surfaces the phantom tokens feed (Armory active pills, identity/memory panel error text, editor inline inputs, activity dock).

## 5. Follow-ups (not this PR)

Tracked in §2 above as items 5–8. Recommended order unchanged from the analysis: tab-color text contrast (5) is cheap and self-contained: worth a short follow-up. Mermaid/Shiki (6) is the largest remaining user-visible gap (code blocks and diagrams never match the app theme). Native/CEF (7) only matters once users are actually choosing a light theme in practice. Bare hex long tail (8) is fix-opportunistically territory.
