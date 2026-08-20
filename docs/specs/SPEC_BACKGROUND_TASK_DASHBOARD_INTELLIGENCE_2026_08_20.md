# Spec: Intelligent Long-Running-Task Dashboard (Phase C)

**Date:** 2026-08-20
**Author:** AgentA
**Status:** Proposed
**Depends on:** `SPEC_BACKGROUND_TASK_PID_CAPTURE_2026_08_20.md` (Phase A), benefits from but does not strictly require `SPEC_BACKGROUND_TASK_TEARDOWN_SURVIVAL_2026_08_20.md` (Phase B) landing first
**Addresses:** rung 4 items in `docs/status/STATUS_ATTACHED_TASK_AXIS_AND_DEV_LOOP_2026_08_15.md` §3 (registry-as-reader), the still-open Swarm gap (§1's last row), and the `DockSnapshotCache` 1-hour TTL problem (§2.2)

## 1. What's already correct (do not re-litigate)

Verified against current code before writing this spec, specifically to avoid re-doing settled work:

- **`turnPhase` and `attachedTask` are already fully decoupled** (`agent-pane-state/types.ts:123-125,326`, `reducer.ts` — no `TurnPhase`-transition case touches `attachedTask`). A turn ending while a background task is still running already correctly flips the footer to "✓ Worked · Ns" instead of staying stuck on "Working…" — this was the literal 12-hour-stuck-status bug from `docs/retro/retro-persistent-agent-working-status-stuck-2026-07-16.md`, and it is already fixed. **Do not add a "am I really idle" heuristic to `turnPhase` — the existing sibling-axis design is correct and deliberate**, per that retro's own explicit correction notice.
- **A dedicated "Running in background" footer render was tried and reverted** (`AgentFooter.tsx:272-276`) on direct 2026-08-10 user feedback: it duplicated the ActivityDock's own running row. **Do not re-add footer text for this.** The reducer's `attachedTask` axis stays (it feeds the watchdog and is meant for Swarm), but its consumer should not be a second copy of what the dock already shows in the same viewport.
- **The dock UI itself has no TTL problem** — it's derived live from transcript replay, `ActivityDock.tsx`'s render has no dependency on any 1-hour cutoff. The 1-hour eviction (`dock_snapshot.rs:65`, `MAX_NODE_AGE_MS`) only affects `DockSnapshotCache`, a **separate, write-only-from-the-frontend, diagnostic mirror** read solely by the `muxspect` CLI (`muxspect_handlers.rs`) — not by the agent-pane UI at all.

## 2. What's actually missing

Given §1, the real gap is narrower than "fix the dashboard's stuck Working status" (already fixed) — it is:

1. **No backend-sourced push/subscription feeds the frontend's dock/attachedTask state at all.** Every signal is derived from replaying *this tab's own* live transcript stream. Reload the pane, or open the workspace in a second tab/window, and the dock/attachedTask state has to be re-derived from scratch — for a task still genuinely running, this mostly works (the transcript still contains the accepted-launch tool call), but it's re-deriving from a stream, not querying a source of truth, and completely fails once `db_background_tasks` (Phase A/B) is the only place recording a survived-a-restart task's existence for a *new* controller generation that has no transcript history of ever launching it (Phase B §3.5's exact scenario).
2. **`muxspect dock`'s 1-hour TTL (§1) does matter for that CLI's own diagnostic value**, even though it doesn't affect the live UI — a `task dev` genuinely running 12+ hours (this repo's own precedent) is invisible to `muxspect dock` after the first hour, which is a real diagnostic gap for anyone debugging via that tool.
3. **Swarm has zero wiring to any of this** (`grep -n "attachedTask\|run_in_background" frontend/app/view/swarm/*.ts*` → zero hits, confirmed still true) — a fleet view that can't show "this agent has a dev server attached" is a real, still-open gap from the original 07-26 report, never picked up.
4. **Reconnect/history-restore scrubs based on session-boundary heuristics** (`scrubOrphanedInProgress`, per `RETRO_TASK_DEV_IDLE_KILL_FALSE_POSITIVE_2026_07_31.md`) rather than an authoritative query — a task's true status should be one lookup away, not inferred from which transcript events happened to survive.

## 3. Design

### 3.1 `db_background_tasks` becomes a genuine read source, not just a write-only mirror

Add a read RPC command (WebSocket, not the `muxspect` HTTP-only route) — `COMMAND_LIST_BACKGROUND_TASKS` (or fold into an existing bootstrap/snapshot RPC if one already exists for block-open) — that returns `background_task_list_for_block(block_id)`'s current rows. Two call sites:

- **On block/pane mount** (wherever the frontend currently establishes its per-block WPS subscription — `ActivityDock.tsx:47-56`'s existing wiring is the closest analog): query this once, seed `attachedTask`/dock state from it *before* any transcript replay has a chance to run, so a survived-restart task (Phase B) is visible immediately rather than only after some other event triggers a re-derivation.
- **On explicit reconnect** (whatever handles the `Disconnected` → reconnected transition in `TurnPhase`): re-query, reconciling rather than blindly trusting either source — if the registry says `Running` but a subsequent transcript replay's own `<task-notification>` says it completed, the transcript event (closer to the actual completion signal, per `websocket.rs:1105-1118`'s own reasoning for why completion has to be client-parsed) wins; the registry is a floor ("at least this much is known to have been running"), not an override of a more specific live signal.

### 3.2 Push updates via the existing WPS-event pattern, not polling

Per the 08-15 status doc §3 point 4's own recommendation: "become *readers* of the registry (via a subscription, same WPS-event pattern `AgentProcessRegistry` already uses for `agent:process-added`/`-exited`)." Concretely: `background_task_observe`/`_complete`/`_set_pid` (`background_tasks.rs`), in addition to writing to SQLite, publish a `background-task-updated` WPS event scoped to `block:<id>`, mirroring the existing chunk-publish pattern bashwrap itself already uses. The frontend's existing per-block WPS subscription (already open for tool-chunk streaming) picks this up and dispatches an `AttachedTaskObserved`/`Cleared`-equivalent command — no new subscription channel needed, just a new event type on the one that already exists.

This directly fixes §2 point 2 as a side effect for anything that also wants it (a future `muxspect dock --watch`-style command could subscribe to the same event instead of polling), even though this spec's primary scope is the frontend UI, not `muxspect` itself.

### 3.3 Swarm surfacing

Once §3.1/§3.2 exist, Swarm's gap (§2 point 3) closes by simply becoming another subscriber: the Swarm pane's per-agent summary row gains a small indicator (icon + count, e.g. "⚙ 1" — deliberately not another full-text "Running in background" string, consistent with §1's don't-duplicate lesson) sourced from the same `background_task_list_for_block`/WPS-event feed, keyed by whichever block each Swarm-tracked agent's primary pane maps to. This is additive and low-risk — no existing Swarm state machine needs to change, only a new read.

### 3.4 `muxspect dock`'s TTL gap

Two options, pick the simpler one at implementation time rather than deciding definitively here:
- (a) Have `DockSnapshotCache`'s eviction check `db_background_tasks` before evicting a `bg: true` node — skip eviction (or refresh `observed_at` from `last_seen_ms`) for anything the durable registry still says is `Running`. Small, localized change to `dock_snapshot.rs`'s existing `retain` call.
- (b) Have `muxspect dock` itself query `db_background_tasks` directly for anything `DockSnapshotCache` no longer has, and merge the two result sets for display. No change to `DockSnapshotCache`'s existing eviction behavior (which is correct and intentional for genuinely-gone panes, per its own doc comment) — only `muxspect`'s presentation layer changes.

Prefer (b): it keeps `DockSnapshotCache`'s existing, already-correct "ephemeral, no live renderer ⇒ nothing to report" semantics untouched (`dock_snapshot.rs:9-11`'s own reasoning is sound and shouldn't be complicated), and confines the fix to the one place that actually has the problem (the CLI's own display logic).

### 3.5 Explicitly not building

- No new footer/composer-area text (§1).
- No change to `turnPhase`'s state machine (§1).
- No UI for stopping a background task from the dashboard in this phase (Phase B §6 flags this as a possible future addition, not required here) — this spec is about *visibility* accuracy, not adding new control-plane actions. If Phase B's Background Task Container work lands first, a stop action becomes easy to add later as a small follow-up; not bundling it here keeps this phase's review surface focused.

## 4. Testing

- Backend: unit tests for the new `COMMAND_LIST_BACKGROUND_TASKS` handler and the new WPS event publish points (mirroring existing `background_tasks.rs`/`websocket.rs` test patterns).
- Frontend: reducer tests for the new dispatch path seeding `attachedTask` from a bootstrap query (distinguish from the existing transcript-derived `AttachedTaskObserved` dispatch — same command type, different trigger, should be idempotent either way per the existing 0→1-edge design).
- Integration: reconnect test — background a task, force a reconnect (or the Phase B live-verify's session-restart repro), confirm the dashboard shows it as running immediately on the new pane/session without waiting for any transcript replay to reach the relevant tool call.
- Swarm: manual/visual check that the new indicator appears and disappears correctly as a tracked agent's background task starts/completes.

## 5. Non-goals

- Redesigning `ActivityDock`'s visual layout — this spec only changes *where the data comes from* for the running/attached state, not how it's rendered, beyond the small additive Swarm indicator in §3.3.
- Windows `AgentProcessRegistry.started_at_ms` (`process_tracker/windows.rs:198`, hardcoded 0, needs `NtQueryInformationProcess`) — rung 5 from the 08-15 status doc, explicitly "not urgent," independent of this work, left for a future pass.
