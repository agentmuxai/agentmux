// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Who added, changed and bound an MCP server.
//!
//! A server row says who added it and who last changed it (`db_mcp_server_origin`, in the store that
//! holds `db_mcp_servers`); a binding says who bound the server to an agent and when
//! (`db_agent_mcp_binding`, in the store that holds `db_agent_mcp_ref`). The label is the one the
//! record of what a connection did uses (`rpc::actor::Actor::label`): `agent:<id>`, `operator`,
//! `person:<id>`, `unproven`, `agentmux`. When a row or a bind was made before this was kept, or by
//! a path that records nothing, it reads [`UNKNOWN`]. When it was added and last changed are the
//! row's own `created_at` and `updated_at`.

use std::collections::HashMap;

use rusqlite::params;
use serde::{Deserialize, Serialize};

use super::error::StoreError;
use super::store::Store;

/// What a row or a bind reads when nobody recorded who made it.
pub const UNKNOWN: &str = "unknown";

/// Who added a server and who last changed it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct McpOrigin {
    pub added_by: String,
    pub changed_by: String,
}

impl Default for McpOrigin {
    fn default() -> Self {
        McpOrigin { added_by: UNKNOWN.to_string(), changed_by: UNKNOWN.to_string() }
    }
}

/// One agent a server is bound to, with who bound it and when.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct McpBinding {
    pub agent_id: String,
    pub bound_by: String,
    /// Epoch milliseconds; 0 when it was not recorded.
    #[ts(type = "number")]
    pub bound_at: i64,
}

impl Store {
    /// Record a write of server `mcp_id`: `changed_by` always, `added_by` only when no origin is recorded yet. Call on
    /// the store that holds `db_mcp_servers`. A server that existed before its origin was kept passes
    /// [`UNKNOWN`] as `added_by`.
    pub(crate) fn mcp_origin_record(&self, mcp_id: &str, added_by: &str, changed_by: &str) -> Result<(), StoreError> {
        self.conn.lock().unwrap().execute(
            "INSERT INTO db_mcp_server_origin (mcp_id, added_by, changed_by) VALUES (?1, ?2, ?3)
             ON CONFLICT(mcp_id) DO UPDATE SET changed_by = excluded.changed_by",
            params![mcp_id, added_by, changed_by],
        )?;
        Ok(())
    }

    /// Every server's origin, by server id; a server with no entry has none recorded.
    pub fn mcp_origin_all(&self) -> Result<HashMap<String, McpOrigin>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT mcp_id, added_by, changed_by FROM db_mcp_server_origin")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, McpOrigin { added_by: row.get(1)?, changed_by: row.get(2)? }))
        })?;
        let mut out = HashMap::new();
        for r in rows {
            let (id, origin) = r?;
            out.insert(id, origin);
        }
        Ok(out)
    }

    /// One server's origin; [`McpOrigin::default`] when none is recorded.
    pub fn mcp_origin_of(&self, mcp_id: &str) -> Result<McpOrigin, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT added_by, changed_by FROM db_mcp_server_origin WHERE mcp_id = ?1")?;
        let found = stmt.query_row(params![mcp_id], |row| Ok(McpOrigin { added_by: row.get(0)?, changed_by: row.get(1)? }));
        match found {
            Ok(origin) => Ok(origin),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(McpOrigin::default()),
            Err(e) => Err(e.into()),
        }
    }

    pub(crate) fn mcp_origin_delete(&self, mcp_id: &str) -> Result<(), StoreError> {
        self.conn.lock().unwrap().execute("DELETE FROM db_mcp_server_origin WHERE mcp_id = ?1", params![mcp_id])?;
        Ok(())
    }

    /// Record who bound `mcp_id` to `agent_id` and when. The first record stays. Call on the store that
    /// holds `db_agent_mcp_ref`, after the bind itself.
    pub(crate) fn mcp_binding_record(&self, agent_id: &str, mcp_id: &str, bound_by: &str, bound_at: i64) -> Result<(), StoreError> {
        self.conn.lock().unwrap().execute(
            "INSERT OR IGNORE INTO db_agent_mcp_binding (agent_id, mcp_id, bound_by, bound_at) VALUES (?1, ?2, ?3, ?4)",
            params![agent_id, mcp_id, bound_by, bound_at],
        )?;
        Ok(())
    }

    /// For each server bound to `agent_id`: who bound it and when. A bind with no record reads
    /// [`UNKNOWN`] and 0.
    pub(crate) fn mcp_bindings_of_agent(&self, agent_id: &str) -> Result<HashMap<String, (String, i64)>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT r.mcp_id, COALESCE(b.bound_by, ?2), COALESCE(b.bound_at, 0)
             FROM db_agent_mcp_ref r
             LEFT JOIN db_agent_mcp_binding b ON b.agent_id = r.agent_id AND b.mcp_id = r.mcp_id
             WHERE r.agent_id = ?1",
        )?;
        let rows = stmt.query_map(params![agent_id, UNKNOWN], |row| {
            Ok((row.get::<_, String>(0)?, (row.get::<_, String>(1)?, row.get::<_, i64>(2)?)))
        })?;
        let mut out = HashMap::new();
        for r in rows {
            let (id, bound) = r?;
            out.insert(id, bound);
        }
        Ok(out)
    }

    /// For each server: the agents it is bound to, who bound each and when, by agent id.
    pub(crate) fn mcp_bindings_by_server(&self) -> Result<HashMap<String, Vec<McpBinding>>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT r.mcp_id, r.agent_id, COALESCE(b.bound_by, ?1), COALESCE(b.bound_at, 0)
             FROM db_agent_mcp_ref r
             LEFT JOIN db_agent_mcp_binding b ON b.agent_id = r.agent_id AND b.mcp_id = r.mcp_id
             ORDER BY r.mcp_id, r.agent_id",
        )?;
        let rows = stmt.query_map(params![UNKNOWN], |row| {
            Ok((
                row.get::<_, String>(0)?,
                McpBinding { agent_id: row.get(1)?, bound_by: row.get(2)?, bound_at: row.get(3)? },
            ))
        })?;
        let mut out: HashMap<String, Vec<McpBinding>> = HashMap::new();
        for r in rows {
            let (mcp_id, binding) = r?;
            out.entry(mcp_id).or_default().push(binding);
        }
        Ok(out)
    }

    pub(crate) fn mcp_binding_delete(&self, agent_id: &str, mcp_id: &str) -> Result<(), StoreError> {
        self.conn
            .lock()
            .unwrap()
            .execute("DELETE FROM db_agent_mcp_binding WHERE agent_id = ?1 AND mcp_id = ?2", params![agent_id, mcp_id])?;
        Ok(())
    }

    pub(crate) fn mcp_binding_delete_server(&self, mcp_id: &str) -> Result<(), StoreError> {
        self.conn.lock().unwrap().execute("DELETE FROM db_agent_mcp_binding WHERE mcp_id = ?1", params![mcp_id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::mcp_servers::McpServer;

    fn server(id: &str, name: &str, global: bool) -> McpServer {
        McpServer { id: id.into(), name: name.into(), transport: "stdio".into(), config: "{}".into(), is_global: global, created_at: 5, updated_at: 5 }
    }

    fn agent(store: &Store, id: &str) {
        store.conn.lock().unwrap().execute("INSERT INTO db_agents (id, name, provider) VALUES (?1, 'A', 'claude')", params![id]).unwrap();
    }

    #[test]
    fn a_write_records_who_added_it_once_and_who_changed_it_each_time() {
        let store = Store::open_in_memory().unwrap();
        store.mcp_server_upsert_unique_global(&server("g", "Catalog", true), "operator").unwrap();
        store.mcp_server_upsert_unique_global(&server("g", "Catalog", true), "agent:korp").unwrap();
        assert_eq!(store.mcp_origin_of("g").unwrap(), McpOrigin { added_by: "operator".into(), changed_by: "agent:korp".into() });
    }

    #[test]
    fn a_server_that_existed_before_it_was_recorded_has_no_known_adder() {
        let store = Store::open_in_memory().unwrap();
        store.mcp_server_insert_raw(&server("old", "Old", true)).unwrap();
        assert_eq!(store.mcp_origin_of("old").unwrap(), McpOrigin::default());
        store.mcp_server_upsert_unique_global(&server("old", "Old", true), "operator").unwrap();
        assert_eq!(store.mcp_origin_of("old").unwrap(), McpOrigin { added_by: UNKNOWN.into(), changed_by: "operator".into() });
    }

    #[test]
    fn a_bind_records_its_binder_once_and_unbinding_and_deleting_forget_it() {
        let store = Store::open_in_memory().unwrap();
        agent(&store, "a1");
        agent(&store, "a2");
        store.mcp_server_upsert_unique_global(&server("g", "Catalog", true), "operator").unwrap();
        store.mcp_server_bind(&store, "a1", "g", "agent:a1", 10).unwrap();
        store.mcp_server_bind(&store, "a1", "g", "operator", 20).unwrap();
        store.mcp_server_bind(&store, "a2", "g", "operator", 30).unwrap();
        let bindings = store.mcp_bindings_by_server().unwrap().remove("g").unwrap();
        assert_eq!(
            bindings,
            vec![
                McpBinding { agent_id: "a1".into(), bound_by: "agent:a1".into(), bound_at: 10 },
                McpBinding { agent_id: "a2".into(), bound_by: "operator".into(), bound_at: 30 },
            ]
        );

        assert!(store.mcp_server_unbind("a1", "g").unwrap());
        store.mcp_server_bind(&store, "a1", "g", "operator", 40).unwrap();
        assert_eq!(store.mcp_bindings_of_agent("a1").unwrap()["g"], ("operator".to_string(), 40));

        assert!(store.mcp_server_delete(&store, "g").unwrap());
        assert!(store.mcp_bindings_by_server().unwrap().is_empty());
        assert_eq!(store.mcp_origin_of("g").unwrap(), McpOrigin::default());
    }

    #[test]
    fn a_bind_made_without_a_record_reads_unknown() {
        let store = Store::open_in_memory().unwrap();
        agent(&store, "a1");
        store.mcp_server_insert_raw(&server("g", "Catalog", true)).unwrap();
        store.conn.lock().unwrap().execute("INSERT INTO db_agent_mcp_ref (agent_id, mcp_id) VALUES ('a1', 'g')", []).unwrap();
        assert_eq!(store.mcp_bindings_of_agent("a1").unwrap()["g"], (UNKNOWN.to_string(), 0));
        // Binding it again does not pretend to be the first bind.
        store.mcp_server_bind(&store, "a1", "g", "operator", 50).unwrap();
        assert_eq!(store.mcp_bindings_of_agent("a1").unwrap()["g"], (UNKNOWN.to_string(), 0));
    }

    #[test]
    fn deleting_an_agent_forgets_who_bound_what_to_it() {
        let store = Store::open_in_memory().unwrap();
        agent(&store, "a1");
        store.mcp_server_upsert_unique_global(&server("g", "Catalog", true), "operator").unwrap();
        store.mcp_server_bind(&store, "a1", "g", "operator", 10).unwrap();
        store.conn.lock().unwrap().execute("DELETE FROM db_agents WHERE id = 'a1'", []).unwrap();
        let left: i64 = store.conn.lock().unwrap().query_row("SELECT COUNT(*) FROM db_agent_mcp_binding", [], |r| r.get(0)).unwrap();
        assert_eq!(left, 0);
    }
}
