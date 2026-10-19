// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::storage::filestore::{FileMeta, FileOpts};

const ZONE: &str = "agent:a1:current";

/// Three lines, `a`, `bb`, `ccc`, at byte 0, 2 and 5, received at 100, 200
/// and 300 ms.
fn transcript() -> FileStore {
    let fs = FileStore::open_in_memory().unwrap();
    fs.make_file(ZONE, OUTPUT_FILE, FileMeta::default(), FileOpts::default()).unwrap();
    fs.append_lines(ZONE, OUTPUT_FILE, b"a\nbb\nccc\n").unwrap();
    fs.make_file(ZONE, TSIDX_FILE, FileMeta::default(), FileOpts::default()).unwrap();
    fs.append_data(ZONE, TSIDX_FILE, b"{\"off\":0,\"ms\":100}\n{\"off\":2,\"ms\":200}\n{\"off\":5,\"ms\":300}\n").unwrap();
    fs
}

fn at(fs: &FileStore, ms: Option<u64>, byte: Option<u64>) -> (u64, u64) {
    let l = locate(fs, ZONE, ms, byte).unwrap();
    (l.line, l.total)
}

#[test]
fn a_byte_locates_the_lines_before_it() {
    let fs = transcript();
    assert_eq!(at(&fs, None, Some(0)), (0, 3));
    assert_eq!(at(&fs, None, Some(2)), (1, 3), "a session ending at byte 2 holds line 0 only");
    assert_eq!(at(&fs, None, Some(4)), (2, 3), "inside line 1: lines 0 and 1 start before it");
    assert_eq!(at(&fs, None, Some(9)), (3, 3));
    assert_eq!(at(&fs, None, Some(1_000)), (3, 3), "past the end is the end");
}

#[test]
fn a_time_locates_the_lines_received_by_then() {
    let fs = transcript();
    assert_eq!(at(&fs, Some(50), None), (0, 3), "before the first line");
    assert_eq!(at(&fs, Some(200), None), (2, 3), "up to the line received at 200");
    assert_eq!(at(&fs, Some(250), None), (2, 3));
    assert_eq!(at(&fs, Some(999), None), (3, 3), "after the last line");
}

#[test]
fn neither_locates_the_end() {
    assert_eq!(at(&transcript(), None, None), (3, 3));
}

#[test]
fn a_time_without_recorded_times_is_refused_not_placed_at_the_end() {
    let fs = FileStore::open_in_memory().unwrap();
    fs.make_file(ZONE, OUTPUT_FILE, FileMeta::default(), FileOpts::default()).unwrap();
    fs.append_lines(ZONE, OUTPUT_FILE, b"a\nbb\n").unwrap();
    let err = locate(&fs, ZONE, Some(100), None).unwrap_err();
    assert!(err.contains("no recorded times"), "{err}");
    assert_eq!(at(&fs, None, Some(2)), (1, 2), "a byte still locates");
}
