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
    CREATE INDEX IF NOT EXISTS idx_ids_job_runs_state ON db_job_runs(state);";

/// Columns added to `db_job_runs` after its first shape, run on every open
/// (`apply_schema`) as `cron::CRON_JOBS_COLUMNS` is: a table an earlier
/// build created gains them, and a new column with a default needs no
/// schema version bump.
const JOB_RUNS_COLUMNS: [&str; 1] = ["ALTER TABLE db_job_runs ADD COLUMN collapsed_into TEXT NOT NULL DEFAULT ''"];

/// Create the table, then add any column it lacks; "duplicate column" is
/// the column already being there.
pub(crate) fn apply_schema(conn: &rusqlite::Connection) -> Result<(), StoreError> {
    conn.execute_batch(SCHEMA)?;
    for stmt in JOB_RUNS_COLUMNS {
        if let Err(e) = conn.execute_batch(stmt) {
            if !e.to_string().contains("duplicate column") {
                return Err(e.into());
            }
        }
    }
    Ok(())
}

const COLS: &str = "id, job_id, job_source, job_name, template_id, template_version, trigger_kind, \
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
    /// The states a run can still move on from, while `ended_at_ms` is
    /// unset: `OPEN_NOT_ENDED`'s list, and [`super::JobRun::is_open`].
    pub const OPEN: [&str; 3] = [SCHEDULED, PENDING, RUNNING];
}

/// Why a run was skipped, missed or stopped, or how a pending one ended
/// (§3.1; the ones P1 writes).
pub mod reason {
    pub const APP_NOT_RUNNING: &str = "app_not_running";
    pub const TARGET_MISSING: &str = "target_missing";
    pub const TARGET_STOPPED: &str = "target_stopped";
    /// A newer pending fire replaced this one.
    pub const SUPERSEDED: &str = "superseded";
    /// The job's next fire time passed before its agent came back.
    pub const EXPIRED: &str = "expired";
}

/// Infrastructure status (§3.1; the ones P1 writes).
pub mod infra {
    pub const OK: &str = "ok";
    pub const ERROR: &str = "error";
    pub const INTERRUPTED: &str = "interrupted";
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
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
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
    pub scheduled_for_ms: Option<i64>,
    pub triggered_by: String,
    pub attempt: i64,
    pub state: String,
    pub reason: String,
    pub infra: String,
    pub outcome: String,
    pub summary: String,
    pub target_agent_uid: String,
    pub block_id: String,
    pub turn_id: Option<i64>,
    pub account_id: String,
    pub provider: String,
    pub started_at_ms: i64,
    pub ended_at_ms: Option<i64>,
    pub cost_usd: f64,
    pub cost_known: bool,
    pub cost_partial: bool,
    pub output_tokens: i64,
    pub steps: i64,
    /// JSON object: the transcript, a Drone run, reported links.
    pub links: serde_json::Value,
    pub skip_count: i64,
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

fn row_to_run(row: &Row) -> rusqlite::Result<JobRun> {
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
                    SET state = ?2, reason = '', block_id = ?3, account_id = ?4, provider = ?5,
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
    /// into `links.reported`. Refused for a run that never ran.
    pub fn job_run_report(
        &self,
        id: &str,
        outcome: &str,
        summary: &str,
        links: &serde_json::Value,
    ) -> Result<bool, StoreError> {
        let mut conn = self.conn().lock().unwrap();
        let tx = conn.transaction()?;
        let current: Option<String> = tx
            .query_row(
                "SELECT links FROM db_job_runs WHERE id = ?1 AND state IN ('running','finished')",
                params![id],
                |r| r.get(0),
            )
            .optional()?;
        let Some(current) = current else {
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
}

#[cfg(test)]
#[path = "job_runs_tests.rs"]
mod tests;
