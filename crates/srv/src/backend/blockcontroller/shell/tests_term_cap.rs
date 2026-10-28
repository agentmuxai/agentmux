// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The cap on a terminal's stored output. The cap is passed in rather than set
//! through the process-wide setting, so these tests don't disturb each other or
//! the shell tests that write `term` files.

use super::*;
use crate::backend::storage::filestore::FileMeta;

const MB: i64 = MIB;

/// Append `chunks` of `chunk` bytes the way the PTY path does, capping after each.
fn flood(fs: &FileStore, block: &str, chunks: usize, chunk: usize, cap: i64) -> usize {
    let data = vec![b'y'; chunk];
    let mut notices = 0;
    for _ in 0..chunks {
        let at = fs.append_data_at(block, TERM_FILE, &data).unwrap();
        if after_append_capped(fs, block, at, &data, cap) {
            notices += 1;
        }
    }
    notices
}

fn stored_bytes(fs: &FileStore, block: &str) -> i64 {
    fs.read_file(block, TERM_FILE).unwrap().map(|b| b.len() as i64).unwrap_or(0)
}

#[test]
fn a_flood_is_stored_up_to_the_cap_and_no_further() {
    let fs = FileStore::open_in_memory().unwrap();
    let cap = 2 * MB;
    fs.make_file("b", TERM_FILE, FileMeta::new(), opts_for(cap)).unwrap();
    // 40 MB in 256 KiB batches, the size the PTY path writes under load.
    let notices = flood(&fs, "b", 160, 256 * 1024, cap);
    assert_eq!(notices, 1, "the first drop is announced once");
    let file = fs.stat("b", TERM_FILE).unwrap().unwrap();
    assert_eq!(file.size, 160 * 256 * 1024, "offsets are absolute");
    assert!(stored_bytes(&fs, "b") <= cap, "the window is the cap");
}

#[test]
fn nothing_is_dropped_or_announced_under_the_cap() {
    let fs = FileStore::open_in_memory().unwrap();
    fs.make_file("b", TERM_FILE, FileMeta::new(), opts_for(8 * MB)).unwrap();
    assert_eq!(flood(&fs, "b", 8, 256 * 1024, 8 * MB), 0);
    assert_eq!(stored_bytes(&fs, "b"), 8 * 256 * 1024);
}

#[test]
fn the_trimmed_notice_is_announced_once_per_file() {
    let fs = FileStore::open_in_memory().unwrap();
    fs.make_file("b", TERM_FILE, FileMeta::new(), opts_for(MB)).unwrap();
    assert_eq!(flood(&fs, "b", 40, 256 * 1024, MB), 1);
    assert_eq!(flood(&fs, "b", 40, 256 * 1024, MB), 0, "already announced");
    assert_eq!(fs.stat("b", TERM_FILE).unwrap().unwrap().meta.get("trimmed"), Some(&serde_json::Value::Bool(true)));
    // Another pane's file has its own notice.
    fs.make_file("c", TERM_FILE, FileMeta::new(), opts_for(MB)).unwrap();
    assert_eq!(flood(&fs, "c", 40, 256 * 1024, MB), 1);
}

#[test]
fn a_file_stored_before_the_cap_is_brought_to_it_on_its_next_write() {
    let fs = FileStore::open_in_memory().unwrap();
    fs.make_file("b", TERM_FILE, FileMeta::new(), FileOpts::default()).unwrap();
    for _ in 0..64 {
        fs.append_data_at("b", TERM_FILE, &vec![b'z'; 256 * 1024]).unwrap();
    }
    assert_eq!(stored_bytes(&fs, "b"), 16 * MB, "stored whole, as before the cap");
    let at = fs.append_data_at("b", TERM_FILE, b"more").unwrap();
    assert!(after_append_capped(&fs, "b", at, b"more", 4 * MB), "converting drops output, so it is announced");
    assert!(stored_bytes(&fs, "b") <= 4 * MB);
    assert!(fs.stat("b", TERM_FILE).unwrap().unwrap().opts.circular);
    assert!(fs.read_file("b", TERM_FILE).unwrap().unwrap().ends_with(b"more"));
}

#[test]
fn zero_lifts_the_limit_and_a_limited_file_is_released() {
    let fs = FileStore::open_in_memory().unwrap();
    fs.make_file("b", TERM_FILE, FileMeta::new(), opts_for(MB)).unwrap();
    let at = fs.append_data_at("b", TERM_FILE, b"x").unwrap();
    assert!(!after_append_capped(&fs, "b", at, b"x", 0));
    let file = fs.stat("b", TERM_FILE).unwrap().unwrap();
    assert!(!file.opts.circular && file.opts.maxsize == 0);
    assert_eq!(flood(&fs, "b", 20, 256 * 1024, 0), 0);
    assert_eq!(stored_bytes(&fs, "b"), 20 * 256 * 1024 + 1, "no cap, nothing dropped");
}

#[test]
fn the_setting_is_whole_megabytes_with_50_as_the_default() {
    use serde_json::json;
    assert_eq!(cap_from_setting(None), 50 * MB);
    assert_eq!(cap_from_setting(Some(&json!(8))), 8 * MB);
    assert_eq!(cap_from_setting(Some(&json!(0))), 0, "0 is no limit");
    assert_eq!(cap_from_setting(Some(&json!(-3))), 50 * MB);
    assert_eq!(cap_from_setting(Some(&json!("big"))), 50 * MB);
    assert_eq!(cap_from_setting(Some(&json!(1_000_000_000))), MAX_MB * MB, "a typo is held to a ceiling");
    assert_eq!(opts_for(0).circular, false);
    assert_eq!(opts_for(8 * MB).maxsize, 8 * MB);
}

#[test]
fn the_pty_write_path_makes_the_file_circular() {
    use crate::backend::mps;
    let broker = mps::Broker::new();
    let fs = std::sync::Arc::new(FileStore::open_in_memory().unwrap());
    super::super::file_ops::handle_append_block_file(&broker, "blk", TERM_FILE, b"hello", Some(&fs), None);
    let file = fs.stat("blk", TERM_FILE).unwrap().unwrap();
    assert_eq!(file.opts.circular, cap_bytes() > 0);
    assert_eq!(fs.read_file("blk", TERM_FILE).unwrap().unwrap(), b"hello");
}

/// A flood against a file-backed store in batches of `chunk` bytes (the PTY
/// path's batch is anywhere from one 4 KiB read to 256 KiB, whatever the
/// flusher found queued): the largest the log was seen to be (sampled every
/// half megabyte written), and where the log and the database ended, in MB.
fn flood_file_backed(chunks: usize, chunk: usize, cap: i64) -> (u64, u64, u64) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("filestore.db");
    let fs = FileStore::open(&db).unwrap();
    fs.make_file("b", TERM_FILE, FileMeta::new(), opts_for(cap)).unwrap();
    let data = vec![b'y'; chunk];
    let size = |n: &str| std::fs::metadata(dir.path().join(n)).map(|m| m.len()).unwrap_or(0);
    let every = (512 * 1024 / chunk).max(1);
    let mut peak = 0;
    for i in 0..chunks {
        let at = fs.append_data_at("b", TERM_FILE, &data).unwrap();
        after_append_capped(&fs, "b", at, &data, cap);
        if i % every == 0 {
            peak = peak.max(size("filestore.db-wal"));
        }
    }
    (peak >> 20, size("filestore.db-wal") >> 20, size("filestore.db") >> 20)
}

#[test]
fn a_flood_leaves_the_database_at_the_cap_and_the_log_short() {
    // 200 MB at a 16 MB cap. Before the writer checkpointed between writes the
    // log held every byte written: a flood that never pauses is never found
    // fully copied back, so the log never restarted.
    let (peak_wal, end_wal, db) = flood_file_backed(800, 256 * 1024, 16 * MB);
    assert!(peak_wal < 64, "the log reached {peak_wal} MB");
    assert!(end_wal < 64, "the log ended at {end_wal} MB");
    assert!(db < 40, "the database ended at {db} MB for a 16 MB cap");
}

/// A measurement, not a check. `MEASURE_MB` written (400 by default) in batches
/// of `MEASURE_CHUNK_KB` (256), `MEASURE_CAP_MB` (50). Run with `--ignored
/// --nocapture`.
#[test]
#[ignore]
fn measure_a_flood_against_a_file_backed_store() {
    let env = |k: &str, d: usize| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
    let chunk = env("MEASURE_CHUNK_KB", 256) * 1024;
    let started = std::time::Instant::now();
    let (peak, end, db) = flood_file_backed(env("MEASURE_MB", 400) * 1024 * 1024 / chunk, chunk, env("MEASURE_CAP_MB", 50) as i64 * MB);
    println!("flood written in {:?}; db {db} MB, log {end} MB at the end, {peak} MB at most", started.elapsed());
}

#[test]
fn text_may_follow_output_that_ends_between_whole_things() {
    assert!(ends_at_boundary(b""));
    assert!(ends_at_boundary(b"plain\r\n"));
    assert!(ends_at_boundary("caf\u{e9}".as_bytes()));
    assert!(ends_at_boundary("\u{1f600}".as_bytes()));
    assert!(ends_at_boundary(b"\x1b[31mred\x1b[0m"), "a finished CSI");
    assert!(ends_at_boundary(b"\x1b]0;title\x07"), "an OSC ended by BEL");
    assert!(ends_at_boundary(b"\x1b]0;title\x1b\\"), "an OSC ended by ST");
    assert!(ends_at_boundary(b"\x1b7"), "a two-byte escape");
    assert!(ends_at_boundary(b"\x1b(B"), "a charset escape with its final byte");
}

#[test]
fn text_may_not_follow_output_that_stops_inside_a_character_or_an_escape() {
    let e_acute = "\u{e9}".as_bytes();
    assert!(!ends_at_boundary(&e_acute[..1]), "half of a two-byte character");
    let smile = "\u{1f600}".as_bytes();
    for cut in 1..4 {
        assert!(!ends_at_boundary(&smile[..cut]), "{cut} of 4 bytes");
    }
    assert!(!ends_at_boundary(b"text\x1b"), "a lone escape");
    assert!(!ends_at_boundary(b"text\x1b[31"), "an unfinished CSI");
    assert!(!ends_at_boundary(b"\x1b[38;5;2"), "an unfinished CSI with parameters");
    assert!(!ends_at_boundary(b"\x1b]0;a title"), "an OSC with no end yet");
    assert!(!ends_at_boundary(b"\x1b("), "a charset escape missing its final byte");
}

#[test]
fn the_notice_waits_for_a_boundary_and_is_then_written_once() {
    let fs = FileStore::open_in_memory().unwrap();
    fs.make_file("b", TERM_FILE, FileMeta::new(), opts_for(MB)).unwrap();
    let write = |bytes: &[u8]| {
        let at = fs.append_data_at("b", TERM_FILE, bytes).unwrap();
        after_append_capped(&fs, "b", at, bytes, MB)
    };
    // Fill past the cap with batches that each end at a boundary but one, and
    // make the batch that first drops output end in the middle of an escape.
    let filler = vec![b'y'; 256 * 1024];
    for _ in 0..4 {
        assert!(!write(&filler));
    }
    let mut cut = filler.clone();
    cut.extend_from_slice(b"\x1b[38;5;");
    assert!(!write(&cut), "output was dropped but the batch ends inside an escape: wait");
    assert_eq!(fs.stat("b", TERM_FILE).unwrap().unwrap().meta.get("trimmed"), None, "not yet announced");
    assert!(write(b"2mgreen\x1b[0m\r\n"), "the next append ends cleanly: announced now");
    assert!(!write(b"after\r\n"), "and only once");
}

#[test]
fn converting_or_widening_a_file_that_lost_nothing_announces_nothing() {
    let fs = FileStore::open_in_memory().unwrap();
    // Stored before the cap, and far under it.
    fs.make_file("b", TERM_FILE, FileMeta::new(), FileOpts::default()).unwrap();
    let at = fs.append_data_at("b", TERM_FILE, b"a few lines\r\n").unwrap();
    assert!(!after_append_capped(&fs, "b", at, b"a few lines\r\n", 50 * MB), "converted, nothing dropped");
    assert!(fs.stat("b", TERM_FILE).unwrap().unwrap().opts.circular);
    assert!(fs.stat("b", TERM_FILE).unwrap().unwrap().meta.get("trimmed").is_none());

    // The setting raised from 50 to 100 MB: the options change, the data doesn't.
    let at = fs.append_data_at("b", TERM_FILE, b"more\r\n").unwrap();
    assert!(!after_append_capped(&fs, "b", at, b"more\r\n", 100 * MB));
    assert_eq!(fs.stat("b", TERM_FILE).unwrap().unwrap().opts.maxsize, 100 * MB);
}

#[test]
fn the_writer_checkpoints_by_the_log_it_has_written_not_the_output() {
    let due_after = |batch: usize| {
        let pending = AtomicI64::new(0);
        let mut at = 0i64;
        let mut appends = 0;
        loop {
            appends += 1;
            let due = relief_due(&pending, at, batch);
            at += batch as i64;
            if due {
                return (appends, at);
            }
        }
    };
    // 256 KB batches write about what they hold: a pass every ~8 MB of output.
    let (_, output) = due_after(256 * 1024);
    assert!((6 * MB..=9 * MB).contains(&output), "{output} bytes of output");
    // 4 KB batches rewrite a 64 KB part each: a pass after 128 of them, 512 KB of output.
    let (appends, output) = due_after(4096);
    assert!((120..=140).contains(&appends) && output <= MB, "{appends} appends, {output} bytes of output");
    // The count starts again after a pass.
    let pending = AtomicI64::new(0);
    assert!(relief_due(&pending, 0, 9 * MB as usize));
    assert!(!relief_due(&pending, 0, 4096));
}
