// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Acceptance tests for module C, graph versions
//! (docs/specs/SPEC_DRONE_GENERAL_AUTOMATION_2026_11_11.md §3.3 "Graph versions",
//! §7 "Versions and replay").
//!
//! They fail until `versions.rs`, its migration and the `upsertdrone` and
//! `deletedrone` handlers are written, and are marked `ignore` so main stays
//! green; the module's PR deletes each `#[ignore]` line and changes nothing
//! else in this file. Run them with
//! `cargo test -p agentmux-srv --bin agentmux-srv drone::versions_acceptance -- --ignored`.

use serde_json::json;

use super::storage::DroneStore;
use super::types::{DroneDefinition, DroneGraph, FlowNode, NodePosition};
use super::versions::{graph_hash, DroneVersionStore, Pin};
use crate::server::drone_handlers::{delete_drone, upsert_drone, DeleteDroneReq};

fn graph(template: &str) -> DroneGraph {
    DroneGraph {
        nodes: vec![FlowNode {
            id: "r1".into(),
            position: NodePosition::default(),
            data: json!({ "kind": "response", "template": template }),
            node_type: None,
        }],
        edges: vec![],
    }
}

fn drone(id: &str, template: &str) -> DroneDefinition {
    let mut d: DroneDefinition = serde_json::from_value(json!({ "id": id, "name": "Echo", "created_at": 0, "updated_at": 0 })).unwrap();
    d.graph = graph(template);
    d
}

#[tokio::test]
async fn the_first_save_is_version_one_then_one_more_each_time() {
    let state = crate::server::tests::test_state();
    let store = &state.id_store;
    let v1 = store.version_save("d1", &graph("a"), "operator", 1_000).unwrap();
    let v2 = store.version_save("d1", &graph("b"), "agenty", 2_000).unwrap();
    assert_eq!((v1.version, v2.version), (1, 2));
    assert_eq!((v1.drone_id.as_str(), v1.saved_by.as_str(), v1.saved_at), ("d1", "operator", 1_000));
    assert_eq!((v2.saved_by.as_str(), v2.saved_at), ("agenty", 2_000));
    assert_eq!(v1.hash, graph_hash(&graph("a")));
    assert_eq!(store.version_save("other", &graph("a"), "operator", 3_000).unwrap().version, 1, "numbering is per drone");
}

#[tokio::test]
async fn a_saved_version_never_changes_however_the_drone_is_edited() {
    let state = crate::server::tests::test_state();
    let store = &state.id_store;
    store.version_save("d1", &graph("first"), "operator", 1).unwrap();
    store.version_save("d1", &graph("second"), "operator", 2).unwrap();
    let again = store.version_get("d1", 1).unwrap().unwrap();
    assert_eq!(graph_hash(&again.graph), graph_hash(&graph("first")));
    assert_eq!(serde_json::to_value(&again.graph).unwrap(), serde_json::to_value(graph("first")).unwrap());
    assert_eq!(store.version_get("d1", 3).unwrap().map(|v| v.version), None);
    assert_eq!(store.version_get("nobody", 1).unwrap().map(|v| v.version), None);
}

#[tokio::test]
async fn a_job_pinned_to_a_number_gets_that_version_and_latest_follows_the_newest() {
    let state = crate::server::tests::test_state();
    let store = &state.id_store;
    for (i, t) in ["a", "b", "c"].iter().enumerate() {
        store.version_save("d1", &graph(t), "operator", i as i64).unwrap();
    }
    let pinned = store.version_resolve("d1", Pin::Number(2)).unwrap().unwrap();
    assert_eq!((pinned.version, pinned.hash), (2, graph_hash(&graph("b"))));
    assert_eq!(store.version_resolve("d1", Pin::Latest).unwrap().unwrap().version, 3);
    store.version_save("d1", &graph("d"), "operator", 9).unwrap();
    assert_eq!(store.version_resolve("d1", Pin::Latest).unwrap().unwrap().version, 4);
    assert_eq!(store.version_resolve("d1", Pin::Number(2)).unwrap().unwrap().version, 2, "a pin does not move");
    assert!(store.version_resolve("d1", Pin::Number(99)).unwrap().is_none());
    assert!(store.version_resolve("nobody", Pin::Latest).unwrap().is_none());
}

#[tokio::test]
async fn the_list_is_newest_first_and_limited() {
    let state = crate::server::tests::test_state();
    let store = &state.id_store;
    for i in 0..5 {
        store.version_save("d1", &graph(&format!("t{i}")), "operator", i).unwrap();
    }
    let all: Vec<i64> = store.version_list("d1", 50).unwrap().iter().map(|v| v.version).collect();
    assert_eq!(all, [5, 4, 3, 2, 1]);
    assert_eq!(store.version_list("d1", 2).unwrap().len(), 2);
    assert_eq!(store.version_list("d1", 0).unwrap().len(), 1, "the limit is clamped to at least 1");
    assert_eq!(store.version_list("d1", 10_000).unwrap().len(), 5, "and to at most 200");
    assert!(store.version_list("nobody", 10).unwrap().is_empty());
}

#[tokio::test]
async fn saving_a_drone_through_the_handler_makes_a_version_each_time() {
    let state = crate::server::tests::test_state();
    let saved = upsert_drone(&state, drone("", "one")).unwrap();
    assert_eq!(state.id_store.version_list(&saved.id, 10).unwrap().len(), 1);
    let mut edited = saved.clone();
    edited.graph = graph("two");
    upsert_drone(&state, edited).unwrap();
    let versions = state.id_store.version_list(&saved.id, 10).unwrap();
    assert_eq!(versions.iter().map(|v| v.version).collect::<Vec<_>>(), [2, 1]);
    assert_eq!(versions[0].hash, graph_hash(&graph("two")));
    assert_eq!(versions[1].hash, graph_hash(&graph("one")), "version 1 is still what was first saved");
    assert_eq!(state.id_store.drone_get(&saved.id).unwrap().unwrap().graph.nodes[0].data["template"], "two", "the definition holds the newest graph");
}

#[tokio::test]
async fn deleting_a_drone_deletes_its_versions_and_only_its_own() {
    let state = crate::server::tests::test_state();
    let a = upsert_drone(&state, drone("", "a")).unwrap();
    let b = upsert_drone(&state, drone("", "b")).unwrap();
    assert!(delete_drone(&state, DeleteDroneReq { id: a.id.clone() }).unwrap().deleted);
    assert!(state.id_store.version_list(&a.id, 10).unwrap().is_empty());
    assert_eq!(state.id_store.version_list(&b.id, 10).unwrap().len(), 1);
    assert_eq!(state.id_store.versions_delete_for(&b.id).unwrap(), 1);
    assert_eq!(state.id_store.versions_delete_for(&b.id).unwrap(), 0, "a second delete finds nothing");
}

#[tokio::test]
async fn the_table_has_no_foreign_key_to_the_definitions() {
    // Versions of a drone that is only in another store are still saved (m0038).
    let state = crate::server::tests::test_state();
    assert_eq!(state.id_store.version_save("only-in-another-store", &graph("x"), "operator", 1).unwrap().version, 1);
}
