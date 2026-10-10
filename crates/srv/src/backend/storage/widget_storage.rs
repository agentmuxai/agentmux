// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A widget package's own key-value store, the `storage` permission
//! (docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §6.3): per
//! package, not per pane, values as JSON text. The limits are the caller's
//! (`backend/widget_access.rs`); this module only counts bytes.

use rusqlite::{params, OptionalExtension};

use super::error::StoreError;
use super::store::Store;

impl Store {
    pub fn widget_storage_get(&self, widget_id: &str, key: &str) -> Result<Option<String>, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(conn
            .query_row(
                "SELECT value FROM db_widget_storage WHERE widget_id = ?1 AND key = ?2",
                params![widget_id, key],
                |row| row.get(0),
            )
            .optional()?)
    }

    /// Sets `key`, unless the package's keys and values would then take more
    /// than `max_total` bytes; returns whether it was set.
    pub fn widget_storage_set(&self, widget_id: &str, key: &str, value: &str, max_total: usize) -> Result<bool, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let others: i64 = tx.query_row(
            "SELECT COALESCE(SUM(length(CAST(key AS BLOB)) + length(CAST(value AS BLOB))), 0)
             FROM db_widget_storage WHERE widget_id = ?1 AND key <> ?2",
            params![widget_id, key],
            |row| row.get(0),
        )?;
        if others as usize + key.len() + value.len() > max_total {
            return Ok(false);
        }
        tx.execute(
            "INSERT OR REPLACE INTO db_widget_storage (widget_id, key, value) VALUES (?1, ?2, ?3)",
            params![widget_id, key, value],
        )?;
        tx.commit()?;
        Ok(true)
    }

    pub fn widget_storage_delete(&self, widget_id: &str, key: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM db_widget_storage WHERE widget_id = ?1 AND key = ?2", params![widget_id, key])?;
        Ok(())
    }

    /// The package's keys that start with `prefix`, sorted.
    pub fn widget_storage_keys(&self, widget_id: &str, prefix: &str) -> Result<Vec<String>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT key FROM db_widget_storage WHERE widget_id = ?1 AND substr(key, 1, length(?2)) = ?2 ORDER BY key",
        )?;
        let keys = stmt.query_map(params![widget_id, prefix], |row| row.get(0))?.collect::<Result<Vec<String>, _>>()?;
        Ok(keys)
    }

    /// Everything the package stored (when it's uninstalled).
    pub fn widget_storage_purge(&self, widget_id: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute("DELETE FROM db_widget_storage WHERE widget_id = ?1", params![widget_id])?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    #[test]
    fn keeps_each_package_apart_within_its_budget() {
        let store = Store::open(Path::new(":memory:")).unwrap();
        assert!(store.widget_storage_set("acme.a", "note:1", "\"hi\"", 100).unwrap());
        assert!(store.widget_storage_set("acme.a", "note:2", "\"yo\"", 100).unwrap());
        assert!(store.widget_storage_set("acme.b", "note:1", "\"other\"", 100).unwrap());
        assert_eq!(store.widget_storage_get("acme.a", "note:1").unwrap().as_deref(), Some("\"hi\""));
        assert_eq!(store.widget_storage_keys("acme.a", "note:").unwrap(), vec!["note:1", "note:2"]);
        assert!(store.widget_storage_keys("acme.a", "x").unwrap().is_empty());
        // Over the budget: refused, and nothing changes.
        assert!(!store.widget_storage_set("acme.a", "big", &"x".repeat(90), 100).unwrap());
        assert_eq!(store.widget_storage_get("acme.a", "big").unwrap(), None);
        // Replacing a value counts the new size, not both.
        assert!(store.widget_storage_set("acme.a", "note:1", &"y".repeat(70), 100).unwrap());
        store.widget_storage_delete("acme.a", "note:2").unwrap();
        assert_eq!(store.widget_storage_keys("acme.a", "").unwrap(), vec!["note:1"]);
        store.widget_storage_purge("acme.a").unwrap();
        assert!(store.widget_storage_keys("acme.a", "").unwrap().is_empty());
        assert_eq!(store.widget_storage_get("acme.b", "note:1").unwrap().as_deref(), Some("\"other\""));
    }
}
