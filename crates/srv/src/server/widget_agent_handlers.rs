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

use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::widget_packages::{self, WidgetKind, WidgetPackageInfo, WidgetState};
use crate::backend::widget_signature::SignatureState;
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
    let event = crate::backend::mps_events::WidgetRequestsEvent { requests: widget_requests::requests().list() };
    state.broker.publish_event::<crate::backend::mps_events::WidgetRequests>(&event, vec![], 0);
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
/// Its pane comes with it, the one the waiting tone plays for.
fn asking_agent(state: &AppState, auth: Option<&agentmux_common::api_types::UiAutomationAuth>) -> (String, Option<String>) {
    const UNVERIFIED: &str = "Something on this computer (not a verified agent)";
    match auth {
        Some(a) => match super::ui_handlers::verified_block_id(state, None, a) {
            Ok(pane) => (a.agent_id.chars().take(64).collect(), Some(pane)),
            Err(e) => {
                tracing::warn!(claimed = %a.agent_id, error = %e, "a widget install's signed identity didn't verify");
                (UNVERIFIED.to_string(), None)
            }
        },
        None => (UNVERIFIED.to_string(), None),
    }
}

/// The package an install request was filed for, by the request's id: the runner approves
/// exactly this version and signer, so a package changed in the meantime is not approved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct InstallTarget {
    pub id: String,
    pub hash: String,
    pub signer: String,
}

fn targets() -> std::sync::MutexGuard<'static, std::collections::HashMap<String, InstallTarget>> {
    static TARGETS: std::sync::OnceLock<std::sync::Mutex<std::collections::HashMap<String, InstallTarget>>> = std::sync::OnceLock::new();
    TARGETS.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}

pub(crate) fn remember_target(request_id: &str, target: InstallTarget) {
    let mut t = targets();
    // A request that ended without running (denied, vetoed, withdrawn) leaves no use for its entry,
    // and what stays is bounded by the open requests there can be.
    t.retain(|id, _| crate::backend::sensitive_requests::registry().get(id).is_some_and(|r| r.status.is_open()));
    // The content is part of what the request shows, so a request joined is for the same content;
    // the first target stands in any case.
    t.entry(request_id.to_string()).or_insert(target);
}

pub(crate) fn take_target(request_id: &str) -> Option<InstallTarget> {
    targets().remove(request_id)
}

/// The facts a widget install request shows the person: every permission, who signed it, and
/// plainly when it is a trusted widget (full access to AgentMux).
pub(crate) fn install_facts(pkg: &WidgetPackageInfo) -> std::collections::BTreeMap<String, String> {
    let name: String = pkg.name.chars().take(80).collect();
    let publisher = match (&pkg.signature.state, &pkg.signature.fingerprint) {
        (SignatureState::Unsigned, _) | (_, None) => format!("{} (unsigned)", pkg.signature.publisher),
        (SignatureState::KeyChanged, Some(f)) => format!("{} (signed by a different key than before, {f})", pkg.signature.publisher),
        (_, Some(f)) => format!("{} (signed, key {f})", pkg.signature.publisher),
    };
    let permissions = if pkg.kind == WidgetKind::Trusted {
        "TRUSTED: full access to AgentMux".to_string()
    } else if pkg.permissions.is_empty() {
        "none".to_string()
    } else {
        pkg.permissions.join(", ")
    };
    // The start of the content's hash: different files are a different request, never a join.
    let content: String = pkg.hash.chars().take(12).collect();
    [("widget", format!("{name} ({} {}, content {content})", pkg.id, pkg.version)), ("publisher", publisher), ("permissions", permissions)]
        .into_iter()
        .map(|(k, v)| (k.to_string(), v))
        .collect()
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

/// `POST /api/v1/widgets/install`: copy the package in and, for a verified agent, file a
/// sensitive request to install it (answered at once with the request's id; the agent is told as
/// it goes). Without a verified agent, ask the user and answer when they do (or when the wait
/// runs out, with the request left for them to answer later in Settings → Widgets).
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
    // Who signed it, so the agent can tell the user what the prompt says
    // (SPEC_WIDGET_SHARING_2026_10_10.md §2.3).
    let signature = pkg.signature.clone();
    match pkg.state {
        WidgetState::Approved => return Json(json!({ "status": "installed", "id": id, "views": views, "signature": signature })).into_response(),
        WidgetState::Invalid => return error(StatusCode::BAD_REQUEST, pkg.error.unwrap_or_else(|| "the package isn't valid".into())),
        WidgetState::Disabled => {
            return Json(json!({ "status": "disabled", "id": id, "views": views, "signature": signature, "message": "the user turned this widget off; it stays off" }))
                .into_response()
        }
        _ => {}
    }
    let (agent, pane) = asking_agent(&state, body.auth.as_ref());
    // A verified agent's install is a sensitive request: the super agent decides and the person
    // can veto the countdown. Something that isn't a verified agent has no one to answer for it,
    // so the person is asked, as before.
    if let Some(uid) = pane.as_ref().and_then(|_| crate::backend::reactive::get_global_handler().get_agent(&agent)).and_then(|r| r.uid) {
        let reason = format!("install the widget {} for the user", pkg.name);
        let facts = install_facts(&pkg);
        // The person is shown every permission, never a shortened list: too many to show whole is
        // refused, and the agent is told what to do.
        if facts["permissions"].chars().count() > crate::backend::sensitive_ops::VALUE_MAX {
            return error(
                StatusCode::BAD_REQUEST,
                format!("{id} asks for too many permissions to show the person whole ({} characters, at most {}); ask for fewer and install again", facts["permissions"].chars().count(), crate::backend::sensitive_ops::VALUE_MAX),
            );
        }
        return match super::sensitive_handlers::file_request(&state, &uid, &agent, "widget_install", &facts, &reason).await {
            Ok(filed) => {
                let request_id = filed["id"].as_str().unwrap_or_default().to_string();
                remember_target(&request_id, InstallTarget { id: id.clone(), hash: pkg.hash.clone(), signer: pkg.signature.fingerprint.clone().unwrap_or_default() });
                Json(json!({
                    "status": "requested",
                    "id": id,
                    "request": request_id,
                    "views": views,
                    "signature": signature,
                    "review": filed["review"],
                }))
                .into_response()
            }
            Err(e) => error(StatusCode::BAD_REQUEST, e),
        };
    }
    let answer = widget_requests::requests().ask(WidgetInstallRequest::of(&pkg, &agent));
    publish_requests(&state);
    // The asking agent's pane (or, unverified, no pane) waits on the user
    // until this returns: the waiting tone and notification for it.
    let _asking = crate::backend::user_attention::Asking::start(
        &state.broker,
        &state.mstore,
        pane.as_deref().unwrap_or(""),
        "widget",
        &format!("{agent} wants to install {}", pkg.name),
    );
    tracing::info!(id = %id, agent = %agent, "an agent asked to install a widget; waiting for the user");
    let wait = body.wait_secs.map(Duration::from_secs).unwrap_or(DEFAULT_WAIT).min(MAX_WAIT);
    match tokio::time::timeout(wait, answer).await {
        Ok(Ok(Answer::Approved)) => Json(json!({ "status": "installed", "id": id, "views": views, "signature": signature })).into_response(),
        Ok(Ok(Answer::Declined)) | Ok(Err(_)) => Json(json!({
            "status": "declined",
            "id": id,
            "signature": signature,
            "message": "the user didn't approve it; it stays in Settings → Widgets until they approve or remove it",
        }))
        .into_response(),
        Err(_) => Json(json!({
            "status": "pending",
            "id": id,
            "signature": signature,
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
