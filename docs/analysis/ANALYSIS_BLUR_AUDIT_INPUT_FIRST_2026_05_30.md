# ANALYSIS: `backdrop-filter: blur` audit — input-first Phase 0.3

**Date:** 2026-05-30
**Author:** AgentY
**Tracks:** [discussion #1161](https://github.com/agentmuxai/agentmux/discussions/1161) Phase 0 "cheap verified wins" → *blur audit*
**Spec:** [`SPEC_INPUT_RESPONSIVENESS_TERMINAL_AND_AGENT_2026_05_29.md`](../specs/SPEC_INPUT_RESPONSIVENESS_TERMINAL_AND_AGENT_2026_05_29.md)

---

## TL;DR

**`backdrop-filter: blur` is NOT an input-first hotspot. No change required.** Every blur in the
frontend is either (a) conditionally rendered behind a Solid `<Show>` — so it is *absent from the
DOM* when inactive, (b) gated on a transient interaction state (`:hover`, drag, resize, magnify,
open modal, disconnected pane), or (c) the single always-on `.block-mask { blur(0.1px) }`, which is
a deliberate, documented compositing hack whose cost is layer-promotion, not Gaussian sampling.

There is **no backdrop blur active over a pane during steady-state typing**, so blur cannot be
taxing the keystroke frame. This closes the "blur audit" cheap-win: the verified outcome is
*reviewed, nothing to fix.*

## Why blur was a suspect

`backdrop-filter: blur(r)` is one of the most expensive CSS effects: the compositor must snapshot
the backdrop and run a separable Gaussian of radius `r` **every frame the element is composited
while its backdrop changes**. An always-on blur sitting over a live terminal/agent pane would re-run
that shader on every PTY paint and every keystroke echo — directly competing with the ≤16 ms
keystroke budget (spec §4 rule 1). So the question worth answering is narrow: *is any blur live
over a pane while the user is typing into it?*

## Every site, classified

| # | File:line | Selector | Radius | Active when | DOM-present when idle? | Verdict |
|---|-----------|----------|--------|-------------|------------------------|---------|
| 1 | `block/block.scss:455` | `.block-mask` | `0.1px` | **always** (per pane) | **yes** | **Keep — intentional.** Load-bearing: forces the focus-ring layer to composite above xterm's `overflow-y:scroll` GPU layer. 0.1px ⇒ no real blur work, just layer promotion (see the in-file comment). |
| 2 | `block/block.scss:336` | `.connstatus-overlay` | `50px` | pane **disconnected** | no — `<Show when={connStatus()?.status !== "connected" …}>` (`blockframe.tsx:603`) | Keep. Absent while connected; the user is not typing into a disconnected pane. *(Note: 50px is a larger radius than the effect needs — see "Optional".)* |
| 3 | `block/block.scss:313` | `.block.ephemeral` | `var(--magnified-block-blur)` = 10px | block **ephemeral/magnified** | no — only on the ephemeral/magnified block | Keep. Magnify view, not steady-state tiling. |
| 4 | `block/block.scss:405` | `.connstatus-error .copy-button` | `8px` | **hover** inside the error overlay | no (inside #2's `<Show>` + `:hover`) | Keep. Doubly gated. |
| 5 | `layout/lib/tilelayout.scss:127` | `.magnified-node-backdrop`, `.ephemeral-node-backdrop` | `var(--block-blur)` = 2px | node **magnified/ephemeral** | no — `<Show when={showMagnifiedBackdrop()}>` / `showEphemeralBackdrop()` (`TileLayout.*.tsx`) | Keep. Conditionally rendered. |
| 6 | `layout/lib/tilelayout.scss:89` | `.tile-node.resizing` | `8px` | **resize drag** | class only present mid-resize | Keep. Transient gesture. |
| 7 | `layout/lib/tilelayout.scss:84` | `.tile-node.dragging` | `8px` (`filter`, not `backdrop-filter`) | **DnD drag** | class only present mid-drag | Keep. Transient; blurs the element itself, no backdrop sampling. |
| 8 | `block/pane-size-badge.scss:24` | size badge | `2px` | **resize** (badge shown) | no | Keep. Transient + tiny element. |
| 9 | `element/markdown.scss:154` | `.codeblock-actions` | `8px` | **hover** over a code block | no (`visibility:hidden` until hover) | Keep. Hover-gated, tiny. |
| 10 | `element/modal.scss:82` | `.modal-backdrop` | `8px` | a **modal is open** | no — backdrop mounts with the modal | Keep. Transient; while a modal owns focus the panes behind are not being typed into. |

## Conclusion

- **Steady-state pane typing involves exactly one backdrop-filter element: `.block-mask` at
  `blur(0.1px)`**, which is intentional and effectively free (it promotes a compositing layer; it
  does not do meaningful blur work). Removing it would *regress* the focus ring on terminal panes
  (documented at `block.scss:480-495`), so it must stay.
- Every other blur is `<Show>`-gated or interaction-transient (disconnect / magnify / resize / drag
  / hover / modal) and **not present in the DOM during normal typing**.
- **Verdict: blur is cleared as an input-first cost. No code change is warranted.** Future input-path
  work should not spend time here; if a regression ever points at blur, the first check is whether a
  new always-mounted (`<Show>`-less) blurred element was introduced over a pane.

## Optional (deliberately not done)

`.connstatus-overlay` uses `blur(50px)`. A 50px separable Gaussian is a larger sampling radius than
the frosted-glass effect needs — `≤16px` is visually indistinguishable at the overlay's opacity.
Reducing it would shave compositor cost, **but only while a pane is disconnected** (the upside is
negligible for input responsiveness) and it would alter the disconnect overlay's appearance. Left
unchanged to avoid a no-benefit visual change; recorded here only so the larger-than-necessary
radius is a known, intentional choice rather than an oversight.

## Method

`rg backdrop-filter|filter:\s*blur` across `frontend/`, then for each hit traced the owning
component's render guard (Solid `<Show>` / class-toggle) to determine DOM presence during idle
typing. CSS-variable defaults resolved: `--block-blur: 2px` (`tilelayout.scss:118`),
`--magnified-block-blur: 10px` (`block.scss:309`).
