// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Trust-on-first-use pin of a REMOTE agent_id's LAN public key.
//! See db_lan_peer_pubkey_pins in migrations.rs (OBJECT_SCHEMA_VERSION v20)
//! and docs/specs/SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md §2.2.
//!
//! Distinct from `agent_lan_keys.rs` (this instance's OWN agents' keypairs,
//! private half included) — this table holds only public keys OBSERVED from
//! LAN peers for agent_ids this instance does not itself host.
//!
//! Why this exists (reagentx P0 on the LAN signing PR): mDNS peer discovery
//! is unauthenticated — any device on the LAN can advertise its own
//! AgentMux instance and locally register an agent under an EXISTING
//! agent's name (e.g. "korp"). Without pinning, `find_agent_lan_pubkey`
//! trusts "whichever peer answers the discovery query first," which an
//! attacker can win just as easily as the legitimate owner — their
//! self-minted keypair would then verify successfully as "korp," defeating
//! the entire point of per-agent signing. Pinning the FIRST key ever
//! observed for an agent_id, and treating a later, DIFFERENT key as a
//! mismatch rather than silently accepting it, is the standard SSH-host-key
//! mitigation for exactly this "no PKI available" situation. It does not
//! eliminate spoofing entirely — an attacker who wins the race on the very
//! first-ever lookup for a given agent_id still gets pinned as
//! authoritative — but it closes the "always fully spoofable, forever"
//! case down to a narrow first-contact race, the same residual risk any
//! TOFU scheme accepts.

use rusqlite::params;

use super::error::StoreError;
use super::store::Store;

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

impl Store {
    /// Look up the pinned public key for a remote agent_id, if one exists.
    pub fn lan_peer_pubkey_pin_load(&self, agent_id: &str) -> Result<Option<String>, StoreError> {
        let key = agent_id.to_lowercase();
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT public_key FROM db_lan_peer_pubkey_pins WHERE agent_id = ?1")?;
        match stmt.query_row(params![key], |row| row.get(0)) {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Get-or-pin: if a pin already exists for this agent_id, returns it
    /// UNCHANGED regardless of what `observed_key_b64` says (the whole
    /// point — a later, different key must never silently overwrite an
    /// established pin). If no pin exists yet, pins `observed_key_b64` and
    /// returns it. Callers compare the return value against
    /// `observed_key_b64`: equal means "trusted, use it"; a mismatch means
    /// "this claimed sender's key just changed unexpectedly" — a red flag,
    /// not a value to fall back to. Race-safe under concurrent first-use
    /// via the same `INSERT OR IGNORE` + re-read pattern
    /// `agent_jekt_key_ensure`/`agent_lan_key_ensure` already establish.
    pub fn lan_peer_pubkey_pin_get_or_set(
        &self,
        agent_id: &str,
        observed_key_b64: &str,
    ) -> Result<String, StoreError> {
        let key = agent_id.to_lowercase();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO db_lan_peer_pubkey_pins (agent_id, public_key, first_seen_at) \
             VALUES (?1, ?2, ?3)",
            params![key, observed_key_b64, now_secs()],
        )?;
        let mut stmt = conn.prepare("SELECT public_key FROM db_lan_peer_pubkey_pins WHERE agent_id = ?1")?;
        stmt.query_row(params![key], |row| row.get(0)).map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object_store() -> Store {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        Store::open(tmp.path()).unwrap()
    }

    #[test]
    fn load_returns_none_when_unpinned() {
        let store = object_store();
        assert!(store.lan_peer_pubkey_pin_load("korp").unwrap().is_none());
    }

    #[test]
    fn first_observation_pins_the_key() {
        let store = object_store();
        let pinned = store.lan_peer_pubkey_pin_get_or_set("korp", "key-a").unwrap();
        assert_eq!(pinned, "key-a");
        assert_eq!(store.lan_peer_pubkey_pin_load("korp").unwrap().as_deref(), Some("key-a"));
    }

    #[test]
    fn a_later_different_key_does_not_overwrite_the_pin() {
        let store = object_store();
        store.lan_peer_pubkey_pin_get_or_set("korp", "key-a").unwrap();
        // A different peer (or an attacker) now claims a different key for
        // the same agent_id — the ORIGINAL pin must win, not the newer claim.
        let result = store.lan_peer_pubkey_pin_get_or_set("korp", "key-b-attacker").unwrap();
        assert_eq!(
            result, "key-a",
            "the first-pinned key must be returned regardless of what a later observation claims"
        );
        assert_eq!(store.lan_peer_pubkey_pin_load("korp").unwrap().as_deref(), Some("key-a"));
    }

    #[test]
    fn repeated_observation_of_the_same_key_is_idempotent() {
        let store = object_store();
        store.lan_peer_pubkey_pin_get_or_set("korp", "key-a").unwrap();
        let result = store.lan_peer_pubkey_pin_get_or_set("korp", "key-a").unwrap();
        assert_eq!(result, "key-a");
    }

    #[test]
    fn different_agent_ids_pin_independently() {
        let store = object_store();
        store.lan_peer_pubkey_pin_get_or_set("korp", "key-a").unwrap();
        store.lan_peer_pubkey_pin_get_or_set("loap", "key-b").unwrap();
        assert_eq!(store.lan_peer_pubkey_pin_load("korp").unwrap().as_deref(), Some("key-a"));
        assert_eq!(store.lan_peer_pubkey_pin_load("loap").unwrap().as_deref(), Some("key-b"));
    }

    #[test]
    fn agent_id_lookup_is_case_insensitive() {
        let store = object_store();
        store.lan_peer_pubkey_pin_get_or_set("Korp", "key-a").unwrap();
        assert_eq!(store.lan_peer_pubkey_pin_load("korp").unwrap().as_deref(), Some("key-a"));
        let result = store.lan_peer_pubkey_pin_get_or_set("KORP", "key-b").unwrap();
        assert_eq!(result, "key-a", "case-insensitive pin lookup must find the existing pin");
    }
}
