# SPEC: Top-bar widget icons — theme-driven, monochrome by default

**Date:** 2026-05-26
**Status:** Draft
**Companion to:** `SPEC_DESIGN_SYSTEM_2026_04_23.md`
**Scope:** strictly the top-bar widget menu (pinned widget bar + More
dropdown). No other icon surface in the app is in scope.

---

## 1. Problem

The top-bar widget menu renders a visually inconsistent row of icons:

- `agent` is warm orange (`#cc785c`)
- `swarm` is amber (`#f59e0b`)
- `drone` is cyan (`#06b6d4`)
- `warden` is violet (`#8b5cf6`)
- `browser`, `editor`, `terminal`, `sysinfo`, `help` are muted grey
  (fall back to `--secondary-text-color`)

The hex values are hardcoded in `agentmux-srv/src/config/widgets.json`
and applied as inline `style={{ color: widget.color }}` at:

- `frontend/app/window/action-widgets.tsx:117` — pinned widget bar
- `frontend/app/window/action-widgets.tsx:192` — More dropdown

These hardcoded hex values:

- **Ignore the active theme.** Themes (`midnight`, `dracula`, `gruvbox`,
  `nord`, `tokyo-night`, `catppuccin`, `high-contrast`, `monokai`) swap
  semantic tokens via `[data-theme="<name>"]` rules — `#cc785c` reads as
  warm orange on every one of them regardless.
- **Mix two visual languages in one menu.** Four icons "branded", five
  neutral, no rule for which is which.
- **Have no spec.** `SPEC_DESIGN_SYSTEM_2026_04_23.md` defines a token
  taxonomy (primitive → semantic → component) that the hardcoded hex
  values bypass.

Desired behavior:

> All grey on default, and also change colors with the theme.

That maps onto the standard nav-chrome pattern (Stripe, Linear, GitHub,
VS Code activity bar, macOS Big Sur+ sidebars): **monochrome icons by
default, brighter on hover, accent on active state, color driven by the
theme.**

## 2. Goals

1. **Default to neutral.** Every icon in the top-bar widget menu and
   the More dropdown uses a single theme-derived color.
2. **React to theme.** Switching `window:theme` changes the icon color
   without per-widget code changes.
3. **Distinguish states.** Hover gets a brighter color; the widget
   whose pane is currently focused gets the theme accent.
4. **No regression in identifiability.** Icon glyph + label is enough
   at bar size; the color was redundant.

## 3. Non-goals

- **Any icon surface that is not the top-bar widget menu.** The
  launcher view's widget grid, agent-pane chrome, swarm pane visuals,
  status bar, modals, and every other icon in the app are explicitly
  out of scope. This spec touches `action-widgets.tsx` and the SCSS /
  theme files it depends on. Nothing else.
- A user-facing color picker UI. Themes already swap tokens.
- Animated, gradient, or shape-changing icons.
- Removing the `color` field from `widgets.json`. The data stays;
  it just stops driving the top-bar icon color. Whether any other
  surface uses it is out of scope here.

## 4. Design

### 4.1 New semantic tokens

Add to `frontend/app/theme.scss` (the `:root` block):

```scss
--widget-icon-color: var(--secondary-text-color);
--widget-icon-hover-color: var(--main-text-color);
--widget-icon-active-color: var(--accent-color);
```

These are **semantic** tokens per the three-tier taxonomy in the
design-system spec. They reference primitive tokens, so per-theme
overrides cascade automatically — any theme that already redefines
`--accent-color` gets a new `--widget-icon-active-color` for free.

A theme that wants a bespoke feel can override directly:

```scss
[data-theme="midnight"] {
    --widget-icon-color: rgb(140, 150, 170);
    --widget-icon-hover-color: rgb(200, 210, 230);
    --widget-icon-active-color: rgb(100, 160, 255);
}
```

### 4.2 Render change

Replace inline-color at `action-widgets.tsx:117` (pinned bar) and
`action-widgets.tsx:192` (More dropdown):

```diff
- <div style={{ color: widget.color }}>
+ <div class="widget-icon">
```

`action-widgets.scss` (new or extended):

```scss
.widget-icon {
    color: var(--widget-icon-color);
    transition: color 120ms ease;
}
.action-widget:hover .widget-icon {
    color: var(--widget-icon-hover-color);
}
.action-widget.is-active .widget-icon {
    color: var(--widget-icon-active-color);
}
```

### 4.3 Active state

Today the top-bar widget bar doesn't render an "active" marker on the
widget whose pane is currently focused. Add an `is-active` class
toggled from the existing focused-block logic. This is what makes
"all grey" feel right — the eye picks out the active pane via accent
color instead of via per-widget brand color.

If wiring active state is more scope than wanted, ship 4.1 + 4.2
first and treat active as a follow-up. The hover state alone is a
clear improvement and is unblocked.

### 4.4 Per-theme overrides (initial set)

Themes that already swap accent should be reviewed. Initial proposal:

| Theme | `--widget-icon-color` | `--widget-icon-hover-color` | `--widget-icon-active-color` |
|---|---|---|---|
| default (`:root`) | `--secondary-text-color` | `--main-text-color` | `--accent-color` |
| `midnight` | `rgb(140 150 170)` | `rgb(220 225 235)` | `--accent-color` |
| `dracula` | `rgb(140 140 160)` | `rgb(248 248 242)` | `--accent-color` |
| `high-contrast` | `rgb(220 220 220)` | `#ffffff` | `--accent-color` |
| others | defaults | defaults | defaults |

Tune during implementation; the point is the seam exists.

### 4.5 The widget `color` field in `widgets.json`

Stays in the JSON. Stops driving the top-bar icon color. Don't
delete the hex values — that's a data cleanup decision unrelated to
this spec. If a non-top-bar surface (out of scope here) wants to
keep using it, it's free to.

## 5. Migration / backwards compatibility

- `widgets.json` schema unchanged.
- Only two render sites change (`action-widgets.tsx:117,192`).
- Verified by audit: those are the only call sites in scope.

## 6. Implementation plan

Three small, isolated PRs in order:

1. **PR 1: Tokens + render swap.** Add the three semantic tokens to
   `:root`; replace inline-color at both `action-widgets.tsx` sites
   with the `widget-icon` class; wire `:hover` styling. Visual
   outcome: all top-bar icons render grey, brighter on hover.

2. **PR 2: Active state.** Add `is-active` class wiring from focused
   block to the widget bar. Visual outcome: the focused widget's
   icon lights up in `--accent-color`.

3. **PR 3: Per-theme overrides.** Add `--widget-icon-*` overrides to
   themes per §4.4. Visual outcome: each theme's bar feels coherent
   with its palette.

## 7. Open questions

- **More dropdown hover state.** Dropdown rows already have a hover
  background. Adding icon-color hover on top might be too much.
  Recommendation: row-hover background only, icon stays
  `--widget-icon-color` inside the dropdown — i.e., apply the hover
  rule only inside the pinned bar, not in the dropdown.
- **Custom themes via `settings.json`.** If users define their own
  theme block, document the new `--widget-icon-*` tokens as part of
  the supported customization surface.

## 8. Acceptance gates

- Visual: top-bar widget icons render in a single muted color on the
  default theme; hover brightens; active widget (if §4.3 ships) shows
  accent color.
- Theme switching: `window:theme` change updates icon color without
  per-widget code changes.
- Regression: existing widget `color` values in `widgets.json` do not
  visibly affect the top-bar widget menu or More dropdown.
- No new lint or type errors.
- Vitest snapshots for `ActionWidgets` (if any) updated for the
  class-based styling.

## 9. References

- `frontend/app/window/action-widgets.tsx:117,192` — render sites in
  scope
- `frontend/app/theme.scss` — semantic token home
- `frontend/app/themes/*.scss` — per-theme overrides
- `agentmux-srv/src/config/widgets.json` — widget defs (`color` field;
  unchanged by this spec)
- `docs/specs/SPEC_DESIGN_SYSTEM_2026_04_23.md` — token taxonomy
- Reference UIs that follow the monochrome-default pattern: Stripe,
  Linear, GitHub, VS Code activity bar, macOS Big Sur+ sidebars.
