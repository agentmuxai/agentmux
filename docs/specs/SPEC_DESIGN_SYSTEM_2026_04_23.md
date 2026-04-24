# Spec: Cohesive Design System

**Date:** 2026-04-23
**Status:** Draft
**Owner:** AgentA
**Related:**
- [SPEC_ROBUST_MODAL_SYSTEM_2026_04_23.md](./SPEC_ROBUST_MODAL_SYSTEM_2026_04_23.md) — consumer; z-index + token names land here first
- `frontend/app/theme.scss` / `frontend/app/reset.scss` / `frontend/app/app.scss` — current token + reset layer
- `frontend/app/mixins.scss` — current (minimal) mixin library

---

## 1. Motivation

The frontend has grown organically over two years. Styling has the good bones of a design system — a reset layer, ~125 CSS custom properties in `theme.scss`, kebab-case BEM-influenced selectors, co-located component SCSS — but every gap accumulates compound interest:

- **~400 raw colour values** (292 hex + 107 rgb/rgba) are baked into component SCSS, bypassing tokens entirely.
- **No spacing scale** — `4px` appears 86 times, `6px` 68 times, `8px` 62 times, ad-hoc.
- **No typography scale** — sizes, weights, and line-heights are scattered literals.
- **Single hard-coded dark theme** — no architecture for light mode or user themes.
- **Monolithic mega-files** — `agent-view.scss` is 4,046 lines, `swarm-view.scss` is 753. Refactors are painful.
- **22 `!important` overrides** — specificity wars masquerading as polish.
- **Tailwind is configured but underutilised** — mixed styling approach with no governance.
- **Z-index tokens exist but aren't universally used** — raw `z-index: 2/5/6` scattered across components.

The modal-system spec (landed before this one or in parallel) proposes re-ranking z-index. That rewire should live here, in the design-system foundation, so the modal spec just consumes the new tokens.

This spec defines a cohesive, documented design system that the whole app can migrate to incrementally, without blocking shipping.

## 2. Current state

### 2.1 What's good

- **Central token file** (`theme.scss`, 164 lines, ~125 tokens) organised by domain: colour, typography (shorthand), z-index, terminal palette.
- **Proper reset layer** (`reset.scss`, 197 lines, `@layer base`) — Tailwind-reset style, box-sizing universal, isolated.
- **Co-located component SCSS** — `Component.tsx` imports `./Component.scss` (Vite pattern).
- **BEM-ish naming** — kebab-case selectors, `--modifier` suffix, minimal collision risk.
- **Container queries used in `agent-view`** — modern responsiveness over `@media`.
- **Z-index tokens exist** for the slots that are tokenised (20 named slots).

### 2.2 What's missing

| Area | State | Gap |
|---|---|---|
| Colour | 125 tokens + ~400 raw literals | No enforcement; token coverage ~60%. |
| Spacing | 1 token (`--gap-size-px: 5px`) | No scale. 4/6/8/12px appear hundreds of times. |
| Typography | 3 shorthand tokens (`--base-font`, `--fixed-font`, `--header-font`) | No size/weight/line-height scale; sizes baked into shorthand. |
| Radii / shadows | Scattered raw values | No tokens for `border-radius` or `box-shadow`. |
| Theming | Single dark theme in `:root` | No light mode, no theme switch architecture. |
| Z-index | 20 tokens but raw `z-index: 1..6` inline | Partial adoption; see §5.3. |
| Responsive | 1 `@media` rule total | No breakpoint tokens, no responsive mixin. |
| Utilities / mixins | 2 mixins (`ellipsis`, `avatar-dims`) | No flex/grid helpers, no media mixins, no colour helpers. |
| `!important` | 22 occurrences | Specificity debt. |
| Mega-files | agent-view.scss @ 4,046 lines | Multiple unrelated concerns in one file. |
| Tailwind | Installed, underused | Mixed approach, no governance on when to use which. |

### 2.3 Numbers at a glance

- **56 SCSS files**, **10,246 total lines**.
- **125 CSS custom properties** defined in `theme.scss`.
- **~400 raw colour literals** across component SCSS.
- **93 inline `style={{…}}` props** in TSX (low but unsystematic).
- **0 breakpoints** (only `prefers-color-scheme` media query).

---

## 3. Goals

- **G1.** Every colour, spacing, typography, radius, shadow, and z-index value on the page resolves to a named token. Zero raw literals in new component SCSS; existing ones migrated over time.
- **G2.** A **spacing scale** (`--space-0` through `--space-12`) usable for padding, margin, gap. Based on 4-px rhythm.
- **G3.** A **typography scale** — sizes, weights, line-heights as independent tokens, plus composition tokens for common text roles (`--text-body`, `--text-caption`, `--text-code`, `--text-h1..h3`).
- **G4.** **Theme architecture** — tokens resolve via `[data-theme="dark"]` / `[data-theme="light"]` selectors; dark is default, light ships as a follow-up milestone.
- **G5.** **Z-index hierarchy** documented, consumed universally (no raw `z-index: N` in component SCSS).
- **G6.** **Mixin library** for the patterns that keep re-appearing — flex-center, abs-fill, truncate-lines, focus-ring, media-up/down, colour-alpha.
- **G7.** **Mega-files split** — `agent-view.scss` (4,046 lines) broken into per-subcomponent files; same for swarm and identity views.
- **G8.** **Tailwind governance**: decide — primary, opt-in, or third-party-only — and document.

## 4. Non-goals

- Replacing SolidJS with CSS-in-JS (emotion, vanilla-extract, etc.). The co-located `.scss` pattern stays.
- Rebranding / visual redesign. Colours and rhythms stay where they are; we just name them.
- Multi-theme switching UI. The architecture ships now; the UI ships later (or never, if nobody asks).
- Global class utility library ("Tailwind replacement"). Mixins + tokens are enough.

---

## 5. Design

### 5.1 Token taxonomy

Three layers, each building on the previous:

```
primitive     →   semantic     →   component
(blue-500)        (accent)         (button-primary-bg)
```

- **Primitive tokens** are raw values: `--color-blue-500: #2563eb`, `--space-2: 8px`, `--radius-md: 6px`. Named once, never referenced directly from components.
- **Semantic tokens** map primitives to meaning: `--accent-color: var(--color-blue-500)`, `--border-color: var(--color-gray-700)`, `--space-panel-padding: var(--space-4)`. Components consume these.
- **Component tokens** (optional) scope semantic tokens to a widget: `--button-primary-bg: var(--accent-color)`. Use sparingly — only when a component needs to theme independently.

Today's `theme.scss` mixes primitive and semantic. The rewrite separates them:

```
theme.scss
  ├── _primitives.scss    // raw values only
  ├── _semantic.scss      // maps primitives → meaning
  └── _themes/
        ├── dark.scss     // default
        └── light.scss    // follow-up
```

### 5.2 Spacing scale

4-px rhythm, named numerically:

| Token | Value | Typical use |
|---|---|---|
| `--space-0` | 0 | zero-out |
| `--space-0-5` | 2px | tight borders / hairline gaps |
| `--space-1` | 4px | icon-to-text, badge inner padding |
| `--space-1-5` | 6px | button gap |
| `--space-2` | 8px | standard gap |
| `--space-3` | 12px | card inner padding |
| `--space-4` | 16px | section gap |
| `--space-5` | 20px | pane padding |
| `--space-6` | 24px | modal padding |
| `--space-8` | 32px | heading margin |
| `--space-12` | 48px | big vertical rhythm |

Half-steps only where the current codebase actually uses them (2 and 6 are both common — dropping them would force hundreds of migrations to the nearest whole step).

### 5.3 Z-index hierarchy

One scale, one source of truth. Migrates all `z-index: N` literals to variables.

```
--zindex-app-background       -1
--zindex-content-base           0
--zindex-layout-behind          1
--zindex-layout-above           2
--zindex-node-strip             3   /* agent-view document strip */
--zindex-pane-overlay           4
--zindex-xterm-viewport-overlay 5
--zindex-drag-overlay        1000
--zindex-flash-error         7000
--zindex-typeahead           8000
--zindex-popover             8500
--zindex-modal               9000
--zindex-flyout-menu         9500
--zindex-context-menu       10000
```

(Matches the modal-system spec's reranking — this spec is the canonical source.)

### 5.4 Typography scale

**Size** (clamps prevent extreme zoom blow-outs when combined with per-pane zoom):

```
--text-xs:   11px
--text-sm:   12px
--text-base: 13px
--text-md:   14px       /* the body default */
--text-lg:   16px
--text-xl:   18px
--text-2xl:  22px
```

**Weight**: `--font-weight-normal (400)`, `--font-weight-medium (500)`, `--font-weight-bold (700)`. The app rarely needs other weights.

**Line-height**: `--leading-tight (1.2)`, `--leading-normal (1.45)`, `--leading-loose (1.65)`.

**Family**: `--font-sans ("Inter", system-ui)`, `--font-mono ("Hack", "JetBrains Mono", monospace)`, `--font-markdown` (unchanged).

**Composite roles** for common patterns:

```scss
--text-body:    var(--text-md) / var(--leading-normal) var(--font-sans);
--text-caption: var(--text-xs) / var(--leading-tight)  var(--font-sans);
--text-code:    var(--text-sm) / var(--leading-normal) var(--font-mono);
```

### 5.5 Radii, shadows, motion

```
--radius-sm:   3px    /* chips, badges */
--radius-md:   6px    /* buttons, inputs */
--radius-lg:  10px    /* cards, modals */
--radius-xl:  16px    /* large surfaces */
--radius-full: 9999px /* pills, avatars */

--shadow-sm: 0 1px 2px rgba(0,0,0,0.15);
--shadow-md: 0 4px 8px rgba(0,0,0,0.25);
--shadow-lg: 0 12px 24px rgba(0,0,0,0.35);
--shadow-modal: 0 24px 48px rgba(0,0,0,0.45);

--motion-fast:   100ms cubic-bezier(0.2, 1, 0.3, 1);
--motion-base:   160ms cubic-bezier(0.2, 1, 0.3, 1);
--motion-slow:   280ms cubic-bezier(0.2, 1, 0.3, 1);
--motion-spring: 400ms cubic-bezier(0.2, 0.9, 0.2, 1.2);
```

### 5.6 Theme architecture

Dark ships as default:

```scss
:root,
[data-theme="dark"] {
    --main-bg-color:    rgb(34, 34, 34);
    --main-text-color:  rgb(229, 231, 235);
    --accent-color:     var(--color-blue-400);
    /* …rest of semantic tokens… */
}

[data-theme="light"] {
    --main-bg-color:    rgb(255, 255, 255);
    --main-text-color:  rgb(23, 23, 23);
    --accent-color:     var(--color-blue-600);
    /* …rest… */
}
```

Switcher is `document.documentElement.dataset.theme = "light" | "dark"`. No SCSS changes in components — they already consume semantic tokens, so swapping the root attribute re-renders everything.

Light mode ships as a milestone *after* the token migration — most colours need the light-mode counterpart defined before we can flip the attribute at runtime. Non-blocking on the primary design-system work.

### 5.7 Mixin library (expanded)

```scss
// frontend/app/mixins.scss
@mixin ellipsis { … }                          // existing, kept
@mixin avatar-dims { … }                       // existing, kept

@mixin flex-center    { display: flex; align-items: center; justify-content: center; }
@mixin flex-col       { display: flex; flex-direction: column; }
@mixin abs-fill       { position: absolute; inset: 0; }
@mixin truncate-lines($n) { display: -webkit-box; -webkit-line-clamp: $n;
                             -webkit-box-orient: vertical; overflow: hidden; }
@mixin focus-ring     { outline: 2px solid var(--accent-color); outline-offset: 2px; }
@mixin media-up($w)   { @media (min-width: #{$w}) { @content; } }
@mixin media-down($w) { @media (max-width: #{$w - 1px}) { @content; } }
@function alpha($token, $a) { @return color-mix(in srgb, var(#{$token}) #{$a * 100%}, transparent); }
```

Governance: **no new component SCSS file is allowed to introduce a raw `z-index`, colour, spacing, or line-height literal**. Lint rule enforces (see §6).

### 5.8 Mega-file decomposition

`agent-view.scss` (4,046 lines) splits into:

```
frontend/app/view/agent/styles/
    index.scss                 (imports the rest)
    _picker.scss
    _card.scss
    _launch-modal.scss
    _document.scss
    _document-node.scss
    _tool-overlay.scss
    _composer.scss
    _status-line.scss
    _activity-log.scss
    _pending-messages.scss
    _search-bar.scss
    _subagent.scss
```

Same pattern for `swarm-view.scss` and `identity-view.scss`. Each sub-file stays under ~300 lines, maps 1:1 with the TSX component it styles, and co-locates with the component folder.

### 5.9 Tailwind governance

Decision: **Tailwind stays opt-in for third-party integrations** (currently `streamdown`). It does **not** become primary. Custom SCSS + tokens is the default. Rationale:

- We already have a token system; Tailwind utilities would duplicate it.
- Mixing approaches in one component file is the current pain.
- Cost of removing Tailwind entirely > benefit. Keeping for the specific libs that expect it.

Document this in `frontend/CLAUDE.md` so future contributors have a clear rule.

---

## 6. Enforcement

Adding a design system without enforcement decays the moment you ship. Two automated guards:

### 6.1 Stylelint

Add `stylelint` + `stylelint-config-standard-scss` + `@double-great/stylelint-a11y` to the frontend toolchain. Rules:

- `color-no-hex` — no hex literals. Override via `/* stylelint-disable-next-line */` only in `_primitives.scss` and for ANSI palette constants (where named tokens don't make sense).
- `declaration-property-value-allowed-list` — `z-index` must match `var\(--zindex-.*\)`.
- Custom rule: `padding`, `margin`, `gap` values must be either `0` or `var(--space-*)`.
- `declaration-no-important` with a per-file allow-list (audit the 22 existing !important, remove or document each).

Runs in pre-commit (new `lint-staged` config) and in CI.

### 6.2 SCSS lint-on-PR

CI job: `npm run lint:scss` fails the PR if any new violation is introduced. Existing violations are grandfathered via `.stylelintignore` and burned down over time; every migration PR removes entries from the ignore list.

---

## 7. Migration plan

### Phase 1 — Foundation (non-breaking, additive)

- **PR 1.** Split `theme.scss` into `_primitives.scss` + `_semantic.scss`. Names stay; add primitive colour tokens referenced by semantic names. No component changes.
- **PR 2.** Add spacing, typography, radius, shadow, motion token families. Not consumed yet; just defined.
- **PR 3.** Add expanded `mixins.scss`. Also unused at first.
- **PR 4.** Publish stylelint config + pre-commit hook. All existing violations grandfathered via `.stylelintignore`; new code must conform.

### Phase 2 — Z-index + modal consumption (unlocks modal spec)

- **PR 5.** Migrate all hard-coded `z-index: N` to `var(--zindex-*)`. No visual change. Re-ranks per §5.3.
- **PR 6.** The modal-system spec PR 1 (new `Modal` primitive) consumes the new tokens — not a design-system PR, but sequenced here so the two specs compose.

### Phase 3 — Colour migration (per-pane, longest work)

- **PRs 7–N.** One PR per `view/` pane migrates its raw hex/rgb values to semantic tokens. Pure refactor — visuals identical. `.stylelintignore` shrinks with each PR. Ordered by pane size: `agent-view` last because it's the biggest.
- Tooling: a one-off script `scripts/find-raw-colors.js` lists every raw colour with file:line so the migration is greppable.

### Phase 4 — Spacing & typography migration

- **PRs.** Same pattern as colour — per-pane migration of hard-coded spacing and font sizes to tokens. Visual diff should be zero per PR; a byte-level SCSS diff is the review signal.

### Phase 5 — Mega-file split

- **PR.** Split `agent-view.scss` into the `_picker.scss`, `_card.scss`, … list from §5.8. One atomic PR; `git mv` + `@use`-import each file. No selectors renamed, no rules dropped — just file-level reorganisation so git blame is clean.
- Swarm and identity split in follow-up PRs on the same template.

### Phase 6 — Light theme

- **PR.** Define every semantic token's light-mode counterpart. Add a hidden theme switcher (`localStorage` + `data-theme` attribute) for internal testing. Ship the UI toggle when it's been soak-tested.

### Phase 7 — Cleanup

- **PR.** Remove the last entries from `.stylelintignore`. Remove the 22 `!important` overrides (audit each — most will turn out to be specificity bugs that disappear when tokens are consumed correctly). Delete dead CSS.

---

## 8. Sequencing with the modal spec

The user asked whether the modal spec or this one comes first. Answer:

**This spec's Phase 1 + 2 land first**, then the modal spec's PRs execute against the new token names.

- Phase 1 (token foundation) is **additive and non-breaking** — three small PRs that define new names without changing any existing code. They can ship in a day.
- Phase 2 (z-index re-rank) is the rewire the modal spec depends on. Landing it before the modal's new primitive keeps the modal PR focused on primitive behaviour, not token wrangling.
- After Phase 2, modal PRs 1–7 execute independently.
- Colour / spacing migration (Phases 3–4) runs in parallel with everything else; the modal work doesn't block it and vice versa.

## 9. Open questions

1. **Light mode urgency.** Do we have a user asking for it, or is it vapor? If vapor, defer Phase 6 indefinitely — the architecture is free to build now, but don't burn cycles defining ~100 light-mode colour values without a customer.
2. **Tailwind removal.** §5.9 keeps Tailwind for third-party libraries. If streamdown et al. can be styled via their own CSS API, removing Tailwind entirely saves ~30KB + tooling complexity. Worth a ~1-day spike.
3. **Component tokens.** §5.1 allows optional component tokens (`--button-primary-bg`). Do we define them preemptively for each existing component, or only when a real need arises? Leaning **only when needed** — premature component tokens are themselves a form of debt.
4. **Stylelint scope creep.** Starting with the rules in §6 is a reasonable MVP. Adding more (selector max-depth, property order, etc.) is easy later. Resist the temptation to ship a huge `.stylelintrc` at once.
5. **Fractional spacing tokens (`--space-1-5`).** Current proposal keeps them. Alternative: round everything to whole steps in the migration. Reviewing the real usage distribution (with the grep script) should inform this before Phase 4.

## 10. Rollout & metrics

- Success signal #1: stylelint CI job stays green with `.stylelintignore` empty after Phase 7.
- Success signal #2: average SCSS file size drops (agent-view.scss from 4,046 → target: 12 files × ~250 lines each).
- Success signal #3: zero raw `z-index` / hex / rgb literals in new component SCSS files over a six-month window.
- Success signal #4: theme-switch tested (dark → light → dark) produces no visual regressions in a canary build.
- Follow-up: periodically audit `!important` count — it should stay at zero after Phase 7 and grow only with justified comments.

## 11. Cross-references

- `SPEC_ROBUST_MODAL_SYSTEM_2026_04_23.md` — direct consumer of the z-index re-rank in §5.3.
- `frontend/app/theme.scss` — current token file being refactored.
- `frontend/app/reset.scss` — kept as-is; already well-layered.
- `frontend/app/view/agent/agent-view.scss` — primary target for Phase 5 mega-file split.
