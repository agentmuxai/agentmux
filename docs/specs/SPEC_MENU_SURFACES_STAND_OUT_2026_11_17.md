# Spec: menus that stand out from the background, in every theme

Owner: Agent1@narko (UI kit). Task #47918 (this spec), build #47919. Filed by AgentY from the operator's request of 2026-11-17.

**Status:** implemented in #47919 (PR #7643); the answers to section 9 are recorded in section 10.

## 1. The problem, and its cause

The operator's example (0.60.32, a dark theme) is the pane menu (Split Right, Split Down, Magnify Pane, Minimize to Header, Close Pane, Pane Color) over a Reviews pane: the menu is the same colour as the pane, its edge is a faint line, no shadow shows, and text from behind shows through and competes with the items.

I reproduced it with the app's own compiled theme and menu styles ("Today" column of the sheets in section 6). Two separate causes:

1. **The surface is the pane's colour, and it is see-through.** `.menu` (every flyout, right-click and popover menu) takes its look from the `menu-frame` mixin, `frontend/app/element/menu-frame.scss:8`: `background: var(--main-bg-color)`. That token is `rgba(r, g, b, var(--window-opacity))` (theme.scss:43, and each theme), the same colour as the pane, scaled by the user's window opacity. At any opacity below 1 the page behind the menu shows through it. This is the ghost text in the screenshot. It is not a blur or a separate effect.
2. **The edge cannot be seen.** Measured per theme (section 3): the menu's surface against the pane is exactly 1.00:1, its 1px border is 1.2:1 to 1.7:1 against the pane (3.7:1 only in High Contrast), and the 30% black shadow disappears on a dark pane.

The same app already has the other behaviour: `.ctx-menu` (the in-app Solid context menu, `components/context-menu.scss`) and `popover-frame` (status-bar readouts) use `--modal-bg-color`, an opaque per-theme literal, so they are not see-through but are still only 1.01 to 1.27:1 from the pane. So the app has two right-click menu looks today.

## 2. Research: what current practice says

Sources I opened (a claim is only listed if the page said it). Pages that did not load (m3.material.io, fluent2.microsoft.design, Apple's HTML pages) are not cited.

- **Dark themes lift the surface; they do not rely on shadow.** Material 3 uses tonal elevation: "increasing tonal elevation uses a more prominent tone, in addition to shadows" ([Android M3](https://developer.android.com/develop/ui/compose/designsystems/material3)); a menu's container is `surface-container` ([material-web menu](https://github.com/material-components/material-web/blob/main/docs/components/menu.md)). Overlays are used where "the background is too dark to reliably portray dark shadows" ([Material dark-theme codelab](https://codelabs.developers.google.com/codelabs/design-material-darktheme)). Apple's dark mode: "the elevated colors are brighter, making foreground interfaces appear to advance" ([HIG Dark Mode](https://developer.apple.com/tutorials/data/design/human-interface-guidelines/dark-mode.json)). shadcn/ui, dark: popover `oklch(0.205 0 0)` on page `oklch(0.145 0 0)`, a 0.06 lightness step ([globals.css](https://raw.githubusercontent.com/shadcn-ui/ui/main/apps/v4/app/globals.css)). That step is one kit's convention, not a standard; M3 and Apple publish none.
- **A 1px edge plus a shadow.** Fluent: flyouts have a 1px stroke at elevation 32, and "shadows and contour (outlines) are used to subtly communicate an object's elevation" ([Layering](https://learn.microsoft.com/en-us/windows/apps/design/signature-experiences/layering)). GitHub Primer, dark: a 1px ring `#3d444d` plus a 0.4-alpha shadow, as one token ([dark.css](https://unpkg.com/@primer/primitives/dist/css/functional/themes/dark.css), [shadow.json5](https://raw.githubusercontent.com/primer/primitives/main/src/tokens/functional/shadow/shadow.json5)). shadcn: border white at 10% in dark. VS Code: a widget shadow of black at 0.36 in dark and 0.16 in light, so a dark theme strengthens the shadow ([editorColors.ts](https://raw.githubusercontent.com/microsoft/vscode/main/src/vs/platform/theme/common/colors/editorColors.ts)).
- **WCAG 2.2.** 1.4.11 Non-text Contrast asks 3:1 against the adjacent colours for a UI component that needs a boundary to be identified, and a boundary "is only required when there is no other visual way"; a drop shadow does not count towards the ratio ([Understanding 1.4.11](https://www.w3.org/WAI/WCAG22/Understanding/non-text-contrast.html)). So a menu edge is a convention, not a WCAG requirement, but 3:1 is the number the standard uses for "a boundary you can see". Item text must meet 4.5:1 against what it is rendered over ([Understanding 1.4.3](https://www.w3.org/WAI/WCAG22/Understanding/contrast-minimum.html)), which for a see-through menu means the worst thing behind it.
- **Translucent menus are the exception, with a solid fallback.** Windows: acrylic for "transient, light-dismiss surfaces such as flyouts and context menus", solid when transparency is off or the window is inactive, and no minimum opacity is given ([Acrylic](https://learn.microsoft.com/en-us/windows/apps/design/style/acrylic)). Apple: "thicker materials, which are more opaque, can provide better contrast for text" ([HIG Materials](https://developer.apple.com/tutorials/data/design/human-interface-guidelines/materials.json)). No first-party source gives a minimum opacity; any number would be our own.
- **Not established by the sources I could open:** Fluent's shadow numbers, Linear's menu values, native macOS or Chrome menu colours.

**What this means for AgentMux:** the menu surface is opaque whatever the window opacity is; it is a step off the pane in dark and light themes; it has a visible 1px edge and a shadow; and the shadow is never the only thing separating it.

## 3. Measurements, every built-in theme

Method: each theme's resolved `--main-bg-color` at window opacity 1 on black is "the pane"; WCAG relative-luminance contrast ratio; script `menu-spec/measure.mjs` (outside the repo). A pane that is a terminal or has its own block colour can differ: I measured the terminal background and the solid block colour too (`menu-spec/measure.json`). The terminal background equals the pane in 10 of the 13 themes. In the default dark theme it is black (the menu today is 1.32:1 from it), in Light 1.08:1, in Midnight 1.03:1; block colours in Nord, Monokai and Dracula sit up to 1.28:1 off the pane. The build's test uses the pane (`--main-bg-color` at opacity 1) and also checks the terminal background.

| Theme | Pane | Menu today (`.menu`) vs pane | `--modal-bg-color` vs pane | Border today vs pane | Item text vs menu |
|---|---|---|---|---|---|
| dark (default) | #222222 | 1.00 | 1.01 | 1.67 | 14.9 |
| catppuccin | #1e1e2e | 1.00 | 1.11 | 1.45 | 11.3 |
| dracula | #282a36 | 1.00 | 1.09 | 1.44 | 13.4 |
| gruvbox | #282828 | 1.00 | 1.27 | 1.43 | 10.8 |
| high-contrast | #000000 | 1.00 | 1.11 | 3.66 | 21.0 |
| midnight | #0a0c16 | 1.00 | 1.05 | 1.26 | 16.3 |
| monokai | #272822 | 1.00 | 1.08 | 1.46 | 13.9 |
| nord | #2e3440 | 1.00 | 1.24 | 1.50 | 10.8 |
| tokyo-night | #1a1b2e | 1.00 | 1.06 | 1.37 | 10.5 |
| light | #f6f6f7 | 1.00 | 1.08 | 1.31 | 17.8 |
| catppuccin-latte | #eff1f5 | 1.00 | 1.13 | 1.23 | 10.0 |
| gruvbox-light | #fbf1c7 | 1.00 | 1.21 | 1.32 | 12.7 |
| solarized-light | #fdf6e3 | 1.00 | 1.14 | 1.20 | 7.2 |

Reading: the menu is the pane (1.00) in all 13 themes; only High Contrast has an edge you can see. A light theme cannot lift much (white against a near-white pane is at most 1.13:1), so there the edge and the shadow do the work.

## 4. Inventory of menu-like surfaces

Findings from a read of `origin/main` (every floating menu, popover and list; file and line in the build task's checklist). The first column is how many surfaces share each look.

| Look | Surfaces | Background | Shadow | Radius |
|---|---|---|---|---|
| `.menu` via `menu-frame` | Logo menu; every JS right-click menu (pane, tab, empty tab, window, Swarm, drone, pane-tab picker); kit `Menu`/`MenuTrigger`/`openMenuAt`; `PopoverMenu` (title-bar menu, widget right-click); the widget "More" dropdown; tab colour panel; agent runtime dropup; browser sign-in list | `--main-bg-color` (alpha: see-through) | 0 8px 24px at .3 | 0 |
| `.ctx-menu` | In-app Solid context menu (files, remotes, editor) | `--modal-bg-color` (opaque) | 0 8px 24px at .5 | `--radius-md` |
| `popover-frame` | Status-bar readouts: CPU cores, disk, token usage, history, update, attention, backend, host | `--modal-bg-color` | 0 4px 12px at .4 | 0 |
| Own, alpha | Version instance panel; agent slash list; Ctrl+R history search; side-question overlay; node peek (blur 6px); browser address suggestions | `--main-bg-color` (alpha) | five different shadows | 0 or none |
| Own, opaque | Status-bar tip; settings search results; backdrop picker; accounts chooser; toasts; tooltips | `--modal-bg-color` / `--elevated-bg-color` | five different shadows | mixed |
| Dialogs, not menus | Command palette; type-ahead | alpha / opaque | `--shadow-modal` | 0 |

Facts that matter: radius tokens are all 0 today, but written two ways (`0` and `var(--radius-md)`), so a future token change would move only some surfaces. Separators are `--tint-4` on `.menu-divider` and `--border-color` on `.ctx-menu-sep`. The z-index comment at theme.scss:714 says `--z-transient` covers the context menus; it does not. Not traced: the browser bookmark and profile menus' container (they look like `.menu`), the `.ctx-menu` mount, the tab panel's z-index.

## 5. Proposal

**One shared menu surface.** A `menu-surface` mixin (replacing the body of `menu-frame`; `popover-frame` and `.ctx-menu` use it too) and five theme tokens:

| Token | Meaning | Default (dark) |
|---|---|---|
| `--menu-bg-color` | the surface: opaque, never derived from `--window-opacity` | per-theme literal, section 5.2 |
| `--menu-edge-color` | the 1px border | per-theme literal |
| `--menu-ring-color` | a 1px outer ring outside the border, for light or mid-grey backdrops | `rgba(0, 0, 0, 0.55)` |
| `--menu-shadow` | drop shadow | `0 8px 24px rgba(0,0,0,.55), 0 1px 3px rgba(0,0,0,.35)` dark; `.25` / `.18` in light themes |
| `--menu-separator-color` | the row separator | the edge colour mixed 55% into the surface |

Hover stays `--hover-bg-color`, text stays `--main-text-color`. Radius stays 0 (the hard-corners rule), written once, in the mixin.

### 5.1 The stated minimum

For every built-in theme, a test pins:
1. **Surface vs pane: at least 1.2:1** in dark themes (menu lighter than the pane) and the menu white (or its lightest allowed step) in light themes, where the pane is already at 1.08 to 1.14 from white.
2. **Edge vs pane: at least 3:1** (the WCAG 1.4.11 number) in the option the operator picks as "firm" (section 5.3 B); at least 1.6:1 in the "quiet" option (A).
3. **Item text vs the menu surface: at least 4.5:1** (all 13 themes already pass, section 3; the new surface must keep it, and `--hover-bg-color` over it must keep it too).
4. **Against an arbitrary backdrop** (a web page, a terminal in another colour, a transparent window over the desktop) no single edge colour can reach 3:1 against every grey: I computed the worst case across greys with the border and the outer ring together, and it is 1.6 to 2.1:1 (on mid greys, 60 to 90 of 255). So the claim is: the surface is opaque, the edge reaches the stated ratio against the pane, and on any backdrop one of the two rings is at least 1.6:1. The lighter surface and the shadow carry the rest. I do not claim 3:1 against any background.
5. **Opacity:** `--menu-bg-color` has alpha 1 in every theme. A menu looks the same at window opacity 0.3 and 1.

### 5.2 Values per theme (literal hex, as `--modal-bg-color` is)

Computed by `menu-spec/gen.mjs`: surface is the pane mixed 9% toward the text colour in a dark theme, white in a light theme. A and B are the two edge options.

| Theme | Pane | Surface | Surface vs pane | Edge A (quiet) | Edge B (firm) |
|---|---|---|---|---|---|
| dark | #222222 | #353535 | 1.30 | #434343 (1.61) | #6d6d6d (3.08) |
| catppuccin | #1e1e2e | #2e2f40 | 1.24 | #40404d (1.60) | #6b6b75 (3.09) |
| dracula | #282a36 | #3b3d47 | 1.31 | #484a54 (1.62) | #73757c (3.08) |
| gruvbox | #282828 | #3a3834 | 1.26 | #484848 (1.62) | #717171 (3.03) |
| high-contrast | #000000 | #171717 | 1.17 | #303030 (1.60) | #595959 (3.01) (keeps its own white border, 3.66) |
| midnight | #0a0c16 | #1e202a | 1.20 | #34353e (1.60) | #5d5f65 (3.05) |
| monokai | #272822 | #3a3b35 | 1.31 | #474843 (1.62) | #70716d (3.03) |
| nord | #2e3440 | #3f4550 | 1.29 | #4d525d (1.60) | #797d85 (3.03) |
| tokyo-night | #1a1b2e | #292b40 | 1.22 | #3f3f4f (1.65) | #686975 (3.10) |
| light | #f6f6f7 | #ffffff | 1.08 | #c5c5c6 (1.60) | #8f8f8f (3.01) |
| catppuccin-latte | #eff1f5 | #ffffff | 1.13 | #bdbec2 (1.64) | #88898c (3.08) |
| gruvbox-light | #fbf1c7 | #ffffff | 1.13 | #c6be9d (1.64) | #8f8971 (3.08) |
| solarized-light | #fdf6e3 | #ffffff | 1.08 | #cac5b6 (1.60) | #938f84 (3.01) |

Item hover (`--hover-bg-color`, a tint over the surface) and the danger row keep working because the surface moves only one step; the build checks both against 4.5:1 in the same test.

### 5.3 Two options for the edge, to choose by looking

- **A, quiet:** lifted surface, edge 1.6:1 against the pane, shadow. Looks like VS Code or shadcn.
- **B, firm:** lifted surface, edge 3:1 against the pane, shadow. Reads as a clear outline, like Primer's ring.

Both are in the mock-up sheets (dark themes and light themes; columns: today, A, B), kept with the author and not in the repo. The text behind is a hand-made busy pane; the menu, the theme and the "today" look are the real compiled styles. Today's column uses window opacity 0.6, as the screenshot that prompted this suggests.

**My recommendation: B.** The operator's complaint is "not clearly delineated"; A is delineated but subtle, B is unmistakable and meets the standard's number. If it feels heavy, ease the edge toward A in one place (the token).

### 5.4 Scope of the build

In: every surface in the first four rows of section 4 that is a menu or a list opened from a control: `.menu`, `.ctx-menu`, `popover-frame`, the instance panel, the slash list, history search, address suggestions, node peek (its blur stays only if the surface under it stays at 4.5:1; I recommend dropping the blur and using the opaque surface), settings search results, and the tab colour panel. Out, listed as open questions: tooltips, toasts, accounts chooser, backdrop picker, and the two dialogs.

### 5.5 Not in this build

Making the two item layouts match (`.ctx-menu` rows have `5px 12px` padding and `.menu-item` rows `var(--space-1) var(--space-1-5)`). Only the surface unifies; the spec for the row metrics is a follow-up if the operator wants one.

## 6. The mock-ups

Two sheets, one for the 9 dark themes and one for the 4 light ones: each row is one theme, three columns: today, A, B. Headless Edge render of the real compiled `theme.scss`, theme files and `flyoutmenu.scss`; icons are plain glyphs, not Font Awesome. They are a mock-up, not a screenshot of the app; the build's screenshots come from a task dev build.

## 7. The build (#47919), for the record

One PR: the mixin and five tokens in `theme.scss` and each theme; `menu-frame`, `popover-frame` and `.ctx-menu` move onto them, then the own-styled surfaces in 5.4; the stale `--z-transient` comment fixed. A test `themes/menu-surface.test.ts` (like `status-text.test.ts`): per theme, the section 5.1 ratios computed from the real theme files; and a scan that fails when a menu-family stylesheet sets its own `background` or `box-shadow` instead of using the mixin. Before and after screenshots per theme from a task dev build; look checks on macOS (Clare) and Linux (Maricon); changeset; one push; merge on muxreview approval. The operator looks at it in the next release before it is called fixed.

## 8. Risks

- A lighter menu changes how every menu looks in one release. Mitigation: the two options are chosen by the operator by looking; the tokens make a later nudge a one-line change per theme.
- Custom or imported themes without the new tokens: the mixin falls back to `--modal-bg-color` and `--border-color` (opaque, today's `.ctx-menu` look), so they never get the see-through menu again.
- Menus over the terminal (xterm draws on a canvas) and over web content (a native layer in the browser pane) are measured against the theme pane only; a menu over a browser pane sits above the page's own colours, which is the arbitrary-backdrop case in 5.1.4.
- Dropping the node-peek blur changes that one surface's feel.

## 9. Questions for the operator, each with my recommendation

1. **Edge: A (quiet) or B (firm)?** Recommend B.
2. **Light themes: a white menu with shadow and edge?** Recommend yes: a near-white pane cannot be lifted, so the edge and shadow do it.
3. **Should the tooltips, toasts, the accounts chooser and the backdrop picker take the same surface?** Recommend the accounts chooser and backdrop picker yes (they are menus in practice); tooltips and toasts later, they are a different kind of layer.
4. **Should the command palette and type-ahead (dialogs) take it?** Recommend no here; they have a dimmed backdrop and their own surface, and the palette is being redesigned (#47899).
5. **Drop the blur on the agent node peek?** Recommend yes: it is the one surface where translucency is a design choice, and it cannot be proven at 4.5:1 over any text.
6. **Unify the row metrics of `.menu-item` and `.ctx-menu-item` as a follow-up?** Recommend yes, as its own small task, not in this build.

## 10. Decisions

Edge B (3:1). A white menu with shadow and edge on the light themes. The accounts chooser and the backdrop picker take the surface; tooltips and toasts later. The command palette and type-ahead stay out. The node peek loses its blur. The menu surface is fully opaque at any window opacity, and a test keeps it looking the same at 0.3 and 1. The row metrics are a follow-up (#47922).

Found while building: the plain error text fell under 4.5:1 on a hovered danger row in eight themes once the surface lifted, so menus use `--menu-danger-text-color` (the error text 65% mixed toward the text colour); and Light and Solarized Light's edges were 2.994:1 and 2.994:1 against the pane, so each is one step darker than the table in 5.2. A pane with a backdrop maps `--menu-bg-color` to its overlay surface and `--menu-edge-color` to its ink border (backdrop/mapped-tokens.json).
