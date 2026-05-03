# Frontend Reducer Implementation Plan

**Date:** 2026-05-03
**Status:** Draft. Each PR section needs a quick "go" before its code lands.
**Reads-this-first:**
- `frontend-reducer-architecture-2026-05-03.md` — the spec roadmap (slices #1–#8)
- `frontend-reducer-conventions-2026-05-03.md` — shape every slice follows
- `agent-pane-document-reducer-2026-05-03.md` — slice #1, shipped as PR #681

## Status snapshot

| Slice | Status | PR | Notes |
|---|---|---|---|
| #1 agent-document | **Shipped** | #681 (0.33.618) | Bug fix; established the pattern |
| #2 conventions | **Spec approved** | — (doc only) | Now drives all later slices |
| #3 source-tagging + global event log | **Spec needs rewrite** | — | Descoped per conventions Q1+Q4 |
| #4 agent-pane-state | Spec pending | — | Highest value, no deps |
| #5 frontend-layout | Spec pending | — | Depends on srv E.4 |
| #6 launcher-event-reducer convergence | Spec pending | — | Cleanup; validates conventions retroactively |
| #7 tab-state | Spec pending | — | Mirror slice |
| #8 pane-tree | **Deferred** | — | Wait for srv E.4.B |

## Sequencing rationale

Dependencies:

```
#1 (shipped) ──────────┐
                       │
#2 (approved) ─────────┼──→ #4 ──→ #6 ──→ #3 ──→ #7 ──→ #5 ──→ (#8 — wait)
                       │       \                           ↑
srv E.4 (in flight) ───┴───────────────────────────────────┘
```

The order is chosen for:

1. **#4 first** — per-pane slice, no upstream dependency, immediate value (cleans up the other multi-writer agent atoms before they bite).
2. **#6 second** — convergence of `launcher-event-reducer.ts` to the conventions. Small. Validates that the conventions actually fit a real pre-existing reducer; surfaces gaps before they multiply.
3. **#3 third** — source-tagging + global event log. Now small (per Q1+Q4). Valuable as a debuggability feature; lights up the audit story for #1, #4, #6.
4. **#7 fourth** — tab-state mirror slice. First pure mirror; exercises §6 echo-loop guard end-to-end.
5. **#5 fifth** — layout mirror, depends on srv E.4.A landing first. If srv E.4.A delays, swap with #7 or pull forward a smaller mirror.
6. **#8 deferred** — needs srv E.4.B to settle the rootnode path-representation question.

## Concrete PRs

Each section below specifies what the PR ships. Effort estimates assume contiguous focus, no surprise blockers.

---

### PR-A — Slice #4: agent-pane-state-reducer

**Goal:** bundle the remaining per-pane agent atoms (`streamingState`, `sessionStats`, `currentTool`, `turnTokens`, `turnActive`, `stopping`, `pendingMessages`) into one slot-keyed reducer that enforces cross-atom invariants.

**Why now:** Same architectural problem as agent-document but smaller blast radius. Multiple writers across `useAgentStream` (lifecycle), `agent-model.ts` (composer/decision flow), and possibly slash commands. No single bug observed yet, but the cohesion `turnActive ↔ streamingState.active ↔ stoppingAtom` is fragile and a future race waits to land.

**Scope:**

| File | Change |
|---|---|
| `frontend/app/store/agent-pane-state/types.ts` | NEW — `AgentPaneState`, `Command`, `Event` |
| `frontend/app/store/agent-pane-state/reducer.ts` | NEW — pure update() |
| `frontend/app/store/agent-pane-state/reducer.test.ts` | NEW — table-driven tests |
| `frontend/app/store/agent-pane-state-store.ts` | NEW — slot map + dispatch (mirrors agent-document-store) |
| `frontend/app/view/agent/state.ts` | Atoms become projections; factory still creates them but readers don't change |
| `frontend/app/view/agent/agent-view.tsx` | Register slot synchronously alongside agent-document slot |
| `frontend/app/view/agent/useAgentStream.ts` | All `setStreaming`, `setSessionStats`, `setCurrentTool`, `setTurnTokens`, `setTurnActive` → dispatch |
| `frontend/app/view/agent/agent-model.ts` | All composer/decision-flow writes to these atoms → dispatch |

**Commands** (proposed):
- `SessionLifecycle { action: "subscribe-up" \| "subscribe-down" }` — toggles `streamingState.active`
- `TurnStart { at }` — `turnActive = true`
- `TurnEnd { stats? }` — `turnActive = false`, optional stats merge
- `RequestStop { at }` — `stopping = true`
- `StopApplied { at }` — clears `stopping`
- `ToolStart { name }` / `ToolEnd` — `currentTool` setter
- `TokenDelta { input?, output? }` — `turnTokens` increment
- `PendingMessageQueued { id, text }` / `PendingMessageAccepted { id }` — pending FIFO

**Invariants the reducer enforces** (the value-add over scattered atoms):
- `turnActive` cannot be true while `streamingState.active` is false
- `stopping` clears automatically on any `TurnEnd`
- `currentTool` and `turnTokens` clear on `TurnEnd`
- `pendingMessages` is a strict FIFO; accepting an id removes it (idempotent for unknown ids)

**Tests:** ~20 reducer tests (one per invariant, plus happy paths).

**Effort:** ~1.5 days. Mostly mechanical migration; the design work is small.

**Risk:** medium — touches the agent view's lifecycle code which is the most-watched code path in the app. Mitigations: ship behind a fresh patch version; smoke-test in `task dev` before merging; keep the reducer's behavior semantically identical to today's atom mutations (only tighten the cohesion).

**Open question:** does `pendingMessagesAtom` belong with the lifecycle atoms or in its own slice? Today it's coupled to the composer + agent-message-accepted event handler; I lean toward keeping it here but could justify a separate composer slice.

---

### PR-B — Slice #6: launcher-event-reducer convergence

**Goal:** harmonize the existing `frontend/app/store/launcher-event-reducer.ts` into the conventions established in spec #2.

**Why now:** Validates the conventions retroactively. If something in the conventions doesn't fit the launcher reducer, that's a sign the conventions need a tweak — better to learn this with a small known-good reducer than mid-way through a bigger slice.

**Scope:**

| File | Change |
|---|---|
| `frontend/app/store/launcher-event-reducer.ts` | Extract pure `update()` to `launcher-event/reducer.ts`; current file becomes the dispatch + slot layer |
| `frontend/app/store/launcher-event/types.ts` | NEW — extracted Command/Event types (from existing inline types) |
| `frontend/app/store/launcher-event/reducer.test.ts` | NEW — backfill table-driven tests for the existing arms |
| `frontend/util/launcher-events.ts` | Wire to the new dispatch (no logic change) |

**No behavior change.** Pure refactor + test backfill. Echo-loop guard (`applyingRemote`) stays exactly as is — it was the model for the convention.

**Effort:** 0.5 day.

**Risk:** low — refactor of code that already works.

**Decision point:** does the launcher-event-reducer's seed mechanism (`seedKnownEntriesFromSnapshot`) generalize to a conventions feature, or stay launcher-specific? Lean toward launcher-specific until a second slice needs it.

---

### PR-C — Slice #3 (revised): source-tagging + global event log

**Goal:** add a small cross-slice helper that tracks WHO initiated each command (user, agent:<id>, system) and aggregates events into a single ring buffer for the diagnostics panel.

**Why now:** Cheap but high-value once enough slices exist. Lights up the audit story — you can ask "what did agent X do in the last hour?" and get a real answer.

**Why this is much smaller than the original spec #3:**
Per conventions Q1+Q4, there's no command bus chokepoint to build. The chokepoint is each slice's `dispatch`. This PR adds two things on top of dispatch:

1. **`CommandSource` type + threading.** Every `dispatch(key, command)` callsite gets an optional `source: CommandSource` parameter. Defaults to `"system"`. Slash commands set `"user"`. Future agent-API surface sets `{ kind: "agent", agentId }`.

2. **Global event log.** A single ring buffer (~500 entries) collects `{slice, key, command, source, events, at}` records. Diagnostics panel surfaces it.

**Scope:**

| File | Change |
|---|---|
| `frontend/app/store/command-source.ts` | NEW — `CommandSource` type + global event log + helpers |
| `frontend/app/store/agent-document-store.ts` | Accept optional `source` parameter on dispatch; record to global log |
| `frontend/app/store/agent-pane-state-store.ts` | Same |
| `frontend/app/store/launcher-event-reducer.ts` | Same |
| All dispatch callsites | Pass `source` where known; default unchanged |

**Effort:** 1 day.

**Risk:** low. Backward-compatible — `source` is optional with a sensible default.

**Decision point:** does the global event log live in memory only, or also write to a file (debug.log)? Default in spec: memory only; opt-in file flag.

---

### PR-D — Slice #7: tab-state-reducer

**Goal:** frontend mirror of srv tab state (active tab, tab order, per-tab metadata). First pure mirror slice; validates §6 echo-loop guard.

**Why now:** After #6 has converged the existing mirror, this is the second mirror — pattern is proven. Tabs are a high-frequency mutation site (new tabs, drags, closes) so the audit log immediately gives value.

**Scope:**

| File | Change |
|---|---|
| `frontend/app/store/tab-state/types.ts` | NEW — TabState, Command (local), Event |
| `frontend/app/store/tab-state/reducer.ts` | NEW — pure update() |
| `frontend/app/store/tab-state/reducer.test.ts` | NEW |
| `frontend/app/store/tab-state-store.ts` | NEW — dispatch + wstore subscription + echo-loop guard |
| `frontend/app/store/global.ts` | Existing tab atoms become projections; factories untouched |
| `frontend/app/tab/*.tsx` | Local UI commands dispatch through the store (no longer write atoms directly) |

**Mirror semantics:**
- Wstore objects update → `applyRemoteEvent({ type: 'TabUpdated', tab })` → dispatch echoes to local atoms
- Local UI commands (e.g. user clicks a tab) → dispatch → emits the command upstream via existing RPC; the echo-loop guard prevents the resulting wstore event from re-emitting

**Effort:** ~3 days (more careful than per-pane slices because mirror semantics are subtler).

**Risk:** medium — touches global tab atoms used in many places. Mitigations: read-side projection means consumers don't change; only mutation sites move.

**Decision point:** does `pendingbackendactions` (per-tab queue of pending layout ops; see srv `LayoutState`) belong here or in slice #5? Lean toward keeping in this slice since it's per-tab.

---

### PR-E — Slice #5: frontend-layout-reducer

**Goal:** frontend mirror of srv Phase E.4 layout reducer (focused node, magnified node, leaf order). Coordinates with srv-side `SPEC_PHASE_E4_LAYOUT_REDUCER_2026-05-01.md`.

**Why this comes after #7:** layout mirror semantics are the same as tab mirror semantics, but layout has thornier per-pane state (focused/magnified vary per-tab). Better to bake the mirror pattern on tab first, then extend to layout.

**Blocking dependency:** srv E.4.A (focused/magnified PR) must ship first. If srv E.4.A is delayed, this PR slot can be filled by another non-blocking slice.

**Scope:**

| File | Change |
|---|---|
| `frontend/app/store/layout-state/types.ts` | NEW |
| `frontend/app/store/layout-state/reducer.ts` | NEW — `SetFocusedNode`, `SetMagnifiedNode`, `RebuildLeafOrder` |
| `frontend/app/store/layout-state/reducer.test.ts` | NEW |
| `frontend/app/store/layout-state-store.ts` | NEW — dispatch + wstore sub + echo-loop guard |
| `frontend/layout/*.ts` | Existing focus/magnify atoms become projections |

**Out of scope** (pending srv E.4.B): `rootnode` path-representation. The current `rootnode` JSON-blob path stays as-is until srv decides on the path representation.

**Effort:** ~3 days (assumes srv E.4.A is in main).

**Risk:** medium — layout state is the most-touched per-frame state in the app. Mirror pattern means reads don't change, but bugs would manifest as visual glitches.

**Decision point:** none new (srv E.4 spec already settled the granularity question for the srv side).

---

### Slice #8 — pane-tree-reducer (deferred)

Wait for srv E.4.B (path representation for rootnode). Until that lands, frontend keeps shipping rootnode mutations directly to the wstore meta path.

---

## Cross-cutting work

Two things outside any specific slice that should land alongside the work above:

### Diagnostics panel surface

After PR-C ships, build a simple diagnostics panel that shows the global event log. Probably a new tab in the existing diagnostics surface, or a slash command (`/events`) that opens a modal. **Effort: 0.5 day.** Punt to whenever bandwidth allows after PR-C.

### Documentation

Each slice's PR includes a short README in its directory linking back to the conventions doc + the slice's own decisions. After all slices land, write a single `frontend/app/store/README.md` summarizing the architecture for new contributors. **Effort: 1 hour at the end.**

## Total effort + calendar

| PR | Effort | Cumulative |
|---|---|---|
| PR-A (#4 agent-pane-state) | 1.5d | 1.5d |
| PR-B (#6 launcher convergence) | 0.5d | 2d |
| PR-C (#3 source-tagging + log) | 1d | 3d |
| PR-D (#7 tab-state) | 3d | 6d |
| PR-E (#5 layout) | 3d (after srv E.4.A) | 9d |
| Diagnostics panel | 0.5d | 9.5d |
| README | 0.1d | ~10d |

**~10 working days of focused work** to converge most of the frontend mutation surface. Some PRs can parallelize if more than one person works the queue (PR-B and PR-C don't depend on each other; PR-D and PR-E are independent of the others once #6 is in).

## Decision points (one-line summary)

1. **PR-A**: pendingMessages in the agent-pane-state slice or its own slice? (lean: same slice)
2. **PR-B**: generalize the seed mechanism, or keep it launcher-specific? (lean: launcher-specific)
3. **PR-C**: global event log in memory only, or also written to debug.log? (lean: memory only, opt-in file)
4. **PR-D**: pendingbackendactions in tab-state slice or layout slice? (lean: tab-state)
5. **Slice #5 timing**: do we hold for srv E.4.A or rearrange? (depends on srv schedule)

## What this plan does NOT commit to

- Replacing the wstore subscription mechanism. wstore stays — it's the inbound event channel for mirror slices.
- Component-level state migration. Per-component `createSignal` for UI chrome (open/closed, hover, etc.) stays as-is.
- Backward-compatibility shims. Each slice migration is a clean cutover; consumers reading projection atoms don't change, but writers must move to dispatch in the same PR.
- A "frontend redux store" object. Slices remain independent modules.

## Stop conditions

If at any point in the plan it becomes clear that an unforeseen architectural concern requires a re-spec, **pause and write a retro before continuing**. Better to absorb the new constraint into the conventions than to ship two slices that disagree on a fundamental.
