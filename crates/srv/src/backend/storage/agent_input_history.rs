// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the user typed to each agent, kept with the agent
//! (`docs/specs/SPEC_AGENT_INPUT_HISTORY_2026_10_18.md`): `db_agent_input_history`,
//! one row per send (a run of the same text is one row counting `sends`), and
//! `db_agent_input_drafts`, the agent's unsent draft. Both in the channel
//! store beside `db_agents`, so they are created, carried by an upgrade and
//! deleted with the agent (`agents/purge.rs`). Local only: nothing here is
//! sent anywhere, and no agent tool reads it.

use rusqlite::{params, OptionalExtension};

use super::error::StoreError;
use super::store::Store;

/// The tables, created on every open of the channel store
/// (`migrations::run_object_schema`). Additive, like `db_agent_bundles`, so no
/// schema version bump; no foreign key, also like it: an agent's rows go with
/// it in `purge_agent_dependents`.
pub(crate) const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS db_agent_input_history (
    id          INTEGER PRIMARY KEY,
    agent_id    TEXT NOT NULL,
    sent_at_ms  INTEGER NOT NULL,
    text        TEXT NOT NULL,
    kind        TEXT NOT NULL,
    source      TEXT NOT NULL,
    session_id  TEXT,
    attachments TEXT,
    sends       INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX IF NOT EXISTS idx_agent_input_history_agent
    ON db_agent_input_history (agent_id, id);
CREATE TABLE IF NOT EXISTS db_agent_input_drafts (
    agent_id      TEXT PRIMARY KEY,
    text          TEXT NOT NULL,
    updated_at_ms INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS db_agent_input_history_backfilled (
    agent_id TEXT PRIMARY KEY,
    at_ms    INTEGER NOT NULL
);
CREATE TABLE IF NOT EXISTS db_agent_input_history_trash (
    trash_id       INTEGER PRIMARY KEY,
    batch_id       TEXT NOT NULL,
    id             INTEGER NOT NULL,
    agent_id       TEXT NOT NULL,
    sent_at_ms     INTEGER NOT NULL,
    text           TEXT NOT NULL,
    kind           TEXT NOT NULL,
    source         TEXT NOT NULL,
    session_id     TEXT,
    attachments    TEXT,
    sends          INTEGER NOT NULL DEFAULT 1,
    deleted_at_ms  INTEGER NOT NULL,
    deleted_by     TEXT NOT NULL,
    deleted_by_uid TEXT NOT NULL DEFAULT ''
);
CREATE INDEX IF NOT EXISTS idx_agent_input_history_trash_agent
    ON db_agent_input_history_trash (agent_id, deleted_at_ms);
CREATE INDEX IF NOT EXISTS idx_agent_input_history_trash_batch
    ON db_agent_input_history_trash (batch_id);
CREATE INDEX IF NOT EXISTS idx_agent_input_history_trash_age
    ON db_agent_input_history_trash (deleted_at_ms);
";

/// How long a forgotten entry waits in the trash before it is gone for good.
pub const INPUT_HISTORY_TRASH_RETENTION_MS: i64 = 7 * 24 * 60 * 60 * 1000;

/// Who forgot an entry: the agent's slug, or `armory-ui` for the AgentMux
/// window, with the agent's UID when the connection proved one.
#[derive(Debug, Clone, Copy)]
pub struct ForgottenBy<'a> {
    pub name: &'a str,
    pub uid: &'a str,
}

/// One forgetting in the trash: a single entry, or everything cleared at once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrashedBatch {
    pub batch_id: String,
    pub agent_id: String,
    pub deleted_at_ms: i64,
    pub deleted_by: String,
    pub count: i64,
    /// The oldest entry's text, to say what it was.
    pub sample: String,
}

/// The longest entry kept, in bytes (a pasted file is not history).
pub const INPUT_HISTORY_MAX_TEXT: usize = 64 * 1024;

/// One entry, as the composer and the history view read it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InputHistoryEntry {
    pub id: i64,
    pub agent_id: String,
    pub sent_at_ms: i64,
    pub text: String,
    /// `message`, `slash` or `bang`.
    pub kind: String,
    /// `composer`, `swarm` or `backfill`.
    pub source: String,
    pub session_id: Option<String>,
    /// JSON `[{name, kind}]` of what was attached; never the contents.
    pub attachments: Option<String>,
    /// How many times in a row it was sent.
    pub sends: i64,
}

/// A send to record.
#[derive(Debug, Clone, Default)]
pub struct NewInputHistoryEntry<'a> {
    pub agent_id: &'a str,
    pub sent_at_ms: i64,
    pub text: &'a str,
    pub kind: &'a str,
    pub source: &'a str,
    pub session_id: Option<&'a str>,
    pub attachments: Option<&'a str>,
}

const COLUMNS: &str = "id, agent_id, sent_at_ms, text, kind, source, session_id, attachments, sends";

fn entry_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<InputHistoryEntry> {
    Ok(InputHistoryEntry {
        id: row.get(0)?,
        agent_id: row.get(1)?,
        sent_at_ms: row.get(2)?,
        text: row.get(3)?,
        kind: row.get(4)?,
        source: row.get(5)?,
        session_id: row.get(6)?,
        attachments: row.get(7)?,
        sends: row.get(8)?,
    })
}

/// Drop trash past the retention; how many entries went.
pub(crate) fn purge_trash(tx: &rusqlite::Connection, now_ms: i64) -> Result<usize, StoreError> {
    Ok(tx.execute(
        "DELETE FROM db_agent_input_history_trash WHERE deleted_at_ms < ?1",
        params![now_ms - INPUT_HISTORY_TRASH_RETENTION_MS],
    )?)
}

/// Move the history rows matching `condition` (which binds `?5` to `key`) into
/// the trash as batch `batch`, recorded with who forgot them; how many moved.
fn trash_rows(
    tx: &rusqlite::Transaction<'_>,
    condition: &str,
    key: &dyn rusqlite::ToSql,
    batch: &str,
    by: ForgottenBy<'_>,
    now_ms: i64,
) -> Result<usize, StoreError> {
    let moved = tx.execute(
        &format!(
            "INSERT INTO db_agent_input_history_trash \
                 (batch_id, id, agent_id, sent_at_ms, text, kind, source, session_id, attachments, sends, \
                  deleted_at_ms, deleted_by, deleted_by_uid) \
             SELECT ?1, id, agent_id, sent_at_ms, text, kind, source, session_id, attachments, sends, ?2, ?3, ?4 \
             FROM db_agent_input_history WHERE {condition}"
        ),
        params![batch, now_ms, by.name, by.uid, key],
    )?;
    tx.execute(&format!("DELETE FROM db_agent_input_history WHERE {}", condition.replace("?5", "?1")), params![key])?;
    Ok(moved)
}

/// `query` for a `LIKE … ESCAPE '\'` substring match.
fn like_pattern(query: &str) -> String {
    let escaped = query.replace('\\', "\\\\").replace('%', "\\%").replace('_', "\\_");
    format!("%{escaped}%")
}

impl Store {
    /// Record a send. The same text and kind as the agent's newest entry adds
    /// to that entry (`sends`, the time) instead of a new row. Keeps the
    /// newest `max` entries. `None` when there is no such agent (a
    /// quick-launch pane has none) or the text is empty or too long.
    pub fn input_history_append(
        &self,
        entry: &NewInputHistoryEntry<'_>,
        max: usize,
    ) -> Result<Option<InputHistoryEntry>, StoreError> {
        if entry.text.trim().is_empty() || entry.text.len() > INPUT_HISTORY_MAX_TEXT {
            return Ok(None);
        }
        let mut guard = self.conn.lock().unwrap();
        let tx = guard.transaction()?;
        let known: Option<i64> =
            tx.query_row("SELECT 1 FROM db_agents WHERE id = ?1", params![entry.agent_id], |r| r.get(0)).optional()?;
        if known.is_none() {
            return Ok(None);
        }
        // Whoever keeps typing still sees the retention honoured, without opening the trash.
        purge_trash(&tx, entry.sent_at_ms)?;
        let newest: Option<InputHistoryEntry> = tx
            .query_row(
                &format!("SELECT {COLUMNS} FROM db_agent_input_history WHERE agent_id = ?1 ORDER BY id DESC LIMIT 1"),
                params![entry.agent_id],
                entry_row,
            )
            .optional()?;
        let id = match newest.filter(|n| n.text == entry.text && n.kind == entry.kind) {
            Some(n) => {
                tx.execute(
                    "UPDATE db_agent_input_history SET sends = sends + 1, sent_at_ms = ?2,                      session_id = COALESCE(?3, session_id) WHERE id = ?1",
                    params![n.id, entry.sent_at_ms, entry.session_id],
                )?;
                n.id
            }
            None => {
                tx.execute(
                    "INSERT INTO db_agent_input_history (agent_id, sent_at_ms, text, kind, source, session_id, attachments)                      VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![
                        entry.agent_id,
                        entry.sent_at_ms,
                        entry.text,
                        entry.kind,
                        entry.source,
                        entry.session_id,
                        entry.attachments
                    ],
                )?;
                tx.last_insert_rowid()
            }
        };
        tx.execute(
            "DELETE FROM db_agent_input_history WHERE agent_id = ?1 AND id NOT IN \
             (SELECT id FROM db_agent_input_history WHERE agent_id = ?1 ORDER BY id DESC LIMIT ?2)",
            params![entry.agent_id, max as i64],
        )?;
        let saved =
            tx.query_row(&format!("SELECT {COLUMNS} FROM db_agent_input_history WHERE id = ?1"), params![id], entry_row)?;
        tx.commit()?;
        Ok(Some(saved))
    }

    /// Whether the agent's history was already filled from its past
    /// conversations ([`Self::input_history_backfill`]).
    pub fn input_history_backfilled(&self, agent_id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let found: Option<i64> = conn
            .query_row("SELECT 1 FROM db_agent_input_history_backfilled WHERE agent_id = ?1", params![agent_id], |r| r.get(0))
            .optional()?;
        Ok(found.is_some())
    }

    /// Fill the agent's history, once, with what was typed in its past
    /// conversations (`typed`, oldest first, with when each was sent). They go
    /// before every entry already kept, with ids below the oldest of any
    /// agent's (ids are one space, and can go below zero), so the order by id stays the order they were sent. Keeps
    /// the newest `max` in all. How many went in; 0 when it was filled before.
    pub fn input_history_backfill(&self, agent_id: &str, typed: &[(i64, String)], max: usize, now_ms: i64) -> Result<usize, StoreError> {
        let mut guard = self.conn.lock().unwrap();
        let tx = guard.transaction()?;
        let done: Option<i64> = tx
            .query_row("SELECT 1 FROM db_agent_input_history_backfilled WHERE agent_id = ?1", params![agent_id], |r| r.get(0))
            .optional()?;
        if done.is_some() {
            return Ok(0);
        }
        // Ids are one space for every agent: start below the oldest of all.
        let oldest: Option<i64> = tx.query_row("SELECT MIN(id) FROM db_agent_input_history", [], |r| r.get(0))?;
        // Never 0: the composer's cache marks a send still on its way with id 0.
        let mut next = oldest.unwrap_or(1).min(0) - 1;
        // What the composer kept is in the transcripts too: only messages sent
        // before this agent's oldest kept entry are new here.
        let kept_since: Option<i64> = tx.query_row(
            "SELECT MIN(sent_at_ms) FROM db_agent_input_history WHERE agent_id = ?1",
            params![agent_id],
            |r| r.get(0),
        )?;
        let mut inserted = 0;
        for (at, text) in typed.iter().filter(|(at, _)| kept_since.is_none_or(|k| *at < k)).rev().take(max) {
            tx.execute(
                "INSERT INTO db_agent_input_history (id, agent_id, sent_at_ms, text, kind, source) VALUES (?1, ?2, ?3, ?4, ?5, 'backfill')",
                params![next, agent_id, at, text, Self::input_kind(text)],
            )?;
            next -= 1;
            inserted += 1;
        }
        tx.execute(
            "DELETE FROM db_agent_input_history WHERE agent_id = ?1 AND id NOT IN \
             (SELECT id FROM db_agent_input_history WHERE agent_id = ?1 ORDER BY id DESC LIMIT ?2)",
            params![agent_id, max as i64],
        )?;
        tx.execute(
            "INSERT INTO db_agent_input_history_backfilled (agent_id, at_ms) VALUES (?1, ?2)",
            params![agent_id, now_ms],
        )?;
        tx.commit()?;
        Ok(inserted)
    }

    /// `message`, `slash` or `bang`, for text read back from a conversation.
    fn input_kind(text: &str) -> &'static str {
        if text.starts_with('/') {
            "slash"
        } else if text.starts_with('!') {
            "bang"
        } else {
            "message"
        }
    }

    /// An agent's entries, newest first: the newest `limit`, or the `limit`
    /// before entry `before_id` (paging back).
    pub fn input_history_list(
        &self,
        agent_id: &str,
        before_id: Option<i64>,
        limit: usize,
    ) -> Result<Vec<InputHistoryEntry>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM db_agent_input_history WHERE agent_id = ?1 AND id < ?2 ORDER BY id DESC LIMIT ?3"
        ))?;
        let rows = stmt.query_map(params![agent_id, before_id.unwrap_or(i64::MAX), limit as i64], entry_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Entries holding `query` (case-insensitive for ASCII), newest first: one
    /// agent's, or every agent's.
    pub fn input_history_search(
        &self,
        agent_id: Option<&str>,
        query: &str,
        limit: usize,
    ) -> Result<Vec<InputHistoryEntry>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(&format!(
            "SELECT {COLUMNS} FROM db_agent_input_history \
             WHERE (?1 IS NULL OR agent_id = ?1) AND text LIKE ?2 ESCAPE '\\' ORDER BY id DESC LIMIT ?3"
        ))?;
        let rows = stmt.query_map(params![agent_id, like_pattern(query), limit as i64], entry_row)?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Forget one entry. It moves to the trash, recorded with who forgot it, and
    /// can be restored until [`INPUT_HISTORY_TRASH_RETENTION_MS`] has passed.
    /// Its agent and the trash batch holding it, when there was such an entry.
    pub fn input_history_delete(&self, id: i64, by: ForgottenBy<'_>, now_ms: i64) -> Result<Option<(String, String)>, StoreError> {
        let mut guard = self.conn.lock().unwrap();
        let tx = guard.transaction()?;
        purge_trash(&tx, now_ms)?;
        let agent: Option<String> = tx
            .query_row("SELECT agent_id FROM db_agent_input_history WHERE id = ?1", params![id], |r| r.get(0))
            .optional()?;
        let Some(agent) = agent else { return Ok(None) };
        let batch = uuid::Uuid::new_v4().to_string();
        trash_rows(&tx, "id = ?5", &id, &batch, by, now_ms)?;
        tx.commit()?;
        Ok(Some((agent, batch)))
    }

    /// Forget an agent's whole history, as one trash batch. The batch and how
    /// many entries went (`None` and 0 when it had none).
    pub fn input_history_clear(&self, agent_id: &str, by: ForgottenBy<'_>, now_ms: i64) -> Result<(Option<String>, usize), StoreError> {
        let mut guard = self.conn.lock().unwrap();
        let tx = guard.transaction()?;
        purge_trash(&tx, now_ms)?;
        let batch = uuid::Uuid::new_v4().to_string();
        let moved = trash_rows(&tx, "agent_id = ?5", &agent_id, &batch, by, now_ms)?;
        tx.commit()?;
        Ok(((moved > 0).then_some(batch), moved))
    }

    /// What was forgotten and can still be restored, most recently forgotten
    /// first: every agent's, or one agent's. Entries past the retention are gone.
    pub fn input_history_trash(&self, agent_id: Option<&str>, now_ms: i64) -> Result<Vec<TrashedBatch>, StoreError> {
        let mut guard = self.conn.lock().unwrap();
        let tx = guard.transaction()?;
        purge_trash(&tx, now_ms)?;
        let batches = {
            let mut stmt = tx.prepare(
                "SELECT batch_id, agent_id, MAX(deleted_at_ms), MIN(deleted_by), COUNT(*), \
                        (SELECT t2.text FROM db_agent_input_history_trash t2 WHERE t2.batch_id = t.batch_id ORDER BY t2.id LIMIT 1) \
                 FROM db_agent_input_history_trash t \
                 WHERE (?1 IS NULL OR agent_id = ?1) \
                 GROUP BY batch_id, agent_id ORDER BY MAX(deleted_at_ms) DESC, batch_id",
            )?;
            let rows = stmt.query_map(params![agent_id], |r| {
                Ok(TrashedBatch {
                    batch_id: r.get(0)?,
                    agent_id: r.get(1)?,
                    deleted_at_ms: r.get(2)?,
                    deleted_by: r.get(3)?,
                    count: r.get(4)?,
                    sample: r.get(5)?,
                })
            })?;
            rows.collect::<Result<Vec<_>, _>>()?
        };
        tx.commit()?;
        Ok(batches)
    }

    /// Bring a trash batch back: each entry returns under its own id when that
    /// is free, else at the newest end (ids are reused once the newest entry is
    /// forgotten and something is typed). The batch leaves the trash. The
    /// agent and how many entries came back; `None` when there is no such batch
    /// (restored, or past the retention).
    pub fn input_history_restore(&self, batch_id: &str, now_ms: i64) -> Result<Option<(String, usize)>, StoreError> {
        let mut guard = self.conn.lock().unwrap();
        let tx = guard.transaction()?;
        purge_trash(&tx, now_ms)?;
        let rows: Vec<(i64, String, i64, String, String, String, Option<String>, Option<String>, i64)> = {
            let mut stmt = tx.prepare(
                "SELECT id, agent_id, sent_at_ms, text, kind, source, session_id, attachments, sends \
                 FROM db_agent_input_history_trash WHERE batch_id = ?1 ORDER BY id",
            )?;
            let found = stmt.query_map(params![batch_id], |r| {
                Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?, r.get(7)?, r.get(8)?))
            })?;
            found.collect::<Result<Vec<_>, _>>()?
        };
        let Some(agent) = rows.first().map(|r| r.1.clone()) else { return Ok(None) };
        for (id, agent_id, sent_at_ms, text, kind, source, session_id, attachments, sends) in &rows {
            let taken: Option<i64> =
                tx.query_row("SELECT 1 FROM db_agent_input_history WHERE id = ?1", params![id], |r| r.get(0)).optional()?;
            // A reused id goes to the newest end; its order among the restored ones stays.
            let id = if taken.is_some() { None } else { Some(*id) };
            tx.execute(
                "INSERT INTO db_agent_input_history (id, agent_id, sent_at_ms, text, kind, source, session_id, attachments, sends) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                params![id, agent_id, sent_at_ms, text, kind, source, session_id, attachments, sends],
            )?;
        }
        tx.execute("DELETE FROM db_agent_input_history_trash WHERE batch_id = ?1", params![batch_id])?;
        tx.commit()?;
        Ok(Some((agent, rows.len())))
    }

    /// Drop what has waited in the trash past the retention.
    pub fn input_history_trash_purge(&self, now_ms: i64) -> Result<usize, StoreError> {
        let mut guard = self.conn.lock().unwrap();
        let tx = guard.transaction()?;
        let gone = purge_trash(&tx, now_ms)?;
        tx.commit()?;
        Ok(gone)
    }

    /// The agent's unsent draft, if it has one.
    pub fn input_draft_get(&self, agent_id: &str) -> Result<Option<String>, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(conn
            .query_row("SELECT text FROM db_agent_input_drafts WHERE agent_id = ?1", params![agent_id], |r| r.get(0))
            .optional()?)
    }

    /// Keep the agent's unsent draft; an empty one removes it, and so does one
    /// over [`INPUT_HISTORY_MAX_TEXT`] (a pasted file isn't kept, and an older
    /// draft mustn't come back in its place). `false` when there is no such
    /// agent.
    pub fn input_draft_set(&self, agent_id: &str, text: &str, now_ms: i64) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let known: Option<i64> =
            conn.query_row("SELECT 1 FROM db_agents WHERE id = ?1", params![agent_id], |r| r.get(0)).optional()?;
        if known.is_none() {
            return Ok(false);
        }
        if text.is_empty() || text.len() > INPUT_HISTORY_MAX_TEXT {
            conn.execute("DELETE FROM db_agent_input_drafts WHERE agent_id = ?1", params![agent_id])?;
        } else {
            conn.execute(
                "INSERT INTO db_agent_input_drafts (agent_id, text, updated_at_ms) VALUES (?1, ?2, ?3) \
                 ON CONFLICT(agent_id) DO UPDATE SET text = excluded.text, updated_at_ms = excluded.updated_at_ms",
                params![agent_id, text, now_ms],
            )?;
        }
        Ok(true)
    }
}

#[cfg(test)]
#[path = "agent_input_history_tests.rs"]
mod tests;
