// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `app.update-status`: what srv knows about newer AgentMux releases
//! (`backend::update_check`), for a window that opens after the last
//! `app:updatestatus` event. Downloading the release it found
//! (`backend::update_download`), restarting into it once agents are idle and
//! stopping that wait (`backend::update_restart`) are the user's clicks only:
//! they come through the host (`POST /api/v1/host/app_update`), which agents,
//! holding the instance auth key, can't impersonate.
//! `app.upgrade-requests`: agents' requests waiting for the user
//! (`backend::upgrade_request`); the user's answer comes only through the
//! host (`POST /api/v1/host/upgrade_request`), which agents can't impersonate.
//! And the agent's side over HTTP: the `App` tool's `version`
//! (`GET /api/v1/app/update-status`) and `upgrade`
//! (`POST /api/v1/agent/self/upgrade`).

use super::*;

pub fn register(engine: &Arc<WshRpcEngine>, _state: &AppState) {
    engine.register_typed(COMMAND_APP_UPDATE_STATUS, move |_req: Option<NoArgsReq>, _ctx| async move {
        Ok::<_, String>(crate::backend::update_check::current_status())
    });
    engine.register_typed(COMMAND_APP_UPGRADE_REQUESTS, move |_req: Option<NoArgsReq>, _ctx| async move {
        Ok::<_, String>(crate::backend::upgrade_request::UpgradeRequestsEvent {
            requests: crate::backend::upgrade_request::list(),
        })
    });
}

/// The user's click in the version panel, relayed by the host.
#[derive(Debug, serde::Deserialize)]
pub(crate) struct HostAppUpdateAction {
    /// `download`, `restart` or `cancel`.
    action: String,
    /// `restart`: without waiting for agents to finish their turn.
    #[serde(default)]
    now: bool,
}

/// `POST /api/v1/host/app_update`: Download, Upgrade & restart (or Restart
/// now) and Cancel in the version panel, relayed by the CEF host with its own
/// token. Answers the status after the action, as `data`.
pub(crate) async fn handle_host_app_update(
    axum::extract::State(state): axum::extract::State<AppState>,
    headers: axum::http::HeaderMap,
    axum::Json(body): axum::Json<HostAppUpdateAction>,
) -> axum::response::Response {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use axum::Json;
    if !crate::server::ui_handlers::from_host(&state, &headers).await {
        return (StatusCode::FORBIDDEN, Json(json!({ "ok": false, "error": "only the AgentMux host can do that" })))
            .into_response();
    }
    let result = match body.action.as_str() {
        "download" => crate::backend::update_download::start(),
        "restart" => crate::backend::update_restart::start(body.now),
        "cancel" => crate::backend::update_restart::cancel(),
        other => Err(format!("unknown action {other:?}")),
    };
    match result {
        Ok(status) => Json(json!({ "ok": true, "data": status })).into_response(),
        Err(e) => (StatusCode::CONFLICT, Json(json!({ "ok": false, "error": e }))).into_response(),
    }
}

/// `POST /api/v1/host/upgrade_request`: the user's Upgrade now / Later on an
/// agent's upgrade request, relayed by the CEF host with its own token. The
/// instance auth key, which every agent has, isn't enough.
pub(crate) async fn handle_host_upgrade_request(
    axum::extract::State(state): axum::extract::State<AppState>,
    headers: axum::http::HeaderMap,
    axum::Json(body): axum::Json<crate::backend::upgrade_request::UpgradeRequestAnswer>,
) -> axum::response::Response {
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use axum::Json;
    if !crate::server::ui_handlers::from_host(&state, &headers).await {
        return (StatusCode::FORBIDDEN, Json(json!({ "ok": false, "error": "only the AgentMux host can answer an upgrade request" })))
            .into_response();
    }
    match crate::backend::upgrade_request::answer(&state, &body.id, &body.version, body.accept) {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(e) => (StatusCode::CONFLICT, Json(json!({ "ok": false, "error": e }))).into_response(),
    }
}

/// `GET /api/v1/app/update-status`: the `App` tool's `version`.
pub(crate) async fn handle_update_status() -> axum::Json<crate::backend::update_check::AppUpdateStatus> {
    axum::Json(crate::backend::update_check::current_status())
}

/// `POST /api/v1/agent/self/upgrade`: the `App` tool's `upgrade`. Starts the
/// upgrade when the user's own turn asked for it (`self_quit::gate`, as for
/// `QuitSelf`), otherwise asks the user in every window.
pub(crate) async fn handle_request_upgrade(
    axum::extract::State(state): axum::extract::State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    axum::Json(req): axum::Json<agentmux_common::api_types::RequestUpgradeRequest>,
) -> axum::response::Response {
    use crate::backend::upgrade_request::{self, Outcome};
    use axum::http::StatusCode;
    use axum::response::IntoResponse;
    use axum::Json;

    let block_id = match crate::server::ui_handlers::verified_block_id(&state, caller.as_deref(), &req.auth) {
        Ok(b) => b,
        Err(e) => return (StatusCode::UNAUTHORIZED, Json(json!({ "error": e }))).into_response(),
    };
    let reason = req.reason.trim();
    if reason.is_empty() {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": "a reason is required" }))).into_response();
    }
    let status = crate::backend::update_check::current_status();
    let version = match upgrade_request::installable(&status, req.version.as_deref()) {
        Ok(v) => v,
        Err(outcome) => return Json(outcome).into_response(),
    };
    let agent = req.auth.agent_id.as_str();
    let provenance = crate::backend::blockcontroller::get_controller(&block_id).and_then(|c| c.turn_provenance());
    let outcome = match crate::sagas::self_quit::gate(provenance.as_ref(), &req.user_instruction) {
        Ok(()) => {
            tracing::info!(agent, version = %version, reason, "upgrade: the user's own turn asked; starting");
            // Prompts other agents raised for this release are answered by it;
            // they and this agent hear if the upgrade stops short.
            let mut askers = upgrade_request::resolve_started(&state, &version);
            askers.push(block_id.clone());
            upgrade_request::start_upgrade(version.clone(), askers);
            Outcome::Started { version }
        }
        Err(refusal) => {
            tracing::info!(agent, version = %version, reason, why = refusal.as_str(), "upgrade: asking the user");
            let id = upgrade_request::ask_user(&state, &block_id, agent, reason, &version);
            Outcome::PendingUser { version, id }
        }
    };
    Json(outcome).into_response()
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use serde_json::json;
    use tower::ServiceExt;

    async fn post(app: &axum::Router, host_token: Option<&str>, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
        let mut b = Request::builder()
            .uri("/api/v1/host/app_update")
            .method("POST")
            .header("X-AuthKey", "test-secret-key")
            .header("Content-Type", "application/json");
        if let Some(token) = host_token {
            b = b.header("X-Host-Token", token);
        }
        let resp = app.clone().oneshot(b.body(Body::from(body.to_string())).unwrap()).await.unwrap();
        let status = resp.status();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap_or_default())
    }

    // §4.1: the version panel's clicks come only through the host. The
    // instance auth key, which every agent has, isn't enough.
    #[tokio::test]
    async fn only_the_host_can_click_in_the_version_panel() {
        let state = crate::server::tests::test_state();
        *state.host_ipc.lock().await = Some(crate::server::state::HostIpc { port: 1, token: "host-ipc-token".into() });
        let app = crate::server::build_router(state);
        for action in ["download", "restart", "cancel"] {
            let (s, body) = post(&app, None, json!({ "action": action })).await;
            assert_eq!(s, StatusCode::FORBIDDEN, "{action}: {body}");
            let (s, _) = post(&app, Some("not-the-token"), json!({ "action": action, "now": true })).await;
            assert_eq!(s, StatusCode::FORBIDDEN, "{action}");
        }
        // The host gets through to the action itself.
        let (s, body) = post(&app, Some("host-ipc-token"), json!({ "action": "launch" })).await;
        assert_eq!(s, StatusCode::CONFLICT, "{body}");
        assert!(body["error"].as_str().unwrap_or_default().contains("unknown action"), "{body}");
    }
}
