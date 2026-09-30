// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Layout, window, workspace and tab queries and actions.
//! Split out of server/mod.rs unchanged (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4.1).

use super::*;

/// `GET /api/v1/layout` — read-only window → workspace → tab → pane tree.
pub(super) async fn handle_layout(State(state): State<AppState>) -> impl IntoResponse {
    Json(service::agent_layout(&state.mstore))
}

/// `GET /api/v1/windows` — flat list of windows.
pub(super) async fn handle_list_windows(State(state): State<AppState>) -> impl IntoResponse {
    Json(service::agent_windows(&state.mstore))
}

/// `GET /api/v1/workspaces` — flat list of workspaces.
pub(super) async fn handle_list_workspaces(State(state): State<AppState>) -> impl IntoResponse {
    Json(service::agent_workspaces(&state.mstore))
}

#[derive(serde::Deserialize)]
pub(super) struct ListTabsQuery {
    /// Limit to this workspace's tabs. Omit (or pass `block_id`) for all tabs.
    #[serde(default)]
    pub(super) workspace_id: Option<String>,
    /// Calling agent's block id — scopes to the caller's own workspace when
    /// `workspace_id` is omitted.
    #[serde(default)]
    pub(super) block_id: Option<String>,
}

/// `GET /api/v1/tabs` — flat list of tabs, optionally scoped to a workspace
/// (explicit `workspace_id`, or the caller's own via `block_id`).
pub(super) async fn handle_list_tabs(
    State(state): State<AppState>,
    Query(q): Query<ListTabsQuery>,
) -> impl IntoResponse {
    let ws_id = q.workspace_id.filter(|w| !w.is_empty()).or_else(|| {
        q.block_id
            .filter(|b| !b.is_empty())
            .and_then(|b| service::resolve_agent_context(&state.mstore, &b).ok())
            .and_then(|ctx| ctx.workspace_id)
    });
    Json(service::agent_tabs(&state.mstore, ws_id.as_deref()))
}

/// `POST /api/v1/tab/activate` — make `tab_id` the active tab in its
/// workspace. Routes through `workspace.SetActiveTab`.
pub(super) async fn handle_tab_activate(
    State(state): State<AppState>,
    Json(req): Json<TabActivateRequest>,
) -> impl IntoResponse {
    let tab_id = req.tab_id.trim().to_string();
    if tab_id.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "missing tab_id" }))).into_response();
    }
    let ws_id = match service::workspace_id_for_tab(&state.mstore, &tab_id) {
        Some(w) => w,
        None => return (StatusCode::NOT_FOUND, Json(json!({ "error": format!("no workspace owns tab {tab_id}") }))).into_response(),
    };
    let call = crate::backend::service::WebCallType {
        service: "workspace".to_string(),
        method: "SetActiveTab".to_string(),
        uicontext: None,
        args: vec![json!(ws_id), json!(tab_id)],
    };
    finish_name_call(&state, call, json!({ "success": true, "tab_id": tab_id })).await
}

/// `POST /api/v1/tab/new` — create (and activate) a new tab in the caller's
/// workspace (or an explicit `workspace_id`). Routes through
/// `workspace.CreateTab`.
pub(super) async fn handle_tab_new(
    State(state): State<AppState>,
    Json(req): Json<TabNewRequest>,
) -> impl IntoResponse {
    let name = req.name.map(|n| n.trim().chars().take(128).collect::<String>()).unwrap_or_default();
    let workspace_id = match req.workspace_id.filter(|w| !w.is_empty()) {
        Some(w) => w,
        None => match resolve_own(&state, req.block_id, |c| c.workspace_id.clone()) {
            Ok(w) => w,
            Err(resp) => return resp,
        },
    };
    let call = crate::backend::service::WebCallType {
        service: "workspace".to_string(),
        method: "CreateTab".to_string(),
        uicontext: None,
        // [ws_id, name, activate]; empty name → backend auto-names tab{N}.
        args: vec![json!(workspace_id), json!(name), json!(true)],
    };
    finish_name_call(&state, call, json!({ "success": true, "workspace_id": workspace_id })).await
}

/// `POST /api/v1/window/focus` — bring a window to the foreground. Defaults to
/// the caller's own window. Routes through `client.FocusWindow`.
pub(super) async fn handle_window_focus(
    State(state): State<AppState>,
    Json(req): Json<WindowFocusRequest>,
) -> impl IntoResponse {
    let window_id = match req.window_id.filter(|w| !w.is_empty()) {
        Some(w) => w,
        None => match resolve_own(&state, req.block_id, |c| c.window_id.clone()) {
            Ok(w) => w,
            Err(resp) => return resp,
        },
    };
    let call = crate::backend::service::WebCallType {
        service: "client".to_string(),
        method: "FocusWindow".to_string(),
        uicontext: None,
        args: vec![json!(window_id)],
    };
    finish_name_call(&state, call, json!({ "success": true, "window_id": window_id })).await
}

// ---- Origin checks ----
