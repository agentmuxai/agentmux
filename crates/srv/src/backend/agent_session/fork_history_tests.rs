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
