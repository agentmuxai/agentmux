# SPEC: Phase E — srv reducer + saga coordinator (first multi-reducer validation)

**Status:** Draft for review (rev 2 — coordinator integrated).
**Date:** 2026-04-29 (post Phase B + D).
**Author:** AgentA.

**Rev 2 changes from rev 1:**
- Added centralized **saga coordinator** in launcher (was: distributed subscribers). Sagas are explicit state machines with correlation IDs. §7 fully rewritten.
- Added **durable event log + bootstrap-replay** for srv crash recovery. §6.4–6.5.
- Added **idempotent SQLite applies** for safe replay. §6.3.
- Added **renderer-side saga buffer + per-source version tracking + resync ordering**. §9.2–9.3.
- **Phase F preview** (§13) — honest realistic-fix framing for browser_panes deadlock and BlockController scope.
- PR sequence split E.1 into E.1a (coordinator) + E.1b (srv reducer skeleton).
**Read first:**
- `docs/retro/multi-reducer-status-2026-04-29.md` — current state across launcher / host / srv
- `docs/retro/multi-reducer-proposal-2026-04-28.md` — long-term design (sections on cross-reducer-sync, sagas, projections)
- `specs/SPEC_WINDOW_PROCESS_STATE_MACHINE_2026_04_27.md` — driving spec for the multi-reducer architecture
- `agentmux-launcher/src/reducer.rs` — pattern to mirror

---

## 1. Goal

Promote `agentmux-srv` to a Redux-style reducer over its canonical state (workspaces, tabs, blocks, layouts, identity accounts), matching the launcher reducer's pattern: `update(&mut state, Cmd, Ctx) -> Vec<Event>`. Pure functional core, no I/O inside the reducer, idempotent apply.

This is **the first validation point** for multi-reducer infrastructure. Two reducers (launcher + srv) communicating via versioned events, with sagas for cross-reducer transitions. Patterns proven here generalize to Phase F (host reducer).

**Non-goals:**
- Replacing srv's storage layer (`WaveStore` / SQLite). Disk persistence stays — reducer state IS the in-memory projection of what's on disk plus live-derived state.
- Changing the wire protocol the frontend / host see for reads. `GetObject` etc. continue working unchanged.
- Cross-platform parity work (Phase 7 territory).

---

## 2. Why srv first

Per `multi-reducer-proposal-2026-04-28.md`:

> srv state is **purer** — no FFI handles, no Win32 sync constraints. srv migration is **lower-risk**, validates the pattern before applying it to host's harder constraints. srv reducer's events (e.g., `WorkspaceCreated`, `TabAdded`) cleanly cross to the launcher reducer for cross-process sync.

Concretely:
- Host has `cef::Browser` FFI handles, `is_quitting` AtomicBool, pre-create handoff queues — all things that resist the reducer pattern. (Hence Phase F's "scaffolding retirement" framing.)
- Srv has plain serializable structs (`Workspace`, `Tab`, `Block`, `LayoutState`, `Client`, `Window`) — exactly the shape a reducer wants.

If we get reducer-pattern wrong on srv, the cost is moderate refactor of one crate. Getting it wrong on host first would entangle CEF lifecycle bugs with reducer architecture bugs in a way that's hard to untangle.

---

## 3. State inventory (what moves into the reducer)

Today srv keeps state in:

| Location | Type | Mutation sites |
|---|---|---|
| `WaveStore` (SQLite-backed) | Persistent objects: `Client`, `Window`, `Workspace`, `Tab`, `LayoutState`, `Block` | `wcore::*` mutation functions (`create_block`, `create_tab`, `delete_workspace`, etc.) |
| `Broker` (in-memory) | WPS subscriptions, presence, route table | `subscribe`, `publish`, `unsubscribe` |
| `BlockController` registry | Live agent / shell processes per block | `start_controller`, `delete_controller` |
| `MessageBus` | Inter-agent messaging | `register`, `send`, `read_messages` |
| Reactive registry | In-memory agent metadata | `register`, `inject` |

**Phase E scope** (canonical reducer state):
- `Workspace`s (id → Workspace)
- `Tab`s (id → Tab; carries `blockids: Vec<String>`)
- `Block`s (id → Block)
- `LayoutState`s (id → LayoutState)
- `Window`s (id → Window; the Wave-level window object distinct from launcher's Win32 windows)
- `Client` singleton (config + window list)

**Phase E does NOT touch** (stays as today, separate from reducer):
- `BlockController` / agent processes — these are the srv-side equivalent of host's CEF `browsers`. FFI-adjacent (live processes, file handles, PTY sessions). Same scaffolding-vs-reducer split that B.5 hit. Defer to a future phase.
- `MessageBus` — purely transient routing; no canonical state worth moving.
- Reactive registry — same.
- `WaveStore`'s SQLite layer — stays as the disk-persistence backing. Reducer state is the in-memory authoritative copy; persistence is a side effect of applying events (see §6).

---

## 4. Wire protocol (cross-reducer)

### 4.1 New `agentmux-common::ipc` types

`Command::Register { kind: ClientKind::Srv, ... }` already exists (B.3). What's new:

```rust
// Commands srv accepts on its IPC pipe (subset — full list in §5).
Command::CreateWorkspace { name: String }
Command::DeleteWorkspace { workspace_id: String }
Command::CreateTab { workspace_id: String, opts: CreateTabOpts }
Command::DeleteTab { workspace_id: String, tab_id: String }
Command::SetActiveTab { workspace_id: String, tab_id: String }
Command::ReorderTabs { workspace_id: String, tab_ids: Vec<String> }
Command::CreateBlock { tab_id: String, def: BlockDef, runtime_opts: Option<RuntimeOpts> }
Command::DeleteBlock { tab_id: String, block_id: String }
Command::UpdateBlockMeta { block_id: String, meta_patch: serde_json::Value }
Command::MoveBlockToTab { block_id: String, src_tab_id: String, dst_tab_id: String, dst_index: Option<u32> }
Command::TearOffBlock { block_id: String, src_tab_id: String, target_window_label: String }
Command::TearOffTab { tab_id: String, src_workspace_id: String, target_window_label: String }
Command::RestoreTornOffTab { tab_id: String, src_workspace_id: String, dst_workspace_id: String }
// + workspace-rename, layout-update, client-meta-update commands
```

### 4.2 New `Event` variants from srv

```rust
Event::WorkspaceCreated { workspace_id: String, name: String, version: u64 }
Event::WorkspaceDeleted { workspace_id: String, version: u64 }
Event::WorkspaceUpdated { workspace_id: String, /* full new state */, version: u64 }
Event::TabCreated { workspace_id: String, tab_id: String, /* tab fields */, version: u64 }
Event::TabDeleted { workspace_id: String, tab_id: String, version: u64 }
Event::ActiveTabChanged { workspace_id: String, tab_id: String, version: u64 }
Event::BlockCreated { tab_id: String, block_id: String, /* block fields */, version: u64 }
Event::BlockDeleted { tab_id: String, block_id: String, version: u64 }
Event::BlockUpdated { block_id: String, /* full new state or patch */, version: u64 }
Event::TabMoved { tab_id: String, src_workspace_id: String, dst_workspace_id: String, version: u64 }
Event::BlockMoved { block_id: String, src_tab_id: String, dst_tab_id: String, version: u64 }
// ...
```

### 4.2.1 Saga correlation IDs (NEW)

Every `Command` and every `Event` gains an optional `saga_id: Option<u64>` field. The saga coordinator (§7) sets this when issuing commands as part of a saga; reducers preserve it through to the events they emit. Subscribers (renderer especially) use the field to:

- **Group events** belonging to the same logical operation (tear-off touches 3 reducers' events that all carry the same `saga_id`).
- **Buffer-until-complete** so the renderer applies cross-reducer state changes atomically rather than mid-flight.
- **Filter `--diag` output** to see one saga's full lifecycle.

`saga_id: None` means "not part of a saga" — the vast majority of events. Buffering only kicks in when `saga_id: Some(_)` is observed.

### 4.2.2 Source tagging (NEW)

Subscribers route events by source. Two options:

- **Implicit via discriminator naming.** `Event::WindowOpened` is launcher-sourced; `Event::WorkspaceCreated` is srv-sourced. Source is a function of the variant.
- **Explicit `source: ReducerSource` field.** More verbose but eliminates the convention dependency.

**Recommendation: implicit.** Naming has been consistent since B.3 and adding a field on every variant is busywork. If a future variant becomes ambiguous, switch to explicit at that point.

### 4.3 GetSnapshot extension

`Event::Snapshot` (D.1) carries launcher state today. Two options for srv:

**Option A — separate reducer, separate snapshot.** New `Command::GetSrvSnapshot` + `Event::SrvSnapshot` reaching srv directly. Cleanest separation; matches the multi-reducer mental model.

**Option B — extend launcher's Snapshot.** Launcher fetches srv state via cross-process query and embeds it in its Snapshot event.

**Recommendation: A.** Reasons:
1. Each reducer is canonical for its domain. Subscribers wanting srv state ask srv; wanting launcher state ask launcher.
2. Snapshot is a heavyweight reply — bundling both forces the launcher into a critical path it shouldn't own.
3. Phase D's broadcast bus handles the routing fine — subscribers see events from BOTH reducers automatically once srv joins the bus.

### 4.4 Pipe topology

Srv currently connects to the launcher's pipe as `ClientKind::Srv` (B.3) for lifecycle facts only. Phase E adds a **reverse channel**: srv hosts its own IPC pipe at `\\.\pipe\agentmux-{hash}\srv-command` that subscribers (host, frontend, Tools) connect to.

| Pipe | Server | Connects | Purpose |
|---|---|---|---|
| `\\.\pipe\agentmux-{hash}\command` | launcher | host, srv, renderer-tools | launcher reducer commands + events |
| **`\\.\pipe\agentmux-{hash}\srv-command`** (new) | srv | host, renderer-tools | **srv reducer commands + events** |

Renderers continue receiving events via the host's CEF JS bridge — the host subscribes to BOTH pipes and forwards both event streams to renderers. The renderer reducer in `frontend/app/store/launcher-event-reducer.ts` becomes a multi-reducer-event reducer: it routes events by their type tags into separate atom domains.

---

## 5. Reducer arms (full list)

Mirroring the launcher reducer's arm-per-command pattern:

```rust
pub fn update(state: &mut State, cmd: Command, ctx: &Ctx) -> Vec<Event> {
    match cmd {
        Command::Register { kind, pid, version } => handle_register(state, ctx, kind, pid, version),
        Command::Goodbye => handle_goodbye(state, ctx),
        Command::GetSrvSnapshot => handle_get_snapshot(state),
        Command::GetEvents { since } => Vec::new(), // intercepted in server, like launcher

        // Workspace lifecycle
        Command::CreateWorkspace { name } => handle_create_workspace(state, ctx, name),
        Command::DeleteWorkspace { workspace_id } => handle_delete_workspace(state, ctx, workspace_id),
        Command::UpdateWorkspaceMeta { workspace_id, meta_patch } => ...,

        // Tab lifecycle
        Command::CreateTab { workspace_id, opts } => handle_create_tab(state, ctx, workspace_id, opts),
        Command::DeleteTab { workspace_id, tab_id } => handle_delete_tab(state, ctx, workspace_id, tab_id),
        Command::SetActiveTab { workspace_id, tab_id } => handle_set_active_tab(state, ctx, workspace_id, tab_id),
        Command::ReorderTabs { workspace_id, tab_ids } => ...,

        // Block lifecycle
        Command::CreateBlock { tab_id, def, runtime_opts } => handle_create_block(state, ctx, tab_id, def, runtime_opts),
        Command::DeleteBlock { tab_id, block_id } => ...,
        Command::UpdateBlockMeta { block_id, meta_patch } => ...,

        // Drag-and-drop / tear-off (sagas — see §7)
        Command::MoveBlockToTab { ... } => ...,
        Command::TearOffBlock { ... } => ...,
        Command::TearOffTab { ... } => ...,
        Command::RestoreTornOffTab { ... } => ...,

        // Layout
        Command::UpdateLayout { layout_id, layout_state } => ...,
    }
}
```

Each handler:
1. Validates against current state (e.g., `tab_id` exists in `state.tabs`).
2. Mutates state in-place.
3. Returns `Vec<Event>` describing what changed (typically one Event per logical change).

Same invariants apply as launcher reducer: pure, sub-millisecond, no async, no I/O.

---

## 6. Persistence — where SQLite fits

The reducer is in-memory. SQLite is on-disk. They reconcile at two boundaries:

### 6.1 Startup: SQLite → reducer + replay-from-HWM

Srv startup is two-phase: load SQLite, then replay event log entries newer than SQLite's high-water mark.

**Phase 1 — load SQLite.** Reads all persistent objects into reducer state:

```rust
let mut state = State::default();
state.workspaces = wstore.list_workspaces()?.into_iter().map(|ws| (ws.oid.clone(), ws)).collect();
state.tabs = wstore.list_tabs()?.into_iter().map(|t| (t.oid.clone(), t)).collect();
// ... etc
let last_persisted_version = wstore.get_persistence_hwm()?; // tracked column
```

**Phase 2 — replay event log from HWM.** Srv reads its own event log (`<data-dir>/srv-events.log`), filters to events with `version > last_persisted_version`, and applies them to in-memory state. This recovers events that were broadcast but not yet persisted at the moment of the last crash.

After both phases, the reducer's state IS the canonical view. Reads (`GetObject`) come from the reducer state. Writes go through the reducer.

### 6.2 Reducer → SQLite (subscriber pattern)

A dedicated subscriber task on the broadcast bus listens for srv events and persists them to SQLite. Same pattern as the host's `apply_event_to_shadow`, but the "shadow" here is the disk:

```rust
// agentmux-srv/src/backend/persist_subscriber.rs (new)
async fn run_persist_subscriber(wstore: WaveStore, mut events_rx: broadcast::Receiver<Event>) {
    loop {
        match events_rx.recv().await {
            Ok(Event::WorkspaceCreated { workspace_id, name, .. }) => {
                let ws = Workspace { oid: workspace_id, name, ..Default::default() };
                let _ = wstore.must_insert(&ws);
            }
            Ok(Event::TabCreated { tab_id, workspace_id, .. }) => { ... }
            // ... etc
            Err(_) => return,
        }
    }
}
```

**Why this design vs in-reducer-write:**
- Keeps the reducer pure (no I/O). Property tests stay easy.
- Persistence becomes idempotent in the same way subscribers are — replay-safe by construction.
- A persistence failure (disk full) is a subscriber problem, not a reducer problem. Reducer state stays consistent; we have a recovery story (replay events from log).

**Trade-off:** there's a moment between reducer-mutation and disk-write where state is "in memory but not on disk." If srv crashes in that window, the event is lost from SQLite but was emitted on the bus. The event log + bootstrap-replay (§6.1 phase 2) recovers it.

### 6.3 Idempotent SQLite applies

Persist-subscriber writes use INSERT-OR-IGNORE / UPDATE-WHERE-version-greater patterns so applying the same event twice is a safe no-op. Required for two reasons:

1. Bootstrap-replay (§6.1) re-applies events already persisted in SQLite — must not error.
2. Subscriber crash-and-restart mid-batch must not leave SQLite in an inconsistent state.

Concrete pattern per object type:

```rust
// idempotent on event_version
fn apply_workspace_created(wstore: &WaveStore, ws: &Workspace, version: u64) -> Result<()> {
    wstore.upsert_workspace_if_version_higher(ws, version)
}
```

The `upsert_*_if_version_higher` family is added once in E.2 and reused.

### 6.4 Durable event log (Phase D.2 hardening)

D.2's event log was best-effort (writes via `write_all`, no `fsync`). For Phase E's recovery story to be correct, the log must be **durable** — events written before a crash MUST survive it.

Hardening (E.1):
- Append: `file.write_all(buf)` then `file.sync_data()` per event.
- Latency: ~ms instead of microseconds. Acceptable for the volume we have (~10–50 events per user action).
- Trade-off considered: batched fsync (group commit). Skipping for E.1 — premature optimization; revisit if profiling shows a hot path.

### 6.5 Startup ordering

Critical sequence:
1. Bootstrap state from SQLite + replay from event log (§6.1).
2. Open IPC pipe (start accepting commands).
3. Start broadcast bus.
4. Start persist-subscriber.

The persist-subscriber is started LAST so that any events emitted during steps 1–3 (e.g., spawned process registrations) don't race against the subscriber's startup sequence.

### 6.3 The `update_object` path

Today srv's `update_object` writes directly to `WaveStore`. Phase E moves it to: `update_object` becomes a thin wrapper that builds the appropriate `Command` and dispatches through the reducer. The HTTP/WS RPC layer continues accepting the same JSON shapes; underneath, mutations go through the reducer.

---

## 7. Sagas (cross-reducer flows) — centralized coordinator

Some flows touch both reducers. Sagas encode them as sequenced cross-process commands correlated by `saga_id`. Phase E adds a **centralized saga coordinator in the launcher** that owns saga lifecycle.

### 7.1 Coordinator design

The coordinator lives in `agentmux-launcher::saga`, runs as a tokio task subscribing to the broadcast bus. Holds:

- `in_flight: HashMap<u64, Box<dyn Saga>>` — active sagas keyed by saga_id
- `next_saga_id: u64` — monotonic
- Channels for issuing commands to launcher (in-process), host pipe, srv pipe

**Saga trait:**

```rust
trait Saga: Send {
    /// Called once when the saga starts. Returns the first command to issue.
    fn start(&mut self, ctx: &SagaCtx) -> SagaAction;

    /// Called when an event with this saga's id arrives on any pipe.
    /// Returns the next action.
    fn on_event(&mut self, event: &Event, ctx: &SagaCtx) -> SagaAction;

    /// Human-readable name for --diag output.
    fn name(&self) -> &'static str;
}

enum SagaAction {
    /// Issue a command on the named pipe with the saga's id stamped.
    IssueCmd { target: PipeTarget, cmd: Command },
    /// Saga is complete; coordinator removes it from in_flight + emits SagaCompleted event.
    Done,
    /// Saga failed at an irrecoverable step; coordinator emits SagaFailed event.
    /// Compensation actions (if any) are encoded as IssueCmd before returning Failed.
    Failed { reason: String },
    /// Saga is waiting for an event it hasn't seen yet; do nothing.
    Wait,
}

enum PipeTarget {
    LauncherSelf, // dispatch to local launcher reducer
    Host,         // forward to host pipe
    Srv,          // forward to srv pipe
}
```

**New events:**

```rust
Event::SagaStarted { saga_id: u64, name: String, version: u64 }
Event::SagaCompleted { saga_id: u64, version: u64 }
Event::SagaFailed { saga_id: u64, reason: String, version: u64 }
```

These are coordinator-emitted; subscribers (renderer especially) use them to know when a saga's effects are durable.

### 7.2 Tear-off saga (worked example)

User tears a block out of a tab to create a new window. Touches all three reducers (host: pool promote, launcher: window registration, srv: block move + tab assignment).

```
saga TearOffBlock(block_id, src_tab_id, dst_window_workspace_name):
    state TearOffState { saga_id, block_id, src_tab_id, dst_window_label?, dst_workspace_id?, dst_tab_id? }

    Step 1 — start: issue Command::PromotePoolWindow{} → host
              wait for: Event::WindowOpened{label, ..} with this saga_id
              (launcher reducer emits this when host's promote completes)

    Step 2 — on Event::WindowOpened{label}:
              record dst_window_label = label
              issue Command::CreateWorkspace{name: dst_window_workspace_name} → srv
              wait for: Event::WorkspaceCreated{workspace_id}

    Step 3 — on Event::WorkspaceCreated{workspace_id}:
              record dst_workspace_id
              issue Command::CreateTab{workspace_id, opts: {make active}} → srv
              wait for: Event::TabCreated{tab_id}

    Step 4 — on Event::TabCreated{tab_id}:
              record dst_tab_id
              issue Command::MoveBlockToTab{block_id, src_tab_id, dst_tab_id, dst_index: 0} → srv
              wait for: Event::BlockMoved{}

    Step 5 — on Event::BlockMoved{}:
              issue Command::AssignWorkspaceToWindow{window_label: dst_window_label, workspace_id: dst_workspace_id} → launcher
              wait for: Event::WindowWorkspaceAssigned{}

    Step 6 — on Event::WindowWorkspaceAssigned{}: return Done

    Compensation (any step fails):
              if dst_window_label set: Command::CloseWindow{label} → launcher
              if dst_workspace_id set: Command::DeleteWorkspace{workspace_id} → srv
              return Failed{reason}
```

The coordinator drives this state machine. Renderer sees the saga via the saga_id-tagged events on the broadcast bus and applies them atomically when `SagaCompleted` arrives (see §9 for renderer-side handling).

### 7.3 Window-close cascade (NOT a saga, just a subscriber)

Single-reducer reaction: host closes a CEF window → launcher emits `Event::WindowClosed{label}`. Srv subscribes (separately from the coordinator) and reacts.

This is **not** a saga because it's one-step and one-reducer-reacts-to-another. No coordination needed. Implemented as a plain broadcast-bus subscriber in srv.

The discrimination test: **multi-step + multi-reducer-issuing-commands → saga**. **Single-reducer-reacting-to-an-event → plain subscriber**. Both patterns coexist; sagas exist only where coordination buys correctness.

### 7.4 Saga lifecycle + observability

- `--diag wrr` adds a `--diag sagas` topic that prints in-flight sagas with their state-machine position.
- Saga events appear in the event log (D.2) with their saga_id, so post-mortem debugging traces an entire flow.
- Saga state is in-memory only for E (lost on launcher restart). If the launcher restarts mid-saga, in-flight sagas are abandoned; subscribers see no further events for those saga_ids. Renderer reducer's saga buffer should time out after ~30s and apply whatever it has (tolerate partial state). Persisting saga state for cross-restart recovery is a Phase F or beyond concern.

### 7.5 What stays a plain subscriber (not coordinator-routed)

- All single-reducer event reactions (window close → workspace cleanup, block delete → controller stop, etc.)
- Persist-subscriber (writes events to SQLite — §6.2)
- Renderer's typed-event-reducer atom updates

Coordinator-routed events are the minority. Most of the system stays simple.

---

## 8. Sub-PR sequence

Per the multi-reducer-status doc decision: Phase E + F stay multi-PR (mega-PRs hurt review quality + bisection). Bundle small cleanups in a "phase exit" PR at the end.

| PR | Scope | Est LoC |
|---|---|---|
| **E.1a** | Saga coordinator infrastructure: `agentmux-launcher::saga` module, `Saga` trait, `SagaCoordinator` task, `SagaStarted`/`SagaCompleted`/`SagaFailed` events, `saga_id` field on `Command` + `Event`. **No actual sagas yet — just the framework.** Plus durable event-log hardening for the launcher (fsync per append) and the two codex P2 carryovers from PR #608 (identity-patch ordering + replay_truncated overflow). | ~500 |
| **E.1b** | New `agentmux-srv::reducer` skeleton: types (`State`, `Cmd`, `Event`), `update` function with arms for `Register` / `Goodbye` / `GetSrvSnapshot` / `GetEvents`. Property tests. New srv pipe + broadcast bus + event log (mirrors launcher's, including persistence HWM tracking for §6.1's bootstrap-replay). Host bridge subscribes to srv pipe. Renderer-side dispatcher receives events from both pipes. No domain commands yet — just plumbing. | ~600 |
| **E.2** | Workspace + Tab + ActiveTab arms. SQLite-bootstrap path **with replay-from-HWM (§6.1)**. Idempotent persist-subscriber (`upsert_*_if_version_higher`). RPC `dispatch_service` migrates these arms to go through the reducer (coexisting with bespoke WaveObjUpdate path). | ~800 |
| **E.3** | Block lifecycle arms (`CreateBlock`, `DeleteBlock`, `UpdateBlockMeta`). Same pattern as E.2. | ~400 |
| **E.4** | Layout state arms. Trickier: `LayoutState` has `pendingbackendactions` that hint at an existing async surface; reducer ingests it as commands. | ~500 |
| **E.5** | Drag/tear-off sagas — first concrete saga consumers of E.1a's coordinator. New `Command::MoveBlockToTab`, `TearOffBlock`, `TearOffTab`, `RestoreTornOffTab`. Plain subscribers for single-reducer reactions (window close → workspace cleanup). | ~700 |
| **E.6** | Renderer-side: `frontend/util/launcher-events.ts` → `reducer-events.ts`; consumes events from both pipes (host bridge forwards both); per-source version tracking; saga-buffer-until-complete logic. Bespoke WPS WaveObjUpdate path retired or demoted. | ~600 |
| **E.7 — exit** | Property tests for srv invariants (workspace owns its tabs, tab owns its blocks, no orphan blocks, etc.) + cross-reducer integration tests for saga flows. `agentmux.exe --diag srv` and `agentmux.exe --diag sagas` Tool clients. Dead code sweep. | ~500 |

**Total: ~4600 LoC across 8 PRs.** Each individually shippable + smoke-testable. No PR exceeds ~800 LoC review surface. The coordinator (E.1a) lands BEFORE the saga consumers (E.5) so the framework is proven before the first real saga depends on it.

---

## 9. Cross-reducer wiring detail (host bridge + renderer)

### 9.1 Host bridge

The CEF JS bridge currently forwards launcher events only. Phase E expands:

```rust
// agentmux-cef/src/launcher_event_bridge.rs becomes reducer_event_bridge.rs
//
// Subscribes to BOTH:
//   - launcher pipe (existing)
//   - srv pipe (NEW)
// Forwards every typed event to every renderer's window.__agentmux_reducer_event
// dispatcher (rename from __agentmux_launcher_event, with backward-compat alias).
//
// The renderer's reducer routes by event-tag to the appropriate atom domain.
```

### 9.2 Renderer per-source version + saga buffer

The renderer's reducer tracks two pieces of resync state:

```ts
type ReducerEventState = {
    launcherSeenVersion: number;
    srvSeenVersion: number;
    // Saga buffering: events with non-null saga_id wait here until
    // SagaCompleted/SagaFailed arrives, then apply atomically.
    sagaBuffer: Map<saga_id, ReducerEvent[]>;
};
```

For non-saga events (`saga_id == null` — the vast majority), apply on arrival. For saga events:

```ts
function dispatch(evt: ReducerEvent) {
    if (evt.event === 'saga_started') {
        sagaBuffer.set(evt.saga_id, []);
        return;
    }
    if (evt.event === 'saga_completed' || evt.event === 'saga_failed') {
        const buffered = sagaBuffer.get(evt.saga_id) ?? [];
        sagaBuffer.delete(evt.saga_id);
        if (evt.event === 'saga_completed') {
            // Apply all buffered events atomically (in version order).
            buffered.sort((a, b) => a.version - b.version).forEach(applyToAtoms);
        }
        // saga_failed: discard buffered events; subscribers see the failure.
        return;
    }
    if (evt.saga_id != null && sagaBuffer.has(evt.saga_id)) {
        sagaBuffer.get(evt.saga_id)!.push(evt);
        return;
    }
    applyToAtoms(evt);
}
```

**Saga timeout safety net:** sagas in flight too long (e.g. 30s) flush their buffer and emit a synthetic `SagaCompleted` so the renderer doesn't hold buffered events forever if the launcher restarts mid-saga. Implementation note: this is a *renderer-side* timeout, not a launcher timeout — the launcher is event-driven (no timers per Phase B discipline).

### 9.3 Resync ordering

When a renderer reconnects (D.3 resync), the order matters:

1. Send `Register` to both pipes.
2. Send `GetSnapshot` + `GetEvents{since}` to both pipes.
3. **Buffer all live broadcast events** from both pipes until both replies have arrived.
4. Apply launcher snapshot, then srv snapshot.
5. Apply launcher event-list, then srv event-list (in arrival order within each).
6. Drain the live buffer (events sorted by per-source version), applying to atoms.

Step 3 is the key cross-pipe-ordering safety mechanism. Without it, a live event arriving on pipe A before pipe B's snapshot reply could reference state that doesn't exist yet from the renderer's perspective.

---

## 10. Open design questions

1. **`UpdateBlockMeta` granularity**. Today `UpdateObjectMeta` accepts a full `MetaMapType`. The reducer can either (a) take the full map (simple, less efficient on large metas) or (b) take a JSON Patch / delta (more efficient, more complexity). Recommend (a) for E.3, optimize later if profiling shows hot paths.

2. **`LayoutState.rootnode` opaque JSON**. The frontend treats it as a black box. The reducer can do the same (store as `serde_json::Value`) or define typed sub-arms (one per layout action). Recommend opaque-JSON for E.4 — frontend semantics shouldn't bleed into srv.

3. **Persisted event log retention policy**. D.2 retained 4096 events / 8 MiB. Srv's event volume might be higher (one event per block-meta-update could be O(N) per second during agent streaming). May need a smaller ring or a separate log file with different retention. Decide during E.2 once we have realistic throughput numbers.

4. **Identity accounts (`agent-config`) — reducer or external?** Stays out of reducer for E.1–E.7. Identity is read-mostly + has its own watcher (`subagent_watcher.rs`); not a clean fit. Revisit if Phase F surfaces tighter coupling.

5. **`BlockController` (live agent processes) — Phase F or never?** Same scaffolding question as host's `browsers`. Live process handles + PTY sessions resist the reducer pattern. Defer indefinitely; treat as Phase E's analog of host scaffolding.

6. **Backward compat with existing WaveObjUpdate WPS broadcasts**. Today srv emits `WaveObjUpdate { updatetype, otype, oid, obj }` over WPS to renderers. Phase E adds a parallel typed-event stream. Coexistence period (E.2–E.5): both fire. Retirement (E.6 or beyond): WaveObjUpdate is demoted to fallback when typed events haven't activated, then deleted.

7. **Saga timeout duration on the renderer side (§7.4 + §9.2).** 30s is a reasonable default but should be observable. Exposing it as a config knob is overkill for E; hard-code with a comment for now and revisit if real users hit timeouts.

8. **Compensation actions for sagas — declarative or imperative?** The current `Saga` trait is imperative (each step decides what to do based on the event). Declarative (annotate each step with its undo command) would be more elegant but couples step definitions tighter. Recommend imperative for E — declarative can be refactored from imperative once we have 5+ sagas to study.

9. **What happens if a saga's `IssueCmd` target pipe is unreachable?** E.g., host disconnected mid-saga. Coordinator emits `SagaFailed`, renderer drops the buffer. But the partial state changes (e.g., a workspace was created but not assigned) remain. Compensation actions fix this — but only if they were specified. Default policy: every saga MUST define compensation; lint/test enforces it.

10. **Persistence of sagas across launcher restart.** Currently in-memory only. If a long-running saga is in flight when launcher restarts, it's abandoned. For Phase E this is acceptable (sagas are seconds-long, restart is rare). For Phase F (where pool-respawn sagas might run continuously), revisit.

---

## 11. Risks (with mitigations)

1. **Cross-pipe event ordering across two pipes.**
   - **Concern:** Renderer sees launcher events on one pipe + srv events on another. Within one pipe, ordering is preserved. Across pipes, no guarantee.
   - **Mitigation:** events reference each other only by stable IDs (workspace_id, window_label) — independent atom-domain updates that converge correctly regardless of order. Saga events use `saga_id` + buffer-until-complete (§9.2). Resync orders snapshot+replay before applying live events (§9.3).
   - **Residual risk:** very low. Most flows don't actually need cross-pipe ordering.

2. **Persistence subscriber lag.**
   - **Concern:** Reducer mutation lands; renderer applies the event; user sees the change; srv crashes BEFORE the persist-subscriber wrote to SQLite.
   - **Mitigation:** durable event log (§6.4 — fsync per append) + bootstrap-replay from HWM (§6.1 phase 2) + idempotent SQLite applies (§6.3). Standard event-sourcing pattern.
   - **Residual risk:** low. Recovery is deterministic; the only loss case is "event log itself fails to fsync" which would be a disk-failure scenario beyond our scope.

3. **Reducer scope creep.** Tempting to migrate `BlockController` / `MessageBus` / Reactive registry into the reducer "for completeness." Resist. Keep the same scaffolding-vs-reducer line that B.5 drew. Reducer is for canonical state with structural invariants; transient routing tables and process registries stay outside.

4. **HTTP/WS RPC contract churn during E.2–E.5.** Frontend may see commands transition from "direct WaveObjUpdate" to "Command + Event roundtrip" mid-PR-sequence. Each PR should leave both paths working until E.6 retires the legacy. (Same coexistence pattern that B.7.3.1 → B.7.3.2 → B.7.3.3 used.)

5. **Saga complexity grows non-linearly.**
   - **Concern:** What's 3-4 sagas in Phase E balloons to 10+ by Phase F. Each one with its own state machine + compensation logic.
   - **Mitigation:** centralized coordinator (§7) — one place to test, observe, and reason about saga lifecycle. `--diag sagas` exposes in-flight sagas. Saga events appear in the event log for post-mortem.
   - **Residual risk:** moderate. Coordinator helps but doesn't make sagas trivial. Watch for it.

6. **Renderer-side multi-source consumption bugs.**
   - **Concern:** Two independent monotonic event streams, overlapping atom domains, saga buffering — easy to silently render wrong state.
   - **Mitigation:** saga buffer is one piece of code, well-tested. Per-source version tracking is straightforward. Source tagging by event-discriminator naming (already consistent since B.3) makes routing trivial.
   - **Residual risk:** moderate. The saga buffer's edge cases (partial saga events, timeout-induced flush) are where bugs hide. Unit-test these explicitly.

---

## 12. Phase E exit criteria

After E.7 merges:
- [ ] `agentmux-launcher::saga::SagaCoordinator` runs as a tokio task, drives sagas via the broadcast bus + per-pipe command dispatch.
- [ ] `agentmux-srv::reducer` is canonical for workspaces, tabs, blocks, layouts, windows-as-Wave-objects.
- [ ] All RPC mutations route through the reducer (no direct `wstore.must_insert` / `wstore.must_update` outside the persist-subscriber).
- [ ] Bootstrap-replay from durable event log works: kill `-9` srv mid-mutation, restart, in-memory state matches what was last broadcast.
- [ ] Idempotent persist-subscriber (`upsert_*_if_version_higher` family).
- [ ] Renderer's reducer-event-reducer consumes both launcher AND srv typed events; saga buffering applies cross-reducer effects atomically.
- [ ] Bespoke WaveObjUpdate channel retired (or demoted to fallback if the no-launcher-mode path can't be killed yet).
- [ ] Property tests cover srv invariants (workspace owns tabs, tab owns blocks, no orphans, version monotonicity, etc.) plus cross-reducer integration tests for at least the tear-off saga.
- [ ] `agentmux.exe --diag srv` and `agentmux.exe --diag sagas` Tool clients work.
- [ ] The two codex P2 carryovers from PR #608 are addressed.

After E.7 → **Phase F unlocks.** Host reducer reuses the multi-reducer infrastructure srv just validated; scaffolding maps (`browsers`, pool) partially migrate (see §13).

---

## 13. Phase F preview (informs Phase E design)

Phase F's job is the host reducer. Three things to know about it now (because they affect Phase E's design):

### 13.1 browser_panes deadlock — realistic fix

The hypothesis "host reducer eliminates the deadlock by design" is **partially true**. CEF callbacks (`on_after_created`, `on_before_close`) demand UI-thread synchronous access to the browsers map. We can't make those callbacks send a command-and-await-reply without blocking the UI thread.

**Realistic Phase F outcome:**
- Read paths through the reducer (via shadow projection, async) — most call sites
- CEF callbacks keep direct `Mutex<HashMap<String, Browser>>` access but enforce **snapshot-and-drop** discipline: take the lock, copy out the `(label, HWND)` pairs you need, drop the lock, then do Win32 work
- A lint or grep-test catches new code that holds the lock across `SendMessage` / `SetWindowRgn`

`set_pane_overlay_clip` becomes the canonical example of the snapshot-and-drop pattern. The deadlock dies because the lock is never held across `SendMessage`, not because the lock disappears.

This framing was clarified during Phase E spec review — better to know now than discover in Phase F.

### 13.2 Saga coordinator gets exercised harder in F

Phase E's tear-off saga is 6 steps and uses the coordinator's machinery once per user action. Phase F adds:
- Pool-respawn-on-promote saga (host emits "promoted N" → host's own saga consumer issues "spawn N+1 to refill")
- Window-cleanup cascade (launcher emits `Event::WindowClosed` → host saga reaps panes + child browsers + drains pool if last)
- Drag-window-between-monitors saga (host observes drag → emits intermediate position events → finalizes)

If Phase E's coordinator design feels heavyweight for E's flows, that's because it's also serving F's. The saga API should be designed for the F flows; E is the validation point.

### 13.3 BlockController stays outside both reducers

Same as host's `browsers`. Live process handles + PTY sessions resist the reducer pattern. Stays in `agentmux-srv::backend::blockcontroller` indefinitely, with snapshot-and-drop discipline at lock boundaries.

Implication for Phase E: don't migrate BlockController state into the srv reducer. Treat it the same way Phase F treats host's `browsers`.

---

## 14. Phase G preview — pure event-sourced (drop SQLite)

**Deferred. Optional. Only worth doing if Phase E validates the reducer pattern end-to-end and there's demonstrable value.**

Phase E keeps SQLite as a projection (the persist-subscriber writes srv reducer events to it; bootstrap reads it). After Phase E ships, the natural follow-up question is: do we need SQLite at all?

A pure event-sourced architecture would replace it with:

- **Event log** as the only on-disk source of truth (already durable post-E.1a).
- **Periodic snapshots** of reducer state written to disk, so startup time stays bounded.
- **Bootstrap = "load latest snapshot + replay events since snapshot."** No SQLite involvement.

### What Phase G would deliver

| Step | Goal |
|---|---|
| G.1 | Snapshot writer task in srv: serializes `State` to disk every N events / M minutes; fsync; rotates oldest snapshots. |
| G.2 | Bootstrap path switches from `wstore.list_*` + replay-from-HWM to `load_snapshot` + replay-since-snapshot. |
| G.3 | One-time migration: first run after this lands reads the old SQLite DB, emits synthetic events to populate the log, then deletes the DB. |
| G.4 | Retire `WaveStore` for the migrated object types (`Workspace`, `Tab`, `Block`, `LayoutState`, `Window`, `Client`). Other WaveStore consumers (subagent metadata, etc.) stay if they aren't reducer-state. |
| G.5 | Log truncation policy: events older than the most recent snapshot can be safely deleted. Implement and tune. |

### Why deferred (not Phase E)

Phase E is already 8 PRs / ~4600 LoC. Adding "drop SQLite" doubles the scope and risk surface (snapshot consistency, log truncation correctness, migration). Risk-reward is poor: cleanliness benefit only materializes after the snapshot work, which is itself meaningful complexity.

After Phase E lands and the reducer pattern is validated end-to-end, dropping SQLite becomes a focused PR sequence rather than a phase-level effort. Defer the decision.

### What Phase G unlocks

- **Single source of truth.** No SQLite ↔ reducer divergence failure modes.
- **No persist-subscriber.** Whole subsystem deleted.
- **Schema evolution via event versions** instead of SQLite migrations.
- **Cleaner mental model** — events are universal currency across all reducers.

### What Phase G does NOT change

- **History data** (`history/`, `claude_adapter`) is already file-based, not in `WaveStore`. Unaffected.
- **Identity accounts** (`agent-config`) have their own watcher. Unaffected.
- **`BlockController`** is live-process state, not stored. Unaffected.
- **`MessageBus`** is transient routing, not persisted. Unaffected.

### Open questions for Phase G (parked)

- Snapshot cadence policy (event-count, time-based, size-based, hybrid).
- Snapshot file format (bincode? JSON-lines? CBOR?). Trade speed vs introspectability.
- Multi-snapshot retention vs single rolling snapshot.
- External tools that may read `wstore.db` today (need to inventory before retirement).

**Bottom line:** Phase G is the right architecture if the reducer pattern works. Phase E is the validation that it works. Make the call after E ships, not before.

---

## 15. What this spec does NOT do

- **Doesn't fully design Phase F.** §13 sketches enough to inform E's design; the full spec lands when E does.
- **Doesn't fully design Phase G.** §14 sketches the option; the full spec is conditional on Phase E success.
- **Doesn't change disk format in Phase E.** SQLite tables stay as-is; persist-subscriber writes the same rows it would have written via direct `wstore.must_*` calls. Phase G changes this.
- **Doesn't address Phase 7.** Cross-platform IPC (Unix domain sockets) is independent of the reducer-architecture work.
- **Doesn't persist saga state across launcher restart.** In-flight sagas are abandoned on launcher crash. Renderer-side timeout safety net handles the renderer-visible consequence. Persistent sagas are Phase F or beyond.
- **Doesn't migrate `BlockController` or `MessageBus` into the srv reducer.** Same scaffolding-vs-reducer discipline as host's `browsers`.
