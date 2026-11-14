// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Which events a job has heard of (docs/specs/SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md §3.7): delivery is at least
//! once, so a source may name the same event twice, and the job's `(job_id, event_id)` here is what turns the second
//! into nothing. It sits beside the run ledger, in the identity store every channel and version on this computer
//! shares, so two instances sharing a job don't both start a run for one event.
//!
//! Only the id is kept, never the event's data. A job forgets an id after [`KEEP_MS`]: a source that repeats an id
//! after that is a new event.

use rusqlite::params;

use super::store::Store;
use super::StoreError;

/// The table, created on every open of the identity store, with no schema version bump.
pub(crate) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS db_job_trigger_events (
        job_id    TEXT NOT NULL,
        event_id  TEXT NOT NULL,
        run_id    TEXT NOT NULL DEFAULT '',
        at_ms     INTEGER NOT NULL,
        PRIMARY KEY (job_id, event_id)
    );
    CREATE INDEX IF NOT EXISTS idx_ids_job_trigger_events_at ON db_job_trigger_events(at_ms);";

/// How long a job remembers an event id.
pub const KEEP_MS: i64 = 7 * 24 * 3600 * 1000;

impl Store {
    /// Record that `job_id` heard of `event_id` now, for run `run_id`. `Ok(true)` the first time, `Ok(false)` when it
    /// already had: another delivery of the same event, here or in another instance.
    pub fn job_event_claim(&self, job_id: &str, event_id: &str, run_id: &str, now_ms: i64) -> Result<bool, StoreError> {
        let conn = self.conn().lock().unwrap();
        let n = conn.execute(
            "INSERT OR IGNORE INTO db_job_trigger_events (job_id, event_id, run_id, at_ms) VALUES (?1, ?2, ?3, ?4)",
            params![job_id, event_id, run_id, now_ms],
        )?;
        Ok(n > 0)
    }

    /// Forget the ids heard before `before_ms`. How many went.
    pub fn job_events_forget_before(&self, before_ms: i64) -> Result<usize, StoreError> {
        let conn = self.conn().lock().unwrap();
        Ok(conn.execute("DELETE FROM db_job_trigger_events WHERE at_ms < ?1", params![before_ms])?)
    }

    /// How many events `job_id` has heard of since `since_ms`.
    pub fn job_events_since(&self, job_id: &str, since_ms: i64) -> Result<i64, StoreError> {
        let conn = self.conn().lock().unwrap();
        Ok(conn.query_row("SELECT COUNT(*) FROM db_job_trigger_events WHERE job_id = ?1 AND at_ms >= ?2", params![job_id, since_ms], |r| r.get(0))?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        let store = Store::open_in_memory().unwrap();
        store.conn().lock().unwrap().execute_batch(SCHEMA).unwrap();
        store
    }

    #[test]
    fn an_event_id_is_new_once_per_job() {
        let s = store();
        assert!(s.job_event_claim("job-a", "e1", "run-1", 100).unwrap());
        assert!(!s.job_event_claim("job-a", "e1", "run-2", 200).unwrap(), "the same event again is nothing");
        assert!(s.job_event_claim("job-a", "e2", "run-3", 300).unwrap());
        assert!(s.job_event_claim("job-b", "e1", "run-4", 400).unwrap(), "another job hears the same id as new");
    }

    #[test]
    fn a_job_forgets_old_ids_and_counts_recent_ones() {
        let s = store();
        for (i, at) in [100, 200, 300].into_iter().enumerate() {
            assert!(s.job_event_claim("job-a", &format!("e{i}"), "r", at).unwrap());
        }
        assert_eq!(s.job_events_since("job-a", 200).unwrap(), 2);
        assert_eq!(s.job_events_forget_before(250).unwrap(), 2);
        assert_eq!(s.job_events_since("job-a", 0).unwrap(), 1);
        assert!(s.job_event_claim("job-a", "e0", "r", 400).unwrap(), "a forgotten id is a new event");
    }
}
