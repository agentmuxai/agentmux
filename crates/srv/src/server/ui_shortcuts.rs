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
// checks `target` is a pane in that window's active tab.

/// Commands no agent may run (owner decision §8.2). The frontend refuses
/// them too, and also refuses PressKeys of any key bound to one; this is
/// the same rule where the request comes in.
fn refused_command(command: &str) -> Option<&'static str> {
    match command {
        "files:deletePermanently" => Some("it can't be undone"),
        "pane:close" => Some(
            "it closes the focused pane at once: use ClosePane, which gives the user 15 seconds to undo \
             (QuitSelf for your own pane)",
        ),
        // Restoring from the Trash isn't supported on macOS yet
        // (backend/fs_ops/trash_worker.rs), so there it can't be undone.
        "files:trash" if cfg!(target_os = "macos") => {
            Some("on macOS it can't be undone yet (restoring from the Trash isn't supported there)")
        }
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

#[cfg(test)]
mod tests {
    use super::refused_command;

    #[test]
    fn refuses_only_the_commands_the_owner_ruled_out() {
        assert!(refused_command("files:deletePermanently").is_some());
        assert!(refused_command("pane:close").unwrap().contains("ClosePane"));
        for allowed in ["tab:close", "split:right", "pane:replaceWithLauncher"] {
            assert!(refused_command(allowed).is_none(), "{allowed} keeps its own confirmation or undo");
        }
        // Trash can be undone everywhere but macOS.
        assert_eq!(refused_command("files:trash").is_some(), cfg!(target_os = "macos"));
    }
}
