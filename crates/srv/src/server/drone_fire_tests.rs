// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::backend::sensitive_requests::registry;
use crate::drone::storage::DroneStore;
use crate::server::drone_handlers::upsert_drone;

fn drone(state: &AppState, name: &str) -> DroneDefinition {
    let def: DroneDefinition = serde_json::from_value(json!({
        "id": "", "name": name, "created_at": 0, "updated_at": 0,
        "graph": { "nodes": [{ "id": "v", "position": { "x": 0, "y": 0 }, "data": { "kind": "variables" } }], "edges": [] },
    }))
    .unwrap();
    upsert_drone(state, def).unwrap()
}

/// File the request the way the Drone tool does, and read it back as the runner gets it.
async fn asked(state: &AppState, d: &DroneDefinition, input: Value) -> Request {
    let answer = ask(state, ("uid-clamk", "clamk"), d, input).await.unwrap();
    registry().get(answer["request_id"].as_str().unwrap()).expect("the request is open")
}

#[tokio::test]
async fn an_unvetoed_request_starts_the_run_as_the_agent_that_asked() {
    let state = crate::server::tests::test_state();
    let d = drone(&state, "Quiet one");
    let request = asked(&state, &d, json!({ "who": "world" })).await;
    run_after_countdown(state.clone(), request).await.unwrap();
    let runs = state.mstore.drone_runs_for(&d.id, 10).unwrap();
    assert_eq!(runs.len(), 1, "one run: {runs:?}");
}

#[tokio::test]
async fn a_drone_edited_while_the_request_waited_is_not_run() {
    let state = crate::server::tests::test_state();
    let d = drone(&state, "Edited");
    let request = asked(&state, &d, Value::Null).await;
    let mut edited = d.clone();
    edited.graph.nodes.push(serde_json::from_value(json!({ "id": "w", "position": { "x": 1, "y": 1 }, "data": { "kind": "variables" } })).unwrap());
    upsert_drone(&state, edited).unwrap();
    let err = run_after_countdown(state.clone(), request).await.unwrap_err();
    assert!(err.contains("edited while the request waited"), "{err}");
    assert!(state.mstore.drone_runs_for(&d.id, 10).unwrap().is_empty());
}

#[tokio::test]
async fn a_request_whose_payload_is_gone_runs_nothing() {
    let state = crate::server::tests::test_state();
    let d = drone(&state, "Gone");
    let request = asked(&state, &d, Value::Null).await;
    held().remove(&request.id);
    let err = run_after_countdown(state.clone(), request).await.unwrap_err();
    assert!(err.contains("payload for this request is gone"), "{err}");
    assert!(state.mstore.drone_runs_for(&d.id, 10).unwrap().is_empty());
}

#[tokio::test]
async fn a_deleted_drone_is_not_run() {
    let state = crate::server::tests::test_state();
    let d = drone(&state, "Deleted");
    let request = asked(&state, &d, Value::Null).await;
    state.id_store.drone_delete(&d.id).unwrap();
    let err = run_after_countdown(state.clone(), request).await.unwrap_err();
    assert!(err.contains("was deleted"), "{err}");
}

#[test]
fn the_payload_is_told_by_size_and_a_short_digest_so_two_payloads_are_two_requests() {
    assert_eq!(payload_fact(&Value::Null), "none");
    let one = payload_fact(&json!({ "a": 1 }));
    let two = payload_fact(&json!({ "a": 2 }));
    assert!(one.starts_with("7 bytes, sha256 ") && one != two, "{one} / {two}");
}
