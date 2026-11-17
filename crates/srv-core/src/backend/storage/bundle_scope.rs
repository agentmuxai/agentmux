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
//! `machine` row: those entries were copied by hand between machines and may
//! differ on purpose, so a push does not overwrite them silently. Such a row is
//! a default, not a choice: `chosen` is 0 on it, and 1 on a row an owner set
//! with [`Store::bundle_scope_set`]. A newer write from the fleet replaces an
//! entry whose `machine` row was only the default (and the entry then follows
//! the fleet); a `machine` row an owner chose is never overwritten.

use rusqlite::{params, OptionalExtension};

use crate::migrations::table_exists;

use super::error::StoreError;
use super::store::Store;

pub const SCOPE_FLEET: &str = "fleet";
pub const SCOPE_MACHINE: &str = "machine";

const SCHEMA: &str = "CREATE TABLE IF NOT EXISTS db_bundle_scope (
        bundle_id TEXT PRIMARY KEY,
        scope TEXT NOT NULL,
        chosen INTEGER NOT NULL DEFAULT 0
    );";

/// Create the table if it is missing, marking the entries that already exist as
/// `machine`, and add the `chosen` column to a table made before it. A no-op
/// once both are there. Rows that exist when the column is added count as
/// defaults: only a row set after this build is known to be an owner's choice.
fn ensure(conn: &rusqlite::Connection) -> Result<(), StoreError> {
    if table_exists(conn, "db_bundle_scope")? {
        let has_chosen: i64 =
            conn.query_row("SELECT COUNT(*) FROM pragma_table_info('db_bundle_scope') WHERE name = 'chosen'", [], |r| r.get(0))?;
        if has_chosen == 0 {
            conn.execute_batch("ALTER TABLE db_bundle_scope ADD COLUMN chosen INTEGER NOT NULL DEFAULT 0;")?;
        }
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

    /// Set an entry's scope to `fleet` or `machine`, as its owner's choice.
    pub fn bundle_scope_set(&self, id: &str, scope: &str) -> Result<(), StoreError> {
        if scope != SCOPE_FLEET && scope != SCOPE_MACHINE {
            return Err(StoreError::Other(format!("scope must be \"{SCOPE_FLEET}\" or \"{SCOPE_MACHINE}\", not \"{scope}\"")));
        }
        let conn = self.conn.lock().unwrap();
        ensure(&conn)?;
        conn.execute(
            "INSERT INTO db_bundle_scope (bundle_id, scope, chosen) VALUES (?1, ?2, 1)
             ON CONFLICT(bundle_id) DO UPDATE SET scope = excluded.scope, chosen = 1",
            params![id, scope],
        )?;
        Ok(())
    }

    /// Whether the entry is `machine` only because the scope table was created while it existed,
    /// and no owner has set it since.
    pub fn bundle_scope_is_default_machine(&self, id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        ensure(&conn)?;
        let row: Option<(String, i64)> = conn
            .query_row("SELECT scope, chosen FROM db_bundle_scope WHERE bundle_id = ?1", params![id], |r| Ok((r.get(0)?, r.get(1)?)))
            .optional()?;
        Ok(matches!(row, Some((scope, 0)) if scope == SCOPE_MACHINE))
    }

    /// Make an entry follow the fleet, without recording it as an owner's choice: the way a
    /// default `machine` row is lifted when the fleet's newer write replaces the entry.
    pub fn bundle_scope_follow_fleet(&self, id: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        ensure(&conn)?;
        conn.execute(
            "INSERT INTO db_bundle_scope (bundle_id, scope, chosen) VALUES (?1, ?2, 0)
             ON CONFLICT(bundle_id) DO UPDATE SET scope = excluded.scope, chosen = 0",
            params![id, SCOPE_FLEET],
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
    fn a_machine_row_made_by_the_migration_is_a_default_and_one_an_owner_sets_is_a_choice() {
        let store = Store::open_in_memory().unwrap();
        store.bundle_upsert(&bundle("old")).unwrap();
        store.bundle_upsert(&bundle("kept")).unwrap();
        store.bundle_scope_ensure().unwrap();
        store.bundle_scope_set("kept", SCOPE_MACHINE).unwrap();

        assert!(store.bundle_scope_is_default_machine("old").unwrap());
        assert!(!store.bundle_scope_is_default_machine("kept").unwrap(), "the owner chose it");
        assert!(!store.bundle_scope_is_default_machine("never-written").unwrap(), "no row is fleet, not a default machine row");

        store.bundle_scope_follow_fleet("old").unwrap();
        assert_eq!(store.bundle_scope_get("old").unwrap(), SCOPE_FLEET);
        assert!(!store.bundle_scope_is_default_machine("old").unwrap());
    }

    #[test]
    fn a_table_made_before_the_chosen_column_gets_it_and_its_rows_count_as_defaults() {
        let store = Store::open_in_memory().unwrap();
        {
            let conn = store.conn.lock().unwrap();
            conn.execute_batch(
                "CREATE TABLE db_bundle_scope (bundle_id TEXT PRIMARY KEY, scope TEXT NOT NULL);
                 INSERT INTO db_bundle_scope VALUES ('a', 'machine'), ('b', 'fleet');",
            )
            .unwrap();
        }
        assert!(store.bundle_scope_is_default_machine("a").unwrap());
        assert_eq!(store.bundle_scope_get("b").unwrap(), SCOPE_FLEET);
        store.bundle_scope_set("a", SCOPE_MACHINE).unwrap();
        assert!(!store.bundle_scope_is_default_machine("a").unwrap());
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
