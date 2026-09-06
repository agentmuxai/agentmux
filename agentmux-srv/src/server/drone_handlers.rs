// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! WSH RPC handlers for the Drone pane (issue #753 Phase 1).
//!
//! Commands:
//!   * `listdrones`       → `Vec<DroneDefinition>`
//!   * `getdrone`         → `Option<DroneDefinition>`
//!   * `upsertdrone`      → `DroneDefinition` (echoed back, normalized)
//!   * `deletedrone`      → `{ deleted: bool }`
//!   * `rundrone`         → `{ run_id: String }` (synchronous; SSE
//!                              streaming added in Phase 1 PR-4)
//!   * `listdroneruns`    → `Vec<DroneRun>`
//!
//! Run streaming: the executor emits `RunEvent`s over an mpsc channel.
//! Phase 1 of this PR drains the channel server-side and stores the
//! final block-state snapshot in `db_drone_runs`. A future commit
//! will tee the channel to the renderer via the existing `wps` event
//! broker so `RunPanel` shows live per-block status.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::backend::rpc_types::{
    COMMAND_DELETE_DRONE, COMMAND_GET_DRONE, COMMAND_LIST_DRONES,
    COMMAND_LIST_DRONE_RUNS, COMMAND_RUN_DRONE, COMMAND_UPSERT_DRONE,
};
use crate::backend::wps::WaveEvent;
use crate::server::AppState;
use crate::backend::rpc::engine::WshRpcEngine;
use crate::drone::executor::{run_drone, RunEvent};
use crate::drone::storage::DroneStore;
use crate::drone::types::{RunStatus, DroneDefinition, DroneRun};

#[derive(Debug, Deserialize)]
struct GetDroneReq {
    id: String,
}

#[derive(Debug, Deserialize)]
struct DeleteDroneReq {
    id: String,
}

#[derive(Debug, Deserialize)]
struct RunDroneReq {
    drone_id: String,
}

#[derive(Debug, Deserialize)]
struct ListRunsReq {
    drone_id: String,
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

pub fn register_drone_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // Drone definition CRUD — routed to id_store (global shared store).
    let id_store = state.id_store.clone();
    engine.register_handler(
        COMMAND_LIST_DRONES,
        Box::new(move |_data, _ctx| {
            let id_store = id_store.clone();
            Box::pin(async move {
                let list = id_store
                    .drone_list()
                    .map_err(|e| format!("listdrones: {e}"))?;
                Ok(Some(serde_json::to_value(&list).unwrap_or_default()))
            })
        }),
    );

    let id_store = state.id_store.clone();
    engine.register_handler(
        COMMAND_GET_DRONE,
        Box::new(move |data, _ctx| {
            let id_store = id_store.clone();
            Box::pin(async move {
                let cmd: GetDroneReq = serde_json::from_value(data)
                    .map_err(|e| format!("getdrone: {e}"))?;
                let row = id_store
                    .drone_get(&cmd.id)
                    .map_err(|e| format!("getdrone: {e}"))?;
                Ok(Some(serde_json::to_value(&row).unwrap_or_default()))
            })
        }),
    );

    let id_store = state.id_store.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_UPSERT_DRONE,
        Box::new(move |data, _ctx| {
            let id_store = id_store.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let mut cmd: DroneDefinition = serde_json::from_value(data)
                    .map_err(|e| format!("upsertdrone: {e}"))?;
                let now = now_ms();
                if cmd.created_at == 0 {
                    cmd.created_at = now;
                }
                cmd.updated_at = now;
                if cmd.id.is_empty() {
                    cmd.id = uuid::Uuid::new_v4().to_string();
                }
                id_store
                    .drone_upsert(&cmd)
                    .map_err(|e| format!("upsertdrone: {e}"))?;
                broker.publish(WaveEvent {
                    event: "drones:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(Some(serde_json::to_value(&cmd).unwrap_or_default()))
            })
        }),
    );

    let id_store = state.id_store.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_DELETE_DRONE,
        Box::new(move |data, _ctx| {
            let id_store = id_store.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let cmd: DeleteDroneReq = serde_json::from_value(data)
                    .map_err(|e| format!("deletedrone: {e}"))?;
                let deleted = id_store
                    .drone_delete(&cmd.id)
                    .map_err(|e| format!("deletedrone: {e}"))?;
                if deleted {
                    broker.publish(WaveEvent {
                        event: "drones:changed".to_string(),
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

    // COMMAND_RUN_DRONE: reads drone definition from id_store (global),
    // writes drone run rows to wstore (per-channel).
    let id_store = state.id_store.clone();
    let wstore = state.wstore.clone();
    let broker = state.broker.clone();
    engine.register_handler(
        COMMAND_RUN_DRONE,
        Box::new(move |data, _ctx| {
            let id_store = id_store.clone();
            let wstore = wstore.clone();
            let broker = broker.clone();
            Box::pin(async move {
                let cmd: RunDroneReq = serde_json::from_value(data)
                    .map_err(|e| format!("rundrone: {e}"))?;
                let wf = id_store
                    .drone_get(&cmd.drone_id)
                    .map_err(|e| format!("rundrone: {e}"))?
                    .ok_or_else(|| {
                        format!("rundrone: drone {} not found", cmd.drone_id)
                    })?;
                let started_at = now_ms();
                let mut handle = run_drone(wf.id.clone(), wf.graph.clone())
                    .await
                    .map_err(|e| format!("rundrone: {e}"))?;
                let run_id = handle.run_id.clone();
                let drone_id = wf.id.clone();

                // Persist a `running` placeholder row synchronously
                // BEFORE returning. Guarantees:
                //   1. The frontend's `refreshRuns` (called right
                //      after this RPC resolves) always sees the row,
                //      no spawn-race against the drain (codex+reagent
                //      P2 from earlier rounds).
                //   2. The drain can run as a background spawn
                //      again — so drones longer than the RPC
                //      timeout (5s default) don't get their drain
                //      truncated mid-flight (codex P1 v0.33.842).
                //   3. Server-restart safety: an orphaned `running`
                //      row signals "drain was interrupted" — a
                //      future startup task can mark stale rows as
                //      `interrupted` (cleaner than the prior
                //      fire-and-forget that left no row at all).
                let placeholder = DroneRun {
                    id: run_id.clone(),
                    drone_id: drone_id.clone(),
                    status: RunStatus::Running.as_str().to_string(),
                    started_at,
                    ended_at: 0,
                    block_states: HashMap::new(),
                    output: String::new(),
                    error: String::new(),
                };
                wstore
                    .drone_run_insert(&placeholder)
                    .map_err(|e| format!("rundrone placeholder: {e}"))?;

                // Drain on a background task; on completion, UPDATE
                // the placeholder row in place.
                let wstore_for_drain = wstore.clone();
                let broker_for_drain = broker.clone();
                let run_id_for_drain = run_id.clone();
                let drone_id_for_drain = drone_id.clone();
                tokio::spawn(async move {
                    let mut last_status;
                    let mut output = String::new();
                    let mut error = String::new();
                    while let Some(ev) = handle.events.recv().await {
                        // For terminal events, persist the final row
                        // BEFORE publishing the event. The frontend
                        // subscription path refreshes the runs list on
                        // RunDone / RunFailed, and if the publish
                        // happens first the refresh sees a stale
                        // `running` row (codex P2 on PR #843).
                        let is_terminal = matches!(
                            ev,
                            RunEvent::RunDone { .. } | RunEvent::RunFailed { .. }
                        );
                        if is_terminal {
                            match &ev {
                                RunEvent::RunDone { output: o, .. } => {
                                    last_status = RunStatus::Done;
                                    // Engine already unwraps Response's
                                    // `{ "value": ... }` wrapper.
                                    output = match o {
                                        serde_json::Value::String(s) => s.clone(),
                                        other => {
                                            serde_json::to_string(other).unwrap_or_default()
                                        }
                                    };
                                }
                                RunEvent::RunFailed { error: e, .. } => {
                                    last_status = RunStatus::Failed;
                                    error = e.clone();
                                }
                                _ => unreachable!(),
                            }
                            let states = handle.final_states.lock().await.clone();
                            let row = DroneRun {
                                id: run_id_for_drain.clone(),
                                drone_id: drone_id_for_drain.clone(),
                                status: last_status.as_str().to_string(),
                                started_at,
                                ended_at: now_ms(),
                                block_states: states,
                                output: output.clone(),
                                error: error.clone(),
                            };
                            match wstore_for_drain.drone_run_update(&row) {
                                Ok(0) => tracing::warn!(
                                    run_id = %run_id_for_drain,
                                    "drone_run_update: placeholder row missing (race?)"
                                ),
                                Ok(_) => {}
                                Err(e) => tracing::warn!(
                                    run_id = %run_id_for_drain,
                                    error = %e,
                                    "drone_run_update failed"
                                ),
                            }
                        }
                        broker_for_drain.publish(WaveEvent {
                            event: format!("dronerun:{}", run_id_for_drain),
                            scopes: vec![],
                            sender: String::new(),
                            persist: 0,
                            data: Some(serde_json::to_value(&ev).unwrap_or_default()),
                        });
                    }
                });

                Ok(Some(serde_json::to_value(&RunResp { run_id }).unwrap_or_default()))
            })
        }),
    );

    let wstore = state.wstore.clone();
    engine.register_handler(
        COMMAND_LIST_DRONE_RUNS,
        Box::new(move |data, _ctx| {
            let wstore = wstore.clone();
            Box::pin(async move {
                let cmd: ListRunsReq = serde_json::from_value(data)
                    .map_err(|e| format!("listdroneruns: {e}"))?;
                let limit = cmd.limit.clamp(0, MAX_LIST_LIMIT);
                let list = wstore
                    .drone_runs_for(&cmd.drone_id, limit)
                    .map_err(|e| format!("listdroneruns: {e}"))?;
                Ok(Some(serde_json::to_value(&list).unwrap_or_default()))
            })
        }),
    );
}

fn now_ms() -> i64 {
    agentmux_common::time::now_ms()
}
