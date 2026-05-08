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
use crate::workflows::data_flow::ExecutionScope;
use crate::workflows::types::{BlockKind, BlockState, FlowEdge, FlowNode, WorkflowGraph};

/// Streaming events emitted to the frontend during a run.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunEvent {
    RunStarted { run_id: String, workflow_id: String },
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

/// Run the workflow end-to-end. Returns a handle whose `events` channel
/// surfaces per-block lifecycle, and a `final_states` mutex callers can
/// snapshot for the run-history record.
pub async fn run_workflow(
    workflow_id: String,
    graph: WorkflowGraph,
) -> Result<RunHandle, String> {
    let run_id = uuid::Uuid::new_v4().to_string();
    let (tx, rx) = mpsc::unbounded_channel();
    let final_states: Arc<tokio::sync::Mutex<HashMap<String, BlockState>>> =
        Arc::new(tokio::sync::Mutex::new(HashMap::new()));

    let states_for_task = final_states.clone();
    let workflow_id_for_evt = workflow_id.clone();
    let run_id_for_task = run_id.clone();
    let tx_for_task = tx.clone();
    tokio::spawn(async move {
        let _ = tx_for_task.send(RunEvent::RunStarted {
            run_id: run_id_for_task.clone(),
            workflow_id: workflow_id_for_evt,
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
    graph: &WorkflowGraph,
    tx: &mpsc::UnboundedSender<RunEvent>,
    final_states: &Arc<tokio::sync::Mutex<HashMap<String, BlockState>>>,
) -> Result<Value, String> {
    if graph.nodes.is_empty() {
        return Err("workflow has no blocks".to_string());
    }

    let layers = topological_layers(graph)?;
    let mut scope = ExecutionScope::new();
    let mut response_output: Option<Value> = None;
    let nodes_by_id: HashMap<String, &FlowNode> =
        graph.nodes.iter().map(|n| (n.id.clone(), n)).collect();

    for layer in layers {
        for block_id in layer {
            let node = nodes_by_id
                .get(&block_id)
                .ok_or_else(|| format!("internal: missing node {block_id}"))?;
            let kind = block_kind_of(node)?;

            let _ = tx.send(RunEvent::BlockStarted {
                run_id: run_id.to_string(),
                block_id: block_id.clone(),
            });
            mark_state(
                final_states,
                &block_id,
                BlockState {
                    status: "running".to_string(),
                    output: None,
                    error: None,
                    started_at: Some(now_ms()),
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
                            started_at: None, // preserve started_at? skip in Phase 1
                            completed_at: Some(now_ms()),
                        },
                    )
                    .await;
                    if matches!(kind, BlockKind::Response) {
                        response_output = Some(output);
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
                            started_at: None,
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
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

/// Returns the layered topological order of the graph. Each layer is
/// the set of blocks whose dependencies are already satisfied —
/// independent branches run together.
pub fn topological_layers(graph: &WorkflowGraph) -> Result<Vec<Vec<String>>, String> {
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
        return Err("workflow contains a cycle".to_string());
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

#[allow(dead_code)]
fn _unused_edge(_e: &FlowEdge) {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflows::types::{FlowEdge, FlowNode, NodePosition};
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
        let g = WorkflowGraph {
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
        let g = WorkflowGraph {
            nodes: vec![n("a", "variables"), n("b", "api")],
            edges: vec![e("e1", "a", "b"), e("e2", "b", "a")],
        };
        assert!(topological_layers(&g).is_err());
    }

    #[test]
    fn topo_rejects_unknown_node_in_edge() {
        let g = WorkflowGraph {
            nodes: vec![n("a", "variables")],
            edges: vec![e("e1", "a", "ghost")],
        };
        assert!(topological_layers(&g).is_err());
    }

    #[test]
    fn topo_single_node_one_layer() {
        let g = WorkflowGraph {
            nodes: vec![n("a", "response")],
            edges: vec![],
        };
        let layers = topological_layers(&g).unwrap();
        assert_eq!(layers, vec![vec!["a".to_string()]]);
    }
}
