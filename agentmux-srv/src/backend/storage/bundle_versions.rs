// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Append-only version history for Global Memory bundle content — mirrors
//! `agent_native_memory_versions.rs`'s shape exactly, adapted for a single
//! `bundle_id` key instead of that table's composite `(agent_id, filename)`
//! (a Bundle's identity is already a single id). See
//! `docs/specs/SPEC_AGENT_FACING_GLOBAL_MEMORY_API_2026_09_15.md` Phase 0 —
//! Global Memory (`db_bundles`) previously had no audit trail at all, only
//! `updated_at`; this exists specifically because Global Memory is about to
//! gain a write path an agent, not just a human at the Armory UI, can reach.
//!
//! Only the WRITE side (`bundle_version_insert`, called on every
//! `globalmemory.write`) is wired into an RPC/MCP surface today.
//! `bundle_version_list`/`bundle_version_get` (the read side — history,
//! diff, revert) are implemented and tested here but deliberately not yet
//! exposed anywhere — a `GlobalMemoryHistory`-equivalent MCP tool /
//! Armory UI is out of scope for this pass. Recording history with no way
//! to see it yet is still strictly better than not recording it at all.

use rusqlite::params;
use sha2::{Digest, Sha256};

use super::error::StoreError;
use super::store::Store;

/// One version of one bundle's Global Memory content, including its full
/// `name`/`instructions`. Deliberately no serde derives, same as
/// `NativeMemoryVersion` — a wire type belongs in `rpc_types`, not here.
#[derive(Debug, Clone, PartialEq)]
pub struct BundleVersion {
    pub id: String,
    pub bundle_id: String,
    pub name: String,
    pub instructions: String,
    pub content_hash: String,
    pub parent_version_id: Option<String>,
    pub source: String,
    pub source_detail: String,
    pub created_at: i64,
}

/// A version's metadata without its full `name`/`instructions` — the
/// list-view shape.
#[derive(Debug, Clone, PartialEq)]
pub struct BundleVersionSummary {
    pub id: String,
    pub content_hash: String,
    pub parent_version_id: Option<String>,
    pub source: String,
    pub source_detail: String,
    pub created_at: i64,
}

fn now_ms() -> i64 {
    agentmux_common::time::now_ms()
}

/// Hashes `name` + `instructions` together (not `instructions` alone) —
/// unlike native memory, where a version's whole identity IS its content, a
/// bundle's `name` is content too (renaming a Global Memory entry without
/// touching its body is a real, meaningful edit an operator or agent might
/// make, and should still produce a new version + a changed hash).
pub(crate) fn content_hash(name: &str, instructions: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(name.as_bytes());
    hasher.update(b"\0");
    hasher.update(instructions.as_bytes());
    hex::encode(hasher.finalize())
}

impl Store {
    /// Records a new version for `bundle_id`, chained onto whatever the
    /// current latest version for this bundle is (`None` for a brand-new
    /// bundle's first version). Every call records a version, even if
    /// content is byte-identical to the previous one — same "simplicity
    /// over cleverness, no dedup-by-hash" posture as
    /// `agent_native_memory_version_insert`. `BEGIN IMMEDIATE` wraps the
    /// SELECT-latest + INSERT for the identical reason that function's own
    /// doc comment gives: without it, two concurrent writers (two srv
    /// processes on the same SQLite file) could both read the same
    /// "latest" and insert sibling versions with the same
    /// `parent_version_id`, corrupting the version chain's linearity.
    pub fn bundle_version_insert(
        &self,
        bundle_id: &str,
        name: &str,
        instructions: &str,
        source: &str,
        source_detail: &str,
    ) -> Result<BundleVersion, StoreError> {
        let id = uuid::Uuid::new_v4().to_string();
        let hash = content_hash(name, instructions);
        let created_at = now_ms();

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let parent_version_id: Option<String> = {
            let mut stmt = tx.prepare(
                "SELECT id FROM db_bundle_versions
                 WHERE bundle_id = ?1
                 ORDER BY created_at DESC, rowid DESC
                 LIMIT 1",
            )?;
            match stmt.query_row(params![bundle_id], |row| row.get::<_, String>(0)) {
                Ok(id) => Some(id),
                Err(rusqlite::Error::QueryReturnedNoRows) => None,
                Err(e) => return Err(e.into()),
            }
        };
        tx.execute(
            "INSERT INTO db_bundle_versions
                 (id, bundle_id, name, instructions, content_hash, parent_version_id, source, source_detail, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                id,
                bundle_id,
                name,
                instructions,
                hash,
                parent_version_id,
                source,
                source_detail,
                created_at,
            ],
        )?;
        tx.commit()?;

        Ok(BundleVersion {
            id,
            bundle_id: bundle_id.to_string(),
            name: name.to_string(),
            instructions: instructions.to_string(),
            content_hash: hash,
            parent_version_id,
            source: source.to_string(),
            source_detail: source_detail.to_string(),
            created_at,
        })
    }

    /// History for one bundle, newest first — metadata only, no content
    /// (mirrors `agent_native_memory_version_list`).
    #[allow(dead_code)] // not yet called — see this module's own doc comment; the read side of a future GlobalMemoryHistory tool, deliberately out of scope for Phase 0/1 (SPEC_AGENT_FACING_GLOBAL_MEMORY_API_2026_09_15.md).
    pub fn bundle_version_list(&self, bundle_id: &str) -> Result<Vec<BundleVersionSummary>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, content_hash, parent_version_id, source, source_detail, created_at
             FROM db_bundle_versions
             WHERE bundle_id = ?1
             ORDER BY created_at DESC, rowid DESC",
        )?;
        let rows = stmt
            .query_map(params![bundle_id], |row| {
                Ok(BundleVersionSummary {
                    id: row.get(0)?,
                    content_hash: row.get(1)?,
                    parent_version_id: row.get(2)?,
                    source: row.get(3)?,
                    source_detail: row.get(4)?,
                    created_at: row.get(5)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// One version by id, full content included — the lookup a diff/revert
    /// feature would build on later (mirrors `agent_native_memory_version_get`).
    #[allow(dead_code)] // not yet called — see this module's own doc comment; kept for the diff/revert follow-up this spec's Phase 0 anticipates.
    pub fn bundle_version_get(&self, version_id: &str) -> Result<Option<BundleVersion>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, bundle_id, name, instructions, content_hash, parent_version_id, source, source_detail, created_at
             FROM db_bundle_versions
             WHERE id = ?1",
        )?;
        match stmt.query_row(params![version_id], |row| {
            Ok(BundleVersion {
                id: row.get(0)?,
                bundle_id: row.get(1)?,
                name: row.get(2)?,
                instructions: row.get(3)?,
                content_hash: row.get(4)?,
                parent_version_id: row.get(5)?,
                source: row.get(6)?,
                source_detail: row.get(7)?,
                created_at: row.get(8)?,
            })
        }) {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_store() -> Store {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        Store::open_shared(tmp.path()).unwrap()
    }

    #[test]
    fn insert_chains_parent_version_id() {
        let store = test_store();
        let v1 = store
            .bundle_version_insert("bundle-1", "Coding Standards", "use tabs", "agent_inferred", "{}")
            .unwrap();
        assert!(v1.parent_version_id.is_none());

        let v2 = store
            .bundle_version_insert("bundle-1", "Coding Standards", "use spaces", "agent_inferred", "{}")
            .unwrap();
        assert_eq!(v2.parent_version_id, Some(v1.id.clone()));
        assert_ne!(v1.content_hash, v2.content_hash);
    }

    #[test]
    fn list_is_newest_first_and_scoped_to_one_bundle() {
        let store = test_store();
        store
            .bundle_version_insert("bundle-a", "A", "content 1", "agent_inferred", "{}")
            .unwrap();
        let a2 = store
            .bundle_version_insert("bundle-a", "A", "content 2", "agent_inferred", "{}")
            .unwrap();
        store
            .bundle_version_insert("bundle-b", "B", "unrelated", "agent_inferred", "{}")
            .unwrap();

        let history = store.bundle_version_list("bundle-a").unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].id, a2.id, "newest first");
    }

    #[test]
    fn get_returns_full_content_by_id() {
        let store = test_store();
        let v = store
            .bundle_version_insert("bundle-1", "Name", "the body", "human", "{}")
            .unwrap();
        let fetched = store.bundle_version_get(&v.id).unwrap().expect("version exists");
        assert_eq!(fetched.instructions, "the body");
        assert_eq!(fetched.bundle_id, "bundle-1");
    }

    #[test]
    fn get_returns_none_for_unknown_id() {
        let store = test_store();
        assert!(store.bundle_version_get("does-not-exist").unwrap().is_none());
    }

    #[test]
    fn renaming_without_changing_instructions_still_changes_the_hash() {
        let store = test_store();
        let v1 = store
            .bundle_version_insert("bundle-1", "Old Name", "same body", "agent_inferred", "{}")
            .unwrap();
        let v2 = store
            .bundle_version_insert("bundle-1", "New Name", "same body", "agent_inferred", "{}")
            .unwrap();
        assert_ne!(v1.content_hash, v2.content_hash, "renaming is a real edit, not a no-op");
    }
}
