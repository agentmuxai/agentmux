# Retro: block-mount BrainSpinner overlay can get stuck visible forever

**Date:** 2026-08-23
**Owner:** AgentY
**Area:** `frontend/app/block/block.tsx` (`Block`'s `ready()` gate), `frontend/app/view/agent/agent-view.tsx` (`AgentPicker` -> `AgentPresentationView` cross-fade)
**Introduced by:** PR #2768 (`f59bb43b7`, 2026-08-22), "feat(layout): cross-fade the remaining pane-mount hard cuts (Phase 3-4)"

---

## Symptom

Reported by the user directly: a "pulsating brain logo" (`BrainSpinner`)
visible in agent panes, seemingly permanently, on top of already-loaded,
actively-rendering content. Confirmed live in the reporting session's own
hosting window via `UIScreenshot` + `UIQuery`: multiple
`.block-ready-gate-host.is-overlay` elements were mounted at full opacity,
carrying no `is-fading` class, in panes that had been actively rendering
new content for the entire (long-running) session. Not a cosmetic flicker —
a real, indefinite stuck state.

## Root cause

`block.tsx`'s `ready()`-gate spinner (added same as the bug in PR #1992,
restructured into a cross-fade by PR #2768) used this shape:

```ts
const [spinnerVisible, setSpinnerVisible] = createSignal(!ready());   // (A)
...
createEffect(on(ready, (isReady) => {
    if (isReady) {
        ...
        setTimeout(() => setSpinnerVisible(false), READY_GATE_FADE_MS);  // (B) — the ONLY place that hides it
    }
    ...
}, { defer: true }));
```

`on(..., { defer: true })` intentionally skips calling the callback on the
effect's first run — the assumption being that if `ready()` is already
`true` by then, (A) already seeded `spinnerVisible` correctly to `false`
and there's nothing to do.

The bug: (A) and the effect's first run are **not the same read, taken at
the same time.** (A) runs synchronously during component construction,
before any child renders. `createEffect` doesn't get its first flush until
after the render commits — a later pass of Solid's scheduler. If `ready()`
flips from `false` (at the moment (A) ran) to `true` in that gap — which is
the *common* case, not a rare edge case, for any block whose `blockData`/
`viewModel` resolve from an already-warm cache within a microtask of mount
— then:

- (A) seeded `spinnerVisible = true` (ready was false at that instant).
- The effect's first run sees `ready() === true`, and `defer` swallows the
  callback — (B) never runs.
- `ready()` doesn't change again (it's already settled `true` and stays
  that way for the rest of the block's life), so the effect never fires a
  second time either.
- `spinnerVisible` is stuck at `true` forever. The `BrainSpinner` stays
  mounted, at full opacity, animating its normal 1.8s CSS pulse
  indefinitely — reading exactly as "a pulsating brain logo that never goes
  away."

The identical shape existed in `agent-view.tsx`'s `AgentPicker` ->
`AgentPresentationView` cross-fade, added in the same PR, seeded from
`agentId()` instead of `ready()`. Same race, same fix needed, not
separately live-reproduced but structurally identical — fixed alongside the
confirmed one rather than left as a known-latent duplicate.

This is a brand-new regression, not a long-standing bug: before PR #2768,
`block.tsx`'s gate was a plain `<Show when={ready()}>` hard cut with no
persisted "shown" signal at all, so this construction-vs-effect timing gap
had nothing to get stuck on. The PR's own description flagged "Not manually
verified in a running browser" for exactly this code.

## Fix

Collapse the two separate reads into one. Instead of seeding a signal at
construction and separately gating a deferred effect, read `ready()` (or
`agentId()`) once per effect run and special-case the *first* run inline,
using a local (non-reactive) `initialized` flag:

```ts
const [spinnerVisible, setSpinnerVisible] = createSignal(true);
let spinnerGateInitialized = false;
createEffect(() => {
    const isReady = ready();
    if (!spinnerGateInitialized) {
        spinnerGateInitialized = true;
        setSpinnerVisible(!isReady);
        setSpinnerFading(false);
        return;
    }
    // ...unchanged fade-out / re-show logic
});
```

Now there is exactly one read of `ready()` that decides the initial state,
and it happens inside the same effect that will later react to changes —
no gap for the two to disagree in.

## Verification

- `npx tsc --noEmit` — clean.
- `npx vitest run` — full suite green, no regressions.
- No dedicated component-render test exists for either `block.tsx` or
  `agent-view.tsx` (`Block`/`AgentViewWrapper` are heavy RPC/store
  orchestration components with no existing render harness — the same gap
  PR #2622 and PR #2768 both explicitly flagged and deferred). Following
  the same precedent rather than introducing a new, riskier test harness
  under this fix; not manually re-verified in a running browser this pass
  either — flagging plainly rather than claiming visual confirmation that
  wasn't done.

## Prevention / follow-ups

- Not done here, deliberately: extracting a shared `createRevealGate`
  primitive for the two now-identical fixed patterns. Would reduce future
  duplication risk but widens this fix's diff and touches both call sites'
  signal names — left as a follow-up rather than bundled into a bug fix.
- Worth a broader sweep for any OTHER `on(signal, ..., { defer: true })`
  use paired with a construction-time seed derived from that same signal —
  this exact shape is the actual bug pattern, not something specific to
  spinners. Not performed in this pass.
