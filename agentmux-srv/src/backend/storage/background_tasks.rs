// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Durable registry of declared long-running (`run_in_background: true`)
//! tasks attached to a block. See `db_background_tasks` in migrations.rs
//! (`OBJECT_SCHEMA_VERSION` v20) and
//! docs/status/STATUS_ATTACHED_TASK_AXIS_AND_DEV_LOOP_2026_08_15.md.
//!
//! This exists because the frontend's own signal chain for "is a
//! long-running task attached to this pane" — transcript replay →
//! in-memory reducer slice → `DockSnapshotCache` — is entirely ephemeral:
//! scoped to one open tab, and (for the srv-side `DockSnapshotCache` mirror)
//! silently evicted after one hour regardless of whether the task is still
//! genuinely running. A `task dev` session in this repo's own retros has
//! run 12+ hours. `id` mirrors the frontend's dock `node_id` (normally the
//! originating tool_use_id) so rows join cleanly with dock data.

use rusqlite::params;

use super::error::StoreError;
use super::store::Store;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BackgroundTaskStatus {
    Running,
    Done,
    Error,
    Stopped,
}

impl BackgroundTaskStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            BackgroundTaskStatus::Running => "running",
            BackgroundTaskStatus::Done => "done",
            BackgroundTaskStatus::Error => "error",
            BackgroundTaskStatus::Stopped => "stopped",
        }
    }

    /// Unrecognized values fall back to `Running` — same "never claim
    /// running-forever silently disappears, never claim a live task is
    /// terminal on a parse hiccup" posture as the rest of this feature
    /// area (`isAcceptedBackgroundLaunch`'s "unrecognized → stopped, never
    /// running-forever" rule is about outgoing classification; this is the
    /// inverse case of reading a row back, where erring toward "still
    /// live" is the safer default for a status this registry itself wrote).
    pub fn from_str(s: &str) -> Self {
        match s {
            "done" => BackgroundTaskStatus::Done,
            "error" => BackgroundTaskStatus::Error,
            "stopped" => BackgroundTaskStatus::Stopped,
            _ => BackgroundTaskStatus::Running,
        }
    }

    pub fn is_terminal(self) -> bool {
        !matches!(self, BackgroundTaskStatus::Running)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BackgroundTask {
    pub id: String,
    pub block_id: String,
    pub label: String,
    pub pid: Option<i64>,
    pub started_at_ms: i64,
    pub status: BackgroundTaskStatus,
    pub last_seen_ms: i64,
    pub ended_at_ms: Option<i64>,
}

impl Store {
    /// Create the row if it doesn't exist yet (status `running`), and
    /// refresh `last_seen_ms` — but only while the row is still `running`.
    /// Never resurrects a row a concurrent `background_task_complete` call
    /// already marked terminal; the terminal status is left untouched.
    pub fn background_task_observe(
        &self,
        id: &str,
        block_id: &str,
        label: &str,
        started_at_ms: i64,
        now_ms: i64,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO db_background_tasks
                (id, block_id, label, pid, started_at_ms, status, last_seen_ms, ended_at_ms)
             VALUES (?1, ?2, ?3, NULL, ?4, 'running', ?5, NULL)",
            params![id, block_id, label, started_at_ms, now_ms],
        )?;
        conn.execute(
            "UPDATE db_background_tasks SET last_seen_ms = ?1 WHERE id = ?2 AND status = 'running'",
            params![now_ms, id],
        )?;
        Ok(())
    }

    /// Record the OS pid once known (bashwrap/process_tracker learn it
    /// after the task has already been observed as declared-background).
    /// No-op (returns `false`) if the row doesn't exist.
    pub fn background_task_set_pid(&self, id: &str, pid: i64) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "UPDATE db_background_tasks SET pid = ?1 WHERE id = ?2",
            params![pid, id],
        )?;
        Ok(rows > 0)
    }

    /// Mark a task terminal. Idempotent — calling this again on an already-
    /// terminal row just overwrites the status/ended_at (last write wins),
    /// which only matters if two different terminal signals race, an edge
    /// case not worth a CAS for a status transition with no live consumer
    /// depending on "first terminal write wins" today.
    pub fn background_task_complete(
        &self,
        id: &str,
        status: BackgroundTaskStatus,
        ended_at_ms: i64,
    ) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "UPDATE db_background_tasks
             SET status = ?1, ended_at_ms = ?2, last_seen_ms = ?2
             WHERE id = ?3",
            params![status.as_str(), ended_at_ms, id],
        )?;
        Ok(rows > 0)
    }

    pub fn background_task_get(&self, id: &str) -> Result<Option<BackgroundTask>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, block_id, label, pid, started_at_ms, status, last_seen_ms, ended_at_ms
             FROM db_background_tasks WHERE id = ?1",
        )?;
        let mut rows = stmt.query_map(params![id], map_row)?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    pub fn background_task_list_for_block(&self, block_id: &str) -> Result<Vec<BackgroundTask>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, block_id, label, pid, started_at_ms, status, last_seen_ms, ended_at_ms
             FROM db_background_tasks WHERE block_id = ?1 ORDER BY started_at_ms ASC",
        )?;
        let iter = stmt.query_map(params![block_id], map_row)?;
        iter.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Global — every still-`running` row across every block. Used by
    /// Swarm-style fleet views and reconnect/history-restore, where the
    /// caller doesn't yet know which block(s) to ask about.
    pub fn background_task_list_running(&self) -> Result<Vec<BackgroundTask>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, block_id, label, pid, started_at_ms, status, last_seen_ms, ended_at_ms
             FROM db_background_tasks WHERE status = 'running' ORDER BY started_at_ms ASC",
        )?;
        let iter = stmt.query_map([], map_row)?;
        iter.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }
}

fn map_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<BackgroundTask> {
    let status: String = row.get(5)?;
    Ok(BackgroundTask {
        id: row.get(0)?,
        block_id: row.get(1)?,
        label: row.get(2)?,
        pid: row.get(3)?,
        started_at_ms: row.get(4)?,
        status: BackgroundTaskStatus::from_str(&status),
        last_seen_ms: row.get(6)?,
        ended_at_ms: row.get(7)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object_store() -> Store {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        Store::open(tmp.path()).unwrap()
    }

    #[test]
    fn observe_creates_a_running_row() {
        let store = object_store();
        store.background_task_observe("t1", "block-a", "task dev", 1000, 1000).unwrap();
        let task = store.background_task_get("t1").unwrap().unwrap();
        assert_eq!(task.block_id, "block-a");
        assert_eq!(task.label, "task dev");
        assert_eq!(task.status, BackgroundTaskStatus::Running);
        assert_eq!(task.started_at_ms, 1000);
        assert_eq!(task.last_seen_ms, 1000);
        assert_eq!(task.pid, None);
        assert_eq!(task.ended_at_ms, None);
    }

    #[test]
    fn observe_is_idempotent_and_refreshes_last_seen() {
        let store = object_store();
        store.background_task_observe("t1", "block-a", "task dev", 1000, 1000).unwrap();
        store.background_task_observe("t1", "block-a", "task dev", 1000, 5000).unwrap();
        let task = store.background_task_get("t1").unwrap().unwrap();
        // started_at_ms is NOT overwritten by a repeat observe (INSERT OR
        // IGNORE leaves the original row alone) — only last_seen_ms moves.
        assert_eq!(task.started_at_ms, 1000);
        assert_eq!(task.last_seen_ms, 5000);
    }

    #[test]
    fn observe_never_resurrects_a_completed_task() {
        let store = object_store();
        store.background_task_observe("t1", "block-a", "task dev", 1000, 1000).unwrap();
        store.background_task_complete("t1", BackgroundTaskStatus::Done, 2000).unwrap();
        // A late/duplicate push racing after completion must not flip the
        // task back to "running" or move its last_seen_ms.
        store.background_task_observe("t1", "block-a", "task dev", 1000, 9000).unwrap();
        let task = store.background_task_get("t1").unwrap().unwrap();
        assert_eq!(task.status, BackgroundTaskStatus::Done);
        assert_eq!(task.last_seen_ms, 2000);
    }

    #[test]
    fn complete_marks_terminal_and_returns_true_when_row_exists() {
        let store = object_store();
        store.background_task_observe("t1", "block-a", "task dev", 1000, 1000).unwrap();
        let changed = store.background_task_complete("t1", BackgroundTaskStatus::Error, 4000).unwrap();
        assert!(changed);
        let task = store.background_task_get("t1").unwrap().unwrap();
        assert_eq!(task.status, BackgroundTaskStatus::Error);
        assert_eq!(task.ended_at_ms, Some(4000));
    }

    #[test]
    fn complete_returns_false_for_unknown_id() {
        let store = object_store();
        assert!(!store.background_task_complete("nope", BackgroundTaskStatus::Done, 1).unwrap());
    }

    #[test]
    fn set_pid_updates_existing_row_and_no_ops_for_unknown_id() {
        let store = object_store();
        store.background_task_observe("t1", "block-a", "task dev", 1000, 1000).unwrap();
        assert!(store.background_task_set_pid("t1", 4242).unwrap());
        assert_eq!(store.background_task_get("t1").unwrap().unwrap().pid, Some(4242));
        assert!(!store.background_task_set_pid("nope", 1).unwrap());
    }

    #[test]
    fn get_returns_none_for_unknown_id() {
        let store = object_store();
        assert!(store.background_task_get("nope").unwrap().is_none());
    }

    #[test]
    fn list_for_block_only_returns_that_blocks_tasks_in_start_order() {
        let store = object_store();
        store.background_task_observe("t1", "block-a", "first", 1000, 1000).unwrap();
        store.background_task_observe("t2", "block-b", "other-block", 1500, 1500).unwrap();
        store.background_task_observe("t3", "block-a", "second", 2000, 2000).unwrap();
        let tasks = store.background_task_list_for_block("block-a").unwrap();
        let ids: Vec<&str> = tasks.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["t1", "t3"]);
    }

    #[test]
    fn list_running_excludes_terminal_tasks_across_all_blocks() {
        let store = object_store();
        store.background_task_observe("t1", "block-a", "still running", 1000, 1000).unwrap();
        store.background_task_observe("t2", "block-b", "finished", 1000, 1000).unwrap();
        store.background_task_complete("t2", BackgroundTaskStatus::Done, 2000).unwrap();
        let running = store.background_task_list_running().unwrap();
        let ids: Vec<&str> = running.iter().map(|t| t.id.as_str()).collect();
        assert_eq!(ids, vec!["t1"]);
    }

    #[test]
    fn status_round_trips_through_str_conversion() {
        for s in [
            BackgroundTaskStatus::Running,
            BackgroundTaskStatus::Done,
            BackgroundTaskStatus::Error,
            BackgroundTaskStatus::Stopped,
        ] {
            assert_eq!(BackgroundTaskStatus::from_str(s.as_str()), s);
        }
    }

    #[test]
    fn unrecognized_status_string_defaults_to_running() {
        assert_eq!(BackgroundTaskStatus::from_str("garbage"), BackgroundTaskStatus::Running);
    }

    #[test]
    fn is_terminal_is_true_for_everything_but_running() {
        assert!(!BackgroundTaskStatus::Running.is_terminal());
        assert!(BackgroundTaskStatus::Done.is_terminal());
        assert!(BackgroundTaskStatus::Error.is_terminal());
        assert!(BackgroundTaskStatus::Stopped.is_terminal());
    }

    /// Survives a fresh `Store::open` against the same file — proves the
    /// row is actually durable (the whole point of this table existing
    /// instead of another in-memory cache), not just readable within one
    /// connection's lifetime.
    #[test]
    fn survives_reopening_the_store() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        {
            let store = Store::open(tmp.path()).unwrap();
            store.background_task_observe("t1", "block-a", "task dev", 1000, 1000).unwrap();
        }
        let reopened = Store::open(tmp.path()).unwrap();
        let task = reopened.background_task_get("t1").unwrap().unwrap();
        assert_eq!(task.label, "task dev");
        assert_eq!(task.status, BackgroundTaskStatus::Running);
    }
}
