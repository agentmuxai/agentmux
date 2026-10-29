# Motion primitives: one shared set of reusable animations on top of the motion policy and tokens

**Status:** active. Approved 2026-10-29 by the operator ("yes, motion is good"): decisions 1-6 as recommended, 7 settled with Camper. Built in the order of §5, one PR per step, each updating this line. Step 1 (the module, its tests, the guard and the Swarm row slide) is #5781; the minimize merge is next.
**Date:** 2026-10-29.
**Requested by:** the operator, through AgentY. Their words (2026-10-28): "do we have a solid animations framework, where we can quickly reuse them in a DRY Manner? lets get that to a spec".
**Author:** Agent1@narko.
**Related:** `SPEC_MOTION_TIERS_AND_STYLESHEET_DRY_2026_10_15.md` (the policy and tokens), `SPEC_MINIMIZE_MERGES_INTO_ADJACENT_MINIMIZED_PANE_2026_10_28.md` (the first consumer, Camper), `SPEC_PANE_TAB_DRAG_LANDING_FLASH_AND_LAST_TAB_CLOSE_2026_09_24.md` (the landing bounce).
**Checked against:** `origin/main` after v0.60.6.

---

## 1. Summary

The app has a motion **policy** (`frontend/app/motion/motion.ts`: tiers, the four categories `status`, `feedback`, `transition`, `decorative`, and `shouldReduceMotion(category)`) and motion **tokens** (`motion/tokens.ts` and theme.scss's `--duration-*` / `--ease-*`, kept equal by a test). It has no motion **primitives**. Every animation that is driven from code writes its own: measure the rectangles, build keyframes, call `el.animate`, remember to clean up on finish **and** on cancel, check the policy, and guard `typeof el.animate`. Seven such hand-written variants exist today (§2.1), and they overlap: the minimize merge has its own clone-and-fly, the Swarm row slide its own FLIP, and the pane-tab landing a signal and a timer.

The proposal: a small module `frontend/app/motion/primitives.ts` (plain DOM, no framework) plus a thin Solid wrapper, with five primitives built on one core `play()`:

- **`play`**: the single place that calls `animate`: policy check, support check, cleanup exactly once, a cancellable handle.
- **`move`**: FLIP. Capture rectangles before a change, play from the old place to the new one (the Swarm row slide; the pane reflow later).
- **`travel` / `mergeInto`**: a clone of an element flies from one rectangle to another and lands with a cue (the minimize merge, the pane-tab drop).
- **`enter` / `exit`**: fade, fade-scale, slide, for elements the code mounts and unmounts.
- **`pulse`**: one attention beat on an element (the landing bounce, a flash, a glow).
- **`glide`**: an offset that decays to nothing, added to whatever else is animating (the transcript's catch-up).

Each takes a **required motion category** (so every animation says which policy bucket it is in), takes durations and easings **by token name**, cleans up on finish and cancel, and returns a handle that can be cancelled. A CI guard keeps new hand-rolled ones out, with a baseline of today's that only shrinks. CSS stays the right tool for state transitions (hover, focus, open/closed); this is for **motion that code measures and starts**.

## 2. What exists today (measured on the current main)

A scan of `frontend/` (SCSS, CSS, TS, TSX; tests and the vendored Font Awesome excluded):

| | Count |
|---|---|
| Files that declare any motion | **66** (59 stylesheets, 7 scripts) |
| `@keyframes` definitions | **31**, all with distinct names (no keyframes name is defined twice) |
| `animation:` / `animation-name:` declarations | 64 |
| `transition:` declarations | 94 |
| `.animate(` calls (Web Animations API) | 6, in 5 files |
| `startViewTransition` | 5 call sites in 2 files |
| Files with motion and **no policy hook** (`when-motion-reduced` / `shouldReduceMotion`) | **43 of 66** |
| Files with literal millisecond timings instead of a token, inside `animation` or `transition` | **13** (e.g. `modal.scss` 140 and 240, `_document.scss` 180 and 90, `drop-feedback.scss` 300) |

The policy count needs a word: today `MOTION_POLICY` is empty in both tiers ("2026-10-15 (operator): everything runs in both tiers"), so a missing hook changes nothing visible now. It matters the day the policy is not empty, and the guard in §6 is about not having to audit 43 files on that day.

### 2.1 The code-driven animations (the ones primitives replace)

| # | Where | What it does | How it is written | Policy | Tokens |
|---|---|---|---|---|---|
| 1 | `app/view/swarm/swarm-order-motion.ts` | Swarm rows slide to a new place when the order changes | FLIP, Y only: capture tops, `animate translateY` | yes (`transition`) | **no: literal 180 ms** and `"ease-out"` |
| 2 | `layout/lib/layoutMinimizeMergeApply.ts` | Merged pills fly into the target pane's strip | clone appended to `body`, original hidden, `animate translate`, restore in `onfinish`/`oncancel` | yes | yes (`DURATION_MODERATE_MS`, `EASE_EMPHASIZED`) |
| 3 | `app/view/agent/virtualization/AgentDocumentVirtualList.tsx` (two places) | The transcript glides when content is added or released | `animate translateY(delta) -> 0`, `composite: "add"`, a `Set` of live animations cancelled together | yes | partly (`EASE_DECELERATE`; its own `GLIDE_MS`, `HOLD_RELEASE_MS`) |
| 4 | `app/util/pane-locate.ts` | A highlight on the pane the user asked to locate | keyframes from `locateKeyframes()`, `onfinish`/`oncancel` remove the overlay, returns a cancel function | **no** | yes |
| 5 | `app/notification/activity-flash.ts` | A flash pattern scheduled to land on the beat of a sound | scheduled keyframes on `::before`, `startTime` pinned to the audio clock | **no** | **no** |
| 6 | `app/workspace/workspace.tsx` + `app/util/view-transition.ts` | The window tab swap | `startViewTransition`, a double `requestAnimationFrame` to measure, skipped-transition errors swallowed | partly | no |
| 7 | `app/element/pane-tab-landing.ts` + `PaneTabStrip.tsx` + `drop-indicators.scss` | The landing bounce of a pane tab dropped or merged | a **signal + 400 ms timer** sets `.pane-tab--landing`; CSS keyframes `am-fade-in` + `tab-bounce` do the rest | yes (CSS) | partly (`tab-bounce` has a literal 300 ms) |

Five different shapes of "start, finish, clean up, cancel" across seven animations. The cleanup-on-cancel part is the one that goes missing (it is easy to remember `onfinish` and forget `oncancel`, which is how a hidden original or a stuck clone stays on screen); each of #2 and #4 spells it out by hand and #1 and #7 have no cancel at all.

### 2.2 The CSS keyframes, grouped by what they are

The 31 names are not duplicates of each other, but they fall into a handful of shapes written again and again:

- **Fades in** (5): `am-fade-in`, `modal-layer-content-in`, `modal-pop-in`, `ui-card-detail-reveal`, `zoom-fade-in-out`.
- **Pulses and beats** (6): `am-pulse`, `brain-spinner-pulse`, `mic-pulse`, `agent-pulse`, `shell-running-pulse`, `agent-decision-panel-armed`.
- **Flashes and highlights** (6): `am-accent-flash`, `agent-attachment-flash`, `search-match-flash`, `settings-search-highlight-fade`, `block-error-copied-fade`, `ui-attention-glow`.
- **Leaving** (2): `agent-row-poof`, `activity-row-depart`.
- **Bounce / drop feedback** (4): `tab-bounce`, `tab-drop-pulse`, `tab-drop-invert-strobe`, `tab-drop-invert-hold`.
- **Spin, blink, sweep, march** (5): `am-spin`, `am-blink`, `am-sweep`, `agent-ant-march`, `modal-dismiss-nudge` (+ `-static`).
- Others: `inside`, `fires`.

Five of them (`inside`, `tab-drop-pulse`, `tab-drop-invert-strobe`, `tab-drop-invert-hold`, `fires`) appear in no `animation` declaration the scan could find: dead, or used through a form the scan misses; the build confirms and deletes the dead ones.

### 2.3 The pane tab drag-and-drop feel, precisely

Camper's merge asks for "the same animation as pane tab dnd", so it is worth saying what that is. It is four things, and only one is an animation: (1) the browser's native drag ghost; (2) a 12 px spacer where the pill will land, which opens instantly (no tween); (3) the **landing bounce** on the pill that arrived, `.pane-tab--landing`, a fade (`am-fade-in`, `DURATION_FAST`) and a scale bounce from 0.9 to 1 (`tab-bounce`, 300 ms literal), cleared by a 400 ms timer (`LANDING_BOUNCE_MS`); (4) a spring-loaded tab switch after 500 ms over a window tab. So "the feel" to reuse is the **landing bounce**, and the travel the minimize merge adds in front of it.

## 3. Design

### 3.1 The module

```
frontend/app/motion/primitives.ts   core + primitives (DOM only, no Solid)
frontend/app/motion/solid.ts        useMotion(): the same functions, cancelled on the component's cleanup
frontend/app/motion/test-utils.ts   a fake Element.animate for jsdom, and an assertion helper
frontend/app/motion/tokens.ts       (exists) the numbers; presets below only name them
```

### 3.2 The core

```ts
type MotionHandle = { cancel(): void; finished: Promise<void> };   // finished resolves on finish AND on cancel, never rejects

function play(el: Element, keyframes: Keyframe[], opts: {
    category: MotionCategory;            // required: no default, so no animation forgets its bucket
    duration: DurationName;              // "instant" | "fast" | "quick" | "base" | "moderate" | "slow" (the tokens)
    easing: EaseName;                    // "standard" | "out" | "inOut" | "emphasized" | "material" | "decelerate" | "bounce"
    composite?: CompositeOperation;      // "add" for the glide
    pseudoElement?: string;
    key?: string;                        // one running motion per (el, key): a new play() with the same key cancels the old
    cleanup?: () => void;                // runs exactly once, on finish or cancel
}): MotionHandle
```

`play` is the only code outside `motion.ts` that may call `animate`. It: returns an already-finished handle (and runs `cleanup`) when `shouldReduceMotion(category)`, or when the engine has no `animate` (jsdom), or when the page is hidden; maps the names to the token values; cancels the previous motion with the same `(el, key)`; wires `onfinish` and `oncancel` to the one cleanup; and never throws into the caller (a failed animation must not break the update that triggered it).

### 3.3 The primitives

- **`capture(els, keyOf)` and `move(before, opts)`** (FLIP). `capture` records each element's rectangle by key; after the DOM changes `move` plays, for every element whose rectangle moved by at least 1 px, a transform from the old place to the new. Axes `x`, `y` or `both`; `zoom` accepted because panes zoom with CSS `zoom` and a transform is in the zoomed box's units (what `swarm-order-motion.ts` does by hand today); a missing key is an element that is new and gets `enter` if the caller asks.
- **`travel(el, from, opts)`** and **`mergeInto(sources, dest, opts)`**. `travel` is for an element that is already at its new place: a copy of it, in a fixed layer on `<body>`, flies there from the rectangle `from` (read before the change) with a translate, while the real element is hidden; then the copy is removed and the element shown, however the motion ends. (The copy is placed in `<body>` in viewport pixels, so no zoom option is needed; a scale was dropped as unused.) `mergeInto` is `travel` for several pieces at once: source rectangles read before the change, destination elements found lazily once the layout has rendered them, paired by index, a per-piece `land` hook; it returns one handle whose `cancel()` removes every clone and restores every original. The shape agreed with Camper is §4.
- **`enter(el, kind)` / `exit(el, kind)`**: kinds `fade`, `fadeScale`, `slideY`, `slideX`. `exit` returns the handle so the caller removes the node when `finished` resolves; with motion off it resolves at once, so the node is removed at once.
- **`pulse(el, kind)`**: kinds `bounce` (0.9 to 1 with a fade, the landing bounce), `flash` (the accent background fades out), `glow` (an outline glow). One beat; the handle cancels it.
- **`glide(el, offset, opts)`**: an offset that decays to zero, `composite: "add"` by default so it sums with another running motion (the transcript catch-up); a set of live glides can be cancelled together through the shared `key`.

Each primitive is a few lines over `play`; the point is that the cleanup, policy, cancel and token naming are written once.

### 3.4 Presets, so a reviewer sees "what feel" at a glance

A `MOTION` table names each primitive's category, duration and easing, in one place:

| Preset | Category | Duration | Easing | Replaces |
|---|---|---|---|---|
| `move` | `transition` | `base` (160) | `out` | the Swarm slide (180 literal) |
| `travel` | `transition` | `moderate` (240) | `emphasized` | the minimize merge's fly (already this) |
| `enter.fade` | `transition` | `fast` | `out` | `am-fade-in` used from code |
| `pulse.bounce` | `feedback` | `slow` (280) | `bounce` | `tab-bounce` (300 literal) |
| `pulse.flash` | `feedback` | `moderate` | `out` | `am-accent-flash`, `search-match-flash`, `agent-attachment-flash` |
| `glide` | `transition` | `moderate` | `decelerate` | the transcript glides |

A few of these move a timing by up to 20 ms to land on a token (180 to 160, 300 to 280); §7 D3 asks whether that is accepted or whether a token is added for the old value.

### 3.5 In Solid

`useMotion()` returns the same functions, bound to the component: every handle it creates is cancelled in `onCleanup`, so an element that unmounts mid-animation never leaves a clone or a hidden original behind. A `use:flip` directive is **not** proposed: a measured FLIP is a call after a change, not something an attribute can express, and the existing code calls it from effects.

### 3.6 What stays CSS

Hover, focus, pressed, open and closed, selected: state-driven transitions stay `transition:` in the stylesheet using `var(--duration-*)` and `var(--ease-*)` and `when-motion-reduced`; they are 94 declarations and they are the right tool. Looping decoration (spinners, the busy pulse, the ant march) stays a CSS keyframe in the one shared `_keyframes.scss`. The primitives are for motion that **code measures or starts**.

### 3.7 Exceptions that stay as they are

- `activity-flash.ts` schedules keyframes against the **audio clock** (it pins `startTime` to the sound's time), which is not a general shape; it stays, listed as the one allowed exception, and calls `shouldReduceMotion` as now.
- The window tab swap uses the browser's view transition (it animates a screenshot of the old window), a different mechanism; `runInViewTransition` stays the one entry point for it.

## 4. The `mergeInto` shape (agreed with Camper, 2026-10-29)

The minimize merge (`layoutMinimizeMergeApply.ts`, Camper-2's #5765) showed two things a "one target element" shape got wrong: the tabs land as **N pills in the target's strip, one per source**, and those pills **do not exist until the layout has rendered the move**, so the destination has to be found lazily. The source pane is gone by then too, so the sources are rectangles read before the move.

```ts
mergeInto(
    sources: ReadonlyArray<DOMRect>,                  // where each piece was, read before the change
    dest: ReadonlyArray<Element> | (() => Element[]), // where each piece is now, paired by index; a thunk is called once the layout has rendered it
    opts?: {
        category?: MotionCategory;                    // default "transition"
        stagger?: number;                             // ms between pieces, default 0 (the merge flies them together)
        land?: (el: Element, i: number) => void;      // per piece, when it arrives: the caller marks it landed (the landing bounce)
        pulse?: "bounce" | "flash" | false;           // a pulse of each arrived piece, default false when `land` is given, else "bounce"
    }
): MotionHandle                                       // cancel() removes every clone and restores every real element
```

How it runs: the thunk is resolved after one animation frame (the spot where today's code waits with a `requestAnimationFrame`); each destination element is hidden (`visibility: hidden`) while its fixed-position clone flies from the source rectangle to the destination's rectangle (`DURATION_MODERATE_MS`, `EASE_EMPHASIZED`, the same as today); on arrival, or on cancel, the real element is shown again and the clone removed, and `land(el, i)` is called on arrival only. With `shouldReduceMotion(category)` true nothing flies, the elements are shown at once and `land` is still called, so the end state is the same. A destination missing at `i` (fewer pills than sources) is skipped.

`land` is where the pane tab strip calls `markLanded` (the landing bounce is the `.pane-tab--landing` class on each arrived pill), so the drop and the merge keep **one** landing cue; moving that bounce itself onto `pulse(pill, "bounce")` is Phase 2 (§5). The strip scroll that reveals the arrivals (`revealArrivals`) stays Camper's, outside the primitive. `layoutMinimizeMergeApply.ts` then shrinks to: read the source rectangles, call `mergeInto`, scroll the strip.

## 5. Which animations move onto it, in order

1. **Phase 1, with Camper's merge:** the module, the test utilities, the guard (§6) with its baseline, and the first two consumers: the Swarm row slide (`move`) and the minimize-merge fly (`mergeInto`). These are the ones with a hand-written FLIP or clone today.
2. **Phase 2, the pane tab feel:** the landing bounce (`pulse.bounce`), shared by the drop and the merge; the tab-bounce literals go; `drop-indicators.scss` loses its dead keyframes.
3. **Phase 3:** the transcript glides (`glide`) and the pane locate overlay (`pulse`/`enter`).
4. **Phase 4, CSS tidy:** one entry in `_keyframes.scss` per shape (fade in, pulse, flash, leave) as mixins or utility classes, the 13 files' literal millisecond timings onto tokens, the dead keyframes deleted. This is the mechanical part and can be done file by file.

## 6. The guard

`scripts/check-motion-gates.mjs` already enforces that `motion.ts` is the only reader of the OS motion setting. A second rule in the same script (so one CI step), with a baseline file like `scripts/check-file-sizes.baseline`:

- `.animate(` and `startViewTransition(` outside `frontend/app/motion/`, `util/view-transition.ts` and the listed exception: **fails**, unless the file is in the baseline (3 files once the Swarm slide moved; they may only be removed).
- A new `@keyframes` outside `_keyframes.scss`: fails unless baselined (today's 23 in 17 files; the other 8 of the 31 are in `_keyframes.scss`).
- A literal `<n>ms` in an `animation:` or `transition:` declaration (or their `-duration`/`-delay`) instead of `var(--duration-*)`: fails unless baselined (15 files when counted this way; §2's 13 was an earlier, looser count).

Counts are per file, outside comments and tests, in `scripts/check-motion-gates.baseline` (`<rule> <path> <count>`). `node scripts/check-motion-gates.mjs --update` lowers the numbers after a migration; it never adds a file or raises a count.
- `play()` called without a `category`: a type error, not a lint.

The baseline only shrinks: removing a migrated file from it is part of that file's PR, and the script fails when a baselined file no longer needs its entry, as the size ratchet does.

## 7. Tests

Each written to fail without its change.

- **Core (jsdom, with the fake `animate`):** `play` runs the animation and cleanup once on finish; cleanup runs once on `cancel()`; `cancel()` after finish does nothing; a second `play` with the same `(el, key)` cancels the first and runs its cleanup; with `shouldReduceMotion(category)` true no animation starts and cleanup still runs; with no `animate` support the handle is finished and cleanup runs; a throwing `animate` never reaches the caller.
- **`move`:** an element that moved by 0.5 px is not animated; one that moved 40 px animates from the old place; `zoom` divides the offset; a key that vanished is skipped.
- **`travel` / `mergeInto`:** the clone exists during the flight and is removed after; the original is hidden and shown again, also after `cancel()`; the pulse plays once on the target after the last arrival; stagger order; `cancel()` mid-flight restores everything.
- **`enter` / `exit`:** `exit` resolves `finished` at once with motion off; at the end of the animation with motion on.
- **Solid:** unmounting a component with a running motion cancels it and removes its clone.
- **Guard:** `check-motion-gates.test.mjs` cases for each rule: a new `.animate(` fails, a baselined one passes, a baselined file that no longer matches fails the ratchet.
- **Tokens:** `tokens.test.ts` already compares the TS values with theme.scss; the presets table is tested to name only existing tokens.
- **Live (my own isolated `task dev`, `AGENTMUX_HOME_OVERRIDE`):** the Swarm slide and the minimize merge before and after, side by side in a short recording; a pane tab drop shows the same bounce.

## 8. Edge cases

- **Two motions on one element** (a slide while a glide is running): `key` separates them; `composite: "add"` sums them where asked; the same key replaces.
- **Element removed mid-flight:** the handle's cleanup runs on cancel; `mergeInto` removes its clones however it ends.
- **Pane zoom:** `move` and `travel` take the zoom of the box the transform is in (the existing code passes it by hand).
- **Hidden page or background window:** `play` skips the animation (a background tab throttles timers and would arrive late).
- **Reduced policy:** with a category in `MOTION_POLICY.reduced`, the primitive runs no motion and still delivers the end state (the original shown, the clone gone, the exiting node removable).

## 9. Decisions for the operator

Each with a recommendation; the rest of the spec assumes the recommendations.

1. **Scope: only code-driven, measured motion goes onto the primitives; CSS state transitions and looping decoration stay CSS.** Recommend **yes**: the 94 `transition:` declarations are correct as they are.
2. **A required motion category on every primitive call (no default).** Recommend **yes**: it is what makes the policy complete.
3. **Move a few timings onto the nearest token (180 to 160, 300 to 280, 200 to 240 or 160) in the migrations**, or add a token for the old value. Recommend **move to tokens**: 20 ms is not visible and the point is one vocabulary.
4. **A baseline-ratchet guard** (today's offenders listed, may only shrink) rather than fixing all 66 files at once. Recommend **yes**.
5. **`activity-flash.ts` stays a documented exception** (audio-clock scheduled), as does the view-transition entry point. Recommend **yes**.
6. **The migration order in §5**, Swarm slide and minimize merge first (they have a hand-written FLIP and clone today), the CSS tidy last. Recommend **yes**.
7. ~~Whether `mergeInto` needs a per-piece `land` hook~~ settled with Camper: yes, and the destination is lazy and paired by index (§4).
