// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Saga durability — durable on-disk log of saga lifecycle.
//
// See `docs/specs/SPEC_SAGA_DURABILITY_2026-05-01.md` for the full
// design. This module ships PR 1 of the spec: schema + log API +
// `SagaCtx` instrumentation. Resume-on-startup, `--diag sagas`, and
// crash-recovery integration tests follow in PR 2.
//
// Why an isolated SQLite file (`sagas.db`) rather than co-locating
// inside `objects.db`: we want saga writes to commit independently
// of the Store migration / connection. A separate connection
// also keeps the saga log's mutex contention isolated from the
// reducer's persistence path. (The two-store atomicity concern from
// spec §2.1 is not load-bearing for PR 1; saga steps are written
// after the reducer-emitted event has already been applied to
// wstore by `apply_event_to_wstore`. Compensate-on-restart in PR 2
// will reconcile any divergence by walking succeeded steps in
// reverse.)
//
// Concurrency: a single `Mutex<Connection>` serializes writes. Each
// `start_step` / `finish_step` call holds the lock for <1ms. If
// profiling shows the mutex becomes hot under load, switch to a
// connection pool — defer until measurement justifies it (spec §5).

use std::path::Path;
use std::sync::Mutex;

use agentmux_common::ipc::{Command, Event};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::backend::storage::error::StoreError;
use crate::backend::storage::migrations::{
    check_schema_compat, run_saga_log_migrations, stamp_version, SAGA_LOG_SCHEMA_VERSION,
};

/// Outcome of a saga, written by `terminate`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SagaOutcome {
    /// Saga ran to completion successfully.
    Completed,
    /// Saga failed without compensation completing (or before
    /// compensation began). `reason` is operator-readable.
    Failed { reason: String },
    /// Saga failed and compensation completed cleanly. `reason` is
    /// the original failure that triggered compensation.
    Compensated { reason: String },
}

impl SagaOutcome {
    fn state_str(&self) -> &'static str {
        match self {
            SagaOutcome::Completed => "completed",
            SagaOutcome::Failed { .. } => "failed",
            SagaOutcome::Compensated { .. } => "compensated",
        }
    }

    fn reason(&self) -> Option<&str> {
        match self {
            SagaOutcome::Completed => None,
            SagaOutcome::Failed { reason } | SagaOutcome::Compensated { reason } => {
                Some(reason.as_str())
            }
        }
    }
}

/// A saga in `running` or `compensating` state at startup. Returned
/// by `unresolved_sagas`; consumed by PR 2's `compensate_unresolved`
/// to walk succeeded steps in reverse and dispatch compensation.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)] // Fields consumed by PR 2's compensate_unresolved.
pub struct UnresolvedSaga {
    pub saga_id: u64,
    pub name: String,
    pub state: String,
    pub started_at: i64,
    pub input_json: String,
    pub steps: Vec<UnresolvedStep>,
}

/// A step row attached to an `UnresolvedSaga`. Steps are returned in
/// `step_index` ascending order; PR 2's reverse-walker iterates over
/// the `succeeded` entries.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)] // Fields consumed by PR 2's compensate_unresolved.
pub struct UnresolvedStep {
    pub step_index: u32,
    pub name: String,
    pub state: String,
    pub cmd_json: String,
    pub output_json: Option<String>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
}

/// Operator-facing snapshot of a recent saga, for `--diag sagas`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)] // Fields consumed by PR 2's `--diag sagas`.
pub struct SagaSnapshot {
    pub saga_id: u64,
    pub name: String,
    pub state: String,
    pub started_at: i64,
    pub terminal_at: Option<i64>,
    pub failure_reason: Option<String>,
    pub step_count: u32,
    /// JSON serialization of the saga's input args, captured by the
    /// caller in `emit_saga_started` (reagent P1 PR #631 — was being
    /// stubbed `null`). Operator-readable provenance for `--diag sagas`.
    pub input_json: String,
}

/// SQLite-backed saga log. Owned by `AppState` as `Arc<SagaLog>`;
/// every `SagaCtx::dispatch` and `compensate` call writes through
/// it. See module-level docs for design notes.
pub struct SagaLog {
    conn: Mutex<Connection>,
}

impl SagaLog {
    /// Open a saga log backed by the given SQLite file. Configures
    /// WAL mode + 5s busy timeout (mirroring `Store::open`) and
    /// applies the schema migration.
    pub fn open(path: &Path) -> Result<Self, StoreError> {
        let conn = Connection::open(path)?;
        Self::configure_and_migrate(conn)
    }

    /// Open an in-memory saga log for testing.
    #[allow(dead_code)]
    pub fn open_in_memory() -> Result<Self, StoreError> {
        let conn = Connection::open_in_memory()?;
        Self::configure_and_migrate(conn)
    }

    fn configure_and_migrate(conn: Connection) -> Result<Self, StoreError> {
        // `foreign_keys=ON` enforces the `saga_step.saga_id REFERENCES
        // saga(saga_id)` declaration; SQLite defaults this to OFF
        // (codex P2 PR #631). Without it, orphan step rows can be
        // written silently — corrupts diagnostics + PR 2's resume
        // logic which reconstructs state from saga + saga_step joins.
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA busy_timeout=5000;
             PRAGMA synchronous=NORMAL;
             PRAGMA temp_store=MEMORY;
             PRAGMA foreign_keys=ON;",
        )?;
        // Safety lock BEFORE migrations — same discipline as wstore /
        // filestore: refuse to touch a newer-schema DB on disk before
        // any mutating step runs. See `check_schema_compat` doc.
        check_schema_compat(&conn, SAGA_LOG_SCHEMA_VERSION, "sagas.db")?;
        run_saga_log_migrations(&conn)?;
        stamp_version(&conn, SAGA_LOG_SCHEMA_VERSION)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Highest existing `saga_id` in the durable log, or 0 if the log
    /// is empty. Used at startup to seed `state.saga_id_alloc` so new
    /// sagas don't reuse IDs from prior srv-process runs (reagent P1
    /// PR #631 — `INSERT` would fail on collision but the saga itself
    /// would have already started, so we seed defensively).
    pub fn max_saga_id(&self) -> Result<u64, StoreError> {
        let conn = self.conn.lock().unwrap();
        // Propagate query errors. (codex P2 PR #631 round 2.) The
        // earlier `.ok().flatten()` swallowed SQLite errors and
        // returned 0 as if the log were empty — a transient read
        // failure would then reseed `saga_id_alloc=0` on the next
        // restart, sagas would reuse IDs 1, 2, 3..., and `start_saga`
        // would reject them (no OR REPLACE), leaving live sagas with
        // no durable lifecycle row. Better to surface the error and
        // let the startup hook log the explicit warning + accept the
        // collision risk knowingly.
        let max: Option<i64> = conn.query_row("SELECT MAX(saga_id) FROM saga", [], |r| r.get(0))?;
        Ok(max.unwrap_or(0).max(0) as u64)
    }

    /// Insert a fresh saga row in `running` state. Called by the
    /// coordinator immediately after `alloc_saga_id` (and before any
    /// per-step writes).
    pub fn start_saga(
        &self,
        saga_id: u64,
        name: &str,
        input: &serde_json::Value,
    ) -> Result<(), StoreError> {
        let now = now_ms();
        let input_json = serde_json::to_string(input)?;
        let conn = self.conn.lock().unwrap();
        // Plain INSERT (not OR REPLACE) so saga_id collisions surface
        // as errors instead of silently overwriting prior runs'
        // history. The allocator is seeded from MAX(saga_id) at
        // startup (see `max_saga_id` + `main.rs`), so collisions
        // shouldn't happen in practice — if they do, that's a bug
        // worth surfacing. (codex P1 + reagent P1 PR #631.)
        conn.execute(
            "INSERT INTO saga
             (saga_id, name, state, started_at, terminal_at, failure_reason, input_json)
             VALUES (?1, ?2, 'running', ?3, NULL, NULL, ?4)",
            params![saga_id as i64, name, now, input_json],
        )?;
        Ok(())
    }

    /// Insert a `pending` step row before dispatching the command.
    /// `name` is a short discriminant string (e.g. "MoveTab"); `cmd`
    /// is serialized as JSON for replay/debugging.
    pub fn start_step(
        &self,
        saga_id: u64,
        step_index: u32,
        name: &str,
        cmd: &Command,
    ) -> Result<(), StoreError> {
        let now = now_ms();
        let cmd_json = serde_json::to_string(cmd)?;
        let conn = self.conn.lock().unwrap();
        // Plain INSERT — same rationale as `start_saga`.
        conn.execute(
            "INSERT INTO saga_step
             (saga_id, step_index, name, state, cmd_json, output_json, started_at, ended_at)
             VALUES (?1, ?2, ?3, 'pending', ?4, NULL, ?5, NULL)",
            params![saga_id as i64, step_index, name, cmd_json, now],
        )?;
        Ok(())
    }

    /// Mark a step `succeeded` and store its emitted events as
    /// JSON, used by compensation to reconstruct context if needed.
    pub fn finish_step(
        &self,
        saga_id: u64,
        step_index: u32,
        output_events: &[Event],
    ) -> Result<(), StoreError> {
        let now = now_ms();
        let output_json = serde_json::to_string(output_events)?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE saga_step
             SET state = 'succeeded', output_json = ?1, ended_at = ?2
             WHERE saga_id = ?3 AND step_index = ?4",
            params![output_json, now, saga_id as i64, step_index],
        )?;
        Ok(())
    }

    /// Mark a step `failed`. Stores the reducer's error message in
    /// `output_json` as `{"error": ...}` so PR 2's `--diag sagas`
    /// can surface it without a separate column.
    pub fn fail_step(
        &self,
        saga_id: u64,
        step_index: u32,
        reason: &str,
    ) -> Result<(), StoreError> {
        let now = now_ms();
        let output_json = serde_json::to_string(&serde_json::json!({ "error": reason }))
            ?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE saga_step
             SET state = 'failed', output_json = ?1, ended_at = ?2
             WHERE saga_id = ?3 AND step_index = ?4",
            params![output_json, now, saga_id as i64, step_index],
        )?;
        Ok(())
    }

    /// Mark a compensation step `compensated`. Same shape as
    /// `finish_step` but distinct state. Called from
    /// `SagaCtx::compensate` after the reducer applies the
    /// compensating command successfully.
    pub fn compensate_step(
        &self,
        saga_id: u64,
        step_index: u32,
        output_events: &[Event],
    ) -> Result<(), StoreError> {
        let now = now_ms();
        let output_json = serde_json::to_string(output_events)?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO saga_step
             (saga_id, step_index, name, state, cmd_json, output_json, started_at, ended_at)
             VALUES (
                 ?1, ?2,
                 COALESCE((SELECT name FROM saga_step WHERE saga_id=?1 AND step_index=?2), 'compensate'),
                 'compensated',
                 COALESCE((SELECT cmd_json FROM saga_step WHERE saga_id=?1 AND step_index=?2), ''),
                 ?3,
                 COALESCE((SELECT started_at FROM saga_step WHERE saga_id=?1 AND step_index=?2), ?4),
                 ?4
             )",
            params![saga_id as i64, step_index, output_json, now],
        )?;
        Ok(())
    }

    /// Highest existing `step_index` for a saga, +1, or 0 if no steps
    /// exist yet. Used by PR 2's resume-on-startup so recovery
    /// compensation rows are appended AFTER the saga's original step
    /// indices (rather than overwriting in place via `compensate_step`,
    /// which would lose the original step's `output_json` provenance).
    pub fn next_step_index(&self, saga_id: u64) -> Result<u32, StoreError> {
        let conn = self.conn.lock().unwrap();
        let max: Option<i64> = conn.query_row(
            "SELECT MAX(step_index) FROM saga_step WHERE saga_id = ?1",
            params![saga_id as i64],
            |r| r.get(0),
        )?;
        Ok((max.unwrap_or(-1) + 1) as u32)
    }

    /// Append a new compensation step row at `step_index`. Distinct
    /// from `compensate_step()` (which overwrites an existing step row
    /// by index): used by PR 2's resume-on-startup to record a
    /// recovery dispatch as a NEW row so the original succeeded
    /// step's provenance is preserved.
    ///
    /// `state='compensated'` if `error.is_none()`, else `'failed'`
    /// (with the reason in `output_json`).
    pub fn append_recovery_step(
        &self,
        saga_id: u64,
        step_index: u32,
        name: &str,
        cmd: &Command,
        events: &[Event],
        error: Option<&str>,
    ) -> Result<(), StoreError> {
        let now = now_ms();
        let cmd_json = serde_json::to_string(cmd)?;
        let (state, output_json) = match error {
            None => ("compensated", serde_json::to_string(events)?),
            Some(err) => (
                "failed",
                serde_json::to_string(&serde_json::json!({ "error": err }))?,
            ),
        };
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO saga_step
             (saga_id, step_index, name, state, cmd_json, output_json, started_at, ended_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?7)",
            params![saga_id as i64, step_index, name, state, cmd_json, output_json, now],
        )?;
        Ok(())
    }

    /// Mark a saga as `compensating` — used by PR 2's resume-on-startup
    /// to flag a saga as undergoing recovery before walking its
    /// succeeded steps in reverse. After all compensating dispatches
    /// run, the caller invokes `terminate()` with the appropriate
    /// outcome (`Compensated` / `Failed`).
    ///
    /// (codex follow-up.) Distinct from `terminate(state='compensated')`
    /// because the saga is only **mid-compensation** here — its
    /// terminal_at and failure_reason are not known yet.
    pub fn mark_compensating(&self, saga_id: u64) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE saga SET state = 'compensating' WHERE saga_id = ?1",
            params![saga_id as i64],
        )?;
        Ok(())
    }

    /// Mark ALL of a saga's remaining `succeeded` forward steps as
    /// `compensated` in one shot. (codex P1 PR #636 round 5.) Used
    /// at `emit_terminal` time when the outcome is `Compensated` —
    /// the saga author is asserting "I've unwound everything," so
    /// any forward step still in `succeeded` is by definition done.
    /// Catches the case where one compensating command undoes
    /// multiple forward steps (e.g. `tear_off_block`'s single
    /// `DeleteWorkspace` undoes both `CreateWorkspace` and
    /// `CreateTab`) — `SagaCtx::compensate`'s per-call stack pop
    /// only marks one, leaving the other as still-`succeeded` and
    /// triggering double-compensation on restart.
    pub fn mark_all_succeeded_steps_compensated(
        &self,
        saga_id: u64,
    ) -> Result<(), StoreError> {
        let now = now_ms();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE saga_step
             SET state = 'compensated', ended_at = ?1
             WHERE saga_id = ?2 AND state = 'succeeded'",
            params![now, saga_id as i64],
        )?;
        Ok(())
    }

    /// Mark an *original* forward step as `compensated`. (codex P1
    /// PR #636 round 2.) Without this, a second crash-recovery
    /// startup re-reads the still-`succeeded` original steps and
    /// replays their inverses again — flipping a previously-recovered
    /// saga into `failed_compensation` (or worse, applying duplicate
    /// side effects like a second delete). The recovery rows
    /// (`append_recovery_step`) live at fresh indices and don't
    /// affect the original step's state on their own; this update
    /// is the explicit "this forward step has been compensated"
    /// marker that the next recovery scan needs to see.
    pub fn mark_step_compensated(
        &self,
        saga_id: u64,
        step_index: u32,
    ) -> Result<(), StoreError> {
        let now = now_ms();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE saga_step
             SET state = 'compensated', ended_at = ?1
             WHERE saga_id = ?2 AND step_index = ?3 AND state = 'succeeded'",
            params![now, saga_id as i64, step_index],
        )?;
        Ok(())
    }

    /// Mark a saga as `failed_compensation` — used by PR 2's resume-
    /// on-startup when at least one compensating dispatch errored. The
    /// saga is left in this terminal state for operator review;
    /// `--diag sagas` surfaces it.
    pub fn mark_failed_compensation(
        &self,
        saga_id: u64,
        reason: &str,
    ) -> Result<(), StoreError> {
        let now = now_ms();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE saga
             SET state = 'failed_compensation', terminal_at = ?1, failure_reason = ?2
             WHERE saga_id = ?3",
            params![now, reason, saga_id as i64],
        )?;
        Ok(())
    }

    /// Write the saga's terminal lifecycle row. Called from
    /// `emit_terminal` after the inner future returns.
    pub fn terminate(&self, saga_id: u64, outcome: SagaOutcome) -> Result<(), StoreError> {
        let now = now_ms();
        let state = outcome.state_str();
        let reason = outcome.reason();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE saga
             SET state = ?1, terminal_at = ?2, failure_reason = ?3
             WHERE saga_id = ?4",
            params![state, now, reason, saga_id as i64],
        )?;
        Ok(())
    }

    /// Return all sagas still in `running`, `compensating`, or
    /// `failed` state, each with its full step list (for PR 2's
    /// compensate-on-restart reverse walk). Used at startup before
    /// the API server begins accepting requests.
    ///
    /// (codex P1 PR #631 round 2.) `failed` is included because
    /// `classify_run_saga_result` records timeout/cancel paths as
    /// `failed` — those are exactly the partial-apply cases this
    /// durability layer exists to recover. A `failed` saga's step
    /// list may have `succeeded` rows whose effects need
    /// compensation; if we filtered them out here, recovery would
    /// silently leave that state in place.
    #[allow(dead_code)] // Used by PR 2's resume-on-startup; tests exercise it now.
    pub fn unresolved_sagas(&self) -> Result<Vec<UnresolvedSaga>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT saga_id, name, state, started_at, input_json
             FROM saga
             WHERE state IN ('running', 'compensating', 'failed')
             ORDER BY saga_id ASC",
        )?;
        let saga_rows: Vec<(i64, String, String, i64, String)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, String>(4)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(stmt);

        let mut out = Vec::with_capacity(saga_rows.len());
        for (saga_id, name, state, started_at, input_json) in saga_rows {
            let mut step_stmt = conn.prepare(
                "SELECT step_index, name, state, cmd_json, output_json, started_at, ended_at
                 FROM saga_step
                 WHERE saga_id = ?1
                 ORDER BY step_index ASC",
            )?;
            let steps: Vec<UnresolvedStep> = step_stmt
                .query_map(params![saga_id], |row| {
                    Ok(UnresolvedStep {
                        step_index: row.get::<_, i64>(0)? as u32,
                        name: row.get::<_, String>(1)?,
                        state: row.get::<_, String>(2)?,
                        cmd_json: row.get::<_, String>(3)?,
                        output_json: row.get::<_, Option<String>>(4)?,
                        started_at: row.get::<_, i64>(5)?,
                        ended_at: row.get::<_, Option<i64>>(6)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            out.push(UnresolvedSaga {
                saga_id: saga_id as u64,
                name,
                state,
                started_at,
                input_json,
                steps,
            });
        }
        Ok(out)
    }

    /// Return up to `limit` recent sagas for `--diag sagas`. Sorted
    /// most-recent-first. `step_count` is the count of `succeeded`
    /// or `compensated` steps (i.e. progress through the saga).
    #[allow(dead_code)] // Used by PR 2's `--diag sagas`; tests exercise it now.
    pub fn snapshot_recent(&self, limit: u32) -> Result<Vec<SagaSnapshot>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT saga_id, name, state, started_at, terminal_at, failure_reason, input_json
             FROM saga
             ORDER BY COALESCE(terminal_at, started_at) DESC
             LIMIT ?1",
        )?;
        let rows: Vec<(i64, String, String, i64, Option<i64>, Option<String>, String)> = stmt
            .query_map(params![limit], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(stmt);

        let mut out = Vec::with_capacity(rows.len());
        for (saga_id, name, state, started_at, terminal_at, failure_reason, input_json) in rows {
            let count: Option<i64> = conn
                .query_row(
                    "SELECT COUNT(*) FROM saga_step
                     WHERE saga_id = ?1 AND state IN ('succeeded', 'compensated')",
                    params![saga_id],
                    |row| row.get(0),
                )
                .optional()?;
            out.push(SagaSnapshot {
                saga_id: saga_id as u64,
                name,
                state,
                started_at,
                terminal_at,
                failure_reason,
                step_count: count.unwrap_or(0) as u32,
                input_json,
            });
        }
        Ok(out)
    }
}

fn now_ms() -> i64 {
    agentmux_common::time::now_ms()
}

/// Discriminant name for a `Command`. Uses the serde tag (the
/// `cmd` field of the snake_case-tagged enum) so the saga log row
/// is easy to read in `--diag sagas`.
pub(crate) fn command_discriminant_name(cmd: &Command) -> String {
    match serde_json::to_value(cmd) {
        Ok(serde_json::Value::Object(map)) => map
            .get("cmd")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| "unknown".to_string()),
        _ => "unknown".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agentmux_common::ipc::{ClientKind, ErrorCode};
    use tempfile::NamedTempFile;

    fn temp_log() -> (NamedTempFile, SagaLog) {
        let f = NamedTempFile::new().expect("tempfile");
        let log = SagaLog::open(f.path()).expect("open");
        (f, log)
    }

    fn ping(nonce: u64) -> Command {
        Command::Ping { nonce }
    }

    fn pong(nonce: u64) -> Event {
        Event::Pong { nonce, version: 0 }
    }

    #[test]
    fn schema_migration_clean_db() {
        let f = NamedTempFile::new().unwrap();
        // First open creates schema.
        let _log = SagaLog::open(f.path()).unwrap();
        // Second open is idempotent (CREATE TABLE IF NOT EXISTS).
        let _log = SagaLog::open(f.path()).unwrap();
    }

    #[test]
    fn round_trip_completed() {
        let (_f, log) = temp_log();
        log.start_saga(1, "tear_off_tab", &serde_json::json!({"tab_id": "abc"}))
            .unwrap();
        log.start_step(1, 0, "Ping", &ping(7)).unwrap();
        log.finish_step(1, 0, &[pong(7)]).unwrap();
        log.start_step(1, 1, "Ping", &ping(8)).unwrap();
        log.finish_step(1, 1, &[pong(8)]).unwrap();
        log.terminate(1, SagaOutcome::Completed).unwrap();

        let snap = log.snapshot_recent(10).unwrap();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].saga_id, 1);
        assert_eq!(snap[0].name, "tear_off_tab");
        assert_eq!(snap[0].state, "completed");
        assert!(snap[0].terminal_at.is_some());
        assert!(snap[0].failure_reason.is_none());
        assert_eq!(snap[0].step_count, 2);
    }

    #[test]
    fn round_trip_failed_with_compensation() {
        let (_f, log) = temp_log();
        log.start_saga(2, "tear_off_block", &serde_json::json!({}))
            .unwrap();
        log.start_step(2, 0, "Ping", &ping(1)).unwrap();
        log.finish_step(2, 0, &[pong(1)]).unwrap();
        log.start_step(2, 1, "Ping", &ping(2)).unwrap();
        log.fail_step(2, 1, "boom").unwrap();
        // Compensation: walk the one succeeded step in reverse.
        log.compensate_step(2, 0, &[pong(99)]).unwrap();
        log.terminate(
            2,
            SagaOutcome::Compensated {
                reason: "boom".to_string(),
            },
        )
        .unwrap();

        let snap = log.snapshot_recent(10).unwrap();
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].state, "compensated");
        assert_eq!(snap[0].failure_reason.as_deref(), Some("boom"));
    }

    #[test]
    fn unresolved_sagas_returns_only_in_flight() {
        let (_f, log) = temp_log();

        // Saga 1: completed.
        log.start_saga(1, "saga_a", &serde_json::json!({})).unwrap();
        log.terminate(1, SagaOutcome::Completed).unwrap();

        // Saga 2: still running.
        log.start_saga(2, "saga_b", &serde_json::json!({"x": 1}))
            .unwrap();
        log.start_step(2, 0, "Ping", &ping(0)).unwrap();
        log.finish_step(2, 0, &[pong(0)]).unwrap();

        // Saga 3: failed (terminal). NOW INCLUDED in unresolved
        // (codex P1 round 2): classify_run_saga_result records
        // timeout/cancel paths as `failed`, and those are exactly
        // the partial-apply cases recovery needs to handle.
        log.start_saga(3, "saga_c", &serde_json::json!({})).unwrap();
        log.terminate(
            3,
            SagaOutcome::Failed {
                reason: "oops".to_string(),
            },
        )
        .unwrap();

        // Saga 4: compensating (terminal not reached).
        log.start_saga(4, "saga_d", &serde_json::json!({})).unwrap();
        // Manually flip to compensating without going through
        // terminate (mirrors what PR 2's compensate path will do).
        log.conn
            .lock()
            .unwrap()
            .execute(
                "UPDATE saga SET state = 'compensating' WHERE saga_id = 4",
                [],
            )
            .unwrap();

        // Saga 5: compensated (terminal — recovery already ran).
        log.start_saga(5, "saga_e", &serde_json::json!({})).unwrap();
        log.terminate(
            5,
            SagaOutcome::Compensated {
                reason: "rolled back".to_string(),
            },
        )
        .unwrap();

        let unresolved = log.unresolved_sagas().unwrap();
        let mut ids: Vec<u64> = unresolved.iter().map(|u| u.saga_id).collect();
        ids.sort();
        // 2 (running), 3 (failed), 4 (compensating). Excludes 1
        // (completed) and 5 (compensated).
        assert_eq!(ids, vec![2, 3, 4]);

        // Saga 2 carries its succeeded step.
        let saga2 = unresolved.iter().find(|u| u.saga_id == 2).unwrap();
        assert_eq!(saga2.state, "running");
        assert_eq!(saga2.steps.len(), 1);
        assert_eq!(saga2.steps[0].state, "succeeded");
        assert_eq!(saga2.steps[0].name, "Ping");

        // Saga 3 is failed but eligible for recovery.
        let saga3 = unresolved.iter().find(|u| u.saga_id == 3).unwrap();
        assert_eq!(saga3.state, "failed");

        // Saga 4 has no steps yet (started but never dispatched).
        let saga4 = unresolved.iter().find(|u| u.saga_id == 4).unwrap();
        assert_eq!(saga4.state, "compensating");
        assert!(saga4.steps.is_empty());
    }

    #[test]
    fn fail_step_records_reason_in_output_json() {
        let (_f, log) = temp_log();
        log.start_saga(5, "saga_fail", &serde_json::json!({})).unwrap();
        log.start_step(5, 0, "Ping", &ping(0)).unwrap();
        log.fail_step(5, 0, "reducer rejected").unwrap();

        // Inspect via unresolved (saga is still 'running' since
        // terminate wasn't called).
        let unresolved = log.unresolved_sagas().unwrap();
        assert_eq!(unresolved.len(), 1);
        let step = &unresolved[0].steps[0];
        assert_eq!(step.state, "failed");
        let parsed: serde_json::Value =
            serde_json::from_str(step.output_json.as_ref().unwrap()).unwrap();
        assert_eq!(parsed["error"], "reducer rejected");
    }

    #[test]
    fn snapshot_recent_orders_most_recent_first_and_respects_limit() {
        let (_f, log) = temp_log();
        for i in 1..=5 {
            log.start_saga(i, &format!("saga_{i}"), &serde_json::json!({}))
                .unwrap();
            log.terminate(i, SagaOutcome::Completed).unwrap();
            // Tiny pause so terminal_at differs (millisecond resolution
            // on most platforms is enough but not guaranteed in
            // tight loops).
            std::thread::sleep(std::time::Duration::from_millis(2));
        }

        let snap = log.snapshot_recent(3).unwrap();
        assert_eq!(snap.len(), 3);
        // Most recent first → ids 5, 4, 3.
        assert_eq!(snap[0].saga_id, 5);
        assert_eq!(snap[1].saga_id, 4);
        assert_eq!(snap[2].saga_id, 3);
    }

    #[test]
    fn command_discriminant_name_uses_serde_tag() {
        assert_eq!(command_discriminant_name(&ping(0)), "ping");
        assert_eq!(
            command_discriminant_name(&Command::Goodbye),
            "goodbye"
        );
        assert_eq!(
            command_discriminant_name(&Command::Register {
                kind: ClientKind::Tool,
                pid: 1,
                version: "v".to_string()
            }),
            "register"
        );
    }

    // Touch the imported types so test compilation surfaces any
    // future enum-shape drift early; these symbols are otherwise
    // referenced only by saga authors.
    #[test]
    fn imports_are_live() {
        let _ = ErrorCode::InvalidCommand;
    }

    // codex P1 PR #631 — start_saga must NOT silently overwrite an
    // existing saga_id row. With the seed-from-MAX(saga_id) startup
    // logic, collisions should never happen in practice; if one ever
    // does, it's a bug worth surfacing.
    #[test]
    fn start_saga_rejects_duplicate_id() {
        let (_tmp, log) = temp_log();
        log.start_saga(42, "tear_off_tab", &serde_json::json!({"a": 1}))
            .unwrap();
        // A second start_saga with the same id must fail (no OR REPLACE).
        let err = log
            .start_saga(42, "tear_off_tab", &serde_json::json!({"a": 2}))
            .unwrap_err();
        // Surfaces as a SQLite UNIQUE constraint violation.
        let msg = format!("{}", err);
        assert!(
            msg.to_lowercase().contains("unique") || msg.to_lowercase().contains("constraint"),
            "expected unique-constraint error, got: {msg}",
        );
    }

    // reagent P1 + codex P1 PR #631 — max_saga_id seeds the
    // allocator at startup; without this, restarts would reuse
    // saga_ids 1, 2, 3... and silently overwrite prior runs.
    #[test]
    fn max_saga_id_returns_highest_persisted() {
        let (_tmp, log) = temp_log();
        assert_eq!(log.max_saga_id().unwrap(), 0); // empty
        log.start_saga(7, "a", &serde_json::Value::Null).unwrap();
        log.start_saga(42, "b", &serde_json::Value::Null).unwrap();
        log.start_saga(13, "c", &serde_json::Value::Null).unwrap();
        assert_eq!(log.max_saga_id().unwrap(), 42);
    }

    // codex P2 PR #631 — foreign keys enabled means a saga_step
    // referencing a non-existent saga_id is rejected.
    #[test]
    fn foreign_keys_enforced_on_saga_step() {
        let (_tmp, log) = temp_log();
        // Try to insert a step for a saga that was never started.
        let err = log
            .start_step(999, 0, "MoveTab", &ping(1))
            .unwrap_err();
        let msg = format!("{}", err);
        assert!(
            msg.to_lowercase().contains("foreign key")
                || msg.to_lowercase().contains("constraint"),
            "expected foreign-key error, got: {msg}",
        );
    }

    // reagent P1 PR #631 — input_json must round-trip the saga's
    // actual input args (was being stubbed with Value::Null).
    #[test]
    fn start_saga_persists_input_json() {
        let (_tmp, log) = temp_log();
        let input = serde_json::json!({
            "tab_id": "tab-abc",
            "source_workspace_id": "ws-1",
        });
        log.start_saga(1, "tear_off_tab", &input).unwrap();
        let snapshot = log.snapshot_recent(1).unwrap();
        assert_eq!(snapshot.len(), 1);
        let parsed: serde_json::Value =
            serde_json::from_str(&snapshot[0].input_json).unwrap();
        assert_eq!(parsed, input);
    }

    // Step 7 — E.7 recovery-from-crash scaffold.
    //
    // Asserts the API surface PR 2's `compensate_unresolved` will
    // need to walk back partially-applied sagas. The full
    // compensate-on-restart logic ships in PR 2; this test pins the
    // shape of `unresolved_sagas()` so PR 2 can wire against a
    // stable contract.
    //
    // Scenario: a saga went through TWO succeeded forward steps,
    // then the srv process crashed mid-third-step (no terminal row).
    // After restart, `unresolved_sagas()` must:
    //   1. Return the saga in `running` state.
    //   2. Surface its succeeded steps in `step_index ASC` order so
    //      PR 2's reverse walker can iterate `.iter().rev()` and
    //      dispatch compensation in LIFO order (most-recently-applied
    //      first, matching standard saga compensation semantics).
    //   3. Preserve `cmd_json` per step so PR 2 can reconstruct the
    //      compensating command shape (e.g., `MoveTab src↔dst` swap).
    //   4. Carry the saga's `input_json` for `--diag sagas` triage.
    #[test]
    fn unresolved_saga_exposes_succeeded_steps_in_index_order_for_reverse_walk() {
        let (_f, log) = temp_log();

        // A saga that got through two forward steps (CreateWorkspace
        // + MoveTab from a tear_off_tab run), then "crashed" before
        // terminate(). No fail/compensate writes — the lifecycle row
        // stays in `running`, the steps stay `succeeded`.
        let input = serde_json::json!({
            "tab_id": "tab-abc",
            "source_workspace_id": "ws-1",
        });
        log.start_saga(101, "tear_off_tab", &input).unwrap();

        let create_cmd = Command::Ping { nonce: 1 }; // stand-in: CreateWorkspace
        let move_cmd = Command::Ping { nonce: 2 }; // stand-in: MoveTab
        log.start_step(101, 0, "CreateWorkspace", &create_cmd)
            .unwrap();
        log.finish_step(101, 0, &[pong(1)]).unwrap();
        log.start_step(101, 1, "MoveTab", &move_cmd).unwrap();
        log.finish_step(101, 1, &[pong(2)]).unwrap();

        // Imagine srv crashed here — no terminate(). On restart we
        // open a fresh SagaLog over the same DB; saga 101 should
        // appear in unresolved_sagas with both succeeded steps.
        let unresolved = log.unresolved_sagas().unwrap();
        assert_eq!(unresolved.len(), 1);
        let saga = &unresolved[0];

        // (1) Saga is in `running` state — PR 2 picks these up first.
        assert_eq!(saga.saga_id, 101);
        assert_eq!(saga.name, "tear_off_tab");
        assert_eq!(saga.state, "running");

        // (2) Steps are returned in ascending step_index order — PR 2's
        // reverse-walker iterates `.iter().rev()` to compensate LIFO.
        assert_eq!(saga.steps.len(), 2);
        assert_eq!(saga.steps[0].step_index, 0);
        assert_eq!(saga.steps[0].name, "CreateWorkspace");
        assert_eq!(saga.steps[0].state, "succeeded");
        assert_eq!(saga.steps[1].step_index, 1);
        assert_eq!(saga.steps[1].name, "MoveTab");
        assert_eq!(saga.steps[1].state, "succeeded");

        // (3) cmd_json round-trips so PR 2 can re-deserialize and
        // construct the compensating command shape.
        let parsed_create: Command =
            serde_json::from_str(&saga.steps[0].cmd_json).unwrap();
        let parsed_move: Command = serde_json::from_str(&saga.steps[1].cmd_json).unwrap();
        assert!(matches!(parsed_create, Command::Ping { nonce: 1 }));
        assert!(matches!(parsed_move, Command::Ping { nonce: 2 }));

        // (4) input_json survives the restart for operator triage.
        let parsed_input: serde_json::Value =
            serde_json::from_str(&saga.input_json).unwrap();
        assert_eq!(parsed_input["tab_id"], "tab-abc");
        assert_eq!(parsed_input["source_workspace_id"], "ws-1");

        // Sanity check: walking succeeded-only in reverse (the actual
        // PR 2 algorithm) yields step indices [1, 0] — most-recently-
        // applied first.
        let reverse_succeeded: Vec<u32> = saga
            .steps
            .iter()
            .rev()
            .filter(|s| s.state == "succeeded")
            .map(|s| s.step_index)
            .collect();
        assert_eq!(reverse_succeeded, vec![1, 0]);
    }

    // Step 7 — E.7 recovery scaffold continued.
    //
    // Failed-mid-step is a different recovery shape: the lifecycle
    // is still `running`, but one step is `failed` and earlier steps
    // are `succeeded`. PR 2's reverse-walker must skip the failed
    // step (its effects didn't apply by definition) and compensate
    // the prior succeeded ones.
    #[test]
    fn unresolved_saga_with_mid_step_failure_exposes_succeeded_prefix() {
        let (_f, log) = temp_log();
        log.start_saga(202, "tear_off_block", &serde_json::json!({})).unwrap();
        log.start_step(202, 0, "Ping", &ping(1)).unwrap();
        log.finish_step(202, 0, &[pong(1)]).unwrap();
        log.start_step(202, 1, "Ping", &ping(2)).unwrap();
        log.fail_step(202, 1, "reducer rejected").unwrap();
        // Imagine crash before compensation could finish — saga
        // lifecycle row stays in `running`.

        let unresolved = log.unresolved_sagas().unwrap();
        let saga = unresolved.iter().find(|s| s.saga_id == 202).unwrap();
        assert_eq!(saga.state, "running");
        // Both step rows present, in index order, with the failure
        // distinguishable so PR 2's recovery skips it.
        assert_eq!(saga.steps.len(), 2);
        assert_eq!(saga.steps[0].state, "succeeded");
        assert_eq!(saga.steps[1].state, "failed");
        // PR 2 will iterate succeeded prefix for compensation —
        // demonstrate the iteration shape.
        let to_compensate: Vec<u32> = saga
            .steps
            .iter()
            .rev()
            .filter(|s| s.state == "succeeded")
            .map(|s| s.step_index)
            .collect();
        assert_eq!(to_compensate, vec![0]);
    }
}
