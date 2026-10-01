# Spec: rules for light themes, enforced across every light theme

**Date:** 2026-10-01
**Status:** proposed
**Author:** agent1
**Related:**
- `SPEC_LIGHT_THEME_AND_DEPTH_FIXES_2026_07_11.md` and
  `SPEC_LIGHT_THEME_DEPTH_AND_MORE_THEMES_2026_07_13.md` (the light theme, its depth
  tokens, the three other light themes)
- `SPEC_COLOR_THEME_TOKEN_HARDENING_2026_09_21.md` (the Tailwind lint rule; its §5 made
  pane-colour headers theme-aware, but not the default header)
**Trigger:** the repo owner, 2026-10-01, reviewing the dev build on a light theme:
1. a solid black pane header "looks unnatural", it should be a lighter, non-solid colour;
2. some light foreground text (written for a black background) must be darkened;
3. faded text on light backgrounds (the status bar) must be darker;
4. these must hold across **all** light themes, so what is the best practice?

## 0. Summary

All three symptoms come from one pattern: **a rule is written once, for the dark default,
and each light theme is expected to remember to override it.** Nothing checks that it did.
This spec fixes the three symptoms and adds the missing check: a **contrast and
no-solid-chrome test that runs over every light theme**, so the next light theme (or the
next hardcoded colour) fails CI instead of reaching a screenshot.

The short answer to "is there a best practice": yes, three that fit together:
1. **Semantic tokens per polarity**, never a colour in a component.
2. **A contrast budget** (WCAG 2.2: 4.5:1 for text, 3:1 for UI parts), checked by a
   script over the actual token values.
3. **No de-emphasis by opacity.** Fade with a dedicated "muted" token that meets the
   budget, not by making the text transparent.

## 1. What is wrong today (verified 2026-10-01 on `main` @ `40902363e`)

### 1.1 Solid dark pane headers on light themes

`frontend/app/block/blockframe.tsx`, `headerStyle` (line ~717):

```ts
const NON_AGENT_DEFAULT_HEADER_BG = "hsl(220, 12%, 16%)";   // line 90
…
style["background-color"] = NON_AGENT_DEFAULT_HEADER_BG;       // line 717, no isLightTheme check
```

Every non-agent pane with no colour of its own (terminal, editor, browser, sysinfo,
swarm…) gets this near-black **opaque** header on every theme. In the dev build on the
Light theme the sampled header is `#24272e`, against a `#fdf6e3` page (13.9:1, a hard
black bar on a cream UI). Its tab pill checks `isLightTheme` (lines 101–124), so the
pill and the bar behind it disagree on a light theme.

The agent-pane path is already right: `headerBgForEffectiveColor` keeps the
"bright" colour on a light theme (`pane-color-menu.ts`, §5 of the hardening spec). Only
the non-agent default missed it.

### 1.2 Foreground text tokens that were tuned for a dark surface

Contrast of each light theme's text tokens against its own solid surface
(`--block-bg-solid-color`), computed with the WCAG 2.2 formula:

| Theme | surface | `--main-text` | `--secondary-text` | `--grey-text` |
|---|---|---|---|---|
| light | `#ffffff` | 19.2 | 7.8 | **4.5** (the bare minimum) |
| catppuccin-latte | `#ffffff` | 11.4 | 6.3 | <span class="am-error">3.2</span> |
| solarized-light | `#fdf6e3` | 7.2 | <span class="am-error">2.9</span> | <span class="am-error">2.5</span> |
| gruvbox-light | `#f9f5d7` | 13.1 | 8.0 | <span class="am-error">3.3</span> |

(The surfaces are approximated as the solid token; the real background is a translucent
tint over the window, which can only be lighter or the same. Treat the table as an
upper bound; the CI check in §4 computes the real composite.)

`--grey-text-color` and (in Solarized) `--secondary-text-color` are below the 4.5:1
WCAG minimum for body text. They are used for real content: axis numbers, captions,
timestamps, helper text.

### 1.3 Faded text, the status bar

`StatusBar.scss` fades items with **opacity** (`0.2`, `0.4`, `0.45`, `0.5`, `0.6`
for quiet, idle and disabled stats). Opacity multiplies away contrast on a light
surface. The secondary text colour after those opacities:

| Theme | at 0.6 | at 0.45 | at 0.2 |
|---|---|---|---|
| light | 2.9 | 2.1 | 1.4 |
| catppuccin-latte | 2.6 | 2.0 | 1.3 |
| solarized-light | 1.8 | 1.5 | 1.2 |
| gruvbox-light | 3.0 | 2.2 | 1.4 |

None reaches 4.5:1, and most don't reach the 3:1 that WCAG asks of UI parts. On a dark
theme the same opacities are tolerable because the base text is bright; on a light theme
the base is already mid-grey and the fade finishes it.

## 2. Rules

Each rule is checkable, and each maps to a test in §4.

**R1: No opaque, near-black chrome on a light theme.**
Pane headers, tab strips, title bars, the status bar and any strip that sits on the
window surface use a **translucent tint of the foreground** (`rgba(0,0,0,0.03–0.08)`,
the pattern `--tab-strip-bg` and `--status-bar-bg` already use) or a light tonal step of
the surface, never an opaque colour with relative luminance below ~0.6. The only
exceptions are the terminal and code surfaces, which are content, not chrome.

**R2: Text on chrome is derived, not assumed.**
Text colour on any header or strip is chosen from the **composited** background
(`pickReadableTextColor`, already used at `blockframe.tsx:718`) or from a token that the
theme guarantees against that surface, and it must meet R3. A header tint changing
polarity must change its text with it.

**R3: Contrast budget.** Measured on the composite of the foreground token over the
surface it actually sits on:

| Use | Minimum |
|---|---|
| Body and label text (any text under ~18 px / 14 px bold) | **4.5 : 1** |
| Large text, icons that carry meaning, input borders, focus rings, chart lines | **3 : 1** |
| Disabled / purely decorative | no minimum, but not below 2 : 1 so it is still findable |

This is WCAG 2.2 SC 1.4.3 (text) and 1.4.11 (non-text), the standard most design
systems (GitHub Primer, Radix, Material 3, Atlassian) hold their light themes to.

**R4: De-emphasise with a token, not with opacity.**
Text may not be faded with `opacity`. Add one muted token per theme that already meets
R3 (`--muted-text-color`, in practice `--secondary-text-color` fixed to ≥ 4.5:1) and use
it. "Quiet" stats in the status bar use the muted token; "disabled" uses a dedicated
`--disabled-text-color` (≥ 2 : 1). Opacity stays allowed for **non-text** (dimming an
icon's background, a fade-in).

**R5: Surfaces get depth from tint and border, not from black.**
In a light theme elevation is expressed by a slightly darker or lighter tonal step
and a hairline border (`--border-color`), optionally a soft shadow; never by a black
fill. Pane colours chosen by the user or by an agent identity are mixed toward the
surface (`color-mix(in srgb, <hue> 18%, var(--block-bg-solid-color))`) for headers, and
kept saturated only for the **border** and small accents.

**R6: One place per decision.** A component never contains a colour literal
(`#…`, `rgb(…)`, `hsl(…)`, `black`) for a theme-dependent purpose. It reads a token; the
token is defined for the dark default and for the light polarity once
(`[data-theme-polarity="light"]`), then each light theme only overrides what is
genuinely theme-specific (accent, palette). A new light theme is correct by default.

**R7: Light themes are declared, not inferred.** `LIGHT_THEME_IDS`
(`menu/base-menus.ts`) stays the single list, and a test asserts every theme whose
`--block-bg-solid-color` is light is in it, and vice versa, so a new light theme cannot
silently get dark-theme behaviour.

## 3. Proposed change

### 3.1 Fix the three symptoms

1. **Default header (R1, R2, R5).** Make the non-agent default header a token:
   `--pane-header-bg`, defined in `theme.scss` as today's `hsl(220, 12%, 16%)` for the dark
   default, and in `[data-theme-polarity="light"]` as a translucent tint
   (`rgba(0,0,0,0.05)` over the surface, same family as `--tab-strip-bg`). `headerStyle`
   and the tab-pill neutral colour both read it, deleting `NON_AGENT_DEFAULT_HEADER_BG`
   and the duplicated `isLightTheme` branches. Text on it is the theme's main text
   colour via the token, not a computed guess.
2. **Text tokens (R3).** Darken `--grey-text-color` and `--secondary-text-color` in the
   four light themes until they reach 4.5:1 on their surface; keep hue, lower lightness.
   Starting points (to be tuned by the script in §4, not guessed): Solarized secondary
   `#586e75`→ keep but not below 4.5:1 (it is already the "base01" value, so use
   `#4a5f66`), grey `#657b83` → `#52666d`; Latte grey `#7c7f93` → `#5c5f77`;
   Gruvbox grey `#7c6f64` → `#5f544b`; Light grey `#6e7781` → `#5b646d`.
3. **Status bar (R4).** Replace the opacity fades in `StatusBar.scss` with
   `--muted-text-color` / `--disabled-text-color`. The visual hierarchy (active, quiet,
   idle, disabled) is kept; each step is a colour, not a transparency.

### 3.2 Make it hold for every light theme

- Add `[data-theme-polarity="light"]` defaults in `theme.scss` for the chrome tokens
  (`--pane-header-bg`, `--muted-text-color`, `--disabled-text-color`, `--tab-strip-bg`,
  `--status-bar-bg`, hover/highlight tints). A light theme then inherits correct light
  chrome without listing them, and overrides only to match its palette.
- Move the existing per-theme duplicates of those tokens out of the four theme files
  where the polarity default now covers them.

## 4. Enforcement (the part that makes it stick)

Three layers, cheapest first.

### 4.1 A contrast test over every theme (the core)

`frontend/app/themes/theme-contrast.test.ts` reads `theme.scss` and every
`themes/*.scss`, resolves each theme's tokens (including the polarity defaults and
`var()` aliases), composites translucent colours over the surface, and asserts the R3
budget for a fixed list of **(foreground token, surface token) pairs**:

| Foreground | Surface | Minimum |
|---|---|---|
| `--main-text-color` | `--block-bg-solid-color`, `--main-bg-color`, `--modal-bg-color` | 4.5 |
| `--secondary-text-color` | same three | 4.5 |
| `--grey-text-color` | `--block-bg-solid-color` | 4.5 |
| `--muted-text-color` | `--status-bar-bg` over `--main-bg-color`, `--tab-strip-bg` over it | 4.5 |
| `--accent-color` | `--block-bg-solid-color` | 3 |
| `--border-color`, `--form-element-border-color` | `--block-bg-solid-color` | 1.5 (hairline) |
| `--error-color`, `--warning-color`, `--success-color` | `--block-bg-solid-color` | 4.5 |
| header text (derived) | `--pane-header-bg` over `--main-bg-color` | 4.5 |

It runs for **all** themes (dark ones too, which pass today at these thresholds or get
an explicit, documented exception list). A new theme is covered the moment its file
exists; a theme that fails prints the pair, the ratio and the minimum. This is the same
mechanism design systems use to keep their themes honest (Primer ships contrast checks per
colour mode; Radix and Leonardo generate scales to a target ratio).

### 4.2 No solid chrome on light themes

In the same test: for every theme in `LIGHT_THEME_IDS`, `--pane-header-bg`,
`--tab-strip-bg` and `--status-bar-bg`, composited over the surface, have relative
luminance ≥ 0.6 (R1). And R7: the set of themes whose solid surface is light equals
`LIGHT_THEME_IDS`.

### 4.3 A literal and opacity lint

Extend `eslint.theme-colors.config.js` (and a small script for SCSS):
- no `#…`, `rgb(…)`, `hsl(…)`, `black`, `white` as a **background** or **text colour**
  in `frontend/app/block`, `element`, `statusbar`, `window` and the shared chrome
  (allow-list for terminal/code surfaces and the theme files themselves);
- no `opacity: <1` on a rule that also sets text content in `statusbar/` and
  `element/PaneTabStrip.scss` (R4).

Today there are ~47 literal backgrounds and ~80 literal light text colours in `*.scss`
and the TSX style objects; the lint starts as a **ratchet** (a committed baseline that can
only shrink), the same approach the RPC contract test uses, so it does not block on the
existing backlog.

### 4.4 Human check

`task dev` on each light theme, one screenshot per theme of: a docked pane header, the
status bar, a modal, the agent picker. The screenshots go in the PR for any theme change.
(Contrast ratios catch legibility; they don't catch "unnatural".)

## 5. Phasing

| Phase | Contents | Size |
|---|---|---|
| **P1** | §3.1: `--pane-header-bg` token, delete `NON_AGENT_DEFAULT_HEADER_BG`; status-bar opacity → tokens; the four light themes' text tokens darkened. Tests for the header token and the status-bar rule | small–medium |
| **P2** | §4.1 and §4.2: the contrast and no-solid-chrome test over all themes, with the dark-theme exception list if any | medium |
| **P3** | §3.2 polarity defaults and removal of per-theme duplicates | small |
| **P4** | §4.3 lint with a ratchet baseline | medium |

P1 changes what the user sees and is the one to do first; P2 stops it recurring and
should land with it or straight after.

## 6. Open questions

1. **Palette fidelity vs the budget.** Solarized's own muted text (`#93a1a1`) is *by
   design* low contrast. The budget overrides that for text that carries content; the
   theme keeps the original colour for decorative uses (borders, comments in code). Is
   that acceptable, or should Solarized Light be allowed a documented exception?
2. **Target level.** 4.5:1 (AA) is proposed. The more modern APCA (Lc 60 for body text) is
   closer to perceived legibility and may be added as a second, advisory check; it isn't
   a standard yet, so the proposal doesn't gate on it.
3. **Translucency over a transparent window.** `--main-bg-color` is alpha-aware and the
   window can be see-through (`--window-opacity`). The test composites over white and over
   black and requires the budget on the **worse** of the two, which is stricter than a
   real desktop, but safe.
4. **User pane colours.** A user can pick a vivid pane colour. R5 mixes it toward the
   surface for the header. Should the border keep the full vivid value? (It already does.)
