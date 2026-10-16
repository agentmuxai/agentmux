// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `app.update-status`: what srv knows about newer AgentMux releases
//! (`backend::update_check`), for a window that opens after the last
//! `app:updatestatus` event. `app.update-download`: download and stage the
//! release it found (`backend::update_download`); progress follows as events.
//! `app.upgrade-restart` / `app.upgrade-cancel`: restart into it once agents
//! are idle, or stop waiting (`backend::update_restart`).
//! `app.upgrade-requests` / `app.upgrade-request-answer`: agents' requests
//! waiting for the user, and the user's answer (`backend::upgrade_request`).
//! And the agent's side over HTTP: the `App` tool's `version`
//! (`GET /api/v1/app/update-status`) and `upgrade`
//! (`POST /api/v1/agent/self/upgrade`).

use super::*;

pub fn register(engine: &Arc<WshRpcEngine>, state: &AppState) {
    engine.register_typed(COMMAND_APP_UPDATE_STATUS, move |_req: Option<NoArgsReq>, _ctx| async move {
        Ok::<_, String>(crate::backend::update_check::current_status())
    });
    // The user's click only: an agent asks through RequestUpgrade, which
    // needs the user's say-so (the spec's §5.1).
    engine.register_typed(COMMAND_APP_UPDATE_DOWNLOAD, move |_req: Option<NoArgsReq>, ctx| async move {
        if !ctx.agent_id.is_empty() {
            return Err("FORBIDDEN: only the user starts downloading an AgentMux release".to_string());
        }
        crate::backend::update_download::start()
    });
    engine.register_typed(
        COMMAND_APP_UPGRADE_RESTART,
        move |req: crate::backend::update_restart::CommandAppUpgradeRestartData, ctx| async move {
            if !ctx.agent_id.is_empty() {
                return Err("FORBIDDEN: only the user restarts AgentMux to upgrade it".to_string());
            }
            crate::backend::update_restart::start(req.now)
        },
    );
    engine.register_typed(COMMAND_APP_UPGRADE_CANCEL, move |_req: Option<NoArgsReq>, ctx| async move {
        if !ctx.agent_id.is_empty() {
            return Err("FORBIDDEN: only the user cancels an AgentMux upgrade".to_string());
        }
        crate::backend::update_restart::cancel()
    });
    engine.register_typed(COMMAND_APP_UPGRADE_REQUESTS, move |_req: Option<NoArgsReq>, _ctx| async move {
        Ok::<_, String>(crate::backend::upgrade_request::UpgradeRequestsEvent {
            requests: crate::backend::upgrade_request::list(),
        })
    });
    let answer_state = state.clone();
    engine.register_typed(
        COMMAND_APP_UPGRADE_REQUEST_ANSWER,
        move |req: crate::backend::upgrade_request::CommandAppUpgradeRequestAnswerData, ctx| {
            let state = answer_state.clone();
            async move {
                if !ctx.agent_id.is_empty() {
                    return Err("FORBIDDEN: only the user answers an agent's upgrade request".to_string());
                }
                crate::backend::upgrade_request::answer(&state, &req.id, req.accept)?;
                Ok(crate::backend::upgrade_request::UpgradeRequestsEvent {
                    requests: crate::backend::upgrade_request::list(),
                })
            }
        },
    );
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
            upgrade_request::start_upgrade();
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
