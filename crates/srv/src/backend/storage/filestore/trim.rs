// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A circular file's head, given back. `read_at`/`read_file` already treat a
//! `circular` file with a `maxsize` as holding only its last `maxsize` bytes
//! (offsets stay absolute), but appends only ever add parts: the rows below the
//! window stay in the database. A terminal's output is the one stream written
//! at line speed with nobody reading it back (`yes` stored 2.5 GB), so its
//! file is made circular and its dead head dropped here.

use rusqlite::params;

use super::core::{FileStore, PART_DATA_SIZE};
use super::types::FileOpts;
use crate::backend::storage::error::StoreError;

/// The first part that still holds bytes of a window of `maxsize` ending at
/// `size`: parts before it are dead.
pub(crate) fn first_live_part(size: i64, maxsize: i64) -> i32 {
    if maxsize <= 0 || size <= maxsize {
        return 0;
    }
    ((size - maxsize) / PART_DATA_SIZE as i64) as i32
}

impl FileStore {
    /// Make `name` circular with a window of `maxsize` bytes (0 or less lifts
    /// the limit), keeping what it holds. Returns whether the options changed.
    #[track_caller]
    pub fn set_circular(&self, zone_id: &str, name: &str, maxsize: i64) -> Result<bool, StoreError> {
        let key = (zone_id.to_string(), name.to_string());
        let changed = self.write_txn(|tx| {
            let opts_json: String = tx
                .query_row("SELECT opts FROM db_wave_file WHERE zoneid = ?1 AND name = ?2", params![zone_id, name], |r| r.get(0))
                .map_err(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => StoreError::NotFound,
                    other => other.into(),
                })?;
            let mut opts: FileOpts = serde_json::from_str(&opts_json).unwrap_or_default();
            let (circular, maxsize) = (maxsize > 0, maxsize.max(0));
            if opts.circular == circular && opts.maxsize == maxsize {
                return Ok(None);
            }
            opts.circular = circular;
            opts.maxsize = maxsize;
            tx.execute(
                "UPDATE db_wave_file SET opts = ?1 WHERE zoneid = ?2 AND name = ?3",
                params![serde_json::to_string(&opts)?, zone_id, name],
            )?;
            Ok(Some(opts))
        })?;
        let Some(opts) = changed else { return Ok(false) };
        if let Some(entry) = self.cache.lock().unwrap().get_mut(&key) {
            if let Some(file) = entry.file.as_mut() {
                file.opts = opts;
            }
        }
        Ok(true)
    }

    /// Copy what the log holds back into the database now, on the writer's
    /// own connection, between two of its writes. The background checkpointer
    /// can't catch up with a writer that never pauses (each pass ends behind the
    /// log's end), so the log is never found fully copied back and never
    /// restarts: a terminal flood grew it to 4.3 GB. A pass made by the writer
    /// itself, with no write in progress, leaves nothing behind, and its next
    /// write starts the log over (cut back to `journal_size_limit`).
    #[track_caller]
    pub fn checkpoint_between_writes(&self) -> Result<(), StoreError> {
        self.lock_conn().query_row("PRAGMA wal_checkpoint(PASSIVE)", [], |_| Ok(()))?;
        Ok(())
    }

    /// Delete the parts of a circular `name` that lie wholly below its window.
    /// Returns how many rows went. A file that isn't circular loses nothing.
    #[track_caller]
    pub fn trim_circular_head(&self, zone_id: &str, name: &str) -> Result<usize, StoreError> {
        let key = (zone_id.to_string(), name.to_string());
        let (first_live, removed) = self.write_txn(|tx| {
            let row: Option<(i64, String)> = tx
                .query_row("SELECT size, opts FROM db_wave_file WHERE zoneid = ?1 AND name = ?2", params![zone_id, name], |r| {
                    Ok((r.get(0)?, r.get(1)?))
                })
                .map(Some)
                .or_else(|e| match e {
                    rusqlite::Error::QueryReturnedNoRows => Ok(None),
                    other => Err(other),
                })?;
            let Some((size, opts_json)) = row else { return Ok((0, 0)) };
            let opts: FileOpts = serde_json::from_str(&opts_json).unwrap_or_default();
            if !opts.circular {
                return Ok((0, 0));
            }
            let first_live = first_live_part(size, opts.maxsize);
            if first_live == 0 {
                return Ok((0, 0));
            }
            let removed = tx.execute(
                "DELETE FROM db_file_data WHERE zoneid = ?1 AND name = ?2 AND partidx < ?3",
                params![zone_id, name, first_live],
            )?;
            // A frame wholly below the window goes too (a terminal's file is
            // never framed; a circular file that was must not keep its head).
            tx.execute(
                "DELETE FROM db_file_frame WHERE zoneid = ?1 AND name = ?2 AND first_part + part_count <= ?3",
                params![zone_id, name, first_live],
            )?;
            Ok((first_live, removed))
        })?;
        if first_live > 0 {
            if let Some(entry) = self.cache.lock().unwrap().get_mut(&key) {
                entry.data_entries.retain(|idx, _| *idx >= first_live);
            }
        }
        Ok(removed)
    }
}
