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
pub async fn handle_muxspect_list(State(state): State<AppState>) -> impl IntoResponse {
    let blocks = state.process_broker.list();
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
    // the process-tracker registry once for both `processes` and
    // `liveness_confidence` — a second, independent read here could race a
    // process starting/exiting between the two calls and return two
    // contradictory snapshots in one response (codex P2 on PR #2380).
    // Derive everything from this one snapshot instead of reading twice.
    let process_status = state.process_broker.status(&q.block_id);
    let controller_status = crate::backend::blockcontroller::get_block_controller_status(&q.block_id);

    Json(json!({
        "block_id": q.block_id,
        "process_status": &process_status,
        "controller_status": controller_status,
        "processes": &process_status.processes,
        "tracking_confidence": process_status.liveness_confidence,
    }))
    .into_response()
}
