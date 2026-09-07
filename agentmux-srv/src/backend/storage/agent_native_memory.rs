// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Durable, location-consistent mirror of each agent's native (Claude Code)
//! memory files. See db_agent_native_memory in migrations.rs
//! (SHARED_STORE_SCHEMA_VERSION v6 / OBJECT_SCHEMA_VERSION v14) and
//! docs/specs/SPEC_NATIVE_MEMORY_DURABLE_SYNC_2026_08_07.md for the full
//! design.
//!
//! Keyed by (agent_id, filename), where agent_id is the stable
//! `AgentDefinition.id` — NOT any live filesystem path, which is
//! channel-relative by design and therefore not the same across
//! channels/instances for the same logical agent.

use rusqlite::params;

use super::error::StoreError;
use super::store::Store;

#[derive(Debug, Clone, Default, PartialEq)]
pub struct NativeMemoryMirrorRow {
    pub filename: String,
    pub content: String,
    pub metadata_type: Option<String>,
    pub size_bytes: i64,
    pub updated_at: i64,
    pub last_seen_path: String,
    /// The live file's own on-disk mtime (ms epoch) as of the last upsert —
    /// NOT `updated_at` (this row's own write time). Callers use this
    /// together with `size_bytes` to detect whether a live file has
    /// actually changed since it was last mirrored, without re-reading its
    /// full content: size alone under-detects a same-byte-length edit
    /// (reagent P1 on PR #2459 — a same-size edit made only on channel A,
    /// never re-read there via `read_file`, would otherwise serve stale
    /// content to channel B forever, since B has no live copy of its own to
    /// self-correct with).
    pub last_seen_mtime_ms: i64,
}

fn now_ms() -> i64 {
    agentmux_common::time::now_ms()
}

/// One `db_agent_native_memory` row, with its `agent_id` — the shape
/// `agent_native_memory_list_all_rows` returns for the (not agent-scoped)
/// full-table scan a one-time migration needs; every other reader in this
/// module is scoped to a single `agent_id` and doesn't need this.
#[derive(Debug, Clone)]
pub struct NativeMemoryMirrorRowWithAgent {
    pub agent_id: String,
    pub filename: String,
    pub content: String,
    pub metadata_type: Option<String>,
    pub updated_at: i64,
}

impl Store {
    /// List every `db_agent_native_memory` row across every agent, content
    /// included — used only by the one-time version-history backfill
    /// migration (`migrations::m0023_native_memory_versions_backfill`).
    /// Not for any live RPC path (those are always scoped to one
    /// `agent_id`, and this loads full content for every row at once).
    pub fn agent_native_memory_list_all_rows(&self) -> Result<Vec<NativeMemoryMirrorRowWithAgent>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT agent_id, filename, content, metadata_type, updated_at FROM db_agent_native_memory",
        )?;
        let rows = stmt
            .query_map(params![], |row| {
                let metadata_type: String = row.get(3)?;
                Ok(NativeMemoryMirrorRowWithAgent {
                    agent_id: row.get(0)?,
                    filename: row.get(1)?,
                    content: row.get(2)?,
                    metadata_type: if metadata_type.is_empty() { None } else { Some(metadata_type) },
                    updated_at: row.get(4)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// List every mirrored file for `agent_id` (no content — callers that
    /// only need list-view metadata should prefer this over
    /// `agent_native_memory_list` to avoid loading full file bodies).
    pub fn agent_native_memory_list_meta(
        &self,
        agent_id: &str,
    ) -> Result<Vec<NativeMemoryMirrorRow>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT filename, '', metadata_type, size_bytes, updated_at, last_seen_path, last_seen_mtime_ms
             FROM db_agent_native_memory WHERE agent_id = ?1",
        )?;
        let rows = stmt
            .query_map(params![agent_id], |row| {
                let metadata_type: String = row.get(2)?;
                Ok(NativeMemoryMirrorRow {
                    filename: row.get(0)?,
                    content: row.get(1)?,
                    metadata_type: if metadata_type.is_empty() { None } else { Some(metadata_type) },
                    size_bytes: row.get(3)?,
                    updated_at: row.get(4)?,
                    last_seen_path: row.get(5)?,
                    last_seen_mtime_ms: row.get(6)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Read one mirrored file's content by `(agent_id, filename)`. Used as
    /// the read-path fallback when the file is missing from the live FS
    /// (e.g. a different channel wrote it, or the live folder was wiped).
    pub fn agent_native_memory_read(
        &self,
        agent_id: &str,
        filename: &str,
    ) -> Result<Option<String>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let mut stmt = conn.prepare(
            "SELECT content FROM db_agent_native_memory WHERE agent_id = ?1 AND filename = ?2",
        )?;
        match stmt.query_row(params![agent_id, filename], |row| row.get::<_, String>(0)) {
            Ok(content) => Ok(Some(content)),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
            Err(e) => Err(e.into()),
        }
    }

    /// Upsert a file's current live-FS content into the mirror. Called from
    /// every `agent:memory:list`/`read_file`/`write_file` RPC on every file
    /// it touches — cheap (one local SQLite write already on the request),
    /// and is what makes native memory durable across a channel switch.
    ///
    /// `size_bytes` is the file's REAL on-disk size, not necessarily
    /// `content.len()` — `content` may be truncated at the RPC layer's
    /// `MAX_MEMORY_FILE_BYTES` cap for a file Claude Code wrote directly
    /// (bypassing `write_file`'s own size guard). Storing the real size (not
    /// the capped one) is what lets `agent:memory:list`'s change-detection
    /// via `(size_bytes, last_seen_mtime_ms)` still work correctly for an
    /// oversized file — comparing against a capped mirror size would never
    /// match the live (larger) size, forcing a full re-read + rewrite on
    /// every single list() call forever (reagent P1 on PR #2459, fourth pass).
    ///
    /// `mtime_ms` is the live file's own on-disk modified time (0 if
    /// unavailable, e.g. a `write_file` call that hasn't re-stat'd the file
    /// it just wrote) — stored as `last_seen_mtime_ms` for the same
    /// change-detection purpose.
    pub fn agent_native_memory_upsert(
        &self,
        agent_id: &str,
        filename: &str,
        content: &str,
        metadata_type: Option<&str>,
        last_seen_path: &str,
        size_bytes: i64,
        mtime_ms: i64,
    ) -> Result<(), StoreError> {
        let conn = self.conn.lock().unwrap();
        conn.execute(
            "INSERT INTO db_agent_native_memory
                 (agent_id, filename, content, metadata_type, size_bytes, updated_at, last_seen_path, last_seen_mtime_ms)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
             ON CONFLICT(agent_id, filename) DO UPDATE SET
                 content = excluded.content,
                 metadata_type = excluded.metadata_type,
                 size_bytes = excluded.size_bytes,
                 updated_at = excluded.updated_at,
                 last_seen_path = excluded.last_seen_path,
                 last_seen_mtime_ms = excluded.last_seen_mtime_ms",
            params![
                agent_id,
                filename,
                content,
                metadata_type.unwrap_or(""),
                size_bytes,
                now_ms(),
                last_seen_path,
                mtime_ms,
            ],
        )?;
        Ok(())
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
    fn upsert_then_read_round_trips() {
        let store = shared_store();
        store
            .agent_native_memory_upsert("agent-1", "MEMORY.md", "# hello", Some("user"), "/tmp/memory/MEMORY.md", 7, 1000)
            .unwrap();

        let content = store.agent_native_memory_read("agent-1", "MEMORY.md").unwrap();
        assert_eq!(content, Some("# hello".to_string()));
    }

    #[test]
    fn read_returns_none_for_an_unmirrored_file() {
        let store = shared_store();
        assert_eq!(store.agent_native_memory_read("agent-1", "MEMORY.md").unwrap(), None);
    }

    #[test]
    fn upsert_is_idempotent_and_updates_in_place() {
        let store = shared_store();
        store
            .agent_native_memory_upsert("agent-1", "MEMORY.md", "v1", None, "/a", 2, 1000)
            .unwrap();
        store
            .agent_native_memory_upsert("agent-1", "MEMORY.md", "v2", Some("user"), "/b", 2, 2000)
            .unwrap();

        let content = store.agent_native_memory_read("agent-1", "MEMORY.md").unwrap();
        assert_eq!(content, Some("v2".to_string()));

        let meta = store.agent_native_memory_list_meta("agent-1").unwrap();
        assert_eq!(meta.len(), 1);
        assert_eq!(meta[0].metadata_type, Some("user".to_string()));
        assert_eq!(meta[0].last_seen_path, "/b");
        assert_eq!(meta[0].size_bytes, 2);
        assert_eq!(meta[0].last_seen_mtime_ms, 2000);
    }

    #[test]
    fn upsert_stores_the_caller_supplied_size_not_the_content_length() {
        // reagent P1 on PR #2459 (fourth pass): for a file exceeding the RPC
        // layer's MAX_MEMORY_FILE_BYTES cap, `content` is truncated but the
        // real on-disk size is larger — callers pass that real size
        // explicitly so a later comparison against the live file's actual
        // size still matches. Deriving size_bytes from content.len() instead
        // would make an oversized file's mirror row NEVER match the live
        // size, forcing a full re-read on every single list() call forever.
        let store = shared_store();
        store
            .agent_native_memory_upsert("agent-1", "MEMORY.md", "truncated...", None, "/a", 50_000_000, 1000)
            .unwrap();

        let meta = store.agent_native_memory_list_meta("agent-1").unwrap();
        assert_eq!(meta[0].size_bytes, 50_000_000, "size_bytes must be the caller-supplied real size, not content.len()");
    }

    #[test]
    fn list_meta_scopes_to_the_requested_agent_only() {
        let store = shared_store();
        store.agent_native_memory_upsert("agent-1", "a.md", "x", None, "/a", 1, 1000).unwrap();
        store.agent_native_memory_upsert("agent-2", "b.md", "y", None, "/b", 1, 1000).unwrap();

        let meta = store.agent_native_memory_list_meta("agent-1").unwrap();
        assert_eq!(meta.len(), 1);
        assert_eq!(meta[0].filename, "a.md");
    }

    #[test]
    fn list_meta_omits_content_by_design() {
        // list_meta is the list-view path — it must not carry full file
        // bodies (that's what agent_native_memory_read is for), so a large
        // mirrored file doesn't get loaded into memory just to render a list.
        let store = shared_store();
        store.agent_native_memory_upsert("agent-1", "a.md", "big content", None, "/a", 11, 1000).unwrap();
        let meta = store.agent_native_memory_list_meta("agent-1").unwrap();
        assert_eq!(meta[0].content, "");
    }

    #[test]
    fn a_file_deleted_from_the_live_fs_after_being_mirrored_once_stays_visible() {
        // Simulates SPEC_NATIVE_MEMORY_DURABLE_SYNC_2026_08_07.md §5(c): the
        // mirror is the only remaining copy once the live FS file is gone —
        // list/read must still return it from here.
        let store = shared_store();
        store.agent_native_memory_upsert("agent-1", "MEMORY.md", "content", None, "/gone", 7, 1000).unwrap();

        let meta = store.agent_native_memory_list_meta("agent-1").unwrap();
        assert_eq!(meta.len(), 1);
        let content = store.agent_native_memory_read("agent-1", "MEMORY.md").unwrap();
        assert_eq!(content, Some("content".to_string()));
    }
}
