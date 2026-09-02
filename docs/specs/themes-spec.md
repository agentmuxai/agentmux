# Spec: Theme System

## Overview

Introduce a runtime theme system for AgentMux. Users can switch themes without restarting. All themes are defined as SCSS files that override the existing CSS custom properties from `theme.scss`.

## Architecture

### Theme Files

Each theme lives in `frontend/app/themes/<name>.scss` and overrides `:root` variables under a `[data-theme="<name>"]` selector:

```scss
// frontend/app/themes/midnight.scss
[data-theme="midnight"] {
  --main-bg-color: rgb(10, 12, 22);
  --accent-color: rgb(100, 160, 255);
  // ...
}
```

The default theme (current) remains in `theme.scss` on `:root` — no selector needed. All named themes inherit from it and override only what differs.

### Theme Application

Set `data-theme` on `<html>` at runtime:

```ts
document.documentElement.setAttribute("data-theme", themeName);
```

CSS specificity: `[data-theme="x"] { ... }` (0,1,0) beats `:root { ... }` (0,0,1) — no `!important` needed.

### State & Persistence

Theme is set via `settings.json` — the same config file users already edit for all other preferences. The setting key is `window:theme`.

**Example `settings.json`:**
```json
{
  "window:theme": "midnight"
}
```

Valid values (see **Themes** section below for full descriptions):

| Value | Theme |
|-------|-------|
| `"default"` | Default — dark grey, blue accent (built-in baseline) |
| `"midnight"` | Midnight — deep navy, electric blue |
| `"high-contrast"` | High Contrast — pure black, white text, cyan accent |
| `"monokai"` | Monokai — warm dark, pink/green palette |
| `"nord"` | Nord — arctic blue-grey tones |
| `"dracula"` | Dracula — purple-forward dark |
| `"catppuccin"` | Catppuccin Mocha — soft pastel dark, mauve accent |
| `"tokyo-night"` | Tokyo Night — deep blue-purple |
| `"gruvbox"` | Gruvbox Dark — retro warm dark, earthy yellows |

Omitting `window:theme` (or setting it to `"default"`) uses the built-in baseline.

- **Jotai atom** `themeAtom` in `frontend/app/store/theme.ts` — reads from the settings store, not localStorage
- Settings changes are watched live — theme applies without restart

### Bundling

All theme SCSS files are imported in `app.scss`:

```scss
// Themes
@use './themes/midnight';
@use './themes/high-contrast';
@use './themes/monokai';
// ...
```

### Theme Picker

New `<ThemePicker />` component in Settings → Appearance. Displays theme name + a small color swatch preview (background + accent). Applies immediately on selection.

---

## Variables Each Theme Must Define

A theme file only needs to override the variables that visually change. The minimum set:

| Group | Variables |
|-------|-----------|
| **Backgrounds** | `--main-bg-color`, `--panel-bg-color`, `--block-bg-color`, `--block-bg-solid-color`, `--modal-bg-color` |
| **Text** | `--main-text-color`, `--secondary-text-color`, `--grey-text-color` |
| **Accent** | `--accent-color`, `--color-accent-400` (+ scale 50–900 in tailwindsetup if needed) |
| **Borders** | `--border-color`, `--form-element-border-color` |
| **Hover/Highlight** | `--hover-bg-color`, `--highlight-bg-color` |
| **Status** | `--error-color`, `--warning-color`, `--success-color` |
| **Terminal ANSI** | All 16 `--term-*` colors + `--term-foreground`, `--term-background` |
| **Buttons** | `--button-green-bg`, `--button-red-bg`, `--button-yellow-bg` |
| **Scrollbars** | `--scrollbar-thumb-color`, `--scrollbar-thumb-hover-color` |

---

## Themes

### 1. Default (existing — no file needed)
Current `theme.scss` — dark grey, blue accent. The baseline all others extend.

---

### 2. Midnight

Deep navy dark — darker and more blue-shifted than Default. Good for late-night sessions.

| Token | Value |
|-------|-------|
| `--main-bg-color` | `rgb(10, 12, 22)` |
| `--panel-bg-color` | `rgba(8, 10, 20, 0.6)` |
| `--block-bg-color` | `rgba(4, 6, 14, 0.7)` |
| `--block-bg-solid-color` | `rgb(4, 6, 14)` |
| `--modal-bg-color` | `rgb(14, 18, 32)` |
| `--main-text-color` | `#e8eaf6` |
| `--secondary-text-color` | `rgb(160, 170, 200)` |
| `--accent-color` | `rgb(100, 160, 255)` |
| `--border-color` | `rgba(100, 140, 255, 0.18)` |
| `--hover-bg-color` | `rgba(100, 160, 255, 0.08)` |
| Terminal bg | `#060810` |
| Terminal fg | `#c8d0e8` |

---

### 3. High Contrast

Maximum legibility — pure black background, white text, bright saturated accent. Accessibility-first.

| Token | Value |
|-------|-------|
| `--main-bg-color` | `#000000` |
| `--panel-bg-color` | `rgba(0, 0, 0, 0.85)` |
| `--block-bg-color` | `#000000` |
| `--block-bg-solid-color` | `#000000` |
| `--modal-bg-color` | `#111111` |
| `--main-text-color` | `#ffffff` |
| `--secondary-text-color` | `#e0e0e0` |
| `--accent-color` | `rgb(0, 220, 255)` |
| `--border-color` | `rgba(255, 255, 255, 0.4)` |
| `--hover-bg-color` | `rgba(255, 255, 255, 0.15)` |
| `--error-color` | `rgb(255, 80, 80)` |
| `--warning-color` | `rgb(255, 220, 0)` |
| `--success-color` | `rgb(0, 240, 100)` |
| Terminal bg | `#000000` |
| Terminal fg | `#ffffff` |
| ANSI colors | Full bright palette — no muted/dark variants |

---

### 4. Monokai

Classic editor theme. Warm dark background with vivid pink/green/orange syntax palette.

| Token | Value |
|-------|-------|
| `--main-bg-color` | `#272822` |
| `--panel-bg-color` | `rgba(36, 37, 31, 0.6)` |
| `--block-bg-color` | `rgba(26, 27, 22, 0.7)` |
| `--block-bg-solid-color` | `#1a1b16` |
| `--modal-bg-color` | `#2d2e27` |
| `--main-text-color` | `#f8f8f2` |
| `--secondary-text-color` | `#cfcfc2` |
| `--accent-color` | `#a6e22e` |
| `--border-color` | `rgba(255, 255, 255, 0.12)` |
| `--hover-bg-color` | `rgba(166, 226, 46, 0.08)` |
| `--error-color` | `#f92672` |
| `--warning-color` | `#e6db74` |
| `--success-color` | `#a6e22e` |
| Terminal fg | `#f8f8f2` |
| Terminal bg | `#272822` |
| `--term-red` | `#f92672` |
| `--term-green` | `#a6e22e` |
| `--term-yellow` | `#e6db74` |
| `--term-blue` | `#66d9ef` |
| `--term-magenta` | `#ae81ff` |
| `--term-cyan` | `#66d9ef` |

---

### 5. Nord

Arctic color palette — cool blue-grey tones, calm and readable.

| Token | Value |
|-------|-------|
| `--main-bg-color` | `#2e3440` |
| `--panel-bg-color` | `rgba(39, 44, 54, 0.6)` |
| `--block-bg-color` | `rgba(30, 34, 42, 0.7)` |
| `--block-bg-solid-color` | `#1e222a` |
| `--modal-bg-color` | `#3b4252` |
| `--main-text-color` | `#eceff4` |
| `--secondary-text-color` | `#d8dee9` |
| `--accent-color` | `#88c0d0` |
| `--border-color` | `rgba(136, 192, 208, 0.2)` |
| `--hover-bg-color` | `rgba(136, 192, 208, 0.08)` |
| `--error-color` | `#bf616a` |
| `--warning-color` | `#ebcb8b` |
| `--success-color` | `#a3be8c` |
| Terminal fg | `#d8dee9` |
| Terminal bg | `#2e3440` |

---

### 6. Dracula

High-contrast purple-forward dark theme — vibrant and widely loved.

| Token | Value |
|-------|-------|
| `--main-bg-color` | `#282a36` |
| `--panel-bg-color` | `rgba(33, 34, 44, 0.6)` |
| `--block-bg-color` | `rgba(24, 25, 32, 0.7)` |
| `--block-bg-solid-color` | `#1e1f29` |
| `--modal-bg-color` | `#2e303d` |
| `--main-text-color` | `#f8f8f2` |
| `--secondary-text-color` | `#c5c8d1` |
| `--accent-color` | `#bd93f9` |
| `--border-color` | `rgba(189, 147, 249, 0.2)` |
| `--hover-bg-color` | `rgba(189, 147, 249, 0.08)` |
| `--error-color` | `#ff5555` |
| `--warning-color` | `#ffb86c` |
| `--success-color` | `#50fa7b` |
| `--link-color` | `#8be9fd` |
| Terminal fg | `#f8f8f2` |
| Terminal bg | `#282a36` |
| `--term-red` | `#ff5555` |
| `--term-green` | `#50fa7b` |
| `--term-yellow` | `#f1fa8c` |
| `--term-blue` | `#6272a4` |
| `--term-magenta` | `#ff79c6` |
| `--term-cyan` | `#8be9fd` |

---

### 7. Catppuccin Mocha

Soft pastel dark — gentle on the eyes, warm mauve tones.

| Token | Value |
|-------|-------|
| `--main-bg-color` | `#1e1e2e` |
| `--panel-bg-color` | `rgba(24, 24, 37, 0.6)` |
| `--block-bg-color` | `rgba(17, 17, 27, 0.7)` |
| `--block-bg-solid-color` | `#11111b` |
| `--modal-bg-color` | `#24273a` |
| `--main-text-color` | `#cdd6f4` |
| `--secondary-text-color` | `#bac2de` |
| `--accent-color` | `#cba6f7` |
| `--border-color` | `rgba(203, 166, 247, 0.18)` |
| `--hover-bg-color` | `rgba(203, 166, 247, 0.07)` |
| `--error-color` | `#f38ba8` |
| `--warning-color` | `#fab387` |
| `--success-color` | `#a6e3a1` |
| Terminal fg | `#cdd6f4` |
| Terminal bg | `#1e1e2e` |

---

### 8. Tokyo Night

Deep blue-purple city aesthetic — popular in VS Code community.

| Token | Value |
|-------|-------|
| `--main-bg-color` | `#1a1b2e` |
| `--panel-bg-color` | `rgba(22, 23, 40, 0.6)` |
| `--block-bg-color` | `rgba(14, 15, 26, 0.7)` |
| `--block-bg-solid-color` | `#0e0f1a` |
| `--modal-bg-color` | `#1f2035` |
| `--main-text-color` | `#c0caf5` |
| `--secondary-text-color` | `#9aa5ce` |
| `--accent-color` | `#7aa2f7` |
| `--border-color` | `rgba(122, 162, 247, 0.18)` |
| `--hover-bg-color` | `rgba(122, 162, 247, 0.07)` |
| `--error-color` | `#f7768e` |
| `--warning-color` | `#e0af68` |
| `--success-color` | `#9ece6a` |
| `--link-color` | `#73daca` |
| Terminal fg | `#c0caf5` |
| Terminal bg | `#1a1b2e` |

---

### 9. Gruvbox Dark

Retro warm dark — earthy yellows, oranges, and greens. Easy on the eyes for long sessions.

| Token | Value |
|-------|-------|
| `--main-bg-color` | `#282828` |
| `--panel-bg-color` | `rgba(32, 32, 32, 0.6)` |
| `--block-bg-color` | `rgba(20, 20, 20, 0.7)` |
| `--block-bg-solid-color` | `#1d2021` |
| `--modal-bg-color` | `#3c3836` |
| `--main-text-color` | `#ebdbb2` |
| `--secondary-text-color` | `#d5c4a1` |
| `--accent-color` | `#d79921` |
| `--border-color` | `rgba(215, 153, 33, 0.2)` |
| `--hover-bg-color` | `rgba(215, 153, 33, 0.08)` |
| `--error-color` | `#cc241d` |
| `--warning-color` | `#d79921` |
| `--success-color` | `#98971a` |
| Terminal fg | `#ebdbb2` |
| Terminal bg | `#282828` |
| `--term-red` | `#cc241d` |
| `--term-green` | `#98971a` |
| `--term-yellow` | `#d79921` |
| `--term-blue` | `#458588` |
| `--term-magenta` | `#b16286` |
| `--term-cyan` | `#689d6a` |

---

## File Structure

```
frontend/app/
├── theme.scss                  # Default theme (unchanged — :root)
├── themes/
│   ├── index.scss              # @use barrel: imports all themes
│   ├── midnight.scss
│   ├── high-contrast.scss
│   ├── monokai.scss
│   ├── nord.scss
│   ├── dracula.scss
│   ├── catppuccin.scss
│   ├── tokyo-night.scss
│   └── gruvbox.scss
├── store/
│   └── theme.ts                # Jotai atom + persistence helpers
└── element/
    └── themepicker.tsx/.scss   # Theme picker component
```

`app.scss` adds one line:
```scss
@use './themes/index';
```

---

## Theme Picker UI (optional — settings.json is primary)

Power users set `window:theme` directly in `settings.json`. The picker is a convenience UI:

- Location: Settings → Appearance tab
- Each option: theme name + horizontal color swatch (bg, accent, text, error chips)
- Active theme highlighted
- Selecting writes `window:theme` to settings store — applies instantly, persists across restarts

---

## Schema Update

Add `window:theme` to `schema/settings.json` so the settings editor validates and autocompletes it:

```json
"window:theme": {
  "type": "string",
  "enum": ["default", "midnight", "high-contrast", "monokai", "nord", "dracula", "catppuccin", "tokyo-night", "gruvbox"],
  "description": "UI color theme. Options: default, midnight, high-contrast, monokai, nord, dracula, catppuccin, tokyo-night, gruvbox"
}
```

Add it alongside the other `window:*` entries.

---

## Implementation Order

1. `schema/settings.json` — add `window:theme` with enum + description
2. `frontend/app/themes/*.scss` — all theme files + barrel
3. `frontend/app/app.scss` — add `@use './themes/index'`
4. `frontend/app/store/theme.ts` — atom that reads from settings store, applies `data-theme` on `<html>`
5. `frontend/app/element/themepicker.tsx` — optional UI picker in Settings → Appearance (writes to settings store)
6. Verify each theme with a smoke test (set in settings.json, reopen app, confirm theme applies)

## Verification Plan

- [ ] Setting `window:theme` in `settings.json` applies the theme on next open
- [ ] Changing the setting live (if settings are watched) applies without restart
- [ ] Omitting `window:theme` renders the default theme correctly
- [ ] All 9 themes render correctly — no invisible text, no broken contrast
- [ ] Terminal ANSI colors apply correctly per theme
- [ ] High Contrast theme meets WCAG AA (4.5:1 minimum contrast ratio)
- [ ] Default theme is unchanged (`:root` values untouched)
- [ ] `schema/settings.json` enum validates correctly (invalid theme name rejected)
