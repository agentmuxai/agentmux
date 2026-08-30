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

/// `bundle_mcp_list`'s response shape — mirrors `McpServerListItem`, but
/// "bound" means a `db_bundle_mcp_ref` row for this bundle, not an agent's
/// `db_agent_mcp_ref` row. Composable model v2,
/// docs/specs/SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md.
#[derive(Debug, Clone, Serialize)]
pub struct McpServerBundleListItem {
    #[serde(flatten)]
    pub server: McpServer,
    pub bound_to_bundle: bool,
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

    /// Every MCP server visible to an agent for config materialization:
    /// `mcp_server_list`'s own+global set, unioned with the agent's bound
    /// bundle's own referenced servers (composable model v2,
    /// docs/specs/SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md, GH issue #2024
    /// item 3) — without this, `db_bundle_mcp_ref` would be exactly as
    /// inert at launch as the bundle's old inline `mcp_servers` JSON column
    /// always was. Deduped by id. Single source of truth for
    /// `write_agent_config_files` — mirrors `effective_skills`'s own
    /// same-source-of-truth requirement (see its doc comment).
    pub fn effective_mcp_servers(&self, agent_id: &str) -> Vec<McpServer> {
        let mut visible: Vec<McpServer> = self
            .mcp_server_list(agent_id)
            .unwrap_or_default()
            .into_iter()
            .map(|item| item.server)
            .collect();
        if let Ok(Some(def)) = self.agent_def_get(agent_id) {
            if !def.memory_id.is_empty() {
                for item in self.bundle_mcp_list(&def.memory_id).unwrap_or_default() {
                    if !visible.iter().any(|s| s.id == item.server.id) {
                        visible.push(item.server);
                    }
                }
            }
        }
        visible
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

    /// Delete a standalone MCP server and purge ref rows (both agent- and
    /// bundle-level — FK cascades may be off on some builds, same reasoning
    /// as `skill_delete`). Returns true if deleted.
    pub fn mcp_server_delete(&self, id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM db_agent_mcp_ref WHERE mcp_id = ?1", params![id])?;
        conn.execute("DELETE FROM db_bundle_mcp_ref WHERE mcp_id = ?1", params![id])?;
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

    // ── Bundle-level references (composable model v2) ──────────────────
    // docs/specs/SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md, GH issue #2024
    // item 3. Mirror the agent-level methods above exactly, keyed by
    // bundle_id via db_bundle_mcp_ref instead of agent_id/db_agent_mcp_ref.
    // Access-control policy (e.g. "only global servers may be bound") is
    // the RPC handler layer's job, same as the agent-level methods above —
    // these are the raw, permissive primitives.

    /// Bundle-level sibling of `mcp_server_list` — this bundle's own
    /// (referenced) + global servers, each annotated with whether this
    /// specific bundle holds the `db_bundle_mcp_ref` row.
    pub fn bundle_mcp_list(&self, bundle_id: &str) -> Result<Vec<McpServerBundleListItem>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT s.id, s.name, s.transport, s.config, s.is_global, s.created_at, s.updated_at,
                    EXISTS(SELECT 1 FROM db_bundle_mcp_ref r WHERE r.mcp_id = s.id AND r.bundle_id = ?1) AS bound_to_bundle
             FROM db_mcp_servers s
             WHERE s.is_global = 1
                OR s.id IN (SELECT mcp_id FROM db_bundle_mcp_ref WHERE bundle_id = ?1)
             ORDER BY s.is_global DESC, s.updated_at DESC",
        )?;
        let rows = stmt.query_map(params![bundle_id], |row| {
            Ok(McpServerBundleListItem {
                server: McpServer {
                    id: row.get(0)?,
                    name: row.get(1)?,
                    transport: row.get(2)?,
                    config: row.get(3)?,
                    is_global: row.get::<_, i64>(4)? != 0,
                    created_at: row.get(5)?,
                    updated_at: row.get(6)?,
                },
                bound_to_bundle: row.get::<_, i64>(7)? != 0,
            })
        })?;
        let mut out = Vec::new();
        for row in rows {
            out.push(row?);
        }
        Ok(out)
    }

    /// Bind an MCP server to a bundle (insert ref row). Idempotent — binding
    /// an already-bound pair is a silent no-op success.
    ///
    /// `id_store` (NOT `self`) is where bundle existence is checked —
    /// reagentx P0 review on PR #2639: bundles are authoritatively written
    /// through `id_store` (the shared store in a normal production
    /// install), not `wstore`/`self`, where this method's own table lives.
    /// `self`'s own local `db_bundles` copy is essentially always empty in
    /// that case, so checking existence there (as the original version of
    /// this method did) made "bundle not found" fire for every real
    /// production bundle. `id_store` is a separate physical store/connection
    /// with no possible cross-database FK to it — same "check at the
    /// application layer, not via FK" pattern as
    /// `identity::resolver::resolve_account`'s cross-store lookups.
    pub fn bundle_mcp_bind(&self, id_store: &Store, bundle_id: &str, mcp_id: &str) -> Result<(), StoreError> {
        let bundle_exists: bool = {
            let conn = id_store.conn.lock().unwrap();
            conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM db_bundles WHERE id = ?1)",
                params![bundle_id],
                |row| row.get(0),
            )?
        };
        if !bundle_exists {
            return Err(StoreError::Other(format!(
                "bundle {bundle_id} not found — cannot bind an MCP server to a nonexistent bundle"
            )));
        }
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO db_bundle_mcp_ref (bundle_id, mcp_id) VALUES (?1, ?2)",
            params![bundle_id, mcp_id],
        )?;
        Ok(())
    }

    /// Atomically create a NEW, PRIVATE (never global) MCP server scoped
    /// and bound directly to a bundle, enforcing bundle-scoped name
    /// uniqueness — the bundle-level analog of `mcp_server_upsert_unique`.
    ///
    /// reagentx P1 review on PR #2639: `bundle_mcp_bind` can only bind
    /// EXISTING global rows (binding an existing private row would let one
    /// bundle "steal" read access to whatever entity that row already
    /// privately belongs to — the same cross-entity IDOR
    /// `mcp.catalog.bind`'s own "only global" restriction guards against).
    /// But globals already reach every agent unconditionally via
    /// `effective_mcp_servers`, so binding one to a bundle has literally no
    /// effect. This method is the missing piece: it creates a BRAND-NEW row
    /// that has never belonged to anyone else, so there's no theft risk —
    /// this is how a bundle gets a genuinely bundle-specific tool that
    /// isn't already visible to every other agent.
    pub fn bundle_mcp_upsert_unique(
        &self,
        id_store: &Store,
        bundle_id: &str,
        server: &McpServer,
        bind_new: bool,
    ) -> Result<(), StoreError> {
        let bundle_exists: bool = {
            let conn = id_store.conn.lock().unwrap();
            conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM db_bundles WHERE id = ?1)",
                params![bundle_id],
                |row| row.get(0),
            )?
        };
        if !bundle_exists {
            return Err(StoreError::Other(format!(
                "bundle {bundle_id} not found — cannot create an MCP server for a nonexistent bundle"
            )));
        }
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let dup: i64 = tx.query_row(
            "SELECT COUNT(*) FROM db_mcp_servers
             WHERE name = ?1 AND id <> ?2 AND (is_global = 1 OR id IN (
               SELECT mcp_id FROM db_bundle_mcp_ref WHERE bundle_id = ?3
             ))",
            params![server.name, server.id, bundle_id],
            |r| r.get(0),
        )?;
        if dup > 0 {
            return Err(StoreError::Other(format!(
                "server name '{}' already bound to this bundle",
                server.name
            )));
        }
        // is_global hardcoded to 0 — this method's entire purpose is
        // creating PRIVATE bundle-scoped content (mirrors
        // mcp_server_upsert_unique_global's opposite hardcode of 1).
        // Referencing an EXISTING global server is bundle_mcp_bind's job.
        tx.execute(
            "INSERT INTO db_mcp_servers (id, name, transport, config, is_global, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, 0, ?5, ?6)
             ON CONFLICT(id) DO UPDATE SET
               name=excluded.name, transport=excluded.transport, config=excluded.config,
               updated_at=excluded.updated_at",
            params![
                server.id, server.name, server.transport, server.config,
                server.created_at, server.updated_at,
            ],
        )?;
        if bind_new {
            tx.execute(
                "INSERT OR IGNORE INTO db_bundle_mcp_ref (bundle_id, mcp_id) VALUES (?1, ?2)",
                params![bundle_id, server.id],
            )?;
        }
        tx.commit()?;
        Ok(())
    }

    /// Unbind an MCP server from a bundle. Returns true if a row was removed.
    pub fn bundle_mcp_unbind(&self, bundle_id: &str, mcp_id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "DELETE FROM db_bundle_mcp_ref WHERE bundle_id = ?1 AND mcp_id = ?2",
            params![bundle_id, mcp_id],
        )?;
        Ok(rows > 0)
    }

    /// Return true if the given MCP server is accessible to the bundle
    /// (global or bundle-bound). Mirrors `mcp_server_is_accessible_to`.
    pub fn bundle_mcp_is_accessible_to(&self, bundle_id: &str, mcp_id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM db_mcp_servers
             WHERE id = ?1 AND (is_global = 1 OR id IN (
               SELECT mcp_id FROM db_bundle_mcp_ref WHERE bundle_id = ?2
             ))",
            rusqlite::params![mcp_id, bundle_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Return true if the bundle has a direct ref binding to this MCP
    /// server (global excluded). Mirrors `mcp_server_is_bound_to` — used
    /// for edit/delete-ownership checks, as opposed to
    /// `bundle_mcp_is_accessible_to`'s broader read-access check.
    pub fn bundle_mcp_is_bound_to(&self, bundle_id: &str, mcp_id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM db_bundle_mcp_ref WHERE bundle_id = ?1 AND mcp_id = ?2",
            rusqlite::params![bundle_id, mcp_id],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }
}

#[cfg(test)]
mod bundle_ref_tests {
    use super::*;
    use crate::backend::storage::memory_bundles::Memory;

    fn make_store() -> Store {
        Store::open_in_memory().unwrap()
    }

    fn insert_bundle(store: &Store, id: &str) {
        store
            .bundle_memory_upsert(&Memory {
                id: id.to_string(),
                name: format!("Bundle {id}"),
                description: String::new(),
                is_blank: false,
                is_global: false,
                provider: "claude".to_string(),
                model: "anthropic".to_string(),
                instructions: String::new(),
                instructions_by_provider: "{}".to_string(),
                context_files: "[]".to_string(),
                mcp_servers: "[]".to_string(),
                skills: "[]".to_string(),
                sort_order: 0,
                created_at: 1_700_000_000_000,
                updated_at: 1_700_000_000_000,
                is_system: false,
            })
            .unwrap();
    }

    fn server(id: &str, name: &str, is_global: bool) -> McpServer {
        McpServer {
            id: id.to_string(),
            name: name.to_string(),
            transport: "stdio".to_string(),
            config: "{}".to_string(),
            is_global,
            created_at: 1_700_000_000_000,
            updated_at: 1_700_000_000_000,
        }
    }

    #[test]
    fn bind_makes_a_private_server_visible_in_bundle_mcp_list() {
        let store = make_store();
        insert_bundle(&store, "bundle-1");
        store.mcp_server_upsert_unique_global(&server("srv-global", "Global", true)).unwrap();
        // A private (non-global) server, upserted without any agent bind —
        // simulates a server that exists but this bundle hasn't referenced.
        store
            .mcp_server_upsert_unique("some-other-agent-context", &server("srv-private", "Private", false), false)
            .unwrap_or(());

        let before = store.bundle_mcp_list("bundle-1").unwrap();
        assert_eq!(before.len(), 1, "only the global server should be visible before any bind: {before:?}");
        assert!(!before[0].bound_to_bundle, "global server isn't bundle-bound yet");

        store.bundle_mcp_bind(&store, "bundle-1", "srv-private").unwrap();
        let after = store.bundle_mcp_list("bundle-1").unwrap();
        assert_eq!(after.len(), 2, "private server must now be visible after binding: {after:?}");
        let private_item = after.iter().find(|i| i.server.id == "srv-private").expect("private server present");
        assert!(private_item.bound_to_bundle);
    }

    #[test]
    fn bind_is_not_visible_to_a_different_bundle() {
        let store = make_store();
        insert_bundle(&store, "bundle-1");
        insert_bundle(&store, "bundle-2");
        store
            .mcp_server_upsert_unique("some-other-agent-context", &server("srv-private", "Private", false), false)
            .unwrap_or(());
        store.bundle_mcp_bind(&store, "bundle-1", "srv-private").unwrap();

        let bundle2_list = store.bundle_mcp_list("bundle-2").unwrap();
        assert!(
            bundle2_list.is_empty(),
            "a private server bound to bundle-1 must not leak into bundle-2's list: {bundle2_list:?}"
        );
    }

    #[test]
    fn bind_errors_when_the_bundle_does_not_exist() {
        let store = make_store();
        store.mcp_server_upsert_unique_global(&server("srv-1", "S", true)).unwrap();
        let result = store.bundle_mcp_bind(&store, "no-such-bundle", "srv-1");
        assert!(result.is_err(), "binding to a nonexistent bundle must error, not silently no-op");
    }

    #[test]
    fn unbind_removes_the_ref_and_is_idempotent() {
        let store = make_store();
        insert_bundle(&store, "bundle-1");
        store
            .mcp_server_upsert_unique("ctx", &server("srv-private", "Private", false), false)
            .unwrap_or(());
        store.bundle_mcp_bind(&store, "bundle-1", "srv-private").unwrap();

        let removed = store.bundle_mcp_unbind("bundle-1", "srv-private").unwrap();
        assert!(removed);
        assert!(store.bundle_mcp_list("bundle-1").unwrap().is_empty());

        let removed_again = store.bundle_mcp_unbind("bundle-1", "srv-private").unwrap();
        assert!(!removed_again, "unbinding an already-unbound pair returns false, not an error");
    }

    #[test]
    fn deleting_a_server_purges_its_bundle_ref_too() {
        let store = make_store();
        insert_bundle(&store, "bundle-1");
        store
            .mcp_server_upsert_unique("ctx", &server("srv-private", "Private", false), false)
            .unwrap_or(());
        store.bundle_mcp_bind(&store, "bundle-1", "srv-private").unwrap();
        assert_eq!(store.bundle_mcp_list("bundle-1").unwrap().len(), 1);

        store.mcp_server_delete("srv-private").unwrap();
        assert!(
            store.bundle_mcp_list("bundle-1").unwrap().is_empty(),
            "the bundle ref row must be purged when the underlying server is deleted"
        );
    }

    /// reagentx P0 review on PR #2639: bundle existence must be checked
    /// against `id_store` (the caller-supplied param), which is where
    /// bundles are authoritatively written in production — NOT against
    /// `self` (wstore), whose own local `db_bundles` copy is essentially
    /// always empty for real bundles. Two-store setup mirrors production:
    /// the bundle exists ONLY in `id_store`.
    #[test]
    fn bind_checks_bundle_existence_in_id_store_not_self() {
        let wstore = make_store();
        let id_store = make_store();
        insert_bundle(&id_store, "bundle-1");
        wstore.mcp_server_upsert_unique_global(&server("srv-1", "S", true)).unwrap();

        let result = wstore.bundle_mcp_bind(&id_store, "bundle-1", "srv-1");
        assert!(result.is_ok(), "must check bundle existence against id_store, not self: {result:?}");
    }

    /// Inverse of the above: a bundle that exists only in `self`'s own
    /// (non-authoritative) local copy — never in `id_store`, where it
    /// should really live — must NOT be treated as existing. This
    /// reproduces the exact reported production bug (checking the wrong
    /// store made every real bind fail with "bundle not found") in the
    /// opposite direction: without the fix, `self` was checked instead of
    /// `id_store`, which would make THIS test's bind wrongly succeed.
    #[test]
    fn bind_fails_when_bundle_exists_only_in_self_not_id_store() {
        let wstore = make_store();
        let id_store = make_store();
        insert_bundle(&wstore, "bundle-1"); // wrong store — simulates the pre-fix bug's mirror image
        wstore.mcp_server_upsert_unique_global(&server("srv-1", "S", true)).unwrap();

        let result = wstore.bundle_mcp_bind(&id_store, "bundle-1", "srv-1");
        assert!(
            result.is_err(),
            "a bundle only present in self's non-authoritative copy must not satisfy the id_store check: {result:?}"
        );
    }

    /// reagentx P1 review on PR #2639: `bundle_mcp_upsert_unique` is the
    /// missing "give this bundle its own tool" path — `bind_to_bundle`
    /// alone can only reference already-global rows, which have no effect
    /// once bound (already unconditionally visible to every agent). This
    /// creates a genuinely NEW private server, bound only to this bundle.
    #[test]
    fn upsert_unique_creates_a_new_private_server_bound_to_the_bundle() {
        let store = make_store();
        insert_bundle(&store, "bundle-1");

        store
            .bundle_mcp_upsert_unique(
                &store,
                "bundle-1",
                &server("new-srv", "Bundle-Only Tool", true), // is_global: true here is IGNORED
                true,
            )
            .unwrap();

        let list = store.bundle_mcp_list("bundle-1").unwrap();
        let item = list.iter().find(|i| i.server.id == "new-srv").expect("newly created server present");
        assert!(item.bound_to_bundle);
        assert!(!item.server.is_global, "upsert_unique must force is_global=false regardless of the input struct");
    }

    #[test]
    fn upsert_unique_rejects_a_duplicate_name_already_bound_to_the_bundle() {
        let store = make_store();
        insert_bundle(&store, "bundle-1");
        store.bundle_mcp_upsert_unique(&store, "bundle-1", &server("srv-a", "Dup Name", true), true).unwrap();

        let result = store.bundle_mcp_upsert_unique(&store, "bundle-1", &server("srv-b", "Dup Name", true), true);
        assert!(result.is_err(), "a second server with the same name bound to the same bundle must be rejected");
    }

    #[test]
    fn is_accessible_to_reflects_global_and_bound_state() {
        let store = make_store();
        insert_bundle(&store, "bundle-1");
        store.mcp_server_upsert_unique_global(&server("srv-global", "Global", true)).unwrap();
        store
            .mcp_server_upsert_unique("ctx", &server("srv-private", "Private", false), false)
            .unwrap_or(());

        assert!(store.bundle_mcp_is_accessible_to("bundle-1", "srv-global").unwrap());
        assert!(!store.bundle_mcp_is_accessible_to("bundle-1", "srv-private").unwrap());
        store.bundle_mcp_bind(&store, "bundle-1", "srv-private").unwrap();
        assert!(store.bundle_mcp_is_accessible_to("bundle-1", "srv-private").unwrap());
    }

    fn insert_agent_with_bundle(store: &Store, id: &str, memory_id: &str) {
        let mut def = crate::backend::storage::store::AgentDefinition {
            conversation_visibility: crate::backend::storage::agents::default_conversation_visibility(),
            id: id.to_string(),
            slug: String::new(),
            name: "Test Agent".to_string(),
            icon: String::new(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1_700_000_000_000,
            agent_type: "host".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 1_700_000_000_000,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
            memory_id: memory_id.to_string(),
        };
        store.agent_def_insert(&mut def).unwrap();
    }

    /// Composable model v2: an MCP server referenced by the agent's OWN
    /// bound bundle (not the agent itself) must show up in
    /// `effective_mcp_servers` — same requirement as
    /// `effective_skills_includes_the_bound_bundles_referenced_skills` in
    /// skills.rs, mirrored here for MCP.
    #[test]
    fn effective_mcp_servers_includes_the_bound_bundles_referenced_servers() {
        let store = make_store();
        insert_bundle(&store, "bundle-1");
        insert_agent_with_bundle(&store, "agent-1", "bundle-1");
        store
            .mcp_server_upsert_unique("some-other-context", &server("bundle-srv", "Bundle Server", false), false)
            .unwrap_or(());
        store.bundle_mcp_bind(&store, "bundle-1", "bundle-srv").unwrap();

        let effective = store.effective_mcp_servers("agent-1");
        let names: Vec<&str> = effective.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names.contains(&"Bundle Server"),
            "a server referenced only by the agent's bound bundle must still be effective: {names:?}"
        );
    }

    /// End-to-end proof of the reagentx P1 fix: a server created via
    /// `bundle_mcp_upsert_unique` (the new "give this bundle its own tool"
    /// path) reaches a spawned agent's effective config — closing the gap
    /// where `bind_to_bundle` alone could only reference already-global
    /// rows that have no effect once bound.
    #[test]
    fn effective_mcp_servers_includes_a_bundle_owned_private_server_created_via_upsert_unique() {
        let store = make_store();
        insert_bundle(&store, "bundle-1");
        insert_agent_with_bundle(&store, "agent-1", "bundle-1");

        store
            .bundle_mcp_upsert_unique(&store, "bundle-1", &server("bundle-own-tool", "Bundle-Owned Tool", false), true)
            .unwrap();

        let effective = store.effective_mcp_servers("agent-1");
        let names: Vec<&str> = effective.iter().map(|s| s.name.as_str()).collect();
        assert!(
            names.contains(&"Bundle-Owned Tool"),
            "a bundle-owned private server created via bundle_mcp_upsert_unique must reach the agent's effective config: {names:?}"
        );
    }

    #[test]
    fn effective_mcp_servers_does_not_duplicate_a_global_server_visible_via_both_agent_and_bundle() {
        let store = make_store();
        insert_bundle(&store, "bundle-1");
        insert_agent_with_bundle(&store, "agent-1", "bundle-1");
        store.mcp_server_upsert_unique_global(&server("global-1", "Global Server", true)).unwrap();

        let effective = store.effective_mcp_servers("agent-1");
        let matches: Vec<_> = effective.iter().filter(|s| s.name == "Global Server").collect();
        assert_eq!(matches.len(), 1, "a global server visible via both the agent and its bundle must not be duplicated: {effective:?}");
    }

    #[test]
    fn effective_mcp_servers_is_unaffected_when_agent_has_no_bundle() {
        let store = make_store();
        let mut def = crate::backend::storage::store::AgentDefinition {
            conversation_visibility: crate::backend::storage::agents::default_conversation_visibility(),
            id: "agent-no-bundle".to_string(),
            slug: String::new(),
            name: "No Bundle".to_string(),
            icon: String::new(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1_700_000_000_000,
            agent_type: "host".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 1_700_000_000_000,
            user_hidden: 0,
            container_image: String::new(),
            container_volumes: "[]".to_string(),
            container_name: String::new(),
            use_ambient_login: 0,
            auto_continue_enabled: 0,
            model_vendor_base_url: String::new(),
            memory_id: String::new(),
        };
        store.agent_def_insert(&mut def).unwrap();
        store.mcp_server_upsert_unique_global(&server("global-1", "Global Server", true)).unwrap();

        let effective = store.effective_mcp_servers("agent-no-bundle");
        assert_eq!(effective.len(), 1, "an agent with no bundle still sees globals, unaffected by the new union logic: {effective:?}");
    }
}
