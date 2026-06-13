// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Agent-content blobs — per-agent key/value content storage (e.g.
//! the "instructions" panel content).
//!
//! Extracted from `store.rs` in Phase R.4 of the storage
//! modularization plan
//! (`docs/specs/SPEC_STORE_MODULARIZATION_2026_05_27.md`). The
//! method surface is unchanged — `Store::agent_content_*` still
//! lives on `Store` via this `impl` block; callers stay on
//! `storage::store::AgentContent` thanks to the re-export.

use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::error::StoreError;
use super::store::Store;

/// A content blob attached to a agent definition (e.g. "instructions").
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentContent {
    pub agent_id: String,
    pub content_type: String,
    pub content: String,
    pub updated_at: i64,
}

impl Store {
    pub fn agent_content_get(
        &self,
        agent_id: &str,
        content_type: &str,
    ) -> Result<Option<AgentContent>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT agent_id, content_type, content, updated_at
             FROM db_agent_content WHERE agent_id=?1 AND content_type=?2",
        )?;
        let result = stmt.query_row(params![agent_id, content_type], |row| {
            Ok(AgentContent {
                agent_id: row.get(0)?,
                content_type: row.get(1)?,
                content: row.get(2)?,
                updated_at: row.get(3)?,
            })
        });
        match result {
            Ok(content) => Ok(Some(content)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StoreError::Sqlite(e)),
        }
    }

    /// Upsert a content blob for an agent.
    pub fn agent_content_set(&self, content: &AgentContent) -> Result<(), StoreError> {
        {
            let conn = self.conn.lock().unwrap();
            conn.execute(
                "INSERT INTO db_agent_content (agent_id, content_type, content, updated_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(agent_id, content_type) DO UPDATE SET content=?3, updated_at=?4",
                params![
                    content.agent_id,
                    content.content_type,
                    content.content,
                    content.updated_at,
                ],
            )?;
        }
        // conn dropped — re-mirror the definition so the global cross-channel
        // record carries the updated content (no-op for seeded templates).
        // (P0.2b.)
        self.registry_def_upsert(&content.agent_id);
        Ok(())
    }

    /// Get all content blobs for an agent.
    pub fn agent_content_get_all(
        &self,
        agent_id: &str,
    ) -> Result<Vec<AgentContent>, StoreError> {
        let contents: Vec<AgentContent> = {
            let conn = self.conn.lock().unwrap();
            let mut stmt = conn.prepare(
                "SELECT agent_id, content_type, content, updated_at
                 FROM db_agent_content WHERE agent_id=?1 ORDER BY content_type ASC",
            )?;
            let rows = stmt.query_map(params![agent_id], |row| {
                Ok(AgentContent {
                    agent_id: row.get(0)?,
                    content_type: row.get(1)?,
                    content: row.get(2)?,
                    updated_at: row.get(3)?,
                })
            })?;
            let mut contents = Vec::new();
            for row in rows {
                contents.push(row?);
            }
            contents
        };
        if !contents.is_empty() {
            return Ok(contents);
        }
        // Cross-channel fallback: a user agent created in another channel has
        // its content on the global definition record, not in this channel's
        // SQLite. Surface it so the agent launches with its instructions.
        // (P0.2c — closes the content/skills cross-channel gap.)
        if let Some(reg) = self.shared_def_registry() {
            if let Ok(Some(rec)) = reg.get(agent_id) {
                return Ok(rec
                    .data
                    .content
                    .iter()
                    .map(|c| AgentContent {
                        agent_id: agent_id.to_string(),
                        content_type: c.content_type.clone(),
                        content: c.content.clone(),
                        updated_at: rec.data.updated_at,
                    })
                    .collect());
            }
        }
        Ok(contents)
    }

    /// Delete a specific content blob. Returns true if a row was deleted.
    #[allow(dead_code)]
    pub fn agent_content_delete(
        &self,
        agent_id: &str,
        content_type: &str,
    ) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "DELETE FROM db_agent_content WHERE agent_id=?1 AND content_type=?2",
            params![agent_id, content_type],
        )?;
        Ok(rows > 0)
    }
}
