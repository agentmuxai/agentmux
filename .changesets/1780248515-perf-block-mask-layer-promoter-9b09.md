---
type: patch
---

perf(block): replace `.block-mask` `backdrop-filter` with `will-change: transform`

`.block-mask` carries the per-pane focus border and used
`backdrop-filter: blur(0.1px)` as a layer-promotion hack to ensure it
composites above xterm's `.xterm-viewport` GPU-scroll layer.
`backdrop-filter` is the most expensive layer-promoter in Chromium —
even at 0.1 px it creates a backdrop layer that samples and blurs the
layers below every frame.

Linux Chromium-Ozone-Wayland measurements (10 panes, idle, captured via
`scripts/capture-trace.cjs` + `Profiler.start`):

- `PaintArtifactCompositor::Update` was ~48 ms per `BeginMainFrame`.
- `BeginMainFrame` fires at the sysinfo cadence (~1 Hz) and dominates the
  frame budget — total `BeginMainFrame` ~92 ms, well above the 16 ms
  budget. Renderer effective frame rate sat near 1 fps.
- V8 was idle 91 % of wall time; the bottleneck was 100 % compositor.

This swap replaces the backdrop sample path with `will-change: transform`
— a cheap GPU-layer-promotion hint that preserves the load-bearing
constraint (focus border ordered above xterm's scroll layer) without
backdrop sampling, blur, or filter work. Empirically takes
`BeginMainFrame` from ~92 ms to ~80 ms (~13 % per-frame reduction);
the rest is split across other backdrop-filter sites (modal, magnify
overlay, conn-status overlay, code-block copy-button) and Wayland-side
compositor cost.

The squash/reorder concern in the original comment is a non-issue: the
explicit `z-index: var(--zindex-block-mask-inner)` already on the rule
sets paint order within the parent stacking context, and Chromium's
squashing groups preserve relative z-order across squashed siblings.

Not a complete fix for the Linux terminal-typing latency
(spec: `aa6f56b9 docs(spec): terminal flow control`), but the largest
single steady-state per-pane compositor cost in the layer tree —
applied to *every* pane, always — and an obvious win that unblocks the
next steps (CEF launch-flag pass, audit of the conditional backdrop
sites, GPU channel flags).
