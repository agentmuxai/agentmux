# Theme System: Light Theme & Depth Gaps — Research Report

**Date:** 2026-07-07
**Scope:** Two questions — (1) does a light theme exist / what's needed to add one, (2) where does the theme system fail to "penetrate," especially the terminal, and what other nooks exist.
**Method:** Three parallel codebase research passes (theme architecture, terminal/xterm integration, hardcoded-color sweep).

---

## Bottom line

1. **No light theme exists anywhere in AgentMux today.** All 9 selectable `window:theme` options — including the one literally named "high-contrast" — use dark/near-black backgrounds. The token infrastructure to add one is solid (209 CSS custom properties, clean `[data-theme="..."]` override pattern), so adding a light theme is mechanically straightforward — but several "nooks" below would need light-aware values too, or the light theme will have visible seams on day one.
2. **The terminal is functionally decoupled from the app theme system.** `theme.scss` defines a complete 22-token terminal palette (`--term-*`) that **nothing reads**. The actual terminal colors come from a separate backend-owned config (`term:theme`, independent of `window:theme`) or a hardcoded JS fallback object. Switching the app theme never recolors the terminal, live or otherwise.
3. Beyond the terminal, there's a real and larger-than-tracked surface of hardcoded colors: a "phantom token" pattern (several `var(--token, #fallback)` sites where `--token` is never actually defined, so the fallback is the only value that ever applies), a fully hand-painted context menu, several third-party libraries (Mermaid, Shiki) pinned to fixed themes independent of the app, and native/CEF-layer pre-paint colors that would need their own mechanism entirely outside CSS.

---

## Dimension 1 — Light theme

### Current state

`frontend/app/theme.scss`'s bare `:root` (209 custom properties across color/typography/spacing/radius/shadow/motion/cursor/z-index/etc.) is the *default* theme and is itself dark (`--main-bg-color: rgba(34,34,34, var(--window-opacity))`, `--main-text-color: #f7f7f7`). Every named theme in `frontend/app/themes/` overrides a subset of those tokens under a `[data-theme="name"]` selector:

| Theme | `--main-bg-color` | `--main-text-color` |
|---|---|---|
| catppuccin (Mocha only) | `#1e1e2e` | `#cdd6f4` |
| dracula | `#282a36` | `#f8f8f2` |
| gruvbox (Dark) | `#282828` | `#ebdbb2` |
| **high-contrast** | `#000000` | `#ffffff` |
| midnight | `rgb(10,12,22)` | `#e8eaf6` |
| monokai | `#272822` | `#f8f8f2` |
| nord | `#2e3440` | `#eceff4` |
| tokyo-night | `#1a1b2e` | `#c0caf5` |

Every single one is dark-background/light-text. "High-contrast" is the classic black-background/white-text WCAG pattern, not a light-background high-contrast variant.

Two things that looked like they might be existing light themes, checked and ruled out:
- **`catppuccin-latte` JS chunk** (seen in build output) — this is the Shiki syntax-highlighter's bundled Catppuccin *Latte* flavor (VSCode-theme-JSON format, `"activityBar.background":"#dce0e8"` etc.), used only for code-block syntax highlighting. It's entirely unrelated to the app-chrome theme system — `frontend/app/themes/catppuccin.scss` only implements Mocha.
- **`prefers-color-scheme: light`** — exactly one hit in the whole frontend, `frontend/app/window/window-controls.win32.scss:55-61`, and it only retints the Win11 caption-button hover/press overlay (white↔black tint), unconnected to `data-theme`/`window:theme`. A separate spec doc (`SPEC_AGENT_PANE_RESPONSIVE_AUX_INFO_2026_06_09.md`) flags OS-light-mode-aware coloring as an explicit *design target, not yet implemented*.

### Mechanism (for building a light theme against)

- Settings key: `window:theme`, schema enum in `dist/schema/settings.json`.
- Picker UI: `frontend/app/view/settings/settings-view.tsx` `<select>`, options sourced from `THEME_OPTIONS` in `frontend/app/menu/base-menus.ts` (also drives the hamburger-menu Theme submenu).
- Applied via `frontend/app/app.tsx`'s `AppSettingsUpdater()`: `document.documentElement.setAttribute("data-theme", theme)` (or removes the attribute for `"default"`). Pure attribute-driven CSS cascade, live-reactive — no reload needed.
- To add a theme: create `frontend/app/themes/<name>.scss` following the existing 8 files' pattern, add it to `index.scss`'s `@use` list, add the id to the settings schema enum and `THEME_OPTIONS`.

### What a light theme needs beyond "just flip the tokens"

Straightforwardly reusing the existing per-theme override pattern covers most surfaces. But per Dimension 2 below, several places don't participate in the token system at all and are hardcoded to values that assume a dark surface (near-black inputs, `rgba(0,0,0,...)` pane title bars, always-white text via phantom tokens). A light theme added today would immediately expose every one of these as a visible bug — e.g. the pane title bar (`rgba(0,0,0,0.6-0.8)`) would render as an opaque black bar on top of a light pane body. **Recommendation: treat the Tier 0/1 items in Dimension 2 as a prerequisite pass before or alongside shipping a light theme**, not a follow-up — a light theme's first impression will be these seams.

The `--term-*` token set already exists and is unused (Dimension 2) — a light theme's terminal would need either real light-appropriate `--term-*` values wired up (once the terminal actually reads them, see below) or its own light entry in the backend `termthemes` config, independent of the CSS work.

---

## Dimension 2 — Where the theme system doesn't penetrate

### 2a. The terminal (primary ask) — confirmed fully decoupled

The embedded terminal (xterm.js, `frontend/app/view/term/`) gets its colors from `computeTheme()` / `computeTermThemeFromSettings()` in `frontend/app/view/term/termutil.ts`, which:

1. Reads `fullConfig.termthemes[term:theme]` — a **backend-owned** config (`agentmux-srv/src/backend/wconfig/types.rs`), keyed by the **separate** `term:theme` setting (not `window:theme`).
2. If that's empty (true by default today), falls back to a hardcoded JS literal, `FALLBACK_TERM_THEME` (`termutil.ts:18-43`) — a hand-authored 21-color palette resembling but not identical to Dracula.
3. **Never** reads any CSS custom property. `getComputedStyle` is used elsewhere in terminal code only for layout/font metrics, never color.

Meanwhile, `theme.scss` and all 8 theme files **already define** a complete matching 22-token terminal palette (`--term-black` … `--term-bright-white`, `--term-foreground/background`, `--term-selection-background`, `--term-cursor-accent`) with an explicit comment: *"these colors should be used by plugins/applications."* Grepping every `.ts`/`.tsx` file for `--term-` returns **zero** hits from actual terminal code — the only consumers are unrelated decorative CSS elsewhere (markdown link color, tool-badge pills, shell-node status dot) that borrow the terminal palette's hues for visual consistency, not the terminal itself.

There **is** a working live-update path (`terminal.options.theme = ...` in `frontend/app/view/term/termtheme.ts` and inline in `AgentInstallModal.tsx`) — but it's wired only to `term:theme`/`term:transparency` changes. `window:theme` has no code path into it at all.

**Net effect:** switching the app's theme instantly recolors every other themed surface via CSS cascade, but the terminal's glyphs, cursor, and selection never change — not live, not on restart, not ever, because the color computation simply doesn't look at `data-theme` or any `--term-*` variable.

**The fix is well-scoped:** `computeTheme()` already has the exact shape needed (`TermThemeType` with the same 16+ fields as CSS's `--term-*` set). The natural repair is to make the terminal's *default* term-theme (when the user hasn't explicitly picked a custom `term:theme`) derive from the currently-active `window:theme`'s `--term-*` tokens via `getComputedStyle(document.documentElement)`, re-run on `data-theme` changes the same way `TermThemeUpdater` already re-runs on `term:theme` changes. This is additive — it wouldn't need to touch the explicit-user-choice path (custom `term:theme` selections should still win), just give the *unset* case a theme-aware default instead of the static `FALLBACK_TERM_THEME`.

Smaller terminal-adjacent gaps found in the same sweep, all hardcoded independent of any theme:
- `term.tsx:102-107` — find-in-terminal search-decoration colors (`matchBorder: "#FFFF00"`, etc.)
- `termsticker.tsx:126,138` — in-terminal "sticker" annotation colors (`"#40cc40aa"` / `"#4040ccaa"`)
- `term.scss:9` — `.connection-btn { background-color: orangered; }`
- `term.scss:216-218` — amber warning badge, `#fff` text (deliberately commented as an intentional contrast exception)
- `xterm.css:81-82` — vendored xterm.js IME composition-view colors (`background:#000;color:#fff`), upstream default

### 2b. Structural "phantom tokens" — the most important non-terminal finding

Several sites reference `var(--token, #fallback)` where `--token` **is never defined anywhere** in `theme.scss` or any theme file — meaning every one of these is permanently pinned to the literal fallback, on every theme, forever, despite reading as theme-aware in a code review:

Undefined tokens: `--accent-text-color`, `--error-text-color`, `--success-text-color`, `--warning-text-color`, `--input-bg-color`, `--elevated-bg-color`, `--button-color`.

Hit list (file:line): `settings.scss:58`, `identity-pane-view.scss:28,93,205,239,244,245`, `memory-view.scss:29,92,113,205,210,211`, `armory-view.scss:65`, `bundle-summary.scss:38`, `_recent-sessions.scss:264`, `_shell-node.scss:199`, `PreLaunchAuthPanel.scss:28,182,201`, `editor-view.scss:218,358`.

This is worth fixing regardless of the light-theme timeline — it's a correctness bug in the token system itself (dead tokens creating an illusion of theming), and every one of these is exactly the kind of thing that turns into a white-text-on-white-background bug the moment a light theme ships.

### 2c. Fully hardcoded, high-traffic surfaces

- **`frontend/app/components/context-menu.scss`** — the entire right-click context menu (every line: background, border, text, hover, disabled states) is hand-painted with literal Catppuccin Mocha hex values (`#1e1e2e`, `#313244`, `#cdd6f4`, `#f38ba8`, `#6c7086`). Zero custom properties in the file. This is probably the single most visible "doesn't theme at all" surface in the app — a very frequently used popup that stays a small dark-purple box regardless of active theme.
- **`frontend/app/block/titlebar.scss:11,12,20,24,55,70`** — every pane's title bar uses `rgba(0,0,0,0.6-0.8)` background and `rgba(255,255,255,0.1)` border instead of `--main-bg-color`/`--border-color`. Directly blocks a light theme (see Dimension 1).
- **`frontend/app/tab/tab.scss:152,157,228`** — colored-tab text/accent forced to `rgba(255,255,255,...)`, an assumption baked in rather than tokenized; already borderline for some existing `TAB_COLORS` swatches (yellow, lime).

### 2d. Third-party libraries left on independent theming

- **Mermaid diagrams** — hardcoded `theme: "dark"` in both `markdown.tsx:46-50` and `streamdown.tsx:393-397`. Diagrams never match the app theme, light or dark.
- **Shiki (code syntax highlighting)** — hardcoded `const ShikiTheme = "github-dark-high-contrast"` in `markdown.tsx:18` and `HighlightedCode.tsx:24`. Every code block in chat and tool overlays uses one fixed theme regardless of app theme. High severity given how common code blocks are.
- **CodeMirror (file editor)** — done correctly, and worth using as the template for fixing the above two: `editor-theme.ts` binds chrome (bg/text/gutters/cursor/selection) to app tokens, with syntax-token colors deliberately left on a fixed highlight style with a documented reason ("the app theme system has no per-token color tokens").
- **Notification popover** (`notificationpopover.tsx:66`) — Tailwind arbitrary-value classes (`bg-[#232323]`) baked into JSX instead of a token.

### 2e. Native/CEF-layer surfaces — outside the CSS system by architecture

These can't be fixed by touching SCSS at all; they'd need a separate native-side theming mechanism (e.g. persisting last-known theme brightness to disk, read before first paint):

- **Splash screen** (`agentmux-launcher/src/splash_config.rs`) — 100% native Win32 GDI drawing, hardcoded RGB constants. Runs before any web content loads.
- **Pre-paint background-color flash guards** (`agentmux-cef/src/floating_pane.rs:244,466`, `app.rs:1145`, `browser_pane/creation_views.rs:134`) — hardcoded dark/black `background_color` values set at window creation to avoid a white flash before CSS paints. Once a light theme exists, this flash would go dark→light instead of matching.
- **Window control system colors** (macOS traffic lights, Windows 11 close-button hover) — deliberately fixed to match OS conventions, correctly documented as intentional, not a gap.

### 2f. Scattered bare hex (lower severity, broad)

A long tail of un-tokenized status/accent colors with no `var()` at all: `drone-view.scss`/`.tsx`, `warden.scss`, `identity-pane-view.scss` status dots, `editor-view.scss` diff indicators, `swarm-view.scss`, `settings.scss`, `_install-modal.scss`, `_recent-sessions.scss`, `modal.scss`, plus several files already using inline `/* stylelint-disable-line color-no-hex */` suppressions (not tracked in `.stylelintignore`, so invisible unless greeped for directly: `StatusBar.scss:386`, `term.scss:218`, `_document-nodes.scss:416`, `_focused-overlay.scss:118`, `_retry-empty.scss:21`, `_picker.scss:215`).

Dev-only surfaces (`frontend/perf/hud.tsx`, `frontend/app/devtools/diag-panel.tsx`, `agent-pane-perf-section.tsx`) are also fully hardcoded but lowest priority — not seen by end users in production.

Checked and ruled out as non-issues: `ProviderLogo.tsx` brand-color SVGs (intentionally fixed), `TAB_COLORS`/`color-swatch-palette.tsx` (these ARE the user-facing color picker's literal options, not a theming bug), and the many `rgba(0,0,0,...)` box-shadows across the codebase (conventional, shadows are black regardless of theme).

---

## Suggested prioritization (not a commitment — for discussion)

1. **Terminal theme-awareness** (2a) — the explicit ask, well-scoped fix identified above (derive default term-theme from `--term-*` via `getComputedStyle`, re-run on `data-theme` change).
2. **Phantom tokens** (2b) — cheap, mechanical, and a real correctness bug independent of any new theme.
3. **Context menu + pane title bar** (2c) — the two most visible "doesn't theme" surfaces; both are self-contained files.
4. **Light theme itself** — once 2b/2c are addressed, adding `frontend/app/themes/light.scss` (or similar) following the existing 8-file pattern is mechanical.
5. Mermaid/Shiki theme wiring (2d) — larger effort (each library's own theming API differs), worth its own follow-up investigation.
6. Native/CEF pre-paint flash guards (2e) — only matters once a light theme ships; needs a persisted-preference read at process start, a different kind of work entirely from the CSS-side items.
7. Scattered bare hex (2f) — long tail, fix opportunistically or in a dedicated sweep PR.

No code changes were made as part of this research. Happy to turn any of the above into a spec + implementation on request.
