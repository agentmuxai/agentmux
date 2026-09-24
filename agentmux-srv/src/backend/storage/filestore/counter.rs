// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Per-file line counter and generation (Phase 5a-2,
//! SPEC_AGENT_PANE_BOUNDED_LIVE_WINDOW_MIGRATION_2026_09_23.md §6.3.7).
//!
//! A counted file's row carries `gen`, a random nonce, and `lines`, its line
//! count under the reader's rule (`lines.rs`), so `(gen, line)` addresses a
//! transcript record: the same line index never names different content
//! under the same `gen`.
//!
//! The five columns (`gen`, `lines`, `lines_size`, `lines_tail`,
//! `lines_modts`) are one **epoch**: all valid, or all NULL. An epoch starts
//! with a fresh `gen` whenever this code can vouch for the count from byte 0:
//! `make_file` (empty), `write_file` (counts the data) and
//! [`FileStore::init_line_counter`] (scans a file that has none). Appends
//! advance it in the same transaction as the write.
//!
//! Older builds share the global transcript store and write without
//! maintaining the epoch. What they can't avoid is the database's own
//! triggers (`run_filestore_migrations`): any write that changes bytes
//! already in a file — by any connection, any build — bumps `rev`, while
//! appends don't. An epoch records the `rev` it was counted at
//! (`lines_rev`) and the size it covers (`lines_size`), so an append by
//! someone else (size moved) or any rewrite (rev moved) makes it invalid.
//! It is then dropped (NULL), never patched, so the next epoch gets a new
//! `gen` and no line index is ever reused for different content. That costs
//! a reader a resync, only while builds are mixed. (`lines_modts` is kept as
//! an extra, conservative signal; timestamps alone can't be trusted — two
//! writes can share a millisecond, Codex on #3631.)

use rusqlite::{params, Connection, OptionalExtension};

use super::core::{FileStore, PART_DATA_SIZE};
use super::lines::{count_append, is_blank_line, LineCount};
use crate::backend::storage::error::StoreError;

/// A fresh generation nonce: 16 hex digits (64 random bits).
pub(super) fn new_gen() -> String {
    let mut s = uuid::Uuid::new_v4().simple().to_string();
    s.truncate(16);
    s
}

/// Where a counted file stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Counted {
    pub gen: String,
    /// Line count; the next complete line appended gets this index.
    pub lines: u64,
}

/// A file's size and, if it is counted, its epoch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineState {
    pub size: i64,
    pub counted: Option<Counted>,
}

/// Where an append landed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppendPos {
    /// Byte offset of the first appended byte.
    pub offset: i64,
    /// `None` when the file is not counted (see the module doc).
    pub counted: Option<CountedAppend>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CountedAppend {
    pub gen: String,
    /// Raw appends: index of the line the first appended byte belongs to
    /// (it may continue an unterminated line). [`FileStore::append_lines`]:
    /// index of the first appended record.
    pub first_line: u64,
    /// Line count after the append.
    pub lines: u64,
}

/// How [`FileStore::append_inner`] treats the data.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum AppendMode {
    /// Append the bytes as given.
    Raw,
    /// Complete, non-blank, `\n`-terminated lines only; a torn tail is closed
    /// first (see [`FileStore::append_lines`]).
    Lines,
}

/// A `db_wave_file` row's size and counter columns.
pub(super) struct Row {
    pub size: i64,
    pub modts: i64,
    /// Random per-row identity (insert trigger); differs across a delete +
    /// re-create even when every other column matches.
    incarnation: Option<Vec<u8>>,
    /// Bumped by every write that rewrites existing bytes (not appends).
    rev: i64,
    gen: Option<String>,
    lines: Option<i64>,
    lines_size: Option<i64>,
    lines_tail: Option<i64>,
    lines_modts: Option<i64>,
    lines_rev: Option<i64>,
}

impl Row {
    /// The epoch, if it is valid: present, and the file has not been written
    /// since by anything that doesn't maintain it.
    pub fn counter(&self) -> Option<(String, LineCount)> {
        let gen = self.gen.clone()?;
        let (lines, tail) = (self.lines?, self.lines_tail?);
        if self.lines_size != Some(self.size)
            || self.lines_rev != Some(self.rev)
            || self.lines_modts != Some(self.modts)
        {
            return None;
        }
        if lines < 0 || tail < 0 || tail > self.size {
            return None;
        }
        Some((gen, LineCount { lines: lines as u64, tail_start: tail as u64 }))
    }

    /// Whether any counter column is set — an epoch to drop when invalid.
    pub fn has_epoch_columns(&self) -> bool {
        self.gen.is_some()
            || self.lines.is_some()
            || self.lines_size.is_some()
            || self.lines_tail.is_some()
            || self.lines_modts.is_some()
            || self.lines_rev.is_some()
    }

    fn state(&self) -> LineState {
        LineState {
            size: self.size,
            counted: self.counter().map(|(gen, c)| Counted { gen, lines: c.lines }),
        }
    }
}

pub(super) fn read_row(conn: &Connection, zone_id: &str, name: &str) -> Result<Option<Row>, StoreError> {
    Ok(conn
        .query_row(
            "SELECT size, modts, gen, lines, lines_size, lines_tail, lines_modts, incarnation, rev, lines_rev
             FROM db_wave_file WHERE zoneid = ?1 AND name = ?2",
            params![zone_id, name],
            |r| {
                Ok(Row {
                    size: r.get(0)?,
                    modts: r.get(1)?,
                    gen: r.get(2)?,
                    lines: r.get(3)?,
                    lines_size: r.get(4)?,
                    lines_tail: r.get(5)?,
                    lines_modts: r.get(6)?,
                    incarnation: r.get(7)?,
                    rev: r.get::<_, Option<i64>>(8)?.unwrap_or(0),
                    lines_rev: r.get(9)?,
                })
            },
        )
        .optional()?)
}

/// SQL fragment clearing the epoch, for writes that can't maintain it.
pub(super) const DROP_EPOCH_SQL: &str =
    "gen = NULL, lines = NULL, lines_size = NULL, lines_tail = NULL, lines_modts = NULL, lines_rev = NULL";

/// Bytes `[offset, offset + len)` of a file, straight from its parts (never
/// the cache). A missing part reads as zeros, as `append_data_at` pads it.
pub(super) fn read_bytes(
    conn: &Connection,
    zone_id: &str,
    name: &str,
    offset: i64,
    len: i64,
) -> Result<Vec<u8>, StoreError> {
    Ok(read_bytes_covered(conn, zone_id, name, offset, len)?.0)
}

/// [`read_bytes`], but `None` if any byte of the range is not actually
/// stored — a part missing inside the size the file claims. That is a file
/// mid-write by a build without transactions (it raises `size` before
/// inserting parts, Codex on #3631); the counter must never count bytes that
/// read as zeros only because they aren't there yet.
pub(super) fn read_bytes_exact(
    conn: &Connection,
    zone_id: &str,
    name: &str,
    offset: i64,
    len: i64,
) -> Result<Option<Vec<u8>>, StoreError> {
    let (bytes, covered) = read_bytes_covered(conn, zone_id, name, offset, len)?;
    Ok((covered == len.max(0) as usize).then_some(bytes))
}

/// The bytes, and how many of them came from stored parts.
fn read_bytes_covered(
    conn: &Connection,
    zone_id: &str,
    name: &str,
    offset: i64,
    len: i64,
) -> Result<(Vec<u8>, usize), StoreError> {
    if len <= 0 {
        return Ok((Vec::new(), 0));
    }
    let pds = PART_DATA_SIZE as i64;
    let (first, last) = ((offset / pds) as i32, ((offset + len - 1) / pds) as i32);
    let mut out = vec![0u8; len as usize];
    let mut covered = 0usize;
    let mut stmt = conn.prepare_cached(
        "SELECT partidx, data FROM db_file_data
         WHERE zoneid = ?1 AND name = ?2 AND partidx BETWEEN ?3 AND ?4",
    )?;
    let rows = stmt.query_map(params![zone_id, name, first, last], |r| {
        Ok((r.get::<_, i32>(0)?, r.get::<_, Vec<u8>>(1)?))
    })?;
    for row in rows {
        let (idx, data) = row?;
        let part_start = idx as i64 * pds;
        // Overlap of this part's bytes with the requested range.
        let from = offset.max(part_start);
        let to = (offset + len).min(part_start + data.len() as i64);
        if from < to {
            out[(from - offset) as usize..(to - offset) as usize]
                .copy_from_slice(&data[(from - part_start) as usize..(to - part_start) as usize]);
            covered += (to - from) as usize;
        }
    }
    Ok((out, covered))
}

/// `data` as complete, non-blank lines, each ending in `\n`; prefixed with a
/// `\n` when `close_tail`, so it can't continue a torn last line.
fn normalize_lines(data: &[u8], close_tail: bool) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len() + 2);
    for line in data.split(|&b| b == b'\n') {
        if !is_blank_line(line) {
            out.extend_from_slice(line);
            out.push(b'\n');
        }
    }
    if close_tail && !out.is_empty() {
        out.insert(0, b'\n');
    }
    out
}

/// How much of a scan's tail [`FileStore::init_line_counter`] re-checks
/// before trusting it.
const INIT_FINGERPRINT_BYTES: i64 = 64;
/// Scan window for [`FileStore::init_line_counter`]; the connection lock is
/// held for one window at a time.
const INIT_SCAN_WINDOW: i64 = 1 << 20;

impl FileStore {
    /// The append behind `append_data_at` and [`Self::append_lines`]: writes
    /// the parts, the size and the counter in one transaction.
    pub(super) fn append_inner(
        &self,
        zone_id: &str,
        name: &str,
        data: &[u8],
        mode: AppendMode,
        now: i64,
    ) -> Result<(AppendPos, i64), StoreError> {
        self.write_txn(|tx| {
            let row = read_row(tx, zone_id, name)?.ok_or(StoreError::NotFound)?;
            let size = row.size;
            let counter = row.counter();

            let normalized;
            let data: &[u8] = match mode {
                AppendMode::Raw => data,
                AppendMode::Lines => {
                    let torn = size > 0 && read_bytes(tx, zone_id, name, size - 1, 1)?[0] != b'\n';
                    normalized = normalize_lines(data, torn);
                    &normalized
                }
            };
            if data.is_empty() {
                let counted = counter.map(|(gen, c)| CountedAppend { gen, first_line: c.lines, lines: c.lines });
                return Ok((AppendPos { offset: size, counted }, size));
            }

            // The counter needs the unterminated last line before the append
            // (normally empty: transcript lines end in `\n`).
            // If those bytes aren't all stored (a build without transactions
            // is mid-write), the count can't be advanced: the epoch is
            // dropped below, like any write it can't vouch for.
            let counted_before = match &counter {
                Some((gen, c)) => {
                    read_bytes_exact(tx, zone_id, name, c.tail_start as i64, size - c.tail_start as i64)?
                        .map(|open| (gen.clone(), *c, open))
                }
                None => None,
            };

            let new_size = size + data.len() as i64;
            let start_part = (size / PART_DATA_SIZE as i64) as i32;
            let offset_in_part = (size % PART_DATA_SIZE as i64) as usize;
            let mut data_offset = 0usize;
            let mut current_part = start_part;
            if offset_in_part > 0 {
                // Keep only the bytes the size covers: anything past it (a
                // torn append by a build without transactions) is not content.
                let mut part: Vec<u8> = tx
                    .query_row(
                        "SELECT data FROM db_file_data WHERE zoneid = ?1 AND name = ?2 AND partidx = ?3",
                        params![zone_id, name, start_part],
                        |r| r.get(0),
                    )
                    .optional()?
                    .unwrap_or_default();
                part.resize(offset_in_part, 0);
                let to_copy = (PART_DATA_SIZE - offset_in_part).min(data.len());
                part.extend_from_slice(&data[..to_copy]);
                data_offset = to_copy;
                tx.execute(
                    "REPLACE INTO db_file_data (zoneid, name, partidx, data) VALUES (?1, ?2, ?3, ?4)",
                    params![zone_id, name, current_part, part],
                )?;
                current_part += 1;
            }
            while data_offset < data.len() {
                let end = (data_offset + PART_DATA_SIZE).min(data.len());
                tx.execute(
                    "REPLACE INTO db_file_data (zoneid, name, partidx, data) VALUES (?1, ?2, ?3, ?4)",
                    params![zone_id, name, current_part, &data[data_offset..end]],
                )?;
                data_offset = end;
                current_part += 1;
            }

            let counted = match counted_before {
                Some((gen, before, open)) => {
                    let after = count_append(before, &open, size as u64, data);
                    let open_counted = !open.is_empty() && !is_blank_line(&open);
                    tx.execute(
                        "UPDATE db_wave_file SET size = ?1, modts = ?2,
                             lines = ?3, lines_size = ?1, lines_tail = ?4, lines_modts = ?2,
                             lines_rev = COALESCE(rev, 0)
                         WHERE zoneid = ?5 AND name = ?6",
                        params![new_size, now, after.lines as i64, after.tail_start as i64, zone_id, name],
                    )?;
                    // Raw: the first byte may continue the open line. Lines:
                    // any open line was closed first (it keeps its index, or
                    // was blank and has none), so the first record gets the
                    // next index.
                    let first_line = match mode {
                        AppendMode::Raw => before.lines - u64::from(open_counted),
                        AppendMode::Lines => before.lines,
                    };
                    Some(CountedAppend { gen, first_line, lines: after.lines })
                }
                None => {
                    if row.has_epoch_columns() {
                        tracing::warn!(
                            zone = %zone_id, name = %name,
                            "line counter dropped: the file was written, or is mid-write, by a build that doesn't maintain it"
                        );
                        tx.execute(
                            &format!(
                                "UPDATE db_wave_file SET size = ?1, modts = ?2, {DROP_EPOCH_SQL}
                                 WHERE zoneid = ?3 AND name = ?4"
                            ),
                            params![new_size, now, zone_id, name],
                        )?;
                    } else {
                        tx.execute(
                            "UPDATE db_wave_file SET size = ?1, modts = ?2 WHERE zoneid = ?3 AND name = ?4",
                            params![new_size, now, zone_id, name],
                        )?;
                    }
                    None
                }
            };
            Ok((AppendPos { offset: size, counted }, new_size))
        })
    }

    /// Append transcript lines. `data` is normalized to complete, non-blank,
    /// `\n`-terminated lines (the reader's rule), and if the file ends
    /// mid-line (a torn record) a `\n` closes it first: the torn line keeps
    /// its index instead of fusing with the new record, which would make both
    /// unparseable. On a counted file, `first_line` is the first appended
    /// line's index and `lines - first_line` how many were appended.
    #[allow(dead_code)] // wired into the transcript writers in 5a-3
    pub fn append_lines(&self, zone_id: &str, name: &str, data: &[u8]) -> Result<AppendPos, StoreError> {
        let now = Self::now_ms();
        let (pos, new_size) = self.append_inner(zone_id, name, data, AppendMode::Lines, now)?;
        // Blank-only input normalizes to nothing: no write, no cache update.
        if new_size > pos.offset {
            self.note_appended(zone_id, name, new_size, now);
        }
        Ok(pos)
    }

    /// A file's size and epoch, from the database (never the cache).
    #[allow(dead_code)] // read by the transcript RPCs in 5a-3
    pub fn line_state(&self, zone_id: &str, name: &str) -> Result<Option<LineState>, StoreError> {
        let conn = self.conn.lock().unwrap();
        Ok(read_row(&conn, zone_id, name)?.map(|r| r.state()))
    }

    /// Start an epoch for a file that has none (written before this code, or
    /// by an older build): count it from byte 0 and mint a fresh `gen`.
    ///
    /// The scan takes the connection lock one window at a time, so appends
    /// continue meanwhile; the final transaction counts what they added. It
    /// gives up (`Ok(None)`, the file stays uncounted) if the scanned bytes
    /// may have changed underneath it, and returns the existing epoch if
    /// another writer started one first. Cost: one read of the file, once per
    /// epoch.
    #[allow(dead_code)] // called by the transcript RPCs in 5a-3
    pub fn init_line_counter(&self, zone_id: &str, name: &str) -> Result<Option<LineState>, StoreError> {
        match self.init_scan(zone_id, name)? {
            InitScan::Done(state) => Ok(state),
            InitScan::Scanned(scan) => self.init_finish(zone_id, name, scan),
        }
    }

    /// Phase 1 of [`Self::init_line_counter`]: the windowed scan.
    pub(super) fn init_scan(&self, zone_id: &str, name: &str) -> Result<InitScan, StoreError> {
        let row = {
            let conn = self.conn.lock().unwrap();
            let Some(row) = read_row(&conn, zone_id, name)? else { return Ok(InitScan::Done(None)) };
            if row.counter().is_some() {
                return Ok(InitScan::Done(Some(row.state())));
            }
            row
        };
        let scan_to = row.size;
        let mut count = LineCount { lines: 0, tail_start: 0 };
        let mut open: Vec<u8> = Vec::new();
        let mut pos = 0i64;
        while pos < scan_to {
            let len = INIT_SCAN_WINDOW.min(scan_to - pos);
            let chunk = {
                let conn = self.conn.lock().unwrap();
                read_bytes_exact(&conn, zone_id, name, pos, len)?
            };
            // Bytes the file claims but doesn't store yet: not countable.
            let Some(chunk) = chunk else { return Ok(InitScan::Done(None)) };
            count = count_append(count, &open, pos as u64, &chunk);
            match chunk.iter().rposition(|&b| b == b'\n') {
                Some(nl) => open = chunk[nl + 1..].to_vec(),
                None => open.extend_from_slice(&chunk),
            }
            pos += len;
        }
        let fp_len = INIT_FINGERPRINT_BYTES.min(scan_to);
        let fingerprint = {
            let conn = self.conn.lock().unwrap();
            read_bytes_exact(&conn, zone_id, name, scan_to - fp_len, fp_len)?
        };
        let Some(fingerprint) = fingerprint else { return Ok(InitScan::Done(None)) };
        Ok(InitScan::Scanned(ScanResult {
            scan_to,
            count,
            open,
            fingerprint,
            incarnation: row.incarnation,
            rev: row.rev,
        }))
    }

    /// Phase 2 of [`Self::init_line_counter`]: validate the scan against the
    /// row as it is now, count what was appended since, and start the epoch,
    /// all in one transaction.
    pub(super) fn init_finish(&self, zone_id: &str, name: &str, scan: ScanResult) -> Result<Option<LineState>, StoreError> {
        let ScanResult { scan_to, count, open, fingerprint, incarnation, rev } = scan;
        let fp_len = fingerprint.len() as i64;
        self.write_txn(|tx| {
            let Some(row) = read_row(tx, zone_id, name)? else { return Ok(None) };
            if row.counter().is_some() {
                return Ok(Some(row.state()));
            }
            // The scanned bytes must still be the file's bytes: the same row
            // (its random incarnation — not deleted and re-created, even
            // within one millisecond, Codex on #3631) and nothing rewritten since the
            // scan began — `rev` is bumped by the database's triggers for any
            // writer, older builds included (review of #3631). Appends don't
            // bump it, and are counted below. The size and tail checks are
            // redundant with `rev`, kept as a second line of defence.
            if row.incarnation != incarnation
                || row.rev != rev
                || row.size < scan_to
                || read_bytes_exact(tx, zone_id, name, scan_to - fp_len, fp_len)?.as_ref() != Some(&fingerprint)
            {
                return Ok(None);
            }
            let Some(appended) = read_bytes_exact(tx, zone_id, name, scan_to, row.size - scan_to)? else {
                return Ok(None);
            };
            let count = count_append(count, &open, scan_to as u64, &appended);
            let gen = new_gen();
            tx.execute(
                "UPDATE db_wave_file SET gen = ?1, lines = ?2, lines_size = ?3, lines_tail = ?4, lines_modts = ?5,
                     lines_rev = COALESCE(rev, 0)
                 WHERE zoneid = ?6 AND name = ?7",
                params![gen, count.lines as i64, row.size, count.tail_start as i64, row.modts, zone_id, name],
            )?;
            Ok(Some(LineState { size: row.size, counted: Some(Counted { gen, lines: count.lines }) }))
        })
    }
}

/// Outcome of [`FileStore::init_scan`].
pub(super) enum InitScan {
    /// Nothing to scan: the file is missing, or already counted.
    Done(Option<LineState>),
    Scanned(ScanResult),
}

/// A finished scan, and the row identity it was taken against.
pub(super) struct ScanResult {
    scan_to: i64,
    count: LineCount,
    open: Vec<u8>,
    fingerprint: Vec<u8>,
    incarnation: Option<Vec<u8>>,
    rev: i64,
}
