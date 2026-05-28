// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Agent session history — append-only per-agent entries with
//! date-bucket + LIKE search.
//!
//! Extracted from `store.rs` in Phase R.4 of the storage
//! modularization plan
//! (`docs/specs/SPEC_STORE_MODULARIZATION_2026_05_27.md`). The
//! method surface is unchanged — `Store::agent_history_*` still
//! lives on `Store` via this `impl` block; callers stay on
//! `storage::store::AgentHistory` thanks to the re-export.

use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::error::StoreError;
use super::store::Store;

/// An append-only session history entry for a agent definition.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentHistory {
    pub id: i64,
    pub agent_id: String,
    pub session_date: String,
    pub entry: String,
    pub timestamp: i64,
}

impl Store {
    /// Append a history entry for an agent. Auto-sets session_date (today) and timestamp.
    pub fn agent_history_append(
        &self,
        agent_id: &str,
        entry: &str,
    ) -> Result<AgentHistory, StoreError> {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        // session_date as YYYY-MM-DD
        let secs = (now / 1000) as u64;
        let days = secs / 86400;
        // Simple date calculation (no chrono dependency needed)
        let session_date = format_epoch_date(days);
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO db_agent_history (agent_id, session_date, entry, timestamp) VALUES (?1, ?2, ?3, ?4)",
            params![agent_id, session_date, entry, now],
        )?;
        let id = conn.last_insert_rowid();
        Ok(AgentHistory {
            id,
            agent_id: agent_id.to_string(),
            session_date,
            entry: entry.to_string(),
            timestamp: now,
        })
    }

    /// List history entries for an agent, with optional date filter and pagination.
    pub fn agent_history_list(
        &self,
        agent_id: &str,
        session_date: Option<&str>,
        limit: i64,
        offset: i64,
    ) -> Result<Vec<AgentHistory>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let (sql, params_vec): (String, Vec<Box<dyn rusqlite::types::ToSql>>) = match session_date {
            Some(date) => (
                "SELECT id, agent_id, session_date, entry, timestamp
                 FROM db_agent_history WHERE agent_id=?1 AND session_date=?2
                 ORDER BY timestamp DESC LIMIT ?3 OFFSET ?4"
                    .to_string(),
                vec![
                    Box::new(agent_id.to_string()),
                    Box::new(date.to_string()),
                    Box::new(limit),
                    Box::new(offset),
                ],
            ),
            None => (
                "SELECT id, agent_id, session_date, entry, timestamp
                 FROM db_agent_history WHERE agent_id=?1
                 ORDER BY timestamp DESC LIMIT ?2 OFFSET ?3"
                    .to_string(),
                vec![
                    Box::new(agent_id.to_string()),
                    Box::new(limit),
                    Box::new(offset),
                ],
            ),
        };
        let params_refs: Vec<&dyn rusqlite::types::ToSql> =
            params_vec.iter().map(|b| b.as_ref()).collect();
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(params_refs.as_slice(), |row| {
            Ok(AgentHistory {
                id: row.get(0)?,
                agent_id: row.get(1)?,
                session_date: row.get(2)?,
                entry: row.get(3)?,
                timestamp: row.get(4)?,
            })
        })?;
        let mut entries = Vec::new();
        for row in rows {
            entries.push(row?);
        }
        Ok(entries)
    }

    /// Search history entries for an agent using LIKE-based matching.
    pub fn agent_history_search(
        &self,
        agent_id: &str,
        query: &str,
        limit: i64,
    ) -> Result<Vec<AgentHistory>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let pattern = format!("%{}%", query);
        let mut stmt = conn.prepare(
            "SELECT id, agent_id, session_date, entry, timestamp
             FROM db_agent_history WHERE agent_id=?1 AND entry LIKE ?2
             ORDER BY timestamp DESC LIMIT ?3",
        )?;
        let rows = stmt.query_map(params![agent_id, pattern, limit], |row| {
            Ok(AgentHistory {
                id: row.get(0)?,
                agent_id: row.get(1)?,
                session_date: row.get(2)?,
                entry: row.get(3)?,
                timestamp: row.get(4)?,
            })
        })?;
        let mut entries = Vec::new();
        for row in rows {
            entries.push(row?);
        }
        Ok(entries)
    }
}

/// Format days-since-epoch as YYYY-MM-DD string.
/// Simple implementation without chrono dependency.
fn format_epoch_date(days_since_epoch: u64) -> String {
    // Algorithm from https://howardhinnant.github.io/date_algorithms.html
    let z = days_since_epoch + 719468;
    let era = z / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{:04}-{:02}-{:02}", y, m, d)
}
