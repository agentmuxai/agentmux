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

/// `mcp_server_list`'s response shape: the server plus whether the requesting
/// agent specifically holds a `db_agent_mcp_ref` row for it. A global server
/// is always visible to every agent (see the query below), but `is_global`
/// alone can't tell the UI whether *this* agent has bound it — without
/// `bound_to_agent`, bind/unbind in the per-agent modal has no way to render
/// as a stateful toggle (see docs/specs, "bound to me" gap tracked in #1960).
#[derive(Debug, Clone, Serialize)]
pub struct McpServerListItem {
    #[serde(flatten)]
    pub server: McpServer,
    pub bound_to_agent: bool,
}

/// `mcp_server_list_global`'s response shape: the server plus how many
/// agents currently hold a `db_agent_mcp_ref` to it — the Armory catalog's
/// "used by N agents" count (gap #2 of #1960).
#[derive(Debug, Clone, Serialize)]
pub struct McpServerCatalogItem {
    #[serde(flatten)]
    pub server: McpServer,
    pub bound_count: i64,
}

impl Store {
    /// List all MCP servers visible to an agent: own (referenced) + global,
    /// each annotated with whether this specific agent holds the bind ref.
    pub fn mcp_server_list(&self, agent_id: &str) -> Result<Vec<McpServerListItem>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT s.id, s.name, s.transport, s.config, s.is_global, s.created_at, s.updated_at,
                    EXISTS(SELECT 1 FROM db_agent_mcp_ref r WHERE r.mcp_id = s.id AND r.agent_id = ?1) AS bound_to_agent
             FROM db_mcp_servers s
             WHERE s.is_global = 1
                OR s.id IN (SELECT mcp_id FROM db_agent_mcp_ref WHERE agent_id = ?1)
             ORDER BY s.is_global DESC, s.updated_at DESC",
        )?;
        let rows = stmt.query_map(params![agent_id], |row| {
            Ok(McpServerListItem {
                server: McpServer {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    transport: row.get(2)?,
                    config: row.get(3)?,
                    is_global: row.get::<_, i64>(4)? != 0,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                },
                bound_to_agent: row.get::<_, i64>(7)? != 0,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// List every GLOBAL MCP server — the Armory catalog view. Unlike
    /// `mcp_server_list`, this takes no `agent_id` and never includes an
    /// agent's private servers; it backs the window-scoped `mcp.catalog.*`
    /// App API (no `check_s1`, so there is no agent context to scope by).
    /// Each row carries `bound_count` — how many agents currently hold a
    /// `db_agent_mcp_ref` to it — per SPEC_V1_MCP_SKILLS_PRIMITIVES_2026_06_30.md
    /// §8 ("used by N agents"), tracked as gap #2 of #1960.
    pub fn mcp_server_list_global(&self) -> Result<Vec<McpServerCatalogItem>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT s.id, s.name, s.transport, s.config, s.is_global, s.created_at, s.updated_at,
                    (SELECT COUNT(*) FROM db_agent_mcp_ref r WHERE r.mcp_id = s.id) AS bound_count
             FROM db_mcp_servers s
             WHERE s.is_global = 1
             ORDER BY s.updated_at DESC",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(McpServerCatalogItem {
                server: McpServer {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    transport: row.get(2)?,
                    config: row.get(3)?,
                    is_global: row.get::<_, i64>(4)? != 0,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                },
                bound_count: row.get(7)?,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Catalog-tier sibling of `mcp_server_list` (above) — same
    /// `bound_to_agent` shape, but deliberately GLOBAL ROWS ONLY, unlike
    /// `mcp_server_list`'s UNION with `agent_id`'s own private servers.
    /// Backs `mcp.catalog.list_for_agent`, which — like every other
    /// `mcp.catalog.*` command — has no `check_s1`, so `agent_id` here is
    /// caller-supplied and unverified. Returning private server rows (whose
    /// `config` can carry secrets: API keys, auth headers, env vars) for an
    /// arbitrary caller-chosen `agent_id` would let any window connection
    /// read any agent's private server config. Global rows carry no
    /// per-agent secret — they're already fully visible via
    /// `mcp_server_list_global` (the Armory catalog) — so exposing them
    /// alongside a caller-chosen agent's bind status is safe.
    /// reagentx P0 on PR #2329.
    pub fn mcp_server_list_global_for_agent(&self, agent_id: &str) -> Result<Vec<McpServerListItem>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT s.id, s.name, s.transport, s.config, s.is_global, s.created_at, s.updated_at,
                    EXISTS(SELECT 1 FROM db_agent_mcp_ref r WHERE r.mcp_id = s.id AND r.agent_id = ?1) AS bound_to_agent
             FROM db_mcp_servers s
             WHERE s.is_global = 1
             ORDER BY s.updated_at DESC",
        )?;
        let rows = stmt.query_map(params![agent_id], |row| {
            Ok(McpServerListItem {
                server: McpServer {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    transport: row.get(2)?,
                    config: row.get(3)?,
                    is_global: row.get::<_, i64>(4)? != 0,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                },
                bound_to_agent: row.get::<_, i64>(7)? != 0,
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

    /// Delete a standalone MCP server and purge ref rows. Returns true if deleted.
    pub fn mcp_server_delete(&self, id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM db_agent_mcp_ref WHERE mcp_id = ?1", params![id])?;
        let rows = conn.execute("DELETE FROM db_mcp_servers WHERE id = ?1", params![id])?;
        Ok(rows > 0)
    }

    /// Bind an MCP server to an agent (insert ref row). Idempotent —
    /// binding an already-bound pair is a silent no-op success.
    ///
    /// Errors if `agent_id` isn't a LOCAL agent definition:
    /// `db_agent_mcp_ref.agent_id` has an ON-enforced FK to
    /// `db_agent_definitions(id)` (store.rs), but the Armory's agent
    /// picker (`ListAgentDefinitionsCommand` → `agent_def_list()`) also
    /// lists cross-channel agents that only exist in another channel's
    /// local database. Binding one of those would otherwise have the FK
    /// silently swallow the `INSERT OR IGNORE` — indistinguishable, by
    /// affected-row-count alone, from the equally-silent "already bound"
    /// case — reporting success while creating nothing. Same fix as
    /// skill_bind — see
    /// docs/reports/REPORT_ARMORY_SKILLS_MARKDOWN_AND_BIND_BUG_2026_07_27.md.
    pub fn mcp_server_bind(&self, agent_id: &str, mcp_id: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        let agent_exists: bool = conn.query_row(
            "SELECT EXISTS(SELECT 1 FROM db_agent_definitions WHERE id = ?1)",
            params![agent_id],
            |row| row.get(0),
        )?;
        if !agent_exists {
            return Err(StoreError::Other(format!(
                "agent {agent_id} not found in this channel's local registry — cross-channel MCP server binding is not supported"
            )));
        }
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

    /// Atomically upsert a GLOBAL MCP server enforcing catalog-wide name
    /// uniqueness (no `agent_id` — unlike `mcp_server_upsert_unique`, this
    /// checks for a duplicate name among *every* global row, not just those
    /// visible to one agent). Reagent P1 on #1948: `agent_config.rs`'s
    /// `build_mcp_config_from_refs` merges servers into a JSON object keyed
    /// by `server.name` — two same-named global servers would silently
    /// clobber each other's config for every agent that has either bound.
    /// `server.is_global` must already be `true`; caller's job.
    pub fn mcp_server_upsert_unique_global(&self, server: &McpServer) -> Result<(), StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let dup: i64 = tx.query_row(
            "SELECT COUNT(*) FROM db_mcp_servers WHERE name = ?1 AND id <> ?2 AND is_global = 1",
            params![server.name, server.id],
            |r| r.get(0),
        )?;
        if dup > 0 {
            return Err(StoreError::Other(format!(
                "a global server named '{}' already exists",
                server.name
            )));
        }
        tx.execute(
            "INSERT INTO db_mcp_servers (id, name, transport, config, is_global, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 1, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
               name=excluded.name, transport=excluded.transport, config=excluded.config,
               updated_at=excluded.updated_at",
            params![
                server.id, server.name, server.transport, server.config,
                server.created_at, server.updated_at,
            ],
        )?;
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
