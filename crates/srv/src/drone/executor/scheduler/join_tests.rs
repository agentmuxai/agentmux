// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A Join block brings branches back together.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::mpsc;

use super::super::{execute, Limits};
use crate::drone::executor::engine::RunEnd;
use crate::drone::executor::schedule;
use crate::drone::types::{DroneGraph, FlowEdge, FlowNode, NodePosition};

fn node(id: &str, data: Value) -> FlowNode {
    FlowNode { id: id.to_string(), position: NodePosition::default(), data, node_type: None }
}

fn edge(id: &str, source: &str, target: &str, handle: Option<&str>) -> FlowEdge {
    FlowEdge { id: id.to_string(), source: source.to_string(), target: target.to_string(), source_handle: handle.map(str::to_string), target_handle: None }
}

/// A branch that works: it sets a variable of its own.
fn works(id: &str) -> FlowNode {
    node(id, json!({ "kind": "variables", "entries": [{ "name": format!("v_{id}"), "value": "1" }] }))
}

/// A branch that fails.
fn breaks(id: &str) -> FlowNode {
    node(id, json!({ "kind": "variables", "entries": [{ "name": format!("v_{id}"), "value": "1", "reducer": "bogus" }] }))
}

fn join(data: Value) -> FlowNode {
    let mut full = json!({ "kind": "join" });
    full.as_object_mut().unwrap().extend(data.as_object().cloned().unwrap_or_default());
    node("j", full)
}

/// `start` fans out to `branches`, which all go into the join `j`, then a Response says `template`.
fn fan(branches: Vec<FlowNode>, j: FlowNode, template: &str) -> DroneGraph {
    let mut nodes = vec![node("start", json!({ "kind": "variables", "entries": [{ "name": "s", "value": "1" }] })), j, node("out", json!({ "kind": "response", "template": template }))];
    let mut edges = vec![edge("to-out", "j", "out", None)];
    for b in branches {
        edges.push(edge(&format!("s-{}", b.id), "start", &b.id, None));
        edges.push(edge(&format!("{}-j", b.id), &b.id, "j", None));
        nodes.push(b);
    }
    DroneGraph { nodes, edges }
}

async fn run(graph: DroneGraph) -> Result<Value, String> {
    let (tx, _rx) = mpsc::unbounded_channel();
    let states = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
    match execute("run-1", (&graph, Value::Null, Limits::default(), "main"), &tx, &states).await {
        Ok(v) => Ok(v),
        Err(RunEnd::Failed(e) | RunEnd::Cancelled(e)) => Err(e),
    }
}

#[tokio::test]
async fn a_join_runs_when_every_branch_has_arrived_and_has_what_each_said() {
    let g = fan(vec![works("b1"), works("b2")], join(json!({})), "{{j.arrived}} arrived");
    assert_eq!(run(g).await, Ok(json!("2 arrived")));
}

#[tokio::test]
async fn a_failed_branch_fails_the_run_unless_the_join_skips_it() {
    let g = fan(vec![works("b1"), breaks("b2")], join(json!({ "mode": "any" })), "never");
    assert!(run(g).await.unwrap_err().contains("block b2 failed"));

    let g = fan(vec![works("b1"), breaks("b2")], join(json!({ "mode": "any", "on_branch_error": "skip" })), "{{j.arrived}} arrived, {{j.failed}} failed");
    let said = run(g).await.unwrap();
    assert!(said.as_str().unwrap().starts_with("1 arrived, "), "{said}");
    assert!(said.as_str().unwrap().contains("b2"), "{said}");
}

#[tokio::test]
async fn all_does_not_hold_with_a_branch_left_out_and_n_of_m_counts_the_ones_that_arrived() {
    let g = fan(vec![works("b1"), breaks("b2")], join(json!({ "mode": "all", "on_branch_error": "skip" })), "never");
    assert!(run(g).await.unwrap_err().contains("every branch must arrive"));

    let three = |n: u64| fan(vec![works("b1"), works("b3"), breaks("b2")], join(json!({ "mode": "n_of_m", "n": n, "on_branch_error": "skip" })), "{{j.arrived}} of 3");
    assert_eq!(run(three(2)).await, Ok(json!("2 of 3")));
    assert!(run(three(3)).await.unwrap_err().contains("2 of 3 branches arrived"));
}

#[tokio::test]
async fn a_join_that_every_branch_failed_into_says_so_and_is_not_skipped() {
    let g = fan(vec![breaks("b1"), breaks("b2")], join(json!({ "mode": "any", "on_branch_error": "skip" })), "never");
    let why = run(g).await.unwrap_err();
    assert!(why.contains("no branch arrived") && why.contains("b1") && why.contains("b2"), "{why}");
}

#[tokio::test]
async fn a_branch_that_did_not_run_is_pruned_and_does_not_count_against_the_join() {
    // A Condition sends the run down one of two branches; both lead into the join.
    let graph = DroneGraph {
        nodes: vec![
            node("start", json!({ "kind": "condition", "expr": "1 == 1" })),
            works("yes"),
            works("no"),
            join(json!({})),
            node("out", json!({ "kind": "response", "template": "{{j.arrived}} arrived, pruned {{j.pruned}}" })),
        ],
        edges: vec![
            edge("e1", "start", "yes", Some("true")),
            edge("e2", "start", "no", Some("false")),
            edge("e3", "yes", "j", None),
            edge("e4", "no", "j", None),
            edge("e5", "j", "out", None),
        ],
    };
    let said = run(graph).await.unwrap();
    assert!(said.as_str().unwrap().starts_with("1 arrived, pruned "), "{said}");
    assert!(said.as_str().unwrap().contains("no"), "{said}");
}

#[test]
fn a_join_with_one_branch_or_a_count_it_cannot_reach_is_refused_before_the_run_starts() {
    let one = DroneGraph { nodes: vec![works("b1"), join(json!({}))], edges: vec![edge("e1", "b1", "j", None)] };
    let issues = schedule::analyze(&one).unwrap_err();
    assert!(issues.iter().any(|i| i.message.contains("needs at least two")), "{issues:?}");

    let many = fan(vec![works("b1"), works("b2")], join(json!({ "mode": "n_of_m", "n": 3 })), "x");
    let issues = schedule::analyze(&many).unwrap_err();
    assert!(issues.iter().any(|i| i.message.contains("only 2 branches")), "{issues:?}");
}
