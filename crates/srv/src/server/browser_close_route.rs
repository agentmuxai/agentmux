// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//! `POST /api/v1/ui/browser/close`: backs Browser's `close`. An agent closes a browser pane it drives, when
//! its job is done. The pane goes through the same check every other agent browser call does
//! (`target_block_id`: it must still be the agent's, not taken over, paused or another agent's), then the
//! shared close path, where the user has 30 s to keep it.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;

use crate::server::ui_handlers::{err_response, target_block_id};
use crate::server::AppState;

pub(crate) async fn handle_ui_browser_close(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<agentmux_common::api_types::UiBrowserCloseRequest>,
) -> impl IntoResponse {
    // Without a pane, the check would resolve to the caller's own, and closing that ends the caller.
    let Some(pane) = req.pane.as_deref().map(str::trim).filter(|p| !p.is_empty()) else {
        return err_response(StatusCode::BAD_REQUEST, "Browser close needs pane: the id Browser open gave you".to_string());
    };
    let block_id = match target_block_id(&state, caller.as_deref(), &req.auth, Some(pane)) {
        Ok(b) => b,
        Err((code, e)) => return err_response(code, e),
    };
    let reason = "the agent is done with its browser pane";
    let (code, body) = crate::server::app_api::pane::close_pane_as(&state, &req.auth.agent_id, &block_id, Some(reason), "Browser close").await;
    (code, Json(body)).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::tests::test_state;
    use agentmux_common::api_types::{UiAutomationAuth, UiBrowserCloseRequest};
    use agentmux_common::ipc::{Command, Event};

    async fn apply(state: &AppState, cmd: Command) -> Vec<Event> {
        let evs = crate::server::service::dispatch_to_reducer(state, cmd).await;
        for ev in &evs {
            crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).unwrap();
        }
        evs
    }

    /// Two blocks in one new tab: `(tab, first, second)`.
    async fn two_blocks(state: &AppState) -> (String, String, String) {
        let ws = apply(state, Command::CreateWorkspace { name: "w".into() }).await;
        let ws = ws.iter().find_map(|e| if let Event::WorkspaceCreated { workspace_id, .. } = e { Some(workspace_id.clone()) } else { None }).unwrap();
        let tab = apply(state, Command::CreateTab { workspace_id: ws, name: "t".into() }).await;
        let tab = tab.iter().find_map(|e| if let Event::TabCreated { tab_id, .. } = e { Some(tab_id.clone()) } else { None }).unwrap();
        let mut ids = Vec::new();
        for _ in 0..2 {
            let evs = apply(state, Command::CreateBlock { tab_id: tab.clone(), meta: serde_json::json!({ "view": "term" }) }).await;
            ids.push(evs.iter().find_map(|e| if let Event::BlockCreated { block_id, .. } = e { Some(block_id.clone()) } else { None }).unwrap());
        }
        (tab, ids.remove(0), ids.remove(0))
    }

    fn auth(state: &AppState, agent_id: &str) -> UiAutomationAuth {
        let key = state.mstore.agent_jekt_key_ensure(agent_id).unwrap();
        let ts_secs = agentmux_common::time::now_secs();
        let sig = agentmux_common::jekt_sign::sign_jekt(&key, "ui-automation-identity", agent_id, "__srv__", ts_secs, "");
        UiAutomationAuth { agent_id: agent_id.to_string(), ts_secs, sig }
    }

    async fn status_of(state: &AppState, req: UiBrowserCloseRequest) -> StatusCode {
        handle_ui_browser_close(State(state.clone()), None, Json(req)).await.into_response().status()
    }

    #[tokio::test]
    async fn close_needs_a_pane_and_only_closes_a_browser_pane_the_agent_drives() {
        let state = test_state();
        let (tab, own, other) = two_blocks(&state).await;
        let agent = format!("close-browser-{}", uuid::Uuid::new_v4());
        state.reactive_handler.register_agent(&agent, &own, Some(&tab)).unwrap();
        let ask = |pane: Option<&str>| UiBrowserCloseRequest { auth: auth(&state, &agent), pane: pane.map(str::to_string) };

        assert_eq!(status_of(&state, ask(None)).await, StatusCode::BAD_REQUEST, "no pane would mean the caller's own");
        assert_eq!(status_of(&state, ask(Some("  "))).await, StatusCode::BAD_REQUEST);
        // The agent's own pane, and a pane that is no browser pane it opened, are refused, not scheduled.
        for pane in [own.as_str(), other.as_str(), "no-such-pane"] {
            let code = status_of(&state, ask(Some(pane))).await;
            assert!(code == StatusCode::FORBIDDEN || code == StatusCode::NOT_FOUND, "{pane}: {code}");
        }
        let s = state.srv_state.lock().await;
        assert!(s.blocks.contains_key(&own) && s.blocks.contains_key(&other), "nothing was closed");
    }
}
