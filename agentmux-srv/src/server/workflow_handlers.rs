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

/// Hard cap on `ListRunsReq.limit` — guards against a malicious or
/// buggy client passing e.g. `i64::MAX` and pulling the entire run
/// history (DoS / memory blow-up). 200 covers any plausible UI page
/// size with headroom; Phase 2 pagination cursor work will move this
/// to client-driven slicing. (kimi P1 on PR #755.)
const MAX_LIST_LIMIT: i64 = 200;

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

                // Drain the event channel inline (vs. tokio::spawn) so
                // the run record is persisted before this RPC resolves.
                // Reasons:
                //   1. A fire-and-forget spawn could be dropped if the
                //      server restarts between RPC return and task
                //      completion — the frontend would hold a run_id
                //      that has no row (kimi P1 on PR #755).
                //   2. The frontend's `refreshRuns` (called right
                //      after the RPC resolves to update the UI list)
                //      races the spawn and shows stale data without
                //      seeing the new row (codex + reagent P2).
                // Phase 1 has no live `workflowrun:<id>` subscription
                // yet, so an awaited drain is correct for Phase 1.
                // Phase 1 PR-4 polish reintroduces streaming once the
                // frontend subscribes — the row will then be written
                // by a spawn AND a running placeholder will be
                // inserted up-front so the UI never lags the truth.
                let mut last_status = RunStatus::Running;
                let mut output = String::new();
                let mut error = String::new();
                while let Some(ev) = handle.events.recv().await {
                    broker.publish(WaveEvent {
                        event: format!("workflowrun:{}", run_id),
                        scopes: vec![],
                        sender: String::new(),
                        persist: 0,
                        data: Some(serde_json::to_value(&ev).unwrap_or_default()),
                    });
                    match &ev {
                        RunEvent::RunDone { output: o, .. } => {
                            last_status = RunStatus::Done;
                            // The engine already unwraps Response's
                            // `{ "value": ... }` wrapper, so `o` is
                            // the bare value (string for the common
                            // case). Coerce to a single column-friendly
                            // string for the run-record.
                            output = match o {
                                serde_json::Value::String(s) => s.clone(),
                                other => serde_json::to_string(other).unwrap_or_default(),
                            };
                        }
                        RunEvent::RunFailed { error: e, .. } => {
                            last_status = RunStatus::Failed;
                            error = e.clone();
                        }
                        _ => {}
                    }
                }
                let states = handle.final_states.lock().await.clone();
                let row = WorkflowRun {
                    id: run_id.clone(),
                    workflow_id: workflow_id.clone(),
                    status: last_status.as_str().to_string(),
                    started_at,
                    ended_at: now_ms(),
                    block_states: states,
                    output,
                    error,
                };
                if let Err(e) = wstore.workflow_run_insert(&row) {
                    tracing::warn!(run_id = %run_id, error = %e, "workflow_run_insert failed");
                }

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
                let limit = cmd.limit.clamp(0, MAX_LIST_LIMIT);
                let list = wstore
                    .workflow_runs_for(&cmd.workflow_id, limit)
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
