// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Per-agent HMAC-SHA256 signing key for host-tier jekt sender verification.
//! See db_agent_jekt_keys in migrations.rs (OBJECT_SCHEMA_VERSION v18) and
//! docs/specs/SPEC_JEKT_TRUST_LAYER_COMPLETION_2026_08_13.md §2.2.
//!
//! One row per locally-registered agent_id. The key is minted once (first
//! call to `agent_jekt_key_ensure` for that agent — normally at spawn, via
//! `agent_config`/`agent_open`) and never leaves this srv instance except by
//! being injected into that ONE agent's own MCP server process env
//! (`AGENTMUX_JEKT_KEY`) — never into any other agent's env, and never
//! returned over any RPC. This is what makes the resulting signature mean
//! something: only the agent it claims to be from ever held the key needed
//! to produce it.
//!
//! As of docs/specs/SPEC_JEKT_HOST_KEY_TTL_ROTATION_2026_09_14.md, a key
//! past `JEKT_KEY_TTL_SECS` old is rotated the next time `agent_jekt_key_ensure`
//! is called for it (i.e. at that agent's next spawn) — see that spec for
//! why this is lazy-at-next-mint rather than invalidating a live key.
//! `agent_jekt_key_load` (the verification-read path) is deliberately
//! unaffected: it has no concept of "too old," only "matches" or "doesn't."

use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use rusqlite::params;

use super::error::StoreError;
use super::store::Store;

fn now_secs() -> i64 {
    agentmux_common::time::now_secs()
}

/// A key older than this is rotated at its agent's next spawn (the next
/// `agent_jekt_key_ensure` call), not invalidated in place — see
/// docs/specs/SPEC_JEKT_HOST_KEY_TTL_ROTATION_2026_09_14.md §2.
const JEKT_KEY_TTL_SECS: i64 = 24 * 60 * 60;

/// 32 bytes of randomness via two v4 UUIDs — avoids adding a `rand`/`getrandom`
/// dependency; `uuid`'s v4 generation is already CSPRNG-backed and `uuid` is
/// already a dependency used throughout this codebase for ids.
fn random_key_bytes() -> [u8; 32] {
    let mut bytes = [0u8; 32];
    bytes[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes
}

impl Store {
    /// Load this agent's signing key if one has already been minted, without
    /// creating one. Returns raw key bytes (decoded from the stored base64).
    pub fn agent_jekt_key_load(&self, agent_id: &str) -> Result<Option<Vec<u8>>, StoreError> {
        let key = agent_id.to_lowercase();
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT hmac_key FROM db_agent_jekt_keys WHERE agent_id = ?1")?;
        let encoded: Option<String> = match stmt.query_row(params![key], |row| row.get(0)) {
            Ok(v) => Some(v),
            Err(rusqlite::Error::QueryReturnedNoRows) => None,
            Err(e) => return Err(e.into()),
        };
        match encoded {
            None => Ok(None),
            Some(encoded) => BASE64
                .decode(&encoded)
                .map(Some)
                .map_err(|e| StoreError::Other(format!("agent_jekt_key: stored key is not valid base64: {e}"))),
        }
    }

    /// Load this agent's signing key together with its mint/last-rotation
    /// time, if one has already been minted.
    fn agent_jekt_key_load_with_created_at(&self, agent_id: &str) -> Result<Option<(Vec<u8>, i64)>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt =
            conn.prepare("SELECT hmac_key, created_at FROM db_agent_jekt_keys WHERE agent_id = ?1")?;
        match stmt.query_row(params![agent_id], |row| {
            let encoded: String = row.get(0)?;
            let created_at: i64 = row.get(1)?;
            Ok((encoded, created_at))
        }) {
            Ok((encoded, created_at)) => {
                let bytes = BASE64.decode(&encoded).map_err(|e| {
                    StoreError::Other(format!("agent_jekt_key: stored key is not valid base64: {e}"))
                })?;
                Ok(Some((bytes, created_at)))
            }
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Return this agent's signing key, minting and persisting a fresh
    /// random one on first use. Race-safe under concurrent first-use: the
    /// insert is `OR IGNORE`, and the winning row (whichever call actually
    /// landed first) is always what gets read back and returned, so two
    /// concurrent callers for the same never-before-seen agent_id agree on
    /// one key instead of each minting and using a different one.
    ///
    /// A key older than `JEKT_KEY_TTL_SECS` is rotated here rather than
    /// returned as-is — see docs/specs/SPEC_JEKT_HOST_KEY_TTL_ROTATION_2026_09_14.md.
    pub fn agent_jekt_key_ensure(&self, agent_id: &str) -> Result<Vec<u8>, StoreError> {
        let key = agent_id.to_lowercase();
        if let Some((existing, created_at)) = self.agent_jekt_key_load_with_created_at(&key)? {
            if now_secs() - created_at < JEKT_KEY_TTL_SECS {
                return Ok(existing);
            }
            let conn = self.conn.lock().unwrap();
            let fresh = random_key_bytes();
            let encoded = BASE64.encode(fresh);
            // Guarded on the stale created_at we just read so two
            // concurrent rotators for the same overdue agent_id agree on
            // one winner instead of each minting a different key — same
            // "re-read regardless of whether our own write won" shape as
            // the first-mint path below.
            conn.execute(
                "UPDATE db_agent_jekt_keys SET hmac_key = ?1, created_at = ?2 \
                 WHERE agent_id = ?3 AND created_at = ?4",
                params![encoded, now_secs(), key, created_at],
            )?;
            let mut stmt = conn.prepare("SELECT hmac_key FROM db_agent_jekt_keys WHERE agent_id = ?1")?;
            let winning: String = stmt.query_row(params![key], |row| row.get(0))?;
            return BASE64
                .decode(&winning)
                .map_err(|e| StoreError::Other(format!("agent_jekt_key: stored key is not valid base64: {e}")));
        }
        let conn = self.conn.lock().unwrap();
        let fresh = random_key_bytes();
        let encoded = BASE64.encode(fresh);
        conn.execute(
            "INSERT OR IGNORE INTO db_agent_jekt_keys (agent_id, hmac_key, created_at) VALUES (?1, ?2, ?3)",
            params![key, encoded, now_secs()],
        )?;
        // Re-read regardless of whether our insert won the race — this is
        // the single source of truth for "what key does this agent have,"
        // not an assumption that our own insert succeeded.
        let mut stmt = conn.prepare("SELECT hmac_key FROM db_agent_jekt_keys WHERE agent_id = ?1")?;
        let winning: String = stmt.query_row(params![key], |row| row.get(0))?;
        BASE64
            .decode(&winning)
            .map_err(|e| StoreError::Other(format!("agent_jekt_key: stored key is not valid base64: {e}")))
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
        assert!(store.agent_jekt_key_load("agentx").unwrap().is_none());
    }

    #[test]
    fn ensure_mints_a_32_byte_key() {
        let store = object_store();
        let key = store.agent_jekt_key_ensure("agentx").unwrap();
        assert_eq!(key.len(), 32);
    }

    #[test]
    fn ensure_is_idempotent_same_agent_gets_same_key() {
        let store = object_store();
        let first = store.agent_jekt_key_ensure("AgentX").unwrap();
        let second = store.agent_jekt_key_ensure("agentx").unwrap(); // case-insensitive
        assert_eq!(first, second);
    }

    #[test]
    fn different_agents_get_different_keys() {
        let store = object_store();
        let a = store.agent_jekt_key_ensure("agentx").unwrap();
        let b = store.agent_jekt_key_ensure("agenty").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn ensure_then_load_round_trips() {
        let store = object_store();
        let minted = store.agent_jekt_key_ensure("agentx").unwrap();
        let loaded = store.agent_jekt_key_load("agentx").unwrap().unwrap();
        assert_eq!(minted, loaded);
    }

    /// Backdates the stored `created_at` for `agent_id` directly via SQL —
    /// simulates "this key was minted a while ago" without sleeping in a
    /// test.
    fn backdate(store: &Store, agent_id: &str, age_secs: i64) {
        let conn = store.conn.lock().unwrap();
        conn.execute(
            "UPDATE db_agent_jekt_keys SET created_at = ?1 WHERE agent_id = ?2",
            params![now_secs() - age_secs, agent_id],
        )
        .unwrap();
    }

    #[test]
    fn ensure_does_not_rotate_before_ttl_expiry() {
        let store = object_store();
        let first = store.agent_jekt_key_ensure("agentx").unwrap();
        backdate(&store, "agentx", JEKT_KEY_TTL_SECS - 60);
        let second = store.agent_jekt_key_ensure("agentx").unwrap();
        assert_eq!(first, second, "a key under the TTL must be returned unchanged");
    }

    #[test]
    fn ensure_rotates_key_after_ttl_expiry() {
        let store = object_store();
        let first = store.agent_jekt_key_ensure("agentx").unwrap();
        backdate(&store, "agentx", JEKT_KEY_TTL_SECS + 60);
        let second = store.agent_jekt_key_ensure("agentx").unwrap();
        assert_ne!(first, second, "a key past the TTL must be rotated to a fresh one");
        assert_eq!(second.len(), 32);
    }

    #[test]
    fn rotation_is_reflected_by_load_not_just_ensure() {
        let store = object_store();
        let first = store.agent_jekt_key_ensure("agentx").unwrap();
        backdate(&store, "agentx", JEKT_KEY_TTL_SECS + 60);
        let rotated = store.agent_jekt_key_ensure("agentx").unwrap();
        let loaded = store.agent_jekt_key_load("agentx").unwrap().unwrap();
        assert_eq!(loaded, rotated);
        assert_ne!(loaded, first, "load must see the rotated key, not the original");
    }

    #[test]
    fn other_agents_keys_are_unaffected_by_one_agents_rotation() {
        let store = object_store();
        let a_before = store.agent_jekt_key_ensure("agentx").unwrap();
        let b = store.agent_jekt_key_ensure("agenty").unwrap();
        backdate(&store, "agentx", JEKT_KEY_TTL_SECS + 60);
        let a_after = store.agent_jekt_key_ensure("agentx").unwrap();
        assert_ne!(a_before, a_after);
        assert_eq!(b, store.agent_jekt_key_load("agenty").unwrap().unwrap());
    }
}
