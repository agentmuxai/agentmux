// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which Global Memory entries follow the fleet
//! (task #12439, "agents operate other machines", section 4.6).
//!
//! An entry's scope is `fleet` (pushed to every proven machine, last write
//! wins) or `machine` (this machine only). Rows live in `db_bundle_scope`,
//! beside `db_bundles`, created on first use without a schema bump (like
//! `db_skill_optout`). An entry with no row is `fleet`, so every new entry
//! follows the fleet by default.
//!
//! The first time the table is created, every entry that exists then gets a
//! `machine` row: those entries were copied by hand between machines, may
//! differ on purpose, and must not be overwritten by whichever machine pushes
//! first. Flipping one to `fleet` is the owner's choice.

use rusqlite::{params, OptionalExtension};

use super::error::StoreError;
use super::store::Store;

pub const SCOPE_FLEET: &str = "fleet";
pub const SCOPE_MACHINE: &str = "machine";

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS db_bundle_scope (
        bundle_id TEXT PRIMARY KEY,
        scope TEXT NOT NULL
    );";

fn table_exists(conn: &rusqlite::Connection, name: &str) -> Result<bool, StoreError> {
    Ok(conn
        .query_row("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?1", params![name], |_| Ok(()))
        .optional()?
        .is_some())
}

/// Create the table if it is missing, marking the entries that already exist as
/// `machine`. A no-op once the table is there.
fn ensure(conn: &rusqlite::Connection) -> Result<(), StoreError> {
    if table_exists(conn, "db_bundle_scope")? {
        return Ok(());
    }
    let tx = conn.unchecked_transaction()?;
    tx.execute_batch(SCHEMA)?;
    if table_exists(&tx, "db_bundles")? {
        tx.execute("INSERT OR IGNORE INTO db_bundle_scope (bundle_id, scope) SELECT id, ?1 FROM db_bundles", params![SCOPE_MACHINE])?;
    }
    tx.commit()?;
    Ok(())
}

impl Store {
    /// Create the scope table now, so the entries that exist before this build
    /// are the ones marked `machine` and not the first ones written after it.
    /// Call once when the identity store is opened.
    pub fn bundle_scope_ensure(&self) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        ensure(&conn)
    }

    /// The entry's scope: its row, or `fleet` when it has none.
    pub fn bundle_scope_get(&self, id: &str) -> Result<String, StoreError> {
        let conn = self.conn.lock().unwrap();
        ensure(&conn)?;
        let row: Option<String> = conn
            .query_row("SELECT scope FROM db_bundle_scope WHERE bundle_id = ?1", params![id], |r| r.get(0))
            .optional()?;
        Ok(row.unwrap_or_else(|| SCOPE_FLEET.to_string()))
    }

    /// The ids of the entries that stay on this machine.
    pub fn bundle_scope_machine_ids(&self) -> Result<std::collections::HashSet<String>, StoreError> {
        let conn = self.conn.lock().unwrap();
        ensure(&conn)?;
        let mut stmt = conn.prepare("SELECT bundle_id FROM db_bundle_scope WHERE scope = ?1")?;
        let rows = stmt.query_map(params![SCOPE_MACHINE], |r| r.get::<_, String>(0))?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// Set an entry's scope to `fleet` or `machine`.
    // The Memory pane and the MCP tool call this in a later PR.
    #[allow(dead_code)]
    pub fn bundle_scope_set(&self, id: &str, scope: &str) -> Result<(), StoreError> {
        if scope != SCOPE_FLEET && scope != SCOPE_MACHINE {
            return Err(StoreError::Other(format!("scope must be \"{SCOPE_FLEET}\" or \"{SCOPE_MACHINE}\", not \"{scope}\"")));
        }
        let conn = self.conn.lock().unwrap();
        ensure(&conn)?;
        conn.execute(
            "INSERT INTO db_bundle_scope (bundle_id, scope) VALUES (?1, ?2)
             ON CONFLICT(bundle_id) DO UPDATE SET scope = excluded.scope",
            params![id, scope],
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::store::Bundle;

    fn bundle(id: &str) -> Bundle {
        serde_json::from_value(serde_json::json!({
            "id": id, "name": id, "instructions": "x", "is_global": true, "mcp_servers": "[]", "skills": "[]",
        }))
        .unwrap()
    }

    #[test]
    fn entries_that_exist_when_the_table_is_created_stay_on_this_machine_and_new_ones_follow_the_fleet() {
        let store = Store::open_in_memory().unwrap();
        store.bundle_upsert(&bundle("old")).unwrap();
        store.bundle_scope_ensure().unwrap();
        store.bundle_upsert(&bundle("new")).unwrap();

        assert_eq!(store.bundle_scope_get("old").unwrap(), SCOPE_MACHINE);
        assert_eq!(store.bundle_scope_get("new").unwrap(), SCOPE_FLEET);
        assert_eq!(store.bundle_scope_get("never-written").unwrap(), SCOPE_FLEET);
    }

    #[test]
    fn ensure_marks_the_existing_entries_once() {
        let store = Store::open_in_memory().unwrap();
        store.bundle_scope_ensure().unwrap();
        store.bundle_upsert(&bundle("later")).unwrap();
        store.bundle_scope_ensure().unwrap();
        assert_eq!(store.bundle_scope_get("later").unwrap(), SCOPE_FLEET, "a second ensure must not mark it");
    }

    #[test]
    fn the_scope_can_be_flipped_and_only_two_values_are_accepted() {
        let store = Store::open_in_memory().unwrap();
        store.bundle_upsert(&bundle("a")).unwrap();
        store.bundle_scope_set("a", SCOPE_MACHINE).unwrap();
        assert_eq!(store.bundle_scope_get("a").unwrap(), SCOPE_MACHINE);
        store.bundle_scope_set("a", SCOPE_FLEET).unwrap();
        assert_eq!(store.bundle_scope_get("a").unwrap(), SCOPE_FLEET);
        assert!(store.bundle_scope_set("a", "everywhere").is_err());
    }
}
