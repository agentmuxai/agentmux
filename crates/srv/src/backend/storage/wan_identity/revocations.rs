// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Retired agent WAN keys, both ways round.
//!
//! **Outgoing.** When this install replaces or removes an agent's key
//! ([`WanIdentityStore::agent_keys_delete`]), a directory that held the old
//! key keeps serving it until told otherwise. The delete writes one
//! *retirement* per directory that was confirmed to hold that fingerprint, in
//! the same transaction, and the publisher (`muxbus/wan_publish.rs`) signs a
//! [`WanKeyRevocation`] for each and puts it there. A retirement is deleted
//! only once its directory has the revocation.
//!
//! **Incoming.** A revocation of any install's key, once its signature
//! checks out, is kept for good. The verifier (`muxbus/wan_verify.rs`)
//! refuses a message signed by that fingerprint, and the cached record for it
//! is dropped. A revocation is signed by the instance and names a
//! fingerprint, so it holds under any account: it is not cleared when the
//! user signs out or switches account.
//!
//! The feed of an account's revocations is polled with a cursor per
//! directory, kept in `wan_meta`.

use agentmux_common::jekt_sign::wan_key_fingerprint;
use agentmux_common::wan_key_revocation::WanKeyRevocation;
use rusqlite::{params, Connection, OptionalExtension};

use super::{decode_32, StoreError, WanIdentityStore};

/// A key this install replaced that `directory` still has to be told about.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Retirement {
    pub agent_id: String,
    pub key_fp: String,
    /// When the key was replaced; the revocation is signed with this time, so
    /// every retry sends the same record.
    pub retired_at: i64,
}

impl WanIdentityStore {
    /// Inside `agent_keys_delete`'s transaction, before the row goes: one
    /// retirement for each directory confirmed to hold `agent_id`'s current
    /// key. A key no directory holds needs none: nobody could have verified it.
    pub(super) fn retire_agent_key_locked(conn: &Connection, agent_id: &str, now: i64) -> Result<(), StoreError> {
        let public_key: Option<String> = conn
            .query_row("SELECT public_key FROM wan_agent_keys WHERE agent_id = ?1", params![agent_id], |r| r.get(0))
            .optional()?;
        let Some(public) = public_key.as_deref().and_then(decode_32) else { return Ok(()) };
        conn.execute(
            "INSERT OR IGNORE INTO wan_key_retirements (agent_id, key_fp, directory, retired_at) \
             SELECT agent_id, key_fp, directory, ?3 FROM wan_agent_key_publications \
             WHERE agent_id = ?1 AND key_fp = ?2",
            params![agent_id, wan_key_fingerprint(&public), now],
        )?;
        Ok(())
    }

    /// `directory` accepted `public_key` for `agent_id`, but the key was
    /// replaced or removed while the publish was in flight, so no publication
    /// could be recorded and [`Self::retire_agent_key_locked`] found none. The
    /// directory holds a key nothing else knows it holds: write the retirement
    /// now. `Ok(false)` for a malformed key.
    pub fn retirement_add(&self, agent_id: &str, public_key: &str, directory: &str, now: i64) -> Result<bool, StoreError> {
        let Some(public) = decode_32(public_key) else { return Ok(false) };
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT OR IGNORE INTO wan_key_retirements (agent_id, key_fp, directory, retired_at) VALUES (?1, ?2, ?3, ?4)",
            params![agent_id.to_lowercase(), wan_key_fingerprint(&public), directory, now],
        )?;
        Ok(true)
    }

    /// The keys `directory` has not yet been told were retired.
    pub fn retirements_pending(&self, directory: &str) -> Result<Vec<Retirement>, StoreError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let mut stmt = conn.prepare(
            "SELECT agent_id, key_fp, retired_at FROM wan_key_retirements WHERE directory = ?1 \
             ORDER BY retired_at, agent_id, key_fp",
        )?;
        let rows = stmt.query_map(params![directory], |r| {
            Ok(Retirement { agent_id: r.get(0)?, key_fp: r.get(1)?, retired_at: r.get(2)? })
        })?;
        Ok(rows.collect::<Result<_, _>>()?)
    }

    /// `directory` has the revocation (or already had one): drop the retirement.
    pub fn retirement_done(&self, agent_id: &str, key_fp: &str, directory: &str) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "DELETE FROM wan_key_retirements WHERE agent_id = ?1 AND key_fp = ?2 AND directory = ?3",
            params![agent_id.to_lowercase(), key_fp, directory],
        )?;
        Ok(())
    }

    /// Keep a revocation whose signature checks out, and drop the cached
    /// record of the key it names. Sticky: nothing un-revokes a key, and a
    /// second revocation of the same key changes nothing. `Ok(false)` when
    /// the revocation does not verify (nothing stored).
    pub fn revoked_key_put(&self, revocation: &WanKeyRevocation, now: i64) -> Result<bool, StoreError> {
        if !revocation.verify() {
            return Ok(false);
        }
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        conn.execute(
            "INSERT OR IGNORE INTO wan_revoked_keys \
             (instance_id, agent_id, channel, key_fp, revoked_at, record, learned_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
            params![
                revocation.instance_id,
                revocation.agent_id,
                revocation.channel,
                revocation.key_fp,
                revocation.revoked_at,
                serde_json::to_string(revocation)?,
                now,
            ],
        )?;
        conn.execute(
            "DELETE FROM wan_peer_records WHERE instance_id = ?1 AND agent_id = ?2 AND channel = ?3 AND key_fp = ?4",
            params![revocation.instance_id, revocation.agent_id, revocation.channel, revocation.key_fp],
        )?;
        Ok(true)
    }

    /// Whether a verified revocation of exactly this key is kept. Case-insensitive
    /// in the instance and agent, as every W3-S identifier is.
    pub fn revoked_key_exists(&self, instance_id: &str, agent_id: &str, channel: &str, key_fp: &str) -> Result<bool, StoreError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        Ok(conn
            .query_row(
                "SELECT 1 FROM wan_revoked_keys \
                 WHERE instance_id = ?1 AND agent_id = ?2 AND channel = ?3 AND key_fp = ?4",
                params![instance_id.to_lowercase(), agent_id.to_lowercase(), channel, key_fp],
                |r| r.get::<_, i64>(0),
            )
            .optional()?
            .is_some())
    }

    /// The revocation kept for this key (the first one stored).
    #[cfg(test)]
    fn revoked_key_get(&self, instance_id: &str, agent_id: &str, channel: &str, key_fp: &str) -> Option<WanKeyRevocation> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let json: String = conn
            .query_row(
                "SELECT record FROM wan_revoked_keys \
                 WHERE instance_id = ?1 AND agent_id = ?2 AND channel = ?3 AND key_fp = ?4",
                params![instance_id.to_lowercase(), agent_id.to_lowercase(), channel, key_fp],
                |r| r.get(0),
            )
            .optional()
            .unwrap()?;
        serde_json::from_str(&json).ok()
    }

    /// `(since, checked_at)` of the feed for `directory`: the cursor to pass
    /// as `since`, and when the feed was last asked (0: never).
    pub fn key_revocation_feed_state(&self, directory: &str) -> Result<(i64, i64), StoreError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let get = |key: String| -> Result<i64, StoreError> {
            Ok(conn
                .query_row("SELECT value FROM wan_meta WHERE key = ?1", params![key], |r| r.get(0))
                .optional()?
                .unwrap_or(0))
        };
        Ok((get(format!("keyrev_since#{directory}"))?, get(format!("keyrev_checked#{directory}"))?))
    }

    /// Record that the feed for `directory` was asked at `checked_at`, and,
    /// when given, where to continue from.
    pub fn key_revocation_feed_note(&self, directory: &str, checked_at: i64, since: Option<i64>) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap_or_else(|e| e.into_inner());
        let put = |key: String, value: i64| {
            conn.execute(
                "INSERT INTO wan_meta (key, value) VALUES (?1, ?2) ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                params![key, value],
            )
        };
        put(format!("keyrev_checked#{directory}"), checked_at)?;
        if let Some(since) = since {
            put(format!("keyrev_since#{directory}"), since)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DIR: &str = "https://relay#acct";
    const OTHER_DIR: &str = "https://relay#other";

    fn temp_store() -> (tempfile::TempDir, WanIdentityStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = WanIdentityStore::open(&dir.path().join("wan-identity").join("wan.db")).unwrap();
        (dir, store)
    }

    fn revocation_for(store: &WanIdentityStore, agent: &str, fp: &str) -> WanKeyRevocation {
        let instance = store.instance_ensure("narko").unwrap();
        WanKeyRevocation::sign(&instance.private_key, agent, "stable", fp, 1_790_000_000).unwrap()
    }

    fn fingerprint(store: &WanIdentityStore, agent: &str) -> String {
        wan_key_fingerprint(&store.agent_key_load(agent).unwrap().unwrap().public_key_bytes().unwrap())
    }

    #[test]
    fn replacing_a_published_key_retires_it_for_each_directory_that_held_it() {
        let (_dir, store) = temp_store();
        let key = store.agent_key_ensure("camper", None).unwrap();
        let fp = fingerprint(&store, "camper");
        store.agent_key_mark_published("camper", &key.public_key, DIR).unwrap();
        store.agent_key_mark_published("camper", &key.public_key, OTHER_DIR).unwrap();

        store.agent_keys_delete(&["Camper".to_string()]).unwrap();

        for directory in [DIR, OTHER_DIR] {
            let pending = store.retirements_pending(directory).unwrap();
            assert_eq!(pending.len(), 1);
            assert_eq!((pending[0].agent_id.as_str(), pending[0].key_fp.as_str()), ("camper", fp.as_str()));
            assert!(pending[0].retired_at > 0);
        }
        assert!(store.retirements_pending("https://relay#nobody").unwrap().is_empty());
    }

    #[test]
    fn a_key_the_directory_took_after_it_was_replaced_is_retired_for_it() {
        let (_dir, store) = temp_store();
        let key = store.agent_key_ensure("camper", None).unwrap();
        let fp = fingerprint(&store, "camper");
        store.agent_keys_delete(&["camper".to_string()]).unwrap();
        assert!(store.retirements_pending(DIR).unwrap().is_empty(), "nothing was confirmed yet");

        assert!(store.retirement_add("Camper", &key.public_key, DIR, 7).unwrap());
        assert!(store.retirement_add("camper", &key.public_key, DIR, 8).unwrap(), "twice is once");
        let pending = store.retirements_pending(DIR).unwrap();
        assert_eq!(pending.len(), 1);
        assert_eq!((pending[0].key_fp.as_str(), pending[0].retired_at), (fp.as_str(), 7));
        assert!(!store.retirement_add("camper", "not a key", DIR, 9).unwrap());
    }

    #[test]
    fn a_key_no_directory_held_needs_no_revocation() {
        let (_dir, store) = temp_store();
        store.agent_key_ensure("camper", None).unwrap();
        store.agent_keys_delete(&["camper".to_string()]).unwrap();
        assert!(store.retirements_pending(DIR).unwrap().is_empty());
    }

    #[test]
    fn only_the_directory_that_held_the_old_key_is_told_not_one_holding_an_older_key() {
        let (_dir, store) = temp_store();
        let old = store.agent_key_ensure("camper", None).unwrap();
        store.agent_key_mark_published("camper", &old.public_key, DIR).unwrap();
        store.agent_keys_delete(&["camper".to_string()]).unwrap();
        // The new key reaches DIR (overwriting its publication row) and is
        // then replaced in turn, never having reached OTHER_DIR.
        let new = store.agent_key_ensure("camper", None).unwrap();
        store.agent_key_mark_published("camper", &new.public_key, DIR).unwrap();
        store.agent_keys_delete(&["camper".to_string()]).unwrap();

        let fps: Vec<String> = store.retirements_pending(DIR).unwrap().into_iter().map(|r| r.key_fp).collect();
        assert_eq!(fps.len(), 2, "both replaced keys are owed to the directory that held them");
        assert!(store.retirements_pending(OTHER_DIR).unwrap().is_empty());
    }

    #[test]
    fn a_retirement_is_kept_until_its_directory_has_it_and_the_same_key_is_not_retired_twice() {
        let (_dir, store) = temp_store();
        let key = store.agent_key_ensure("camper", None).unwrap();
        store.agent_key_mark_published("camper", &key.public_key, DIR).unwrap();
        store.agent_keys_delete(&["camper".to_string()]).unwrap();
        let pending = store.retirements_pending(DIR).unwrap();
        assert_eq!(pending.len(), 1);

        store.retirement_done("CAMPER", &pending[0].key_fp, OTHER_DIR).unwrap();
        assert_eq!(store.retirements_pending(DIR).unwrap().len(), 1, "another directory's answer does not clear it");
        store.retirement_done("CAMPER", &pending[0].key_fp, DIR).unwrap();
        assert!(store.retirements_pending(DIR).unwrap().is_empty());
    }

    #[test]
    fn a_revocation_is_kept_only_if_it_verifies_and_is_sticky() {
        let (_dir, store) = temp_store();
        let fp = "x".repeat(43);
        let good = revocation_for(&store, "camper", &fp);
        let mut forged = good.clone();
        forged.revoked_at += 1;
        assert!(!store.revoked_key_put(&forged, 5).unwrap());
        assert!(!store.revoked_key_exists(&good.instance_id, "camper", "stable", &fp).unwrap());

        assert!(store.revoked_key_put(&good, 5).unwrap());
        let instance = store.instance_ensure("narko").unwrap();
        let later = WanKeyRevocation::sign(&instance.private_key, "camper", "stable", &fp, good.revoked_at + 100).unwrap();
        assert!(store.revoked_key_put(&later, 9).unwrap());
        assert!(store.revoked_key_exists(&good.instance_id.to_uppercase(), "CAMPER", "stable", &fp).unwrap());
        assert_eq!(store.revoked_key_get(&good.instance_id, "camper", "stable", &fp), Some(good.clone()), "the first one stays");
        assert!(!store.revoked_key_exists(&good.instance_id, "camper", "beta", &fp).unwrap(), "another channel is another key");
        assert!(!store.revoked_key_exists(&good.instance_id, "lark", "stable", &fp).unwrap());
    }

    #[test]
    fn a_revocation_drops_the_cached_record_of_that_key_only() {
        let (_dir, store) = temp_store();
        let instance = store.instance_ensure("narko").unwrap();
        let (_other_dir, other) = temp_store();
        let key = other.agent_key_ensure("camper", None).unwrap();
        let record = crate::muxbus::wan_publish::certify(&instance, "camper", "stable", &key).unwrap();
        let other_key = other.agent_key_ensure("lark", None).unwrap();
        let kept = crate::muxbus::wan_publish::certify(&instance, "lark", "stable", &other_key).unwrap();
        store.peer_record_put(&record).unwrap();
        store.peer_record_put(&kept).unwrap();

        let revocation = WanKeyRevocation::sign(&instance.private_key, "camper", "stable", &record.key_fp, 1).unwrap();
        assert!(store.revoked_key_put(&revocation, 2).unwrap());

        assert!(store.peer_record_get(&record.instance_id, "camper", "stable", &record.key_fp).unwrap().is_none());
        assert!(store.peer_record_get(&kept.instance_id, "lark", "stable", &kept.key_fp).unwrap().is_some());
    }

    #[test]
    fn revoked_keys_survive_a_cache_clear() {
        let (_dir, store) = temp_store();
        let revocation = revocation_for(&store, "camper", &"y".repeat(43));
        store.revoked_key_put(&revocation, 1).unwrap();
        store.peer_cache_clear().unwrap();
        assert!(store.revoked_key_exists(&revocation.instance_id, "camper", "stable", &revocation.key_fp).unwrap());
    }

    #[test]
    fn the_feed_cursor_is_kept_per_directory() {
        let (_dir, store) = temp_store();
        assert_eq!(store.key_revocation_feed_state(DIR).unwrap(), (0, 0));
        store.key_revocation_feed_note(DIR, 100, Some(77)).unwrap();
        store.key_revocation_feed_note(OTHER_DIR, 200, None).unwrap();
        assert_eq!(store.key_revocation_feed_state(DIR).unwrap(), (77, 100));
        assert_eq!(store.key_revocation_feed_state(OTHER_DIR).unwrap(), (0, 200));
        store.key_revocation_feed_note(DIR, 300, None).unwrap();
        assert_eq!(store.key_revocation_feed_state(DIR).unwrap(), (77, 300), "no new cursor leaves the old one");
    }
}
