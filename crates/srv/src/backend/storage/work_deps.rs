// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Tasks that start after others (docs/specs/SPEC_WARDEN_KANBAN_2026_10_20.md
//! §6.15): "#14 starts after #12". A task waiting on a task that isn't
//! finished stays in the Backlog: no claim takes it, no reminder or "next"
//! names it. Done or cancelled finishes a task for this; a failed one still
//! holds its followers, since the operator decides what happens to it.
//!
//! Kept in `db_work_deps`, beside the board's other tables (`work_board.rs`),
//! so older builds on the same computer never see it. An older build's claim
//! doesn't know the rule either and may take a waiting task: acceptable, as
//! with the board's other states (§10).

use std::collections::{HashMap, HashSet};

use rusqlite::{params, Connection};
use serde::{Deserialize, Serialize};

use super::store::Store;
use super::work_board::BoardTask;
use super::work_queue::work_state;
use super::StoreError;

/// The table, created with the board's (`work_board::SCHEMA`).
pub(crate) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS db_work_deps (
        work_id   TEXT NOT NULL,
        after_id  TEXT NOT NULL,
        PRIMARY KEY (work_id, after_id)
    );
    CREATE INDEX IF NOT EXISTS idx_ids_work_deps_after ON db_work_deps(after_id);";

/// Whether the task aliased `q` waits on one that isn't finished, for the
/// queries that hand tasks out (claim, the agent's waiting tasks).
pub(crate) const WAITS_ON_UNFINISHED: &str = "EXISTS (SELECT 1 FROM db_work_deps d
         JOIN db_work_queue p ON p.id = d.after_id
        WHERE d.work_id = q.id AND p.state NOT IN ('done', 'cancelled'))";

/// The most tasks one can wait on.
pub const MAX_AFTER: usize = 10;

/// A task another starts after, as its card shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AfterTask {
    pub id: String,
    pub num: i64,
    pub title: String,
    pub state: String,
}

impl AfterTask {
    /// Whether it no longer holds its followers.
    pub fn finished(&self) -> bool {
        self.state == work_state::DONE || self.state == work_state::CANCELLED
    }
}

/// Every dependency, by the task that waits.
fn all_after(conn: &Connection) -> Result<HashMap<String, Vec<AfterTask>>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT d.work_id, p.id, COALESCE(c.num, 0), p.title, p.state
           FROM db_work_deps d
           JOIN db_work_queue p ON p.id = d.after_id
           LEFT JOIN db_work_cards c ON c.work_id = p.id
          ORDER BY c.num",
    )?;
    let rows = stmt.query_map([], |r| {
        Ok((r.get::<_, String>(0)?, AfterTask { id: r.get(1)?, num: r.get(2)?, title: r.get(3)?, state: r.get(4)? }))
    })?;
    let mut out: HashMap<String, Vec<AfterTask>> = HashMap::new();
    for row in rows {
        let (work_id, after) = row?;
        out.entry(work_id).or_default().push(after);
    }
    Ok(out)
}

/// Fill each task's `card.after`.
pub(crate) fn attach_after(conn: &Connection, tasks: &mut [BoardTask]) -> Result<(), StoreError> {
    let mut all = all_after(conn)?;
    if all.is_empty() {
        return Ok(());
    }
    for t in tasks {
        if let Some(after) = all.remove(&t.item.id) {
            t.card.after = after;
        }
    }
    Ok(())
}

impl Store {
    /// Why `work_id` can't start after `after_ids` (task ids), or `None` when
    /// it can: each must exist, not be the task itself, and not wait on it,
    /// however indirectly.
    pub fn work_deps_refusal(&self, work_id: &str, after_ids: &[String]) -> Result<Option<String>, StoreError> {
        if after_ids.len() > MAX_AFTER {
            return Ok(Some(format!("a task can start after at most {MAX_AFTER} others")));
        }
        let conn = self.conn().lock().unwrap();
        let mut edges: HashMap<String, Vec<String>> = HashMap::new();
        {
            let mut stmt = conn.prepare("SELECT work_id, after_id FROM db_work_deps")?;
            let rows = stmt.query_map([], |r| Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?)))?;
            for row in rows {
                let (w, a) = row?;
                edges.entry(w).or_default().push(a);
            }
        }
        for after in after_ids {
            if after == work_id {
                return Ok(Some("a task can't start after itself".into()));
            }
            let exists: i64 = conn.query_row("SELECT COUNT(*) FROM db_work_queue WHERE id = ?1", params![after], |r| r.get(0))?;
            if exists == 0 {
                return Ok(Some(format!("no task {after}")));
            }
            // Does `after` already wait on `work_id`, through any chain?
            let mut seen = HashSet::new();
            let mut stack = vec![after.clone()];
            while let Some(next) = stack.pop() {
                if next == work_id {
                    return Ok(Some("that would make the tasks wait on each other".into()));
                }
                if seen.insert(next.clone()) {
                    stack.extend(edges.get(&next).into_iter().flatten().cloned());
                }
            }
        }
        Ok(None)
    }

    /// Replace the tasks `work_id` starts after. Check
    /// [`Store::work_deps_refusal`] first.
    pub fn work_deps_set(&self, work_id: &str, after_ids: &[String]) -> Result<(), StoreError> {
        let mut conn = self.conn().lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute("DELETE FROM db_work_deps WHERE work_id = ?1", params![work_id])?;
        for after in after_ids {
            tx.execute("INSERT OR IGNORE INTO db_work_deps (work_id, after_id) VALUES (?1, ?2)", params![work_id, after])?;
        }
        tx.commit()?;
        Ok(())
    }

    /// The tasks `work_id` starts after.
    pub fn work_deps_of(&self, work_id: &str) -> Result<Vec<AfterTask>, StoreError> {
        let conn = self.conn().lock().unwrap();
        Ok(all_after(&conn)?.remove(work_id).unwrap_or_default())
    }

    /// Drop the dependencies of tasks no longer in the queue, and on them.
    pub fn work_deps_prune(&self) -> Result<usize, StoreError> {
        let conn = self.conn().lock().unwrap();
        Ok(conn.execute(
            "DELETE FROM db_work_deps
              WHERE work_id NOT IN (SELECT id FROM db_work_queue)
                 OR after_id NOT IN (SELECT id FROM db_work_queue)",
            [],
        )?)
    }
}

#[cfg(test)]
#[path = "work_deps_tests.rs"]
mod tests;
