// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// tasks.enqueue and tasks.list for an agent of another machine
// (SPEC_AGENTS_OPERATE_OTHER_MACHINES_2026_10_30.md §4.3).

use serde_json::{json, Value};

use super::{tasks_enqueue, tasks_list};
use crate::server::tests::test_state;
use crate::server::AppState;

const ACTOR: &str = "naki@narko";

/// A state with the stored agent `Zed` (slug `zed`), which a remote task can be filed for.
fn state_with_zed() -> (AppState, String) {
    let state = test_state();
    let mut def = crate::backend::storage::agents::test_agent_def("zed-uid", "Zed", "claude", "host", 1, "");
    state.mstore.agent_def_insert(&mut def).unwrap();
    (state, def.id)
}

#[tokio::test]
async fn a_remote_task_is_filed_numbered_and_carded_as_the_agent_that_asked() {
    let (state, zed_uid) = state_with_zed();
    let out = tasks_enqueue(&state, ACTOR, json!({ "title": "Check the build", "payload": "run the tests on this machine", "target_agent": "zed" })).await.unwrap();
    let (id, num) = (out["id"].as_str().unwrap().to_string(), out["num"].as_i64().unwrap());
    assert!(num > 0);

    let task = state.work_store.work_board_get(&id).unwrap().unwrap();
    assert_eq!(task.item.target_agent_uid, zed_uid, "the target is resolved here, on the machine that owns it");
    assert_eq!(task.item.created_by, ACTOR);
    assert_eq!(task.item.created_by_uid, "", "an agent of another machine has no uid here");
    assert_eq!((task.card.requested_by.as_str(), task.card.requested_by_kind.as_str()), (ACTOR, "agent"), "never the operator");
    assert_eq!(task.card.filed_by, ACTOR);
    assert!(!task.card.review_required, "a handoff between agents waits for no Accept");

    let history = state.work_store.work_events(&id, 10).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!((history[0].actor.as_str(), history[0].actor_kind.as_str()), (ACTOR, "agent"));
}

#[tokio::test]
async fn a_remote_request_can_ask_for_review_and_file_a_spec_with_its_build() {
    let (state, _) = state_with_zed();
    let out = tasks_enqueue(&state, ACTOR, json!({ "title": "Feature", "payload": "p", "target_agent": "zed", "first": "spec", "review": true })).await.unwrap();
    assert!(out["build_num"].as_i64().unwrap() > out["num"].as_i64().unwrap(), "{out}");
    let spec = state.work_store.work_board_get(out["id"].as_str().unwrap()).unwrap().unwrap();
    assert_eq!(spec.item.title, "Spec: Feature");
    assert!(spec.card.review_required);
    let build = state.work_store.work_board_get(out["build_id"].as_str().unwrap()).unwrap().unwrap();
    assert_eq!(build.card.requested_by, ACTOR);
}

#[tokio::test]
async fn a_target_that_names_nobody_here_is_refused_and_nothing_is_filed() {
    let (state, _) = state_with_zed();
    let e = tasks_enqueue(&state, ACTOR, json!({ "title": "t", "payload": "p", "target_agent": "nobody" })).await.unwrap_err();
    assert_eq!(e.code, "no_such_agent");
    assert!(e.message.contains("nobody"), "{}", e.message);
    assert!(tasks_list(&state, Value::Null).unwrap()["items"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn a_request_with_unknown_fields_or_no_title_is_refused() {
    let (state, _) = state_with_zed();
    // A group is a local idea, and so is a claimed filer: neither is a parameter.
    for params in [
        json!({ "title": "t", "payload": "p", "target_group": "g" }),
        json!({ "title": "t", "payload": "p", "created_by": "the-operator" }),
        json!({ "title": "t" }),
    ] {
        let e = tasks_enqueue(&state, ACTOR, params.clone()).await.unwrap_err();
        assert_eq!(e.code, "bad_params", "{params}: {}", e.message);
    }
    let e = tasks_enqueue(&state, ACTOR, json!({ "title": "  ", "payload": "p" })).await.unwrap_err();
    assert_eq!((e.code, e.message.as_str()), ("bad_params", "title is required"));
    assert!(tasks_list(&state, Value::Null).unwrap()["items"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn the_board_lists_what_was_filed_with_numbers_and_can_be_filtered() {
    let (state, _) = state_with_zed();
    let a = tasks_enqueue(&state, ACTOR, json!({ "title": "one", "payload": "p" })).await.unwrap();
    tasks_enqueue(&state, ACTOR, json!({ "title": "two", "payload": "p", "hold": true })).await.unwrap();
    let all = tasks_list(&state, json!({})).unwrap();
    assert_eq!(all["items"].as_array().unwrap().len(), 2);
    let open = tasks_list(&state, json!({ "state": "open" })).unwrap();
    let items = open["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!((items[0]["title"].as_str(), items[0]["num"].as_i64()), (Some("one"), a["num"].as_i64()));
    assert_eq!(tasks_list(&state, json!({ "limit": 1 })).unwrap()["items"].as_array().unwrap().len(), 1);
    assert_eq!(tasks_list(&state, json!({ "nope": 1 })).unwrap_err().code, "bad_params");
}
