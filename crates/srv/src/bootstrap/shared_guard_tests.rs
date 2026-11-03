// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use std::cell::Cell;

const NONE: Duration = Duration::ZERO;

fn writer(pid: u32, started_s: u64, version: &str) -> Writer {
    Writer { pid, started_s, version: version.to_string(), channel: String::new() }
}

/// Put `w` in place `seq` as a running srv that registered then would be.
fn register_at(dir: &Path, seq: u64, w: &Writer) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(format!("{seq}.json")), serde_json::to_vec(w).unwrap()).unwrap();
}

fn names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir).unwrap().map(|e| e.unwrap().file_name().to_string_lossy().into_owned()).collect();
    names.sort();
    names
}

#[test]
fn an_earlier_srv_of_another_version_keeps_the_folder_and_this_one_runs_without_it() {
    let tmp = tempfile::tempdir().unwrap();
    let owner = writer(10, 100, "0.60.10");
    register_at(tmp.path(), 1, &owner);
    let me = writer(20, 200, "0.60.12");
    assert_eq!(claim_in(tmp.path(), &me, &|_| true, NONE, NONE), Claim::Deferred(owner));
    assert_eq!(names(tmp.path()), ["1.json"], "a srv that gave the folder up leaves no file of its own");
}

#[test]
fn srvs_of_one_version_share_the_folder_as_they_always_have() {
    let tmp = tempfile::tempdir().unwrap();
    register_at(tmp.path(), 1, &writer(10, 100, "0.60.12"));
    let me = writer(20, 200, "0.60.12");
    assert_eq!(claim_in(tmp.path(), &me, &|_| true, NONE, NONE), Claim::Owner);
    assert_eq!(names(tmp.path()), ["1.json", "2.json"]);
}

#[test]
fn a_srv_that_registers_later_never_makes_an_earlier_one_give_up() {
    let tmp = tempfile::tempdir().unwrap();
    let first = writer(10, 100, "0.60.10");
    assert_eq!(claim_in(tmp.path(), &first, &|_| true, NONE, NONE), Claim::Owner);
    let later = writer(20, 200, "0.60.12");
    assert_eq!(claim_in(tmp.path(), &later, &|_| true, NONE, NONE), Claim::Deferred(first.clone()));
    assert_eq!(claim_in(tmp.path(), &first, &|_| true, NONE, NONE), Claim::Owner, "and the first one still owns it when it looks again");
}

/// The order is that of registration, not of process age: the srv that started later but registered
/// first is the owner, and the one that started earlier but registered second gives way.
#[test]
fn the_first_to_register_owns_the_folder_whichever_process_is_older() {
    let tmp = tempfile::tempdir().unwrap();
    let (older, newer) = (writer(10, 100, "0.60.10"), writer(20, 200, "0.60.12"));
    assert_eq!(claim_in(tmp.path(), &newer, &|_| true, NONE, NONE), Claim::Owner);
    assert_eq!(claim_in(tmp.path(), &older, &|_| true, NONE, NONE), Claim::Deferred(newer));
}

#[test]
fn two_srvs_taking_the_same_place_get_different_ones() {
    let tmp = tempfile::tempdir().unwrap();
    let (a, b) = (writer(10, 100, "0.60.10"), writer(20, 100, "0.60.12"));
    assert_eq!(register(tmp.path(), &a).unwrap(), 1);
    assert_eq!(register(tmp.path(), &b).unwrap(), 2);
    assert_eq!(names(tmp.path()), ["1.json", "2.json"], "the file written aside is gone");
}

#[test]
fn a_file_left_by_a_srv_that_is_gone_is_not_a_writer() {
    let tmp = tempfile::tempdir().unwrap();
    register_at(tmp.path(), 1, &writer(10, 100, "0.60.10"));
    let me = writer(20, 200, "0.60.12");
    assert_eq!(claim_in(tmp.path(), &me, &|_| false, NONE, NONE), Claim::Owner);
    assert_eq!(names(tmp.path()), ["2.json"], "the stale file is removed");
}

#[test]
fn a_srv_that_is_leaving_is_waited_for() {
    let tmp = tempfile::tempdir().unwrap();
    register_at(tmp.path(), 1, &writer(10, 100, "0.60.10"));
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
fn a_registration_still_being_written_aside_is_not_read() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("10.tmp"), "{\"pid\":10,").unwrap();
    let me = writer(20, 200, "0.60.12");
    assert_eq!(claim_in(tmp.path(), &me, &|_| true, NONE, NONE), Claim::Owner);
    assert_eq!(names(tmp.path()), ["1.json", "10.tmp"]);
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
