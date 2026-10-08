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
use super::managed::{ManagedResource, Owner};
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
    /// Drop every skill and MCP ref belonging to `bundle_id`. Bundles no
    /// longer carry MCP servers (`m0036_drop_bundle_mcp`), but an older build
    /// sharing the channel can still write a bundle MCP ref, so those go too.
    ///
    /// Call after deleting the bundle itself. `bundle_delete` lives on the
    /// store that owns `db_bundles`, which does not have the ref tables at all
    /// — they sit beside the catalog tables they key into — so the cleanup
    /// cannot happen inside it and has to be driven from the handler, where
    /// both stores are in scope. Without it a deleted bundle leaves its refs
    /// behind, and a future bundle reusing that id would inherit them.
    ///
    /// Returns `(skill_refs_removed, mcp_refs_removed)`.
    pub fn bundle_unbind_all_components(&self, bundle_id: &str) -> Result<(usize, usize), StoreError> {
        let skills = self
            .managed_unbind_all_for_bundle::<super::skills::Skill>(bundle_id)?;
        let mcp = self.managed_unbind_all_for_bundle::<McpServer>(bundle_id)?;
        Ok((skills, mcp))
    }
}

impl ManagedResource for McpServer {
    const TABLE: &'static str = "db_mcp_servers";
    const REF_COL: &'static str = "mcp_id";
    const AGENT_REF_TABLE: &'static str = "db_agent_mcp_ref";
    // Still defined, so an older build on the same channel finds it, but
    // nothing reads it: bundles don't carry MCP servers
    // (`SPEC_BUNDLE_CONTENTS_MEMORY_NOT_MCP_2026_10_07.md` §3.1).
    const BUNDLE_REF_TABLE: &'static str = "db_bundle_mcp_ref";
    const COLUMNS: &'static [&'static str] =
        &["id", "name", "transport", "config", "is_global", "created_at", "updated_at"];
    const UPDATE_COLUMNS: &'static [&'static str] = &["name", "transport", "config", "updated_at"];
    const NAME_NOUN: &'static str = "server";
    const ARTICLE_NOUN: &'static str = "an MCP server";
    const KIND: &'static str = "MCP server";

    fn from_row(row: &rusqlite::Row<'_>) -> rusqlite::Result<Self> {
        Ok(McpServer {
            id: row.get(0)?,
            name: row.get(1)?,
            transport: row.get(2)?,
            config: row.get(3)?,
            is_global: row.get::<_, i64>(4)? != 0,
            created_at: row.get(5)?,
            updated_at: row.get(6)?,
        })
    }
    fn values(&self) -> Vec<rusqlite::types::Value> {
        use rusqlite::types::Value;
        vec![
            Value::Text(self.id.clone()),
            Value::Text(self.name.clone()),
            Value::Text(self.transport.clone()),
            Value::Text(self.config.clone()),
            Value::Integer(if self.is_global { 1 } else { 0 }),
            Value::Integer(self.created_at),
            Value::Integer(self.updated_at),
        ]
    }
    fn id(&self) -> &str {
        &self.id
    }
    fn name(&self) -> &str {
        &self.name
    }
    fn is_global(&self) -> bool {
        self.is_global
    }
    fn updated_at(&self) -> i64 {
        self.updated_at
    }
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
    pub fn mcp_server_list(&self, catalog: &Store, agent_id: &str) -> Result<Vec<McpServerListItem>, StoreError> {
        Ok(self
            .managed_list::<McpServer>(catalog, Owner::Agent, agent_id)?
            .into_iter()
            .map(|(server, bound_to_agent)| McpServerListItem { server, bound_to_agent })
            .collect())
    }

    /// Every MCP server an agent launches with: the ones bound to it in
    /// Connectors, plus the global ones (`mcp_server_list`). Not its bundles':
    /// a bundle carries no MCP servers
    /// (`SPEC_BUNDLE_CONTENTS_MEMORY_NOT_MCP_2026_10_07.md` §3.1). Single
    /// source of truth for `write_agent_config_files`.
    pub fn effective_mcp_servers(&self, catalog: &Store, agent_id: &str) -> Vec<McpServer> {
        self.mcp_server_list(catalog, agent_id)
            .unwrap_or_default()
            .into_iter()
            .map(|item| item.server)
            .collect()
    }

    /// List every GLOBAL MCP server — the Armory catalog view. Unlike
    /// `mcp_server_list`, this takes no `agent_id` and never includes an
    /// agent's private servers; it backs the window-scoped `mcp.catalog.*`
    /// App API (no `check_s1`, so there is no agent context to scope by).
    /// Each row carries `bound_count` — how many agents currently hold a
    /// `db_agent_mcp_ref` to it — per SPEC_V1_MCP_SKILLS_PRIMITIVES_2026_06_30.md
    /// §8 ("used by N agents"), tracked as gap #2 of #1960.
    pub fn mcp_server_list_global(&self, catalog: &Store) -> Result<Vec<McpServerCatalogItem>, StoreError> {
        Ok(self
            .managed_list_global::<McpServer>(catalog)?
            .into_iter()
            .map(|(server, bound_count)| McpServerCatalogItem { server, bound_count })
            .collect())
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
    pub fn mcp_server_list_global_for_agent(&self, catalog: &Store, agent_id: &str) -> Result<Vec<McpServerListItem>, StoreError> {
        Ok(self
            .managed_list_global_for_agent::<McpServer>(catalog, agent_id)?
            .into_iter()
            .map(|(server, bound_to_agent)| McpServerListItem { server, bound_to_agent })
            .collect())
    }

    /// Get a standalone MCP server by id.
    pub fn mcp_server_get(&self, id: &str) -> Result<Option<McpServer>, StoreError> {
        self.managed_get::<McpServer>(id)
    }

    /// Delete a standalone MCP server and purge ref rows (both agent- and
    /// bundle-level — FK cascades may be off on some builds, same reasoning
    /// as `skill_delete`). Returns true if deleted.
    pub fn mcp_server_delete(&self, catalog: &Store, id: &str) -> Result<bool, StoreError> {
        self.managed_delete::<McpServer>(catalog, id)
    }

    /// Bind an MCP server to an agent (insert ref row). Idempotent —
    /// binding an already-bound pair is a silent no-op success.
    ///
    /// Errors if `agent_id` isn't a LOCAL agent:
    /// `db_agent_mcp_ref.agent_id` has an ON-enforced FK to
    /// `db_agents(id)` (store.rs; was `db_agent_definitions(id)` before
    /// Phase 3c, #3088), but the Armory's agent
    /// picker (`ListAgentDefinitionsCommand` → `agent_def_list()`) also
    /// lists cross-channel agents that only exist in another channel's
    /// local database. Binding one of those would otherwise have the FK
    /// silently swallow the `INSERT OR IGNORE` — indistinguishable, by
    /// affected-row-count alone, from the equally-silent "already bound"
    /// case — reporting success while creating nothing. Same fix as
    /// skill_bind — see
    /// docs/reports/REPORT_ARMORY_SKILLS_MARKDOWN_AND_BIND_BUG_2026_07_27.md.
    pub fn mcp_server_bind(&self, catalog: &Store, agent_id: &str, mcp_id: &str) -> Result<(), StoreError> {
        self.managed_bind_agent::<McpServer>(catalog, agent_id, mcp_id)
    }

    /// Unbind an MCP server from an agent. Returns true if a row was removed.
    pub fn mcp_server_unbind(&self, agent_id: &str, mcp_id: &str) -> Result<bool, StoreError> {
        self.managed_unbind::<McpServer>(Owner::Agent, agent_id, mcp_id)
    }

    /// Atomically upsert an MCP server enforcing per-agent name uniqueness, and
    /// (when `bind_new`) bind it — all in one transaction so concurrent
    /// `mcp.upsert` calls for the same name can't both pass a check and insert
    /// duplicate-named bindings. Returns an error if another server visible to
    /// the agent (bound or global) already uses the name.
    pub fn mcp_server_upsert_unique(
        &self,
        catalog: &Store,
        agent_id: &str,
        server: &McpServer,
        bind_new: bool,
    ) -> Result<(), StoreError> {
        self.managed_upsert_unique(catalog, Owner::Agent, agent_id, server, bind_new, None)
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
        self.managed_upsert_unique_global(server)
    }

    // ── Migration-only raw access — see skills.rs's identical trio (same
    // Phase 2 of SPEC_DURABLE_BINDINGS_2026_09_10.md, same reasoning). ──

    pub(crate) fn mcp_server_list_all_raw(&self) -> Result<Vec<McpServer>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, transport, config, is_global, created_at, updated_at FROM db_mcp_servers",
        )?;
        let rows = stmt.query_map([], |row| {
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
        for r in rows {
            out.push(r?);
        }
        Ok(out)
    }

    /// Returns 1 if a row was actually inserted, 0 if `OR IGNORE` found one
    /// already there — see `Skill::skill_insert_raw`'s doc comment.
    pub(crate) fn mcp_server_insert_raw(&self, server: &McpServer) -> Result<usize, StoreError> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "INSERT OR IGNORE INTO db_mcp_servers
                (id, name, transport, config, is_global, created_at, updated_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                server.id,
                server.name,
                server.transport,
                server.config,
                server.is_global as i64,
                server.created_at,
                server.updated_at,
            ],
        )?;
        Ok(rows)
    }

    pub(crate) fn mcp_server_find_global_by_name(&self, name: &str) -> Result<Option<McpServer>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, name, transport, config, is_global, created_at, updated_at
             FROM db_mcp_servers
             WHERE is_global = 1 AND name = ?1
             LIMIT 1",
        )?;
        let result = stmt.query_row(params![name], |row| {
            Ok(McpServer {
                id: row.get(0)?,
                name: row.get(1)?,
                transport: row.get(2)?,
                config: row.get(3)?,
                is_global: row.get::<_, i64>(4)? != 0,
                created_at: row.get(5)?,
                updated_at: row.get(6)?,
            })
        });
        match result {
            Ok(s) => Ok(Some(s)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Mirrors `Skill::skill_rewrite_ref_id` exactly, for the MCP ref tables
    /// — including its insert-then-delete merge semantics (Codex P2, PR
    /// #3183). Returns the number of ref rows actually repointed — see
    /// `Skill::skill_rewrite_ref_id`'s doc comment.
    pub(crate) fn mcp_server_rewrite_ref_id(&self, old_id: &str, new_id: &str) -> Result<usize, StoreError> {
        if old_id == new_id {
            return Ok(0);
        }
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO db_agent_mcp_ref (agent_id, mcp_id)
                SELECT agent_id, ?2 FROM db_agent_mcp_ref WHERE mcp_id = ?1",
            params![old_id, new_id],
        )?;
        let agent = conn.execute("DELETE FROM db_agent_mcp_ref WHERE mcp_id = ?1", params![old_id])?;
        conn.execute(
            "INSERT OR IGNORE INTO db_bundle_mcp_ref (bundle_id, mcp_id)
                SELECT bundle_id, ?2 FROM db_bundle_mcp_ref WHERE mcp_id = ?1",
            params![old_id, new_id],
        )?;
        let bundle = conn.execute("DELETE FROM db_bundle_mcp_ref WHERE mcp_id = ?1", params![old_id])?;
        Ok(agent + bundle)
    }

    /// Return true if the given MCP server is accessible to the agent (global or bound).
    /// Used for read and mutation access checks.
    pub fn mcp_server_is_accessible_to(&self, catalog: &Store, agent_id: &str, mcp_id: &str) -> Result<bool, StoreError> {
        self.managed_is_accessible_to::<McpServer>(catalog, Owner::Agent, agent_id, mcp_id)
    }

    /// Return true if the agent has a direct ref binding to this MCP server.
    /// Used for delete access — an agent may only delete servers it directly bound.
    pub fn mcp_server_is_bound_to(&self, agent_id: &str, mcp_id: &str) -> Result<bool, StoreError> {
        self.managed_is_bound_to::<McpServer>(Owner::Agent, agent_id, mcp_id)
    }

}


#[cfg(test)]
mod effective_tests {
    use super::*;
    use crate::backend::storage::bundles::Bundle;

    fn server(id: &str, name: &str, is_global: bool) -> McpServer {
        McpServer {
            id: id.to_string(),
            name: name.to_string(),
            transport: "stdio".to_string(),
            config: "{}".to_string(),
            is_global,
            created_at: 1,
            updated_at: 1,
        }
    }

    fn agent(store: &Store, id: &str, memory_id: &str) {
        let mut def = crate::backend::storage::store::AgentDefinition {
            conversation_visibility: crate::backend::storage::agents::default_conversation_visibility(),
            id: id.to_string(),
            slug: String::new(),
            name: id.to_string(),
            icon: String::new(),
            provider: "claude".to_string(),
            description: String::new(),
            working_directory: String::new(),
            shell: String::new(),
            provider_flags: String::new(),
            auto_start: 0,
            restart_on_crash: 0,
            idle_timeout_minutes: 0,
            created_at: 1,
            agent_type: "host".to_string(),
            environment: String::new(),
            agent_bus_id: String::new(),
            is_seeded: 0,
            accounts: String::new(),
            parent_id: String::new(),
            branch_label: String::new(),
            updated_at: 1,
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

    fn bundle_row(id: &str) -> Bundle {
        Bundle {
            id: id.to_string(),
            name: id.to_string(),
            description: String::new(),
            is_blank: false,
            is_global: false,
            provider: String::new(),
            model: String::new(),
            instructions: String::new(),
            instructions_by_provider: "{}".to_string(),
            context_files: "[]".to_string(),
            mcp_servers: "[]".to_string(),
            skills: "[]".to_string(),
            sort_order: 0,
            created_at: 1,
            updated_at: 1,
            is_system: false,
        }
    }

    fn bundle(store: &Store, id: &str) {
        store
            .bundle_upsert(&Bundle {
                id: id.to_string(),
                name: id.to_string(),
                description: String::new(),
                is_blank: false,
                is_global: false,
                provider: String::new(),
                model: String::new(),
                instructions: String::new(),
                instructions_by_provider: "{}".to_string(),
                context_files: "[]".to_string(),
                mcp_servers: "[]".to_string(),
                skills: "[]".to_string(),
                sort_order: 0,
                created_at: 1,
                updated_at: 1,
                is_system: false,
            })
            .unwrap();
    }

    fn bundle_ref(store: &Store, bundle_id: &str, mcp_id: &str) {
        store
            .conn
            .lock()
            .unwrap()
            .execute(
                "INSERT INTO db_bundle_mcp_ref (bundle_id, mcp_id) VALUES (?1, ?2)",
                params![bundle_id, mcp_id],
            )
            .unwrap();
    }

    /// A bundle ref left by an older build reaches no agent: an agent launches
    /// with its own and the global servers only.
    #[test]
    fn an_agent_launches_with_its_own_and_global_servers_not_its_bundles() {
        let store = Store::open_in_memory().unwrap();
        bundle(&store, "own");
        bundle(&store, "picked");
        agent(&store, "agent-1", "own");
        store.agent_bundles_set("agent-1", &["picked".to_string()]).unwrap();

        store.mcp_server_upsert_unique_global(&server("g", "Global", true)).unwrap();
        store.mcp_server_upsert_unique(&store, "agent-1", &server("a", "Agent's", false), true).unwrap();
        for (bundle_id, id, name) in [("own", "b1", "Own bundle's"), ("picked", "b2", "Picked bundle's")] {
            store.mcp_server_insert_raw(&server(id, name, false)).unwrap();
            bundle_ref(&store, bundle_id, id);
        }

        let mut names: Vec<String> =
            store.effective_mcp_servers(&store, "agent-1").into_iter().map(|s| s.name).collect();
        names.sort();
        assert_eq!(names, vec!["Agent's".to_string(), "Global".to_string()]);
    }

    /// A bundle write never stores MCP configs, whatever the caller sends, and
    /// a read never returns any (see `bundles::NO_INLINE_MCP_SERVERS`).
    #[test]
    fn a_bundle_write_never_stores_mcp_servers_and_a_read_never_returns_them() {
        let store = Store::open_in_memory().unwrap();
        let inline = r#"[{"name":"gh","env":{"GITHUB_TOKEN":"secret"}}]"#;
        let mut user = Bundle { mcp_servers: inline.to_string(), ..bundle_row("user") };
        store.bundle_upsert(&user).unwrap();
        user.id = "system".to_string();
        user.name = "system".to_string();
        user.is_system = true;
        store.bundle_upsert_system(&user).unwrap();

        let stored: i64 = store
            .conn
            .lock()
            .unwrap()
            .query_row("SELECT COUNT(*) FROM db_bundles WHERE mcp_servers != '[]'", [], |r| r.get(0))
            .unwrap();
        assert_eq!(stored, 0);

        // Left by an older build: still not read back.
        store.conn.lock().unwrap().execute("UPDATE db_bundles SET mcp_servers = ?1", params![inline]).unwrap();
        for id in ["user", "system"] {
            assert_eq!(store.bundle_get(id).unwrap().unwrap().mcp_servers, "[]");
        }
    }

    #[test]
    fn deleting_a_bundle_still_clears_a_leftover_bundle_ref() {
        let store = Store::open_in_memory().unwrap();
        store.mcp_server_insert_raw(&server("b1", "Leftover", false)).unwrap();
        bundle_ref(&store, "gone", "b1");
        assert_eq!(store.bundle_unbind_all_components("gone").unwrap(), (0, 1));
    }
}
