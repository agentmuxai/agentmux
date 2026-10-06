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

/// Reads inside one [`FileStore::read_snapshot`]: every call sees the same
/// moment of the database.
pub struct SnapshotReader<'a> {
    tx: &'a rusqlite::Transaction<'a>,
}

/// A file as a [`SnapshotReader`] sees it.
#[derive(Debug, Clone)]
pub struct SnapshotFile {
    pub size: i64,
    /// Valid generation; `None` when the file is not counted.
    pub gen: Option<String>,
    pub meta: FileMeta,
}

impl SnapshotReader<'_> {
    pub fn file(&self, zone_id: &str, name: &str) -> Result<Option<SnapshotFile>, StoreError> {
        let Some(row) = super::counter::read_row(self.tx, zone_id, name)? else { return Ok(None) };
        let meta: String = self.tx.query_row(
            "SELECT meta FROM db_wave_file WHERE zoneid = ?1 AND name = ?2",
            params![zone_id, name],
            |r| r.get(0),
        )?;
        Ok(Some(SnapshotFile {
            size: row.size,
            gen: row.counter().map(|(gen, _)| gen),
            meta: serde_json::from_str(&meta).unwrap_or_default(),
        }))
    }

    /// `[offset, offset + len)`, or `None` if any of it isn't stored.
    pub fn bytes(&self, zone_id: &str, name: &str, offset: i64, len: i64) -> Result<Option<Vec<u8>>, StoreError> {
        super::counter::read_bytes_exact(self.tx, zone_id, name, offset, len)
    }
}

/// Write `data` at byte `offset` of a file's stored parts, inside `tx`,
/// reading and rewriting only the parts the range touches. A part shorter
/// than the range is extended (with zeros up to `offset`, which callers
/// never leave as a gap). Leaves the file's recorded size to the caller.
fn patch_range_in(tx: &rusqlite::Transaction<'_>, zone_id: &str, name: &str, offset: i64, data: &[u8]) -> Result<(), StoreError> {
    if data.is_empty() {
        return Ok(());
    }
    let pds = super::core::PART_DATA_SIZE as i64;
    let end = offset + data.len() as i64;
    for idx in (offset / pds)..=((end - 1) / pds) {
        let part_start = idx * pds;
        let mut part: Vec<u8> = tx
            .query_row(
                "SELECT data FROM db_file_data WHERE zoneid = ?1 AND name = ?2 AND partidx = ?3",
                params![zone_id, name, idx as i32],
                |r| r.get(0),
            )
            .optional()?
            .unwrap_or_default();
        let from = offset.max(part_start);
        let to = end.min(part_start + pds);
        let (lo, hi) = ((from - part_start) as usize, (to - part_start) as usize);
        if part.len() < hi {
            part.resize(hi, 0);
        }
        part[lo..hi].copy_from_slice(&data[(from - offset) as usize..(to - offset) as usize]);
        tx.execute(
            "REPLACE INTO db_file_data (zoneid, name, partidx, data) VALUES (?1, ?2, ?3, ?4)",
            params![zone_id, name, idx as i32, part],
        )?;
    }
    Ok(())
}

/// Drop a file's stored bytes from `new_size` on, inside `tx`.
fn truncate_parts_in(tx: &rusqlite::Transaction<'_>, zone_id: &str, name: &str, new_size: i64) -> Result<(), StoreError> {
    let pds = super::core::PART_DATA_SIZE as i64;
    let keep_parts = (new_size + pds - 1) / pds;
    tx.execute(
        "DELETE FROM db_file_data WHERE zoneid = ?1 AND name = ?2 AND partidx >= ?3",
        params![zone_id, name, keep_parts as i32],
    )?;
    let in_last = (new_size % pds) as usize;
    if in_last > 0 {
        let last = (keep_parts - 1) as i32;
        let part: Option<Vec<u8>> = tx
            .query_row(
                "SELECT data FROM db_file_data WHERE zoneid = ?1 AND name = ?2 AND partidx = ?3",
                params![zone_id, name, last],
                |r| r.get(0),
            )
            .optional()?;
        if let Some(mut part) = part.filter(|p| p.len() > in_last) {
            part.truncate(in_last);
            tx.execute(
                "REPLACE INTO db_file_data (zoneid, name, partidx, data) VALUES (?1, ?2, ?3, ?4)",
                params![zone_id, name, last, part],
            )?;
        }
    }
    Ok(())
}

/// See [`FileStore::derived_snapshot`].
#[derive(Debug, Clone)]
pub struct DerivedSnapshot {
    pub output_size: i64,
    /// `output`'s valid generation; `None` when it is not counted.
    pub output_gen: Option<String>,
    pub derived: Option<DerivedView>,
}

#[derive(Debug, Clone)]
pub struct DerivedView {
    pub size: i64,
    /// The requested bytes; `None` if any of them isn't stored yet.
    pub bytes: Option<Vec<u8>>,
    pub meta: FileMeta,
}

impl FileStore {
    /// Replace `name`'s content with `data` (creating the file if missing)
    /// and delete the files in `drop`, in one transaction. The new content
    /// starts a new counted epoch with a fresh `gen` (counter.rs).
    #[track_caller]
    pub fn replace_file(&self, zone_id: &str, name: &str, data: &[u8], drop: &[&str]) -> Result<(), StoreError> {
        self.replace_inner(zone_id, name, data, None, drop, None).map(|_| ())
    }

    /// Replace `name`'s content with `data` (creating the file if missing)
    /// and merge `meta` into its metadata (a null value removes a key), in
    /// one transaction — so the metadata can describe the content without a
    /// window where it describes other content.
    #[track_caller]
    pub fn put_file_with_meta(&self, zone_id: &str, name: &str, data: &[u8], meta: FileMeta) -> Result<(), StoreError> {
        self.replace_inner(zone_id, name, data, Some(meta), &[], None).map(|_| ())
    }

    /// [`Self::put_file_with_meta`], only if `guard`'s valid generation is
    /// still `expected_gen` — checked in the same transaction as the write.
    /// For a file derived from `guard` (an index of it): if `guard` was
    /// replaced while it was being derived, nothing is written and this
    /// returns `false`. With `expected_gen` `None` (an uncounted `guard`, no
    /// generation to compare) it always writes.
    #[track_caller]
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

    /// Patch a file derived from `guard` (an index of it) in place, in one
    /// transaction: write `head` at offset 0 and `tail` at `tail_at`, the file
    /// ending where `tail` does — only if `guard` is still generation
    /// `expected_gen` and `name` is still `expected_size` bytes, the state the
    /// patch was computed from. Otherwise nothing is written and this returns
    /// `false`. Only the parts the two ranges touch are written: extending
    /// `output.idx` used to rewrite all of it (15 MB for a 1.9 M-line
    /// transcript) for every few lines appended.
    #[track_caller]
    #[allow(clippy::too_many_arguments)]
    pub fn patch_derived_if(
        &self,
        zone_id: &str,
        name: &str,
        head: &[u8],
        tail_at: i64,
        tail: &[u8],
        guard: &str,
        expected_gen: &str,
        expected_size: i64,
    ) -> Result<bool, StoreError> {
        if tail_at < head.len() as i64 {
            return Err(StoreError::Other(format!("{zone_id}/{name}: patch tail at {tail_at} overlaps its head")));
        }
        let now = agentmux_common::time::now_ms();
        let written = self.write_txn(|tx| {
            let current = super::counter::read_row(tx, zone_id, guard)?.and_then(|r| r.counter()).map(|(gen, _)| gen);
            if current.as_deref() != Some(expected_gen) {
                return Ok(false);
            }
            let size: Option<i64> = tx
                .query_row(
                    "SELECT size FROM db_wave_file WHERE zoneid = ?1 AND name = ?2",
                    params![zone_id, name],
                    |r| r.get(0),
                )
                .optional()?;
            if size != Some(expected_size) || tail_at > expected_size {
                return Ok(false);
            }
            patch_range_in(tx, zone_id, name, 0, head)?;
            patch_range_in(tx, zone_id, name, tail_at, tail)?;
            let new_size = tail_at + tail.len() as i64;
            if new_size < expected_size {
                truncate_parts_in(tx, zone_id, name, new_size)?;
            }
            tx.execute(
                "UPDATE db_wave_file SET size = ?1, modts = ?2 WHERE zoneid = ?3 AND name = ?4",
                params![new_size, now, zone_id, name],
            )?;
            Ok(true)
        })?;
        if written {
            self.forget_cached(zone_id, &[name]);
        }
        Ok(written)
    }

    /// Delete several files of one zone in one transaction. Missing files
    /// are not an error.
    #[track_caller]
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

    #[track_caller]
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
        let now = agentmux_common::time::now_ms();
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
    #[track_caller]
    pub fn read_bytes_db(&self, zone_id: &str, name: &str, offset: i64, len: i64) -> Result<Vec<u8>, StoreError> {
        let conn = self.lock_conn();
        super::counter::read_bytes_exact(&conn, zone_id, name, offset, len)?.ok_or_else(|| {
            StoreError::Other(format!("{zone_id}/{name}: bytes {offset}..{} not stored yet", offset + len))
        })
    }

    /// Run `f` with a [`SnapshotReader`]: everything it reads comes from one
    /// database snapshot — for a decision and the reads it licenses (an
    /// index judged fresh, then its entries and the bytes they point at),
    /// which must not straddle another instance's replace (Codex on #3634).
    #[track_caller]
    pub fn read_snapshot<T>(&self, f: impl FnOnce(&SnapshotReader<'_>) -> Result<T, StoreError>) -> Result<T, StoreError> {
        self.read_txn(|tx| f(&SnapshotReader { tx }))
    }

    /// A file (`output`) and a file derived from it (`output.idx`), read in
    /// ONE snapshot (Codex on #3634): `output`'s size and valid generation,
    /// and the derived file's size, its first `head` bytes (all of it when
    /// `head` is `None`) and its metadata. `None` when `output` doesn't exist.
    #[track_caller]
    pub fn derived_snapshot(
        &self,
        zone_id: &str,
        output: &str,
        derived: &str,
        head: Option<i64>,
    ) -> Result<Option<DerivedSnapshot>, StoreError> {
        self.read_txn(|tx| {
            let Some(row) = super::counter::read_row(tx, zone_id, output)? else { return Ok(None) };
            let output_gen = row.counter().map(|(gen, _)| gen);
            let derived_row: Option<(i64, String)> = tx
                .query_row(
                    "SELECT size, meta FROM db_wave_file WHERE zoneid = ?1 AND name = ?2",
                    params![zone_id, derived],
                    |r| Ok((r.get(0)?, r.get(1)?)),
                )
                .optional()?;
            let derived = match derived_row {
                None => None,
                Some((size, meta)) => {
                    let len = head.map_or(size, |h| h.min(size));
                    Some(DerivedView {
                        size,
                        bytes: super::counter::read_bytes_exact(tx, zone_id, derived, 0, len)?,
                        meta: serde_json::from_str(&meta).unwrap_or_default(),
                    })
                }
            };
            Ok(Some(DerivedSnapshot { output_size: row.size, output_gen, derived }))
        })
    }

    /// A file's metadata, read from the database rather than the cache.
    #[track_caller]
    pub fn meta_db(&self, zone_id: &str, name: &str) -> Result<Option<FileMeta>, StoreError> {
        let conn = self.lock_conn();
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
