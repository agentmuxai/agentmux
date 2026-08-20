// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Append-only version history for native memory content. See
//! `db_agent_native_memory_versions` in migrations.rs
//! (`SHARED_STORE_SCHEMA_VERSION` v8 / `OBJECT_SCHEMA_VERSION` v24 /
//! `IDENTITY_STORE_SCHEMA_VERSION` v3) and
//! docs/specs/SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md for
//! the full design.
//!
//! Additive to `db_agent_native_memory` (the current-value mirror in
//! `agent_native_memory.rs`) — never read on that module's hot path
//! (`list`/`read_file`), only by the `agent:memory:history`/`diff`/`revert`
//! RPCs. A version row is inserted on every `agent:memory:write_file` call
//! (source-tagged; see the spec's §4.1) and, separately, by
//! `agent:memory:revert` (source `"revert"`, never overwriting or deleting
//! a prior row — a revert is always a new version, git-revert-style).

use rusqlite::params;
use sha2::{Digest, Sha256};

use super::error::StoreError;
use super::store::Store;

/// One version of one memory file's content, including its full body.
#[derive(Debug, Clone, PartialEq)]
pub struct NativeMemoryVersion {
    pub id: String,
    pub agent_id: String,
    pub filename: String,
    pub content: String,
    pub content_hash: String,
    /// The version this one superseded, or `None` for the first version
    /// ever recorded for this `(agent_id, filename)`.
    pub parent_version_id: Option<String>,
    /// `"human"` | `"agent_inferred"` | `"jekt"` | `"external_fs_write"` |
    /// `"revert"` — not a Rust enum: this column is read/written by
    /// forward-compatible callers (new sources can be added without a
    /// schema migration), and validated at the RPC layer instead.
    pub source: String,
    /// JSON blob — jekt marker fields when `source == "jekt"`, detection
    /// method when `source == "external_fs_write"`, `"{}"` otherwise.
    pub source_detail: String,
    pub session_id: String,
    pub created_at: i64,
}

/// A version's metadata without its full content — the list-view shape,
/// mirroring `NativeMemoryMirrorRow`'s own list_meta/read split in
/// `agent_native_memory.rs` (a history view renders many rows at once; a
/// diff/revert call reads exactly one or two full bodies).
#[derive(Debug, Clone, PartialEq)]
pub struct NativeMemoryVersionSummary {
    pub id: String,
    pub content_hash: String,
    pub parent_version_id: Option<String>,
    pub source: String,
    pub source_detail: String,
    pub session_id: String,
    pub created_at: i64,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as i64
}

/// SHA-256 hex digest of a memory file's content — used both to populate
/// `content_hash` on insert and, later, by §4.5 drift detection to compare
/// a live file's current hash against the last recorded version without
/// storing the live content redundantly.
pub(crate) fn content_hash(content: &str) -> String {
    let mut hasher = Sha256::new();
    hasher.update(content.as_bytes());
    hex::encode(hasher.finalize())
}

impl Store {
    /// Record a new version of `(agent_id, filename)`'s content, chained
    /// onto whatever version was previously latest for this file (`None`
    /// for the first write ever recorded). Always inserts, even when
    /// `content` is byte-identical to the previous version — simplicity
    /// over cleverness; dedup-by-hash is a possible future optimization,
    /// not this one.
    ///
    /// reagent P2 (re-review): the read-latest-then-insert sequence is a
    /// single connection-lock acquisition, not two — an earlier revision
    /// released the lock between reading `parent_version_id` and
    /// inserting, so two truly concurrent calls for the same `(agent_id,
    /// filename)` (e.g. two panes of the same agent identity writing at
    /// once) could both read the same latest id and insert sibling
    /// versions instead of a linear chain.
    ///
    /// reagent P2 (second re-review, PR #2674): an in-process `Mutex` only
    /// serializes callers sharing this one `Store`/`Connection` — it does
    /// nothing for two SEPARATE `srv` processes (e.g. two channels/
    /// instances of AgentMux) each holding their own `Connection` to the
    /// same shared-store SQLite file. Those two connections' SELECT-latest
    /// and INSERT could still interleave across processes, producing
    /// sibling versions with the same `parent_version_id`. Wrapping the
    /// read+insert in a `BEGIN IMMEDIATE` transaction closes that gap at
    /// the SQLite level: it acquires the RESERVED lock up front (before
    /// the SELECT runs), so a second connection's own `BEGIN IMMEDIATE`
    /// blocks (up to `busy_timeout`, already configured at open) until
    /// this one commits — true cross-process mutual exclusion, unlike a
    /// plain deferred transaction (whose lock isn't acquired until the
    /// first write, after the read has already happened).
    pub fn agent_native_memory_version_insert(
        &self,
        agent_id: &str,
        filename: &str,
        content: &str,
        source: &str,
        source_detail: &str,
        session_id: &str,
    ) -> Result<NativeMemoryVersion, StoreError> {
        let id = uuid::Uuid::new_v4().to_string();
        let hash = content_hash(content);
        let created_at = now_ms();

        let mut conn = self.conn.lock().unwrap();
        let tx = conn.transaction_with_behavior(rusqlite::TransactionBehavior::Immediate)?;
        let parent_version_id: Option<String> = {
            let mut stmt = tx.prepare(
                "SELECT id FROM db_agent_native_memory_versions
                 WHERE agent_id = ?1 AND filename = ?2
                 ORDER BY created_at DESC, rowid DESC
                 LIMIT 1",
            )?;
            match stmt.query_row(params![agent_id, filename], |row| row.get::<_, String>(0)) {
                Ok(id) => Some(id),
                Err(rusqlite::Error::QueryReturnedNoRows) => None,
                Err(e) => return Err(e.into()),
            }
        };
        tx.execute(
            "INSERT INTO db_agent_native_memory_versions
                 (id, agent_id, filename, content, content_hash, parent_version_id, source, source_detail, session_id, created_at)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            params![
                id,
                agent_id,
                filename,
                content,
                hash,
                parent_version_id,
                source,
                source_detail,
                session_id,
                created_at,
            ],
        )?;
        tx.commit()?;

        Ok(NativeMemoryVersion {
            id,
            agent_id: agent_id.to_string(),
            filename: filename.to_string(),
            content: content.to_string(),
            content_hash: hash,
            parent_version_id,
            source: source.to_string(),
            source_detail: source_detail.to_string(),
            session_id: session_id.to_string(),
            created_at,
        })
    }

    /// List every version of `(agent_id, filename)`, newest first — no
    /// content (list-view; see [`NativeMemoryVersionSummary`]'s own doc).
    pub fn agent_native_memory_version_list(
        &self,
        agent_id: &str,
        filename: &str,
    ) -> Result<Vec<NativeMemoryVersionSummary>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, content_hash, parent_version_id, source, source_detail, session_id, created_at
             FROM db_agent_native_memory_versions
             WHERE agent_id = ?1 AND filename = ?2
             ORDER BY created_at DESC, rowid DESC",
        )?;
        let rows = stmt
            .query_map(params![agent_id, filename], |row| {
                Ok(NativeMemoryVersionSummary {
                    id: row.get(0)?,
                    content_hash: row.get(1)?,
                    parent_version_id: row.get(2)?,
                    source: row.get(3)?,
                    source_detail: row.get(4)?,
                    session_id: row.get(5)?,
                    created_at: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// The most recently recorded version of `(agent_id, filename)`, or
    /// `None` if no version has ever been recorded. Used both to derive a
    /// new write's `parent_version_id` and (§4.5 drift detection, not yet
    /// built) to compare a live file's current hash against the last known
    /// version.
    pub fn agent_native_memory_version_latest(
        &self,
        agent_id: &str,
        filename: &str,
    ) -> Result<Option<NativeMemoryVersion>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, content, content_hash, parent_version_id, source, source_detail, session_id, created_at
             FROM db_agent_native_memory_versions
             WHERE agent_id = ?1 AND filename = ?2
             ORDER BY created_at DESC, rowid DESC
             LIMIT 1",
        )?;
        let agent_id_owned = agent_id.to_string();
        let filename_owned = filename.to_string();
        match stmt.query_row(params![agent_id, filename], |row| {
            Ok(NativeMemoryVersion {
                id: row.get(0)?,
                agent_id: agent_id_owned.clone(),
                filename: filename_owned.clone(),
                content: row.get(1)?,
                content_hash: row.get(2)?,
                parent_version_id: row.get(3)?,
                source: row.get(4)?,
                source_detail: row.get(5)?,
                session_id: row.get(6)?,
                created_at: row.get(7)?,
            })
        }) {
            Ok(v) => Ok(Some(v)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Read one version's full content by id — for diff/revert, which need
    /// exactly one or two full bodies, not a whole file's history.
    pub fn agent_native_memory_version_get(
        &self,
        version_id: &str,
    ) -> Result<Option<NativeMemoryVersion>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT id, agent_id, filename, content, content_hash, parent_version_id, source, source_detail, session_id, created_at
             FROM db_agent_native_memory_versions
             WHERE id = ?1",
        )?;
        match stmt.query_row(params![version_id], |row| {
            Ok(NativeMemoryVersion {
                id: row.get(0)?,
                agent_id: row.get(1)?,
                filename: row.get(2)?,
                content: row.get(3)?,
                content_hash: row.get(4)?,
                parent_version_id: row.get(5)?,
                source: row.get(6)?,
                source_detail: row.get(7)?,
                session_id: row.get(8)?,
                created_at: row.get(9)?,
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

    fn shared_store() -> Store {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        Store::open_shared(tmp.path()).unwrap()
    }

    #[test]
    fn first_write_has_no_parent() {
        let store = shared_store();
        let v = store
            .agent_native_memory_version_insert("agent-1", "MEMORY.md", "v1", "human", "{}", "sess-1")
            .unwrap();
        assert_eq!(v.parent_version_id, None);
        assert_eq!(v.content, "v1");
        assert_eq!(v.content_hash, content_hash("v1"));
    }

    #[test]
    fn second_write_chains_onto_the_first() {
        let store = shared_store();
        let v1 = store
            .agent_native_memory_version_insert("agent-1", "MEMORY.md", "v1", "human", "{}", "")
            .unwrap();
        let v2 = store
            .agent_native_memory_version_insert("agent-1", "MEMORY.md", "v2", "agent_inferred", "{}", "")
            .unwrap();
        assert_eq!(v2.parent_version_id, Some(v1.id));
    }

    #[test]
    fn a_no_op_write_still_records_a_new_version() {
        // Simplicity over cleverness (spec §4.1's own test-plan note): two
        // writes with byte-identical content still produce two version rows,
        // chained onto each other — no dedup-by-hash in v1.
        let store = shared_store();
        let v1 = store
            .agent_native_memory_version_insert("agent-1", "MEMORY.md", "same", "human", "{}", "")
            .unwrap();
        let v2 = store
            .agent_native_memory_version_insert("agent-1", "MEMORY.md", "same", "human", "{}", "")
            .unwrap();
        assert_ne!(v1.id, v2.id);
        assert_eq!(v2.parent_version_id, Some(v1.id));
        assert_eq!(v1.content_hash, v2.content_hash);
    }

    // Regression for reagent P2 on PR #2674 (second re-review): an
    // in-process Mutex only serializes callers sharing one Store/Connection
    // — it does nothing for two SEPARATE connections to the same
    // shared-store sqlite file (the real-world case: two srv processes,
    // e.g. two AgentMux channels/instances, both writing the same agent's
    // same memory file at close to the same moment). Opens the SAME
    // on-disk file via two independent `Store` instances (simulating two
    // processes) and fires one truly-concurrent insert from each — without
    // the `BEGIN IMMEDIATE` fix, both connections' SELECT-latest could read
    // the same pre-existing "latest" and each insert a version with that
    // same parent, instead of one correctly chaining onto the other.
    //
    // Run several rounds rather than one shot — a race that depends on
    // exact timing is more convincing (and less likely to pass by luck on
    // a fast machine) when it holds up repeatedly, not just once.
    #[test]
    fn concurrent_inserts_from_two_separate_connections_form_a_linear_chain() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().to_path_buf();
        let store_a = std::sync::Arc::new(Store::open_shared(&path).unwrap());
        let store_b = std::sync::Arc::new(Store::open_shared(&path).unwrap());

        for round in 0..8 {
            let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
            let (ba, bb) = (barrier.clone(), barrier.clone());
            let (sa, sb) = (store_a.clone(), store_b.clone());
            let filename = format!("MEMORY-{round}.md");
            let (fa, fb) = (filename.clone(), filename.clone());

            let ha = std::thread::spawn(move || {
                ba.wait();
                sa.agent_native_memory_version_insert("agent-1", &fa, "A", "human", "{}", "").unwrap()
            });
            let hb = std::thread::spawn(move || {
                bb.wait();
                sb.agent_native_memory_version_insert("agent-1", &fb, "B", "human", "{}", "").unwrap()
            });
            let va = ha.join().unwrap();
            let vb = hb.join().unwrap();

            // Exactly one must chain onto the other — never both `None`
            // parents (both saw an empty table) and never two unrelated
            // versions with no parent/child relationship between them.
            let a_onto_b = va.parent_version_id == Some(vb.id.clone());
            let b_onto_a = vb.parent_version_id == Some(va.id.clone());
            assert!(
                a_onto_b != b_onto_a,
                "round {round}: exactly one of A/B must chain onto the other — A.parent={:?} B.parent={:?}",
                va.parent_version_id, vb.parent_version_id,
            );
        }
    }

    #[test]
    fn list_returns_newest_first() {
        let store = shared_store();
        store
            .agent_native_memory_version_insert("agent-1", "MEMORY.md", "v1", "human", "{}", "")
            .unwrap();
        store
            .agent_native_memory_version_insert("agent-1", "MEMORY.md", "v2", "human", "{}", "")
            .unwrap();
        let list = store.agent_native_memory_version_list("agent-1", "MEMORY.md").unwrap();
        assert_eq!(list.len(), 2);
        // Newest (v2, no children) first.
        assert!(list[0].content_hash == content_hash("v2"));
        assert!(list[1].content_hash == content_hash("v1"));
    }

    #[test]
    fn list_scopes_to_the_requested_agent_and_filename_only() {
        let store = shared_store();
        store
            .agent_native_memory_version_insert("agent-1", "a.md", "x", "human", "{}", "")
            .unwrap();
        store
            .agent_native_memory_version_insert("agent-1", "b.md", "y", "human", "{}", "")
            .unwrap();
        store
            .agent_native_memory_version_insert("agent-2", "a.md", "z", "human", "{}", "")
            .unwrap();

        let list = store.agent_native_memory_version_list("agent-1", "a.md").unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].content_hash, content_hash("x"));
    }

    #[test]
    fn list_omits_content_by_design() {
        let store = shared_store();
        store
            .agent_native_memory_version_insert("agent-1", "MEMORY.md", "some content", "human", "{}", "")
            .unwrap();
        // NativeMemoryVersionSummary has no `content` field at all — this
        // test documents that as intentional (compile-time enforced), not
        // just a runtime assertion. If this ever fails to compile because a
        // future refactor added a content field, that regressed the
        // list-view/full-body split agent_native_memory.rs's own docs
        // established as the pattern to follow.
        let list = store.agent_native_memory_version_list("agent-1", "MEMORY.md").unwrap();
        assert_eq!(list.len(), 1);
    }

    #[test]
    fn latest_returns_none_when_no_version_exists() {
        let store = shared_store();
        assert_eq!(
            store.agent_native_memory_version_latest("agent-1", "MEMORY.md").unwrap(),
            None
        );
    }

    #[test]
    fn get_reads_a_specific_version_by_id_with_full_content() {
        let store = shared_store();
        let v1 = store
            .agent_native_memory_version_insert("agent-1", "MEMORY.md", "v1", "human", "{}", "")
            .unwrap();
        store
            .agent_native_memory_version_insert("agent-1", "MEMORY.md", "v2", "human", "{}", "")
            .unwrap();

        // get() must return v1's content even though it's no longer latest.
        let got = store.agent_native_memory_version_get(&v1.id).unwrap();
        assert_eq!(got, Some(v1));
    }

    #[test]
    fn get_returns_none_for_an_unknown_id() {
        let store = shared_store();
        assert_eq!(store.agent_native_memory_version_get("nope").unwrap(), None);
    }

    #[test]
    fn source_and_source_detail_round_trip_exactly() {
        let store = shared_store();
        let detail = r#"{"FROM":"github-consumer","TIER":"sensitive","TRUST":"network-claimed"}"#;
        let v = store
            .agent_native_memory_version_insert("agent-1", "MEMORY.md", "content", "jekt", detail, "sess-42")
            .unwrap();
        assert_eq!(v.source, "jekt");
        assert_eq!(v.source_detail, detail);
        assert_eq!(v.session_id, "sess-42");

        let list = store.agent_native_memory_version_list("agent-1", "MEMORY.md").unwrap();
        assert_eq!(list[0].source, "jekt");
        assert_eq!(list[0].source_detail, detail);
    }
}
