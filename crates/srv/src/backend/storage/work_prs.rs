// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! What GitHub said about a task's PR, for the board's last two columns
//! (docs/specs/SPEC_KANBAN_STAGES_COLOURS_AND_NARROW_VIEW_2026_10_23.md §4):
//! **Merged** when its PR merged, **Released** once a release of its
//! repository contains that merge. srv's PR watcher writes it
//! (`backend/kanban/prs.rs`); nothing else does.
//!
//! Kept in `db_work_prs`, beside the board's other tables, so older builds on
//! the same computer never see it. Pruned with the tasks it belongs to.
//!
//! And a task's document (`db_work_docs`): the path of the spec or report a
//! `spec` or `research` task wrote (`links.doc`), which the card of the build
//! that starts after it shows (§5.2).

use rusqlite::{params, params_from_iter, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::store::Store;
use super::work_board::BoardTask;
use super::StoreError;

/// The table, created with the board's (`work_board::SCHEMA`).
pub(crate) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS db_work_prs (
        work_id     TEXT PRIMARY KEY,
        merged_at   INTEGER,
        released_in TEXT NOT NULL DEFAULT '',
        released_at INTEGER,
        updated_at  INTEGER NOT NULL DEFAULT 0
    );
    CREATE TABLE IF NOT EXISTS db_work_docs (
        work_id    TEXT PRIMARY KEY,
        path       TEXT NOT NULL,
        updated_at INTEGER NOT NULL DEFAULT 0
    );";

/// Task ids per lookup query, well under SQLite's bound-parameter limit.
const IDS_PER_QUERY: usize = 500;

/// A task's PR, as far as srv has learned.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkPr {
    /// When it merged (ms); `None` while it hasn't, or srv hasn't seen it.
    pub merged_at: Option<i64>,
    /// The release that contains the merge (its tag, e.g. `v0.59.41`); empty
    /// until one does.
    #[serde(default)]
    pub released_in: String,
    /// When that release was made (ms).
    pub released_at: Option<i64>,
}

fn row_to_pr(r: &rusqlite::Row) -> rusqlite::Result<WorkPr> {
    Ok(WorkPr { merged_at: r.get("merged_at")?, released_in: r.get("released_in")?, released_at: r.get("released_at")? })
}

/// Fill each task's `card.pr` and `card.doc`.
pub(crate) fn attach_prs(conn: &Connection, tasks: &mut [BoardTask]) -> Result<(), StoreError> {
    let ids: Vec<String> = tasks.iter().map(|t| t.item.id.clone()).collect();
    let mut found = std::collections::HashMap::new();
    let mut docs = std::collections::HashMap::new();
    for chunk in ids.chunks(IDS_PER_QUERY) {
        let marks = vec!["?"; chunk.len()].join(",");
        let sql = format!("SELECT work_id, merged_at, released_in, released_at FROM db_work_prs WHERE work_id IN ({marks})");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(chunk.iter()), |r| Ok((r.get::<_, String>("work_id")?, row_to_pr(r)?)))?;
        for row in rows {
            let (id, pr) = row?;
            found.insert(id, pr);
        }
        let sql = format!("SELECT work_id, path FROM db_work_docs WHERE work_id IN ({marks})");
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(chunk.iter()), |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
        for row in rows {
            let (id, path) = row?;
            docs.insert(id, path);
        }
    }
    for t in tasks {
        t.card.pr = found.remove(&t.item.id);
        t.card.doc = docs.remove(&t.item.id).unwrap_or_default();
    }
    Ok(())
}

impl Store {
    /// What is known about `work_id`'s PR.
    pub fn work_pr_get(&self, work_id: &str) -> Result<Option<WorkPr>, StoreError> {
        let conn = self.conn().lock().unwrap();
        Ok(conn
            .query_row(
                "SELECT merged_at, released_in, released_at FROM db_work_prs WHERE work_id = ?1",
                params![work_id],
                row_to_pr,
            )
            .optional()?)
    }

    /// Record that `work_id`'s PR merged at `merged_at`. Returns whether that
    /// was news.
    pub fn work_pr_merged(&self, work_id: &str, merged_at: i64, now_ms: i64) -> Result<bool, StoreError> {
        let conn = self.conn().lock().unwrap();
        let n = conn.execute(
            "INSERT INTO db_work_prs (work_id, merged_at, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(work_id) DO UPDATE SET merged_at = excluded.merged_at, updated_at = excluded.updated_at
              WHERE db_work_prs.merged_at IS NULL",
            params![work_id, merged_at, now_ms],
        )?;
        Ok(n > 0)
    }

    /// Record that the release `tag`, made at `released_at`, contains
    /// `work_id`'s merged PR. Only once, and only after its merge is known.
    /// The task counts as changed, so a finished one stays on the board its
    /// week after it ships, and boards elsewhere see it move. Returns whether
    /// that was news.
    pub fn work_pr_released(&self, work_id: &str, tag: &str, released_at: i64, now_ms: i64) -> Result<bool, StoreError> {
        let mut conn = self.conn().lock().unwrap();
        let tx = conn.transaction()?;
        let n = tx.execute(
            "UPDATE db_work_prs SET released_in = ?2, released_at = ?3, updated_at = ?4
              WHERE work_id = ?1 AND merged_at IS NOT NULL AND released_in = ''",
            params![work_id, tag, released_at, now_ms],
        )?;
        if n > 0 {
            tx.execute("UPDATE db_work_queue SET updated_at = MAX(updated_at, ?2) WHERE id = ?1", params![work_id, now_ms])?;
            tx.execute("UPDATE db_work_cards SET updated_at = MAX(updated_at, ?2) WHERE work_id = ?1", params![work_id, now_ms])?;
        }
        tx.commit()?;
        Ok(n > 0)
    }

    /// Drop what is known about tasks no longer in the queue.
    pub fn work_prs_prune(&self) -> Result<usize, StoreError> {
        let conn = self.conn().lock().unwrap();
        let prs = conn.execute("DELETE FROM db_work_prs WHERE work_id NOT IN (SELECT id FROM db_work_queue)", [])?;
        let docs = conn.execute("DELETE FROM db_work_docs WHERE work_id NOT IN (SELECT id FROM db_work_queue)", [])?;
        Ok(prs + docs)
    }

    /// Set the path of the document `work_id` wrote (`links.doc`).
    pub fn work_doc_set(&self, work_id: &str, path: &str, now_ms: i64) -> Result<(), StoreError> {
        let conn = self.conn().lock().unwrap();
        conn.execute(
            "INSERT INTO db_work_docs (work_id, path, updated_at) VALUES (?1, ?2, ?3)
             ON CONFLICT(work_id) DO UPDATE SET path = excluded.path, updated_at = excluded.updated_at",
            params![work_id, path, now_ms],
        )?;
        Ok(())
    }
}

#[cfg(test)]
#[path = "work_prs_tests.rs"]
mod tests;
