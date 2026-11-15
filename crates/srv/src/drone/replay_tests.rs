// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::collections::BTreeMap;

use serde_json::json;

use super::*;
use crate::drone::checkpoint::sample;

fn fork(real: bool) -> Fork {
    Fork { run_id: "run-1".into(), step: 3, real }
}

fn replay(original: &[(&str, serde_json::Value)]) -> Replay {
    Replay { fork: fork(false), original: original.iter().map(|(id, v)| (id.to_string(), v.clone())).collect::<BTreeMap<_, _>>() }
}

#[test]
fn a_stub_says_what_the_block_said_before_and_that_it_is_a_stub() {
    let r = replay(&[("send", json!({ "delivered": true, "to": "agenty", "history": [{ "old": 1 }] }))]);
    assert_eq!(r.stub("send"), json!({ "delivered": true, "to": "agenty", "stubbed": true }), "an older pass of a loop is not carried");
}

#[test]
fn a_stub_costs_nothing_whatever_the_block_cost_before() {
    let r = replay(&[("ask", json!({ "response": "ok", "cost_usd": 1.25 }))]);
    assert_eq!(r.stub("ask"), json!({ "response": "ok", "stubbed": true }));
}

#[test]
fn a_block_that_said_nothing_before_says_only_that_it_was_stubbed() {
    assert_eq!(replay(&[]).stub("send"), json!({ "stubbed": true }));
    assert_eq!(replay(&[("n", json!(7))]).stub("n"), json!({ "stubbed": true, "value": 7 }));
}

#[test]
fn the_new_run_starts_from_the_step_with_nothing_spent_and_no_wait_pending() {
    let mut at = sample(4);
    at.frontier.running = vec!["r".into()];
    at.frontier.ready = vec!["a".into(), "r".into()];
    at.resume_on.insert("w1".into(), "key".into());
    at.position.spent_by.insert("n1".into(), 0.5);
    let c = fork_checkpoint(&at, "run-2", fork(false));
    assert_eq!((c.run_id.as_str(), c.step), ("run-2", 0));
    assert_eq!(c.fork, Some(fork(false)));
    assert_eq!((c.cost_usd, c.elapsed_ms), (0.0, 0));
    assert!(c.deadlines.is_empty() && c.resume_on.is_empty() && c.wait_timeouts.is_empty());
    assert!(c.position.spent_by.is_empty());
    assert!(c.frontier.running.is_empty() && c.frontier.waiting.is_empty());
    assert_eq!(c.frontier.ready, ["r", "w1", "a"], "what was running, then what was waiting, then the rest, none twice");
}

#[test]
fn what_the_run_knew_stays_what_it_knew() {
    let at = sample(4);
    let c = fork_checkpoint(&at, "run-2", fork(true));
    assert_eq!((&c.state, &c.outputs, &c.bits, &c.trigger, &c.graph, &c.drone_id, c.run_cap_usd), (&at.state, &at.outputs, &at.bits, &at.trigger, &at.graph, &at.drone_id, at.run_cap_usd));
    assert_eq!(c.position.finished, at.position.finished);
}

#[test]
fn a_replaying_checkpoint_survives_being_written_and_read() {
    let c = fork_checkpoint(&sample(2), "run-2", fork(false));
    let back = crate::drone::checkpoint::decode(&crate::drone::checkpoint::encode(&c).unwrap()).unwrap();
    assert_eq!(back.fork, Some(fork(false)));
    assert!(crate::drone::checkpoint::decode(&crate::drone::checkpoint::encode(&sample(2)).unwrap()).unwrap().fork.is_none());
}
