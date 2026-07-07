# Report: agent pane blank-load period + brain-logo loading indicator

**Status:** Investigation complete — no code changed. Written to inform a
design before implementation.
**Author:** AgentX
**Date:** 2026-07-04

## Ask

When an agent pane with a lot of content is loading, it's blank for a bit
before the transcript appears. Show the existing pulsating brain logo
immediately when the pane mounts, then fade it out once the UI has actually
painted.

## Where the blank period actually comes from

There are two independent causes, and they stack for content-heavy panes.

### 1. `block.tsx` renders literally nothing until the pane object resolves

`frontend/app/block/block.tsx:299` — the `ready` memo:
```ts
const ready = createMemo(() => !loading() && !isBlank(props.nodeModel.blockId) && blockData() != null && viewModel() != null);
```
Until this flips true, `<Show when={ready()}>` (around line 308) renders
**nothing at all** — no fallback. Once `ready()` is true, the view component
loads behind a `<Suspense fallback={<CenteredDiv>Loading...</CenteredDiv>}>`
(lines 128 and 263) — plain text, no spinner, no brand.

This is stage-one blank: waiting on the block's WaveObject metadata +
`makeViewModel()`. Usually fast, but it's still a zero-indicator window today.

### 2. Content-heavy history replay is a single synchronous blocking parse

`frontend/app/view/agent/hooks/useHistoryPagination.ts`, `onMount` (~line 181):
- Dispatches `InitStart` immediately, then asynchronously tries a v2 snapshot
  restore, falling back to v1, falling back to raw NDJSON replay.
- `RESTORE_WINDOW_LINES = 5_000` (line 116) — on restore, up to 5000 trailing
  NDJSON lines are fetched and parsed via `parseHistoryLines()`
  (`frontend/app/view/agent/parseHistoryLines.ts`, 77 lines) — a **pure,
  synchronous, non-yielding** loop: per line, `JSON.parse` → `translator
  .translate()` → `parser.parseLine()`, building a `DocumentNode[]`.
- The result is bulk-dispatched in one `batch(...)` call (lines 286/348/410),
  which drives the reducer + `AgentDocumentVirtualList`
  (`frontend/app/view/agent/virtualization/AgentDocumentVirtualList.tsx`) to
  mount/measure the initially-visible rows.

`docs/specs/SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27.md` already measured
this exact path for tab-switching and found the dominant cost (500-600ms for
heavy sessions) is **browser-side layout/paint of the resulting DOM tree**,
not any single JS function — markdown memoization, virtualizer
`measureElement`, and partition recompute were each profiled and ruled out as
the bottleneck. So "a lot of content" → more NDJSON lines parsed synchronously
in step 2, and more DOM nodes for the browser to lay out in step 3 — both
scale with conversation size, matching the report exactly.

### The existing reveal-gate doesn't cover this case

AgentMux already has a hide-until-settled mechanism for a *similar* problem —
`frontend/app/store/tab-reveal.ts` + `frontend/app/workspace/workspace.tsx` —
used for whole-**tab** switches: `holdRevealGate()` hides the active tab
(`visibility:hidden; opacity:0`) before an async tab operation, and
`scheduleRevealLift()` (a `PerformanceObserver` watching for Long Tasks, with
an 80ms settle window and an 800ms hard cap) reveals it with a 120ms
opacity fade once things quiet down.

Confirmed via grep: `holdRevealGate`/`scheduleRevealLift` have **no other
callers**. They only fire from `createTab()` and `setActiveTab()` in
`frontend/app/store/global.ts`. **Opening a new agent pane inside an
already-visible tab — split, view-swap, swarm sub-agent click, tear-off
landing — has no gating and no indicator at all.** This is very likely the
specific scenario behind the report: not "switching tabs is blank" (already
gated), but "a new/growing pane inside a tab I'm already looking at is
blank."

`docs/specs/SPEC_TAB_CONTENT_REVEAL_GATE.md` (the reveal gate's original
spec, issue #774) explicitly lists both "fade-in transition on reveal" and
"per-block skeletons" as **out of scope**, filed for later. This is that
later.

## The brain logo: exists, but not as a reusable component

`index.html` (lines 20-61, 82-130) has the pulsating brain today — as
**static inline SVG + CSS baked into the HTML shell**, not a SolidJS
component:
```css
#startup-loading svg {
    width: 96px; height: 96px; opacity: 0.9;
    animation: startup-pulse 1.8s ease-in-out infinite;
}
@keyframes startup-pulse {
    0%, 100% { opacity: 0.9; transform: scale(1) translateZ(0); }
    50%       { opacity: 0.65; transform: scale(0.97) translateZ(0); }
}
```
Same SVG is also saved standalone at
`frontend/logos/agentmux-logo-brain-alternate.svg`.

`frontend/app/init/startup-splash.ts` controls it:
`fadeOutStartupSplash()` adds a `.fading` class, waits for `transitionend`
(or a 320ms timeout), then **permanently removes the DOM node**. It's called
from `tab-reveal.ts`'s `liftGate()` — every tab-reveal gate lift calls it, but
it's a no-op after the very first call (element already gone).

**Net: the asset is real and already shipped, but it's wired as a
non-reusable, one-shot, singleton overlay.** To use it per-pane, the SVG +
`@keyframes startup-pulse` need to become an actual component (something like
`<BrainSpinner/>`) — not a second call into `startup-splash.ts`, which cannot
be re-armed.

## A clean, already-existing hook point for "fade out now"

`frontend/app/view/agent/state.ts:87,179` — `initPhaseAtom`, a
`SignalPair<InitPhase>` initialized to `{ kind: "InitPending" }`, flipped to
`InitReady` (success) or `InitFailed` (error) once the initial history load
resolves.

`useHistoryPagination.ts:74` — `onHistoryReady?: () => void`, already fired
exactly once (lines 319, 360, 393, 427, 445) the moment history load
finishes, success **or** failure (fail-open). Today it's consumed only to
gate the new-message enter animation ("history rows shouldn't animate on
open/restore").

This is the fade-out trigger, already built and already firing at the right
moment — currently unused for any visual loading-state purpose.

## What would actually need to be built

1. **Extract the brain into a real component** (SVG + `startup-pulse`
   keyframes) — `frontend/app/element/` seems like the natural home, matching
   where other small reusable visual elements live.
2. **A per-pane (not global-singleton) show/fade signal.** The existing
   `tabSwitching` signal in `tab-reveal.ts` is a single module-level boolean,
   not keyed by block/pane id — it can't represent "this specific pane is
   still settling" independent of others. Needs either a small
   `Map<blockId, boolean>`-backed variant, or a signal instantiated per pane
   instance (e.g. owned by the agent view's own model, alongside
   `initPhaseAtom`).
3. **Wire it into `block.tsx` around the `ready` memo (line 299) and the
   `Suspense` fallback (lines 128/263)** — that's the true "nothing rendered
   yet" window, before the agent view (and therefore `initPhaseAtom`) even
   exists. The brain needs to show from pane-mount, not from
   agent-view-mount, to cover stage-one blank too.
4. **Fade out on `InitReady`/`InitFailed`** (i.e. on `onHistoryReady` firing),
   mirroring the existing `opacity 120-200ms ease-out` idiom already used at
   both the tab-reveal and startup-splash layers — same visual language, new
   scope.
5. **Respect `prefersReducedMotion()`** (already imported/used in
   `workspace.tsx`) — skip the animation/transition, show/hide instantly.
6. Decide whether this should show unconditionally on every pane mount, or
   only kick in past some minimum delay (e.g. don't flash the brain for a
   pane that resolves in 50ms) — the tab-reveal gate's `SETTLE_MS`/
   `MAX_GATE_MS` pattern (80ms / 800ms) is a reasonable template: show the
   brain only if settling takes longer than a short grace period, so fast
   panes don't flicker a logo in and immediately out.

## Prior art / related specs (for context, not yet implemented against)

- `docs/specs/SPEC_TAB_CONTENT_REVEAL_GATE.md` — origin of the tab-level gate;
  explicitly scoped out fade-in and per-block skeletons.
- `docs/specs/SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27.md` — measured the
  same 500-600ms paint cost this report describes, proposed (but per that
  doc, did not confirm shipped) lazy tool-block hydration and a snapshot
  pre-warm, either of which would reduce how long the brain needs to stay
  visible rather than just covering for it.
- No spec, issue, or PR proposes using the brain logo as a per-pane loading
  indicator specifically — this is new scope, not a revival of an abandoned
  attempt.

## Suggested next step

Small, scoped implementation: extract `<BrainSpinner/>`, add a per-pane
show/fade signal gated on `ready` (block.tsx) union `initPhaseAtom !==
"InitReady"/"InitFailed"` (agent view), fade out on whichever resolves last.
Should not require touching `useHistoryPagination.ts`'s parse/dispatch logic
itself — this is a visual cover for the existing cost, not a fix to the cost.
Reducing the 500-600ms paint cost itself (lazy tool-block hydration, snapshot
pre-warm) is a separate, larger effort already scoped in
`SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27.md` and out of scope here.
