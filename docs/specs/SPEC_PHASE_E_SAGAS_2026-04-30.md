# Phase E Sagas — Full Specification

**Date:** 2026-04-30
**Status:** Specification (not yet implemented)
**Supersedes the "Option D register-after" plan in** `docs/retro/phase-e-tear-off-and-remaining-2026-04-30.md`.
**Replaces spec §7 of** `docs/specs/SPEC_PHASE_E_SRV_REDUCER_2026_04_29.md` **with concrete per-saga state machines.**

---

## 1. Goal

Eliminate the reducer/wcore inconsistency window by routing **every state mutation** through the srv reducer. The remaining offenders (audit table in `phase-e-tear-off-and-remaining-2026-04-30.md` §3) are mostly **multi-entity multi-step operations** — tear-off creates a workspace + window + moves a tab; restore moves a tab back + deletes the orphan workspace. These can't be modeled as single pure-functional reducer commands without compromising the reducer's "one mutation pass, no I/O" invariant.

Sagas exist exactly for this shape. The E.1a saga coordinator is already in place (`agentmux-launcher::saga`); this spec describes the per-operation saga implementations and the RPC integration that sits in front of them.

**End state when this spec ships:** the reducer is the sole writer of workspace / tab / block / layout state. Every wcore-direct path in service.rs §3 either becomes a reducer-dispatch handler (single-step) or a saga-driven RPC handler (multi-step). The persist subscriber writes SQLite for everything.

## 2. Status quo

Already shipped (Phase E up to #618):

| | |
|---|---|
| E.1a | Saga coordinator framework: `Saga` trait, `SagaCoordinator` task, `SagaStarted`/`SagaCompleted`/`SagaFailed` events. **No saga implementations yet.** |
| E.1b | Srv reducer skeleton + new srv pipe + broadcast bus + event log |
| E.2 | Workspace lifecycle arms (CreateWorkspace / DeleteWorkspace) |
| E.2b | Tab lifecycle arms (CreateTab / DeleteTab / SetActiveTab) |
| E.3 | Block lifecycle arms (CreateBlock / DeleteBlock) |
| E.2c.1 | Persist subscriber plumbing (idempotent SQLite write-back) |
| E.2c.2 | Workspace RPC migrated through reducer; full-resync-on-lag |
| E.2c.3 / E.2c.3b | Tab RPC migrated (CreateTab + SetActiveTab + CloseTab + ReorderTab) |
| E.2c.4 | Block RPC migrated (CreateBlock + DeleteBlock with meta) |
| E.2c.5a | Rust host bridge to srv pipe (forwards events to renderer) |

What remains, ordered by dependency:

| | |
|---|---|
| **E.2c.5b** | TypeScript renderer dispatcher (`window.__agentmux_srv_event` + atom routing) |
| **E.2c.6 (hot-fix)** | Lazy-load (Option A) for the tear-off → CreateTab regression — ships in parallel as the smoke unblock |
| **E.4** | Layout state — minimal slice (focused/magnified node tracking) |
| **E.5 (this spec)** | Sagas + atomic single-step migrations for everything in audit §3 |
| **E.6** | Renderer multi-source consumption + saga buffering |
| **E.7** | Property tests + integration tests + `--diag` Tools |

## 3. The remaining problem (recap)

14 wcore-direct paths in `agentmux-srv/src/server/service.rs`. Categorized by shape:

### 3a. Multi-step (saga shape)
- `TearOffTab` — create new workspace + new window + move tab to new workspace
- `TearOffBlock` — create new workspace + new window + new tab + move block to new tab
- `RestoreTornOffTab` — move tab back to source workspace + delete now-orphan workspace + close torn-off window
- `MoveTabToWorkspace` — remove tab from src workspace's tab_ids + add to dst workspace's tab_ids + update activetabid in both if affected
- `MoveBlockToTab` (cross-workspace) — remove block from src tab's block_ids + add to dst tab's block_ids + update block.parentoref
- `CreateWindow` — create new workspace + create window pointing at it
- `CloseWindow` — close window + (conditionally) cascade-delete orphan workspace if no other window references it

### 3b. Single-step (atomic command, no saga)
- `PromoteBlockToTab` — within one workspace: create new tab + move block to it + update parents
- `UpdateTabIds` — overwrite workspace.tabids wholesale
- `UpdateWorkspace` — rename workspace
- `UpdateTabName` — rename tab
- `UpdateObjectMeta` — meta merge on workspace/tab/block
- `UpdateObject` — generic wave-obj update (catch-all)
- `SwitchWorkspace` — change which workspace a window points at

### 3c. Already-saga (or trivially equivalent)
None yet.

---

## 4. Saga design

### 4.1 Coordinator contract (recap from E.1a)

```rust
pub trait Saga: Send + 'static {
    /// Unique saga identity. Carried on all commands/events the
    /// coordinator dispatches on behalf of this saga so subscribers
    /// can correlate.
    fn saga_id(&self) -> u64;

    /// Called once when the coordinator starts the saga. Returns
    /// the initial set of Commands to dispatch into the reducer.
    /// Empty Vec means "wait for an external event before doing anything"
    /// (rare; most sagas have an immediate first step).
    fn start(&mut self) -> Vec<Command>;

    /// Called by the coordinator for every Event the reducer emits
    /// while this saga is in flight. Returns either:
    ///   - Vec<Command> to dispatch next (continues the saga)
    ///   - SagaStep::Completed { result } to mark success
    ///   - SagaStep::Failed { reason } to mark failure (coordinator
    ///     emits SagaFailed; sub-spec defines compensation)
    ///
    /// IMPORTANT: must NOT mutate reducer state directly; only
    /// returns commands the coordinator will dispatch.
    fn on_event(&mut self, event: &Event) -> SagaStep;
}

pub enum SagaStep {
    Continue(Vec<Command>),
    Completed { result: serde_json::Value },
    Failed { reason: String },
}
```

The coordinator:
- Owns `Vec<Box<dyn Saga>>` — active sagas
- Subscribes to the broadcast bus
- For each Event, calls `saga.on_event(event)` for every active saga; processes each saga's returned commands (dispatches via `reducer::update`)
- Emits `Event::SagaStarted { saga_id, kind }` when a saga is registered
- Emits `Event::SagaCompleted { saga_id, result }` or `Event::SagaFailed { saga_id, reason }` when a saga's `on_event` returns a terminal `SagaStep`

### 4.2 Saga correlation IDs on commands/events

Every `Command` and every `Event` gains an optional `saga_id: Option<u64>` field. The coordinator sets this to `Some(saga.saga_id())` on commands it dispatches; the reducer copies it through to emitted events. Subscribers (renderer especially) use it to:

- **Group events** belonging to the same logical operation.
- **Buffer-until-complete** so the renderer applies cross-entity changes atomically rather than mid-flight (E.6).
- **Filter `--diag` output** to see one saga's full lifecycle.

`saga_id: None` = "not part of a saga" (the vast majority of events from RPC-direct dispatches).

### 4.3 Compensation semantics

When a saga's step N fails, the saga's `on_event` returns `SagaStep::Failed { reason }`. The coordinator emits `SagaFailed` and calls a separate `Saga::compensate(&mut self) -> Vec<Command>` method (see expanded trait below) to roll back already-applied steps. Compensating commands are dispatched in REVERSE order of the original steps.

**Expanded trait:**
```rust
pub trait Saga: Send + 'static {
    fn saga_id(&self) -> u64;
    fn start(&mut self) -> Vec<Command>;
    fn on_event(&mut self, event: &Event) -> SagaStep;
    /// Called when a step fails. Returns commands that undo the
    /// already-completed steps (in reverse order). Default: no-op.
    /// Compensation is best-effort — if a compensating command itself
    /// fails, the coordinator logs and continues.
    fn compensate(&mut self) -> Vec<Command> { Vec::new() }
}
```

### 4.4 Idempotency

Every reducer command in a saga must be idempotent OR carry enough information to detect "already applied". Specifically:

- `CreateWorkspace` is NOT idempotent on retry (UUID generated per call). Sagas avoid retry by tracking which steps have already started; the coordinator only re-invokes `start` once per saga registration.
- `Move*` operations are idempotent if the target state matches (move block to tab where it already is = no-op).
- `Delete*` are silently idempotent.

Sagas should be designed so each step's success is observable from the events alone. If a saga state machine can't tell whether step N succeeded from the event stream, it can't safely compensate.

### 4.5 RPC integration model

For each saga-driven RPC, the handler:

1. Generates a fresh `saga_id` (monotonic counter on `srv_state`).
2. Constructs a `Saga` instance with the inputs.
3. Registers it with the coordinator (`coordinator.register(Box::new(saga))`).
4. Awaits `Event::SagaCompleted { saga_id }` OR `Event::SagaFailed { saga_id }` on the broadcast bus.
5. Returns the saga's `result` to the HTTP caller (or surfaces the failure reason).

Steps 4-5 happen via a tokio oneshot channel set up at registration: the coordinator sends the terminal event into the channel; the RPC handler awaits.

```rust
async fn dispatch_saga<S: Saga + 'static>(
    state: &AppState,
    saga: S,
) -> Result<serde_json::Value, String> {
    let saga_id = saga.saga_id();
    let (tx, rx) = tokio::sync::oneshot::channel();
    state.saga_coordinator.register(Box::new(saga), tx).await;
    match rx.await {
        Ok(SagaOutcome::Completed(result)) => Ok(result),
        Ok(SagaOutcome::Failed(reason)) => Err(reason),
        Err(_) => Err("saga channel closed before completion".into()),
    }
}
```

**Timeout:** RPC handler wraps `rx.await` in `tokio::time::timeout(SAGA_TIMEOUT, ...)`. Default 5 seconds. Timeout cancels the saga (coordinator marks it failed + dispatches compensation).

---

## 5. Wire protocol additions

### 5.1 New single-step commands (no saga)

Atomic commands the reducer supports directly. Each gets a corresponding event.

```rust
Command::PromoteBlockToTab { block_id, src_tab_id, workspace_id }
Command::ReorderTabsBulk { workspace_id, tab_ids: Vec<String> }       // for UpdateTabIds
Command::RenameWorkspace { workspace_id, name }                        // for UpdateWorkspace
Command::RenameTab { tab_id, name }                                    // for UpdateTabName
Command::UpdateBlockMeta { block_id, meta_patch: serde_json::Value }   // for UpdateObjectMeta on Block
Command::UpdateWorkspaceMeta { workspace_id, meta_patch }
Command::UpdateTabMeta { tab_id, meta_patch }
Command::SwitchWorkspace { window_id, workspace_id }                   // window_id needs to be tracked in reducer
```

Note: `UpdateObject` (the catch-all generic update) deconstructs at the RPC layer into one of the typed commands above based on otype. No `Command::UpdateObject` in the reducer.

### 5.2 Saga-internal commands (only used inside saga step machines)

```rust
Command::CreateWindow { window_id, workspace_id, pos, size }          // window_id pre-assigned
Command::CloseWindowInternal { window_id }
Command::MoveTab { tab_id, src_workspace_id, dst_workspace_id, dst_index }
Command::MoveBlock { block_id, src_tab_id, dst_tab_id, dst_index }
Command::DeleteWorkspaceCascade { workspace_id }                       // explicit cascade delete; same as current DeleteWorkspace but with explicit naming
```

### 5.3 Saga lifecycle events (already in E.1a)

```rust
Event::SagaStarted { saga_id, kind: String, version }
Event::SagaCompleted { saga_id, result: serde_json::Value, version }
Event::SagaFailed { saga_id, reason: String, version }
```

**`kind`** is a string discriminator: `"tear_off_tab"`, `"create_window"`, etc. Lets `--diag` filter by saga type.

### 5.4 Window state in reducer (NEW)

To support window-related sagas, the reducer needs a `windows: HashMap<String, WindowRecord>` map. Currently the reducer doesn't track windows; window state is wcore-direct via `Window` SQLite rows.

```rust
pub struct WindowRecord {
    pub window_id: String,
    pub workspace_id: String,
    // Future: pos, size, focused — not needed for sagas in scope here.
}
```

`Command::CreateWindow` inserts; `Command::CloseWindowInternal` removes; `Command::SwitchWorkspace` updates `workspace_id`.

Bootstrap loads windows from wstore (alongside workspaces).

---

## 6. Per-saga specifications

For each saga: trigger / steps / events watched / compensation / RPC return.

### 6.1 TearOffTab

**Trigger RPC:** `("workspace", "TearOffTab")` with `tab_id` and source `ws_id`.

**Steps:**
1. `CreateWorkspace { name: <derived from tab name> }` → wait for `WorkspaceCreated { workspace_id: new_ws_id }`
2. `MoveTab { tab_id, src_workspace_id: src_ws_id, dst_workspace_id: new_ws_id, dst_index: 0 }` → wait for `TabMoved`
3. `CreateWindow { window_id: <new uuid>, workspace_id: new_ws_id, pos: <derived>, size: <derived> }` → wait for `WindowOpened`
4. `Completed { result: { new_workspace_id, new_window_id } }`

**Compensation (reverse order on failure):**
- After step 3 fail: `MoveTab { tab_id, src: new_ws_id, dst: src_ws_id }` then `DeleteWorkspaceCascade { new_ws_id }`
- After step 2 fail: `DeleteWorkspaceCascade { new_ws_id }`
- After step 1 fail: nothing to compensate; saga just fails

**Subscriber side-effects:** `WorkspaceCreated` writes new workspace; `TabMoved` updates both workspaces' `tabids`; `WindowOpened` writes new Window row; `WorkspaceDeleted` (compensation) cascades.

**RPC return:** `{ new_workspace_id, new_window_id }` — the host needs the window id to actually open the CEF window. Host bridge subscribes to `WindowOpened` and creates the CEF window; saga doesn't directly call CEF.

### 6.2 TearOffBlock

**Trigger RPC:** `("workspace", "TearOffBlock")` with `block_id`, `src_tab_id`, `src_ws_id`.

**Steps:**
1. `CreateWorkspace` → wait for `WorkspaceCreated { new_ws_id }`
2. `CreateTab { workspace_id: new_ws_id, name: <derived> }` → wait for `TabCreated { new_tab_id }`
3. `MoveBlock { block_id, src_tab_id, dst_tab_id: new_tab_id, dst_index: 0 }` → wait for `BlockMoved`
4. `CreateWindow { workspace_id: new_ws_id, ... }` → wait for `WindowOpened { new_window_id }`
5. `Completed { result: { new_workspace_id, new_tab_id, new_window_id } }`

**Compensation:** reverse steps 4 → 1.

### 6.3 RestoreTornOffTab

**Trigger RPC:** `("workspace", "RestoreTornOffTab")` with `tab_id`, source `src_ws_id` (the original workspace), and the torn-off `torn_ws_id`.

**Steps:**
1. `MoveTab { tab_id, src_workspace_id: torn_ws_id, dst_workspace_id: src_ws_id, dst_index: <append> }` → wait for `TabMoved`
2. Inspect `TabMoved` event: if `torn_ws_id` is now empty (no tabs left), proceed to step 3; otherwise complete.
3. `DeleteWorkspaceCascade { torn_ws_id }` → wait for `WorkspaceDeleted`
4. (Window close is host-side: host's existing CEF window-close logic fires when the workspace is gone. No saga step needed.)
5. `Completed { result: {} }`

**Compensation:** if step 1 fails, no rollback needed. If step 3 fails (e.g., the torn workspace has tabs again somehow), saga fails but step 1's MoveTab is already committed.

### 6.4 MoveTabToWorkspace

**Trigger RPC:** `("workspace", "MoveTabToWorkspace")` with `tab_id`, `src_ws_id`, `dst_ws_id`, `insert_index`.

**Steps:**
1. `MoveTab { tab_id, src_workspace_id: src_ws_id, dst_workspace_id: dst_ws_id, dst_index: insert_index }` → wait for `TabMoved`
2. `Completed { result: {} }`

Single-step saga (could also be modeled as a plain reducer command; using saga shape for uniformity with TearOff and to carry `saga_id` for renderer buffering).

### 6.5 MoveBlockToTab (cross-workspace)

**Trigger RPC:** `("workspace", "MoveBlockToTab")` with `block_id`, `src_tab_id`, `dst_tab_id`, `dst_index`.

**Steps:**
1. `MoveBlock { block_id, src_tab_id, dst_tab_id, dst_index }` → wait for `BlockMoved`
2. `Completed { result: {} }`

Same shape as 6.4. The "cross-workspace" detection is in the reducer's `handle_move_block` which adjusts both tabs' block_ids regardless of which workspace they belong to.

### 6.6 CreateWindow

**Trigger RPC:** `("window", "CreateWindow")`.

**Steps:**
1. `CreateWorkspace { name: "Workspace" }` → wait for `WorkspaceCreated { new_ws_id }`
2. `CreateWindow { window_id: <new uuid>, workspace_id: new_ws_id, pos, size }` → wait for `WindowOpened { new_window_id }`
3. `Completed { result: { new_window_id, new_workspace_id } }`

**Compensation:** if step 2 fails, `DeleteWorkspaceCascade { new_ws_id }`.

### 6.7 CloseWindow

**Trigger RPC:** `("window", "CloseWindow")` with `window_id`.

**Steps:**
1. Read window's `workspace_id` from reducer state.
2. `CloseWindowInternal { window_id }` → wait for `WindowClosed`
3. Check if any other window in `state.windows` references the same `workspace_id`. If yes, `Completed`. If no:
4. `DeleteWorkspaceCascade { workspace_id }` → wait for `WorkspaceDeleted`
5. `Completed { result: {} }`

**Compensation:** if step 4 fails, nothing to do (window's already closed).

### 6.8 Single-step migrations (not sagas)

Each becomes a direct reducer dispatch in the RPC handler. Pattern:

```rust
("workspace", "PromoteBlockToTab") => {
    let events = dispatch_to_reducer(state, Command::PromoteBlockToTab { ... }).await;
    // surface errors; apply synchronously to wstore; publish; return.
}
```

Reducer handlers added:
- `handle_promote_block_to_tab` — validates block + tab exist; assigns new tab UUID; mutates state.
- `handle_reorder_tabs_bulk` — validates `tab_ids` is a permutation of workspace's current tab_ids; replaces.
- `handle_rename_workspace` / `_tab` — trivial field update.
- `handle_update_*_meta` — JSON merge on the appropriate record's meta map. **Note:** reducer's WorkspaceRecord/TabRecord/BlockRecord don't currently carry meta; needs adding.
- `handle_switch_workspace` — updates `state.windows[window_id].workspace_id`.

**Reducer state additions for meta:**
```rust
pub struct WorkspaceRecord {
    // existing fields...
    pub meta: serde_json::Map<String, serde_json::Value>,  // NEW
}
// similarly for TabRecord, BlockRecord
```

Bootstrap loads meta from wstore. Subscriber writes meta on each create/update event.

---

## 7. Persist subscriber changes

Add apply handlers for the new events:

| Event | Apply action |
|---|---|
| `Event::TabMoved { tab_id, src_ws, dst_ws, dst_index, .. }` | wcore::move_tab_to_workspace (or equivalent direct mutation) |
| `Event::BlockMoved { block_id, src_tab, dst_tab, dst_index, .. }` | wcore::move_block_to_tab |
| `Event::WindowOpened { window_id, workspace_id, .. }` | insert Window row |
| `Event::WindowClosed { window_id, .. }` | delete Window row |
| `Event::WorkspaceRenamed / TabRenamed / BlockMetaUpdated / etc.` | update appropriate row |

All idempotent via "check current state, no-op if already match". Handles the same way the existing `apply_workspace_created` etc. do.

Saga lifecycle events (`SagaStarted` / `SagaCompleted` / `SagaFailed`) are NOT persisted — they're transient. Subscribers ignore them.

---

## 8. Test plan

### 8.1 Reducer property tests

For each new reducer arm: unit tests covering happy path + invalid inputs + idempotency.

### 8.2 Saga property tests

Per saga, test:
- Successful path: simulate happy-path event sequence; assert saga completes with expected result
- Step-N failure: simulate failure event at each step boundary; assert correct compensation commands
- Out-of-order events: feed unrelated events between saga's expected ones; assert saga ignores them

### 8.3 Integration tests (Phase E.7)

End-to-end: real srv reducer + real coordinator + real subscriber + in-memory wstore. RPC handler kicks off saga; assert wstore state matches expected after `SagaCompleted`. Crash mid-saga (kill subscriber after step N); assert bootstrap recovers correctly.

### 8.4 Smoke

Manual portable: tear off tab, restore tab, move tab between workspaces, create window via UI, close window. After each action: `--diag srv` should show coherent state matching the UI.

---

## 9. Sequencing & PR breakdown

To keep PRs reviewable (~500-800 LoC each):

| PR | Scope | Est LoC |
|---|---|---|
| **E.5.1** | Saga trait extension (compensate + on_event) + coordinator wiring + correlation IDs | ~350 |
| **E.5.2** | Reducer state: windows map + bootstrap + extract_version + tests | ~300 |
| **E.5.3** | New atomic commands: PromoteBlockToTab, ReorderTabsBulk, Rename*, Update*Meta, SwitchWorkspace + reducer arms + subscriber applies | ~600 |
| **E.5.4** | RPC migration of single-step paths in §3b — they all become reducer-direct dispatches | ~300 |
| **E.5.5** | TearOffTab saga (first concrete saga; proves the pattern) | ~400 |
| **E.5.6** | TearOffBlock + RestoreTornOffTab sagas | ~400 |
| **E.5.7** | MoveTabToWorkspace + MoveBlockToTab(cross-ws) sagas | ~300 |
| **E.5.8** | CreateWindow + CloseWindow sagas (last because window state requires E.5.2 to land first) | ~400 |
| **E.5.9** | Drop Option-A `ensure_workspace_in_reducer` helper (no longer needed; all paths reducer-routed) | ~50 (deletion) |

**Total: ~3100 LoC across 9 PRs.** Each individually shippable + testable. The Option-A hot-fix from `phase-e-tear-off-and-remaining-2026-04-30.md` ships in parallel as the immediate smoke unblock; E.5.9 retires it.

---

## 10. Open questions

1. **Does the renderer need to wait for `SagaCompleted` before applying any event?** E.6 specifies "saga buffer-until-complete" — but in practice the renderer is read-mostly and applying intermediate events shouldn't break the UI. Decision: **buffer optionally, default off in E.5; E.6 can flip to on with measurement.**

2. **What happens if an RPC times out but the saga is still running?** The HTTP caller gets a timeout error; the saga keeps progressing. If it eventually completes, the resulting state changes still propagate (events still emit). UX: the user sees the change happen "after" the API said it failed. **Acceptable** — better than aborting mid-way and leaving inconsistent state.

3. **Should saga registration be idempotent?** A double-fire from a flaky frontend could trigger two TearOffTab sagas for the same tab. Each generates its own saga_id; both run. The second's MoveTab might fail because the tab is no longer in src_workspace. Saga compensates. End state: one new workspace (from the first saga); the second's compensation cleans its own new workspace. **Acceptable but wasteful.** Future: the RPC layer could dedupe by hashing the input; out of scope for E.5.

4. **How does the host bridge surface saga events to the renderer?** Same pipe, same dispatcher (E.2c.5b). The renderer's atom router treats `SagaStarted/Completed/Failed` as metadata events; only acts on the wrapped per-step events.

5. **What about Layout state?** Sagas that touch tabs may need to update layouts (tear-off creates a new layout for the new workspace's tab). Layout migration is E.4, separate from sagas. Sagas in E.5 either skip layout updates (host catches up via existing wcore::heal_layout call paths) OR include `Command::CreateLayoutState` once E.4 lands. Decision: **defer layout integration to a follow-up E.5b once E.4 ships.**

6. **CreateBlock with explicit tab_id arg (TOCTOU race)** — the existing CreateBlock handler accepts an optional explicit tab_id at args[2] to avoid the active-tab-changed race. Sagas that create blocks (TearOffBlock step 3) inherently know the destination tab_id. Direct saga dispatches bypass the TOCTOU concern. **No change needed.**

---

## 11. Decision

This spec is the plan. Implementation proceeds via the 9-PR sequence in §9.

**Immediate next steps:**

1. **Ship the Option-A hot-fix** (already on `agenta/phase-e-2c-fix-tear-off-workspace`) — unblock smoke.
2. **Open E.5.1** — saga trait extension + coordinator wiring + correlation IDs. Foundational; gates everything else.
3. Iterate through E.5.2 → E.5.9 per the table.

E.2c.5b (TypeScript renderer dispatcher) can ship in parallel with the E.5.x series — it doesn't depend on saga work.

E.4 (Layout) can also ship in parallel; saga integration with layouts is deferred to E.5b.

---

## 12. Cross-references

- `docs/specs/SPEC_PHASE_E_SRV_REDUCER_2026_04_29.md` §7 — original (sketchy) saga spec; this doc supersedes.
- `docs/retro/phase-e-status-2026-04-30.md` — phase status snapshot
- `docs/retro/phase-e-tear-off-and-remaining-2026-04-30.md` — the bug analysis that motivated this spec
- `docs/retro/multi-reducer-status-2026-04-29.md` — running phase progress
