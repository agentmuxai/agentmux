// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Window, tab, pane and workspace naming.
//! Split out of server/mod.rs unchanged (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4.1).

use super::*;

/// `POST /api/v1/window/name` — set a window's display name
/// (`window:displayname`), which the frontend turns into the OS/taskbar title.
/// Defaults to the caller's own window (resolved from `block_id`). Routes
/// through the same `object.UpdateObjectMeta` service path the InstancePanel
/// rename uses, so persistence + live-title update are identical.
/// agentmux-mcp's `SetWindowName` tool POSTs here.
pub(super) async fn handle_window_name(
    State(state): State<AppState>,
    Json(req): Json<WindowNameRequest>,
) -> impl IntoResponse {
    // window:displayname is documented as ≤64 chars (window-title.ts).
    let name: String = req.name.trim().chars().take(64).collect();
    if name.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "name must not be empty" }))).into_response();
    }

    let window_id = match req.window_id.filter(|w| !w.is_empty()) {
        Some(w) => w,
        None => {
            let block_id = req.block_id.unwrap_or_default();
            if block_id.is_empty() {
                return (StatusCode::BAD_REQUEST, Json(json!({ "error": "provide window_id or block_id" }))).into_response();
            }
            match service::resolve_agent_context(&state.mstore, &block_id) {
                Ok(ctx) => match ctx.window_id {
                    Some(w) => w,
                    None => {
                        return (
                            StatusCode::NOT_FOUND,
                            Json(json!({ "error": "no live window for this agent (tab not attached to a window)" })),
                        )
                            .into_response()
                    }
                },
                Err(e) => return (StatusCode::NOT_FOUND, Json(json!({ "error": e }))).into_response(),
            }
        }
    };

    let call = crate::backend::service::WebCallType {
        service: "object".to_string(),
        method: "UpdateObjectMeta".to_string(),
        uicontext: None,
        args: vec![
            json!(format!("window:{window_id}")),
            json!({ "window:displayname": name }),
        ],
    };
    finish_name_call(
        &state,
        call,
        json!({ "success": true, "window_id": window_id, "name": name }),
    )
    .await
}

/// Trim a user-supplied name and clamp to `max` chars; `None` if empty.
pub(super) fn clean_name(raw: &str, max: usize) -> Option<String> {
    let n: String = raw.trim().chars().take(max).collect();
    if n.is_empty() {
        None
    } else {
        Some(n)
    }
}

/// `POST /api/v1/tab/name` — rename a tab. Defaults to the caller's own tab
/// (resolved from `block_id`). Routes through `object.UpdateTabName`.
pub(super) async fn handle_tab_name(
    State(state): State<AppState>,
    Json(req): Json<TabNameRequest>,
) -> impl IntoResponse {
    let name = match clean_name(&req.name, 128) {
        Some(n) => n,
        None => return (StatusCode::BAD_REQUEST, Json(json!({ "error": "name must not be empty" }))).into_response(),
    };
    let tab_id = match req.tab_id.filter(|t| !t.is_empty()) {
        Some(t) => t,
        None => match resolve_own(&state, req.block_id, |c| Some(c.tab_id.clone())) {
            Ok(t) => t,
            Err(resp) => return resp,
        },
    };
    let call = crate::backend::service::WebCallType {
        service: "object".to_string(),
        method: "UpdateTabName".to_string(),
        uicontext: None,
        args: vec![json!(tab_id), json!(name)],
    };
    finish_name_call(&state, call, json!({ "success": true, "tab_id": tab_id, "name": name })).await
}

/// `POST /api/v1/pane/title` — set a pane's display title (`frame:title`).
/// Targets the caller's own pane (its `block_id`). Routes through
/// `object.UpdateObjectMeta`.
pub(super) async fn handle_pane_title(
    State(state): State<AppState>,
    Json(req): Json<PaneTitleRequest>,
) -> impl IntoResponse {
    let title = match clean_name(&req.title, 128) {
        Some(t) => t,
        None => return (StatusCode::BAD_REQUEST, Json(json!({ "error": "title must not be empty" }))).into_response(),
    };
    let block_id = match req.block_id.filter(|b| !b.is_empty()) {
        Some(b) => b,
        None => return (StatusCode::BAD_REQUEST, Json(json!({ "error": "missing block_id" }))).into_response(),
    };
    let call = crate::backend::service::WebCallType {
        service: "object".to_string(),
        method: "UpdateObjectMeta".to_string(),
        uicontext: None,
        args: vec![
            json!(format!("block:{block_id}")),
            json!({ "frame:title": title }),
        ],
    };
    finish_name_call(&state, call, json!({ "success": true, "block_id": block_id, "title": title })).await
}

/// `POST /api/v1/workspace/name` — rename a workspace. Defaults to the
/// caller's own workspace (resolved from `block_id`). Routes through
/// `workspace.UpdateWorkspace`.
pub(super) async fn handle_workspace_name(
    State(state): State<AppState>,
    Json(req): Json<WorkspaceNameRequest>,
) -> impl IntoResponse {
    let name = match clean_name(&req.name, 128) {
        Some(n) => n,
        None => return (StatusCode::BAD_REQUEST, Json(json!({ "error": "name must not be empty" }))).into_response(),
    };
    let workspace_id = match req.workspace_id.filter(|w| !w.is_empty()) {
        Some(w) => w,
        None => match resolve_own(&state, req.block_id, |c| c.workspace_id.clone()) {
            Ok(w) => w,
            Err(resp) => return resp,
        },
    };
    let call = crate::backend::service::WebCallType {
        service: "workspace".to_string(),
        method: "UpdateWorkspace".to_string(),
        uicontext: None,
        args: vec![json!(workspace_id), json!(name)],
    };
    finish_name_call(&state, call, json!({ "success": true, "workspace_id": workspace_id, "name": name })).await
}

/// Resolve a target id from the caller's own context (via `block_id`), using
/// `pick` to select the field. Returns the route's error response on failure
/// (missing block_id, unresolvable block, or the field is `None`).
pub(super) fn resolve_own(
    state: &AppState,
    block_id: Option<String>,
    pick: impl Fn(&service::AgentContext) -> Option<String>,
) -> Result<String, Response> {
    let block_id = block_id.unwrap_or_default();
    if block_id.is_empty() {
        return Err((StatusCode::BAD_REQUEST, Json(json!({ "error": "provide an explicit target id or block_id" }))).into_response());
    }
    let ctx = service::resolve_agent_context(&state.mstore, &block_id)
        .map_err(|e| (StatusCode::NOT_FOUND, Json(json!({ "error": e }))).into_response())?;
    pick(&ctx).filter(|s| !s.is_empty()).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "no such target resolved for this agent" })),
        )
            .into_response()
    })
}

/// Run a naming service call and map the result to a JSON HTTP response.
pub(super) async fn finish_name_call(
    state: &AppState,
    call: crate::backend::service::WebCallType,
    ok_body: serde_json::Value,
) -> Response {
    let result = service::run_service_call(state, &call).await;
    if let Some(err) = result.error {
        return (name_call_error_status(&err), Json(json!({ "error": err }))).into_response();
    }
    Json(ok_body).into_response()
}

/// Map a naming service-call error to an HTTP status. The service layer
/// returns plain `String` errors, so this matches on error text — the same
/// established pattern as `app_api_error_status` above, but with not-found
/// split out as a real 404 (that helper lumps it into 400): a caller
/// holding a stale window/tab/workspace id from an earlier `Layout` call
/// is a not-found, not a malformed request. Everything unrecognized stays
/// 500 (genuine service faults, e.g. SQLite write failures).
/// SPEC_WINDOW_NAME_API_HARDENING_2026_08_08.md §3.2.
pub(super) fn name_call_error_status(e: &str) -> StatusCode {
    if e.contains("not found") {
        StatusCode::NOT_FOUND
    } else if e.contains("invalid") {
        StatusCode::BAD_REQUEST
    } else {
        StatusCode::INTERNAL_SERVER_ERROR
    }
}
