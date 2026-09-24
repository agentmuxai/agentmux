// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Multi-file replace and delete in one transaction (Phase 5a-2b,
//! SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md §6.3.7 item 4).
//!
//! A transcript `output` has sidecars that describe its bytes (`output.idx`
//! line offsets, `output.tsidx` receive times). Replacing or deleting
//! `output` and its sidecars in separate steps leaves a window — or, after a
//! crash, a permanent state — where a sidecar describes content that no
//! longer exists, and a reader trusts it by coincidence of size. These do
//! the whole change in one transaction.

use rusqlite::{params, OptionalExtension};

use super::core::FileStore;
use super::{FileMeta, FileOpts};
use crate::backend::storage::error::StoreError;

impl FileStore {
    /// Replace `name`'s content with `data` (creating the file if missing)
    /// and delete the files in `drop`, in one transaction. The new content
    /// starts a new counted epoch with a fresh `gen` (counter.rs).
    pub fn replace_file(&self, zone_id: &str, name: &str, data: &[u8], drop: &[&str]) -> Result<(), StoreError> {
        self.replace_inner(zone_id, name, data, None, drop, None).map(|_| ())
    }

    /// Replace `name`'s content with `data` (creating the file if missing)
    /// and merge `meta` into its metadata (a null value removes a key), in
    /// one transaction — so the metadata can describe the content without a
    /// window where it describes other content.
    pub fn put_file_with_meta(&self, zone_id: &str, name: &str, data: &[u8], meta: FileMeta) -> Result<(), StoreError> {
        self.replace_inner(zone_id, name, data, Some(meta), &[], None).map(|_| ())
    }

    /// [`Self::put_file_with_meta`], only if `guard`'s valid generation is
    /// still `expected_gen` — checked in the same transaction as the write.
    /// For a file derived from `guard` (an index of it): if `guard` was
    /// replaced while it was being derived, nothing is written and this
    /// returns `false`. With `expected_gen` `None` (an uncounted `guard`, no
    /// generation to compare) it always writes.
    pub fn put_file_with_meta_if(
        &self,
        zone_id: &str,
        name: &str,
        data: &[u8],
        meta: FileMeta,
        guard: &str,
        expected_gen: Option<&str>,
    ) -> Result<bool, StoreError> {
        self.replace_inner(zone_id, name, data, Some(meta), &[], Some((guard, expected_gen)))
    }

    /// Delete several files of one zone in one transaction. Missing files
    /// are not an error.
    pub fn delete_files(&self, zone_id: &str, names: &[&str]) -> Result<(), StoreError> {
        self.write_txn(|tx| {
            for name in names {
                tx.execute("DELETE FROM db_wave_file WHERE zoneid = ?1 AND name = ?2", params![zone_id, name])?;
                tx.execute("DELETE FROM db_file_data WHERE zoneid = ?1 AND name = ?2", params![zone_id, name])?;
            }
            Ok(())
        })?;
        self.forget_cached(zone_id, names);
        Ok(())
    }

    fn replace_inner(
        &self,
        zone_id: &str,
        name: &str,
        data: &[u8],
        meta: Option<FileMeta>,
        drop: &[&str],
        guard: Option<(&str, Option<&str>)>,
    ) -> Result<bool, StoreError> {
        debug_assert!(!drop.contains(&name), "replace_file would drop the file it writes");
        let now = Self::now_ms();
        let opts_json = serde_json::to_string(&FileOpts::default())?;
        let written = self.write_txn(|tx| {
            if let Some((guard, Some(expected))) = guard {
                let current = super::counter::read_row(tx, zone_id, guard)?.and_then(|r| r.counter()).map(|(gen, _)| gen);
                if current.as_deref() != Some(expected) {
                    return Ok(false);
                }
            }
            let current_meta: Option<String> = tx
                .query_row(
                    "SELECT meta FROM db_wave_file WHERE zoneid = ?1 AND name = ?2",
                    params![zone_id, name],
                    |r| r.get(0),
                )
                .optional()?;
            if current_meta.is_none() {
                tx.execute(
                    "INSERT INTO db_wave_file (zoneid, name, size, createdts, modts, opts, meta)
                     VALUES (?1, ?2, 0, ?3, ?3, ?4, '{}')",
                    params![zone_id, name, now, opts_json],
                )?;
            }
            Self::replace_content_in(tx, zone_id, name, data, now)?;
            if let Some(meta) = meta {
                let mut merged: FileMeta =
                    serde_json::from_str(current_meta.as_deref().unwrap_or("{}")).unwrap_or_default();
                for (k, v) in meta {
                    if v.is_null() {
                        merged.remove(&k);
                    } else {
                        merged.insert(k, v);
                    }
                }
                tx.execute(
                    "UPDATE db_wave_file SET meta = ?1 WHERE zoneid = ?2 AND name = ?3",
                    params![serde_json::to_string(&merged)?, zone_id, name],
                )?;
            }
            for d in drop {
                tx.execute("DELETE FROM db_wave_file WHERE zoneid = ?1 AND name = ?2", params![zone_id, d])?;
                tx.execute("DELETE FROM db_file_data WHERE zoneid = ?1 AND name = ?2", params![zone_id, d])?;
            }
            Ok(true)
        })?;
        if written {
            let mut forget = vec![name];
            forget.extend_from_slice(drop);
            self.forget_cached(zone_id, &forget);
        }
        Ok(written)
    }

    /// Bytes `[offset, offset + len)` of a file, read from the database, not
    /// clamped to this process's cached size (which another process's append
    /// can make stale); callers bound `len` by a size read from the database
    /// (`line_state`). An error if any byte of the range isn't stored — a
    /// file mid-write by a build without transactions claims bytes before it
    /// holds them — so nothing is ever indexed or served as zeros.
    pub fn read_bytes_db(&self, zone_id: &str, name: &str, offset: i64, len: i64) -> Result<Vec<u8>, StoreError> {
        let conn = self.conn.lock().unwrap();
        super::counter::read_bytes_exact(&conn, zone_id, name, offset, len)?.ok_or_else(|| {
            StoreError::Other(format!("{zone_id}/{name}: bytes {offset}..{} not stored yet", offset + len))
        })
    }

    /// A file's metadata, read from the database rather than the cache.
    pub fn meta_db(&self, zone_id: &str, name: &str) -> Result<Option<FileMeta>, StoreError> {
        let conn = self.conn.lock().unwrap();
        let meta: Option<String> = conn
            .query_row(
                "SELECT meta FROM db_wave_file WHERE zoneid = ?1 AND name = ?2",
                params![zone_id, name],
                |r| r.get(0),
            )
            .optional()?;
        Ok(meta.map(|m| serde_json::from_str(&m).unwrap_or_default()))
    }
}
