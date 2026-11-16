// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A transcript part lost inside the size its file claims (the 2026-10-22
//! shared-store repair left three such files: `.recover` could not save a page,
//! and no complete copy existed to rebuild from). In the shared store every
//! writer commits size and parts together, so a missing part there is lost
//! data, not a write in progress: the transcript's text reads it as blank, as it
//! already reads a damaged part, and an append onto it pads with spaces. Before
//! this, the pane opened at the newest lines, hit the gap, and stayed empty.
//! A per-channel store, which an older build without transactions may still
//! write, keeps refusing such a read, and binary files (`output.idx`,
//! `output.tsidx`) keep refusing it everywhere: spaces would be offsets there.

use super::*;
use crate::backend::blockcontroller::shell::{extend_output_idx, read_via_index};
use std::path::Path;

const ZONE: &str = "agent:x:current";
const PART: i64 = 64 * 1024;

/// About 300 KB of JSON lines: five parts.
fn transcript() -> Vec<u8> {
    let mut text = Vec::new();
    for i in 0..300 {
        text.extend_from_slice(format!("{{\"n\":{i},\"pad\":\"{}\"}}\n", "x".repeat(1000)).as_bytes());
    }
    text
}

fn write(fs: &FileStore, text: &[u8]) {
    fs.make_file(ZONE, "output", FileMeta::default(), FileOpts::default()).unwrap();
    fs.append_lines(ZONE, "output", text).unwrap();
}

/// Delete part `idx` of `name`, leaving the file's recorded size as it was.
fn lose_part(path: &Path, name: &str, idx: i64) {
    let conn = rusqlite::Connection::open(path).unwrap();
    let gone = conn
        .execute("DELETE FROM db_file_data WHERE zoneid = ?1 AND name = ?2 AND partidx = ?3", rusqlite::params![ZONE, name, idx])
        .unwrap();
    assert_eq!(gone, 1);
}

fn size(fs: &FileStore) -> i64 {
    fs.read_snapshot(|s| Ok(s.file(ZONE, "output")?.map(|f| f.size))).unwrap().unwrap()
}

#[test]
fn in_the_shared_store_a_lost_last_part_reads_as_blank_and_the_newest_lines_are_served() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("filestore.db");
    let fs = FileStore::open_global_transcripts(&path).unwrap();
    let text = transcript();
    write(&fs, &text);
    let total = size(&fs);
    let last = (total - 1) / PART;
    lose_part(&path, "output", last);

    // The whole file reads, the lost part as spaces, and keeps its length.
    let bytes = fs.read_bytes_db(ZONE, "output", 0, total).unwrap();
    assert_eq!(bytes.len() as i64, total);
    assert!(bytes[(last * PART) as usize..].iter().all(|&b| b == b' '), "the lost part is blank");
    assert_eq!(&bytes[..(last * PART) as usize], &text[..(last * PART) as usize], "everything before it is intact");

    // The index builds over it, and the newest page (what a pane opens at) is
    // served: the lines before the gap, the last of them cut by it.
    let lines = extend_output_idx(&fs, ZONE).expect("the index builds past a lost part");
    assert!(lines > 0);
    let tail = read_via_index(&fs, ZONE, lines.saturating_sub(10), 10).expect("the newest lines are served");
    assert_eq!(tail.total, lines);
    let shown = String::from_utf8_lossy(&tail.raw);
    assert!(shown.contains("\"n\":"), "real lines come back, not only blanks");
}

#[test]
fn in_the_shared_store_an_append_onto_a_lost_last_part_pads_with_spaces_not_zeros() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("filestore.db");
    let fs = FileStore::open_global_transcripts(&path).unwrap();
    write(&fs, &transcript());
    let before = size(&fs);
    lose_part(&path, "output", (before - 1) / PART);

    fs.append_lines(ZONE, "output", b"{\"after\":true}\n").unwrap();
    let after = size(&fs);
    let bytes = fs.read_bytes_db(ZONE, "output", 0, after).unwrap();
    assert!(!bytes.contains(&0u8), "no NUL bytes: a zero-filled gap reads as one long garbage line");
    let text = String::from_utf8_lossy(&bytes);
    assert!(text.trim_end().ends_with("{\"after\":true}"), "the new line lands after the gap, on a line of its own");
    let lines = extend_output_idx(&fs, ZONE).unwrap();
    let tail = read_via_index(&fs, ZONE, lines - 1, 1).unwrap();
    assert_eq!(String::from_utf8_lossy(&tail.raw).trim_end(), "{\"after\":true}");
}

#[test]
fn a_per_channel_store_still_refuses_a_part_missing_inside_the_size() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("filestore.db");
    let fs = FileStore::open(&path).unwrap();
    write(&fs, &transcript());
    let total = size(&fs);
    lose_part(&path, "output", (total - 1) / PART);
    let err = fs.read_bytes_db(ZONE, "output", 0, total).unwrap_err();
    assert!(err.to_string().contains("not stored yet"), "{err}");
}

#[test]
fn a_binary_file_with_a_lost_part_is_still_refused_in_the_shared_store() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("filestore.db");
    let fs = FileStore::open_global_transcripts(&path).unwrap();
    write(&fs, &transcript());
    let lines = extend_output_idx(&fs, ZONE).unwrap();
    assert!(lines > 0);
    let idx_size = fs.read_snapshot(|s| Ok(s.file(ZONE, "output.idx")?.map(|f| f.size))).unwrap().unwrap();
    lose_part(&path, "output.idx", 0);
    assert!(fs.read_bytes_db(ZONE, "output.idx", 0, idx_size).is_err(), "offsets are never invented from spaces");
    // The index is rebuilt from the transcript instead, and reads work again.
    assert_eq!(extend_output_idx(&fs, ZONE), Some(lines));
    assert!(read_via_index(&fs, ZONE, 0, 5).is_some());
}
