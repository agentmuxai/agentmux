// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
//! The Supervisor reset sets Steady and Drive panes to Off once and keeps their history.

use super::*;
use crate::backend::obj::Block;

fn store_with(meta: Value) -> (Store, String) {
    let store = Store::open_in_memory().unwrap();
    let mut block = Block { oid: uuid::Uuid::new_v4().to_string(), ..Default::default() };
    block.meta = meta.as_object().unwrap().clone().into_iter().collect();
    store.insert(&mut block).unwrap();
    (store, block.oid)
}

fn meta_of(store: &Store, oid: &str) -> Map<String, Value> {
    store.get_raw("block", oid).unwrap().unwrap()["meta"].as_object().unwrap().clone()
}

fn history(n: usize) -> Value {
    Value::Array((0..n).map(|i| json!({ "at_ms": i, "action": "continued", "reason": format!("step {i}") })).collect())
}

#[test]
fn a_drive_pane_goes_to_off_and_keeps_its_history() {
    let (store, oid) = store_with(json!({
        "view": "agent",
        "supervisor:mode": "drive",
        "supervisor:state": { "mode": "drive", "status": "continued", "continuations": 4, "limit": 15,
            "run_started_at_ms": 5, "no_progress": 1, "last_reply": "abc", "history": history(2) }
    }));

    assert_eq!(reset_blocks(&store, 99).unwrap(), 1);

    let meta = meta_of(&store, &oid);
    assert!(!meta.contains_key("supervisor:mode"), "the mode is cleared");
    assert_eq!(meta["view"], "agent", "other keys stay");
    let state = &meta["supervisor:state"];
    assert_eq!(state["mode"], "off");
    assert_eq!(state["status"], "watching");
    assert_eq!(state["continuations"], 0);
    assert!(state.get("last_reply").is_none());
    let history = state["history"].as_array().unwrap();
    assert_eq!(history.len(), 3, "two kept, one added");
    assert_eq!(history[0]["reason"], "step 0");
    assert_eq!(history[2]["action"], "mode_set");
    assert_eq!(history[2]["at_ms"], 99);
}

#[test]
fn a_steady_pane_with_no_state_goes_to_off() {
    let (store, oid) = store_with(json!({ "supervisor:mode": "Steady" }));
    assert_eq!(reset_blocks(&store, 7).unwrap(), 1);
    let meta = meta_of(&store, &oid);
    assert!(!meta.contains_key("supervisor:mode"));
    assert_eq!(meta["supervisor:state"]["history"].as_array().unwrap().len(), 1);
}

#[test]
fn a_pane_already_off_is_left_alone_and_a_second_run_changes_nothing() {
    let (store, oid) = store_with(json!({ "supervisor:state": { "mode": "off", "history": history(3) } }));
    assert_eq!(reset_blocks(&store, 1).unwrap(), 0);
    assert_eq!(meta_of(&store, &oid)["supervisor:state"]["history"].as_array().unwrap().len(), 3);

    let (store, _) = store_with(json!({ "supervisor:mode": "drive" }));
    assert_eq!(reset_blocks(&store, 1).unwrap(), 1);
    assert_eq!(reset_blocks(&store, 2).unwrap(), 0, "once");
}

#[test]
fn the_history_stays_within_its_cap() {
    let (store, oid) = store_with(json!({
        "supervisor:mode": "drive",
        "supervisor:state": { "mode": "drive", "history": history(HISTORY_MAX) }
    }));
    reset_blocks(&store, 50).unwrap();
    let state = &meta_of(&store, &oid)["supervisor:state"];
    let history = state["history"].as_array().unwrap();
    assert_eq!(history.len(), HISTORY_MAX);
    assert_eq!(history[0]["reason"], "step 1", "the oldest went");
    assert_eq!(history[HISTORY_MAX - 1]["at_ms"], 50);
}

#[test]
fn a_pane_with_another_meta_or_an_unknown_mode_is_untouched() {
    let (store, _) = store_with(json!({ "view": "term" }));
    assert_eq!(reset_blocks(&store, 1).unwrap(), 0);
    let (store, _) = store_with(json!({ "supervisor:mode": "off" }));
    assert_eq!(reset_blocks(&store, 1).unwrap(), 0);
}
