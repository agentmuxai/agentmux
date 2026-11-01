# SPEC: an Animations page in Settings, with four presets that fill editable numbers

**Date:** 2026-11-01
**Author:** Agent1@narko
**Status:** approved by the operator, 2026-11-01 ("Approve the Animations spec", relayed by AgentY). Task #15460 (spec), build #15461.
**Verified against:** `origin/main` @ `b88a28904`, by reading `frontend/app/motion/`, `theme.scss` (the motion tokens), `frontend/app/view/settings/` and `schema/settings.json`. The audit and the research behind it are Clare's `REPORT_ANIMATION_AUDIT_2026_11_01.md` (uncommitted); its numbers are used here and cited by section.
**Builds on:** [`SPEC_MOTION_TIERS_AND_STYLESHEET_DRY_2026_10_15.md`](SPEC_MOTION_TIERS_AND_STYLESHEET_DRY_2026_10_15.md) (the tiers, the categories, `--motion-scale`) and [`SPEC_MOTION_PRIMITIVES_2026_10_29.md`](SPEC_MOTION_PRIMITIVES_2026_10_29.md) (the JS primitives).

## 0. The ask

From the operator, via AgentY, 2026-10-31:

> "ok, we need to make sure animations are well placed throughout, with an option to disable them in settings. lets do an audit as well as a new page in settings dedicated to animations"

> "we'd want animations disabled, quick, and smooth, and slow, and research best practices, for users configuring their experience"

> "the animation fields would be numbers, both those 4 categories would be presets"

And the standing rule of 2026-10-15: the operator keeps Windows "Animation effects" off and still wants the app's motion. The OS setting must not switch the app's motion off for them.

## 1. Summary

1. **One new Settings page, "Animations".** A segmented control (Off, Quick, Smooth, Slow, and Follow system) over editable numbers. Off, Quick, Smooth and Slow are presets: choosing one fills the numbers in. Editing a number turns the choice into Custom and keeps it there.
2. **The main number is "Animation duration", a percentage of the designed timing.** 0 means off; otherwise 25 to 300, in steps of 5; default 100. It is the existing `--motion-scale` times 100, so the CSS side is already built. Presets: Off 0, Quick 50, Smooth 100, Slow 150.
3. **The default is Smooth, not Follow system.** Follow system is offered and is opt-in (D1). A default that follows the OS would turn the operator's motion off.
4. **The page is only honest if the number reaches everything.** Today 141 of 144 transitioning elements follow `--motion-scale`, but 39 CSS declarations with a raw time, every JS duration and a few inline styles do not (report §5). The build routes them through one scale first, then adds the page.
5. **Zero must be safe.** Off sets durations to 0. Anything that waits for `transitionend`, `animationend` or `finished` must not wait forever (§6).
6. **Not scaled, by design:** infinite loops (their rate is their meaning), flashes (WCAG 2.3.1: at most 3 a second), and every computed duration is capped at 1000 ms.

## 2. What exists

- **CSS tokens** (`theme.scss`): `--motion-scale: 1` and `--duration-instant/fast/quick/base/moderate/slow/pulse` (80, 100, 120, 160, 240, 280, 1200 ms), each `calc(Nms * var(--motion-scale))`. `--motion-scale-decorative: 1` multiplies the four `--duration-decorative-*` tokens on top. `--motion-spring` is `calc(400ms * var(--motion-scale))`.
- **Tiers and policy** (`motion/motion.ts`): `MotionTier` full or reduced, read once from the OS query; four categories (status, feedback, transition, decorative); `MOTION_POLICY` is a constant and reduces nothing in either tier (operator, 2026-10-15). `shouldReduceMotion(category)` and `<html data-motion-reduce>` are derived from it.
- **JS** (`motion/tokens.ts`, `primitives.ts`): the same durations as unscaled constants, plus about 25 constants elsewhere (report §5).
- **Settings:** `window:reducedmotion` exists in the schema and is marked "Deprecated, ignored". There is no motion row anywhere in Settings.

## 3. Design

### 3.1 The settings

Four keys, in a new `animation:` group (the colon convention of the other keys), stored with the rest of the settings and broadcast to every window:

| Key | Type | Meaning | Default |
|---|---|---|---|
| `animation:preset` | `"system" \| "off" \| "quick" \| "smooth" \| "slow" \| "custom"` | What the segmented control shows | `"smooth"` |
| `animation:speed` | number | "Animation duration", percent. 0, or 25 to 300 in steps of 5 | 100 |
| `animation:polish` | number | "Hover and fade polish", percent. 0, or 25 to 300 in steps of 5. It is `--motion-scale-decorative` | 100 |
| `animation:reducemovement` | boolean | Replace slides, zooms and bounces with fades | false |

`window:reducedmotion` stays deprecated and ignored.

### 3.2 Presets fill the numbers

| Preset | Duration % | Hover and fade polish % | Reduce movement |
|---|---|---|---|
| Off | 0 | 0 | on |
| Quick | 50 | 100 | off |
| Smooth (default) | 100 | 100 | off |
| Slow | 150 | 100 | off |
| Follow system | the row it resolves to, read-only (§3.3) | | |

Choosing a preset writes `animation:preset` and the three numbers. Editing any number writes `animation:preset: "custom"`. The last custom numbers are remembered, so choosing a preset does not lose them: "Use my custom values" returns to them. Quick and Slow leave polish at 100 so hover lifts do not stack to a quarter of their size (0.5 times 0.5).

Clamp on write: 0 stays 0; 1 to 24 snaps to 25 (at 25% the 80 ms token is 20 ms, about one frame, which looks like 0 and not like a quick motion); above 300 snaps to 300; other values snap to the step.

### 3.3 Follow system

`"system"` is the only value that reads the OS. With no OS preference it resolves to Smooth. With "reduce" it resolves to the reduced tier. Today that tier's policy reduces nothing, so it equals Smooth; when the operator later turns the policy on, it replaces slides, zooms and bounces with fades and keeps the timing (Apple and WebKit guidance, report Part B §2 and §4).

Every other value ignores the OS, in both directions. When the OS asks for reduced motion and the user chose something else, the page shows one line: "Your system asks for reduced motion. AgentMux is not following it because you chose Smooth." with a Follow system button. While in Follow system, the numbers show the values it resolved to, read-only, labelled "from your system". Each window resolves the OS query for itself, as today.

### 3.4 What each value does

A resolver turns the settings into three effective values: `scale` (speed / 100), `decorativeScale` (polish / 100) and the set of reduced categories.

- **Duration:** `--motion-scale` on `<html>` is `scale`. Every `--duration-*` and `--motion-spring` token is capped: `min(calc(Nms * var(--motion-scale)), 1000ms)`.
- **Polish:** `--motion-scale-decorative` on `<html>` is `decorativeScale`.
- **Reduce movement:** the reduced categories are `transition` and `decorative` (their reduced treatments are fades and no hover lifts), through the existing `<html data-motion-reduce>` and `shouldReduceMotion`. The `MOTION_POLICY` table stops being a constant: it is derived from the settings (and, in Follow system, the tier). The tier names stay.
- **Off:** scale 0 and all four categories reduced, so loops with literal periods (status) and flashes also take their static treatments.
- **JS:** a reactive `motionScale()` in `motion/motion.ts` returns the same number written to `--motion-scale`. `motion/tokens.ts` becomes functions of it (`durationBase()` and so on, or a `ms(n)` helper), `play()` and the primitives read it, and the other JS constants import their durations from `tokens.ts`. The 1000 ms cap applies there too.

### 3.5 Not scaled

- **Infinite loops** (spin, pulse, march, blink): the period is the message. They get named tokens (`--duration-spin`, `--duration-pulse` exists) and are unaffected by the number. Under Off they become static (a static ring plus text).
- **Flashes:** never faster than 334 ms apart, in every preset (WCAG 2.3.1). The activity flash and the tab drop strobe are checked against it.
- **The transcript scroll follower's time constant** (`FOLLOW_TAU_MS`): a control loop, not an animation.

## 4. The page

A section "Animations" in the Settings rail, after Appearance (icon `wand-magic-sparkles`; `SettingsSection` gets `"animations"`, `SETTINGS_SECTION_LABELS` and `SETTINGS_SECTIONS` gain the row, the search index gains the rows below).

Top to bottom:

1. **Segmented control:** Follow system, Off, Quick, Smooth, Slow. One help line under the selected choice: Off "Nothing moves. Changes happen instantly."; Quick "Short, snappy animations."; Smooth "The default."; Slow "Longer, gentler animations."; Follow system "Uses your computer's setting." with the OS name and where it lives (Windows: Settings, Accessibility, Visual effects, Animation effects).
2. **Live readout:** "Menus open in 160 ms" computed from the base token, so the percentage means something.
3. **Preview:** a small region with three samples that use the chosen settings (not the OS): a card that slides and fades in, a toggle and a button press, and a collapsing panel, with a Replay button. Under Off it says "No animation". The segmented control and sliders do not animate their own highlight when Off is chosen.
4. **Advanced** (a fold): "Animation duration (%)" as a slider and a number field (the kit's `SliderControl` and `NumberControl`), "Hover and fade polish (%)" the same, and the "Reduce movement" switch. Each shows its range and "Lower is quicker". Out-of-range entries snap with a short message. "Use my custom values" and "Reset to default" (Smooth, 100, 100, off).
5. **The OS notice** of §3.3 when it applies.

Changes apply at once to every window; there is no Save. A mistaken value is one click away: Reset, or choosing a preset.

The command palette gains "Animations: Off", "Animations: Quick", "Animations: Smooth", "Animations: Slow", so the setting is reachable without opening Settings.

## 5. Where it lives

- **Schema and types:** `schema/settings.json`, the srv settings struct (`crates/srv/src/backend/wconfig/types.rs`), `frontend/types/srv-types.d.ts` and `settings-template.jsonc` gain the four keys. `gen-rpc-stubs --check` is run.
- **Resolver:** `frontend/app/motion/animation-settings.ts` (new): clamp and snap, the preset table, `resolveAnimation(settings, tier)`, and the code that writes `--motion-scale`, `--motion-scale-decorative` and `data-motion-reduce` on `<html>`. `initMotion()` (the only reader of the OS query, kept so by `check-motion-gates.mjs`) calls it, and again on every settings change and every OS change.
- **Page:** `frontend/app/view/settings/sections/animations-section.tsx` and its test, with the preview in its own file.

## 6. Zero must be safe

With duration 0 an animation fires no `transitionend`, `animationend` or `finished` that a caller can wait on in the usual time. Every such wait is checked and given a fallback (a timeout, or a check of `motionScale()` first). The audit names them (report Fix 3): the reveal gate, the FLIP holds, `PaneLoadingCover`, the spinner fade-outs, `strip-reveal`'s 800 ms fallback. The build adds one test that sets the scale to 0 and mounts each of them.

## 7. Build

The ranked fixes of the report are the order of work. Small PRs, each merged on its own:

1. **Core (Agent1):** the four keys, the resolver, the `motionScale()` accessor, the capped tokens, `tokens.ts` and `primitives.ts` on the scale, the zero-safety checks for the primitives, and the Animations page with the preview and the command palette entries. After it the number reaches every CSS token and every primitive.
2. **JS constants (Agent1):** the other JS durations (`pane-locate`, `LANDING_BOUNCE_MS`, `revealCharMs`, the three 200 ms fades, the inline styles at `TileLayout.core.tsx:577`, `image-viewer.tsx:326`, `av-viewer.tsx:36`) onto `tokens.ts`.
3. **CSS raw times (Clare, by area):** the 39 declarations with a raw `ms` or `s` onto tokens. Names: a token for each current value that already has one; the loop periods get `--duration-spin` and a short list (report Fix 2). Agent1 approves the names.
4. **The guard (Clare):** `check-motion-gates.mjs` learns that `s` is a unit, and its baseline falls.
5. **Per area (owners):** pane open and close (#15458), overlay entrances (report Fix 5), the policy bypasses (Fix 7).

Order matters for honesty: PR 1 can ship the page, but until 2 and 3 land the number leaves 39 CSS declarations and some JS constants behind. The page says nothing about this; it is closed by 2 and 3, and PR 1's description lists what is still unscaled.

## 8. Tests

- **Resolver:** each preset's values; Custom on a number edit; clamp and snap (0, 3, 24, 25, 301); the remembered custom values; Follow system with and without the OS preference; an explicit choice winning over the OS both ways.
- **Tokens:** the capped CSS expression and `tokens.ts` agree (the existing `tokens.test.ts` is extended); a duration of 3 times 400 ms is 1000 ms.
- **Zero:** each wait in §6 completes with scale 0.
- **Page:** choosing a preset fills the numbers; editing one shows Custom; Reset; the OS notice; the readout; search finds the rows.
- **Live:** in an isolated dev build, Off, Quick, Smooth and Slow measured on a transitioning element's computed duration (the report's method: 141 of 144 scaled at `--motion-scale: 4`; after the build the three stragglers too).

## 9. Decisions

| # | Decision | Recommendation |
|---|---|---|
| D1 | The default | **Smooth.** Follow system is opt-in. The report recommended Follow system as the default; with Windows animation effects off (the operator's machine) that would turn their motion off, against their 2026-10-15 rule. |
| D2 | What "Reduce movement" reduces | `transition` and `decorative`. Off reduces all four. |
| D3 | The range | Off is exactly 0; otherwise 25 to 300, step 5. |
| D4 | The cap | 1000 ms on every computed duration. |
| D5 | Loops and flashes | Not scaled; flashes at least 334 ms apart. |
| D6 | Is "Hover and fade polish" a number? | Yes, under Advanced, because the tokens already have the variable. Per-token milliseconds are not exposed: the seven tokens are one related scale. |
| D7 | `window:reducedmotion` | Stays deprecated and ignored. |

## 10. Built after the page, not in its PRs

Pane open, close, split and reflow motion (#15458) and the entrances for menus, popovers and toasts (report Fix 5) are part of the same program: they are step 5 of §7, built by their area's owners after the page, and every one of them reads the same scale, so the setting governs them from their first day. They are not in PRs 1 to 4 because they are new animations, not the control. PR 1 says in its description, and the page says in a line, which areas the setting covers so far.

Out of scope altogether: the consolidation of the nine busy-indicator periods beyond naming them, and native browser pane reflow.
