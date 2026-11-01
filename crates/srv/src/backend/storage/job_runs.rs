// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The run ledger: one row per time a job was meant to run, including the
//! times it didn't (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §3.1).
//!
//! It lives in the always-global identity store, beside the work queue, so
//! every channel and version on this computer sees one ledger. The unique
//! index on `(job_id, scheduled_for_ms)` is the slot lease: a scheduler
//! inserts its slot's row before it fires, and a second scheduler sharing the
//! job finds the slot taken and doesn't fire.
//!
//! Nothing here deletes a run: history is kept forever (§3.1, "Retention:
//! none"). A skip folded into the previous row of the same job and reason
//! keeps its own row, marked `collapsed_into` that row and left out of
//! history, so its slot stays leased.

use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use super::store::Store;
use super::StoreError;

/// The table, created on every open of the identity store, with no schema
/// version bump (see `work_claims::SCHEMA`).
pub(crate) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS db_job_runs (
        id               TEXT PRIMARY KEY,
        job_id           TEXT NOT NULL,
        job_source       TEXT NOT NULL,
        job_name         TEXT NOT NULL DEFAULT '',
        template_id      TEXT NOT NULL DEFAULT '',
        template_version TEXT NOT NULL DEFAULT '',
        trigger_kind     TEXT NOT NULL,
        scheduled_for_ms INTEGER,
        triggered_by     TEXT NOT NULL DEFAULT '',
        attempt          INTEGER NOT NULL DEFAULT 1,
        state            TEXT NOT NULL,
        reason           TEXT NOT NULL DEFAULT '',
        infra            TEXT NOT NULL DEFAULT '',
        outcome          TEXT NOT NULL DEFAULT 'unknown',
        summary          TEXT NOT NULL DEFAULT '',
        target_agent_uid TEXT NOT NULL DEFAULT '',
        block_id         TEXT NOT NULL DEFAULT '',
        turn_id          INTEGER,
        account_id       TEXT NOT NULL DEFAULT '',
        provider         TEXT NOT NULL DEFAULT '',
        started_at_ms    INTEGER NOT NULL,
        ended_at_ms      INTEGER,
        cost_usd         REAL NOT NULL DEFAULT 0,
        cost_known       INTEGER NOT NULL DEFAULT 0,
        cost_partial     INTEGER NOT NULL DEFAULT 0,
        output_tokens    INTEGER NOT NULL DEFAULT 0,
        steps            INTEGER NOT NULL DEFAULT 0,
        links            TEXT NOT NULL DEFAULT '{}',
        skip_count       INTEGER NOT NULL DEFAULT 0,
        last_skip_at_ms  INTEGER,
        reporter         TEXT NOT NULL DEFAULT '',
        recorded_by      TEXT NOT NULL DEFAULT '',
        collapsed_into   TEXT NOT NULL DEFAULT ''
    );
    CREATE INDEX IF NOT EXISTS idx_ids_job_runs_job_started ON db_job_runs(job_id, started_at_ms);
    CREATE UNIQUE INDEX IF NOT EXISTS idx_ids_job_runs_slot ON db_job_runs(job_id, scheduled_for_ms)
        WHERE scheduled_for_ms IS NOT NULL;
    CREATE INDEX IF NOT EXISTS idx_ids_job_runs_state ON db_job_runs(state);
    CREATE INDEX IF NOT EXISTS idx_ids_job_runs_account_started ON db_job_runs(account_id, started_at_ms);
    CREATE TABLE IF NOT EXISTS db_job_flags (
        key        TEXT PRIMARY KEY,
        value      TEXT NOT NULL DEFAULT '',
        set_at_ms  INTEGER NOT NULL,
        set_by     TEXT NOT NULL DEFAULT ''
    );";

/// Columns added to `db_job_runs` after its first shape, run on every open
/// (`apply_schema`) as `cron::JOB_TABLES` is: a table an earlier
/// build created gains them, and a new column with a default needs no
/// schema version bump.
const JOB_RUNS_COLUMNS: [&str; 1] = ["ALTER TABLE db_job_runs ADD COLUMN collapsed_into TEXT NOT NULL DEFAULT ''"];

/// Create the tables (the ledger's, and its incidents', `job_incidents`),
/// then add any column the ledger lacks; "duplicate column" is the column
/// already being there.
pub(crate) fn apply_schema(conn: &rusqlite::Connection) -> Result<(), StoreError> {
    conn.execute_batch(SCHEMA)?;
    conn.execute_batch(super::job_incidents::SCHEMA)?;
    conn.execute_batch(super::job_cloud::SCHEMA)?;
    for stmt in JOB_RUNS_COLUMNS {
        if let Err(e) = conn.execute_batch(stmt) {
            if !e.to_string().contains("duplicate column") {
                return Err(e.into());
            }
        }
    }
    Ok(())
}

pub(super) const COLS: &str = "id, job_id, job_source, job_name, template_id, template_version, trigger_kind, \
     scheduled_for_ms, triggered_by, attempt, state, reason, infra, outcome, summary, target_agent_uid, \
     block_id, turn_id, account_id, provider, started_at_ms, ended_at_ms, cost_usd, cost_known, \
     cost_partial, output_tokens, steps, links, skip_count, last_skip_at_ms, reporter, recorded_by";

/// Run states (§3.1).
pub mod state {
    pub const SCHEDULED: &str = "scheduled";
    pub const SKIPPED: &str = "skipped";
    /// Catch-up `latest`'s one coalesced fire, waiting for its agent.
    pub const PENDING: &str = "pending";
    pub const RUNNING: &str = "running";
    pub const FINISHED: &str = "finished";
    pub const MISSED: &str = "missed";
    /// A cloud run that reported start and then nothing within its time limit (§4.4).
    pub const LOST: &str = "lost";
    /// The states a run can still move on from, while `ended_at_ms` is
    /// unset: `OPEN_NOT_ENDED`'s list, and [`super::JobRun::is_open`].
    pub const OPEN: [&str; 3] = [SCHEDULED, PENDING, RUNNING];
}

/// Why a run was skipped, missed or stopped, or how a pending one ended
/// (§3.1; the ones P1 writes).
pub mod reason {
    pub const APP_NOT_RUNNING: &str = "app_not_running";
    /// A cloud job's expected slot and its grace passed with no event for it (§4.4).
    pub const NO_REPORT: &str = "no_report";
    /// A cloud run's `finished` event came after the watchdog gave it up.
    pub const LATE: &str = "late";
    pub const TARGET_MISSING: &str = "target_missing";
    pub const TARGET_STOPPED: &str = "target_stopped";
    /// A newer pending fire replaced this one.
    pub const SUPERSEDED: &str = "superseded";
    /// The job's next fire time passed before its agent came back.
    pub const EXPIRED: &str = "expired";
    /// The job was paused when its slot came.
    pub const PAUSED: &str = "paused";
    /// Every job was paused by the operator (Pause all) when its slot came.
    pub const PAUSED_ALL: &str = "paused_all";
    /// The job's previous run was still running (policy `overlap`): a skipped
    /// slot, or the one buffered run waiting for it.
    pub const OVERLAP: &str = "overlap";
    /// A run of the job's concurrency group is running; this one waits.
    pub const GROUP_BUSY: &str = "group_busy";
    /// Three unattended agent runs are running already; this one waits.
    pub const GLOBAL_LIMIT: &str = "global_limit";
    /// An infrastructure failure's retry, waiting out its backoff.
    pub const RETRY_WAIT: &str = "retry_wait";
    /// The run was stopped at the job's `timeout_minutes`.
    pub const TIMEOUT: &str = "timeout";
    /// The job's spend today, with the run's own cap, would pass its daily
    /// cap (§5.3): the slot is skipped.
    pub const JOB_CAP: &str = "job_cap";
    /// The same for the account the run would use.
    pub const ACCOUNT_CAP: &str = "account_cap";
    /// A `message` job reached its messages a day to another agent.
    pub const MESSAGE_CAP: &str = "message_cap";
    /// The run was stopped at its per-run cap.
    pub const RUN_CAP: &str = "run_cap";
    /// The skips a cap makes: the once-a-day alert and "over cap today".
    pub const CAPS: [&str; 3] = [JOB_CAP, ACCOUNT_CAP, MESSAGE_CAP];
    /// The reasons a `scheduled` run waits to start with.
    pub const WAITING: [&str; 4] = [OVERLAP, GROUP_BUSY, GLOBAL_LIMIT, RETRY_WAIT];
}

/// Infrastructure status (§3.1; the ones P1 writes).
pub mod infra {
    pub const OK: &str = "ok";
    pub const ERROR: &str = "error";
    pub const TIMEOUT: &str = "timeout";
    pub const INTERRUPTED: &str = "interrupted";
    /// The run reached its per-run cap and was stopped (§5.3).
    pub const OVER_BUDGET: &str = "over_budget";
}

/// What a run concluded, separate from whether it ran (§2).
pub const OUTCOMES: &[&str] = &["ok", "findings", "failed", "unknown"];

/// The longest `summary` kept (§3.1).
pub const SUMMARY_MAX_CHARS: usize = 300;

/// `text` cut to [`SUMMARY_MAX_CHARS`], on a character boundary.
pub fn clip_summary(text: &str) -> String {
    let text = text.trim();
    if text.chars().count() <= SUMMARY_MAX_CHARS {
        return text.to_string();
    }
    let mut s: String = text.chars().take(SUMMARY_MAX_CHARS - 1).collect();
    s.push('…');
    s
}

/// One row of `db_job_runs`. Field meanings are the spec's table (§3.1).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct JobRun {
    pub id: String,
    pub job_id: String,
    pub job_source: String,
    pub job_name: String,
    #[serde(default)]
    pub template_id: String,
    #[serde(default)]
    pub template_version: String,
    pub trigger: String,
    #[ts(type = "number | null")]
    pub scheduled_for_ms: Option<i64>,
    pub triggered_by: String,
    #[ts(type = "number")]
    pub attempt: i64,
    pub state: String,
    pub reason: String,
    pub infra: String,
    pub outcome: String,
    pub summary: String,
    pub target_agent_uid: String,
    pub block_id: String,
    #[ts(type = "number | null")]
    pub turn_id: Option<i64>,
    pub account_id: String,
    pub provider: String,
    #[ts(type = "number")]
    pub started_at_ms: i64,
    #[ts(type = "number | null")]
    pub ended_at_ms: Option<i64>,
    pub cost_usd: f64,
    pub cost_known: bool,
    pub cost_partial: bool,
    #[ts(type = "number")]
    pub output_tokens: i64,
    #[ts(type = "number")]
    pub steps: i64,
    /// JSON object: the transcript, a Drone run, reported links.
    #[ts(type = "Record<string, unknown>")]
    pub links: serde_json::Value,
    #[ts(type = "number")]
    pub skip_count: i64,
    #[ts(type = "number | null")]
    pub last_skip_at_ms: Option<i64>,
    #[serde(default)]
    pub reporter: String,
    pub recorded_by: String,
}

impl JobRun {
    /// A new row for `job_id` in `state`, started now, with every other field
    /// empty.
    pub fn new(id: &str, job_id: &str, job_source: &str, job_name: &str, trigger: &str, state: &str, now_ms: i64) -> Self {
        JobRun {
            id: id.to_string(),
            job_id: job_id.to_string(),
            job_source: job_source.to_string(),
            job_name: job_name.to_string(),
            trigger: trigger.to_string(),
            attempt: 1,
            state: state.to_string(),
            outcome: "unknown".to_string(),
            started_at_ms: now_ms,
            links: serde_json::json!({}),
            ..Default::default()
        }
    }

    /// Whether the run can still move on: what `OPEN_NOT_ENDED` selects.
    pub fn is_open(&self) -> bool {
        self.ended_at_ms.is_none() && state::OPEN.contains(&self.state.as_str())
    }
}

pub(super) fn row_to_run(row: &Row) -> rusqlite::Result<JobRun> {
    let links: String = row.get("links")?;
    Ok(JobRun {
        id: row.get("id")?,
        job_id: row.get("job_id")?,
        job_source: row.get("job_source")?,
        job_name: row.get("job_name")?,
        template_id: row.get("template_id")?,
        template_version: row.get("template_version")?,
        trigger: row.get("trigger_kind")?,
        scheduled_for_ms: row.get("scheduled_for_ms")?,
        triggered_by: row.get("triggered_by")?,
        attempt: row.get("attempt")?,
        state: row.get("state")?,
        reason: row.get("reason")?,
        infra: row.get("infra")?,
        outcome: row.get("outcome")?,
        summary: row.get("summary")?,
        target_agent_uid: row.get("target_agent_uid")?,
        block_id: row.get("block_id")?,
        turn_id: row.get("turn_id")?,
        account_id: row.get("account_id")?,
        provider: row.get("provider")?,
        started_at_ms: row.get("started_at_ms")?,
        ended_at_ms: row.get("ended_at_ms")?,
        cost_usd: row.get("cost_usd")?,
        cost_known: row.get::<_, i64>("cost_known")? != 0,
        cost_partial: row.get::<_, i64>("cost_partial")? != 0,
        output_tokens: row.get("output_tokens")?,
        steps: row.get("steps")?,
        links: serde_json::from_str(&links).unwrap_or_else(|_| serde_json::json!({})),
        skip_count: row.get("skip_count")?,
        last_skip_at_ms: row.get("last_skip_at_ms")?,
        reporter: row.get("reporter")?,
        recorded_by: row.get("recorded_by")?,
    })
}

/// A finished turn's figures, from the turn ledger (§3.2).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct RunCost {
    pub cost_usd: f64,
    pub cost_known: bool,
    pub output_tokens: i64,
    pub steps: i64,
}

/// States a run can still move on from. A pending run that was superseded
/// or expired keeps its state, with `ended_at_ms` set: it is closed. The
/// list is [`state::OPEN`] (a test holds them equal).
const OPEN_NOT_ENDED: &str = "state IN ('scheduled','pending','running') AND ended_at_ms IS NULL";

/// A decisive run: one that finished, or failed (a miss, a lost run, an
/// expired pending fire, a failing infrastructure status or outcome). The
/// same rule as `jobs::verdict::decisive`, whose test checks the two agree.
const DECISIVE: &str = "((state IN ('finished','missed','lost')
                             OR (state = 'pending' AND reason = 'expired')
                             OR infra IN ('error','timeout','over_budget','interrupted')
                             OR outcome = 'failed')
                            AND NOT (json_valid(links) AND json_extract(links, '$.retried_by') IS NOT NULL))";

impl Store {
    /// Create the table if this store doesn't have it yet (a test store, or
    /// one opened before this build). Idempotent.
    pub fn job_runs_ensure_schema(&self) -> Result<(), StoreError> {
        apply_schema(&self.conn().lock().unwrap())
    }

    /// Insert `run`. `Ok(false)` when its slot (`job_id`, `scheduled_for_ms`)
    /// or its id is already taken: another scheduler has this slot.
    pub fn job_run_insert(&self, run: &JobRun) -> Result<bool, StoreError> {
        let conn = self.conn().lock().unwrap();
        let n = conn.execute(
            &format!(
                "INSERT OR IGNORE INTO db_job_runs ({COLS}) VALUES \
                 (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,?28,?29,?30,?31,?32)"
            ),
            params![
                run.id,
                run.job_id,
                run.job_source,
                run.job_name,
                run.template_id,
                run.template_version,
                run.trigger,
                run.scheduled_for_ms,
                run.triggered_by,
                run.attempt,
                run.state,
                run.reason,
                run.infra,
                run.outcome,
                run.summary,
                run.target_agent_uid,
                run.block_id,
                run.turn_id,
                run.account_id,
                run.provider,
                run.started_at_ms,
                run.ended_at_ms,
                run.cost_usd,
                run.cost_known as i64,
                run.cost_partial as i64,
                run.output_tokens,
                run.steps,
                run.links.to_string(),
                run.skip_count,
                run.last_skip_at_ms,
                run.reporter,
                run.recorded_by,
            ],
        )?;
        Ok(n > 0)
    }

    pub fn job_run_get(&self, id: &str) -> Result<Option<JobRun>, StoreError> {
        let conn = self.conn().lock().unwrap();
        Ok(conn
            .query_row(&format!("SELECT {COLS} FROM db_job_runs WHERE id = ?1"), params![id], row_to_run)
            .optional()?)
    }

    /// A job's runs, newest first, at most `limit`, started before `before`
    /// when given (for paging).
    pub fn job_runs_for_job(&self, job_id: &str, limit: i64, before: Option<i64>) -> Result<Vec<JobRun>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLS} FROM db_job_runs
              WHERE job_id = ?1 AND collapsed_into = '' AND (?2 IS NULL OR started_at_ms < ?2)
              ORDER BY started_at_ms DESC, rowid DESC LIMIT ?3"
        ))?;
        let rows = stmt.query_map(params![job_id, before, limit.max(0)], row_to_run)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// The job's newest run, if any.
    pub fn job_run_last(&self, job_id: &str) -> Result<Option<JobRun>, StoreError> {
        Ok(self.job_runs_for_job(job_id, 1, None)?.pop())
    }

    /// The job's newest decisive run: one that finished, or failed (a miss,
    /// a lost run, an expired pending fire, a failing infrastructure status
    /// or outcome). Skips, and runs still scheduled, pending or running, are
    /// passed over however many there are. The same rule as
    /// `jobs::verdict::decisive`, whose test checks the two agree.
    pub fn job_run_last_decisive(&self, job_id: &str) -> Result<Option<JobRun>, StoreError> {
        let conn = self.conn().lock().unwrap();
        Ok(conn
            .query_row(
                &format!(
                    "SELECT {COLS} FROM db_job_runs
                      WHERE job_id = ?1 AND collapsed_into = '' AND {DECISIVE}
                      ORDER BY started_at_ms DESC, rowid DESC LIMIT 1"
                ),
                params![job_id],
                row_to_run,
            )
            .optional()?)
    }

    /// The job's decisive runs that have ended, in the order they ended
    /// (`ended_at_ms`, then `id`): the ones after `after` (an ended time and
    /// a run id), at most `limit`; with no `after`, only the newest. What
    /// the incident keeper walks (`jobs::incidents`).
    pub fn job_runs_decisive_after(
        &self,
        job_id: &str,
        after: Option<(i64, &str)>,
        limit: i64,
    ) -> Result<Vec<JobRun>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let base = format!(
            "SELECT {COLS} FROM db_job_runs
              WHERE job_id = ?1 AND collapsed_into = '' AND ended_at_ms IS NOT NULL AND {DECISIVE}"
        );
        let rows = match after {
            Some((ended, id)) => {
                let mut stmt = conn.prepare(&format!(
                    "{base} AND (ended_at_ms > ?2 OR (ended_at_ms = ?2 AND id > ?3)) ORDER BY ended_at_ms, id LIMIT ?4"
                ))?;
                let rows = stmt.query_map(params![job_id, ended, id, limit.max(0)], row_to_run)?;
                rows.collect::<Result<Vec<_>, _>>()?
            }
            None => {
                let mut stmt = conn.prepare(&format!("{base} ORDER BY ended_at_ms DESC, id DESC LIMIT 1"))?;
                let rows = stmt.query_map(params![job_id], row_to_run)?;
                rows.collect::<Result<Vec<_>, _>>()?
            }
        };
        Ok(rows)
    }

    /// Turn the just-inserted placeholder `id` into a skip for `reason`, or,
    /// when the job's previous row is a skip for the same reason, fold it
    /// into that row (`skip_count` + `count`), so consecutive skips show as
    /// one row (§3.1). A folded placeholder stays, marked `collapsed_into`
    /// and hidden from history, so the unique index keeps its slot taken: a
    /// second scheduler accounting for the same slot finds it leased and
    /// doesn't count it again. Returns the id of the row that shows the skip.
    pub fn job_run_skip(&self, id: &str, reason: &str, count: i64, now_ms: i64) -> Result<String, StoreError> {
        let mut conn = self.conn().lock().unwrap();
        let tx = conn.transaction()?;
        let job_id: Option<String> =
            tx.query_row("SELECT job_id FROM db_job_runs WHERE id = ?1", params![id], |r| r.get(0)).optional()?;
        let Some(job_id) = job_id else {
            return Ok(id.to_string());
        };
        let previous: Option<(String, String, String)> = tx
            .query_row(
                "SELECT id, state, reason FROM db_job_runs
                  WHERE job_id = ?1 AND id <> ?2 AND collapsed_into = ''
                  ORDER BY started_at_ms DESC, rowid DESC LIMIT 1",
                params![job_id, id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let held_by = match previous {
            Some((prev, st, why)) if st == state::SKIPPED && why == reason => {
                tx.execute(
                    "UPDATE db_job_runs SET skip_count = skip_count + ?2, last_skip_at_ms = ?3 WHERE id = ?1",
                    params![prev, count, now_ms],
                )?;
                tx.execute(
                    "UPDATE db_job_runs SET state = ?2, reason = ?3, collapsed_into = ?4, ended_at_ms = ?5 WHERE id = ?1",
                    params![id, state::SKIPPED, reason, prev, now_ms],
                )?;
                prev
            }
            _ => {
                tx.execute(
                    "UPDATE db_job_runs SET state = ?2, reason = ?3, skip_count = ?4, last_skip_at_ms = ?5,
                            ended_at_ms = ?5
                      WHERE id = ?1",
                    params![id, state::SKIPPED, reason, count, now_ms],
                )?;
                id.to_string()
            }
        };
        tx.commit()?;
        Ok(held_by)
    }

    /// A delivered run is being worked on: who has it, from when.
    pub fn job_run_mark_running(
        &self,
        id: &str,
        target_agent_uid: &str,
        block_id: &str,
        account_id: &str,
        provider: &str,
        now_ms: i64,
    ) -> Result<bool, StoreError> {
        let conn = self.conn().lock().unwrap();
        let n = conn.execute(
            &format!(
                "UPDATE db_job_runs
                    SET state = ?2, reason = '', block_id = ?3,
                        account_id = CASE WHEN ?4 <> '' THEN ?4 ELSE account_id END, provider = ?5,
                        target_agent_uid = CASE WHEN ?6 <> '' THEN ?6 ELSE target_agent_uid END,
                        started_at_ms = ?7, ended_at_ms = NULL
                  WHERE id = ?1 AND {OPEN_NOT_ENDED}"
            ),
            params![id, state::RUNNING, block_id, account_id, provider, target_agent_uid, now_ms],
        )?;
        Ok(n > 0)
    }

    /// A fire that reached no one: `missed` (closed) or, under catch-up
    /// `latest`, `pending` (waiting for its agent), with `why`.
    pub fn job_run_set_undelivered(&self, id: &str, new_state: &str, why: &str, now_ms: i64) -> Result<bool, StoreError> {
        let ended = (new_state != state::PENDING).then_some(now_ms);
        let conn = self.conn().lock().unwrap();
        let n = conn.execute(
            &format!(
                "UPDATE db_job_runs SET state = ?2, reason = ?3, ended_at_ms = ?4
                  WHERE id = ?1 AND {OPEN_NOT_ENDED}"
            ),
            params![id, new_state, why, ended],
        )?;
        Ok(n > 0)
    }

    /// End a job's waiting pending fires (other than `except`) with `why`
    /// (`superseded`, `expired`). Returns their ids.
    pub fn job_runs_end_pending(&self, job_id: &str, except: &str, why: &str, now_ms: i64) -> Result<Vec<String>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let mut stmt = conn.prepare(
            "UPDATE db_job_runs SET reason = ?3, ended_at_ms = ?4
              WHERE job_id = ?1 AND id <> ?2 AND state = 'pending' AND ended_at_ms IS NULL
              RETURNING id",
        )?;
        let rows = stmt.query_map(params![job_id, except, why, now_ms], |r| r.get(0))?;
        Ok(rows.collect::<Result<Vec<String>, _>>()?)
    }

    /// End one waiting pending fire with `why`.
    pub fn job_run_end_one_pending(&self, id: &str, why: &str, now_ms: i64) -> Result<bool, StoreError> {
        let conn = self.conn().lock().unwrap();
        let n = conn.execute(
            "UPDATE db_job_runs SET reason = ?2, ended_at_ms = ?3
              WHERE id = ?1 AND state = 'pending' AND ended_at_ms IS NULL",
            params![id, why, now_ms],
        )?;
        Ok(n > 0)
    }

    /// The cron runs that take a place: running, or admitted and being
    /// delivered (`scheduled` with no waiting reason). Overlap, concurrency
    /// groups and the global limit count these.
    pub fn job_runs_occupying(&self) -> Result<Vec<JobRun>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLS} FROM db_job_runs
              WHERE job_source = 'cron' AND ended_at_ms IS NULL
                AND (state = 'running' OR (state = 'scheduled' AND reason = ''))
              ORDER BY started_at_ms"
        ))?;
        let rows = stmt.query_map([], row_to_run)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// The cron runs waiting to start, oldest first: `scheduled` with a
    /// waiting reason ([`reason::WAITING`]).
    pub fn job_runs_waiting(&self) -> Result<Vec<JobRun>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let list = reason::WAITING.iter().map(|r| format!("'{r}'")).collect::<Vec<_>>().join(",");
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLS} FROM db_job_runs
              WHERE job_source = 'cron' AND state = 'scheduled' AND ended_at_ms IS NULL AND reason IN ({list})
              ORDER BY started_at_ms, rowid"
        ))?;
        let rows = stmt.query_map([], row_to_run)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Make the just-inserted `scheduled` run wait for `why`.
    pub fn job_run_set_wait(&self, id: &str, why: &str) -> Result<bool, StoreError> {
        let conn = self.conn().lock().unwrap();
        let n = conn.execute(
            "UPDATE db_job_runs SET reason = ?2 WHERE id = ?1 AND state = 'scheduled' AND ended_at_ms IS NULL",
            params![id, why],
        )?;
        Ok(n > 0)
    }

    /// A waiting run starts: its reason is cleared if it still is `from`
    /// (only one scheduler admits it), and its start is now.
    pub fn job_run_admit(&self, id: &str, from: &str, now_ms: i64) -> Result<bool, StoreError> {
        let conn = self.conn().lock().unwrap();
        let n = conn.execute(
            "UPDATE db_job_runs SET reason = '', started_at_ms = ?3
              WHERE id = ?1 AND state = 'scheduled' AND reason = ?2 AND ended_at_ms IS NULL",
            params![id, from, now_ms],
        )?;
        Ok(n > 0)
    }

    /// Close a still-open run that something stopped (the timeout, a
    /// `cancel_other` overlap): `finished` with `infra`, `why` as its reason
    /// and `summary`. `false` when it was already closed.
    pub fn job_run_stop(&self, id: &str, infra_status: &str, why: &str, summary: &str, now_ms: i64) -> Result<bool, StoreError> {
        let conn = self.conn().lock().unwrap();
        let n = conn.execute(
            &format!(
                "UPDATE db_job_runs
                    SET state = 'finished', infra = ?2, reason = ?3, summary = ?4, ended_at_ms = ?5
                  WHERE id = ?1 AND {OPEN_NOT_ENDED}"
            ),
            params![id, infra_status, why, clip_summary(summary), now_ms],
        )?;
        Ok(n > 0)
    }

    /// Close an open run that did its work at once (a filed task): `finished`,
    /// infra `ok`, with the outcome it concluded.
    pub fn job_run_conclude(&self, id: &str, outcome: &str, summary: &str, now_ms: i64) -> Result<bool, StoreError> {
        let conn = self.conn().lock().unwrap();
        let n = conn.execute(
            &format!(
                "UPDATE db_job_runs
                    SET state = 'finished', infra = 'ok', outcome = ?2, summary = ?3, ended_at_ms = ?4
                  WHERE id = ?1 AND {OPEN_NOT_ENDED}"
            ),
            params![id, outcome, clip_summary(summary), now_ms],
        )?;
        Ok(n > 0)
    }

    /// The turn a running run started or joined; `partial` when it joined a
    /// turn already running, so its cost includes other work.
    pub fn job_run_bind_turn(&self, id: &str, turn_id: i64, partial: bool) -> Result<bool, StoreError> {
        let conn = self.conn().lock().unwrap();
        let n = conn.execute(
            "UPDATE db_job_runs SET turn_id = ?2, cost_partial = ?3 WHERE id = ?1 AND state = 'running'",
            params![id, turn_id, partial as i64],
        )?;
        Ok(n > 0)
    }

    /// Close a still-open run as `finished` with `infra`, its cost, and a
    /// summary when the run has none yet. `false` when it was already closed.
    pub fn job_run_finish(
        &self,
        id: &str,
        infra_status: &str,
        cost: Option<RunCost>,
        summary: &str,
        now_ms: i64,
    ) -> Result<bool, StoreError> {
        let conn = self.conn().lock().unwrap();
        let c = cost.unwrap_or_default();
        let n = conn.execute(
            &format!(
                "UPDATE db_job_runs
                    SET state = ?10, infra = ?2, ended_at_ms = ?3,
                        cost_usd = CASE WHEN ?4 THEN ?5 ELSE cost_usd END,
                        cost_known = CASE WHEN ?4 THEN ?6 ELSE cost_known END,
                        output_tokens = CASE WHEN ?4 THEN ?7 ELSE output_tokens END,
                        steps = CASE WHEN ?4 THEN ?8 ELSE steps END,
                        summary = CASE WHEN summary = '' THEN ?9 ELSE summary END
                  WHERE id = ?1 AND {OPEN_NOT_ENDED}"
            ),
            params![
                id,
                infra_status,
                now_ms,
                cost.is_some(),
                c.cost_usd,
                c.cost_known as i64,
                c.output_tokens,
                c.steps,
                clip_summary(summary),
                state::FINISHED
            ],
        )?;
        Ok(n > 0)
    }

    /// The run's own verdict (§3.2): its outcome, a summary, links merged
    /// into `links.reported`. Refused for a run that never ran. A report
    /// that changes the outcome of a run that has ended moves the incident
    /// keeper's cursor back to just before it, in the same transaction, so
    /// the run is judged again (`job_incidents`).
    pub fn job_run_report(
        &self,
        id: &str,
        outcome: &str,
        summary: &str,
        links: &serde_json::Value,
    ) -> Result<bool, StoreError> {
        let mut conn = self.conn().lock().unwrap();
        let tx = conn.transaction()?;
        let current: Option<(String, String, String, Option<i64>)> = tx
            .query_row(
                "SELECT links, outcome, job_id, ended_at_ms FROM db_job_runs WHERE id = ?1 AND state IN ('running','finished')",
                params![id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?)),
            )
            .optional()?;
        let Some((current, was, job_id, ended)) = current else {
            return Ok(false);
        };
        let mut merged: serde_json::Value = serde_json::from_str(&current).unwrap_or_else(|_| serde_json::json!({}));
        if !merged.is_object() {
            merged = serde_json::json!({});
        }
        if !links.is_null() {
            merged["reported"] = links.clone();
        }
        tx.execute(
            "UPDATE db_job_runs SET outcome = ?2, summary = ?3, links = ?4 WHERE id = ?1",
            params![id, outcome, clip_summary(summary), merged.to_string()],
        )?;
        if let (true, Some(ended)) = (was != outcome, ended) {
            tx.execute(
                "UPDATE db_job_incident_cursors SET ended_at_ms = ?2, run_id = ''
                  WHERE job_id = ?1 AND (ended_at_ms > ?2 OR (ended_at_ms = ?2 AND run_id >= ?3))",
                params![job_id, ended, id],
            )?;
        }
        tx.commit()?;
        Ok(true)
    }

    /// Merge `extra`'s keys into the run's `links`.
    pub fn job_run_add_links(&self, id: &str, extra: &serde_json::Value) -> Result<(), StoreError> {
        let mut conn = self.conn().lock().unwrap();
        let tx = conn.transaction()?;
        let current: Option<String> =
            tx.query_row("SELECT links FROM db_job_runs WHERE id = ?1", params![id], |r| r.get(0)).optional()?;
        let Some(current) = current else {
            return Ok(());
        };
        let mut merged: serde_json::Value = serde_json::from_str(&current).unwrap_or_else(|_| serde_json::json!({}));
        if let (Some(m), Some(e)) = (merged.as_object_mut(), extra.as_object()) {
            for (k, v) in e {
                m.insert(k.clone(), v.clone());
            }
        }
        tx.execute("UPDATE db_job_runs SET links = ?2 WHERE id = ?1", params![id, merged.to_string()])?;
        tx.commit()?;
        Ok(())
    }

    /// Every open run of `job_source` this channel wrote (`recorded_by`
    /// `<channel>@…`), in `states`.
    pub fn job_runs_open_recorded_by(
        &self,
        channel: &str,
        job_source: &str,
        states: &[&str],
    ) -> Result<Vec<JobRun>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let list = states.iter().map(|s| format!("'{}'", s.replace('\'', ""))).collect::<Vec<_>>().join(",");
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLS} FROM db_job_runs
              WHERE job_source = ?1 AND state IN ({list})
                AND (recorded_by = ?2 OR substr(recorded_by, 1, length(?2) + 1) = ?2 || '@')
              ORDER BY started_at_ms"
        ))?;
        let rows = stmt.query_map(params![job_source, channel], row_to_run)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Every open run `channel` follows for the channel that fired it
    /// (`links.followed_by_channel`, #1988).
    pub fn job_runs_open_followed_by(&self, channel: &str) -> Result<Vec<JobRun>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLS} FROM db_job_runs
              WHERE {OPEN_NOT_ENDED}
                AND CASE WHEN json_valid(links) THEN json_extract(links, '$.followed_by_channel') END = ?1
              ORDER BY started_at_ms"
        ))?;
        let rows = stmt.query_map(params![channel], row_to_run)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Record `skip`, a `skipped` row for one slot (`scheduled_for_ms` set,
    /// `skip_count` 1). When the job's newest row is a skip for the same
    /// reason, the slot is counted into that row (`skip_count` + 1,
    /// `last_skip_at_ms`) and `skip` is kept hidden under it
    /// (`collapsed_into`), so consecutive skips show as one row while every
    /// slot keeps its own row in the unique `(job_id, scheduled_for_ms)`
    /// index: the slot stays leased, and another scheduler sharing the job
    /// can't fire it later (after Resume all, say). A slot already taken (by
    /// another instance's count, or a fire) counts nothing.
    pub fn job_run_record_skip_slot(&self, skip: &JobRun) -> Result<(), StoreError> {
        let mut conn = self.conn().lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let newest: Option<(String, String, String)> = tx
            .query_row(
                "SELECT id, state, reason FROM db_job_runs
                  WHERE job_id = ?1 AND collapsed_into = ''
                  ORDER BY started_at_ms DESC, rowid DESC LIMIT 1",
                params![skip.job_id],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )
            .optional()?;
        let fold_into = newest.filter(|(_, st, why)| st == state::SKIPPED && *why == skip.reason).map(|(id, _, _)| id);
        let inserted = tx.execute(
            &format!(
                "INSERT OR IGNORE INTO db_job_runs ({COLS}, collapsed_into) VALUES \
                 (?1,?2,?3,?4,?5,?6,?7,?8,?9,?10,?11,?12,?13,?14,?15,?16,?17,?18,?19,?20,?21,?22,?23,?24,?25,?26,?27,?28,?29,?30,?31,?32,?33)"
            ),
            params![
                skip.id,
                skip.job_id,
                skip.job_source,
                skip.job_name,
                skip.template_id,
                skip.template_version,
                skip.trigger,
                skip.scheduled_for_ms,
                skip.triggered_by,
                skip.attempt,
                skip.state,
                skip.reason,
                skip.infra,
                skip.outcome,
                skip.summary,
                skip.target_agent_uid,
                skip.block_id,
                skip.turn_id,
                skip.account_id,
                skip.provider,
                skip.started_at_ms,
                skip.ended_at_ms,
                skip.cost_usd,
                skip.cost_known as i64,
                skip.cost_partial as i64,
                skip.output_tokens,
                skip.steps,
                skip.links.to_string(),
                if fold_into.is_some() { 0 } else { skip.skip_count },
                skip.last_skip_at_ms,
                skip.reporter,
                skip.recorded_by,
                fold_into.as_deref().unwrap_or(""),
            ],
        )?;
        if inserted > 0 {
            if let Some(into) = &fold_into {
                tx.execute(
                    "UPDATE db_job_runs SET skip_count = skip_count + 1,
                            last_skip_at_ms = MAX(COALESCE(last_skip_at_ms, 0), ?2)
                      WHERE id = ?1",
                    params![into, skip.scheduled_for_ms],
                )?;
            }
        }
        tx.commit()?;
        Ok(())
    }

    /// The job's spend since `since_ms`: the summed `cost_usd` of its runs
    /// started then or later, and whether any of them reported a cost.
    pub fn job_runs_cost_since(&self, job_id: &str, since_ms: i64) -> Result<(f64, bool), StoreError> {
        let conn = self.conn().lock().unwrap();
        Ok(conn.query_row(
            "SELECT COALESCE(SUM(cost_usd), 0), COALESCE(MAX(cost_known), 0) FROM db_job_runs
              WHERE job_id = ?1 AND collapsed_into = '' AND started_at_ms >= ?2",
            params![job_id, since_ms],
            |r| Ok((r.get::<_, f64>(0)?, r.get::<_, i64>(1)? != 0)),
        )?)
    }

    /// A switch every job honours (`db_job_flags`), if it is set.
    pub fn job_flag_get(&self, key: &str) -> Result<Option<JobFlag>, StoreError> {
        let conn = self.conn().lock().unwrap();
        Ok(conn
            .query_row(
                "SELECT value, set_at_ms, set_by FROM db_job_flags WHERE key = ?1",
                params![key],
                |r| Ok(JobFlag { value: r.get(0)?, set_at_ms: r.get(1)?, set_by: r.get(2)? }),
            )
            .optional()?)
    }

    /// Set the switch `key`, by `set_by`, now. Setting it again keeps the
    /// first time it was set.
    pub fn job_flag_set(&self, key: &str, value: &str, set_by: &str, now_ms: i64) -> Result<(), StoreError> {
        let conn = self.conn().lock().unwrap();
        conn.execute(
            "INSERT INTO db_job_flags (key, value, set_at_ms, set_by) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            params![key, value, now_ms, set_by],
        )?;
        Ok(())
    }

    /// Clear the switch `key`. `false` when it wasn't set.
    pub fn job_flag_clear(&self, key: &str) -> Result<bool, StoreError> {
        let conn = self.conn().lock().unwrap();
        Ok(conn.execute("DELETE FROM db_job_flags WHERE key = ?1", params![key])? > 0)
    }
}

/// A switch in `db_job_flags`: its value, when it was set and by whom.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct JobFlag {
    pub value: String,
    #[ts(type = "number")]
    pub set_at_ms: i64,
    pub set_by: String,
}

#[cfg(test)]
#[path = "job_runs_tests.rs"]
mod tests;
