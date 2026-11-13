// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A run's handle and its event stream. The scheduler that runs the graph is
//! `scheduler.rs`.

use std::collections::HashMap;
use std::sync::Arc;

use serde_json::Value;
use tokio::sync::mpsc;

use crate::drone::types::{BlockState, DroneGraph};

/// Streaming events emitted to the frontend during a run.
#[derive(Debug, Clone, serde::Serialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum RunEvent {
    RunStarted { run_id: String, drone_id: String },
    BlockStarted { run_id: String, block_id: String },
    BlockDone {
        run_id: String,
        block_id: String,
        #[ts(type = "unknown")]
        output: Value,
    },
    BlockError {
        run_id: String,
        block_id: String,
        error: String,
    },
    RunDone {
        run_id: String,
        #[ts(type = "unknown")]
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
/// snapshot for the run-history record. `trigger` is what started the run
/// (`{{trigger.*}}`); `Null` when nothing did. `run_cap_usd` ends the run
/// before its next block once its Agent blocks have spent that much (a job's
/// per-run cap); `None` is no cap, as for a run started by hand.
pub async fn run_drone(
    drone_id: String,
    graph: DroneGraph,
    trigger: Value,
    run_cap_usd: Option<f64>,
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
        match super::scheduler::execute(&run_id_for_task, (&graph, trigger, run_cap_usd, &drone_id), &tx_for_task, &states_for_task).await {
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

pub(super) async fn mark_state(
    states: &Arc<tokio::sync::Mutex<HashMap<String, BlockState>>>,
    id: &str,
    state: BlockState,
) {
    let mut g = states.lock().await;
    g.insert(id.to_string(), state);
}

#[cfg(test)]
mod tests {
    use super::super::scheduler::over_run_cap;
    use super::*;
    use crate::drone::types::{FlowEdge, FlowNode, NodePosition};
    use serde_json::json;

    fn n(id: &str, kind: &str) -> FlowNode {
        FlowNode {
            id: id.to_string(),
            position: NodePosition::default(),
            data: json!({ "kind": kind }),
            node_type: None,
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

        let handle = run_drone("wf1".to_string(), g, Value::Null, None).await.unwrap();
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

    /// A cap ends the run before the next block once the Agent blocks before it
    /// have spent it; a run with no cap, with room, or that spent nothing goes on.
    #[tokio::test]
    async fn a_run_is_held_to_its_cap_between_blocks() {
        assert_eq!(over_run_cap(0.0, None), None);
        assert_eq!(over_run_cap(3.0, None), None);
        assert_eq!(over_run_cap(3.0, Some(5.0)), None);
        assert_eq!(over_run_cap(3.0, Some(3.0)), Some("stopped: the run spent $3.00, its cap of $3.00".to_string()));
        assert_eq!(over_run_cap(0.0, Some(0.0)), None, "a run that spent nothing has not reached a cap of 0");

        // The cap is passed through: a run with room, or none, finishes.
        let mut resp = n("r1", "response");
        resp.data = json!({ "kind": "response", "template": "done" });
        for cap in [None, Some(5.0), Some(0.0)] {
            let g = DroneGraph { nodes: vec![resp.clone()], edges: vec![] };
            let mut handle = run_drone("wf1".to_string(), g, Value::Null, cap).await.unwrap();
            let mut out = None;
            while let Some(ev) = handle.events.recv().await {
                if let RunEvent::RunDone { output, .. } = ev {
                    out = Some(output);
                }
            }
            assert_eq!(out, Some(json!("done")), "{cap:?}");
        }
    }

    /// `{{trigger.*}}` reads what started the run; with nothing it stays visible.
    #[tokio::test]
    async fn a_run_reads_its_trigger() {
        let mut resp = n("r1", "response");
        resp.data = json!({ "kind": "response", "template": "{{trigger.repo}}#{{trigger.pr.number}} {{trigger.none}}" });
        let g = DroneGraph { nodes: vec![resp], edges: vec![] };
        let input = json!({ "repo": "agentmux", "pr": { "number": 12 } });
        let mut handle = run_drone("wf1".to_string(), g, input, None).await.unwrap();
        let mut out = None;
        while let Some(ev) = handle.events.recv().await {
            match ev {
                RunEvent::RunDone { output, .. } => out = Some(output),
                RunEvent::RunFailed { error, .. } => panic!("run failed: {error}"),
                _ => {}
            }
        }
        assert_eq!(out, Some(json!("agentmux#12 {{trigger.none}}")));
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

        let handle = run_drone("wf1".to_string(), g, Value::Null, None).await.unwrap();
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
