// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Per-agent Ed25519 keypair for LAN-tier jekt sender verification.
//! See db_agent_lan_keys in migrations.rs (OBJECT_SCHEMA_VERSION v19) and
//! docs/specs/SPEC_JEKT_LAN_TIER_SIGNING_2026_08_15.md §2.1.
//!
//! Mirrors agent_jekt_keys.rs exactly, asymmetric instead of symmetric: one
//! row per locally-registered agent_id, minted once (first call to
//! `agent_lan_key_ensure`) and never leaves this srv instance except by the
//! PRIVATE half being injected into that ONE agent's own MCP server process
//! env (`AGENTMUX_LAN_KEY`) — never into any other agent's env, and never
//! returned over any RPC. The public half is not secret — it's what gets
//! handed to LAN peers on request (see `LanDiscovery::find_agent_lan_pubkey`)
//! so they can verify this agent's outgoing signatures.

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use rusqlite::params;

use super::error::StoreError;
use super::store::Store;

fn now_secs() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// 32 bytes of randomness via two v4 UUIDs — same rationale as
/// `agent_jekt_keys::random_key_bytes`: avoids adding a `rand`/`getrandom`
/// dependency, `uuid`'s v4 generation is already CSPRNG-backed, and
/// `ed25519_dalek::SigningKey::from_bytes` accepts any 32 bytes of
/// randomness as a valid seed (deterministic derivation from the seed, not
/// a call into an RNG itself).
fn random_seed_bytes() -> [u8; 32] {
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

impl Store {
    /// Load this agent's LAN keypair if one has already been minted,
    /// without creating one.
    pub fn agent_lan_key_load(&self, agent_id: &str) -> Result<Option<AgentLanKeypair>, StoreError> {
        let key = agent_id.to_lowercase();
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT public_key, private_key FROM db_agent_lan_keys WHERE agent_id = ?1")?;
        match stmt.query_row(params![key], |row| {
            Ok(AgentLanKeypair {
                public_key: row.get(0)?,
                private_key: row.get(1)?,
            })
        }) {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Load only this agent's PUBLIC key — the half safe to hand to a LAN
    /// peer's pubkey-lookup query (see `LanDiscovery::find_agent_lan_pubkey`
    /// and the `GET /agentmux/reactive/agent` handler it calls). Never loads
    /// or touches the private half.
    pub fn agent_lan_public_key_load(&self, agent_id: &str) -> Result<Option<String>, StoreError> {
        let key = agent_id.to_lowercase();
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT public_key FROM db_agent_lan_keys WHERE agent_id = ?1")?;
        match stmt.query_row(params![key], |row| row.get(0)) {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Return this agent's LAN keypair, minting and persisting a fresh
    /// random one on first use. Race-safe under concurrent first-use — same
    /// `INSERT OR IGNORE` + re-read pattern as `agent_jekt_key_ensure`: two
    /// concurrent callers for the same never-before-seen agent_id agree on
    /// one keypair instead of each minting and using a different one.
    pub fn agent_lan_key_ensure(&self, agent_id: &str) -> Result<AgentLanKeypair, StoreError> {
        let key = agent_id.to_lowercase();
        if let Some(existing) = self.agent_lan_key_load(&key)? {
            return Ok(existing);
        }
        let conn = self.conn.lock().unwrap();
        let seed = random_seed_bytes();
        let (public_bytes, private_bytes) = agentmux_common::jekt_sign::generate_lan_keypair(seed);
        let public_key = BASE64.encode(public_bytes);
        let private_key = BASE64.encode(private_bytes);
        conn.execute(
            "INSERT OR IGNORE INTO db_agent_lan_keys (agent_id, public_key, private_key, created_at) \
             VALUES (?1, ?2, ?3, ?4)",
            params![key, public_key, private_key, now_secs()],
        )?;
        // Re-read regardless of whether our insert won the race — single
        // source of truth for "what keypair does this agent have," not an
        // assumption that our own insert succeeded.
        let mut stmt =
            conn.prepare("SELECT public_key, private_key FROM db_agent_lan_keys WHERE agent_id = ?1")?;
        stmt.query_row(params![key], |row| {
            Ok(AgentLanKeypair {
                public_key: row.get(0)?,
                private_key: row.get(1)?,
            })
        })
        .map_err(Into::into)
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
