// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;

fn at(minutes: u64) -> Instant {
    static START: OnceLock<Instant> = OnceLock::new();
    *START.get_or_init(Instant::now) + Duration::from_secs(minutes * 60)
}

#[test]
fn an_agent_may_make_ten_shaped_captures_an_hour_matching_or_not() {
    let mut l = Ledger::default();
    for i in 0..SHAPED {
        assert!(l.start("agent-a", at(i as u64)).is_ok(), "{i}");
    }
    let err = l.start("agent-a", at(11)).unwrap_err();
    assert!(err.contains("10 captures with a shape") && err.contains("not for testing what one says"), "{err}");
    // A match counted as much as a miss did: nothing was refused for missing.
    assert!(l.start("agent-b", at(11)).is_ok(), "per agent");
    // An hour after the first, the oldest are forgotten.
    assert!(l.start("agent-a", at(61)).is_ok());
}

#[test]
fn the_fourth_miss_in_an_hour_ends_shapes_for_an_hour() {
    let mut l = Ledger::default();
    for i in 0..MISSES {
        assert!(l.start("agent-a", at(i as u64)).is_ok());
        assert!(!l.miss("agent-a", at(i as u64)), "miss {} is allowed", i + 1);
    }
    assert!(l.start("agent-a", at(5)).is_ok(), "three misses still leave a shape to try");
    assert!(l.miss("agent-a", at(5)), "the fourth is one too many");
    let err = l.start("agent-a", at(6)).unwrap_err();
    assert!(err.contains("refused for you for another"), "{err}");
    assert!(l.start("agent-a", at(5 + 61)).is_ok(), "an hour after the fourth, shapes are back");
}

#[test]
fn a_second_pane_starts_nothing_new() {
    // The ledger is by agent, not by pane: the handler gives it the agent's id only.
    let mut l = Ledger::default();
    for i in 0..SHAPED {
        l.start("agent-a", at(i as u64)).unwrap();
    }
    assert!(l.start("agent-a", at(10)).is_err(), "whichever pane the tenth-and-first capture is on");
}

#[test]
fn the_shared_ledger_ignores_the_case_of_an_agents_name() {
    assert!(start("Agent-Case-Test").is_ok());
    let mut misses = 0;
    while misses < MISSES + 1 {
        misses += 1;
        let said = miss("agent-case-test");
        assert_eq!(!said.is_empty(), misses > MISSES, "miss {misses}: {said:?}");
    }
    assert!(start("AGENT-CASE-TEST").unwrap_err().contains("refused for you"));
}
