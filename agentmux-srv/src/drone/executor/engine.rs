// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! DAG executor — topological sort + per-layer runner.
//!
//! Phase 1 is sequential per layer (one block at a time). Phase 2
//! parallelizes independent branches with `tokio::spawn`.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::mpsc;

use super::blocks;
use crate::drone::data_flow::ExecutionScope;
use crate::drone::types::{BlockKind, BlockState, FlowEdge, FlowNode, DroneGraph};

/// Streaming events emitted to the frontend during a run.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunEvent {
    RunStarted { run_id: String, drone_id: String },
    BlockStarted { run_id: String, block_id: String },
    BlockDone {
        run_id: String,
        block_id: String,
        output: Value,
    },
    BlockError {
        run_id: String,
        block_id: String,
        error: String,
    },
    RunDone {
        run_id: String,
        output: Value,
    },
    RunFailed { run_id: String, error: String },
}

pub struct RunHandle {
    pub run_id: String,
    pub events: mpsc::UnboundedReceiver<RunEvent>,
    pub final_states: Arc<tokio::sync::Mutex<HashMap<String, BlockState>>>,
}

/// Run the drone end-to-end. Returns a handle whose `events` channel
/// surfaces per-block lifecycle, and a `final_states` mutex callers can
/// snapshot for the run-history record.
pub async fn run_drone(
    drone_id: String,
    graph: DroneGraph,
) -> Result<RunHandle, String> {
    let run_id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = mpsc::unbounded_channel();
    let final_states: Arc<tokio::sync::Mutex<HashMap<String, BlockState>>> =
        Arc::new(tokio::sync::Mutex::new(HashMap::new()));

    let states_for_task = final_states.clone();
    let drone_id_for_evt = drone_id.clone();
    let run_id_for_task = run_id.clone();
    let tx_for_task = tx.clone();
    tokio::spawn(async move {
        let _ = tx_for_task.send(RunEvent::RunStarted {
            run_id: run_id_for_task.clone(),
            drone_id: drone_id_for_evt,
        });
        match execute(&run_id_for_task, &graph, &tx_for_task, &states_for_task).await {
            Ok(output) => {
                let _ = tx_for_task.send(RunEvent::RunDone {
                    run_id: run_id_for_task,
                    output,
                });
            }
            Err(e) => {
                let _ = tx_for_task.send(RunEvent::RunFailed {
                    run_id: run_id_for_task,
                    error: e,
                });
            }
        }
    });

    Ok(RunHandle {
        run_id,
        events: rx,
        final_states,
    })
}

async fn execute(
    run_id: &str,
    graph: &DroneGraph,
    tx: &mpsc::UnboundedSender<RunEvent>,
    final_states: &Arc<tokio::sync::Mutex<HashMap<String, BlockState>>>,
) -> Result<Value, String> {
    if graph.nodes.is_empty() {
        return Err("drone has no blocks".to_string());
    }

    let layers = topological_layers(graph)?;
    let mut scope = ExecutionScope::new();
    let mut response_output: Option<Value> = None;
    let nodes_by_id: HashMap<String, &FlowNode> =
        graph.nodes.iter().map(|n| (n.id.clone(), n)).collect();
    // Blocks whose execution is pruned by a Condition's non-matching
    // branch (or transitively, because all their incoming edges came
    // from skipped/pruned sources). They consume no events and run no
    // block runner — important so the false-branch's API/agent calls
    // do not fire side effects when the condition is true.
    let mut skipped: HashSet<String> = HashSet::new();

    for layer in layers {
        for block_id in layer {
            let node = nodes_by_id
                .get(&block_id)
                .ok_or_else(|| format!("internal: missing node {block_id}"))?;
            let kind = block_kind_of(node)?;

            if should_skip(&block_id, graph, &nodes_by_id, &scope, &skipped) {
                skipped.insert(block_id.clone());
                mark_state(
                    final_states,
                    &block_id,
                    BlockState {
                        status: "skipped".to_string(),
                        output: None,
                        error: None,
                        started_at: None,
                        completed_at: Some(now_ms()),
                    },
                )
                .await;
                continue;
            }

            let _ = tx.send(RunEvent::BlockStarted {
                run_id: run_id.to_string(),
                block_id: block_id.clone(),
            });
            // Captured once at BlockStarted so the running / done / error
            // transitions all carry the same start timestamp — otherwise
            // the run-history record loses per-block duration (reagent P2).
            let started_at = now_ms();
            mark_state(
                final_states,
                &block_id,
                BlockState {
                    status: "running".to_string(),
                    output: None,
                    error: None,
                    started_at: Some(started_at),
                    completed_at: None,
                },
            )
            .await;

            let result = match kind {
                BlockKind::Variables => blocks::variables::run(node, &mut scope).await,
                BlockKind::Api => blocks::api::run(node, &scope).await,
                BlockKind::Condition => blocks::condition::run(node, &scope).await,
                BlockKind::Response => blocks::response::run(node, &scope).await,
                BlockKind::Agent => blocks::agent::run(node, &scope).await,
            };

            match result {
                Ok(output) => {
                    scope.outputs.insert(block_id.clone(), output.clone());
                    let _ = tx.send(RunEvent::BlockDone {
                        run_id: run_id.to_string(),
                        block_id: block_id.clone(),
                        output: output.clone(),
                    });
                    mark_state(
                        final_states,
                        &block_id,
                        BlockState {
                            status: "done".to_string(),
                            output: Some(output.clone()),
                            error: None,
                            started_at: Some(started_at),
                            completed_at: Some(now_ms()),
                        },
                    )
                    .await;
                    if matches!(kind, BlockKind::Response) {
                        // Response emits `{ "value": <resolved-template> }`;
                        // unwrap to the bare value so the run's final
                        // output is the user-facing string (or whatever
                        // the template resolved to), not the wrapper
                        // JSON. (codex P3 on PR #755.)
                        response_output = Some(
                            output
                                .get("value")
                                .cloned()
                                .unwrap_or(output),
                        );
                    }
                }
                Err(e) => {
                    let _ = tx.send(RunEvent::BlockError {
                        run_id: run_id.to_string(),
                        block_id: block_id.clone(),
                        error: e.clone(),
                    });
                    mark_state(
                        final_states,
                        &block_id,
                        BlockState {
                            status: "error".to_string(),
                            output: None,
                            error: Some(e.clone()),
                            started_at: Some(started_at),
                            completed_at: Some(now_ms()),
                        },
                    )
                    .await;
                    return Err(format!("block {block_id} failed: {e}"));
                }
            }
        }
    }

    Ok(response_output.unwrap_or(Value::Null))
}

async fn mark_state(
    states: &Arc<tokio::sync::Mutex<HashMap<String, BlockState>>>,
    id: &str,
    state: BlockState,
) {
    let mut g = states.lock().await;
    g.insert(id.to_string(), state);
}

fn now_ms() -> i64 {
    agentmux_common::time::now_ms()
}

/// Returns the layered topological order of the graph. Each layer is
/// the set of blocks whose dependencies are already satisfied —
/// independent branches run together.
pub fn topological_layers(graph: &DroneGraph) -> Result<Vec<Vec<String>>, String> {
    let mut indegree: HashMap<String, usize> = HashMap::new();
    let mut outedges: HashMap<String, Vec<String>> = HashMap::new();
    let known: HashSet<String> = graph.nodes.iter().map(|n| n.id.clone()).collect();
    for n in &graph.nodes {
        indegree.entry(n.id.clone()).or_insert(0);
        outedges.entry(n.id.clone()).or_insert_with(Vec::new);
    }
    for e in &graph.edges {
        if !known.contains(&e.source) || !known.contains(&e.target) {
            return Err(format!(
                "edge references unknown node ({} → {})",
                e.source, e.target
            ));
        }
        *indegree.entry(e.target.clone()).or_insert(0) += 1;
        outedges
            .entry(e.source.clone())
            .or_insert_with(Vec::new)
            .push(e.target.clone());
    }

    let mut layers: Vec<Vec<String>> = Vec::new();
    let mut frontier: VecDeque<String> = indegree
        .iter()
        .filter_map(|(id, deg)| if *deg == 0 { Some(id.clone()) } else { None })
        .collect();

    while !frontier.is_empty() {
        let mut layer: Vec<String> = Vec::new();
        let next_frontier: Vec<String> = frontier.drain(..).collect();
        for id in &next_frontier {
            layer.push(id.clone());
        }
        // Stable order by id within a layer (for deterministic tests).
        layer.sort();
        for id in &layer {
            if let Some(targets) = outedges.get(id).cloned() {
                for t in targets {
                    if let Some(d) = indegree.get_mut(&t) {
                        if *d > 0 {
                            *d -= 1;
                            if *d == 0 {
                                frontier.push_back(t);
                            }
                        }
                    }
                }
            }
        }
        layers.push(layer);
    }

    let scheduled: usize = layers.iter().map(|l| l.len()).sum();
    if scheduled != graph.nodes.len() {
        return Err("drone contains a cycle".to_string());
    }
    Ok(layers)
}

fn block_kind_of(node: &FlowNode) -> Result<BlockKind, String> {
    let kind = node
        .data
        .get("kind")
        .and_then(|v| v.as_str())
        .ok_or_else(|| format!("node {} has no `kind` field", node.id))?;
    BlockKind::parse(kind).ok_or_else(|| format!("unknown block kind: {kind}"))
}

/// Decide whether `node_id` should be pruned given the per-run state.
///
/// A block runs iff at least one of its incoming edges is "active":
///   * The edge's source is NOT in `skipped`, AND
///   * If the source is a Condition block, the edge's `source_handle`
///     ("true" / "false") matches the boolean stored in the Condition's
///     `result` output.
///
/// Blocks with no incoming edges are root nodes and never skip.
/// Topological ordering guarantees every source has already been
/// processed (run or skipped) by the time we evaluate a target here.
fn should_skip(
    node_id: &str,
    graph: &DroneGraph,
    nodes_by_id: &HashMap<String, &FlowNode>,
    scope: &ExecutionScope,
    skipped: &HashSet<String>,
) -> bool {
    let mut had_incoming = false;
    let mut had_active = false;
    for edge in &graph.edges {
        if edge.target != node_id {
            continue;
        }
        had_incoming = true;
        if edge_is_active(edge, nodes_by_id, scope, skipped) {
            had_active = true;
            break;
        }
    }
    had_incoming && !had_active
}

/// True iff the given edge carries control + data flow under the
/// current scope. See `should_skip` for the full ruleset.
fn edge_is_active(
    edge: &FlowEdge,
    nodes_by_id: &HashMap<String, &FlowNode>,
    scope: &ExecutionScope,
    skipped: &HashSet<String>,
) -> bool {
    if skipped.contains(&edge.source) {
        return false;
    }
    let src_node = match nodes_by_id.get(&edge.source) {
        Some(n) => n,
        None => return true,
    };
    let is_condition = src_node
        .data
        .get("kind")
        .and_then(|v| v.as_str())
        == Some("condition");
    if !is_condition {
        return true;
    }
    let cond_result = scope
        .outputs
        .get(&edge.source)
        .and_then(|v| v.get("result"))
        .and_then(|v| v.as_bool());
    match (edge.source_handle.as_deref(), cond_result) {
        (Some("true"), Some(r)) => r,
        (Some("false"), Some(r)) => !r,
        // Unhandled / pre-spec edges off a Condition — be permissive
        // (Phase 2 will tighten this once the canvas always sets a
        // source_handle on condition edges).
        _ => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::drone::types::{FlowEdge, FlowNode, NodePosition};
    use serde_json::json;

    fn n(id: &str, kind: &str) -> FlowNode {
        FlowNode {
            id: id.to_string(),
            position: NodePosition::default(),
            data: json!({ "kind": kind }),
            node_type: String::new(),
        }
    }

    fn e(id: &str, src: &str, dst: &str) -> FlowEdge {
        FlowEdge {
            id: id.to_string(),
            source: src.to_string(),
            target: dst.to_string(),
            source_handle: None,
            target_handle: None,
        }
    }

    #[test]
    fn topo_orders_diamond() {
        // a → b, a → c, b → d, c → d
        let g = DroneGraph {
            nodes: vec![n("a", "variables"), n("b", "api"), n("c", "api"), n("d", "response")],
            edges: vec![
                e("e1", "a", "b"),
                e("e2", "a", "c"),
                e("e3", "b", "d"),
                e("e4", "c", "d"),
            ],
        };
        let layers = topological_layers(&g).unwrap();
        assert_eq!(layers.len(), 3);
        assert_eq!(layers[0], vec!["a"]);
        assert_eq!(layers[1], vec!["b", "c"]);
        assert_eq!(layers[2], vec!["d"]);
    }

    #[test]
    fn topo_rejects_cycle() {
        let g = DroneGraph {
            nodes: vec![n("a", "variables"), n("b", "api")],
            edges: vec![e("e1", "a", "b"), e("e2", "b", "a")],
        };
        assert!(topological_layers(&g).is_err());
    }

    #[test]
    fn topo_rejects_unknown_node_in_edge() {
        let g = DroneGraph {
            nodes: vec![n("a", "variables")],
            edges: vec![e("e1", "a", "ghost")],
        };
        assert!(topological_layers(&g).is_err());
    }

    #[test]
    fn topo_single_node_one_layer() {
        let g = DroneGraph {
            nodes: vec![n("a", "response")],
            edges: vec![],
        };
        let layers = topological_layers(&g).unwrap();
        assert_eq!(layers, vec![vec!["a".to_string()]]);
    }

    // ────────────────────────────────────────────────────────────────
    // Condition branch pruning (codex P2 on PR #755)
    // ────────────────────────────────────────────────────────────────

    fn eh(id: &str, src: &str, dst: &str, handle: Option<&str>) -> FlowEdge {
        FlowEdge {
            id: id.to_string(),
            source: src.to_string(),
            target: dst.to_string(),
            source_handle: handle.map(|s| s.to_string()),
            target_handle: None,
        }
    }

    fn nodes_map<'a>(g: &'a DroneGraph) -> HashMap<String, &'a FlowNode> {
        g.nodes.iter().map(|n| (n.id.clone(), n)).collect()
    }

    fn scope_with_cond(cond_id: &str, result: bool) -> ExecutionScope {
        let mut scope = ExecutionScope::new();
        scope
            .outputs
            .insert(cond_id.to_string(), json!({ "result": result }));
        scope
    }

    #[test]
    fn skip_root_node_runs() {
        // No incoming edges — always runs regardless of state.
        let g = DroneGraph {
            nodes: vec![n("a", "variables")],
            edges: vec![],
        };
        let nodes = nodes_map(&g);
        assert!(!should_skip(
            "a",
            &g,
            &nodes,
            &ExecutionScope::new(),
            &HashSet::new()
        ));
    }

    #[test]
    fn skip_unconditional_chain_runs() {
        // a → b (a is plain Variables, not a condition).
        let g = DroneGraph {
            nodes: vec![n("a", "variables"), n("b", "api")],
            edges: vec![e("e1", "a", "b")],
        };
        let nodes = nodes_map(&g);
        assert!(!should_skip(
            "b",
            &g,
            &nodes,
            &ExecutionScope::new(),
            &HashSet::new()
        ));
    }

    #[test]
    fn skip_condition_false_branch_when_result_true() {
        // c (condition, true) → t via "true" handle (active)
        //                    → f via "false" handle (inactive)
        let g = DroneGraph {
            nodes: vec![n("c", "condition"), n("t", "api"), n("f", "api")],
            edges: vec![
                eh("e1", "c", "t", Some("true")),
                eh("e2", "c", "f", Some("false")),
            ],
        };
        let nodes = nodes_map(&g);
        let scope = scope_with_cond("c", true);
        let skipped = HashSet::new();
        assert!(!should_skip("t", &g, &nodes, &scope, &skipped));
        assert!(should_skip("f", &g, &nodes, &scope, &skipped));
    }

    #[test]
    fn skip_condition_true_branch_when_result_false() {
        let g = DroneGraph {
            nodes: vec![n("c", "condition"), n("t", "api"), n("f", "api")],
            edges: vec![
                eh("e1", "c", "t", Some("true")),
                eh("e2", "c", "f", Some("false")),
            ],
        };
        let nodes = nodes_map(&g);
        let scope = scope_with_cond("c", false);
        let skipped = HashSet::new();
        assert!(should_skip("t", &g, &nodes, &scope, &skipped));
        assert!(!should_skip("f", &g, &nodes, &scope, &skipped));
    }

    #[test]
    fn skip_transitive_through_skipped_source() {
        // c (condition, true) → f via "false" (skipped) → x
        // x's only incoming is from skipped `f`, so x must also skip
        // — guards against false-branch side effects past depth 1.
        let g = DroneGraph {
            nodes: vec![n("c", "condition"), n("f", "api"), n("x", "agent")],
            edges: vec![
                eh("e1", "c", "f", Some("false")),
                e("e2", "f", "x"),
            ],
        };
        let nodes = nodes_map(&g);
        let scope = scope_with_cond("c", true);
        let mut skipped = HashSet::new();
        skipped.insert("f".to_string());
        assert!(should_skip("x", &g, &nodes, &scope, &skipped));
    }

    #[test]
    fn join_runs_if_any_incoming_active() {
        // a (active) → d, b (skipped) → d
        // d has one active source — Phase 1 any-active semantics.
        let g = DroneGraph {
            nodes: vec![n("a", "variables"), n("b", "api"), n("d", "response")],
            edges: vec![e("e1", "a", "d"), e("e2", "b", "d")],
        };
        let nodes = nodes_map(&g);
        let mut skipped = HashSet::new();
        skipped.insert("b".to_string());
        assert!(!should_skip(
            "d",
            &g,
            &nodes,
            &ExecutionScope::new(),
            &skipped
        ));
    }

    #[test]
    fn condition_edge_without_handle_is_permissive() {
        // Pre-spec edges off a Condition block lack a source_handle.
        // Don't accidentally prune them — Phase 2 will require the
        // handle once the canvas always sets it.
        let g = DroneGraph {
            nodes: vec![n("c", "condition"), n("x", "api")],
            edges: vec![eh("e1", "c", "x", None)],
        };
        let nodes = nodes_map(&g);
        let scope = scope_with_cond("c", false);
        assert!(!should_skip("x", &g, &nodes, &scope, &HashSet::new()));
    }

    #[tokio::test]
    async fn execute_preserves_started_at_in_block_state() {
        // Reagent P2: BlockDone used to overwrite started_at with None,
        // making per-block duration uncomputable from the persisted
        // run record. Verify the timestamp survives the running -> done
        // transition.
        let mut vars_node = n("v1", "variables");
        // Variables block reads `entries: [{name, value}]`, not `vars`.
        // Earlier draft used the wrong key — the test still passed
        // because it only asserts started_at, but the Variables block
        // was running an empty-entries no-op rather than the intended
        // path. (reagent P2 on PR #755 round 7.)
        vars_node.data = json!({
            "kind": "variables",
            "entries": [{ "name": "v", "value": 1 }]
        });
        let mut resp_node = n("r1", "response");
        resp_node.data = json!({ "kind": "response", "template": "done" });
        let g = DroneGraph {
            nodes: vec![vars_node, resp_node],
            edges: vec![e("e1", "v1", "r1")],
        };

        let handle = run_drone("wf1".to_string(), g).await.unwrap();
        let mut rx = handle.events;
        while let Some(ev) = rx.recv().await {
            if matches!(ev, RunEvent::RunDone { .. } | RunEvent::RunFailed { .. }) {
                break;
            }
        }

        let states = handle.final_states.lock().await;
        let v1 = states.get("v1").expect("v1 state");
        let r1 = states.get("r1").expect("r1 state");
        assert_eq!(v1.status, "done");
        assert!(
            v1.started_at.is_some() && v1.started_at.unwrap() > 0,
            "v1 started_at must survive the done transition; got {:?}",
            v1.started_at
        );
        assert!(v1.completed_at.is_some());
        assert!(v1.completed_at.unwrap() >= v1.started_at.unwrap());
        assert!(r1.started_at.is_some());
        assert!(r1.completed_at.is_some());
    }

    #[test]
    fn flow_edge_serializes_with_camelcase_handle_fields() {
        // Wire format must match xyflow + the frontend TS
        // `DroneFlowEdge` shape (sourceHandle / targetHandle).
        // Snake-case would silently drop the field on the frontend
        // roundtrip, leaving the executor's branch-pruning permissive.
        let edge = FlowEdge {
            id: "e1".to_string(),
            source: "a".to_string(),
            target: "b".to_string(),
            source_handle: Some("true".to_string()),
            target_handle: None,
        };
        let json = serde_json::to_string(&edge).unwrap();
        assert!(
            json.contains("\"sourceHandle\":\"true\""),
            "expected sourceHandle in JSON; got {json}"
        );
        // Roundtrip preserves the handle.
        let parsed: FlowEdge = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed.source_handle.as_deref(), Some("true"));
    }

    #[tokio::test]
    async fn execute_skips_pruned_branch_end_to_end() {
        // Variables(v=10) → Condition({{var.v}} < 5) → Response("hit_true")
        //                                            ↘ Response("hit_false")
        // Result: cond is false, true-branch Response is skipped,
        // false-branch Response runs and becomes the run output.
        //
        // The Variables block reads `entries: [{name, value}]` (not
        // `vars`), so the data shape mirrors what the canvas emits.
        // An earlier draft of this test used the wrong key and the
        // condition passed by coincidence (unresolved `{{var.v}}`
        // string-compared against `5`); reagent caught it.
        let mut vars_node = n("v1", "variables");
        vars_node.data = json!({
            "kind": "variables",
            "entries": [{ "name": "v", "value": 10 }]
        });
        let mut cond_node = n("c1", "condition");
        cond_node.data = json!({
            "kind": "condition",
            "expr": "{{var.v}} < 5"
        });
        let mut t_resp = n("rt", "response");
        t_resp.data = json!({
            "kind": "response",
            "template": "hit_true"
        });
        let mut f_resp = n("rf", "response");
        f_resp.data = json!({
            "kind": "response",
            "template": "hit_false"
        });
        let g = DroneGraph {
            nodes: vec![vars_node, cond_node, t_resp, f_resp],
            edges: vec![
                e("e1", "v1", "c1"),
                eh("e2", "c1", "rt", Some("true")),
                eh("e3", "c1", "rf", Some("false")),
            ],
        };

        let handle = run_drone("wf1".to_string(), g).await.unwrap();
        // Drain events to completion.
        let mut rx = handle.events;
        let mut got_done_ids: Vec<String> = Vec::new();
        let mut final_output: Option<Value> = None;
        while let Some(ev) = rx.recv().await {
            match ev {
                RunEvent::BlockDone { block_id, .. } => got_done_ids.push(block_id),
                RunEvent::RunDone { output, .. } => {
                    final_output = Some(output);
                    break;
                }
                RunEvent::RunFailed { error, .. } => panic!("run failed: {error}"),
                _ => {}
            }
        }

        // false-branch ran; true-branch was skipped — no BlockDone for "rt".
        assert!(got_done_ids.contains(&"rf".to_string()));
        assert!(
            !got_done_ids.contains(&"rt".to_string()),
            "true branch must NOT run when condition is false; got: {got_done_ids:?}"
        );

        // Final state for the skipped block records "skipped".
        let states = handle.final_states.lock().await;
        let rt_state = states.get("rt").expect("rt state recorded");
        assert_eq!(rt_state.status, "skipped");

        // The run's response output is the false-branch's resolution,
        // unwrapped from Response's `{ "value": ... }` envelope so
        // downstream consumers see the bare string (codex P3).
        assert_eq!(final_output, Some(json!("hit_false")));
    }
}
