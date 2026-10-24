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
//!
//! Every write checks the list and writes it in one immediate transaction,
//! so two edits (from this srv or another on the computer) can't each pass
//! the check and together make tasks wait on each other; a new task's row
//! and its list are written together, so no claim or prune sees one without
//! the other.

use std::collections::{HashMap, HashSet};

use rusqlite::{params, params_from_iter, Connection, TransactionBehavior};
use serde::{Deserialize, Serialize};

use super::store::Store;
use super::work_board::BoardTask;
use super::work_queue::{insert_item, work_state, WorkItem};
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
/// Task ids per lookup query, well under SQLite's bound-parameter limit.
const IDS_PER_QUERY: usize = 500;

/// A task another starts after (or, in a card's `followers`, one that starts
/// after it), as its card shows it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AfterTask {
    pub id: String,
    pub num: i64,
    pub title: String,
    pub state: String,
    /// Its kind: a follower of an accepted `spec` or `research` task is
    /// Spec'd (SPEC_KANBAN_STAGES_COLOURS_AND_NARROW_VIEW_2026_10_23.md §4).
    #[serde(default)]
    pub kind: String,
}

impl AfterTask {
    /// Whether it no longer holds its followers.
    pub fn finished(&self) -> bool {
        self.state == work_state::DONE || self.state == work_state::CANCELLED
    }
}

/// The dependencies of these tasks, by the task that waits.
fn after_of(conn: &Connection, ids: &[&str]) -> Result<HashMap<String, Vec<AfterTask>>, StoreError> {
    linked(conn, ids, "d.work_id", "d.after_id")
}

/// The tasks that start after these, by the task they wait on.
fn followers_of(conn: &Connection, ids: &[&str]) -> Result<HashMap<String, Vec<AfterTask>>, StoreError> {
    linked(conn, ids, "d.after_id", "d.work_id")
}

/// The other end of each of these tasks' dependencies: `by` is the column
/// that names them, `other` the task shown.
fn linked(conn: &Connection, ids: &[&str], by: &str, other: &str) -> Result<HashMap<String, Vec<AfterTask>>, StoreError> {
    let mut out: HashMap<String, Vec<AfterTask>> = HashMap::new();
    for chunk in ids.chunks(IDS_PER_QUERY) {
        let marks = vec!["?"; chunk.len()].join(",");
        let sql = format!(
            "SELECT {by}, p.id, COALESCE(c.num, 0), p.title, p.state, p.kind
               FROM db_work_deps d
               JOIN db_work_queue p ON p.id = {other}
               LEFT JOIN db_work_cards c ON c.work_id = p.id
              WHERE {by} IN ({marks})
              ORDER BY c.num"
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_from_iter(chunk.iter()), |r| {
            Ok((
                r.get::<_, String>(0)?,
                AfterTask { id: r.get(1)?, num: r.get(2)?, title: r.get(3)?, state: r.get(4)?, kind: r.get(5)? },
            ))
        })?;
        for row in rows {
            let (work_id, after) = row?;
            out.entry(work_id).or_default().push(after);
        }
    }
    Ok(out)
}

/// Fill each task's `card.after` and `card.followers`.
pub(crate) fn attach_after(conn: &Connection, tasks: &mut [BoardTask]) -> Result<(), StoreError> {
    let ids: Vec<&str> = tasks.iter().map(|t| t.item.id.as_str()).collect();
    let mut after = after_of(conn, &ids)?;
    let mut followers = followers_of(conn, &ids)?;
    for t in tasks {
        if let Some(a) = after.remove(&t.item.id) {
            t.card.after = a;
        }
        if let Some(f) = followers.remove(&t.item.id) {
            t.card.followers = f;
        }
    }
    Ok(())
}

/// Why `work_id` can't start after `after_ids`, or `None` when it can: each
/// must exist, not be the task itself, and not wait on it, however
/// indirectly.
fn refusal(conn: &Connection, work_id: &str, after_ids: &[String]) -> Result<Option<String>, StoreError> {
    if after_ids.len() > MAX_AFTER {
        return Ok(Some(format!("a task can start after at most {MAX_AFTER} others")));
    }
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

fn replace(conn: &Connection, work_id: &str, after_ids: &[String]) -> Result<(), StoreError> {
    conn.execute("DELETE FROM db_work_deps WHERE work_id = ?1", params![work_id])?;
    for after in after_ids {
        conn.execute("INSERT OR IGNORE INTO db_work_deps (work_id, after_id) VALUES (?1, ?2)", params![work_id, after])?;
    }
    Ok(())
}

impl Store {
    /// Replace the tasks `work_id` starts after (ids), unless the list is
    /// refused: then nothing changes and the answer says why.
    pub fn work_deps_set(&self, work_id: &str, after_ids: &[String]) -> Result<Option<String>, StoreError> {
        let mut conn = self.conn().lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(why) = refusal(&tx, work_id, after_ids)? {
            return Ok(Some(why));
        }
        replace(&tx, work_id, after_ids)?;
        tx.commit()?;
        Ok(None)
    }

    /// Enqueue `item` starting after `after_ids`, the row and the list in one
    /// transaction, unless the list is refused: then nothing is enqueued and
    /// the answer says why.
    pub fn work_queue_enqueue_after(&self, item: &WorkItem, after_ids: &[String]) -> Result<Option<String>, StoreError> {
        let mut conn = self.conn().lock().unwrap();
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        if let Some(why) = refusal(&tx, &item.id, after_ids)? {
            return Ok(Some(why));
        }
        insert_item(&tx, item)?;
        replace(&tx, &item.id, after_ids)?;
        tx.commit()?;
        Ok(None)
    }

    /// The tasks `work_id` starts after.
    pub fn work_deps_of(&self, work_id: &str) -> Result<Vec<AfterTask>, StoreError> {
        let conn = self.conn().lock().unwrap();
        Ok(after_of(&conn, &[work_id])?.remove(work_id).unwrap_or_default())
    }

    /// The tasks that start after `work_id`.
    pub fn work_followers_of(&self, work_id: &str) -> Result<Vec<AfterTask>, StoreError> {
        let conn = self.conn().lock().unwrap();
        Ok(followers_of(&conn, &[work_id])?.remove(work_id).unwrap_or_default())
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
