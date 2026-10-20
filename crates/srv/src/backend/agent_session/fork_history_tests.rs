// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::storage::filestore::{FileMeta, FileOpts};

const PARENT: &str = "parent-def";
const FORK: &str = "fork-def";

fn line(sid: &str, n: usize) -> String {
    format!("{{\"type\":\"assistant\",\"session_id\":\"{sid}\",\"n\":{n}}}\n")
}

/// A parent whose transcript holds an older session, then `last`.
fn parent_with(gfs: &FileStore, last: &str) -> Vec<u8> {
    let zone = agent_current_zone(PARENT);
    let mut data = String::new();
    for n in 0..3 {
        data += &line("older-session", n);
    }
    for n in 3..6 {
        data += &line(last, n);
    }
    gfs.make_file(&zone, OUTPUT_FILE, FileMeta::new(), FileOpts::default()).unwrap();
    gfs.append_lines_stamped(&zone, OUTPUT_FILE, data.as_bytes(), TSIDX_FILE).unwrap();
    write_zone_file(gfs, &zone, SNAPSHOT_FILE, b"{\"collapsed\":[\"n1\"]}").unwrap();
    data.into_bytes()
}

fn read_all(gfs: &FileStore, zone: &str, name: &str) -> Vec<u8> {
    gfs.read_file(zone, name).unwrap().unwrap_or_default()
}

#[test]
fn a_fork_of_the_last_session_gets_the_parents_history_exactly() {
    let gfs = FileStore::open_in_memory().unwrap();
    let data = parent_with(&gfs, "sess-b");

    let out = copy_parent_history(&gfs, PARENT, FORK, "sess-b", "session_id").unwrap();

    let parent = agent_current_zone(PARENT);
    let sidecars = read_all(&gfs, &parent, TSIDX_FILE).len() + read_all(&gfs, &parent, SNAPSHOT_FILE).len();
    assert_eq!(out, ForkHistory::Copied { bytes: (data.len() + sidecars) as u64 });
    let fork = agent_current_zone(FORK);
    assert_eq!(read_all(&gfs, &fork, OUTPUT_FILE), data);
    assert_eq!(read_all(&gfs, &fork, TSIDX_FILE), read_all(&gfs, &agent_current_zone(PARENT), TSIDX_FILE));
    assert_eq!(read_all(&gfs, &fork, SNAPSHOT_FILE), b"{\"collapsed\":[\"n1\"]}");
    assert_eq!(inherited_session(&gfs, &fork).as_deref(), Some("sess-b"));
    assert_eq!(inherited_session(&gfs, &agent_current_zone(PARENT)), None, "the parent is not a fork");
}

#[test]
fn a_last_event_larger_than_the_tail_window_still_names_its_session() {
    let gfs = FileStore::open_in_memory().unwrap();
    let zone = agent_current_zone(PARENT);
    // A tool result of 700 KB: the first window starts inside it.
    let big = format!("{{\"type\":\"user\",\"session_id\":\"sess-big\",\"text\":\"{}\"}}\n", "x".repeat(700 * 1024));
    gfs.make_file(&zone, OUTPUT_FILE, FileMeta::new(), FileOpts::default()).unwrap();
    gfs.append_lines(&zone, OUTPUT_FILE, format!("{}{big}", line("sess-big", 0)).as_bytes()).unwrap();

    let out = copy_parent_history(&gfs, PARENT, FORK, "sess-big", "session_id").unwrap();

    assert!(matches!(out, ForkHistory::Copied { .. }), "got {out:?}");
}

#[test]
fn a_fork_of_an_older_session_copies_nothing() {
    let gfs = FileStore::open_in_memory().unwrap();
    parent_with(&gfs, "sess-b");

    let out = copy_parent_history(&gfs, PARENT, FORK, "older-session", "session_id").unwrap();

    assert_eq!(out, ForkHistory::NotTheLastSession);
    assert!(gfs.stat(&agent_current_zone(FORK), OUTPUT_FILE).unwrap().is_none());
    assert_eq!(inherited_session(&gfs, &agent_current_zone(FORK)), None);
}

#[test]
fn a_fork_with_history_of_its_own_keeps_it() {
    let gfs = FileStore::open_in_memory().unwrap();
    parent_with(&gfs, "sess-b");
    let fork = agent_current_zone(FORK);
    gfs.make_file(&fork, OUTPUT_FILE, FileMeta::new(), FileOpts::default()).unwrap();
    gfs.append_lines(&fork, OUTPUT_FILE, line("fork-session", 0).as_bytes()).unwrap();

    let out = copy_parent_history(&gfs, PARENT, FORK, "sess-b", "session_id").unwrap();

    assert_eq!(out, ForkHistory::ForkHasHistory);
    assert_eq!(read_all(&gfs, &fork, OUTPUT_FILE), line("fork-session", 0).into_bytes());
}

#[test]
fn nothing_to_copy_without_parent_history_or_a_session() {
    let gfs = FileStore::open_in_memory().unwrap();
    assert_eq!(copy_parent_history(&gfs, PARENT, FORK, "sess-b", "session_id").unwrap(), ForkHistory::NoParentHistory);
    assert!(copy_parent_history(&gfs, PARENT, FORK, "", "session_id").is_err());
    assert!(copy_parent_history(&gfs, PARENT, PARENT, "sess-b", "session_id").is_err());
    assert!(copy_parent_history(&gfs, PARENT, FORK, "sess-b", "").is_err());
}


/// A parent with `older-session` (3 lines) then `sess-b` (3 lines), and a
/// sidecar with a record for each (written whole: appends close together share
/// one). Returns the bytes and where the older session ends.
fn parent_in_two_batches(gfs: &FileStore) -> (Vec<u8>, usize) {
    let zone = agent_current_zone(PARENT);
    let older: String = (0..3).map(|n| line("older-session", n)).collect();
    let newer: String = (3..6).map(|n| line("sess-b", n)).collect();
    gfs.make_file(&zone, OUTPUT_FILE, FileMeta::new(), FileOpts::default()).unwrap();
    gfs.append_lines(&zone, OUTPUT_FILE, format!("{older}{newer}").as_bytes()).unwrap();
    let sidecar = format!("{{\"off\":0,\"ms\":1000}}
{{\"off\":{},\"ms\":2000}}
", older.len());
    write_zone_file(gfs, &zone, TSIDX_FILE, sidecar.as_bytes()).unwrap();
    write_zone_file(gfs, &zone, SNAPSHOT_FILE, b"{\"collapsed\":[\"n1\"]}").unwrap();
    (format!("{older}{newer}").into_bytes(), older.len())
}

#[test]
fn a_fork_of_an_older_session_gets_the_history_up_to_where_it_ends() {
    let gfs = FileStore::open_in_memory().unwrap();
    let (data, end) = parent_in_two_batches(&gfs);
    let parent = agent_current_zone(PARENT);
    let fork = agent_current_zone(FORK);

    let out = copy_older_session_history(&gfs, PARENT, FORK, "older-session", "session_id", &parent, end as i64).unwrap();

    assert!(matches!(out, ForkHistory::Copied { bytes } if bytes > end as u64), "got {out:?}");
    assert_eq!(read_all(&gfs, &fork, OUTPUT_FILE), &data[..end]);
    // The sidecar stops at the same point: one record (the first batch's).
    let side = read_all(&gfs, &fork, TSIDX_FILE);
    let parent_side = read_all(&gfs, &parent, TSIDX_FILE);
    assert!(!side.is_empty() && side.len() < parent_side.len() && parent_side.starts_with(&side));
    assert_eq!(side.iter().filter(|&&b| b == b'\n').count(), 1);
    // The parent's snapshot is where the parent is now, not here.
    assert!(gfs.stat(&fork, SNAPSHOT_FILE).unwrap().is_none());
    assert_eq!(inherited_session(&gfs, &fork).as_deref(), Some("older-session"));
    // The parent keeps everything.
    assert_eq!(read_all(&gfs, &parent, OUTPUT_FILE), data);
    // The fork continues after the copied history.
    gfs.append_lines(&fork, OUTPUT_FILE, line("fork-session", 0).as_bytes()).unwrap();
    assert!(read_all(&gfs, &fork, OUTPUT_FILE).starts_with(&data[..end]));
}

#[test]
fn a_cut_that_does_not_end_in_the_session_is_undone() {
    let gfs = FileStore::open_in_memory().unwrap();
    let (_, end) = parent_in_two_batches(&gfs);
    let parent = agent_current_zone(PARENT);
    // A stale record: the cut lands in the middle of a line, then at the end
    // of a line of the later session.
    let out = copy_older_session_history(&gfs, PARENT, FORK, "older-session", "session_id", &parent, end as i64 + 1).unwrap();
    assert_eq!(out, ForkHistory::NotTheLastSession);
    let second = end + line("sess-b", 3).len();
    let out = copy_older_session_history(&gfs, PARENT, FORK, "older-session", "session_id", &parent, second as i64).unwrap();

    assert_eq!(out, ForkHistory::NotTheLastSession);
    assert!(gfs.stat(&agent_current_zone(FORK), OUTPUT_FILE).unwrap().is_none());
}

#[test]
fn an_older_session_needs_a_transcript_a_session_and_a_place_to_end() {
    let gfs = FileStore::open_in_memory().unwrap();
    let parent = agent_current_zone(PARENT);
    assert_eq!(copy_older_session_history(&gfs, PARENT, FORK, "s", "session_id", &parent, 10).unwrap(), ForkHistory::NoParentHistory);
    assert!(copy_older_session_history(&gfs, PARENT, FORK, "", "session_id", &parent, 10).is_err());
    assert!(copy_older_session_history(&gfs, PARENT, FORK, "s", "session_id", &parent, 0).is_err());
    assert!(copy_older_session_history(&gfs, PARENT, PARENT, "s", "session_id", &parent, 10).is_err());
}
