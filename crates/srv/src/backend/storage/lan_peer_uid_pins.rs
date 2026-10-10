// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Trust-on-first-use pin of a REMOTE agent's UID key, per (name, UID)
//! (identity M4d-6, `SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md`
//! §6.5.10). The LAN v2 counterpart of `lan_peer_pubkey_pins.rs`, and set only
//! after that name pin matched: UIDs are public, so a pin keyed by UID alone
//! could be squatted by a peer answering for its own name.

use rusqlite::params;

use super::error::StoreError;
use super::store::Store;

use agentmux_common::time::now_secs;

impl Store {
    /// Get-or-pin, as `lan_peer_pubkey_pin_get_or_set`: an existing pin for
    /// `(agent_id, uid)` is returned unchanged, otherwise `observed_key_b64`
    /// is pinned and returned. A return that differs from `observed_key_b64`
    /// means the UID's key changed under that name.
    pub fn lan_peer_uid_pin_get_or_set(
        &self,
        agent_id: &str,
        uid: &str,
        observed_key_b64: &str,
    ) -> Result<String, StoreError> {
        let name = agent_id.to_lowercase();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO db_lan_peer_uid_pins (agent_id, uid, public_key, first_seen_at) \
             VALUES (?1, ?2, ?3, ?4)",
            params![name, uid, observed_key_b64, now_secs()],
        )?;
        let mut stmt = conn.prepare("SELECT public_key FROM db_lan_peer_uid_pins WHERE agent_id = ?1 AND uid = ?2")?;
        stmt.query_row(params![name, uid], |row| row.get(0)).map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object_store() -> Store {
        Store::open(std::path::Path::new(":memory:")).unwrap()
    }

    #[test]
    fn the_first_key_for_a_name_and_uid_is_pinned_and_a_later_one_is_not() {
        let store = object_store();
        assert_eq!(store.lan_peer_uid_pin_get_or_set("Korp", "uid-1", "key-a").unwrap(), "key-a");
        assert_eq!(store.lan_peer_uid_pin_get_or_set("korp", "uid-1", "key-b").unwrap(), "key-a");
    }

    #[test]
    fn the_same_uid_under_another_name_pins_separately() {
        let store = object_store();
        store.lan_peer_uid_pin_get_or_set("korp", "uid-1", "key-a").unwrap();
        assert_eq!(store.lan_peer_uid_pin_get_or_set("lark", "uid-1", "key-b").unwrap(), "key-b");
        assert_eq!(store.lan_peer_uid_pin_get_or_set("korp", "uid-2", "key-c").unwrap(), "key-c");
    }
}
