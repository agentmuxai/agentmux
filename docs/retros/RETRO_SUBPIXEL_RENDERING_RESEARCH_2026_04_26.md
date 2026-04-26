# CSS Sub-Pixel Rendering — 2024-2026 Best-Practice Playbook

**Date:** 2026-04-26
**Status:** Research findings (no implementation yet)
**Trigger:** Tab/separator distance inconsistency (see
            `RETRO_TAB_SEPARATOR_DISTANCE_2026_04_26.md`). User
            wanted research into industry-wide solutions and a
            scaffolding-level fix that stabilises rendering
            across the whole app, not just tabs.
**Audience:** Senior engineer making a one-time design-system
              investment

---

## Executive Summary

1. **The CSS spec finally caught up.** As of Baseline 2024 (May),
   `round()` from CSS Values & Units L4 ships in all major
   browsers, and `zoom` is a standardized property (Firefox 126+).
   For the first time, a design system can authoritatively say
   *"this dimension snaps to a device pixel"* in pure CSS.
2. **`zoom` and `transform: scale()` are not interchangeable, and
   the modern guidance is *both* — for different jobs.** `zoom`
   participates in layout (good for chrome that should reflow);
   `transform: scale()` is purely visual (good for animations and
   pixel-stable small UI). Neither is a complete answer to
   sub-pixel jitter on its own.
3. **Border snapping is now spec'd behavior.** `border`, `outline`,
   and `column-rule` are auto-snapped to device pixels by every
   modern engine. `box-shadow`, `width`, `padding`, `margin`, and
   any flex slot are *not* — that gap is the entire root-cause
   class for "uneven tab spacing"-style bugs.
4. **The "modern correct" app-chrome scaling pattern is
   `rem`-based scaling at the root, not `zoom` at a chrome
   wrapper.** If you can't migrate, the next-best thing is
   `transform: scale()` with width compensation, GPU-promote the
   chrome layer, and snap critical dimensions with
   `round(down, …, 1px)`.
5. **Three design-system primitives carry 80% of the value** for
   a SCSS codebase: a `--snap` length token, a `pixel-snap()`
   SCSS function that emits `round(down, …, var(--snap))`, and a
   "pair" rule for separators (always-even slot containing
   always-odd or always-even line — never mix parity).

---

## Section 1 — Root-Cause Taxonomy in 2026

The "uneven tab spacing" class of bug stems from five distinct
mechanisms in the rendering pipeline. The first three are
inherent to the engine; the last two are something a design
system can prevent.

### 1.1 LayoutUnit fractional layout (Blink/WebKit) and twips (Gecko)

Blink and WebKit do layout in `LayoutUnit`, which has 1/64 px
resolution — nearly continuous. Gecko does layout in 1/60 px
("twips"). Both engines then snap to device pixels at paint time.
([WebKit LayoutUnit wiki][webkit-layoutunit])

The implication: **two siblings whose layout positions differ by
less than 1 device pixel will round to either the same or
adjacent device pixels depending on their absolute position.** A
row of N auto-width tabs whose right edges fall at fractional
positions {73.4, 154.6, 235.1, 316.7…} will paint with 1px gap
variance. The engine isn't "wrong" — there's no way to render a
0.4px shift correctly without anti-aliasing the whole tab edge.

### 1.2 Pixel snapping is per-rectangle, not per-row

> "Pixel snapping applies rounding to the logical top left point
> of the rectangle, then moves the resulting edges to the nearest
> pixel boundaries." — [Chen Hui Jing][chenhuijing], summarizing
> Blink behavior.

Each box is snapped *independently* on its own top-left. There is
no "shared" snap grid for a flex row's children, which is exactly
why adjacent siblings can land at different sub-pixel decisions
even at integer DPR.

### 1.3 `zoom` worsens snapping by multiplying every length by a
non-integer

When `zoom: 1.25` is applied at a wrapper, every descendant
length is multiplied. A 7px separator becomes 8.75 device pixels.
A 1px line becomes 1.25. The snapping algorithm runs *after* the
multiplication, so the rounding bucket for each element is
essentially a function of its absolute position in the multiplied
coordinate space — which varies tab to tab. Border snapping
([Brosset 2024][brosset]) snaps `border` and `outline` to integer
device pixels but does *not* snap `box-shadow`, `width`, or flex
slot widths.

### 1.4 Border snapping is asymmetric — `border` snaps,
`box-shadow` does not

This is the new (2024) Values & Units L4 behavior. Brosset's
example: a 5px shadow + 5px border + 5px outline at DPR 1.5 paints
as **7.5px shadow, 7px border, 7px outline.** The shadow is the
odd one out. ([Brosset 2024][brosset])

For a tab-bar separator that's currently a 1px-wide *child
element* (i.e., not a border/outline of the slot, but a `::before`
block), the engine treats it like any other length — no special
snapping. **Re-implementing the separator as `outline` or
`border-left` on the slot would automatically opt into border
snapping.**

### 1.5 Parity mismatch (the design-system-preventable one)

A 7px slot containing a 1px line: `(7 - 1) / 2 = 3` on each side.
Even at DPR 1, this is fine. At DPR 1.5 with `zoom: 1.25`, the
slot's actual paint width is `7 * 1.5 * 1.25 = 13.125` device
pixels, the line is `1 * 1.5 * 1.25 = 1.875`, and the centered
position depends on where in the device-pixel grid the slot's
top-left landed.

**Rule of thumb (from the same family of pitfalls as Apple's
`centerScanRect:`):** *Never center an odd-physical-pixel element
inside an even-physical-pixel container, or vice versa.* The math
doesn't divide. Pick same-parity slot/line pairs (e.g., 8px slot
/ 2px line, or 7px slot / 1px line *only at integer DPR with no
zoom*).

---

## Section 2 — Solutions Ranked by Leverage

### Tier S: Architectural changes (highest leverage, most disruptive)

#### S1. Replace `zoom` with `rem`-based scaling at `:root`

> "Using rem units makes them ideal for font sizing… when users
> change their preferred font size in browser settings, all
> rem-based sizes scale accordingly." ([Chrome 146 release notes][chrome146])

Set `:root { font-size: calc(16px * var(--zoomfactor)); }` and
express *every* chrome dimension in `rem`. This is what VS Code,
Slack, Linear-on-web, and most modern Electron apps do. Rendering
stays in Blink's normal LayoutUnit pipeline with no extra
multiplier; sub-pixel snapping uses the engine's normal
heuristics; `getBoundingClientRect()` is honest; accessibility
scaling Just Works.

**Pro:** Eliminates §1.3 entirely.
**Con:** Touches every CSS file with hardcoded `px`. Migration
cost is real but mechanical.

#### S2. Replace `zoom` with `transform: scale()` + width compensation

The classic "fake zoom" pattern: scale a fixed-size container,
then `width: calc(100% / var(--zoomfactor))` to fill the viewport.

Per Jake Archibald ([2025][archibald]) and OpenReplay
([2024][openreplay]):
- `transform: scale()` runs in the GPU compositor — sub-pixel
  positioning is preserved through the scale.
- It does **not** participate in layout, so children layout is
  computed at unscaled dimensions. The engine snaps *once*, at the
  unzoomed sizes.
- Animatable, unlike `zoom`.

**Pro:** Eliminates §1.3 without touching every `px`.
**Con:** Layout-vs-visual divergence. `getBoundingClientRect()`
returns post-transform dimensions in some browsers, pre-transform
in others. Hit-testing usually correct in modern Chromium but
text-selection bounds can be a hair off. *Don't* use this for
terminal panes — terminals are sub-pixel-sensitive and `transform:
scale` makes glyph hinting drift.

#### S3. Change *which* dimension carries the design intent

The current bug is "tabs are auto-width, so their right edges fall
at fractional positions." If tabs were `flex: 1 1 0` (equal-width)
or `flex-basis: round(down, calc(100% / var(--N)), 1px)`, every
tab would land on a snapped boundary by construction.

This trades visual semantics ("tab as wide as its name") for
layout determinism ("tab fills its share of the bar"). Most modern
apps with consistent tab strips (Chrome, Edge, Arc, modern
Firefox) use the second model — auto-width tabs are reserved for
short, bounded sets.

### Tier A: App-wide scaffolding (high leverage, low disruption)

#### A1. The `--snap` token + `pixel-snap()` SCSS function

Add to your token layer:

```scss
:root {
  // 1px in CSS pixels = exactly 1 device pixel only when DPR=1.
  // At DPR=2, 0.5px = 1 device pixel. Compute once.
  --dpr: 1;             // updated by JS on load + matchMedia change
  --snap: calc(1px / var(--dpr));
}

@function pixel-snap($value) {
  @return round(down, #{$value}, var(--snap));
}

@mixin pixel-snap-width($value) {
  inline-size: pixel-snap($value);
}
```

Update `--dpr` from JS on load and on
`window.matchMedia('(resolution: 1dppx)').onchange`. This gives
every component a one-line opt-in to device-pixel snapping.
Browser support: `round()` is Baseline 2024 ([MDN][mdn-round]).

**Caveat from MDN:** *"For known values, use custom properties
instead. Using `round()` is redundant if these have known values."*
— so don't `round()` literal `7px`; reserve it for `calc()`-derived
widths.

#### A2. The "Separator" primitive — never roll your own

Create a single SCSS partial / token-driven mixin that bakes in
the parity rule:

```scss
@mixin separator($slot-width, $line-width, $color) {
  // Enforce same-parity at lint time (see A4 below).
  flex: 0 0 #{$slot-width};
  // Use border-right on the slot itself — opts into spec-defined
  // border-snapping (Patrick Brosset 2024). A child <div> does not.
  border-right: $line-width solid $color;
  // Visually center the snapped line inside the slot:
  margin-right: calc(($slot-width - $line-width) / -2);
  margin-left:  calc(($slot-width - $line-width) /  2);
}
```

The key move is **using `border` instead of a `::before` child** —
borders snap, child boxes don't. ([Brosset 2024][brosset])

#### A3. GPU-promote the chrome wrapper (modest)

```scss
.window-header, .tab-bar-scroll {
  will-change: transform;
  transform: translateZ(0);
  backface-visibility: hidden;
}
```

This forces the chrome onto its own compositor layer. Per
[Smashing Magazine 2016][smashing] and the Mozilla bug history,
GPU layers preserve sub-pixel positioning during transforms — so
if you go with S2 (transform: scale), you essentially *need* this
for the scaled chrome to not look fuzzy. **However**,
GPU-promotion can hurt text crispness on macOS (it disables
sub-pixel font AA, which is already disabled OS-wide since Mojave
per [dbushell 2024][dbushell], so the cost is small in 2026 but
non-zero on Windows). **Apply only to chrome, never to body text
or terminal panes.**

#### A4. Stylelint custom rule: "no fractional px in chrome layer"

There's no off-the-shelf "snap to pixel" stylelint rule, but
custom plugins are well-supported ([Stylelint custom rules][stylelint-custom]).
Two rules to enforce:

1. **Disallow odd-pixel widths inside even-pixel parents** in
   files matching `chrome/**/*.scss`. Heuristic, but catches the
   §1.5 parity bug.
2. **Disallow `width:` literal values; require `inline-size:
   var(--*)` or `pixel-snap(...)`** in chrome.

Apple's AppKit has a similar rule in code review form:
`backingAlignedRect:options:` is the canonical "snap to backing
pixels" call. ([Apple HighRes APIs][apple-hires])

#### A5. Font smoothing token

Per dbushell's 2024 reassessment, applying `-webkit-font-smoothing:
antialiased` is now the recommended baseline (since Mojave 2018
disabled OS-wide subpixel AA anyway):

```scss
:root {
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
}
```

This is one less variable in the chrome's perceived crispness.
([dbushell 2024][dbushell])

### Tier B: Per-component patches (low leverage)

#### B1. `outline: 1px solid` instead of a child `::before` line

Opts into spec-defined border-snapping. Already covered in A2.

#### B2. Switch to `flex: 1` for equal-width tabs

Already covered in S3.

#### B3. Bump separator from 7px/1px to 8px/2px

Covered in the prior retro as "Path E". Mostly camouflage — the
underlying jitter is still there, you just made a target larger
than the variance.

---

## Section 3 — How Major Design Systems & Apps Handle This

### Adobe Spectrum — explicitly acknowledges the precision tax

> *"In the large scale, the `--swc-scale-factor` custom property
> has a fractional value, which means depending on the value you
> are calculating from you may achieve a non-whole number, and the
> conversion can in some cases have unexpected effects on pixel
> precision delivery of text content."* —
> [Spectrum Web Components Core Tokens][spectrum-tokens]

Spectrum exposes both `--spectrum-global-dimension-size-N`
(scaled) and `--spectrum-global-dimension-static-size-N`
(unscaled, integer px) — you pick "static" for anything where
pixel precision matters more than scaling. This is a clean
**design-token bifurcation pattern**: every dimension token has
a "scaled" and "static" variant, and component authors choose.
Worth stealing.

### Microsoft Fluent UI 2 — device-independent pixels in tokens

The Fluent token pipeline stores every dimension as
device-independent pixels in JSON, then per-platform tooling
decides how to emit them. ([Fluent UI Token Pipeline JSON
ref][fluent-tokens]) On the web, that means `px` literals in CSS,
and Fluent does *not* attempt browser-side pixel snapping at the
token layer — it relies on browser default behavior plus Windows
High DPI being handled by Chromium's scaling.

### Apple SwiftUI / AppKit — `backingAlignedRect:options:` is the canonical primitive

> *"For consistent pixel alignment on high resolution displays,
> the NSView, NSWindow, and NSScreen classes provide a
> `backingAlignedRect:options:` method that accepts rectangles in
> local coordinates and ensures the result is aligned on backing
> store pixel boundaries."* — [Apple HighRes APIs][apple-hires]

The `NSAlignmentOptions` flag set lets you choose "push outward to
next backing pixel" vs "round inward," per edge. **CSS `round()`
with `up` / `down` / `nearest` strategies is the direct conceptual
port** — Apple solved this in 2009 with backing-aligned rect, and
CSS got the same primitive in 2024.

### Figma — vector internally, snap-to-pixel as a per-frame setting

Figma's canvas is purely vector, and "Snap to pixel grid" is a
per-frame toggle. Designers complain that toggling it modifies
fractional coordinates ([Figma forum][figma]) — i.e., even Figma
can't fix sub-pixel without rounding. The ecosystem advice is the
**8pt or 4pt base grid** ("every dimension is a multiple of N") —
that reduces the surface area where fractions can appear in the
first place.

### Chromium / VS Code / Electron-class apps

VS Code, Slack, Discord, and Linear-desktop don't use CSS `zoom`.
They use:
1. Electron's `webFrame.setZoomFactor()` (or
   `BrowserWindow.webContents.setZoomFactor()`), which adjusts
   Chromium's *device scale factor* internally — this is the same
   code path as system DPI scaling, so layout happens at the
   natural integer device-pixel grid for that scale.
   ([Electron BrowserWindow docs][electron-bw])
2. `rem`-based root sizing for any in-page UI that should respect
   a separate scale.

CEF has the same `--force-device-scale-factor` flag
([Chromium fractional scaling discussion][chromium-frac]). **For
an app that uses CEF, the cleanest "scale the chrome" pattern is
to drive Chromium's device scale factor, not CSS `zoom` on a
wrapper.** This is the genuinely-best-in-class answer for our
stack.

### Modern terminal emulators (Warp, WezTerm, Ghostty)

All three sidestep the CSS pipeline entirely — they're
GPU-textured terminals (Metal/D3D11/OpenGL) with custom glyph
atlases. Font rendering happens at the device-pixel grid by
construction, and "zoom" is implemented by re-rasterizing the
atlas at a new font size, not by scaling the rendered output.
**Lesson:** for the *terminal* part of AgentMux, never scale via
CSS — change the font size and let the terminal re-layout.
(You're already doing this, per the per-pane zoom retro / PR #86.)

### Tailwind CSS — no `round()` utilities yet

Tailwind v4 (2025) introduced CSS-variable-backed tokens and
`@theme` but does **not** yet ship `round()`-based utilities.
([Tailwind v4 guide][tailwind4]) If you're a Tailwind shop, you'd
write your own utility plugin. Worth flagging because the absence
shows that "pixel snapping at the utility layer" is not yet an
industry-standard idiom — your team would be ahead of the curve.

### Radix / Mantine / Chakra

None of them publish sub-pixel guidance. Their tabs/separator
components produce styled boxes and let you provide widths — the
rendering precision is the consumer's problem. This is the
dominant pattern in the React ecosystem and a non-finding worth
noting: *the design-system layer is the right layer to fix this,
and nobody has yet*.

---

## Section 4 — Recommended Scaffolding Changes for AgentMux

The following set is designed to be done once and stop the bug
class app-wide. It assumes you keep `zoom` for now (S1/S2 are
bigger projects to discuss separately).

### 4.1 New token layer — `frontend/app/tokens/_pixel.scss`

```scss
// =============================================================
// Pixel snapping primitives
// Single source of truth for "this dimension lives on a device pixel"
// =============================================================

:root {
  // Updated by JS at startup + on resolution change.
  // Default to 1; the JS bootstrap overwrites before paint.
  --dpr: 1;

  // 1 device pixel expressed in CSS pixels.
  --snap: calc(1px / var(--dpr));

  // 1 device pixel inside the zoomed chrome.
  // (Only meaningful when --zoomfactor != 1.)
  --snap-chrome: calc(1px / (var(--dpr) * var(--zoomfactor)));
}

// SCSS function — emit at *authoring* time when the value is known.
// Use this when you're writing literal dimension expressions.
@function snap($value, $strategy: down) {
  @return round(#{$strategy}, #{$value}, var(--snap));
}

@function snap-chrome($value, $strategy: down) {
  @return round(#{$strategy}, #{$value}, var(--snap-chrome));
}

// Mixin — for the common case of "make this width snap to a device pixel."
@mixin snap-size($w: null, $h: null) {
  @if $w { inline-size: snap($w); }
  @if $h { block-size: snap($h); }
}
```

**JS bootstrap** (single line in your app-init):

```ts
const setDpr = () => document.documentElement.style.setProperty(
  "--dpr", String(window.devicePixelRatio)
);
setDpr();
matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`)
  .addEventListener("change", setDpr);
```

### 4.2 New primitive — `frontend/app/element/_separator.scss`

```scss
// =============================================================
// Separator primitive
// Always use this. Never roll a 1px <div> in 7px parent again.
// =============================================================

@mixin v-separator($slot, $line, $color) {
  // Lint-time check: warn if slot and line are different parity.
  // (Implement as a stylelint plugin — see Section 4.5.)
  flex: 0 0 #{$slot};
  display: block;
  // border-right participates in spec-defined border-snapping
  // (Patrick Brosset 2024). A child <div> does not.
  border-right: #{$line} solid $color;

  // Center the line inside the slot using negative margin —
  // pushes the snapped border to the slot's geometric center.
  $offset: ($slot - $line) * 0.5;
  margin-right: -#{$offset};
  margin-left:   #{$offset};
}
```

Then in `tabbar.scss`:

```scss
.tab-separator {
  @include v-separator(8px, 2px, var(--tab-separator-color));
  // 8/2 instead of 7/1 — same parity (both even), still subtle.
}
```

### 4.3 Chrome layer GPU promotion — one place only

```scss
// chrome/_root.scss — applied once, at the chrome wrapper.
.window-header {
  // Promote the entire title bar / tab strip into its own compositor layer.
  // Sub-pixel positions are preserved through the layer transform.
  will-change: transform;
  transform: translateZ(0);
  // Prevents 1px sub-pixel font wobble on Windows when zoom changes.
  backface-visibility: hidden;
}

// CRITICAL: do NOT promote terminal panes or body text.
.term-pane, .editor-pane { will-change: auto; transform: none; }
```

### 4.4 Font smoothing baseline

```scss
// tokens/_typography.scss
:root {
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
  text-rendering: geometricPrecision;
}
```

`text-rendering: geometricPrecision` is a hint for "favor
consistent glyph spacing over speed" — in modern Blink it tends to
suppress some hinting that contributes to "text bunches differently
per tab" effects. Test before adopting widely; the spec leaves the
implementation pretty free.

### 4.5 Stylelint custom plugin — `stylelint-no-parity-mismatch`

```js
// .stylelintrc/plugins/no-parity-mismatch.js
// Flags @include v-separator($slot, $line) where parities differ.
const stylelint = require("stylelint");
const ruleName = "agentmux/no-parity-mismatch";
module.exports = stylelint.createPlugin(ruleName, () => (root, result) => {
  root.walkAtRules("include", (rule) => {
    const m = /v-separator\(\s*(\d+)px\s*,\s*(\d+)px/.exec(rule.params);
    if (!m) return;
    const [slot, line] = [+m[1], +m[2]];
    if ((slot % 2) !== (line % 2)) {
      stylelint.utils.report({
        message: `v-separator: slot (${slot}px) and line (${line}px) parity mismatch — will not center on a device pixel.`,
        node: rule, result, ruleName,
      });
    }
  });
});
```

There is no community plugin for this — you build it. ~30 lines.
([Stylelint custom plugin guide][stylelint-custom])

### 4.6 The "static dimension" token convention (steal from Spectrum)

For every dimension token in `theme.scss`, expose two variants:

```scss
:root {
  --space-7-scaled: calc(7px * var(--zoomfactor));   // for general layout
  --space-7-static: 7px;                              // for pixel-critical usage
  --space-8-scaled: calc(8px * var(--zoomfactor));
  --space-8-static: 8px;
}
```

Tab separators, dividers, hairlines, focus rings, drag handles →
use `-static`. Padding, margins, gaps → use `-scaled`. This is
exactly Spectrum's `static-size-*` pattern.
([Spectrum Core Tokens][spectrum-tokens])

---

## Section 5 — Migration Plan

A staged rollout that lets you ship value without a "big bang."

### Phase 0 — Measure (½ day)

1. Add a `--debug-pixel-grid` mode that overlays a 1-device-pixel
   grid via a fixed-position SVG. Toggle from the dev console.
2. Take screenshots of the tab bar at zoom levels 1.0, 1.1, 1.25,
   1.33, 1.5 on a DPR=1 monitor and a DPR=2 monitor. Document the
   variance — this is your before-baseline.

### Phase 1 — Land the scaffolding (1 day)

1. Add `tokens/_pixel.scss`, the JS DPR bootstrap, and
   `element/_separator.scss`.
2. Add the `text-rendering` / font-smoothing token block.
3. **Do not migrate any consumer yet.** Ship as additive;
   everything still works.
4. Run a smoke test: `task dev`, open AgentMux, confirm `--dpr` is
   set in DevTools and `round()` resolves on hover-inspect.

### Phase 2 — Migrate the tab bar only (½ day)

1. Replace `.tab-separator` `::before` with `@include
   v-separator(8px, 2px, …)`.
2. Add `will-change: transform; transform: translateZ(0)` to
   `.window-header`.
3. Add `flex-basis: round(down, calc(100% / var(--tab-count)),
   var(--snap))` *if* you're willing to switch to equal-width
   tabs. Otherwise leave auto-width and rely on the separator fix
   alone.
4. Compare against Phase 0 baselines. Decision gate: if variance
   is ≤ 1 device pixel at all tested zooms, ship; otherwise back
   out and revisit S1/S2.

### Phase 3 — Spread to other chrome surfaces (1 day, opportunistic)

Status bar dividers, sidebar resize handles, settings-row borders
— anywhere that uses a hardcoded 1px child line gets migrated to
`@include v-separator(...)`.

### Phase 4 — Linting (½ day)

Add the `agentmux/no-parity-mismatch` stylelint plugin. Wire into
your existing post-edit hook (you already have TS type checking
via hooks per `~/.claude/CLAUDE.md`).

### Phase 5 — Architectural decision: keep `zoom` or migrate? (separate retro)

Phase 2 fixes the immediate bug. Once it's stable for a release
cycle, schedule a retro on:
- **S1** (rem-based scaling): the canonical answer; high
  migration cost.
- **S2** (transform: scale + width compensation): low migration
  cost; behavioral risk on terminal panes.
- **CEF device-scale-factor approach**: drive
  `--force-device-scale-factor` from your zoom slider instead of
  CSS `zoom`. Closest to what VS Code does. Requires CEF host
  changes, not CSS.

**My ranked recommendation for the retro:** CEF
device-scale-factor first, S1 second, S2 third, status-quo +
scaffolding last. The scaffolding alone gets you 80% of the way;
the architectural choice gets the last 20% and unlocks
chrome-wide benefits beyond tab spacing.

### Rollback plan

Every phase is additive (new tokens, new mixins) except Phase 2's
separator markup change. Keep the old `::before` separator under a
`:where(.legacy-separator)` selector for one release; flip a CSS
variable to switch between them in dev for A/B comparison.

### Success metrics

- Tab-to-separator distance variance ≤ 1 device pixel at zoom
  levels {1.0, 1.1, 1.25, 1.33, 1.5} on DPR ∈ {1, 1.5, 2}.
- No new "blurry text" reports from users on Windows.
- Stylelint catches at least one parity mismatch in CI within the
  first month (dogfooding evidence the rule fires).

---

## Section 6 — Sources

### Primary technical references

- [John Resig — Sub-Pixel Problems in CSS (2008, the canonical post)][resig]
- [Patrick Brosset — Invasion of the Border Snappers (2024) — definitive modern explainer for border/outline/column-rule snapping][brosset]
- [Chen Hui Jing — Sub-pixel rendering and borders][chenhuijing]
- [Crisal — Border rounding in CSS (Gecko vs WebKit/Blink algorithm comparison)][crisal]
- [Robert O'Callahan — Subpixel Layout And Rendering (Mozilla)][rocallahan]
- [WebKit Wiki — LayoutUnit (1/64 px resolution)][webkit-layoutunit]
- [Chromium — Blink Coordinate Spaces (CSS px vs DIP vs physical px)][chromium-coords]

### Spec & MDN references

- [MDN — CSS `round()` function (Baseline 2024)][mdn-round]
- [MDN — CSS `zoom` property (Baseline 2024, Firefox 126+)][mdn-zoom]
- [MDN — `transform-function: scale()`][mdn-scale]
- [W3C — CSS Snapshot 2026][w3c-snapshot]
- [CSSWG Issue #10729 — Proposal to retrieve sub-pixel border values][csswg-10729]
- [Mozilla Bug 287624 — round CSS border widths to nearest pixel][bz-287624]
- [Mozilla Bug 1181554 — box-shadow rendered unevenly at uneven scaling factors][bz-1181554]

### `zoom` vs `transform: scale` — modern guidance

- [Jake Archibald — Animating zooming: transform order matters (2025)][archibald]
- [OpenReplay — Using CSS `zoom` to Scale UI Elements (2024)][openreplay]
- [Re-rastering composited layers on scale change (Chrome blog)][chrome-reraster]

### High-DPI / fractional DPR

- [web.dev — Pixel-perfect rendering with `devicePixelContentBox`][webdev-dpcb]
- [DisplayPixels — Understanding Device Pixel Ratio for web devs][displaypixels]
- [Tom Roth — Understanding the Device Pixel Ratio][tomroth]
- [Igalia — HiDPI support in Chromium for Wayland][igalia]
- [Dieulot — CSS retina hairline, the easy way (the `transform: scaleY(0.5)` 0.5px trick)][dieulot]

### Font smoothing

- [David Bushell — What's the deal with WebKit Font Smoothing? (2024 reassessment)][dbushell]
- [Stanislas — How to fix font rendering on macOS Mojave][stanislas]

### Design systems

- [Adobe Spectrum — Platform scale page][spectrum-platform]
- [Adobe Spectrum Web Components — Core Tokens (`--swc-scale-factor` precision caveat)][spectrum-tokens]
- [Microsoft Fluent UI Token Pipeline — JSON format reference (DIP-based tokens)][fluent-tokens]
- [Fluent 2 Design System — Design Tokens][fluent2]
- [Tailwind CSS v4 — Ultimate 2025 Styling Guide][tailwind4]
- [Radix Primitives — Separator][radix-sep]

### Apple — the canonical pixel-snap APIs

- [Apple — APIs for Supporting High Resolution (`backingAlignedRect:options:`, `centerScanRect:`, `NSAlignmentOptions`)][apple-hires]
- [Apple Developer — `NSView.alignmentRect(forFrame:)`][apple-alignrect]

### GPU compositing

- [Smashing Magazine — CSS GPU Animation: Doing It Right][smashing]
- [Paul Irish — Why moving elements with translate() is better than pos:abs top/left][paulirish]
- [Mozilla Bug 739176 — No sub-pixel rendering while transitioning translate transforms][bz-739176]

### Containment & content-visibility (largely orthogonal)

- [web.dev — content-visibility CSS property][webdev-cv]
- [CSS Wizardry — What Is CSS Containment (2026 update)][csswizardry]

### Linting

- [Stylelint — Customize / custom plugins][stylelint-custom]
- [stylelint-no-px (a precedent for unit-policing rules)][stylelint-no-px]

### Electron / Chromium chrome scaling pattern

- [Electron BrowserWindow API (`webContents.setZoomFactor`)][electron-bw]
- [Chrome 146 release notes — root font-size scales with OS text scale][chrome146]
- [OddBird — Designing for User Font-size and Zoom (2025)][oddbird]
- [Chromium fractional scaling discussion][chromium-frac]

### Other useful background

- [Simon Battersby — Browsers and fractional pixels][simonb]
- [Acko.net — CSS Sub-pixel Background Misalignments][acko]
- [Hacker News thread — fractional 1px border at 96 dpi][hn-1px]

### Prior internal retros (referenced)

- `docs/retros/RETRO_TAB_SEPARATOR_DISTANCE_2026_04_26.md`
- `docs/retros/RETRO_TAB_GAPS_ARCHITECTURE_ANALYSIS_2026_04_25.md`
- `docs/specs/SPEC_TAB_BAR_FIRST_PRINCIPLES_2026_04_25.md`

[resig]: https://johnresig.com/blog/sub-pixel-problems-in-css/
[brosset]: https://patrickbrosset.com/articles/2024-06-21-invasion-of-the-border-snappers/
[chenhuijing]: https://chenhuijing.com/blog/about-subpixel-rendering-in-browsers/
[crisal]: https://crisal.io/words/2020/06/13/rounding-borders.html
[rocallahan]: https://robert.ocallahan.org/2008/01/subpixel-layout-and-rendering_22.html
[webkit-layoutunit]: https://trac.webkit.org/wiki/LayoutUnit
[chromium-coords]: https://www.chromium.org/developers/design-documents/blink-coordinate-spaces/
[mdn-round]: https://developer.mozilla.org/en-US/docs/Web/CSS/round
[mdn-zoom]: https://developer.mozilla.org/en-US/docs/Web/CSS/zoom
[mdn-scale]: https://developer.mozilla.org/en-US/docs/Web/CSS/transform-function/scale
[w3c-snapshot]: https://www.w3.org/TR/css-2026/
[csswg-10729]: https://github.com/w3c/csswg-drafts/issues/10729
[bz-287624]: https://bugzilla.mozilla.org/show_bug.cgi?id=287624
[bz-1181554]: https://bugzilla.mozilla.org/show_bug.cgi?id=1181554
[archibald]: https://jakearchibald.com/2025/animating-zooming/
[openreplay]: https://blog.openreplay.com/css-zoom-scale-ui-elements/
[chrome-reraster]: https://developer.chrome.com/blog/re-rastering-composite
[webdev-dpcb]: https://web.dev/device-pixel-content-box/
[displaypixels]: https://displaypixels.io/learn/device-pixel-ratio-explained.html
[tomroth]: https://tomroth.dev/dpr/
[igalia]: https://blogs.igalia.com/adunaev/2020/11/13/hidpi-support-in-chromium-for-wayland/
[dieulot]: http://dieulot.net/css-retina-hairline
[dbushell]: https://dbushell.com/2024/11/05/webkit-font-smoothing/
[stanislas]: https://stanislas.blog/2018/09/how-to-fix-font-rendering-macos-10-14-mojave/
[spectrum-platform]: https://spectrum.adobe.com/page/platform-scale/
[spectrum-tokens]: https://opensource.adobe.com/spectrum-web-components/tools/core-tokens/
[fluent-tokens]: https://microsoft.github.io/fluentui-token-pipeline/json.html
[fluent2]: https://fluent2.microsoft.design/design-tokens
[tailwind4]: https://walidezzat.hashnode.dev/tailwind-css-v4-complete-guide
[radix-sep]: https://www.radix-ui.com/primitives/docs/components/separator
[apple-hires]: https://developer.apple.com/library/archive/documentation/GraphicsAnimation/Conceptual/HighResolutionOSX/APIs/APIs.html
[apple-alignrect]: https://developer.apple.com/documentation/appkit/nsview/1526905-alignmentrect
[smashing]: https://www.smashingmagazine.com/2016/12/gpu-animation-doing-it-right/
[paulirish]: https://www.paulirish.com/2012/why-moving-elements-with-translate-is-better-than-posabs-topleft/
[bz-739176]: https://bugzilla.mozilla.org/show_bug.cgi?id=739176
[webdev-cv]: https://web.dev/articles/content-visibility
[csswizardry]: https://csswizardry.com/2026/04/what-is-css-containment-and-how-can-i-use-it/
[stylelint-custom]: https://stylelint.io/user-guide/customize/
[stylelint-no-px]: https://github.com/meowtec/stylelint-no-px
[electron-bw]: https://www.electronjs.org/docs/latest/api/browser-window
[chrome146]: https://developer.chrome.com/release-notes/146
[oddbird]: https://www.oddbird.net/2025/07/22/size-preferences/
[chromium-frac]: https://github.com/hyprwm/Hyprland/discussions/11627
[simonb]: https://www.simonbattersby.com/blog/browsers-and-fractional-pixels/
[acko]: https://acko.net/blog/css-sub-pixel-background-misalignments/
[hn-1px]: https://news.ycombinator.com/item?id=30797281
