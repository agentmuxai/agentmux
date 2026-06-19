// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Memory bundles — the agent's personality and capability stack
//! (provider, model, instructions, context files, MCP servers, skills).
//!
//! Extracted from `store.rs` in Phase R.3 of the storage
//! modularization plan
//! (`docs/specs/SPEC_STORE_MODULARIZATION_2026_05_27.md`). The
//! method surface is unchanged — `Store::bundle_memory_*` still
//! lives on `Store` via this `impl` block; callers stay on
//! `storage::store::Memory` thanks to the re-export.

use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::error::StoreError;
use super::store::Store;

/// A Memory bundle — the agent's personality and capability stack.
/// Provider, model, instructions, and JSON-encoded arrays of context
/// files / MCP servers / skills. Agent definitions shadow-migrate into this
/// table during the v7 migration.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Memory {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub is_blank: bool,
    /// Global bundles are injected into every agent's CLAUDE.md at launch,
    /// regardless of per-agent memory selection. Managed in the Trust Center
    /// (Identity & Memory hamburger modal). Seeded from workspace-wide rule sets.
    #[serde(default)]
    pub is_global: bool,
    /// "claude" | "codex" | "gemini" | empty string
    #[serde(default)]
    pub provider: String,
    #[serde(default)]
    pub model: String,
    #[serde(default)]
    pub instructions: String,
    /// JSON-encoded array; the renderer types it as `[{path, content}]`.
    #[serde(default = "default_json_array_string")]
    pub context_files: String,
    /// JSON-encoded array of MCP server configs.
    #[serde(default = "default_json_array_string")]
    pub mcp_servers: String,
    /// JSON-encoded array of skill IDs.
    #[serde(default = "default_json_array_string")]
    pub skills: String,
    pub created_at: i64,
    pub updated_at: i64,
}

fn default_json_array_string() -> String {
    "[]".to_string()
}

impl Store {
    pub fn bundle_memory_list(&self) -> Result<Vec<Memory>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, description, is_blank, is_global, provider, model, instructions,
                    context_files, mcp_servers, skills, created_at, updated_at
             FROM db_memory_bundles
             ORDER BY is_blank ASC, is_global DESC, updated_at DESC",
        )?;
        let iter = stmt.query_map([], map_memory_row)?;
        let mut out = Vec::new();
        for r in iter {
            out.push(r?);
        }
        Ok(out)
    }

    /// Returns only the global bundles (`is_global = 1`), ordered by name.
    /// Called at agent launch to inject workspace-wide rules into every agent.
    pub fn bundle_memory_list_global(&self) -> Result<Vec<Memory>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, description, is_blank, is_global, provider, model, instructions,
                    context_files, mcp_servers, skills, created_at, updated_at
             FROM db_memory_bundles
             WHERE is_global = 1
             ORDER BY name ASC",
        )?;
        let iter = stmt.query_map([], map_memory_row)?;
        let mut out = Vec::new();
        for r in iter {
            out.push(r?);
        }
        Ok(out)
    }

    pub fn bundle_memory_get(&self, id: &str) -> Result<Option<Memory>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, description, is_blank, is_global, provider, model, instructions,
                    context_files, mcp_servers, skills, created_at, updated_at
             FROM db_memory_bundles WHERE id = ?1",
        )?;
        let result = stmt.query_row(params![id], map_memory_row);
        match result {
            Ok(m) => Ok(Some(m)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    pub fn bundle_memory_upsert(&self, memory: &Memory) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO db_memory_bundles
                (id, name, description, is_blank, is_global, provider, model, instructions,
                 context_files, mcp_servers, skills, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13)
             ON CONFLICT(id) DO UPDATE SET
                name = excluded.name,
                description = excluded.description,
                is_global = excluded.is_global,
                provider = excluded.provider,
                model = excluded.model,
                instructions = excluded.instructions,
                context_files = excluded.context_files,
                mcp_servers = excluded.mcp_servers,
                skills = excluded.skills,
                updated_at = excluded.updated_at",
            params![
                memory.id,
                memory.name,
                memory.description,
                memory.is_blank as i64,
                memory.is_global as i64,
                memory.provider,
                memory.model,
                memory.instructions,
                memory.context_files,
                memory.mcp_servers,
                memory.skills,
                memory.created_at,
                memory.updated_at,
            ],
        )?;
        Ok(())
    }

    /// Delete a Memory bundle. Refuses to delete the blank singleton.
    pub fn bundle_memory_delete(&self, id: &str) -> Result<bool, StoreError> {
        if id == "blank" {
            return Err(StoreError::Other(
                "cannot delete the blank Memory singleton".to_string(),
            ));
        }
        // Seeded bundles (IDs prefixed "seed-") are workspace defaults that
        // re-seed on every startup; blocking deletion is cleaner than a
        // tombstone table and avoids the re-creation loop.
        if id.starts_with("seed-") {
            return Err(StoreError::Other(
                "cannot delete a seeded Memory bundle; toggle is_global or clear its instructions instead".to_string(),
            ));
        }
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute("DELETE FROM db_memory_bundles WHERE id = ?1", params![id])?;
        Ok(rows > 0)
    }
}

fn map_memory_row(row: &rusqlite::Row) -> rusqlite::Result<Memory> {
    Ok(Memory {
        id: row.get(0)?,
        name: row.get(1)?,
        description: row.get(2)?,
        is_blank: row.get::<_, i64>(3)? != 0,
        is_global: row.get::<_, i64>(4)? != 0,
        provider: row.get(5)?,
        model: row.get(6)?,
        instructions: row.get(7)?,
        context_files: row.get(8)?,
        mcp_servers: row.get(9)?,
        skills: row.get(10)?,
        created_at: row.get(11)?,
        updated_at: row.get(12)?,
    })
}
