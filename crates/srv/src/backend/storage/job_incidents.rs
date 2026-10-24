// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Job incidents: a job's consecutive failures, from the first to the next
//! success, and the one Kanban task that stands for them
//! (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §2, §3.4).
//!
//! The work queue has no dedupe key, so the job keeps its incident's task
//! here, beside the run ledger in the identity store. At most one incident
//! per job is open (`ended_at_ms` unset): a partial unique index, so two
//! instances sharing the ledger can't both open one. Every change is a
//! conditional UPDATE on what the caller last read, so a failure is counted,
//! and a recovery noted, once.

use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use super::store::Store;
use super::StoreError;

/// The table, created with the run ledger's (`job_runs::apply_schema`) on
/// every open, with no schema version bump.
pub(crate) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS db_job_incidents (
        id                TEXT PRIMARY KEY,
        job_id            TEXT NOT NULL,
        job_source        TEXT NOT NULL DEFAULT '',
        job_name          TEXT NOT NULL DEFAULT '',
        work_id           TEXT NOT NULL DEFAULT '',
        task_num          INTEGER,
        owner_uid         TEXT NOT NULL DEFAULT '',
        first_run_id      TEXT NOT NULL,
        last_run_id       TEXT NOT NULL,
        failures          INTEGER NOT NULL DEFAULT 1,
        opened_at_ms      INTEGER NOT NULL,
        last_failed_at_ms INTEGER NOT NULL,
        ended_at_ms       INTEGER,
        end_reason        TEXT NOT NULL DEFAULT '',
        ended_by_run      TEXT NOT NULL DEFAULT ''
    );
    CREATE UNIQUE INDEX IF NOT EXISTS idx_ids_job_incidents_open ON db_job_incidents(job_id)
        WHERE ended_at_ms IS NULL;
    CREATE INDEX IF NOT EXISTS idx_ids_job_incidents_job ON db_job_incidents(job_id, opened_at_ms);";

/// How an incident ended (`end_reason`).
pub mod ended {
    /// The job's next decisive run succeeded (§3.4). Its task stays open for
    /// its owner (§10, decision 6).
    pub const RECOVERED: &str = "recovered";
    /// Its task was finished or cancelled while the job still failed: the
    /// next failure opens a new incident.
    pub const TASK_CLOSED: &str = "task_closed";
}

/// One row of `db_job_incidents`.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct JobIncident {
    pub id: String,
    pub job_id: String,
    pub job_source: String,
    pub job_name: String,
    /// The Kanban task (`db_work_queue.id`); empty until it is filed.
    pub work_id: String,
    /// Its short number, `#num`.
    #[ts(type = "number | null")]
    pub task_num: Option<i64>,
    /// The owning leader the task went to; empty when none resolved.
    pub owner_uid: String,
    pub first_run_id: String,
    pub last_run_id: String,
    /// Failures in a row, this incident's first included.
    #[ts(type = "number")]
    pub failures: i64,
    #[ts(type = "number")]
    pub opened_at_ms: i64,
    #[ts(type = "number")]
    pub last_failed_at_ms: i64,
    #[ts(type = "number | null")]
    pub ended_at_ms: Option<i64>,
    /// One of [`ended`], once it ended.
    pub end_reason: String,
    /// The run that recovered it.
    pub ended_by_run: String,
}

const COLS: &str = "id, job_id, job_source, job_name, work_id, task_num, owner_uid, first_run_id, last_run_id, \
                    failures, opened_at_ms, last_failed_at_ms, ended_at_ms, end_reason, ended_by_run";

fn row_to_incident(row: &Row) -> rusqlite::Result<JobIncident> {
    Ok(JobIncident {
        id: row.get("id")?,
        job_id: row.get("job_id")?,
        job_source: row.get("job_source")?,
        job_name: row.get("job_name")?,
        work_id: row.get("work_id")?,
        task_num: row.get("task_num")?,
        owner_uid: row.get("owner_uid")?,
        first_run_id: row.get("first_run_id")?,
        last_run_id: row.get("last_run_id")?,
        failures: row.get("failures")?,
        opened_at_ms: row.get("opened_at_ms")?,
        last_failed_at_ms: row.get("last_failed_at_ms")?,
        ended_at_ms: row.get("ended_at_ms")?,
        end_reason: row.get("end_reason")?,
        ended_by_run: row.get("ended_by_run")?,
    })
}

impl Store {
    /// Open `incident` (its first failure) unless the job already has an open
    /// one. `false` when it has: the caller counts the failure into that one.
    pub fn job_incident_open(&self, incident: &JobIncident) -> Result<bool, StoreError> {
        let conn = self.conn().lock().unwrap();
        let n = conn.execute(
            &format!(
                "INSERT OR IGNORE INTO db_job_incidents ({COLS}) VALUES
                 (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, NULL, '', '')"
            ),
            params![
                incident.id,
                incident.job_id,
                incident.job_source,
                incident.job_name,
                incident.work_id,
                incident.task_num,
                incident.owner_uid,
                incident.first_run_id,
                incident.last_run_id,
                incident.failures,
                incident.opened_at_ms,
                incident.last_failed_at_ms,
            ],
        )?;
        Ok(n > 0)
    }

    /// The job's open incident, if any.
    pub fn job_incident_open_for(&self, job_id: &str) -> Result<Option<JobIncident>, StoreError> {
        let conn = self.conn().lock().unwrap();
        Ok(conn
            .query_row(
                &format!("SELECT {COLS} FROM db_job_incidents WHERE job_id = ?1 AND ended_at_ms IS NULL"),
                params![job_id],
                row_to_incident,
            )
            .optional()?)
    }

    /// The job's newest incident, open or ended.
    pub fn job_incident_latest(&self, job_id: &str) -> Result<Option<JobIncident>, StoreError> {
        let conn = self.conn().lock().unwrap();
        Ok(conn
            .query_row(
                &format!(
                    "SELECT {COLS} FROM db_job_incidents WHERE job_id = ?1
                      ORDER BY opened_at_ms DESC, rowid DESC LIMIT 1"
                ),
                params![job_id],
                row_to_incident,
            )
            .optional()?)
    }

    /// The task filed for incident `id`.
    pub fn job_incident_set_task(&self, id: &str, work_id: &str, task_num: i64) -> Result<(), StoreError> {
        let conn = self.conn().lock().unwrap();
        conn.execute(
            "UPDATE db_job_incidents SET work_id = ?2, task_num = ?3 WHERE id = ?1",
            params![id, work_id, task_num],
        )?;
        Ok(())
    }

    /// Count run `run_id` into open incident `id` as another failure, if its
    /// last counted run is still `seen_run_id`. Returns the new count, or
    /// `None` when someone else counted first or it ended.
    pub fn job_incident_fail_again(
        &self,
        id: &str,
        seen_run_id: &str,
        run_id: &str,
        now_ms: i64,
    ) -> Result<Option<i64>, StoreError> {
        let conn = self.conn().lock().unwrap();
        Ok(conn
            .query_row(
                "UPDATE db_job_incidents
                    SET failures = failures + 1, last_run_id = ?3, last_failed_at_ms = ?4
                  WHERE id = ?1 AND last_run_id = ?2 AND ended_at_ms IS NULL
                  RETURNING failures",
                params![id, seen_run_id, run_id, now_ms],
                |r| r.get(0),
            )
            .optional()?)
    }

    /// End open incident `id` (`why`, one of [`ended`]; `by_run` the run that
    /// recovered it). `false` when it had already ended.
    pub fn job_incident_end(&self, id: &str, why: &str, by_run: &str, now_ms: i64) -> Result<bool, StoreError> {
        let conn = self.conn().lock().unwrap();
        let n = conn.execute(
            "UPDATE db_job_incidents SET ended_at_ms = ?2, end_reason = ?3, ended_by_run = ?4
              WHERE id = ?1 AND ended_at_ms IS NULL",
            params![id, now_ms, why, by_run],
        )?;
        Ok(n > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn incident(id: &str, job: &str, run: &str) -> JobIncident {
        JobIncident {
            id: id.into(),
            job_id: job.into(),
            first_run_id: run.into(),
            last_run_id: run.into(),
            failures: 1,
            ..Default::default()
        }
    }

    #[test]
    fn a_job_has_at_most_one_open_incident_and_each_failure_counts_once() {
        let store = Store::open_in_memory().unwrap();
        store.job_runs_ensure_schema().unwrap();
        assert!(store.job_incident_open(&incident("i1", "j", "r1")).unwrap());
        assert!(!store.job_incident_open(&incident("i2", "j", "r2")).unwrap(), "j already has one open");
        assert_eq!(store.job_incident_fail_again("i1", "r1", "r2", 5).unwrap(), Some(2));
        assert_eq!(store.job_incident_fail_again("i1", "r1", "r2", 5).unwrap(), None, "counted already");
        assert!(store.job_incident_end("i1", ended::RECOVERED, "r3", 9).unwrap());
        assert!(!store.job_incident_end("i1", ended::RECOVERED, "r3", 9).unwrap());
        assert_eq!(store.job_incident_fail_again("i1", "r2", "r4", 10).unwrap(), None, "ended");
        assert!(store.job_incident_open_for("j").unwrap().is_none());
        assert!(store.job_incident_open(&incident("i3", "j", "r4")).unwrap(), "a new one after the end");
        assert_eq!(store.job_incident_latest("j").unwrap().unwrap().id, "i3");
    }
}
