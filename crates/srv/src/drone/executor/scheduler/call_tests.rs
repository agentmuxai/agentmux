// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A Drone block calls another drone as a step.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::mpsc;

use super::super::{execute, Limits};
use super::MAX_DEPTH;
use crate::drone::executor::engine::{Cancel, RunEnd, RunEvent};
use crate::drone::host::DroneLookup;
use crate::drone::types::{DroneGraph, FlowEdge, FlowNode, NodePosition};

fn node(id: &str, data: Value) -> FlowNode {
    FlowNode { id: id.to_string(), position: NodePosition::default(), data, node_type: None }
}

fn edge(id: &str, source: &str, target: &str, handle: Option<&str>) -> FlowEdge {
    FlowEdge { id: id.to_string(), source: source.to_string(), target: target.to_string(), source_handle: handle.map(str::to_string), target_handle: None }
}

fn response(id: &str, template: &str) -> FlowNode {
    node(id, json!({ "kind": "response", "template": template }))
}

fn call(id: &str, drone: &str, inputs: Value) -> FlowNode {
    node(id, json!({ "kind": "drone", "drone": drone, "inputs": inputs }))
}

/// The drones a test has saved, by id; every one is version 7.
struct Saved(HashMap<String, DroneGraph>);

impl DroneLookup for Saved {
    fn graph(&self, id: &str, version: Option<i64>) -> Result<(DroneGraph, i64), String> {
        match (self.0.get(id), version) {
            (Some(_), Some(n)) if n != 7 => Err(format!("drone {id} has no saved version {n}")),
            (Some(g), _) => Ok((g.clone(), 7)),
            (None, _) => Err(format!("there is no drone {id}")),
        }
    }
}

/// A drone that answers with what it is given as `x`.
fn echo() -> DroneGraph {
    DroneGraph { nodes: vec![response("out", "got {{trigger.x}}")], edges: vec![] }
}

/// A drone that calls `callee` once and answers with what it said.
fn caller_of(callee: &str) -> DroneGraph {
    DroneGraph {
        nodes: vec![call("call", callee, json!({ "x": "hi" })), response("out", "{{call.value}}")],
        edges: vec![edge("e1", "call", "out", None)],
    }
}

fn saved(drones: Vec<(&str, DroneGraph)>) -> Option<Arc<dyn DroneLookup>> {
    Some(Arc::new(Saved(drones.into_iter().map(|(id, g)| (id.to_string(), g)).collect())))
}

struct Ran {
    result: Result<Value, String>,
    events: Vec<RunEvent>,
}

/// Run `graph` as the drone `id`, with these drones saved.
async fn run_as(id: &str, graph: DroneGraph, trigger: Value, lookup: Option<Arc<dyn DroneLookup>>) -> Ran {
    let (tx, mut rx) = mpsc::unbounded_channel();
    let states = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
    let limits = Limits { lookup, ..Limits::default() };
    let ended = execute("run-1", (&graph, trigger, limits, id), &tx, &states).await;
    drop(tx);
    let mut events = Vec::new();
    while let Some(ev) = rx.recv().await {
        events.push(ev);
    }
    let result = match ended {
        Ok(v) => Ok(v),
        Err(RunEnd::Failed(e) | RunEnd::Cancelled(e)) => Err(e),
    };
    Ran { result, events }
}

#[tokio::test]
async fn a_drone_block_runs_the_other_drone_with_its_inputs_and_takes_what_it_says() {
    let graph = DroneGraph {
        nodes: vec![call("call", "echo", json!({ "x": "{{trigger.word}}!" })), response("out", "{{call.value}} (v{{call.version}})")],
        edges: vec![edge("e1", "call", "out", None)],
    };
    let ran = run_as("main", graph, json!({ "word": "hello" }), saved(vec![("echo", echo())])).await;
    assert_eq!(ran.result, Ok(json!("got hello! (v7)")));
    let started: Vec<_> = ran.events.iter().filter_map(|e| if let RunEvent::ChildStarted { block_id, drone_id, version, .. } = e { Some((block_id.as_str(), drone_id.as_str(), *version)) } else { None }).collect();
    assert_eq!(started, [("call", "echo", 7)]);
    assert!(ran.events.iter().any(|e| matches!(e, RunEvent::ChildEnded { status, cost_usd, .. } if status == "done" && *cost_usd == 0.0)));
    assert!(!ran.events.iter().any(|e| matches!(e, RunEvent::RunDone { run_id, .. } if run_id != "run-1")), "the callee's own events are not the caller's");
}

#[tokio::test]
async fn a_version_that_was_pinned_is_the_one_called_and_one_that_is_missing_fails_the_block() {
    let pinned = |v: i64| DroneGraph { nodes: vec![node("call", json!({ "kind": "drone", "drone": "echo", "version": v })), response("out", "{{call.version}}")], edges: vec![edge("e1", "call", "out", None)] };
    assert_eq!(run_as("main", pinned(7), Value::Null, saved(vec![("echo", echo())])).await.result, Ok(json!("7")));
    let why = run_as("main", pinned(2), Value::Null, saved(vec![("echo", echo())])).await.result.unwrap_err();
    assert!(why.contains("block call failed: drone echo has no saved version 2"), "{why}");
    let why = run_as("main", caller_of("ghost"), Value::Null, saved(vec![])).await.result.unwrap_err();
    assert!(why.contains("there is no drone ghost"), "{why}");
    let why = run_as("main", caller_of("echo"), Value::Null, None).await.result.unwrap_err();
    assert!(why.contains("there are no drones to call from here"), "{why}");
}

#[tokio::test]
async fn a_callee_that_fails_fails_the_block_and_its_error_edge_takes_it() {
    let breaks = DroneGraph { nodes: vec![node("bad", json!({ "kind": "variables", "entries": [{ "name": "x", "value": "1", "reducer": "bogus" }] }))], edges: vec![] };
    let ran = run_as("main", caller_of("breaks"), Value::Null, saved(vec![("breaks", breaks.clone())])).await;
    assert!(ran.result.unwrap_err().contains("block call failed: drone breaks failed: block bad failed"));
    assert!(ran.events.iter().any(|e| matches!(e, RunEvent::ChildEnded { status, .. } if status == "failed")));

    let mut handled = caller_of("breaks");
    handled.nodes.push(response("handler", "caught: {{call.error}}"));
    handled.edges.push(edge("e2", "call", "handler", Some("error")));
    let ran = run_as("main", handled, Value::Null, saved(vec![("breaks", breaks)])).await;
    assert!(ran.result.unwrap().as_str().unwrap().starts_with("caught: block call failed: drone breaks failed"));
}

#[tokio::test]
async fn a_drone_cannot_call_itself_directly_or_through_the_ones_it_calls() {
    let drones = || saved(vec![("a", caller_of("b")), ("b", caller_of("a")), ("me", caller_of("me"))]);
    let why = run_as("a", caller_of("b"), Value::Null, drones()).await.result.unwrap_err();
    assert!(why.contains("drone a cannot be called from b: it is already running above it (a > b)"), "{why}");
    let why = run_as("me", caller_of("me"), Value::Null, drones()).await.result.unwrap_err();
    assert!(why.contains("drone me cannot be called from me"), "{why}");
}

#[tokio::test]
async fn calls_nest_at_most_four_deep() {
    // d1 calls d2 calls d3 calls d4 calls d5: the fifth is one too many.
    let chain: Vec<(String, DroneGraph)> = (1..=5).map(|n| (format!("d{n}"), if n < 5 { caller_of(&format!("d{}", n + 1)) } else { echo() })).collect();
    let lookup = || saved(chain.iter().map(|(id, g)| (id.as_str(), g.clone())).collect());
    let why = run_as("d1", caller_of("d2"), Value::Null, lookup()).await.result.unwrap_err();
    assert!(why.contains(&format!("drones call at most {MAX_DEPTH} deep")), "{why}");
    // One level less is allowed.
    let short = saved(chain[..4].iter().map(|(id, g)| (id.as_str(), if id == "d4" { echo() } else { g.clone() })).collect());
    assert_eq!(run_as("d1", caller_of("d2"), Value::Null, short).await.result, Ok(json!("got hi")));
}

#[tokio::test]
async fn a_for_each_can_have_a_drone_for_its_body() {
    let graph = DroneGraph {
        nodes: vec![
            node("each", json!({ "kind": "foreach", "items": "{{trigger.list}}", "body": "work" })),
            call("work", "echo", json!({ "x": "{{item.name}}" })),
            response("out", "{{each.count}} done"),
        ],
        edges: vec![edge("e1", "each", "out", None)],
    };
    let list = json!({ "list": [{ "name": "a" }, { "name": "b" }, { "name": "c" }] });
    let ran = run_as("main", graph, list, saved(vec![("echo", echo())])).await;
    assert_eq!(ran.result, Ok(json!("3 done")));
    assert_eq!(ran.events.iter().filter(|e| matches!(e, RunEvent::ChildStarted { .. })).count(), 3);
}

#[tokio::test]
async fn stopping_the_run_stops_the_drone_it_called() {
    let waits = DroneGraph { nodes: vec![node("hold", json!({ "kind": "wait", "on": "never" }))], edges: vec![] };
    let graph = caller_of("waits");
    let cancel = Cancel::default();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let states = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
    let limits = Limits { cancel: cancel.clone(), lookup: saved(vec![("waits", waits)]), ..Limits::default() };
    let task = tokio::spawn(async move { execute("run-1", (&graph, Value::Null, limits, "main"), &tx, &states).await });
    while let Some(ev) = rx.recv().await {
        if matches!(ev, RunEvent::ChildStarted { .. }) {
            break;
        }
    }
    assert!(cancel.cancel("asked to stop"));
    match task.await.unwrap() {
        Err(RunEnd::Cancelled(why)) => assert_eq!(why, "asked to stop"),
        Ok(_) => panic!("expected the run to end cancelled, but it finished"),
        Err(RunEnd::Failed(why)) => panic!("expected the run to end cancelled, but it failed: {why}"),
    }
    let mut ended = None;
    while let Some(ev) = rx.recv().await {
        if let RunEvent::ChildEnded { status, .. } = ev {
            ended = Some(status);
        }
    }
    assert_eq!(ended.as_deref(), Some("cancelled"));
}

#[tokio::test]
async fn a_call_that_runs_out_of_its_own_time_stops_the_drone_it_called() {
    let waits = DroneGraph { nodes: vec![node("hold", json!({ "kind": "wait", "on": "never" }))], edges: vec![] };
    let graph = DroneGraph { nodes: vec![node("call", json!({ "kind": "drone", "drone": "waits", "timeout_seconds": 1 }))], edges: vec![] };
    let started = std::time::Instant::now();
    let ran = run_as("main", graph, Value::Null, saved(vec![("waits", waits)])).await;
    assert!(ran.result.unwrap_err().contains("block call failed: timed out after 1 seconds"));
    // The callee was stopped with the call, and said so: it is not left running.
    assert!(ran.events.iter().any(|e| matches!(e, RunEvent::ChildEnded { status, .. } if status == "cancelled")), "{:?}", ran.events.len());
    assert!(started.elapsed() < std::time::Duration::from_secs(10), "{:?}", started.elapsed());
}

#[tokio::test]
async fn what_a_stopped_callee_spent_is_taken_before_the_next_block_starts() {
    let waits = DroneGraph { nodes: vec![node("hold", json!({ "kind": "wait", "on": "never" }))], edges: vec![] };
    let graph = DroneGraph {
        nodes: vec![node("call", json!({ "kind": "drone", "drone": "waits", "timeout_seconds": 1 })), response("handler", "went on")],
        edges: vec![edge("e1", "call", "handler", Some("error"))],
    };
    let ran = run_as("main", graph, Value::Null, saved(vec![("waits", waits)])).await;
    assert_eq!(ran.result, Ok(json!("went on")));
    let at = |f: &dyn Fn(&RunEvent) -> bool| ran.events.iter().position(f).unwrap();
    let ended = at(&|e| matches!(e, RunEvent::ChildEnded { .. }));
    let next = at(&|e| matches!(e, RunEvent::BlockStarted { block_id, .. } if block_id == "handler"));
    assert!(ended < next, "the stopped callee ended ({ended}) before the next block started ({next})");
}

#[test]
fn what_a_stopped_callee_spent_waits_for_it_to_end_and_is_taken_once() {
    let late = super::LateSpend::default();
    late.start("c1", "call");
    assert!(!late.idle());
    late.finish("c1", 0.5);
    assert!(late.idle());
    assert_eq!(late.take(), [("call".to_string(), 0.5)]);
    assert!(late.take().is_empty());
    // A caller that took the spend itself leaves nothing to take twice.
    late.start("c2", "call");
    late.finish("c2", 1.0);
    late.forget("c2");
    assert!(late.take().is_empty());
}

#[tokio::test]
async fn spend_that_was_not_in_a_blocks_output_is_added_to_its_state() {
    use crate::backend::storage::drone_types::BlockState;
    let states = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
    let state = |output| BlockState { status: "error".to_string(), output, error: Some("x".to_string()), started_at: None, completed_at: None };
    states.lock().await.insert("a".to_string(), state(None));
    states.lock().await.insert("b".to_string(), state(Some(json!({ "cost_usd": 0.25, "value": 1 }))));
    super::add_cost(&states, "a", 0.5).await;
    super::add_cost(&states, "b", 0.5).await;
    super::add_cost(&states, "ghost", 0.5).await;
    let g = states.lock().await;
    assert_eq!(g["a"].output, Some(json!({ "cost_usd": 0.5 })));
    assert_eq!(g["b"].output, Some(json!({ "cost_usd": 0.75, "value": 1 })));
}

#[tokio::test]
async fn a_call_set_wrongly_is_refused_before_the_run_starts() {
    let none = DroneGraph { nodes: vec![node("call", json!({ "kind": "drone" }))], edges: vec![] };
    let why = run_as("main", none, Value::Null, saved(vec![])).await.result.unwrap_err();
    assert!(why.contains("needs `drone`"), "{why}");
}

#[test]
fn what_a_callee_may_spend_is_what_its_caller_has_left() {
    assert_eq!(super::remaining_cap(Some(5.0), 1.25), Some(3.75));
    assert_eq!(super::remaining_cap(Some(5.0), 9.0), Some(0.0), "never below nothing");
    assert_eq!(super::remaining_cap(None, 3.0), None, "no cap to share");
}
