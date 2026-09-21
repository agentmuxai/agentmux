# SPEC: harden the Tailwind color system against silent per-theme breakage

**Date:** 2026-09-21
**Status:** implemented — see §6 for exactly what shipped
**Related:** `docs/specs/SPEC_AGENT_HEADER_COLOR_UNIFICATION_2026_09_20.md` (the
pane-color consolidation work that surfaced this — its own precedent fix used
the exact bug pattern documented here, on the ICON only, and left the LABEL
text next to it still broken), `docs/specs/SPEC_LIGHT_THEME_AND_DEPTH_FIXES
_2026_07_11.md` and `SPEC_LIGHT_THEME_DEPTH_AND_MORE_THEMES_2026_07_13.md`
(prior light-theme contrast fixes — this is the same bug class recurring).

---

## 0. Motivation

A live bug report: on the "Light" theme, the top widget bar's text (icon +
label) reads as washed-out/too-light, and hovering makes the label
disappear entirely (the icon correctly darkens on hover; the text does not).

Chasing that one bug down to its root cause found a real, previously
undocumented architectural gap — this app has TWO parallel, disconnected
color mechanisms, and nothing stops a component from using the wrong one:

1. **This app's own theme system** — CSS custom properties defined once per
   theme in `frontend/app/themes/*.scss` (`--main-text-color`,
   `--secondary-text-color`, `--hover-bg-color`, etc.), swapped live via
   `[data-theme="..."]` on `<html>` (`frontend/app/app.tsx`). Correctly
   theme-aware by construction.
2. **Tailwind's own literal palette** (`text-white`, `bg-black`,
   `bg-gray-800`, and their `slate`/`neutral`/`zinc`/`stone` siblings) — these
   compile to a FIXED color, full stop. They have no relationship to
   `[data-theme]` at all, on any theme, ever.

Mechanism 2 was already known to be a trap: `action-widgets.scss`'s own
comment (predating this spec) documents a fix for the WIDGET ICON's hover
color losing to a Tailwind `hover:text-white` utility from a shared Tooltip
wrapper, on exactly this bug. That fix beat the Tailwind class with a more
specific CSS selector — for the icon only. The widget's LABEL text, right
next to it, had no such override and was never touched. This spec is the
fix for the label, plus everywhere else the same pattern was found.

## 1. Root cause #1: two Tailwind tokens were never wired to the theme system

`frontend/tailwindsetup.css`'s `@theme` block defines ~15 custom Tailwind
color tokens (`--color-hover`, `--color-panel`, `--color-foreground`, etc.)
as fixed literals. Every theme file re-declares the ones it needs inside its
own `[data-theme="..."]` block as a `var(--...)` reference to that theme's
real variable — e.g. `--color-hover: var(--hover-bg-color);` — which is what
makes `bg-hover`/`text-foreground`/etc. actually theme-aware: CSS custom
properties resolve against whatever's currently in scope, so the SAME
Tailwind utility class picks up a different real value depending on which
`[data-theme]` is active.

**`--color-hoverbg` and `--color-highlightbg` were never included in that
per-theme sync**, in any of the 13 theme files (checked directly, not
assumed). They permanently held `tailwindsetup.css`'s hardcoded
`rgba(255, 255, 255, 0.2)` — a value that happens to equal `theme.scss`'s
own (default theme) `--hover-bg-color`/`--highlight-bg-color`, which is
presumably why nobody noticed: the DEFAULT theme was never actually broken
by this. Every other theme, including every light-family one, was — a
`hover:bg-hoverbg` renders as a near-invisible white tint over an
already-light background, every time, on every non-default theme, and
always did.

Usages found (`bg-hoverbg`/`bg-highlightbg`, either polarity):
`frontend/app/window/action-widgets.tsx`, `frontend/app/modals/about.tsx`,
`frontend/app/element/quicktips.tsx` (`bg-highlightbg` once, plus a
`from-highlightbg/…` gradient stop).

## 2. Root cause #2: raw Tailwind palette literals used for theme-dependent purposes

Separately, several components reach for Tailwind's OWN built-in palette
(`white`/`black`/`gray-N`) directly, for things that should track the theme:
ambient/hover text color, hover/active surface tints, and (one case) a
tooltip's own background paired with theme-aware text. A full repo grep
(`\b(hover:|focus:|active:|group-hover:)?(text|bg|border|ring|divide|
placeholder|from|via|to|fill|stroke)-(white|black|gray-\d+|slate-\d+|
neutral-\d+|zinc-\d+|stone-\d+)(/[\d.\[\]]+)?\b` across every `.tsx` under
`frontend/app`) found 43 raw occurrences across 6 files. Classified by hand:

| File | Verdict | Why |
|---|---|---|
| `frontend/app/window/action-widgets.tsx` (1 occurrence, `hover:text-white`) | **Bug — fixed** | The originally-reported symptom: widget label text goes invisible on hover on a light theme. |
| `frontend/app/view/launcher/launcher.tsx` (5 occurrences) | **Bug — fixed** | Identical `hover:text-white` pattern on the launcher's back-breadcrumb, plus a widget-tile grid using raw `bg-white/…`/`text-white` for its hover/selected states. |
| `frontend/app/element/tooltip.tsx` (1 occurrence, `bg-gray-800`) | **Bug — fixed** | Tooltip background hardcoded dark; its own text (`text-foreground`) correctly flips to DARK on a light theme — dark-on-dark-gray on any light theme. |
| `frontend/app/element/quicktips.tsx` (34 occurrences) | **Bug — fixed** | The onboarding "Quick Tips" panel: every row/card hover and every card's resting background used raw `white`/`black` tints at various opacities, plus one `border-gray-700`. Never exercised against a light theme. |
| `frontend/app/element/markdown.tsx` (1 occurrence, `bg-white/[0.03]`) | **Bug — fixed** | A markdown table's `<thead>` tint; low severity (3% white is nearly a no-op either way) but still wrong-polarity, fixed for consistency. |
| `frontend/app/element/dragoverlay.tsx` (3 occurrences) | **Not a bug — annotated** | A drag-and-drop overlay that paints its OWN fixed black scrim + chip over whatever pane is underneath. Genuinely theme-independent by design — this is the escape hatch the new lint rule (§4) is FOR, not a violation of it. Given `eslint-disable-next-line` comments with a one-line reason each. |

SCSS `:hover` rules using a literal `white`/`#fff` text color were audited
too (8 files) — every one found is white text on that SAME rule's own fixed,
saturated background (a solid error-red button, an amber warning badge, a
Windows caption-button's red hover fill) — genuinely theme-independent,
left as-is.

## 3. Fixes applied

- **§1's tokens**: added `--color-hoverbg: var(--hover-bg-color);` and
  `--color-highlightbg: var(--highlight-bg-color);` to all 12 non-default
  theme files' existing "Tailwind @theme token sync" block, right next to
  the already-correct `--color-hover` line. `about.tsx`'s existing
  `hover:bg-hoverbg` usages needed no code change — they're automatically
  correct now that the token itself is fixed.
- **§2's genuine bugs**: each `hover:text-white` replaced with
  `hover:text-foreground` (the already-theme-synced token, which resolves
  to the exact color the widget icon's own hover state already used —
  `--widget-icon-hover-color: var(--main-text-color)` in
  `frontend/app/theme.scss`, so text and icon now agree). `quicktips.tsx`'s
  raw white/black tints mapped onto the existing theme-synced tokens by
  visual role: `hover:bg-white/5` → `hover:bg-hover` (row hover, subtle),
  `bg-black/20`/`hover:bg-black/30` → `bg-panel`/`hover:bg-highlightbg`
  (card surfaces, stronger), `border-white/10`/`border-gray-700` →
  `border-border`. `tooltip.tsx`'s `bg-gray-800` → `bg-modalbg` (paired
  correctly with its existing `text-foreground`).
- **Not fixed here, tracked as a follow-up**: `theme.scss` (the DEFAULT
  theme) has no "Tailwind @theme token sync" block AT ALL — none of its own
  variables are wired to the `--color-*` tokens via `var(...)`; the default
  theme's Tailwind utilities work today only because `tailwindsetup.css`'s
  hardcoded literals happen to have been copied from `theme.scss`'s values
  at some point in the past. Not currently causing a visible bug (values
  still match), but it's the same "silently drifts if either side changes
  without the other" risk this whole spec exists to close, and deserves the
  same full sync treatment the other 12 themes already have.

## 4. New lint rule — `eslint.theme-colors.config.js`

A standalone ESLint flat-config file, run via `npm run lint:theme-colors`
and wired into CI (`.github/workflows/ci-pr.yml`'s `vitest` job, right after
`tsc --noEmit`). Deliberately NOT added to the repo's main
`eslint.config.js`: that file's `recommended` ruleset currently has 1200+
pre-existing violations (mostly `@typescript-eslint/no-explicit-any` and
`no-unused-vars`) across the frontend — a real but entirely separate cleanup
effort. Folding this rule into that config would make a CI gate meant to
catch one specific, well-understood bug class fail immediately for 1200
unrelated reasons, which defeats the point of having a gate at all.

The rule (`no-restricted-syntax`, matched against both plain string
`Literal`s and template-literal `TemplateElement`s) flags the exact pattern
from §2: a raw Tailwind palette utility (`white`/`black`/`gray-N`/etc.,
optionally behind `hover:`/`focus:`/`active:`/`group-hover:`, optionally
with an opacity modifier) anywhere in a `.tsx` file under `frontend/app`.
It is not a blanket ban on those words — `dragoverlay.tsx`'s three
genuinely-safe usages (§2's table) demonstrate the intended escape hatch:
an inline `// eslint-disable-next-line no-restricted-syntax -- <reason>`
comment, placed on its own line immediately before the line carrying the
class string (a comment placed before a multi-line JSX opening tag but NOT
immediately before the specific attribute line does NOT suppress the
violation — confirmed the hard way while writing these three).

Three files (`AgentLaunchModal.tsx`, `AgentQuestionPanel.tsx`,
`editor-view.tsx`) are excluded from this config entirely: they carry
pre-existing `eslint-disable-next-line jsx-a11y/...`/`solid/no-innerhtml`
comments referencing plugins that were apparently planned but never
actually added as a dependency anywhere in this repo (not even in the main
`eslint.config.js`) — an unrelated, separate gap. Referencing an unknown
rule in a disable comment is itself an ESLint error independent of any
rule this file defines, so these three would fail under ANY config that
doesn't load those plugins.

**Also fixed in passing**: `eslint.config.js` itself was silently
non-functional — `tseslint.config(...)` already returns a flat-config
array, and the old `export default [baseConfig, eslintConfigPrettier]`
nested that array inside another array, which ESLint 8's flat-config loader
rejects outright (`TypeError: Unexpected array`). `npx eslint` had never
actually run successfully against this repo before this PR. Fixed by
spreading (`export default [...baseConfig, eslintConfigPrettier]`) — this
is what made it possible to discover the 1200-violation count in §4 above
at all.

## 5. Theme-aware header darkening (separate but related change, same PR)

Landed alongside this hardening work, in the same PR: the pane-color
consolidation from `SPEC_AGENT_HEADER_COLOR_UNIFICATION_2026_09_20.md` (an
agent pane's header darkens relative to its border) was made
theme-polarity-aware — dark themes keep the darkened look, light themes
keep the pre-2026-09-20 "bright" behavior (header matches the border's
full-strength color) — since a near-black, low-lightness header reads as
broken/muddy against a light UI. See
`frontend/app/block/pane-color-menu.ts`'s `headerBgForEffectiveColor` and
its own tests for the mechanism; not re-documented in full here since it's
one function in a sibling PR's own commit, not part of this spec's own
audit.

## 6. What shipped

All of §3 (both root-cause fixes across all identified files) and §4 (the
new lint rule, CI wiring, and the `eslint.config.js` fix) are implemented on
this branch. §3's `theme.scss` follow-up is explicitly NOT included — see
that bullet for why it's lower priority and safe to defer.

Verification: `npx vitest run` (4389 tests, 0 failures — this is a
CSS-class-only change, no new test coverage was written for individual
class-name swaps since there's no existing visual-regression test
infrastructure for this; verified by re-running the lint rule itself
against the whole `frontend/app` tree, `npx tsc --noEmit`, and live in
`task dev`), `npm run lint:theme-colors` (clean after the fixes, confirmed
it fails loudly against a deliberately-reintroduced violation first).
