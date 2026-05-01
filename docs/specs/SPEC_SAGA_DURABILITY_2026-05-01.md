# SPEC: Saga durability — durable saga log

**Date:** 2026-05-01
**Status:** Draft — design choices below need user/reviewer sign-off before code.
**Author:** AgentA
**Reads-this-first:**
- `docs/retro/next-steps-architecture-completeness-2026-05-01.md` — step 4.
- `docs/retro/reducer-architecture-gaps-2026-05-01.md` — §3 saga state durability.
- `docs/retro/saga-coordinator-location-analysis-2026-04-30.md` — Phase A coordinator placement.
- `agentmux-srv/src/sagas/mod.rs` — current in-memory saga coordinator (Phase E.5.5 shipped).

---

## 0. The problem this spec solves

Today's saga coordinator (`agentmux-srv/src/sagas/mod.rs`) is **memory-resident**. Saga state lives in the future driving `run_saga` and the lock-protected reducer state. If the srv process crashes mid-saga, that future is dropped, the in-memory state is gone, and the saga's already-applied steps remain in SQLite without any record of which saga produced them.

This is fine for the current saga set (tear-off, restore, promote) — they complete in tens of milliseconds, the failure window is tiny, and recovery via SQLite reconciliation is acceptable.

It is **not** fine for the next class of sagas: long-running flows that take seconds to minutes. The motivating example is "spawn a remote agent, wait for it to register, attach it to a tab" — a saga that waits on external IPC and survives across srv-restart-during-step-2 means the user doesn't lose their work because srv crashed.

This spec turns the saga coordinator into a **durable** state machine: every step transition writes to disk, srv crashes are recoverable, and replay-on-restart resumes or rolls back in-flight sagas.

---

## 1. Scope

**In scope:**
- A durable on-disk record of every saga's lifecycle.
- Resume-on-startup logic: scan the log, finish or compensate any saga that didn't reach a terminal state.
- A new operator command `--diag sagas` showing in-flight + recently-terminal sagas.
- Backwards compatibility: existing saga code (in `sagas/tear_off_tab.rs` etc.) compiles unchanged; durability is added by the coordinator, not the saga authors.
- Recovery tests: kill srv between every pair of steps in every saga, verify recovery.

**Out of scope:**
- **Distributed sagas across srv processes.** Single-srv-process durability only.
- **Saga history retention beyond N days / M sagas.** Default retention TBD per §6; configurable.
- **Compensating an already-compensated saga.** Idempotency relied on, not enforced by the log.
- **Cross-reducer saga coordination across launcher↔host↔srv.** The launcher coordinator (E.1a stub) gets the same durability infra in step 6 (Phase F.5 / F.6); this spec is srv-side.

---

## 2. The four design choices

The rest of this spec depends on these. Listed with my recommendation; these are the ones to push back on if you disagree.

### 2.1 Storage: SQLite vs JSON files

**Recommendation:** SQLite (new tables in the existing `agentmux.db`).

| | SQLite | Per-saga JSON files |
|---|---|---|
| Atomicity vs reducer state | Same DB, same transaction = trivially atomic | Two-store atomicity is a design problem |
| Operator visibility | `sqlite3` queries, `--diag sagas` reads tables | `ls ~/.agentmux/sagas/`, manual JSON parse |
| Retention/cleanup | `DELETE` queries, indexed by `terminal_at` | Filesystem mtime sweeping |
| Crash safety | WAL handles partial writes | Per-file `fsync`; write-temp-then-rename pattern |
| Schema migrations | Existing migration framework | Hand-rolled JSON versioning |

The deciding factor: every saga step today already writes to SQLite via the persist subscriber. Putting saga durability in the same database means a saga step can be one transaction: reducer-event-write + saga-step-write commit together or roll back together. JSON files force a two-store atomic commit which is a known-hard problem.

### 2.2 Granularity: per-step rows vs per-saga document

**Recommendation:** per-step rows.

```sql
CREATE TABLE saga (
    saga_id        INTEGER PRIMARY KEY,    -- matches in-memory alloc
    name           TEXT NOT NULL,          -- e.g. "tear_off_tab"
    state          TEXT NOT NULL,          -- "running" | "completed" | "failed" | "compensating" | "compensated"
    started_at     INTEGER NOT NULL,       -- unix ms
    terminal_at    INTEGER,                -- unix ms; NULL while running
    failure_reason TEXT,                   -- NULL unless failed/compensated
    input_json     TEXT NOT NULL           -- caller's input, for replay/debugging
);

CREATE TABLE saga_step (
    saga_id     INTEGER NOT NULL REFERENCES saga(saga_id),
    step_index  INTEGER NOT NULL,          -- 0, 1, 2, ...
    name        TEXT NOT NULL,             -- e.g. "move_tab"
    state       TEXT NOT NULL,             -- "pending" | "succeeded" | "failed" | "compensated"
    cmd_json    TEXT NOT NULL,             -- the Command this step dispatched
    output_json TEXT,                      -- emitted events (for compensation context)
    started_at  INTEGER NOT NULL,
    ended_at    INTEGER,
    PRIMARY KEY (saga_id, step_index)
);

CREATE INDEX saga_state_idx ON saga(state) WHERE state IN ('running', 'compensating');
CREATE INDEX saga_terminal_idx ON saga(terminal_at);
```

Why per-step rather than per-saga JSON document:
- Resume logic queries `WHERE state = 'running'` cheaply.
- Each step's row is small enough that `INSERT OR REPLACE` per step is cheap.
- `--diag sagas` can render step-by-step progress without parsing a blob.
- Schema migrations apply via the existing framework.

### 2.3 Write cadence: per-step sync vs WAL group commit

**Recommendation:** per-step `INSERT` + commit, leveraging SQLite's WAL mode (already on).

The per-step write happens inside `SagaCtx::dispatch`, in the same transaction that writes the reducer-emitted events to the persist subscriber's tables. WAL mode means group-commit is automatic; we don't manually batch.

Why not async/batched: the saga set we have completes in ~10ms. An extra synchronous `INSERT` per step is negligible. The simplicity of "the durable record matches reducer state at every commit point" is worth more than the lost millisecond.

If a future saga has 100+ steps and the per-step write becomes load-bearing, *that* saga can opt into an async log. Default stays synchronous.

### 2.4 Replay vs compensate on restart

**Recommendation:** **compensate**. Do not resume.

When srv restarts and finds a `running` saga in the log:
1. Mark it `compensating`.
2. Walk its `succeeded` steps in reverse.
3. For each, dispatch the corresponding compensating command (saga-author-defined).
4. Mark it `compensated` once compensation completes.

Why not resume:
- Resume requires reproducing the exact in-memory context the saga was driving — futures, awaiting RPC calls, partial parses. That's prohibitively complex for what is, in practice, a rare event.
- Compensation is already required for the in-memory failure path. We get it "free" once we wire the durability log up to it.
- For the user, compensate-on-restart is *correct* behavior: their tear-off didn't complete, so the tab stays where it was. Resume-on-restart could partially complete a tear-off into a window that no longer exists.

The exception: long-running sagas (the motivating remote-agent example). For those, the saga author opts into resume by implementing a `try_resume` method on the saga. Default is compensate; resume is per-saga opt-in.

---

## 3. API changes

### 3.1 New: `SagaLog`

```rust
// agentmux-srv/src/sagas/log.rs

pub struct SagaLog {
    db: Arc<Mutex<Connection>>,
}

impl SagaLog {
    pub fn open(db_path: &Path) -> Result<Self, Error>;

    /// Called by the coordinator when a saga starts.
    pub fn start_saga(&self, saga_id: u64, name: &str, input: &serde_json::Value) -> Result<(), Error>;

    /// Called by SagaCtx::dispatch *before* dispatching the command.
    pub fn start_step(&self, saga_id: u64, step_index: u32, name: &str, cmd: &Command) -> Result<(), Error>;

    /// Called by SagaCtx::dispatch *after* the reducer emits non-error events.
    pub fn finish_step(&self, saga_id: u64, step_index: u32, output: &[Event]) -> Result<(), Error>;

    /// Called by SagaCtx::dispatch when the reducer emits Event::Error.
    pub fn fail_step(&self, saga_id: u64, step_index: u32, reason: &str) -> Result<(), Error>;

    /// Called by run_saga's terminal-emit path.
    pub fn terminate(&self, saga_id: u64, outcome: SagaOutcome) -> Result<(), Error>;

    /// Called at startup. Returns sagas in `running` or `compensating` state
    /// that need resolution.
    pub fn unresolved_sagas(&self) -> Result<Vec<UnresolvedSaga>, Error>;

    /// `--diag sagas` operator command consumes this.
    pub fn snapshot_recent(&self, limit: u32) -> Result<Vec<SagaSnapshot>, Error>;
}
```

### 3.2 Changes to `SagaCtx`

```rust
impl<'a> SagaCtx<'a> {
    // Add an internal step counter
    step_index: AtomicU32,

    pub async fn dispatch(&self, cmd: Command) -> Result<Vec<Event>, String> {
        let idx = self.step_index.fetch_add(1, Ordering::Relaxed);
        let step_name = command_discriminant_name(&cmd);

        self.state.saga_log.start_step(self.saga_id, idx, step_name, &cmd)
            .map_err(|e| e.to_string())?;

        let events = crate::server::service::dispatch_to_reducer(self.state, cmd).await;

        if let Some(message) = events.iter().find_map(|e| match e {
            Event::Error { message, .. } => Some(message.clone()),
            _ => None,
        }) {
            let _ = self.state.saga_log.fail_step(self.saga_id, idx, &message);
            return Err(message);
        }

        for ev in &events {
            crate::persist_subscriber::apply_event_to_wstore(ev, &self.state.wstore)
                .map_err(|e| e.to_string())?;
        }

        let _ = self.state.saga_log.finish_step(self.saga_id, idx, &events);
        crate::server::service::publish_events(self.state, &events);
        Ok(events)
    }

    // compensate() gets the same instrumentation but writes step state="compensated"
}
```

The shape of `compensate` follows the same pattern; the saga-author code in `sagas/tear_off_tab.rs` etc. requires no changes.

### 3.3 Resume-on-startup

```rust
// agentmux-srv/src/sagas/mod.rs

pub async fn resolve_unresolved_sagas_on_startup(state: &AppState) -> Result<(), Error> {
    let unresolved = state.saga_log.unresolved_sagas()?;
    for saga in unresolved {
        tracing::warn!(
            saga_id = saga.saga_id,
            name = %saga.name,
            "[saga] resolving unresolved saga from previous srv run"
        );
        compensate_unresolved(state, &saga).await?;
    }
    Ok(())
}

async fn compensate_unresolved(state: &AppState, saga: &UnresolvedSaga) -> Result<(), Error> {
    // Walk succeeded steps in reverse, dispatch compensating commands per saga
    // kind. Mark the saga "compensated" on completion or "failed" if
    // compensation itself fails.
}
```

Wired into `main.rs` at srv start, after the persist subscriber loads but before the API server begins accepting requests. Resolution is synchronous at startup — we don't accept new work until the previous run's mess is cleaned up.

### 3.4 Operator command: `--diag sagas`

```
$ agentmux --diag sagas
Recent sagas (last 50):
[2026-05-01T05:21:00Z] saga_id=42 name=tear_off_tab state=completed steps=4/4 dur=18ms
[2026-05-01T05:21:01Z] saga_id=43 name=restore_torn_off_tab state=compensated steps=2/4 dur=2.1s reason="dest window closed during saga"
[2026-05-01T05:22:14Z] saga_id=44 name=promote_block_to_tab state=running started=05:22:14Z steps=1/?  ← in flight

In-flight count: 1
Recently failed: 1
Total stored: 87
```

Mirrors `--diag srv` shape from PR #626.

---

## 4. Database migration

### 4.1 Schema

The two tables in §2.2 land via the existing migration framework. Single migration, version bump.

### 4.2 Existing-database compatibility

On first start with the new schema:
- Tables created if missing.
- No backfill — all pre-existing sagas are gone (they were in-memory anyway).
- `unresolved_sagas()` returns empty.

No user-visible migration; smooth upgrade.

---

## 5. Concurrency

The `SagaLog`'s SQLite connection is `Arc<Mutex<Connection>>`. Multiple sagas can run concurrently in srv (no global saga lock today); their `start_step` / `finish_step` calls serialize through the mutex.

Lock-hold time per call: <1ms (single `INSERT OR REPLACE`). The mutex is not load-bearing for performance.

If profiling shows the mutex becomes hot under load, the SQLite connection can be moved to per-saga (each saga gets its own connection from a pool) — defer until measurement justifies it.

---

## 6. Retention

Default: keep the last 1000 sagas, plus all sagas terminated in the last 7 days.

Cleanup: a background tokio task runs every 6 hours, executes a single `DELETE` against the saga + saga_step tables.

Retention is configurable via `~/.agentmux/config.toml` `[saga_log]` section (deferred — ship with hardcoded defaults; configuration is a follow-up if anyone asks).

---

## 7. Testing

### 7.1 Unit tests

- `SagaLog::start_saga` / `start_step` / `finish_step` / `terminate` round-trip correctly.
- `unresolved_sagas` returns only `running` + `compensating` rows.
- Retention sweep removes rows older than the threshold.

### 7.2 Integration tests

For each existing saga (`tear_off_tab`, `restore_torn_off_tab`, `tear_off_block`, `promote_block_to_tab`):

- Run the saga to completion. Assert log entries match the expected step sequence.
- Force-fail the saga at each step boundary. Assert compensation runs and log shows `compensated`.
- **Crash-recovery test:** start the saga, kill the srv process between steps N and N+1, restart srv, assert `compensate_unresolved` runs and the saga ends `compensated`. Repeat for every step boundary in every saga.

The crash-recovery harness uses `std::process::Child::kill` on a real srv subprocess — not a mock. This is the test that justifies the entire spec.

### 7.3 Property tests

Add to the existing proptest suite:
- For random sequences of `(start_saga, start_step, finish_step | fail_step, terminate)` calls, the log table reaches a consistent state — no `running` saga without a `started_at`, no `completed` saga with `pending` steps, etc.

---

## 8. Risk register

| Risk | Severity | Mitigation |
|---|---|---|
| Per-step SQLite writes become a bottleneck on long sagas | Low | Default cadence is fine for known sagas; per-saga async opt-in available if needed. |
| Resume-on-restart races with API server startup | Medium | Resolution is synchronous before the API accepts requests; documented. |
| Compensation cascade fails (compensating command itself errors) | Medium | Log marks the saga `failed_compensation`; operator must manually reconcile. Rare; alerting via `--diag sagas` recently-failed list. |
| Schema migration fails on already-deployed db | Low | Standard migration framework rollback applies. |
| Saga authors forget to implement compensation, and durability log surfaces gaps | Medium-Low | Compensation is already required by the in-memory failure path; durability changes nothing about the contract. |
| `--diag sagas` reads a saga mid-step and shows torn state | Very Low | Read uses a snapshot transaction; SQLite WAL gives a consistent point-in-time view. |

---

## 9. Sub-PR sequence (tightened)

| PR | Scope | Estimate |
|---|---|---|
| **1** | Schema migration + `SagaLog` API + `SagaCtx` instrumentation. All existing sagas remain functionally identical; the log is now populated. | ~600 LOC |
| **2** | Resume-on-startup + `--diag sagas` + crash-recovery integration tests. | ~400 LOC |

If PR 1 reveals scope creep, PR 2 splits into "resume-on-startup" and "diag + tests" sub-PRs. Default plan: 2 PRs.

---

## 10. What this spec does NOT close

- **Long-running saga authoring conventions.** When a saga genuinely waits seconds (the remote-agent example), it should yield + parking; this spec doesn't define those primitives. Defer to the first such saga.
- **Distributed sagas across srv processes.** Out of scope.
- **Resume-instead-of-compensate semantics for short sagas.** Compensate is the default; opt-in resume is a per-saga decision.
- **Launcher / host coordinator durability.** Step 6 of the architecture-completeness plan ports this design to the launcher coordinator stub.

---

## 11. Cross-references

- `next-steps-architecture-completeness-2026-05-01.md` — step 4.
- `reducer-architecture-gaps-2026-05-01.md` — gap §3 saga state durability.
- `saga-coordinator-location-analysis-2026-04-30.md` — Path A coordinator placement; this spec inherits all its assumptions.
- `agentmux-srv/src/sagas/mod.rs` — current coordinator shape (this spec extends).
- `agentmux-srv/src/persist_subscriber.rs` — existing SQLite write path; saga log piggybacks on its transaction.
- `SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md` §7 — Phase F sagas (pool-respawn, window-cleanup) that consume this durability when launcher coordinator gets the same treatment.
