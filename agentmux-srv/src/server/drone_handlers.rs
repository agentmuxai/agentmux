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
//! will tee the channel to the renderer via the existing `mps` event
//! broker so `RunPanel` shows live per-block status.

use std::collections::HashMap;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::backend::rpc_types::{
    COMMAND_DELETE_DRONE, COMMAND_GET_DRONE, COMMAND_LIST_DRONES,
    COMMAND_LIST_DRONE_RUNS, COMMAND_RUN_DRONE, COMMAND_UPSERT_DRONE,
};
use crate::backend::mps::MuxEvent;
use crate::server::AppState;
use crate::backend::rpc::engine::WshRpcEngine;
use crate::drone::executor::{run_drone, RunEvent};
use crate::drone::storage::DroneStore;
use crate::drone::types::{RunStatus, DroneDefinition, DroneRun};

/// Request for `listdrones`, which ignores its payload.
///
/// Registered as `Option<Self>`: the stub sends `{}` and a client that omits
/// `data` sends `null`, and serde accepts each of those from only one of
/// `()` and a struct. `Option<Self>` takes both.
#[derive(Debug, Default, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct ListDronesReq {}

#[derive(Debug, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct GetDroneReq {
    pub id: String,
}

#[derive(Debug, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct DeleteDroneReq {
    pub id: String,
}

#[derive(Debug, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct RunDroneReq {
    pub drone_id: String,
}

#[derive(Debug, Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct ListRunsReq {
    pub drone_id: String,
    /// `serde(default)`, so a caller may omit it -- which ts-rs cannot express
    /// on a non-`Option` field. The stub derives `ListDroneRunsInput` from the
    /// generated type to restore that; see the comment there.
    #[serde(default = "default_limit")]
    #[ts(type = "number")]
    pub limit: i64,
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

#[derive(Debug, Serialize, Deserialize, ts_rs::TS)]
#[ts(rename = "DeleteDroneResp")]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct DeleteResp {
    pub deleted: bool,
}

#[derive(Debug, Serialize, Deserialize, ts_rs::TS)]
#[ts(rename = "RunDroneResp")]
#[ts(export, export_to = "../../frontend/types/rpc/")]
pub struct RunResp {
    pub run_id: String,
}

pub fn register_drone_handlers(engine: &Arc<WshRpcEngine>, state: &AppState) {
    // Drone definition CRUD — routed to id_store (global shared store).
    let id_store = state.id_store.clone();
    engine.register_typed(
        COMMAND_LIST_DRONES,
        // `Option<_>` -- see `ListDronesReq`: it is what makes both encodings
        // of "no argument" deserialize.
        move |_req: Option<ListDronesReq>, _ctx| {
            let id_store = id_store.clone();
            async move {
                let list = id_store
                    .drone_list()
                    .map_err(|e| format!("listdrones: {e}"))?;
                Ok(list)
            }
        },
    );

    let id_store = state.id_store.clone();
    engine.register_typed(
        COMMAND_GET_DRONE,
        move |cmd: GetDroneReq, _ctx| {
            let id_store = id_store.clone();
            async move {
                let row = id_store
                    .drone_get(&cmd.id)
                    .map_err(|e| format!("getdrone: {e}"))?;
                Ok(row)
            }
        },
    );

    let id_store = state.id_store.clone();
    let broker = state.broker.clone();
    engine.register_typed(
        COMMAND_UPSERT_DRONE,
        move |mut cmd: DroneDefinition, _ctx| {
            let id_store = id_store.clone();
            let broker = broker.clone();
            async move {
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
                broker.publish(MuxEvent {
                    event: "drones:changed".to_string(),
                    scopes: vec![],
                    sender: String::new(),
                    persist: 0,
                    data: None,
                });
                Ok(cmd)
            }
        },
    );

    let id_store = state.id_store.clone();
    let broker = state.broker.clone();
    engine.register_typed(
        COMMAND_DELETE_DRONE,
        move |cmd: DeleteDroneReq, _ctx| {
            let id_store = id_store.clone();
            let broker = broker.clone();
            async move {
                let deleted = id_store
                    .drone_delete(&cmd.id)
                    .map_err(|e| format!("deletedrone: {e}"))?;
                if deleted {
                    broker.publish(MuxEvent {
                        event: "drones:changed".to_string(),
                        scopes: vec![],
                        sender: String::new(),
                        persist: 0,
                        data: None,
                    });
                }
                Ok(DeleteResp { deleted })
            }
        },
    );

    // COMMAND_RUN_DRONE: reads drone definition from id_store (global),
    // writes drone run rows to mstore (per-channel).
    let id_store = state.id_store.clone();
    let mstore = state.mstore.clone();
    let broker = state.broker.clone();
    engine.register_typed(
        COMMAND_RUN_DRONE,
        move |cmd: RunDroneReq, _ctx| {
            let id_store = id_store.clone();
            let mstore = mstore.clone();
            let broker = broker.clone();
            async move {
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
                mstore
                    .drone_run_insert(&placeholder)
                    .map_err(|e| format!("rundrone placeholder: {e}"))?;

                // Drain on a background task; on completion, UPDATE
                // the placeholder row in place.
                let mstore_for_drain = mstore.clone();
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
                            match mstore_for_drain.drone_run_update(&row) {
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
                        broker_for_drain.publish(MuxEvent {
                            event: format!("dronerun:{}", run_id_for_drain),
                            scopes: vec![],
                            sender: String::new(),
                            persist: 0,
                            data: Some(serde_json::to_value(&ev).unwrap_or_default()),
                        });
                    }
                });

                Ok(RunResp { run_id })
            }
        },
    );

    let mstore = state.mstore.clone();
    engine.register_typed(
        COMMAND_LIST_DRONE_RUNS,
        move |cmd: ListRunsReq, _ctx| {
            let mstore = mstore.clone();
            async move {
                let limit = cmd.limit.clamp(0, MAX_LIST_LIMIT);
                let list = mstore
                    .drone_runs_for(&cmd.drone_id, limit)
                    .map_err(|e| format!("listdroneruns: {e}"))?;
                Ok(list)
            }
        },
    );
}

fn now_ms() -> i64 {
    agentmux_common::time::now_ms()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every drone command records its request and response type, which is the
    /// property the generated bindings are derived from — a command that
    /// silently falls back to `register_handler` disappears from here rather
    /// than being recorded wrong, and that absence is what this catches.
    // `tokio::test`, not `test`: `test_state()` builds the fs-watch pool,
    // which spawns, so it needs a runtime.
    #[tokio::test]
    async fn register_typed_records_every_drone_command() {
        let (engine, _rx) = WshRpcEngine::new();
        let state = crate::server::tests::test_state();
        register_drone_handlers(&engine, &state);
        let schema = engine.schema_json();
        let rows = schema.as_array().unwrap();
        let find = |cmd: &str| {
            rows.iter()
                .find(|r| r["command"] == cmd)
                .unwrap_or_else(|| panic!("{cmd} missing from the schema — did it fall back to register_handler?"))
        };

        for (cmd, req, resp) in [
            (COMMAND_GET_DRONE, "GetDroneReq", "DroneDefinition"),
            (COMMAND_UPSERT_DRONE, "DroneDefinition", "DroneDefinition"),
            (COMMAND_DELETE_DRONE, "DeleteDroneReq", "DeleteResp"),
            (COMMAND_RUN_DRONE, "RunDroneReq", "RunResp"),
        ] {
            let row = find(cmd);
            assert_eq!(row["requestName"], req, "{cmd} request");
            assert!(
                row["responseName"].as_str().unwrap().contains(resp),
                "{cmd} response should mention {resp}, got {:?}",
                row["responseName"],
            );
        }

        // The two list commands answer `Vec<_>`, whose recorded name keeps its
        // full path (truncating at the last `::` would yield `DroneRun>`).
        for (cmd, elem) in [
            (COMMAND_LIST_DRONES, "DroneDefinition"),
            (COMMAND_LIST_DRONE_RUNS, "DroneRun"),
        ] {
            let got = find(cmd)["responseName"].as_str().unwrap().to_string();
            assert!(
                got.starts_with("alloc::vec::Vec<") && got.contains(elem),
                "{cmd} should answer Vec<{elem}>, got {got:?}",
            );
        }
    }

    /// `listdrones` takes no argument, and the two encodings of that both
    /// reach the server: the stub sends `{}`, a client that omits `data`
    /// sends `null`. A bare struct rejects the second, a `()` rejects the
    /// first — `Option<ListDronesReq>` is what takes both.
    #[test]
    fn listdrones_request_accepts_both_encodings_of_no_argument() {
        for payload in [serde_json::json!({}), serde_json::Value::Null] {
            serde_json::from_value::<Option<ListDronesReq>>(payload.clone())
                .unwrap_or_else(|e| panic!("{payload} should deserialize, got {e}"));
        }
        // ...and the bare struct really does reject null, so the Option is
        // load-bearing rather than decoration.
        assert!(serde_json::from_value::<ListDronesReq>(serde_json::Value::Null).is_err());
    }

    /// `limit` is `serde(default)`, which ts-rs cannot express on a non-Option
    /// field — the stub's `ListDroneRunsInput` restores the optionality by
    /// deriving from the generated type. Pin the server half: drop the default
    /// and that derived TS type becomes a lie.
    #[test]
    fn listruns_request_accepts_a_missing_limit() {
        let req: ListRunsReq =
            serde_json::from_value(serde_json::json!({ "drone_id": "d1" }))
                .expect("limit is serde(default) and may be omitted");
        assert_eq!(req.drone_id, "d1");
        assert_eq!(req.limit, default_limit(), "an omitted limit falls back to the default");
    }
}
