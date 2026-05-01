# SPEC: Phase F — host reducer (third reducer in the multi-reducer architecture)

**Date:** 2026-05-01
**Status:** Draft — architectural shape only, no implementation yet.
**Supersedes:** Phase F sketches in `SPEC_PHASE_E_SRV_REDUCER_2026_04_29.md` §13.
**Reads-this-first:**
- `docs/retro/multi-reducer-proposal-2026-04-28.md` — the long-term vision
- `docs/specs/SPEC_PHASE_E_SRV_REDUCER_2026_04_29.md` — Phase E (now mostly shipped)
- `docs/retro/saga-coordinator-location-analysis-2026-04-30.md` — coordinator-placement decisions
- `docs/retro/phase-e-status-2026-05-01.md` — current status
- `docs/retro/b5-migration-architecture-2026-04-28.md` — why some host state CAN'T migrate

---

## 0. The two scoping decisions you need to make before reading this spec

This spec answers "what does Phase F look like" assuming both decisions go in their default direction:

1. **Do we want a host reducer at all?** Default: yes. Phase E proved the multi-reducer pattern works for srv (state correctness, saga coordination, observability). The same gains apply to host's mutable state.
2. **What's the scope of the host reducer?** Default: **the easy parts.** CEF `browsers` map and the warm window pool stay as scaffolding because their CEF-callback access patterns resist the reducer model (see §3). Everything else with a clean lifecycle moves.

If either decision flips, large parts of this spec become stale. **Re-validate both before scheduling implementation.**

---

## 1. Goal

Promote `agentmux-cef`'s remaining mutable state to a Redux-style reducer over the bits that have a clean lifecycle. Mirror the launcher reducer's pattern (`update(&mut state, Cmd, Ctx) -> Vec<Event>`). Pure functional core, no I/O inside the reducer, idempotent apply.

This is the third reducer in the multi-reducer architecture (after launcher in Phase B and srv in Phase E). After Phase F, three reducers communicate via versioned events; the saga coordinator (already shipped, srv-side) gains additional consumers in host.

**End state when this spec ships:**
- Host has a reducer at `agentmux-cef::reducer`
- `host_state.active_drag` / `pending_window_creations` / per-window tear-off hook state route through it
- CEF `browsers` map and warm pool **stay scaffolding** (snapshot-and-drop discipline at lock boundaries — see §3)
- BlockController stays outside reducer entirely (out-of-process state)
- New saga consumers in host (pool-respawn-on-promote, window-cleanup cascade)

---

## 2. Why host last (not first or second)

### 2.1 What's in host state today

| Field | Owner | Migration class |
|---|---|---|
| `browsers: Mutex<HashMap<String, Browser>>` | host | **Scaffolding (forever).** CEF FFI handles; UI-thread sync required by callbacks. |
| Warm window `pool: Mutex<VecDeque<String>>` | host | **Scaffolding (forever).** Pool maintenance triggered by CEF lifecycle callbacks. |
| `pending_window_creations: Mutex<VecDeque<...>>` | host | **Migrate.** Clean lifecycle: enqueue on create call, dequeue in `on_after_created`. |
| `active_drag: Mutex<Option<DragSession>>` | host | **Migrate.** Single owner per gesture, well-defined start/end events. |
| Tear-off hook state per window | host | **Migrate (probably).** Per-window install/uninstall lifecycle. Validate during PR scoping. |
| `pending_window_labels` (legacy) | launcher (since B.5) | Already migrated. |
| `window_meta` | launcher (since B.5) | Already migrated. |
| `instance_registry` | launcher (since B.5) | Already migrated. |
| `host_meta` | host | Stays as synchronous cache (see `b5-migration-architecture-2026-04-28.md`). |
| BlockController (PTY processes, agent-spawned children) | srv | Stays out-of-reducer entirely. Live OS processes resist projection. |

### 2.2 What "host last" means for sequencing

- Phase B already migrated host's WIN32-window-tracking state to the launcher (the easy half).
- Phase E shipped the srv reducer (first multi-reducer validation point — see `phase-e-status-2026-05-01.md`).
- Phase F migrates the **remaining host state with a clean lifecycle**. The CEF FFI / pool callback patterns that resisted Phase B's migration ratchet still resist a reducer pattern; they stay scaffolding indefinitely.

The implication for "when to start Phase F": Phase E.7 phase-exit (proptests + diag) needs to be solid first — Phase F borrows the saga coordinator pattern and reducer testing infrastructure verbatim. Don't start F until E.7 confirms the foundation.

---

## 3. State inventory — what moves into the host reducer

### 3.1 Migrating (the host reducer's domain)

```rust
pub struct HostState {
    /// Cross-window drag session — exactly one in flight at a time.
    /// Replaces today's Mutex<Option<DragSession>>.
    pub active_drag: Option<DragSession>,

    /// Pending window-creation handoffs, FIFO. Replaces today's
    /// Mutex<VecDeque<PendingWindowCreation>>. Enqueued by
    /// renderer-driven calls (open_window_at_position, tear_off_pool_promote);
    /// dequeued in on_after_created (the CEF callback reads via
    /// snapshot-and-drop — see §6).
    pub pending_window_creations: VecDeque<PendingWindowCreation>,

    /// Per-window tear-off hook state. Mirrors what
    /// `agentmux-cef/src/commands/tear_off_hook.rs` tracks today
    /// in module-level Mutexes.
    pub tear_off_hooks: HashMap<String /* window_label */, TearOffHookState>,

    /// Lifecycle phase, mirroring launcher and srv reducers' pattern.
    pub lifecycle: HostLifecyclePhase,

    /// Monotonic event-version counter (per-host-process). Same
    /// invariant as launcher / srv reducers.
    pub event_version: u64,
}
```

### 3.2 NOT migrating (stays scaffolding / out-of-reducer)

- **`browsers: Mutex<HashMap<String, Browser>>`** — CEF Browser FFI handles. Touched by CEF callbacks (`on_after_created`, `on_before_close`) on the UI thread. Can't make those callbacks "dispatch a command and await a reply" without deadlocking the UI thread. **Stays as `Mutex<HashMap>`** with snapshot-and-drop discipline (see §6).
- **Warm pool `VecDeque<String>`** — pool maintenance is triggered by CEF lifecycle callbacks (`on_after_created` for spawn-completion, `on_before_close` for refill-on-promote). Same UI-thread sync constraint. **Stays scaffolding** with snapshot-and-drop.
- **`host_meta`** — synchronous read cache, intentionally NOT a reducer field per Phase B's b5 follow-up.
- **`process_tracker` registry (`AgentProcessRegistry`)** — wraps OS-level Job Object / cgroup handles. Stays out-of-reducer.
- **`BlockController` instances** — live PTY processes + child agent processes. Out-of-reducer (analogous to host's `browsers` for CEF).

### 3.3 Why the snapshot-and-drop pattern survives

Phase E validated that the reducer pattern handles **state with clean event-driven lifecycle** very well. CEF FFI handles aren't that — their lifecycle is dictated by the UI thread, with synchronous demands the reducer can't satisfy.

The snapshot-and-drop pattern (already shipped post-Phase B for `set_pane_overlay_clip`, see `b5-migration-architecture-2026-04-28.md`) is the established way to integrate FFI-handle state with the rest of the system. Phase F doesn't re-litigate this; it confirms it as the *long-term* answer for `browsers` + pool.

---

## 4. Wire protocol additions

### 4.1 New `agentmux-common::ipc::Command` variants from host

```rust
// Drag lifecycle (replaces start_cross_drag / update_cross_drag /
// complete_cross_drag / cancel_cross_drag direct commands).
Command::StartHostDrag { drag_id, source_window, drag_type, payload }
Command::UpdateHostDrag { drag_id, screen_x, screen_y, target_window: Option<String> }
Command::CompleteHostDrag { drag_id, target_window: Option<String>, screen_x, screen_y }
Command::CancelHostDrag { drag_id }

// Pending-window queue (the renderer's window-create RPCs go through
// the reducer instead of touching pending_window_creations directly).
Command::EnqueuePendingWindowCreation { label, kind, parent_instance_id, urls }
Command::DequeuePendingWindowCreation { label }

// Tear-off hook lifecycle (currently scattered across module-level
// state in tear_off_hook.rs).
Command::StartTearOffTracking { source_label, dest_label, ... }
Command::StopTearOffTracking { dest_label }
```

### 4.2 New `Event` variants from host

```rust
Event::HostDragStarted { drag_id, source_window, drag_type, version }
Event::HostDragUpdated { drag_id, target_window: Option<String>, screen_x, screen_y, version }
Event::HostDragCompleted { drag_id, result: "drop" | "tearoff" | "cancel", version }
Event::PendingWindowEnqueued { label, kind, version }
Event::PendingWindowDequeued { label, version }
Event::TearOffTrackingStarted { source_label, dest_label, version }
Event::TearOffTrackingStopped { dest_label, version }
```

These flow on the launcher pipe (host→launcher→subscribers, same as Phase B's `Event::WindowOpened` etc.). The renderer's existing `__agentmux_launcher_event` dispatcher (Phase B.7.3) consumes them.

### 4.3 Source tagging — three reducers now

Phase E added `source: "launcher" | "srv"` tagging. Phase F extends with `"host"`:

```rust
pub enum EventSource { Launcher, Srv, Host }
```

Renderer's per-source version tracking (E.6) gains a third bucket. Saga coordinator's correlation needs a third pipe-target option:

```rust
pub enum PipeTarget { LauncherSelf, Host, Srv }
                                   ^^^^ — already exists in launcher::saga from E.1a;
                                          gains real consumers in F (today it's framework-only)
```

---

## 5. Reducer arms (sketch)

```rust
pub fn update(state: &mut HostState, cmd: Command, ctx: &Ctx) -> Vec<Event> {
    match cmd {
        Command::StartHostDrag { .. } => handle_start_host_drag(state, ...),
        Command::UpdateHostDrag { .. } => handle_update_host_drag(state, ...),
        Command::CompleteHostDrag { .. } => handle_complete_host_drag(state, ...),
        Command::CancelHostDrag { .. } => handle_cancel_host_drag(state, ...),
        Command::EnqueuePendingWindowCreation { .. } => handle_enqueue(...),
        Command::DequeuePendingWindowCreation { .. } => handle_dequeue(...),
        Command::StartTearOffTracking { .. } => handle_start_tearoff(...),
        Command::StopTearOffTracking { .. } => handle_stop_tearoff(...),

        // Misrouted srv / launcher commands return soft errors.
        // Same pattern as launcher's misrouted-srv arms (E.5.x).
        other => misrouted_command_error(state, &other),
    }
}
```

Same invariants apply as launcher / srv reducers: pure functional, sub-millisecond hold of the state mutex, no async, no I/O.

---

## 6. The `browsers` integration — snapshot-and-drop

Browsers is the load-bearing question. Three approaches, with this spec recommending option C:

### Option A: Migrate browsers into the reducer
**Rejected.** CEF callbacks (`on_after_created`, `on_before_close`) need synchronous read-write access on the UI thread. Routing those through the reducer (with `await reply`) deadlocks. This was clarified during Phase E spec review (`SPEC_PHASE_E_SRV_REDUCER_2026_04_29.md` §13).

### Option B: Keep browsers as `Mutex<HashMap>`, ignore the reducer
**Rejected.** Doesn't compose with the rest of host state needing reducer routing — drag callbacks read browsers, hit-test reads browsers. Keeping it siloed forces every consumer to understand "this slice goes through reducer, that slice doesn't" with no clear discipline.

### Option C: Snapshot-and-drop ✓ recommended
- `browsers` stays as `Mutex<HashMap<String, Browser>>` for direct UI-thread access.
- Every read site **takes the lock, copies out only the (label, HWND) pairs it needs, drops the lock**, then does Win32 work.
- Reducer arms that need browser info take a `&BrowserSnapshot` parameter (shape: `Vec<(String, HWND)>` or similar). The dispatch shim takes the snapshot, calls into the reducer.
- A clippy lint or grep CI check catches new code that holds the browsers lock across `SendMessage`, `SetWindowRgn`, `PostMessageW`, etc.

**The example to follow:** `set_pane_overlay_clip` (post-B.5d). The deadlock that motivated this dies because the lock is never held across `SendMessage`, not because the lock disappears.

### Why this is "good enough"
The reducer's value is *consistency of the state machine* + *cross-arm invariants* + *event sourcing*. Browsers is a directory of FFI handles, not a state machine. The snapshot pattern gives consumers a consistent view per call without the reducer overhead.

---

## 7. Saga coordinator usage in Phase F

Phase E shipped a srv-side saga coordinator (Path A from `saga-coordinator-location-analysis-2026-04-30.md`). The launcher-side coordinator from E.1a is currently a labeled stub. **Phase F revives the launcher-side coordinator for cross-process sagas that span launcher + host.**

### 7.1 First cross-process saga: pool-respawn-on-promote

```
saga PoolRespawnOnPromote(promoted_label):
    Step 1 — start: emit Event::PoolWindowPromoted { label: promoted_label }
                    (already exists in Phase B)
    Step 2 — issue Command::SpawnPoolWindow → host
                    wait for: Event::PoolWindowAdded { label: new_label }
    Step 3 — Done

    Compensation: none (failure to refill is logged + retried on next promote)
```

Today this happens implicitly in `window_pool::promote_pool_window` calling `spawn_pool_window` directly. Phase F formalizes it as a saga so the renderer can buffer "you're getting a tear-off + the pool is refilling" atomically.

### 7.2 Second: window-cleanup cascade

```
saga WindowCleanupCascade(closed_label):
    Step 1 — on Event::WindowClosed { label }:
              issue Command::ReapPanes { label } → host
              wait for: Event::PanesReaped { label }
    Step 2 — issue Command::DrainPoolIfLast → host
              wait for: Event::PoolDrained or Event::PoolNotLast
    Step 3 — Done
```

Today this is implicit in `wcore::close_window` + multiple host close-path branches. Phase F makes it a single state machine.

### 7.3 Third (more speculative): drag-window-between-monitors

Already mostly works via Win32 SC_MOVE; might not need a saga. Defer until the tear-off spec (`SPEC_TAB_TEAR_OFF_SIZE_PRESERVATION_2026_04_26.md`) lands and we see what its drag-tracking looks like in practice.

---

## 8. Persistence

Host state is **session-only**. Unlike srv's reducer which persists to SQLite via the persist subscriber, host's reducer state is rebuilt on launcher restart from:
- Reading current CEF `browsers` map (the source of truth for which windows exist).
- Replaying recent `Event::WindowOpened` from the launcher's event log.
- Empty `active_drag` / `pending_window_creations` / `tear_off_hooks` (these are transient).

**Implication:** no Phase F persist subscriber. The reducer state is a session-scoped projection over external sources of truth (`browsers` + launcher event stream).

---

## 9. Sub-PR sequence

Tentative breakdown — re-validate during scoping:

| PR | Scope | Estimate |
|---|---|---|
| **F.1** | Host reducer skeleton — `host_state::HostState` + dispatch table + `update` function. No arms; all commands return misrouted errors. | ~200 LOC |
| **F.2** | `pending_window_creations` arm migration. Lowest-risk: lifecycle is clean, single producer, single consumer. | ~250 LOC |
| **F.3** | Drag arms migration (StartHostDrag / UpdateHostDrag / CompleteHostDrag / CancelHostDrag). Replaces today's `commands/drag.rs::start_cross_drag` etc. | ~400 LOC |
| **F.4** | Tear-off hook arms migration. Couples with the tear-off spec (`SPEC_TAB_TEAR_OFF_SIZE_PRESERVATION_2026_04_26.md`); may be folded into that spec's Phase 2 SC_MOVE work. | ~300 LOC |
| **F.5** | Pool-respawn saga (cross-process: launcher coordinator dispatches to host). First real consumer of the launcher saga coordinator. | ~250 LOC |
| **F.6** | Window-cleanup cascade saga. Subsumes today's implicit cascade in `wcore::close_window` + host close paths. | ~300 LOC |
| **F.7** | Cleanup audit + property tests for host reducer arms (mirrors E.7). | ~400 LOC |

**Total: ~2100 LOC across 7 PRs.** Spread over multi-week effort.

---

## 10. Open questions / non-goals

### 10.1 Open

- **Tear-off hook arms (F.4) — coupling with the tear-off spec.** The Chrome-faithful tear-off spec (`SPEC_TAB_TEAR_OFF_SIZE_PRESERVATION_2026_04_26.md`, Phases 2-7 unstarted) replaces today's `tear_off_hook.rs` machinery. F.4 should follow OR fold into that spec's Phase 2. Decide during scoping.
- **Saga coordinator placement for cross-launcher-host sagas.** Phase E went srv-side per Path A analysis. Phase F's pool-respawn + window-cleanup sagas span launcher↔host. Default: revive the launcher coordinator from E.1a (already framework-stub'd). Validate during F.5 scoping.
- **Process-tracker registry.** Currently outside the reducer. Could migrate read paths but not the OS-handle ownership. Open question for F.7 cleanup.

### 10.2 Non-goals

- **Migrating CEF `browsers` map into the reducer** — explicitly NOT.
- **Migrating warm pool into the reducer** — explicitly NOT.
- **Migrating BlockController** — out-of-reducer indefinitely.
- **Dropping snapshot-and-drop discipline** — still required for everything in §3.2.
- **Phase G event-sourced** — separate spec; conditional on Phase E + F validating end-to-end.

---

## 11. Risk register

| Risk | Severity | Mitigation |
|---|---|---|
| `browsers` snapshot-and-drop discipline regresses (new code holds lock across `SendMessage`) | High | Clippy lint or CI grep-test catching the pattern. Already-shipped `set_pane_overlay_clip` is the canonical example. |
| Cross-process saga (launcher→host) hits IPC latency that's user-visible | Medium | Saga timeout (5 s default per E.1a). If pool-respawn saga regularly hits timeout, the warm pool's first-paint guarantee breaks; add per-saga p99 telemetry in F.5. |
| Tear-off hook migration breaks the tear-off spec's Phase 2 implementation | Medium | Pause F.4 until tear-off Phase 2 is well-scoped; either fold it into the tear-off spec's PR sequence, or rebase tear-off Phase 2 onto F.4. |
| Reducer migration introduces visible UX regressions on drag / pool / window-create | Medium | Each F.x PR ships behind smoke tests covering the affected user flow. |
| Phase F is implemented before Phase E.7 lands → no proptests, no diag tools, harder to debug | Medium | Block F.1 on E.7 phase-exit (proptests + `--diag srv` + `--diag host` if added). |

---

## 12. What this spec does NOT close (carried-forward gaps)

Same gaps as Phase E (per `saga-coordinator-location-analysis-2026-04-30.md` §6.7):
- Per-step SQLite transactions in subscriber — addressed by F1.A in Phase E. Doesn't apply to host (no persist subscriber).
- Saga state durability across srv crash — same Phase G+ deferral as before.
- Renderer-side registration as saga step — Phase F's window-cleanup cascade may surface this; defer to that PR.

---

## 13. Cross-references

- `docs/specs/SPEC_PHASE_E_SRV_REDUCER_2026_04_29.md` §13 — Phase F preview (this spec supersedes).
- `docs/retro/multi-reducer-proposal-2026-04-28.md` — long-form vision.
- `docs/retro/saga-coordinator-location-analysis-2026-04-30.md` — coordinator placement reasoning.
- `docs/retro/b5-migration-architecture-2026-04-28.md` — why `browsers` resists the ratchet.
- `docs/specs/SPEC_TAB_TEAR_OFF_SIZE_PRESERVATION_2026_04_26.md` — Phase 2-7 unstarted; relevant to F.4.
- `docs/retro/phase-e-status-2026-05-01.md` — current Phase E status.
- `docs/retro/next-steps-2026-05-01.md` — forward plan listing this as deferred.

---

## 14. History

- **2026-05-01** initial draft — staking out the architecture, no implementation. Open questions in §10.
