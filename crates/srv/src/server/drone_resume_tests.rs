// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

use super::*;
use crate::backend::storage::drone_store::CheckpointRow;
use crate::drone::executor::run_drone;
use crate::drone::recorder::MemorySink;
use crate::drone::types::{DroneDefinition, FlowEdge, FlowNode, NodePosition, RunStatus};
use crate::drone::versions::Pin;
use crate::server::drone_handlers::upsert_drone;

fn node(id: &str, data: Value) -> FlowNode {
    FlowNode { id: id.to_string(), position: NodePosition::default(), data, node_type: None }
}

fn edge(id: &str, source: &str, target: &str) -> FlowEdge {
    FlowEdge { id: id.to_string(), source: source.to_string(), target: target.to_string(), source_handle: None, target_handle: None }
}

/// start sets n; step appends an "x"; the third block is the response: three blocks in a line.
fn line() -> DroneGraph {
    DroneGraph {
        nodes: vec![
            node("start", json!({ "kind": "variables", "entries": [{ "name": "n", "value": "x" }] })),
            node("step", json!({ "kind": "variables", "entries": [{ "name": "n", "value": "{{var.n}}x" }] })),
            node("done", json!({ "kind": "response", "template": "{{var.n}}!" })),
        ],
        edges: vec![edge("e1", "start", "step"), edge("e2", "step", "done")],
    }
}

/// A saved drone with `graph`.
fn saved(state: &AppState, graph: DroneGraph) -> DroneDefinition {
    let def: DroneDefinition = serde_json::from_value(json!({ "id": "", "name": "resume", "created_at": 0, "updated_at": 0 })).unwrap();
    upsert_drone(state, DroneDefinition { graph, ..def }).unwrap()
}

struct Crashed {
    drone_id: String,
    run_id: String,
    /// What the run said when it ran to the end the first time.
    output: Value,
    rows: Vec<CheckpointRow>,
}

/// A run that went to its end the first time, whose checkpoints are the ones
/// a crash after step `kept - 1` would have left in the store, with the run's row still `running`.
async fn crashed_after(state: &AppState, drone: &DroneDefinition, kept: usize) -> Crashed {
    let version = state.id_store.version_resolve(&drone.id, Pin::Latest).unwrap().unwrap();
    let sink = Arc::new(MemorySink::default());
    let persist = Persist { sink: sink.clone(), graph: GraphRef { version: version.version, hash: version.hash } };
    let mut handle = run_drone(drone.id.clone(), drone.graph.clone(), Value::Null, (None, Some(persist))).await.unwrap();
    let mut output = Value::Null;
    while let Some(ev) = handle.events.recv().await {
        if let crate::drone::executor::RunEvent::RunDone { output: o, .. } = ev {
            output = o;
        }
    }
    let rows = sink.rows_of(&handle.run_id);
    for row in &rows[..kept] {
        state.mstore.drone_checkpoint_put(&handle.run_id, row.step, &row.kind, &row.body, 1).unwrap();
    }
    state
        .mstore
        .drone_run_insert(&DroneRun {
            id: handle.run_id.clone(),
            drone_id: drone.id.clone(),
            status: RunStatus::Running.as_str().to_string(),
            started_at: 5,
            ended_at: 0,
            block_states: HashMap::new(),
            output: String::new(),
            error: String::new(),
        })
        .unwrap();
    Crashed { drone_id: drone.id.clone(), run_id: handle.run_id, output, rows }
}

/// The run's row once it is no longer `running` (the drain writes it as the run ends).
async fn ended(state: &AppState, c: &Crashed) -> DroneRun {
    for _ in 0..200 {
        let row = state.mstore.drone_runs_for(&c.drone_id, 10).unwrap().into_iter().find(|r| r.id == c.run_id).unwrap();
        if row.status != RunStatus::Running.as_str() {
            return row;
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("the run did not end");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_left_running_goes_on_from_its_newest_checkpoint_and_ends_as_it_would_have() {
    let state = crate::server::tests::test_state();
    let drone = saved(&state, line());
    // Step 0, and the checkpoints after `start` and `step`: `done` is still to run.
    let c = crashed_after(&state, &drone, 3).await;
    assert_eq!(c.output, json!("xx!"));

    let resumed = resume_runs(&state, None).await;
    assert_eq!(resumed, HashSet::from([c.run_id.clone()]));
    let row = ended(&state, &c).await;
    assert_eq!((row.status.as_str(), row.output.as_str()), ("done", "xx!"), "{row:?}");
    assert_eq!(row.started_at, 5, "it is the same run, not a new one");
    assert_eq!(row.block_states["start"].status, "done", "what ran before the restart still shows");
    assert_eq!(row.block_states["done"].status, "done");

    // It kept writing checkpoints, from where the old process stopped.
    let rows = state.mstore.drone_checkpoints_for(&c.run_id).unwrap();
    assert_eq!(rows.len(), c.rows.len());
    assert_eq!(rows[..3], c.rows[..3]);
    assert_eq!(load(&rows).unwrap().unwrap().outputs["done"]["value"], json!("xx!"));
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_with_no_checkpoint_is_left_for_the_restart_to_interrupt() {
    let state = crate::server::tests::test_state();
    let drone = saved(&state, line());
    let c = crashed_after(&state, &drone, 0).await;
    assert!(resume_runs(&state, None).await.is_empty());
    let row = state.mstore.drone_runs_for(&c.drone_id, 10).unwrap().remove(0);
    assert_eq!(row.status, RunStatus::Running.as_str(), "this only decides which runs go on; the restart closes the rest");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_whose_graph_was_edited_and_not_kept_is_not_taken_up() {
    let state = crate::server::tests::test_state();
    let drone = saved(&state, line());
    let c = crashed_after(&state, &drone, 3).await;
    // The checkpoint names version 1 and a hash; make the hash one no version of this drone has.
    let mut bytes: Value = serde_json::from_slice(&c.rows[0].body).unwrap();
    bytes["graph"] = json!({ "version": 0, "hash": "not-this-graph" });
    state.mstore.drone_run_delete(&c.run_id).unwrap();
    state.mstore.drone_checkpoint_put(&c.run_id, 0, "full", &serde_json::to_vec(&bytes).unwrap(), 1).unwrap();
    state
        .mstore
        .drone_run_insert(&DroneRun { id: c.run_id.clone(), drone_id: c.drone_id.clone(), status: "running".into(), started_at: 5, ended_at: 0, block_states: HashMap::new(), output: String::new(), error: String::new() })
        .unwrap();
    assert!(resume_runs(&state, None).await.is_empty());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_run_stopped_in_a_step_with_effects_is_not_run_again_on_its_own() {
    let state = crate::server::tests::test_state();
    // `call` is a POST the API block refuses, so its mark is the newest checkpoint.
    let graph = DroneGraph {
        nodes: vec![
            node("first", json!({ "kind": "variables", "entries": [{ "name": "a", "value": "1" }] })),
            node("call", json!({ "kind": "api", "method": "POST", "url": "ftp://nowhere.invalid/x" })),
        ],
        edges: vec![edge("e1", "first", "call")],
    };
    let drone = saved(&state, graph);
    let c = crashed_after(&state, &drone, 3).await;
    assert_eq!(resume_runs(&state, None).await.len(), 1);
    let row = ended(&state, &c).await;
    assert_eq!(row.status, RunStatus::Failed.as_str());
    assert!(row.error.contains("block call was running when the run stopped") && row.error.contains("not run again on its own"), "{}", row.error);
}
