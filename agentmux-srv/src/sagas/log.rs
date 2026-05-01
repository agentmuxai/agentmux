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
// of the WaveStore migration / connection. A separate connection
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
use crate::backend::storage::migrations::run_saga_log_migrations;

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
}

/// SQLite-backed saga log. Owned by `AppState` as `Arc<SagaLog>`;
/// every `SagaCtx::dispatch` and `compensate` call writes through
/// it. See module-level docs for design notes.
pub struct SagaLog {
    conn: Mutex<Connection>,
}

impl SagaLog {
    /// Open a saga log backed by the given SQLite file. Configures
    /// WAL mode + 5s busy timeout (mirroring `WaveStore::open`) and
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
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA busy_timeout=5000;
             PRAGMA synchronous=NORMAL;
             PRAGMA temp_store=MEMORY;",
        )?;
        run_saga_log_migrations(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
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
        conn.execute(
            "INSERT OR REPLACE INTO saga
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
        conn.execute(
            "INSERT OR REPLACE INTO saga_step
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

    /// Return all sagas still in `running` or `compensating` state,
    /// each with its full step list (for PR 2's compensate-on-restart
    /// reverse walk). Used at startup before the API server begins
    /// accepting requests.
    #[allow(dead_code)] // Used by PR 2's resume-on-startup; tests exercise it now.
    pub fn unresolved_sagas(&self) -> Result<Vec<UnresolvedSaga>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT saga_id, name, state, started_at, input_json
             FROM saga
             WHERE state IN ('running', 'compensating')
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
            "SELECT saga_id, name, state, started_at, terminal_at, failure_reason
             FROM saga
             ORDER BY COALESCE(terminal_at, started_at) DESC
             LIMIT ?1",
        )?;
        let rows: Vec<(i64, String, String, i64, Option<i64>, Option<String>)> = stmt
            .query_map(params![limit], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)?,
                    row.get::<_, Option<i64>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(stmt);

        let mut out = Vec::with_capacity(rows.len());
        for (saga_id, name, state, started_at, terminal_at, failure_reason) in rows {
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
            });
        }
        Ok(out)
    }
}

fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
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

        // Saga 3: failed (terminal).
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

        let unresolved = log.unresolved_sagas().unwrap();
        let ids: Vec<u64> = unresolved.iter().map(|u| u.saga_id).collect();
        assert_eq!(ids, vec![2, 4]);

        // Saga 2 carries its succeeded step.
        let saga2 = unresolved.iter().find(|u| u.saga_id == 2).unwrap();
        assert_eq!(saga2.state, "running");
        assert_eq!(saga2.steps.len(), 1);
        assert_eq!(saga2.steps[0].state, "succeeded");
        assert_eq!(saga2.steps[0].name, "Ping");

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
}
