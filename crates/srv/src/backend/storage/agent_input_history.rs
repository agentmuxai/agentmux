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

/// The newest entries kept per agent; older ones are dropped as new ones come.
pub const INPUT_HISTORY_MAX: usize = 10_000;
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
            "DELETE FROM db_agent_input_history WHERE agent_id = ?1 AND id NOT IN              (SELECT id FROM db_agent_input_history WHERE agent_id = ?1 ORDER BY id DESC LIMIT ?2)",
            params![entry.agent_id, max as i64],
        )?;
        let saved =
            tx.query_row(&format!("SELECT {COLUMNS} FROM db_agent_input_history WHERE id = ?1"), params![id], entry_row)?;
        tx.commit()?;
        Ok(Some(saved))
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

    /// Keep the agent's unsent draft; an empty one removes it. `false` when
    /// there is no such agent.
    pub fn input_draft_set(&self, agent_id: &str, text: &str, now_ms: i64) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let known: Option<i64> =
            conn.query_row("SELECT 1 FROM db_agents WHERE id = ?1", params![agent_id], |r| r.get(0)).optional()?;
        if known.is_none() {
            return Ok(false);
        }
        if text.is_empty() {
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
