// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

const NOW: u64 = 1_000_000_000;

fn proc(pid: u32, parent: Option<u32>, exe: &str, command: &str, started_at_ms: u64) -> TrackedProcess {
    TrackedProcess { pid, command: command.into(), rss_bytes: 0, started_at_ms, parent_pid: parent, exe: exe.into() }
}

fn task(pid: Option<i64>, started_at_ms: i64, status: BackgroundTaskStatus) -> BackgroundTask {
    BackgroundTask {
        id: "t".into(),
        block_id: "b".into(),
        label: "cargo test".into(),
        pid,
        started_at_ms,
        status,
        last_seen_ms: started_at_ms,
        ended_at_ms: None,
        owner_tool_use_id: None,
    }
}

#[test]
fn a_tree_counts_once_it_has_run_past_the_grace_period() {
    let young = [proc(10, None, "cargo", "cargo build", NOW - GRACE_MS + 1)];
    assert!(from_tracker(NOW, &young).is_empty(), "a command about to finish is not background work");
    let old = [proc(10, None, "cargo", "cargo build", NOW - GRACE_MS), proc(11, Some(10), "rustc", "rustc --crate-name x", NOW - 1000)];
    let items = from_tracker(NOW, &old);
    assert_eq!(items.len(), 1, "one tree, not one item per process");
    assert_eq!((items[0].kind, items[0].label.as_str()), (Kind::Process, "cargo build"));
}

#[test]
fn a_tree_whose_start_is_unknown_counts() {
    assert_eq!(from_tracker(NOW, &[proc(10, None, "node", "node server.js", 0)]).len(), 1);
}

#[test]
fn daemons_that_outlive_their_command_do_not_count() {
    let members = [
        proc(10, None, "sccache", "sccache --start-server", NOW - 600_000),
        proc(20, None, "git", "git fsmonitor--daemon run --detach", NOW - 600_000),
        proc(30, None, "node", "node dev-server.js", NOW - 600_000),
    ];
    let items = from_tracker(NOW, &members);
    assert_eq!(items.iter().map(|i| i.label.as_str()).collect::<Vec<_>>(), ["node dev-server.js"]);
}

#[test]
fn a_declared_shell_counts_while_its_pid_is_tracked() {
    let members = [proc(77, None, "cargo", "cargo test", NOW - 5000)];
    let tasks = [task(Some(77), (NOW - 5000) as i64, BackgroundTaskStatus::Running)];
    let items = from_tasks(NOW as i64, &tasks, &members);
    assert_eq!((items.len(), items[0].kind), (1, Kind::Shell));
    assert!(from_tasks(NOW as i64, &tasks, &[]).is_empty(), "a row stuck running, its process gone, is not work");
}

#[test]
fn a_declared_shell_that_ended_or_has_no_pid_for_long_does_not_count() {
    let done = [task(None, NOW as i64 - 1000, BackgroundTaskStatus::Done)];
    assert!(from_tasks(NOW as i64, &done, &[]).is_empty());
    let fresh = [task(None, NOW as i64 - 1000, BackgroundTaskStatus::Running)];
    assert_eq!(from_tasks(NOW as i64, &fresh, &[]).len(), 1);
    let stale = [task(None, NOW as i64 - NO_PID_FOR_MS - 1, BackgroundTaskStatus::Running)];
    assert!(from_tasks(NOW as i64, &stale, &[]).is_empty(), "no pid to check, so only for a bounded time");
}

#[test]
fn an_agent_with_nothing_running_is_not_in_the_set() {
    assert!(set_of(NOW, &[], &[]).is_empty());
}

#[test]
fn the_set_lists_the_declared_shell_before_the_trees_left_behind() {
    let members = [proc(77, None, "cargo", "cargo test", NOW - 60_000), proc(90, None, "node", "node server.js", NOW - 60_000)];
    let tasks = [task(Some(77), (NOW - 60_000) as i64, BackgroundTaskStatus::Running)];
    let set = set_of(NOW, &tasks, &members);
    assert_eq!(set[0].kind, Kind::Shell);
    assert!(set.iter().any(|i| i.kind == Kind::Process && i.label == "node server.js"));
}

#[test]
fn changes_names_who_entered_who_left_and_keeps_when_each_began() {
    let mut before = HashMap::new();
    let now = HashMap::from([("a".to_string(), 100u64), ("b".to_string(), 100u64)]);
    let (entered, left) = changes(&mut before, &now, 100);
    assert_eq!((entered.len(), left.len()), (2, 0));
    let now = HashMap::from([("b".to_string(), 160u64)]);
    let (entered, left) = changes(&mut before, &now, 160);
    assert!(entered.is_empty());
    assert_eq!(left, vec![("a".to_string(), 60)], "a left after 60 ms in the set");
    assert_eq!(before.get("b"), Some(&100), "b's start is kept, not reset by a later look");
}
