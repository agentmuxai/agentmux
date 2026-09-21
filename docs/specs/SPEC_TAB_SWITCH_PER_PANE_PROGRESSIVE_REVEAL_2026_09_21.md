# SPEC: Per-pane progressive reveal on tab switch, and a readiness signal that survives sustained streaming

**Date:** 2026-09-21
**Author:** Oozp
**Status:** draft — design, pre-implementation. §6's diagnostic logging is the one piece already shipped (this session, ahead of the rest of this spec, so Phase 0 data can start accumulating during review).
**Related:**
`frontend/app/store/tab-reveal.ts` (the gate this spec extends),
`frontend/app/util/settle-detector.ts` (the per-instance settle primitive it extends),
`docs/specs/SPEC_TAB_CONTENT_REVEAL_GATE.md` (original whole-tab gate design, #774),
`docs/specs/SPEC_PANE_BLOCK_STACK_MOUNT_FLICKER_2026_08_22.md` (added the leaf-scoped gate this spec wires up),
`docs/specs/SPEC_TAB_CLOSE_BUTTON_SELECT_FLASH_2026_08_25.md` §9 (made the whole-tab gate targeted),
`docs/specs/SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27.md` (the reveal-cascade cost inventory this spec's Phase 2/3 items build on),
`docs/analysis/ANALYSIS_CHROME_PARITY_TAB_SWITCH_LATENCY_2026_09_15.md` (the View Transition layer this spec sits underneath),
`docs/specs/TRACKING_TYPING_AND_TERMINAL_RESPONSIVENESS_2026_09_21.md` (umbrella — this is a new open item, not yet added there pending review)

---

## 1. Problem

Owner scenario (verbatim intent, 2026-09-21): an instance with **50 agents across 6 tabs, 8-9 agents per tab**, several busy at any moment. Two asks:

1. Switching to a tab should show panes **as each becomes ready**, not one long blank period followed by everything at once.
2. The existing 800ms hard cap on the reveal gate "doesn't sound healthy" — the goal is to **cover content until it's actually ready**, not force it open on a clock.

Both are really one problem: **today's reveal gate answers "is the tab ready?" with a single global signal (main-thread long-task quiet) that a continuously busy tab can permanently fail to satisfy** — and the only two things built on top of that signal are "wait" and "give up after 800ms." Neither request can be met by tuning that number. The signal itself is the wrong shape once panes are independently, indefinitely busy.

## 2. Current architecture (as of this session, verified in code)

- Tabs stay mounted (`content-visibility: hidden` while inactive, not `display:none`/unmount) — `workspace.tsx`.
- A switch is masked by a real `document.startViewTransition()` cross-fade, with a reveal gate underneath it (`tab-reveal.ts`).
- The **whole-tab gate** (`tabSwitching`/`holdRevealGate`/`scheduleRevealLift`) is a single boolean per tab switch. It reveals when either (a) 80ms pass with no `longtask` PerformanceObserver entries anywhere on the page, or (b) `MAX_GATE_MS` (800ms) elapses regardless. All panes in the tab reveal **atomically** — there is no per-pane granularity in this path today.
- A **leaf-scoped gate** (`gatingNodeIds`/`holdLeafRevealGate`/`scheduleLeafRevealLift`) already exists, generation-token-safe against overlapping operations, built for block-stack pushes (the "+" button, Quick Fork, Agent History) — **not currently used for tab-switch reveal at all.**
- Both gates use the same detection primitive (`scheduleOnSettle` in `settle-detector.ts`, or its hand-rolled twin `startDetector` in `tab-reveal.ts`): watch for a quiet window in the `longtask` PerformanceObserver stream, or give up at `maxMs`.

## 3. Diagnosis

**The quiet-window signal is a poor fit for a pane whose job is to keep producing long tasks.** An actively streaming agent's own token-dispatch work shows up as exactly the `longtask` entries the detector is waiting to see stop. For a pane like that, "80ms of quiet" isn't merely slow to arrive — it is not expected to arrive at all while the stream continues. The 800ms cap is not an arbitrary conservative number that could safely be raised or removed; it is the *only* thing preventing an indefinite blank tab in that case. Raising it makes the bad case worse (longer blank); removing it makes the bad case unbounded (a genuinely stuck-looking tab). The comment in `tab-reveal.ts` already names this explicitly: *"protects against perma-busy content (streaming agent, etc.) holding the gate open."*

Two more consequences of the signal being **global** rather than **per-pane**:

- **Cross-contamination.** The whole-tab gate watches `longtask` entries from *anywhere on the page* — a busy pane in a *different, currently-visible* part of the UI can hold open the gate for a tab switch that has nothing to do with it (and vice versa: a genuinely-still-loading pane can get force-revealed early because some unrelated task happened to go quiet).
- **All-or-nothing granularity.** Because there is one boolean for the whole tab, a tab with 8 quiet panes and 1 busy one waits (or gets capped) as a unit — the 8 ready panes get no benefit from being ready.

This connects to the main-thread-dispatch finding from the same investigation this spec follows from: `useAgentStream.ts` dispatches every streamed token into the store with no visibility gating, so a backgrounded pane's business is real main-thread cost, and — per this section — it is *also* real interference with any sibling reveal that happens to be gated on the same global quiet signal.

## 4. Proposed design

### 4.1 A per-pane readiness signal that isn't "nothing is happening"

Replace "main thread has been quiet for 80ms" with a **structural mount milestone**, scoped to one pane: its virtualizer has completed its first successful `measureElement` pass and the currently-visible rows have painted. This is a one-time event that happens early in a pane's life and, critically, **does not require the pane to stop producing output** — a streaming agent's *initial* layout settles in well under a second even though it will keep dispatching updates indefinitely afterward. Concretely, this is the same milestone `SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27.md` Phase 3 already identified as worth isolating ("trust the estimator for the first frame... then trigger a single measurement pass") — that phase was never shipped; this spec's Phase 1 (§7) is the trigger for finally building it, now with a consumer that needs it.

Ongoing streaming after that point is not a readiness concern — it is business-as-usual rendering, exactly like a foreground pane's ongoing updates are today.

### 4.2 Wire the existing leaf gate to tab-switch reveal

`holdLeafRevealGate(nodeId)` / `scheduleLeafRevealLift(nodeId, generation)` already do generation-token-safe per-node gating. On a tab switch, instead of (or in addition to — see §4.5) the single whole-tab `holdRevealGate`, hold a leaf gate per pane in the destination tab, and resolve each one independently on its own §4.1 signal. No new gate primitive needed — this is new call sites on an existing one.

### 4.3 Ordered, deliberate reveal — not a free-for-all

**This is the trap to avoid.** The whole-tab atomic gate exists *specifically* because panes popping in individually, in whatever order they happen to finish, was already shipped once and named a bug: `SPEC_TAB_CONTENT_REVEAL_GATE.md` calls it "a piecemeal mount cascade," and the fix was "the user perceives an atomic before/after transition" instead. Naively resolving 8 independent leaf gates in whatever order they settle reintroduces that same visual chaos in a new location.

The version that avoids it:

1. **Skeleton layout first, always.** The instant the tab becomes active, lay out every pane's slot at its correct size (reusing the existing row/pane size estimator — `agentPerfStore.recordEstimatorMeasurement`) so the grid never jumps as content fills in. This alone removes most of the "piecemeal" feel even before any content-ordering decision.
2. **Deliberate fill order**, not arrival order: reading order (top-left → bottom-right) or last-focused-pane-first. A pane that resolves its §4.1 signal out of turn still displays in its assigned slot in its assigned order — the *visual* sequence is authored, even though the underlying readiness events arrive in whatever order they arrive.
3. A small fixed stagger between reveals (on the order of one or two frames) so sequential fill reads as intentional loading rather than a flicker, even when several panes are actually ready at nearly the same instant.

### 4.4 Per-pane cap, not a bigger or smaller global one

Each leaf gate keeps its own safety-net cap (as it already does — `MAX_GATE_MS` on `holdLeafRevealGate`'s initial timer). Once gating is per-pane, this cap only ever blanks **one** pane's slot for a worst case, never the whole tab, and it fires far less often in practice because it's no longer waiting on unrelated siblings' business (§3). It can likely be tightened *and* fire less — both true at once, since the two are addressing different failures (a shorter cap bounds one pane's worst case better; per-pane scoping means far fewer switches ever need the cap at all).

**Explicitly not proposed:** removing the cap, or raising it. Per §3, an unbounded wait on "this pane has gone fully quiet" is not achievable for a continuously streaming pane by construction — no cap value fixes that. What actually gets you "covered until ready" is redefining *ready* (§4.1) so it's a milestone streaming can't indefinitely withhold.

### 4.5 The outer whole-tab transition still exists, narrowed

The `document.startViewTransition()` cross-fade + `fadeOutStartupSplash()` tie-in still needs *some* "tab is presentable" moment to key off, separate from any individual pane. Proposal: key it off skeleton-layout-complete (§4.3.1) rather than all-panes-settled — the outer cross-fade finishes once the grid shape is correct, and panes fill in underneath, inside their own slots, per §4.1–§4.3. This keeps the whole-tab gate for what it's actually good at (masking the initial structural pop) without making it gate on the same thing 8 independent panes are separately gating on.

## 5. What this does not change

- Virtualization internals, the streaming-markdown throttle, or egress fairness — unrelated layers, already covered in `TRACKING_TYPING_AND_TERMINAL_RESPONSIVENESS_2026_09_21.md`.
- The background-dispatch-cost problem (busy panes costing main-thread script time while hidden, regardless of paint) — real, related, but a different fix (gating *dispatch*, not *reveal*). Noted as a candidate Phase 4, not in scope here.
- First-visit-only cost (`SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27.md` Phase 4, snapshot pre-warm) — still a separate, never-shipped item.

## 6. Phase 0 — measurement (shipped this session, ahead of the rest of this spec)

There was **no existing telemetry** for how often the whole-tab gate actually hits `MAX_GATE_MS` versus settles naturally — that absence was itself the first finding. Shipped now, additive and low-risk:

- `settle-detector.ts`'s `scheduleOnSettle` callback now receives `{ hitCap: boolean; elapsedMs: number }` instead of nothing.
- `tab-reveal.ts` logs every gate resolution via the existing `[perf]` console convention (`frontend/perf/observers.ts`'s prefix), tagged by scope (`whole-tab` / `leaf node=<id>`), `source` (`settle` vs `orphaned-hold` — the latter meaning the paired schedule call never arrived at all, a bug signal distinct from ordinary busy-ness), `hitCap`, and elapsed ms. Queryable via `muxlog host '[perf] tab-reveal'`.
- Existing `tab-reveal.test.ts` suite (21 tests) passes unmodified against the new signature; `settle-detector`'s only other caller (`agent-view.tsx`) is unaffected — it ignores the callback argument.

**Next action before Phase 1 lands:** collect real `[perf] tab-reveal` data across normal multi-tab, multi-busy-agent usage, to confirm (not assume) that cap-hits cluster around busy tabs as §3 predicts, and to get a real distribution of elapsed-ms for the "settle" case — which sizes how aggressive §4.3's stagger needs to be.

## 7. Phased plan

| Phase | Scope | Depends on |
|---|---|---|
| 0 | Diagnostic logging (§6) | — shipped |
| 1 | Per-pane structural readiness signal (§4.1) — the never-shipped Phase 3 of `SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27.md`, now with a real consumer | 0's data, to validate the milestone is actually cheap/early relative to streaming duration |
| 2 | Wire leaf gates to tab-switch reveal, skeleton-first + ordered fill (§4.2, §4.3) | 1 |
| 3 | Narrow the outer whole-tab transition to skeleton-complete (§4.5); tune/shrink the per-pane cap using real Phase-0/2 data | 2 |
| 4 (candidate, separate spec) | Gate background dispatch itself, not just reveal — closes the loop with the main-thread-cost finding in §3 | independent of 1-3 |

## 8. Risks

- **Skeleton-height accuracy.** If the size estimator misses badly, panes will visibly resize as real content replaces the skeleton — same risk `SPEC_AGENT_PANE_TAB_SWITCH_PERF_2026_05_27.md` Phase 2 already flagged for its own skeleton use; same mitigation (tune from `agentPerfStore`'s existing estimator-accuracy data).
- **View Transition interaction.** `ANALYSIS_CHROME_PARITY_TAB_SWITCH_LATENCY_2026_09_15.md` already documents one real bug from the outer transition's timing being decoupled from the reveal gate's clock (content-visibility flips after the transition's update callback, not synchronously with it). Narrowing what the outer gate waits for (§4.5) changes that timing relationship again — needs the same re-verification, not an assumption that the existing fix still holds.
- **Ordering under rapid tab-switch spam.** The whole-tab gate's `startDetector` is explicitly idempotent for this (documented: "handles rapid Ctrl-Tab spam"); the leaf gate's generation-token design handles the analogous per-node case. Both primitives already have this property — the risk is wiring §4.2/§4.3's new orchestration on top without breaking it, not the primitives themselves.
- **Test coverage.** Needs new tests at the orchestration layer (which pane reveals in which order, under a mix of fast/slow/never-settling panes) — the existing 21 tests cover the two gate primitives in isolation, not a multi-pane tab-switch scenario.

## 9. Open questions

1. Reading-order vs last-focused-first for §4.3.2 — no strong evidence either way yet; worth an actual UX call, not an engineering default.
2. Should §4.5's outer transition wait for skeleton-complete, or for the *first* pane's real content (whichever resolves first)? The former is simpler and matches "the grid shape is right"; the latter might feel faster since something real appears the instant the cross-fade finishes. Undecided — revisit with Phase 0/2 data in hand.
