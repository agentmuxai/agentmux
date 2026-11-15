// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use std::time::Duration;

use serde_json::{json, Value};

use super::*;
use crate::drone::types::{DroneDefinition, DroneGraph, FlowEdge, FlowNode, NodePosition};
use crate::server::drone_handlers::{start_drone_run, upsert_drone, RunDroneReq};

fn node(id: &str, data: Value) -> FlowNode {
    FlowNode { id: id.to_string(), position: NodePosition::default(), data, node_type: None }
}

fn edge(id: &str, source: &str, target: &str) -> FlowEdge {
    FlowEdge { id: id.to_string(), source: source.to_string(), target: target.to_string(), source_handle: None, target_handle: None }
}

/// start sets n; step appends an "x"; done answers: three blocks in a line.
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

fn saved(state: &AppState) -> DroneDefinition {
    let def: DroneDefinition = serde_json::from_value(json!({ "id": "", "name": "replay", "created_at": 0, "updated_at": 0 })).unwrap();
    upsert_drone(state, DroneDefinition { graph: line(), ..def }).unwrap()
}

/// The run's row once it has ended.
async fn finished(state: &AppState, drone_id: &str, run_id: &str) -> DroneRun {
    for _ in 0..400 {
        if let Some(row) = state.mstore.drone_runs_for(drone_id, 50).unwrap().into_iter().find(|r| r.id == run_id) {
            if ended(&row).is_ok() {
                return row;
            }
        }
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    panic!("the run did not end");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_finished_run_is_replayed_as_a_new_run_from_a_step() {
    let state = crate::server::tests::test_state();
    let drone = saved(&state);
    let first = start_drone_run(&state, RunDroneReq { drone_id: drone.id.clone(), input: None }).await.unwrap().run_id;
    assert_eq!(finished(&state, &drone.id, &first).await.output, "xx!");

    // From step 1: `start` is done, `step` and `done` are still to run.
    let r = replay_drone_run(&state, ReplayRunReq { run_id: first.clone(), from_step: 1, do_it_for_real: false }).await.unwrap();
    assert_eq!((r.forked_from.as_str(), r.from_step, r.real), (first.as_str(), 1, false));
    assert_ne!(r.run_id, first);
    let row = finished(&state, &drone.id, &r.run_id).await;
    assert_eq!((row.status.as_str(), row.output.as_str()), ("done", "xx!"), "{row:?}");

    let rows = state.mstore.drone_checkpoints_for(&r.run_id).unwrap();
    assert_eq!((rows[0].step, rows[0].kind.as_str()), (0, "full"));
    let last = load(&rows).unwrap().unwrap();
    assert_eq!(last.run_id, r.run_id);
    assert_eq!(last.fork, Some(Fork { run_id: first.clone(), step: 1, real: false }));
    assert_eq!(last.cost_usd, 0.0);
    // The run replayed is as it was.
    assert!(load(&state.mstore.drone_checkpoints_for(&first).unwrap()).unwrap().unwrap().fork.is_none());
}

#[tokio::test(flavor = "multi_thread")]
async fn a_step_the_run_did_not_reach_and_a_run_that_is_not_there_are_refused() {
    let state = crate::server::tests::test_state();
    let drone = saved(&state);
    let first = start_drone_run(&state, RunDroneReq { drone_id: drone.id.clone(), input: None }).await.unwrap().run_id;
    finished(&state, &drone.id, &first).await;
    let err = replay_drone_run(&state, ReplayRunReq { run_id: first.clone(), from_step: 99, do_it_for_real: false }).await.unwrap_err();
    assert!(err.contains("checkpoints 0 to 3, not 99"), "{err}");
    let err = replay_drone_run(&state, ReplayRunReq { run_id: "nope".into(), from_step: 0, do_it_for_real: false }).await.unwrap_err();
    assert!(err.contains("kept no checkpoints"), "{err}");
}

#[test]
fn a_run_that_is_still_going_is_not_replayed() {
    let row = |status: &str| DroneRun {
        id: "r".into(),
        drone_id: "d".into(),
        status: status.into(),
        started_at: 0,
        ended_at: 0,
        block_states: Default::default(),
        output: String::new(),
        error: String::new(),
    };
    for live in ["running", "waiting", "needs_attention"] {
        assert!(ended(&row(live)).unwrap_err().contains("still"), "{live}");
    }
    for over in ["done", "failed", "cancelled", "interrupted"] {
        assert!(ended(&row(over)).is_ok(), "{over}");
    }
}

#[tokio::test(flavor = "multi_thread")]
async fn a_replay_taken_up_after_a_restart_is_still_a_replay() {
    let state = crate::server::tests::test_state();
    let drone = saved(&state);
    let first = start_drone_run(&state, RunDroneReq { drone_id: drone.id.clone(), input: None }).await.unwrap().run_id;
    finished(&state, &drone.id, &first).await;
    let r = replay_drone_run(&state, ReplayRunReq { run_id: first.clone(), from_step: 1, do_it_for_real: false }).await.unwrap();
    finished(&state, &drone.id, &r.run_id).await;
    let c = load(&state.mstore.drone_checkpoints_for(&r.run_id).unwrap()).unwrap().unwrap();
    let again = replay_for(&state, &c).unwrap().expect("it is a replay");
    assert!(!again.fork.real);
    assert_eq!(again.original["done"]["value"], json!("xx!"), "what the run it replays said is found again");
    // A run that is not a replay has none.
    assert!(replay_for(&state, &load(&state.mstore.drone_checkpoints_for(&first).unwrap()).unwrap().unwrap()).unwrap().is_none());
}
