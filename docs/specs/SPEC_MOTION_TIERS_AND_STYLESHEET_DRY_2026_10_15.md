# SPEC: two motion tiers, one motion policy, and a DRY pass over the stylesheets

**Date:** 2026-10-15
**Author:** AgentY@narko
**Status:** active. Approved 2026-10-15: the operator took every recommendation in §8 ("go with recommendations, get it all built"). Phases in §7; each PR updates this line. Phases A (#4968), B (#4971) and C (token bugs and mechanical swaps) are built.
**Verified against:** `agentmux` `main` @ `fb6b20ba8`, by reading code (three read-only inventories, spot-checked by hand) and this machine's Windows setting.
**Scope:** every animation, transition and motion gate in `frontend/` (CSS, SCSS, TS, the vendored Font Awesome CSS, SVG assets, `index.html`), the `window:reducedmotion` setting end to end, and the shared stylesheet layer (`theme.scss`, `mixins.scss`, `element/ui/`).
**Related:**
- [`SPEC_PANE_DRAG_TO_TAB_2026_07_10.md`](SPEC_PANE_DRAG_TO_TAB_2026_07_10.md) addendum A1 (lines 324-334): the 2026-07-11 decision to stop honouring the OS setting app-wide.
- [`SPEC_AGENT_PANE_BUSY_RING_IN_BORDER_2026_10_14.md`](SPEC_AGENT_PANE_BUSY_RING_IN_BORDER_2026_10_14.md) lines 119 and 169: "reduced motion is static", the change that made the busy ring stop.
- [`SPEC_AGENT_PANE_ONE_WAY_FLOW_2026_10_07.md`](SPEC_AGENT_PANE_ONE_WAY_FLOW_2026_10_07.md) lines 191 and 205, and [`SPEC_AGENT_PANE_NO_CATCH_UP_MOTION_2026_10_15.md`](SPEC_AGENT_PANE_NO_CATCH_UP_MOTION_2026_10_15.md): the transcript's own motion, under active work.
- [`SPEC_AGENT_ACTIVITY_TAB_FLASH_2026_09_23.md`](SPEC_AGENT_ACTIVITY_TAB_FLASH_2026_09_23.md), [`SPEC_AGENT_ROW_DELETE_ANIMATION_2026_09_16.md`](SPEC_AGENT_ROW_DELETE_ANIMATION_2026_09_16.md), [`SPEC_MODAL_TRANSITIONS_2026_05_18.md`](SPEC_MODAL_TRANSITIONS_2026_05_18.md), [`SPEC_DESIGN_SYSTEM_2026_04_23.md`](SPEC_DESIGN_SYSTEM_2026_04_23.md), [`SPEC_COLOR_THEME_TOKEN_HARDENING_2026_09_21.md`](SPEC_COLOR_THEME_TOKEN_HARDENING_2026_09_21.md).

## 0. The ask

From the operator, 2026-10-15, after the agent pane's busy ring stopped moving on their machine:

> "lets do a roundup of our 'no animation' flags. I keep the OS at animations off, but that doesnt mean I dont want any animations. Lets formalize 2 levels, one for users with OS animations on, and one for users with off. Just so we setup the infra properly now, but have them all enabled, regarding of windows animation effects. Later we may revise it and shut certain animations off if reduced animations are set in the OS level. lets write a solid spec that covers styling throughout. and take also a moment to scan for opprtunuties to DRY the stylesheets"

## 1. Summary

1. **Today there are four motion gates, and only the one nobody chose works.**
   - The app's own atom is hard-wired to `false`.
   - The SCSS mixin is a no-op.
   - The Settings toggle is saved but never read.
   - Meanwhile 14 raw `@media (prefers-reduced-motion: reduce)` blocks, one direct `matchMedia` check, and a rule in the vendored Font Awesome CSS still follow the OS. Every one of them was added after the 2026-07-11 decision that said the app would not.
   - On a machine with Windows "Animation effects" off (the operator's), the busy ring is frozen. So are the attention glow, the busy status dots, the install-step pulse, the drop indicator, row entrances, panel opens and the transcript's glides; the tab cross-fade is cut to 1 ms.
2. **This spec makes motion one formal system:**
   - There are two tiers, `full` (the OS allows animation) and `reduced` (the OS asks for less). The tier is detected in one place.
   - Every animation is put in one category: status, feedback, transition or decorative. Flashes are bound by a separate safety rule.
   - A single policy table says, per tier and category, whether that category is reduced.
   - Each animation can declare its own reduced treatment, and that treatment applies only when the policy says so.
   - **For now the policy reduces nothing in either tier**, so everything runs the same whatever the OS says. A later revision changes one table, not 30 files.
3. **The stylesheet layer has real bugs and much duplication.**
   - Two transitions never animate, because a duration-plus-easing token is used as a bare duration.
   - One shadow token is malformed.
   - 27 variables are used but never defined.
   - Of the 978 font sizes, 447 are raw px values that exactly equal a token.
   - Only 5 of 143 durations use a motion token.
   - The shared mixins already cover most of the repetition, but most have no users.
   - §6 ranks the clean-ups. The motion-related ones are part of this work; the rest is a follow-on plan.

## 2. What is there today

### 2.1 The gates

| Gate | Where | State |
|---|---|---|
| App atom `prefersReducedMotionAtom` | `frontend/app/store/global.ts:84` | `createMemo(() => false)`. It used to be `reducedMotionSetting() \|\| reducedMotionSystemPreference()`. About 15 consumers read it, so their reduced branches are unreachable. |
| OS signal `reducedMotionSystemPreference` | `global.ts:76`, kept current by a `matchMedia` listener at `:229-233` | Set, never read. |
| Setting `window:reducedmotion` | `crates/srv/src/backend/wconfig/types.rs:238`, `schema/settings.json:199`, `settings-template.jsonc:40`, `frontend/types/srv-types.d.ts:924`; read into `reducedMotionSetting` (`global.ts:75`) | Never read for effect. The Settings toggle (`view/settings/sections/appearance-section.tsx:62-68, 206-215`) is described as "Disable CSS animations and transitions", which is false. |
| Root class `.prefers-reduced-motion` | added from the atom at `app.tsx:462`; styled at `app.scss:216-227` | Never applied. The rule is also invalid CSS: `none` is not a valid duration, timing function or delay, and it never covered `animation-*`. |
| SCSS mixin `respect-reduced-motion` | `mixins.scss:467-476` | `@if false { @content }`, a no-op by design. Five call sites (`modal.scss:297`, `_composer-strip.scss:585`, `_document-nodes.scss:194, 220, 522`) declare reduced treatments that are never emitted. |
| **Raw OS media queries** | 14 blocks, §2.2 | **Live.** |
| **Direct `matchMedia`** | `view/agent/virtualization/AgentDocumentVirtualList.tsx:406-409` (an ancestor-class check, then `matchMedia`) | **Live.** It turns off the transcript's content glides (`:477`, `:493`) and shortens the scroll follower's time constant (`scroll/one-way-flow.ts:86,131`, `scroll/follower.ts:22,43`). |
| **Vendored Font Awesome** | `public/fontawesome/css/fontawesome.min.css` | **Live.** Under the OS setting its own media query runs `fa-spin`, `fa-pulse` and the like once, in 1 ms. |
| Native | `crates/cef`, `crates/launcher`, `crates/common` | No motion switch. Chromium reports the OS setting to CSS and `matchMedia` as-is. |

On Windows, Chromium maps the "Animation effects" setting (`SystemParametersInfo(SPI_GETCLIENTAREAANIMATION)`) to `prefers-reduced-motion: reduce`. It reads `false` on narko.

### 2.2 The live OS media queries, and what they do today when the OS asks for less

| File:line | What stops or changes | Added |
|---|---|---|
| `app.scss:329` | tab-switch view transition cut to 1 ms | #3239 |
| `drop-feedback.scss:38` | the drop-target invert strobe is dropped and the steady pulse stays (a flash-safety opt-out, see §4.2) | #3694 |
| `drop-feedback.scss:53` | the strobe-exempt descendant: `animation: none` | #3694 |
| `drag/drop-indicators.scss:73` | the file-drop indicator fade-in | #3960 |
| `element/install/install-steps.scss:88` | the active install step's pulse | #3661 |
| `element/ui/card.scss:240` | the busy status dot's pulse | #4851 |
| `element/ui/ui.scss:58-64` | the attention glow (`.ui-tone-attention`) | #4604 |
| `view/agent/agent-view.scss:391` | **the agent busy ring's march** | #4934 |
| `view/agent/styles/_control-bar.scss:233` | the compaction progress (no width transition; at "over" a static 100% at half opacity) | #4220 |
| `styles/_document-nodes.scss:2906` | the wheel-edge skid shadow | #4335 |
| `styles/_document.scss:303` | the row enter fade and rise | #4178 |
| `styles/_document.scss:329` | the one-way flow fade shortened from 180 to 90 ms ("quicker, not gone", owner) | #4438 |
| `styles/_recent-sessions.scss:224` | My Agents row delete and move cut to 1 ms | #3280 |
| `view/identity/styles/_detail.scss:128` | the identity detail panel open | #4317 |
| `view/swarm/swarm-stats.scss:16` | the swarm stats open | |

### 2.3 What moves

The inventory found 38 keyframes and about 134 transition declarations in 53 files, plus JS-driven motion:
- Web Animations: the activity flash, `notification/activity-flash.ts:267`, deliberately ungated; and the transcript glides.
- A rAF scroll follower.
- `startViewTransition` in `workspace/workspace.tsx:131`.
- Smooth scroll to a search hit, `AgentDocumentVirtualList.tsx:1167,1175`, ungated.
- The ASCII and brain spinners.
- The status line's type-out, `view/agent/status/present-status.ts:54`.
- TransitionGroup moves in My Agents.
- The FLIP height change in `view/agent/resize-contract.ts:164`.

§5 classifies them. Motion tokens exist (`--motion-fast` 100 ms, `--motion-base` 160 ms, `--motion-slow` 280 ms, `--motion-spring` 400 ms, `theme.scss:427-430`), but only five declarations use them. JS keeps its own constants (`GLIDE_MS` 180, `HOLD_RELEASE_MS` 160, `WIDTH_TRANSITION_MS` 160, `HEIGHT_FLIP_MS` 150, `SPINNER_FRAME_MS` 120, `FADE_MS` 200).

### 2.4 Why the decision did not hold

The 2026-07-11 decision removed every raw media block and neutralised the atom and the mixin. It left nothing to stop the next author from writing a raw `@media (prefers-reduced-motion: reduce)` again:
- no shared rule naming a category;
- no check in CI;
- no comment where an author would look.

Thirteen authors did exactly that, each with a reasonable local intent. Several comments still describe the atom as the "real gate" (`app.scss:318-321`, `AgentFooter.tsx:200-201`, `MyAgentsList.tsx:975-978`, `modal.tsx:35`). `docs/architecture/PANE_LAYOUT_AND_REFLOW_ARCHITECTURE.md:132-141` still says the class reset honours the OS setting. The infrastructure below is mainly about making the right thing the only easy thing.

## 3. Goals and non-goals

**Goals**
- One detection of the OS preference, giving one tier per window.
- One policy table that decides, per tier and category, what is reduced.
- Every animation declared once, with its category, and its reduced treatment written next to it but dormant until the policy turns it on.
- **Today: every animation runs in both tiers**, whatever Windows "Animation effects" says. That includes the busy ring, which is the visible fix.
- A CI check so no raw OS motion query or ad-hoc `matchMedia` comes back.
- Motion values (durations, easings, keyframes) come from shared tokens and one keyframes file. The two token bugs that stop transitions are fixed.
- A ranked, phased plan for the rest of the stylesheet duplication.

**Non-goals**
- Choosing which animations a future revision reduces. The table makes that a one-line change; this spec does not make it.
- Redesigning any animation. Values change only where a token merges two values a few ms apart, and each merge is listed.
- Changing the transcript's scroll behaviour beyond routing its gate. That work belongs to `SPEC_AGENT_PANE_NO_CATCH_UP_MOTION_2026_10_15.md`.

## 4. Design

### 4.1 Tiers

| Tier | When | Meaning |
|---|---|---|
| `full` | the OS allows animation (`prefers-reduced-motion: no-preference`) | the designed motion |
| `reduced` | the OS asks for less (`prefers-reduced-motion: reduce`; on Windows, "Animation effects" off; on macOS, Reduce Motion) | for now, the same designed motion |

**Detection.** A new module, `frontend/app/motion/motion.ts`, owns the only `matchMedia("(prefers-reduced-motion: reduce)")` in the app, with a change listener. It exports `motionTier(): "full" | "reduced"`. It replaces `reducedMotionSystemPreference`, and the listener at `global.ts:229-233` moves here.

**A future user override.** `window:reducedmotion` is not wired to anything in this spec (§8, Q2). The module reads the tier through one function, so an override ("follow the system / always full / always reduced") later changes one line.

**Every window.** Each AgentMux window and floating pane is its own document, and each runs the module at startup. So each sets its own tier, and a window opened after the OS setting changed agrees with the others.

### 4.2 Categories and the flash rule

Every animation gets exactly one category:

| Category | Meaning | Examples |
|---|---|---|
| `status` | says that something is in progress, or what state it is in | busy ring, spinners, busy dots, install step pulse, compaction progress, typing cursor, countdown pulse, mic ripple, attention glow |
| `feedback` | confirms or guides what the user is doing right now | drop strobe and pulse, drop indicator, insertion gap, landing bounce, activity flash, copy and search flashes, dismiss nudge |
| `transition` | moves between two states | tab cross-fade, modal and menu entrance, row enter and exit, panel open and collapse, glides, scroll follower, smooth scroll, spinner fade-outs, type-out |
| `decorative` | polish with no information | hover lifts, colour fades on hover and focus |

**Flashes are not a motion category.** A flash (a large change in luminance, such as the drop-target invert strobe) is bound by a separate rule in both tiers: **no more than three flashes in any one second** (WCAG 2.3.1). The rule is checked in review and noted at each flash's definition. The drop strobe makes 2 flashes in 800 ms, 2.5 per second, so it complies in both tiers. Its current OS opt-out (`drop-feedback.scss:38,53`) was added as a seizure-safety escape hatch, not as motion policy. §8 Q1 asks whether it goes.

### 4.3 The policy table

One object in `motion.ts` decides which categories are reduced in each tier:

```ts
// Which categories run their reduced treatment, per tier.
// 2026-10-15 (operator): everything runs in both tiers.
export const MOTION_POLICY: Record<MotionTier, readonly MotionCategory[]> = {
    full: [],
    reduced: [],
};
```

`motion.ts` publishes the result as an attribute on the document root, kept in sync with the tier:

```html
<html data-motion-tier="reduced" data-motion-reduce="">
```

`data-motion-reduce` lists the reduced categories, separated by spaces. It is empty today in both tiers. JS reads the same answer through `shouldReduceMotion(category): boolean`.

The attribute goes on `<html>`, not the app's root `div`, so that portaled menus, toasts, pseudo-elements and the view transition's `::view-transition-*` pseudo-elements all see it.

### 4.4 CSS: declare the reduced treatment, dormant

A new mixin in `mixins.scss` replaces both `respect-reduced-motion` and every raw `@media (prefers-reduced-motion: reduce)`:

```scss
@use "sass:list";

/// The reduced treatment for one animation, applied only when the motion
/// policy reduces `$category` in this window's tier
/// (SPEC_MOTION_TIERS_AND_STYLESHEET_DRY_2026_10_15.md §4.3). Never write a
/// raw prefers-reduced-motion media query: CI rejects it (§4.6).
@mixin when-motion-reduced($category) {
    @if not list.index((status, feedback, transition, decorative), $category) {
        @error "when-motion-reduced: unknown category #{$category}";
    }
    :root[data-motion-reduce~="#{$category}"] & {
        @content;
    }
}
```

Each site keeps the reduced treatment its author chose. That preserves the intent; it does not delete it. For example, the busy ring becomes:

```scss
&--active::before {
    animation: agent-ant-march 0.5s linear infinite;
    @include mixins.when-motion-reduced(status) {
        animation: none; // the same broken pattern, still (busy-ring spec §2.5)
    }
}
```

With an empty policy, nothing inside the mixin ever matches, so the ring marches in both tiers. If a later revision adds `status` to `reduced`, the ring freezes again for OS-off users, along with every other status animation, from one line.

**Attribute selector cost.** It is one attribute selector on `:root`, matched only for the declarations inside the mixin. It is no measurable cost next to the current media queries.

**Specificity.** The wrapper puts `:root[...]` (a pseudo-class and an attribute, 0,2,0) in front of the site's own selector, so a reduced treatment beats the site's normal rule without `!important`. That fixes the `ui.scss:58` case, where a bare `animation: none` loses on specificity today.

**Font Awesome.** The vendored CSS's own reduced-motion rule cannot be routed through the mixin. `app.scss` gets a matching override that restores Font Awesome's normal spin and pulse under `:root[data-motion-tier="reduced"]:not([data-motion-reduce~="status"])`: in the reduced tier, unless the policy reduces `status`. That gives the vendored icons the same policy as everything else.

### 4.5 JS: one question, by category

`prefersReducedMotionAtom` is replaced by `shouldReduceMotion(category)`, and each consumer passes its category:

| Consumer | Category |
|---|---|
| `element/AsciiSpinner.tsx:42-49` | status |
| `element/BrainSpinner.tsx:29-33` (its pulse also gets the CSS mixin, which it lacks today) | status |
| `element/PaneLoadingCover.tsx:57` | transition |
| `view/agent/components/AgentShellSubblock.tsx:401` | transition |
| `view/browser/browser-view.tsx:197` | transition |
| `workspace/workspace.tsx:37,130` (view transition) | transition |
| `element/PaneTabStrip.tsx:464, 487` | transition (width), feedback (landing cue) |
| `view/agent/components/AgentFooter.tsx:207,250,274` (type-out) | transition |
| `components/MyAgentsList.tsx:984-1013` | transition |
| `view/agent/resize-contract.ts:139` (FLIP) | transition |
| `layout/lib/TileLayout.core.tsx:556-557` (reveal gate) | transition |
| `AgentDocumentVirtualList.tsx:406-409` (glides and follower τ; drops its own `matchMedia` and class check) | transition |
| `notification/activity-flash.ts:267` (no gate today) | feedback, plus the flash rule |
| `AgentDocumentVirtualList.tsx:1167,1175` smooth scroll (no gate today) | transition |

**Deleted:**
- the dead dependency at `layout/lib/layoutModelHooks.ts:147`;
- the root class `.prefers-reduced-motion` (`app.tsx:462`) and its invalid reset (`app.scss:216-227`);
- `reducedMotionSetting` and `reducedMotionSystemPreference` (`global.ts:75-76`);
- `modal-dismiss-nudge-static`, whose static variant moves into the mixin.

**Tests.** Existing tests that mock the atom (`AsciiSpinner.test.tsx`, `ToolOverlayLog.test.tsx`, `resize-contract.test.ts`, `block.test.tsx`, `PaneLoadingCover.test.tsx`, `AgentShellSubblock.test.tsx`) mock `shouldReduceMotion` instead. Their reduced branches stay tested, ready for when the policy uses them.

### 4.6 Guarding it

Add a check, `scripts/check-motion-gates.mjs`, to the same CI job as `check-ui-primitives.mjs` and `check-no-transition-all.sh` (`ci-pr.yml`). It fails when `frontend/` (excluding `public/fontawesome/`) contains:
- `prefers-reduced-motion` outside `mixins.scss`, `motion.ts` and its test;
- `matchMedia(` with `reduced-motion` outside `motion.ts`;
- a `when-motion-reduced(` with an unknown category. The mixin's `@error` catches this at build time; the script reports the file.

The policy table also gets a unit test. It pins "nothing reduced in either tier", so a change to the policy is deliberate and shows in review.

### 4.7 Motion tokens

Split `theme.scss`'s motion tokens into durations and easings, and keep the pairs:

| Token | Value | Replaces (raw, from the inventory) |
|---|---|---|
| `--duration-instant` | 80 ms | 80 ms ×44 (line controls, `element/ui/_line.scss:103`, `ui.scss:157,331,372,449,480`) |
| `--duration-fast` | 100 ms | 100 ms / 0.1 s ×33, `--motion-fast` |
| `--duration-quick` | 120 ms | 120 ms / 0.12 s ×17, the view transition, the placeholder glide |
| `--duration-base` | 160 ms | 150 ms / 0.15 s ×38 and 160 ms (merge: 150 becomes 160, §5.3) |
| `--duration-moderate` | 240 ms | 240 ms ×3 (its own token, §8 Q5) |
| `--duration-slow` | 280 ms | `--motion-slow` |
| `--ease-standard` | `ease` | `ease` ×87 |
| `--ease-out` | `ease-out` | ×29 |
| `--ease-in-out` | `ease-in-out` | the status pulses (×12) |
| `--ease-emphasized` | `cubic-bezier(0.2, 1, 0.3, 1)` | the current `--motion-*` curve, the modal entrance |
| `--ease-material` | `cubic-bezier(0.4, 0, 0.2, 1)` | the collapse transition and the FLIP |
| `--ease-decelerate` | `cubic-bezier(0.2, 0, 0, 1)` | row enter |
| `--ease-bounce` | `cubic-bezier(0.34, 1.56, 0.64, 1)` | landing bounce |
| `--motion-fast/base/slow/spring` | kept, now `var(--duration-*) var(--ease-*)` | |

**A future speed control.** `--motion-scale: 1` multiplies every duration token: `--duration-base: calc(160ms * var(--motion-scale))`. Today it is 1 in both tiers. Making the reduced tier slower or quicker (as #1715 did, and as the one-way-flow fade did) then becomes a single declaration on `:root[data-motion-tier="reduced"]`. That is why durations must come from tokens before the policy can change.

**JS constants** move to `frontend/app/motion/tokens.ts`, with the same names and values, and a unit test that compares them with `theme.scss`. JS animations also respect `--motion-scale` by reading it once per tier change.

**Bugs fixed here:** `transition-duration: var(--motion-fast)` at `element/modal.scss:187` and `view/agent/styles/_composer-strip.scss:314`. The pair token is invalid as a duration, so the declaration computes to 0 s and the dismiss button's and the composer strip's colour fades never animate. Both become `--duration-fast`.

**As built (phase B).**
- Durations and easings in `transition` and `animation` declarations use the tokens. Three kinds of value stay literal:
  - values with no token (90, 140, 180, 200, 250, 300 and 400 ms; 0.5, 0.9, 1 and 1.5 s);
  - the loop period of an infinite animation, which is rhythm, not a transition;
  - the reduced treatments inside `when-motion-reduced` (and the raw view-transition one in `app.scss`), which stay as written.
- `--motion-spring` keeps 400 ms and its own curve, also scaled: `calc(400ms * var(--motion-scale))`.
- `control-transition` also takes `$ease` (default `--ease-standard`). The two bug-fix sites pass `--ease-emphasized`, the curve of the `--motion-fast` pair they named.
- `collapse-transition` covers the tool panel collapse in `_document-nodes.scss` and `_shell-node.scss`.
- JS reads the same values from `motion/tokens.ts`; its test compares them with `theme.scss`.
  - `HOLD_RELEASE_MS`, `WIDTH_TRANSITION_MS` and `HEIGHT_FLIP_MS` (150 becomes 160) are `DURATION_BASE_MS`.
  - The tile layout's overlay hold (150 ms) is too, because it must match the placeholder's exit fade, which became 160 ms.
  - The reveal gate and the media-ready fade use `DURATION_QUICK_MS`.
  - `GLIDE_MS` (180), `FADE_MS` (200) and `SPINNER_FRAME_MS` (a frame rate) stay literal.
- `tokens.ts` holds unscaled values. Reading `--motion-scale` in JS waits until the scale is anything but 1.

### 4.8 One keyframes file

A new `frontend/app/_keyframes.scss`, loaded once from `app.scss` (keyframes are global, and loading them once avoids the emitted-per-`@include` trap documented at `app.scss:271`):

| Keyframes | Replaces |
|---|---|
| `am-fade-in` | `cross-drag-fade-in`, `drop-indicator-in`, `modal-fade-in`, `new-tab-fade-in`, `agent-tool-summary-fade-in` (identical bodies) |
| `am-spin` | `status-icon-spin`, `agent-spin`, `amux-spin` (`init/error-display.ts:304`, inline); `agent-attachment-spin` keeps its −90° start through a `--spin-from` variable |
| `am-pulse` (uses `--pulse-min`, default 0.4) | `install-step-pulse`, `ui-status-dot-pulse` (0.35), `maintenance-pulse` (0.25), the identity `pulse` (0.4), `ctx-text-pulse` (0.8 to 0.4), `amux-question-countdown-pulse` (0.6). This also removes the generic global name `pulse`. |
| `am-blink` | `shell-cursor-blink`, `btw-overlay-cursor-blink` |
| `am-accent-flash` (uses `--flash-end-opacity`) | `term-activity-flash`, `swarm-activity-flash` |
| `am-highlight-fade` | `search-match-flash`, `settings-search-highlight-fade` (different durations stay at their sites) |

Kept as they are, because they are one of a kind:
- `agent-ant-march`, `agent-compaction-indeterminate` and `tab-drop-invert-strobe`, which carries the flash-rule note;
- `tab-drop-pulse`, `tab-bounce`, `modal-pop-in`, `modal-layer-content-in` and `agent-row-poof`;
- `brain-spinner-pulse`, plus its copy `startup-pulse` in `index.html:57`, which has to stay inline because it paints before any stylesheet loads. Its sync comment stays.

`tab-bounce`'s second use, on `.agent-activity-row.activity-flash-out` (`_shell-node.scss:243`), gets its own name, because it carries a different meaning.

**As built (phase B).**
- `am-pulse` also takes `--pulse-max` (default 1), for `ctx-text-pulse`'s 0.8. The sites set `--pulse-min` 0.35 (install step, busy dot), 0.25 (maintenance) and 0.6 (question countdown). The identity dot uses the 0.4 default.
- `am-accent-flash` has no end colour: it settles to the element's own colour. That is what both originals ended on (`inherit` on an element that sets no colour; the swarm summary's own `--secondary-text-color`). The pane header sets `--flash-end-opacity: 0.6`.
- The attachment ring sets `--spin-from: -90deg`.
- `am-blink` uses the shell cursor's body. The btw cursor's 50.01 % stop becomes 50 %, a 0.1 ms difference at 1 s.
- `am-highlight-fade` was not made. The two highlights differ in both their start and end colours, so a shared body would be only variables. `search-match-flash` and `settings-search-highlight-fade` stay where they are.
- Only the departure (`.activity-flash-out`) gets its own name, `activity-row-depart`. The dock row's arrival (`.activity-flash-in`) is a landing, the same meaning as a tab's, and keeps `tab-bounce`.
- `error-display.ts`'s reconnect spinner uses `am-spin`. `bootstrap.ts` imports `app.scss` before anything that can show the error page, so the inline `amux-spin` is gone.

Dead keyframes and CSS found by the inventory are deleted after a final check for class names built at run time:
- `float-up` (`tailwindsetup.css:115`);
- `browser-loading` and `.browser-loading-bar` (`browser-view.scss:174-178`);
- `.block-header-animation-wrap` (`block.scss:52`);
- `.tab.animate` (`tab.scss:87`).

## 5. Every animation, classified

"Reduced treatment" is what the site declares inside `when-motion-reduced`. Today it is dormant everywhere. Rows marked † already have a treatment written somewhere, whether in a raw media query, behind the dead atom, or in the no-op mixin; it moves into the mixin unchanged.

### 5.1 Status

| Animation | Where | Reduced treatment (dormant) |
|---|---|---|
| busy ring march `agent-ant-march` | `agent-view.scss:380,398` | static pattern † |
| compaction progress | `_control-bar.scss:212,218,349` | no width transition; static at "over" † |
| spinner dot `agent-pulse` | `_control-bar.scss:343,354` | none yet |
| card spinner, status icon, attachment spinner, reconnect ⟳ (`am-spin`) | `_picker.scss:563`, `StatusBar.scss:106`, `_attachments.scss:365`, `error-display.ts:293` | none yet |
| brain spinner pulse | `BrainSpinner.scss:18,48` | no pulse † |
| ASCII spinner | `AsciiSpinner.tsx:19-29` | static `*` † |
| install step, status dots, maintenance, identity checking, ctx critical, question countdown (`am-pulse`) | §4.8 sites | install step and busy dot: none †; ctx critical: 2.4 s †; others none yet |
| shell running border, shell and btw cursors | `_shell-node.scss:20,189`, `_btw.scss:118` | none yet |
| mic ripple | `MicButton.scss:24,37` | none yet |
| attention glow | `ui.scss:43,67` | none † |
| connection dots (SVG) | `asset/dots-anim-4.svg` | none yet; an SVG asset can't see the root attribute, so a reduced variant would be a second asset |
| progress bars (CPU cores, attachments, settings) | `_cpu-cores-popover.scss:144`, `_attachments.scss:120`, `settings.scss:304` | none yet |
| Font Awesome spin and pulse | vendored | restored by the override in §4.4 |

### 5.2 Feedback

| Animation | Where | Reduced treatment (dormant) |
|---|---|---|
| drop-target invert strobe | `drop-indicators.scss:102`, mixin `drop-target-hover` | flash rule applies in both tiers; motion-reduced: pulse only † (§8 Q1) |
| drop-target pulse, landing bounce | `drop-indicators.scss:82,92` | none yet |
| drop indicator fade-in | `drop-indicators.scss:27,64` | none † |
| cross-window drop overlay | `cross-window-drop-overlay.scss:35` | none yet |
| floating redock placeholder, rearrange placeholder glide, enter/exit, resize-line reveal | `app.scss:69`, `tilelayout.scss:83,228,261` | none. The glide is explicitly exempt "by design" (`tilelayout.scss:276`) and stays out of every policy (§8 Q4). |
| insertion gap in the tab bar | `tabbar.scss:143,161` | none yet |
| activity flash (Web Animations) | `activity-flash.ts:267` | flash rule; none yet |
| term and swarm activity flash, attachment flash, search and settings highlights, error-copied, decision-panel armed | §4.8 and the inventory | none yet |
| dismiss nudge | `modal.scss:267,270` | static ring † |
| zoom indicator | `zoomindicator.scss:28` | none yet |
| wheel-edge skid | `_document-nodes.scss:2904` | none † |
| landing cue on doc-tab close | `PaneTabStrip.tsx:487` | none † |

### 5.3 Transition

| Animation | Where | Reduced treatment (dormant) |
|---|---|---|
| tab-switch view transition | `app.scss:313`, `workspace.tsx:131` | 1 ms / skip † |
| modal backdrop and panel, modal layer | `modal.scss:86,99`, `modal-layer.scss:19` | none † (modal); none yet (layer) |
| new tab fade | `tab.scss:249` | none yet |
| tool summary, fade-in-once, live tail | `_document-nodes.scss:192,218,520` | none † |
| row enter (old path), one-way-flow fade | `_document.scss:290-329` | none †, 90 ms † |
| transcript glides, scroll follower | `AgentDocumentVirtualList.tsx:477,493`, `follower.ts` | no glide, τ 25 ms † |
| smooth scroll to a search hit | `AgentDocumentVirtualList.tsx:1167,1175` | none yet (instant) |
| My Agents poof and reorder | `_recent-sessions.scss:179,220`, `MyAgentsList.tsx` | 1 ms † |
| identity detail, swarm stats | `_detail.scss:119`, `swarm-stats.scss:9` | none † |
| tool panel collapse | `_document-nodes.scss:618`, `_shell-node.scss:45` | none yet |
| tool overlay FLIP | `resize-contract.ts:164` | none † |
| pane tab strip width | `PaneTabStrip.tsx:467` | none † |
| leaf reveal gate | `TileLayout.core.tsx:557` | none † |
| spinner and loading-cover fade-outs | `BrainSpinner.scss:27`, `PaneLoadingCover.scss:71`, `browser-view.scss:397,422`, `_composer-strip.scss:727` | instant † |
| status type-out | `present-status.ts:54`, `AgentFooter.tsx:250` | instant † |
| startup splash fade | `index.html`, `startup-splash.ts:18,25` | none yet (pre-stylesheet; JS only) |
| media ready fade | `media-view.tsx:440,471,480` | none yet |

### 5.4 Decorative

About 100 hover and focus colour transitions of 80-150 ms, and a few hover lifts (`color-swatch-palette.scss:17`, `_launch-modal-body.scss:161`, `_picker.scss:359`, `_recent-sessions.scss:448,451`, the `ui.scss:479` switch thumb). They get tokens (§4.7) but no individual reduced treatment. If a future policy reduces `decorative`, one global rule in `app.scss` zeroes their durations instead of a mixin at every site:

```scss
:root[data-motion-reduce~="decorative"] { --motion-scale-decorative: 0; }
```

That rule needs the decorative transitions to use `--duration-decorative-*` tokens, defined as `calc(<value> * var(--motion-scale-decorative, 1))`, which phase D1 (§7) gives them.

## 6. Stylesheet DRY: ranked

The order is by payoff against risk. The DRY items go in phases C and D (§7); phase A doesn't depend on any of them. Counts are approximate (from greps).

### 6.1 Real bugs (fix first)

1. `--motion-fast` used as `transition-duration` (`modal.scss:187`, `_composer-strip.scss:314`): those transitions never run. Fixed by §4.7.
2. `--modal-box-shadow: box-shadow: 0px 8px …` (`theme.scss:248`) carries its own property name, so its one user (`element/search.scss:11`) gets no shadow.
3. **27 variables used and never defined**, each silently using its fallback or nothing:
   - `--tertiary-text-color` alone has 18 uses.
   - Others are misspellings of real tokens: `--panel-bg` and `--panel-bg-alt` (real: `--panel-bg-color`), `--input-bg` and `--color-input-bg` (`--input-bg-color`), `--highlight-bg`, `--text-color`, `--border-radius-sm`, `--radius-2`, `--space-2-5`.
   - Each one is either defined in `theme.scss` or corrected at its use. `check-ui-primitives.mjs` already counts undefined variables, so its ratchet locks the result in.
4. The invalid `.prefers-reduced-motion` reset (`app.scss:216`). It is deleted by §4.5.

### 6.2 Mechanical and visually neutral

5. **Font sizes:**
   - 447 raw px sizes equal an existing token: 10 px ×121 → `--text-2xs`, 11 px ×133 → `--text-xs`, 12 px ×98 → `--text-sm`, 13 px ×64 → `--text-base`, 14 px ×31 → `--text-md`.
   - 9 px ×20 needs a `--text-3xs`, or a decision to raise it.
   - Related: Tailwind's `@theme` declares a second `--text-xxs: 10px` (0 uses), and `--text-title` is defined two different ways (`tailwindsetup.css` and `theme.scss:407`). Remove the duplicates.
6. **Dead hex fallbacks on tokens that are always defined:**
   - 119 of the 420 `var(--x, fallback)` uses have a hex fallback.
   - `--warning-color` alone carries six different ones, and `--error-color, #ff6b6b` appears ×15.
   - `var(--radius-sm, 4px)` and `var(--radius-md, 6px)` actively mislead, because both tokens are 0.
   - Remove the fallbacks after 6.1.3 defines the missing variables.
7. **Copy-paste inside the shared layer:** `.ui-density-comfortable` (`theme.scss:494-502`) repeats `:root` (`:464-470`), and `element/primitive-list-detail.scss` has three byte-identical class blocks.
8. **Motion values** (§4.7):
   - 143 raw durations become tokens.
   - The `transition-property: color, background-color, border-color, outline-color, box-shadow, opacity;` line appears six times, with its "Not `all`" comment copied seven times. It becomes `@mixin control-transition($duration: var(--duration-instant))`.
   - The identical collapse transition (`_document-nodes.scss:619`, `_shell-node.scss:46`) becomes `@mixin collapse-transition`.

**As built (phase C).**
- Item 2: `--modal-box-shadow` is a valid shadow, so the search box shows it.
- Item 3: undefined variables went from 27 to 0.
  - Corrected at their use: `--highlight-bg` → `--highlight-bg-color`, `--text-color` → `--main-text-color`, `--border-radius-sm` → `--radius-sm`, `--secondary-bg-color` → `--block-bg-color` (the editor, which already fell back to it) or `--panel-bg-color` (the WebFetch result, like the file's other cards), `--space-2-5` → `--space-2` or `--space-3`.
  - Defined in `theme.scss`, at the fallback most call sites showed: `--tertiary-text-color` (white at 0.5; 0.3 had as many uses, all in one file), with a light-polarity override at half the text colour; `--error-bg-color`; `--button-secondary-bg-color`. `--am-window-fade`, which the Linux host sets inline, gets its default of 1 in `:root.am-uniform-fade`.
  - Where the fallback differed from the real token, or the call sites differed from each other, the call site keeps the value it rendered, as a literal: `--panel-bg`, `--panel-bg-alt`, `--input-bg`, `--color-input-bg`, `--code-bg-color`, `--color-code-bg`, `--color-warning-bg`, `--border-color-faint`, `--selected-bg-color`, `--radius-2` (4 px), `--font-family` (`sans-serif`), Tailwind's `--default-font-*-settings` (`normal`), and the other alphas of `--tertiary-text-color` and `--button-secondary-bg-color`. The tints are candidates for item 16.
  - Not ours to define: Shiki sets `--shiki-*` on each highlighted run, and `var(--conn-icon-color-${n})` is built in a template literal. The check now leaves both out.
- Visible changes. Each was a variable with no fallback, so the style rendered nothing:
  - `--highlight-bg`: eight backgrounds in the Swarm pane and on the copy-error button (hovers, the active agent, a count pill, the retired badge, the error text box) now show.
  - `--space-2-5`: the error banner and the prerequisite modal row get their padding (it was 0), 8 × 12 px.
  - `--secondary-bg-color`: the WebFetch result's header and body sit on `--panel-bg-color`.
  - In the light themes, the five `--tertiary-text-color` sites read at half the text colour instead of half white.
- Item 5: 467 raw px font sizes became tokens, outside `themes/`: 9 px ×20 (the new `--text-3xs`), 10 ×121, 11 ×133, 12 ×98, 13 ×64, 14 ×31. The 26 sizes equal to `--text-lg` to `--text-3xl` (16, 18, 22, 28 px) and the sizes with no token stay. Tailwind's unused `--text-xxs` and `--text-title` are gone; `theme.scss`'s `--text-title` composite, which wins over Tailwind's layered one, stays.
- Item 6: 274 fallbacks on tokens that `:root` always sets were dropped (108 hex, 62 `rgb()`/`rgba()`, 60 px, 30 numbers, 14 others such as shadows). The 26 whose fallback is another `var()` stay.
- Item 7: `:root` and `.ui-density-comfortable` share one rule; the three list/detail blocks are one rule with a selector list.
- Item 20 (§6.6): `check-ui-primitives.mjs` also ratchets, per file, raw durations in transitions and animations, raw px font sizes, fallbacks on always-defined tokens and raw z-indexes.

### 6.3 Shared mixins that exist and nobody uses

9. Adopt the existing mixins where the blocks match exactly:
   - `flex-center` has 0 uses against 79 hand-written blocks;
   - `abs-fill` has 0 against 24 blocks, plus 7 fixed full-screen overlays that get a `fixed-fill`;
   - `truncate-lines` has 0 against 5 hand-written line clamps.

   Add the two that are missing:
   - `truncate`, the 114 overflow/ellipsis/nowrap blocks without `ellipsis`'s `display: block`;
   - `label-caps`, the 46 uppercase labels, which use 8 different letter-spacings and should settle on one.
10. **Focus rings** come in three styles across 54 `:focus-visible` rules: the `focus-ring` box-shadow (4 uses), a 2 px outline offset 1 px (about 8), and a 1 px outline offset −1 px (about 6). Pick one (§8 Q6), make it the `focus-ring` mixin, and ratchet the others.
11. **Button resets and close buttons:**
    - There are 22 hand-written resets (transparent background, no border, no padding), including identical stat buttons in `statusbar/_cpu-cores-popover.scss:7-26` and `_disk-volumes-popover.scss:7-26`.
    - There are 21 hand-rolled close and dismiss buttons, which move to the `element/ui` IconButton.
    - `check-ui-primitives.mjs` already counts 184 hand-rolled buttons, so this is that ratchet's backlog.

### 6.4 Near-duplicate stylesheets

12. The three approval windows (`credential-approval`, `memory-adoption-approval`, `ssh-approval-window.scss`) differ only in their class prefix, and each one's header points at the others. Make them one `approval-window` partial.
13. The status-bar popovers (`_cpu-cores-popover`, `_disk-volumes-popover`, `_token-usage`) move their stat button and rows into the existing `popover-frame`.
14. The agent modals (`AgentNativeMemoryModal.scss`, `AgentPrimitiveModal.scss`, `BundlePrimitiveSection.scss`) share one list/detail structure and move onto `primitive-list-detail`.
15. The cards in `_picker.scss` and `native-memory-manager.scss` hand-roll name, description and slug styles while `element/ui/card.scss` exists. Move them onto it.

### 6.5 Needs a design decision (later)

16. **Colour tints:**
    - There are 391 `color-mix()` calls. About 57 of them are neutral tints of the text colour at seven different percentages, and error and accent tints each come in five or six steps.
    - About 41 raw `rgba(255,255,255,…)` literals outside the theme files don't follow the theme.
    - The tinted "notice box" is repeated in 14 places.
    - Proposal: tint tokens (`--tint-1…4`, `--error-bg` / `--error-line` and the same for warning and accent) and a `notice($tone)` mixin built on `_line.scss`'s tone model. Collapsing the percentages into a few steps shifts some colours slightly.
17. **Z-index:**
    - There are two scales that disagree: `--zindex-modal-wrapper: 500` against `--z-modal: 9000`.
    - 54 of the 91 declarations use raw numbers, from 1 to 10000.
    - 10 tokens are unused.
    - Merging them needs visual QA of every overlay.
18. **Dead CSS:**
    - A rough scan found 111 of 2,510 class names with no reference in TS or TSX.
    - Hand-checked examples: all of `_import-modal-body.scss`, `identity/styles/_assignments.scss`, `term.scss:33-58`, `.flex-spacer`, `.os-scrollbar*` and `.u-cursor-*` in `app.scss`, and about 17 `agent-header*` classes.
    - Each removal is checked against class names built at run time (`sh-*` is one such family, and is not dead).

### 6.6 Small and safe

19. **Outdated prefixes and properties:**
    - Prefixes the CEF target doesn't need: `-webkit-backdrop-filter` ×3, `-webkit-mask` and `-webkit-mask-composite` (`agent-view.scss:341-342`), `-moz-appearance`, `-webkit-user-select`.
    - `word-break: break-word` ×10 is deprecated; use `overflow-wrap: anywhere`.
    - `text-wrap: nowrap` ×2 should be `white-space: nowrap`.
    - 11 `!important` declarations have no comment saying why.
20. **Enforcement:**
    - Stylelint doesn't run in CI. Run locally, it reports 487 `color-no-hex` findings, most in `themes/*.scss`, which lack the disable comment that `theme.scss` has. So the `.stylelintignore` grandfather list means nothing.
    - Rather than turning stylelint on all at once, extend `check-ui-primitives.mjs`'s ratchet with counts for raw durations, raw px font sizes, hex fallbacks and raw z-index. Each count may only go down.

## 7. Phases

Each phase is its own PR with a changeset, and each leaves the app working.

| Phase | Contents | Visible change |
|---|---|---|
| **A. Motion runs everywhere (small, first)** | `motion.ts` (tier, policy, root attribute, `shouldReduceMotion`); the `when-motion-reduced` mixin; all 14 raw media blocks, the `respect-reduced-motion` sites and the virtual list's `matchMedia` moved into it; the Font Awesome override; the JS consumers moved to `shouldReduceMotion`; the dead plumbing deleted; `check-motion-gates.mjs` in CI; the policy unit test; the busy-ring spec's lines 119 and 169, the 07-11 addendum, the stale comments, and `PANE_LAYOUT_AND_REFLOW_ARCHITECTURE.md:132-141` corrected. The Settings toggle as decided in §8 Q2. | For OS-off users, everything in §2.2 animates again, including the busy ring. Nothing changes for OS-on users. |
| **B. Motion tokens and keyframes** | §4.7 tokens and `--motion-scale`, `motion/tokens.ts`, the two `--motion-fast` bug fixes, `_keyframes.scss` (§4.8), dead keyframes removed | The dismiss button and composer strip colour fades start animating; 150 ms durations become 160 ms (§8 Q5). |
| **C. Token bugs and mechanical swaps** | §6.1.2-3, §6.2 (font sizes, hex fallbacks, shared-layer copy-paste), the ratchet counts (§6.6.20) | None intended. A missing-variable fix can change what a fallback used to show; each is listed in its PR. |
| **D1. Shared mixins** | §6.3, plus `--duration-decorative-*` for §5.4 | None intended |
| **D2. Near-duplicate sheets** | §6.4 | None intended |
| **E. Design decisions** | §6.5, each with screenshots | Small, reviewed |

**Coordination:**
- Phase A edits `AgentDocumentVirtualList.tsx` and `scroll/`, which Loap-3 is changing now (#4962, #4963, `SPEC_AGENT_PANE_NO_CATCH_UP_MOTION_2026_10_15.md`). The change there is limited to replacing the local `reducedMotion()` with `shouldReduceMotion("transition")`. Check with Loap-3 before opening it.
- The busy-ring change is AgentA's (#4934). Tell AgentA that the ring marches again in both tiers, and why.

## 8. Decisions for the operator

**Decided 2026-10-15:** the recommendation in every row below.

| # | Question | Recommendation |
|---|---|---|
| Q1 | The drop strobe's OS opt-out (`drop-feedback.scss:38,53`) was added as seizure safety, not motion policy. Under "everything runs", does it go? | Yes. The strobe makes 2 flashes at 2.5 per second, under the 3-per-second limit, in both tiers. Its reduced treatment (pulse only) stays declared, dormant, in `feedback`. The flash rule (§4.2) goes in the mixin's doc comment and in review. |
| Q2 | The "Reduce motion" toggle in Settings is dead and describes something it doesn't do. | Remove the toggle in phase A, and keep `window:reducedmotion` in the schema as a deprecated key that is ignored, so old settings files still load. When the policy first makes the tiers differ, replace it with a three-way override (Follow system / Full / Reduced) read by `motion.ts`. |
| Q3 | Two earlier owner decisions had OS-off users see the transcript's motion quicker, not off: the 90 ms one-way-flow fade and the 25 ms follower τ (`SPEC_AGENT_PANE_ONE_WAY_FLOW_2026_10_07.md:191,205`). Under "all enabled regardless", they get the full timings too. | Full timings in both tiers, as asked. The quicker values stay declared and dormant in `transition`, and `--motion-scale` (§4.7) can bring them back as one value. |
| Q4 | The tile layout's placeholder glide is exempt from reduced motion "by design" (`tilelayout.scss:276`). | Keep it outside the policy entirely: it shows where a drop lands, and it is never reduced. |
| Q5 | Merging durations into tokens moves 150 ms to 160 ms (×38), and 240 ms to 280 ms or to a new 240 ms token. | 150 → 160 ms (not visible); add `--duration-moderate: 240ms` rather than stretch those three. |
| Q6 | Three focus-ring styles. | The 2 px accent outline with a 1 px offset: it doesn't clip inside `overflow: hidden` the way the box-shadow does in some panes. |

## 9. Testing

- **Unit:**
  - `motion.ts`: tier from a mocked `matchMedia`, change events, the root attributes, `shouldReduceMotion` per policy;
  - the policy pin (§4.6);
  - `tokens.ts` against `theme.scss`;
  - the existing reduced-branch tests rewired to `shouldReduceMotion`.
- **Check:** `check-motion-gates.mjs` with fixtures for each rule it enforces.
- **Both tiers, by emulation:** `scripts/ui-screenshots/one-way-soak.mjs` already drives CDP `Emulation.setEmulatedMedia` with `--motion reduce|full|os`. Use the same call in a dev build to record the busy ring, a drop onto a tab, a modal and a row entrance in each tier, and check they are the same while the policy is empty.
- **On a real machine:** a dev build on Windows with "Animation effects" off (narko today) shows the busy ring marching and a modal fading in. Repeat with it on.
- **Policy dry run (not committed):** put `status` in the `reduced` list, check the busy ring freezes only under emulated reduce, then revert. This proves the dormant path works before anyone relies on it.
