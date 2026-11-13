// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The drone store and the shape of its rows, pinned before the types and the store move
//! out of `drone/` into the storage layer (crates/srv core split). The same tests ran on
//! the unchanged code and run again after the move: they name the old `crate::drone::...`
//! paths, which must keep resolving, so nothing here changes with it.

use std::collections::HashMap;

use crate::backend::storage::store::Store;
use crate::drone::storage::DroneStore;
use crate::drone::types::{
    BlockKind, BlockState, DroneDefinition, DroneGraph, DroneRun, DroneViewport, FlowEdge, FlowNode, NodePosition,
    RunStatus,
};

fn definition(id: &str, updated_at: i64) -> DroneDefinition {
    DroneDefinition {
        id: id.into(),
        name: format!("drone {id}"),
        description: "d".into(),
        graph: DroneGraph {
            nodes: vec![FlowNode {
                id: "n1".into(),
                position: NodePosition { x: 1.5, y: -2.0 },
                data: serde_json::json!({ "kind": "variables" }),
                node_type: Some("variables".into()),
            }],
            edges: vec![FlowEdge {
                id: "e1".into(),
                source: "n1".into(),
                target: "n2".into(),
                source_handle: Some("out".into()),
                target_handle: None,
            }],
        },
        viewport: DroneViewport { x: 3.0, y: 4.0, zoom: 0.5 },
        created_at: 10,
        updated_at,
    }
}

fn run(id: &str, drone_id: &str, status: &str, started_at: i64) -> DroneRun {
    DroneRun {
        id: id.into(),
        drone_id: drone_id.into(),
        status: status.into(),
        started_at,
        ended_at: 0,
        block_states: HashMap::new(),
        output: String::new(),
        error: String::new(),
    }
}

#[test]
fn a_definition_round_trips_through_the_store_and_lists_newest_first() {
    let store = Store::open_in_memory().unwrap();
    store.drone_upsert(&definition("a", 100)).unwrap();
    store.drone_upsert(&definition("b", 200)).unwrap();

    let got = store.drone_get("a").unwrap().expect("a is stored");
    assert_eq!(serde_json::to_value(&got).unwrap(), serde_json::to_value(definition("a", 100)).unwrap());
    assert!(store.drone_get("missing").unwrap().is_none());

    let ids: Vec<String> = store.drone_list().unwrap().into_iter().map(|d| d.id).collect();
    assert_eq!(ids, ["b", "a"], "newest update first");

    // An upsert of an existing id rewrites it in place and keeps `created_at`.
    let mut changed = definition("a", 300);
    changed.name = "renamed".into();
    changed.created_at = 999;
    store.drone_upsert(&changed).unwrap();
    let again = store.drone_get("a").unwrap().unwrap();
    assert_eq!((again.name.as_str(), again.created_at, again.updated_at), ("renamed", 10, 300));
    assert_eq!(store.drone_list().unwrap().len(), 2);

    assert!(store.drone_delete("a").unwrap());
    assert!(!store.drone_delete("a").unwrap(), "a second delete finds nothing");
}

#[test]
fn runs_insert_update_list_newest_first_and_delete() {
    let store = Store::open_in_memory().unwrap();
    store.drone_upsert(&definition("a", 1)).unwrap();
    for (id, at) in [("r1", 1), ("r2", 3), ("r3", 2)] {
        store.drone_run_insert(&run(id, "a", "running", at)).unwrap();
    }
    assert!(store.drone_run_insert(&run("r1", "a", "running", 9)).is_err(), "a duplicate run id fails loudly");

    let ids = |limit| -> Vec<String> { store.drone_runs_for("a", limit).unwrap().into_iter().map(|r| r.id).collect() };
    assert_eq!(ids(10), ["r2", "r3", "r1"]);
    assert_eq!(ids(2), ["r2", "r3"], "the limit applies after ordering");

    let mut done = run("r2", "a", "done", 3);
    done.ended_at = 8;
    done.output = "{\"ok\":true}".into();
    done.block_states.insert(
        "n1".into(),
        BlockState {
            status: "done".into(),
            output: Some(serde_json::json!(1)),
            error: None,
            started_at: Some(3),
            completed_at: Some(8),
        },
    );
    assert_eq!(store.drone_run_update(&done).unwrap(), 1);
    assert_eq!(store.drone_run_update(&run("nope", "a", "done", 0)).unwrap(), 0, "no row, no change");
    let back = store.drone_runs_for("a", 1).unwrap().remove(0);
    assert_eq!(serde_json::to_value(&back).unwrap(), serde_json::to_value(&done).unwrap());

    assert_eq!(store.drone_run_delete("r1").unwrap(), 1);
    assert_eq!(store.drone_run_delete("r1").unwrap(), 0);
    assert_eq!(store.drone_runs_delete_for("a").unwrap(), 2);
    assert!(store.drone_runs_for("a", 10).unwrap().is_empty());
}

#[test]
fn a_restart_interrupts_only_the_runs_still_running() {
    let store = Store::open_in_memory().unwrap();
    store.drone_run_insert(&run("live", "a", "running", 1)).unwrap();
    let mut finished = run("finished", "a", "done", 2);
    finished.ended_at = 5;
    store.drone_run_insert(&finished).unwrap();
    let mut failed_running = run("with-error", "a", "running", 3);
    failed_running.error = "boom".into();
    store.drone_run_insert(&failed_running).unwrap();

    assert_eq!(store.drone_runs_mark_interrupted("failed", 77).unwrap(), 2);

    let by_id: HashMap<String, DroneRun> =
        store.drone_runs_for("a", 10).unwrap().into_iter().map(|r| (r.id.clone(), r)).collect();
    assert_eq!((by_id["live"].status.as_str(), by_id["live"].ended_at), ("failed", 77));
    assert_eq!(by_id["live"].error, "interrupted: AgentMux restarted during this run");
    assert_eq!(by_id["with-error"].error, "boom", "an error already recorded is kept");
    assert_eq!((by_id["finished"].status.as_str(), by_id["finished"].ended_at), ("done", 5));
}

#[test]
fn the_wire_shape_of_a_definition_a_run_and_the_enums() {
    assert_eq!(
        serde_json::to_value(definition("a", 100)).unwrap(),
        serde_json::json!({
            "id": "a",
            "name": "drone a",
            "description": "d",
            "graph": {
                "nodes": [{
                    "id": "n1",
                    "position": { "x": 1.5, "y": -2.0 },
                    "data": { "kind": "variables" },
                    "type": "variables",
                }],
                "edges": [{ "id": "e1", "source": "n1", "target": "n2", "sourceHandle": "out" }],
            },
            "viewport": { "x": 3.0, "y": 4.0, "zoom": 0.5 },
            "created_at": 10,
            "updated_at": 100,
        })
    );

    // A row missing its optional parts reads back with their defaults.
    let sparse: DroneDefinition = serde_json::from_value(serde_json::json!({
        "id": "s", "name": "sparse", "created_at": 1, "updated_at": 2,
    }))
    .unwrap();
    assert_eq!((sparse.description.as_str(), sparse.graph.nodes.len(), sparse.viewport.zoom), ("", 0, 1.0));

    let mut r = run("r", "a", "running", 1);
    r.block_states.insert(
        "n1".into(),
        BlockState { status: "running".into(), output: None, error: None, started_at: None, completed_at: None },
    );
    assert_eq!(
        serde_json::to_value(&r).unwrap(),
        serde_json::json!({
            "id": "r", "drone_id": "a", "status": "running", "started_at": 1, "ended_at": 0,
            "block_states": { "n1": { "status": "running" } },
            "output": "", "error": "",
        })
    );

    let kinds = [
        (BlockKind::AppQuery, "app_query"),
        (BlockKind::ForEach, "foreach"),
        (BlockKind::Agent, "agent"),
    ];
    for (kind, wire) in kinds {
        assert_eq!(serde_json::to_value(kind).unwrap(), serde_json::json!(wire));
        assert_eq!(BlockKind::parse(wire), Some(kind));
    }
    assert_eq!(BlockKind::parse("nope"), None);
    assert_eq!(serde_json::to_value(RunStatus::Failed).unwrap(), serde_json::json!("failed"));
}
