// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Allowlist for a responding agent's `conversation_visibility: trusted_peers`
//! mode. See `db_conversation_trust_grants` in migrations.rs
//! (OBJECT_SCHEMA_VERSION v26) and
//! docs/specs/SPEC_MUXSPECT_CROSS_TIER_CONVERSATION_VISIBILITY_2026_08_21.md
//! Phase B.
//!
//! Mirrors `lan_peer_pubkey_pins.rs`'s exact shape (own module, get/set-style
//! methods, case-insensitive agent_id lookups) — the explicit precedent this
//! table is modeled on. Distinct concern, though: pubkey pins record an
//! OBSERVED fact (trust-on-first-use) that must never silently change;
//! trust grants record an EXPLICIT, revocable decision an agent's owner
//! makes about who may read that agent's conversation content. A grant can
//! be added or removed at any time — there is no "first write wins"
//! invariant here.

use rusqlite::params;

use super::error::StoreError;
use super::store::Store;

fn now_secs() -> i64 {
    agentmux_common::time::now_secs()
}

impl Store {
    /// True if `agent_id` has explicitly granted `requester_agent_id` access
    /// to its conversation content on `tier` ("lan" or "wan"). A grant for
    /// one tier's cryptographic identity guarantee is never assumed to
    /// cover another — a caller must check the grant for the ACTUAL tier
    /// the incoming `transcript_request` arrived on.
    pub fn conversation_trust_grant_check(
        &self,
        agent_id: &str,
        requester_agent_id: &str,
        tier: &str,
    ) -> Result<bool, StoreError> {
        let agent_key = agent_id.to_lowercase();
        let requester_key = requester_agent_id.to_lowercase();
        let conn = self.conn.lock().unwrap();
        let count: i64 = conn.query_row(
            "SELECT COUNT(*) FROM db_conversation_trust_grants \
             WHERE agent_id = ?1 AND granted_peer_agent_id = ?2 AND tier = ?3",
            params![agent_key, requester_key, tier],
            |row| row.get(0),
        )?;
        Ok(count > 0)
    }

    /// Grant `requester_agent_id` access to `agent_id`'s conversation
    /// content on `tier`. Idempotent — granting an already-granted peer is
    /// a no-op, not an error or a duplicate row (the compound primary key
    /// on (agent_id, granted_peer_agent_id, tier) enforces this at the
    /// schema level too; `INSERT OR REPLACE` keeps this call site simple
    /// rather than requiring callers to distinguish "insert" from
    /// "already exists").
    pub fn conversation_trust_grant_add(
        &self,
        agent_id: &str,
        requester_agent_id: &str,
        tier: &str,
    ) -> Result<(), StoreError> {
        let agent_key = agent_id.to_lowercase();
        let requester_key = requester_agent_id.to_lowercase();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT OR REPLACE INTO db_conversation_trust_grants \
             (agent_id, granted_peer_agent_id, tier, granted_at) VALUES (?1, ?2, ?3, ?4)",
            params![agent_key, requester_key, tier, now_secs()],
        )?;
        Ok(())
    }

    /// Revoke a previously-granted peer. A no-op (not an error) if no such
    /// grant exists — revoking is idempotent, same as granting.
    pub fn conversation_trust_grant_revoke(
        &self,
        agent_id: &str,
        requester_agent_id: &str,
        tier: &str,
    ) -> Result<(), StoreError> {
        let agent_key = agent_id.to_lowercase();
        let requester_key = requester_agent_id.to_lowercase();
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "DELETE FROM db_conversation_trust_grants \
             WHERE agent_id = ?1 AND granted_peer_agent_id = ?2 AND tier = ?3",
            params![agent_key, requester_key, tier],
        )?;
        Ok(())
    }

    /// List every peer `agent_id` has granted access to, across all tiers —
    /// for a settings UI/CLI to show the current allowlist.
    pub fn conversation_trust_grant_list(
        &self,
        agent_id: &str,
    ) -> Result<Vec<(String, String, i64)>, StoreError> {
        let agent_key = agent_id.to_lowercase();
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT granted_peer_agent_id, tier, granted_at FROM db_conversation_trust_grants \
             WHERE agent_id = ?1 ORDER BY granted_at DESC",
        )?;
        let rows = stmt.query_map(params![agent_key], |row| {
            Ok((row.get(0)?, row.get(1)?, row.get(2)?))
        })?;
        let mut out = Vec::new();
        for r in rows {
            out.push(r?);
        }
        Ok(out)
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
    fn check_returns_false_when_ungranted() {
        let store = object_store();
        assert!(!store.conversation_trust_grant_check("korp", "loap", "lan").unwrap());
    }

    #[test]
    fn add_then_check_returns_true() {
        let store = object_store();
        store.conversation_trust_grant_add("korp", "loap", "lan").unwrap();
        assert!(store.conversation_trust_grant_check("korp", "loap", "lan").unwrap());
    }

    #[test]
    fn grant_is_scoped_to_its_own_tier() {
        let store = object_store();
        store.conversation_trust_grant_add("korp", "loap", "lan").unwrap();
        assert!(
            !store.conversation_trust_grant_check("korp", "loap", "wan").unwrap(),
            "a LAN grant must not be assumed to cover WAN — different cryptographic identity guarantee"
        );
    }

    #[test]
    fn grant_is_scoped_to_its_own_responding_agent() {
        let store = object_store();
        store.conversation_trust_grant_add("korp", "loap", "lan").unwrap();
        assert!(
            !store.conversation_trust_grant_check("agentx", "loap", "lan").unwrap(),
            "a grant made by one agent must not apply to a different agent's own visibility check"
        );
    }

    #[test]
    fn revoke_removes_the_grant() {
        let store = object_store();
        store.conversation_trust_grant_add("korp", "loap", "lan").unwrap();
        store.conversation_trust_grant_revoke("korp", "loap", "lan").unwrap();
        assert!(!store.conversation_trust_grant_check("korp", "loap", "lan").unwrap());
    }

    #[test]
    fn revoke_of_a_never_granted_peer_is_a_harmless_no_op() {
        let store = object_store();
        store.conversation_trust_grant_revoke("korp", "loap", "lan").unwrap();
        assert!(!store.conversation_trust_grant_check("korp", "loap", "lan").unwrap());
    }

    #[test]
    fn granting_twice_is_idempotent() {
        let store = object_store();
        store.conversation_trust_grant_add("korp", "loap", "lan").unwrap();
        store.conversation_trust_grant_add("korp", "loap", "lan").unwrap();
        let list = store.conversation_trust_grant_list("korp").unwrap();
        assert_eq!(list.len(), 1, "granting the same peer/tier twice must not duplicate the row");
    }

    #[test]
    fn agent_id_lookup_is_case_insensitive() {
        let store = object_store();
        store.conversation_trust_grant_add("Korp", "Loap", "lan").unwrap();
        assert!(store.conversation_trust_grant_check("korp", "loap", "lan").unwrap());
        assert!(store.conversation_trust_grant_check("KORP", "LOAP", "lan").unwrap());
    }

    #[test]
    fn list_returns_every_grant_for_the_agent_across_tiers() {
        let store = object_store();
        store.conversation_trust_grant_add("korp", "loap", "lan").unwrap();
        store.conversation_trust_grant_add("korp", "agentx", "wan").unwrap();
        let list = store.conversation_trust_grant_list("korp").unwrap();
        assert_eq!(list.len(), 2);
        let peers: std::collections::HashSet<_> = list.iter().map(|(p, _, _)| p.clone()).collect();
        assert_eq!(peers, ["loap", "agentx"].into_iter().map(String::from).collect());
    }

    #[test]
    fn list_never_returns_a_different_agents_grants() {
        let store = object_store();
        store.conversation_trust_grant_add("korp", "loap", "lan").unwrap();
        store.conversation_trust_grant_add("agentx", "loap", "lan").unwrap();
        let list = store.conversation_trust_grant_list("korp").unwrap();
        assert_eq!(list.len(), 1);
    }
}
