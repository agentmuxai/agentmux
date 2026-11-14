// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Which events a job has heard of (docs/specs/SPEC_DRONE_REAL_WORLD_JOBS_2026_11_11.md §3.7): delivery is at least
//! once, so a source may name the same event twice, and the job's `(job_id, event_id)` here is what turns the second
//! into nothing. It sits beside the run ledger, in the identity store every channel and version on this computer
//! shares, so two instances sharing a job don't both start a run for one event.
//!
//! The id table keeps only the id. The event's data is kept apart, per run, in `db_job_event_inputs`, for as long as
//! the run may still need it: while it waits out a delay or its turn, and across a retry. The run's end (or [`KEEP_MS`])
//! lets it go. A job forgets an id after [`KEEP_MS`]: a source that repeats an id after that is a new event.

use rusqlite::{params, OptionalExtension};

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
    CREATE INDEX IF NOT EXISTS idx_ids_job_trigger_events_at ON db_job_trigger_events(at_ms);
    CREATE TABLE IF NOT EXISTS db_job_event_inputs (
        run_id  TEXT PRIMARY KEY,
        job_id  TEXT NOT NULL,
        input   TEXT NOT NULL,
        at_ms   INTEGER NOT NULL
    );
    CREATE INDEX IF NOT EXISTS idx_ids_job_event_inputs_at ON db_job_event_inputs(at_ms);";

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

    /// Forget the ids heard, and the inputs held, before `before_ms`. How many ids went.
    pub fn job_events_forget_before(&self, before_ms: i64) -> Result<usize, StoreError> {
        let conn = self.conn().lock().unwrap();
        conn.execute("DELETE FROM db_job_event_inputs WHERE at_ms < ?1", params![before_ms])?;
        Ok(conn.execute("DELETE FROM db_job_trigger_events WHERE at_ms < ?1", params![before_ms])?)
    }

    /// Keep `input` (what the graph reads as `{{trigger.*}}`) for run `run_id`.
    pub fn job_event_input_put(&self, run_id: &str, job_id: &str, input: &serde_json::Value, now_ms: i64) -> Result<(), StoreError> {
        let conn = self.conn().lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO db_job_event_inputs (run_id, job_id, input, at_ms) VALUES (?1, ?2, ?3, ?4)",
            params![run_id, job_id, input.to_string(), now_ms],
        )?;
        Ok(())
    }

    /// The input kept for run `run_id`, if any.
    pub fn job_event_input_get(&self, run_id: &str) -> Result<Option<serde_json::Value>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let text: Option<String> =
            conn.query_row("SELECT input FROM db_job_event_inputs WHERE run_id = ?1", params![run_id], |r| r.get(0)).optional()?;
        Ok(text.and_then(|t| serde_json::from_str(&t).ok()))
    }

    /// Let go of the input of run `run_id`.
    pub fn job_event_input_drop(&self, run_id: &str) -> Result<(), StoreError> {
        let conn = self.conn().lock().unwrap();
        conn.execute("DELETE FROM db_job_event_inputs WHERE run_id = ?1", params![run_id])?;
        Ok(())
    }

    /// Hand the input of run `from` to run `to` (a retry takes over the event of the attempt before it).
    pub fn job_event_input_carry(&self, from: &str, to: &str) -> Result<(), StoreError> {
        let conn = self.conn().lock().unwrap();
        conn.execute("UPDATE db_job_event_inputs SET run_id = ?2 WHERE run_id = ?1", params![from, to])?;
        Ok(())
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

    #[test]
    fn an_input_is_kept_per_run_carried_dropped_and_forgotten_with_age() {
        let s = store();
        let input = serde_json::json!({ "outcome": "failed", "n": 3 });
        s.job_event_input_put("run-1", "job-a", &input, 100).unwrap();
        assert_eq!(s.job_event_input_get("run-1").unwrap(), Some(input.clone()));
        assert_eq!(s.job_event_input_get("run-2").unwrap(), None);

        s.job_event_input_carry("run-1", "run-2").unwrap();
        assert_eq!((s.job_event_input_get("run-1").unwrap(), s.job_event_input_get("run-2").unwrap()), (None, Some(input)));

        s.job_event_input_drop("run-2").unwrap();
        assert_eq!(s.job_event_input_get("run-2").unwrap(), None);

        s.job_event_input_put("old", "job-a", &serde_json::json!({}), 100).unwrap();
        s.job_event_input_put("new", "job-a", &serde_json::json!({}), 900).unwrap();
        s.job_events_forget_before(500).unwrap();
        assert_eq!((s.job_event_input_get("old").unwrap(), s.job_event_input_get("new").unwrap().is_some()), (None, true));
    }
}
