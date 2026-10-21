// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The routes behind `Desktop` (docs/specs/SPEC_DESKTOP_DRIVING_2026_10_20.md
//! §5.4–5.10, §6). srv is the single gate: the MCP tool asks `authorize`
//! before it looks at or touches a native app, and srv answers from the
//! grants, the blocklist, the limits and the Stop flag in
//! `backend::desktop_gate`. The user's answers (a prompt, Stop) come only
//! through the host (`/api/v1/host/desktop_request`, `/desktop_stop`), which
//! agents can't impersonate.

use std::sync::{Arc, Mutex};

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::{json, Value};

use agentmux_common::time::now_ms_u64;
use agentmux_common::api_types::{
    DesktopAuthorizeRequest, DesktopPolicyRequest, DesktopRecordRequest, DesktopReleaseRequest, DesktopWindowRef,
};

use crate::backend::desktop_gate::{self as gate, Check, Choice, DesktopEvent, DesktopRequest};
use crate::backend::rpc_types::*;
use crate::backend::wconfig::SettingsType;
use crate::server::AppState;

use super::*;

/// Actions `authorize` is asked about.
const ACTIONS: &[&str] = &["look", "click", "scroll", "type", "key", "wait"];

static ASKING: Mutex<Option<crate::backend::user_attention::Asking>> = Mutex::new(None);

pub fn register(engine: &Arc<WshRpcEngine>, _state: &AppState) {
    engine.register_typed(COMMAND_APP_DESKTOP, move |_req: Option<NoArgsReq>, _ctx| async move {
        Ok::<_, String>(snapshot())
    });
}

fn snapshot() -> DesktopEvent {
    gate::with_gate(|g| g.event(now_ms_u64()))
}

/// Tell every window what is waiting and what is granted, and ask for the
/// user's attention while a prompt is open.
fn publish(state: &AppState) {
    let event = snapshot();
    let mut asking = ASKING.lock().unwrap_or_else(|e| e.into_inner());
    *asking = event.requests.first().map(|r| {
        crate::backend::user_attention::Asking::start(
            &state.broker,
            &state.mstore,
            "",
            "desktop",
            &format!("{} asks to control {}", r.agent, r.app),
        )
    });
    drop(asking);
    state.broker.publish_event::<crate::backend::mps_events::AppDesktop>(&event, vec![], 0);
}

/// Tell the agent in pane `block_id` something, as an `[AgentMux]` note.
fn tell(block_id: &str, text: &str) {
    let handler = crate::backend::reactive::get_global_handler();
    let Some(target) = handler
        .list_agents()
        .into_iter()
        .find(|r| r.block_id == block_id)
        .map(|r| r.uid.unwrap_or(r.agent_id))
    else {
        return;
    };
    let _ = handler.deliver_system_note(&target, text);
}

struct Policy {
    enabled: bool,
    added: Vec<String>,
    allowed: Vec<String>,
    grant_minutes: u64,
}

fn names(settings: &SettingsType, key: &str) -> Vec<String> {
    settings
        .extra
        .get(key)
        .and_then(Value::as_array)
        .map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default()
}

fn policy(state: &AppState) -> Policy {
    let settings = state.config_watcher.get_settings();
    Policy {
        enabled: settings.extra.get(gate::SETTING_ENABLED).and_then(Value::as_bool).unwrap_or(false),
        added: names(&settings, gate::SETTING_BLOCKLIST),
        allowed: names(&settings, gate::SETTING_ALLOWLIST),
        grant_minutes: settings
            .extra
            .get(gate::SETTING_GRANT_MINUTES)
            .and_then(Value::as_u64)
            .filter(|m| (1..=24 * 60).contains(m))
            .unwrap_or(gate::DEFAULT_GRANT_MINUTES),
    }
}

const OFF: &str = "Desktop driving is off. The user turns it on in Settings (desktop:enabled); until they do, nothing is captured or controlled.";

fn caller_block(
    state: &AppState,
    caller: Option<&axum::Extension<crate::server::caller::Caller>>,
    auth: &agentmux_common::api_types::UiAutomationAuth,
) -> Result<String, Response> {
    crate::server::ui_handlers::verified_block_id(state, caller.map(|c| &c.0), auth)
        .map_err(|e| (StatusCode::UNAUTHORIZED, Json(json!({ "error": e }))).into_response())
}

fn audit_line(agent: &str, w: &DesktopWindowRef, action: &str, decision: &str, extra: Value) {
    gate::audit(&json!({
        "ts_ms": now_ms_u64(),
        "agent": agent,
        "app": gate::app_key(&w.app),
        "window": w.id,
        "action": action,
        "decision": decision,
        "details": extra,
    }));
}

fn reply(status: &str, message: &str) -> Response {
    Json(json!({ "status": status, "message": message })).into_response()
}

/// `POST /api/v1/agent/desktop/policy`: whether `Desktop` is on, and each
/// listed window's state (`blocked` with a reason, or `granted`).
pub(crate) async fn handle_policy(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<DesktopPolicyRequest>,
) -> Response {
    let block_id = match caller_block(&state, caller.as_ref(), &req.auth) {
        Ok(b) => b,
        Err(r) => return r,
    };
    let p = policy(&state);
    let agent = req.auth.agent_id.as_str();
    let now = now_ms_u64();
    let windows: Vec<Value> = req
        .windows
        .iter()
        .map(|w| {
            let blocked = gate::blocked_reason(&w.app, &w.title, &p.added, &p.allowed);
            let granted = blocked.is_none() && gate::with_gate(|g| g.has_grant(agent, &block_id, &gate::app_key(&w.app), now));
            json!({ "id": w.id, "blocked": blocked, "granted": granted })
        })
        .collect();
    Json(json!({ "enabled": p.enabled, "windows": windows })).into_response()
}

/// `POST /api/v1/agent/desktop/authorize`: may this agent do `action` on that
/// window now? Waits while the user is asked.
pub(crate) async fn handle_authorize(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<DesktopAuthorizeRequest>,
) -> Response {
    let block_id = match caller_block(&state, caller.as_ref(), &req.auth) {
        Ok(b) => b,
        Err(r) => return r,
    };
    let agent = req.auth.agent_id.clone();
    let w = &req.window;
    if !ACTIONS.contains(&req.action.as_str()) {
        return (StatusCode::BAD_REQUEST, Json(json!({ "error": format!("unknown desktop action {:?}", req.action) }))).into_response();
    }
    let p = policy(&state);
    if !p.enabled {
        return reply("refused", OFF);
    }
    if let Some(why) = gate::blocked_reason(&w.app, &w.title, &p.added, &p.allowed) {
        audit_line(&agent, w, &req.action, "refused:blocked", json!({ "why": why }));
        return reply("refused", &format!("{} can't be controlled: {why}.", gate::app_key(&w.app)));
    }
    let app = gate::app_key(&w.app);
    // At most a few spins, while TooFast waits half a second. Ask never waits
    // for the user here: the call answers `pending` at once, so the agent stays
    // free for the user's messages, and the answer comes as an [AgentMux] note.
    for _ in 0..8 {
        match gate::with_gate(|g| g.check(&agent, &block_id, &app, now_ms_u64())) {
            Check::Allow { grant_id } => {
                return Json(json!({ "status": "allow", "grant_id": grant_id, "message": "" })).into_response();
            }
            Check::Stopped => {
                audit_line(&agent, w, &req.action, "stopped", Value::Null);
                return reply("stopped", "The user pressed Stop. Don't carry on with this app; ask them what they want.");
            }
            Check::Denied => {
                audit_line(&agent, w, &req.action, "denied", Value::Null);
                return reply("denied", &format!("The user said no to controlling {app}. Don't ask again this session."));
            }
            Check::TooFast { wait_ms } => tokio::time::sleep(std::time::Duration::from_millis(wait_ms)).await,
            Check::Ask => {
                if gate::with_gate(|g| g.asking(&agent, &block_id, &app)) {
                    return reply("pending", &pending_message(&app));
                }
                let id = uuid::Uuid::new_v4().to_string();
                let request = DesktopRequest {
                    id: id.clone(),
                    agent: agent.clone(),
                    app: app.clone(),
                    title: w.title.chars().take(120).collect(),
                    asked_ms: now_ms_u64(),
                    block_id: block_id.clone(),
                    grant_minutes: p.grant_minutes,
                };
                let rx = gate::with_gate(|g| g.ask(request));
                publish(&state);
                let message = pending_message(&app);
                let (state, w, action) = (state.clone(), w.clone(), req.action.clone());
                tokio::spawn(async move {
                    let note = match tokio::time::timeout(gate::ANSWER_WAIT, rx).await {
                        Ok(Ok(Choice::Deny)) => {
                            audit_line(&agent, &w, &action, "denied", json!({ "by": "user" }));
                            format!("[AgentMux] The user said no to you controlling {app}. Don't ask again this session.")
                        }
                        Ok(Ok(choice)) => {
                            audit_line(&agent, &w, &action, "granted", json!({ "choice": choice }));
                            let how_long = if choice == Choice::Timed {
                                format!("for {} minutes", p.grant_minutes)
                            } else {
                                "for this session".to_string()
                            };
                            format!("[AgentMux] The user allowed you to control {app} {how_long}. Call Desktop again to carry on: look first, since the window may have changed.")
                        }
                        // The prompt was dropped without an answer: Stop.
                        Ok(Err(_)) => format!("[AgentMux] The user pressed Stop on {app}. Don't carry on with it; ask them what they want."),
                        Err(_) => {
                            gate::with_gate(|g| g.abandon(&id));
                            audit_line(&agent, &w, &action, "no_answer", Value::Null);
                            format!("[AgentMux] The user didn't answer your request to control {app} in 10 minutes, so nothing was done. Ask them in chat before asking again.")
                        }
                    };
                    publish(&state);
                    tell(&block_id, &note);
                });
                return reply("pending", &message);
            }
        }
    }
    reply("refused", "Couldn't get a slot to act; try again.")
}

/// What a call that asked the user (or found them already asked) answers.
fn pending_message(app: &str) -> String {
    format!(
        "Asked the user to let you control {app}; nothing was done yet. Don't wait for it: carry on with \
         other work or end your turn. An [AgentMux] note brings their answer, then call Desktop again."
    )
}

/// `POST /api/v1/agent/desktop/record`: what an authorized action did.
pub(crate) async fn handle_record(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<DesktopRecordRequest>,
) -> Response {
    if let Err(r) = caller_block(&state, caller.as_ref(), &req.auth) {
        return r;
    }
    gate::audit(&json!({
        "ts_ms": now_ms_u64(),
        "agent": req.auth.agent_id,
        "app": gate::app_key(&req.window.app),
        "window": req.window.id,
        "action": req.action,
        "grant": req.grant_id,
        "result": req.result,
        "details": req.details,
    }));
    publish(&state);
    Json(json!({ "ok": true })).into_response()
}

/// `POST /api/v1/agent/desktop/release`: the agent gives its grant back.
pub(crate) async fn handle_release(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<DesktopReleaseRequest>,
) -> Response {
    let block_id = match caller_block(&state, caller.as_ref(), &req.auth) {
        Ok(b) => b,
        Err(r) => return r,
    };
    let app = gate::app_key(&req.window.app);
    let released = gate::with_gate(|g| g.release(&req.auth.agent_id, &block_id, &app));
    if released {
        audit_line(&req.auth.agent_id, &req.window, "release", "released", Value::Null);
        publish(&state);
    }
    Json(json!({ "released": released })).into_response()
}

#[derive(Debug, serde::Deserialize)]
pub(crate) struct HostDesktopStop {
    block_id: String,
}

fn host_only() -> Response {
    (StatusCode::FORBIDDEN, Json(json!({ "ok": false, "error": "only the AgentMux host can answer a desktop request" }))).into_response()
}

/// `POST /api/v1/host/desktop_request`: the user's answer to a prompt, relayed
/// by the CEF host with its own token. The instance auth key, which every
/// agent has, isn't enough.
pub(crate) async fn handle_host_request(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<gate::DesktopAnswer>,
) -> Response {
    if !crate::server::ui_handlers::from_host(&state, &headers).await {
        return host_only();
    }
    let minutes = policy(&state).grant_minutes;
    match gate::with_gate(|g| g.answer(&body.id, body.choice, now_ms_u64(), minutes)) {
        Ok(()) => {
            publish(&state);
            Json(json!({ "ok": true })).into_response()
        }
        Err(e) => (StatusCode::CONFLICT, Json(json!({ "ok": false, "error": e }))).into_response(),
    }
}

/// `POST /api/v1/host/desktop_stop`: the user's Stop on the agent in a pane.
pub(crate) async fn handle_host_stop(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<HostDesktopStop>,
) -> Response {
    if !crate::server::ui_handlers::from_host(&state, &headers).await {
        return host_only();
    }
    let revoked = gate::with_gate(|g| g.stop(&body.block_id));
    publish(&state);
    gate::audit(&json!({ "ts_ms": now_ms_u64(), "action": "stop", "decision": "stopped", "block": body.block_id, "revoked": revoked }));
    tell(&body.block_id, "[AgentMux] The user pressed Stop on your desktop control. Your grants are gone; ask them before controlling an app again.");
    Json(json!({ "ok": true, "revoked": revoked })).into_response()
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use serde_json::json;
    use tower::ServiceExt;

    async fn post(app: &axum::Router, uri: &str, host_token: Option<&str>, body: serde_json::Value) -> (StatusCode, serde_json::Value) {
        let mut b = Request::builder()
            .uri(uri)
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

    async fn router() -> axum::Router {
        let state = crate::server::tests::test_state();
        *state.host_ipc.lock().await = Some(crate::server::state::HostIpc { port: 1, token: "host-ipc-token".into() });
        crate::server::build_router(state)
    }

    // §5.4: the user's answer and Stop come only through the host. The
    // instance auth key, which every agent has, isn't enough.
    #[tokio::test]
    async fn only_the_host_can_answer_a_prompt_or_press_stop() {
        let app = router().await;
        for (uri, body) in [
            ("/api/v1/host/desktop_request", json!({ "id": "r1", "choice": "timed" })),
            ("/api/v1/host/desktop_stop", json!({ "block_id": "b1" })),
        ] {
            let (s, out) = post(&app, uri, None, body.clone()).await;
            assert_eq!(s, StatusCode::FORBIDDEN, "{uri}: {out}");
            let (s, _) = post(&app, uri, Some("not-the-token"), body).await;
            assert_eq!(s, StatusCode::FORBIDDEN, "{uri}");
        }
    }

    #[tokio::test]
    async fn the_host_gets_through_and_an_unknown_prompt_is_a_conflict() {
        let app = router().await;
        let (s, out) = post(&app, "/api/v1/host/desktop_request", Some("host-ipc-token"), json!({ "id": "nope", "choice": "timed" })).await;
        assert_eq!(s, StatusCode::CONFLICT, "{out}");
        let (s, out) = post(&app, "/api/v1/host/desktop_stop", Some("host-ipc-token"), json!({ "block_id": "no-such-pane" })).await;
        assert_eq!(s, StatusCode::OK, "{out}");
        assert_eq!(out["revoked"], 0);
    }

    // An agent whose signature doesn't check out never reaches the gate.
    #[tokio::test]
    async fn an_agent_route_without_a_valid_signature_is_refused() {
        let app = router().await;
        let window = json!({ "id": 1, "app": "notepad.exe", "title": "t" });
        for (uri, body) in [
            ("/api/v1/agent/desktop/authorize", json!({ "agent_id": "x", "ts_secs": 1, "sig": "bad", "action": "look", "window": window })),
            ("/api/v1/agent/desktop/policy", json!({ "agent_id": "x", "ts_secs": 1, "sig": "bad", "windows": [] })),
            ("/api/v1/agent/desktop/release", json!({ "agent_id": "x", "ts_secs": 1, "sig": "bad", "window": window })),
        ] {
            let (s, out) = post(&app, uri, None, body).await;
            assert_eq!(s, StatusCode::UNAUTHORIZED, "{uri}: {out}");
        }
    }
}
