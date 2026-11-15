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

// ── Wait ────────────────────────────────────────────────────────────────────

use crate::backend::drone_wait;

/// `first`, then a Wait block set as `wait` says, then a response that reads what it came to.
fn waiting(wait: Value, template: &str) -> DroneGraph {
    let mut data = json!({ "kind": "wait" });
    data.as_object_mut().unwrap().extend(wait.as_object().cloned().unwrap_or_default());
    DroneGraph {
        nodes: vec![vars("first", "a", "1"), node("wait", data), response("done", template)],
        edges: vec![edge("e1", "first", "wait", None), edge("e2", "wait", "done", None)],
    }
}

/// How many of a run's checkpoints it had kept when it stopped at its wait.
fn waiting_at(kept: &Kept) -> usize {
    let rows = kept.rows();
    (1..=rows.len()).find(|n| load(&rows[..*n]).unwrap().unwrap().frontier.waiting == ["wait"]).expect("a checkpoint says the run is waiting")
}

#[tokio::test]
async fn a_run_that_waits_for_a_time_keeps_where_it_waits_and_goes_on_when_the_time_comes() {
    let graph = waiting(json!({ "for": "300ms" }), "late={{wait.late}}");
    let started = std::time::Instant::now();
    let kept = run_kept(graph.clone(), Value::Null).await;
    assert_eq!(kept.result, Ok(json!("late=false")));
    assert!(started.elapsed() >= std::time::Duration::from_millis(300), "it waited");

    let n = waiting_at(&kept);
    let at_wait = load(&kept.rows()[..n]).unwrap().unwrap();
    assert!(at_wait.deadlines["wait"] > 0 && at_wait.resume_on.is_empty());
    assert!(at_wait.frontier.running.is_empty(), "a wait is not a step with effects");
    let last = load(&kept.rows()).unwrap().unwrap();
    assert!(last.deadlines.is_empty() && last.frontier.waiting.is_empty(), "nothing is waited for once it is over");
    assert_eq!(last.bits["wait"], (false, false), "a time brings nothing from outside");
}

#[tokio::test]
async fn a_restart_takes_a_wait_up_where_it_was_and_does_not_start_it_over() {
    let graph = waiting(json!({ "for": "700ms" }), "late={{wait.late}}");
    let sink = Arc::new(MemorySink::default());
    let mut handle = run_drone("d".to_string(), graph.clone(), Value::Null, (None, Some(Persist { sink: sink.clone(), graph: graph_ref() }))).await.unwrap();
    while let Some(ev) = handle.events.recv().await {
        if matches!(ev, RunEvent::Waiting { .. }) {
            break;
        }
    }
    // The first process is gone; what it kept is what the restart finds.
    assert!(crate::backend::drone_cancel::cancel_run(&handle.run_id, "test over"));
    while handle.events.recv().await.is_some() {}
    let c = load(&sink.rows_of(&handle.run_id)).unwrap().unwrap();
    let deadline = c.deadlines["wait"];

    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    let (tx, mut rx) = mpsc::unbounded_channel();
    let states = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
    let ended = resume((&graph, &c, Limits::default()), &tx, &states).await;
    let finished = agentmux_common::time::now_ms();
    assert_eq!(ended.ok(), Some(json!("late=false")));
    drop(tx);
    while rx.recv().await.is_some() {}
    // It waited out what was left of the deadline it kept: not the whole 700 ms again, and not less.
    assert!(finished >= deadline, "it did not go on before its time");
    assert!(finished - deadline < 150, "it did not start the wait over: {} ms after its deadline", finished - deadline);
}

#[tokio::test]
async fn a_wait_whose_time_passed_while_agentmux_was_stopped_ends_at_once_and_says_it_was_late() {
    let graph = waiting(json!({ "for": "1h" }), "late={{wait.late}}");
    // The run is at its wait: what it has kept is what a restart would find.
    let sink = Arc::new(MemorySink::default());
    let handle = run_drone("d".to_string(), graph.clone(), Value::Null, (None, Some(Persist { sink: sink.clone(), graph: graph_ref() }))).await.unwrap();
    let (run_id, mut events) = (handle.run_id.clone(), handle.events);
    while let Some(ev) = events.recv().await {
        if matches!(ev, RunEvent::Waiting { .. }) {
            break;
        }
    }
    let mut c = load(&sink.rows_of(&run_id)).unwrap().unwrap();
    assert_eq!(c.frontier.waiting, ["wait"]);
    c.deadlines.insert("wait".to_string(), agentmux_common::time::now_ms() - 5_000);

    let (tx, mut rx) = mpsc::unbounded_channel();
    let states = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
    let ended = resume((&graph, &c, Limits::default()), &tx, &states).await;
    assert_eq!(ended.ok(), Some(json!("late=true")));
    drop(tx);
    let mut came = None;
    while let Some(ev) = rx.recv().await {
        if let RunEvent::BlockDone { block_id, output, .. } = ev {
            if block_id == "wait" {
                came = Some(output);
            }
        }
    }
    let came = came.unwrap();
    assert!(came["late_ms"].as_i64().unwrap() >= 5_000, "{came}");
}

#[tokio::test]
async fn a_wait_on_an_event_goes_on_when_its_key_is_used_and_what_came_is_outside_text() {
    let graph = waiting(json!({ "on": "deploy.finished" }), "n={{wait.payload.n}}");
    let sink = Arc::new(MemorySink::default());
    let mut handle = run_drone("d".to_string(), graph.clone(), Value::Null, (None, Some(Persist { sink: sink.clone(), graph: graph_ref() }))).await.unwrap();
    let run_id = handle.run_id.clone();
    let mut key = String::new();
    let mut ended = Vec::new();
    while let Some(ev) = handle.events.recv().await {
        match ev {
            RunEvent::Waiting { on, key: Some(k), resume_at: None, .. } => {
                assert_eq!(on.as_deref(), Some("deploy.finished"));
                assert_eq!(k.len(), 64);
                assert!(drone_wait::signal("not-the-key", json!(1)).is_err(), "a guess does nothing");
                drone_wait::signal(&k, json!({ "n": 7 })).unwrap();
                key = k;
            }
            RunEvent::WaitEnded { late, .. } => ended.push(late),
            RunEvent::RunDone { output, .. } => assert_eq!(output, json!("n=7")),
            RunEvent::RunFailed { error, .. } => panic!("{error}"),
            _ => {}
        }
    }
    assert!(!key.is_empty() && ended == [false]);
    assert!(drone_wait::signal(&key, json!(1)).is_err(), "a key answers once");
    let rows = sink.rows_of(&run_id);
    let kept_key = (1..=rows.len()).find_map(|n| load(&rows[..n]).unwrap().unwrap().resume_on.get("wait").cloned());
    assert_eq!(kept_key.as_deref(), Some(key.as_str()), "the key was kept before the run waited");
    assert_eq!(load(&rows).unwrap().unwrap().bits["wait"], (true, false), "what the event brought is outside text");
}

#[tokio::test]
async fn a_restart_takes_up_a_wait_on_an_event_with_the_same_key() {
    let graph = waiting(json!({ "on": "deploy.finished" }), "n={{wait.payload.n}}");
    let sink = Arc::new(MemorySink::default());
    let mut handle = run_drone("d".to_string(), graph.clone(), Value::Null, (None, Some(Persist { sink: sink.clone(), graph: graph_ref() }))).await.unwrap();
    let mut key = String::new();
    while let Some(ev) = handle.events.recv().await {
        if let RunEvent::Waiting { key: Some(k), .. } = ev {
            key = k;
            break;
        }
    }
    // The first process is gone: its wait goes with it.
    assert!(crate::backend::drone_cancel::cancel_run(&handle.run_id, "test over"));
    while handle.events.recv().await.is_some() {}
    assert!(drone_wait::signal(&key, json!(1)).is_err(), "a stopped run no longer waits");

    let c = load(&sink.rows_of(&handle.run_id)).unwrap().unwrap();
    assert_eq!(c.resume_on["wait"], key);
    let (tx, mut events) = mpsc::unbounded_channel();
    let states = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
    let (graph2, c2) = (graph.clone(), c.clone());
    let task = tokio::spawn(async move { resume((&graph2, &c2, Limits::default()), &tx, &states).await.ok() });
    let again = loop {
        if let RunEvent::Waiting { key: Some(k), .. } = events.recv().await.expect("the run waits again") {
            break k;
        }
    };
    assert_eq!(again, key, "the same wait, so the same key");
    drone_wait::signal(&key, json!({ "n": 3 })).unwrap();
    assert_eq!(task.await.unwrap(), Some(json!("n=3")));
}

#[tokio::test]
async fn a_run_that_is_stopped_while_it_waits_ends_and_its_key_stops_working() {
    let graph = waiting(json!({ "on": "never" }), "unreached");
    let mut handle = run_drone("d".to_string(), graph, Value::Null, (None, None)).await.unwrap();
    let mut key = String::new();
    while let Some(ev) = handle.events.recv().await {
        if let RunEvent::Waiting { key: Some(k), .. } = ev {
            key = k;
            break;
        }
    }
    assert!(crate::backend::drone_cancel::cancel_run(&handle.run_id, "asked to stop"));
    let mut cancelled = false;
    while let Some(ev) = handle.events.recv().await {
        cancelled |= matches!(ev, RunEvent::RunCancelled { .. });
    }
    assert!(cancelled);
    assert!(drone_wait::signal(&key, json!(1)).is_err());
}

#[tokio::test]
async fn a_wait_set_wrongly_is_refused_before_the_run_starts_and_a_limit_ends_an_event_wait() {
    let none = run_kept(waiting(json!({}), "x"), Value::Null).await;
    assert!(none.result.unwrap_err().contains("needs `for`, `until` or `on`"));
    let both = run_kept(waiting(json!({ "for": "1m", "on": "x" }), "x"), Value::Null).await;
    assert!(both.result.unwrap_err().contains("only one of"));

    let limited = run_kept(waiting(json!({ "on": "never", "timeout_seconds": 1 }), "x"), Value::Null).await;
    assert!(limited.result.unwrap_err().contains("block wait failed: timed out after 1 seconds"));
}

/// The run at its wait, stopped there: what it has kept is what a restart would find.
async fn stopped_at_its_wait(graph: &DroneGraph) -> Checkpoint {
    let sink = Arc::new(MemorySink::default());
    let mut handle = run_drone("d".to_string(), graph.clone(), Value::Null, (None, Some(Persist { sink: sink.clone(), graph: graph_ref() }))).await.unwrap();
    while let Some(ev) = handle.events.recv().await {
        if matches!(ev, RunEvent::Waiting { .. }) {
            break;
        }
    }
    assert!(crate::backend::drone_cancel::cancel_run(&handle.run_id, "test over"));
    while handle.events.recv().await.is_some() {}
    load(&sink.rows_of(&handle.run_id)).unwrap().unwrap()
}

/// What a resumed run came to: its result and how long it took.
async fn taken_up(graph: &DroneGraph, c: &Checkpoint) -> (Result<Value, String>, std::time::Duration) {
    let started = std::time::Instant::now();
    let (tx, _rx) = mpsc::unbounded_channel();
    let states = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
    let ended = resume((graph, c, Limits::default()), &tx, &states).await;
    let result = match ended {
        Ok(v) => Ok(v),
        Err(RunEnd::Failed(e) | RunEnd::Cancelled(e)) => Err(e),
    };
    (result, started.elapsed())
}

#[tokio::test]
async fn a_waits_own_time_out_is_kept_and_a_restart_does_not_give_it_its_whole_time_again() {
    let graph = waiting(json!({ "for": "1h", "timeout_seconds": 60 }), "late={{wait.late}}");
    let c = stopped_at_its_wait(&graph).await;
    let now = agentmux_common::time::now_ms();
    assert!((c.wait_timeouts["wait"] - now - 60_000).abs() < 5_000, "the limit was kept with the wait");

    // AgentMux was stopped for two hours: both the time and the limit are past, and the limit came first.
    let mut after = c.clone();
    after.deadlines.insert("wait".to_string(), now - 3_600_000);
    after.wait_timeouts.insert("wait".to_string(), now - 7_200_000 + 60_000);
    let (result, took) = taken_up(&graph, &after).await;
    assert!(result.unwrap_err().contains("block wait failed: timed out after 1 minutes"));
    assert!(took < std::time::Duration::from_secs(2), "it did not wait again: {took:?}");
}

#[tokio::test]
async fn an_event_wait_whose_time_out_passed_while_stopped_fails_at_once_and_one_with_time_left_gets_only_that() {
    let graph = waiting(json!({ "on": "never", "timeout_seconds": 600 }), "unreached");
    let c = stopped_at_its_wait(&graph).await;
    let now = agentmux_common::time::now_ms();

    let mut past = c.clone();
    past.wait_timeouts.insert("wait".to_string(), now - 1_000);
    let (result, took) = taken_up(&graph, &past).await;
    assert!(result.unwrap_err().contains("timed out after 10 minutes"));
    assert!(took < std::time::Duration::from_secs(2), "{took:?}");

    let mut left = c.clone();
    left.wait_timeouts.insert("wait".to_string(), now + 400);
    let (result, took) = taken_up(&graph, &left).await;
    assert!(result.unwrap_err().contains("timed out after 10 minutes"));
    assert!(took >= std::time::Duration::from_millis(300) && took < std::time::Duration::from_secs(3), "only what was left: {took:?}");
}

#[tokio::test]
async fn a_wait_that_ends_in_time_leaves_no_limit_behind() {
    let graph = waiting(json!({ "for": "100ms", "timeout_seconds": 60 }), "late={{wait.late}}");
    let kept = run_kept(graph, Value::Null).await;
    assert_eq!(kept.result, Ok(json!("late=false")));
    assert!(load(&kept.rows()).unwrap().unwrap().wait_timeouts.is_empty());
}

#[tokio::test]
async fn an_event_wait_past_its_limit_does_not_open_its_key_again_so_a_resend_cannot_end_it() {
    let graph = waiting(json!({ "on": "never", "timeout_seconds": 600 }), "unreached");
    let c = stopped_at_its_wait(&graph).await;
    let key = c.resume_on["wait"].clone();
    let mut past = c.clone();
    past.wait_timeouts.insert("wait".to_string(), agentmux_common::time::now_ms() - 1_000);

    let (tx, mut events) = mpsc::unbounded_channel();
    let states = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
    let g = graph.clone();
    let task = tokio::spawn(async move { resume((&g, &past, Limits::default()), &tx, &states).await.map_err(|e| match e { RunEnd::Failed(e) | RunEnd::Cancelled(e) => e }) });
    // The source sends its event again at once: there is nothing to receive it, and no Waiting was announced.
    let _ = drone_wait::signal(&key, json!({ "late": true }));
    let why = task.await.unwrap().unwrap_err();
    assert!(why.contains("timed out after 10 minutes"), "{why}");
    while let Some(ev) = events.recv().await {
        assert!(!matches!(ev, RunEvent::Waiting { .. }), "a wait that is over is not announced");
    }
}

/// `first` fans out to two POSTs that cannot get anywhere: both have effects and are ready together.
fn two_calls() -> DroneGraph {
    let call = |id: &str| node(id, json!({ "kind": "api", "method": "POST", "url": "ftp://nowhere.invalid/x" }));
    DroneGraph {
        nodes: vec![vars("first", "a", "1"), call("p1"), call("p2"), response("done", "{{p1.value}} {{p2.value}}")],
        edges: vec![edge("e1", "first", "p1", None), edge("e2", "first", "p2", None), edge("e3", "p1", "done", None), edge("e4", "p2", "done", None)],
    }
}

#[tokio::test]
async fn blocks_with_effects_that_start_together_are_marked_in_one_checkpoint() {
    let kept = run_kept(two_calls(), Value::Null).await;
    assert!(kept.result.is_err());
    // Step 0, the checkpoint after `first`, then one mark for both, kept before either starts.
    assert_eq!(kept.steps[..3], [step(0, None), step(1, Some("first")), step(2, Some("p1"))]);
    let marked = load(&kept.rows()[..3]).unwrap().unwrap();
    assert_eq!(marked.frontier.running, vec!["p1".to_string(), "p2".to_string()]);
    assert!(marked.frontier.ready.is_empty());
}

#[tokio::test]
async fn a_restart_asks_about_each_block_that_was_running_in_turn() {
    let graph = two_calls();
    let kept = run_kept(graph.clone(), Value::Null).await;
    let mut p = park(&kept, &graph, 3);
    let (first, _) = attention(&mut p).await;
    assert_eq!(first, "p1");
    decide(&p.run_id, Decision::Skip { output: json!({ "value": "one" }) }).unwrap();
    let (second, _) = attention(&mut p).await;
    assert_eq!(second, "p2", "it asks about the next one, and still has it marked until it is answered");
    decide(&p.run_id, Decision::Skip { output: json!({ "value": "two" }) }).unwrap();
    assert_eq!(p.task.await.unwrap(), Ok(json!("one two")));
    let last = load(&p.sink.rows_of(&p.run_id)).unwrap().unwrap();
    assert!(last.frontier.running.is_empty());
}
