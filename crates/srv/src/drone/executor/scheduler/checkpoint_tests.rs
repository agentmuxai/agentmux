// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A checkpoint is enough to take a run up again: from any of a run's
//! checkpoints, the rest of the run comes out the same as it did the first time.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::sync::mpsc;

use super::super::resume;
use super::super::{DroneGraph, FlowEdge, FlowNode, Limits, RunEnd, RunEvent};
use crate::backend::storage::drone_store::CheckpointRow;
use crate::drone::checkpoint::{Checkpoint, GraphRef};
use crate::drone::executor::run_drone;
use crate::drone::recorder::{load, MemorySink, Persist};
use crate::drone::types::NodePosition;

fn node(id: &str, data: Value) -> FlowNode {
    FlowNode { id: id.to_string(), position: NodePosition::default(), data, node_type: None }
}

fn edge(id: &str, source: &str, target: &str, handle: Option<&str>) -> FlowEdge {
    FlowEdge { id: id.to_string(), source: source.to_string(), target: target.to_string(), source_handle: handle.map(str::to_string), target_handle: None }
}

fn vars(id: &str, name: &str, value: &str) -> FlowNode {
    node(id, json!({ "kind": "variables", "entries": [{ "name": name, "value": value }] }))
}

fn response(id: &str, template: &str) -> FlowNode {
    node(id, json!({ "kind": "response", "template": template }))
}

/// start sets n; step appends an "x"; the guard asks whether n is `want`: true
/// goes on to `done`, false goes round to step.
fn counting_loop(want: &str, guard: Value) -> DroneGraph {
    let mut cond = json!({ "kind": "condition", "expr": format!("{{{{var.n}}}} == {want}") });
    cond.as_object_mut().unwrap().extend(guard.as_object().cloned().unwrap_or_default());
    DroneGraph {
        nodes: vec![vars("start", "n", "x"), vars("step", "n", "{{var.n}}x"), node("guard", cond), response("done", "{{var.n}}")],
        edges: vec![edge("e1", "start", "step", None), edge("e2", "step", "guard", None), edge("e3", "guard", "done", Some("true")), edge("e4", "guard", "step", Some("false"))],
    }
}

/// What started the run is outside text, so what is made of it is tainted.
fn from_the_trigger() -> DroneGraph {
    DroneGraph {
        nodes: vec![vars("start", "n", "{{trigger.msg}}"), vars("more", "m", "{{var.n}}!"), response("done", "{{var.m}}")],
        edges: vec![edge("e1", "start", "more", None), edge("e2", "more", "done", None)],
    }
}

fn with_handler() -> DroneGraph {
    let risky = node("risky", json!({ "kind": "variables", "entries": [{ "name": "x", "value": "1", "reducer": "bogus" }] }));
    DroneGraph {
        nodes: vec![vars("first", "a", "1"), risky, response("handler", "caught: {{risky.error}}"), response("fine", "all good")],
        edges: vec![edge("e0", "first", "risky", None), edge("e1", "risky", "fine", None), edge("e2", "risky", "handler", Some("error"))],
    }
}

fn graph_ref() -> GraphRef {
    GraphRef { version: 1, hash: "h".to_string() }
}

struct Kept {
    result: Result<Value, String>,
    done: Vec<(String, Value)>,
    steps: Vec<(u32, Option<String>)>,
    /// How many blocks had finished (`BlockDone`) when each checkpoint was kept.
    /// A block that failed has none, so this is not the step number.
    done_at: Vec<usize>,
    sink: Arc<MemorySink>,
    run_id: String,
}

impl Kept {
    fn rows(&self) -> Vec<CheckpointRow> {
        self.sink.rows_of(&self.run_id)
    }
}

type Drained = (Result<Value, String>, Vec<(String, Value)>, Vec<(u32, Option<String>)>, Vec<usize>);

async fn drain(mut events: mpsc::UnboundedReceiver<RunEvent>) -> Drained {
    let (mut done, mut steps, mut done_at, mut result) = (Vec::new(), Vec::new(), Vec::new(), Err("no end".to_string()));
    while let Some(ev) = events.recv().await {
        match ev {
            RunEvent::BlockDone { block_id, output, .. } => done.push((block_id, output)),
            RunEvent::StepCheckpointed { step, block_id, .. } => {
                steps.push((step, block_id));
                done_at.push(done.len());
            }
            RunEvent::RunDone { output, .. } => result = Ok(output),
            RunEvent::RunFailed { error, .. } => result = Err(error),
            _ => {}
        }
    }
    (result, done, steps, done_at)
}

async fn run_kept_on(sink: Arc<MemorySink>, graph: DroneGraph, trigger: Value) -> Kept {
    run_kept_capped(sink, graph, trigger, None).await
}

async fn run_kept_capped(sink: Arc<MemorySink>, graph: DroneGraph, trigger: Value, run_cap_usd: Option<f64>) -> Kept {
    let handle = run_drone("d".to_string(), graph, trigger, (run_cap_usd, Some(Persist { sink: sink.clone(), graph: graph_ref() }))).await.unwrap();
    let run_id = handle.run_id.clone();
    let (result, done, steps, done_at) = drain(handle.events).await;
    Kept { result, done, steps, done_at, sink, run_id }
}

async fn run_kept(graph: DroneGraph, trigger: Value) -> Kept {
    run_kept_on(Arc::new(MemorySink::default()), graph, trigger).await
}

/// Take the run up again from the first `n` rows (its newest checkpoint is
/// step `n - 1`), keeping on into a sink that holds just those rows, the way a
/// restart finds the store.
async fn resumed_from(kept: &Kept, graph: &DroneGraph, n: usize) -> Kept {
    let rows = kept.rows();
    let sink = Arc::new(MemorySink::default());
    for row in &rows[..n] {
        sink.rows.lock().unwrap().push((kept.run_id.clone(), row.clone()));
    }
    let c: Checkpoint = load(&rows[..n]).unwrap().unwrap();
    let (tx, rx) = mpsc::unbounded_channel();
    let states = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
    let limits = Limits { persist: Some(Persist { sink: sink.clone(), graph: c.graph.clone() }), ..Limits::default() };
    let ended = resume((graph, &c, limits), &tx, &states).await;
    drop(tx);
    let (_, done, steps, done_at) = drain(rx).await;
    let result = match ended {
        Ok(v) => Ok(v),
        Err(RunEnd::Failed(e) | RunEnd::Cancelled(e)) => Err(e),
    };
    Kept { result, done, steps, done_at, sink, run_id: kept.run_id.clone() }
}

/// A checkpoint without what only the clock decides.
fn timeless(mut c: Checkpoint) -> Checkpoint {
    c.elapsed_ms = 0;
    c.position.loop_elapsed_ms.clear();
    c
}

fn newest(kept: &Kept) -> Checkpoint {
    timeless(load(&kept.rows()).unwrap().unwrap())
}

#[tokio::test]
async fn a_run_keeps_a_checkpoint_before_its_first_block_and_one_after_each() {
    let kept = run_kept(counting_loop("xxx", json!({})), Value::Null).await;
    assert_eq!(kept.result, Ok(json!("xxx")));
    let ran: Vec<&str> = kept.done.iter().map(|(id, _)| id.as_str()).collect();
    assert_eq!(ran, ["start", "step", "guard", "step", "guard", "done"]);
    let expected: Vec<(u32, Option<String>)> = std::iter::once((0, None)).chain(ran.iter().enumerate().map(|(i, id)| (i as u32 + 1, Some(id.to_string())))).collect();
    assert_eq!(kept.steps, expected, "step 0 before any block, then one per block that ran");
    let rows = kept.rows();
    assert_eq!(rows.len(), expected.len());
    assert_eq!(rows[0].kind, "full");
    assert!(rows[1..].iter().all(|r| r.kind == "diff"));
    let last = load(&rows).unwrap().unwrap();
    assert_eq!(last.step as usize, rows.len() - 1);
    assert_eq!(last.drone_id, "d");
    assert_eq!(last.graph, graph_ref());
    assert_eq!(last.outputs["done"]["value"], kept.result.clone().unwrap());
}

#[tokio::test]
async fn from_any_checkpoint_the_rest_of_the_run_comes_out_the_same() {
    let graphs = [
        ("a loop that goes round three times", counting_loop("xxxx", json!({})), Value::Null),
        ("a value from the trigger", from_the_trigger(), json!({ "msg": "hi" })),
        ("a failure handled by an error edge", with_handler(), Value::Null),
    ];
    for (what, graph, trigger) in graphs {
        let kept = run_kept(graph.clone(), trigger).await;
        let total = kept.rows().len();
        assert!(total > 2, "{what}");
        for n in 1..=total {
            let again = resumed_from(&kept, &graph, n).await;
            assert_eq!(again.result, kept.result, "{what}, from checkpoint {}", n - 1);
            // The blocks that ran after the checkpoint are the ones that ran
            // after it the first time, with the same output, and no others.
            let first_time: Vec<_> = kept.done.iter().skip(kept.done_at[n - 1]).cloned().collect();
            assert_eq!(again.done, first_time, "{what}, from checkpoint {}", n - 1);
            // And what the run left is the same, bits and all.
            assert_eq!(newest(&again), newest(&kept), "{what}, from checkpoint {}", n - 1);
        }
    }
}

#[tokio::test]
async fn a_resumed_run_keeps_the_taint_of_what_was_read_before() {
    let kept = run_kept(from_the_trigger(), json!({ "msg": "hi" })).await;
    let c = newest(&kept);
    assert!(c.state.vars["n"].tainted && c.state.vars["m"].tainted, "{:?}", c.state.vars);
    // Resumed after `start`, `more` reads a tainted `n`, as it did the first time.
    let again = resumed_from(&kept, &from_the_trigger(), 2).await;
    let c2 = newest(&again);
    assert_eq!((c2.state.vars["m"].tainted, c2.state.vars["m"].derived), (c.state.vars["m"].tainted, c.state.vars["m"].derived));
    assert_eq!(c2.bits, c.bits);
}

#[tokio::test]
async fn a_loops_history_and_count_carry_over() {
    let graph = counting_loop("xxxx", json!({}));
    let kept = run_kept(graph.clone(), Value::Null).await;
    // After the guard's second pass (start, step, guard, step, guard): two rounds are behind it.
    let again = resumed_from(&kept, &graph, 5).await;
    assert_eq!(again.result, kept.result);
    let c = newest(&again);
    assert_eq!(c.loop_counters["guard"], 2);
    assert_eq!(c.outputs["guard"], newest(&kept).outputs["guard"]);
}

#[tokio::test]
async fn a_checkpoint_from_another_graph_is_refused() {
    let kept = run_kept(counting_loop("xxx", json!({})), Value::Null).await;
    let c = load(&kept.rows()[..3]).unwrap().unwrap();
    let other = DroneGraph { nodes: vec![response("elsewhere", "no")], edges: vec![] };
    let (tx, _rx) = mpsc::unbounded_channel();
    let states = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
    match resume((&other, &c, Limits::default()), &tx, &states).await {
        Err(RunEnd::Failed(why)) => assert!(why.contains("does not fit its graph"), "{why}"),
        _ => panic!("a checkpoint of another graph must not be taken up"),
    }
}

#[tokio::test]
async fn a_refused_write_does_not_stop_the_run_and_the_next_is_a_diff_from_the_last_kept() {
    let sink = Arc::new(MemorySink::default());
    // The first write goes, the second is refused.
    sink.refuse.lock().unwrap().extend([false, true]);
    let kept = run_kept_on(sink, from_the_trigger(), json!({ "msg": "hi" })).await;
    assert_eq!(kept.result, Ok(json!("hi!")));
    let rows = kept.rows();
    let last = load(&rows).unwrap().unwrap();
    assert_eq!(last.outputs["done"]["value"], json!("hi!"), "the newest kept checkpoint is still whole after a gap");
    assert!(kept.steps.len() < 4, "an event only for a checkpoint that was kept: {:?}", kept.steps);
}

#[tokio::test]
async fn a_run_that_uses_up_its_steps_ends_and_says_so() {
    // Three blocks a pass: more than a thousand steps in 400 passes.
    let kept = run_kept(counting_loop("never", json!({ "max_iterations": 500 })), Value::Null).await;
    let why = kept.result.clone().unwrap_err();
    assert!(why.contains("keeps at most 1000 steps"), "{why}");
    assert_eq!(kept.rows().len(), 1000, "steps 0 to 999 were kept");
}

#[tokio::test]
async fn a_run_with_nowhere_to_keep_checkpoints_runs_as_before() {
    let handle = run_drone("d".to_string(), from_the_trigger(), json!({ "msg": "hi" }), (None, None)).await.unwrap();
    let (result, _, steps, _) = drain(handle.events).await;
    assert_eq!(result, Ok(json!("hi!")));
    assert!(steps.is_empty());
}

/// `first` sets a variable, then `call` makes a request that cannot get anywhere
/// (a scheme the API block refuses), so the block fails the moment it runs.
fn calling(method: &str) -> DroneGraph {
    let call = node("call", json!({ "kind": "api", "method": method, "url": "ftp://nowhere.invalid/x" }));
    DroneGraph { nodes: vec![vars("first", "a", "1"), call], edges: vec![edge("e1", "first", "call", None)] }
}

fn calling_with_a_handler() -> DroneGraph {
    let mut g = calling("POST");
    g.nodes.push(response("handler", "caught: {{call.error}}"));
    g.edges.push(edge("e2", "call", "handler", Some("error")));
    g
}

fn step(n: u32, id: Option<&str>) -> (u32, Option<String>) {
    (n, id.map(str::to_string))
}

#[tokio::test]
async fn a_block_with_effects_is_marked_running_before_it_starts() {
    let kept = run_kept(calling("POST"), Value::Null).await;
    assert!(kept.result.is_err());
    // Step 0, the checkpoint after `first`, then the mark before `call`: it failed, so nothing follows.
    assert_eq!(kept.steps, vec![step(0, None), step(1, Some("first")), step(2, Some("call"))]);
    let last = load(&kept.rows()).unwrap().unwrap();
    assert_eq!(last.frontier.running, vec!["call".to_string()]);
    assert!(!last.outputs.contains_key("call"));
}

#[tokio::test]
async fn a_block_that_only_reads_is_not_marked() {
    let kept = run_kept(calling("GET"), Value::Null).await;
    assert_eq!(kept.steps, vec![step(0, None), step(1, Some("first"))]);
    assert!(load(&kept.rows()).unwrap().unwrap().frontier.running.is_empty());
}

#[tokio::test]
async fn a_failure_an_error_edge_handles_clears_the_mark() {
    let kept = run_kept(calling_with_a_handler(), Value::Null).await;
    // The mark before `call`, then the checkpoint after it (it failed, and its error edge carried on), then the handler.
    assert_eq!(kept.steps, vec![step(0, None), step(1, Some("first")), step(2, Some("call")), step(3, Some("call")), step(4, Some("handler"))]);
    let last = load(&kept.rows()).unwrap().unwrap();
    assert!(last.frontier.running.is_empty(), "the block is over, however it ended");
    assert_eq!(last.outputs["call"]["failed"], json!(true));
}

#[tokio::test]
async fn a_restart_waits_before_running_a_marked_block_again() {
    let graph = calling("POST");
    let kept = run_kept(graph.clone(), Value::Null).await;
    let mut parked = park(&kept, &graph, 3);
    let (block, why) = attention(&mut parked).await;
    assert_eq!(block, "call");
    assert!(why.contains("block call was running when the run stopped") && why.contains("may already have done its work"), "{why}");
    assert!(!parked.task.is_finished(), "the run waits instead of running it");
    parked.cancel.cancel("test over");
    assert!(parked.task.await.unwrap().unwrap_err().starts_with("cancelled: "));

    // From before the mark the block has not started, so it is simply run.
    let before = resumed_from(&kept, &graph, 2).await;
    let why = before.result.unwrap_err();
    assert!(why.contains("block call failed"), "{why}");
}

#[tokio::test]
async fn a_mark_that_cannot_be_kept_stops_the_run_before_the_block_starts() {
    let sink = Arc::new(MemorySink::default());
    // Step 0 and the checkpoint after `first` are kept; the mark is refused.
    sink.refuse.lock().unwrap().extend([false, false, true]);
    let kept = run_kept_on(sink, calling("POST"), Value::Null).await;
    let why = kept.result.unwrap_err();
    assert!(why.contains("could not keep its place before call") && !why.contains("block call failed"), "{why}");
}

use crate::backend::drone_decision::{decide, is_waiting, Decision};
use crate::drone::executor::engine::Cancel;

/// `first`, then `call` (a POST that cannot get anywhere), then a response that reads what `call` said.
fn calling_then_answering() -> DroneGraph {
    let mut g = calling("POST");
    g.nodes.push(response("done", "{{call.value}}"));
    g.edges.push(edge("e2", "call", "done", None));
    g
}

/// A resumed run in the background, as the server runs it: its events, its way
/// to be stopped, and where it keeps checkpoints.
struct Parked {
    task: tokio::task::JoinHandle<Result<Value, String>>,
    events: mpsc::UnboundedReceiver<RunEvent>,
    cancel: Cancel,
    sink: Arc<MemorySink>,
    run_id: String,
}

fn park(kept: &Kept, graph: &DroneGraph, n: usize) -> Parked {
    let rows = kept.rows();
    let sink = Arc::new(MemorySink::default());
    for row in &rows[..n] {
        sink.rows.lock().unwrap().push((kept.run_id.clone(), row.clone()));
    }
    let c = load(&rows[..n]).unwrap().unwrap();
    let (tx, events) = mpsc::unbounded_channel();
    let cancel = Cancel::default();
    let limits = Limits { persist: Some(Persist { sink: sink.clone(), graph: c.graph.clone() }), cancel: cancel.clone(), ..Limits::default() };
    let graph = graph.clone();
    let task = tokio::spawn(async move {
        let states = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
        match resume((&graph, &c, limits), &tx, &states).await {
            Ok(v) => Ok(v),
            Err(RunEnd::Failed(e)) => Err(e),
            Err(RunEnd::Cancelled(e)) => Err(format!("cancelled: {e}")),
        }
    });
    Parked { task, events, cancel, sink, run_id: kept.run_id.clone() }
}

/// The block a parked run needs attention for, and why.
async fn attention(p: &mut Parked) -> (String, String) {
    while let Some(ev) = p.events.recv().await {
        if let RunEvent::NeedsAttention { block_id, reason, .. } = ev {
            return (block_id, reason);
        }
    }
    panic!("the run ended without asking");
}

/// What a parked run sends after it was answered, until it ends.
async fn the_rest(events: &mut mpsc::UnboundedReceiver<RunEvent>) -> Vec<RunEvent> {
    let mut out = Vec::new();
    while let Some(ev) = events.recv().await {
        out.push(ev);
    }
    out
}

#[tokio::test]
async fn a_run_told_to_skip_the_step_goes_on_with_the_output_it_was_given_as_outside_text() {
    let graph = calling_then_answering();
    let kept = run_kept(graph.clone(), Value::Null).await;
    let mut p = park(&kept, &graph, 3);
    attention(&mut p).await;
    assert!(is_waiting(&p.run_id));
    decide(&p.run_id, Decision::Skip { output: json!({ "value": "given" }) }).unwrap();
    assert_eq!(p.task.await.unwrap(), Ok(json!("given")));
    let events = the_rest(&mut p.events).await;
    assert!(events.iter().any(|e| matches!(e, RunEvent::AttentionAnswered { choice, .. } if choice == "skip")));
    assert!(events.iter().any(|e| matches!(e, RunEvent::BlockDone { block_id, .. } if block_id == "call")), "the skipped step shows as done");
    assert!(!events.iter().any(|e| matches!(e, RunEvent::BlockStarted { block_id, .. } if block_id == "call")), "it was not run");
    let last = load(&p.sink.rows_of(&p.run_id)).unwrap().unwrap();
    assert_eq!(last.bits["call"], (true, false), "what was supplied is outside text");
    assert_eq!(last.bits["done"], (true, false), "and so is what is made of it");
    assert!(last.frontier.running.is_empty());
}

#[tokio::test]
async fn the_cost_a_skip_supplies_counts_against_the_runs_cap() {
    let graph = calling_then_answering();
    // The cap is the run's own: it is kept in the checkpoint, so a restart finds it.
    let kept = run_kept_capped(Arc::new(MemorySink::default()), graph.clone(), Value::Null, Some(5.0)).await;
    let mut p = park(&kept, &graph, 3);
    attention(&mut p).await;
    decide(&p.run_id, Decision::Skip { output: json!({ "value": "given", "cost_usd": 5.0 }) }).unwrap();
    let why = p.task.await.unwrap().unwrap_err();
    assert!(why.contains("the run spent $5.00, its cap of $5.00"), "{why}");
    let events = the_rest(&mut p.events).await;
    assert!(!events.iter().any(|e| matches!(e, RunEvent::BlockStarted { block_id, .. } if block_id == "done")), "the next block did not start");
}

#[tokio::test]
async fn a_run_told_to_run_the_step_again_marks_it_and_runs_it() {
    let graph = calling_then_answering();
    let kept = run_kept(graph.clone(), Value::Null).await;
    let mut p = park(&kept, &graph, 3);
    attention(&mut p).await;
    decide(&p.run_id, Decision::RunAgain).unwrap();
    let why = p.task.await.unwrap().unwrap_err();
    assert!(why.contains("block call failed"), "it ran again, and failed as it did the first time: {why}");
    let events = the_rest(&mut p.events).await;
    assert!(events.iter().any(|e| matches!(e, RunEvent::AttentionAnswered { choice, .. } if choice == "run_again")));
    assert!(events.iter().any(|e| matches!(e, RunEvent::BlockStarted { block_id, .. } if block_id == "call")));
    let rows = p.sink.rows_of(&p.run_id);
    assert_eq!(rows.len(), 4, "the mark is kept again before it runs: {rows:?}");
}

#[tokio::test]
async fn a_run_that_is_cancelled_while_it_waits_ends_cancelled_and_stops_waiting() {
    let graph = calling_then_answering();
    let kept = run_kept(graph.clone(), Value::Null).await;
    let mut p = park(&kept, &graph, 3);
    attention(&mut p).await;
    assert!(p.cancel.cancel("its job's timeout"));
    let why = p.task.await.unwrap().unwrap_err();
    assert_eq!(why, "cancelled: its job's timeout");
    assert!(!is_waiting(&p.run_id));
    assert!(decide(&p.run_id, Decision::RunAgain).is_err(), "nothing is left to answer");
    let events = the_rest(&mut p.events).await;
    assert!(!events.iter().any(|e| matches!(e, RunEvent::BlockStarted { .. })), "the step was never run");
}
