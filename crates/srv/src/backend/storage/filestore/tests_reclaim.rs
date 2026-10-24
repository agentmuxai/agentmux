// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Giving disk space back (`reclaim.rs`): the rewrite keeps every byte and
//! frame, swaps only when alone, and every interrupted state comes right.

use std::path::{Path, PathBuf};

use super::frames::{FrameOutcome, CODEC_ZSTD, FRAME_PARTS};
use super::reclaim::{self, Outcome, Paths, Recovered};
use super::{FileMeta, FileOpts, FileStore};

const N: &str = "output";

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

fn read_all(fs: &FileStore, zone: &str) -> Vec<u8> {
    fs.read_file(zone, N).unwrap().unwrap()
}

/// A shared store with three transcripts, the first framed, and a fourth
/// deleted so the file holds free pages. Returns the store's path and what
/// each kept transcript reads.
fn store_with_free_pages(dir: &Path) -> (PathBuf, Vec<(String, Vec<u8>)>) {
    let path = dir.join("filestore.db");
    let fs = FileStore::open_global_transcripts(&path).unwrap();
    let mut kept = Vec::new();
    for (i, zone) in ["agent:a:current", "agent:b:current", "agent:c:current", "agent:gone:current"].iter().enumerate() {
        let data = transcript(40 * 65536 + 777, i as u64 + 1);
        fs.make_file(zone, N, FileMeta::new(), FileOpts::default()).unwrap();
        fs.append_lines(zone, N, &data).unwrap();
        if i < 3 {
            kept.push((zone.to_string(), data));
        }
    }
    assert!(matches!(fs.frame_parts("agent:a:current", N, 0, FRAME_PARTS, CODEC_ZSTD).unwrap(), FrameOutcome::Framed { .. }));
    fs.delete_zone("agent:gone:current").unwrap();
    fs.checkpoint_truncate_apart().unwrap();
    let (_, _, free, auto_vacuum) = fs.page_stats().unwrap();
    assert!(free > 500, "the delete left free pages ({free})");
    assert_eq!(auto_vacuum, 0);
    (path, kept)
}

fn alone() -> Option<String> {
    None
}

#[test]
fn nothing_happens_without_a_request() {
    let dir = tempfile::tempdir().unwrap();
    let (path, _) = store_with_free_pages(dir.path());
    let before = std::fs::metadata(&path).unwrap().len();
    assert_eq!(reclaim::run_at_start(&path, alone, 1), Outcome::NotRequested);
    assert_eq!(std::fs::metadata(&path).unwrap().len(), before);
}

#[test]
fn a_reclaim_shrinks_the_file_keeps_every_byte_and_turns_on_incremental() {
    let dir = tempfile::tempdir().unwrap();
    let (path, kept) = store_with_free_pages(dir.path());
    let before = std::fs::metadata(&path).unwrap().len();
    reclaim::request(&path, "test", 1).unwrap();
    let Outcome::Done(last) = reclaim::run_at_start(&path, alone, 2) else { panic!("expected Done") };
    assert_eq!(last.before_bytes, before);
    assert!(last.after_bytes < before, "{} < {before}", last.after_bytes);
    let paths = Paths::for_db(&path);
    assert!(!paths.marker.exists() && !paths.new.exists() && !paths.pre.exists());
    assert_eq!(reclaim::status(&path).1.unwrap().after_bytes, last.after_bytes);

    let fs = FileStore::open_global_transcripts(&path).unwrap();
    for (zone, data) in &kept {
        assert_eq!(&read_all(&fs, zone), data, "{zone} reads the same");
    }
    let frames: i64 = fs.lock_conn().query_row("SELECT count(*) FROM db_file_frame", [], |r| r.get(0)).unwrap();
    assert_eq!(frames, 1, "the frame survived");
    let (_, _, free, auto_vacuum) = fs.page_stats().unwrap();
    assert_eq!((free, auto_vacuum), (0, 2));
    // Appends keep working, and line counts are what they were.
    fs.append_lines("agent:b:current", N, b"{\"after\":true}\n").unwrap();
    assert!(read_all(&fs, "agent:b:current").ends_with(b"{\"after\":true}\n"));
}

#[test]
fn another_srv_running_makes_it_wait_and_keep_the_request() {
    let dir = tempfile::tempdir().unwrap();
    let (path, _) = store_with_free_pages(dir.path());
    let before = std::fs::metadata(&path).unwrap().len();
    reclaim::request(&path, "test", 1).unwrap();
    let other = || Some("agentmux-srv (pid 7) is running".to_string());
    assert_eq!(reclaim::run_at_start(&path, other, 2), Outcome::Waiting("agentmux-srv (pid 7) is running".to_string()));
    assert_eq!(std::fs::metadata(&path).unwrap().len(), before);
    let (marker, _) = reclaim::status(&path);
    assert_eq!(marker.unwrap().waiting_for.as_deref(), Some("agentmux-srv (pid 7) is running"));
    // Alone next time: it runs.
    assert!(matches!(reclaim::run_at_start(&path, alone, 3), Outcome::Done(_)));
}

#[test]
fn an_srv_that_appears_during_the_rewrite_stops_the_swap() {
    let dir = tempfile::tempdir().unwrap();
    let (path, kept) = store_with_free_pages(dir.path());
    reclaim::request(&path, "test", 1).unwrap();
    let calls = std::cell::Cell::new(0);
    let late = || {
        calls.set(calls.get() + 1);
        (calls.get() > 1).then(|| "a late srv".to_string())
    };
    assert_eq!(reclaim::run_at_start(&path, late, 2), Outcome::Waiting("a late srv".to_string()));
    assert!(!Paths::for_db(&path).new.exists(), "the rewrite was deleted");
    let fs = FileStore::open_global_transcripts(&path).unwrap();
    assert_eq!(fs.page_stats().unwrap().3, 0, "the original is untouched");
    assert_eq!(read_all(&fs, &kept[0].0), kept[0].1);
}

#[test]
fn an_incremental_store_needs_no_reclaim() {
    let dir = tempfile::tempdir().unwrap();
    let (path, _) = store_with_free_pages(dir.path());
    reclaim::request(&path, "test", 1).unwrap();
    assert!(matches!(reclaim::run_at_start(&path, alone, 2), Outcome::Done(_)));
    reclaim::request(&path, "test", 3).unwrap();
    assert_eq!(reclaim::run_at_start(&path, alone, 4), Outcome::NotNeeded);
    assert!(!Paths::for_db(&path).marker.exists());
}

#[test]
fn cancel_withdraws_the_request() {
    let dir = tempfile::tempdir().unwrap();
    let (path, _) = store_with_free_pages(dir.path());
    reclaim::request(&path, "test", 1).unwrap();
    reclaim::cancel(&path).unwrap();
    assert_eq!(reclaim::run_at_start(&path, alone, 2), Outcome::NotRequested);
}

/// Copy the store (with no open connection) to `to`.
fn copy_store(from: &Path, to: &Path) {
    std::fs::copy(from, to).unwrap();
}

#[test]
fn an_interrupted_rewrite_is_deleted() {
    let dir = tempfile::tempdir().unwrap();
    let (path, _) = store_with_free_pages(dir.path());
    let paths = Paths::for_db(&path);
    std::fs::write(&paths.new, b"half a rewrite").unwrap();
    assert_eq!(reclaim::recover(&paths), Recovered::DroppedNew);
    assert!(!paths.new.exists() && path.exists());
}

#[test]
fn a_swap_interrupted_between_its_renames_rolls_back() {
    let dir = tempfile::tempdir().unwrap();
    let (path, kept) = store_with_free_pages(dir.path());
    let paths = Paths::for_db(&path);
    std::fs::rename(&path, &paths.pre).unwrap();
    std::fs::write(&paths.new, b"a rewrite not moved in").unwrap();
    assert_eq!(reclaim::recover(&paths), Recovered::RolledBack);
    assert!(path.exists() && !paths.pre.exists() && !paths.new.exists());
    let fs = FileStore::open_global_transcripts(&path).unwrap();
    assert_eq!(read_all(&fs, &kept[1].0), kept[1].1);
}

#[test]
fn a_finished_swap_drops_the_original_once_the_new_store_opens() {
    let dir = tempfile::tempdir().unwrap();
    let (path, _) = store_with_free_pages(dir.path());
    let paths = Paths::for_db(&path);
    copy_store(&path, &paths.pre);
    assert_eq!(reclaim::recover(&paths), Recovered::Finished);
    assert!(path.exists() && !paths.pre.exists());
}

#[test]
fn a_swapped_in_store_that_doesnt_open_is_swapped_back() {
    let dir = tempfile::tempdir().unwrap();
    let (path, kept) = store_with_free_pages(dir.path());
    let paths = Paths::for_db(&path);
    std::fs::rename(&path, &paths.pre).unwrap();
    std::fs::write(&path, b"not a database").unwrap();
    assert_eq!(reclaim::recover(&paths), Recovered::RolledBack);
    let fs = FileStore::open_global_transcripts(&path).unwrap();
    assert_eq!(read_all(&fs, &kept[2].0), kept[2].1);
}

#[test]
fn nothing_touches_a_reclaim_in_progress() {
    // Another srv holds the gate exclusively, as a reclaim does for its whole
    // run: a starting srv must leave its rewrite and its moved-aside original alone.
    let dir = tempfile::tempdir().unwrap();
    let (path, _) = store_with_free_pages(dir.path());
    let paths = Paths::for_db(&path);
    std::fs::write(&paths.new, b"a rewrite in progress").unwrap();
    let held = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(&paths.gate).unwrap();
    held.lock().unwrap();
    assert_eq!(reclaim::recover_if_alone(&paths), Recovered::Nothing);
    reclaim::request(&path, "test", 1).unwrap();
    assert!(matches!(reclaim::run_at_start(&path, alone, 2), Outcome::Waiting(_)));
    assert!(paths.new.exists(), "the other srv's rewrite is untouched");
    drop(held);
    assert_eq!(reclaim::recover_if_alone(&paths), Recovered::DroppedNew);
}

#[test]
fn incremental_steps_give_freed_pages_back() {
    let dir = tempfile::tempdir().unwrap();
    let (path, kept) = store_with_free_pages(dir.path());
    reclaim::request(&path, "test", 1).unwrap();
    assert!(matches!(reclaim::run_at_start(&path, alone, 2), Outcome::Done(_)));
    let fs = FileStore::open_global_transcripts(&path).unwrap();
    // Free about 12 MB, as the framer or a delete would.
    let big = transcript(12 << 20, 99);
    fs.make_file("agent:big:current", N, FileMeta::new(), FileOpts::default()).unwrap();
    fs.append_lines("agent:big:current", N, &big).unwrap();
    fs.delete_zone("agent:big:current").unwrap();
    let (_, _, free, _) = fs.page_stats().unwrap();
    assert!(free >= reclaim::INCREMENTAL_MIN_FREE_PAGES, "{free} pages free");
    let given = fs.reclaim_free_pages().unwrap();
    assert_eq!(given, free);
    assert_eq!(fs.page_stats().unwrap().2, 0);
    fs.checkpoint_truncate_apart().unwrap();
    for (zone, data) in &kept {
        assert_eq!(&read_all(&fs, zone), data);
    }
}

#[test]
fn incremental_steps_do_nothing_in_a_store_without_auto_vacuum() {
    let dir = tempfile::tempdir().unwrap();
    let (path, _) = store_with_free_pages(dir.path());
    let fs = FileStore::open_global_transcripts(&path).unwrap();
    assert_eq!(fs.reclaim_free_pages().unwrap(), 0);
}

/// Reclaim a snapshot of a real store, for measuring: `AGENTMUX_RECLAIM_COPY=<copy of
/// filestore.db> cargo test -p agentmux-srv --bin agentmux-srv reclaim_a_real_copy -- --ignored --nocapture`.
/// Never point it at a store an AgentMux has open.
#[test]
#[ignore]
fn reclaim_a_real_copy() {
    let Ok(path) = std::env::var("AGENTMUX_RECLAIM_COPY") else { return };
    let path = PathBuf::from(path);
    reclaim::request(&path, "measure", 1).unwrap();
    let started = std::time::Instant::now();
    let outcome = reclaim::run_at_start(&path, alone, 2);
    println!("{outcome:?} in {:.1} s", started.elapsed().as_secs_f64());
    let fs = FileStore::open_global_transcripts(&path).unwrap();
    println!("after: page_stats {:?}", fs.page_stats().unwrap());
}
