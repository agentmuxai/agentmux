// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::*;
use crate::drone::storage::DroneStore;
use crate::server::drone_handlers::upsert_drone;

fn saved(state: &AppState, name: &str, nodes: Value) -> DroneDefinition {
    let def: DroneDefinition = serde_json::from_value(json!({
        "id": "", "name": name, "created_at": 0, "updated_at": 0,
        "graph": { "nodes": nodes, "edges": [] },
    }))
    .unwrap();
    upsert_drone(state, def).unwrap()
}

fn node(id: &str, data: Value) -> Value {
    json!({ "id": id, "position": { "x": 0, "y": 0 }, "data": data })
}

async fn call(state: &AppState, action: &str, body: Value) -> Result<Value, String> {
    act(state, "agent-uid-1", action, &body).await
}

/// A drone that changes nothing outside AgentMux can be fired, and the run is there when the call returns.
#[tokio::test]
async fn an_agent_fires_a_harmless_drone_with_a_payload() {
    let state = crate::server::tests::test_state();
    let drone = saved(&state, "Say hello", json!([node("v", json!({ "kind": "variables" }))]));
    let fired = call(&state, "fire", json!({ "drone_id": drone.id, "input": { "who": "world" } })).await.unwrap();
    let run_id = fired["run_id"].as_str().expect("a run id").to_string();
    let runs = state.mstore.drone_runs_for(&drone.id, 10).unwrap();
    assert!(runs.iter().any(|r| r.id == run_id), "the run row exists: {runs:?}");
    let listed = call(&state, "runs", json!({ "drone_id": drone.id })).await.unwrap();
    assert!(listed["runs"].as_array().unwrap().iter().any(|r| r["id"] == run_id.as_str()));
}

/// A drone is found by its name too, and a name two drones share is refused rather than guessed.
#[tokio::test]
async fn a_drone_is_found_by_its_name_and_a_shared_name_is_refused() {
    let state = crate::server::tests::test_state();
    let one = saved(&state, "Nightly triage", json!([]));
    let got = call(&state, "get", json!({ "drone_id": "nightly TRIAGE" })).await.unwrap();
    assert_eq!(got["id"], one.id.as_str());
    saved(&state, "Nightly triage", json!([]));
    let err = call(&state, "get", json!({ "drone_id": "Nightly triage" })).await.unwrap_err();
    assert!(err.starts_with("INVALID_ARG") && err.contains("more than one"), "{err}");
    let err = call(&state, "get", json!({ "drone_id": "nothing like it" })).await.unwrap_err();
    assert!(err.starts_with("NOT_FOUND"), "{err}");
    let err = call(&state, "get", json!({})).await.unwrap_err();
    assert!(err.starts_with("INVALID_ARG"), "{err}");
}

/// What could turn the payload into a change outside AgentMux is a person's to run: an API call that is not a
/// read, a Vault credential, a command, another drone. `list` says so before the agent tries.
#[tokio::test]
async fn a_drone_that_could_act_on_the_payload_outside_agentmux_is_not_fired_by_an_agent() {
    let state = crate::server::tests::test_state();
    let cases = [
        ("posts", node("p", json!({ "kind": "api", "method": "POST", "url": "https://example.com/hook" })), "writes outside"),
        ("reads with a key", node("g", json!({ "kind": "api", "method": "GET", "url": "https://example.com/x", "auth": { "vault": "some-entry", "as": "bearer" } })), "Vault credential"),
        ("runs", node("r", json!({ "kind": "run", "command": ["true"] })), "runs a command"),
        ("calls", node("c", json!({ "kind": "drone", "drone_id": "other" })), "calls another drone"),
    ];
    for (name, n, why) in cases {
        let d = saved(&state, name, json!([n]));
        let err = call(&state, "fire", json!({ "drone_id": d.id })).await.unwrap_err();
        assert!(err.starts_with("FORBIDDEN") && err.contains(why), "{name}: {err}");
        assert!(state.mstore.drone_runs_for(&d.id, 10).unwrap().is_empty(), "{name}: nothing started");
        let listed = call(&state, "list", json!({})).await.unwrap();
        let row = listed["drones"].as_array().unwrap().iter().find(|r| r["id"] == d.id.as_str()).unwrap().clone();
        assert_eq!(row["fireable"], false, "{name}");
        assert!(row["why_not"].as_str().unwrap().contains(why), "{name}: {row}");
    }
    // A plain read is not a write (not fired here: the run would make the request).
    let read = saved(&state, "reads", json!([node("g", json!({ "kind": "api", "method": "GET", "url": "https://example.com/x" }))]));
    assert_eq!(why_not_fireable(&read), None);
}

/// The payload is an object within the input limit; anything else is refused before a run starts.
#[tokio::test]
async fn the_payload_is_an_object_within_the_limit() {
    let state = crate::server::tests::test_state();
    let d = saved(&state, "takes input", json!([]));
    for bad in [json!("text"), json!([1, 2]), json!(7)] {
        let err = call(&state, "fire", json!({ "drone_id": d.id, "input": bad })).await.unwrap_err();
        assert!(err.starts_with("INVALID_ARG"), "{err}");
    }
    let big = json!({ "log": "x".repeat(MAX_INPUT_BYTES + 1) });
    let err = call(&state, "fire", json!({ "drone_id": d.id, "input": big })).await.unwrap_err();
    assert!(err.starts_with("INVALID_ARG") && err.contains("at most"), "{err}");
    assert!(state.mstore.drone_runs_for(&d.id, 10).unwrap().is_empty(), "nothing started");
    assert!(call(&state, "fire", json!({ "drone_id": d.id })).await.is_ok(), "no input is fine");
}

/// A drone that reads a Vault credential may have fetched data the agent has no grant for: its runs show the
/// status and times, never the output or the error text, and no entry name is shown anywhere.
#[tokio::test]
async fn a_drone_that_reads_a_credential_shows_an_agent_no_output() {
    let state = crate::server::tests::test_state();
    let keyed = saved(&state, "keyed", json!([node("g", json!({ "kind": "api", "method": "GET", "url": "https://example.com/x", "auth": { "vault": "secret-entry" } }))]));
    let plain = saved(&state, "plain", json!([]));
    for (d, tag) in [(&keyed, "keyed"), (&plain, "plain")] {
        let row = crate::drone::types::DroneRun {
            id: format!("run-{tag}"),
            drone_id: d.id.clone(),
            status: "done".into(),
            started_at: 1,
            ended_at: 2,
            block_states: Default::default(),
            output: "{\"fetched\":\"private\"}".into(),
            error: "boom".into(),
        };
        state.mstore.drone_run_insert(&row).unwrap();
    }
    // A drone that calls the keyed one passes its Response up as its own value: it is withheld too.
    let caller = saved(&state, "caller", json!([node("c", json!({ "kind": "drone", "drone_id": keyed.id }))]));
    let row = crate::drone::types::DroneRun {
        id: "run-caller".into(),
        drone_id: caller.id.clone(),
        status: "done".into(),
        started_at: 1,
        ended_at: 2,
        block_states: Default::default(),
        output: "{\"fetched\":\"private\"}".into(),
        error: "boom".into(),
    };
    state.mstore.drone_run_insert(&row).unwrap();
    for d in [&keyed, &caller] {
        let hidden = call(&state, "runs", json!({ "drone_id": d.id })).await.unwrap();
        let text = hidden.to_string();
        assert!(!text.contains("private") && !text.contains("boom") && !text.contains("secret-entry"), "{}: {text}", d.name);
        assert_eq!(hidden["runs"][0]["status"], "done");
        assert!(hidden["output_withheld"].is_string());
    }
    let shown = call(&state, "runs", json!({ "drone_id": plain.id })).await.unwrap();
    assert!(shown["runs"][0]["output"].as_str().unwrap().contains("private"));
    assert!(shown.get("output_withheld").is_none());
}

#[tokio::test]
async fn an_unknown_action_is_refused() {
    let state = crate::server::tests::test_state();
    let err = call(&state, "delete", json!({})).await.unwrap_err();
    assert!(err.starts_with("INVALID_ARG") && err.contains("list, get, fire or runs"), "{err}");
}

/// `get` tells an agent what a person would be asked before running it.
#[tokio::test]
async fn get_names_what_the_drone_writes_and_which_credentials_it_reads() {
    let state = crate::server::tests::test_state();
    let d = saved(
        &state,
        "mixed",
        json!([
            node("a", json!({ "kind": "agent", "task": "look" })),
            node("p", json!({ "kind": "api", "method": "DELETE", "url": "https://example.com/x", "auth": { "vault": "entry-1" } })),
        ]),
    );
    let got = call(&state, "get", json!({ "drone_id": d.id })).await.unwrap();
    assert_eq!(got["agents"], 1);
    assert_eq!(got["writes_outside"][0], "p: DELETE https://example.com/x");
    assert_eq!(got["credentials"], 1, "how many, never which entry");
    assert!(!got.to_string().contains("entry-1"), "{got}");
    assert_eq!(got["kinds"], json!(["agent", "api"]));
    assert_eq!(got["fireable"], false);
}
