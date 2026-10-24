// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Job incidents: a job's consecutive failures, from the first to the next
//! success, and the one Kanban task that stands for them
//! (docs/specs/SPEC_JOBS_AND_TEMPLATES_2026_10_24.md §2, §3.4).
//!
//! The work queue has no dedupe key, so the job keeps its incident's task
//! here, beside the run ledger in the identity store. At most one incident
//! per job is open (`ended_at_ms` unset): a partial unique index. An
//! incident whose task was closed while the job still failed has ended, but
//! is not recovered yet (`recovered_at_ms` unset), and still remembers its
//! notification is up (`notified`), so the next success takes it down.
//!
//! `db_job_incident_cursors` is how far the keeper has walked each job's
//! decisive runs (`jobs::incidents`); it moves on only after a run was
//! applied, and a report that changes a run's outcome moves it back
//! (`job_run_report`). `db_job_incident_runs` holds every failure counted
//! into an incident, written in the same transaction as the count, so a run
//! walked again is never counted twice.

use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use super::store::Store;
use super::StoreError;

/// The tables, created with the run ledger's (`job_runs::apply_schema`) on
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
        ended_by_run      TEXT NOT NULL DEFAULT '',
        recovered_at_ms   INTEGER,
        notified          INTEGER NOT NULL DEFAULT 0
    );
    CREATE UNIQUE INDEX IF NOT EXISTS idx_ids_job_incidents_open ON db_job_incidents(job_id)
        WHERE ended_at_ms IS NULL;
    CREATE INDEX IF NOT EXISTS idx_ids_job_incidents_job ON db_job_incidents(job_id, opened_at_ms);
    CREATE TABLE IF NOT EXISTS db_job_incident_cursors (
        job_id      TEXT PRIMARY KEY,
        ended_at_ms INTEGER NOT NULL,
        run_id      TEXT NOT NULL
    );
    CREATE TABLE IF NOT EXISTS db_job_incident_runs (
        run_id        TEXT PRIMARY KEY,
        incident_id   TEXT NOT NULL,
        counted_at_ms INTEGER NOT NULL
    );";

/// How an incident ended (`end_reason`).
pub mod ended {
    /// The job's next decisive run succeeded (§3.4). Its task stays open for
    /// its owner (§10, decision 6).
    pub const RECOVERED: &str = "recovered";
    /// Its task was finished or cancelled while the job still failed: the
    /// next failure opens a new incident, and the next success still
    /// recovers this one (its notification).
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
    /// When the job succeeded again; unset while it still fails, even after
    /// its task was closed.
    #[ts(type = "number | null")]
    pub recovered_at_ms: Option<i64>,
    /// Its notification is up: raised when it opened, taken down when it
    /// recovers.
    pub notified: bool,
}

const COLS: &str = "id, job_id, job_source, job_name, work_id, task_num, owner_uid, first_run_id, last_run_id, \
                    failures, opened_at_ms, last_failed_at_ms, ended_at_ms, end_reason, ended_by_run, \
                    recovered_at_ms, notified";

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
        recovered_at_ms: row.get("recovered_at_ms")?,
        notified: row.get::<_, i64>("notified")? != 0,
    })
}

/// Whether run `run_id` was counted into an incident.
fn counted(conn: &rusqlite::Connection, run_id: &str) -> Result<bool, StoreError> {
    Ok(conn
        .query_row("SELECT 1 FROM db_job_incident_runs WHERE run_id = ?1", params![run_id], |_| Ok(()))
        .optional()?
        .is_some())
}

/// Record run `run_id` as counted into incident `incident_id`.
fn count_run(conn: &rusqlite::Connection, run_id: &str, incident_id: &str, now_ms: i64) -> Result<(), StoreError> {
    conn.execute(
        "INSERT OR REPLACE INTO db_job_incident_runs (run_id, incident_id, counted_at_ms) VALUES (?1, ?2, ?3)",
        params![run_id, incident_id, now_ms],
    )?;
    Ok(())
}

/// How far the keeper has walked a job's decisive runs: the last one it
/// took, by when it ended and its id.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IncidentCursor {
    pub ended_at_ms: i64,
    pub run_id: String,
}

impl Store {
    /// Open `incident` (its first failure, `first_run_id`) unless the job
    /// already has an open one, or that run was counted already. `false`
    /// when it wasn't opened.
    pub fn job_incident_open(&self, incident: &JobIncident) -> Result<bool, StoreError> {
        let mut conn = self.conn().lock().unwrap();
        let tx = conn.transaction()?;
        if counted(&tx, &incident.first_run_id)? {
            return Ok(false);
        }
        let n = tx.execute(
            &format!(
                "INSERT OR IGNORE INTO db_job_incidents ({COLS}) VALUES
                 (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, NULL, '', '', NULL, ?13)"
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
                incident.notified as i64,
            ],
        )?;
        if n > 0 {
            count_run(&tx, &incident.first_run_id, &incident.id, incident.opened_at_ms)?;
        }
        tx.commit()?;
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
    #[cfg(test)]
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
    /// last counted run is still `seen_run_id` and `run_id` wasn't counted
    /// before. Returns the new count, or `None` when it wasn't counted.
    pub fn job_incident_fail_again(
        &self,
        id: &str,
        seen_run_id: &str,
        run_id: &str,
        now_ms: i64,
    ) -> Result<Option<i64>, StoreError> {
        let mut conn = self.conn().lock().unwrap();
        let tx = conn.transaction()?;
        if counted(&tx, run_id)? {
            return Ok(None);
        }
        let count: Option<i64> = tx
            .query_row(
                "UPDATE db_job_incidents
                    SET failures = failures + 1, last_run_id = ?3, last_failed_at_ms = ?4
                  WHERE id = ?1 AND last_run_id = ?2 AND ended_at_ms IS NULL
                  RETURNING failures",
                params![id, seen_run_id, run_id, now_ms],
                |r| r.get(0),
            )
            .optional()?;
        if count.is_some() {
            count_run(&tx, run_id, id, now_ms)?;
        }
        tx.commit()?;
        Ok(count)
    }

    /// Whether run `run_id` was counted into an incident as a failure.
    pub fn job_incident_run_counted(&self, run_id: &str) -> Result<bool, StoreError> {
        let conn = self.conn().lock().unwrap();
        counted(&conn, run_id)
    }

    /// The incident run `run_id` recovered while it was open, if any: a run
    /// judged a success that a later report says failed.
    pub fn job_incident_recovered_by(&self, job_id: &str, run_id: &str) -> Result<Option<JobIncident>, StoreError> {
        let conn = self.conn().lock().unwrap();
        Ok(conn
            .query_row(
                &format!(
                    "SELECT {COLS} FROM db_job_incidents
                      WHERE job_id = ?1 AND ended_by_run = ?2 AND end_reason = ?3 AND recovered_at_ms IS NOT NULL"
                ),
                params![job_id, run_id, ended::RECOVERED],
                row_to_incident,
            )
            .optional()?)
    }

    /// Reopen incident `id`, recovered by run `run_id`, which turned out to
    /// have failed: open again, its notification up, and `run_id` counted.
    /// `None` when it can't be (the job has another open incident, or the
    /// run was counted already); else the new count.
    pub fn job_incident_reopen(&self, id: &str, run_id: &str, now_ms: i64) -> Result<Option<i64>, StoreError> {
        let mut conn = self.conn().lock().unwrap();
        let tx = conn.transaction()?;
        if counted(&tx, run_id)? {
            return Ok(None);
        }
        let count: Option<i64> = tx
            .query_row(
                "UPDATE OR IGNORE db_job_incidents
                    SET ended_at_ms = NULL, end_reason = '', ended_by_run = '', recovered_at_ms = NULL, notified = 1,
                        failures = failures + 1, last_run_id = ?2, last_failed_at_ms = ?3
                  WHERE id = ?1 AND ended_by_run = ?2 AND recovered_at_ms IS NOT NULL
                  RETURNING failures",
                params![id, run_id, now_ms],
                |r| r.get(0),
            )
            .optional()?;
        if count.is_some() {
            count_run(&tx, run_id, id, now_ms)?;
        }
        tx.commit()?;
        Ok(count)
    }

    /// End open incident `id` because its task was closed while the job
    /// still failed. It stays unrecovered. `false` when it had already ended.
    pub fn job_incident_end_task_closed(&self, id: &str, now_ms: i64) -> Result<bool, StoreError> {
        let conn = self.conn().lock().unwrap();
        let n = conn.execute(
            "UPDATE db_job_incidents SET ended_at_ms = ?2, end_reason = ?3
              WHERE id = ?1 AND ended_at_ms IS NULL",
            params![id, now_ms, ended::TASK_CLOSED],
        )?;
        Ok(n > 0)
    }

    /// Run `run_id`, a success, recovers every incident of the job not yet
    /// recovered: the open one ends (`recovered`), one whose task was closed
    /// keeps its end, and each notification is marked down. Returns them as
    /// they were before.
    pub fn job_incidents_recover(&self, job_id: &str, run_id: &str, now_ms: i64) -> Result<Vec<JobIncident>, StoreError> {
        let mut conn = self.conn().lock().unwrap();
        let tx = conn.transaction()?;
        let before = {
            let mut stmt = tx.prepare(&format!(
                "SELECT {COLS} FROM db_job_incidents WHERE job_id = ?1 AND recovered_at_ms IS NULL ORDER BY opened_at_ms"
            ))?;
            let rows = stmt.query_map(params![job_id], row_to_incident)?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        tx.execute(
            "UPDATE db_job_incidents
                SET recovered_at_ms = ?3, ended_by_run = ?2, notified = 0,
                    end_reason = CASE WHEN ended_at_ms IS NULL THEN ?4 ELSE end_reason END,
                    ended_at_ms = COALESCE(ended_at_ms, ?3)
              WHERE job_id = ?1 AND recovered_at_ms IS NULL",
            params![job_id, run_id, now_ms, ended::RECOVERED],
        )?;
        // A run counted as a failure that a report turned into a success.
        tx.execute("DELETE FROM db_job_incident_runs WHERE run_id = ?1", params![run_id])?;
        tx.commit()?;
        Ok(before)
    }

    /// How far the keeper has walked job `job_id`'s decisive runs.
    pub fn job_incident_cursor(&self, job_id: &str) -> Result<Option<IncidentCursor>, StoreError> {
        let conn = self.conn().lock().unwrap();
        Ok(conn
            .query_row(
                "SELECT ended_at_ms, run_id FROM db_job_incident_cursors WHERE job_id = ?1",
                params![job_id],
                |r| Ok(IncidentCursor { ended_at_ms: r.get(0)?, run_id: r.get(1)? }),
            )
            .optional()?)
    }

    /// Move job `job_id`'s cursor past an applied run: from `from` (`None`:
    /// it has none yet) to `to`. `false` when it isn't at `from` any more:
    /// another keeper moved it, or a report moved it back.
    pub fn job_incident_cursor_advance(
        &self,
        job_id: &str,
        from: Option<&IncidentCursor>,
        to: &IncidentCursor,
    ) -> Result<bool, StoreError> {
        let conn = self.conn().lock().unwrap();
        let n = match from {
            None => conn.execute(
                "INSERT OR IGNORE INTO db_job_incident_cursors (job_id, ended_at_ms, run_id) VALUES (?1, ?2, ?3)",
                params![job_id, to.ended_at_ms, to.run_id],
            )?,
            Some(from) => conn.execute(
                "UPDATE db_job_incident_cursors SET ended_at_ms = ?2, run_id = ?3
                  WHERE job_id = ?1 AND ended_at_ms = ?4 AND run_id = ?5",
                params![job_id, to.ended_at_ms, to.run_id, from.ended_at_ms, from.run_id],
            )?,
        };
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
            notified: true,
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
        assert!(store.job_incident_end_task_closed("i1", 9).unwrap());
        assert!(!store.job_incident_end_task_closed("i1", 9).unwrap());
        assert_eq!(store.job_incident_fail_again("i1", "r2", "r4", 10).unwrap(), None, "ended");
        assert!(store.job_incident_open_for("j").unwrap().is_none());
        assert!(store.job_incident_open(&incident("i3", "j", "r4")).unwrap(), "a new one after the end");

        let recovered = store.job_incidents_recover("j", "r5", 20).unwrap();
        assert_eq!(recovered.iter().map(|i| i.id.as_str()).collect::<Vec<_>>(), ["i1", "i3"], "the closed one too");
        let i3 = store.job_incident_latest("j").unwrap().unwrap();
        assert_eq!((i3.end_reason.as_str(), i3.recovered_at_ms, i3.notified), (ended::RECOVERED, Some(20), false));
        assert!(store.job_incidents_recover("j", "r6", 30).unwrap().is_empty());
    }

    #[test]
    fn the_cursor_moves_only_from_where_it_was() {
        let store = Store::open_in_memory().unwrap();
        store.job_runs_ensure_schema().unwrap();
        let at = |ms: i64, id: &str| IncidentCursor { ended_at_ms: ms, run_id: id.into() };
        assert!(store.job_incident_cursor_advance("j", None, &at(1, "a")).unwrap());
        assert!(!store.job_incident_cursor_advance("j", None, &at(1, "a")).unwrap(), "taken");
        assert!(store.job_incident_cursor_advance("j", Some(&at(1, "a")), &at(2, "b")).unwrap());
        assert!(!store.job_incident_cursor_advance("j", Some(&at(1, "a")), &at(3, "c")).unwrap(), "moved on");
        assert_eq!(store.job_incident_cursor("j").unwrap(), Some(at(2, "b")));
    }
}
