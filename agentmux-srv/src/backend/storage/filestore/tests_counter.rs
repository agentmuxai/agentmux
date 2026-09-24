// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The per-file line counter and generation (`counter.rs`, Phase 5a-2).

use std::collections::HashSet;
use std::path::Path;
use std::sync::{Arc, Barrier};

use proptest::prelude::*;
use rusqlite::params;

use super::{AppendPos, FileMeta, FileOpts, FileStore, LineState};
use crate::backend::blockcontroller::shell::rebuild_output_idx;

// The indexer reads `output` in zone = block id.
const ZONE: &str = "block-1";
const NAME: &str = "output";

fn mem() -> FileStore {
    FileStore::open_in_memory().unwrap()
}

fn counted(state: &Option<LineState>) -> (String, u64) {
    let c = state.as_ref().unwrap().counted.as_ref().expect("file should be counted");
    (c.gen.clone(), c.lines)
}

fn pos(p: &AppendPos) -> (u64, u64) {
    let c = p.counted.as_ref().expect("append should be counted");
    (c.first_line, c.lines)
}

/// The reader's addressing, as the read path computes it — from the
/// database, like the read paths: an older build's write (simulated below on
/// the raw connection) never passes through this store's `stat` cache.
fn reader_lines(fs: &FileStore) -> Vec<String> {
    let size = fs.line_state(ZONE, NAME).unwrap().unwrap().size;
    let data = fs.read_bytes_db(ZONE, NAME, 0, size).unwrap();
    String::from_utf8_lossy(&data).lines().filter(|l| !l.trim().is_empty()).map(str::to_string).collect()
}

/// What an srv build without the counter does on append: size and parts, no
/// counter columns.
fn old_build_append(fs: &FileStore, data: &[u8]) {
    let conn = fs.conn().lock().unwrap();
    let size: i64 = conn
        .query_row("SELECT size FROM db_wave_file WHERE zoneid = ?1 AND name = ?2", params![ZONE, NAME], |r| r.get(0))
        .unwrap();
    assert!(size + data.len() as i64 <= 64 * 1024, "helper writes within part 0");
    let mut part: Vec<u8> = conn
        .query_row("SELECT data FROM db_file_data WHERE zoneid = ?1 AND name = ?2 AND partidx = 0", params![ZONE, NAME], |r| r.get(0))
        .unwrap_or_default();
    part.truncate(size as usize);
    part.extend_from_slice(data);
    conn.execute("REPLACE INTO db_file_data (zoneid, name, partidx, data) VALUES (?1, ?2, 0, ?3)", params![ZONE, NAME, part]).unwrap();
    conn.execute(
        "UPDATE db_wave_file SET size = ?1, modts = modts + 1 WHERE zoneid = ?2 AND name = ?3",
        params![size + data.len() as i64, ZONE, NAME],
    )
    .unwrap();
}

/// An older build's append of any size: parts first (the last one extended,
/// then new ones from the old size), then `size` — no counter columns.
fn old_build_append_parts(fs: &FileStore, data: &[u8]) {
    const PART: usize = 64 * 1024;
    let conn = fs.conn().lock().unwrap();
    let size: i64 = conn
        .query_row("SELECT size FROM db_wave_file WHERE zoneid = ?1 AND name = ?2", params![ZONE, NAME], |r| r.get(0))
        .unwrap();
    let mut at = size as usize;
    let mut rest = data;
    while !rest.is_empty() {
        let (idx, in_part) = (at / PART, at % PART);
        let mut part: Vec<u8> = conn
            .query_row(
                "SELECT data FROM db_file_data WHERE zoneid = ?1 AND name = ?2 AND partidx = ?3",
                params![ZONE, NAME, idx as i64],
                |r| r.get(0),
            )
            .unwrap_or_default();
        part.truncate(in_part);
        let take = (PART - in_part).min(rest.len());
        part.extend_from_slice(&rest[..take]);
        conn.execute("REPLACE INTO db_file_data (zoneid, name, partidx, data) VALUES (?1, ?2, ?3, ?4)", params![ZONE, NAME, idx as i64, part])
            .unwrap();
        at += take;
        rest = &rest[take..];
    }
    conn.execute(
        "UPDATE db_wave_file SET size = ?1, modts = modts + 1 WHERE zoneid = ?2 AND name = ?3",
        params![at as i64, ZONE, NAME],
    )
    .unwrap();
}

/// Clear the epoch, as a write this build couldn't vouch for does.
fn drop_epoch(fs: &FileStore) {
    fs.conn()
        .lock()
        .unwrap()
        .execute(&format!("UPDATE db_wave_file SET {}", super::counter::DROP_EPOCH_SQL), [])
        .unwrap();
}

#[test]
fn a_new_file_is_counted_from_zero_with_a_fresh_generation() {
    let fs = mem();
    fs.make_file(ZONE, NAME, FileMeta::new(), FileOpts::default()).unwrap();
    fs.make_file(ZONE, "other", FileMeta::new(), FileOpts::default()).unwrap();
    let (gen, lines) = counted(&fs.line_state(ZONE, NAME).unwrap());
    assert_eq!((gen.len(), lines), (16, 0));
    assert_ne!(gen, counted(&fs.line_state(ZONE, "other").unwrap()).0);
    assert_eq!(fs.line_state(ZONE, "missing").unwrap(), None);
}

#[test]
fn raw_appends_report_the_line_each_starts_in() {
    let fs = mem();
    fs.make_file(ZONE, NAME, FileMeta::new(), FileOpts::default()).unwrap();
    assert_eq!(pos(&fs.append_data_pos(ZONE, NAME, b"a\n").unwrap()), (0, 1));
    // An unterminated line is counted, and a continuation belongs to it.
    assert_eq!(pos(&fs.append_data_pos(ZONE, NAME, b"b").unwrap()), (1, 2));
    assert_eq!(pos(&fs.append_data_pos(ZONE, NAME, b"c\n  \nd\n").unwrap()), (1, 3));
    // A blank open line isn't counted until it has content.
    assert_eq!(pos(&fs.append_data_pos(ZONE, NAME, b" ").unwrap()), (3, 3));
    assert_eq!(pos(&fs.append_data_pos(ZONE, NAME, b"e\n").unwrap()), (3, 4));
    assert_eq!(reader_lines(&fs), ["a", "bc", "d", " e"]);
}

#[test]
fn append_lines_normalizes_and_closes_a_torn_tail() {
    let fs = mem();
    fs.make_file(ZONE, NAME, FileMeta::new(), FileOpts::default()).unwrap();
    fs.append_data(ZONE, NAME, b"{\"torn\":").unwrap();
    let p = fs.append_lines(ZONE, NAME, b"x\n\n \r\ny").unwrap();
    assert_eq!(p.offset, 8);
    // The torn record keeps index 0; the two new lines are 1 and 2.
    assert_eq!(pos(&p), (1, 3));
    assert_eq!(fs.read_file(ZONE, NAME).unwrap().unwrap(), b"{\"torn\":\nx\ny\n");
    // Nothing to append: no bytes, position unchanged.
    let p = fs.append_lines(ZONE, NAME, b"\n  \n").unwrap();
    assert_eq!((p.offset, pos(&p)), (13, (3, 3)));
    assert_eq!(fs.stat(ZONE, NAME).unwrap().unwrap().size, 13);
}

#[test]
fn an_append_that_writes_nothing_leaves_the_cached_row_alone() {
    let fs = mem();
    fs.make_file(ZONE, NAME, FileMeta::new(), FileOpts::default()).unwrap();
    fs.append_lines(ZONE, NAME, b"a\n").unwrap();
    let cached = fs.stat(ZONE, NAME).unwrap().unwrap();
    std::thread::sleep(std::time::Duration::from_millis(5));
    fs.append_data_pos(ZONE, NAME, b"").unwrap();
    fs.append_lines(ZONE, NAME, b"\n \n\r\n").unwrap();
    let after = fs.stat(ZONE, NAME).unwrap().unwrap();
    assert_eq!((after.size, after.modts), (cached.size, cached.modts), "the cache must keep matching the database");
    let db_modts: i64 = fs
        .conn()
        .lock()
        .unwrap()
        .query_row("SELECT modts FROM db_wave_file WHERE zoneid = ?1 AND name = ?2", params![ZONE, NAME], |r| r.get(0))
        .unwrap();
    assert_eq!(after.modts, db_modts);
}

#[test]
fn replacing_or_recreating_a_file_starts_a_new_generation() {
    let fs = mem();
    fs.make_file(ZONE, NAME, FileMeta::new(), FileOpts::default()).unwrap();
    fs.append_lines(ZONE, NAME, b"a\nb\n").unwrap();
    let (g0, _) = counted(&fs.line_state(ZONE, NAME).unwrap());

    fs.write_file(ZONE, NAME, b"x\n\ny\nz").unwrap();
    let (g1, lines) = counted(&fs.line_state(ZONE, NAME).unwrap());
    assert_ne!(g1, g0);
    assert_eq!(lines, 3);
    assert_eq!(pos(&fs.append_data_pos(ZONE, NAME, b"!\n").unwrap()), (2, 3));

    fs.delete_file(ZONE, NAME).unwrap();
    fs.make_file(ZONE, NAME, FileMeta::new(), FileOpts::default()).unwrap();
    let (g2, lines) = counted(&fs.line_state(ZONE, NAME).unwrap());
    assert!(g2 != g0 && g2 != g1);
    assert_eq!(lines, 0);
}

#[test]
fn a_metadata_write_keeps_the_epoch() {
    let fs = mem();
    fs.make_file(ZONE, NAME, FileMeta::new(), FileOpts::default()).unwrap();
    fs.append_lines(ZONE, NAME, b"a\n").unwrap();
    let before = counted(&fs.line_state(ZONE, NAME).unwrap());
    let mut m = FileMeta::new();
    m.insert("k".into(), serde_json::json!(1));
    fs.write_meta(ZONE, NAME, m, true).unwrap();
    assert_eq!(counted(&fs.line_state(ZONE, NAME).unwrap()), before);
}

#[test]
fn an_older_builds_append_is_caught_up_and_keeps_the_generation() {
    let fs = mem();
    fs.make_file(ZONE, NAME, FileMeta::new(), FileOpts::default()).unwrap();
    fs.append_lines(ZONE, NAME, b"a\nb\n").unwrap();
    let (g0, _) = counted(&fs.line_state(ZONE, NAME).unwrap());

    old_build_append(&fs, b"c\n");
    // Behind, not counted, until someone catches it up.
    assert_eq!(fs.line_state(ZONE, NAME).unwrap().unwrap().counted, None);
    assert_eq!(counted(&fs.init_line_counter(ZONE, NAME).unwrap()), (g0.clone(), 3));

    // An append catches up inline, in its own transaction.
    old_build_append(&fs, b"d\n");
    let p = fs.append_lines(ZONE, NAME, b"e\n").unwrap();
    assert_eq!(p.counted.as_ref().unwrap().gen, g0);
    assert_eq!(pos(&p), (4, 5));
    assert_eq!(reader_lines(&fs), ["a", "b", "c", "d", "e"]);
    // Already counted: init returns the epoch unchanged.
    assert_eq!(counted(&fs.init_line_counter(ZONE, NAME).unwrap()), (g0, 5));
}

#[test]
fn a_write_this_build_cant_vouch_for_drops_the_epoch_and_init_starts_a_new_one() {
    let fs = mem();
    fs.make_file(ZONE, NAME, FileMeta::new(), FileOpts::default()).unwrap();
    fs.append_lines(ZONE, NAME, b"a\nb\n").unwrap();
    let (g0, _) = counted(&fs.line_state(ZONE, NAME).unwrap());
    // A rewrite of counted bytes (here in place, same size): `rev` moves.
    fs.write_at(ZONE, NAME, 0, b"x").unwrap();
    assert_eq!(fs.line_state(ZONE, NAME).unwrap().unwrap().counted, None);
    // Appends go on uncounted rather than guess.
    assert_eq!(fs.append_lines(ZONE, NAME, b"d\n").unwrap().counted, None);
    let (g1, lines) = counted(&fs.init_line_counter(ZONE, NAME).unwrap());
    assert_ne!(g1, g0, "a re-counted epoch must not reuse the old generation");
    assert_eq!(lines, 3);
    assert_eq!(reader_lines(&fs), ["x", "b", "d"]);
}

#[test]
fn a_caught_up_epoch_continues_an_open_line() {
    let fs = mem();
    fs.make_file(ZONE, NAME, FileMeta::new(), FileOpts::default()).unwrap();
    fs.append_data(ZONE, NAME, b"a\n  ").unwrap(); // a blank open line: not counted
    let (g0, _) = counted(&fs.line_state(ZONE, NAME).unwrap());
    // The older build's bytes complete it, and it becomes a line: a new
    // index, so the generation is kept.
    old_build_append(&fs, b"b\nc");
    assert_eq!(counted(&fs.init_line_counter(ZONE, NAME).unwrap()), (g0.clone(), 3));
    // Now the counted open line "c" is continued: line 2 would name "cd"
    // under the generation it was counted as "c" in (Codex on #3663).
    old_build_append(&fs, b"d\n");
    // An append doesn't extend that epoch: it is left for init ...
    assert_eq!(fs.append_data_pos(ZONE, NAME, b"e\n").unwrap().counted, None);
    // ... which catches it up under a new generation.
    let (g1, lines) = counted(&fs.init_line_counter(ZONE, NAME).unwrap());
    assert_ne!(g1, g0);
    assert_eq!(lines, 4);
    assert_eq!(reader_lines(&fs), ["a", "  b", "cd", "e"]);
}

#[test]
fn a_counted_open_line_completed_into_whitespace_gets_a_new_generation() {
    // Codex on #3663: a split UTF-8 sequence counts as a line ("\xe3\x80"
    // decodes to U+FFFD); completed into U+3000 it is blank, the line
    // vanishes, and its index would go to the next record.
    let fs = mem();
    fs.make_file(ZONE, NAME, FileMeta::new(), FileOpts::default()).unwrap();
    fs.append_data(ZONE, NAME, b"a\n\xe3\x80").unwrap();
    let (g0, lines) = counted(&fs.line_state(ZONE, NAME).unwrap());
    assert_eq!(lines, 2);
    old_build_append(&fs, b"\x80\nb\n");
    let (g1, lines) = counted(&fs.catch_up_line_counter(ZONE, NAME).unwrap());
    assert_ne!(g1, g0, "index 1 now names \"b\", not the old line");
    assert_eq!(lines, 2);
    assert_eq!(reader_lines(&fs), ["a", "b"]);
}

#[test]
fn an_epoch_far_behind_is_left_for_init_not_dropped() {
    let fs = mem();
    fs.make_file(ZONE, NAME, FileMeta::new(), FileOpts::default()).unwrap();
    fs.append_lines(ZONE, NAME, b"a\n").unwrap();
    let (g0, _) = counted(&fs.line_state(ZONE, NAME).unwrap());
    // More than an append catches up inline, over many parts and windows.
    let mut big = Vec::new();
    for i in 0..50_000 {
        big.extend_from_slice(format!("{i}:{}\n", "q".repeat(i % 80)).as_bytes());
    }
    assert!(big.len() > (1 << 20) + (1 << 20) / 2);
    old_build_append_parts(&fs, &big);
    // The append isn't counted, but leaves the epoch to catch up.
    assert_eq!(fs.append_lines(ZONE, NAME, b"z\n").unwrap().counted, None);
    assert_eq!(counted(&fs.init_line_counter(ZONE, NAME).unwrap()), (g0, 50_002));
    assert_eq!(reader_lines(&fs).len(), 50_002);
}

#[test]
fn a_catch_up_gives_up_if_the_epoch_moves_during_its_scan() {
    let fs = mem();
    fs.make_file(ZONE, NAME, FileMeta::new(), FileOpts::default()).unwrap();
    fs.append_lines(ZONE, NAME, b"a\n").unwrap();
    let (g0, _) = counted(&fs.line_state(ZONE, NAME).unwrap());
    old_build_append(&fs, b"b\n");
    let scan = scanned(&fs);
    // Another instance catches it up (and appends) first: the scan's base is
    // gone, and finishing returns that instance's epoch — not one built on
    // the stale base.
    fs.append_lines(ZONE, NAME, b"c\n").unwrap();
    assert_eq!(counted(&fs.init_finish(ZONE, NAME, scan).unwrap()), (g0.clone(), 3));
    // Behind again, and dropped by a rewrite before a scan finishes: nothing.
    old_build_append(&fs, b"d\n");
    let scan = scanned(&fs);
    drop_epoch(&fs);
    assert_eq!(fs.init_finish(ZONE, NAME, scan).unwrap(), None);
}

#[test]
fn a_catch_up_gives_up_if_the_file_is_rewritten_during_its_scan() {
    let fs = mem();
    fs.make_file(ZONE, NAME, FileMeta::new(), FileOpts::default()).unwrap();
    fs.append_lines(ZONE, NAME, b"aaaa\nbbbb\n").unwrap();
    let (g0, _) = counted(&fs.line_state(ZONE, NAME).unwrap());
    old_build_append(&fs, b"cccc\n");
    let scan = scanned(&fs);
    fs.write_at(ZONE, NAME, 5, b"    ").unwrap(); // "bbbb" becomes blank
    assert_eq!(fs.init_finish(ZONE, NAME, scan).unwrap(), None);
    // Rewritten: a new epoch from byte 0, with a new generation.
    let (g1, lines) = counted(&fs.init_line_counter(ZONE, NAME).unwrap());
    assert_ne!(g1, g0);
    assert_eq!(lines, 2);
}

#[test]
fn a_database_from_an_older_build_gains_the_columns_and_its_rows_can_be_counted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("filestore.db");
    {
        let conn = rusqlite::Connection::open(&path).unwrap();
        conn.execute_batch(
            "CREATE TABLE db_wave_file (zoneid TEXT NOT NULL, name TEXT NOT NULL, size INTEGER NOT NULL DEFAULT 0,
                 createdts INTEGER NOT NULL DEFAULT 0, modts INTEGER NOT NULL DEFAULT 0,
                 opts TEXT NOT NULL DEFAULT '{}', meta TEXT NOT NULL DEFAULT '{}', PRIMARY KEY (zoneid, name));
             CREATE TABLE db_file_data (zoneid TEXT NOT NULL, name TEXT NOT NULL, partidx INTEGER NOT NULL,
                 data BLOB NOT NULL, PRIMARY KEY (zoneid, name, partidx));",
        )
        .unwrap();
        let content: &[u8] = b"one\n\ntwo\r\nthree";
        conn.execute(
            "INSERT INTO db_wave_file (zoneid, name, size, createdts, modts) VALUES (?1, ?2, ?3, 1, 1)",
            params![ZONE, NAME, content.len() as i64],
        )
        .unwrap();
        conn.execute("INSERT INTO db_file_data VALUES (?1, ?2, 0, ?3)", params![ZONE, NAME, content]).unwrap();
    }
    // Instances opening the old database at once: each switches it to WAL
    // and adds the columns without failing on the other's locks.
    let opened: Vec<FileStore> = (0..4)
        .map(|_| {
            let path = path.clone();
            std::thread::spawn(move || FileStore::open(&path).unwrap())
        })
        .collect::<Vec<_>>()
        .into_iter()
        .map(|h| h.join().unwrap())
        .collect();
    let fs = &opened[0];
    assert_eq!(fs.line_state(ZONE, NAME).unwrap().unwrap().counted, None);
    let (_, lines) = counted(&fs.init_line_counter(ZONE, NAME).unwrap());
    assert_eq!(lines, 3);
    // "three" was torn: append_lines closes it.
    assert_eq!(pos(&opened[1].append_lines(ZONE, NAME, b"four\n").unwrap()), (3, 4));
    assert_eq!(reader_lines(fs), ["one", "two", "three", "four"]);
}

/// A file without an epoch (dropped, or never counted), ready for init.
fn uncounted(fs: &FileStore, content: &[u8]) {
    fs.make_file(ZONE, NAME, FileMeta::new(), FileOpts::default()).unwrap();
    fs.append_data(ZONE, NAME, content).unwrap();
    drop_epoch(fs);
    assert_eq!(fs.line_state(ZONE, NAME).unwrap().unwrap().counted, None);
}

fn scanned(fs: &FileStore) -> super::counter::ScanResult {
    match fs.init_scan(ZONE, NAME).unwrap() {
        super::counter::InitScan::Scanned(scan) => scan,
        super::counter::InitScan::Done(_) => panic!("expected a scan"),
    }
}

#[test]
fn init_gives_up_when_bytes_it_scanned_are_rewritten_in_place() {
    // Review of #3631: size and the scanned tail are not proof that the
    // middle of the file is unchanged.
    let fs = mem();
    uncounted(&fs, b"aaaa\nbbbb\ncccc\n");
    let scan = scanned(&fs);
    // Same size, same tail; the middle line becomes blank (3 lines -> 2).
    fs.write_at(ZONE, NAME, 5, b"    ").unwrap();
    assert_eq!(fs.init_finish(ZONE, NAME, scan).unwrap(), None);
    assert_eq!(counted(&fs.init_line_counter(ZONE, NAME).unwrap()).1, 2);
}

#[test]
fn init_gives_up_when_the_file_is_recreated_during_the_scan() {
    // Deleted and re-created by a writer that doesn't start an epoch (an
    // older build, which deletes the row, then its parts, then inserts), in
    // the same millisecond, with the same size and bytes: every column but
    // the trigger-set incarnation comes out equal (Codex on #3631), and `rev`
    // starts over with the new row.
    let fs = mem();
    let content: &[u8] = b"one\ntwo\n";
    uncounted(&fs, content);
    let scan = scanned(&fs);
    {
        let conn = fs.conn().lock().unwrap();
        let (created, modts): (i64, i64) = conn
            .query_row("SELECT createdts, modts FROM db_wave_file WHERE zoneid = ?1 AND name = ?2", params![ZONE, NAME], |r| Ok((r.get(0)?, r.get(1)?)))
            .unwrap();
        conn.execute("DELETE FROM db_wave_file WHERE zoneid = ?1 AND name = ?2", params![ZONE, NAME]).unwrap();
        conn.execute("DELETE FROM db_file_data WHERE zoneid = ?1 AND name = ?2", params![ZONE, NAME]).unwrap();
        conn.execute(
            "INSERT INTO db_wave_file (zoneid, name, size, createdts, modts) VALUES (?1, ?2, ?3, ?4, ?5)",
            params![ZONE, NAME, content.len() as i64, created, modts],
        )
        .unwrap();
        conn.execute("INSERT INTO db_file_data (zoneid, name, partidx, data) VALUES (?1, ?2, 0, ?3)", params![ZONE, NAME, content])
            .unwrap();
    }
    assert_eq!(fs.init_finish(ZONE, NAME, scan).unwrap(), None);
}

fn rev(fs: &FileStore) -> i64 {
    fs.conn()
        .lock()
        .unwrap()
        .query_row("SELECT COALESCE(rev, 0) FROM db_wave_file WHERE zoneid = ?1 AND name = ?2", params![ZONE, NAME], |r| r.get(0))
        .unwrap()
}

/// An older build's `write_file`: delete the parts, insert the new ones,
/// update size and modts — here to exactly the values they had.
fn old_build_replace_keeping_size_and_modts(fs: &FileStore, content: &[u8]) {
    let conn = fs.conn().lock().unwrap();
    let modts: i64 = conn
        .query_row("SELECT modts FROM db_wave_file WHERE zoneid = ?1 AND name = ?2", params![ZONE, NAME], |r| r.get(0))
        .unwrap();
    conn.execute("DELETE FROM db_file_data WHERE zoneid = ?1 AND name = ?2", params![ZONE, NAME]).unwrap();
    conn.execute("INSERT INTO db_file_data (zoneid, name, partidx, data) VALUES (?1, ?2, 0, ?3)", params![ZONE, NAME, content])
        .unwrap();
    conn.execute(
        "UPDATE db_wave_file SET size = ?1, modts = ?2 WHERE zoneid = ?3 AND name = ?4",
        params![content.len() as i64, modts, ZONE, NAME],
    )
    .unwrap();
}

#[test]
fn appends_never_bump_rev() {
    // The triggers must tell an append (a pure extension of the last part)
    // from a rewrite, or every append would drop the epoch.
    let fs = mem();
    fs.make_file(ZONE, NAME, FileMeta::new(), FileOpts::default()).unwrap();
    for i in 0..40 {
        // Some lines span a part boundary.
        let line = format!("{i}:{}\n", "z".repeat(if i % 9 == 0 { 70_000 } else { 50 }));
        fs.append_lines(ZONE, NAME, line.as_bytes()).unwrap();
        fs.append_data(ZONE, NAME, b"raw\n").unwrap();
    }
    assert_eq!(rev(&fs), 0);
    assert_eq!(counted(&fs.line_state(ZONE, NAME).unwrap()).1, 80);
}

#[test]
fn an_older_builds_same_size_replace_in_the_same_millisecond_drops_the_epoch() {
    // Codex P1 on #3631: size and a millisecond timestamp can both match
    // after a replace; the database's own trigger still records it.
    let fs = mem();
    fs.make_file(ZONE, NAME, FileMeta::new(), FileOpts::default()).unwrap();
    fs.append_lines(ZONE, NAME, b"aaaa\nbbbb\n").unwrap();
    assert!(fs.line_state(ZONE, NAME).unwrap().unwrap().counted.is_some());
    old_build_replace_keeping_size_and_modts(&fs, b"x\ny\nz\nw\nv\n");
    assert_eq!(fs.line_state(ZONE, NAME).unwrap().unwrap().counted, None);
    // And nothing appended afterwards is given a position in the old epoch.
    assert_eq!(fs.append_lines(ZONE, NAME, b"after\n").unwrap().counted, None);
    assert_eq!(counted(&fs.init_line_counter(ZONE, NAME).unwrap()).1, 6);
}

#[test]
fn init_gives_up_when_an_older_build_replaces_the_file_during_the_scan() {
    // Codex P1 on #3631: same length, same last bytes, different lines in
    // the already-scanned region.
    let fs = mem();
    uncounted(&fs, b"a\nb\nc\nd\n.......tail-that-stays\n");
    let scan = scanned(&fs);
    old_build_replace_keeping_size_and_modts(&fs, b"ab\n\ncd\n\n.......tail-that-stays\n");
    assert_eq!(fs.init_finish(ZONE, NAME, scan).unwrap(), None);
    assert_eq!(counted(&fs.init_line_counter(ZONE, NAME).unwrap()).1, 3);
}

#[test]
fn a_file_mid_write_by_an_older_build_is_not_counted_and_its_fill_bumps_rev() {
    // Codex P1 on #3631: an older build's write_file on an empty file raises
    // `size` first and inserts part 0 after, in separate statements.
    let fs = mem();
    uncounted(&fs, b"");
    let content: &[u8] = b"a\nb\nc\n";
    fs.conn()
        .lock()
        .unwrap()
        .execute("UPDATE db_wave_file SET size = ?1, modts = modts + 1 WHERE zoneid = ?2 AND name = ?3", params![content.len() as i64, ZONE, NAME])
        .unwrap();
    // Between the two statements: bytes claimed but not stored are never
    // counted (they would read as zeros).
    assert_eq!(fs.init_line_counter(ZONE, NAME).unwrap(), None);
    let before = rev(&fs);
    fs.conn()
        .lock()
        .unwrap()
        .execute("INSERT INTO db_file_data (zoneid, name, partidx, data) VALUES (?1, ?2, 0, ?3)", params![ZONE, NAME, content])
        .unwrap();
    // Filling bytes the file already claimed is a rewrite, not an append.
    assert_eq!(rev(&fs), before + 1);
    assert_eq!(counted(&fs.init_line_counter(ZONE, NAME).unwrap()).1, 3);
}

#[test]
fn an_older_builds_append_order_does_not_bump_rev() {
    // Older builds append by writing the parts, then raising `size` — the
    // new part starts at the old size, so it is an append, not a fill.
    let fs = mem();
    fs.make_file(ZONE, NAME, FileMeta::new(), FileOpts::default()).unwrap();
    fs.append_lines(ZONE, NAME, b"a\n").unwrap();
    old_build_append(&fs, b"b\n");
    {
        // A new part: starts exactly at the file's size.
        let conn = fs.conn().lock().unwrap();
        conn.execute("UPDATE db_wave_file SET size = 65536, modts = modts + 1 WHERE zoneid = ?1 AND name = ?2", params![ZONE, NAME]).unwrap();
        conn.execute("REPLACE INTO db_file_data (zoneid, name, partidx, data) VALUES (?1, ?2, 0, ?3)", params![ZONE, NAME, vec![b'x'; 65536]]).unwrap();
    }
    let rev_before = rev(&fs);
    {
        let conn = fs.conn().lock().unwrap();
        conn.execute("INSERT INTO db_file_data (zoneid, name, partidx, data) VALUES (?1, ?2, 1, ?3)", params![ZONE, NAME, b"y\n".to_vec()]).unwrap();
        conn.execute("UPDATE db_wave_file SET size = 65538 WHERE zoneid = ?1 AND name = ?2", params![ZONE, NAME]).unwrap();
    }
    assert_eq!(rev(&fs), rev_before, "a new part at the old size is an append");
}

#[test]
fn a_stamped_append_writes_the_line_and_its_stamp_together() {
    let fs = mem();
    fs.make_file(ZONE, NAME, FileMeta::new(), FileOpts::default()).unwrap();
    fs.append_lines(ZONE, NAME, b"{\"a\":1}\n").unwrap();
    let p = fs.append_lines_stamped(ZONE, NAME, b"{\"b\":2}\n", "output.tsidx").unwrap();
    assert_eq!((p.offset, pos(&p)), (8, (1, 2)));
    let stamps = fs.read_file(ZONE, "output.tsidx").unwrap().unwrap();
    let stamp: serde_json::Value = serde_json::from_slice(&stamps[..stamps.len() - 1]).unwrap();
    assert_eq!(stamp["off"], serde_json::json!(8));
    // Nothing appended: no stamp.
    fs.append_lines_stamped(ZONE, NAME, b"\n \n", "output.tsidx").unwrap();
    assert_eq!(fs.read_file(ZONE, "output.tsidx").unwrap().unwrap(), stamps);

    // If the stamp can't be written, the line isn't either.
    fs.conn()
        .lock()
        .unwrap()
        .execute_batch(
            "CREATE TRIGGER no_stamp BEFORE INSERT ON db_file_data WHEN NEW.name = 'output.tsidx'
             BEGIN SELECT RAISE(ABORT, 'injected'); END;",
        )
        .unwrap();
    let before = fs.line_state(ZONE, NAME).unwrap().unwrap();
    assert!(fs.append_lines_stamped(ZONE, NAME, b"{\"c\":3}\n", "output.tsidx").is_err());
    assert_eq!(fs.line_state(ZONE, NAME).unwrap().unwrap(), before);
}

#[test]
fn init_counts_a_file_larger_than_one_scan_window() {
    let fs = mem();
    fs.make_file(ZONE, NAME, FileMeta::new(), FileOpts::default()).unwrap();
    // 3 MB of lines of assorted lengths, some spanning scan windows.
    let mut expected = 0u64;
    for i in 0..6000u64 {
        let line = format!("{i}:{}\n", "y".repeat((i * 97 % 900) as usize));
        fs.append_data(ZONE, NAME, line.as_bytes()).unwrap();
        expected += 1;
    }
    drop_epoch(&fs);
    let (_, lines) = counted(&fs.init_line_counter(ZONE, NAME).unwrap());
    assert_eq!(lines, expected);
}

#[test]
fn concurrent_line_appends_from_two_stores_get_distinct_lines_that_address_them() {
    const WRITERS: usize = 6;
    const LINES: usize = 80;
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("filestore.db");
    let stores: Vec<Arc<FileStore>> = (0..2).map(|_| Arc::new(FileStore::open(Path::new(&path)).unwrap())).collect();
    stores[0].make_file(ZONE, NAME, FileMeta::new(), FileOpts::default()).unwrap();
    let start = Arc::new(Barrier::new(WRITERS));
    let results: Vec<(String, AppendPos)> = (0..WRITERS)
        .map(|w| {
            let store = Arc::clone(&stores[w % 2]);
            let start = Arc::clone(&start);
            std::thread::spawn(move || {
                start.wait();
                (0..LINES)
                    .map(|i| {
                        let line = format!("{{\"w\":{w},\"i\":{i}}}");
                        let p = store.append_lines(ZONE, NAME, format!("{line}\n").as_bytes()).unwrap();
                        (line, p)
                    })
                    .collect::<Vec<_>>()
            })
        })
        .collect::<Vec<_>>()
        .into_iter()
        .flat_map(|h| h.join().unwrap())
        .collect();

    // Read through a store with no cached row: a store's `stat` cache doesn't
    // see another connection's appends (5a-3 reads positions from the
    // database for that reason).
    let lines = reader_lines(&FileStore::open(Path::new(&path)).unwrap());
    assert_eq!(lines.len(), WRITERS * LINES);
    let gen = counted(&stores[1].line_state(ZONE, NAME).unwrap()).0;
    let mut seen = HashSet::new();
    for (line, p) in &results {
        let c = p.counted.as_ref().unwrap();
        assert_eq!(c.gen, gen);
        assert_eq!(c.lines, c.first_line + 1);
        assert!(seen.insert(c.first_line), "line index {} handed out twice", c.first_line);
        assert_eq!(&lines[c.first_line as usize], line, "index {} must address the record appended there", c.first_line);
    }
}

#[derive(Debug, Clone)]
enum MixedOp {
    /// This build appends raw bytes.
    New(Vec<u8>),
    /// This build appends transcript lines.
    NewLines(Vec<u8>),
    /// An older build appends raw bytes.
    Old(Vec<u8>),
    /// A reader counts (`line_count`).
    Init,
}

fn arb_chunk() -> impl Strategy<Value = Vec<u8>> {
    prop::collection::vec(
        prop_oneof![4 => Just(b'\n'), 2 => Just(b' '), 1 => Just(b'\r'), 1 => Just(0xe3u8), 1 => Just(0x80u8), 6 => b'a'..=b'z'],
        0..60,
    )
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 64, ..ProptestConfig::default() })]

    /// Builds mixed on one file, appending only. Whenever the file is
    /// counted its count is the reader's, and a line index never names
    /// different content under the same generation — the guarantee the pane
    /// relies on (Codex on #3663: catching up must not keep a generation
    /// whose counted open line it changes). The one exception is this build's
    /// own raw append continuing the open last line: by design it reports
    /// that line as the one it starts in.
    #[test]
    fn mixed_build_appends_never_rename_a_line_under_one_generation(ops in prop::collection::vec(
        prop_oneof![
            arb_chunk().prop_map(MixedOp::New),
            arb_chunk().prop_map(MixedOp::NewLines),
            arb_chunk().prop_map(MixedOp::Old),
            Just(MixedOp::Init),
        ],
        1..24,
    )) {
        let fs = mem();
        fs.make_file(ZONE, NAME, FileMeta::new(), FileOpts::default()).unwrap();
        // (gen, index) -> the content it named when last seen.
        let mut named: std::collections::HashMap<(String, usize), String> = std::collections::HashMap::new();
        let mut gens = HashSet::new();
        for op in &ops {
            // The open (unterminated, counted) last line before this op, if any.
            let before = reader_lines(&fs);
            let size = fs.line_state(ZONE, NAME).unwrap().unwrap().size;
            let ends_open = size > 0 && fs.read_bytes_db(ZONE, NAME, size - 1, 1).unwrap()[0] != b'\n';
            let open_index = (ends_open && !before.is_empty()).then(|| before.len() - 1);
            match op {
                MixedOp::New(b) => { fs.append_data_pos(ZONE, NAME, b).unwrap(); }
                MixedOp::NewLines(b) => { fs.append_lines(ZONE, NAME, b).unwrap(); }
                MixedOp::Old(b) => old_build_append(&fs, b),
                MixedOp::Init => { fs.init_line_counter(ZONE, NAME).unwrap(); }
            }
            let Some(c) = fs.line_state(ZONE, NAME).unwrap().unwrap().counted else { continue };
            let lines = reader_lines(&fs);
            prop_assert_eq!(c.lines, lines.len() as u64);
            gens.insert(c.gen.clone());
            for (i, line) in lines.iter().enumerate() {
                let key = (c.gen.clone(), i);
                if let Some(prev) = named.get(&key) {
                    let by_design = matches!(op, MixedOp::New(_)) && open_index.map_or(false, |k| i >= k);
                    // The reader (`str::lines`) keeps a trailing `\r` on an
                    // unterminated last line and drops it once a `\n` closes
                    // the line (CRLF): the same record, read two ways.
                    let same = prev.trim_end_matches('\r') == line.trim_end_matches('\r');
                    prop_assert!(same || by_design, "gen {} line {}: {:?} became {:?} after {:?}", c.gen, i, prev, line, op);
                }
                named.insert(key, line.clone());
            }
        }
        let (_, lines) = counted(&fs.init_line_counter(ZONE, NAME).unwrap());
        prop_assert_eq!(lines, reader_lines(&fs).len() as u64);
    }

    /// Appends that never continue a counted open line (complete lines, as
    /// transcripts are written): one generation throughout.
    #[test]
    fn whole_line_appends_from_mixed_builds_keep_one_generation(ops in prop::collection::vec(
        prop_oneof![
            arb_chunk().prop_map(|b| MixedOp::NewLines(b)),
            arb_chunk().prop_map(|mut b| { b.push(b'\n'); MixedOp::Old(b) }),
            Just(MixedOp::Init),
        ],
        1..24,
    )) {
        let fs = mem();
        fs.make_file(ZONE, NAME, FileMeta::new(), FileOpts::default()).unwrap();
        let (g0, _) = counted(&fs.line_state(ZONE, NAME).unwrap());
        for op in &ops {
            match op {
                MixedOp::NewLines(b) => { fs.append_lines(ZONE, NAME, b).unwrap(); }
                MixedOp::Old(b) => old_build_append(&fs, b),
                MixedOp::Init => { fs.init_line_counter(ZONE, NAME).unwrap(); }
                MixedOp::New(_) => unreachable!(),
            }
            if let Some(c) = fs.line_state(ZONE, NAME).unwrap().unwrap().counted {
                prop_assert_eq!(&c.gen, &g0);
                prop_assert_eq!(c.lines, reader_lines(&fs).len() as u64);
            }
        }
        prop_assert_eq!(counted(&fs.init_line_counter(ZONE, NAME).unwrap()).0, g0);
    }

    /// Raw appends of arbitrary bytes keep the counter equal to the
    /// `output.idx` indexer's count, the addressing reads use.
    #[test]
    fn counter_equals_the_indexer(chunks in prop::collection::vec(
        prop::collection::vec(prop_oneof![4 => Just(b'\n'), 2 => Just(b' '), 1 => Just(b'\r'),
            1 => Just(0x0bu8), 1 => Just(0xe3u8), 1 => Just(0x80u8), 6 => b'a'..=b'z'], 0..300),
        1..10,
    )) {
        let fs = mem();
        fs.make_file(ZONE, NAME, FileMeta::new(), FileOpts::default()).unwrap();
        for chunk in &chunks {
            fs.append_data(ZONE, NAME, chunk).unwrap();
            let state = fs.line_state(ZONE, NAME).unwrap().unwrap();
            let indexed = rebuild_output_idx(&fs, ZONE, state.size as u64, state.counted.as_ref().map(|c| c.gen.clone())).unwrap();
            prop_assert_eq!(state.counted.unwrap().lines, indexed);
        }
    }
}

#[test]
fn a_catch_up_never_scans_a_file_it_cannot_catch_up() {
    use super::counter::{InitScan, ScanFrom};
    // ReAgent on #3663: behind when first seen, rewritten before the scan
    // reads the row. Catch-up must not fall back to a whole-file count; the
    // decision is made on the row the scan itself reads.
    let fs = mem();
    fs.make_file(ZONE, NAME, FileMeta::new(), FileOpts::default()).unwrap();
    fs.append_lines(ZONE, NAME, b"aaaa\nbbbb\n").unwrap();
    old_build_append(&fs, b"cccc\n");
    fs.write_at(ZONE, NAME, 0, b"x").unwrap(); // a rewrite: `rev` moves
    assert!(matches!(fs.init_scan_from(ZONE, NAME, ScanFrom::CatchUpOnly).unwrap(), InitScan::Done(_)), "must not scan");
    let state = fs.catch_up_line_counter(ZONE, NAME).unwrap().unwrap();
    assert_eq!(state.counted, None, "nothing to catch up, and no full count started");
    assert_eq!(fs.line_state(ZONE, NAME).unwrap().unwrap().counted, None);
    // A reader that may pay for it (line_count) still counts it.
    assert_eq!(counted(&fs.init_line_counter(ZONE, NAME).unwrap()).1, 3);

    // Legacy: no epoch at all — the same.
    let legacy = mem();
    uncounted(&legacy, b"a\nb\n");
    assert!(matches!(legacy.init_scan_from(ZONE, NAME, ScanFrom::CatchUpOnly).unwrap(), InitScan::Done(_)));
    assert_eq!(legacy.catch_up_line_counter(ZONE, NAME).unwrap().unwrap().counted, None);
}

#[test]
fn a_catch_up_on_a_read_path_keeps_the_generation() {
    let fs = mem();
    fs.make_file(ZONE, NAME, FileMeta::new(), FileOpts::default()).unwrap();
    fs.append_lines(ZONE, NAME, b"a\n").unwrap();
    let (g0, _) = counted(&fs.line_state(ZONE, NAME).unwrap());
    old_build_append(&fs, b"b\nc\n");
    assert_eq!(counted(&fs.catch_up_line_counter(ZONE, NAME).unwrap()), (g0.clone(), 3));
    // Already current: returned as is.
    assert_eq!(counted(&fs.catch_up_line_counter(ZONE, NAME).unwrap()), (g0, 3));
    assert_eq!(fs.catch_up_line_counter(ZONE, "missing").unwrap(), None);
}
