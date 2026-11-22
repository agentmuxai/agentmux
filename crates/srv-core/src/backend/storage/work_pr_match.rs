// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! Which card a pull request belongs to, and the list of those that belong to none
//! (the Kanban and GitHub spec, §4).
//!
//! Our pull requests carry two hints: the **task reference** (`#12`, or `board#12`) and the **agent tag** (the
//! id of the agent that opened it). A task number repeats across boards and computers, so the match never
//! guesses: it names exactly one card of **this** computer, or says why it names none. The rules, the first
//! that applies deciding:
//!
//! 1. the card already has the pull request (and it was not taken off the card);
//! 2. a full reference `board#12`: task 12 of that board, if the agent holds or last held it;
//! 3. a bare `#12`: the tasks numbered 12 that the agent holds or held in the last 14 days, across boards:
//!    exactly one is the card; two or more, or none, is unmatched;
//! 4. a card that names a repository is matched only to a pull request of that repository.
//!
//! A pull request that no rule decides is kept in `db_work_pr_unmatched` for a person to link by hand: the
//! last 50, a closed one forgotten after a week.

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::store::Store;
use super::work_card_prs::github_repo;
use super::StoreError;

/// The table, created with the board's (`work_board::SCHEMA`).
pub(crate) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS db_work_pr_unmatched (
        repo       TEXT NOT NULL,
        number     INTEGER NOT NULL,
        state      TEXT NOT NULL DEFAULT 'open',
        agent_id   TEXT NOT NULL DEFAULT '',
        task_ref   TEXT NOT NULL DEFAULT '',
        why        TEXT NOT NULL DEFAULT '',
        head       TEXT NOT NULL DEFAULT '',
        updated_at INTEGER NOT NULL DEFAULT 0,
        first_seen INTEGER NOT NULL DEFAULT 0,
        last_seen  INTEGER NOT NULL DEFAULT 0,
        PRIMARY KEY (repo, number)
    );";

/// How long a task still counts as the agent's after it was last theirs.
pub const HELD_WINDOW_MS: i64 = 14 * 24 * 60 * 60 * 1000;
/// How many unmatched pull requests are kept.
pub const UNMATCHED_KEPT: i64 = 50;
/// How long a closed or merged one is kept.
pub const UNMATCHED_CLOSED_KEPT_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// The task a pull request names: `#12` or `12` (any board), or `work#12` (that board).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskRef {
    Bare(i64),
    Board { board: String, num: i64 },
}

impl TaskRef {
    /// `#12`, `12`, `work#12`; nothing else.
    pub fn parse(s: &str) -> Option<TaskRef> {
        let s = s.trim();
        let (board, n) = match s.split_once('#') {
            Some((b, n)) => (b.trim(), n),
            None => ("", s),
        };
        let num: i64 = n.trim().parse().ok().filter(|n| *n > 0)?;
        if board.is_empty() {
            return Some(TaskRef::Bare(num));
        }
        let ok = board.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
        ok.then(|| TaskRef::Board { board: board.to_ascii_lowercase(), num })
    }
}

/// An agent of this computer, as the tag on a pull request names it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentRef {
    /// The agent's id (what a task's `claimed_by` holds).
    pub id: String,
    pub uid: String,
}

/// Why a pull request matched no card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Why {
    /// It carries no task reference.
    NoReference,
    /// It carries no agent tag, or the tag names no agent of this computer.
    NoAgent,
    /// The agent holds, and held in the last 14 days, no task with that number.
    NoTask,
    /// Two or more tasks of the agent have that number.
    Ambiguous,
    /// The pull request was not opened by AgentMux's own GitHub identity, so a tag on it proves nothing.
    Outsider,
}

impl Why {
    pub fn as_str(self) -> &'static str {
        match self {
            Why::NoReference => "no task reference",
            Why::NoAgent => "no agent of this computer",
            Why::NoTask => "no task of that agent with that number",
            Why::Ambiguous => "more than one task of that agent has that number",
            Why::Outsider => "not opened by AgentMux's own GitHub identity",
        }
    }
}

/// What the match decided.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Matched {
    /// The card (its work id).
    Card(String),
    Unmatched(Why),
}

/// A pull request no rule linked, for a person to link by hand.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct UnmatchedPr {
    pub repo: String,
    pub number: u64,
    pub url: String,
    /// `open`, `merged` or `closed`.
    pub state: String,
    /// The agent the tag named, when it is one of this computer's.
    pub agent_id: String,
    pub task_ref: String,
    pub why: String,
    pub updated_at: i64,
    pub last_seen: i64,
}

fn card_names_another_repo(card_repo: &str, pr_repo: &str) -> bool {
    github_repo(card_repo).is_some_and(|r| !r.eq_ignore_ascii_case(pr_repo))
}

/// The tasks numbered `num` (on `board`, when one is named) that `agent` holds, or held and touched since
/// `since_ms`, not counting a card that took this pull request off, nor one that names another repository.
fn candidates(conn: &Connection, repo: &str, number: u64, num: i64, board: Option<&str>, agent: &AgentRef, since_ms: i64) -> Result<Vec<String>, StoreError> {
    let mut stmt = conn.prepare(
        "SELECT q.id, c.repo FROM db_work_queue q
           JOIN db_work_cards c ON c.work_id = q.id
          WHERE c.board_num = ?1
            AND (?2 IS NULL OR c.board_id IN (SELECT id FROM db_boards WHERE slug = ?2 COLLATE NOCASE))
            AND ((q.claimed_by_uid <> '' AND q.claimed_by_uid = ?3) OR (q.claimed_by <> '' AND q.claimed_by = ?4 COLLATE NOCASE))
            AND (q.state IN ('claimed', 'blocked', 'review') OR q.updated_at >= ?5)
            AND NOT EXISTS (SELECT 1 FROM db_work_card_prs x
                             WHERE x.work_id = q.id AND x.repo = ?6 AND x.number = ?7 AND x.unlinked = 1)",
    )?;
    let rows = stmt.query_map(params![num, board, agent.uid, agent.id, since_ms, repo, number as i64], |r| {
        Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
    })?;
    let mut out = Vec::new();
    for row in rows {
        let (id, card_repo) = row?;
        if !card_names_another_repo(&card_repo, repo) {
            out.push(id);
        }
    }
    Ok(out)
}

impl Store {
    /// Which card the pull request `repo#number` belongs to, from what it carries: a task reference and the
    /// agent it names (`agent`, when that is an agent of this computer). It never guesses.
    pub fn work_pr_match(
        &self,
        repo: &str,
        number: u64,
        task_ref: Option<&TaskRef>,
        agent: Option<&AgentRef>,
        now_ms: i64,
    ) -> Result<Matched, StoreError> {
        let conn = self.conn().lock().unwrap();
        // 1. A card that already has it.
        let linked: Option<String> = conn
            .query_row(
                "SELECT work_id FROM db_work_card_prs WHERE repo = ?1 AND number = ?2 AND unlinked = 0 ORDER BY linked_at DESC LIMIT 1",
                params![repo, number as i64],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(id) = linked {
            return Ok(Matched::Card(id));
        }
        let Some(task_ref) = task_ref else { return Ok(Matched::Unmatched(Why::NoReference)) };
        let Some(agent) = agent else { return Ok(Matched::Unmatched(Why::NoAgent)) };
        let since = now_ms - HELD_WINDOW_MS;
        // 2 and 3. The number, on the named board or on any, among the agent's own tasks.
        let (num, board) = match task_ref {
            TaskRef::Bare(n) => (*n, None),
            TaskRef::Board { board, num } => (*num, Some(board.as_str())),
        };
        let mut found = candidates(&conn, repo, number, num, board, agent, since)?;
        found.sort();
        found.dedup();
        Ok(match found.as_slice() {
            [one] => Matched::Card(one.clone()),
            [] => Matched::Unmatched(Why::NoTask),
            _ => Matched::Unmatched(Why::Ambiguous),
        })
    }

    /// Note a pull request that matched no card, for a person to link (a newer note replaces the older).
    #[allow(clippy::too_many_arguments)]
    pub fn work_pr_unmatched_note(
        &self,
        repo: &str,
        number: u64,
        state: &str,
        agent_id: &str,
        task_ref: &str,
        why: Why,
        head: &str,
        updated_at: i64,
        now_ms: i64,
    ) -> Result<(), StoreError> {
        let mut conn = self.conn().lock().unwrap();
        let tx = conn.transaction()?;
        tx.execute(
            "INSERT INTO db_work_pr_unmatched (repo, number, state, agent_id, task_ref, why, head, updated_at, first_seen, last_seen)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?9)
             ON CONFLICT(repo, number) DO UPDATE SET
                 state = excluded.state, agent_id = excluded.agent_id, task_ref = excluded.task_ref, why = excluded.why,
                 head = excluded.head, updated_at = excluded.updated_at, last_seen = excluded.last_seen
              WHERE excluded.updated_at > db_work_pr_unmatched.updated_at",
            params![repo, number as i64, state, agent_id, task_ref, why.as_str(), head, updated_at, now_ms],
        )?;
        // The last 50 stay; a closed one goes after a week.
        tx.execute(
            "DELETE FROM db_work_pr_unmatched WHERE state <> 'open' AND last_seen < ?1",
            params![now_ms - UNMATCHED_CLOSED_KEPT_MS],
        )?;
        tx.execute(
            "DELETE FROM db_work_pr_unmatched WHERE rowid NOT IN
                 (SELECT rowid FROM db_work_pr_unmatched ORDER BY last_seen DESC, rowid DESC LIMIT ?1)",
            params![UNMATCHED_KEPT],
        )?;
        tx.commit()?;
        Ok(())
    }

    /// The pull requests no card has, newest first.
    pub fn work_pr_unmatched_list(&self) -> Result<Vec<UnmatchedPr>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT repo, number, state, agent_id, task_ref, why, updated_at, last_seen
               FROM db_work_pr_unmatched ORDER BY last_seen DESC, rowid DESC",
        )?;
        let rows = stmt.query_map([], |r| {
            let (repo, number): (String, i64) = (r.get(0)?, r.get(1)?);
            Ok(UnmatchedPr {
                url: super::work_card_prs::pr_url(&repo, number as u64),
                repo,
                number: number as u64,
                state: r.get(2)?,
                agent_id: r.get(3)?,
                task_ref: r.get(4)?,
                why: r.get(5)?,
                updated_at: r.get(6)?,
                last_seen: r.get(7)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Take a pull request off the unmatched list (it was linked, or a person dismissed it). Returns whether it
    /// was there.
    pub fn work_pr_unmatched_remove(&self, repo: &str, number: u64) -> Result<bool, StoreError> {
        let conn = self.conn().lock().unwrap();
        Ok(conn.execute("DELETE FROM db_work_pr_unmatched WHERE repo = ?1 AND number = ?2", params![repo, number as i64])? > 0)
    }
}

#[cfg(test)]
#[path = "work_pr_match_tests.rs"]
mod tests;
