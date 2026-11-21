// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use serde_json::json;

use super::super::{lookup, Verification};
use super::*;
use crate::server::tests::test_state;

fn caller() -> RemoteCaller {
    RemoteCaller {
        agent_uid: "uid-claimed-by-the-requester".into(),
        slug: "agenty".into(),
        machine: "elsewhere".into(),
        instance_id: "inst-1".into(),
        install_trusted: true,
        verification: Verification::Lan,
        reason: "which agents are not on Sonnet".into(),
    }
}

#[test]
fn it_is_a_read_that_needs_an_approved_install() {
    let op = lookup("agents.status").expect("the op is in the table");
    assert!(!op.changes_state, "it changes nothing");
    assert!(op.needs_approved_install, "a first-seen pin is too thin for a fleet's state");
}

#[test]
fn the_query_is_the_params_as_text_without_the_askers_host() {
    let query = query_of(&json!({ "model": "!sonnet", "limit": 5, "host": "somewhere", "state": null, "view": "status" })).unwrap();
    let mut pairs: Vec<_> = query.into_iter().collect();
    pairs.sort();
    assert_eq!(pairs, vec![("limit".to_string(), "5".to_string()), ("model".to_string(), "!sonnet".to_string()), ("view".to_string(), "status".to_string())]);
    assert!(query_of(&json!(null)).unwrap().is_empty());
    assert!(query_of(&json!(["model"])).is_err());
    assert!(query_of(&json!({ "fields": ["model"] })).unwrap_err().contains("fields"));
}

#[test]
fn a_row_loses_the_places_of_this_machine_and_keeps_its_state() {
    let mut row = json!({
        "name": "Loap", "id": "u1", "working_directory": "/home/someone/work", "block_id": "b1", "model_label": "Sonnet 5.5",
        "runtime": { "state": "idle", "block_id": "b1", "waiting_in": "b2", "provider": "claude" },
    });
    without_local_places(&mut row);
    assert_eq!(row, json!({ "name": "Loap", "id": "u1", "model_label": "Sonnet 5.5", "runtime": { "state": "idle", "provider": "claude" } }));
}

#[tokio::test]
async fn a_list_answers_with_the_agents_a_page_and_who_answered() {
    let state = test_state();
    let said = status(&state, &caller(), json!({ "model": "!sonnet", "fields": "model" })).await.unwrap();
    assert_eq!(said["view"], "list");
    assert_eq!(said["host"], json!(state.hostname));
    assert!(said["as_of_ms"].as_u64().unwrap() > 0 && said["version"].is_string());
    assert!(said["answer"]["agents"].is_array() && said["answer"]["page"]["limit"] == 200, "a page is bounded when the asker names none: {said}");
}

#[tokio::test]
async fn a_status_answers_with_lines_and_counts_and_a_wrong_query_is_refused_by_name() {
    let state = test_state();
    let said = status(&state, &caller(), json!({ "view": "status", "group_by": "model" })).await.unwrap();
    assert_eq!(said["view"], "status");
    assert!(said["answer"]["lines"].is_array() && said["answer"]["counts"]["by_state"].is_object(), "{said}");
    let refused = status(&state, &caller(), json!({ "view": "everything" })).await.unwrap_err();
    assert_eq!(refused.code, "bad_params");
    let refused = status(&state, &caller(), json!({ "fields": "bogus" })).await.unwrap_err();
    assert!(refused.message.contains("bogus"), "{}", refused.message);
}
