# SPEC: Host Reducer Buildout — Phase H

**Date:** 2026-05-02
**Status:** Draft — implementation plan, replaces PR #651
**Author:** AgentA
**Branch base:** `main` HEAD `48ad4e58`

**Companion docs:**
- `docs/retro/reducer-architecture-current-state-2026-05-02.md` — verified current-state catalog (what IS in reducers, what isn't)
- `docs/specs/SPEC_WINDOW_FLEET_REDUCER_2026-05-02.md` — 2026-05-02 freeze investigation, dump analysis, why per-window-isolated RequestContext + browser panes triggers it
- `docs/specs/SPEC_HOST_WINDOW_CREATION_RUNNER_2026-05-02.md` — superseded by this spec; PR #651 to be closed
- `docs/specs/SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md` — original F.1 spec; this is its successor
- `docs/retro/migration-pattern.md` — the a→b→c→d→e ratchet used in B.5 and reused here

---

## TL;DR

Build out the host reducer to own all host-side state currently held in raw `Mutex` / `RwLock` / `AtomicBool` / global statics: pane lifecycle, the `state.browsers` CEF handle map, drag state, tear-off hook state, pool windows, top-level window creation. Use the same a→b→c→d→e migration ratchet that retired the host-side `WindowInstanceRegistry` in Phase B.5.

**Architectural directives from the 2026-05-02 freeze investigation:**

1. **NO TIMERS. NO WATCHDOGS.** Reducer state transitions react only to observable signals (CEF callbacks, OS events, user actions). If a signal never fires, the reducer state holds permanently — failure becomes visible (operator can `--diag windows` and see a stuck state) rather than silently absorbed by a deadline.
2. **Fail-fast for user-initiated operations.** When an operation cannot proceed because reducer state forbids it (e.g., busy in-flight creation), return an error to the caller instead of queueing silently. Background operations (pool refill) may queue; user-facing ones return errors.
3. **Single source of truth.** Every host-side state field is owned by exactly one reducer arm. No parallel mirrors except the read-only `shadow_*` projections of launcher state that already exist.
4. **Cross-state invariants are first-class.** The reducer is the place to declare "operation X is forbidden while state Y is in flux." Today such invariants are invisible — buried in lock orderings and ad-hoc checks.

This is a multi-phase, multi-week effort comparable in scope to the saga reducer migration shipped in PRs #641-#649. **PR #651 is closed in favor of this plan.**

---

## Why this supersedes PR #651

PR #651 added a top-level window creation runner with a 30-second watchdog. It addressed a real symptom (the host UI thread freezing under concurrent CEF browser creation) by serializing creation with a deadline-based eviction.

**Three problems with that approach, surfaced by 2026-05-02 testing:**

1. **The watchdog is brittle.** Verified user critique: timers are stand-ins for "I don't know if this will resolve." When the deadline fires, the runner evicts the in-flight slot but the wedged CEF browser keeps existing — its `on_after_created` callback can fire later and collide with a different creation, producing the "closed wrong window" symptom.
2. **The runner doesn't address the root trigger.** User-confirmed: the freeze does NOT reproduce when no browser pane exists. With a pane present, opening a top-level window deadlocks two CEF threads in `EventPairLow` + `Unknown` lock-wait. The runner serializes top-level creation but doesn't coordinate with pane state, so the trigger condition still arises.
3. **The runner is a one-off architectural addition.** It introduces in-flight slot semantics specific to one operation. Other operations (drag, tear-off, pool refill, pane lifecycle) remain in raw mutexes. No consistency, no replay, no `--diag` visibility for the rest.

The right fix is **the full host reducer.** PR #651's runner code becomes one phase of this larger plan, simplified to remove the watchdog and rely on observable CEF signals instead.

---

## Scope summary

**In scope (covered by phases below):**
- Pane lifecycle (replaces `BrowserPaneManager::PaneStateMachine`)
- `state.browsers` CEF handle registry
- `active_drag` state
- `window_pool`, `unpromoted_pool_labels`, `window_pool_respawn_in_flight`
- `is_quitting`
- Top-level window creation runner (event-driven, no watchdog)
- Cross-state invariants (refuse operations during incompatible in-flight states)
- Cross-state sagas (`NewWindowSaga`, `PaneCreateSaga`)
- Durability (`HostReducerLog` SQLite + recovery walker + `--diag windows`)
- Wire-promote of selected events for launcher saga subscribers

**Out of scope:**
- Win32 subclass scaffolding (`PANE_WNDPROCS`, `PANE_HWND_CONTEXT`, `ALLOW_PANE_FOCUS_ONCE`, `PANE_REDIRECT_LAST_AT`) — these are CEF-callback-internal mechanics, not application state. Stay in their current global statics.
- WRR position-debounce + Win32 hook handles — same reasoning.
- Shadow projections (`shadow_instance_registry`, `shadow_backend_window_ids`, `shadow_window_meta`) — already correctly NOT in host reducer; launcher is authoritative.
- IPC config / sidecar handles / auth key / zoom factor — set-once values, no reducer benefit.

**Estimated scope:** 15-20 PRs over 2-3 weeks of wall-clock with bot review oscillation. Comparable to PRs #641-#649 (saga reducer migration: 9 PRs, ~5 hours wall-clock + days of bot iteration).

---

## Migration ratchet — recap of B.5

Each piece of state migrates through five steps. This is the proven pattern from B.5 (which retired host-side `WindowInstanceRegistry` in favor of launcher's `instance_registry`).

| Step | Behavior | Read source | Write source |
|---|---|---|---|
| **a** — parallel writes | Both old field and new reducer state mutated on every change. Reads still hit old field. | Old | Both |
| **b** — flip reads (with fallback) | Reads prefer reducer; fall back to old field if reducer is empty. Drift logged. | Reducer (with fallback) | Both |
| **c** — flip reads (no fallback) | Reads only consult reducer. Old field still mutated for safety net. | Reducer | Both |
| **d** — drop old writes | Old field becomes vestigial. Only reducer is mutated. | Reducer | Reducer only |
| **e** — delete old field | Old field removed from `AppState`. PR is largely deletions. | Reducer | Reducer |

**Each step is a separate PR.** Bots review each independently. If step c reveals drift, step d doesn't ship until drift is zero.

---

## Phase H.0 — Foundations

**Goal:** Add reducer scaffolding for ALL the new state at once, even though the migrations land sequentially. This avoids per-phase reducer-skeleton churn.

**State additions to `HostState`:**

```rust
pub struct HostState {
    // F.1 (existing):
    pub pending_window_creations: VecDeque<PendingWindowCreation>,
    pub lifecycle: HostLifecyclePhase,
    pub event_version: u64,
    
    // H — pane lifecycle (replaces PaneStateMachine):
    pub panes: HashMap<String /* block_id */, PaneEntry>,
    
    // H — CEF browser handle registry (replaces state.browsers):
    pub browsers: HashMap<String /* label */, BrowserHandle>,
    
    // H — drag state (replaces active_drag):
    pub active_drag: Option<DragSession>,
    
    // H — pool state (replaces window_pool + unpromoted_pool_labels + atomic):
    pub pool: PoolState,
    
    // H — top-level creation runner (PR #651 idea, sans watchdog):
    pub top_level_creation: TopLevelCreationState,
    
    // H — quit state (replaces is_quitting AtomicBool):
    pub quit_state: QuitState,
}

pub enum PaneLifecycle {
    Live,                              // pane alive, accepting operations
    Closing { since: Instant },        // close requested, awaiting CEF teardown
    // No `Closed` variant — entry is removed from map on close-completion event
}

pub struct PaneEntry {
    pub block_id: String,
    pub label: String,
    pub lifecycle: PaneLifecycle,
}

pub struct BrowserHandle {
    pub label: String,
    pub browser: cef::Browser,         // the FFI handle (Clone, refcounted)
    pub kind: BrowserKind,             // TopLevel | Pane
    pub registered_at: Instant,
}

pub enum BrowserKind {
    TopLevel { is_pool: bool },
    Pane { block_id: String },
}

pub struct PoolState {
    pub queue: VecDeque<String>,        // labels of fully-ready pool windows
    pub unpromoted: HashSet<String>,    // labels spawned but not yet promoted
    pub respawn_in_flight: bool,        // single-flight semaphore for refill
}

pub struct TopLevelCreationState {
    pub queue: VecDeque<TopLevelCreationRequest>,
    pub in_flight: Option<InFlightCreation>,
    pub history: VecDeque<CompletedCreation>,
    pub next_creation_id: u64,
}

pub struct InFlightCreation {
    pub creation_id: u64,
    pub label: String,
    pub started_at: Instant,
    pub phase: CreationPhase,
    // NO `deadline` field. NO watchdog.
}

pub enum CreationPhase {
    Started,                  // BeginCreationEffect emitted
    BrowserCallbackFired,     // CEF on_after_created fired
    RendererProcessTerminated,// CEF on_render_process_terminated fired (failure path)
}

pub enum QuitState {
    Running,
    Draining { reason: QuitReason },
    Quit,
}
```

**Action additions to `HostCommand`:** see per-phase sections.

**Event additions to `HostEvent`:** see per-phase sections. Some events promoted to `agentmux-common::ipc::Event` for cross-process subscribers (Phase H.9).

**Test additions:** unit tests for each new arm. Proptests for invariants (singleton in-flight, FIFO ordering, no orphan transitions).

**Effort:** ~1 day. One PR, low risk (no behavior change — fields exist but no callers yet).

**Acceptance:** all type definitions compile. Reducer dispatch covers all new commands. No existing behavior changes.

---

## Phase H.1 — Pane lifecycle into reducer

The `BrowserPaneManager::PaneStateMachine` is the smallest cohesive piece to migrate first. Doing this first also unblocks the "no top-level creation while pane in transition" invariant that (per the freeze diagnosis) might prevent the deadlock entirely.

### Commands

```rust
HostCommand::EnqueuePaneCreate { block_id: String, label: String }
HostCommand::CompletePaneCreate { block_id: String }     // after CEF on_after_created for pane
HostCommand::EnqueuePaneClose { block_id: String }
HostCommand::CompletePaneClose { block_id: String }      // after CEF on_before_close for pane
HostCommand::AbortPaneCreate { block_id: String, reason: String }  // CEF callback never fired
```

### Events

```rust
HostEvent::PaneCreateRequested { block_id, label, version }
HostEvent::PaneLive { block_id, label, version }
HostEvent::PaneClosing { block_id, version }
HostEvent::PaneClosed { block_id, version }
HostEvent::PaneCreationFailed { block_id, reason, version }
```

### Reducer rules

- `EnqueuePaneCreate` → if block_id already in map, reject with Error event. Else insert with `Live` (or new `Pending` if we want to track create-in-flight). Emit `PaneCreateRequested`.
- `CompletePaneCreate` → confirm transition to `Live` (no-op if already `Live`).
- `EnqueuePaneClose` → flip to `Closing { since: now }`. Emit `PaneClosing`.
- `CompletePaneClose` → remove from map. Emit `PaneClosed`.
- `AbortPaneCreate` → remove from map (no entry to clean up if create never completed).

### Migration steps (a→e)

| Step | Description |
|---|---|
| H.1.a | Wire `PaneStateMachine::try_register_live` etc. to ALSO dispatch `EnqueuePaneCreate` to the reducer. Existing PaneStateMachine continues to be authoritative; reducer state mirrors. |
| H.1.b | `BrowserPaneManager` reads (e.g., `defocus_all` iterating live panes) prefer reducer; fall back to PaneStateMachine. Log drift target=`pane-reducer:drift`. |
| H.1.c | Reads switch to reducer-only. PaneStateMachine still writes. |
| H.1.d | Drop PaneStateMachine writes. `try_register_live` etc. become thin shims around reducer dispatch. |
| H.1.e | Delete `PaneStateMachine` struct, `PaneEntry`, etc. `BrowserPaneManager` becomes a stateless utility around reducer queries + CEF FFI. |

**Per-step PRs:** 5. Each is small (~50-200 lines). Each ships independently.

---

## Phase H.2 — `state.browsers` into reducer

This is the highest-traffic mutex in the host (40+ lock sites). Migrating it consolidates all CEF handle access through the reducer.

The CEF `Browser` handle is `Clone` (refcounted) and `Send`-via-Mutex. Storing it in `HostState` (which lives in `parking_lot::Mutex<HostState>`) is safe.

### Commands

```rust
HostCommand::RegisterBrowser { label: String, browser: cef::Browser, kind: BrowserKind }
HostCommand::UnregisterBrowser { label: String }
```

### Events

```rust
HostEvent::BrowserRegistered { label, kind, version }
HostEvent::BrowserUnregistered { label, version }
```

### Reducer queries (read-only, snapshot-and-drop)

```rust
// On AppState — bypass `host_dispatch` for reads (no event emission needed).
impl AppState {
    pub fn get_browser(&self, label: &str) -> Option<cef::Browser> {
        self.host_state.lock().browsers.get(label).map(|b| b.browser.clone())
    }
    pub fn list_browsers(&self) -> Vec<(String, BrowserKind)> {
        self.host_state.lock().browsers.iter()
            .map(|(k, v)| (k.clone(), v.kind.clone())).collect()
    }
    pub fn browser_kind(&self, label: &str) -> Option<BrowserKind> {
        self.host_state.lock().browsers.get(label).map(|b| b.kind.clone())
    }
}
```

### Migration steps (a→e)

The 40+ existing `state.browsers.lock()` call sites split into two patterns:
1. **Read-only** (~30 sites): `browsers.lock().get(&label).cloned()` → use `state.get_browser(label)` or `state.list_browsers()`.
2. **Mutating** (~10 sites): `browsers.lock().insert(...)` / `.remove(...)` → use `host_dispatch(RegisterBrowser/UnregisterBrowser)`.

| Step | Description |
|---|---|
| H.2.a | Add reducer arms; mirror writes (when `state.browsers.insert` happens, also dispatch `RegisterBrowser`). |
| H.2.b | Add read helpers on `AppState`. Convert read sites to use them, falling back to `state.browsers.lock()` if reducer is empty. Log drift. |
| H.2.c | Convert read sites to reducer-only (no fallback). |
| H.2.d | Convert write sites: `state.browsers.lock().insert(...)` → `host_dispatch(RegisterBrowser)`. Reducer effect performs the insert into `state.browsers` for now (still mirrored). |
| H.2.e | Make `state.browsers` either go away or become an internal field of `HostState.browsers` (it already is — phase out the standalone field). |

**Per-step PRs:** 5. Some larger (the H.2.b read-conversion touches many files).

**Risk:** medium — large surface area. Mitigation: do read sites first (H.2.b/c), defer write sites until reads are stable.

---

## Phase H.3 — Drag state

`active_drag: Mutex<Option<DragSession>>` (state.rs:302) becomes a reducer field. Drag is naturally a state machine — start, in-flight, end/cancel.

### Commands

```rust
HostCommand::StartDrag { drag_id, drag_type, source_window, source_workspace_id, source_tab_id, payload }
HostCommand::EndDrag { drag_id, outcome: DragOutcome }
```

### Reducer rules

- `StartDrag` while `active_drag.is_some()` → reject (only one drag at a time).
- `EndDrag` matching active drag's `drag_id` → clear, emit `DragEnded`.

### Migration steps

Standard a→e. ~3-4 PRs (drag is touched in fewer places than `state.browsers`).

---

## Phase H.4 — Pool state

`window_pool`, `unpromoted_pool_labels`, `window_pool_respawn_in_flight` collapse into `HostState.pool: PoolState`.

This phase is **important for the freeze investigation** because pool refill is one of the operations whose ordering relative to top-level creation matters. With pool in the reducer, "no pool refill while top-level creation in flight" becomes a declarable invariant.

### Commands

```rust
HostCommand::PoolWindowSpawnStart { label }
HostCommand::PoolWindowReady { label }              // renderer-ready signal
HostCommand::PoolWindowDestroyedBeforePromote { label }
HostCommand::PromotePoolWindow { /* fields per existing promote */ }
HostCommand::PoolDrainAll                           // shutdown
```

### Events

```rust
HostEvent::PoolWindowEntered { label, queue_len_after }
HostEvent::PoolWindowLeft { label, queue_len_after, reason: PoolLeaveReason }
HostEvent::PoolEmpty                                // for sagas to react
```

### Reducer effect: pool refill

When a pool window leaves (promote / destroy-before-promote / shutdown drain), the reducer checks: should we refill?

```rust
// In reducer's PoolWindowLeft handler:
if !state.pool.respawn_in_flight 
   && state.pool.queue.len() < POOL_TARGET_SIZE 
   && state.quit_state == QuitState::Running {
    state.pool.respawn_in_flight = true;
    out.events.push(HostEvent::Effect(EffectKind::SpawnPoolWindow));
}
```

Effect handler runs `commands::window_pool::spawn_pool_window_now` (the imperative CEF call). On `PoolWindowReady`, reducer clears `respawn_in_flight`.

### Migration steps

a→e for each of the three fields. ~5-6 PRs.

---

## Phase H.5 — Quit state

`is_quitting: AtomicBool` becomes `HostState.quit_state: QuitState`. Three states: Running → Draining → Quit.

### Commands

```rust
HostCommand::BeginDrain { reason: QuitReason }
HostCommand::ConfirmDrained                          // pool empty + browsers empty
```

### Reducer rules

- `BeginDrain` from `Running` → transition to `Draining`, emit pool-drain effects.
- `ConfirmDrained` from `Draining` → transition to `Quit`, emit `HostQuitReady`.
- All operations that today read `is_quitting` (e.g., `spawn_pool_window`, top-level creation) gate on `quit_state != Running`.

**~2 PRs.** Small and contained.

---

## Phase H.6 — Top-level window creation runner (event-driven, no watchdog)

This is PR #651's idea, redesigned per the no-timer directive.

### State (already declared in H.0)

`HostState.top_level_creation` with queue, in-flight slot, history.

### Commands

```rust
HostCommand::EnqueueTopLevelWindow { request: TopLevelCreationRequest }
HostCommand::TopLevelCallbackFired { label }         // from CEF on_after_created
HostCommand::TopLevelRendererTerminated { label, status }   // from CEF on_render_process_terminated
HostCommand::TopLevelExternallyClosed { label }      // from CEF on_before_close (cancel mid-create)
```

### Events

```rust
HostEvent::TopLevelCreationRequested { creation_id, request, version }
HostEvent::TopLevelCreationStarted { creation_id, label, version }
HostEvent::TopLevelCreationCompleted { creation_id, label, latency_ms, version }
HostEvent::TopLevelCreationFailed { creation_id, label, reason, version }
HostEvent::TopLevelQueueLengthChanged { len, version }
```

### Reducer rules

- `EnqueueTopLevelWindow`:
  - If `quit_state != Running` → reject with Error.
  - If `top_level_creation.in_flight.is_some()` and request is **user-initiated** → return error (fail-fast). Caller (e.g., `commands/window.rs::open_window_with_kind`) propagates error to frontend.
  - If `in_flight.is_some()` and request is **background** (pool refill) → enqueue to back of queue.
  - If `in_flight.is_none()` → start immediately. Emit `BeginCreationEffect`.

- `TopLevelCallbackFired`:
  - If matches in-flight label → advance phase to `BrowserCallbackFired`. **This is the SUCCESS signal.** No deadline involved. Mark completed, push to history, advance queue.
  - If doesn't match in-flight (e.g., a pool window registering, or an orphan from a prior failed attempt) → close the orphan browser via reducer effect to prevent label collision.

- `TopLevelRendererTerminated`:
  - If matches in-flight label → fail in-flight with `Reason::RendererTerminated { status }`. Push to history. Advance queue.
  - If doesn't match → log diagnostic. Browser is gone; nothing to do.

- `TopLevelExternallyClosed`:
  - If matches in-flight label → fail in-flight with `Reason::ExternallyClosed`. Advance queue.

**No timer-driven transitions.** If neither `TopLevelCallbackFired`, `TopLevelRendererTerminated`, nor `TopLevelExternallyClosed` fires for an in-flight request, the slot stays occupied. New user-initiated creates fail-fast with a visible error. Pool refill blocks (acceptable — user can still operate the existing window).

### Effect handler

`BeginCreationEffect` → calls `ui_tasks::post_create_window` (existing). The reducer's effect dispatch is the ONLY place that calls `post_create_window` after this phase lands.

### Failure modes

| Scenario | What happens |
|---|---|
| CEF `on_after_created` fires within ms | Normal — completed, queue advances |
| CEF renderer crashes during init | `on_render_process_terminated` fires → failed, queue advances |
| User closes the still-loading window | `on_before_close` fires → failed, queue advances |
| CEF wedges with no callback ever firing | In-flight slot stays occupied permanently. User-initiated creates fail-fast with error. `agentmux --diag windows` shows the stuck creation, operator can decide to restart |
| User opens 5 windows rapidly | First starts immediately. Subsequent 4 fail-fast with "AgentMux is busy creating a window — try again in a moment." |
| Pool refill triggered while user create in-flight | Pool request queues (background). Runs after user create completes |

### CEF callback wiring

These hooks must exist or be added in `agentmux-cef/src/client.rs`:
- `on_after_created` (already exists — line 130) → dispatch `TopLevelCallbackFired { label }` if label is top-level (not pane).
- `on_render_process_terminated` → dispatch `TopLevelRendererTerminated { label, status }`. **NEW HOOK** — needs to be implemented if not present.
- `on_before_close` (already exists) → if label has an in-flight creation, dispatch `TopLevelExternallyClosed { label }`.

### Migration steps

| Step | Description |
|---|---|
| H.6.a | Add reducer arms + effect handler. Add CEF callback wiring. Existing `post_create_window` call sites unchanged — they still call directly. |
| H.6.b | Convert `commands/window.rs::open_window_with_kind` to dispatch `EnqueueTopLevelWindow`. Pool refill and tear-off paths still call `post_create_window` directly. Drift logged. |
| H.6.c | Convert `commands/window_pool.rs::spawn_pool_window` to dispatch `EnqueueTopLevelWindow` (background flag). |
| H.6.d | Make `post_create_window` private; only the reducer effect handler calls it. |
| H.6.e | Delete legacy queueing code paths. |

**~5 PRs.**

---

## Phase H.7 — Cross-state sagas

Now that pane state, browser state, and top-level creation are all in the reducer, we can express invariants spanning them.

### `NewWindowSaga`

Triggered by: `HostEvent::TopLevelCreationRequested`.

Runs in the launcher (where the saga coordinator lives — see Phase H.9 for wire-promote of relevant events).

```
trigger: TopLevelCreationRequested { creation_id, request }
state: AwaitingCallback
on_event: 
  TopLevelCreationCompleted { creation_id } → Done
  TopLevelCreationFailed { creation_id, reason } → Failed
compensation: none (reducer already cleaned up via TopLevelCreationFailed)
```

### `PaneCreateSaga`

Triggered by: `HostEvent::PaneCreateRequested`.

```
trigger: PaneCreateRequested { block_id, label }
state: AwaitingPaneLive
on_event:
  PaneLive { block_id } → Done
  PaneCreationFailed { block_id, reason } → Failed
```

### Cross-state invariant: refuse new top-level creation while a pane is in `Closing`

Hypothesis to test (per freeze diagnosis): the deadlock is triggered when a top-level window is being created while a pane is in CEF's renderer-init / browser-init lifecycle. Adding a reducer rule:

```rust
HostCommand::EnqueueTopLevelWindow { request } if user_initiated => {
    let any_pane_in_transition = state.panes.values()
        .any(|p| matches!(p.lifecycle, PaneLifecycle::Closing { .. }));
    if any_pane_in_transition {
        return reject_error("pane mid-transition; retry shortly");
    }
    // ... proceed
}
```

This is a **probe** — not a guaranteed fix. The actual deadlock surface needs verification via the trace data we get from H.6 in production. If the rule doesn't help, we relax it. If it does, it stays.

### Migration steps

H.7 requires H.1, H.2, H.6 to be complete. Lands as 1-2 PRs once those are stable.

---

## Phase H.8 — Durability + `--diag windows`

Mirror the LSD-1/LSD-3 pattern from PRs #641-#647.

### `HostReducerLog` SQLite

`<data-dir>/host-reducer.db`. Schema captures the lifecycle of each reducer-managed entity:

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
```

### Recovery walker

On host startup, mark any `outcome IS NULL` rows from a previous host_pid as `failed_recovery`. Mirrors LSD-3 from PR #647.

### `agentmux --diag windows`

Read-only access to the host-reducer log + live state if host is running. Shows:
- Recent pane creations (last 50, with state)
- Recent top-level creations (last 50, with phase and latency)
- Current in-flight top-level (if any) with how long it's been stuck
- Pool state (queue length, unpromoted count, respawn-in-flight)

**~3 PRs.** Adds value once H.6 is shipped (no point earlier).

---

## Phase H.9 — Wire-promote selected events

Some host reducer events should cross IPC for launcher saga subscribers. Today host events are intentionally host-internal (F.1 design). Phase H.9 promotes a curated subset.

### Events to promote (host → launcher broadcast)

```rust
// Promote into agentmux-common::ipc::Event::
HostPaneCreated { block_id, label }
HostPaneClosed { block_id }
HostTopLevelCreationStarted { creation_id, label }
HostTopLevelCreationCompleted { creation_id, label, latency_ms }
HostTopLevelCreationFailed { creation_id, label, reason }
HostQuitDraining
HostQuitReady
```

These let launcher sagas (existing or new) subscribe to host lifecycle without polling.

### Effect on existing host-internal events

Most host events stay host-internal (they're noise outside the host process). Only the ones above get promoted.

**~2 PRs.** Coordinated with launcher saga work that wants to consume them.

---

## Saga tests

Each new saga gets:
- Unit tests of the saga state machine (pure)
- Integration tests against the host reducer (no CEF in the loop)
- Proptests where applicable

Same testing discipline as PRs #641-#649.

---

## Rollout sequencing

```
H.0 (foundations)
 ├─ H.1 (panes)        ──┐
 ├─ H.2 (browsers map) ──┤
 ├─ H.3 (drag)         ──┼── independent, can land in any order
 ├─ H.4 (pool)         ──┤
 └─ H.5 (quit)         ──┘
       └─→ H.6 (top-level runner — depends on H.4 quit + H.0 state)
            └─→ H.7 (cross-state sagas — depends on H.1, H.6)
                 └─→ H.8 (durability + --diag windows)
                      └─→ H.9 (wire-promote events)
```

H.0 first (one PR, ~1 day). Then H.1-H.5 in parallel (each is independent of the others — different mutex). H.6 after H.4 + H.5 (needs quit state + pool state). H.7 needs H.1 and H.6 stable. H.8 and H.9 are gravy at the end.

Total: **15-20 PRs over 2-3 weeks**, mirroring the saga reducer migration shipped in PRs #641-#649.

---

## Observability throughout the migration

Each phase adds events to `HostEvent`. Each event is logged via `state.rs::log_host_event` (the existing observability hook). This means:

- Even before H.8 ships, every reducer-mediated state change is in the host log
- Operators can correlate freeze symptoms with reducer state (which pane is `Closing`? which top-level creation is in-flight?)
- The freeze investigation that drove this whole spec gets a reproducible answer rather than a dump-walking exercise

---

## What this does NOT solve

Honest list of things this spec doesn't fix:

1. **The CEF v146 underlying wedge.** If CEF's `window_create_top_level` deadlocks because of a renderer-process LPC race (the hypothesis from the 2026-05-02 trace), the reducer makes the failure observable and recoverable but doesn't make CEF un-deadlock. We rely on observable signals (renderer terminated, etc.) to escape. If CEF gives no signal, in-flight stays stuck — we trade silence for visible failure, not failure for success.

2. **Existing Win32 subclass scaffolding.** `PANE_WNDPROCS`, `PANE_HWND_CONTEXT`, `ALLOW_PANE_FOCUS_ONCE`, `PANE_REDIRECT_LAST_AT` stay as global statics. They're CEF-callback mechanics, not application state.

3. **Backend persistence.** `WaveStore` (objects.db) remains separate. The host reducer is session state; persistent workspace data is in srv.

4. **Frontend state.** Solid stores in the renderer process are unaffected. They're a different layer.

5. **Cross-process atomicity.** The host reducer is in-process. Cross-process operations (e.g., "is the launcher still alive when I dispatch this command?") still rely on the launcher↔host pipe semantics.

---

## Open questions

1. **Does the H.7 invariant ("refuse top-level create while pane closing") actually fix the freeze?** Won't know until H.6 ships and we observe. Worst case: invariant doesn't help, we relax it. Best case: deadlock disappears.

2. **Does CEF reliably fire `on_render_process_terminated` for a wedged renderer?** Per the 2026-05-02 dump, render workers stayed `Responding=True` during the wedge — they weren't dead. So `on_render_process_terminated` may not fire. If true, the only escape from a wedge is operator-initiated restart. Acceptable per the no-timer directive.

3. **Should `state.browsers` migration be H.2 or H.6?** Doing it earlier consolidates more code through reducer mediation. Doing it later defers the highest-risk migration. Spec proposes earlier (H.2) — revisit if H.1 surfaces unexpected complexity.

4. **Does the wire-promote in H.9 introduce launcher dependencies on host reducer events?** If yes, those become tightly coupled. Proposed: launcher sagas subscribe to specific events but don't require them (saga's `on_event` is a passive observer; missing events stall the saga without crash). This is the same shape as existing PoolRespawn / WindowCleanupCascade.

5. **What happens to PR #651's smoke-test data and 16 unit tests?** Tests are reusable for H.6 (the reducer logic is similar; the watchdog code becomes deletion). Smoke data (the trace from 0.33.582 testing, dump from 0.33.580) is preserved in `docs/specs/SPEC_WINDOW_FLEET_REDUCER_2026-05-02.md` for reference.

---

## Decision log

| Date | Decision | Rationale |
|---|---|---|
| 2026-05-02 | NO timers, NO watchdogs in any reducer arm. | User directive: brittleness of arbitrary deadlines + orphan-browser symptom from PR #651 watchdog. |
| 2026-05-02 | Fail-fast for user-initiated operations under contention. | Visible error > silent queueing. Pool refill (background) may queue. |
| 2026-05-02 | Migrate state via the proven a→b→c→d→e ratchet from B.5. | Same shape that successfully retired host-side `WindowInstanceRegistry`. Each step is a separate PR. |
| 2026-05-02 | Close PR #651 in favor of this plan. | The runner is one piece of a larger consistent reducer; shipping it standalone with watchdog creates orphans. |
| 2026-05-02 | Host reducer events stay host-internal except a curated subset (H.9). | Most host state is irrelevant outside the host process. Curated wire-promote keeps the IPC surface small. |

---

End of spec.
