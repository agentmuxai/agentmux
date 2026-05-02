# SPEC: Host Reducer Buildout — 5-PR Plan

**Date:** 2026-05-02
**Status:** Draft — operational plan
**Author:** AgentA
**Branch base:** `main` HEAD `48ad4e58`

**Companion docs:**
- `docs/specs/SPEC_HOST_REDUCER_PHASE_H_2026-05-02.md` — granular Phase H spec; this doc is its 5-PR compression. Reference the granular spec for full type definitions, reducer semantics, and decision log.
- `docs/retro/reducer-architecture-current-state-2026-05-02.md` — verified factual catalog of what's in / out of reducers today.
- `docs/specs/SPEC_WINDOW_FLEET_REDUCER_2026-05-02.md` — 2026-05-02 freeze investigation (dump analysis, root cause).

---

## Overview

Migrate all host-side mutable state (panes, CEF browser handles, drag, pool, quit, top-level creation) into the host reducer. **Five PRs**, each one representing a logical phase. The granular a→b→c→d→e migration ratchet from B.5 still applies — it's now compressed into per-PR commit sequences instead of separate PRs.

**Architectural directives (unchanged from Phase H spec):**
1. NO timers, NO watchdogs in any reducer arm. Event-driven only.
2. Fail-fast for user-initiated operations under contention.
3. Single source of truth — every state field owned by exactly one reducer arm.
4. Cross-state invariants are first-class.

**Total effort:** 2-3 weeks wall-clock + bot review iteration. Same ballpark as the saga reducer migration (PRs #641-#649).

---

## PR sequence at a glance

| # | Branch | Title | Depends on | Effort | Risk |
|---|---|---|---|---|---|
| 1 | `agenta/h1-foundations` | feat(cef): host reducer foundations — state, commands, events, no behavior change | main | 1 day | Low |
| 2 | `agenta/h2-panes-and-browsers` | feat(cef): migrate pane lifecycle + state.browsers map into host reducer | PR #1 | 4-5 days | High |
| 3 | `agenta/h3-drag-pool-quit` | feat(cef): migrate drag, pool, quit state into host reducer | PR #1 (parallel to #2) | 3-4 days | Medium |
| 4 | `agenta/h4-top-level-runner-and-sagas` | feat(cef): event-driven top-level window creation runner + cross-state sagas | PR #1, #2, #3 | 3-4 days | High |
| 5 | `agenta/h5-durability-diag-wire-promote` | feat(cef): host reducer durable log + --diag windows + wire-promoted events | PR #1-#4 | 2-3 days | Low |

PR #2 and PR #3 can run in parallel after PR #1 lands. PR #4 needs both. PR #5 is gravy at the end.

---

## PR #1 — Foundations

**Branch:** `agenta/h1-foundations`
**Title:** `feat(cef): host reducer foundations — state, commands, events, no behavior change`
**Effort:** ~1 day
**Risk:** Low (no behavior change)

### Goal

Add ALL the new `HostState` fields, `HostCommand` variants, `HostEvent` variants, and reducer arms required by PRs #2-#5 — but DON'T wire any callers yet. Existing state continues to be authoritative; new reducer fields are present but dormant. This avoids reducer-scaffolding churn in every subsequent PR.

### Files touched

- `agentmux-cef/src/state.rs` — add new types: `PaneEntry`, `PaneLifecycle`, `BrowserHandle`, `BrowserKind`, `DragSession` (already exists), `PoolState`, `TopLevelCreationRequest`, `InFlightCreation`, `CreationPhase`, `CompletedCreation`, `QuitState`, `QuitReason`.
- `agentmux-cef/src/reducer.rs` — extend `HostState` with new fields (default-initialized empty/None); add `HostCommand` variants; add `HostEvent` variants; add reducer arms.
- `agentmux-cef/src/state.rs::log_host_event` — extend match to log new events.

### State added to `HostState`

```rust
pub struct HostState {
    // F.1 (existing):
    pub pending_window_creations: VecDeque<PendingWindowCreation>,
    pub lifecycle: HostLifecyclePhase,
    pub event_version: u64,
    
    // H.1:
    pub panes: HashMap<String /* block_id */, PaneEntry>,
    
    // H.2:
    pub browsers: HashMap<String /* label */, BrowserHandle>,
    
    // H.3:
    pub active_drag: Option<DragSession>,
    
    // H.4:
    pub pool: PoolState,
    
    // H.5:
    pub quit_state: QuitState,
    
    // H.6:
    pub top_level_creation: TopLevelCreationState,
}
```

Type definitions per granular Phase H spec §"Phase H.0".

### Commands added

```rust
HostCommand::EnqueuePaneCreate { block_id, label }
HostCommand::CompletePaneCreate { block_id }
HostCommand::EnqueuePaneClose { block_id }
HostCommand::CompletePaneClose { block_id }
HostCommand::AbortPaneCreate { block_id, reason }

HostCommand::RegisterBrowser { label, browser, kind }
HostCommand::UnregisterBrowser { label }

HostCommand::StartDrag { drag_id, drag_type, source_window, source_workspace_id, source_tab_id, payload }
HostCommand::EndDrag { drag_id, outcome }

HostCommand::PoolWindowSpawnStart { label }
HostCommand::PoolWindowReady { label }
HostCommand::PoolWindowDestroyedBeforePromote { label }
HostCommand::PromotePoolWindow { label, /* ... */ }
HostCommand::PoolDrainAll

HostCommand::BeginDrain { reason }
HostCommand::ConfirmDrained

HostCommand::EnqueueTopLevelWindow { request, source: TopLevelSource }
HostCommand::TopLevelCallbackFired { label }
HostCommand::TopLevelRendererTerminated { label, status }
HostCommand::TopLevelExternallyClosed { label }
```

`TopLevelSource::{User, Background}` — distinguishes fail-fast (user) from queue-able (background pool refill).

### Events added

```rust
HostEvent::PaneCreateRequested { block_id, label, version }
HostEvent::PaneLive { block_id, label, version }
HostEvent::PaneClosing { block_id, version }
HostEvent::PaneClosed { block_id, version }
HostEvent::PaneCreationFailed { block_id, reason, version }

HostEvent::BrowserRegistered { label, kind, version }
HostEvent::BrowserUnregistered { label, version }

HostEvent::DragStarted { drag_id, drag_type, source_window, version }
HostEvent::DragEnded { drag_id, outcome, version }

HostEvent::PoolWindowEntered { label, queue_len_after, version }
HostEvent::PoolWindowLeft { label, queue_len_after, reason, version }
HostEvent::PoolEmpty { version }

HostEvent::QuitDraining { reason, version }
HostEvent::QuitReady { version }

HostEvent::TopLevelCreationRequested { creation_id, request, source, version }
HostEvent::TopLevelCreationStarted { creation_id, label, version }
HostEvent::TopLevelCreationCompleted { creation_id, label, latency_ms, version }
HostEvent::TopLevelCreationFailed { creation_id, label, reason, version }
HostEvent::TopLevelQueueLengthChanged { len, version }
```

Plus an `HostEvent::Effect(EffectKind)` carrier for side-effect-bearing events (used by PR #4's effect handler).

### Reducer arms

All arms implemented per the granular spec's reducer rules (§"Phase H.1" through "Phase H.6"). Arms are pure functions, no I/O.

### Tests

- Unit test per command (input → state change + emitted events).
- Proptests for invariants:
  - `panes` map: every `EnqueuePaneCreate` followed by either `CompletePaneCreate` (Live) or `AbortPaneCreate` (removed) — no orphans.
  - `top_level_creation.in_flight`: at most one Some across any action sequence.
  - `pool.respawn_in_flight`: never two concurrent spawn-starts.
  - `quit_state`: monotonic — Running → Draining → Quit, no regression.
- Existing F.1 tests remain green (untouched).

### Acceptance criteria

- All new types compile.
- All new commands route through `update()`.
- All new events log via `log_host_event`.
- New reducer fields have no callers yet (dead-code warnings expected; `#[allow(dead_code)]` on the new fields with TODO comments referencing PR #2-#5).
- `cargo test reducer::tests` → 100% pass, including new tests.
- No existing behavior changes — PaneStateMachine, state.browsers HashMap, active_drag, etc. all still work as before.

### Bump

Patch version (e.g., 0.33.580). Internal-only changes.

---

## PR #2 — Panes + Browsers

**Branch:** `agenta/h2-panes-and-browsers`
**Title:** `feat(cef): migrate pane lifecycle + state.browsers map into host reducer`
**Effort:** ~4-5 days
**Risk:** High (touches 40+ lock sites for `state.browsers`; pane lifecycle is core operational logic)

### Goal

Migrate two tightly-coupled pieces in one PR:
1. **`PaneStateMachine` → `HostState.panes`** (replaces `BrowserPaneManager::PaneStateMachine`).
2. **`AppState.browsers: Mutex<HashMap>` → `HostState.browsers`** (replaces direct mutex; consolidates 40+ lock sites through reducer queries).

These migrate together because the `state.browsers` map contains both top-level browsers AND pane browsers, and pane lifecycle transitions trigger browser-handle insertions/removals. Migrating them separately would require carefully coordinated parallel writes — easier to do them together.

### Internal commit sequence (the a→e ratchet, compressed)

PR #2 will have ~10-15 commits internally, structured for stepwise review:

```
commit 1 (H.1.a): pane parallel writes — PaneStateMachine.try_register_live ALSO dispatches EnqueuePaneCreate
commit 2 (H.2.a): browser parallel writes — state.browsers.insert ALSO dispatches RegisterBrowser
commit 3 (H.1.b): pane reads with fallback — list_live_panes prefers reducer, falls back to PaneStateMachine
commit 4 (H.2.b): browser reads with fallback — get_browser/list_browsers prefer reducer, fall back to AppState.browsers
commit 5 (H.1.c+H.2.c): flip reads — no fallback, pure reducer reads
commit 6 (H.1.d+H.2.d): drop legacy writes — PaneStateMachine and AppState.browsers become read-only/skeletal
commit 7 (H.1.e+H.2.e): delete legacy fields — PaneStateMachine struct gone; AppState.browsers field gone
commit 8: integration tests + smoke verification
```

This lets reviewers walk the PR commit-by-commit while shipping atomically.

### Files touched

- `agentmux-cef/src/pane/lifecycle.rs` — `PaneStateMachine` first becomes a thin shim, then deleted.
- `agentmux-cef/src/browser_panes.rs` — `BrowserPaneManager` reads via reducer queries; close/focus/resize call sites update.
- `agentmux-cef/src/pane/callbacks.rs` — `on_after_created_pane` dispatches `CompletePaneCreate`; `on_before_close` dispatches `EnqueuePaneClose` + `CompletePaneClose`.
- `agentmux-cef/src/pane/creation.rs` — pane creation dispatches `EnqueuePaneCreate` + `RegisterBrowser`.
- `agentmux-cef/src/client.rs` — `on_after_created` dispatches `RegisterBrowser` for top-level (existing `state.browsers.insert` call site, line ~205).
- `agentmux-cef/src/state.rs` — read helpers (`get_browser`, `list_browsers`, `list_live_panes`, etc.) added to `AppState`. Eventually `pub browsers` field deleted.
- All ~40 `state.browsers.lock()` call sites — converted to reducer query helpers.

### Migration of `state.browsers` lock sites

Categorize the 40+ sites by usage pattern:

| Pattern | Conversion |
|---|---|
| `browsers.lock().get(&label).cloned()` | `state.get_browser(label)` |
| `browsers.lock().contains_key(&label)` | `state.has_browser(label)` |
| `browsers.lock().keys().collect()` | `state.list_browser_labels()` |
| `browsers.lock().iter().filter(...)` | `state.list_browsers().iter().filter(...)` (clones, but FFI handles are refcounted so cheap) |
| `browsers.lock().insert(label, browser)` | `state.host_dispatch(RegisterBrowser{label, browser, kind})` |
| `browsers.lock().remove(&label)` | `state.host_dispatch(UnregisterBrowser{label})` |

The reducer effect for `RegisterBrowser` performs the actual map insert into `HostState.browsers`. For `UnregisterBrowser`, removes from map and emits `BrowserUnregistered`.

**Snapshot-and-drop preserved:** read helpers acquire the host_state lock, clone the value, drop the lock — same discipline as `host_dispatch`.

### Tests

- Pane lifecycle: create → live → close → closed (via reducer).
- Browser registration: register top-level, register pane (matched to live pane entry), unregister.
- Read helpers: `get_browser` returns clone of registered browser; `list_live_panes` returns all panes in `Live` state.
- Drift detection (commits 3-4): assert reducer state and PaneStateMachine state are consistent across a workload.
- Existing tests for `BrowserPaneManager` adapted to use reducer-backed `AppState`.

### Smoke verification (manual, before merge)

- Open AgentMux. Create a tab with multiple browser blocks. Close the tab. All browsers cleanly unregistered.
- Drag-and-drop a tab between windows. Browser handles correctly tracked.
- Open multiple top-level windows. Each registered. Close each. Each unregistered.
- **Freeze probe:** open a top-level window with a browser pane present. (Without PR #4's runner, this may still freeze — that's expected. PR #2 doesn't claim to fix the freeze, only to make pane state observable in the reducer.)

### Acceptance criteria

- `cargo test` 100% pass.
- 0 remaining `state.browsers.lock()` call sites in `agentmux-cef/src/` (grep verifies).
- 0 remaining references to `PaneStateMachine` (grep verifies).
- Smoke checklist above passes.
- `BrowserPaneManager` is now a stateless utility wrapper around reducer queries.

### Bump

Minor version bump (e.g., 0.34.0). Architectural change, even though externally-visible behavior is unchanged.

### Risk mitigation

- **Smoke test before merge.** This PR touches the highest-traffic mutex in the host. Land only after a thorough manual exercise.
- **Internal commit boundaries.** Each commit is reviewable independently; if step c surfaces drift, hold the PR until investigated rather than merging the whole thing.
- **Drift detection in commits 3-4** runs in production-like conditions (smoke test) before commit 5 flips reads to reducer-only.

---

## PR #3 — Drag + Pool + Quit

**Branch:** `agenta/h3-drag-pool-quit`
**Title:** `feat(cef): migrate drag, pool, quit state into host reducer`
**Effort:** ~3-4 days
**Risk:** Medium (multiple state pieces, but each is small and self-contained)

### Goal

Migrate three smaller, independent state pieces into the reducer:
- `active_drag: Mutex<Option<DragSession>>` → `HostState.active_drag`
- `window_pool` + `unpromoted_pool_labels` + `window_pool_respawn_in_flight` atomic → `HostState.pool: PoolState`
- `is_quitting: AtomicBool` → `HostState.quit_state: QuitState`

Can run in parallel with PR #2 since these state pieces don't overlap with panes/browsers.

### Internal commit sequence

```
commit 1 (H.3.a→e): drag migration (active_drag → HostState.active_drag, full a→e in one batch since few call sites)
commit 2 (H.4.a→b): pool parallel writes + reads with fallback
commit 3 (H.4.c→e): pool flip reads, drop writes, delete legacy fields
commit 4 (H.5.a→e): quit state migration (small, single batch)
commit 5: integration tests + smoke
```

### Files touched

- `agentmux-cef/src/state.rs` — read helpers; eventually delete `active_drag`, `window_pool`, `unpromoted_pool_labels`, `window_pool_respawn_in_flight`, `is_quitting` fields.
- `agentmux-cef/src/commands/drag.rs` — drag start/end via reducer.
- `agentmux-cef/src/commands/window_pool.rs` — `spawn_pool_window` dispatches `PoolWindowSpawnStart`; `mark_pool_window_renderer_ready` dispatches `PoolWindowReady`; `promote_pool_window` dispatches `PromotePoolWindow`.
- `agentmux-cef/src/wrr/mod.rs` (or wherever `is_quitting` is read) — read via reducer.
- `agentmux-cef/src/client.rs::on_before_close` — dispatch `BeginDrain` when last user-visible window closes.

### Pool refill effect

When the reducer's `PoolWindowLeft` arm runs and `pool.queue.len() < POOL_TARGET_SIZE`:

```rust
if !state.pool.respawn_in_flight 
   && state.pool.queue.len() < POOL_TARGET_SIZE 
   && state.quit_state == QuitState::Running {
    state.pool.respawn_in_flight = true;
    out.events.push(HostEvent::Effect(EffectKind::SpawnPoolWindow));
}
```

The effect handler (in PR #1's `host_dispatch_with_effects`) executes the imperative CEF call. On `PoolWindowReady`, reducer clears `respawn_in_flight`.

### Tests

- Drag start/end: invariant — at most one `Some` active at a time.
- Pool: spawn → ready → enter queue → promote → leave queue → refill effect emitted (mocked CEF call counts).
- Quit: Running → Draining → Quit transitions; subsequent commands rejected per state.

### Smoke verification

- Drag a tab between two windows. Drag state cleanly registered and cleared.
- Tear off a tab to create a new window. Pool window correctly promoted; refill triggered.
- Close the last window. Drain mode kicks in; no further pool refills.

### Acceptance criteria

- `cargo test` 100% pass.
- 0 remaining `active_drag.lock()` / `window_pool.lock()` / `is_quitting.load()` call sites outside the reducer.
- All existing tear-off + drag tests still pass (smoke-tested manually).

### Bump

Patch version.

### Risk mitigation

- Drag and pool are well-tested code paths today (`tear_off_hook.rs`, `window_pool.rs` have substantial coverage). Migration shouldn't change their semantics.
- Quit state is small (~3 transitions). Easy to exhaustively test.

---

## PR #4 — Top-level Runner + Cross-state Sagas

**Branch:** `agenta/h4-top-level-runner-and-sagas`
**Title:** `feat(cef): event-driven top-level window creation runner + cross-state sagas`
**Effort:** ~3-4 days
**Risk:** High (the actual freeze fix; new CEF callback wiring; cross-state invariants are new territory)

**Depends on:** PR #1 (foundations), PR #2 (panes + browsers in reducer), PR #3 (pool + quit in reducer).

### Goal

The freeze fix. Build the top-level window creation runner using ONLY observable CEF callbacks (no timers, no watchdogs). Add cross-state invariants that the reducer can now enforce (e.g., refuse top-level creation while panes are mid-transition).

### Internal commit sequence

```
commit 1 (H.6.a): wire CEF callbacks — on_after_created, on_render_process_terminated, on_before_close — to dispatch reducer commands
commit 2 (H.6.b): reroute commands/window.rs::open_window_with_kind through reducer (user-initiated; fail-fast)
commit 3 (H.6.c): reroute commands/window_pool.rs::spawn_pool_window through reducer (background; queues)
commit 4 (H.6.d): make post_create_window private; only effect handler calls it
commit 5 (H.6.e): delete legacy direct queueing code paths
commit 6 (H.7): cross-state invariant — reducer rejects EnqueueTopLevelWindow if any pane is in PaneLifecycle::Closing
commit 7 (H.7): NewWindowSaga + PaneCreateSaga (launcher-side sagas observing host events; depends on H.5's wire-promote — note this may need to defer to PR #5 or land minimally)
commit 8: integration tests + smoke
```

### Files touched

- `agentmux-cef/src/client.rs` — add `on_render_process_terminated` callback. Existing `on_after_created` and `on_before_close` updated to dispatch top-level lifecycle commands.
- `agentmux-cef/src/commands/window.rs::open_window_with_kind` — dispatch `EnqueueTopLevelWindow { source: User }`. Return error if reducer rejects (busy in-flight or pane in transition).
- `agentmux-cef/src/commands/window_pool.rs::spawn_pool_window` — dispatch `EnqueueTopLevelWindow { source: Background }`. Reducer queues if busy.
- `agentmux-cef/src/ui_tasks.rs::post_create_window` — visibility narrowed; only `host_dispatch_with_effects` calls it.
- `agentmux-launcher/src/saga/new_window_saga.rs` — new saga (if H.7 fully lands here; otherwise deferred).
- `agentmux-launcher/src/saga/pane_create_saga.rs` — new saga.

### Reducer rules (per granular spec §"Phase H.6")

#### `EnqueueTopLevelWindow { request, source }`:

- If `quit_state != Running` → reject with Error.
- If `source == User` AND `top_level_creation.in_flight.is_some()` → return error `BusyInFlight`. Caller propagates to frontend.
- If `source == User` AND any pane has `lifecycle: Closing` → return error `PaneInTransition`. (Probe; verify it helps in production. May need to be a configurable feature flag.)
- If `source == Background` → enqueue to back. Auto-advance if idle.
- If idle → start immediately. Emit `BeginCreationEffect`.

#### `TopLevelCallbackFired { label }`:

- If matches in-flight → mark `Completed`, push to history, advance queue.
- If doesn't match → close orphan browser via reducer effect (prevent label collision).

#### `TopLevelRendererTerminated { label, status }`:

- If matches in-flight → mark `Failed { reason: RendererTerminated { status } }`, advance queue.

#### `TopLevelExternallyClosed { label }`:

- If matches in-flight → mark `Failed { reason: ExternallyClosed }`, advance queue.

#### **No timer-driven transitions.** No `TopLevelTimeoutTick`. No watchdog.

### Cross-state invariant — H.7 probe

Per the granular Phase H spec §"Phase H.7":

```rust
HostCommand::EnqueueTopLevelWindow { request, source: TopLevelSource::User } => {
    let any_pane_closing = state.panes.values()
        .any(|p| matches!(p.lifecycle, PaneLifecycle::Closing { .. }));
    if any_pane_closing {
        return reject_error("pane mid-transition; retry shortly");
    }
    // ... proceed
}
```

This is the **freeze probe**. The 2026-05-02 freeze investigation identified that opening a top-level window while a browser pane exists triggers a CEF deadlock. This invariant blocks the trigger condition. Verifiable in production immediately after this PR ships.

If the invariant doesn't help (the deadlock is wider than "Closing" panes), relax to "any pane existing in any state" or remove. If it helps, keep.

### Sagas (H.7)

Two sagas added in launcher, observing host events broadcast across IPC. **This requires H.5 (wire-promote) to be partially complete** — specifically, `HostEvent::TopLevelCreationStarted/Completed/Failed` and `HostEvent::PaneLive/Closed/CreationFailed` need to cross IPC. PR #4 may need to include a minimal wire-promote of just these events (the rest waits for PR #5).

```rust
// agentmux-launcher/src/saga/new_window_saga.rs
pub struct NewWindowSaga { creation_id: u64 }
impl Saga for NewWindowSaga {
    fn start(&mut self, _ctx: &SagaCtx) -> SagaAction { SagaAction::Wait }
    fn on_event(&mut self, event: &Event, _ctx: &SagaCtx) -> SagaAction {
        match event {
            Event::HostTopLevelCreationCompleted { creation_id, .. } if *creation_id == self.creation_id => SagaAction::Done,
            Event::HostTopLevelCreationFailed { creation_id, reason, .. } if *creation_id == self.creation_id => SagaAction::Failed { reason: reason.clone() },
            _ => SagaAction::Wait,
        }
    }
    fn name(&self) -> &'static str { "new_window" }
    // No timeout — relies on observable CEF events.
}
```

`PaneCreateSaga` analogous.

### Tests

- Reducer arms for top-level lifecycle: every state transition.
- User-initiated rejection scenarios: busy in-flight, pane in transition, quit draining.
- Background queueing: pool refill chains correctly.
- Orphan-browser handling: `TopLevelCallbackFired` for unknown label closes the orphan.
- Saga completion: `TopLevelCreationCompleted` event terminates `NewWindowSaga::Done`.
- Saga failure: `TopLevelCreationFailed` terminates as `Failed`.

### Smoke verification (CRITICAL — this is the freeze test)

- **Repro the freeze scenario:** Open AgentMux. Add a browser pane block. Open a new top-level window via "open new window". Verify:
  - Either succeeds normally (no deadlock — invariant kicked in or CEF coopered), OR
  - User-initiated request fails fast with visible error (reducer rejected because pane transition).
  - **The host UI thread does NOT freeze.**
- Open multiple windows back-to-back. Each completes or fails-fast in turn.
- Trigger CEF renderer crash mid-creation (kill a child process). Reducer evicts in-flight as `Failed`, queue advances.
- Pool refill: tear off a tab. New pool window created. Pool refill triggered correctly.

### Acceptance criteria

- `cargo test` 100% pass including new saga tests.
- **Freeze repro from 2026-05-02 either does not occur OR fails fast with visible error (no UI hang).**
- Operator can `--diag windows` to see in-flight state (foundation for PR #5).
- 0 remaining direct calls to `post_create_window` outside the reducer effect handler.

### Bump

Minor version bump (e.g., 0.35.0). Major behavior change.

### Risk mitigation

- **This is the freeze fix.** Do extensive manual smoke testing before merging. The bot reviewers will not catch a deadlock — only a human exercise can.
- The H.7 invariant is a probe — log when it fires so we can measure if it's actually blocking the trigger condition.
- If smoke testing reveals the cross-state invariant doesn't fix the freeze, **don't merge with the invariant declared as the fix**. Revisit the diagnosis (probably need a debugger walk of the wedged threads).

### Open question for this PR

Does CEF reliably fire `on_render_process_terminated` for the wedged-renderer case? Per the 2026-05-02 dump, render workers stayed `Responding=True` during the wedge — they weren't dead. If CEF doesn't fire the callback, the in-flight slot stays occupied permanently when wedged. User-initiated creates fail-fast with visible error; pool refill blocks until restart. **Acceptable per the no-timer directive, but worth verifying.**

---

## PR #5 — Durability + Diag + Wire-promote

**Branch:** `agenta/h5-durability-diag-wire-promote`
**Title:** `feat(cef): host reducer durable log + --diag windows + wire-promoted events`
**Effort:** ~2-3 days
**Risk:** Low (additive observability, no semantic change)

**Depends on:** PR #1-#4.

### Goal

Make the host reducer's state inspectable via SQLite log + operator command, and promote selected events to the IPC wire so launcher sagas can subscribe natively.

### Internal commit sequence

```
commit 1 (H.8): HostReducerLog SQLite at <data-dir>/host-reducer.db, schema, append per reducer arm
commit 2 (H.8): recovery walker — mark orphan rows from prior host_pid as failed_recovery
commit 3 (H.8): agentmux --diag windows operator command
commit 4 (H.9): wire-promote remaining HostEvent variants to agentmux-common::ipc::Event
commit 5: integration tests + acceptance smoke
```

### Files added

- `agentmux-cef/src/host_reducer_log.rs` — SQLite wrapper analogous to `agentmux-launcher/src/saga/log/mod.rs`.
- `agentmux-cef/src/host_reducer_recovery.rs` — recovery walker.
- `agentmux-launcher/src/diag.rs` — extend with `--diag windows`.

### Schema (`<data-dir>/host-reducer.db`)

```sql
CREATE TABLE pane_creation (
    seq INTEGER PRIMARY KEY,
    block_id TEXT NOT NULL,
    label TEXT NOT NULL,
    created_at INTEGER NOT NULL,
    closed_at INTEGER,
    final_state TEXT NOT NULL  -- 'live' | 'closed' | 'failed_recovery'
);

CREATE TABLE top_level_creation (
    creation_id INTEGER PRIMARY KEY,
    label TEXT NOT NULL,
    started_at INTEGER NOT NULL,
    finished_at INTEGER,
    outcome TEXT NOT NULL,  -- 'completed' | 'renderer_terminated' | 'externally_closed' | 'failed_recovery'
    failure_reason TEXT,
    host_pid INTEGER NOT NULL,
    host_version TEXT NOT NULL
);

CREATE INDEX idx_pane_started ON pane_creation(created_at);
CREATE INDEX idx_top_level_started ON top_level_creation(started_at);
```

### Recovery walker

```rust
// agentmux-cef/src/host_reducer_recovery.rs
pub fn recover_orphans(log: &HostReducerLog, current_pid: u32) {
    let now_ms = chrono::Utc::now().timestamp_millis();
    log.mark_orphan_panes(now_ms, "failed_recovery");
    log.mark_orphan_top_level(now_ms, "failed_recovery", current_pid);
}
```

Called from `agentmux-cef/src/main.rs` BEFORE the first reducer dispatch.

### `--diag windows`

```
$ agentmux --diag windows

== HOST REDUCER STATE (live) ==
Top-level in-flight: creation_id=42, label=window-abc, started 8s ago, phase=Started
Top-level queued: 2 (next: window-xyz, source=Background)
Pool: 2 ready, 1 unpromoted, refill-in-flight=false
Panes live: 3 (b1, b2, b7)
Panes closing: 0
Quit state: Running

== RECENT TOP-LEVEL CREATIONS (last 20) ==
   ID  | Label                    | Started        | Outcome              | Latency
   ----|--------------------------|----------------|----------------------|--------
   42  | window-abc...            | 12:34:56       | (in-flight)          | ----
   41  | window-pool-...          | 12:34:55       | completed            | 142ms
   40  | window-...               | 12:34:51       | renderer_terminated  | 8902ms
   ...

== RECENT PANE LIFECYCLE (last 20) ==
   Block ID  | Label             | Created    | Closed     | State
   ----------|-------------------|------------|------------|-------
   b7        | browser-pane-b7-3 | 12:34:42   | (live)     | live
   b2        | browser-pane-b2-2 | 12:34:30   | 12:34:48   | closed
   ...
```

Reads from SQLite (works even if host is wedged or has crashed) plus optionally queries the running host's reducer state via a debug IPC.

### Wire-promote (H.9)

Promote these `HostEvent` variants to `agentmux-common::ipc::Event` so they cross IPC:

```rust
Event::HostPaneCreated { block_id, label }
Event::HostPaneClosed { block_id }
Event::HostTopLevelCreationStarted { creation_id, label }
Event::HostTopLevelCreationCompleted { creation_id, label, latency_ms }
Event::HostTopLevelCreationFailed { creation_id, label, reason }
Event::HostQuitDraining
Event::HostQuitReady
```

(Some may already be promoted in PR #4 if needed for sagas; PR #5 ensures the full set.)

### Tests

- Log writes: every reducer arm produces the corresponding SQLite row.
- Recovery walker: simulated crash → restart → orphan rows marked `failed_recovery`.
- `--diag windows`: reads from SQLite + (optionally) live state; renders correctly.

### Smoke verification

- Run app, open + close windows + panes. `--diag windows` shows correct history.
- Force-kill host mid-creation. Restart. `--diag windows` shows the orphan as `failed_recovery`.
- Verify wire-promoted events show up in launcher's broadcast (e.g., a saga subscribed to `HostTopLevelCreationCompleted` fires).

### Acceptance criteria

- `host-reducer.db` exists after first host run.
- Schema correct.
- `--diag windows` works whether host is running or not.
- Wire-promoted events visible in launcher event log.

### Bump

Patch version.

### Risk

Low. Pure observability + wire-promote of already-existing events. No state semantic changes.

---

## Cross-PR concerns

### Backward compatibility

- PRs #1, #3, #5 are internal-only; no wire format changes.
- PR #2 changes the SHAPE of host-internal types (PaneStateMachine deleted) — no wire impact.
- PR #4 adds new wire `Event` variants (for sagas) — older launchers without these recognized just log+skip per existing pattern (`launcher_ipc.rs:269-275`).

### Testing during the multi-PR migration

Between each PR:
- Build a portable. Smoke-test the full app.
- Repro the freeze scenario (open browser pane, open top-level window). Document outcome.
- Tail `~/.agentmux/logs/agentmux-host-v<version>.log.*` for any `[host-reducer]` warnings or errors.

After PR #4 specifically:
- The freeze should either NOT occur or fail-fast with a visible error. If neither, the H.7 invariant probe is wrong and we revisit.

### Decision points during the migration

| After | Decision |
|---|---|
| PR #2 | Confirm 0 `state.browsers.lock()` call sites. If any sneaked in, halt and clean up before PR #3. |
| PR #3 | Confirm tear-off and pool refill still work end-to-end. |
| PR #4 | **Confirm freeze is gone or fails-fast.** If the bug persists despite the cross-state invariant, revisit diagnosis (probably need debugger trace of the wedged threads). |
| PR #5 | Confirm `--diag windows` provides actionable info during a real freeze repro. |

### Rollback per PR

Each PR is independently revertable. Critical: if PR #4 lands and the freeze bug worsens (unlikely, but possible), revert PR #4 alone — PRs #1-#3 stay merged, the runner architecture still exists, just nobody routes through it.

### Configuration knobs (introduced incrementally)

```toml
[host.reducer]
# H.4 (introduced PR #3)
pool_target_size = 2  # default; configurable

# H.6 (introduced PR #4)
top_level_creation_user_fail_fast = true  # default; can be set false to revert to legacy queueing for debugging

# H.7 (introduced PR #4)
refuse_top_level_during_pane_close = true  # the freeze probe; false to disable

# H.8 (introduced PR #5)
host_reducer_log_retention_days = 7
```

---

## Time estimate (per-PR, including bot review)

| PR | Implementation | Tests + smoke | Bot review | Total |
|---|---|---|---|---|
| #1 | 1 day | 1 day | 1-2 rounds | 2-3 days |
| #2 | 3 days | 1 day | 3-5 rounds | 5-7 days |
| #3 | 2 days | 1 day | 2-3 rounds | 3-5 days |
| #4 | 2 days | 1 day (smoke-heavy) | 3-5 rounds | 5-7 days |
| #5 | 2 days | 0.5 day | 2-3 rounds | 3-4 days |

**Total wall-clock: 2.5-3.5 weeks** with bot oscillation. **Active work: ~10 days.**

PR #2 + PR #3 in parallel saves ~3-5 days off the critical path. PRs #4 and #5 are sequential.

---

## What this 5-PR plan does NOT change from the granular Phase H spec

- All architectural directives unchanged (no timers, fail-fast, single source of truth, cross-state invariants).
- All state migrations end at the same destination (full host reducer ownership).
- All sagas defined per the granular spec.
- Durability schema same.
- Failure modes same.

The compression is purely operational — fewer, larger PRs instead of 15-20 small ones. Same end state.

---

## Open questions

1. **Should H.7's pane-closing invariant be configurable?** Spec says yes (config knob). Default-on but operators can disable for debugging.

2. **Does `on_render_process_terminated` actually fire when CEF wedges?** Need to verify in PR #4. If not, accept that wedges leave permanent in-flight state.

3. **Does PR #2's `state.browsers` migration require BOTH read helpers AND saga-style snapshot mutex management?** Probably. The 40+ call sites have varied patterns; some need the reducer state directly (for invariant checks), others just need the browser handle. Read helpers handle the latter; commands handle the former.

4. **Is the 5-PR sequencing optimal vs alternatives?** Considered:
   - **Foundation + freeze fix in PR 1** (combine #1 + #4 minimal) — no, foundation needs to land cleanly first to avoid scaffolding churn.
   - **Smaller PRs, more of them** — this is what the granular spec does (15-20 PRs). User asked for 5.
   - **Skip PR 5** — durability/diag is gravy. Could defer indefinitely. Spec ships it because operator visibility makes future debugging tractable.

5. **What happens to existing PR #650 (already merged)?** Stays. The Phase 1 tracing it added is still useful and lives compatibly with the reducer migration.

---

## Cross-references

- Granular Phase H spec: `docs/specs/SPEC_HOST_REDUCER_PHASE_H_2026-05-02.md`
- Current state catalog: `docs/retro/reducer-architecture-current-state-2026-05-02.md`
- Freeze investigation: `docs/specs/SPEC_WINDOW_FLEET_REDUCER_2026-05-02.md`
- Original Phase F spec: `docs/specs/SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md`
- Saga reducer migration retro: `docs/retro/saga-architecture-migration-complete-2026-05-02.md`

---

End of 5-PR plan.
