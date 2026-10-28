// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Per-agent Ed25519 keypair for LAN-tier jekt sender verification.
//! See db_agent_lan_keys in migrations.rs (OBJECT_SCHEMA_VERSION v19) and
//! docs/specs/SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md §2.1.
//!
//! Mirrors agent_jekt_keys.rs exactly, asymmetric instead of symmetric: one
//! row per locally-registered agent_id, minted once (first call to
//! `agent_lan_key_ensure`); the PRIVATE half leaves this srv instance only
//! for that ONE agent (its MCP fetches it with its own identity token from
//! `GET /agentmux/agents/self/keys`, with a copy in its MCP config as
//! `AGENTMUX_LAN_KEY`) — never into any other agent's env, and never served
//! to any other caller. The public half is not secret — it's what gets
//! handed to LAN peers on request (see `LanDiscovery::find_agent_lan_pubkey`)
//! so they can verify this agent's outgoing signatures.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use rusqlite::params;

use super::error::StoreError;
use super::host_lan_keys::{short, HostKey, HostKeyKind, HostLanKeys};
use super::store::Store;

use agentmux_common::time::now_secs;

/// 32 bytes of randomness via two v4 UUIDs, for every key table in this
/// module (LAN, WAN, jekt, WAN identity) and the agent tokens. Avoids adding a `rand`/`getrandom`
/// dependency: `uuid`'s v4 generation is already CSPRNG-backed, and
/// `ed25519_dalek::SigningKey::from_bytes` accepts any 32 bytes of
/// randomness as a valid seed (deterministic derivation from the seed, not
/// a call into an RNG itself).
pub(super) fn random_seed_bytes() -> [u8; 32] {
    let mut bytes = [0u8; 32];
    bytes[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes
}

/// An agent's LAN signing keypair, both halves base64-encoded for
/// storage/transport.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentLanKeypair {
    pub public_key: String,
    pub private_key: String,
}

/// This channel's row for `key` (folded), and when it was minted.
fn read_local(conn: &rusqlite::Connection, key: &str) -> Result<Option<(AgentLanKeypair, i64)>, StoreError> {
    let mut stmt =
        conn.prepare("SELECT public_key, private_key, created_at FROM db_agent_lan_keys WHERE agent_id = ?1")?;
    match stmt.query_row(params![key], |row| {
        Ok((AgentLanKeypair { public_key: row.get(0)?, private_key: row.get(1)? }, row.get(2)?))
    }) {
        Ok(v) => Ok(Some(v)),
        Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

fn write_local(conn: &rusqlite::Connection, key: &str, pair: &AgentLanKeypair, created_at: i64) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO db_agent_lan_keys (agent_id, public_key, private_key, created_at) VALUES (?1, ?2, ?3, ?4) ON CONFLICT (agent_id) DO UPDATE SET public_key = ?2, private_key = ?3, created_at = ?4",
        params![key, pair.public_key, pair.private_key, created_at],
    )?;
    Ok(())
}

/// This channel's key for `key`, made to agree with the host's (`host_lan_keys.rs`): the host's key
/// replaces the channel's own, and a channel key the host has no key for becomes the host's when
/// `publish`. Without a host table, or when it cannot be reached, the channel's own key.
pub(super) fn aligned_name_key(
    conn: &rusqlite::Connection,
    host: Option<&HostLanKeys>,
    key: &str,
    publish: bool,
) -> Result<Option<AgentLanKeypair>, StoreError> {
    let local = read_local(conn, key)?;
    let Some(host) = host else { return Ok(local.map(|(pair, _)| pair)) };
    let candidate = local.as_ref().filter(|_| publish).map(|(pair, at)| HostKey { pair: pair.clone(), created_at: *at });
    match host.claim(HostKeyKind::Name, key, candidate.as_ref()) {
        Ok(Some(h)) => {
            if local.as_ref().map(|(pair, _)| pair) != Some(&h.pair) {
                write_local(conn, key, &h.pair, h.created_at)?;
                tracing::info!(agent = key, key = short(&h.pair.public_key), "LAN key: adopted the host's key for the agent");
            }
            Ok(Some(h.pair))
        }
        Ok(None) => Ok(local.map(|(pair, _)| pair)),
        Err(e) => {
            tracing::warn!(agent = key, error = %e, "LAN key: the host's key table is unreachable, using this channel's own");
            Ok(local.map(|(pair, _)| pair))
        }
    }
}

impl Store {
    /// Load this agent's LAN keypair if one has already been minted,
    /// without creating one. The host's key when it has one (see `host_lan_keys.rs`).
    pub fn agent_lan_key_load(&self, agent_id: &str) -> Result<Option<AgentLanKeypair>, StoreError> {
        let key = agent_id.to_lowercase();
        let host = self.host_lan_keys();
        let conn = self.conn.lock().unwrap();
        aligned_name_key(&conn, host.as_deref(), &key, true)
    }

    /// Load only this agent's PUBLIC key — the half safe to hand to a LAN
    /// peer's pubkey-lookup query (see `LanDiscovery::find_agent_lan_pubkey`
    /// and the `GET /agentmux/reactive/agent` handler it calls). Never loads
    /// or touches the private half.
    pub fn agent_lan_public_key_load(&self, agent_id: &str) -> Result<Option<String>, StoreError> {
        Ok(self.agent_lan_key_load(agent_id)?.map(|pair| pair.public_key))
    }

    /// Replace this agent's LAN keypair with a fresh one, if it has one (none is minted for an agent
    /// that never had one). The old private key signs nothing that verifies from then on. `true` when a
    /// keypair was replaced. The host's key is replaced too, so every channel signs with the new one.
    pub fn agent_lan_key_rotate(&self, agent_id: &str) -> Result<bool, StoreError> {
        let (public_bytes, private_bytes) = agentmux_common::jekt_sign::generate_lan_keypair(random_seed_bytes());
        let key = agent_id.to_lowercase();
        let fresh = AgentLanKeypair { public_key: BASE64.encode(public_bytes), private_key: BASE64.encode(private_bytes) };
        let now = now_secs();
        let conn = self.conn.lock().unwrap();
        if read_local(&conn, &key)?.is_none() {
            return Ok(false);
        }
        // The host's key first, and a failure fails the rotation: with the old key left there, the next
        // load on any channel would adopt it over the new one and undo the rotation.
        if let Some(host) = self.host_lan_keys() {
            host.replace(HostKeyKind::Name, &key, &HostKey { pair: fresh.clone(), created_at: now })?;
        }
        let changed = conn.execute(
            "UPDATE db_agent_lan_keys SET public_key = ?1, private_key = ?2, created_at = ?3 WHERE agent_id = ?4",
            params![fresh.public_key, fresh.private_key, now, key],
        )?;
        Ok(changed > 0)
    }

    /// Return this agent's LAN keypair: the host's if it has one, else this channel's own (which then
    /// becomes the host's), else a fresh random one, minted and published on first use. Race-safe under
    /// concurrent first-use: the `INSERT OR IGNORE` on the channel row and on the host row each leave
    /// every caller with the one keypair that won.
    pub fn agent_lan_key_ensure(&self, agent_id: &str) -> Result<AgentLanKeypair, StoreError> {
        let key = agent_id.to_lowercase();
        let host = self.host_lan_keys();
        let conn = self.conn.lock().unwrap();
        if let Some(existing) = aligned_name_key(&conn, host.as_deref(), &key, true)? {
            return Ok(existing);
        }
        let seed = random_seed_bytes();
        let (public_bytes, private_bytes) = agentmux_common::jekt_sign::generate_lan_keypair(seed);
        conn.execute(
            "INSERT OR IGNORE INTO db_agent_lan_keys (agent_id, public_key, private_key, created_at) VALUES (?1, ?2, ?3, ?4)",
            params![key, BASE64.encode(public_bytes), BASE64.encode(private_bytes), now_secs()],
        )?;
        // Re-read regardless of whether our insert won the race — single
        // source of truth for "what keypair does this agent have," not an
        // assumption that our own insert succeeded. Publishing it to the host here is what makes a
        // second channel, minting at the same moment, end up with this one.
        aligned_name_key(&conn, host.as_deref(), &key, true)?.ok_or(StoreError::NotFound)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object_store() -> Store {
        Store::open(std::path::Path::new(":memory:")).unwrap()
    }

    #[test]
    fn rotate_replaces_an_existing_keypair_and_mints_none() {
        let store = object_store();
        assert!(!store.agent_lan_key_rotate("agentx").unwrap(), "no keypair, nothing replaced");
        assert!(store.agent_lan_key_load("agentx").unwrap().is_none(), "and none minted");
        let old = store.agent_lan_key_ensure("agentx").unwrap();
        assert!(store.agent_lan_key_rotate("AgentX").unwrap());
        let new = store.agent_lan_key_load("agentx").unwrap().unwrap();
        assert_ne!(old.private_key, new.private_key);
        assert_ne!(old.public_key, new.public_key);
    }

    #[test]
    fn load_returns_none_when_unminted() {
        let store = object_store();
        assert!(store.agent_lan_key_load("agentx").unwrap().is_none());
        assert!(store.agent_lan_public_key_load("agentx").unwrap().is_none());
    }

    #[test]
    fn ensure_mints_a_valid_base64_keypair() {
        let store = object_store();
        let keypair = store.agent_lan_key_ensure("agentx").unwrap();
        let pub_bytes = BASE64.decode(&keypair.public_key).unwrap();
        let priv_bytes = BASE64.decode(&keypair.private_key).unwrap();
        assert_eq!(pub_bytes.len(), 32);
        assert_eq!(priv_bytes.len(), 32);
        assert_ne!(keypair.public_key, keypair.private_key);
    }

    #[test]
    fn ensure_is_idempotent_same_agent_gets_same_keypair() {
        let store = object_store();
        let first = store.agent_lan_key_ensure("AgentX").unwrap();
        let second = store.agent_lan_key_ensure("agentx").unwrap(); // case-insensitive
        assert_eq!(first, second);
    }

    #[test]
    fn different_agents_get_different_keypairs() {
        let store = object_store();
        let a = store.agent_lan_key_ensure("agentx").unwrap();
        let b = store.agent_lan_key_ensure("agenty").unwrap();
        assert_ne!(a.public_key, b.public_key);
        assert_ne!(a.private_key, b.private_key);
    }

    #[test]
    fn ensure_then_load_round_trips() {
        let store = object_store();
        let minted = store.agent_lan_key_ensure("agentx").unwrap();
        let loaded = store.agent_lan_key_load("agentx").unwrap().unwrap();
        assert_eq!(minted, loaded);
    }

    #[test]
    fn public_key_load_matches_the_public_half_of_the_full_keypair() {
        let store = object_store();
        let minted = store.agent_lan_key_ensure("agentx").unwrap();
        let public_only = store.agent_lan_public_key_load("agentx").unwrap().unwrap();
        assert_eq!(minted.public_key, public_only);
    }

    #[test]
    fn a_minted_keypair_can_sign_and_verify_a_real_jekt() {
        // End-to-end sanity through the actual jekt_sign functions, not just
        // storage round-tripping — proves the stored encoding is exactly
        // what generate_lan_keypair/sign_lan_jekt/verify_lan_jekt expect.
        let store = object_store();
        let keypair = store.agent_lan_key_ensure("agentx").unwrap();
        let private = BASE64.decode(&keypair.private_key).unwrap();
        let public = BASE64.decode(&keypair.public_key).unwrap();
        let sig = agentmux_common::jekt_sign::sign_lan_jekt(
            &private, "msg-1", "agentx", "agenty", 1_000, "hello",
        )
        .unwrap();
        assert!(agentmux_common::jekt_sign::verify_lan_jekt(
            &public, "msg-1", "agentx", "agenty", 1_000, "hello", &sig,
        ));
    }
}
