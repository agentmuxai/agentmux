// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The Kanban side of the task queue (docs/specs/SPEC_WARDEN_KANBAN_2026_10_20.md).
//!
//! A task is still one `db_work_queue` row; this module adds what the board
//! needs beside it, in tables of its own so older builds on the same computer
//! (which share this store) never see them:
//!
//! - `db_work_cards`: one per task. Its short number (`#12`), the repository,
//!   branch and PR, the operator's order in the backlog (`rank`), whether a
//!   person must accept it, the agent's checklist, note and question, and who
//!   asked for it.
//! - `db_work_events`: the task's history, one row per move, note, question,
//!   answer or link.
//!
//! And the transitions the board adds to the queue's own (`work_queue.rs`):
//! hold and unhold (by the operator or an agent), a holder pausing its own
//! task, claiming one named task, assign, ask and answer, submit, accept and
//! send back, retry, and start (a claim made for an agent by the operator).
//! Each is one
//! conditional UPDATE, like the queue's, so it can't race another.

use rusqlite::{params, OptionalExtension, Row};
use serde::{Deserialize, Serialize};

use super::store::Store;
use super::work_deps::{attach_after, AfterTask, WAITS_ON_UNFINISHED};
use super::work_queue::{holder_update, row_to_item, work_state, ClaimFilter, HolderMatch, WorkItem, COLS};
use super::StoreError;

/// The tables, created on every open of the identity store, with no schema
/// version bump (see `work_claims::SCHEMA`).
pub(crate) const SCHEMA: &str = "
    CREATE TABLE IF NOT EXISTS db_work_cards (
        num               INTEGER PRIMARY KEY AUTOINCREMENT,
        work_id           TEXT NOT NULL UNIQUE,
        repo              TEXT NOT NULL DEFAULT '',
        branch            TEXT NOT NULL DEFAULT '',
        pr_url            TEXT NOT NULL DEFAULT '',
        rank              REAL,
        review_required   INTEGER NOT NULL DEFAULT 0,
        checklist         TEXT NOT NULL DEFAULT '',
        question          TEXT NOT NULL DEFAULT '',
        note              TEXT NOT NULL DEFAULT '',
        requested_by      TEXT NOT NULL DEFAULT '',
        requested_by_kind TEXT NOT NULL DEFAULT '',
        request_text      TEXT NOT NULL DEFAULT '',
        filed_by          TEXT NOT NULL DEFAULT '',
        last_reminded_at  INTEGER,
        updated_at        INTEGER NOT NULL DEFAULT 0
    );
    CREATE TABLE IF NOT EXISTS db_work_events (
        id          INTEGER PRIMARY KEY AUTOINCREMENT,
        work_id     TEXT NOT NULL,
        at          INTEGER NOT NULL,
        actor       TEXT NOT NULL DEFAULT '',
        actor_kind  TEXT NOT NULL DEFAULT '',
        kind        TEXT NOT NULL,
        from_state  TEXT NOT NULL DEFAULT '',
        to_state    TEXT NOT NULL DEFAULT '',
        body        TEXT NOT NULL DEFAULT ''
    );
    CREATE INDEX IF NOT EXISTS idx_ids_work_events_work ON db_work_events(work_id, at);";

/// How long a finished task's history is kept after it last changed.
pub const EVENTS_RETENTION_MS: i64 = 30 * 24 * 60 * 60 * 1000;

/// Who asked for a task (`requested_by_kind`).
pub mod requested {
    /// The operator: in the Kanban, or by a message that started (or joined)
    /// the turn in which an agent filed it.
    pub const OPERATOR: &str = "operator";
    /// Another agent's message started that turn.
    pub const AGENT: &str = "agent";
    /// A schedule, a service notice or AgentMux itself.
    pub const OTHER: &str = "other";
    /// srv couldn't tell (the call carried no agent token).
    pub const UNKNOWN: &str = "unknown";
}

/// One step of a task's checklist; `status` is `pending`, `in_progress` or
/// `completed`, as the providers write it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChecklistItem {
    pub text: String,
    pub status: String,
}

/// The board's fields of one task.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct WorkCard {
    /// The task's short number, `#num`.
    pub num: i64,
    pub repo: String,
    pub branch: String,
    pub pr_url: String,
    /// The operator's order in the backlog, in `created_at` units; `None`
    /// means by when it was filed.
    pub rank: Option<f64>,
    pub review_required: bool,
    pub checklist: Vec<ChecklistItem>,
    /// The holder's open question while the task is `blocked`.
    pub question: String,
    /// The holder's last progress note.
    pub note: String,
    /// Who asked: the operator, or the agent whose message started the turn.
    pub requested_by: String,
    /// One of [`requested`].
    pub requested_by_kind: String,
    /// The operator's own words, when an agent filed the task during a turn
    /// the operator started.
    pub request_text: String,
    /// The agent that wrote the task down (empty when the operator did).
    pub filed_by: String,
    pub last_reminded_at: Option<i64>,
    pub updated_at: i64,
    /// The tasks this one starts after (`work_deps.rs`), finished or not.
    #[serde(default)]
    pub after: Vec<AfterTask>,
    /// The tasks that start after this one, finished or not.
    #[serde(default)]
    pub followers: Vec<AfterTask>,
    /// What GitHub said about its PR: merged when, released in which
    /// release (`work_prs.rs`). `None` until srv has looked.
    #[serde(default)]
    pub pr: Option<super::work_prs::WorkPr>,
    /// The path of the document it wrote (`links.doc`), empty when none.
    #[serde(default)]
    pub doc: String,
}

/// A task as the board shows it: the queue row and its card.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BoardTask {
    #[serde(flatten)]
    pub item: WorkItem,
    pub card: WorkCard,
}

/// One line of a task's history.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkEvent {
    pub id: i64,
    pub work_id: String,
    pub at: i64,
    /// Who: an agent's name, `operator`, or `agentmux`.
    pub actor: String,
    /// `agent`, `operator`, `system`, or `delegate` (an agent deciding on a
    /// task in review).
    pub actor_kind: String,
    /// `created`, `filed_for_later`, `claimed`, `started`, `held`, `unheld`,
    /// `assigned`, `ranked`, `edited`, `updated`, `asked`, `answered`,
    /// `submitted`, `completed`, `accepted`, `sent_back`, `released`,
    /// `expired`, `cancelled`, `retried`, `linked`, `reminded`, `tracked`.
    pub kind: String,
    pub from_state: String,
    pub to_state: String,
    pub body: String,
}

/// What a card is created with. Everything else starts empty.
#[derive(Debug, Clone, Default)]
pub struct NewCard {
    pub repo: String,
    pub branch: String,
    pub review_required: bool,
    pub requested_by: String,
    pub requested_by_kind: String,
    pub request_text: String,
    pub filed_by: String,
}

/// The card fields an agent's `update` or `submit` may set; `None` leaves one
/// as it is.
#[derive(Debug, Clone, Default)]
pub struct CardUpdate {
    pub note: Option<String>,
    pub checklist: Option<Vec<ChecklistItem>>,
    pub branch: Option<String>,
    pub pr_url: Option<String>,
    /// The path of the document the task wrote (`db_work_docs`).
    pub doc: Option<String>,
}

const CARD_COLS: &str = "c.num, c.repo, c.branch, c.pr_url, c.rank, c.review_required, c.checklist, \
                         c.question, c.note, c.requested_by, c.requested_by_kind, c.request_text, \
                         c.filed_by, c.last_reminded_at, c.updated_at";

fn row_to_card(row: &Row) -> rusqlite::Result<WorkCard> {
    let checklist: String = row.get("checklist")?;
    Ok(WorkCard {
        num: row.get("num")?,
        repo: row.get("repo")?,
        branch: row.get("branch")?,
        pr_url: row.get("pr_url")?,
        rank: row.get("rank")?,
        review_required: row.get::<_, i64>("review_required")? != 0,
        checklist: serde_json::from_str(&checklist).unwrap_or_default(),
        question: row.get("question")?,
        note: row.get("note")?,
        requested_by: row.get("requested_by")?,
        requested_by_kind: row.get("requested_by_kind")?,
        request_text: row.get("request_text")?,
        filed_by: row.get("filed_by")?,
        last_reminded_at: row.get("last_reminded_at")?,
        updated_at: row.get("updated_at")?,
        after: Vec::new(),
        followers: Vec::new(),
        pr: None,
        doc: String::new(),
    })
}

fn row_to_event(row: &Row) -> rusqlite::Result<WorkEvent> {
    Ok(WorkEvent {
        id: row.get("id")?,
        work_id: row.get("work_id")?,
        at: row.get("at")?,
        actor: row.get("actor")?,
        actor_kind: row.get("actor_kind")?,
        kind: row.get("kind")?,
        from_state: row.get("from_state")?,
        to_state: row.get("to_state")?,
        body: row.get("body")?,
    })
}

/// The queue's columns, qualified for a join with `db_work_cards c`.
fn q_cols() -> String {
    COLS.split(',').map(|c| format!("q.{}", c.trim())).collect::<Vec<_>>().join(", ")
}

/// A transition's result: the task's state before, and after.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Moved {
    pub from: String,
    pub to: String,
}

impl Store {
    /// The card of `work_id`, created empty when the task has none (a task an
    /// older build enqueued). Returns its number.
    pub fn work_card_ensure(&self, work_id: &str, now_ms: i64) -> Result<i64, StoreError> {
        let conn = self.conn().lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO db_work_cards (work_id, updated_at) VALUES (?1, ?2)",
            params![work_id, now_ms],
        )?;
        Ok(conn.query_row("SELECT num FROM db_work_cards WHERE work_id = ?1", params![work_id], |r| r.get(0))?)
    }

    /// Create the card of a task just enqueued, or fill the empty one a claim
    /// or a list made for it meanwhile (`work_card_ensure`). Returns its number.
    pub fn work_card_create(&self, work_id: &str, card: &NewCard, now_ms: i64) -> Result<i64, StoreError> {
        let conn = self.conn().lock().unwrap();
        conn.execute(
            "INSERT INTO db_work_cards
                (work_id, repo, branch, review_required, requested_by, requested_by_kind,
                 request_text, filed_by, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)
             ON CONFLICT(work_id) DO UPDATE SET
                repo = excluded.repo, branch = excluded.branch, review_required = excluded.review_required,
                requested_by = excluded.requested_by, requested_by_kind = excluded.requested_by_kind,
                request_text = excluded.request_text, filed_by = excluded.filed_by, updated_at = excluded.updated_at",
            params![
                work_id,
                card.repo,
                card.branch,
                card.review_required as i64,
                card.requested_by,
                card.requested_by_kind,
                card.request_text,
                card.filed_by,
                now_ms
            ],
        )?;
        Ok(conn.query_row("SELECT num FROM db_work_cards WHERE work_id = ?1", params![work_id], |r| r.get(0))?)
    }

    /// The task with this short number, or this id.
    pub fn work_board_get(&self, id_or_num: &str) -> Result<Option<BoardTask>, StoreError> {
        let num: Option<i64> = id_or_num.trim().trim_start_matches('#').parse().ok();
        let conn = self.conn().lock().unwrap();
        let sql = format!(
            "SELECT {q}, {CARD_COLS} FROM db_work_queue q JOIN db_work_cards c ON c.work_id = q.id
              WHERE q.id = ?1 OR c.num = ?2",
            q = q_cols()
        );
        let Some(t) = conn
            .query_row(&sql, params![id_or_num.trim(), num.unwrap_or(-1)], |r| {
                Ok(BoardTask { item: row_to_item(r)?, card: row_to_card(r)? })
            })
            .optional()?
        else {
            return Ok(None);
        };
        let mut one = [t];
        attach_after(&conn, &mut one)?;
        super::work_prs::attach_prs(&conn, &mut one)?;
        let [t] = one;
        Ok(Some(t))
    }

    /// The tasks the board shows: every one not finished, and the finished
    /// ones (done, cancelled, failed) that changed since `finished_since_ms`.
    /// Gives every task a card first, so older builds' tasks get numbers.
    pub fn work_board_list(&self, finished_since_ms: i64, limit: usize, now_ms: i64) -> Result<Vec<BoardTask>, StoreError> {
        let conn = self.conn().lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO db_work_cards (work_id, updated_at)
             SELECT id, ?1 FROM db_work_queue WHERE id NOT IN (SELECT work_id FROM db_work_cards)",
            params![now_ms],
        )?;
        let sql = format!(
            "SELECT {q}, {CARD_COLS} FROM db_work_queue q JOIN db_work_cards c ON c.work_id = q.id
              WHERE q.state NOT IN ('{done}', '{cancelled}', '{failed}') OR q.updated_at >= ?1
              ORDER BY q.updated_at DESC
              LIMIT ?2",
            q = q_cols(),
            done = work_state::DONE,
            cancelled = work_state::CANCELLED,
            failed = work_state::FAILED,
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![finished_since_ms, limit as i64], |r| {
            Ok(BoardTask { item: row_to_item(r)?, card: row_to_card(r)? })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        attach_after(&conn, &mut out)?;
        super::work_prs::attach_prs(&conn, &mut out)?;
        Ok(out)
    }

    /// The newest change to any task, for a board in another instance that
    /// didn't hear the event (`0` when there are none).
    pub fn work_board_stamp(&self) -> Result<i64, StoreError> {
        let conn = self.conn().lock().unwrap();
        let q: i64 = conn.query_row("SELECT COALESCE(MAX(updated_at), 0) FROM db_work_queue", [], |r| r.get(0))?;
        let c: i64 = conn.query_row("SELECT COALESCE(MAX(updated_at), 0) FROM db_work_cards", [], |r| r.get(0))?;
        Ok(q.max(c))
    }

    /// The tasks waiting for this agent (open, targeted at it by UID or by
    /// name, claimable now), in the order it will take them: the claim's
    /// order (§6.5).
    pub fn work_board_waiting_for(&self, agent_id: &str, uid: &str, limit: usize, now_ms: i64) -> Result<Vec<BoardTask>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let sql = format!(
            "SELECT {q}, {CARD_COLS} FROM db_work_queue q JOIN db_work_cards c ON c.work_id = q.id
              WHERE q.state = '{open}' AND q.attempts < q.max_attempts
                AND (q.not_before IS NULL OR q.not_before <= ?4)
                AND NOT {WAITS_ON_UNFINISHED}
                AND ((q.target_agent_uid <> '' AND q.target_agent_uid = ?2)
                     OR (q.target_agent_uid = '' AND q.target_agent <> '' AND q.target_agent = ?1))
              ORDER BY q.priority DESC, COALESCE(c.rank, q.created_at) ASC
              LIMIT ?3",
            q = q_cols(),
            open = work_state::OPEN,
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params![agent_id, uid, limit as i64, now_ms], |r| {
            Ok(BoardTask { item: row_to_item(r)?, card: row_to_card(r)? })
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Add one line to a task's history.
    pub fn work_event_add(&self, ev: &WorkEvent) -> Result<(), StoreError> {
        let conn = self.conn().lock().unwrap();
        conn.execute(
            "INSERT INTO db_work_events (work_id, at, actor, actor_kind, kind, from_state, to_state, body)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            params![ev.work_id, ev.at, ev.actor, ev.actor_kind, ev.kind, ev.from_state, ev.to_state, ev.body],
        )?;
        Ok(())
    }

    /// A task's history, oldest first (the newest `limit`).
    pub fn work_events(&self, work_id: &str, limit: usize) -> Result<Vec<WorkEvent>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT * FROM (SELECT id, work_id, at, actor, actor_kind, kind, from_state, to_state, body
                              FROM db_work_events WHERE work_id = ?1 ORDER BY at DESC, id DESC LIMIT ?2)
              ORDER BY at ASC, id ASC",
        )?;
        let rows = stmt.query_map(params![work_id, limit as i64], row_to_event)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Forget the history of tasks finished more than [`EVENTS_RETENTION_MS`]
    /// ago. Returns how many lines went.
    pub fn work_events_prune(&self, now_ms: i64) -> Result<usize, StoreError> {
        let conn = self.conn().lock().unwrap();
        Ok(conn.execute(
            &format!(
                "DELETE FROM db_work_events WHERE work_id IN (
                    SELECT id FROM db_work_queue
                     WHERE state IN ('{done}', '{cancelled}', '{failed}') AND updated_at < ?1)",
                done = work_state::DONE,
                cancelled = work_state::CANCELLED,
                failed = work_state::FAILED,
            ),
            params![now_ms - EVENTS_RETENTION_MS],
        )?)
    }

    /// Set the card fields an agent's `update` or `submit` carries.
    pub fn work_card_update(&self, work_id: &str, u: &CardUpdate, now_ms: i64) -> Result<(), StoreError> {
        let checklist = u.checklist.as_ref().map(|c| serde_json::to_string(c).unwrap_or_default());
        let mut conn = self.conn().lock().unwrap();
        let tx = conn.transaction()?;
        // A different PR: what was learned about the old one (its merge, its
        // release) isn't this one's.
        if let Some(pr) = &u.pr_url {
            tx.execute(
                "DELETE FROM db_work_prs WHERE work_id = ?1
                   AND EXISTS (SELECT 1 FROM db_work_cards WHERE work_id = ?1 AND pr_url <> ?2)",
                params![work_id, pr],
            )?;
        }
        tx.execute(
            "UPDATE db_work_cards
                SET note = COALESCE(?2, note),
                    checklist = COALESCE(?3, checklist),
                    branch = COALESCE(?4, branch),
                    pr_url = COALESCE(?5, pr_url),
                    updated_at = ?6
              WHERE work_id = ?1",
            params![work_id, u.note, checklist, u.branch, u.pr_url, now_ms],
        )?;
        tx.commit()?;
        drop(conn);
        if let Some(doc) = &u.doc {
            self.work_doc_set(work_id, doc, now_ms)?;
        }
        Ok(())
    }

    /// Set a card's question (`""` clears it).
    pub fn work_card_set_question(&self, work_id: &str, question: &str, now_ms: i64) -> Result<(), StoreError> {
        let conn = self.conn().lock().unwrap();
        conn.execute(
            "UPDATE db_work_cards SET question = ?2, updated_at = ?3 WHERE work_id = ?1",
            params![work_id, question, now_ms],
        )?;
        Ok(())
    }

    /// The operator's edits to a card: its order in the backlog, whether it
    /// needs review, its repository. `None` leaves one as it is.
    pub fn work_card_edit(
        &self,
        work_id: &str,
        rank: Option<f64>,
        review_required: Option<bool>,
        repo: Option<&str>,
        filed_by: Option<&str>,
        now_ms: i64,
    ) -> Result<(), StoreError> {
        let conn = self.conn().lock().unwrap();
        conn.execute(
            "UPDATE db_work_cards
                SET rank = COALESCE(?2, rank),
                    review_required = COALESCE(?3, review_required),
                    repo = COALESCE(?4, repo),
                    filed_by = COALESCE(?5, filed_by),
                    updated_at = ?6
              WHERE work_id = ?1",
            params![work_id, rank, review_required.map(|b| b as i64), repo, filed_by, now_ms],
        )?;
        Ok(())
    }

    /// Record that the waiting tasks of an agent were named in a reminder.
    pub fn work_cards_mark_reminded(&self, work_ids: &[String], now_ms: i64) -> Result<(), StoreError> {
        let conn = self.conn().lock().unwrap();
        for id in work_ids {
            conn.execute(
                "UPDATE db_work_cards SET last_reminded_at = ?2 WHERE work_id = ?1",
                params![id, now_ms],
            )?;
        }
        Ok(())
    }

    /// The operator's edits to the task itself. `None` leaves a field as it is.
    pub fn work_task_edit(
        &self,
        work_id: &str,
        title: Option<&str>,
        payload: Option<&str>,
        kind: Option<&str>,
        priority: Option<i64>,
        now_ms: i64,
    ) -> Result<bool, StoreError> {
        let conn = self.conn().lock().unwrap();
        Ok(conn.execute(
            "UPDATE db_work_queue
                SET title = COALESCE(?2, title), payload = COALESCE(?3, payload),
                    kind = COALESCE(?4, kind), priority = COALESCE(?5, priority), updated_at = ?6
              WHERE id = ?1",
            params![work_id, title, payload, kind, priority, now_ms],
        )? > 0)
    }

    /// A one-statement state change: `from` (any of) to `to`, with `set`
    /// (extra assignments) applied. Returns the state it moved from.
    fn work_move(
        &self,
        work_id: &str,
        from: &[&str],
        to: &str,
        set: &str,
        extra: &[&dyn rusqlite::ToSql],
        now_ms: i64,
    ) -> Result<Option<Moved>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let before: Option<String> = conn
            .query_row("SELECT state FROM db_work_queue WHERE id = ?1", params![work_id], |r| r.get(0))
            .optional()?;
        let Some(before) = before else { return Ok(None) };
        if !from.contains(&before.as_str()) {
            return Ok(None);
        }
        let set = if set.is_empty() { String::new() } else { format!(", {set}") };
        let sql = format!(
            "UPDATE db_work_queue SET state = ?1, updated_at = ?2{set} WHERE id = ?3 AND state = ?4"
        );
        let mut binds: Vec<&dyn rusqlite::ToSql> = vec![&to, &now_ms, &work_id, &before];
        binds.extend_from_slice(extra);
        let n = conn.execute(&sql, binds.as_slice())?;
        Ok((n > 0).then(|| Moved { from: before, to: to.to_string() }))
    }

    /// Put a waiting task on hold, so no agent takes it.
    pub fn work_hold(&self, work_id: &str, now_ms: i64) -> Result<Option<Moved>, StoreError> {
        self.work_move(work_id, &[work_state::OPEN], work_state::HELD, "", &[], now_ms)
    }

    /// Release a held task to whoever it is for.
    pub fn work_unhold(&self, work_id: &str, now_ms: i64) -> Result<Option<Moved>, StoreError> {
        self.work_move(work_id, &[work_state::HELD], work_state::OPEN, "", &[], now_ms)
    }

    /// The holder pauses its task (`Tasks` `hold` on its own claim,
    /// SPEC_KANBAN_AGENTS_HOLD_AND_UNHOLD_2026_10_22.md §3.2): `claimed` to
    /// `held`, the claim given up. Pausing isn't failing, so the attempt
    /// doesn't count against the task: its limit grows by one. `attempts`
    /// itself never goes down, since it is the fence: the next claim gets a
    /// new number, and a late call from this one is refused. A task it took
    /// from anyone or from a group becomes its own, so the pause comes back to
    /// it. Fenced like the queue's holder transitions.
    pub fn work_hold_own(
        &self,
        id: &str,
        agent_id: &str,
        caller_uid: &str,
        attempt: i64,
        now_ms: i64,
    ) -> Result<Option<HolderMatch>, StoreError> {
        let conn = self.conn().lock().unwrap();
        // Every right-hand side reads the row as it was, so the targets are
        // decided from the old (empty) ones and the old holder.
        holder_update(
            &conn,
            |holder| {
                format!(
                    "UPDATE db_work_queue
                        SET state = '{h}', max_attempts = max_attempts + 1,
                            target_agent = CASE WHEN target_agent = '' AND target_agent_uid = ''
                                                THEN claimed_by ELSE target_agent END,
                            target_agent_uid = CASE WHEN target_agent = '' AND target_agent_uid = ''
                                                    THEN claimed_by_uid ELSE target_agent_uid END,
                            target_group = '',
                            claimed_by = '', claimed_by_uid = '', claim_expires = NULL, updated_at = ?4
                      WHERE id = ?1 AND {holder} AND attempts = ?3 AND state = '{c}'",
                    h = work_state::HELD,
                    c = work_state::CLAIMED,
                )
            },
            params![id, agent_id, attempt, now_ms, "", caller_uid],
        )
    }

    /// An agent claims one named task (`Tasks` `claim` with `id`, §3.3 of the
    /// same spec): open, or held (claiming it lifts the hold), and only when
    /// [`Store::work_queue_claim`] would let this claimer take it: for it, for
    /// anyone or for one of its groups, of the asked-for kind if any,
    /// attempts left, not before its time, and not waiting on an unfinished
    /// task. One conditional UPDATE, so a
    /// task that moved meanwhile is never taken. Returns the claimed row and
    /// the state it was in.
    pub fn work_claim_one(
        &self,
        id: &str,
        filter: &ClaimFilter,
        now_ms: i64,
        lease_ms: i64,
    ) -> Result<Option<(WorkItem, String)>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let before: Option<String> = conn
            .query_row("SELECT state FROM db_work_queue WHERE id = ?1", params![id], |r| r.get(0))
            .optional()?;
        let Some(before) = before.filter(|s| s == work_state::OPEN || s == work_state::HELD) else {
            return Ok(None);
        };
        // Bound, never interpolated, as in the queue's own claim.
        let groups: String = if filter.groups.is_empty() {
            String::new()
        } else {
            let marks: Vec<String> = (0..filter.groups.len()).map(|i| format!("?{}", i + 8)).collect();
            format!(" OR q.target_group IN ({})", marks.join(","))
        };
        let sql = format!(
            "UPDATE db_work_queue AS q
                SET state = '{claimed}', claimed_by = ?1, claimed_by_uid = ?5, claim_expires = ?2,
                    attempts = attempts + 1, updated_at = ?3
              WHERE q.id = ?4 AND q.state = ?6
                AND q.attempts < q.max_attempts
                AND (q.not_before IS NULL OR q.not_before <= ?3)
                AND ((q.target_agent_uid <> '' AND q.target_agent_uid = ?5)
                     OR (q.target_agent_uid = '' AND (q.target_agent = '' OR q.target_agent = ?1)))
                AND (q.target_group = ''{groups})
                AND (?7 = '' OR q.kind = ?7)
                AND NOT {waits}
              RETURNING {COLS}",
            claimed = work_state::CLAIMED,
            waits = WAITS_ON_UNFINISHED,
        );
        let expires = now_ms + lease_ms;
        let kind = filter.kind.clone().unwrap_or_default();
        let mut binds: Vec<&dyn rusqlite::ToSql> =
            vec![&filter.agent_id, &expires, &now_ms, &id, &filter.agent_uid, &before, &kind];
        for g in &filter.groups {
            binds.push(g);
        }
        Ok(conn.query_row(&sql, binds.as_slice(), row_to_item).optional()?.map(|item| (item, before)))
    }

    /// Give a waiting task to an agent (`agent`, its UID when known), or to
    /// anyone (both empty). Only while it waits: open or held.
    pub fn work_assign(&self, work_id: &str, agent: &str, uid: &str, now_ms: i64) -> Result<bool, StoreError> {
        let conn = self.conn().lock().unwrap();
        Ok(conn.execute(
            &format!(
                "UPDATE db_work_queue SET target_agent = ?2, target_agent_uid = ?3, target_group = '', updated_at = ?4
                  WHERE id = ?1 AND state IN ('{open}', '{held}')",
                open = work_state::OPEN,
                held = work_state::HELD,
            ),
            params![work_id, agent, uid, now_ms],
        )? > 0)
    }

    /// The operator starts a waiting task on an agent: a claim made for it, a
    /// new attempt, the task given to that agent. Returns the claimed row.
    pub fn work_start_for(
        &self,
        work_id: &str,
        agent: &str,
        uid: &str,
        now_ms: i64,
        lease_ms: i64,
    ) -> Result<Option<WorkItem>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let sql = format!(
            "UPDATE db_work_queue
                SET state = '{claimed}', claimed_by = ?2, claimed_by_uid = ?3, claim_expires = ?4,
                    target_agent = ?2, target_agent_uid = ?3, target_group = '',
                    attempts = attempts + 1,
                    max_attempts = MAX(max_attempts, attempts + 1),
                    updated_at = ?5
              WHERE id = ?1 AND state IN ('{open}', '{held}')
              RETURNING {COLS}",
            claimed = work_state::CLAIMED,
            open = work_state::OPEN,
            held = work_state::HELD,
        );
        Ok(conn
            .query_row(&sql, params![work_id, agent, uid, now_ms + lease_ms, now_ms], row_to_item)
            .optional()?)
    }

    /// Undo [`Store::work_start_for`] when the task couldn't be delivered: it
    /// waits again as it did before (`state`, `target_agent`, its UID), and
    /// the attempt it was given is taken back. Only while that claim stands.
    pub fn work_unstart(
        &self,
        work_id: &str,
        agent: &str,
        attempt: i64,
        state: &str,
        target_agent: &str,
        target_uid: &str,
        now_ms: i64,
    ) -> Result<bool, StoreError> {
        let conn = self.conn().lock().unwrap();
        Ok(conn.execute(
            &format!(
                "UPDATE db_work_queue
                    SET state = ?4, attempts = attempts - 1, claimed_by = '', claimed_by_uid = '',
                        claim_expires = NULL, target_agent = ?5, target_agent_uid = ?6, updated_at = ?7
                  WHERE id = ?1 AND claimed_by = ?2 AND attempts = ?3 AND state = '{c}'",
                c = work_state::CLAIMED
            ),
            params![work_id, agent, attempt, state, target_agent, target_uid, now_ms],
        )? > 0)
    }

    /// The holder asks the operator something: `claimed` to `blocked`, lease
    /// paused. Fenced like the queue's holder transitions.
    pub fn work_ask(
        &self,
        id: &str,
        agent_id: &str,
        caller_uid: &str,
        attempt: i64,
        now_ms: i64,
    ) -> Result<Option<HolderMatch>, StoreError> {
        let conn = self.conn().lock().unwrap();
        holder_update(
            &conn,
            |holder| {
                format!(
                    "UPDATE db_work_queue SET state = '{b}', claim_expires = NULL, updated_at = ?4
                      WHERE id = ?1 AND {holder} AND attempts = ?3 AND state = '{c}'",
                    b = work_state::BLOCKED,
                    c = work_state::CLAIMED
                )
            },
            params![id, agent_id, attempt, now_ms, "", caller_uid],
        )
    }

    /// The holder hands the task in: `claimed` (or `blocked`) to `review`,
    /// keeping the holder for a send-back. Fenced.
    pub fn work_submit(
        &self,
        id: &str,
        agent_id: &str,
        caller_uid: &str,
        attempt: i64,
        result: &str,
        now_ms: i64,
    ) -> Result<Option<HolderMatch>, StoreError> {
        let conn = self.conn().lock().unwrap();
        holder_update(
            &conn,
            |holder| {
                format!(
                    "UPDATE db_work_queue SET state = '{r}', result = ?4, claim_expires = NULL, updated_at = ?5
                      WHERE id = ?1 AND {holder} AND attempts = ?3 AND state IN ('{c}', '{b}')",
                    r = work_state::REVIEW,
                    c = work_state::CLAIMED,
                    b = work_state::BLOCKED,
                )
            },
            params![id, agent_id, attempt, result, now_ms, caller_uid],
        )
    }

    /// The operator answered the holder's question: `blocked` back to
    /// `claimed`, with a fresh lease.
    pub fn work_answered(&self, work_id: &str, now_ms: i64, lease_ms: i64) -> Result<Option<Moved>, StoreError> {
        let expires = now_ms + lease_ms;
        self.work_move(work_id, &[work_state::BLOCKED], work_state::CLAIMED, "claim_expires = ?5", &[&expires], now_ms)
    }

    /// The operator accepts a handed-in task.
    pub fn work_accept(&self, work_id: &str, now_ms: i64) -> Result<Option<Moved>, StoreError> {
        self.work_move(work_id, &[work_state::REVIEW], work_state::DONE, "claim_expires = NULL", &[], now_ms)
    }

    /// The operator sends a handed-in task back to its holder, who still
    /// holds it (same attempt, fresh lease).
    pub fn work_send_back(&self, work_id: &str, now_ms: i64, lease_ms: i64) -> Result<Option<Moved>, StoreError> {
        let expires = now_ms + lease_ms;
        self.work_move(work_id, &[work_state::REVIEW], work_state::CLAIMED, "claim_expires = ?5", &[&expires], now_ms)
    }

    /// The operator sends a handed-in task back when its holder is gone: it
    /// waits again, for that agent.
    pub fn work_send_back_waiting(&self, work_id: &str, now_ms: i64) -> Result<Option<Moved>, StoreError> {
        self.work_move(
            work_id,
            &[work_state::REVIEW, work_state::BLOCKED],
            work_state::OPEN,
            "target_agent = claimed_by, target_agent_uid = claimed_by_uid, claimed_by = '', claimed_by_uid = '', \
             claim_expires = NULL, max_attempts = MAX(max_attempts, attempts + 1)",
            &[],
            now_ms,
        )
    }

    /// Whether one of the tasks numbered `nums` is waiting on `agent`: open,
    /// on hold, claimed or blocked, and targeted at it or held by it (by
    /// UID, else by name). A message that names such a task is handing the
    /// agent work (`backend::cold_hold`).
    pub fn work_numbers_waiting_on(&self, nums: &[i64], agent_uid: &str, agent_slug: &str) -> Result<bool, StoreError> {
        if nums.is_empty() {
            return Ok(false);
        }
        let conn = self.conn().lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT 1 FROM db_work_queue q JOIN db_work_cards c ON c.work_id = q.id
              WHERE c.num = ?1 AND q.state IN ('open', 'held', 'claimed', 'blocked')
                AND (q.target_agent_uid = ?2 OR q.claimed_by_uid = ?2
                     OR (q.target_agent_uid = '' AND ?3 != '' AND q.target_agent = ?3)
                     OR (q.claimed_by_uid = '' AND ?3 != '' AND q.claimed_by = ?3))
              LIMIT 1",
        )?;
        for n in nums {
            if stmt.exists(params![n, agent_uid, agent_slug])? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Whether `filer` filed a task for `agent` at or after `since_ms` that is
    /// still waiting or being worked (open, on hold, claimed or blocked). A
    /// message from the filer right after is the nudge that task needs
    /// (`backend::cold_hold`).
    pub fn work_filed_for_since(&self, filer_uid: &str, filer_name: &str, agent_uid: &str, agent_slug: &str, since_ms: i64) -> Result<bool, StoreError> {
        let conn = self.conn().lock().unwrap();
        let found: Option<i64> = conn
            .query_row(
                "SELECT 1 FROM db_work_queue q
                  WHERE q.created_at >= ?1 AND q.state IN ('open', 'held', 'claimed', 'blocked')
                    AND ((?2 != '' AND q.created_by_uid = ?2) OR (q.created_by_uid = '' AND ?3 != '' AND q.created_by = ?3))
                    AND (q.target_agent_uid = ?4 OR (q.target_agent_uid = '' AND ?5 != '' AND q.target_agent = ?5))
                  LIMIT 1",
                params![since_ms, filer_uid, filer_name, agent_uid, agent_slug],
                |r| r.get(0),
            )
            .optional()?;
        Ok(found.is_some())
    }

    /// The tasks whose latest acceptance, since `since_ms`, was an agent's:
    /// work id to (the agent, the history line's body, when).
    pub fn work_delegate_accepts(&self, since_ms: i64) -> Result<std::collections::HashMap<String, (String, String, i64)>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT e.work_id, e.actor, e.body, e.at FROM db_work_events e
              WHERE e.kind = 'accepted' AND e.actor_kind = 'delegate' AND e.at >= ?1
                AND e.id = (SELECT MAX(id) FROM db_work_events WHERE work_id = e.work_id AND kind = 'accepted')",
        )?;
        let rows = stmt.query_map(params![since_ms], |r| Ok((r.get::<_, String>(0)?, (r.get(1)?, r.get(2)?, r.get(3)?))))?;
        rows.collect::<Result<_, _>>().map_err(Into::into)
    }

    /// Take back an agent's acceptance: Done to Review again.
    pub fn work_reopen(&self, work_id: &str, now_ms: i64) -> Result<Option<Moved>, StoreError> {
        self.work_move(work_id, &[work_state::DONE], work_state::REVIEW, "", &[], now_ms)
    }

    /// The operator tries a failed task again: it waits, with its attempts
    /// reset.
    pub fn work_retry(&self, work_id: &str, now_ms: i64) -> Result<Option<Moved>, StoreError> {
        self.work_move(
            work_id,
            &[work_state::FAILED],
            work_state::OPEN,
            "attempts = 0, claimed_by = '', claimed_by_uid = '', claim_expires = NULL",
            &[],
            now_ms,
        )
    }

    /// The operator takes a claimed or blocked task back from its holder
    /// (R5 of docs/specs/SPEC_UNRESPONSIVE_AGENTS_2026_10_23.md): it waits again,
    /// its claim and question gone. With `retarget` it is given to that agent
    /// (name, UID; both empty for anyone) in the same step, so no claim lands
    /// between. Taking back isn't the task failing: the attempt doesn't count
    /// against it (its limit grows by one). `attempts` stays as the fence: the
    /// next claim gets a new number, and a late call from the old holder is
    /// refused twice over, since the task no longer says it holds it. Returns
    /// the row as it now is.
    pub fn work_take_back(
        &self,
        work_id: &str,
        retarget: Option<(&str, &str)>,
        reason: &str,
        now_ms: i64,
    ) -> Result<Option<WorkItem>, StoreError> {
        let (set, agent, uid) = match retarget {
            Some((agent, uid)) => (true, agent, uid),
            None => (false, "", ""),
        };
        let conn = self.conn().lock().unwrap();
        let sql = format!(
            "UPDATE db_work_queue
                SET state = '{open}', claimed_by = '', claimed_by_uid = '', claim_expires = NULL,
                    max_attempts = max_attempts + 1,
                    target_agent = CASE WHEN ?2 THEN ?3 ELSE target_agent END,
                    target_agent_uid = CASE WHEN ?2 THEN ?4 ELSE target_agent_uid END,
                    target_group = CASE WHEN ?2 THEN '' ELSE target_group END,
                    result = ?5, updated_at = ?6
              WHERE id = ?1 AND state IN ('{claimed}', '{blocked}')
              RETURNING {COLS}",
            open = work_state::OPEN,
            claimed = work_state::CLAIMED,
            blocked = work_state::BLOCKED,
        );
        let item = conn
            .query_row(&sql, params![work_id, set, agent, uid, reason, now_ms], row_to_item)
            .optional()?;
        if item.is_some() {
            conn.execute("UPDATE db_work_cards SET question = '', updated_at = ?2 WHERE work_id = ?1", params![work_id, now_ms])?;
        }
        Ok(item)
    }

    /// Extend the lease of a claim whose holder srv knows is alive (§6.8:
    /// activity is the heartbeat). Only the live claim, never a newer one.
    pub fn work_extend_lease(&self, work_id: &str, attempt: i64, now_ms: i64, lease_ms: i64) -> Result<bool, StoreError> {
        let conn = self.conn().lock().unwrap();
        Ok(conn.execute(
            &format!(
                "UPDATE db_work_queue SET claim_expires = ?3
                  WHERE id = ?1 AND attempts = ?2 AND state = '{c}'
                    AND (claim_expires IS NULL OR claim_expires < ?3)",
                c = work_state::CLAIMED
            ),
            params![work_id, attempt, now_ms + lease_ms],
        )? > 0)
    }

    /// Every claimed task, for the liveness check.
    pub fn work_claimed_all(&self) -> Result<Vec<WorkItem>, StoreError> {
        let conn = self.conn().lock().unwrap();
        let mut stmt = conn.prepare(&format!("SELECT {COLS} FROM db_work_queue WHERE state = ?1"))?;
        let rows = stmt.query_map(params![work_state::CLAIMED], row_to_item)?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Whether this agent released (or let expire) this task in the last
    /// `window_ms`: a reminder must not offer it again (§10).
    pub fn work_recently_released_by(&self, work_id: &str, agent: &str, now_ms: i64, window_ms: i64) -> Result<bool, StoreError> {
        let conn = self.conn().lock().unwrap();
        let n: i64 = conn.query_row(
            "SELECT COUNT(*) FROM db_work_events
              WHERE work_id = ?1 AND actor = ?2 AND kind IN ('released', 'expired') AND at >= ?3",
            params![work_id, agent, now_ms - window_ms],
            |r| r.get(0),
        )?;
        Ok(n > 0)
    }
}

#[cfg(test)]
#[path = "work_board_tests.rs"]
mod tests;
