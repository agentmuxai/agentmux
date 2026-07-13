# SPEC — subagent lifecycle: no reducer, no liveness, "working" forever

**Date:** 2026-07-12
**Author:** AgentX
**Status:** Draft
**Scope:** `SubagentInfo`/`ActiveSubagent` status end to end —
`agentmux-srv/src/backend/subagent_watcher.rs` (backend) and
`frontend/app/view/swarm/swarm-model.ts` / `swarm-view.tsx` (frontend).
Explicitly **not** the parent agent pane's own `TurnPhase` — that
already has a reducer and was already fixed (see §2).
**Related (must-read first):**
`docs/specs/REPORT_AGENT_PANE_STATE_RECONCILIATION_2026_07_07.md`
(Finding 1 — the parent-pane fix this spec extends to subagents;
Finding 2 — subagent completion detection, already fixed, confirmed
in §2 below),
`docs/specs/REPORT_SWARM_SUBAGENT_HISTORY_FLOOD_2026_07_07.md`
(the "flood on reopen" mechanism this spec's Goal 2 targets — that
report fixed row *count* via grouping; this spec fixes event *volume*
and *correctness*, which grouping didn't touch),
`docs/specs/SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23.md` (precedent
shape — the reducer pattern this spec proposes mirrors it),
issue reported directly by the user: *"the subagent lifecycle is
shakey .. do we have a reducer system for it? We always see 'working'
no matter what, even when agentmux first loads, if I open an agent, I
see a flood of working subchanges that never stop."*

---

## 1. Summary

Subagent status (`SubagentInfo.status: Active | Completed`, mirrored
frontend-side as `ActiveSubagent.status`) has **no reducer and no
liveness check anywhere in the pipeline**. It is a raw "have I read a
`type:"result"` JSONL line for this file yet" boolean, set once at
file-discovery time and revisited at exactly one call site. It has
zero connection to whether the process that would write that line is
still alive. Every pane reopen (including the very first restore after
an app launch, since restored panes carry a persisted `agent:sessionid`
and immediately replay their full subagent history) re-derives this
boolean from whatever's on disk — any subagent that crashed, was
killed, or was interrupted by a prior app/srv restart comes back
`Active` forever, with nothing anywhere capable of correcting it. The
frontend renders `sub.status === "active"` directly as "working," with
no reconciliation step at all — contrast this with the parent agent
pane's own `TurnPhase`, which **does** have a real reducer
(`agent-pane-state/reducer.ts`) and **was** already fixed for exactly
this "stuck/wrong status at mount" class of bug via `ReconcileTurnActive`,
seeded from a genuine backend liveness signal
(`health_monitor.is_active_turn()`).

This spec proposes closing that gap: give subagent status a real
liveness bound (subagents run **inside** their parent agent's own CLI
process — a Task-tool call is synchronous within the parent's turn, so
a subagent literally cannot still be active once its parent's turn has
ended), and give the transition itself an explicit, reviewable shape
instead of an inline field mutation buried in a filesystem-watcher
callback.

## 2. Where we are today

### 2.1 Backend — `agentmux-srv/src/backend/subagent_watcher.rs`

`SubagentInfo.status` is set in exactly two places, both inside one
function, `process_jsonl_change`:

- **On creation** (line 668): every new `SubagentState` is constructed
  with `status: SubagentStatus::Active` unconditionally, the instant
  its JSONL file is first observed.
- **On completion** (lines 710-721): flips to `Completed` **only if**
  the *last event read in this batch* matches the `Result` event-type
  discriminant — a real `"type":"result"` JSONL line. (This half is
  already correct — it replaced an earlier bug, tracked in
  `REPORT_AGENT_PANE_STATE_RECONCILIATION_2026_07_07.md` Finding 2,
  where completion was matched against a placeholder string that real
  output almost never produced. Confirmed fixed in current code —
  completion detection itself is not this spec's problem.)

**There is no staleness/liveness check anywhere in this file.** The
only timeout-like logic that exists at all is `refresh_workflow_status`
(lines 969-982), and it applies solely to **workflow-level** aggregates
(`WorkflowInfo.status`, a 60s-quiet heuristic gated on
`agents_done >= agents_total`) — it never touches an individual
`SubagentInfo.status`.

**On `agentmux-srv` restart:** `SubagentWatcher::new()` starts with
empty `sessions`/`workflows`/`watched_agents` maps — no persistence, no
on-disk index rebuild at startup. State only repopulates when a pane
(re)registers:

- `watch_agent()` installs a live filesystem watcher for *new* events
  going forward — deliberately does no history scan on its own.
- `scan_session_subagents()` (called from the reactive-register path,
  only when the reopening block already has a persisted
  `agent:sessionid`) replays **every** `agent-*.jsonl` file found under
  that session's `subagents/` dir through `process_jsonl_change` — the
  exact same create-Active/complete-only-on-Result logic above, run in
  a tight loop over however many subagent files that session
  accumulated, ever.

So: any subagent whose JSONL file lacks a terminal `Result` line —
because its process crashed, was killed, was interrupted by an earlier
app/srv restart, or is a subagent from a session the user hasn't
touched in weeks — replays as `Active`, indistinguishable from one that
is genuinely running right now, because nothing checks whether the
*process* that would write that line still exists.

### 2.2 `ListActive` / `GetInfo` RPCs — `agentmux-srv/src/server/service/misc.rs`

Both call straight through to `SubagentWatcher::list_active()` /
`get_info()`, which clone whatever's currently sitting in the
in-memory map. **No recomputation, no liveness check, no
cross-reference against process/controller state at read time.**
Whatever `process_jsonl_change` last wrote is exactly what's reported.

### 2.3 Frontend — two unreconciled pipelines

**Parent/root agent pane status** flows through a real reducer with an
explicit `TurnPhase` discriminated union
(`frontend/app/store/agent-pane-state/reducer.ts`,
`types.ts` — see `SPEC_AGENT_PANE_STATE_MACHINE_2026_05_23.md` for the
full design). Critically, this pipeline **has already been fixed** for
the "stuck/wrong status at mount" class of bug
(`REPORT_AGENT_PANE_STATE_RECONCILIATION_2026_07_07.md` Finding 1): a
`ReconcileTurnActive` reducer case (`reducer.ts:345-369`) seeds
`turnPhase` from the backend's real, process-tied
`health_monitor.is_active_turn()` signal
(`agentmux-srv/src/backend/blockcontroller/persistent.rs:251`),
surfaced over the wire as `GetControllerStatus().turn_active` and wired
at pane mount via `agent-view.tsx:467-473`. The reducer guards it to
only ever *promote* the mount-default `Idle` — it never overrides a
phase a live stream already produced (`reducer.ts:354`).

**Subagent rows bypass this pipeline entirely.** `SubagentRow`
(`swarm-view.tsx:369`) renders its status chip directly:

```tsx
<AgentStatusChip status={sub.status === "active" ? "working" : "idle"} />
```

`sub.status` is `ActiveSubagent.status` (`swarm-model.ts:23`),
populated verbatim from `SubagentInfo.status` over the wire (§2.1/§2.2)
— no `TurnPhase`, no reducer, no reconciliation of any kind.
`SwarmViewModel._subagents` is a flat SolidJS signal mutated directly
from event handlers and RPC responses; the only merge step
(`mergeSubagentsPreservingIdentity`) is a *reference-identity*
optimization to stop `<For>` from remounting rows — it has no concept
of valid/invalid transitions, it just overwrites whatever fields the
backend most recently reported.

**Answer to "do we have a reducer system for it": partially, and not
where it matters most.** The parent pane does; subagents do not, on
either side of the wire. The fix that solved this exact symptom for
the top-level agent pane was never extended to subagents, and no
backend concept analogous to `health_monitor` exists for subagents to
reconcile against even if a frontend-only fix were attempted.

### 2.4 The "flood on open" mechanism, precisely

At `SwarmViewModel` construction, `loadAll()` fires `loadSubagents()`
once, and two WS subscriptions are wired: `subagent:spawned` and
`subagent:completed` each call `void this.loadSubagents()` — **every
single spawn/completion event triggers a full `subagent.ListActive`
RPC round-trip.** No debouncing, batching, or coalescing on the
frontend side.

Backend side, `scan_session_subagents` → `scan_subagents_dir` →
`process_jsonl_change` runs once per `agent-*.jsonl` file found on a
pane reopen. Each call independently broadcasts its own
`subagent:spawned` WS event if the subagent wasn't already tracked in
this process's memory — which, on a fresh `agentmux-srv` process or a
first-time-this-session pane open, is *every single one of them*. The
200ms fs-notify debounce (`subagent_watcher.rs:362-369`) applies only
to live filesystem events, not to this synchronous backfill scan — the
scan fires its broadcasts inline, unthrottled, one per file.
`REPORT_SWARM_SUBAGENT_HISTORY_FLOOD_2026_07_07.md` Finding 4 already
captured live traces of 45 and 227 subagents replayed this way in a
single reopen.

**No polling exists anywhere in this pipeline** — confirmed by
exhaustive search of `frontend/app/view/swarm/`. The "flood that never
stops" is fully explained by the combination of: (1) every backfilled
subagent broadcasts its own event, each independently triggering a
`loadSubagents()` RPC; (2) a meaningful fraction of those subagents
have no terminal marker on disk and get `Active` with nothing to ever
correct it; (3) the view renders that raw field with no reconciliation,
so every stale entry displays as perpetually working; and (4) the
existing workflow/name grouping fix (issue #1624-adjacent work,
`agentx/swarm-group-same-name-subagents`) reduces *row count* but not
*event volume* or *correctness* — a stale subagent still reports
"active" inside its collapsed group.

## 3. Motivation

Concretely, from the user's report: opening any agent pane with
session history shows a wall of subagents stuck on "working," including
ones from days-old sessions that could not possibly still be running.
This isn't cosmetic — it actively misleads: a user checking "is
anything still running before I close this" cannot trust the Swarm
view at all right now, because the answer is "everything, forever."

## 4. Goals / non-goals

**Goals:**

1. Give subagent status a real liveness bound, so a subagent whose
   parent turn has ended can never continue displaying as "working."
2. Make the transition an explicit, reviewable state change — not an
   inline field mutation inside a filesystem-watcher callback — so the
   legal-state set and every path into "Active" are auditable in one
   place, mirroring the `agent-pane-state` reducer precedent.
3. Cut backfill event/RPC volume on reopen — a session replay should
   not cost one `subagent:spawned` broadcast (and one
   frontend `loadSubagents()` round-trip) per file.
4. Preserve everything already shipped and working: completion
   detection via the `Result` discriminant (§2.1), workflow/name
   grouping, inline-expand detail, Haiku naming.

**Non-goals:**

- Redesigning workflow-level status (`WorkflowInfo.status` /
  `refresh_workflow_status`) — out of scope, already has its own
  (heuristic but functioning) staleness handling.
- Persisting subagent state across `agentmux-srv` restarts — the fix
  here is correctness of the *rebuild*, not adding durable storage.
- Touching the parent agent pane's own `TurnPhase` reducer — already
  correct (§2.3); this spec only adds an analogous mechanism for
  subagents, it does not modify the existing one.

## 5. The core insight: a subagent cannot outlive its parent's turn

A subagent is not an independent OS process AgentMux spawns and loses
track of — it's a Task-tool call made **inside the parent agent's own
CLI process**, synchronously within the parent's own turn
(`subagent_watcher.rs:181`: *"`parent_block_id` is the pane/block that
owns this Claude instance"*). This means the parent block's own
`turn_active` — already tracked, already correct, already reconciled
via `ReconcileTurnActive` (§2.3) — is a valid, zero-new-infrastructure
upper bound on every one of its subagents' liveness:

> **If the parent block's turn is not active, no subagent whose
> `parent_block_id` is that block can genuinely still be `Active`.**

This is the backend-side equivalent of what `health_monitor` already
gives the frontend for the parent pane itself — we don't need a new
liveness primitive, we need to propagate the one that already exists
down to the subagents it structurally bounds.

## 6. Proposed design

### 6.1 Backend — explicit transition function + reconciliation pass

Replace the two inline mutation sites in `process_jsonl_change` with a
single, named transition function subagents flow through — not a full
Rust enum-state-machine crate, just a deliberate seam so every path
into a status change is one reviewable function instead of scattered
`state.info.status = ...` assignments:

```rust
enum SubagentTransition {
    Discovered,                 // new JSONL file observed
    ResultEventSeen,            // Result discriminant read from the tail
    ParentTurnEnded,            // reconciliation pass, see below
}

fn apply_subagent_transition(info: &mut SubagentInfo, t: SubagentTransition) {
    match (info.status, t) {
        (_, SubagentTransition::Discovered) if !matches!(info.status, SubagentStatus::Completed) => {
            info.status = SubagentStatus::Active;
        }
        (SubagentStatus::Active, SubagentTransition::ResultEventSeen) => {
            info.status = SubagentStatus::Completed;
        }
        (SubagentStatus::Active, SubagentTransition::ParentTurnEnded) => {
            info.status = SubagentStatus::Abandoned; // new variant, see 6.2
        }
        _ => {} // no-op: already Completed/Abandoned, or a transition that doesn't apply
    }
}
```

Add a reconciliation pass, mirroring `refresh_workflow_status`'s
existing shape (same file, called from the same place workflow status
already gets refreshed): whenever a parent block's `turn_active` flips
to `false` (the persistent controller already publishes this — see
`persistent.rs:319`, *"Publish the turn_active flip so the Swarm
view's live [...] signal"* — this comment already anticipates a
consumer that doesn't exist yet), walk that block's subagents and
apply `ParentTurnEnded` to any still `Active`. This closes the gap at
the moment it's created, not just at the next reopen.

### 6.2 New terminal status: `Abandoned`

`SubagentStatus` gains a third variant, `Abandoned` — a subagent whose
parent turn ended without a `Result` line ever appearing (crashed,
killed, interrupted by a restart). This is deliberately **not** folded
into `Completed`: a completed subagent finished its work; an abandoned
one didn't, and the distinction is useful information (a user
debugging "why didn't this sub-task's output show up" wants to know it
never finished, not that it silently succeeded). Frontend maps it to a
new `AgentStatusChip` state — visually distinct from both "working"
and "idle," something like "interrupted" — not "working," and not
silently hidden either.

### 6.3 Startup/reopen reconciliation, not just going-forward

`scan_session_subagents` (the backfill-on-reopen path, §2.1) should, at
the end of its replay, apply the same `ParentTurnEnded` reconciliation
for any subagent it just replayed as `Active` whose parent block's
current `turn_active` is false at that moment — closing the case the
user actually reported ("even when agentmux first loads, if I open an
agent, I see a flood of working"). This is the direct fix for the
symptom: a session reopened after the app restarted has, by
definition, no live turn for any of its historical subagents unless
the user has just re-launched a new turn — so the reconciliation should
almost always resolve every backfilled `Active` entry to either
`Completed` (if it has a `Result` line) or `Abandoned` (if it doesn't),
leaving zero stale "working" rows on a cold reopen.

### 6.4 Cut backfill event volume

`scan_subagents_dir`'s replay currently broadcasts one
`subagent:spawned` per file, inline, unthrottled (§2.4). Batch it: emit
one `subagent:spawned-batch` (or reuse `subagent:spawned` with a list
payload) after the full directory scan completes, instead of N
individual events. Frontend's handler becomes "refresh once after the
batch settles" instead of "refresh once per event" — same fan-in shape
`REPORT_SWARM_SUBAGENT_HISTORY_FLOOD_2026_07_07.md` already established
for the live fs-notify path (the 200ms debounce), just applied to the
backfill path too, which currently has none.

### 6.5 Frontend — reconcile, don't just relay

`ActiveSubagent.status` becomes the same three-way
`"active" | "completed" | "abandoned"` the backend now produces —
`swarm-model.ts` and `swarm-view.tsx` are already pure projections of
whatever the backend reports (§2.3), so once the backend's status is
trustworthy, the frontend mostly needs a type update and a new status
chip mapping (§6.2), not a rewrite. The one behavioral addition: since
`buildTree()` already has `statuses: Map<blockId, "running" | "idle">`
in scope (from `agentStatusesAtom`, itself fed by
`GetControllerStatus`/`controllerstatus`, §2.3), it can defensively
downgrade any subagent still reporting `"active"` whose
`parent_block_id`'s status is `"idle"` to a display-only "likely stale"
treatment — a client-side backstop for the (should-be-rare, once §6.1
ships) case where the backend hasn't reconciled yet. This does **not**
replace the backend fix — it's a belt-and-suspenders UI guard, since
the frontend already has the exact same "is the parent block's turn
active" signal available for free.

## 7. Rollout

1. **Backend, additive:** add `SubagentStatus::Abandoned`, the
   transition function (§6.1), and the parent-turn-ended reconciliation
   pass (§6.1/§6.3). No wire-format break — `Abandoned` is a new enum
   variant frontend code that hasn't been updated yet would just fail
   to recognize (falls through to a default "unknown" render, same as
   any other unrecognized string field elsewhere in this codebase).
2. **Backend, event volume:** batch the backfill-scan broadcast (§6.4).
   Independent of #1 — can ship separately, is lower-risk.
3. **Frontend:** wire the three-way status, new chip state, and the
   defensive parent-idle downgrade (§6.5).
4. Each step ships as its own PR with tests, per this repo's usual
   changeset workflow — no reason to land this as one large diff.

## 8. Tests

- Backend: unit tests for `apply_subagent_transition` covering every
  cell of the transition table in §6.1 (including the no-op cases —
  `ParentTurnEnded` on an already-`Completed` subagent must not
  downgrade it).
- Backend: `scan_session_subagents` integration test — seed a session
  with (a) a subagent JSONL ending in a `Result` line, (b) one with no
  terminal line, parent turn currently inactive; assert (a) → `Completed`,
  (b) → `Abandoned`, not `Active`, after the scan.
- Backend: assert the backfill scan emits one batched event, not N.
- Frontend: `swarm-model.test.ts` — extend `ActiveSubagent` fixtures
  for the third status value; add a `groupSubagentsByWorkflow`
  case confirming `"abandoned"` subagents group/sort correctly
  alongside `"active"`/`"completed"` ones (should behave like
  `"completed"` for grouping purposes — both are terminal).
- Frontend: `AgentStatusChip` renders a distinct label/style for
  `"abandoned"`, not "working."

## 9. Open questions

1. **Reconciliation trigger granularity** — should `ParentTurnEnded`
   fire from the exact moment `persistent.rs` flips `turn_active` to
   false (real-time, requires wiring a new call site there), or is it
   sufficient to only reconcile at `scan_session_subagents` time (reopen/backfill
   only, simpler, but leaves a subagent stuck "Active" for the rest of
   the CURRENT session between the parent turn ending and the next
   pane reopen)? Recommend starting with the reopen-time-only version
   (§6.3, closes the user's reported symptom directly, smallest diff)
   and evaluating whether the live case is common enough in practice
   to warrant the real-time wiring as a fast-follow.
2. **`Abandoned` vs. surfacing nothing** — is a new visible status
   worth the UI surface, or should an abandoned subagent just silently
   stop rendering as "working" and fall into the existing "idle"
   bucket? Recommend keeping it distinct (§6.2's reasoning) but this is
   a product call, not an engineering one.
3. **Does `MAX_SUBAGENT_EVENTS` truncation ever discard the `Result`
   line** for a very long-running subagent, causing a false
   `Abandoned` for something that actually completed? Needs a direct
   check against `subagent_watcher.rs`'s truncation logic before
   implementation — flagged here since events are trimmed oldest-first
   and completion detection reads "the last event in the batch," which
   should be safe (truncation drops from the front, not the tail), but
   worth confirming explicitly in the implementing PR.
