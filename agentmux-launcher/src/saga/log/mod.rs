// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// LSD-1 — durable launcher saga log + API.
//
// Spec: `docs/specs/SPEC_LAUNCHER_SAGA_DURABILITY_2026-05-01.md`
//   - §3.1 storage (separate SQLite file at
//     `~/.agentmux/launcher-sagas.db`, WAL + 5s busy timeout +
//     foreign_keys=ON; same configuration as srv saga log)
//   - §3.2 schema (see `schema.rs`)
//   - §3.3 API surface (this module)
//   - §4 PR1 scope: log exists in isolation, NO coordinator wiring
//
// Design parallels to `agentmux-srv/src/sagas/log.rs`. Method names
// match where shape allows (`open`, `start_saga`, `terminate_saga`,
// `start_step`, `finish_step`, `fail_step`, `unresolved_sagas`,
// `mark_failed_compensation`, `snapshot_recent`, `max_saga_id`) so
// anyone reading both feels at home (LSD spec §3.3 last paragraph).
//
// Differences from srv's `SagaLog` driven by LSD spec §3.2:
//   - `target` column on the step table — launcher sagas dispatch to
//     self / host / srv; the column carries the `PipeTarget` so
//     `--diag sagas` can show "where did this step go?"
//   - No `compensated` saga state — F.5/F.6 sagas don't auto-compensate
//     (LSD spec §3.5). `failed_compensation` is the recovery-marked
//     terminal state for unresolved sagas at startup (PR LSD-3 wires
//     the recovery walker; this PR just defines the row).
//   - Timestamps are RFC3339 TEXT instead of epoch-ms INTEGER. Same
//     storage cost; greppable in raw SQLite shells. Conversion happens
//     in `now_rfc3339()` below.
//   - `vacuum_older_than(cutoff)` API is NEW relative to srv's log
//     (LSD spec §3.6 retention; srv doesn't ship this yet).
//
// Concurrency: a single `Mutex<Connection>` serializes writes. Each
// `start_step` / `finish_step` call holds the lock for <1ms; launcher
// saga rate is ≤ a few per second (LSD spec §3.7 — F.5/F.6 fire on
// rare user-initiated triggers). No connection pool needed.
//
// **PR LSD-1 is foundations-only.** The saga coordinator does NOT
// call any of these methods yet; LSD-2 wires them in. The module is
// declared on the saga tree (via `mod log;` in `saga/mod.rs`) and
// re-exports `LauncherSagaLog` so coordinator code can pick it up
// later without further plumbing changes.

use std::path::Path;
use std::sync::Mutex;

use agentmux_common::ipc::{Command, Event};
use chrono::{DateTime, Utc};
use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::PipeTarget;

mod schema;

#[cfg(test)]
mod tests;

/// Errors from the launcher saga log. Wraps the three error sources
/// the API can encounter: SQLite, JSON serialization, and (for the
/// public `open(path)` constructor) underlying file IO. Distinct
/// from srv's `StoreError` because srv's WaveStore wraps additional
/// migration-specific variants the launcher log doesn't need.
#[derive(Debug, thiserror::Error)]
pub enum LogError {
    #[error("sqlite: {0}")]
    Sqlite(#[from] rusqlite::Error),
    #[error("json: {0}")]
    Json(#[from] serde_json::Error),
    #[error("io: {0}")]
    Io(#[from] std::io::Error),
}

/// Outcome of a launcher saga, written by `terminate_saga`.
///
/// PR LSD-1 declares all variants up-front so the LSD-2 coordinator
/// wiring + LSD-3 recovery walker can land without further enum
/// edits. `dead_code` is suppressed at the enum level because every
/// variant is wired in a follow-up PR — opt-in suppression at the
/// type avoids per-variant `#[allow]` clutter.
///
/// Mirrors srv's `SagaOutcome` shape but adds `FailedCompensation`
/// (LSD spec §3.5 — recovery-walker terminal state) and removes
/// `Compensated` (no auto-compensation in launcher sagas yet).
///
/// PR LSD-2 calls `terminate_saga(.., Completed)` from `apply_action`
/// when a saga returns `SagaAction::Done`, and `Failed` when a saga
/// returns `SagaAction::Failed` or is evicted by the same-kind
/// concurrent gate. PR LSD-3 calls `mark_failed_compensation`
/// directly from the recovery walker; that path uses the dedicated
/// helper below rather than `terminate_saga(FailedCompensation { .. })`
/// because recovery wants to be idempotent across repeated
/// crash-restart cycles (the row may already be in
/// `failed_compensation` from a prior recovery).
///
/// Note: launcher sagas have no `Compensated` terminal state today
/// (per LSD spec §3.2 + §7 open question). F.5/F.6 sagas don't
/// auto-compensate; the schema CHECK constraint on `launcher_saga.state`
/// intentionally omits `'compensated'`. If a future class-D/E saga
/// needs compensation, add the variant + matching CHECK constraint
/// migration together — never one without the other.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // every variant wired by PR LSD-2 / LSD-3; pinned today by tests.rs
pub enum SagaOutcome {
    /// Saga ran to completion successfully. `SagaAction::Done` path.
    Completed,
    /// Saga failed with no compensation having run.
    Failed { reason: String },
    /// Saga was unresolved at launcher restart and the recovery walker
    /// marked it as such. Distinct from `Failed` so operators can
    /// filter for "interesting" cases via `--diag sagas`.
    /// Constructed by PR LSD-3's recovery walker (via
    /// `mark_failed_compensation`); included here for symmetry with
    /// the schema CHECK constraint.
    FailedCompensation { reason: String },
}

impl SagaOutcome {
    fn state_str(&self) -> &'static str {
        match self {
            SagaOutcome::Completed => "completed",
            SagaOutcome::Failed { .. } => "failed",
            SagaOutcome::FailedCompensation { .. } => "failed_compensation",
        }
    }

    fn reason(&self) -> Option<&str> {
        match self {
            SagaOutcome::Completed => None,
            SagaOutcome::Failed { reason }
            | SagaOutcome::FailedCompensation { reason } => Some(reason.as_str()),
        }
    }
}

/// Serialize a `PipeTarget` to the schema's `target` column. Mirrors
/// srv's `command_discriminant_name` style (snake_case strings rather
/// than Debug formatting) so `--diag sagas` output is greppable.
fn pipe_target_str(t: PipeTarget) -> &'static str {
    match t {
        PipeTarget::LauncherSelf => "launcher_self",
        PipeTarget::Host => "host",
        PipeTarget::Srv => "srv",
    }
}

/// A saga in `running`, `compensating`, or `failed` state at startup.
/// Returned by `unresolved_sagas`; consumed by PR LSD-3's recovery
/// walker to mark each as `failed_compensation` (LSD spec §3.5).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)] // Fields consumed by PR LSD-3's recovery walker.
pub struct UnresolvedLauncherSaga {
    pub saga_id: u64,
    pub name: String,
    pub state: String,
    pub started_at: String,
    pub input_json: String,
    pub failure_reason: Option<String>,
    pub steps: Vec<UnresolvedLauncherStep>,
}

/// A step row attached to an `UnresolvedLauncherSaga`. Steps are
/// returned in `step_index` ascending order.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)] // Fields consumed by PR LSD-3's recovery walker.
pub struct UnresolvedLauncherStep {
    pub step_index: u32,
    pub name: String,
    pub state: String,
    pub target: Option<String>,
    pub cmd_json: Option<String>,
    pub output_json: Option<String>,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub failure_reason: Option<String>,
}

/// Operator-facing snapshot of a recent saga, for `--diag sagas`.
/// Returned by `snapshot_recent`. Sorted most-recent-first by
/// `COALESCE(ended_at, started_at)`.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(dead_code)] // Fields consumed by PR LSD-3's `--diag sagas`.
pub struct SagaSummary {
    pub saga_id: u64,
    pub name: String,
    pub state: String,
    pub started_at: String,
    pub ended_at: Option<String>,
    pub failure_reason: Option<String>,
    /// Count of steps in `succeeded` or `compensated` state — i.e.
    /// progress through the saga.
    pub step_count: u32,
    /// JSON of saga input args, for operator triage.
    pub input_json: String,
}

/// SQLite-backed launcher saga log. Owned by `SagaCoordinator` as
/// `Arc<LauncherSagaLog>` once PR LSD-2 wires it; PR LSD-1 only
/// constructs and tests it in isolation.
pub struct LauncherSagaLog {
    conn: Mutex<Connection>,
}

impl LauncherSagaLog {
    /// Open a saga log backed by the given SQLite file. Configures
    /// WAL mode + 5s busy timeout + `foreign_keys=ON` (mirroring
    /// `SagaLog::open` in `agentmux-srv/src/sagas/log.rs`) and
    /// applies the schema migration from `schema.rs`.
    ///
    /// Idempotent: reopening the same DB applies the same DDL via
    /// `CREATE TABLE IF NOT EXISTS` — no double-creation, no error.
    #[allow(dead_code)] // wired in PR LSD-2 (`main.rs` opens the log on startup)
    pub fn open(path: &Path) -> Result<Self, LogError> {
        let conn = Connection::open(path)?;
        Self::configure_and_migrate(conn)
    }

    /// Open an in-memory saga log for testing. Used by `tests.rs`
    /// and by future PR LSD-2 coordinator integration tests.
    #[allow(dead_code)] // exercised under #[cfg(test)] only; see saga/log/tests.rs
    pub fn open_in_memory() -> Result<Self, LogError> {
        let conn = Connection::open_in_memory()?;
        Self::configure_and_migrate(conn)
    }

    fn configure_and_migrate(conn: Connection) -> Result<Self, LogError> {
        // Same pragma block as srv's `SagaLog::configure_and_migrate`
        // (codex P2 PR #631). `foreign_keys=ON` enforces the
        // `launcher_saga_step.saga_id REFERENCES launcher_saga(saga_id)`
        // declaration; without it, orphan step rows can be written
        // silently and corrupt diagnostics.
        conn.execute_batch(
            "PRAGMA journal_mode=WAL;
             PRAGMA busy_timeout=5000;
             PRAGMA synchronous=NORMAL;
             PRAGMA temp_store=MEMORY;
             PRAGMA foreign_keys=ON;",
        )?;
        schema::run_migrations(&conn)?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    /// Highest existing `saga_id` in the durable log, or 0 if empty.
    /// PR LSD-2 calls this at coordinator startup to seed
    /// `next_saga_id` so a launcher restart doesn't reuse ids that
    /// already have rows in the log.
    #[allow(dead_code)] // wired in PR LSD-2 (`SagaCoordinator::new` seed)
    pub fn max_saga_id(&self) -> Result<u64, LogError> {
        let conn = self.conn.lock().unwrap();
        // Mirror srv's `max_saga_id` — propagate query errors so a
        // transient SQLite read failure doesn't silently reseed the
        // allocator to 0 and re-collide with prior rows (codex P2 PR
        // #631 round 2 rationale; same hazard applies here).
        let max: Option<i64> =
            conn.query_row("SELECT MAX(saga_id) FROM launcher_saga", [], |r| r.get(0))?;
        Ok(max.unwrap_or(0).max(0) as u64)
    }

    /// Insert a fresh saga row in `running` state. Plain INSERT (not
    /// OR REPLACE): a duplicate saga_id is a bug worth surfacing,
    /// not a silent overwrite. Same rationale as srv's `start_saga`
    /// (codex P1 + reagent P1 PR #631).
    #[allow(dead_code)] // wired in PR LSD-2 (`SagaCoordinator::spawn_saga`)
    pub fn start_saga(
        &self,
        saga_id: u64,
        name: &str,
        input: &serde_json::Value,
    ) -> Result<(), LogError> {
        let now = now_rfc3339();
        let input_json = serde_json::to_string(input)?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO launcher_saga
             (saga_id, name, state, started_at, ended_at, input_json, failure_reason)
             VALUES (?1, ?2, 'running', ?3, NULL, ?4, NULL)",
            params![saga_id as i64, name, now, input_json],
        )?;
        Ok(())
    }

    /// Write a saga's terminal lifecycle row. Called by PR LSD-2's
    /// `apply_action` when the saga returns `Done` / `Failed`. The
    /// recovery walker uses `mark_failed_compensation` instead.
    #[allow(dead_code)] // wired in PR LSD-2
    pub fn terminate_saga(&self, saga_id: u64, outcome: SagaOutcome) -> Result<(), LogError> {
        let now = now_rfc3339();
        let state = outcome.state_str();
        let reason = outcome.reason();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE launcher_saga
             SET state = ?1, ended_at = ?2, failure_reason = ?3
             WHERE saga_id = ?4",
            params![state, now, reason, saga_id as i64],
        )?;
        Ok(())
    }

    /// Insert a `pending` step row before dispatching the command.
    /// `name` is a short discriminant string (e.g. "issue_cmd_host_reap_panes");
    /// `cmd` is serialized as JSON for replay/debugging; `target`
    /// records which pipe the command was destined for so `--diag
    /// sagas` can show provenance.
    #[allow(dead_code)] // wired in PR LSD-2 (`SagaCoordinator::apply_action::IssueCmd`)
    pub fn start_step(
        &self,
        saga_id: u64,
        step_index: u32,
        name: &str,
        target: PipeTarget,
        cmd: &Command,
    ) -> Result<(), LogError> {
        let now = now_rfc3339();
        let cmd_json = serde_json::to_string(cmd)?;
        let target_str = pipe_target_str(target);
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO launcher_saga_step
             (saga_id, step_index, name, state, cmd_json, target, started_at, ended_at, output_json, failure_reason)
             VALUES (?1, ?2, ?3, 'pending', ?4, ?5, ?6, NULL, NULL, NULL)",
            params![
                saga_id as i64,
                step_index,
                name,
                cmd_json,
                target_str,
                now
            ],
        )?;
        Ok(())
    }

    /// Mark a step `succeeded` and store the awaited event as JSON.
    /// PR LSD-2 calls this from `route_event_to_sagas` when a saga's
    /// `on_event` consumes its awaited bus event.
    #[allow(dead_code)] // wired in PR LSD-2
    pub fn finish_step(
        &self,
        saga_id: u64,
        step_index: u32,
        output: &Event,
    ) -> Result<(), LogError> {
        let now = now_rfc3339();
        let output_json = serde_json::to_string(output)?;
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE launcher_saga_step
             SET state = 'succeeded', output_json = ?1, ended_at = ?2
             WHERE saga_id = ?3 AND step_index = ?4",
            params![output_json, now, saga_id as i64, step_index],
        )?;
        Ok(())
    }

    /// Mark a step `failed`. Stores the reason in the step's
    /// `failure_reason` column (distinct from srv's log, which
    /// stuffs the reason into `output_json` as `{"error": ...}`;
    /// LSD schema gives us a dedicated column so we use it).
    #[allow(dead_code)] // wired in PR LSD-2 (saga timeout / dispatch error path)
    pub fn fail_step(
        &self,
        saga_id: u64,
        step_index: u32,
        reason: &str,
    ) -> Result<(), LogError> {
        let now = now_rfc3339();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "UPDATE launcher_saga_step
             SET state = 'failed', failure_reason = ?1, ended_at = ?2
             WHERE saga_id = ?3 AND step_index = ?4",
            params![reason, now, saga_id as i64, step_index],
        )?;
        Ok(())
    }

    /// Return all sagas still in `running`, `compensating`, or
    /// `failed` state, each with its full step list. PR LSD-3's
    /// startup recovery walker iterates this list and calls
    /// `mark_failed_compensation` on each (LSD spec §3.5).
    ///
    /// `failed` is included for symmetry with srv's `unresolved_sagas`
    /// (codex P1 PR #631 round 2): a launcher saga marked `failed`
    /// without restart-time recovery would still benefit from the
    /// `failed_compensation` upgrade so `--diag sagas` consistently
    /// surfaces it as "needs operator attention."
    #[allow(dead_code)] // wired in PR LSD-3 (recovery walker)
    pub fn unresolved_sagas(&self) -> Result<Vec<UnresolvedLauncherSaga>, LogError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT saga_id, name, state, started_at, input_json, failure_reason
             FROM launcher_saga
             WHERE state IN ('running', 'compensating', 'failed')
             ORDER BY saga_id ASC",
        )?;
        let saga_rows: Vec<(i64, String, String, String, String, Option<String>)> = stmt
            .query_map([], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    row.get::<_, Option<String>>(5)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(stmt);

        let mut out = Vec::with_capacity(saga_rows.len());
        for (saga_id, name, state, started_at, input_json, failure_reason) in saga_rows {
            let mut step_stmt = conn.prepare(
                "SELECT step_index, name, state, target, cmd_json,
                        output_json, started_at, ended_at, failure_reason
                 FROM launcher_saga_step
                 WHERE saga_id = ?1
                 ORDER BY step_index ASC",
            )?;
            let steps: Vec<UnresolvedLauncherStep> = step_stmt
                .query_map(params![saga_id], |row| {
                    Ok(UnresolvedLauncherStep {
                        step_index: row.get::<_, i64>(0)? as u32,
                        name: row.get::<_, String>(1)?,
                        state: row.get::<_, String>(2)?,
                        target: row.get::<_, Option<String>>(3)?,
                        cmd_json: row.get::<_, Option<String>>(4)?,
                        output_json: row.get::<_, Option<String>>(5)?,
                        started_at: row.get::<_, String>(6)?,
                        ended_at: row.get::<_, Option<String>>(7)?,
                        failure_reason: row.get::<_, Option<String>>(8)?,
                    })
                })?
                .collect::<Result<Vec<_>, _>>()?;
            out.push(UnresolvedLauncherSaga {
                saga_id: saga_id as u64,
                name,
                state,
                started_at,
                input_json,
                failure_reason,
                steps,
            });
        }
        Ok(out)
    }

    /// Fetch the step rows for a single saga regardless of saga state.
    /// `unresolved_sagas` filters out `failed_compensation` (and other
    /// terminal states), but `--diag sagas` needs to surface step rows
    /// for sagas the recovery walker just marked `failed_compensation`
    /// — operators triaging a recovered crash need to see what was
    /// pending when the launcher exited. (codex P1 PR #647 round 1.)
    pub fn get_saga_steps(&self, saga_id: u64) -> Result<Vec<UnresolvedLauncherStep>, LogError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT step_index, name, state, target, cmd_json,
                    output_json, started_at, ended_at, failure_reason
             FROM launcher_saga_step
             WHERE saga_id = ?1
             ORDER BY step_index ASC",
        )?;
        let steps: Vec<UnresolvedLauncherStep> = stmt
            .query_map(params![saga_id as i64], |row| {
                Ok(UnresolvedLauncherStep {
                    step_index: row.get::<_, i64>(0)? as u32,
                    name: row.get::<_, String>(1)?,
                    state: row.get::<_, String>(2)?,
                    target: row.get::<_, Option<String>>(3)?,
                    cmd_json: row.get::<_, Option<String>>(4)?,
                    output_json: row.get::<_, Option<String>>(5)?,
                    started_at: row.get::<_, String>(6)?,
                    ended_at: row.get::<_, Option<String>>(7)?,
                    failure_reason: row.get::<_, Option<String>>(8)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(steps)
    }

    /// Mark a saga as `failed_compensation` — the recovery walker's
    /// terminal write. Idempotent across repeated calls (the saga
    /// stays in `failed_compensation`; only `ended_at` and
    /// `failure_reason` get overwritten with the latest values).
    /// LSD spec §3.5 — operator-review terminal state.
    #[allow(dead_code)] // wired in PR LSD-3 (recovery walker)
    pub fn mark_failed_compensation(
        &self,
        saga_id: u64,
        reason: &str,
    ) -> Result<(), LogError> {
        let now = now_rfc3339();
        let conn = self.conn.lock().unwrap();
        // Preserve original failure_reason when already populated.
        // A saga in `failed` state pre-crash carries the precise
        // original cause (timeout, dispatch error, etc.) that
        // operators need for post-mortem. Recovery transitions
        // state to `failed_compensation` but augments rather than
        // replaces failure_reason — appends the restart context
        // so both signals are visible in `--diag sagas`.
        // (codex P2 PR #647 round 1.)
        conn.execute(
            "UPDATE launcher_saga
             SET state = 'failed_compensation',
                 ended_at = ?1,
                 failure_reason = CASE
                     WHEN failure_reason IS NULL OR failure_reason = ''
                       THEN ?2
                     ELSE failure_reason || ' | recovered: ' || ?2
                 END
             WHERE saga_id = ?3",
            params![now, reason, saga_id as i64],
        )?;
        Ok(())
    }

    /// Return up to `limit` recent sagas for `--diag sagas`. Sorted
    /// most-recent-first by `COALESCE(ended_at, started_at)`. Mirrors
    /// srv's `snapshot_recent` shape.
    #[allow(dead_code)] // wired in PR LSD-3 (`--diag sagas` printer)
    pub fn snapshot_recent(&self, limit: usize) -> Result<Vec<SagaSummary>, LogError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT saga_id, name, state, started_at, ended_at, failure_reason, input_json
             FROM launcher_saga
             ORDER BY COALESCE(ended_at, started_at) DESC, saga_id DESC
             LIMIT ?1",
        )?;
        let rows: Vec<(
            i64,
            String,
            String,
            String,
            Option<String>,
            Option<String>,
            String,
        )> = stmt
            .query_map(params![limit as i64], |row| {
                Ok((
                    row.get::<_, i64>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, Option<String>>(4)?,
                    row.get::<_, Option<String>>(5)?,
                    row.get::<_, String>(6)?,
                ))
            })?
            .collect::<Result<Vec<_>, _>>()?;
        drop(stmt);

        let mut out = Vec::with_capacity(rows.len());
        for (saga_id, name, state, started_at, ended_at, failure_reason, input_json) in rows {
            let count: Option<i64> = conn
                .query_row(
                    "SELECT COUNT(*) FROM launcher_saga_step
                     WHERE saga_id = ?1 AND state IN ('succeeded', 'compensated')",
                    params![saga_id],
                    |row| row.get(0),
                )
                .optional()?;
            out.push(SagaSummary {
                saga_id: saga_id as u64,
                name,
                state,
                started_at,
                ended_at,
                failure_reason,
                step_count: count.unwrap_or(0) as u32,
                input_json,
            });
        }
        Ok(out)
    }

    /// Delete saga rows whose `ended_at` is before `cutoff` AND whose
    /// state is terminal (`completed`, `failed`, `failed_compensation`).
    /// Returns the number of rows deleted. In-flight sagas (`running`,
    /// `compensating`) are NEVER vacuumed — that would mask
    /// crashed-mid-saga incidents from the recovery walker (LSD spec §3.6).
    ///
    /// `ON DELETE CASCADE` on `launcher_saga_step.saga_id` ensures
    /// the corresponding step rows go with the saga in a single
    /// SQLite transaction — no manual cleanup needed.
    // LSD-4: wired by `main.rs::run_windows` startup retention task.
    pub fn vacuum_older_than(&self, cutoff: DateTime<Utc>) -> Result<usize, LogError> {
        let cutoff_str = cutoff.to_rfc3339();
        let conn = self.conn.lock().unwrap();
        let removed = conn.execute(
            "DELETE FROM launcher_saga
             WHERE state IN ('completed', 'failed', 'failed_compensation')
               AND ended_at IS NOT NULL
               AND ended_at < ?1",
            params![cutoff_str],
        )?;
        Ok(removed)
    }
}

/// RFC3339 timestamp for `started_at` / `ended_at` columns. Single
/// helper so test+production paths agree on format precisely.
fn now_rfc3339() -> String {
    Utc::now().to_rfc3339()
}
