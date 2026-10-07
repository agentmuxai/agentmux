// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Paired devices: the viewer grants a device receives at pairing, and each
//! agent's "hide from paired devices" flag. See `db_viewer_devices` and
//! `db_agents.hide_from_devices` in migrations.rs (OBJECT_SCHEMA_VERSION v43)
//! and `backend::viewer`.
//!
//! A device's token is never stored, only its SHA-256 (lowercase hex), so a
//! copy of this database opens no feed. Revoking deletes the row.

use rusqlite::params;

use super::error::StoreError;
use super::store::Store;

/// One paired device, as Settings lists it. Never carries the token hash.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewerDevice {
    pub device_id: String,
    pub device_name: String,
    /// Standard base64 of the device's 32-byte public key, '' when it sent none.
    pub device_key: String,
    pub created_ms: i64,
    /// 0 until the device's first request after pairing.
    pub last_seen_ms: i64,
}

impl Store {
    /// Record a new paired device.
    pub fn viewer_device_insert(
        &self,
        device_id: &str,
        token_hash: &str,
        device_name: &str,
        device_key: &str,
        created_ms: i64,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO db_viewer_devices \
             (device_id, token_hash, device_name, device_key, created_ms, last_seen_ms) \
             VALUES (?1, ?2, ?3, ?4, ?5, 0)",
            params![device_id, token_hash, device_name, device_key, created_ms],
        )?;
        Ok(())
    }

    /// Every paired device, oldest first.
    pub fn viewer_device_list(&self) -> Result<Vec<ViewerDevice>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT device_id, device_name, device_key, created_ms, last_seen_ms \
             FROM db_viewer_devices ORDER BY created_ms, device_id",
        )?;
        let rows = stmt.query_map([], |row| {
            Ok(ViewerDevice {
                device_id: row.get(0)?,
                device_name: row.get(1)?,
                device_key: row.get(2)?,
                created_ms: row.get(3)?,
                last_seen_ms: row.get(4)?,
            })
        })?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// `(device_id, token_hash)` of every paired device, for the bearer check,
    /// which compares each hash in constant time rather than looking one up.
    pub fn viewer_device_token_hashes(&self) -> Result<Vec<(String, String)>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT device_id, token_hash FROM db_viewer_devices")?;
        let rows = stmt.query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?;
        Ok(rows.collect::<Result<Vec<_>, _>>()?)
    }

    /// Stamp a device's last request. False when it was revoked meanwhile.
    pub fn viewer_device_touch(&self, device_id: &str, last_seen_ms: i64) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "UPDATE db_viewer_devices SET last_seen_ms = ?2 WHERE device_id = ?1",
            params![device_id, last_seen_ms],
        )?;
        Ok(rows > 0)
    }

    /// Revoke a device. False when there was no such device.
    pub fn viewer_device_delete(&self, device_id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute("DELETE FROM db_viewer_devices WHERE device_id = ?1", params![device_id])?;
        Ok(rows > 0)
    }

    /// Whether agent `id` (a definition id) is hidden from paired devices.
    /// False for an unknown agent.
    pub fn agent_hide_from_devices_get(&self, id: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        match conn.query_row(
            "SELECT hide_from_devices FROM db_agents WHERE id = ?1",
            params![id],
            |row| row.get::<_, i64>(0),
        ) {
            Ok(v) => Ok(v != 0),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(false),
            Err(e) => Err(e.into()),
        }
    }

    /// Hide agent `id` from paired devices, or show it again. False for an
    /// unknown agent. Leaves `updated_at` alone, like `agent_last_runtime_set`:
    /// who may watch the agent is not an edit of how it runs.
    pub fn agent_hide_from_devices_set(&self, id: &str, hidden: bool) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap();
        let rows = conn.execute(
            "UPDATE db_agents SET hide_from_devices = ?2 WHERE id = ?1",
            params![id, hidden as i64],
        )?;
        Ok(rows > 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn devices_are_listed_touched_and_revoked() {
        let store = Store::open_in_memory().unwrap();
        assert!(store.viewer_device_list().unwrap().is_empty());
        store.viewer_device_insert("d1", "aa", "Pixel", "", 100).unwrap();
        store.viewer_device_insert("d2", "bb", "Tablet", "a2V5", 200).unwrap();
        let list = store.viewer_device_list().unwrap();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].device_name, "Pixel");
        assert_eq!(list[1].device_key, "a2V5");
        assert_eq!(list[0].last_seen_ms, 0);

        assert!(store.viewer_device_touch("d1", 5_000).unwrap());
        assert_eq!(store.viewer_device_list().unwrap()[0].last_seen_ms, 5_000);
        assert_eq!(
            store.viewer_device_token_hashes().unwrap().len(),
            2,
            "the bearer check sees every device"
        );

        assert!(store.viewer_device_delete("d1").unwrap());
        assert!(!store.viewer_device_delete("d1").unwrap(), "already gone");
        assert!(!store.viewer_device_touch("d1", 6_000).unwrap());
        assert_eq!(store.viewer_device_list().unwrap().len(), 1);
    }

    #[test]
    fn a_token_hash_belongs_to_one_device() {
        let store = Store::open_in_memory().unwrap();
        store.viewer_device_insert("d1", "same", "A", "", 1).unwrap();
        assert!(store.viewer_device_insert("d2", "same", "B", "", 2).is_err());
    }

    #[test]
    fn the_hide_flag_defaults_off_and_is_kept_per_agent() {
        let store = Store::open_in_memory().unwrap();
        {
            let conn = store.conn.lock().unwrap();
            conn.execute_batch(
                "INSERT INTO db_agents (id, name, provider, created_at) VALUES ('a1', 'One', 'claude', 0);
                 INSERT INTO db_agents (id, name, provider, created_at) VALUES ('a2', 'Two', 'claude', 0);",
            )
            .unwrap();
        }
        assert!(!store.agent_hide_from_devices_get("a1").unwrap());
        assert!(store.agent_hide_from_devices_set("a1", true).unwrap());
        assert!(store.agent_hide_from_devices_get("a1").unwrap());
        assert!(!store.agent_hide_from_devices_get("a2").unwrap(), "another agent is unaffected");
        assert!(store.agent_hide_from_devices_set("a1", false).unwrap());
        assert!(!store.agent_hide_from_devices_get("a1").unwrap());
        assert!(!store.agent_hide_from_devices_set("nope", true).unwrap());
        assert!(!store.agent_hide_from_devices_get("nope").unwrap());
    }
}
