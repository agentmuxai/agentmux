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
//! The WRITE side (`bundle_version_insert`, called via
//! `Store::bundle_upsert_with_version` in `bundles.rs` on every
//! `globalmemory.write` AND the Armory UI's own `upsertmemory` save path)
//! and the READ side (`bundle_version_list`/`bundle_version_get`, called by
//! `global_memory_history_impl`/`global_memory_diff_impl`/
//! `global_memory_revert_impl` in `app_api/mod.rs`, backing the
//! `GlobalMemoryHistory`/`GlobalMemoryDiff`/`GlobalMemoryRevert` MCP tools)
//! are both wired into the agent-facing MCP surface now — see
//! `docs/specs/SPEC_AGENT_FACING_GLOBAL_MEMORY_API_2026_09_15.md`. No
//! Armory UI surface for this yet — that remains out of scope (MCP/REST
//! only, per that spec's own non-goals).

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
    /// The TRUSTED writer identity — an agent's own `AGENTMUX_AGENT_ID`
    /// (unforgeable, stamped server-side by the caller of
    /// `bundle_upsert_with_version`, never read from caller-supplied
    /// `source`/`source_detail`) or `"armory-ui"` for a write through the
    /// human-facing Armory editor. Distinct from `source` on purpose
    /// (codex P2, PR #3237): `source` is free-form annotation the caller
    /// can say anything in (an agent could label its own write
    /// `source: "human"`), so it alone cannot answer "who actually made
    /// this change" — this field is what a reviewer should trust for that.
    pub written_by: String,
    /// Identity M4c-1 (SPEC_AGENT_IDENTITY_CARRIED_NOT_DERIVED_2026_09_23.md
    /// §6.5.9): the writer's UID beside `written_by`, taken from the
    /// request's `Caller` — its `X-Agent-Token` — never from a body field.
    /// Empty = unknown: an Unattributed write, the Armory UI, a seeder.
    /// Dual-written; nothing branches on it.
    pub written_by_uid: String,
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
    pub written_by: String,
    /// See [`BundleVersion::written_by_uid`].
    pub written_by_uid: String,
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

/// Appends a version row within an ALREADY-OPEN transaction — the shared
/// implementation both `Store::bundle_version_insert` below (opens its own
/// transaction) and `Store::bundle_upsert_with_version` (`bundles.rs`,
/// reuses the SAME transaction as its own `db_bundles` write, so the bundle
/// row and its version can never observably diverge under a concurrent
/// writer — codex P2, PR #3237) call, rather than duplicating the
/// parent-chain-lookup + INSERT SQL in two places. `pub(crate)`, not
/// private: `bundles.rs` is a sibling module in this same crate, not this
/// one.
#[allow(clippy::too_many_arguments)]
pub(crate) fn bundle_version_insert_tx(
    tx: &rusqlite::Transaction,
    bundle_id: &str,
    name: &str,
    instructions: &str,
    source: &str,
    source_detail: &str,
    written_by: &str,
    written_by_uid: &str,
) -> Result<BundleVersion, StoreError> {
    let id = uuid::Uuid::new_v4().to_string();
    let hash = content_hash(name, instructions);
    let created_at = now_ms();
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
             (id, bundle_id, name, instructions, content_hash, parent_version_id, source, source_detail, written_by, created_at, written_by_uid)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            id,
            bundle_id,
            name,
            instructions,
            hash,
            parent_version_id,
            source,
            source_detail,
            written_by,
            created_at,
            written_by_uid,
        ],
    )?;

    Ok(BundleVersion {
        id,
        bundle_id: bundle_id.to_string(),
        name: name.to_string(),
        instructions: instructions.to_string(),
        content_hash: hash,
        parent_version_id,
        source: source.to_string(),
        source_detail: source_detail.to_string(),
        written_by: written_by.to_string(),
        written_by_uid: written_by_uid.to_string(),
        created_at,
    })
}

impl Store {
    /// Records a new version for `bundle_id`, chained onto whatever the
    /// current latest version for this bundle is (`None` for a brand-new
    /// bundle's first version). Every call records a version, even if
    /// content is byte-identical to the previous one — same "simplicity
    /// over cleverness, no dedup-by-hash" posture as
    /// `agent_native_memory_version_insert`. `BEGIN IMMEDIATE` for the same
    /// cross-process reason that function's own doc comment gives.
    ///
    /// Standalone (not composed with a `db_bundles` write) — kept for
    /// direct testing and any future caller that only needs to append a
    /// version without also touching the bundle row itself. Production
    /// write paths use `Store::bundle_upsert_with_version` (`bundles.rs`)
    /// instead, which keeps the bundle row and its version atomic.
    pub fn bundle_version_insert(
        &self,
        bundle_id: &str,
        name: &str,
        instructions: &str,
        source: &str,
        source_detail: &str,
        written_by: &str,
    ) -> Result<BundleVersion, StoreError> {
        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        // No `Caller` reaches this standalone path, so no writer UID.
        let version = bundle_version_insert_tx(&tx, bundle_id, name, instructions, source, source_detail, written_by, "")?;
        tx.commit()?;
        Ok(version)
    }

    /// History for one bundle, newest first — metadata only, no content
    /// (mirrors `agent_native_memory_version_list`). Not called by the
    /// Operator Config reseed path — `Store::bundle_reseed_system_if_owned`
    /// (`bundles.rs`) performs its own inline `SELECT written_by,
    /// content_hash, source_detail ... LIMIT 1` query instead, so its
    /// ownership check and the write it gates happen in one transaction
    /// (see that method's own doc comment for why). Backs the
    /// `GlobalMemoryHistory` MCP tool (`global_memory_history_impl`,
    /// `server/app_api/mod.rs`) as of
    /// SPEC_AGENT_FACING_GLOBAL_MEMORY_API_2026_09_15.md's Phase-3
    /// follow-up — also still exercised directly by this file's own tests.
    pub fn bundle_version_list(&self, bundle_id: &str) -> Result<Vec<BundleVersionSummary>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, content_hash, parent_version_id, source, source_detail, written_by, created_at, written_by_uid
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
                    written_by: row.get(5)?,
                    created_at: row.get(6)?,
                    written_by_uid: row.get(7)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// One version by id, full content included — the lookup `diff`/`revert`
    /// build on (mirrors `agent_native_memory_version_get`). Called by
    /// `global_memory_diff_impl`/`global_memory_revert_impl`
    /// (`app_api/mod.rs`), which back the `GlobalMemoryDiff`/
    /// `GlobalMemoryRevert` MCP tools.
    pub fn bundle_version_get(&self, version_id: &str) -> Result<Option<BundleVersion>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, bundle_id, name, instructions, content_hash, parent_version_id, source, source_detail, written_by, created_at, written_by_uid
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
                written_by: row.get(8)?,
                created_at: row.get(9)?,
                written_by_uid: row.get(10)?,
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
        Store::open_shared(std::path::Path::new(":memory:")).unwrap()
    }

    #[test]
    fn insert_chains_parent_version_id() {
        let store = test_store();
        let v1 = store
            .bundle_version_insert("bundle-1", "Coding Standards", "use tabs", "agent_inferred", "{}", "agent-1")
            .unwrap();
        assert!(v1.parent_version_id.is_none());

        let v2 = store
            .bundle_version_insert("bundle-1", "Coding Standards", "use spaces", "agent_inferred", "{}", "agent-1")
            .unwrap();
        assert_eq!(v2.parent_version_id, Some(v1.id.clone()));
        assert_ne!(v1.content_hash, v2.content_hash);
    }

    #[test]
    fn list_is_newest_first_and_scoped_to_one_bundle() {
        let store = test_store();
        store
            .bundle_version_insert("bundle-a", "A", "content 1", "agent_inferred", "{}", "agent-1")
            .unwrap();
        let a2 = store
            .bundle_version_insert("bundle-a", "A", "content 2", "agent_inferred", "{}", "agent-1")
            .unwrap();
        store
            .bundle_version_insert("bundle-b", "B", "unrelated", "agent_inferred", "{}", "agent-1")
            .unwrap();

        let history = store.bundle_version_list("bundle-a").unwrap();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0].id, a2.id, "newest first");
    }

    #[test]
    fn get_returns_full_content_by_id() {
        let store = test_store();
        let v = store
            .bundle_version_insert("bundle-1", "Name", "the body", "human", "{}", "armory-ui")
            .unwrap();
        let fetched = store.bundle_version_get(&v.id).unwrap().expect("version exists");
        assert_eq!(fetched.instructions, "the body");
        assert_eq!(fetched.bundle_id, "bundle-1");
        assert_eq!(fetched.written_by, "armory-ui");
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
            .bundle_version_insert("bundle-1", "Old Name", "same body", "agent_inferred", "{}", "agent-1")
            .unwrap();
        let v2 = store
            .bundle_version_insert("bundle-1", "New Name", "same body", "agent_inferred", "{}", "agent-1")
            .unwrap();
        assert_ne!(v1.content_hash, v2.content_hash, "renaming is a real edit, not a no-op");
    }

    /// codex P2, PR #3237: `written_by` must reflect the TRUSTED caller
    /// identity, independent of whatever the caller claims via
    /// `source`/`source_detail` — an agent could otherwise claim
    /// `source: "human"` with nothing to contradict it.
    #[test]
    fn written_by_is_independent_of_the_caller_supplied_source() {
        let store = test_store();
        let v = store
            .bundle_version_insert("bundle-1", "Name", "body", "human", "{}", "agent-should-not-be-trusted-as-human")
            .unwrap();
        assert_eq!(v.source, "human", "source is caller-supplied annotation, kept as-is");
        assert_eq!(v.written_by, "agent-should-not-be-trusted-as-human", "written_by is the real, separate identity field");
    }
}
