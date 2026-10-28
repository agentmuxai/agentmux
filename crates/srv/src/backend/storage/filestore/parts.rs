// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Reading a file's parts past a damaged one.
//!
//! A part whose row SQLite can't read ("database disk image is malformed")
//! used to fail the whole read, so one bad 64 KB piece at the end of a
//! conversation made its newest page, and with it the Agent History view,
//! unreadable. Such a part now reads as spaces of its length: blank under the
//! line rule (`lines.rs`), so readers skip it, and every byte after it keeps
//! its offset, so `output.idx` still finds the lines that follow. Only the
//! lines inside the damaged part are lost, and each read of it is logged.
//!
//! A part with no row at all, inside the size its file claims, is treated the
//! same way, but only in the shared transcript store and only for transcript
//! text ([`lost_reads_blank`]). Elsewhere it means a build without transactions
//! is mid-write (it raised the size before storing the parts), so it stays
//! "not stored yet". In the shared store every writer commits size and parts
//! together (its version stamp keeps older builds out, #5203), so a missing part
//! can only be lost: the 2026-10-22 repair left some at the very end of a
//! transcript, and the pane, which opens at the newest lines, stayed empty.

use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};

use rusqlite::{params, Connection, ErrorCode, OptionalExtension};

use super::core::{GLOBAL_TRANSCRIPT_STORE_VERSION, PART_DATA_SIZE};
use crate::backend::storage::error::StoreError;

/// Store `data` as part `idx` of a file: replace the part if it has a row,
/// insert it if not. The same result as `REPLACE INTO db_file_data`, without
/// what `REPLACE` costs a transcript append.
///
/// `REPLACE` deletes and inserts, so SQLite keeps a statement journal for it:
/// the original content of every page the statement touches. A tail part is up
/// to 64 KB (about 17 pages, 70 KB with the journal's page numbers), more than
/// the 64 KB SQLite holds in memory, so the journal went to a new temp file
/// for every appended line: 32% of the bytes srv wrote in the memory lab's
/// 10-agent run (REPORT_SRV_WRITE_AMPLIFICATION_2026_10_27.md). An `UPDATE` of
/// one row, or an `INSERT` of a new one, needs none.
///
/// The revision triggers on `db_file_data` see the same change either way: the
/// `INSERT` trigger for a new row, the `UPDATE OF data` trigger for a changed
/// one, each bumping `rev` only when the old content is not a prefix of the new.
pub(super) fn put_part(conn: &Connection, zone_id: &str, name: &str, idx: i32, data: &[u8]) -> Result<(), StoreError> {
    let changed = conn
        .prepare_cached("UPDATE db_file_data SET data = ?4 WHERE zoneid = ?1 AND name = ?2 AND partidx = ?3")?
        .execute(params![zone_id, name, idx, data])?;
    if changed == 0 {
        conn.prepare_cached("INSERT INTO db_file_data (zoneid, name, partidx, data) VALUES (?1, ?2, ?3, ?4)")?
            .execute(params![zone_id, name, idx, data])?;
    }
    Ok(())
}

/// How many bytes a line-oriented file's tail row is stored with when it holds
/// `content` bytes: `content` rounded up to a power of two, between 4 KB and a
/// whole part. The tail row is written once at that size and appended to in
/// place ([`write_in_place`]); only when it fills up is it rewritten, at twice
/// the size, so the cost of a line no longer grows with the part. Bytes of the
/// row past the file's `size` are slack: spaces (blank under the line rule, so a
/// reader that ignores `size` sees a blank line), never content.
pub(super) fn tail_capacity(content: usize) -> usize {
    content.max(4096).next_power_of_two().min(PART_DATA_SIZE)
}

/// Test helper: the state an older build leaves, or finds after its first append: the file's last
/// row cut to its content, `rev` not moved. The tests that simulate an older build's append do
/// this first so they test what they are about (counter catch-up), not the one thing an older
/// append does to a padded row, which is to bump `rev` (its `REPLACE` writes a row shorter than
/// the old one, which the revision trigger reads as a rewrite): a new generation, once, when a
/// file's writer goes from the new build to an older one (tests_tail_slack.rs).
#[cfg(test)]
pub(crate) fn strip_slack(conn: &Connection, zone_id: &str, name: &str) {
    let row: Option<(i64, Option<i64>)> = conn
        .query_row("SELECT size, rev FROM db_wave_file WHERE zoneid = ?1 AND name = ?2", params![zone_id, name], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()
        .unwrap();
    let Some((size, rev)) = row else { return };
    if size <= 0 {
        return;
    }
    let pds = PART_DATA_SIZE as i64;
    let (idx, content) = ((size - 1) / pds, size - (size - 1) / pds * pds);
    conn.execute(
        "UPDATE db_file_data SET data = substr(data, 1, ?4) WHERE zoneid = ?1 AND name = ?2 AND partidx = ?3 AND length(data) > ?4",
        params![zone_id, name, idx, content],
    )
    .unwrap();
    conn.execute("UPDATE db_wave_file SET rev = ?3 WHERE zoneid = ?1 AND name = ?2 AND rev IS NOT ?3", params![zone_id, name, rev]).unwrap();
}

/// [`put_part`] for the tail row of an append: the file's bytes before its old
/// size are unchanged, so `rev` must not move. The `UPDATE OF data` trigger sees
/// the old row's slack replaced by content and bumps it, because it compares the
/// whole old row (slack included) with the start of the new one; the bump is
/// undone here, in the same transaction.
pub(super) fn put_tail_part(conn: &Connection, zone_id: &str, name: &str, idx: i32, data: &[u8]) -> Result<(), StoreError> {
    let rev: Option<Option<i64>> = conn
        .query_row("SELECT rev FROM db_wave_file WHERE zoneid = ?1 AND name = ?2", params![zone_id, name], |r| r.get(0))
        .optional()?;
    put_part(conn, zone_id, name, idx, data)?;
    if let Some(before) = rev {
        conn.execute("UPDATE db_wave_file SET rev = ?3 WHERE zoneid = ?1 AND name = ?2 AND rev IS NOT ?3", params![zone_id, name, before])?;
    }
    Ok(())
}

/// Write `bytes` into part `idx` at `offset`, in the stored row, when the row
/// is already long enough to hold them (the slack of [`tail_capacity`]):
/// SQLite's incremental blob write touches only the pages those bytes lie on,
/// where [`put_part`] writes the whole part. `false` when the row is missing or
/// too short, for the caller to rewrite it. Only for an append: the bytes
/// before `offset` are not changed, so the revision triggers (which an
/// incremental write does not fire) would not have bumped `rev` either.
pub(super) fn write_in_place(conn: &Connection, zone_id: &str, name: &str, idx: i32, offset: usize, bytes: &[u8]) -> Result<bool, StoreError> {
    use std::io::{Seek, SeekFrom, Write};
    let row: Option<(i64, i64)> = conn
        .prepare_cached("SELECT rowid, length(data) FROM db_file_data WHERE zoneid = ?1 AND name = ?2 AND partidx = ?3")?
        .query_row(params![zone_id, name, idx], |r| Ok((r.get(0)?, r.get(1)?)))
        .optional()?;
    let Some((rowid, len)) = row else { return Ok(false) };
    if (len as usize) < offset + bytes.len() {
        return Ok(false);
    }
    let mut blob = conn.blob_open(rusqlite::MAIN_DB, "db_file_data", "data", rowid, false)?;
    let io = |e: std::io::Error| blob_io_error(e, || format!("in-place write of part {idx} of {zone_id}/{name}"));
    blob.seek(SeekFrom::Start(offset as u64)).map_err(io)?;
    blob.write_all(bytes).map_err(io)?;
    Ok(true)
}

/// A blob read or write reports SQLite's own error wrapped in an `io::Error`. Hand that error back as
/// it was, so a corrupt page still reads as `SQLITE_CORRUPT` to `transcript::damage::is_corrupt`
/// (which marks the zone damaged and starts the per-channel fallback); any other io error is
/// described by `what`.
fn blob_io_error(e: std::io::Error, what: impl FnOnce() -> String) -> StoreError {
    let text = e.to_string();
    match e.into_inner().map(|inner| inner.downcast::<rusqlite::Error>()) {
        Some(Ok(sqlite)) => StoreError::Sqlite(*sqlite),
        _ => StoreError::Other(format!("{}: {text}", what())),
    }
}

/// Whether a part missing inside `name`'s size is lost content, to read as
/// blank: transcript text (`output`) in the shared store, the one database
/// stamped [`GLOBAL_TRANSCRIPT_STORE_VERSION`]. Binary files keep refusing
/// (spaces would read as offsets or stamps), and so does every per-channel
/// store. Asked only after a read came up short.
pub(super) fn lost_reads_blank(conn: &Connection, name: &str) -> Result<bool, StoreError> {
    if name != "output" {
        return Ok(false);
    }
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    Ok(version >= GLOBAL_TRANSCRIPT_STORE_VERSION)
}

/// Log a file read past a lost part, once per file in this process: every
/// read of a pane's newest page would otherwise log it again.
pub(super) fn note_lost(zone_id: &str, name: &str) {
    static SEEN: LazyLock<Mutex<HashSet<(String, String)>>> = LazyLock::new(Default::default);
    if SEEN.lock().unwrap_or_else(|e| e.into_inner()).insert((zone_id.to_string(), name.to_string())) {
        tracing::warn!(zone = zone_id, file = name, "filestore: a lost part inside the file's size reads as blank");
    }
}

/// SQLite reported the database file itself as damaged.
pub(super) fn is_corrupt(e: &rusqlite::Error) -> bool {
    matches!(e, rusqlite::Error::SqliteFailure(f, _) if f.code == ErrorCode::DatabaseCorrupt)
}

/// Parts `start..=end` of a file of `size` bytes, by index. A part with no row
/// is left out, as before; a damaged one is spaces.
pub(super) fn load_parts(
    conn: &Connection,
    zone_id: &str,
    name: &str,
    start: i32,
    end: i32,
    size: i64,
) -> Result<HashMap<i32, Vec<u8>>, StoreError> {
    // One range query, not one per part: reading an 800 MB transcript part by
    // part was 12,270 queries and took 12 s in a release build. Rows and
    // frames are read in one snapshot (frames.rs `in_snapshot`).
    super::frames::in_snapshot(conn, || match load_range(conn, zone_id, name, start, end) {
        Err(StoreError::Sqlite(e)) if is_corrupt(&e) => load_each(conn, zone_id, name, start, end, size),
        other => other,
    })
}

/// `[offset, offset + len)` of a file and how many of its bytes are stored,
/// as `counter::read_bytes_covered` reads them, for when its range query hit
/// a damaged part: that part counts as stored, as spaces.
pub(super) fn read_covered_past_damage(
    conn: &Connection,
    zone_id: &str,
    name: &str,
    offset: i64,
    len: i64,
    fill: u8,
) -> Result<(Vec<u8>, usize), StoreError> {
    if len <= 0 {
        return Ok((Vec::new(), 0));
    }
    let size: i64 = conn.query_row(
        "SELECT size FROM db_wave_file WHERE zoneid = ?1 AND name = ?2",
        params![zone_id, name],
        |r| r.get(0),
    )?;
    let pds = PART_DATA_SIZE as i64;
    let (first, last) = ((offset / pds) as i32, ((offset + len - 1) / pds) as i32);
    let parts = load_each(conn, zone_id, name, first, last, size)?;
    let mut out = vec![fill; len as usize];
    let mut covered = 0usize;
    for (idx, data) in &parts {
        let part_start = *idx as i64 * pds;
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

fn load_range(conn: &Connection, zone_id: &str, name: &str, start: i32, end: i32) -> Result<HashMap<i32, Vec<u8>>, StoreError> {
    let mut stmt = conn.prepare_cached(
        "SELECT partidx, data FROM db_file_data \
         WHERE zoneid = ?1 AND name = ?2 AND partidx BETWEEN ?3 AND ?4",
    )?;
    let rows = stmt.query_map(params![zone_id, name, start, end], |row| {
        Ok((row.get::<_, i32>(0)?, row.get::<_, Vec<u8>>(1)?))
    })?;
    let mut parts = HashMap::new();
    for row in rows {
        let (idx, data) = row?;
        parts.insert(idx, data);
    }
    drop(stmt);
    // Parts with no row may be in a compressed frame (frames.rs).
    super::frames::fill_parts(conn, zone_id, name, start, end, &mut parts, false)?;
    Ok(parts)
}

/// The slow path, only after the range query hit damage: each part on its own.
fn load_each(
    conn: &Connection,
    zone_id: &str,
    name: &str,
    start: i32,
    end: i32,
    size: i64,
) -> Result<HashMap<i32, Vec<u8>>, StoreError> {
    // Which parts exist comes from the primary key's index, without reading
    // the rows themselves.
    let present: Vec<i32> = {
        let mut stmt = conn.prepare_cached(
            "SELECT partidx FROM db_file_data WHERE zoneid = ?1 AND name = ?2 AND partidx BETWEEN ?3 AND ?4",
        )?;
        let rows = stmt.query_map(params![zone_id, name, start, end], |row| row.get::<_, i32>(0))?;
        rows.collect::<Result<_, _>>()?
    };
    let mut one = conn.prepare_cached("SELECT data FROM db_file_data WHERE zoneid = ?1 AND name = ?2 AND partidx = ?3")?;
    let mut parts = HashMap::with_capacity(present.len());
    for idx in present {
        match one.query_row(params![zone_id, name, idx], |row| row.get::<_, Vec<u8>>(0)) {
            Ok(data) => {
                parts.insert(idx, data);
            }
            Err(e) if is_corrupt(&e) => {
                let len = (size - idx as i64 * PART_DATA_SIZE as i64).clamp(0, PART_DATA_SIZE as i64) as usize;
                tracing::warn!(zone = zone_id, file = name, part = idx, "filestore: a damaged part reads as blank");
                parts.insert(idx, vec![b' '; len]);
            }
            Err(e) => return Err(e.into()),
        }
    }
    drop(one);
    // A frame holding parts with no row fills them; a damaged frame reads as blank.
    super::frames::fill_parts(conn, zone_id, name, start, end, &mut parts, true)?;
    Ok(parts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_blob_write_that_hits_a_corrupt_page_stays_a_sqlite_corruption_error() {
        let corrupt = rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CORRUPT), None);
        let e = blob_io_error(std::io::Error::other(corrupt), || "write".into());
        assert!(crate::backend::transcript::damage::is_corrupt(&e), "{e:?}");
        let other = blob_io_error(std::io::Error::new(std::io::ErrorKind::InvalidInput, "seek"), || "write".into());
        assert!(matches!(&other, StoreError::Other(m) if m.contains("write") && m.contains("seek")), "{other:?}");
    }

    fn store() -> Connection {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE db_file_data (zoneid TEXT NOT NULL, name TEXT NOT NULL, partidx INTEGER NOT NULL, \
             data BLOB NOT NULL, PRIMARY KEY (zoneid, name, partidx))",
        )
        .unwrap();
        conn
    }

    fn put(conn: &Connection, idx: i32, data: &[u8]) {
        conn.execute("INSERT INTO db_file_data VALUES ('z', 'output', ?1, ?2)", params![idx, data]).unwrap();
    }

    #[test]
    fn reads_every_part_in_the_range() {
        let conn = store();
        put(&conn, 0, b"a\n");
        put(&conn, 1, b"b\n");
        put(&conn, 2, b"c\n");
        let parts = load_parts(&conn, "z", "output", 1, 2, 2 * PART_DATA_SIZE as i64 + 2).unwrap();
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[&1], b"b\n");
        assert_eq!(parts[&2], b"c\n");
    }

    #[test]
    fn only_a_damaged_database_takes_the_slow_path() {
        let corrupt = rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_CORRUPT), None);
        assert!(is_corrupt(&corrupt));
        let busy = rusqlite::Error::SqliteFailure(rusqlite::ffi::Error::new(rusqlite::ffi::SQLITE_BUSY), None);
        assert!(!is_corrupt(&busy));
        assert!(!is_corrupt(&rusqlite::Error::QueryReturnedNoRows));
    }

    #[test]
    fn a_damaged_part_reads_as_blank_and_the_others_as_written() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("filestore.db");
        let pds = PART_DATA_SIZE;
        {
            let conn = Connection::open(&path).unwrap();
            conn.execute_batch("PRAGMA page_size = 4096; PRAGMA journal_mode = DELETE;").unwrap();
            conn.execute_batch(
                "CREATE TABLE db_file_data (zoneid TEXT NOT NULL, name TEXT NOT NULL, partidx INTEGER NOT NULL, \
                 data BLOB NOT NULL, PRIMARY KEY (zoneid, name, partidx))",
            )
            .unwrap();
            conn.execute_batch("CREATE TABLE db_wave_file (zoneid TEXT NOT NULL, name TEXT NOT NULL, size INTEGER NOT NULL)").unwrap();
            conn.execute("INSERT INTO db_wave_file VALUES ('z', 'output', ?1)", params![4 * pds as i64]).unwrap();
            for idx in 0..4u8 {
                put(&conn, idx as i32, &vec![b'0' + idx; pds]);
            }
        }
        // The last row's blob ends in a chain of overflow pages at the end of
        // the file. Point the second-to-last page's "next" link past the end:
        // reading that row now fails as SQLite reports a damaged database.
        let mut bytes = std::fs::read(&path).unwrap();
        let at = bytes.len() - 2 * 4096;
        bytes[at..at + 4].copy_from_slice(&[0xff; 4]);
        std::fs::write(&path, &bytes).unwrap();

        let conn = Connection::open(&path).unwrap();
        let err = load_range(&conn, "z", "output", 0, 3).unwrap_err();
        assert!(matches!(&err, StoreError::Sqlite(e) if is_corrupt(e)), "{err}");

        let parts = load_parts(&conn, "z", "output", 0, 3, 4 * pds as i64).unwrap();
        for idx in 0..3u8 {
            assert_eq!(parts[&(idx as i32)], vec![b'0' + idx; pds]);
        }
        assert_eq!(parts[&3], vec![b' '; pds]);

        // The exact read the snapshot reader and the counter's catch-up both
        // use, across the damaged part's start.
        let at = 3 * pds as i64 - 10;
        let bytes = super::super::counter::read_bytes_exact(&conn, "z", "output", at, 20).unwrap().unwrap();
        assert_eq!(bytes, [vec![b'2'; 10], vec![b' '; 10]].concat());
    }

    #[test]
    fn the_slow_path_reads_what_the_range_query_reads() {
        let conn = store();
        put(&conn, 0, b"a\n");
        put(&conn, 2, b"c\n");
        let size = 2 * PART_DATA_SIZE as i64 + 2;
        assert_eq!(load_each(&conn, "z", "output", 0, 2, size).unwrap(), load_range(&conn, "z", "output", 0, 2).unwrap());
    }
}
