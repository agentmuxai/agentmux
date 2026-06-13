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
        let local = {
            let result = stmt.query_row(params![agent_id, content_type], |row| {
                Ok(AgentContent {
                    agent_id: row.get(0)?,
                    content_type: row.get(1)?,
                    content: row.get(2)?,
                    updated_at: row.get(3)?,
                })
            });
            match result {
                Ok(content) => Some(content),
                Err(rusqlite::Error::QueryReturnedNoRows) => None,
                Err(e) => return Err(StoreError::Sqlite(e)),
            }
        };
        drop(stmt);
        drop(conn);
        if local.is_some() {
            return Ok(local);
        }
        // Cross-channel fallback (same gate as agent_content_get_all): only for
        // an agent absent from local SQLite. Launch reads `env` via this single
        // path, so without it a cross-channel agent would launch missing its
        // env vars. (codex P2 on #1385.)
        if self.agent_def_exists_local(agent_id)? {
            return Ok(local);
        }
        if let Some(reg) = self.shared_def_registry() {
            if let Ok(Some(rec)) = reg.get(agent_id) {
                if let Some(c) = rec
                    .data
                    .content
                    .iter()
                    .find(|c| c.content_type == content_type)
                {
                    return Ok(Some(AgentContent {
                        agent_id: agent_id.to_string(),
                        content_type: c.content_type.clone(),
                        content: c.content.clone(),
                        updated_at: rec.data.updated_at,
                    }));
                }
            }
        }
        Ok(local)
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
    /// LOCAL channel's content blobs only — NO cross-channel fallback. Used
    /// by the def-registry mirror (which always operates on a local agent, so
    /// must never read the global record) and by `agent_content_get_all`.
    pub(super) fn agent_content_get_all_local(
        &self,
        agent_id: &str,
    ) -> Result<Vec<AgentContent>, StoreError> {
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
        Ok(contents)
    }

    pub fn agent_content_get_all(
        &self,
        agent_id: &str,
    ) -> Result<Vec<AgentContent>, StoreError> {
        let local = self.agent_content_get_all_local(agent_id)?;
        if !local.is_empty() {
            return Ok(local);
        }
        // Local is empty. Fall back to the global record ONLY for a
        // cross-channel agent (one absent from local SQLite). A locally-known
        // agent with genuinely empty content must return empty — otherwise
        // deleting its content would resurrect it from the global record.
        // (reagent P1 on #1385.)
        if self.agent_def_exists_local(agent_id)? {
            return Ok(local);
        }
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
        Ok(local)
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
