// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use serde_json::{json, Value};

use super::*;
use crate::drone::executor::run_drone;
use crate::drone::types::NodePosition;

fn node(id: &str, data: Value) -> FlowNode {
    FlowNode { id: id.to_string(), position: NodePosition::default(), data, node_type: None }
}

fn edge(id: &str, source: &str, target: &str, handle: Option<&str>) -> FlowEdge {
    FlowEdge {
        id: id.to_string(),
        source: source.to_string(),
        target: target.to_string(),
        source_handle: handle.map(str::to_string),
        target_handle: None,
    }
}

fn vars(id: &str, name: &str, value: &str) -> FlowNode {
    node(id, json!({ "kind": "variables", "entries": [{ "name": name, "value": value }] }))
}

fn response(id: &str, template: &str) -> FlowNode {
    node(id, json!({ "kind": "response", "template": template }))
}

/// start sets n to "x"; step adds an "x"; the guard asks whether n is `want`.
/// True goes on to `done`, false goes round to step. `guard` adds to the
/// guard's own settings.
fn counting_loop(want: &str, guard: Value, done_template: &str) -> DroneGraph {
    let mut cond = json!({ "kind": "condition", "expr": format!("{{{{var.n}}}} == {want}") });
    cond.as_object_mut().unwrap().extend(guard.as_object().cloned().unwrap_or_default());
    DroneGraph {
        nodes: vec![vars("start", "n", "x"), vars("step", "n", "{{var.n}}x"), node("guard", cond), response("done", done_template)],
        edges: vec![
            edge("e1", "start", "step", None),
            edge("e2", "step", "guard", None),
            edge("e3", "guard", "done", Some("true")),
            edge("e4", "guard", "step", Some("false")),
        ],
    }
}

struct Outcome {
    result: Result<Value, String>,
    /// Each block's finished outputs, in the order they came.
    done: Vec<(String, Value)>,
    states: HashMap<String, BlockState>,
}

impl Outcome {
    fn times(&self, id: &str) -> usize {
        self.done.iter().filter(|(b, _)| b == id).count()
    }
}

async fn run(graph: DroneGraph) -> Outcome {
    let mut handle = run_drone("d".to_string(), graph, Value::Null, None).await.unwrap();
    let mut done = Vec::new();
    let mut result = Err("no end".to_string());
    while let Some(ev) = handle.events.recv().await {
        match ev {
            RunEvent::BlockDone { block_id, output, .. } => done.push((block_id, output)),
            RunEvent::RunDone { output, .. } => result = Ok(output),
            RunEvent::RunFailed { error, .. } => result = Err(error),
            _ => {}
        }
    }
    let states = handle.final_states.lock().await.clone();
    Outcome { result, done, states }
}

#[tokio::test]
async fn a_guarded_loop_goes_round_until_its_guard_lets_the_run_out() {
    let out = run(counting_loop("xxxx", json!({}), "{{var.n}}")).await;
    assert_eq!(out.result, Ok(json!("xxxx")));
    assert_eq!(out.times("start"), 1);
    assert_eq!(out.times("step"), 3, "x, then three more to reach xxxx");
    assert_eq!(out.times("guard"), 3);
    assert_eq!(out.times("done"), 1, "the block after the loop runs once, when the loop is over");
    assert_eq!(out.states["done"].status, "done");
}

#[tokio::test]
async fn a_loop_that_hits_its_limit_ends_the_run_and_says_why() {
    let out = run(counting_loop("never", json!({ "max_iterations": 3 }), "unreached")).await;
    let why = out.result.clone().unwrap_err();
    assert!(why.contains("stopped after 3 of at most 3 iterations") && why.contains("guard"), "{why}");
    assert_eq!(out.times("step"), 3, "three passes, no more");
    assert_eq!(out.times("done"), 0);
}

#[tokio::test]
async fn a_loop_may_be_left_at_its_limit_and_the_run_goes_on() {
    let g = counting_loop("never", json!({ "max_iterations": 3, "on_cap": "exit_loop" }), "capped={{guard.capped}} n={{var.n}}");
    let out = run(g).await;
    assert_eq!(out.result, Ok(json!("capped=true n=xxxx")));
    assert_eq!(out.times("step"), 3, "three passes, then out");
}

#[tokio::test]
async fn a_block_that_runs_again_can_read_its_earlier_outputs() {
    let out = run(counting_loop("xxxx", json!({}), "{{var.n}}")).await;
    let steps: Vec<&Value> = out.done.iter().filter(|(b, _)| b == "step").map(|(_, o)| o).collect();
    assert!(steps[0].get("history").is_none(), "the first run has none");
    assert_eq!(steps[1]["history"].as_array().unwrap().len(), 1);
    let last = steps[2]["history"].as_array().unwrap();
    assert_eq!(last.len(), 2);
    assert_eq!(last[1]["vars"]["n"], "xxx", "newest last");
    assert!(last[1].get("history").is_none(), "a history does not nest");
}

#[tokio::test]
async fn a_loop_with_no_condition_in_it_fails_before_any_block_runs() {
    let g = DroneGraph {
        nodes: vec![vars("start", "n", "x"), vars("a", "n", "{{var.n}}x"), vars("b", "n", "{{var.n}}y")],
        edges: vec![edge("e1", "start", "a", None), edge("e2", "a", "b", None), edge("e3", "b", "a", None)],
    };
    let out = run(g).await;
    assert!(out.result.unwrap_err().contains("no Condition"));
    assert!(out.done.is_empty(), "nothing ran");
}

#[tokio::test]
async fn a_block_after_two_branches_runs_once_when_either_is_taken() {
    // c (true) → t → d; c (false) → f → d. Only t runs, so d runs after it.
    let g = DroneGraph {
        nodes: vec![
            node("c", json!({ "kind": "condition", "expr": "1 < 2" })),
            vars("t", "v", "t"),
            vars("f", "v", "f"),
            response("d", "{{var.v}}"),
        ],
        edges: vec![
            edge("e1", "c", "t", Some("true")),
            edge("e2", "c", "f", Some("false")),
            edge("e3", "t", "d", None),
            edge("e4", "f", "d", None),
        ],
    };
    let out = run(g).await;
    assert_eq!(out.result, Ok(json!("t")));
    assert_eq!(out.states["f"].status, "skipped");
    assert_eq!(out.times("d"), 1);
}

#[tokio::test]
async fn a_block_outside_a_loop_runs_once_however_many_times_the_loop_goes_round() {
    // The guard's false branch also leads to `side`, which is not part of the loop.
    let mut g = counting_loop("xxxx", json!({}), "{{var.n}}");
    g.nodes.push(response("side", "side"));
    g.edges.push(edge("e5", "guard", "side", Some("false")));
    let out = run(g).await;
    assert_eq!(out.result, Ok(json!("xxxx")));
    assert_eq!(out.times("step"), 3);
    assert_eq!(out.times("side"), 1);
}

#[test]
fn a_loops_cost_and_time_limits_end_it_with_their_reason() {
    let (tx, _rx) = mpsc::unbounded_channel();
    let states: States = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
    let graph = counting_loop("never", json!({ "max_cost_usd": 1.0, "max_minutes": 5 }), "unreached");
    let plan = schedule::analyze(&graph).unwrap();
    let guard = plan.guards["guard"].clone();
    let mut s = Scheduler::new("r", &graph, plan, ExecutionScope::new(), None, &tx, &states);
    assert_eq!(s.loop_cap("guard", &guard), None, "nothing spent, nothing elapsed");
    s.spent_by.insert("step", 0.6);
    s.spent_by.insert("guard", 0.5);
    s.spent_by.insert("done", 9.0); // outside the loop: not the loop's spend
    let why = s.loop_cap("guard", &guard).unwrap();
    assert!(why.contains("the loop at guard spent $1.10, its cap of $1.00"), "{why}");
    s.spent_by.clear();
    s.loop_started.insert("guard".to_string(), Instant::now() - std::time::Duration::from_secs(6 * 60));
    let why = s.loop_cap("guard", &guard).unwrap();
    assert!(why.contains("ran 6.0 minutes, its cap of 5 minutes"), "{why}");
}

#[tokio::test]
async fn a_loops_clock_starts_with_the_first_block_of_its_cycle() {
    let (tx, _rx) = mpsc::unbounded_channel();
    let states: States = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
    let graph = counting_loop("never", json!({ "max_minutes": 5 }), "unreached");
    let plan = schedule::analyze(&graph).unwrap();
    let mut s = Scheduler::new("r", &graph, plan, ExecutionScope::new(), None, &tx, &states);
    s.enqueue("start", false);
    s.step("start", false).await.unwrap();
    assert!(s.loop_started.is_empty(), "the start block is not in the cycle");
    s.step("step", false).await.unwrap();
    assert!(s.loop_started.contains_key("guard"), "the first pass counts toward the loop's time, before the guard has gone round");
}

#[tokio::test]
async fn a_block_with_a_forward_edge_and_a_back_edge_keeps_its_forward_edge() {
    // b leads on to c inside the cycle and also back to the guard a; c leads back too.
    let g = DroneGraph {
        nodes: vec![
            vars("start", "n", "x"),
            node("a", json!({ "kind": "condition", "expr": "{{var.n}} == xxxx" })),
            vars("b", "n", "{{var.n}}x"),
            vars("c", "m", "y"),
            response("done", "{{var.n}}"),
        ],
        edges: vec![
            edge("e1", "start", "a", None),
            edge("e2", "a", "b", Some("false")),
            edge("e3", "a", "done", Some("true")),
            edge("e4", "b", "c", None),
            edge("e5", "b", "a", None),
            edge("e6", "c", "a", None),
        ],
    };
    // Two ways back into one guard is not a loop with a clear count, so only
    // the edge's own fate is asserted: c is not skipped by the trip back.
    let out = run(g).await;
    assert!(out.times("c") >= 1, "c ran: its edge from b was not erased by the trip back");
}

#[tokio::test]
async fn a_loop_collects_into_a_list_and_adds_up_a_number_with_reducers() {
    let mut g = counting_loop("xxxx", json!({}), "{{var.log}} total={{var.total}}");
    g.nodes[1] = node(
        "step",
        json!({ "kind": "variables", "entries": [
            { "name": "n", "value": "{{var.n}}x" },
            { "name": "log", "value": "{{var.n}}", "reducer": "append" },
            { "name": "total", "value": 10, "reducer": "add" },
        ]}),
    );
    let out = run(g).await;
    assert_eq!(out.result, Ok(json!(r#"["xx","xxx","xxxx"] total=30"#)));
}

#[tokio::test]
async fn the_run_marks_what_came_from_outside_as_tainted_for_the_state() {
    let (tx, _rx) = mpsc::unbounded_channel();
    let states: States = Arc::new(tokio::sync::Mutex::new(HashMap::new()));
    let graph = DroneGraph {
        nodes: vec![
            vars("a", "x", "{{trigger.title}}"),
            node("c", json!({ "kind": "condition", "expr": "{{var.x}} == hi" })),
            response("r", "{{c.result}} {{var.x}}"),
        ],
        edges: vec![edge("e1", "a", "c", None), edge("e2", "c", "r", None)],
    };
    let plan = schedule::analyze(&graph).unwrap();
    let scope = ExecutionScope::with_trigger(json!({ "title": "hi" }));
    let mut s = Scheduler::new("r", &graph, plan, scope, None, &tx, &states);
    s.run().await.unwrap();
    assert!(s.scope.state.get("x").unwrap().tainted);
    assert_eq!(s.scope.bits["c"], (false, true), "a boolean made from tainted text is derived, not tainted");
    assert_eq!(s.scope.bits["r"], (true, false), "a response reading the tainted variable is tainted");
}
