// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! HTTP handlers backing the `muxspect` CLI — Phase 1 of
//! `docs/specs/SPEC_MUXSPECT_LIVE_INTROSPECTION_TOOL_2026_08_01.md`.
//!
//! Diagnostic-only surface: a thin read composition over `ProcessBroker`
//! (Phase A of the process-tracking consolidation,
//! `agentmux-srv/src/broker/process.rs`) and its sibling registries — never
//! a new independent snapshot of process/turn state (spec §5.1/§3 point 8).
//! Reached the same way `agentmux-mcp` already reaches every other
//! `/api/v1/*` route: plain HTTP, `X-AuthKey` header, `$AGENTMUX_LOCAL_URL`/
//! `$AGENTMUX_AUTH_KEY` inherited from the caller's own environment (spec
//! §5.2) — no new IPC mechanism, no new auth scheme.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Json};
use serde_json::json;

use super::AppState;

/// `GET /api/v1/muxspect/list` — every controller-backed block's current
/// `ProcessStatus`, full detail (unlike `agent.tracked-blocks`, which
/// intentionally returns only `block_ids` for its Swarm-pane contract).
/// Each row also carries `is_agent` — `ProcessStatus::is_agent()`'s complete
/// classification rule (subprocess/persistent/acp are ALWAYS agents
/// regardless of `is_agent_pane`, which only applies to shell/cmd) — so
/// consumers don't reimplement that rule themselves (codex P2 on PR #2380:
/// the CLI's own naive `is_agent_pane`-only rendering mislabeled exactly
/// those three controller types).
pub async fn handle_muxspect_list(State(state): State<AppState>) -> impl IntoResponse {
    let blocks: Vec<serde_json::Value> = state
        .process_broker
        .list()
        .into_iter()
        .map(|status| {
            let is_agent = status.is_agent();
            let mut value = serde_json::to_value(&status).unwrap_or_default();
            if let Some(obj) = value.as_object_mut() {
                obj.insert("is_agent".to_string(), json!(is_agent));
            }
            value
        })
        .collect();
    Json(json!({ "blocks": blocks })).into_response()
}

#[derive(serde::Deserialize)]
pub struct MuxspectDescribeQuery {
    pub block_id: String,
}

/// `GET /api/v1/muxspect/describe?block_id=X` — composes `ProcessBroker`
/// status, the coarse `BlockControllerRuntimeStatus`, and the OS-process
/// tree for one block into a single response. This is the "describe
/// everything about block X" query
/// `REPORT_PROCESS_ARCHITECTURE_STATE_AND_RETHINK_2026_07_22.md` §5.4 named
/// as missing — getting this picture today takes 2-3 separate, uncomposed
/// RPC round-trips.
pub async fn handle_muxspect_describe(
    State(state): State<AppState>,
    Query(q): Query<MuxspectDescribeQuery>,
) -> impl IntoResponse {
    if q.block_id.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "missing block_id" })),
        )
            .into_response();
    }

    // `process_status` (via `ProcessBroker::compute_status`) already reads
    // BOTH the process-tracker registry (for `processes`/
    // `liveness_confidence`) AND the controller's `BlockControllerRuntimeStatus`
    // (carried on `process_status.controller_status`) in one pass. A second,
    // independent read of either would risk a process starting/exiting or a
    // turn starting/finishing between the two calls, returning two
    // contradictory snapshots in one response (codex P2 on PR #2380, twice —
    // once for the process list, once for the controller status). Derive
    // everything from this one snapshot instead of reading twice.
    let process_status = state.process_broker.status(&q.block_id);
    let is_agent = process_status.is_agent();

    Json(json!({
        "block_id": q.block_id,
        "process_status": &process_status,
        "is_agent": is_agent,
        "controller_status": &process_status.controller_status,
        "processes": &process_status.processes,
        "tracking_confidence": process_status.liveness_confidence,
    }))
    .into_response()
}
