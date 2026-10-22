// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Compressed frames of a file's parts
//! (SPEC_HISTORY_STORE_COMPRESSION_AND_COMPACTION_2026_10_21.md §4).
//!
//! A frame holds up to [`FRAME_PARTS`] consecutive whole 64 KiB parts of one
//! file, compressed together, in `db_file_frame`. A part a frame covers has no
//! row in `db_file_data`. Readers take a part's row when it has one and fill
//! the parts without a row from frames, so **a part row always wins over a
//! frame**: a writer that knows nothing of frames leaves correct data behind.
//!
//! Frames change how bytes are stored, never which bytes a file holds: byte
//! offsets, line numbers and the line counter stay what they were. Framing
//! and unframing put the file's `rev` back after their swap (the triggers on
//! `db_file_data` bump it for any deleted or filled-in part), so the counter
//! doesn't recount a file whose bytes didn't change.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex};

use rusqlite::{params, Connection, OptionalExtension};
use sha2::{Digest, Sha256};

use super::core::{FileStore, PART_DATA_SIZE};
use crate::backend::storage::error::StoreError;

/// Parts in a full frame: 1 MiB, where compression levels off on real
/// transcripts (spec §2).
pub(crate) const FRAME_PARTS: i32 = 16;
/// The most parts any frame may hold. Reads look this far back for a frame
/// that started before the range they want.
const MAX_FRAME_PARTS: i32 = 64;
/// Codec 1: the frame's bytes, compressed with zstd.
pub(crate) const CODEC_ZSTD: i64 = 1;
/// Codec 2: the frame with its streaming deltas stored as uuid and length,
/// each block's text once (deltas.rs), compressed with zstd.
pub(crate) const CODEC_DELTAS: i64 = 2;
/// The most a codec 2 payload may exceed its frame by (its string and block
/// tables): a bound on decompression.
const DELTAS_SLACK: usize = 64 * 1024;
const ZSTD_LEVEL: i32 = 3;

const PDS: usize = PART_DATA_SIZE;

/// Create the frame table. Every filestore gets it (it is empty unless a
/// framer writes to it); a build that knows nothing of frames ignores it.
pub(super) fn ensure_schema(conn: &Connection) -> Result<(), StoreError> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS db_file_frame (
             zoneid      TEXT    NOT NULL,
             name        TEXT    NOT NULL,
             first_part  INTEGER NOT NULL,
             part_count  INTEGER NOT NULL,
             codec       INTEGER NOT NULL,
             raw_len     INTEGER NOT NULL,
             check_hash  BLOB    NOT NULL,
             data        BLOB    NOT NULL,
             PRIMARY KEY (zoneid, name, first_part)
         )",
    )?;
    Ok(())
}

/// One frame row.
#[derive(Debug, Clone)]
pub(crate) struct Frame {
    pub first_part: i32,
    pub part_count: i32,
    pub codec: i64,
    pub raw_len: i64,
    pub check_hash: Vec<u8>,
    pub data: Vec<u8>,
}

impl Frame {
    fn last_part(&self) -> i32 {
        self.first_part + self.part_count - 1
    }
}

/// What [`FileStore::frame_parts`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum FrameOutcome {
    /// The parts are a frame now: `raw` logical bytes stored as `stored`.
    Framed { raw: usize, stored: usize, codec: i64 },
    /// Nothing changed, and why.
    Skipped(&'static str),
}

/// The error a frame that fails to decode, or decodes to the wrong bytes,
/// raises: SQLite's own "damaged database", so every reader's damage handling
/// (`parts.rs`, `transcript::damage`) treats a bad frame as it treats a bad row.
fn corrupt(what: String) -> StoreError {
    StoreError::Sqlite(rusqlite::Error::SqliteFailure(
        rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CORRUPT),
        Some(what),
    ))
}

fn sha256(bytes: &[u8]) -> Vec<u8> {
    Sha256::digest(bytes).to_vec()
}

fn zstd_compress(bytes: &[u8]) -> Result<Vec<u8>, StoreError> {
    zstd::bulk::compress(bytes, ZSTD_LEVEL).map_err(|e| StoreError::Other(format!("frame compress: {e}")))
}

/// Encode a frame's logical bytes with `codec`. Codec 2 falls back to codec
/// 1 when the frame has no delta line to leave out (deltas.rs). Returns the
/// codec used and the stored bytes.
pub(crate) fn encode(codec: i64, raw: &[u8]) -> Result<(i64, Vec<u8>), StoreError> {
    match codec {
        CODEC_ZSTD => Ok((CODEC_ZSTD, zstd_compress(raw)?)),
        CODEC_DELTAS => match super::deltas::encode(raw) {
            Some((payload, _)) => Ok((CODEC_DELTAS, zstd_compress(&payload)?)),
            None => Ok((CODEC_ZSTD, zstd_compress(raw)?)),
        },
        other => Err(StoreError::Other(format!("frame codec {other} can't encode"))),
    }
}

/// Decode a frame and check its bytes against its hash, without the cache.
pub(crate) fn decode_uncached(frame: &Frame) -> Result<Vec<u8>, StoreError> {
    let expect = frame.part_count as i64 * PDS as i64;
    if frame.part_count < 1 || frame.part_count > MAX_FRAME_PARTS || frame.raw_len != expect {
        return Err(corrupt(format!("frame at part {} has a bad shape", frame.first_part)));
    }
    let raw = match frame.codec {
        CODEC_ZSTD => zstd::bulk::decompress(&frame.data, frame.raw_len as usize)
            .map_err(|e| corrupt(format!("frame at part {}: {e}", frame.first_part)))?,
        CODEC_DELTAS => {
            let payload = zstd::bulk::decompress(&frame.data, frame.raw_len as usize + DELTAS_SLACK)
                .map_err(|e| corrupt(format!("frame at part {}: {e}", frame.first_part)))?;
            super::deltas::decode(&payload, frame.raw_len as usize)
                .map_err(|e| corrupt(format!("frame at part {}: {e}", frame.first_part)))?
        }
        other => return Err(corrupt(format!("frame at part {}: unknown codec {other}", frame.first_part))),
    };
    if raw.len() as i64 != frame.raw_len || sha256(&raw) != frame.check_hash {
        return Err(corrupt(format!("frame at part {}: its bytes don't match its hash", frame.first_part)));
    }
    Ok(raw)
}

/// The last frames decoded: a History page read touches one or two frames,
/// and paging through a conversation touches each a few times. A hit needs
/// the same hash and the same stored bytes, so a frame damaged after it was
/// read is still found damaged.
type Cached = (Vec<u8>, Vec<u8>, Arc<Vec<u8>>);
static CACHE: LazyLock<Mutex<Vec<Cached>>> = LazyLock::new(Default::default);
const CACHE_FRAMES: usize = 12;

/// Decode a frame (from the cache when it was decoded lately).
pub(crate) fn decode(frame: &Frame) -> Result<Arc<Vec<u8>>, StoreError> {
    {
        let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(i) = cache.iter().position(|(hash, data, _)| *hash == frame.check_hash && *data == frame.data) {
            let hit = cache.remove(i);
            let bytes = hit.2.clone();
            cache.push(hit);
            return Ok(bytes);
        }
    }
    let bytes = Arc::new(decode_uncached(frame)?);
    let mut cache = CACHE.lock().unwrap_or_else(|e| e.into_inner());
    if cache.len() >= CACHE_FRAMES {
        cache.remove(0);
    }
    cache.push((frame.check_hash.clone(), frame.data.clone(), bytes.clone()));
    Ok(bytes)
}

fn frame_from_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<Frame> {
    Ok(Frame {
        first_part: r.get(0)?,
        part_count: r.get(1)?,
        codec: r.get(2)?,
        raw_len: r.get(3)?,
        check_hash: r.get(4)?,
        data: r.get(5)?,
    })
}

/// A store opened read-only skips its migrations, so an old one may have no
/// frame table: it has no frames.
fn no_frame_table(e: &rusqlite::Error) -> bool {
    matches!(e, rusqlite::Error::SqliteFailure(_, Some(msg)) if msg.contains("no such table: db_file_frame"))
}

/// The frames of a file that hold any of parts `start..=end`, in order.
pub(crate) fn frames_overlapping(conn: &Connection, zone_id: &str, name: &str, start: i32, end: i32) -> Result<Vec<Frame>, StoreError> {
    let stmt = conn.prepare_cached(
        "SELECT first_part, part_count, codec, raw_len, check_hash, data FROM db_file_frame
         WHERE zoneid = ?1 AND name = ?2 AND first_part <= ?4 AND first_part > ?3 - ?5
           AND first_part + part_count > ?3
         ORDER BY first_part",
    );
    let mut stmt = match stmt {
        Err(e) if no_frame_table(&e) => return Ok(Vec::new()),
        other => other?,
    };
    let rows = stmt.query_map(params![zone_id, name, start, end, MAX_FRAME_PARTS], frame_from_row)?;
    Ok(rows.collect::<Result<_, _>>()?)
}

/// Whether any frame holds a part of `start..=end` (no blob is read).
fn any_frame(conn: &Connection, zone_id: &str, name: &str, start: i32, end: i32) -> Result<bool, StoreError> {
    let stmt = conn.prepare_cached(
        "SELECT 1 FROM db_file_frame
         WHERE zoneid = ?1 AND name = ?2 AND first_part <= ?4 AND first_part > ?3 - ?5
           AND first_part + part_count > ?3
         LIMIT 1",
    );
    let mut stmt = match stmt {
        Err(e) if no_frame_table(&e) => return Ok(false),
        other => other?,
    };
    Ok(stmt.exists(params![zone_id, name, start, end, MAX_FRAME_PARTS])?)
}

/// Run `f`, a read that queries part rows and then frames, in one snapshot.
/// Between the two queries another connection may unframe (rows back, frame
/// gone) or frame (rows gone, frame in): read as separate statements, a part
/// could then be in neither place. A connection already in a transaction (a
/// snapshot reader, a write) has its snapshot; a pooled reader in autocommit
/// gets one here. `f` must only read.
pub(super) fn in_snapshot<T>(conn: &Connection, f: impl FnOnce() -> Result<T, StoreError>) -> Result<T, StoreError> {
    if !conn.is_autocommit() {
        return f();
    }
    conn.execute_batch("BEGIN")?;
    let out = f();
    // Only reads ran, so ending with ROLLBACK is the same as COMMIT, and it
    // can't fail where COMMIT does: SQLite refuses to commit a transaction
    // that met a damaged page, which a read past damage does on purpose.
    if let Err(e) = conn.execute_batch("ROLLBACK") {
        tracing::warn!(error = %e, "filestore: ending a read snapshot failed");
    }
    out
}

/// Fill the parts of `start..=end` that `parts` doesn't hold from frames.
/// `tolerant`: a frame that fails to decode fills its parts with spaces (the
/// damaged-part rule of `parts.rs`) instead of failing the read.
pub(super) fn fill_parts(
    conn: &Connection,
    zone_id: &str,
    name: &str,
    start: i32,
    end: i32,
    parts: &mut HashMap<i32, Vec<u8>>,
    tolerant: bool,
) -> Result<(), StoreError> {
    if parts.len() >= (end - start + 1).max(0) as usize {
        return Ok(());
    }
    for frame in frames_overlapping(conn, zone_id, name, start, end)? {
        let bytes = match decode(&frame) {
            Ok(bytes) => Some(bytes),
            Err(StoreError::Sqlite(e)) if tolerant && super::parts::is_corrupt(&e) => {
                tracing::warn!(zone = zone_id, file = name, first_part = frame.first_part, error = %e, "filestore: a damaged frame reads as blank");
                None
            }
            Err(e) => return Err(e),
        };
        for idx in start.max(frame.first_part)..=end.min(frame.last_part()) {
            if parts.contains_key(&idx) {
                continue;
            }
            let part = match &bytes {
                Some(b) => {
                    let off = (idx - frame.first_part) as usize * PDS;
                    b[off..off + PDS].to_vec()
                }
                None => vec![b' '; PDS],
            };
            parts.insert(idx, part);
        }
    }
    Ok(())
}

/// For a byte-range read: copy into `out` (the bytes `[offset, offset +
/// out.len())` of the file) the parts of `first..=last` that `seen` doesn't
/// hold, from frames, and return how many bytes that covered. A frame that
/// fails to decode is an error (the caller's damage path takes over).
pub(super) fn fill_range(
    conn: &Connection,
    zone_id: &str,
    name: &str,
    offset: i64,
    out: &mut [u8],
    first: i32,
    last: i32,
    seen: &HashSet<i32>,
) -> Result<usize, StoreError> {
    if seen.len() >= (last - first + 1).max(0) as usize {
        return Ok(0);
    }
    let len = out.len() as i64;
    let mut covered = 0usize;
    for frame in frames_overlapping(conn, zone_id, name, first, last)? {
        let bytes = decode(&frame)?;
        for idx in first.max(frame.first_part)..=last.min(frame.last_part()) {
            if seen.contains(&idx) {
                continue;
            }
            let part_start = idx as i64 * PDS as i64;
            let from = offset.max(part_start);
            let to = (offset + len).min(part_start + PDS as i64);
            if from < to {
                let src = (idx - frame.first_part) as i64 * PDS as i64 + (from - part_start);
                out[(from - offset) as usize..(to - offset) as usize]
                    .copy_from_slice(&bytes[src as usize..(src + (to - from)) as usize]);
                covered += (to - from) as usize;
            }
        }
    }
    Ok(covered)
}

/// One part's bytes, from its row or its frame; `None` when it has neither.
pub(super) fn read_part(conn: &Connection, zone_id: &str, name: &str, idx: i32) -> Result<Option<Vec<u8>>, StoreError> {
    let row: Option<Vec<u8>> = conn
        .query_row(
            "SELECT data FROM db_file_data WHERE zoneid = ?1 AND name = ?2 AND partidx = ?3",
            params![zone_id, name, idx],
            |r| r.get(0),
        )
        .optional()?;
    if row.is_some() {
        return Ok(row);
    }
    let mut parts = HashMap::new();
    fill_parts(conn, zone_id, name, idx, idx, &mut parts, false)?;
    Ok(parts.remove(&idx))
}

fn read_rev(conn: &Connection, zone_id: &str, name: &str) -> Result<Option<i64>, StoreError> {
    Ok(conn
        .query_row(
            "SELECT rev FROM db_wave_file WHERE zoneid = ?1 AND name = ?2",
            params![zone_id, name],
            |r| r.get::<_, Option<i64>>(0),
        )
        .optional()?
        .flatten())
}

fn put_rev(conn: &Connection, zone_id: &str, name: &str, rev: Option<i64>) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE db_wave_file SET rev = ?1 WHERE zoneid = ?2 AND name = ?3",
        params![rev, zone_id, name],
    )?;
    Ok(())
}

/// Turn the frames holding any of parts `start..=end` back into part rows
/// (a part that already has a row keeps it: the row is newer) and delete
/// them. The file's bytes don't change, so its `rev` is put back. Called by
/// every write into a file's middle before it writes.
pub(super) fn unframe(conn: &Connection, zone_id: &str, name: &str, start: i32, end: i32) -> Result<usize, StoreError> {
    if !any_frame(conn, zone_id, name, start, end)? {
        return Ok(0);
    }
    let rev = read_rev(conn, zone_id, name)?;
    let frames = frames_overlapping(conn, zone_id, name, start, end)?;
    for frame in &frames {
        let bytes = decode(frame)?;
        for idx in frame.first_part..=frame.last_part() {
            let off = (idx - frame.first_part) as usize * PDS;
            conn.execute(
                "INSERT OR IGNORE INTO db_file_data (zoneid, name, partidx, data) VALUES (?1, ?2, ?3, ?4)",
                params![zone_id, name, idx, &bytes[off..off + PDS]],
            )?;
        }
        conn.execute(
            "DELETE FROM db_file_frame WHERE zoneid = ?1 AND name = ?2 AND first_part = ?3",
            params![zone_id, name, frame.first_part],
        )?;
    }
    put_rev(conn, zone_id, name, rev)?;
    Ok(frames.len())
}

/// Delete a file's frames (`name` given) or a whole zone's.
pub(super) fn delete_frames(conn: &Connection, zone_id: &str, name: Option<&str>) -> Result<(), StoreError> {
    match name {
        Some(name) => conn.execute("DELETE FROM db_file_frame WHERE zoneid = ?1 AND name = ?2", params![zone_id, name])?,
        None => conn.execute("DELETE FROM db_file_frame WHERE zoneid = ?1", params![zone_id])?,
    };
    Ok(())
}

/// Before a truncate to `new_size`: frames wholly past the new end go, and a
/// frame holding the new last part is turned back into rows, so the truncate
/// can cut that part and a later append finds it as a row.
pub(super) fn truncate_frames(conn: &Connection, zone_id: &str, name: &str, new_size: i64) -> Result<(), StoreError> {
    let keep_parts = ((new_size + PDS as i64 - 1) / PDS as i64) as i32;
    conn.execute(
        "DELETE FROM db_file_frame WHERE zoneid = ?1 AND name = ?2 AND first_part >= ?3",
        params![zone_id, name, keep_parts],
    )?;
    if keep_parts > 0 {
        unframe(conn, zone_id, name, keep_parts - 1, i32::MAX - MAX_FRAME_PARTS)?;
    }
    Ok(())
}

/// Copy a file's frames to another zone (the part rows are copied by the caller).
pub(super) fn copy_frames(conn: &Connection, src_zone: &str, dst_zone: &str, name: &str) -> Result<(), StoreError> {
    conn.execute(
        "INSERT INTO db_file_frame (zoneid, name, first_part, part_count, codec, raw_len, check_hash, data)
         SELECT ?1, name, first_part, part_count, codec, raw_len, check_hash, data FROM db_file_frame
         WHERE zoneid = ?2 AND name = ?3",
        params![dst_zone, src_zone, name],
    )?;
    Ok(())
}

/// For a copy of the first `whole` full parts of a file: copy the frames
/// wholly inside them, and write the parts of a frame that straddles `whole`
/// as rows in `dst_zone`.
pub(super) fn copy_frames_below(conn: &Connection, src_zone: &str, dst_zone: &str, name: &str, whole: i32) -> Result<(), StoreError> {
    if whole <= 0 {
        return Ok(());
    }
    conn.execute(
        "INSERT INTO db_file_frame (zoneid, name, first_part, part_count, codec, raw_len, check_hash, data)
         SELECT ?1, name, first_part, part_count, codec, raw_len, check_hash, data FROM db_file_frame
         WHERE zoneid = ?2 AND name = ?3 AND first_part + part_count <= ?4",
        params![dst_zone, src_zone, name, whole],
    )?;
    for frame in frames_overlapping(conn, src_zone, name, whole - 1, whole - 1)? {
        if frame.last_part() < whole {
            continue; // wholly inside: copied above
        }
        let bytes = decode(&frame)?;
        for idx in frame.first_part..whole {
            let off = (idx - frame.first_part) as usize * PDS;
            conn.execute(
                "INSERT OR IGNORE INTO db_file_data (zoneid, name, partidx, data) VALUES (?1, ?2, ?3, ?4)",
                params![dst_zone, name, idx, &bytes[off..off + PDS]],
            )?;
        }
    }
    Ok(())
}

/// Move a file's frames to another zone, with its rows.
pub(super) fn move_frames(conn: &Connection, src_zone: &str, dst_zone: &str, name: &str) -> Result<(), StoreError> {
    conn.execute(
        "UPDATE db_file_frame SET zoneid = ?1 WHERE zoneid = ?2 AND name = ?3",
        params![dst_zone, src_zone, name],
    )?;
    Ok(())
}

/// Frame parts `first..first + count` of a file in one transaction: encode
/// them, write the frame, read it back, decode it and compare every byte with
/// the rows, and only then delete the rows and put `rev` back
/// (spec §5.1). Skips, changing nothing, unless every part is a full row,
/// none is the file's last part, and no frame holds any of them.
pub(crate) fn frame_parts_in(
    conn: &Connection,
    zone_id: &str,
    name: &str,
    first: i32,
    count: i32,
    codec: i64,
) -> Result<FrameOutcome, StoreError> {
    if count < 1 || count > MAX_FRAME_PARTS || first < 0 {
        return Ok(FrameOutcome::Skipped("bad range"));
    }
    let last = first + count - 1;
    let Some(size) = conn
        .query_row(
            "SELECT size FROM db_wave_file WHERE zoneid = ?1 AND name = ?2",
            params![zone_id, name],
            |r| r.get::<_, i64>(0),
        )
        .optional()?
    else {
        return Ok(FrameOutcome::Skipped("no such file"));
    };
    // The file's last part takes appends; it and anything after stay rows.
    let last_part_of_file = ((size - 1).max(0) / PDS as i64) as i32;
    if size <= 0 || last >= last_part_of_file {
        return Ok(FrameOutcome::Skipped("reaches the file's last part"));
    }
    if any_frame(conn, zone_id, name, first, last)? {
        return Ok(FrameOutcome::Skipped("already framed"));
    }
    let mut raw = Vec::with_capacity(count as usize * PDS);
    {
        let mut stmt = conn.prepare_cached(
            "SELECT partidx, data FROM db_file_data WHERE zoneid = ?1 AND name = ?2 AND partidx BETWEEN ?3 AND ?4 ORDER BY partidx",
        )?;
        let mut rows = stmt.query(params![zone_id, name, first, last])?;
        let mut expect = first;
        while let Some(row) = rows.next()? {
            let idx: i32 = row.get(0)?;
            let data = row.get_ref(1)?.as_blob().map_err(|e| StoreError::Other(format!("part {idx}: {e}")))?;
            if idx != expect || data.len() != PDS {
                return Ok(FrameOutcome::Skipped("a part is missing or short"));
            }
            raw.extend_from_slice(data);
            expect += 1;
        }
        if expect != last + 1 {
            return Ok(FrameOutcome::Skipped("a part is missing or short"));
        }
    }
    let (codec, data) = encode(codec, &raw)?;
    let frame = Frame { first_part: first, part_count: count, codec, raw_len: raw.len() as i64, check_hash: sha256(&raw), data };
    conn.execute(
        "INSERT INTO db_file_frame (zoneid, name, first_part, part_count, codec, raw_len, check_hash, data)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        params![zone_id, name, frame.first_part, frame.part_count, frame.codec, frame.raw_len, frame.check_hash, frame.data],
    )?;
    // Read back what was written and check it against the rows, every byte.
    let stored = frames_overlapping(conn, zone_id, name, first, first)?
        .into_iter()
        .find(|f| f.first_part == first)
        .ok_or_else(|| StoreError::Other("frame not found after insert".to_string()))?;
    if decode_uncached(&stored)? != raw {
        return Err(StoreError::Other(format!("{zone_id}/{name}: frame at part {first} doesn't decode to its parts")));
    }
    let rev = read_rev(conn, zone_id, name)?;
    conn.execute(
        "DELETE FROM db_file_data WHERE zoneid = ?1 AND name = ?2 AND partidx BETWEEN ?3 AND ?4",
        params![zone_id, name, first, last],
    )?;
    put_rev(conn, zone_id, name, rev)?;
    Ok(FrameOutcome::Framed { raw: raw.len(), stored: stored.data.len(), codec })
}

impl FileStore {
    /// Frame parts `first..first + count` of a file (see [`frame_parts_in`]).
    #[track_caller]
    pub(crate) fn frame_parts(&self, zone_id: &str, name: &str, first: i32, count: i32, codec: i64) -> Result<FrameOutcome, StoreError> {
        let out = self.write_txn(|tx| frame_parts_in(tx, zone_id, name, first, count, codec))?;
        self.forget_cached(zone_id, &[name]);
        Ok(out)
    }

    /// Turn every frame of a file back into part rows; the number undone.
    /// For `decompress` (spec §5.4), which comes with the recovery tool.
    #[cfg_attr(not(test), allow(dead_code))]
    #[track_caller]
    pub(crate) fn unframe_file(&self, zone_id: &str, name: &str) -> Result<usize, StoreError> {
        let out = self.write_txn(|tx| unframe(tx, zone_id, name, 0, i32::MAX - MAX_FRAME_PARTS))?;
        self.forget_cached(zone_id, &[name]);
        Ok(out)
    }

    /// Where a file's frames start (part indices).
    pub(crate) fn frame_starts(&self, zone_id: &str, name: &str) -> Result<HashSet<i32>, StoreError> {
        self.read_conn(|conn| {
            let mut stmt = conn.prepare_cached("SELECT first_part FROM db_file_frame WHERE zoneid = ?1 AND name = ?2")?;
            let rows = stmt.query_map(params![zone_id, name], |r| r.get::<_, i32>(0))?;
            Ok(rows.collect::<Result<_, _>>()?)
        })
    }

    /// Stamp the store `GLOBAL_TRANSCRIPT_STORE_FRAMES_VERSION` (never lowering a
    /// higher one): from here on a build that can't read frames refuses it.
    /// The framer calls this before its first frame.
    pub(crate) fn stamp_frames_version(&self) -> Result<(), StoreError> {
        self.write_txn(|tx| {
            let found: i64 = tx.query_row("PRAGMA user_version", [], |r| r.get(0))?;
            let want = super::core::GLOBAL_TRANSCRIPT_STORE_FRAMES_VERSION;
            if found < want {
                tx.execute_batch(&format!("PRAGMA user_version = {want};"))?;
            }
            Ok(())
        })
    }

    /// The bytes the store's frames hold and take: `(logical, stored, frames)`.
    /// For the Storage numbers (spec §7), which come with the recovery tool.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn frame_totals(&self) -> Result<(i64, i64, i64), StoreError> {
        self.read_conn(|conn| {
            Ok(conn.query_row(
                "SELECT COALESCE(SUM(raw_len), 0), COALESCE(SUM(length(data)), 0), COUNT(*) FROM db_file_frame",
                [],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)),
            )?)
        })
    }
}

#[cfg(test)]
#[path = "tests_frames.rs"]
mod tests;
