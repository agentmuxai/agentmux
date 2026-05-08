// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! WSH RPC handlers for the Workflows pane (issue #753 Phase 1).
//!
//! Commands:
//!   * `listworkflows`       → `Vec<WorkflowDefinition>`
//!   * `getworkflow`         → `Option<WorkflowDefinition>`
//!   * `upsertworkflow`      → `WorkflowDefinition` (echoed back, normalized)
//!   * `deleteworkflow`      → `{ deleted: bool }`
//!   * `runworkflow`         → `{ run_id: String }` (synchronous; SSE
//!                              streaming added in Phase 1 PR-4)
//!   * `listworkflowruns`    → `Vec<WorkflowRun>`
//!
//! Run streaming: the executor emits `RunEvent`s over an mpsc channel.
//! Phase 1 of this PR drains the channel server-side and stores the
//! final block-state snapshot in `db_workflow_runs`. A future commit
//! will tee the channel to the renderer via the existing `wps` event
//! broker so `RunPanel` shows live per-block status.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

use crate::backend::rpc_types::{
    COMMAND_DELETE_WORKFLOW, COMMAND_GET_WORKFLOW, COMMAND_LIST_WORKFLOWS,
    COMMAND_LIST_WORKFLOW_RUNS, COMMAND_RUN_WORKFLOW, COMMAND_UPSERT_WORKFLOW,
};
use crate::backend::wps::WaveEvent;
use crate::server::AppState;
use crate::backend::rpc::engine::WshRpcEngine;
use crate::workflows::executor::{run_workflow, RunEvent};
use crate::workflows::storage::WorkflowStore;
use crate::workflows::types::{RunStatus, WorkflowDefinition, WorkflowRun};

#[derive(Debug, Deserialize)]
struct GetWorkflowReq {
    id: String,
}

#[derive(Debug, Deserialize)]
struct DeleteWorkflowReq {
    id: String,
}

#[derive(Debug, Deserialize)]
struct RunWorkflowReq {
    workflow_id: String,
}

#[derive(Debug, Deserialize)]
struct ListRunsReq {
    workflow_id: String,
    #[serde(default = "default_limit")]
    limit: i64,
}

fn default_limit() -> i64 {
    50
}

#[derive(Debug, Serialize)]
struct DeleteResp {
    deleted: bool,
}

#[derive(Debug, Serialize)]
struct RunResp {
    run_id: String,
}

pub fn register_workflow_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    let wstore = state.wstore.clone();
    engine.register_handler(
        COMMAND_LIST_WORKFLOWS,
        Box::new(move |_data, _ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                let list = wstore
                    .workflow_list()
                    .map_err(|e| format!("listworkflows: {e}"))?;
                Ok(Some(serde_json::to_value(&list).unwrap_or_default()))
            })
        }),
    );

    let wstore = state.wstore.clone();
    engine.register_handler(
        COMMAND_GET_WORKFLOW,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                let cmd: GetWorkflowReq = serde_json::from_value(data)
                    .map_err(|e| format!("getworkflow: {e}"))?;
                let row = wstore
                    .workflow_get(&cmd.id)
                    .map_err(|e| format!("getworkflow: {e}"))?;
                Ok(Some(serde_json::to_value(&row).unwrap_or_default()))
            })
        }),
    );

    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_UPSERT_WORKFLOW,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let mut cmd: WorkflowDefinition = serde_json::from_value(data)
                    .map_err(|e| format!("upsertworkflow: {e}"))?;
                let now = now_ms();
                if cmd.created_at == 0 {
                    cmd.created_at = now;
                }
                cmd.updated_at = now;
                if cmd.id.is_empty() {
                    cmd.id = uuid::Uuid::new_v4().to_string();
                }
                wstore
                    .workflow_upsert(&cmd)
                    .map_err(|e| format!("upsertworkflow: {e}"))?;
                broker.publish(WaveEvent {
                    event: "workflows:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&cmd).unwrap_or_default()))
            })
        }),
    );

    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_DELETE_WORKFLOW,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let cmd: DeleteWorkflowReq = serde_json::from_value(data)
                    .map_err(|e| format!("deleteworkflow: {e}"))?;
                let deleted = wstore
                    .workflow_delete(&cmd.id)
                    .map_err(|e| format!("deleteworkflow: {e}"))?;
                if deleted {
                    broker.publish(WaveEvent {
                        event: "workflows:changed".to_string(),
                        scopes: vec![],
                        sender: String::new(),
                        persist: 0,
                        data: None,
                    });
                }
                Ok(Some(serde_json::to_value(&DeleteResp { deleted }).unwrap_or_default()))
            })
        }),
    );

    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_RUN_WORKFLOW,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let cmd: RunWorkflowReq = serde_json::from_value(data)
                    .map_err(|e| format!("runworkflow: {e}"))?;
                let wf = wstore
                    .workflow_get(&cmd.workflow_id)
                    .map_err(|e| format!("runworkflow: {e}"))?
                    .ok_or_else(|| {
                        format!("runworkflow: workflow {} not found", cmd.workflow_id)
                    })?;
                let started_at = now_ms();
                let mut handle = run_workflow(wf.id.clone(), wf.graph.clone())
                    .await
                    .map_err(|e| format!("runworkflow: {e}"))?;
                let run_id = handle.run_id.clone();
                let workflow_id = wf.id.clone();

                // Drain events on a background task so the RPC returns
                // immediately. The frontend will subscribe to per-run
                // events via the broker (Phase 1 PR-4 wiring).
                let wstore_for_drain = wstore.clone();
                let broker_for_drain = broker.clone();
                let run_id_for_drain = run_id.clone();
                let workflow_id_for_drain = workflow_id.clone();
                let final_states = handle.final_states.clone();
                tokio::spawn(async move {
                    let mut last_status = RunStatus::Running;
                    let mut output = String::new();
                    let mut error = String::new();
                    while let Some(ev) = handle.events.recv().await {
                        // Emit each event for live RunPanel.
                        broker_for_drain.publish(WaveEvent {
                            event: format!("workflowrun:{}", run_id_for_drain),
                            scopes: vec![],
                            sender: String::new(),
                            persist: 0,
                            data: Some(serde_json::to_value(&ev).unwrap_or_default()),
                        });
                        match &ev {
                            RunEvent::RunDone { output: o, .. } => {
                                last_status = RunStatus::Done;
                                output = serde_json::to_string(o).unwrap_or_default();
                            }
                            RunEvent::RunFailed { error: e, .. } => {
                                last_status = RunStatus::Failed;
                                error = e.clone();
                            }
                            _ => {}
                        }
                    }
                    let states = final_states.lock().await.clone();
                    let row = WorkflowRun {
                        id: run_id_for_drain.clone(),
                        workflow_id: workflow_id_for_drain,
                        status: last_status.as_str().to_string(),
                        started_at,
                        ended_at: now_ms(),
                        block_states: states,
                        output,
                        error,
                    };
                    if let Err(e) = wstore_for_drain.workflow_run_insert(&row) {
                        tracing::warn!(run_id = %run_id_for_drain, error = %e, "workflow_run_insert failed");
                    }
                });

                Ok(Some(serde_json::to_value(&RunResp { run_id }).unwrap_or_default()))
            })
        }),
    );

    let wstore = state.wstore.clone();
    engine.register_handler(
        COMMAND_LIST_WORKFLOW_RUNS,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                let cmd: ListRunsReq = serde_json::from_value(data)
                    .map_err(|e| format!("listworkflowruns: {e}"))?;
                let list = wstore
                    .workflow_runs_for(&cmd.workflow_id, cmd.limit)
                    .map_err(|e| format!("listworkflowruns: {e}"))?;
                Ok(Some(serde_json::to_value(&list).unwrap_or_default()))
            })
        }),
    );
}

fn now_ms() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}
