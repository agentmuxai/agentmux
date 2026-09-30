// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! HTTP adapters over app_api: memory, global memory, fleet stop and stop forwarding.
//! Split out of server/mod.rs unchanged (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4.1).

use super::*;

/// Classify an app-API impl error string into an HTTP status. FORBIDDEN and
/// argument/not-found errors are the caller's fault (4xx); the rest are 5xx.
pub(super) fn app_api_error_status(e: &str) -> StatusCode {
    if e.starts_with("FORBIDDEN") {
        StatusCode::FORBIDDEN
    } else if e.contains("not found")
        || e.contains("provide ")
        || e.contains("not a regular file")
        || e.contains("too large")
        || e.contains("invalid")
    {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}

pub(super) fn app_api_response(result: Result<serde_json::Value, String>) -> axum::response::Response {
    match result {
        Ok(v) => (StatusCode::OK, Json(v)).into_response(),
        Err(e) => (app_api_error_status(&e), Json(json!({ "error": e }))).into_response(),
    }
}

#[derive(serde::Deserialize)]
pub(super) struct AgentMemoryListQuery {
    pub(super) agent_id: String,
}

/// `GET /api/v1/agent/memory/list?agent_id=<slug>` — list the agent's own
/// native-memory markdown files. Backs the `MemoryList` MCP tool.
pub(super) async fn handle_agent_memory_list(
    State(state): State<AppState>,
    caller: Option<Extension<caller::Caller>>,
    Query(q): Query<AgentMemoryListQuery>,
) -> impl IntoResponse {
    actor::check_actor(
        &state,
        caller.as_deref(),
        actor::ActorSite::MemoryList,
        Some(&q.agent_id),
    );
    app_api_response(app_api::memory_list_impl(&state, app_api::SelfOwner::of(caller.as_deref(), &q.agent_id, "m4c.memory_owner_by_name")))
}

#[derive(serde::Deserialize)]
pub(super) struct AgentMemoryReadQuery {
    pub(super) agent_id: String,
    pub(super) filename: String,
}

/// `GET /api/v1/agent/memory/read?agent_id=<slug>&filename=<f>` — read one of
/// the agent's own memory files. Backs the `MemoryRead` MCP tool.
pub(super) async fn handle_agent_memory_read(
    State(state): State<AppState>,
    caller: Option<Extension<caller::Caller>>,
    Query(q): Query<AgentMemoryReadQuery>,
) -> impl IntoResponse {
    actor::check_actor(
        &state,
        caller.as_deref(),
        actor::ActorSite::MemoryRead,
        Some(&q.agent_id),
    );
    app_api_response(app_api::memory_read_impl(&state, app_api::SelfOwner::of(caller.as_deref(), &q.agent_id, "m4c.memory_owner_by_name"), &q.filename))
}

#[derive(serde::Deserialize)]
pub(super) struct AgentMemoryWriteProvenanceReq {
    pub(super) source: String,
    // reagent P2 on PR #2674 (re-review): plain #[serde(default)] on a bare
    // serde_json::Value yields Value::Null when the caller supplies `source`
    // but omits `detail` — `.to_string()` on that is the literal string
    // "null", not the "{}" every no-provenance write path uses. Same bug
    // class already fixed once in rpc_types/memory.rs's sibling
    // NativeMemoryWriteProvenance (its own `default_detail()`), just
    // recurring here in this HTTP/App-API request struct.
    #[serde(default = "default_agent_memory_write_detail")]
    pub(super) detail: serde_json::Value,
}

pub(super) fn default_agent_memory_write_detail() -> serde_json::Value {
    serde_json::json!({})
}

#[derive(serde::Deserialize)]
pub(super) struct AgentMemoryWriteRequest {
    pub(super) agent_id: String,
    pub(super) filename: String,
    pub(super) content: String,
    #[serde(default)]
    pub(super) provenance: Option<AgentMemoryWriteProvenanceReq>,
}

/// `POST /api/v1/agent/memory/write` — create/overwrite one of the agent's own
/// memory files (atomic tmp→rename). Backs the `MemoryWrite` MCP tool — the
/// primary write path an agent actually uses, so `provenance` (optional,
/// see docs/specs/SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md
/// §4.1) threads through to the version history here, not just on the
/// WebSocket RPC's `agent:memory:write_file`.
pub(super) async fn handle_agent_memory_write(
    State(state): State<AppState>,
    caller: Option<Extension<caller::Caller>>,
    Json(req): Json<AgentMemoryWriteRequest>,
) -> impl IntoResponse {
    actor::check_actor(
        &state,
        caller.as_deref(),
        actor::ActorSite::MemoryWrite,
        Some(&req.agent_id),
    );
    let mut detail_str = String::new();
    let provenance = if let Some(p) = req.provenance.as_ref() {
        detail_str = p.detail.to_string();
        Some(app_api::MemoryWriteProvenance { source: &p.source, detail: &detail_str })
    } else {
        None
    };
    match app_api::memory_write_impl(&state, app_api::SelfOwner::of(caller.as_deref(), &req.agent_id, "m4c.memory_owner_by_name"), &req.filename, &req.content, provenance) {
        Ok(()) => (StatusCode::OK, Json(json!({ "ok": true }))).into_response(),
        Err(e) => (app_api_error_status(&e), Json(json!({ "error": e }))).into_response(),
    }
}

/// `POST /api/v1/fleet/bulk-stop` — stop many agent panes (block ids) at
/// once. Backs the `FleetBulkStop` MCP tool. Infallible per-target (never
/// a single bool) — see `FleetActionResult`'s doc comment.
///
/// Agents' stops wait for each target's user override (§6.5 of
/// SPEC_AGENT_SELF_QUIT_2026_09_24.md): 202 with a request per target, or 200
/// as before when nothing had to wait. `auth` (optional, signed by
/// agentmux-mcp) names the caller on the banner and in the audit log.
pub(super) async fn handle_fleet_bulk_stop(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    let req: crate::backend::rpc_types::CommandFleetBulkStopData = match serde_json::from_value(body.clone()) {
        Ok(r) => r,
        Err(e) => return (StatusCode::UNPROCESSABLE_ENTITY, Json(json!({ "error": e.to_string() }))).into_response(),
    };
    let by = body
        .get("auth")
        .and_then(|a| serde_json::from_value::<agentmux_common::api_types::UiAutomationAuth>(a.clone()).ok())
        .filter(|a| ui_handlers::verified_block_id(&state, caller.as_deref(), a).is_ok())
        .map(|a| a.agent_id)
        .unwrap_or_else(|| "an agent".to_string());
    let (result, pending) =
        app_api::fleet::fleet_bulk_stop_with_override(&state, &by, req.targets, req.signal.as_deref(), req.staged).await;
    if pending.is_empty() {
        return (StatusCode::OK, Json(result)).into_response();
    }
    (
        StatusCode::ACCEPTED,
        Json(json!({
            "status": "pending_user_override",
            "wait_at_least_ms": crate::sagas::pending_shutdown::OVERRIDE_WINDOW.as_millis() as u64,
            "deadline_ms": pending.iter().map(|p| p.deadline_ms).max(),
            "pending": pending.iter().map(|p| json!({ "block_id": p.block_id, "request_id": p.request_id })).collect::<Vec<_>>(),
            "succeeded": result.succeeded,
            "failed": result.failed,
            "aborted_early": result.aborted_early,
        })),
    )
        .into_response()
}

#[derive(serde::Deserialize)]
pub(super) struct AgentStopForwardRequest {
    pub(super) block_id: String,
    pub(super) signal: Option<String>,
}

#[derive(serde::Deserialize)]
pub(super) struct AgentStopPendingForwardRequest {
    pub(super) block_id: String,
    pub(super) signal: Option<String>,
    /// The asking agent, as the forwarding instance verified it. Shown on the
    /// banner; this instance takes it as claimed.
    #[serde(default)]
    pub(super) by: String,
}

/// `POST /agentmux/agent/stop-pending` — another instance on this machine
/// forwards an agent's `FleetBulkStop` for a block running HERE: open this
/// instance's own override window (banner, chime, Keep running) rather than
/// stopping at once. The caller polls `/api/v1/agent/shutdown/{request_id}`
/// here. Same trust model and gate as `/agentmux/agent/stop`.
pub(super) async fn handle_agent_stop_pending_forward(
    State(state): State<AppState>,
    Json(req): Json<AgentStopPendingForwardRequest>,
) -> impl IntoResponse {
    if crate::backend::blockcontroller::get_controller(&req.block_id).is_none() {
        let error = format!("NOT_RUNNING: no controller for block {}", req.block_id);
        return (StatusCode::OK, Json(json!({ "success": false, "error": error }))).into_response();
    }
    let by = if req.by.trim().is_empty() { "an agent on another AgentMux" } else { req.by.trim() };
    let v = crate::sagas::pending_shutdown::request(
        &state,
        &req.block_id,
        by,
        "FleetBulkStop",
        "",
        crate::sagas::pending_shutdown::Action::Stop { signal: req.signal },
    );
    state.reactive_handler.log_fleet_action_audit(
        Some(by),
        &v.target,
        &req.block_id,
        "fleet.bulk-stop",
        true,
        None,
        &v.request_id,
        Some(&format!("{} (forwarded from another channel)", crate::sagas::pending_shutdown::audit_note(&v, by, "FleetBulkStop"))),
    );
    let mut pending = serde_json::to_value(&v).unwrap_or_default();
    pending["joined"] = json!(v.joined);
    (StatusCode::OK, Json(json!({ "success": true, "pending": pending }))).into_response()
}

/// `POST /agentmux/agent/stop` — the cross-channel bulk-stop forward
/// target. See `fleet_bulk_stop_impl`'s doc comment for the caller side;
/// this is just `stop_one_agent_block` run against THIS instance's own
/// (in-process) controller registry, for a target the CALLING channel
/// couldn't find in its own — same one-hop-forward shape as
/// `/agentmux/reactive/inject`'s cross-channel tier, no further forwarding.
pub(super) async fn handle_agent_stop_forward(
    State(_state): State<AppState>,
    Json(req): Json<AgentStopForwardRequest>,
) -> impl IntoResponse {
    match app_api::agent_io::stop_one_agent_block(&req.block_id, req.signal.as_deref()) {
        Ok(result) => (StatusCode::OK, Json(serde_json::json!({ "success": true, "result": result }))).into_response(),
        Err(e) => (StatusCode::OK, Json(serde_json::json!({ "success": false, "error": e }))).into_response(),
    }
}

#[derive(serde::Deserialize)]
pub(super) struct AgentMemoryHistoryQuery {
    pub(super) agent_id: String,
    pub(super) filename: String,
}

/// `GET /api/v1/agent/memory/history?agent_id=<slug>&filename=<f>` — list
/// every recorded version of one memory file, newest first. Backs the
/// `MemoryHistory` MCP tool.
pub(super) async fn handle_agent_memory_history(
    State(state): State<AppState>,
    caller: Option<Extension<caller::Caller>>,
    Query(q): Query<AgentMemoryHistoryQuery>,
) -> impl IntoResponse {
    actor::check_actor(
        &state,
        caller.as_deref(),
        actor::ActorSite::MemoryHistory,
        Some(&q.agent_id),
    );
    app_api_response(app_api::memory_history_impl(&state, app_api::SelfOwner::of(caller.as_deref(), &q.agent_id, "m4c.memory_owner_by_name"), &q.filename))
}

#[derive(serde::Deserialize)]
pub(super) struct AgentMemoryDiffQuery {
    pub(super) agent_id: String,
    pub(super) from_version_id: String,
    pub(super) to_version_id: String,
}

/// `GET /api/v1/agent/memory/diff?agent_id=<slug>&from_version_id=&to_version_id=`
/// — a line-based diff between two recorded versions, both of which must
/// belong to `agent_id` (reagent P1 — see `memory_diff_impl`'s own doc for
/// why). Backs the `MemoryDiff` MCP tool.
pub(super) async fn handle_agent_memory_diff(
    State(state): State<AppState>,
    caller: Option<Extension<caller::Caller>>,
    Query(q): Query<AgentMemoryDiffQuery>,
) -> impl IntoResponse {
    actor::check_actor(
        &state,
        caller.as_deref(),
        actor::ActorSite::MemoryDiff,
        Some(&q.agent_id),
    );
    app_api_response(app_api::memory_diff_impl(&state, app_api::SelfOwner::of(caller.as_deref(), &q.agent_id, "m4c.memory_owner_by_name"), &q.from_version_id, &q.to_version_id))
}

#[derive(serde::Deserialize)]
pub(super) struct AgentMemoryRevertRequest {
    pub(super) agent_id: String,
    pub(super) filename: String,
    pub(super) target_version_id: String,
}

/// `POST /api/v1/agent/memory/revert` — restore a memory file's live
/// content to a prior version, recorded as a NEW version (`source:
/// "revert"`) — never rewrites or deletes history. Backs the
/// `MemoryRevert` MCP tool.
pub(super) async fn handle_agent_memory_revert(
    State(state): State<AppState>,
    caller: Option<Extension<caller::Caller>>,
    Json(req): Json<AgentMemoryRevertRequest>,
) -> impl IntoResponse {
    actor::check_actor(
        &state,
        caller.as_deref(),
        actor::ActorSite::MemoryRevert,
        Some(&req.agent_id),
    );
    app_api_response(app_api::memory_revert_impl(&state, app_api::SelfOwner::of(caller.as_deref(), &req.agent_id, "m4c.memory_owner_by_name"), &req.filename, &req.target_version_id))
}

#[derive(serde::Deserialize)]
pub(super) struct AgentGlobalMemoryWriteProvenanceReq {
    pub(super) source: String,
    // Same "default() on a bare Value yields Null, not {}" trap as
    // AgentMemoryWriteProvenanceReq above — see that struct's own comment.
    #[serde(default = "default_agent_globalmemory_write_detail")]
    pub(super) detail: serde_json::Value,
}

pub(super) fn default_agent_globalmemory_write_detail() -> serde_json::Value {
    serde_json::json!({})
}

#[derive(serde::Deserialize)]
pub(super) struct AgentGlobalMemoryWriteRequest {
    pub(super) agent_id: String,
    #[serde(default)]
    pub(super) id: Option<String>,
    pub(super) name: String,
    pub(super) content: String,
    #[serde(default)]
    pub(super) provenance: Option<AgentGlobalMemoryWriteProvenanceReq>,
}

/// `POST /api/v1/agent/globalmemory/write` — create a new ordinary Global
/// Memory entry, or update an existing one by `id`. Never reaches a
/// system-tier row either way — see `global_memory_write_impl`'s own doc
/// comment for the full invariant. Backs the `GlobalMemoryWrite` MCP tool.
pub(super) async fn handle_agent_globalmemory_write(
    State(state): State<AppState>,
    caller: Option<Extension<caller::Caller>>,
    Json(req): Json<AgentGlobalMemoryWriteRequest>,
) -> impl IntoResponse {
    actor::check_actor(
        &state,
        caller.as_deref(),
        actor::ActorSite::GlobalMemoryWrite,
        Some(&req.agent_id),
    );
    let mut detail_str = String::new();
    let provenance = if let Some(p) = req.provenance.as_ref() {
        detail_str = p.detail.to_string();
        Some(app_api::GlobalMemoryWriteProvenance { source: &p.source, detail: &detail_str })
    } else {
        None
    };
    // Identity M4c-1 (§6.5.9): the writer's UID is the request's `Caller` —
    // its token — never a body field; `""` when Unattributed.
    let written_by_uid = caller::attributed_uid(caller.as_deref());
    app_api_response(app_api::global_memory_write_impl(
        &state,
        &req.agent_id,
        &written_by_uid,
        req.id.as_deref(),
        &req.name,
        &req.content,
        provenance,
    ))
}

/// `GET /api/v1/agent/globalmemory/list` — ordinary (non-system) Global
/// Memory entries, summary only (id/name/updated_at, no content). Backs the
/// `GlobalMemoryList` MCP tool.
pub(super) async fn handle_agent_globalmemory_list(State(state): State<AppState>) -> impl IntoResponse {
    app_api_response(app_api::global_memory_list_impl(&state))
}

#[derive(serde::Deserialize)]
pub(super) struct AgentGlobalMemoryReadQuery {
    pub(super) id: String,
}

/// `GET /api/v1/agent/globalmemory/read?id=<id>` — full content of one
/// ordinary Global Memory entry. Backs the `GlobalMemoryRead` MCP tool.
pub(super) async fn handle_agent_globalmemory_read(
    State(state): State<AppState>,
    Query(q): Query<AgentGlobalMemoryReadQuery>,
) -> impl IntoResponse {
    app_api_response(app_api::global_memory_read_impl(&state, &q.id))
}

#[derive(serde::Deserialize)]
pub(super) struct AgentGlobalMemoryRemoveRequest {
    pub(super) id: String,
}

/// `POST /api/v1/agent/globalmemory/remove` — demote a Global Memory entry
/// (clears `is_global`; the bundle row itself survives, same as the Armory
/// UI's own "Remove" button). Backs the `GlobalMemoryRemove` MCP tool.
pub(super) async fn handle_agent_globalmemory_remove(
    State(state): State<AppState>,
    Json(req): Json<AgentGlobalMemoryRemoveRequest>,
) -> impl IntoResponse {
    app_api_response(app_api::global_memory_remove_impl(&state, &req.id))
}

#[derive(serde::Deserialize)]
pub(super) struct AgentGlobalMemoryHistoryQuery {
    pub(super) id: String,
}

/// `GET /api/v1/agent/globalmemory/history?id=<id>` — list every recorded
/// version of one Global Memory entry, newest first. Refuses a system-tier,
/// blank, or non-global id the same way `read`/`remove` already do — see
/// `global_memory_history_impl`'s own doc comment. Backs the
/// `GlobalMemoryHistory` MCP tool.
pub(super) async fn handle_agent_globalmemory_history(
    State(state): State<AppState>,
    Query(q): Query<AgentGlobalMemoryHistoryQuery>,
) -> impl IntoResponse {
    app_api_response(app_api::global_memory_history_impl(&state, &q.id))
}

#[derive(serde::Deserialize)]
pub(super) struct AgentGlobalMemoryDiffQuery {
    pub(super) id: String,
    pub(super) from_version_id: String,
    pub(super) to_version_id: String,
}

/// `GET /api/v1/agent/globalmemory/diff?id=<id>&from_version_id=&to_version_id=`
/// — a line-based diff between two recorded versions, both of which must
/// belong to `id` (same ownership check as `/api/v1/agent/memory/diff`'s
/// `agent_id` scoping — see `global_memory_diff_impl`'s own doc comment for
/// why). Backs the `GlobalMemoryDiff` MCP tool.
pub(super) async fn handle_agent_globalmemory_diff(
    State(state): State<AppState>,
    Query(q): Query<AgentGlobalMemoryDiffQuery>,
) -> impl IntoResponse {
    app_api_response(app_api::global_memory_diff_impl(&state, &q.id, &q.from_version_id, &q.to_version_id))
}

#[derive(serde::Deserialize)]
pub(super) struct AgentGlobalMemoryRevertRequest {
    pub(super) agent_id: String,
    pub(super) id: String,
    pub(super) version_id: String,
}

/// `POST /api/v1/agent/globalmemory/revert` — restore a Global Memory
/// entry's live content to a prior recorded version, recorded as a NEW
/// version (`source: "revert"`) — never rewrites or deletes history, same
/// as `/api/v1/agent/memory/revert`. Never reaches a system-tier row — see
/// `global_memory_revert_impl`'s own doc comment. Backs the
/// `GlobalMemoryRevert` MCP tool.
pub(super) async fn handle_agent_globalmemory_revert(
    State(state): State<AppState>,
    caller: Option<Extension<caller::Caller>>,
    Json(req): Json<AgentGlobalMemoryRevertRequest>,
) -> impl IntoResponse {
    actor::check_actor(
        &state,
        caller.as_deref(),
        actor::ActorSite::GlobalMemoryRevert,
        Some(&req.agent_id),
    );
    // Identity M4c-1: as for write, the writer's UID is the `Caller`'s.
    let written_by_uid = caller::attributed_uid(caller.as_deref());
    app_api_response(app_api::global_memory_revert_impl(
        &state,
        &req.agent_id,
        &written_by_uid,
        &req.id,
        &req.version_id,
    ))
}
