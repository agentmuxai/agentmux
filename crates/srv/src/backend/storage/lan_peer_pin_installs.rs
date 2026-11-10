// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which install vouched for a LAN name's pinned key, and the keys a name has moved off.
//! See `db_lan_pin_installs` and `db_lan_retired_keys` in migrations.rs (`OBJECT_SCHEMA_VERSION` v45).
//!
//! The binding is a row of its own, tied to the key it was made for: when the name pin moves to another
//! key, the old row names a key the pin no longer holds and is ignored until the new key is bound, so a
//! pin is never read as bound to an install that vouched for a different key.

use rusqlite::params;

use super::error::StoreError;
use super::store::Store;

/// The install a pin is bound to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinInstall {
    pub instance_id: String,
    pub host_hint: String,
}

/// A key a name has moved off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RetiredKey {
    pub retired_at: i64,
    /// An install id, or `operator` / an agent's id when a person or an agent accepted the move.
    pub retired_by: String,
}

impl Store {
    /// The install `agent_id`'s current pin is bound to, if it is. A binding made for another key does not count.
    pub fn lan_pin_installed(&self, agent_id: &str) -> Result<Option<PinInstall>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT i.instance_id, i.host_hint FROM db_lan_pin_installs i \
             JOIN db_lan_peer_pubkey_pins p ON p.agent_id = i.agent_id AND p.public_key = i.public_key \
             WHERE i.agent_id = ?1",
        )?;
        match stmt.query_row(params![agent_id.to_lowercase()], |r| Ok(PinInstall { instance_id: r.get(0)?, host_hint: r.get(1)? })) {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Bind `agent_id`'s pin to `install`, but only if the pin holds `key_b64` and is not bound already.
    /// `true` when it is bound to that install afterwards. A binding is never rewritten here: moving it is
    /// the pin's own move (`lan_peer_pubkey_pin_move`), which the caller binds again.
    pub fn lan_pin_bind(&self, agent_id: &str, key_b64: &str, install: &PinInstall, now: i64) -> Result<bool, StoreError> {
        let name = agent_id.to_lowercase();
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction()?;
        let pinned: Option<String> =
            tx.query_row("SELECT public_key FROM db_lan_peer_pubkey_pins WHERE agent_id = ?1", params![name], |r| r.get(0)).ok();
        if pinned.as_deref() != Some(key_b64) {
            return Ok(false);
        }
        tx.execute("DELETE FROM db_lan_pin_installs WHERE agent_id = ?1 AND public_key != ?2", params![name, key_b64])?;
        tx.execute(
            "INSERT OR IGNORE INTO db_lan_pin_installs (agent_id, public_key, instance_id, host_hint, bound_at) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![name, key_b64, install.instance_id, install.host_hint, now],
        )?;
        let bound: String =
            tx.query_row("SELECT instance_id FROM db_lan_pin_installs WHERE agent_id = ?1", params![name], |r| r.get(0))?;
        tx.commit()?;
        Ok(bound == install.instance_id)
    }

    /// Remember that `agent_id` moved off the key with fingerprint `key_fp`. The first record of a key stays.
    pub fn lan_retired_put(&self, agent_id: &str, key_fp: &str, retired_at: i64, retired_by: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO db_lan_retired_keys (agent_id, key_fp, retired_at, retired_by) VALUES (?1, ?2, ?3, ?4)",
            params![agent_id.to_lowercase(), key_fp, retired_at, retired_by],
        )?;
        Ok(())
    }

    /// When and by whom `agent_id` moved off the key with fingerprint `key_fp`, if it did.
    pub fn lan_retired_get(&self, agent_id: &str, key_fp: &str) -> Result<Option<RetiredKey>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT retired_at, retired_by FROM db_lan_retired_keys WHERE agent_id = ?1 AND key_fp = ?2")?;
        match stmt.query_row(params![agent_id.to_lowercase(), key_fp], |r| Ok(RetiredKey { retired_at: r.get(0)?, retired_by: r.get(1)? })) {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn store() -> Store {
        Store::open(std::path::Path::new(":memory:")).unwrap()
    }

    fn install(id: &str) -> PinInstall {
        PinInstall { instance_id: id.into(), host_hint: "narko".into() }
    }

    #[test]
    fn a_pin_is_bound_once_and_only_for_the_key_it_holds() {
        let s = store();
        s.lan_peer_pubkey_pin_get_or_set("korp", "key-a").unwrap();
        assert!(s.lan_pin_installed("korp").unwrap().is_none(), "unbound until an install is recorded");
        assert!(!s.lan_pin_bind("korp", "key-other", &install("i1"), 1).unwrap(), "not the pinned key");
        assert!(s.lan_pin_bind("Korp", "key-a", &install("i1"), 1).unwrap());
        assert_eq!(s.lan_pin_installed("korp").unwrap(), Some(install("i1")));
        assert!(!s.lan_pin_bind("korp", "key-a", &install("i2"), 2).unwrap(), "a binding is not rewritten");
        assert_eq!(s.lan_pin_installed("korp").unwrap(), Some(install("i1")));
    }

    #[test]
    fn a_binding_made_for_an_older_key_is_not_read_for_the_new_one() {
        let s = store();
        s.lan_peer_pubkey_pin_get_or_set("korp", "key-a").unwrap();
        s.lan_pin_bind("korp", "key-a", &install("i1"), 1).unwrap();
        s.lan_peer_pubkey_pin_replace("korp", "key-b").unwrap();
        assert!(s.lan_pin_installed("korp").unwrap().is_none());
        assert!(s.lan_pin_bind("korp", "key-b", &install("i2"), 3).unwrap());
        assert_eq!(s.lan_pin_installed("korp").unwrap(), Some(install("i2")));
    }

    #[test]
    fn forgetting_a_pin_forgets_its_binding() {
        let s = store();
        s.lan_peer_pubkey_pin_get_or_set("korp", "key-a").unwrap();
        s.lan_pin_bind("korp", "key-a", &install("i1"), 1).unwrap();
        s.lan_peer_pin_forget("korp").unwrap();
        s.lan_peer_pubkey_pin_get_or_set("korp", "key-a").unwrap();
        assert!(s.lan_pin_installed("korp").unwrap().is_none());
    }

    #[test]
    fn a_retired_key_is_remembered_with_when_and_by_whom() {
        let s = store();
        assert!(s.lan_retired_get("korp", "fp1").unwrap().is_none());
        s.lan_retired_put("Korp", "fp1", 100, "i1").unwrap();
        s.lan_retired_put("korp", "fp1", 200, "operator").unwrap();
        assert_eq!(s.lan_retired_get("KORP", "fp1").unwrap(), Some(RetiredKey { retired_at: 100, retired_by: "i1".into() }));
        assert!(s.lan_retired_get("korp", "fp2").unwrap().is_none());
    }
}
