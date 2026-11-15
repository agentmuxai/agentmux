// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Branches that are ready together run together.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::mpsc;

use super::super::{execute, Limits};
use super::{MAX_AGENTS, PARALLELISM};
use crate::drone::executor::engine::{RunEnd, RunEvent};
use crate::drone::types::{DroneGraph, FlowEdge, FlowNode, NodePosition};

fn node(id: &str, data: Value) -> FlowNode {
    FlowNode { id: id.to_string(), position: NodePosition::default(), data, node_type: None }
}

fn edge(id: &str, source: &str, target: &str) -> FlowEdge {
    FlowEdge { id: id.to_string(), source: source.to_string(), target: target.to_string(), source_handle: None, target_handle: None }
}

/// `start` fans out to blocks `c1`..`cN`, each made by `make`.
fn fan(count: usize, make: impl Fn(&str) -> FlowNode) -> DroneGraph {
    let mut nodes = vec![node("start", json!({ "kind": "variables", "entries": [{ "name": "s", "value": "1" }] }))];
    let mut edges = Vec::new();
    for n in 1..=count {
        let id = format!("c{n}");
        edges.push(edge(&format!("e{n}"), "start", &id));
        nodes.push(make(&id));
    }
    DroneGraph { nodes, edges }
}

fn condition(id: &str) -> FlowNode {
    node(id, json!({ "kind": "condition", "expr": "1 == 1" }))
}

async fn run(graph: DroneGraph) -> (Result<Value, String>, Vec<RunEvent>) {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let states = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
    let ended = execute("run-1", (&graph, Value::Null, Limits::default(), "main"), &tx, &states).await;
    drop(tx);
    let mut events = Vec::new();
    while let Some(ev) = rx.recv().await {
        events.push(ev);
    }
    (ended.map_err(|end| match end {
        RunEnd::Failed(e) | RunEnd::Cancelled(e) => e,
    }), events)
}

fn started(events: &[RunEvent]) -> Vec<&str> {
    events.iter().filter_map(|e| if let RunEvent::BlockStarted { block_id, .. } = e { Some(block_id.as_str()) } else { None }).collect()
}

fn done(events: &[RunEvent]) -> Vec<&str> {
    events.iter().filter_map(|e| if let RunEvent::BlockDone { block_id, .. } = e { Some(block_id.as_str()) } else { None }).collect()
}

/// How many of the blocks `c*` had started when the first of them ended, however it ended.
fn started_before_the_first_ends(events: &[RunEvent]) -> usize {
    let mut started = 0;
    for ev in events {
        match ev {
            RunEvent::BlockStarted { block_id, .. } if block_id.starts_with('c') => started += 1,
            RunEvent::BlockDone { block_id, .. } | RunEvent::BlockError { block_id, .. } if block_id.starts_with('c') => return started,
            _ => {}
        }
    }
    started
}

#[tokio::test]
async fn blocks_that_are_ready_together_start_together_and_are_taken_up_in_order() {
    let (result, events) = run(fan(3, condition)).await;
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(started_before_the_first_ends(&events), 3);
    assert_eq!(done(&events), ["start", "c1", "c2", "c3"]);
    assert_eq!(started(&events), ["start", "c1", "c2", "c3"]);
}

#[tokio::test]
async fn no_more_than_the_parallelism_run_at_once_and_the_rest_wait_their_turn() {
    let (result, events) = run(fan(PARALLELISM + 2, condition)).await;
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(started_before_the_first_ends(&events), PARALLELISM);
    assert_eq!(done(&events).len(), PARALLELISM + 3, "all of them ran: {:?}", done(&events));
}

#[tokio::test]
async fn no_more_agents_than_the_fleet_allows_start_at_once() {
    // An Agent block with no task fails the moment it runs, which is enough to see how many started.
    let (result, events) = run(fan(MAX_AGENTS + 2, |id| node(id, json!({ "kind": "agent" })))).await;
    assert!(result.unwrap_err().contains("block c1 failed"));
    assert_eq!(started_before_the_first_ends(&events), MAX_AGENTS);
}

#[tokio::test]
async fn a_branch_that_fails_does_not_lose_what_the_others_said() {
    let mut g = fan(3, condition);
    g.nodes[2] = node("c2", json!({ "kind": "condition", "expr": "(" }));
    let (result, events) = run(g).await;
    assert!(result.unwrap_err().contains("block c2 failed"));
    assert_eq!(done(&events), ["start", "c1", "c3"], "the others were taken up");
    assert!(events.iter().any(|e| matches!(e, RunEvent::BlockError { block_id, .. } if block_id == "c2")));
}

#[tokio::test]
async fn a_block_that_writes_the_run_runs_by_itself() {
    // Two Variables blocks are not batched: each is done before the next starts.
    let (result, events) = run(fan(2, |id| node(id, json!({ "kind": "variables", "entries": [{ "name": format!("v_{id}"), "value": "1" }] })))).await;
    assert!(result.is_ok(), "{result:?}");
    assert_eq!(started_before_the_first_ends(&events), 1);
}
