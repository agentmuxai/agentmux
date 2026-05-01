# Cross-Process Dispatch — Launcher → Host Command Pipe

**Date:** 2026-05-01
**Status:** Draft 1 — for review
**Depends on:** [`SPEC_SAGA_ARCHITECTURE_TARGET_2026-05-01.md`](./SPEC_SAGA_ARCHITECTURE_TARGET_2026-05-01.md) (read first for context)
**Companion:** [`SPEC_LAUNCHER_SAGA_DURABILITY_2026-05-01.md`](./SPEC_LAUNCHER_SAGA_DURABILITY_2026-05-01.md)

This spec covers Thread 3 of the saga reducer architecture — the launcher → host command wire. After this lands:

- F.5 (`pool_respawn_on_promote`) and F.6 (`window_cleanup_cascade`) sagas stop being narrators and start *driving* host-side actions.
- Saga timeouts and retries become meaningful for `IssueCmd::Host` actions.
- Per-saga event correlation makes concurrent same-kind sagas safe.

This is a single-PR-or-small-batch effort. Estimated scope: ~600–900 LOC, mostly in `agentmux-launcher/src/host_pipe/` (new module) plus surgical edits to `saga/mod.rs`, `ipc/server.rs`, host process code, and Command/Event schemas.

---

## 1. Problem

Today, saga-issued `IssueCmd { target: PipeTarget::Host, cmd }` actions are **logged-only**. See `agentmux-launcher/src/saga/mod.rs::apply_action` — for `Host`, it logs the intent and returns. The saga then proceeds as if the command was dispatched (waits for a `Report*` event from host that the host's *existing implicit code path* eventually emits).

This works for ship-it sagas because:

1. F.5 issues `SpawnPoolWindow` after `PoolWindowPromoted` — but the host already replenishes the pool implicitly inside its `promote_pool_window` function.
2. F.6 issues `ReapPanes` and `DrainPoolIfLast` after `WindowClosed` — but the host already does both inside its `on_before_close` callback.

The saga is a passive observer — it bracket-records lifecycle but doesn't *control* the work.

What this prevents:

- **No retry on failure.** If host's implicit code path fails partway, saga has no way to re-issue.
- **No saga-level timeouts.** Saga's `Wait` state is open-ended; host could silently never emit the awaited event.
- **No new host-driven actions.** Any feature that doesn't already happen implicitly (forced pool drain on quota, force-quit agent's windows, etc.) can't be added until this wire exists.
- **No crash recovery for in-flight host actions.** If launcher crashes after issuing `ReapPanes` but before `PanesReaped` arrives, there's no record of the intent (saga-as-narrator only logs after dispatch, which never happened).

---

## 2. Goal

Build a **launcher → host command channel** that:

1. Transmits saga-issued `Command` payloads to host over an existing or new pipe.
2. Carries `saga_id` correlation in both directions (Command + Report* events).
3. Surfaces transmission failures (broken pipe, host crash) as saga-visible errors.
4. Reconnects automatically on host respawn.
5. Integrates with launcher saga durability (companion spec) so dispatched-but-incomplete commands are recoverable.

**Non-goals:**

- Renderer ↔ host direct connection. Renderer talks to srv; srv events flow to launcher; launcher drives host. Host is a leaf, not a peer.
- Separate disk persistence in host. Host reflects launcher state; if it falls out of sync, host restart resyncs from launcher's event log.
- Multi-host scaling. Single-machine.

---

## 3. Design

### 3.1 Channel choice

**Use the existing `command pipe`** (`\\.\pipe\agentmux-{hash}\command`) **bidirectionally.**

Today this pipe carries:

- host → launcher: `Report*` Commands (host as client, launcher as server)
- launcher → host: `Event::*` from launcher's event bus (server-pushed)

We extend it with:

- launcher → host: **saga-issued Commands** (server-pushed Commands, alongside Events)

This means the host's read loop, which currently expects only `Event` JSON frames, now expects a tagged union of `Event` or `Command`. We add an envelope:

```rust
// agentmux-common/src/ipc.rs (new)
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HostFrame {
    Event(Event),
    Command(Command),  // saga-issued; carries saga_id
}
```

Host frames are still newline-delimited JSON. Host parses each line, dispatches by `kind`.

**Why not a new pipe?** Adds connection management overhead, doubles handshake latency on host start, and we already have ordering guarantees on the existing pipe. One pipe = one ordering domain; sagas can rely on "a Command issued at T1 arrives before an Event published at T2 > T1."

**Why not multiplex with a separate channel for saga commands only?** Same answer — ordering. If we bifurcate, we have to add explicit happens-before markers across channels. Single pipe is simpler.

### 3.2 Command payloads

The Commands relevant to host (the ones that today exist as misroute-error stubs in launcher reducer):

- `SpawnPoolWindow { saga_id }` — replenish pool by one window
- `ReapPanes { label, saga_id }` — close all panes in window `label`
- `DrainPoolIfLast { label, saga_id }` — if `label` is the last user-facing window, drain the pool

`saga_id: u64` is **mandatory on every host-bound command** so the host can echo it on the corresponding `Report*` Command back up to launcher.

Future host-bound commands (post-Thread-3) follow the same pattern. Host-bound commands must:

1. Carry `saga_id`
2. Have a corresponding `Report*` Command that carries the same `saga_id`
3. Be idempotent or carry an idempotency token (the saga_id alone suffices for one-shot commands; for repeatable commands like `SpawnPoolWindow` the `saga_id` is the idempotency token — host must dedupe by saga_id within a window).

### 3.3 Report* echo of saga_id

The host's `Report*` Commands (host → launcher) must include `saga_id: Option<u64>` so:

- if the report is a *response to a saga-issued command*, `saga_id` is `Some(N)` matching the original command
- if the report is *organic* (user closed a window, host detected pool replenishment from non-saga-driven code path), `saga_id` is `None`

Launcher reducer copies the `saga_id` from the report Command into the resulting Event:

```rust
// before
Event::PanesReaped { label }

// after
Event::PanesReaped { label, saga_id: Option<u64> }
```

The saga's `on_event()` filters by `event.saga_id == self.expected_saga_id`. Mismatched events return `SagaAction::Wait`.

This is the **per-saga correlation** mechanism (target spec §3.7). It retires evict-and-replace as a workaround.

### 3.4 SagaCoordinator dispatch

`apply_action()` for `IssueCmd { target: Host, cmd }`:

```rust
// before (today)
SagaAction::IssueCmd { target: PipeTarget::Host, cmd } => {
    log::info!("[saga {}] IssueCmd::Host (log-only): {:?}", saga_id, cmd);
    true  // still in_flight
}

// after
SagaAction::IssueCmd { target: PipeTarget::Host, cmd } => {
    let cmd_with_id = inject_saga_id(cmd, saga_id);
    match self.host_pipe.send_command(&cmd_with_id).await {
        Ok(()) => true,  // dispatched; saga waits for echo via on_event
        Err(e) => {
            log::error!("[saga {}] host pipe send failed: {}", saga_id, e);
            self.emit_failed(saga_id, &format!("host pipe send failed: {}", e));
            false  // remove from in_flight; saga is Failed
        }
    }
}
```

`inject_saga_id(cmd, saga_id)` is a small helper that pattern-matches the Command variant and fills in the `saga_id` field. Forgetting to inject for a new variant should be a compile error — use exhaustive match with `_` arm that panics in debug, returns Err in release (defense in depth).

`self.host_pipe` is a new `Arc<HostPipe>` field on `SagaCoordinator`, holding the writer half of the host pipe.

### 3.5 HostPipe wrapper

New module `agentmux-launcher/src/host_pipe/`:

- `mod.rs` — public `HostPipe` struct + `send_command()` method
- `connection.rs` — internal: holds connected pipe writer, reconnect logic
- `tests.rs` — unit tests with in-memory mock pipe

```rust
pub struct HostPipe {
    inner: Arc<RwLock<HostPipeInner>>,
}

struct HostPipeInner {
    writer: Option<NamedPipeServer>,  // None when host is down
    reconnect_notify: Notify,
    pending_buffer: VecDeque<HostFrame>,  // bounded; drops oldest on overflow
}

impl HostPipe {
    pub async fn send_command(&self, cmd: &Command) -> Result<(), HostPipeError> {
        let inner = self.inner.read().await;
        match &inner.writer {
            Some(w) => write_frame(w, &HostFrame::Command(cmd.clone())).await,
            None => Err(HostPipeError::HostNotConnected),
        }
    }

    pub async fn send_event(&self, event: &Event) -> Result<(), HostPipeError> {
        // already-existing event fanout, refactored to go through this struct
    }

    pub async fn run_connection_loop(&self, accept: NamedPipeServer) {
        // accept new host; on disconnect, log + retry; manages pending_buffer drain
    }
}

pub enum HostPipeError {
    HostNotConnected,
    WriteFailed(io::Error),
    Serialize(serde_json::Error),
}
```

`pending_buffer`: when `IssueCmd::Host` fires while host is reconnecting, we *don't* fail the saga — we buffer the command (up to a bound — say 64 frames). On reconnect, we drain in order. If the buffer overflows, the oldest pending command's saga gets `SagaFailed { reason: "host pipe backpressure dropped command" }`.

This buys saga resilience to brief host crashes without making every saga retry-aware.

### 3.6 Routing rules

Saga's `apply_action` decides target by `PipeTarget`:

- `PipeTarget::LauncherSelf` → in-process: call launcher reducer directly via `state.dispatch_local(cmd)` (new method, mirrors srv saga dispatch path)
- `PipeTarget::Host` → `host_pipe.send_command(cmd_with_saga_id)`
- `PipeTarget::Srv` → reserved; future class D/E sagas. Out of scope here.

`PipeTarget::LauncherSelf` is currently `#[allow(dead_code)]` because no saga uses it. Wire it up symmetrically to `Host` so it's exercise-able even before a real consumer arrives — saves a follow-up PR.

### 3.7 Host-side changes

Host process (`agentmux-cef/src/...` — find the IPC client):

1. **Read loop:** parse each line as `HostFrame` (envelope). Dispatch by `kind`:
   - `event` → existing event-handling code (state sync)
   - `command` → new command-handling code (saga-driven actions)
2. **Command handlers:**
   - `Command::SpawnPoolWindow { saga_id }` → spawn pool window; on success emit `Command::ReportPoolWindowAdded { label, saga_id: Some(saga_id) }` back up the pipe
   - `Command::ReapPanes { label, saga_id }` → reap; emit `Command::ReportPanesReaped { label, saga_id: Some(saga_id) }`
   - `Command::DrainPoolIfLast { label, saga_id }` → check; emit `Command::ReportPoolDrainDecision { label, drained: bool, saga_id: Some(saga_id) }`
3. **Idempotency:** track most-recent N saga_ids per command kind in a small LRU; if a duplicate arrives (e.g. launcher reissued after partial timeout), the host already-completed it and re-emits the same Report.
4. **Error reporting:** if a command fails (window not found, etc.), emit a corresponding `Report*` with a failure-flag field, OR emit a generic `Command::ReportSagaActionFailed { saga_id, reason }`. Recommend the latter for v1 — keeps the schema small.

### 3.8 Reducer arms

Launcher reducer adds Event-emission arms for the `Report*` Commands when carrying saga_id:

- `Command::ReportPanesReaped { label, saga_id }` → `Event::PanesReaped { label, saga_id }`
- `Command::ReportPoolDrainDecision { label, drained, saga_id }` → either `Event::PoolDrained { label, saga_id }` or `Event::PoolNotLast { label, saga_id }` (existing F.6 schema)
- `Command::ReportPoolWindowAdded { label, saga_id }` → `Event::PoolWindowAdded { label, saga_id }`

Existing Report arms unchanged for `saga_id: None` cases (organic events).

Launcher reducer also handles a new arm:

- `Command::ReportSagaActionFailed { saga_id, reason }` → `Event::SagaFailed { saga_id, reason }` (or `Event::Error` if we want to distinguish; recommend a distinct `SagaActionFailed` event for surface clarity)

The saga coordinator's bus loop receives `SagaActionFailed` and treats it as a saga termination signal: emit the saga's `SagaFailed` and remove from in_flight.

### 3.9 Connection lifecycle

Host pipe connection states (launcher's view):

```
[disconnected] --(host accepts)--> [connected] --(send fails)--> [disconnected]
     ^                                  |                              |
     |                                  v                              v
     +----- (host respawn) -----[draining_pending]<---(reconnect)---+
                                                                    |
                                                                    v
                                                              [connected]
```

Drain semantics: on reconnect, replay `pending_buffer` in FIFO order. If a buffered command's saga has already terminated (timed out + compensated while host was down), it's still re-sent — host's idempotency LRU absorbs it.

**Bound on disconnection time:** if host is down for >30s, drop pending buffer + emit `SagaFailed { reason: "host unreachable" }` for every saga whose dispatch was buffered. 30s is arbitrary; revisit after soak.

### 3.10 Saga timeouts

With cross-process dispatch wired, each `IssueCmd::Host` action gets an *implicit* timeout via the existing `run_saga()` 5s wrapper (srv side; launcher coordinator has no timeout today — companion spec adds it).

For long-running host actions (e.g. drain of a workspace with many panes), 5s may be too short. Add a per-saga timeout override:

```rust
trait Saga {
    fn timeout(&self) -> Duration { Duration::from_secs(5) }
    // existing methods
}
```

Default 5s; overridable per saga. F.6 cascade saga should override to ~30s.

---

## 4. Implementation plan

PRs in order, each independently mergeable:

### PR 1: Schema additions (~150 LOC)

- Add `saga_id` field to `Command::SpawnPoolWindow`, `Command::ReapPanes`, `Command::DrainPoolIfLast` (mandatory, `u64`).
- Add `saga_id: Option<u64>` to corresponding `Report*` Commands and resulting Events.
- Add `Command::ReportSagaActionFailed { saga_id: u64, reason: String }` + `Event::SagaActionFailed { saga_id: u64, reason: String }`.
- Add `HostFrame { Event, Command }` envelope.
- Update reducer arms to plumb `saga_id` through.
- Update existing tests; add fixture serializing/deserializing both arms of `HostFrame`.

This PR is purely schema — no behavior change. Reagent + codex will likely find no issues; it's a soak step before the wire goes live.

### PR 2: Host pipe wrapper + framing (~300 LOC)

- New module `agentmux-launcher/src/host_pipe/`.
- Refactor existing event fanout to host through `HostPipe::send_event()`.
- Add `send_command()` (reads from broadcast bus is unchanged; this is push-down only).
- Connection loop with reconnect + bounded `pending_buffer`.
- Unit tests with mock pipe that drops/holds/reorders frames.
- No saga-side wiring yet — `apply_action` for `Host` still log-only. PR 2 just establishes the infrastructure.

### PR 3: Wire saga dispatch (~200 LOC)

- `apply_action` dispatches via `host_pipe.send_command()` instead of logging.
- Add `inject_saga_id()` helper.
- Add per-saga `timeout()` method to Saga trait; default 5s, F.6 overrides to 30s.
- Saga coordinator listens for `Event::SagaActionFailed` and terminates the matching saga.
- F.5 + F.6 stop being narrators; full integration test that crashes host mid-saga and verifies saga fails-out.

### PR 4: Per-saga correlation (~150 LOC)

- Saga `on_event()` filters by `event.saga_id == self.expected_saga_id` (PoolRespawn + WindowCleanupCascade).
- Remove evict-and-replace policy from coordinator (the workaround is no longer needed).
- Concurrent-promote and concurrent-window-close test cases now pass without false-positive saga failures.

### PR 5: Host-side saga_id LRU (~100 LOC, host code)

- Add idempotency LRU keyed by (saga_id, command-kind).
- Test: send same command twice; second send re-emits same Report.

Merging these together is a ~900 LOC effort. Splitting per the above keeps each PR bot-reviewable and lets reagent/codex catch a single concern at a time.

---

## 5. Risks + open questions

1. **Schema change on hot pipe.** Adding `saga_id` to existing Commands breaks any in-flight host. We rebuild on every release, so no real-world consumer cares — but during PR 1 soak, in-progress host instances connecting to a new launcher will fail to parse. *Mitigation:* `#[serde(default)]` on `saga_id` for one release cycle; remove the default in PR 4 once we've shipped the pair.
2. **Pending buffer overflow under host crash loops.** If host crashes-respawns rapidly, sagas pile up in buffer. *Mitigation:* bound is 64; oldest gets `SagaFailed`. May be too small; revisit on soak.
3. **Per-saga `timeout()` doesn't help when host never replies but reports a different saga.** If host's saga_id LRU drops the original (LRU eviction), the saga waits forever. *Mitigation:* LRU bound >> typical concurrent-saga count (say 256); plus saga timeout fires regardless.
4. **`PipeTarget::LauncherSelf` activation.** We're wiring it preemptively (no consumer). Risk is unused complexity rotting. *Mitigation:* small surface, exercised by tests; if we delete it later, the diff is contained.
5. **Recovery if launcher crashes mid-dispatch.** PR 3 wire is *transmit-then-wait*. If launcher crashes after `host_pipe.send_command()` returns Ok but before saga marks it succeeded, on restart the launcher saga (without durability — companion spec) is gone; host has already actioned the command and emitted the Report; launcher's *reducer event log* (which has durability) preserves the Report → so on restart, the new launcher process can replay events from event log to reach correct state. **The saga itself is forfeit, but state is correct.** This is acceptable for class C sagas.

---

## 6. Acceptance criteria

A PR landing each of the 5 sub-PRs is "done" when:

- All cargo tests in `agentmux-launcher` and `agentmux-cef` pass.
- Property tests in `saga/integration_tests.rs` (mirroring F.7 pattern) verify:
  - Saga issued command → Report received → Event emitted → saga.on_event matches → saga terminates Completed
  - Concurrent same-kind sagas correlate by saga_id; no premature SagaFailed
  - Host disconnect drops pending sagas as `SagaFailed { reason: "host unreachable" }` after 30s
- Manual smoke test: open AgentMux, open + close 3 windows rapidly, observe in `~/.agentmux/log/launcher.log`:
  - Each WindowCleanupCascade saga fires once
  - Each completes with a single SagaCompleted bracket (not a SagaFailed/SagaCompleted pair from eviction)
- No `#[allow(dead_code)]` markers remain on `PipeTarget::Host` or `SagaAction::Failed`.

---

## 7. Cross-references

- Architecture target: [`SPEC_SAGA_ARCHITECTURE_TARGET_2026-05-01.md`](./SPEC_SAGA_ARCHITECTURE_TARGET_2026-05-01.md) §3.5, §3.7
- Companion durability spec: [`SPEC_LAUNCHER_SAGA_DURABILITY_2026-05-01.md`](./SPEC_LAUNCHER_SAGA_DURABILITY_2026-05-01.md)
- F.5 saga: `agentmux-launcher/src/saga/pool_respawn.rs`
- F.6 saga: `agentmux-launcher/src/saga/window_cleanup.rs`
- Saga coordinator: `agentmux-launcher/src/saga/mod.rs`
- Existing IPC server: `agentmux-launcher/src/ipc/server.rs`

---

End of cross-process dispatch spec.
