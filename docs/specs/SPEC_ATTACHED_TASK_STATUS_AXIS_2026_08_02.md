# Spec: an orthogonal "attached task" status axis, sibling to `TurnPhase`

**Status:** active — reducer slice + dispatch wiring + hasRunningPromotedTool shipped (PR #2489); the AgentFooter render from #2489 was deliberately reverted 2026-08-10 (dock running-row is the indicator); §6 item 4 (Swarm pane / muxspect surfacing) not started. Verified 2026-08-10.
**Author:** Agent A (agenta-07017)
**Builds on:**
- `docs/specs/REPORT_LONGRUNNING_TOOLCALL_DOCK_VISIBILITY_2026_07_16.md` (Agent2) — original analysis, §6/§7 design direction.
- `docs/specs/REPORT_LONGRUNNING_TOOLCALL_AUTODETECT_STATUS_2026_07_26.md` (Agent3) — verification pass + synthesized 5-step recommendation (§3), open questions resolved (§4).
- `docs/retro/retro-persistent-agent-working-status-stuck-2026-07-16.md` — the Agent1 incident this whole line of work generalizes from.

This document does **not** re-derive the diagnosis both prior reports already did well — it starts from their conclusion ("turn-phase and attached-process-liveness are orthogonal axes, and the reducer only models one") and answers the one question they left as a to-be-decided implementation detail: **what does "two-axis pane status" actually look like in `agent-pane-state`'s types/reducer**, since a naive reading of "add a TurnPhase state for a long-running attached process" would violate an invariant the reducer's own docs already state explicitly.

## 1. What's already shipped (re-verified against `main` today, since the two prior reports)

Both prior reports are ~1-2 weeks old. Re-checked directly against the current tree:

- **Step 1 (duration-threshold tool-call promotion) is DONE.** `frontend/app/view/agent/activity/tool-adapter.ts` exists: `TOOL_PROMOTION_MS = 30_000`, `toolActivities()` promotes any Bash `ToolNode` still running (or that ran) past the threshold into a `PinnedActivity` of kind `"tool"`, independent of command text — exactly report §4.2's resolved recommendation (duration-first, not regex-first).
- **The `AgentWorkingRow` suppression half of step 3 is DONE.** `hasRunningPromotedTool()` in the same file is the exact signal report §4.3 called for ("suppress the working row's own tool text once the dock already shows it") — already exported, ready for a consumer.
- **Not yet wired**: nothing currently *calls* `hasRunningPromotedTool` from `AgentFooter.tsx` (checked: no reference to it outside `tool-adapter.ts`/its test). The signal exists; the consumer doesn't yet.
- **Still open, confirmed today:**
  - `BashParams` (`frontend/app/view/agent/types.ts`) still has no `run_in_background` field — step 2 (backgrounded-task threading) is unbuilt.
  - `AgentProcessRegistry`'s `started_at_ms` is still hardcoded to `0` on Windows (`agentmux-srv/src/backend/process_tracker/windows.rs:198`) — no process-age data exists yet, so an OS-process-duration heuristic (the fallback for detached work with no self-declared signal at all — a bare `nohup foo &`, no MCP Shell tool, no `run_in_background`) has no data to key on.
  - No reducer-level field exists yet for "is there a live attached task independent of the turn." This is this spec's subject.

So the remaining gap is narrower than either prior report scoped it: the foreground case (step 1) is handled; what's missing is representing a task that outlives its *initiating tool call* (the Agent1 shape: a `run_in_background: true` Bash call that returns almost instantly, then the real work runs detached) at the reducer level, so `TurnPhase` reaching `Idle`/`Done` doesn't silently erase the fact that something is still running.

## 2. Why this is NOT a new `TurnPhase` variant

`types.ts`'s own doc comment on `TurnPhase` is explicit: *"Single source of truth for the turn lifecycle... the only place where 'is the agent working', 'is a stop in flight', and 'did the stream drop' are encoded."* Three concrete reasons folding attached-task liveness into this union is the wrong shape:

1. **`workingFromPhase`/`isWorking` gate real turn-bookkeeping** — `sessionStats`/`sessionTotals` accumulation (`TurnEnd`), the failure-recovery reducer (`FailureObserved` unconditionally ends "working" phases), and the bounded-timeout watchdogs (`SUBMIT_TIMEOUT_MS`/`INTERRUPT_TIMEOUT_MS`) all key off exactly `{Submitting, Streaming, Interrupting}`. An attached background task is real, but it is *not* a turn — the CLI's own turn genuinely ended (a real `result` event fired); crediting an attached task's liveness as "still working" would corrupt session-stats semantics (a turn's cost/duration/token totals are per-*turn*, and a backgrounded task has no such boundary) and would need every one of those existing arms to defensively special-case a phase that was never meant to represent a turn state at all.
2. **There is a direct precedent for the correct shape, already in this file**: `compacting: CompactionState | null` (added `SPEC_COMPACTION_DETECTION_AND_HANDLING_2026_07_31.md`). Compaction is *also* a concurrent-but-orthogonal thing — it can be "in progress" while the turn is `Streaming`, and it needs its own start/elapsed timestamp independent of the turn's own state. It was deliberately added as a **sibling field**, not a `TurnPhase` variant, for exactly the same reason: folding it into `TurnPhase` would have meant a `Streaming+Compacting` cross-product state (or worse, losing "compacting" the instant some other transition fired). Its own review history (7 rounds of reagent/codex findings, all "a transition forgot to clear/preserve `compacting`") is a live demonstration of how much defensive plumbing an orthogonal concern needs when it's *correctly* kept separate — folding it into the turn union instead would only relocate that same plumbing into a combinatorial variant explosion.
3. **An attached task can legitimately outlive the entire pane's turn history** — a dev server started in turn 3 can still be running when turn 9 ends. `TurnPhase` resets to `Idle`/`Done` on every `TurnEnd`/`TurnReset`; nothing about "was there a task started 6 turns ago that's still alive" belongs on a per-turn enum.

**Conclusion: add a sibling field, `attachedTask: AttachedTaskState | null`, mirroring `compacting`'s shape and lifecycle discipline exactly.** This is the concrete answer to "design a TurnPhase state for a long-running attached process" — the correct fix is recognizing that axis needs to live *beside* `TurnPhase`, not inside it, matching a decision this codebase already made once for the structurally identical "compaction" case.

## 3. New type

```ts
/**
 * Live "at least one agent-declared long-running task is attached to this
 * pane" state — independent of `turnPhase`. Sourced from the dock's own
 * aggregated activity list (ActivityDock.tsx already computes "is anything
 * currently running" from shell/subagent/tool adapters); NOT from raw
 * AgentProcessRegistry process counts, which include incidental CLI helper
 * processes (language servers, watchers) that are not meaningfully
 * "long-running work" and would make this axis noisy. See
 * SPEC_ATTACHED_TASK_STATUS_AXIS_2026_08_02.md §4 for why process-count
 * was rejected as the data source.
 *
 * `since` is the wall-clock ms the FIRST currently-still-live task in the
 * current unbroken run of "≥1 running" started — NOT reset by a second
 * task starting while one is already running (mirrors `compacting`'s
 * `startedAt`, which is also "when this episode began," not "when the
 * most recent event landed").
 */
export interface AttachedTaskState {
    since: number;
}
```

Added to `AgentPaneState` as `attachedTask: AttachedTaskState | null`, initialized `null` in `initialState()`.

## 4. Why the data source is the dock's aggregate, not `AgentProcessRegistry`

`useProcessCount.ts` already gives a live, per-block process count via `agent:process-added`/`agent:process-exited`. It was considered and rejected as this axis's source:

- It counts **every** OS process in the block's job/cgroup, including short-lived or incidental ones a CLI spawns as part of normal operation (a language-server child, a git subprocess, etc.) — there is no classification, and (per §1) no age data to filter on even if there were. Wiring "count > 0" straight into a status axis would make it fire almost constantly for a healthy, idle agent, which is worse than the current silence.
- The dock's own `PinnedActivity` aggregate (`shell-adapter.ts` + `subagent-adapter.ts` + `tool-adapter.ts`) is **already** the system's answer to "what counts as agent-declared long-running work" — it exists specifically to apply that judgment (duration thresholds, MCP-tool-explicit shells, promoted Bash calls) so nothing downstream has to re-derive it. This axis should consume that same, already-curated signal, not bypass it with a raw process count.
- Once step 2 (`run_in_background` threading) lands, a detached background task becomes its own dock-adapter-produced `PinnedActivity` the same way — so this axis's data source doesn't need to change when that ships; it already subsumes the future case by construction.

## 5. Reducer commands

Two new `AgentPaneCommand` variants, dispatched from the same layer that already recomputes the dock's live activity list (deferred — see §6):

```ts
/** Fires on the 0→1 transition of "≥1 PinnedActivity is `running` for this
 *  block" — i.e. once, when an attached task episode begins. A caller
 *  dispatching this while `state.attachedTask` is already set is a no-op
 *  (mirrors `DetailsExpand`'s idempotent-if-already-open pattern) — this
 *  must NOT reset `since` every time a second task starts running
 *  alongside an already-running one. */
| { type: "AttachedTaskObserved"; at: number }
/** Fires on the 1→0 transition — the last currently-running PinnedActivity
 *  for this block just ended. No-op if `attachedTask` is already null. */
| { type: "AttachedTaskCleared" }
```

Reducer arms (same file, alongside `DetailsExpand`/`DetailsCollapse`):

```ts
case "AttachedTaskObserved": {
    if (state.attachedTask) return { state, events: [] };
    return {
        state: { ...state, attachedTask: { since: command.at } },
        events: [{ type: "attached-task-observed", at: command.at }],
    };
}
case "AttachedTaskCleared": {
    if (!state.attachedTask) return { state, events: [] };
    return { state: { ...state, attachedTask: null }, events: [{ type: "attached-task-cleared" }] };
}
```

No interaction with any `TurnPhase` arm required — this is the entire point of keeping it a sibling field. `TurnEnd`/`TurnReset`/`ReconcileTurnActive`/`FailureObserved` etc. are all unmodified; unlike `compacting`, this field's lifecycle is driven entirely by its own two commands, not implicitly cleared by turn transitions, because a task attached in one turn can legitimately survive into the next (§2 point 3) — clearing it on `TurnEnd`/`TurnReset` would be the wrong invariant (it would resurrect the exact "the dev server is still running but the UI has forgotten" bug this whole line of work exists to fix).

## 6. Deferred to a follow-up pass (needs live UI verification)

Per the standing project-docket rule ("stop and ask before anything needing live/human UI testing"), this pass ships only the pure, unit-testable reducer slice (§3/§5 + tests). Not yet wired:

1. **Dispatch call site.** `ActivityDock.tsx` (or a small new hook beside it) needs to derive "≥1 running `PinnedActivity`" from its existing `visible()` memo and dispatch `AttachedTaskObserved`/`AttachedTaskCleared` on the 0→1/1→0 edges. This touches a live-rendering component and should be checked against a running pane (a real Shell-tool task or a promoted `sleep`) before merging.
2. **`AgentFooter.tsx` rendering.** When `turnPhase.kind` is `Idle` or a terminal `Done` outcome AND `state.attachedTask != null`, show a calm "Running: N background task(s) (Ns)" status instead of nothing — this is the actual user-visible fix for the Agent1-class "pane looks idle but isn't really" symptom. Needs a real long-running-task repro to confirm the elapsed timer and copy read well.
3. **Wiring `hasRunningPromotedTool` into `AgentWorkingRow`** (already-exported signal, per §1 — just needs a consumer) — small, but still a rendering change worth checking live alongside (1)/(2) rather than in isolation.
4. **Swarm-pane / muxspect surfacing** (report's step 4) — now more tractable than when the source reports were written, since `muxspect` (Phase 1, PR #2380) and Process Broker Phase B's `controller_status` (PR #2376, not yet merged) both landed in the meantime. Left for a later pass once (1)-(3) are live-verified and this axis is proven correct in practice.

## 7. Non-goals

- Does not implement step 2 (`run_in_background` threading) — that remains its own follow-up; this axis is designed so step 2 slots into it as just another `PinnedActivity` producer, no reducer changes needed when it lands.
- Does not touch `AgentProcessRegistry`/`started_at_ms` — the Windows process-age gap (§1) is orthogonal to this axis (§4 explains why raw process counts were never this axis's intended source in the first place).
