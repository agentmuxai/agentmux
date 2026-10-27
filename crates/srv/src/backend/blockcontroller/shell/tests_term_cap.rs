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
        if after_append_capped(fs, block, at, data.len(), cap) {
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
    assert!(after_append_capped(&fs, "b", at, 4, 4 * MB), "converting drops output, so it is announced");
    assert!(stored_bytes(&fs, "b") <= 4 * MB);
    assert!(fs.stat("b", TERM_FILE).unwrap().unwrap().opts.circular);
    assert!(fs.read_file("b", TERM_FILE).unwrap().unwrap().ends_with(b"more"));
}

#[test]
fn zero_lifts_the_limit_and_a_limited_file_is_released() {
    let fs = FileStore::open_in_memory().unwrap();
    fs.make_file("b", TERM_FILE, FileMeta::new(), opts_for(MB)).unwrap();
    let at = fs.append_data_at("b", TERM_FILE, b"x").unwrap();
    assert!(!after_append_capped(&fs, "b", at, 1, 0));
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

/// A flood against a file-backed store in the PTY path's batch size: the
/// largest the log was seen to be (sampled every few writes), and where the
/// log and the database ended, in MB.
fn flood_file_backed(chunks: usize, cap: i64) -> (u64, u64, u64) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("filestore.db");
    let fs = FileStore::open(&db).unwrap();
    fs.make_file("b", TERM_FILE, FileMeta::new(), opts_for(cap)).unwrap();
    let data = vec![b'y'; 256 * 1024];
    let size = |n: &str| std::fs::metadata(dir.path().join(n)).map(|m| m.len()).unwrap_or(0);
    let mut peak = 0;
    for i in 0..chunks {
        let at = fs.append_data_at("b", TERM_FILE, &data).unwrap();
        after_append_capped(&fs, "b", at, data.len(), cap);
        if i % 8 == 0 {
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
    let (peak_wal, end_wal, db) = flood_file_backed(800, 16 * MB);
    assert!(peak_wal < 64, "the log reached {peak_wal} MB");
    assert!(end_wal < 64, "the log ended at {end_wal} MB");
    assert!(db < 40, "the database ended at {db} MB for a 16 MB cap");
}

/// A measurement, not a check. `MEASURE_CHUNKS` of 256 KiB (1600 by default),
/// `MEASURE_CAP_MB` (50). Run with `--ignored --nocapture`.
#[test]
#[ignore]
fn measure_a_flood_against_a_file_backed_store() {
    let env = |k: &str, d: usize| std::env::var(k).ok().and_then(|v| v.parse().ok()).unwrap_or(d);
    let started = std::time::Instant::now();
    let (peak, end, db) = flood_file_backed(env("MEASURE_CHUNKS", 1600), env("MEASURE_CAP_MB", 50) as i64 * MB);
    println!("flood written in {:?}; db {db} MB, log {end} MB at the end, {peak} MB at most", started.elapsed());
}
