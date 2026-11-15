// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A container agent's own CPU and memory limits (`db_agent_container_limits`), beside the agent
//! definition rather than a column of it: a column of `db_agents` is read and written in a few dozen
//! places. An absent row, or an empty value, is "not set": the app-wide setting and then the default
//! computed from the Docker daemon decide (`backend::container::limits`). `0` is "no limit".
//! `docs/specs/SPEC_CONTAINER_AGENT_RESOURCE_LIMITS_2026_10_29.md` §3.
//!
//! The table is created here, on first use (`CREATE TABLE IF NOT EXISTS`), rather than in the schema batch of
//! `migrations.rs`: no schema bump, since an older build ignores a table it does not know, and that file has a size ceiling.

use rusqlite::{params, OptionalExtension};

use super::error::StoreError;
use super::store::Store;
use agentmux_common::time::now_secs;

const CREATE_TABLE: &str = "CREATE TABLE IF NOT EXISTS db_agent_container_limits (
    agent_id   TEXT PRIMARY KEY,
    memory_mb  TEXT NOT NULL DEFAULT '',
    cpus       TEXT NOT NULL DEFAULT '',
    updated_at INTEGER NOT NULL DEFAULT 0
)";

impl Store {
    /// `(memory_mb, cpus)` as stored for `agent_id`, or `None` when it has no row.
    pub fn container_limits_get(&self, agent_id: &str) -> Result<Option<(String, String)>, StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(CREATE_TABLE)?;
        Ok(conn
            .query_row(
                "SELECT memory_mb, cpus FROM db_agent_container_limits WHERE agent_id = ?1",
                params![agent_id.to_lowercase()],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .optional()?)
    }

    /// Stores both values for `agent_id`; empty means "not set".
    pub fn container_limits_set(&self, agent_id: &str, memory_mb: &str, cpus: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute_batch(CREATE_TABLE)?;
        conn.execute(
            "INSERT INTO db_agent_container_limits (agent_id, memory_mb, cpus, updated_at) VALUES (?1, ?2, ?3, ?4)
             ON CONFLICT(agent_id) DO UPDATE SET memory_mb = ?2, cpus = ?3, updated_at = ?4",
            params![agent_id.to_lowercase(), memory_mb, cpus, now_secs()],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_agent_with_no_row_has_no_limits_set_and_a_set_replaces_the_row() {
        let store = Store::open(std::path::Path::new(":memory:")).unwrap();
        assert_eq!(store.container_limits_get("Aria").unwrap(), None);
        store.container_limits_set("Aria", "4096", "").unwrap();
        assert_eq!(store.container_limits_get("aria").unwrap(), Some(("4096".to_string(), String::new())), "keyed case-insensitively");
        store.container_limits_set("aria", "0", "2").unwrap();
        assert_eq!(store.container_limits_get("ARIA").unwrap(), Some(("0".to_string(), "2".to_string())));
    }
}
