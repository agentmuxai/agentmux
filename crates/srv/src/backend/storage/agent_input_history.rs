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
";

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

    /// Remove one entry; its agent, when there was one.
    pub fn input_history_delete(&self, id: i64) -> Result<Option<String>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let agent: Option<String> = conn
            .query_row("SELECT agent_id FROM db_agent_input_history WHERE id = ?1", params![id], |r| r.get(0))
            .optional()?;
        conn.execute("DELETE FROM db_agent_input_history WHERE id = ?1", params![id])?;
        Ok(agent)
    }

    /// Remove an agent's whole history; how many entries went.
    pub fn input_history_clear(&self, agent_id: &str) -> Result<usize, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(conn.execute("DELETE FROM db_agent_input_history WHERE agent_id = ?1", params![agent_id])?)
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
