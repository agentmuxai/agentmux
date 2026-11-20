// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a container agent's `agentmux-bashwrap` reports about a Bash call, kept beside the durable
//! background-task registry (`background_tasks.rs`) and never in it.
//!
//! The process id comes from the container's own pid namespace: on this host it would name some other
//! process, so it is never written to `db_background_tasks.pid` (the host's column, which the liveness
//! sweep signals and checks) and never used for host process labels. It is filed here with the container's
//! name, and only the container-side check (`background_task_container_sweep`) reads it. The call's
//! description is for the registry row's label only.
//!
//! Rows are keyed by the pane and the call's id, so a container can file only for its own pane's calls.
//! The report usually comes before the registry row exists, which is why it has a table of its own: the row,
//! made later by the CLI's task feed or the pane, finds it ([`Store::background_task_observe`]).

use rusqlite::params;

use super::error::StoreError;
use super::store::Store;

/// The longest description kept: a label, not a transcript.
const MAX_DESCRIPTION_CHARS: usize = 200;

/// A running background task whose process is in a container.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContainerTask {
    pub id: String,
    pub block_id: String,
    pub container: String,
    /// The process id in the container's pid namespace.
    pub pid: i64,
    pub started_at_ms: i64,
}

/// A description as the registry label: control characters out, trimmed, cut to a label's length. `None`
/// when nothing is left.
pub fn clean_description(raw: &str) -> Option<String> {
    let text: String = raw.chars().filter(|c| !c.is_control()).collect();
    let text = text.trim();
    (!text.is_empty()).then(|| text.chars().take(MAX_DESCRIPTION_CHARS).collect())
}

impl Store {
    /// File what the container reported for the call `id` of `block_id`: a pid, a description or both. A
    /// field not given keeps what an earlier report filed.
    pub fn background_container_call_file(
        &self,
        block_id: &str,
        id: &str,
        container: &str,
        pid: Option<i64>,
        description: Option<&str>,
        at_ms: i64,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO db_background_container_calls (block_id, id, container, pid, description, at_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6)
             ON CONFLICT(block_id, id) DO UPDATE SET
                container = excluded.container,
                pid = COALESCE(excluded.pid, pid),
                description = COALESCE(excluded.description, description),
                at_ms = excluded.at_ms",
            params![block_id, id, container, pid, description, at_ms],
        )?;
        Ok(())
    }

    /// The description a container filed for the call `id` of `block_id`.
    pub fn background_container_call_description(&self, block_id: &str, id: &str) -> Result<Option<String>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT description FROM db_background_container_calls WHERE block_id = ?1 AND id = ?2")?;
        let mut rows = stmt.query_map(params![block_id, id], |r| r.get::<_, Option<String>>(0))?;
        Ok(rows.next().transpose()?.flatten())
    }

    /// Every `running` registry row whose process a container filed a pid for.
    pub fn background_container_running(&self) -> Result<Vec<ContainerTask>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT t.id, t.block_id, c.container, c.pid, t.started_at_ms
             FROM db_background_tasks t
             JOIN db_background_container_calls c ON c.block_id = t.block_id AND c.id = t.id
             WHERE t.status = 'running' AND c.pid IS NOT NULL
             ORDER BY t.started_at_ms ASC",
        )?;
        let rows = stmt.query_map([], |r| {
            Ok(ContainerTask { id: r.get(0)?, block_id: r.get(1)?, container: r.get(2)?, pid: r.get(3)?, started_at_ms: r.get(4)? })
        })?;
        rows.collect::<Result<Vec<_>, _>>().map_err(Into::into)
    }

    /// Forget reports filed before `before_ms`: a call that old has long ended, or its row did.
    pub fn background_container_calls_prune(&self, before_ms: i64) -> Result<usize, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.execute("DELETE FROM db_background_container_calls WHERE at_ms < ?1", params![before_ms])?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        Store::open(std::path::Path::new(":memory:")).unwrap()
    }

    #[test]
    fn a_description_is_a_clean_short_label() {
        assert_eq!(clean_description("  Run the build \u{7}\n").as_deref(), Some("Run the build"));
        assert_eq!(clean_description(" \t\n"), None);
        assert_eq!(clean_description(&"x".repeat(500)).unwrap().chars().count(), MAX_DESCRIPTION_CHARS);
    }

    #[test]
    fn a_later_report_adds_to_an_earlier_one_and_never_blanks_it() {
        let s = store();
        s.background_container_call_file("b1", "t1", "agentmux-a", Some(7), None, 10).unwrap();
        s.background_container_call_file("b1", "t1", "agentmux-a", None, Some("Run it"), 20).unwrap();
        assert_eq!(s.background_container_call_description("b1", "t1").unwrap().as_deref(), Some("Run it"));
        s.background_task_observe("t1", "b1", "Bash", 5, 5).unwrap();
        let rows = s.background_container_running().unwrap();
        assert_eq!(rows, vec![ContainerTask { id: "t1".into(), block_id: "b1".into(), container: "agentmux-a".into(), pid: 7, started_at_ms: 5 }]);
    }

    #[test]
    fn a_container_row_is_listed_only_for_its_own_pane_and_while_running() {
        let s = store();
        // Call ids are unique across panes: t1 is b2's row. Pane b1's container files a pid for it anyway.
        s.background_task_observe("t1", "b2", "Bash", 5, 5).unwrap();
        s.background_task_observe("t2", "b1", "Bash", 5, 5).unwrap();
        s.background_container_call_file("b1", "t1", "agentmux-a", Some(7), None, 10).unwrap();
        s.background_container_call_file("b1", "t2", "agentmux-a", None, Some("no pid"), 10).unwrap();
        assert!(s.background_container_running().unwrap().is_empty(), "filed under the wrong pane, and no pid");
        s.background_container_call_file("b2", "t1", "agentmux-b", Some(8), None, 10).unwrap();
        assert_eq!(s.background_container_running().unwrap().len(), 1);
        s.background_task_complete("t1", crate::backend::storage::background_tasks::BackgroundTaskStatus::Done, 9).unwrap();
        assert!(s.background_container_running().unwrap().is_empty(), "an ended task is not checked");
    }

    #[test]
    fn old_reports_are_pruned() {
        let s = store();
        s.background_container_call_file("b1", "old", "c", Some(1), None, 10).unwrap();
        s.background_container_call_file("b1", "new", "c", Some(2), None, 100).unwrap();
        assert_eq!(s.background_container_calls_prune(50).unwrap(), 1);
        assert!(s.background_container_call_description("b1", "new").is_ok());
    }
}
