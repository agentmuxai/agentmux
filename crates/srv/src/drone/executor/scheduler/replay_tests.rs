// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A replay starts a new run from a step of an earlier one and does not repeat the world.

use std::collections::{BTreeMap, HashMap};
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::mpsc;

use super::super::{execute, replay, Limits};
use crate::drone::checkpoint::{Checkpoint, Fork, GraphRef};
use crate::drone::executor::engine::{RunEnd, RunEvent};
use crate::drone::host::DroneLookup;
use crate::drone::recorder::{load, MemorySink, Persist};
use crate::drone::replay::{fork_checkpoint, Replay};
use crate::drone::types::{DroneGraph, FlowEdge, FlowNode, NodePosition};

fn node(id: &str, data: Value) -> FlowNode {
    FlowNode { id: id.to_string(), position: NodePosition::default(), data, node_type: None }
}

fn edge(id: &str, source: &str, target: &str) -> FlowEdge {
    FlowEdge { id: id.to_string(), source: source.to_string(), target: target.to_string(), source_handle: None, target_handle: None }
}

fn start() -> FlowNode {
    node("start", json!({ "kind": "variables", "entries": [{ "name": "s", "value": "1" }] }))
}

fn response(id: &str, template: &str) -> FlowNode {
    node(id, json!({ "kind": "response", "template": template }))
}

struct Saved(HashMap<String, DroneGraph>);

impl DroneLookup for Saved {
    fn graph(&self, id: &str, _version: Option<i64>) -> Result<(DroneGraph, i64), String> {
        self.0.get(id).map(|g| (g.clone(), 7)).ok_or_else(|| format!("there is no drone {id}"))
    }
}

fn saved() -> Option<Arc<dyn DroneLookup>> {
    let echo = DroneGraph { nodes: vec![response("out", "got {{trigger.x}}")], edges: vec![] };
    Some(Arc::new(Saved(HashMap::from([("echo".to_string(), echo)]))))
}

fn graph_ref() -> GraphRef {
    GraphRef { version: 1, hash: "h".to_string() }
}

struct Ran {
    result: Result<Value, String>,
    events: Vec<RunEvent>,
    sink: Arc<MemorySink>,
}

impl Ran {
    /// What block `id` said when it was done.
    fn said(&self, id: &str) -> Option<Value> {
        self.events.iter().find_map(|e| if let RunEvent::BlockDone { block_id, output, .. } = e { (block_id == id).then(|| output.clone()) } else { None })
    }
}

async fn drain(ended: Result<Value, RunEnd>, mut rx: mpsc::UnboundedReceiver<RunEvent>, sink: Arc<MemorySink>) -> Ran {
    let mut events = Vec::new();
    while let Some(ev) = rx.recv().await {
        events.push(ev);
    }
    let result = ended.map_err(|end| match end {
        RunEnd::Failed(e) | RunEnd::Cancelled(e) => e,
    });
    Ran { result, events, sink }
}

async fn first_run(graph: &DroneGraph) -> Ran {
    let (tx, rx) = mpsc::unbounded_channel();
    let states = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
    let sink = Arc::new(MemorySink::default());
    let limits = Limits { persist: Some(Persist { sink: sink.clone(), graph: graph_ref() }), lookup: saved(), ..Limits::default() };
    let ended = execute("run-1", (graph, Value::Null, limits, "d"), &tx, &states).await;
    drop(tx);
    drain(ended, rx, sink).await
}

/// Replay `graph` from checkpoint `step` of `first`, in a run of its own, with `original` as what its stubs say.
async fn replayed_with(graph: &DroneGraph, first: &Ran, (step, real): (usize, bool), (original, bits): (BTreeMap<String, Value>, BTreeMap<String, (bool, bool)>)) -> Ran {
    let rows = first.sink.rows_of("run-1");
    let at: Checkpoint = load(&rows[..=step]).unwrap().unwrap();
    let fork = Fork { run_id: "run-1".to_string(), step: step as u32, real };
    let c = fork_checkpoint(&at, "run-2", fork.clone());
    let sink = Arc::new(MemorySink::default());
    let limits = Limits { persist: Some(Persist { sink: sink.clone(), graph: graph_ref() }), lookup: saved(), replay: Some(Arc::new(Replay { fork, original, bits })), ..Limits::default() };
    let (tx, rx) = mpsc::unbounded_channel();
    let states = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
    let ended = replay((graph, &c, limits), &tx, &states).await;
    drop(tx);
    drain(ended, rx, sink).await
}

/// Replay from `step` of `first`, with what its newest checkpoint says.
async fn replayed(graph: &DroneGraph, first: &Ran, step: usize, real: bool) -> Ran {
    let newest: Checkpoint = load(&first.sink.rows_of("run-1")).unwrap().unwrap();
    replayed_with(graph, first, (step, real), (newest.outputs, newest.bits)).await
}

fn calls_a_drone() -> DroneGraph {
    DroneGraph {
        nodes: vec![start(), node("call", json!({ "kind": "drone", "drone": "echo", "inputs": { "x": "hi" } })), response("out", "{{call.value}}")],
        edges: vec![edge("e1", "start", "call"), edge("e2", "call", "out")],
    }
}

fn children(ran: &Ran) -> usize {
    ran.events.iter().filter(|e| matches!(e, RunEvent::ChildStarted { .. })).count()
}

#[tokio::test]
async fn a_replay_does_not_call_the_drone_again_and_the_block_says_what_it_said_before() {
    let g = calls_a_drone();
    let first = first_run(&g).await;
    assert_eq!(first.result, Ok(json!("got hi")));
    assert_eq!(children(&first), 1);
    let again = replayed(&g, &first, 1, false).await;
    assert_eq!(again.result, Ok(json!("got hi")), "the rest of the run read what the stub said");
    assert_eq!(children(&again), 0, "the callee did not run");
    let said = again.said("call").unwrap();
    assert_eq!(said["stubbed"], json!(true));
    assert_eq!(said["value"], json!("got hi"));
}

#[tokio::test]
async fn asked_to_do_it_for_real_the_drone_is_called_again() {
    let g = calls_a_drone();
    let first = first_run(&g).await;
    let again = replayed(&g, &first, 1, true).await;
    assert_eq!(again.result, Ok(json!("got hi")));
    assert_eq!(children(&again), 1);
}

#[tokio::test]
async fn a_replay_is_a_run_of_its_own_that_says_where_it_came_from() {
    let g = calls_a_drone();
    let first = first_run(&g).await;
    let kept = first.sink.rows_of("run-1");
    let again = replayed(&g, &first, 1, false).await;
    let rows = again.sink.rows_of("run-2");
    assert_eq!(rows[0].step, 0);
    assert_eq!(rows[0].kind, "full", "its checkpoints begin whole, like any run's");
    let last: Checkpoint = load(&rows).unwrap().unwrap();
    assert_eq!(last.run_id, "run-2");
    assert_eq!(last.fork, Some(Fork { run_id: "run-1".to_string(), step: 1, real: false }));
    assert!(first.sink.rows_of("run-2").is_empty() && first.sink.rows_of("run-1") == kept, "the run replayed is untouched");
    assert_eq!(last.cost_usd, 0.0);
}

#[tokio::test]
async fn blocks_that_start_together_are_stubbed_too_and_a_block_that_only_reads_still_runs() {
    // Two messages in one batch, which would fail here (no server) if they ran, and a condition beside them.
    let g = DroneGraph {
        nodes: vec![
            start(),
            node("m1", json!({ "kind": "message", "to": "agenty", "body": "a" })),
            node("m2", json!({ "kind": "message", "to": "loap", "body": "b" })),
            node("c", json!({ "kind": "condition", "expr": "1 == 1" })),
            response("out", "{{m1.delivered}}"),
        ],
        edges: vec![edge("e1", "start", "m1"), edge("e2", "start", "m2"), edge("e3", "start", "c"), edge("e4", "m1", "out")],
    };
    let first = first_run(&g).await;
    assert!(first.result.is_err(), "run for real, a message has nowhere to go here");
    let original = BTreeMap::from([("m1".to_string(), json!({ "delivered": true })), ("m2".to_string(), json!({ "delivered": true }))]);
    let again = replayed_with(&g, &first, (1, false), (original, BTreeMap::new())).await;
    assert_eq!(again.result, Ok(json!("true")), "{:?}", again.result);
    assert_eq!(again.said("m2").unwrap()["stubbed"], json!(true));
    assert!(again.said("c").is_some_and(|o| o.get("stubbed").is_none()), "a condition ran as it always does");
}

fn error_edge(id: &str, source: &str, target: &str) -> FlowEdge {
    FlowEdge { source_handle: Some("error".to_string()), ..edge(id, source, target) }
}

/// A message that fails here (it has nowhere to go) and a graph that handles it: `ok` if it worked, `handled` if not.
fn a_message_the_graph_handles() -> DroneGraph {
    DroneGraph {
        nodes: vec![start(), node("m", json!({ "kind": "message", "to": "agenty", "body": "a" })), response("ok", "fine"), response("handled", "caught {{m.failed}}")],
        edges: vec![edge("e1", "start", "m"), edge("e2", "m", "ok"), error_edge("e3", "m", "handled")],
    }
}

#[tokio::test]
async fn a_block_that_failed_and_was_handled_fails_again_in_the_replay_and_takes_the_error_edge() {
    let g = a_message_the_graph_handles();
    let first = first_run(&g).await;
    assert!(first.said("handled").is_some() && first.said("ok").is_none(), "run for real, the message failed and its error edge took it");
    let again = replayed(&g, &first, 1, false).await;
    assert!(again.said("handled").is_some(), "the replay follows the failure the run had");
    assert!(again.said("ok").is_none(), "and does not take the road of a block that worked");
}

#[tokio::test]
async fn blocks_that_start_together_follow_the_failure_they_had_too() {
    let g = DroneGraph {
        nodes: vec![
            start(),
            node("m1", json!({ "kind": "message", "to": "agenty", "body": "a" })),
            node("m2", json!({ "kind": "message", "to": "loap", "body": "b" })),
            response("ok1", "fine"),
            response("caught1", "caught"),
            response("ok2", "fine"),
            response("caught2", "caught"),
        ],
        edges: vec![
            edge("e1", "start", "m1"),
            edge("e2", "start", "m2"),
            edge("e3", "m1", "ok1"),
            error_edge("e4", "m1", "caught1"),
            edge("e5", "m2", "ok2"),
            error_edge("e6", "m2", "caught2"),
        ],
    };
    let first = first_run(&g).await;
    assert!(first.said("caught1").is_some() && first.said("caught2").is_some());
    let again = replayed(&g, &first, 1, false).await;
    assert!(again.said("caught1").is_some() && again.said("caught2").is_some());
    assert!(again.said("ok1").is_none() && again.said("ok2").is_none());
}

#[tokio::test]
async fn a_for_each_that_is_stubbed_is_as_tainted_as_it_was() {
    let g = DroneGraph {
        nodes: vec![
            start(),
            node("each", json!({ "kind": "foreach", "items": "{{trigger.list}}", "body": "tell", "on_item_error": "continue" })),
            node("tell", json!({ "kind": "message", "to": "agenty", "body": "{{item.name}}" })),
            response("out", "{{each.count}}"),
        ],
        edges: vec![edge("e1", "start", "each"), edge("e2", "each", "out")],
    };
    let (tx, rx) = mpsc::unbounded_channel();
    let states = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
    let sink = Arc::new(MemorySink::default());
    let limits = Limits { persist: Some(Persist { sink: sink.clone(), graph: graph_ref() }), lookup: saved(), ..Limits::default() };
    let trigger = json!({ "list": [{ "name": "a" }, { "name": "b" }] });
    let ended = execute("run-1", (&g, trigger, limits, "d"), &tx, &states).await;
    drop(tx);
    let first = drain(ended, rx, sink).await;
    let before: Checkpoint = load(&first.sink.rows_of("run-1")).unwrap().unwrap();
    assert!(before.bits["each"].0, "a list that came with the trigger is outside text, and so is the for-each over it");
    let again = replayed(&g, &first, 1, false).await;
    assert_eq!(children(&again), 0);
    let after: Checkpoint = load(&again.sink.rows_of("run-2")).unwrap().unwrap();
    assert_eq!(after.bits["each"], before.bits["each"], "the stub keeps what the for-each was");
    assert_eq!(after.bits["each"].0, true);
}
