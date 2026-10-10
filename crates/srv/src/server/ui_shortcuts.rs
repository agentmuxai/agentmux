// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Agent-facing shortcuts (`ListShortcuts` / `RunCommand` / `PressKeys`),
//! docs/specs/PLAN_SHORTCUTS_VERIFIED_AND_APP_API_2026_10_10.md §4. Same
//! identity model as the rest of `ui_handlers`: the caller's own pane is
//! derived from its verified identity, never taken from the request.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;

use agentmux_common::api_types::{UiPressKeysRequest, UiRunCommandRequest, UiShortcutsListRequest};

use super::ui_handlers::{err_response, proxy_data, target_block_id};
use super::AppState;

// Each acts in the window that holds the caller's own pane (owner decision §8.1): the
// host resolves that window from the verified block and calls the page's
// `window.__agentmux_shortcuts` (frontend keybindings/app-api.ts), which
// reveals `target` (a pane in that window, in any tab) before acting on it.

/// Commands no agent may run: agents are first-class owners, so only what
/// has no protection. A permanent delete can't be undone. The frontend
/// refuses it too, and PressKeys of any key bound to it.
fn refused_command(command: &str) -> Option<&'static str> {
    match command {
        "files:deletePermanently" => Some("it can't be undone"),
        _ => None,
    }
}

/// `POST /api/v1/ui/shortcuts/list` — backs `ListShortcuts`.
pub(crate) async fn handle_ui_shortcuts_list(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<UiShortcutsListRequest>,
) -> impl IntoResponse {
    let block_id = match target_block_id(&state, caller.as_deref(), &req.auth, None) {
        Ok(b) => b,
        Err((code, e)) => return err_response(code, e),
    };
    proxy_data(&state, "list_shortcuts", json!({ "block_id": block_id }), std::time::Duration::from_secs(10)).await
}

/// `POST /api/v1/ui/shortcuts/run` — backs `RunCommand`.
pub(crate) async fn handle_ui_run_command(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<UiRunCommandRequest>,
) -> impl IntoResponse {
    let block_id = match target_block_id(&state, caller.as_deref(), &req.auth, None) {
        Ok(b) => b,
        Err((code, e)) => return err_response(code, e),
    };
    if let Some(why) = refused_command(&req.command) {
        return err_response(StatusCode::FORBIDDEN, format!("{} is not available to agents: {why}", req.command));
    }
    if req.command == "pane:close" {
        return close_pane(&state, &req, &block_id).await;
    }
    tracing::info!(agent_id = %req.auth.agent_id, block_id = %block_id, command = %req.command, "[ui-automation] run command");
    proxy_data(
        &state,
        "run_command",
        json!({ "block_id": block_id, "command": req.command, "target": req.target }),
        std::time::Duration::from_secs(10),
    )
    .await
}

/// `POST /api/v1/ui/shortcuts/press` — backs `PressKeys`.
pub(crate) async fn handle_ui_press_keys(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<UiPressKeysRequest>,
) -> impl IntoResponse {
    let block_id = match target_block_id(&state, caller.as_deref(), &req.auth, None) {
        Ok(b) => b,
        Err((code, e)) => return err_response(code, e),
    };
    tracing::info!(agent_id = %req.auth.agent_id, block_id = %block_id, keys = %req.keys, "[ui-automation] press keys");
    proxy_data(
        &state,
        "press_keys",
        json!({ "block_id": block_id, "keys": req.keys, "target": req.target }),
        std::time::Duration::from_secs(10),
    )
    .await
}

/// RunCommand `pane:close`: the pane named by `target`, closed the way
/// ClosePane closes it (`app_api::pane::close_pane_as`). A pane the agent is
/// in closes at once; another agent's pane waits 15 s for its user to keep
/// it, and the answer says so. The key itself closes the focused pane at
/// once, with no undo, so the page never runs it for the App API.
async fn close_pane(state: &AppState, req: &UiRunCommandRequest, own_block_id: &str) -> axum::response::Response {
    let Some(target) = req.target.as_deref().map(str::trim).filter(|t| !t.is_empty()) else {
        return err_response(
            StatusCode::BAD_REQUEST,
            "pane:close needs `target`, the pane to close (another agent's pane gets the user's 15-second undo)".to_string(),
        );
    };
    let (code, body) = crate::server::app_api::pane::close_pane_as(
        state,
        &req.auth.agent_id,
        own_block_id,
        target,
        Some("RunCommand pane:close"),
        "RunCommand",
    )
    .await;
    match code {
        StatusCode::OK => Json(json!({ "ok": true, "data": { "ran": true, "closed": body } })).into_response(),
        StatusCode::ACCEPTED => Json(json!({ "ok": true, "data": {
            "ran": false,
            "pending": body,
            "reason": "it's another agent's pane: its user has 15 seconds to keep it, and it closes after that unless they do",
        } }))
        .into_response(),
        _ => err_response(code, body.get("error").and_then(|e| e.as_str()).unwrap_or("close failed").to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::refused_command;

    #[test]
    fn refuses_only_the_commands_the_owner_ruled_out() {
        assert!(refused_command("files:deletePermanently").is_some());
        assert!(refused_command("pane:close").is_none(), "pane:close goes through ClosePane's undo");
        for allowed in ["tab:close", "split:right", "pane:replaceWithLauncher", "files:trash"] {
            assert!(refused_command(allowed).is_none(), "{allowed} keeps its own confirmation or undo");
        }
    }
}
