// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Standalone MCP Server primitives (v1 composable model).
//!
//! Agents reference servers via db_agent_mcp_ref. The `config` field stores
//! the full server object JSON (command/args/env for stdio; url/headers for
//! SSE) that gets merged into `.mcp.json` at agent launch.

use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::error::StoreError;
use super::store::Store;

/// A standalone MCP Server primitive (v1 composable model).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServer {
    pub id: String,
    pub name: String,
    pub transport: String,
    pub config: String,
    pub is_global: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Store {
    /// List all MCP servers visible to an agent: own (referenced) + global.
    pub fn mcp_server_list(&self, agent_id: &str) -> Result<Vec<McpServer>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, transport, config, is_global, created_at, updated_at
             FROM db_mcp_servers
             WHERE is_global = 1
                OR id IN (SELECT mcp_id FROM db_agent_mcp_ref WHERE agent_id = ?1)
             ORDER BY is_global DESC, updated_at DESC",
        )?;
        let rows = stmt.query_map(params![agent_id], |row| {
            Ok(McpServer {
                id: row.get(0)?,
                name: row.get(1)?,
                transport: row.get(2)?,
                config: row.get(3)?,
                is_global: row.get::<_, i64>(4)? != 0,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Get a standalone MCP server by id.
    pub fn mcp_server_get(&self, id: &str) -> Result<Option<McpServer>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let result = conn.query_row(
            "SELECT id, name, transport, config, is_global, created_at, updated_at
             FROM db_mcp_servers WHERE id = ?1",
            params![id],
            |row| {
                Ok(McpServer {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    transport: row.get(2)?,
                    config: row.get(3)?,
                    is_global: row.get::<_, i64>(4)? != 0,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                })
            },
        );
        match result {
            Ok(s) => Ok(Some(s)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(StoreError::Sqlite(e)),
        }
    }

    /// Upsert a standalone MCP server. Caller must have stripped is_global escalation.
    pub fn mcp_server_upsert(&self, server: &McpServer) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO db_mcp_servers (id, name, transport, config, is_global, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET
               name=excluded.name, transport=excluded.transport, config=excluded.config,
               updated_at=excluded.updated_at",
            params![
                server.id,
                server.name,
                server.transport,
                server.config,
                if server.is_global { 1i64 } else { 0i64 },
                server.created_at,
                server.updated_at,
            ],
        )?;
        Ok(())
    }

    /// Delete a standalone MCP server and purge ref rows. Returns true if deleted.
    pub fn mcp_server_delete(&self, id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM db_agent_mcp_ref WHERE mcp_id = ?1", params![id])?;
        let rows = conn.execute("DELETE FROM db_mcp_servers WHERE id = ?1", params![id])?;
        Ok(rows > 0)
    }

    /// Bind an MCP server to an agent (insert ref row). Idempotent.
    pub fn mcp_server_bind(&self, agent_id: &str, mcp_id: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO db_agent_mcp_ref (agent_id, mcp_id) VALUES (?1, ?2)",
            params![agent_id, mcp_id],
        )?;
        Ok(())
    }

    /// Unbind an MCP server from an agent. Returns true if a row was removed.
    pub fn mcp_server_unbind(&self, agent_id: &str, mcp_id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "DELETE FROM db_agent_mcp_ref WHERE agent_id = ?1 AND mcp_id = ?2",
            params![agent_id, mcp_id],
        )?;
        Ok(rows > 0)
    }

    /// Atomically upsert an MCP server enforcing per-agent name uniqueness, and
    /// (when `bind_new`) bind it — all in one transaction so concurrent
    /// `mcp.upsert` calls for the same name can't both pass a check and insert
    /// duplicate-named bindings. Returns an error if another server visible to
    /// the agent (bound or global) already uses the name.
    pub fn mcp_server_upsert_unique(
        &self,
        agent_id: &str,
        server: &McpServer,
        bind_new: bool,
    ) -> Result<(), StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let dup: i64 = tx.query_row(
            "SELECT COUNT(*) FROM db_mcp_servers
             WHERE name = ?1 AND id <> ?2 AND (is_global = 1 OR id IN (
               SELECT mcp_id FROM db_agent_mcp_ref WHERE agent_id = ?3
             ))",
            params![server.name, server.id, agent_id],
            |r| r.get(0),
        )?;
        if dup > 0 {
            return Err(StoreError::Other(format!(
                "server name '{}' already bound to this agent",
                server.name
            )));
        }
        tx.execute(
            "INSERT INTO db_mcp_servers (id, name, transport, config, is_global, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)
             ON CONFLICT(id) DO UPDATE SET
               name=excluded.name, transport=excluded.transport, config=excluded.config,
               updated_at=excluded.updated_at",
            params![
                server.id, server.name, server.transport, server.config,
                if server.is_global { 1i64 } else { 0i64 },
                server.created_at, server.updated_at,
            ],
        )?;
        if bind_new {
            tx.execute(
                "INSERT OR IGNORE INTO db_agent_mcp_ref (agent_id, mcp_id) VALUES (?1, ?2)",
                params![agent_id, server.id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Return true if the given MCP server is accessible to the agent (global or bound).
    /// Used for read and mutation access checks.
    pub fn mcp_server_is_accessible_to(&self, agent_id: &str, mcp_id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM db_mcp_servers
             WHERE id = ?1 AND (is_global = 1 OR id IN (
               SELECT mcp_id FROM db_agent_mcp_ref WHERE agent_id = ?2
             ))",
            rusqlite::params![mcp_id, agent_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Return true if the agent has a direct ref binding to this MCP server.
    /// Used for delete access — an agent may only delete servers it directly bound.
    pub fn mcp_server_is_bound_to(&self, agent_id: &str, mcp_id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM db_agent_mcp_ref WHERE agent_id = ?1 AND mcp_id = ?2",
            rusqlite::params![agent_id, mcp_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }
}
