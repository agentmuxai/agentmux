# Sysinfo Plot — Continuous-Monitor Animation

**Date:** 2026-05-03
**Status:** Analysis + spec draft. Brittleness assessment in §5 — read that first before deciding to build.
**Scope:** `frontend/app/view/sysinfo/sysinfo-plot.tsx`

## Problem

The sysinfo plot today re-renders on each new sample. Samples arrive at the configured interval (typically 1s), so the chart **snaps** rather than glides. Users watching the chart for "is the CPU still pegged?" want the visual continuity of a real monitoring tool — the line should feel like it's flowing in from the right, not punching in once a second.

## Goal

Between samples, the plot slides left at a rate that makes the next sample arrival feel continuous. On sample arrival, the plot rebases to the new data without visible discontinuity.

This is a **visual** improvement only — the underlying data and sample rate don't change.

## The math (linear)

Given:
- `intervalMs` — configured sample interval (e.g. 1000)
- `lastSampleTs` — backend timestamp of the most recent sample
- `lastSampleArrivalT` — wall-clock time we received that sample (`Date.now()`)
- `visibleSpanMs` — total time span the chart shows
- `chartWidth` — pixels available for the data area

Then at any wall-clock time `T`:

```
elapsed   = T - lastSampleArrivalT
progress  = min(elapsed / intervalMs, 1.0)           // 0..1, capped
rightEdge = lastSampleTs + intervalMs * progress     // virtual right-edge time

# Pixel-shift form (for CSS transform):
pxPerMs    = chartWidth / visibleSpanMs
shiftPx    = (rightEdge - lastSampleTs) * pxPerMs    // = pxPerMs * intervalMs * progress
```

Apply each frame as `transform: translateX(-shiftPx + 'px')` on the SVG/canvas data layer.

The `min(..., 1.0)` cap matters: if the next sample is late (network jitter, backend hiccup), the chart "stalls" at the next-expected position rather than sliding off into space. When the late sample arrives, we reset and slide the new amount — the user sees a brief pause but no glitch.

## Smoothing options

Linear works. If a smoother feel is wanted, replace `progress` with an easing curve:

```
linear:    progress
ease-out:  1 - (1 - progress)^2          # slows near sample boundary
ease-in:   progress^2                    # accelerates into the boundary
spring:    requires a spring system, two params (stiffness, damping)
```

**Recommendation: linear.** At 1Hz the easing window is the entire second; non-linear curves make the chart feel "rubbery" or "laggy" because the eye perceives the speed change. Linear matches what users expect from `top`/`htop`/Activity Monitor.

## Implementation sketch

Two viable architectures:

### A. CSS transform on the SVG (recommended)

1. Render the plot once per sample at its true position (existing `Plot.plot()` call).
2. Wrap the plot's data layer (SVG `<g>` or canvas) in a div with `overflow: hidden`.
3. Each frame (RAF), compute `shiftPx` and apply `transform: translateX(-shiftPx)` to the data layer.
4. On new sample arrival: stop the RAF, re-render with new data, reset transform to 0, restart RAF.
5. Axis ticks/labels: leave them as-is (snap on re-render). The slight mismatch between sliding line + snapping axis ticks is invisible at 1s intervals.

**Pros:** GPU-accelerated transform, near-zero CPU per frame. Re-render frequency unchanged (1Hz).
**Cons:** transform clipping requires the wrapper `overflow: hidden` to be precise; the slid pixels need to come from somewhere — either we render the plot wider than visible (1 sample's worth of buffer on the right), OR the rightmost edge appears empty as the chart slides.

The "render wider, clip to visible" approach is cleanest. Add `intervalMs` worth of pixels to the right of the visible domain when calling `Plot.plot()`; the wrapper masks them; sliding reveals them progressively.

### B. Re-render every RAF tick

Simply call `Plot.plot()` 60 times per second with an updated x-domain. No transform tricks.

**Pros:** simpler. Axis ticks slide too (no snap mismatch).
**Cons:** ~60× CPU cost vs. today. At 8 sysinfo panes open this is noticeable. `Plot.plot()` recreates DOM nodes; not designed for high-frequency re-render.

A is better. Build A.

## Edge cases to design for

| Case | Handling |
|---|---|
| Tab inactive (browser pauses RAF) | On `visibilitychange` → visible: snap transform to truth (compute `shiftPx` at current wall-clock; instantly apply). Resume RAF. |
| Window resize during animation | Recompute `pxPerMs` from new `chartWidth`; current frame's `shiftPx` recalculates naturally next tick. Visual hiccup of one frame, acceptable. |
| Sample arrives early (rare) | Reset baseline as usual; `progress` jumps from <1.0 back to 0; no visual artifact because `rightEdge` always lands at `lastSampleTs`. |
| Sample arrives very late (>2× intervalMs) | Stalled chart; on arrival, reset baseline. User sees a stutter — accurately reflects that a sample was late. Don't try to hide this. |
| Pane closed mid-animation | `onCleanup` cancels the RAF. Standard. |
| Multiple plots in same pane (CPU + mem + net) | Each owns its own RAF loop. At 60Hz with ~5 plots, cost is 300 transform updates/sec — still trivial because transform doesn't trigger layout. |
| Sample interval changes mid-stream | Stop animation, re-render plot at new domain width, restart with new `intervalMs`. |
| User scrolls the plot back in time (history view) | Animation only applies in "live" mode (data continues to scroll in from the right). If a "frozen" mode exists for historical inspection, animation is disabled there. |

## Brittleness assessment — honest

**The animation itself is not brittle.** The math is bounded (`min(..., 1.0)` cap), the transform is decoupled from re-rendering, and edge cases reset to truth on each new sample.

**Three real risks:**

1. **Sample timing assumptions.** The animation assumes samples arrive at roughly `intervalMs` cadence. In practice samples have jitter from:
   - Backend cron tick (sysinfo subscription is on the srv side, fires every 2s per logs)
   - WebSocket latency
   - Frontend event-loop pressure

   If average jitter exceeds ~10% of `intervalMs`, the animation will frequently hit the `min(..., 1.0)` cap and stall briefly before each sample. At 1s intervals with 100ms jitter, this is invisible. At 200ms intervals with 100ms jitter, it's distracting.

   **Mitigation:** measure actual inter-sample arrival distribution before shipping. If P95 jitter > 15% of interval, consider a small "look-ahead" (animate to 95% of next position; the last 5% snaps on actual arrival).

2. **Plot library re-render cost stays the same.** The animation only smooths between samples — each sample still triggers a full `Plot.plot()` rebuild. If the user has many sysinfo plots open and the underlying re-render is already a perf concern, the animation doesn't help (and the per-frame transform adds a small constant cost on top).

   **Mitigation:** if perf becomes a problem, the bigger fix is moving off `@observablehq/plot` to a canvas renderer, which is a much larger undertaking. Don't tie that to this PR.

3. **Visual mismatch between sliding data and snapping axis.** Acceptable at 1s intervals; might be jarring at faster rates (e.g., a 200ms sample interval where axis ticks would visibly snap every 5 ticks). Can be addressed by rendering axis ticks at 0.5× sample frequency so each tick lasts longer.

   **Mitigation:** if axis snap becomes an issue, manually re-render the axis on each RAF (Plot exposes the axis component). Skip until/unless reported.

**Three risks I'd dismiss:**

- **DPR / zoom changes**: handled naturally by `pxPerMs = chartWidth / visibleSpanMs` since chartWidth comes from `getBoundingClientRect()` each tick.
- **CSS transform compositing layer leaks**: SVG wrappers are well-trodden territory in chart libraries; no real risk.
- **Memory growth from running RAF**: bounded — single closure per plot; no allocation per tick if `shiftPx` is set via `style.transform = 'translateX(' + n + 'px)'`.

**Net call:** moderately brittle if rolled out without measurement; robust if you measure jitter first and accept the perf cost stays as-is. The math is the safe part. **Build it.**

## Spec

### Files affected

| File | Change |
|---|---|
| `frontend/app/view/sysinfo/sysinfo-plot.tsx` | Wrap data layer in masked div; add RAF loop with transform updates; reset on new sample |
| `frontend/app/view/sysinfo/sysinfo-plot.scss` (new) | Mask wrapper styling (`overflow: hidden`, fixed width) |
| `frontend/app/view/sysinfo/sysinfo-model.ts` | (No change — already exposes `intervalSecs`) |

### Behavior change

- Plot data slides smoothly from right to left between samples.
- On new sample arrival: imperceptible reset (transform → 0, plot re-renders with new data).
- Tooltip + hover behavior: unchanged (hover position is in screen space, transform doesn't affect it).

### Configuration

Add a single user-toggleable setting (default ON) in case the animation causes issues for some users:

```
sysinfo:smooth-animation = true | false   (block meta)
```

Off → today's behavior verbatim. On → animation enabled.

### Tests

Hard to unit-test animation behavior — rely on:
- A storybook-style demo page where the dev can see various `intervalMs` values rendering correctly.
- Smoke checklist:
  - 1s sample interval → smooth slide
  - Tab away for 30s, return → no glitch on resume
  - Window resize during animation → no glitch
  - Open 5 sysinfo panes simultaneously → no perceptible CPU bump in Task Manager

## Out of scope

- Migration off `@observablehq/plot` to a canvas/WebGL renderer (separate spec, much larger).
- Animation for non-time-series plots in the app (the sparkline mode in `sysinfo-plot.tsx` could use the same treatment but is rendered without axes — defer until requested).
- Predictive rendering (linear extrapolation past the last sample). Honest > smooth; we shouldn't draw what we don't know.
- Pause / resume controls in the UI.

## Estimated effort

~1–1.5 days:
- 0.5d: implement the RAF loop + transform plumbing + mask wrapper
- 0.25d: handle the 8 edge cases in the table above
- 0.25d: setting + meta plumbing
- 0.25d: smoke + tuning

## Decision needed before implementing

1. Measure actual sample jitter first (10 minutes of work — log inter-arrival times in `sysinfo-model.ts` and inspect the distribution). If P95 > 15% of interval, iterate on the spec before building.
2. Accept that this is visual polish, not a correctness fix. The right call if you want monitoring-tool-quality feel; the wrong call if perf is already tight.
