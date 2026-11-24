// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! How many imports wait for a person at once, and which one makes room.

use super::*;

fn staged_at(base: Instant, n: u64) -> Staged {
    Staged { file_name: format!("f{n}"), entries: Vec::new(), at: base + Duration::from_millis(n) }
}

#[test]
fn a_fifth_staged_import_pushes_out_the_oldest_and_only_that() {
    let base = Instant::now();
    let mut held: HashMap<String, Staged> = (0..MAX_STAGED as u64).map(|n| (format!("t{n}"), staged_at(base, n))).collect();
    make_room(&mut held, MAX_STAGED);
    assert_eq!(held.len(), MAX_STAGED - 1, "room for one more");
    assert!(!held.contains_key("t0") && held.contains_key("t1") && held.contains_key(&format!("t{}", MAX_STAGED - 1)), "the oldest went");
    make_room(&mut held, MAX_STAGED);
    assert_eq!(held.len(), MAX_STAGED - 1, "already room: nothing more goes");
}

#[test]
fn a_test_run_holds_far_more_than_the_limit_a_person_gets() {
    assert_eq!(MAX_STAGED, 4);
    assert!(stage_limit() > MAX_STAGED, "tests share one table; another test's imports must not push out this one's");
}
