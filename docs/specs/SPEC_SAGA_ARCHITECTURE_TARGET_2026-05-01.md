# Saga + Reducer + IPC Architecture — Target End State

**Date:** 2026-05-01
**Status:** Draft 1 — for review
**Audience:** AgentA, future contributors, reagent/codex reviewing PRs that touch these surfaces.

This spec is the **unified target picture** for AgentMux's saga + reducer + IPC system. It does not specify implementation steps — that's the job of the companion specs:

- `SPEC_CROSS_PROCESS_DISPATCH_2026-05-01.md` — Launcher→Host command pipe (Thread 3, the biggest gap).
- `SPEC_LAUNCHER_SAGA_DURABILITY_2026-05-01.md` — durability + recovery for launcher-side sagas.

Read this doc first to understand *where* we're going. Then read the companion specs to understand *how* the next PRs get there.

---

## 1. Why this exists

AgentMux runs as **three cooperating processes** on the same machine, plus the renderer (JS/TS frontend) running inside CEF render workers:

| Process | Role | What it owns |
|---------|------|--------------|
| **launcher** (`agentmux-launcher`) | top-level native Rust binary; spawns srv + host under Job Object J0 (KILL_ON_JOB_CLOSE); runs launcher reducer + saga coordinator | window/pane state machine, pool of pre-warmed CEF windows, IPC pipes |
| **srv** (`agentmux-srv`, sidecar) | RPC server for the renderer; state-of-record | workspaces, tabs, blocks, controllers, srv saga durability log |
| **host** (`agentmux-cef`, CEF main) | CEF process; spawns + manages CEF render workers; executes window/pane operations on launcher's behalf | live CEF state — render-worker pool, per-pane DevTools, browser profiles |
| **renderer** (`frontend/`, JS/TS) | UI inside each CEF render worker | client-side view-model; sends RPCs to srv |

These processes share state through **named-pipe IPC** (Windows; UDS on other OSes) carrying a stream of `Command` requests and `Event` responses, both encoded as newline-delimited JSON.

The **saga** is the unit of orchestrated state change that crosses one or more processes. Sagas wrap reducer dispatches with:

- **lifecycle brackets** (`SagaStarted` → `SagaCompleted` / `SagaFailed`)
- **per-step durability** (so a crash mid-saga can replay or compensate)
- **compensation** (best-effort inverse for forward steps that succeeded before a failure)

We arrived here piecemeal — Phase E pulled srv state under a reducer; Phase E.5–E.7 added the srv saga framework + durability + integration tests; Phase F added a launcher reducer + launcher-side sagas (F.5 PoolRespawn, F.6 WindowCleanupCascade) and the saga-as-narrator pattern.

What's left is the connecting tissue. **The whole point of this doc is to describe the system *as if it already worked end-to-end*** — so we have a clear target to drive PRs toward, and so we can recognize when a proposed change deviates from the target.

---

## 2. Inventory: what exists today (2026-05-01)

### Srv side

**Reducer** (`agentmux-srv/src/reducer.rs`) — pure function `update(state, cmd, ctx) -> Vec<Event>`. Every state mutation in srv goes through here. Total function (no panic on input). 26 command arms covering workspaces, tabs, blocks, windows, focus/magnify, meta updates, MoveTab, MoveBlock.

**Sagas** (`agentmux-srv/src/sagas/`) — 7 shipped:

1. `tear_off_tab` — migrate tab to new workspace (CreateWorkspace → MoveTab)
2. `restore_torn_off_tab` — move tab back; delete source if empty
3. `tear_off_block` — block → new tab in new workspace (CreateWorkspace → CreateTab → MoveBlock)
4. `promote_block_to_tab` — block → new tab in same workspace (CreateTab → MoveBlock)
5. `delete_tab` — DeleteTab + post-saga controller cleanup
6. `delete_block` — DeleteBlock + post-saga controller cleanup
7. `delete_workspace` — cascade DeleteTab loop → DeleteWorkspace; saga-as-narrator (reducer cascades atomically)

**Saga framework** (`agentmux-srv/src/sagas/mod.rs`) — `SagaCtx { state, saga_id, step_index, forward_step_stack }`; `dispatch()` writes pending → calls reducer → applies events to wstore (SQLite) → marks succeeded → publishes; `compensate()` is best-effort with idempotent forward-step-pop. `run_saga()` wraps in 5s timeout. Terminal: Ok→Completed, "timed out" Err→Failed, other Err→Compensated.

**Saga durability** (`agentmux-srv/src/sagas/log.rs`) — SQLite `sagas.db`, WAL mode, 5s busy timeout. Per-step rows. `unresolved_sagas()` enumerates sagas in non-terminal states for recovery.

**Saga recovery** (`agentmux-srv/src/sagas/recovery.rs`) — at startup, `compensate_unresolved()` walks each unresolved saga's succeeded steps in reverse, derives inverse via `derive_inverse_command()`, dispatches inverse, marks compensated. Hard error if inverse not derivable or if any step is `pending` (mid-dispatch crash).

### Launcher side

**Reducer** (`agentmux-launcher/src/reducer.rs`) — same pattern as srv. Host-only commands (`ReportWindowOpened`, `ReportPoolWindowAdded`, `ReportPoolWindowPromoted`, `ReportPanesReaped`, `ReportPoolDrainDecision`, etc.) gated by `enforce_host_only()`. Misrouted srv-bound commands return `Error("…routed to wrong reducer")`.

**Sagas** (`agentmux-launcher/src/saga/`) — 2 shipped:

1. `pool_respawn_on_promote` (F.5) — triggered by `Event::PoolWindowPromoted`; issues `Command::SpawnPoolWindow` (currently log-only); waits for `Event::PoolWindowAdded` to complete.
2. `window_cleanup_cascade` (F.6) — triggered by `Event::WindowClosed { crash_detected: false }`; issues `Command::ReapPanes` (log-only) → waits for `Event::PanesReaped` → issues `Command::DrainPoolIfLast` (log-only) → waits for `Event::PoolDrained` or `Event::PoolNotLast` to complete.

**Saga framework** (`agentmux-launcher/src/saga/mod.rs`) — `SagaCoordinator { next_saga_id, in_flight: HashMap<u64, Box<dyn Saga>>, events_tx, state }`. Trait `Saga` has `start()` and `on_event()`, both returning `SagaAction { IssueCmd, Done, Wait, Failed }`. `run_coordinator()` consumes the broadcast bus, matches triggers (evict-and-replace for same-kind), feeds events to in-flight sagas, applies actions.

**No durability** — launcher sagas live in memory; if launcher crashes mid-saga, they're abandoned. Restart starts fresh.

### IPC

| Pipe | Direction | Wire format | Status |
|------|-----------|-------------|--------|
| `\\.\pipe\agentmux-{hash}\command` | host ↔ launcher | newline-delimited JSON (Command/Event) | ✅ shipped, bidirectional |
| `\\.\pipe\agentmux-{hash}\srv-command` | renderer ↔ srv (and launcher ↔ srv) | newline-delimited JSON | ✅ shipped, bidirectional |
| **launcher → host (saga IssueCmd)** | launcher → host | (NOT YET WIRED) | ❌ saga-as-narrator only |

Per-connection protocol: Register-first, then any number of Command frames. Server fans out events on a separate task. `GetEvents` is intercepted before reducer (returns `Event::EventList` privately to caller).

### The gap

Every cross-process gap traces back to one missing wire: **launcher → host command transmission**. F.5 and F.6 sagas issue `IssueCmd { target: Host, cmd }` actions, but `apply_action()` currently logs them and returns. The host process behaves correctly for now because the existing implicit code path (in `promote_pool_window`, `on_before_close`, etc.) does the work the saga *would* drive. The saga is a passive narrator.

This works for ship-it sagas, but breaks down the moment we want:

- a saga to *retry* a host action on failure
- a saga to *time out* a host action and compensate
- a saga to drive an action that doesn't already happen implicitly (e.g. forced pool drain on quota change)
- crash recovery to *resume* a host action that was in flight

So Thread 3 is "wire `IssueCmd::Host` to a real pipe." Everything downstream depends on it.

---

## 3. Target: the full system

This section describes the system **after** the missing pieces land. Implementation specs (cross-process dispatch, launcher durability) follow this picture.

### 3.1 Process map

```
+----------+        srv-command pipe        +----------+        command pipe         +----------+
| renderer | <-----------------------------> |   srv    |                              |  host    |
| (CEF)    |     newline-delim JSON          | (sidecar)|                              | (CEF)    |
+----------+                                  +----------+                              +----------+
     ^                                              ^                                        ^
     |                                              |                                        |
     |                                  +-----------+--------------+                          |
     |                                  |     launcher (native)    |                          |
     +----------------------------------+  - launcher reducer       +--------------------------+
                                        |  - saga coordinator       |
                                        |  - launcher→host pipe ----+
                                        |  - launcher→srv client    |
                                        +---------------------------+
```

Three pipes, all newline-delimited JSON, all bidirectional:

1. **command pipe** (host ↔ launcher) — already shipped. Carries `Report*` events from host *up* to launcher reducer; carries saga-driven `IssueCmd::Host` commands *down* to host (THIS IS THE NEW WIRE).
2. **srv-command pipe** (renderer/launcher ↔ srv) — already shipped. Carries renderer RPCs to srv reducer; carries srv events *out* to all subscribers (including launcher when it's a client).
3. **(implicit)** launcher→srv saga commands — issued via the same srv-command pipe, with the launcher acting as a client to srv (no new wire needed; just usage).

### 3.2 Saga taxonomy

Sagas are classified by **which reducers they drive**:

| Saga class | Driver process | Reducers driven | Examples |
|------------|----------------|-----------------|----------|
| **A. Pure srv** | srv | srv reducer only | `tear_off_tab`, `delete_block`, `delete_workspace` |
| **B. Pure launcher** | launcher | launcher reducer only | (none yet — `pool_respawn` and `window_cleanup` issue commands to *host*, which has no reducer; from the saga's POV this is class C) |
| **C. Launcher → host** | launcher | launcher reducer + host (no reducer; host is a side-effect target) | `pool_respawn_on_promote`, `window_cleanup_cascade` |
| **D. Cross-process (srv ↔ launcher)** | either | srv reducer + launcher reducer | (NONE YET; example: a renderer-initiated "create new window" that needs srv to register a workspace AND launcher to spawn the actual OS window — currently solved without a saga because both calls are independent) |
| **E. Three-tier (srv → launcher → host)** | srv | srv + launcher + host | (NONE YET; theoretical: a "kill agent" saga that updates srv state, drains its launcher window pool, and reaps host CEF processes) |

**Rule of thumb:** sagas live in the process whose reducer they drive *first*. If a saga needs to drive *both* srv and launcher reducers, it lives in srv (because srv has the durability story and recovery walker; launcher's recovery story is being added in the launcher-durability spec).

Class B and class D are theoretical right now. They'll show up when the renderer needs orchestrated multi-state transitions that today live as ad-hoc client logic.

### 3.3 Command + Event taxonomy

Every Command has a **target reducer** baked into its name + serde routing:

| Prefix / suffix | Routed to | Source |
|-----------------|-----------|--------|
| `CreateWorkspace`, `MoveTab`, `DeleteBlock`, … | srv reducer | renderer or launcher saga |
| `Report*` (e.g. `ReportWindowOpened`, `ReportPanesReaped`) | launcher reducer | host process only |
| `SpawnPoolWindow`, `ReapPanes`, `DrainPoolIfLast` | host (no reducer; side effect) | launcher saga |

Reducers reject misrouted commands with `Error("'X' is a launcher→host command; sent to launcher pipe by mistake")` — already the pattern in `agentmux-launcher/src/reducer.rs`.

Events flow only one direction per process:

- srv reducer events (e.g. `WorkspaceCreated`) → broadcast on srv events_tx → fan out to renderer + launcher (both clients).
- launcher reducer events (e.g. `PoolWindowAdded`, `WindowClosed`) → broadcast on launcher events_tx → fan out to renderer + host (both clients).
- host emits **no events** of its own. It only sends `Report*` *commands* up to launcher; the *launcher reducer* is what produces events that reflect those reports.

This last point is important: **host is a state-modifying client of launcher**, not a peer. Symmetry is intentionally broken.

### 3.4 Sagas that span processes

The big use cases for cross-process sagas (class D and E above):

**Force-quit agent.** (Hypothetical, illustrative.) User clicks "kill all this agent's work." Saga is class E:

1. srv: `MarkAgentTerminated { agent_id }` (state mutation; recoverable)
2. srv: enumerate `block_id`s for that agent → for each, `DeleteBlock { tab_id, block_id }` (cascade)
3. launcher: for each window owned by that agent, `IssueCmd::Host(ReapPanes { label })` then `IssueCmd::Host(DrainPoolIfLast { label })` (existing F.6 cascade subroutine)
4. srv: `MarkAgentReaped { agent_id }` (final mutation)

This saga lives in srv because step 1 is srv and srv has the recovery story. Steps 3 cross to the launcher. Today they'd be impossible to drive transactionally; with cross-process dispatch wired, srv's saga can `IssueCmd::Launcher(...)` and wait for the launcher reducer to emit a corresponding event back to srv.

**Implication: the launcher needs to emit a small set of "saga progress" events that srv can subscribe to.** Already partly done — `Event::SagaStarted/Completed/Failed` exist on both sides. We extend with launcher-side reducer events srv cares about (`WindowClosed`, `PoolDrained`, etc. — already broadcast).

### 3.5 Durable cross-process orchestration

For class D/E sagas to be crash-safe, srv's saga log must record *every* dispatched command, including ones routed to launcher or host. The recipient process doesn't need a separate log — it just emits an event when done, and srv's saga log captures the result via the normal `finish_step()` write.

**Per-step semantics for cross-process dispatch:**

1. srv saga step: `dispatch(Command::ReapPanes { label })` (but this command's target is host, not srv reducer)
2. SagaCtx detects the target via routing rules → marks step `pending` in saga log → forwards command to launcher's command pipe (NOT srv reducer)
3. SagaCtx awaits the *correlated event* from launcher's events_tx (e.g. `Event::PanesReaped { label, saga_id }` — note `saga_id` correlation; see §3.7)
4. On event receipt: SagaCtx marks step `succeeded`, pushes step index on forward stack
5. On timeout: SagaCtx marks step `failed`, returns Err → saga compensation walks forward stack as usual

This is the model that the cross-process dispatch spec implements concretely.

### 3.6 Recovery semantics across processes

When srv crashes mid-cross-process saga:

- launcher may still be processing the issued command → it'll complete and emit the event; nobody is listening; event lands in launcher's event log on disk; saga step row in srv's saga log is `pending` forever.
- srv restarts, runs `compensate_unresolved()`, sees `pending` step → cannot auto-recover (per existing rule) → marks `failed_compensation`, requires operator review.

When launcher crashes mid-saga (its own class C saga, e.g. window_cleanup):

- launcher loses in-memory saga state on restart.
- The saga's effects (host-side panes reaped, pool drained) may have completed or partially completed.
- **This is what `SPEC_LAUNCHER_SAGA_DURABILITY_2026-05-01.md` solves.** The launcher saga log mirrors srv's design at smaller scale.

When host crashes:

- launcher's `command pipe` connection drops → reconnect loop (already shipped) → host reconnects on respawn.
- Any saga waiting for a `Report*` event from host either retries (after timeout) or fails-out on saga timeout → compensation runs (which for class C is typically "log it; next trigger event tries again").

### 3.7 Per-saga event correlation

Today's launcher sagas have a known limitation: if two `PoolWindowPromoted` events fire concurrently, both spawn `pool_respawn_on_promote` sagas, but the first `PoolWindowAdded` event matches *both* sagas (eviction + replacement is the workaround). The proper fix is **per-saga sequence numbers**:

- Every saga-issued Command carries a `saga_id` (already allocated in `SagaCtx`).
- The recipient process echoes `saga_id` on its corresponding `Report*` event.
- The launcher reducer copies `saga_id` from the report into the resulting Event.
- Saga `on_event()` checks `event.saga_id == self.expected_saga_id`; ignores otherwise.

This is invasive (touches Command/Event schemas, host code, launcher reducer arms), but it's the only way to get clean concurrent-saga semantics. Phase F.7 left it open; Thread 3 should land it together with cross-process dispatch (the wire format change is small once we're already adding a wire).

---

## 4. Boundaries and non-goals

**Not in scope for this architecture:**

- **Distributed sagas across machines.** All three processes run on one host. We rely on filesystem ordering, single-writer SQLite, and named-pipe FIFO ordering. If we ever go multi-host, all of this needs to be revisited.
- **Two-phase commit.** Sagas are eventual-consistency with compensation. We accept partial states + record-only compensation for irreversible deletes. There is no global atomic.
- **Hot-reload of sagas.** Saga code changes require a full restart. Recovery is for crashes, not redeploys.
- **Self-healing under reducer schema changes.** If we change a Command variant, in-flight sagas that captured the old variant in saga log become unparseable. Migration is manual; recovery code already gracefully reports them as unrecoverable.

**Explicitly future work, not addressed here:**

- Renderer-initiated cross-process orchestration (the renderer is currently a thin RPC client; class D sagas waiting for renderer participation are out of scope).
- Saga retry policies. Today sagas either succeed or compensate-once. Real retry-with-backoff is future work.
- Saga prioritization or queueing. We rely on broadcast bus order + reducer mutex serialization.

---

## 5. Invariants the system must preserve

These are the load-bearing properties that any future PR touching saga/reducer/IPC must not break:

1. **Reducers are total functions of (State, Command) → Vec<Event>.** No panics on input. Unknown commands return `Error::InvalidCommand`. All input is from named pipes; treat it as adversarial.
2. **Reducers are pure modulo state mutation.** No I/O, no spawning tasks, no timers. Side effects belong in saga steps.
3. **Saga steps are idempotent at the saga-log layer.** `mark_step_compensated` is an UPDATE; replays are safe. `forward_step_stack.pop()` is sequential within a saga's lifetime; compensation is single-threaded per saga.
4. **Each saga has a unique monotonic saga_id.** Allocated via atomic; persisted; collision-checked at startup.
5. **Cross-process commands carry their saga_id.** Reports back from the recipient carry the same id, so the originating saga can correlate against concurrent siblings.
6. **Misrouted commands are explicit errors, not silent drops.** Both reducers reject foreign commands with a fatal `Error` so misuse surfaces immediately.
7. **Crash recovery walks succeeded steps in reverse order.** It does not retry failed forward steps. Pending steps are unrecoverable (operator review).
8. **Compensation is best-effort.** Reducer errors during compensation are logged-but-swallowed; the saga still terminates Compensated. Operators inspect the log for partial states.
9. **Launcher event bus is the single source of truth for launcher state.** Host doesn't have a separate state of record; its state is reflected only in launcher reducer events.
10. **Saga-as-narrator is acceptable as a temporary state.** When a saga issues a command that isn't yet wired to its target process, it logs the intent in the durable saga log and returns Wait/Done as if dispatch happened. The recipient's existing implicit code path provides the side effect. This pattern is *retired* once cross-process dispatch lands.

---

## 6. Map of open work

The companion specs cover the implementation. Cross-references:

- **§3.5 + §3.7** → [`SPEC_CROSS_PROCESS_DISPATCH_2026-05-01.md`](./SPEC_CROSS_PROCESS_DISPATCH_2026-05-01.md) — wire protocol, routing, correlation, retries.
- **§3.6 (launcher half)** → [`SPEC_LAUNCHER_SAGA_DURABILITY_2026-05-01.md`](./SPEC_LAUNCHER_SAGA_DURABILITY_2026-05-01.md) — saga log + recovery walker on launcher side.
- **§3.4 (class D/E examples)** → not specced; on-demand as use cases arise. The cross-process dispatch spec lays the groundwork; building specific class D sagas is per-feature.

Roadmap doc that names the threads + lessons learned: [`../retro/phase-fg-roadmap-2026-05-01.md`](../retro/phase-fg-roadmap-2026-05-01.md).

End-of-batch-2 status: [`../retro/phase-fg-status-2026-05-01.md`](../retro/phase-fg-status-2026-05-01.md).

---

## 7. Glossary

- **Saga** — a coordinated sequence of state-changing commands with lifecycle brackets, durability, and compensation. The unit of multi-step orchestration.
- **Reducer** — a pure function `(State, Command, Ctx) → Vec<Event>` that mutates state and emits events. Total; never panics on input.
- **Event** — a fact emitted by a reducer reflecting a state change. Broadcast on the process's event bus; logged to disk; consumed by sagas + clients.
- **Command** — a request to mutate state. Always targets one reducer (srv or launcher) or one external recipient (host). Carries optional `force` flag for compensation paths.
- **IssueCmd** — saga action variant that emits a Command. Target is `LauncherSelf` (in-process reducer), `Srv` (over the srv-command pipe), or `Host` (over the new launcher→host wire).
- **Compensation** — best-effort inverse of a forward step. Runs when a later forward step fails. Idempotent at the log layer.
- **Saga-as-narrator** — pattern where a saga *logs* an `IssueCmd::Host` to the durable saga log without actually transmitting it to host (because the wire isn't built yet); the host's existing implicit code path provides the side effect. Retired post-Thread-3.
- **Evict-and-replace** — coordinator policy: if a new trigger event matches an in-flight saga of the same kind, evict the in-flight saga (emit `SagaFailed { reason: "evicted" }`) and spawn a fresh one. Trades off premature failure markers to prevent permanent stalls.

---

End of architecture target spec.
