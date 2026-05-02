# Reducer Architecture — Current State Report

**Date:** 2026-05-02
**Branch reflected:** `main` HEAD `48ad4e58` (post-PR #650 merge)
**Author:** AgentA
**Scope:** factual catalog of current reducer + saga architecture and state that lives outside it. No speculation; every claim cites file:line in source.

**In-flight not reflected in this report:**
- PR #651 (`feat(cef): host reducer-driven top-level window creation runner — Phase 2`) — open, not merged. Adds `top_level_creation_queue`, `in_flight_top_level_creation`, watchdog. Not on main.

---

## Three reducers — one per process

AgentMux has three reducers, one per process, modeled after the same pattern: pure-functional `update(&mut State, Cmd) -> events`, snapshot-and-drop mutex discipline, monotonic `event_version`, lifecycle-phase gating.

| Process | Reducer state | Reducer fn | Mounted on |
|---|---|---|---|
| Launcher | `agentmux-launcher/src/state.rs:130` (`State`) | `agentmux-launcher/src/reducer.rs:79` (`update`) | `Arc<Mutex<State>>` |
| Srv | `agentmux-srv/src/state.rs:119` (`State`) | `agentmux-srv/src/reducer.rs:41` (`update`) | `Arc<Mutex<State>>` |
| Host (CEF) | `agentmux-cef/src/reducer.rs:52` (`HostState`) | `agentmux-cef/src/reducer.rs:170` (`update`) | `AppState.host_state: parking_lot::Mutex<HostState>` (state.rs:234) |

---

## Launcher reducer

### State (`State` struct, `agentmux-launcher/src/state.rs:130-200`)

| Field | Type | Purpose |
|---|---|---|
| `lifecycle` | `LifecyclePhase` | Bootstrap → Running → Quitting → Dead |
| `processes` | `HashMap<u32, ProcessRecord>` | PID → process metadata |
| `windows` | `HashMap<String, WindowMirror>` | Phase B.4 — label → top-level window mirror (kind, parent_label) |
| `pool` | `HashSet<String>` | pool window labels |
| `instance_registry` | `HashMap<String, u32>` | Phase B.5 — label → instance number |
| `next_instance_num` | `u32` | starts at 2; "main" pre-seeded as 1 |
| `backend_window_ids` | `HashMap<String, String>` | Phase B.5 step a — label → backend window id |
| `event_version` | `u64` | monotonic event counter |
| `next_client_id` | `u64` | client id allocator |
| `monitors` | `Vec<Rect>` | Phase B.9.1 WRR — monitor topology |
| `pending_hwnds` | `HashMap<u64, PendingHwnd>` | Phase B.9.1 WRR — HWND → pending metadata |

### Commands (subset of `agentmux-common::ipc::Command` handled by launcher)

All wire-serializable (cross-process IPC). Cited at `reducer.rs`:

`Register`, `Goodbye`, `Ping` (lifecycle); `ReportWindowOpened`, `ReportWindowClosed`, `ReportPoolWindowAdded`, `ReportPoolWindowRemoved`, `ReportPoolWindowPromoted` (host→launcher mirror updates); `ReportHwndOpened`, `ReportHwndPositionChanged`, `ReportHwndForegroundChanged`, `ReportHwndVisibilityChanged`, `ReportHwndIconicChanged`, `ReportHwndDestroyed`, `ReportMonitorTopologyChanged`, `ReportWindowHwndsReleased` (Phase B.9.1 WRR mirror); plus saga-bound reports (`ReportPanesReaped`, `ReportPoolDrainDecision`, `ReportSagaActionFailed`).

### Events (subset of `agentmux-common::ipc::Event` emitted by launcher)

All wire-serializable. Broadcast to subscribers via tokio broadcast channel.

`ProcessSpawned`, `LifecyclePhaseChanged`, `Registered`, `Pong`, `ProcessExited` (lifecycle); `WindowOpened`, `WindowClosed` (B.4); `PoolWindowAdded`, `PoolWindowRemoved`, `PoolWindowPromoted` (B.5); `WindowInstanceAssigned`, `WindowInstanceReleased` (B.5e); `BackendWindowIdRegistered`, `BackendWindowIdUnregistered` (B.5 step a); `HwndDriftClassified` (B.9.1 WRR); plus saga lifecycle events (`SagaStarted`, `SagaCompleted`, `SagaFailed`).

### Lifecycle gating

`Bootstrap → Running` on first Register (`reducer.rs:178-189`). Quitting placeholder per B.9.3.

### Durability

- **No SQLite for reducer state.** Window mirror, instance registry, etc. are session-only — bootstrapped empty on launcher start, fed by host reports.
- **Saga durability is separate** — see saga section below.

---

## Srv reducer

### State (`State` struct, `agentmux-srv/src/state.rs:119-149`)

| Field | Type | Purpose |
|---|---|---|
| `lifecycle` | `LifecyclePhase` | Same shape as launcher |
| `processes` | `HashMap<u32, ProcessRecord>` | client process metadata |
| `event_version` | `u64` | monotonic event counter |
| `next_client_id` | `u64` | client id allocator |
| `workspaces` | `HashMap<String, WorkspaceRecord>` | E.2 — workspace_id → workspace |
| `tabs` | `HashMap<String, TabRecord>` | E.2b — tab_id → tab (with `block_ids: Vec<String>` ordered, `focused_node_id`, `magnified_node_id`) |
| `blocks` | `HashMap<String, BlockRecord>` | E.3 — block_id → block |
| `windows` | `HashMap<String, WindowRecord>` | E.5 — window_id ↔ workspace mapping |

### Commands

Wire-serializable. From `reducer.rs:43-83`:

Lifecycle: `Register`, `Goodbye`, `Ping`, `GetSrvSnapshot`.

Workspace/tab/block: `CreateWorkspace`, `DeleteWorkspace`, `CreateTab`, `DeleteTab`, `SetActiveTab`, `ReorderTab`, `CreateBlock`, `DeleteBlock`, `SetFocusedNode` (E.4), `SetMagnifiedNode` (E.4), `RenameWorkspace`, `RenameTab`, `UpdateWorkspaceMeta`, `UpdateTabMeta`, `UpdateBlockMeta`, `MoveTab`, `MoveBlock`, `ReorderTabsBulk`.

Window-binding: `CreateWindow` (E.5), `CloseWindowInternal` (E.5), `SwitchWorkspace` (E.5).

### Events

Wire-serializable. From `reducer.rs`:

`WorkspaceCreated/Deleted`, `TabCreated/Deleted/ActiveChanged/Reordered`, `BlockCreated/Deleted`, `FocusedNodeChanged`, `MagnifiedNodeChanged`, `WindowCreated/Closed/WorkspaceSwitched`, plus generic `ProcessSpawned`/`Registered`/`Pong`/`ProcessExited`/`LifecyclePhaseChanged`.

### Durability

- **Reducer state is session-only.** Workspace/tab/block durability is in `WaveStore` (objects.db SQLite, see `reference_persistence_files.md` memory) — that's a *separate* persistence layer, not the reducer.
- **Saga log SQLite at `~/.agentmux/sagas.db`** (`agentmux-srv/src/sagas/log.rs:11`) — see saga section.

---

## Host reducer (CEF process)

### State (`HostState`, `agentmux-cef/src/reducer.rs:52-76`)

| Field | Type | Purpose |
|---|---|---|
| `pending_window_creations` | `VecDeque<PendingWindowCreation>` | F.1 — FIFO pre-create handoff: caller pushes label/kind/parent_instance_id; `client::on_after_created` pops |
| `lifecycle` | `HostLifecyclePhase` | Bootstrapping → Running → ShuttingDown |
| `event_version` | `u64` | monotonic event counter |

That is **the entire host reducer state today on main.** All other host-side state (CEF browser handles, pane lifecycle, drag state, pool windows, focus state, etc.) lives outside the reducer — see "State NOT in any reducer" below.

### Commands (`HostCommand`, `reducer.rs:101-113`)

**Host-internal only — not wire-serializable.** F.1 comment (`reducer.rs:22-28`) explicitly states host commands do not cross IPC.

- `EnqueuePendingWindowCreation { entry }`
- `DequeuePendingWindowCreation`

### Events (`HostEvent`, `reducer.rs:123-149`)

**Host-internal only — not wire-serializable.**

- `PendingWindowEnqueued { label, queue_len_after, version }`
- `PendingWindowDequeued { label, queue_len_after, version }`
- `PendingWindowQueueEmpty { version }`
- `Error { message, version }`

### Durability

**None.** Session-only in-memory state.

---

## Saga coordinator (launcher only)

Lives in `agentmux-launcher/src/saga/mod.rs`. Subscribes to launcher event broadcast, runs in-flight sagas, dispatches saga actions.

### Saga trait (`saga/mod.rs:180-197`)

```rust
pub trait Saga: Send {
    fn start(&mut self, ctx: &SagaCtx) -> SagaAction;
    fn on_event(&mut self, event: &Event, ctx: &SagaCtx) -> SagaAction;
    fn name(&self) -> &'static str;
    fn input_snapshot(&self) -> serde_json::Value { Value::Null }
    fn timeout(&self) -> Duration { Duration::from_secs(5) }
}
```

### `SagaAction` (`saga/mod.rs:130-149`)

```rust
enum SagaAction {
    IssueCmd { target: PipeTarget, cmd: Command },  // dispatch and stay in-flight
    Done,                                            // saga succeeded
    Failed { reason: String },                       // saga failed (CPD-3 host-send-error)
    Wait,                                            // wait for next event
}
```

`PipeTarget` (`saga/mod.rs:112-116`): `LauncherSelf`, `Host`, `Srv`. F.5+ dispatches to `Host` are LIVE via `HostPipe::send_command()`.

### Active sagas — verified at `agentmux-launcher/src/saga/mod.rs:835-852`

```rust
fn match_trigger(event: &Event) -> Option<Box<dyn Saga>> {
    match event {
        Event::PoolWindowPromoted { label, .. } =>
            Some(Box::new(pool_respawn::PoolRespawn::new(label.clone()))),
        Event::WindowClosed { label, crash_detected: false, .. } =>
            Some(Box::new(window_cleanup::WindowCleanupCascade::new(label.clone()))),
        _ => None,
    }
}
```

**Two and only two sagas exist:**

1. **`pool_respawn::PoolRespawn`** (`saga/pool_respawn.rs`)
   - Trigger: `Event::PoolWindowPromoted`
   - Actions: `IssueCmd { target: Host, cmd: Command::SpawnPoolWindow }`
   - Termination: `Event::PoolWindowAdded` → `Done`
   - Compensation: none

2. **`window_cleanup::WindowCleanupCascade`** (`saga/window_cleanup.rs`)
   - Trigger: `Event::WindowClosed { crash_detected: false }` (clean closes only — crash-detected closes skip the cascade per codex P1 PR #637)
   - Actions: `IssueCmd { target: Host, cmd: Command::ReapPanes }`, then `IssueCmd { target: Host, cmd: Command::DrainPoolIfLast }`
   - Termination: after both reports received → `Done`
   - Compensation: none (cleanup is idempotent)

### Cross-process dispatch (CPD-1 to CPD-5, all LIVE)

| Item | Where | Status |
|---|---|---|
| Launcher saga sends `Command` to host | `HostPipe::send_command()` in `agentmux-launcher/src/host_pipe/mod.rs` | LIVE |
| `HostFrame::{Event, Command}` envelope | wire format | LIVE |
| Host receives command, dispatches via `LiveActionRunner` | `agentmux-cef/src/saga_dispatch.rs:331-352` | LIVE |
| Per-saga `claim_terminal` atomic guard | `saga/mod.rs` | LIVE |
| `HostPipe::cancel_saga(saga_id)` purges pending buffer on terminal | `host_pipe/mod.rs` | LIVE |
| `host_session_id` generation prevents stale fanout | `host_pipe/mod.rs` | LIVE |
| Host-side idempotency LRU (256 entries) | `agentmux-cef/src/saga_dispatch.rs::SagaIdempotencyLru` | LIVE |
| Per-saga event correlation via `saga_id` | event payloads carry `saga_id` (CPD-4) | LIVE |
| `Saga::timeout()` — default 5s, F.6 overrides 30s | `saga/mod.rs:200-202` | LIVE |

### Saga durability (LSD-1 to LSD-4, all LIVE)

**`LauncherSagaLog`** at `~/.agentmux/launcher-sagas.db` (`agentmux-launcher/src/saga/log/mod.rs:8-9`):

| Capability | File:line | Status |
|---|---|---|
| Schema: `launcher_saga` (saga_id, name, state, started_at, ended_at, failure_reason, input_json) + `launcher_saga_step` | `saga/log/schema.rs` | LIVE |
| WAL mode + 5s busy timeout + foreign_keys=ON | `saga/log/mod.rs` | LIVE |
| `start_saga` / `terminate_saga` / `start_step` / `finish_step` / `fail_step` | `saga/log/mod.rs:214-350` | LIVE |
| `next_saga_id` seeded from `max_saga_id() + 1` on startup | `saga/log/mod.rs` | LIVE (LSD-1) |
| Recovery walker — marks unresolved sagas `failed_compensation` on restart, preserves original `failure_reason` | `agentmux-launcher/src/saga/recovery.rs` | LIVE (LSD-3, PR #647) |
| Startup retention vacuum (default 7 days, configurable) | `saga/log/mod.rs` | LIVE (LSD-4, PR #646) |
| `--diag sagas` operator command (read-only via `LauncherSagaLog::open_read_only`) | `agentmux-launcher/src/diag.rs` | LIVE (LSD-3) |
| `--diag sagas` runs BEFORE CEF runtime check | `agentmux-launcher/src/main.rs` | LIVE |

**`SagaLog`** at `~/.agentmux/sagas.db` (`agentmux-srv/src/sagas/log.rs:11`):

Per-srv saga log. Same schema shape as launcher's. Used by srv-internal sagas (none active today; reserved for E-class).

---

## State NOT in any reducer

**This is the operational gap.** Every entry below is a piece of mutable state currently protected by a raw `Mutex` / `RwLock` / `AtomicBool` / `OnceLock<Mutex<...>>`, mutated outside the reducer's `update()` function, with no event log, no replay, no `--diag` visibility, no cross-state invariant enforcement.

Order: most freeze-relevant first.

### Host (CEF) — pane and browser state (the freeze surface)

| State | File:line | Type | Why it's not in reducer | Mutated at |
|---|---|---|---|---|
| `state.browsers` | `agentmux-cef/src/state.rs:210` | `Mutex<HashMap<String, Browser>>` | FFI handle to Chromium browser; can't serialize over IPC. Locked directly across the codebase (40+ call sites — see grep `state\.browsers\.lock`). | `client.rs::on_after_created` (insert), `pane/callbacks.rs` (remove), `commands/drag.rs`, `commands/window.rs`, `commands/window_pool.rs` |
| `BrowserPaneManager::PaneStateMachine` | `agentmux-cef/src/pane/lifecycle.rs:60-62` | own internal `Mutex<HashMap<String, PaneEntry>>` | Independent state machine with `PaneLifecycle::{Live, Closing}`; never integrated into host reducer | `try_register_live` / `try_mark_closing` / `remove` / `drain_by_label` |
| `ALLOW_PANE_FOCUS_ONCE` | `agentmux-cef/src/pane/hwnd.rs:71-72` | global `AtomicBool` | Win32 WM_SETFOCUS subclass flag; mutated from CEF callbacks + pane WndProc hook | `commands/window.rs::focus_pane` (set), pane WndProc (load/swap) |
| `PANE_WNDPROCS` | `agentmux-cef/src/pane/hwnd.rs:24-26` | global `Mutex<HashMap<usize, isize>>` | HWND → original WndProc, for subclass delegation | `install_pane_focus_redirect` (insert), pane WndProc cleanup |
| `PANE_HWND_CONTEXT` | `agentmux-cef/src/pane/hwnd.rs:39-41` | global `Mutex<HashMap<usize, PaneContext>>` | Pane HWND → context (state Arc + block_id). Lookup target for the WndProc hook. | `install_pane_focus_redirect`, `remove_contexts_for_block` |
| `PANE_REDIRECT_LAST_AT` | `agentmux-cef/src/pane/hwnd.rs:81-83` | global `Mutex<HashMap<usize, Instant>>` | Per-root rate-limit map for focus redirects (PR #650 freeze fix) | `should_redirect_pane_focus_to_root` |
| `active_drag` | `agentmux-cef/src/state.rs:302` | `Mutex<Option<DragSession>>` | Cross-window drag session (singleton). Spec §3.3 says Phase F.3 will retire. | `commands/drag.rs` start/end |
| `window_pool` | `agentmux-cef/src/state.rs:252` | `Mutex<VecDeque<String>>` | Pre-warmed hidden window queue. Spec §3.2 says NOT migrating — needs synchronous host-local checks. | `commands/window_pool.rs::spawn_pool_window` (push), `promote_pool_window` (pop) |
| `unpromoted_pool_labels` | `agentmux-cef/src/state.rs:266` | `Mutex<HashSet<String>>` | Labels of pool windows spawned but not yet promoted. | `spawn_pool_window` (insert), `promote_pool_window` (remove), destroy-before-promote |
| `window_pool_respawn_in_flight` | `agentmux-cef/src/state.rs:272` | `AtomicBool` | Single-flight semaphore for pool refill. | `spawn_pool_window` swap |
| `is_quitting` | `agentmux-cef/src/state.rs:281` | `AtomicBool` | "Last user-visible window is closing" flag. Cross-window read site. | `on_before_close` (set), `spawn_pool_window` (read guard) |

### Host (CEF) — Win32 / WRR scaffolding

| State | File:line | Type | Notes |
|---|---|---|---|
| `HOOK_HANDLES` | `agentmux-cef/src/wrr/win_event.rs` | `OnceLock<Mutex<Vec<HookHandle>>>` | Win32 SetWinEventHook handles |
| `POSITION_DEBOUNCE_MAP` | `agentmux-cef/src/wrr/position_debounce.rs` | `OnceLock<Mutex<HashMap<u64, Instant>>>` | Per-HWND debounce timestamps for ReportHwndPositionChanged |

### Host (CEF) — shadow projections of launcher state

These are read-only projections fed by launcher `Event` subscriptions (`launcher_ipc::apply_event_to_shadow`). Host code never mutates these directly; they're not in the host reducer because the launcher reducer is authoritative.

| State | File:line | Source event |
|---|---|---|
| `shadow_instance_registry` | `state.rs:161` | `WindowInstanceAssigned` / `WindowInstanceReleased` |
| `shadow_backend_window_ids` | `state.rs:171` | `BackendWindowIdRegistered` / `BackendWindowIdUnregistered` |
| `shadow_window_meta` | `state.rs:183` | `WindowOpened` / `WindowClosed` |

### Host (CEF) — config and IPC caches

These are setup-time values, not really "reducible" — included for completeness.

| State | File:line | Notes |
|---|---|---|
| `auth_key`, `backend_endpoints`, `sidecar_child`, `backend_pid`, `backend_started_at`, `zoom_factor`, `client_id`, `window_id`, `active_tab_id`, `window_init_status`, `version_data_dir`, `version_config_dir`, `user_home_dir`, `debug_port` | `state.rs:122-316` | All `Mutex<...>` in AppState — set once at startup or rarely. Low value to migrate. |
| `browser_api: BrowserApiState` | `state.rs:309` | CDP target cache + connection pool internals |

### Launcher — none

All launcher state is in the reducer's `State` struct. The only persistence outside is the saga log SQLite — which is the saga coordinator's concern, not "state not in reducer."

### Srv — none functional

All workspace/tab/block/window state is in the reducer's `State` struct. The objects.db (`WaveStore`) persistence is *separate* — it's the durable storage layer that the reducer's session state is rehydrated from on boot, but that's a different concern from "is this in the reducer."

---

## Wire serialization summary

- **All launcher reducer events** are in `agentmux-common::ipc::Event` → wire-serializable, broadcast to subscribers.
- **All srv reducer events** are in `agentmux-common::ipc::Event` → wire-serializable.
- **All host reducer events** (`HostEvent` in `agentmux-cef/src/reducer.rs:123`) are **NOT** in `agentmux-common::ipc` → host-internal only by design (F.1 comment, `reducer.rs:22-28`).
- **Cross-process saga commands** (`SpawnPoolWindow`, `ReapPanes`, `DrainPoolIfLast`) ARE in `agentmux-common::ipc::Command` → wire-serializable. CPD-3 dispatches them launcher→host via `HostPipe::send_command`.

---

## Phase migration roadmap as of 2026-05-02 main

Drawn from the spec headers, plus the actual state of `match_trigger` and reducer struct contents:

| Phase | Item | Status |
|---|---|---|
| B.4 | Launcher window mirror | LIVE |
| B.5 (a→b→c→d→e) | Window meta migration to launcher | LIVE |
| B.5e | Sequential instance numbers in launcher | LIVE |
| B.7 | Client launcher event bridge | LIVE |
| B.9.1 | WRR (Window Reality Reconciliation) — launcher mirror of OS HWND topology | LIVE |
| B.9.3 | WRR drain mode → quit propagation | LIVE |
| E (whole) | Srv reducer (workspaces, tabs, blocks, windows) | LIVE |
| E.2c | Saga durability for srv (`sagas.db`) | LIVE |
| F.1 | Host reducer (`pending_window_creations` only) | LIVE |
| F.3 | Drag arms in host reducer | DEFERRED |
| F.4 | Tear-off hook arms in host reducer | DEFERRED |
| F.5 | Launcher saga coordinator + cross-process saga dispatch | LIVE (PRs #629-#640) |
| F.6 | `WindowCleanupCascade` saga | LIVE (PR #635) |
| F.7 | Host reducer proptests | LIVE (PR #640) |
| LSD-1 | Launcher saga log foundation | LIVE (PR #641) |
| LSD-2 | Coordinator wiring to LauncherSagaLog | LIVE (PR #645) |
| LSD-3 | Recovery walker + `--diag sagas` | LIVE (PR #647) |
| LSD-4 | Retention vacuum | LIVE (PR #646) |
| CPD-1 | Saga_id schema for host commands | LIVE (PR #643) |
| CPD-2 | HostPipe wrapper with reconnect + buffer | LIVE (PR #642) |
| CPD-3 | Wire saga dispatch — F.5 stops being narrator | LIVE (PR #644) |
| CPD-4 | Per-saga event correlation | LIVE (PR #648) |
| CPD-5 | Host-side saga_id LRU + HostFrame parser | LIVE (PR #649) |
| Phase 2 (ad-hoc, this session) | Host reducer-driven top-level window-create runner | **OPEN PR #651, NOT MERGED** |

---

## Implications for the 2026-05-02 freeze investigation

The freeze reproduces deterministically when:
- A browser pane (a `defwidget@browser` block) exists in any window
- The user opens a new top-level window (TearOff or "open new window")

User-confirmed: **without browser panes, the app is stable**. The wedge fingerprint is two AboveNormal CEF threads in `EventPairLow` + `Unknown` lock-wait, after CEF's `on_after_created` log line.

**What the reducer architecture does NOT cover today (relevant to this bug):**

1. **Browser pane lifecycle** is in `PaneStateMachine` (own state, own mutex) — invisible to the host reducer. The reducer cannot enforce "no pane operations during top-level creation" because pane state isn't reducer-mediated.

2. **`state.browsers`** is a raw `Mutex<HashMap>` containing both top-level browsers AND pane browsers. The lock is taken at 40+ call sites including some that hold across CEF FFI calls. This is the lock the wedged UI thread is waiting on at `client.rs:149` per the Phase 1 trace data.

3. **No central event log** combining pane lifecycle + top-level lifecycle + drag state + pool state. When the freeze happens, there's no replayable record of "what state was the host in 200ms before the wedge."

4. **No saga supervising compound pane operations.** Pane create + load + focus runs as imperative CEF calls. A saga with timeout + compensation could detect and recover from a partial-state pane.

5. **Cross-state invariants are not declarable.** Today's reducer arms are independent. There's no place to express "the host reducer rejects a top-level creation request while a pane is in `PaneLifecycle::Live` with non-zero in-flight focus operations."

**What integrating panes into the host reducer would buy:**

- Single ordered event log: `PaneCreated`, `PaneFocused`, `PaneClosed`, `TopLevelEnqueued`, `TopLevelStarted`, `TopLevelCallback`, etc., interleaved with monotonic `event_version`. Replayable via `--diag windows` + a durable log analogous to LSD-1.
- Reducer rule: refuse `EnqueueTopLevelWindow` while any pane is mid-transition (or, more nuanced, signal to the runner to wait until panes are quiesced).
- Saga model for pane operations: `PaneCreateSaga`, with timeout and compensation. The same pattern PRs #641-#649 shipped for cross-process work.
- Operator visibility: `--diag panes` reads the same SQLite the reducer writes to, even when the host is wedged or has crashed.

**Cost estimate:** comparable in scope to E-class (srv reducer migration) — Phase F.6 in the existing host reducer spec was the placeholder for pane state but was deferred indefinitely. To do it well: ~2-3 weeks of phased work mirroring B.5's a→b→c→d→e ratchet (parallel writes → flip reads → drop writes → delete legacy field).

---

## What's missing from this report

This is a structural catalog, not a behavioral one. It does not document:

- Operational behavior of in-flight sagas (how often they fire, latency distributions)
- The actual correctness invariants each reducer enforces (only their type-system invariants — what `apply()` actually returns)
- Drift between intent (specs) and implementation (this is hinted at in some spec references but not exhaustively audited)

For those, the relevant docs are `docs/specs/SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md`, `docs/retro/saga-architecture-migration-complete-2026-05-02.md`, and individual saga PR descriptions.

---

End of report.
