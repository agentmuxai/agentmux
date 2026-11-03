// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use std::cell::Cell;

const NONE: Duration = Duration::ZERO;

fn writer(pid: u32, started_s: u64, version: &str) -> Writer {
    Writer { pid, started_s, version: version.to_string(), channel: String::new() }
}

/// Write `w`'s file as a running srv would.
fn register(dir: &Path, w: &Writer) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(format!("{}.json", w.pid)), serde_json::to_vec(w).unwrap()).unwrap();
}

fn files(dir: &Path) -> usize {
    std::fs::read_dir(dir).map(|d| d.count()).unwrap_or(0)
}

#[test]
fn an_earlier_srv_of_another_version_keeps_the_folder_and_this_one_runs_without_it() {
    let tmp = tempfile::tempdir().unwrap();
    let owner = writer(10, 100, "0.60.10");
    register(tmp.path(), &owner);
    let me = writer(20, 200, "0.60.12");
    let claim = claim_in(tmp.path(), &me, &|_| true, NONE, NONE);
    assert_eq!(claim, Claim::Deferred(owner));
    assert_eq!(files(tmp.path()), 1, "a srv that gave the folder up leaves no file of its own");
}

#[test]
fn srvs_of_one_version_share_the_folder_as_they_always_have() {
    let tmp = tempfile::tempdir().unwrap();
    register(tmp.path(), &writer(10, 100, "0.60.12"));
    let me = writer(20, 200, "0.60.12");
    assert_eq!(claim_in(tmp.path(), &me, &|_| true, NONE, NONE), Claim::Owner);
    assert_eq!(files(tmp.path()), 2);
}

#[test]
fn a_later_srv_of_another_version_never_makes_an_earlier_one_give_up() {
    let tmp = tempfile::tempdir().unwrap();
    register(tmp.path(), &writer(20, 200, "0.60.12"));
    let me = writer(10, 100, "0.60.10");
    assert_eq!(claim_in(tmp.path(), &me, &|_| true, NONE, NONE), Claim::Owner);
}

#[test]
fn two_srvs_starting_together_agree_on_who_is_first() {
    let tmp = tempfile::tempdir().unwrap();
    let (a, b) = (writer(10, 100, "0.60.10"), writer(20, 100, "0.60.12"));
    // Both have written their files before either looks.
    register(tmp.path(), &a);
    register(tmp.path(), &b);
    assert_eq!(claim_in(tmp.path(), &a, &|_| true, NONE, NONE), Claim::Owner);
    register(tmp.path(), &b);
    assert_eq!(claim_in(tmp.path(), &b, &|_| true, NONE, NONE), Claim::Deferred(a));
}

#[test]
fn a_file_left_by_a_srv_that_is_gone_is_not_a_writer() {
    let tmp = tempfile::tempdir().unwrap();
    register(tmp.path(), &writer(10, 100, "0.60.10"));
    let me = writer(20, 200, "0.60.12");
    assert_eq!(claim_in(tmp.path(), &me, &|_| false, NONE, NONE), Claim::Owner);
    assert_eq!(files(tmp.path()), 1, "the stale file is removed");
}

#[test]
fn a_srv_that_is_leaving_is_waited_for() {
    let tmp = tempfile::tempdir().unwrap();
    register(tmp.path(), &writer(10, 100, "0.60.10"));
    let me = writer(20, 200, "0.60.12");
    let looks = Cell::new(0);
    let alive = |_: &Writer| {
        looks.set(looks.get() + 1);
        looks.get() < 3
    };
    assert_eq!(claim_in(tmp.path(), &me, &alive, Duration::from_secs(5), Duration::from_millis(1)), Claim::Owner);
    assert_eq!(looks.get(), 3);
}

#[test]
fn the_person_is_told_who_has_the_folder_and_what_to_do() {
    let owner = Writer { channel: "stable".into(), ..writer(10, 100, "0.60.10") };
    let me = writer(20, 200, "0.60.12");
    let i = deferred_incident(&me, &owner);
    assert_eq!(i.kind, "shared_stores_in_use");
    assert!(i.what.contains("AgentMux 0.60.10 (stable)") && i.what.contains("AgentMux 0.60.12") && i.what.contains("Close"), "{}", i.what);
    let i = too_new_incident("store.db", 9, 7, &me);
    assert_eq!(i.kind, "shared_store_newer");
    assert!(i.what.contains("older than your data") && i.what.contains("Update AgentMux"), "{}", i.what);
}
