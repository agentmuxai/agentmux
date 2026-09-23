// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Per-agent local identity token, keyed by the agent's UID (`db_agents.id`).
//!
//! Identity M1 of `docs/specs/SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md`
//! (§6, §9.1). See `db_agent_tokens` in migrations.rs (OBJECT_SCHEMA_VERSION
//! v37).
//!
//! **What it is.** A secret the server generates for one agent and hands to
//! that agent's process environment at spawn (`AGENTMUX_AGENT_TOKEN`, set by
//! `build_persistent_spawn_env`). Only the server and the agent it spawned
//! ever hold the value, which is what will let a later phase (M4) map
//! token → UID on a request and treat *that* as the caller's identity,
//! instead of trusting an `agent_id` the request body asserts. Spec §2
//! rule 2: identity is proven, not asserted.
//!
//! **What it is not.** Not the cloud Cognito M2M credential in
//! `muxbus/agent_credentials.rs` — that one needs a prior human login and
//! falls back to a shared token by design, both of which disqualify it
//! (spec §0.3). This is minted locally, with no network and no fallback.
//!
//! **Lifetime.** Long-lived for the agent's lifetime and revoked when the
//! agent is deleted (`purge_agent_dependents`), never renewed on a timer:
//! spec §6.3 — expiry mid-task turns a healthy agent silent, and renewal is a
//! liveness dependency on the very path being secured. Persisted here, in
//! the server's own store, so an srv restart does not strand every running
//! agent with a token nothing can verify (same section). This is the
//! server persisting its own secret where only the server reads it — not a
//! name-keyed, agent-readable file in shared space, which §6.3 rules out.
//!
//! **Keyed by UID, not name.** `db_agent_jekt_keys` and the registry entry
//! files are keyed by display name, so two agents sharing a name share one
//! credential (spec §6.3). This table cannot have that defect: `db_agents.id`
//! is a `PRIMARY KEY`, and every non-template row is a UUID.
//!
//! **Nothing reads the token yet.** M1 mints and carries; M4 verifies. Until
//! then the value is inert, which is what makes this phase revertible by
//! ignoring the new field.

use rusqlite::params;

use super::error::StoreError;
use super::store::Store;

fn now_secs() -> i64 {
    agentmux_common::time::now_secs()
}

/// 32 bytes of CSPRNG randomness, hex-encoded (64 chars). Same source as
/// `agent_jekt_keys::random_key_bytes` — `uuid`'s v4 generation is already
/// CSPRNG-backed and already a dependency; hex rather than base64 so the
/// value is safe in any environment-variable or shell context without
/// quoting.
fn random_token() -> String {
    let mut bytes = [0u8; 32];
    bytes[..16].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    bytes[16..].copy_from_slice(uuid::Uuid::new_v4().as_bytes());
    hex::encode(bytes)
}

impl Store {
    /// This agent's token if one has been minted, without minting.
    ///
    /// The verification read (M4) is the production caller; until it lands
    /// only tests read this, hence the `dead_code` allowance outside test
    /// builds. Deletion does not go through here — `purge_agent_dependents`
    /// covers `db_agent_tokens` directly.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn agent_token_load(&self, uid: &str) -> Result<Option<String>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare("SELECT token FROM db_agent_tokens WHERE agent_id = ?1")?;
        match stmt.query_row(params![uid], |row| row.get::<_, String>(0)) {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// This agent's token, minting and persisting one on first use.
    ///
    /// Race-safe under concurrent first use, the same way
    /// `agent_jekt_key_ensure` is: the insert is `OR IGNORE` and the row is
    /// re-read regardless of whether this call's insert won, so two spawns
    /// of a never-before-seen agent agree on one token instead of each
    /// holding a different one. An empty `uid` is refused rather than given
    /// a token: a token keyed by `""` would be shared by every caller that
    /// failed to resolve an identity, which is the shared-credential defect
    /// this table exists to remove.
    pub fn agent_token_ensure(&self, uid: &str) -> Result<String, StoreError> {
        if uid.trim().is_empty() {
            return Err(StoreError::Other(
                "agent_token_ensure: refusing to mint a token for an empty uid".to_string(),
            ));
        }
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR IGNORE INTO db_agent_tokens (agent_id, token, created_at) VALUES (?1, ?2, ?3)",
            params![uid, random_token(), now_secs()],
        )?;
        let mut stmt = conn.prepare("SELECT token FROM db_agent_tokens WHERE agent_id = ?1")?;
        Ok(stmt.query_row(params![uid], |row| row.get(0))?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn object_store() -> Store {
        Store::open_in_memory().unwrap()
    }

    #[test]
    fn load_is_none_until_minted_then_ensure_is_stable() {
        let store = object_store();
        assert_eq!(store.agent_token_load("uid-a").unwrap(), None);

        let first = store.agent_token_ensure("uid-a").unwrap();
        assert_eq!(first.len(), 64, "32 random bytes, hex-encoded");
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));

        // Second ensure returns the persisted value, not a fresh mint.
        assert_eq!(store.agent_token_ensure("uid-a").unwrap(), first);
        assert_eq!(
            store.agent_token_load("uid-a").unwrap().as_deref(),
            Some(first.as_str())
        );
    }

    /// The property M4 will rely on: two agents never share a token, even
    /// when everything a human can see about them (their names) collides.
    /// The table is keyed by UID, so the names never enter into it.
    #[test]
    fn distinct_uids_get_distinct_tokens() {
        let store = object_store();
        let a = store.agent_token_ensure("4f3c-agenty").unwrap();
        let b = store.agent_token_ensure("9b2e-agenty").unwrap();
        assert_ne!(a, b);
    }

    #[test]
    fn empty_uid_is_refused_not_given_a_shared_token() {
        let store = object_store();
        assert!(store.agent_token_ensure("").is_err());
        assert!(store.agent_token_ensure("   ").is_err());
        assert_eq!(store.agent_token_load("").unwrap(), None);
    }
}
