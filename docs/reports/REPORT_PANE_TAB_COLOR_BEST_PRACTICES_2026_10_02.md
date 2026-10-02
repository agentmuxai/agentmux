# REPORT: colouring pane-header tab pills — what other products do, what the guidelines say, and where AgentMux stands

**Date:** 2026-10-02
**Status:** analysis — research and measurements only; nothing changed. §6 proposes rules for a follow-up spec.
**Author:** AgentY, at the owner's request
**Prompted by:** the owner noticing that AgentX's pane header (an "Accounts" tab plus the red "AgentX" tab) stays the neutral grey instead of red. That is the specified behaviour of `SPEC_PANE_HEADER_TAIL_COLOR_2026_09_21.md` (two different tab colours → neutral tail, with an uncoloured tab counted as its own colour). The owner asked for the best-practice rule for colouring tab pills and headers in a UI like ours: coloured agents plus other kinds of pane.
**Related:** `SPEC_PANE_HEADER_TAIL_COLOR_2026_09_21.md`, `SPEC_AGENT_HEADER_COLOR_UNIFICATION_2026_09_20.md`, `SPEC_AGENT_COLOR_2026_08_08.md`, `SPEC_PANE_TABS_UNIVERSAL_CMUX_REDESIGN_2026_09_17.md`, `SPEC_AGENT_PANE_HEADER_COLOR_THEME_2026_06_23.md`.

## 1. Summary

1. **Every product we looked at keeps the tab row itself neutral and puts identity colour on the individual tab.** None derives the row's colour from the active tab; the one that did (iTerm2's minimal theme) had it filed as a bug. Whole-surface tinting (VS Code + Peacock, Arc, Slack) appears only where the surface has exactly one identity.
2. **The guidelines say the same thing from the other side:** neutral large surfaces, colour in small marks (Fluent, Apple HIG, NN/g's 60-30-10), a subtle tint (about 5–12%) at most on large areas (Material, Atlassian), identity colour stable rather than changing on navigation (NN/g), and selection/focus carried by a separate channel from identity (Primer, Atlassian, Fluent).
3. **Our rule for the header tail is directionally right** (neutral when a pane mixes identities, never following the active tab), but it is **too strict for the owner's case**: an uncoloured utility tab is not a competing identity, so a pane with one agent and some uncoloured tabs has one identity and may keep a stable, subtle tint of it.
4. **Measured on what we ship today (§4):** the active-tab underline fails WCAG's 3:1 non-text contrast against its own pill for red, blue, purple and pink in the dark theme (2.4–2.7:1); in the light theme the header, the coloured pill and the underline are the same colour (1:1), so the active underline is invisible on a coloured tab; and because colours are derived in HSL, perceived lightness ranges from 0.55 (blue) to 0.82 (yellow) in OKLCH, so some agents look much louder than others.

## 2. How AgentMux colours a pane today

From `frontend/app/block/pane-color-menu.ts`, `blockframe.tsx`, `PaneChrome.tsx`, `PaneTabStrip.scss` on main:

| Element | Dark theme | Light theme |
|---|---|---|
| Identity colour (agent, `frame:hue`) | `hsl(h, 65%, 52%)` | same |
| Active-tab underline (`--pane-tab-underline`) and focused pane ring | identity colour | identity colour |
| Coloured pill background (active and inactive) | `hsl(h, 42%, 24%)` | identity colour (full strength) |
| Uncoloured pill background | `hsl(220, 12%, 16%)` (non-agent) or block surface | block surface |
| Header row, single-colour pane | `hsl(h, 28%, 16%)` | identity colour (full strength) |
| Header row, mixed pane (§2 of the tail spec) | `hsl(220, 12%, 16%)` | block surface |

Rule for "mixed" today: compare every tab's resolved header colour; an uncoloured tab counts as a distinct colour; more than one distinct value → neutral tail.

## 3. What other products do

Full survey with sources is condensed here; links in §8.

| Product | Where identity colour goes | Active vs inactive | Container with mixed identities |
|---|---|---|---|
| Chrome / Edge tab groups | chip on the group label + a line under its tabs; tab body neutral | active grouped tab gets an outline in the group colour, same stroke weight | ungrouped tabs carry no colour; 9 fixed colours |
| Firefox containers / tab groups | a coloured line on each tab (+ label in the URL bar) | line on both; URL bar shows the active tab's container | each tab its own line; 8–10 fixed colours |
| Windows Terminal | tab fill (`tabColor`) | inactive coloured tabs automatically dimmed | tab row is a theme colour (focused/unfocused), never derived from tabs |
| iTerm2 | tab fill or outline | inactive tabs outline or low-alpha fill; optional dimming of unfocused panes | empty bar taking the selected tab's colour was filed as a bug (#7614) |
| Kitty / WezTerm | tab fill | separate `active_bg` / `inactive_bg` per tab | bar background independent |
| JetBrains File Colors | pastel fill on tab and tree | separate "selected tab" keys | ordered rules, last match wins |
| VS Code + Peacock | title, activity and status bars per window | unfocused window: same hue at reduced alpha | one identity per window, never mixed |
| Arc Spaces, Slack | whole sidebar per space/workspace | — | one identity per view |
| tmux / Zellij | pane border only, and only for focus | the border colour *is* focus | no per-pane identity |

Recurring patterns: colour in small marks when a container holds several identities; a neutral tab row that does not inherit tab colours; inactive = same hue at lower intensity, active = full intensity; focus as its own signal; uncoloured items stay neutral (nobody invents a "mixed" colour); small fixed palettes resolved per theme, or free colour with derived text contrast.

User complaints that line up with our history: colour that changes on tab switch (iTerm2 #7614, #2593; Firefox collapsed groups), bars made more saturated being "more distracting" (Firefox containers #2939), active tab hard to tell apart (Windows Terminal #18564, #9173), contrast failures from tinting (Windows Terminal #13246), and colour-blind users finding only 5–6 of 9 group colours usable.

## 4. Our current colours against the guidelines (measured)

Computed with the WCAG relative-luminance formula and Oklab (`color_check.py`, kept outside the repo; the inputs are the HSL formulas in §2).

| Hue | underline vs own header (dark) | underline vs neutral header (dark) | **underline vs its pill (dark)** | underline vs header/pill (light) | OKLCH L of identity |
|---|---|---|---|---|---|
| red 0 | 3.25 | 3.10 | **2.61** | **1.00** | 0.58 |
| orange 30 | 4.86 | 5.11 | 3.42 | **1.00** | 0.69 |
| yellow 55 | 7.39 | 8.55 | 4.53 | **1.00** | 0.82 |
| green 120 | 6.79 | 7.56 | 4.33 | **1.00** | 0.76 |
| cyan 185 | 6.56 | 7.30 | 4.21 | **1.00** | 0.76 |
| blue 220 | **3.05** | **2.96** | **2.43** | **1.00** | 0.55 |
| purple 280 | **2.94** | **2.80** | **2.40** | **1.00** | 0.56 |
| pink 330 | 3.42 | 3.31 | **2.71** | **1.00** | 0.59 |

WCAG SC 1.4.11 requires 3:1 for a selection or focus indicator against adjacent colours. White pill text passes easily (7.9–12.8:1), so labels are fine; the indicators are not:
- Dark theme: the active underline sits against its own pill and fails for half the hues; blue and purple are marginal even against the header.
- Light theme: a coloured tab's pill, its underline and the header are all the identity colour, so on a coloured pane the "which tab is active" cue is carried by nothing but the label.
- Equal HSL lightness is not equal perceived lightness: yellow, green and cyan identities read about 1.4× as bright as blue and purple ones.

## 5. Guidelines that apply (with numbers)

1. **3:1 for state indicators** against adjacent colours; inactive components exempt (WCAG 2.2 SC 1.4.11). A 2 CSS px focus perimeter with 3:1 change between states (SC 2.4.13, AAA).
2. **Colour never the only cue** (SC 1.4.1); NN/g on tabs: at least two selection indicators (e.g. shared background with the panel + underline or bold label), no thin low-contrast single-pixel strokes. About 8% of men and 0.5% of women have a colour-vision deficiency; red/green, blue/purple and green/grey collide.
3. **Neutral large surfaces, colour in small accents.** Fluent ("avoid … brand colors … on large surfaces"), Apple HIG ("use color sparingly"), NN/g 60-30-10.
4. **Separate identity from selection/focus.** Primer's accent role is "selected, active and focus"; Atlassian's accent colours "don't communicate any specific meaning" and are for user-chosen categories; Fluent shows focus as a thicker stroke while the control's colour does not change.
5. **Subtle tint for areas, strong colour for marks.** Material state layers 8% hover / 10% focus / 16% drag; the old surface tint was 5–14%; Atlassian pairs subtle accent backgrounds with accent borders to reach 3:1. Large areas also look more saturated than the same colour in a small patch.
6. **Contrast by tone distance.** In Google's HCT, a tone difference of 40 guarantees about 3:1 (50 for about 4.5:1; check exact values).
7. **Normalise user colours perceptually.** Keep the hue, clamp OKLCH lightness and chroma per theme so no identity is louder; HSL lightness differs by hue.
8. **Palette size.** 5–7 colours for fast search, 12 at most (Healey, Ware, Spectrum); shipped products use 8–9.
9. **Identity colour stays put;** don't recolour large areas on navigation (NN/g consistency, change blindness).

## 6. Proposed rules for AgentMux pane tabs

These are proposals for a follow-up spec, not decisions.

- **P1. The header row never follows the active tab.** (Already true for mixed panes; keep it.)
- **P2. Count identities, not tabs.** An uncoloured tab (Accounts, CPU, terminal without a colour) is not an identity. If a pane's coloured tabs share **one** identity, the tail may carry a **stable, subtle tint** of it whichever tab is selected; with two or more identities it is neutral; with none, neutral. This fixes the owner's AgentX case without bringing back colour that moves on tab switch.
- **P3. Keep the tint subtle on large areas.** Header tint at roughly 8–12% of the identity over the neutral header (dark) and the same on light, instead of the light theme's full-strength header. Large surfaces stay calm; the pills carry identity.
- **P4. Pills carry identity; the active pill adds a second, non-hue cue.** Inactive coloured pill: low-intensity identity tint. Active pill: connects to the pane body (shared surface, as NN/g and Chrome do) plus a 2 px identity-coloured stroke and a heavier label. Uncoloured pills stay neutral.
- **P5. Indicators meet 3:1 in both themes.** Derive the underline/focus colour per theme from the identity hue in OKLCH so it clears 3:1 against both the pill and the header (dark: raise lightness; light: darken), checked by a unit test across the hue palette.
- **P6. Pane focus is its own signal.** Thickness or glow of the pane ring (and optionally dimming unfocused panes), not a change of hue.
- **P7. Normalise identity colours in OKLCH.** Keep the user's hue; fix L and C per theme for the identity, pill and tint values, so a yellow agent is not louder than a blue one. Keep presets to about 8–10 and allow custom hues through the same normalisation.
- **P8. Colour never alone.** Agent name (and icon where present) always on the pill; consider warning when two open agents' hues collide for common colour-vision deficiencies.

## 7. Suggested next steps

1. Owner decision on P2 (the AgentX case) — small, isolated change to `headerTailBg` in `PaneChrome.tsx` plus the tail spec's §2/§3.1 and tests; can ship on its own.
2. A spec for P3–P7 with before/after screenshots in both themes, since it changes the look of every pane.
3. Fix the measured contrast failures (§4) as part of that spec: the light-theme underline is the most visible problem.

## 8. Sources

Products: Chrome tabGroups API https://developer.chrome.com/docs/extensions/reference/api/tabGroups · Chromium tab_style_views.cc https://chromium.googlesource.com/chromium/src/+/166ca3b2fffc3d6f72a987d81de82c88eece9454/chrome/browser/ui/views/tabs/tab_style_views.cc · XDA on tab groups https://www.xda-developers.com/google-chrome-tab-groups/ · Firefox tab groups https://support.mozilla.org/en-US/kb/tab-groups · Firefox containers colours https://developer.mozilla.org/en-US/docs/Mozilla/Add-ons/WebExtensions/API/contextualIdentities/getSupportedColors · containers saturation complaint https://github.com/mozilla/multi-account-containers/issues/2939 · Windows Terminal profile appearance https://learn.microsoft.com/en-us/windows/terminal/customize-settings/profile-appearance · themes https://learn.microsoft.com/en-us/windows/terminal/customize-settings/themes · dimmed tabs https://github.com/microsoft/terminal/issues/13246 · active tab hard to see https://github.com/microsoft/terminal/issues/18564 · iTerm2 tab bar follows selected tab (bug) https://gitlab.com/gnachman/iterm2/-/work_items/7614 · iTerm2 appearance https://iterm2.com/documentation-preferences-appearance.html · Kitty set-tab-color https://man.archlinux.org/man/extra/kitty/kitten-@-set-tab-color.1.en · WezTerm https://wezterm.org/config/lua/window-events/format-tab-title.html · Warp tabs https://docs.warp.dev/terminal/windows/tabs/ · JetBrains File Colors https://www.jetbrains.com/help/idea/configuring-scopes-and-file-colors.html · VS Code theme colours https://code.visualstudio.com/api/references/theme-color · Peacock https://github.com/johnpapa/vscode-peacock/blob/main/docs/guide/README.md · Arc https://tidbits.com/2023/05/01/arc-will-change-the-way-you-work-on-the-web/ · Slack themes https://slack.com/help/articles/205166337-Change-your-Slack-theme · tmux https://www.nevis.columbia.edu/cgi-bin/man.sh?man=1+tmux · Zellij themes https://zellij.dev/documentation/themes.html

Guidelines: WCAG 2.2 non-text contrast https://www.w3.org/WAI/WCAG22/Understanding/non-text-contrast.html · use of colour https://www.w3.org/WAI/WCAG22/Understanding/use-of-color.html · focus appearance https://www.w3.org/WAI/WCAG22/Understanding/focus-appearance.html · NN/g tabs https://www.nngroup.com/articles/tabs-used-right/ · NN/g colour https://www.nngroup.com/articles/color-enhance-design/ · NN/g preattentive https://www.nngroup.com/articles/dashboards-preattentive/ · NN/g change blindness https://www.nngroup.com/articles/change-blindness/ · Fluent 2 colour https://fluent2.microsoft.design/color · Apple HIG colour https://developer.apple.com/design/human-interface-guidelines/color · Primer colour usage https://primer.style/product/getting-started/foundations/color-usage/ · Atlassian accents https://atlassian.design/foundations/color/accents/ · Material colour roles (Android docs) https://github.com/material-components/material-components-android/blob/master/docs/theming/Color.md · HCT tone/contrast https://github.com/material-foundation/material-color-utilities/blob/main/typescript/hct/hct.ts · OKLCH in CSS https://evilmartians.com/chronicles/oklch-in-css-why-quit-rgb-hsl · Oklab https://bottosson.github.io/posts/oklab/ · Healey 1996 https://www.cs.ubc.ca/sites/default/files/tr/1996/TR-96-10_0.pdf · colour-vision deficiency (NEI) https://www.nei.nih.gov/learn-about-eye-health/eye-conditions-and-diseases/color-blindness/types-color-blindness

Two caveats: the Material state-layer and surface-tint numbers come from sources quoting the official tokens (the Material site renders only with JavaScript), and Chrome's "outline on the active grouped tab" is from XDA and an older Chromium file; the 2023 redesign may render it differently.
