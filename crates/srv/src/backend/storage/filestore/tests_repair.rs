// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Rebuilding a damaged shared store (`repair.rs`): what can be read is kept, what cannot is named,
//! the damaged original is never deleted, and every interrupted state comes right.

use std::path::{Path, PathBuf};

use super::frames::{FrameOutcome, CODEC_ZSTD, FRAME_PARTS};
use super::repair::{self, Outcome, Paths};
use super::{FileMeta, FileOpts, FileStore};

const N: &str = "output";
const ZONES: [&str; 3] = ["agent:a:current", "agent:b:current", "agent:c:current"];

fn transcript(bytes: usize, seed: u64) -> Vec<u8> {
    let mut out = Vec::with_capacity(bytes + 1024);
    let mut x = seed;
    let mut n = 0u64;
    while out.len() < bytes {
        x = x.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let body: String = (0..(40 + (x >> 40) as usize % 700)).map(|i| (b'a' + ((x >> (i % 48)) as u8 % 26)) as char).collect();
        out.extend_from_slice(format!("{{\"n\":{n},\"text\":\"{body}\"}}\n").as_bytes());
        n += 1;
    }
    out
}

/// A shared store with three transcripts, the first framed. Returns its path and what each reads.
fn store(dir: &Path) -> (PathBuf, Vec<Vec<u8>>) {
    let path = dir.join("filestore.db");
    let fs = FileStore::open_global_transcripts(&path).unwrap();
    let mut kept = Vec::new();
    for (i, zone) in ZONES.iter().enumerate() {
        let data = transcript(40 * 65536 + 777, i as u64 + 1);
        fs.make_file(zone, N, FileMeta::new(), FileOpts::default()).unwrap();
        fs.append_lines(zone, N, &data).unwrap();
        kept.push(data);
    }
    assert!(matches!(fs.frame_parts(ZONES[0], N, 0, FRAME_PARTS, CODEC_ZSTD).unwrap(), FrameOutcome::Framed { .. }));
    fs.checkpoint_truncate_apart().unwrap();
    (path, kept)
}

/// Overwrite the frame table's root page, as a cross-linked page looks to a query that walks it.
fn damage_the_frame_table(path: &Path) {
    use std::io::{Seek, SeekFrom, Write};
    let root: i64 = {
        let c = rusqlite::Connection::open(path).unwrap();
        c.query_row("SELECT rootpage FROM sqlite_master WHERE name = 'db_file_frame'", [], |r| r.get(0)).unwrap()
    };
    let mut f = std::fs::OpenOptions::new().write(true).open(path).unwrap();
    f.seek(SeekFrom::Start((root as u64 - 1) * 4096)).unwrap();
    f.write_all(&[0xFFu8; 4096]).unwrap();
}

fn checks_out(path: &Path) -> bool {
    let c = rusqlite::Connection::open(path).unwrap();
    repair::check(&c).is_ok()
}

fn alone() -> Option<String> {
    None
}

#[test]
fn a_store_that_passes_its_check_is_left_as_it_is() {
    let dir = tempfile::tempdir().unwrap();
    let (path, _) = store(dir.path());
    assert_eq!(repair::run_at_start(&path, alone, 1), Outcome::NotRequested);
    repair::request(&path, "test", 1).unwrap();
    let before = std::fs::read(&path).unwrap();
    assert_eq!(repair::run_at_start(&path, alone, 2), Outcome::NotNeeded);
    assert_eq!(std::fs::read(&path).unwrap(), before, "not a byte of it written");
    assert!(!repair::requested(&path), "the request is dropped");
}

#[test]
fn a_damaged_store_is_rebuilt_keeping_what_reads_and_naming_what_does_not() {
    let dir = tempfile::tempdir().unwrap();
    let (path, kept) = store(dir.path());
    damage_the_frame_table(&path);
    assert!(!checks_out(&path), "the damage is real");
    let original = std::fs::read(&path).unwrap();

    repair::request(&path, "test", 1).unwrap();
    let Outcome::Done(last) = repair::run_at_start(&path, alone, 7) else { panic!("expected Done") };

    // The damaged original is kept, whole, and the request is gone.
    assert_eq!(std::fs::read(&last.kept_as).unwrap(), original);
    assert!(last.kept_as.ends_with(".damaged-7"), "{}", last.kept_as);
    assert!(!repair::requested(&path));
    assert!(checks_out(&path), "the new store passes its check");

    // What was on the damaged page is named, with its zone; the other zones are whole.
    assert!(last.rows_lost >= 1, "{last:?}");
    assert!(last.lost.iter().all(|l| l.table == "db_file_frame" && l.zone.as_deref() == Some(ZONES[0])), "{:?}", last.lost);
    assert_eq!(last.damaged_zones, vec![ZONES[0].to_string()]);
    assert!(last.restore_pending);
    let fs = FileStore::open_global_transcripts(&path).unwrap();
    assert_eq!(fs.read_file(ZONES[1], N).unwrap().unwrap(), kept[1]);
    assert_eq!(fs.read_file(ZONES[2], N).unwrap().unwrap(), kept[2]);
    assert_eq!(fs.stat(ZONES[0], N).unwrap().unwrap().size, kept[0].len() as i64, "the file's row is kept");
    assert_eq!(repair::status(&path).1.unwrap().rows_lost, last.rows_lost);
}

#[test]
fn nothing_is_rebuilt_while_another_srv_runs_and_the_request_waits() {
    let dir = tempfile::tempdir().unwrap();
    let (path, _) = store(dir.path());
    damage_the_frame_table(&path);
    repair::request(&path, "test", 1).unwrap();
    let other = || Some("agentmux-srv (pid 7) is running".to_string());
    assert_eq!(repair::run_at_start(&path, other, 2), Outcome::Waiting("agentmux-srv (pid 7) is running".to_string()));
    assert!(repair::requested(&path), "it is asked again at the next start");
    assert!(!checks_out(&path), "the store is as it was");
    // An srv that shows up after the rebuild, before the swap, stops it too.
    let calls = std::cell::Cell::new(0);
    let late = || {
        calls.set(calls.get() + 1);
        (calls.get() > 1).then(|| "a late srv".to_string())
    };
    assert_eq!(repair::run_at_start(&path, late, 3), Outcome::Waiting("a late srv".to_string()));
    assert!(!Paths::for_db(&path).new.exists(), "the rebuilt file is removed");
    assert!(!checks_out(&path));
}

#[test]
fn a_swap_interrupted_between_its_renames_puts_the_original_back() {
    let dir = tempfile::tempdir().unwrap();
    let (path, _) = store(dir.path());
    let paths = Paths::for_db(&path);
    std::fs::rename(&path, &paths.pre).unwrap();
    std::fs::write(&paths.new, b"half a store").unwrap();
    repair::recover(&paths);
    assert!(path.exists() && !paths.pre.exists() && !paths.new.exists());
    assert!(checks_out(&path));
}

#[test]
fn a_swap_that_finished_keeps_the_original_where_a_repair_keeps_it() {
    let dir = tempfile::tempdir().unwrap();
    let (path, _) = store(dir.path());
    let paths = Paths::for_db(&path);
    std::fs::copy(&path, &paths.pre).unwrap();
    repair::recover(&paths);
    assert!(path.exists() && !paths.pre.exists());
    assert!(std::fs::read_dir(dir.path()).unwrap().flatten().any(|e| e.file_name().to_string_lossy().contains(".damaged-")));
}
