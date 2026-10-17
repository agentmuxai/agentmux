// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Copying files from one zone to another inside the database: a fork's
//! transcript starts as an exact copy of its parent's
//! (`agent_session::fork_history`), and a long transcript (gigabytes) must
//! not pass through memory on the way.

use rusqlite::{params, OptionalExtension};

use super::core::FileStore;
use super::counter::new_gen;
use crate::backend::storage::error::StoreError;

/// A source file row, as far as a copy needs it.
struct SourceRow {
    size: i64,
    opts: String,
    meta: String,
    lines: Option<i64>,
    lines_size: Option<i64>,
    lines_tail: Option<i64>,
    rev: Option<i64>,
    lines_rev: Option<i64>,
}

impl FileStore {
    /// Copy `names` from `src_zone` into `dst_zone`, part rows and all, in
    /// one transaction: a reader sees all of them or none, and an append to
    /// the source after the copy started lands after it, never inside.
    ///
    /// Copies nothing and returns `Ok(None)` when `dst_zone` already holds
    /// any of `names`, so it can't overwrite history. A name the source
    /// lacks is skipped. Returns the bytes copied otherwise.
    ///
    /// Reads the database, not this process's cache: transcript appends are
    /// written straight to it (`append_lines`). A file written through the
    /// cache (`write_file`) may be behind; copy those with `read_file`.
    ///
    /// Each copy is a new file: a fresh `gen`, and its own incarnation (set
    /// by the insert trigger). Its line count is carried over when the
    /// source's count is valid for its bytes; otherwise the copy starts
    /// uncounted at zero and the counter catches up (counter.rs).
    #[track_caller]
    pub fn copy_files_to_zone(&self, src_zone: &str, dst_zone: &str, names: &[&str]) -> Result<Option<u64>, StoreError> {
        let now = agentmux_common::time::now_ms();
        let copied = self.write_txn(|tx| {
            for name in names {
                let exists = tx
                    .query_row(
                        "SELECT 1 FROM db_wave_file WHERE zoneid = ?1 AND name = ?2",
                        params![dst_zone, name],
                        |_| Ok(()),
                    )
                    .optional()?
                    .is_some();
                if exists {
                    return Ok(None);
                }
            }
            let mut copied = 0u64;
            for name in names {
                let src = tx
                    .query_row(
                        "SELECT size, opts, meta, lines, lines_size, lines_tail, rev, lines_rev
                         FROM db_wave_file WHERE zoneid = ?1 AND name = ?2",
                        params![src_zone, name],
                        |r| {
                            Ok(SourceRow {
                                size: r.get(0)?,
                                opts: r.get(1)?,
                                meta: r.get(2)?,
                                lines: r.get(3)?,
                                lines_size: r.get(4)?,
                                lines_tail: r.get(5)?,
                                rev: r.get(6)?,
                                lines_rev: r.get(7)?,
                            })
                        },
                    )
                    .optional()?;
                let Some(src) = src else { continue };
                // Parts before the file row: the revision trigger counts a
                // part inserted inside a range the row already claims as a
                // rewrite (migrations.rs), and a copy is not one.
                tx.execute(
                    "INSERT INTO db_file_data (zoneid, name, partidx, data)
                     SELECT ?1, name, partidx, data FROM db_file_data WHERE zoneid = ?2 AND name = ?3",
                    params![dst_zone, src_zone, name],
                )?;
                let counted = src.lines.is_some()
                    && src.lines_size.is_some()
                    && src.rev.unwrap_or(0) == src.lines_rev.unwrap_or(0);
                // The count is stamped with the copy's own `modts`: a count is
                // valid only while the two match (counter.rs, `Row::counter`).
                let (lines, lines_size, lines_tail) = if counted {
                    (src.lines, src.lines_size, src.lines_tail.or(Some(0)))
                } else {
                    (Some(0), Some(0), Some(0))
                };
                tx.execute(
                    "INSERT INTO db_wave_file (zoneid, name, size, createdts, modts, opts, meta,
                         gen, lines, lines_size, lines_tail, lines_modts, rev, lines_rev)
                     VALUES (?1, ?2, ?3, ?4, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?4, 0, 0)",
                    params![dst_zone, name, src.size, now, src.opts, src.meta, new_gen(), lines, lines_size, lines_tail],
                )?;
                copied += src.size.max(0) as u64;
            }
            Ok(Some(copied))
        })?;
        // Rows this process may have cached for the destination are stale.
        self.forget_cached(dst_zone, names);
        Ok(copied)
    }
}
