# SPEC: the colour you pick is the colour you see (tabs first), with the filters and overlays audited

**Date:** 2026-11-18
**Author:** Camper@narko
**Status:** active. Approved by the operator, 2026-11-18 (relayed by AgentY): every decision of §2 as recommended, except D3, where the operator chose candidate C (§5.3), which changes the bar (§5.7). Spec task #48010; built as #48011.
**Verified against:** `origin/main` @ `8a9f964262`, by reading the files cited, and by running the shipped colour functions (`scratch/colour-audit.ts`, `scratch/colour-audit2.ts`: they import `theme/palette.ts` and `block/pane-color-scheme.ts` and print what each surface paints). **Not yet measured on a running build:** the numbers below come from the same functions the app runs and from the CSS cascade as written; a computed-style and screenshot pass in an isolated task dev is phase 0 of the build (§8).
**Related, read first:** [`SPEC_SHARED_PALETTE_12_HUES_BLACK_WHITE_2026_11_16.md`](SPEC_SHARED_PALETTE_12_HUES_BLACK_WHITE_2026_11_16.md) (the 14 slots and their role tones), [`SPEC_TAB_COLOR_DESATURATION_2026_08_13.md`](SPEC_TAB_COLOR_DESATURATION_2026_08_13.md) (why tab fills are quiet), [`SPEC_PANE_BACKGROUND_THEMES_2026_11_15.md`](SPEC_PANE_BACKGROUND_THEMES_2026_11_15.md) (backdrops; Agent1's area).
**Boundary:** pane backdrops, their tone-move and ink, and the shared menu surface belong to Agent1. This spec only proposes how their swatch is drawn (§6, phase 4) and marks each such change "Agent1 decides".

## 0. The ask

A person picks a colour for a tab (or a pane, a widget, a backdrop) and the thing does not become that colour. The picker shows one solid colour, while a tab is a selected look, an unselected look and a highlight bar, drawn in a theme, with filters and overlays that nobody listed. They want the cause found, every filter or overlay over a coloured element audited and cleaned up, and a spec so that what is picked is what is seen.

## 1. Summary

1. **The swatch is the bar, not the tab.** A hue's swatch is painted in the `identity` tone (`slotSwatchColor`, `pane-color-scheme.ts:127-129`). On a window tab that tone paints only the 2 px bar of the selected tab (`tab.scss:84-97`, `225-229`). The fill of the selected tab is the `pillActive` tone and of the others the `pill` tone: much darker (dark theme) or paler (light theme) and about half the chroma. Swatch to selected fill is **36.5 ΔE** in the dark theme and **34.8** in the light one, to unselected **43** and **40** (ΔE is OKLab distance times 100; 1 to 2 is the least a person sees). Nothing in the picker says so.
2. **One pick, six different colours, and surfaces choose among them differently.** The stored value is a theme-neutral hex (L 0.70, C 0.14: never drawn). The swatch, the bar, the focused ring and the agent's name use `identity`; the selected fill uses `pillActive`; the unselected fill `pill`; a pane header `headerTint` (49 ΔE from the swatch); an unfocused ring `border` (30 ΔE); a top-bar widget icon `widgetTint` (10 ΔE). A browser profile's mark paints the **stored hex raw** (`browser-tab-badge.ts:16-19`): 5 ΔE from the swatch in the dark theme, **18 ΔE in the light** one. Black and White are worse: the swatch is pure black or white, the bar is a grey (`#9e9e9e` on dark for Black, `#696969` on light for White).
3. **Hover and activity effects move the colour by more than selection does.** A hovered coloured window tab gets `filter: brightness(1.12)` on the whole tab (`tab.scss:231-239`), which also lifts the bar the swatch matched (3.5 to 6.8 ΔE in dark) and the active tab (the rule has no `:not(.active)`); a hovered pane tab pill gets `brightness(1.4)` (`PaneTabStrip.scss:330-333`): **8.4 ΔE in dark, 9.9 in light, more than the step from unselected to selected (6.6 and 5.8)**. So hovering a tab shows a colour further from its resting colour than selecting it does.

The fix is one pipeline and one honest picker (§4, §5): the picker shows the tab as it will be drawn (selected fill and bar, in the current theme), every state is a named token and not a filter, and a test pins the swatch to the painted colour for every surface, state and theme.

## 2. Decisions for the operator

| # | Decision | Recommendation |
|---|---|---|
| D1 | The principle. A: the swatch shows what the selected, focused element looks like in this theme, every other state is a defined step from it. B: the picked colour is used raw on one named part (the bar) and the rest is derived, said in the picker. | **A.** It is what "the colour I picked" means to a person looking at a tab; B is what happens today without saying so. |
| D2 | Make the picker honest by drawing the swatch as a small sample of the tab (selected fill with its bar) or keep a solid swatch and change the tab to be closer to it. | **Both, mildly:** the swatch becomes a two-part sample (fill and bar), and the selected fill is lifted a step toward the bar's tone (§5.3 shows the three candidates for you to compare as screenshots before one is chosen). The quiet look asked for in August stays: unselected tabs do not change. |
| D3 | How strong should the selected tab's fill be: today (dark `L 0.39 C 0.09`), a step (`L 0.45 C 0.12`), or the bar's own tone as fill. | Recommended the step. **Decided: C** (dark `L 0.52 C 0.13`, light `L 0.78 C 0.13`), from the screenshots of both themes. The bar changes with it (§5.7). |
| D4 | Hover and activity effects: replace every `filter` and overlay on a coloured element with a named token one step from the resting colour. | **Yes.** No filter touches a user-coloured element; the guard (§7) enforces it. |
| D5 | Black and White: the swatch shows the sample like every colour (so Black on a dark theme is shown as the grey bar and the dark fill it makes), with their names and check mark kept. | **Yes.** A pure-black chip that paints a grey bar is the clearest mismatch in the audit. |
| D6 | A colour that cannot keep its label readable (the light theme's Black tab: label 4.2:1) is adjusted, and the sample shows the adjusted colour. | **Adjust the fill, never the text, and show it.** Floors in §5.4. |
| D7 | Colours people already chose. | **Nothing is migrated:** the stored value is a palette slot, the tone is derived at paint time. What changes is stated per phase (§6) and visible in the before/after screenshots. |
| D8 | Phase order. | Tabs, then pane tabs and headers, then widgets, profiles and agents, then backdrops (Agent1 decides). |

## 3. The audit

### 3.1 Where a person picks a colour, and what it then paints

| Picker | Stored | Paints (role tone, `pane-color-scheme.ts:37-89`) |
|---|---|---|
| Window tab, right-click, "Tab color" (`tab.tsx:61-66`) | `tab:color` = the slot's stored hex (`slot.hex`, theme-neutral) | Selected fill `pillActive`, others `pill`, bar `identity` (`tab.tsx:31-37`) |
| Pane, context menu "Pane Color" (`blockframe.tsx:169-194`) | `frame:color` = slot id (`pane-color-menu.ts:28-38`); also the agent's `ui:color` hex when an agent pane | Header `headerTint` (`blockframe.tsx:69-75`, `694-724`), pane tab pills `pill`/`pillActive` (`:77-94`), pill bar `identity`, focused ring `identity`, unfocused ring `border` (`blockframe.tsx:1015`, `1052`), the agent's name `identity` |
| Settings > Widget colors (`widget-colors.tsx`) | `pane:colors` slot id | Every pane of that widget type as above; the top-bar widget icon `widgetTint` (`action-widgets-config.ts:53`) |
| Browser profile colour (`new-profile-modal.tsx:60`, `browser-section.tsx:228`) | the slot's stored hex | The profile mark: the **stored hex raw**, only a neutral is toned (`browser-tab-badge.ts:16-19`) |
| Agent identity colour (`ui:color`) | slot hex / `frame:activebordercolor` snapshot at launch | As the pane's `identity`, `border`, `headerTint` through `resolvePaneIdentity` (`pane-identity.ts:87-102`) |
| Pane backdrop (`frontend/app/backdrop/`) | `backdrop` record (built-in id or custom hex) | The surface after `nudgeSurface` (up to 0.08 of OKLCH lightness for contrast, more for a "readable" colour: `ink.ts:92`, `181-198`) and its ink; the picker notes a nudge |

`slotOf` (`palette.ts:90-104`) snaps any older stored value to the nearest of the 12 hues (or Black/White by lightness), so an old picked colour is already a slot when drawn.

### 3.2 What a pick of Blue paints, per state and theme

| Part | Dark: colour | ΔE from swatch | Light: colour | ΔE from swatch |
|---|---|---|---|---|
| Stored value | `#61a0f4` | 5.0 | `#61a0f4` | 18 |
| Swatch / bar / focused ring | `#75b0ff` | 0 | `#0e66c8` | 0 |
| Selected window tab fill | `#214574` | 36.3 | `#b3d4ff` | 35.5 |
| Unselected window tab fill | `#203654` | 42.5 | `#d0e3fc` | 41.0 |
| Pane header tint | `#1b2737` | 49.0 | `#e4f0ff` | 45.4 |

Across the 12 hues the spread is small (dark: 36.3 to 36.7 selected, 42.5 to 43.1 unselected, 48.9 to 49.6 header; light: 33.9 to 35.5, 39.2 to 41.2, 43.3 to 45.6): the gap is the scheme's design, not one hue's fault. Other roles against the swatch: unfocused ring 30.3 to 30.8 (dark), 27.9 to 29.8 (light); top-bar widget icon 9.1 to 10.7 and 7.0 to 10.8.

### 3.3 Black and White

| Part | Dark: Black | Dark: White | Light: Black | Light: White |
|---|---|---|---|---|
| Swatch | `#000000` | `#ffffff` | `#000000` | `#ffffff` |
| Bar / ring | `#9e9e9e` | `#f5f5f5` | `#0b0b0b` | `#696969` |
| Selected fill | `#000000` | `#636363` | `#a4a4a4` | `#ffffff` |
| Unselected fill | `#0d0d0d` | `#4d4d4d` | `#bebebe` | `#fafafa` |
| Selected fill ΔE from swatch | 0 | 50 | 72 | 0 |

The role tones for neutrals are chosen so a line can be seen on the theme (`pane-color-scheme.ts:80-89`); the swatch is not. In the light theme a Black tab's label (secondary text on `#bebebe`) is **4.2:1**, under the 4.5 floor.

### 3.4 Filters, overlays and states over a coloured element

| Where | What | For | Effect on the picked colour | Verdict |
|---|---|---|---|---|
| `tab.scss:231-239` | `filter: brightness(1.12)` on `.tab-inner` of a hovered or dragged coloured tab, **active tab included** | A hover lift | Fill +3.1 ΔE (dark) to +7.0 (light); lifts the bar and the label too (bar +3.5 to +6.8 dark) | Replace by a `tabHover` token (§5.2); never on the active tab |
| `PaneTabStrip.scss:330-333` | `filter: brightness(1.4)` on a hovered coloured pane pill | A hover lift | 8.4 ΔE dark, 9.9 light: more than selection (6.6, 5.8) | Replace by a `pillHover` token a step (about 3 ΔE) from `pill` |
| `tab.scss:74-81` | `::before` fill `rgb(from main-text r g b / 0.12)`, opacity driven by the activity flash | Tool-call activity | Washes the fill and bar toward the text colour while it runs | Keep as a defined `flash` step; make it not cover the bar |
| `PaneTabStrip.scss:296-306` | `::before` `oklch(from underline l+0.06 c×1.4 / 0.9)` | Activity on a pane pill | Brighter, more saturated version of the bar tone | Keep as a token (`pillFlash`); drive from the same module |
| `tab.scss:175-188`, `PaneTabStrip.scss:308-319` | Hover tint `rgb(from main-text / 0.1)` and `--tint-1` over the neutral background | Hover on an uncoloured tab | None on a coloured one (the colour rules win with `!important`) | Keep; the `!important` goes with the tokens |
| `tab.scss:156`, `166` | `background: … !important` | Make the pick beat state backgrounds | Hides every state rule above | Remove once states are tokens |
| `tab.scss:99-101`, `159-170` | Label colour `--secondary-text-color` / `--main-text-color` | Readability | Not adjusted to the fill: 4.2:1 for light-theme Black | §5.4 |
| `PaneChrome.scss:56-69`, `87-88`, `104-115` | Ring `::after`, busy band `color-mix(… 40% / 70%)`, locate flash `color-mix(… 90%)` and `16%` wash | Selection, busy, locate | The ring is the swatch tone (0 ΔE) when focused; the busy and flash mixes are by design, transient | Keep; list as named states |
| `blockframe.tsx:712`, `721` | Header text `pickReadableTextColor(bg)` = pure black or white | Readability | Text only | Keep (contrast floor already held); route through §5.4 |
| `block.scss:65-66`, `88-93`, `274` | `block-bg-color` at 50 to 70 percent on preview, ephemeral and magnified headers | Previews | Only an uncoloured header (an inline colour wins) | Keep; check in phase 2 that a coloured header never loses its inline colour |
| `block.scss:113`, `tilelayout.scss:402` | `backdrop-filter: blur(--magnified-block-blur / --block-blur)` | Frosted magnified or tiled block | Blurs what is behind a translucent surface | Keep; Agent1 owns (#47696) |
| Window opacity (`app.tsx:107-131`, `theme.scss`) | `--window-opacity` in `--main-bg-color` | Transparent windows | Only translucent surfaces; coloured fills are opaque hex | Keep; phase 0 measures one translucent window |
| `drop-feedback.scss`, `drag/drop-indicators.scss` | `filter: invert()` flash on a drop target | Drop attention | Temporary, over a tab | Keep (motion, not a colour state) |
| `tower-view.scss:1053` | `filter: brightness(1.15)` | A Tower element | Not a user-coloured surface in the paths read | Out of scope |

Other places a `filter` or `opacity` sits in `frontend/**/*.scss` were read and none is on a user-coloured surface: `window-controls.darwin.scss:65`, `pdf-viewer.scss:33`, `modal.scss:91`, and the blur uses listed by `grep`.

### 3.5 Is the swatch honest

| Picker | Drawn with | Honest? |
|---|---|---|
| Window tab, Settings widget colours, browser profile | `slotSwatchColor` (`palette-swatches.tsx:47-50`): `identity` tone for a hue, the stored hex for Black and White | **No** for a tab (the bar's tone only), for a widget (an icon at `widgetTint`, 10 ΔE away), and for a profile in the light theme (the mark is the stored hex, 18 ΔE away) |
| Pane Color submenu | `slotSwatchColor` (`blockframe.tsx:180`) | Matches the ring (0 ΔE); not the header, pills or border |
| Backdrop picker | A small pane in its own ink (`BackdropPicker.tsx`) | Close; a nudge and its numbers are reported in a readout, which is the right pattern |

## 4. Cause

The palette spec fixed the identity tone, and the 2026-10 report fixed the quiet tones for fills so that agents do not shout and the bar still clears 3:1. Both were right for a coloured pane. The picker then took the one tone that is the loudest, and nobody connected it to the tab, which is mostly fill. Hover effects were added as `filter`s per component rather than as states of the colour, which is why they are bigger than the selection step and reach the bar.

## 5. The spec

### 5.1 Principle (D1)

The picker shows what the **selected, focused** element looks like in the current theme (its fill and its bar). Every other state (unselected, hovered, flashing, dragging, unfocused) is a named step from it, defined once. A colour that cannot read is adjusted before it is drawn, and the sample shows the adjusted colour.

What a person should expect for a tab with a bar, in dark and in light: the sample in the menu is the tab they get when it is selected; unselected tabs are the same hue, quieter, by the same step as today; hovering moves a tab one small step, never to a colour the menu did not show.

### 5.2 One pipeline

One module, `theme/paint.ts` (it extends `pane-color-scheme.ts`, whose `PANE_COLOR_TOKENS` and `NEUTRAL_TONES` stay the one place to tune):

```
paint(slot, theme, surface, state) -> { fill, bar, ink }
surface: windowTab | paneTab | header | ring | widgetIcon | profileMark | agentName
state:   selected | unselected | hover | flash | focused | unfocused
```

- A surface never reads `slot.hex`, `identity` or any tone itself: it asks `paint`. The stored hex stays in the file and is read only to find the slot (`slotOf`).
- `hover` and `flash` are tones in the same table, not `filter`s or `rgb(from …)` on top. `windowTab.hover` is one step from `unselected` and from `selected`.
- The picker draws `sample(slot, theme, surface)`, the same call with `state: selected`: for a tab, a small tab (fill, bar, a short label in the real ink). For a surface that is a line or an icon (ring, widget, profile) the sample is that part itself, so a ring's swatch can stay a dot.
- Every tone is a CSS custom property the component sets from `paint` (`--tab-fill`, `--tab-bar`, `--tab-ink`, per state), so the stylesheet holds no colour arithmetic and no `!important`.
- Removed: both `brightness()` filters; the `!important` on the tab fill; the profile mark's raw hex (it takes `profileMark`); the neutral special case in `profileMarkColor` (folded into the table).

### 5.3 Tab tones (D2, D3)

The selected fill is candidate **C**: the bar's own hue at `L 0.52 C 0.13` (dark) and `L 0.78 C 0.13` (light). The unselected fill is today's (`L 0.33 C 0.06` dark, `L 0.91 C 0.04` light), so an unselected tab is clearly unselected beside a selected one (15 to 20 ΔE for the hues). Black and White keep tab tones of their own (`TAB_NEUTRAL_TONES` in `theme/paint.ts`), because a black fill on a dark theme and a white one on a light theme are the surface itself. The earlier candidates (A, today; B, a step stronger) were compared as screenshots and are not built.

### 5.4 Readability

Floors, held by `paint` and tested: label on its fill 4.5:1 for selected and unselected and hover; bar on its fill 3:1, and 3:1 on the strip behind it; ring on the surface behind it as today (`pane-color-scheme.test.ts`). A colour that fails is moved in OKLCH lightness only (hue and chroma kept, the method `toneMove` in `backdrop/ink.ts:247` already has), and the sample shows the result with the usual name and check mark. The light theme's Black tab today (4.2:1) is the first case.

### 5.5 Themes

The role tones depend on dark or light only (`isLightThemeActive`, `LIGHT_THEME_IDS` in `menu/base-menus.ts`: light, catppuccin-latte, solarized-light, gruvbox-light), while the surfaces they sit on and the text tokens vary across the 13 themes (`frontend/app/themes/*.scss`, `theme.scss`). The audit's contrast figures use the default dark theme's text and the white and black surfaces. The build's test runs the table below for **every theme's own text tokens**, so a theme whose text is dimmer (for example `tokyo-night`'s `#9aa5ce`) is caught, not assumed. A theme-specific adjustment, if any is needed, goes in `paint`, never in a stylesheet.

### 5.6 Migration (D7)

No stored value changes. Phase 1 changes what the selected tab's fill looks like (candidate B) and removes hover brightening; the unselected tab and the bar are the same. Phase 3 changes a browser profile's mark in the light theme (from the raw pastel hex to the `identity` tone the swatch always showed, an 18 ΔE change). The build lists each change per phase in its PR with before and after screenshots.

### 5.7 The bar (follows from D3)

C's brighter fill leaves the identity tone under the 3:1 floor against the selected fill (2.5 for amber in dark, 2.7 for green in light). The bar stays the picked hue and is moved in lightness only, away from the fill (lighter on a dark theme, darker on a light one, the side of the strip behind it), until it holds 3.15:1 against the selected fill and its hover. So it also holds against the strip: 3:1 or better on every slot in both themes. It never changes with the state.

### 5.8 What the build does with the theme's own text

The label is drawn in the theme's `--main-text-color` (selected) and `--secondary-text-color` (others). `paint.ts` takes those two colours (`tab/tab-ink.ts` reads them from the document once the window's theme attribute is on it) and makes every fill, hover included, hold 4.5:1 for the label and 3:1 for the close mark under them. The table test runs the same checks under all 13 themes' own tokens.

## 6. Phases (the operator can try each on its own)

| Phase | Scope | Screenshots | Look checks |
|---|---|---|---|
| 0 | A task dev pass: computed styles of the tab, pane pill, header and ring for the 14 slots in dark and light, hover and flash states, one translucent window; compares them with §3 | The numbers | None (no visible change) |
| 1 | **Window tabs:** `paint` for `windowTab`, the sample swatch in the tab menu, tab tones (D2, D3), hover and flash as tokens, no `!important`, no `filter` | Before and after: 14 slots x selected, unselected, hover x dark, light | macOS and Linux |
| 2 | **Pane tabs, header, ring:** `paneTab`, `header`, `ring`; the Pane Color menu's sample; remove `brightness(1.4)` | As above, for a pane with one and with several tabs | macOS and Linux |
| 3 | **Widget icons, browser profile marks, agent names, Swarm rows, attention tints:** one call each; the profile mark fix | As above | macOS and Linux |
| 4 | **Backdrops (Agent1 decides):** the backdrop swatch and readout use the same sample rule; the nudge is shown in the picker as it is now | Before and after for a nudged custom colour | macOS and Linux |

## 7. Tests and guards

- **Table-driven:** for each slot (14), theme (dark, light, and every theme's own text tokens for the contrast rows), surface and state, the sample's colours equal what `paint` returns and what the rendered element's computed style resolves to (jsdom for the custom properties; the phase 0 pass checks one real window). A row fails if a swatch and its painted surface differ by more than the stated step.
- **Floors:** the contrast floors of §5.4 for every row.
- **Guard:** `scripts/check-colour-surfaces.mjs`, a CI step next to the existing gates. It reads the stylesheets and fails on a `filter`, `opacity`, `mix-blend-mode` or `backdrop-filter` declared on a selector that names a user-coloured element (a list kept in the script: `.tab-colored`, `.pane-tab--colored`, the header, ring and pill classes, `.palette-swatch`), or on `!important` over a fill the pipeline sets. An exception needs a comment with a reason in the script's own table, as `check-function-sizes` does.
- **No raw reads:** a test (or the same guard) fails when a component outside `theme/` reads `slot.hex` or the `identity` tone to paint a surface.

## 8. Phase 0: measured on a running build

Done on 2026-11-18, in an isolated off-screen task dev of `origin/main` @ `4b18c7c574` (dark and light theme, all 14 slots, a selected and an unselected window tab, synthetic pointer only). Each slot was set the way the tab menu sets it (`UpdateObjectMeta` of `tab:color`), then computed styles were read from the real tab, its `::after` bar and the real tab menu's swatch.

- **The cascade paints exactly what the functions return:** swatch, selected fill, unselected fill and bar are 0.0 ΔE from `slotSwatchColor` / `slotRoleColor` in both themes, for all 14 slots. So §3.2 and §3.3 stand as written; the mismatch is the design, not a stray rule.
- **Swatch to selected fill:** dark 36.3 to 36.7, light 33.9 to 35.5; to unselected: dark 42.5 to 43.1, light 39.2 to 41.2 (hues). The bar is the swatch (0.0), 2 px high.
- **Hover:** `filter: brightness(1.12)` is computed on the hovered tab's inner box for both the unselected **and the selected tab**. A hovered selected tab's fill moves 3.1 to 3.3 ΔE in dark and 5.6 to 7.8 in light, and its bar 3.5 to 6.8 in dark and 4.4 to 4.8 in light; an unselected tab's fill moves 2.4 to 2.7 in dark and 6.5 to 8.1 in light.
- **Black and White:** as §3.3 (Black dark: fill `#000000`, bar `#9e9e9e`; White light: fill `#ffffff`, bar `#696969`; Black light selected fill is 71.9 ΔE from its pure-black swatch).
- **Not measured here:** pane tab pills, headers and rings (phase 2 measures them before it changes them), a translucent window, and the other 12 themes' text tokens (§5.5, in the build's test table).

---
Agent: Camper@narko
