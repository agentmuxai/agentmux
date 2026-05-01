# Launcher Saga Durability + Recovery

**Date:** 2026-05-01
**Status:** Draft 1 — for review
**Depends on:** [`SPEC_SAGA_ARCHITECTURE_TARGET_2026-05-01.md`](./SPEC_SAGA_ARCHITECTURE_TARGET_2026-05-01.md)
**Companion:** [`SPEC_CROSS_PROCESS_DISPATCH_2026-05-01.md`](./SPEC_CROSS_PROCESS_DISPATCH_2026-05-01.md)

This spec adds **durability + crash recovery** to launcher-side sagas, mirroring what srv already has (#634 + #636) at smaller scale. It's the second half of "the full saga reducer architecture": after cross-process dispatch lands, sagas can do real work; this spec makes that work crash-safe.

Estimated scope: ~500–700 LOC, mostly new files under `agentmux-launcher/src/saga/log/` plus surgical edits to `saga/mod.rs` and `main.rs`.

---

## 1. Problem

Launcher-side sagas (F.5 PoolRespawn, F.6 WindowCleanupCascade) live entirely in memory:

- `SagaCoordinator::in_flight: Mutex<HashMap<u64, Box<dyn Saga>>>` — wiped on launcher restart.
- No persistent log of saga lifecycle events (the reducer event log has `Event::SagaStarted/Completed/Failed`, but those are after-the-fact bracket markers, not step-by-step state).
- If launcher crashes mid-saga:
  - The saga's effects-so-far (host-side panes reaped, pool drained) may be partially applied.
  - On restart, launcher has no idea the saga existed.
  - Whatever the saga *would have* driven via cross-process dispatch (post-Thread-3) is now in limbo.

For class C sagas (launcher → host), this is *almost* fine — the launcher's reducer event log captures the host-emitted `Report*` events, and on restart the launcher can replay events to reach correct state. The saga is forfeit, but state is consistent.

But "almost" breaks down when:

1. **A saga issued an `IssueCmd::Host` and then launcher crashed before host replied.** Host's reply lands at no listener; the action might or might not have completed; nobody is tracking. Operator review needed, but there's no breadcrumb for the operator.
2. **A saga is mid-compensation.** It walked some forward steps, drove inverses, and crashed. On restart there's no trace of what it was compensating, so the operator has to reverse-engineer from event log + screenshots.
3. **Future class D/E sagas.** When sagas span srv + launcher, srv has durability but launcher doesn't. The operator would see a half-recovered picture.

---

## 2. Goal

Add a **launcher saga log** with:

- Per-saga lifecycle row (`saga_id`, `name`, `state`, started/ended timestamps, input snapshot)
- Per-step rows (`step_index`, `name`, `state`, command JSON, output events JSON, started/ended timestamps)
- Recovery walker on startup that:
  - Surfaces unresolved sagas to operator (via `--diag sagas` and a structured log line)
  - Marks them `failed_compensation { reason: "launcher restart" }` (best-effort; we don't auto-recover launcher sagas — see §3.5)
- Bounded retention (delete completed sagas older than N days)

**Non-goals:**

- Auto-replay of launcher sagas on restart. Launcher sagas drive host-side effects we can't safely re-issue (they may have already happened). We *fail-out* unresolved sagas on restart, not replay them.
- Cross-process saga reconciliation. If a srv saga issued a command to launcher and both crashed, srv's recovery handles its half; launcher's recovery handles its half. They don't talk.

---

## 3. Design

### 3.1 Storage

SQLite at `~/.agentmux/launcher-sagas.db`, separate from srv's `sagas.db`. Distinct file because:

- Different process owners → no SQLite contention
- Different schema lifecycle → schema migrations can move at different speeds
- Diagnostic clarity → operators can grep one or the other

WAL mode, 5s busy timeout, foreign keys ON — same as srv saga log.

### 3.2 Schema

```sql
CREATE TABLE launcher_saga (
    saga_id INTEGER PRIMARY KEY,
    name TEXT NOT NULL,
    state TEXT NOT NULL CHECK (state IN ('running', 'completed', 'failed', 'compensating', 'failed_compensation')),
    started_at TEXT NOT NULL,  -- RFC3339
    ended_at TEXT,
    input_json TEXT NOT NULL,  -- saga init args
    failure_reason TEXT
);

CREATE TABLE launcher_saga_step (
    saga_id INTEGER NOT NULL REFERENCES launcher_saga(saga_id) ON DELETE CASCADE,
    step_index INTEGER NOT NULL,
    name TEXT NOT NULL,  -- e.g. "issue_cmd_host_reap_panes"
    state TEXT NOT NULL CHECK (state IN ('pending', 'succeeded', 'failed', 'compensated')),
    cmd_json TEXT,  -- the IssueCmd payload, if applicable
    target TEXT,  -- "launcher_self" | "host" | "srv"
    started_at TEXT NOT NULL,
    ended_at TEXT,
    output_json TEXT,  -- the awaited event, if received
    failure_reason TEXT,
    PRIMARY KEY (saga_id, step_index)
);

CREATE INDEX idx_launcher_saga_state ON launcher_saga(state);
CREATE INDEX idx_launcher_saga_step_state ON launcher_saga_step(saga_id, state);
```

Schema mirrors srv's `sagas.db` with these differences:

- `target` column on step (since launcher sagas dispatch to multiple targets — self, host, srv)
- No `compensated` state for the saga (launcher sagas don't auto-compensate; see §3.5)
- `failed_compensation` is the recovery-marked terminal state for unresolved sagas at startup

### 3.3 LauncherSagaLog API

New module `agentmux-launcher/src/saga/log/`:

```rust
pub struct LauncherSagaLog {
    conn: Mutex<Connection>,
}

impl LauncherSagaLog {
    pub fn open(path: PathBuf) -> Result<Self, LogError>;
    pub fn open_in_memory() -> Result<Self, LogError>;  // for tests

    // Lifecycle
    pub fn start_saga(&self, saga_id: u64, name: &str, input: &Value) -> Result<(), LogError>;
    pub fn terminate_saga(&self, saga_id: u64, outcome: SagaOutcome) -> Result<(), LogError>;

    // Steps
    pub fn start_step(&self, saga_id: u64, idx: u32, name: &str, target: PipeTarget, cmd: &Command) -> Result<(), LogError>;
    pub fn finish_step(&self, saga_id: u64, idx: u32, output: &Event) -> Result<(), LogError>;
    pub fn fail_step(&self, saga_id: u64, idx: u32, reason: &str) -> Result<(), LogError>;

    // Recovery
    pub fn unresolved_sagas(&self) -> Result<Vec<UnresolvedLauncherSaga>, LogError>;
    pub fn mark_failed_compensation(&self, saga_id: u64, reason: &str) -> Result<(), LogError>;

    // Maintenance
    pub fn snapshot_recent(&self, n: usize) -> Result<Vec<SagaSummary>, LogError>;  // for --diag
    pub fn vacuum_older_than(&self, cutoff: DateTime<Utc>) -> Result<usize, LogError>;  // retention

    // Schema seed
    pub fn max_saga_id(&self) -> Result<u64, LogError>;  // for next_saga_id init
}
```

Method names match srv's `SagaLog` where possible (so anyone reading both feels at home).

### 3.4 Coordinator integration

`SagaCoordinator` gains an `Arc<LauncherSagaLog>` field. Methods that change saga state also write to the log:

```rust
pub fn spawn_saga(&self, saga: Box<dyn Saga>) -> u64 {
    let saga_id = self.next_id();
    let name = saga.name();
    let input = saga.input_snapshot();  // new method on Saga trait
    self.log.start_saga(saga_id, name, &input).expect("saga log write");
    self.emit_started(saga_id, name);
    let action = saga.start(&self.ctx(saga_id));
    self.apply_action(saga_id, name, action);
    saga_id
}

fn apply_action(&self, saga_id: u64, name: &str, action: SagaAction) -> bool {
    match action {
        SagaAction::IssueCmd { target, cmd } => {
            let step_index = self.alloc_step_index(saga_id);
            let step_name = derive_step_name(&cmd);
            self.log.start_step(saga_id, step_index, &step_name, target, &cmd)?;
            // ... existing dispatch (host_pipe / launcher reducer / srv pipe)
            true
        }
        SagaAction::Done => {
            self.log.terminate_saga(saga_id, SagaOutcome::Completed)?;
            self.emit_completed(saga_id);
            false
        }
        SagaAction::Failed { reason } => {
            self.log.terminate_saga(saga_id, SagaOutcome::Failed { reason: reason.clone() })?;
            self.emit_failed(saga_id, &reason);
            false
        }
        SagaAction::Wait => true,
    }
}

fn route_event_to_sagas(&self, event: &Event) {
    for saga_id in self.in_flight_ids() {
        // ... existing on_event routing
        // when a saga's on_event "consumes" a target event (succeeded step):
        //   self.log.finish_step(saga_id, step_index, event)?;
    }
}
```

The hard question: **how does the coordinator know which step_index a given event "completes"?** Per-saga state has to track `awaiting_step: Option<u32>`. When the coordinator dispatches an `IssueCmd`, it allocates step_index and stashes it on the in_flight saga record. When `on_event` consumes the awaited event (returning anything other than `Wait`), the coordinator calls `finish_step(awaiting_step)`.

This requires a small extension to the `Saga` trait or to the coordinator's per-saga record. Recommend a wrapper struct:

```rust
struct InFlightSaga {
    saga: Box<dyn Saga>,
    awaiting_step: Option<u32>,
}
```

The coordinator owns `HashMap<u64, InFlightSaga>` instead of `HashMap<u64, Box<dyn Saga>>`. Minimal API surface change.

### 3.5 Recovery on startup

On launcher boot (in `main.rs`, before `run_coordinator()`):

```rust
let unresolved = saga_log.unresolved_sagas()?;
for saga in unresolved {
    log::warn!(
        "[saga-recovery] saga {} ({}) was {} when launcher last exited; marking failed_compensation",
        saga.saga_id, saga.name, saga.state
    );
    saga_log.mark_failed_compensation(
        saga.saga_id,
        &format!("launcher restarted while saga in state '{}'", saga.state),
    )?;
}
log::info!("[saga-recovery] processed {} unresolved sagas", unresolved.len());
```

**We do not auto-replay or compensate launcher sagas.** Reasons:

1. Their effects (host-side reap, pool drain) are already partially applied to live OS state. Re-running them might double-act.
2. The launcher reducer event log captures all observable state changes; replaying the saga doesn't add information.
3. Operator review is the right escape hatch.

The operator sees these via `agentmux --diag sagas` (companion command, mirrors srv's `--diag sagas`):

```
$ agentmux --diag sagas
Recent launcher sagas (last 50):
  saga_id=1234 name=window_cleanup_cascade state=failed_compensation
    started=2026-05-01T14:05:23Z ended=2026-05-01T14:11:17Z (recovered on restart)
    failure: launcher restarted while saga in state 'running'
    steps:
      0  issue_cmd_host_reap_panes        target=host  state=succeeded   cmd={"label":"win-3"}
      1  await_panes_reaped               target=host  state=succeeded   evt={"label":"win-3"}
      2  issue_cmd_host_drain_pool        target=host  state=pending     cmd={"label":"win-3"}
      [step 2 was in-flight when launcher exited]
```

This gives the operator enough to manually inspect host state (was the pool actually drained?) and decide whether to take action.

### 3.6 Retention

Launcher saga log grows unbounded otherwise. Add a vacuum task that runs once per launcher startup:

```rust
let cutoff = Utc::now() - chrono::Duration::days(7);
let removed = saga_log.vacuum_older_than(cutoff)?;
log::info!("[saga-log] vacuumed {} sagas older than {}", removed, cutoff);
```

Retention is 7 days for completed/failed/compensated sagas. Sagas in `running` or `compensating` state are never vacuumed (would mask in-flight crashes).

7 days is arbitrary; tune based on `--diag sagas` use cases.

### 3.7 Performance

Launcher saga rate is much lower than srv saga rate:

- F.5 fires once per pool-window-promote (rare; user-triggered)
- F.6 fires once per window close (occasional)

A SQLite write per step on this rate is invisible (microseconds). No buffering needed; synchronous writes are fine.

Future class D/E sagas may push higher rates if they involve loops (e.g. force-quit-agent iterating over windows). Revisit if measured.

---

## 4. Implementation plan

### PR 1: Schema + LauncherSagaLog API (~250 LOC)

- New `agentmux-launcher/src/saga/log/mod.rs`, `schema.rs`, `tests.rs`.
- Implement `open`, `start_saga`, `terminate_saga`, `start_step`, `finish_step`, `fail_step`, `unresolved_sagas`, `mark_failed_compensation`, `max_saga_id`, `snapshot_recent`, `vacuum_older_than`.
- Tests for round-trip serialization, idempotent compensation marks, schema migrations.
- No coordinator integration yet — the log exists in isolation.

### PR 2: Coordinator integration (~200 LOC)

- Add `Arc<LauncherSagaLog>` to `SagaCoordinator`.
- Wrap in_flight sagas in `InFlightSaga { saga, awaiting_step }`.
- Wire `start_saga` / `terminate_saga` / `start_step` / `finish_step` / `fail_step` calls into `spawn_saga`, `apply_action`, `route_event_to_sagas`.
- Tests verify a saga's full lifecycle leaves the expected log state.

### PR 3: Recovery walker + --diag sagas (~150 LOC)

- Add startup recovery in `main.rs`.
- Extend `--diag sagas` to query launcher saga log alongside srv saga log.
- Add the formatted printer.
- Integration test: simulate crash by abruptly dropping coordinator mid-saga; restart; verify saga marked `failed_compensation`.

### PR 4: Retention vacuum (~50 LOC)

- Add `vacuum_older_than` call to startup.
- Configurable retention via `~/.agentmux/config.toml` (e.g. `[saga.launcher] retention_days = 7`).

Total: ~650 LOC across 4 PRs. PR 1 and PR 2 are big enough to warrant their own review cycles; PR 3 and PR 4 are small.

---

## 5. Risks + open questions

1. **SQLite at startup adds boot latency.** Estimate: <50ms for open + migrate + recovery walk + vacuum. If this becomes a problem (it shouldn't), defer recovery to a background task.
2. **Schema migrations.** First version. We commit to never breaking schema in-place — only additive changes via `ALTER TABLE` in migration files. Document this in the schema module.
3. **LauncherSagaLog and srv SagaLog drift.** They share design but have different schemas (target column, lacking compensated terminal). Keep an eye on naming/method symmetry; if they diverge a lot, factor common bits into a `saga-log-core` crate later.
4. **Saga that's mid-step at crash time has step state 'pending' AND the corresponding action was actually transmitted to host.** On restart we mark the saga `failed_compensation`. Host may still send the Report → it lands at the new launcher process which has no idea what saga it relates to → reducer event log records it as organic (saga_id=None). State is correct; saga lifecycle is lost. **This is acceptable** — operator can inspect step rows to see what was attempted.
5. **Concurrency between recovery walker and bus loop.** Recovery walker runs BEFORE coordinator's `run()`. If it doesn't finish before the first event arrives, the coordinator may try to write to log entries that recovery already touched. *Mitigation:* sequential by `await`. Don't spawn coordinator until recovery returns.

---

## 6. Acceptance criteria

- All cargo tests in `agentmux-launcher` pass (including new saga log tests + integration tests).
- Manual test: kill -9 launcher mid-`window_cleanup_cascade` saga; restart; verify:
  - `agentmux --diag sagas` shows the saga as `failed_compensation`
  - Step rows show what was attempted
  - Subsequent F.6 sagas work normally
- `~/.agentmux/launcher-sagas.db` does not exceed 10 MB after 1000 simulated sagas (vacuum is working).
- No `#[allow(dead_code)]` markers introduced and not used.

---

## 7. Open design questions for review

- **Should `Compensated` exist as a launcher saga state?** Today F.5 + F.6 don't compensate. Future sagas might. Recommend leaving the column off until we have a saga that uses it; trivial migration to add later.
- **Should the launcher saga log live in the same SQLite file as the launcher reducer event log?** Pros: one connection, one transaction boundary. Cons: more lock contention, harder to vacuum independently. Recommend separate file (current spec).
- **Should saga input snapshots include reducer state?** Useful for replaying scenarios in tests. Risky for size (state can be large). Recommend NOT serializing state into saga log; rely on reducer event log for state replay.

---

## 8. Cross-references

- Architecture target: [`SPEC_SAGA_ARCHITECTURE_TARGET_2026-05-01.md`](./SPEC_SAGA_ARCHITECTURE_TARGET_2026-05-01.md) §3.6
- Cross-process dispatch: [`SPEC_CROSS_PROCESS_DISPATCH_2026-05-01.md`](./SPEC_CROSS_PROCESS_DISPATCH_2026-05-01.md)
- Srv saga durability spec (existing pattern to mirror): [`SPEC_SAGA_DURABILITY_2026-05-01.md`](./SPEC_SAGA_DURABILITY_2026-05-01.md)
- Srv saga log: `agentmux-srv/src/sagas/log.rs`
- Srv recovery: `agentmux-srv/src/sagas/recovery.rs`

---

End of launcher saga durability spec.
