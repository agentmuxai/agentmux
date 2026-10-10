// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What agents can do with widgets
//! (docs/specs/SPEC_USER_WIDGETS_AND_WIDGET_API_2026_10_09.md §10), for
//! agentmux-mcp's `WidgetList`, `WidgetInstall` and `OpenWidget`: list the
//! packages, and install one and wait for the user's answer. An agent never
//! approves: the prompt is the user's, and only the host carries the answer.

use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;

use crate::backend::mps::EVENT_WIDGET_REQUESTS;
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::widget_packages::{self, WidgetState};
use crate::backend::widget_requests::{self, Answer, WidgetInstallRequest};

use super::AppState;

const DEFAULT_WAIT: Duration = Duration::from_secs(300);
const MAX_WAIT: Duration = Duration::from_secs(600);

#[derive(Debug, Clone, serde::Serialize, Deserialize, ts_rs::TS)]
#[ts(export, export_to = "../../../frontend/types/rpc/")]
pub struct WidgetRequestsResult {
    pub requests: Vec<WidgetInstallRequest>,
}

/// Tells every UI the requests now waiting.
pub(crate) fn publish_requests(state: &AppState) {
    state.broker.publish(crate::backend::mps::MuxEvent {
        event: EVENT_WIDGET_REQUESTS.to_string(),
        scopes: vec![],
        sender: String::new(),
        persist: 0,
        data: Some(json!({ "requests": widget_requests::requests().list() })),
    });
}

pub fn register(engine: &Arc<WshRpcEngine>) {
    // widgets.requests: the install requests waiting for the user, for a UI
    // that starts after they were made.
    engine.register_typed("widgets.requests", |_req: serde_json::Value, _ctx| async move {
        Ok(WidgetRequestsResult { requests: widget_requests::requests().list() })
    });
}

fn error(status: StatusCode, message: impl Into<String>) -> Response {
    (status, Json(json!({ "error": message.into() }))).into_response()
}

/// Who the prompt says is asking: the agent whose signature verified, or,
/// without one, a plain "not a verified agent" (never a name the caller chose).
fn asking_agent(state: &AppState, auth: Option<&agentmux_common::api_types::UiAutomationAuth>) -> String {
    const UNVERIFIED: &str = "Something on this computer (not a verified agent)";
    match auth {
        Some(a) => match super::ui_handlers::verified_block_id(state, None, a) {
            Ok(_) => a.agent_id.chars().take(64).collect(),
            Err(e) => {
                tracing::warn!(claimed = %a.agent_id, error = %e, "a widget install's signed identity didn't verify");
                UNVERIFIED.to_string()
            }
        },
        None => UNVERIFIED.to_string(),
    }
}

/// `GET /api/v1/widgets`: every package and its state.
pub(super) async fn handle_widgets_list() -> Response {
    match widget_packages::service() {
        Some(s) => Json(json!({ "packages": s.list() })).into_response(),
        None => error(StatusCode::SERVICE_UNAVAILABLE, "widget packages aren't available on this AgentMux"),
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct InstallBody {
    path: String,
    #[serde(default)]
    replace: bool,
    /// The asking agent's signed identity, which names it in the prompt.
    /// Anything holding the auth key can call this route, so a name in the
    /// body proves nothing; a signature only that agent can make does.
    #[serde(default)]
    auth: Option<agentmux_common::api_types::UiAutomationAuth>,
    /// How long to wait for the user's answer.
    #[serde(default)]
    wait_secs: Option<u64>,
}

/// `POST /api/v1/widgets/install`: copy the package in, ask the user, and
/// answer when they do (or when the wait runs out, with the request left
/// for them to answer later in Settings → Widgets).
pub(super) async fn handle_widgets_install(State(state): State<AppState>, Json(body): Json<InstallBody>) -> Response {
    let Some(svc) = widget_packages::service() else {
        return error(StatusCode::SERVICE_UNAVAILABLE, "widget packages aren't available on this AgentMux");
    };
    let (dir, source) = (svc.widgets_dir.clone(), std::path::PathBuf::from(&body.path));
    let installed = tokio::task::spawn_blocking(move || widget_packages::install(&dir, &source, body.replace)).await;
    let id = match installed {
        Ok(Ok(id)) => id,
        Ok(Err(e)) => return error(StatusCode::BAD_REQUEST, e),
        Err(e) => return error(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()),
    };
    let packages = widget_packages::refresh_off_thread(&state.config_watcher, &state.event_bus, &state.broker).await;
    let Some(pkg) = packages.into_iter().find(|p| p.id == id) else {
        return error(StatusCode::INTERNAL_SERVER_ERROR, format!("{id} was copied in but isn't listed"));
    };
    let views: Vec<String> = pkg.panes.iter().map(|p| p.view.clone()).collect();
    match pkg.state {
        WidgetState::Approved => return Json(json!({ "status": "installed", "id": id, "views": views })).into_response(),
        WidgetState::Invalid => return error(StatusCode::BAD_REQUEST, pkg.error.unwrap_or_else(|| "the package isn't valid".into())),
        WidgetState::Disabled => {
            return Json(json!({ "status": "disabled", "id": id, "views": views, "message": "the user turned this widget off; it stays off" }))
                .into_response()
        }
        _ => {}
    }
    let agent = asking_agent(&state, body.auth.as_ref());
    let answer = widget_requests::requests().ask(WidgetInstallRequest::of(&pkg, &agent));
    publish_requests(&state);
    tracing::info!(id = %id, agent = %agent, "an agent asked to install a widget; waiting for the user");
    let wait = body.wait_secs.map(Duration::from_secs).unwrap_or(DEFAULT_WAIT).min(MAX_WAIT);
    match tokio::time::timeout(wait, answer).await {
        Ok(Ok(Answer::Approved)) => Json(json!({ "status": "installed", "id": id, "views": views })).into_response(),
        Ok(Ok(Answer::Declined)) | Ok(Err(_)) => Json(json!({
            "status": "declined",
            "id": id,
            "message": "the user didn't approve it; it stays in Settings → Widgets until they approve or remove it",
        }))
        .into_response(),
        Err(_) => Json(json!({
            "status": "pending",
            "id": id,
            "message": "the user hasn't answered yet; the request stays open in AgentMux",
        }))
        .into_response(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    async fn send(app: &axum::Router, req: Request<Body>) -> (StatusCode, serde_json::Value) {
        let resp = app.clone().oneshot(req).await.unwrap();
        let status = resp.status();
        let body = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&body).unwrap_or_default())
    }

    fn post(uri: &str, headers: &[(&str, &str)], body: serde_json::Value) -> Request<Body> {
        let mut b = Request::builder().uri(uri).method("POST").header("X-AuthKey", "test-secret-key").header("Content-Type", "application/json");
        for (k, v) in headers {
            b = b.header(*k, *v);
        }
        b.body(Body::from(body.to_string())).unwrap()
    }

    // §10: an agent's install waits for the user; its auth key can't answer
    // for them, only the host can, and the agent then hears "installed".
    #[tokio::test]
    async fn an_agents_install_waits_for_the_users_answer_through_the_host() {
        let tmp = tempfile::tempdir().unwrap();
        widget_packages::WidgetPackages::for_tests();
        let src = tmp.path().join("src").join("acme.agentmade");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::write(
            src.join("widget.json"),
            json!({ "manifestVersion": 1, "id": "acme.agentmade", "name": "Made by an agent", "version": "1.0.0", "kind": "sandboxed",
                    "permissions": ["storage"], "contributes": { "panes": [{ "name": "main" }] } })
                .to_string(),
        )
        .unwrap();
        std::fs::write(src.join("index.html"), "<p>hi</p>").unwrap();

        let state = crate::server::tests::test_state();
        *state.host_ipc.lock().await = Some(crate::server::state::HostIpc { port: 1, token: "host-ipc-token".into() });
        let app = crate::server::build_router(state);

        let install = {
            let app = app.clone();
            let path = src.to_string_lossy().into_owned();
            tokio::spawn(async move {
                // A name in the body is ignored: only a signed identity names the agent.
                send(&app, post("/api/v1/widgets/install", &[], json!({ "path": path, "agent": "AgentMux", "wait_secs": 20 }))).await
            })
        };
        // The request shows up for the user.
        let mut req = None;
        for _ in 0..100 {
            req = widget_requests::requests().list().into_iter().find(|r| r.id == "acme.agentmade");
            if req.is_some() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
        let req = req.expect("the install asks the user");
        assert_eq!(req.agent, "Something on this computer (not a verified agent)");
        assert_eq!(req.permissions, ["storage".to_string()]);
        // Only the host can answer for the user.
        let decision = json!({ "id": "acme.agentmade", "hash": req.hash, "decision": "approve" });
        let (s, _) = send(&app, post("/api/v1/host/widget_approval", &[], decision.clone())).await;
        assert_eq!(s, StatusCode::FORBIDDEN);
        assert!(!install.is_finished(), "still waiting");
        let (s, _) = send(&app, post("/api/v1/host/widget_approval", &[("X-Host-Token", "host-ipc-token")], decision)).await;
        assert_eq!(s, StatusCode::OK);
        let (s, body) = install.await.unwrap();
        assert_eq!(s, StatusCode::OK, "{body}");
        assert_eq!(body["status"], "installed");
        assert_eq!(body["views"], json!(["ext:acme.agentmade/main"]));
        // By then the list says so too, for the agent's next step, OpenWidget.
        let listed = widget_packages::service().unwrap().list().into_iter().find(|p| p.id == "acme.agentmade").unwrap();
        assert_eq!(listed.state, WidgetState::Approved);
        assert!(widget_requests::requests().list().iter().all(|r| r.id != "acme.agentmade"));
    }
}
