// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The routes of Bypass (docs/specs/SPEC_BROWSER_PANE_BYPASS_2026_10_16.md):
//! an agent's `request_bypass` and `end_bypass`, the person's Bypass button
//! and End bypass through the host, and the check that lets a committing
//! click through without asking. The grants themselves are `browser_bypass`.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;

use crate::server::browser_bypass as bypass;
use crate::server::ui_handlers::{
    banner_for, err_response, from_host, get_host_ipc, owning_agent, proxy_to_host_timeout, proxy_value, target_block_id,
};
use crate::server::AppState;

/// The pane a bypass for `target` is recorded on: the pane itself, or for
/// a popup window (which has no AgentMux header of its own) its opener. A
/// popup pane has its own header, so it has its own grant (bypass spec §4.2).
fn bypass_pane(target: &str) -> String {
    banner_for(target).0
}

/// The address `target` (a pane or popup window) shows, as the host reports
/// it from the page itself: never block meta, which a client can write.
async fn page_url(state: &AppState, target: &str) -> Result<String, String> {
    let host = get_host_ipc(state).await?;
    let script = json!({ "block_id": target, "script": "location.href", "await_promise": false });
    let v = proxy_to_host_timeout(state, &host, "eval", script, None).await?;
    v.pointer("/data/result")
        .and_then(|r| r.as_str())
        .map(str::to_string)
        .ok_or_else(|| "couldn't read the page's address".to_string())
}

/// The click `ask` describes (a committing click's `needs_approval`), made
/// without the person's approval if they granted `agent` a bypass on the
/// site the page is on (bypass spec §4.1). `approved` is the act request to
/// send for it. `page` is the address the host read from the page itself. A
/// payment still asks (§11.2). None: ask as usual.
pub(crate) async fn bypassed_click(
    state: &AppState,
    block_id: &str,
    agent: &str,
    ask: &serde_json::Value,
    approved: serde_json::Value,
) -> Option<Response> {
    let g = bypass::active_for(&bypass_pane(block_id), agent)?;
    let what = ask.get("what").and_then(|v| v.as_str()).unwrap_or("");
    let page = ask.get("page").and_then(|v| v.as_str()).unwrap_or("");
    let origin = agentmux_common::allowed_origins::target_origin(page).unwrap_or_default();
    if !g.covers(page) {
        tracing::info!(agent_id = %agent, block_id = %block_id, origin = %origin, "[browser-bypass] paused: the page is off the granted sites, so the click asks");
        return None;
    }
    if bypass::is_payment(what) {
        tracing::info!(agent_id = %agent, block_id = %block_id, what = %what, "[browser-bypass] a payment still asks");
        return None;
    }
    tracing::info!(
        agent_id = %agent, block_id = %block_id, what = %what, origin = %origin,
        "[browser-bypass] committing click made without asking"
    );
    // The page may confirm it while the click is still running.
    crate::server::browser_dialog::note_click(block_id, what, crate::server::browser_dialog::How::Auto);
    Some(match proxy_value(state, "act", approved, std::time::Duration::from_secs(20)).await {
        Ok(mut data) => {
            if let Some(o) = data.as_object_mut() {
                o.insert("bypassed".to_string(), json!(true));
                o.insert("bypass_minutes_left".to_string(), json!(g.minutes_left()));
            }
            (StatusCode::OK, Json(json!({ "ok": true, "data": data }))).into_response()
        }
        Err((code, e)) => err_response(code, e),
    })
}

/// `POST /api/v1/ui/browser/request_bypass` — backs Browser's `request_bypass`
/// (SPEC_BROWSER_PANE_BYPASS_2026_10_16.md §6): a banner asks the person to
/// let this agent act without asking first, on the site the pane shows, for a
/// while; the call waits for their answer. Only their click in the pane,
/// relayed by the host, grants it.
pub(crate) async fn handle_ui_browser_request_bypass(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<agentmux_common::api_types::UiBrowserRequestBypassRequest>,
) -> impl IntoResponse {
    let agent = req.auth.agent_id.clone();
    let block_id = match target_block_id(&state, caller.as_deref(), &req.auth, req.pane.as_deref()) {
        Ok(b) => b,
        Err((code, e)) => return err_response(code, e),
    };
    let pane = bypass_pane(&block_id);
    let granted = |g: &bypass::Grant| {
        (
            StatusCode::OK,
            Json(json!({ "ok": true, "data": {
                "answer": "granted", "origins": g.origins, "minutes": g.minutes,
                "minutes_left": g.minutes_left(), "expires_at": g.expires_at_ms,
            } })),
        )
            .into_response()
    };
    if let Some(g) = bypass::active_for(&pane, &agent) {
        return granted(&g);
    }
    let (banner_at, in_window) = banner_for(&block_id);
    let is_browser = state
        .mstore
        .get::<crate::backend::obj::Block>(&banner_at)
        .ok()
        .flatten()
        .is_some_and(|b| b.meta.get("view").and_then(|v| v.as_str()) == Some("browser"));
    if !is_browser || crate::server::browser_owner::owner_of(&pane).is_none() {
        return err_response(
            StatusCode::BAD_REQUEST,
            "Browser's request_auto_approve action needs a browser pane you opened: pass the `pane` its open action returned".to_string(),
        );
    }
    if let Some(left) = bypass::cooling_down(&pane) {
        return err_response(
            StatusCode::TOO_MANY_REQUESTS,
            format!(
                "the user said No to auto-approve on this pane; don't ask again for {} more minutes. Carry on with \
                 per-click approvals, or ask them in chat",
                left.as_secs().div_ceil(60)
            ),
        );
    }
    let reason: String = req.reason.trim().chars().take(300).collect();
    if reason.is_empty() {
        return err_response(StatusCode::BAD_REQUEST, "say what you need it for (`reason`)".to_string());
    }
    if let Err(e) = crate::server::app_api::connections::host_to_ask_user(&state).await {
        return err_response(StatusCode::SERVICE_UNAVAILABLE, e);
    }
    let url = match page_url(&state, &block_id).await {
        Ok(u) => u,
        Err(e) => return err_response(StatusCode::BAD_GATEWAY, e),
    };
    let origins = match bypass::origins_for(&pane, &url) {
        Ok(o) => o,
        Err(e) => return err_response(StatusCode::BAD_REQUEST, e),
    };
    let minutes = bypass::clamp_minutes(req.minutes);
    let owner_before = crate::server::browser_owner::owner_of(&pane);
    tracing::info!(agent_id = %agent, pane = %pane, origins = ?origins, minutes, "[browser-bypass] asking the user");
    let (answer, _waiting) = match crate::server::browser_attention::ask(
        &state,
        &banner_at,
        &agent,
        crate::server::browser_attention::Kind::Bypass,
        json!({ "reason": reason, "origins": origins, "minutes": minutes, "window": in_window }),
        std::time::Duration::from_secs(10 * 60),
    )
    .await
    {
        Ok(a) => a,
        Err(e) => return err_response(e.status(), e.to_string()),
    };
    use crate::server::browser_attention::Answer;
    let answer = match answer {
        // A Take over while the banner was up wins over an Allow after it.
        Answer::Yes if crate::server::browser_owner::owner_of(&pane) != owner_before => {
            return err_response(StatusCode::FORBIDDEN, "the user took over this pane, so auto-approve wasn't turned on".to_string());
        }
        Answer::Yes => return granted(&bypass::grant(&state, &pane, &agent, origins, minutes)),
        Answer::Cancelled => {
            bypass::note_declined(&pane);
            tracing::info!(agent_id = %agent, pane = %pane, "[browser-bypass] the user said No");
            "declined"
        }
        Answer::TimedOut => "timed_out",
    };
    (StatusCode::OK, Json(json!({ "ok": true, "data": { "answer": answer } }))).into_response()
}

/// `POST /api/v1/ui/browser/end_bypass` — backs Browser's `end_bypass`: an
/// agent ends its own bypass when its job is done. An agent can end a grant,
/// never start one.
pub(crate) async fn handle_ui_browser_end_bypass(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<agentmux_common::api_types::UiBrowserEndBypassRequest>,
) -> impl IntoResponse {
    let block_id = match target_block_id(&state, caller.as_deref(), &req.auth, req.pane.as_deref()) {
        Ok(b) => b,
        Err((code, e)) => return err_response(code, e),
    };
    let pane = bypass_pane(&block_id);
    let ended = bypass::active_for(&pane, &req.auth.agent_id).is_some() && bypass::end(&state, &pane, "the agent ended it");
    (StatusCode::OK, Json(json!({ "ok": true, "data": { "ended": ended } }))).into_response()
}

/// `POST /api/v1/host/browser_bypass` — the person's Bypass button and End
/// bypass, relayed by the CEF host (bypass spec §5). Only the host can call
/// it: an agent holding the instance auth key gets 403, so no agent can grant
/// itself a bypass. Body `{block_id, on, minutes?, agent, origins}`: `agent`
/// and `origins` are what the confirmation showed; the grant is made only if
/// they are still true.
pub(crate) async fn handle_host_browser_bypass(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    Json(body): Json<serde_json::Value>,
) -> impl IntoResponse {
    if !from_host(&state, &headers).await {
        return err_response(StatusCode::FORBIDDEN, "only the AgentMux host can turn auto-approve on or off".to_string());
    }
    let s = |k: &str| body.get(k).and_then(|v| v.as_str()).unwrap_or("").trim().to_string();
    let block_id = s("block_id");
    let pane = bypass_pane(&block_id);
    if body.get("on").and_then(|v| v.as_bool()) != Some(true) {
        let ended = bypass::end(&state, &pane, "the user ended it");
        return (StatusCode::OK, Json(json!({ "ok": true, "data": { "ended": ended } }))).into_response();
    }
    let block = match state.mstore.get::<crate::backend::obj::Block>(&pane) {
        Ok(Some(b)) => b,
        _ => return err_response(StatusCode::NOT_FOUND, "the pane is gone".to_string()),
    };
    let Some(owner) = owning_agent(&block, &pane) else {
        return err_response(StatusCode::CONFLICT, "no agent drives this pane any more".to_string());
    };
    if !owner.eq_ignore_ascii_case(&s("agent")) {
        return err_response(StatusCode::CONFLICT, format!("{owner} drives this pane now, not who you were shown; look again"));
    }
    if crate::server::browser_attention::waiting_on_user(&pane).is_some() {
        return err_response(StatusCode::CONFLICT, "the pane is waiting for your answer to its banner; answer that first".to_string());
    }
    let url = match page_url(&state, &pane).await {
        Ok(u) => u,
        Err(e) => return err_response(StatusCode::BAD_GATEWAY, e),
    };
    let origins = match bypass::origins_for(&pane, &url) {
        Ok(o) => o,
        Err(e) => return err_response(StatusCode::BAD_REQUEST, e),
    };
    let shown: Vec<String> = body
        .get("origins")
        .and_then(|v| v.as_array())
        .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_string)).collect())
        .unwrap_or_default();
    let shown = agentmux_common::allowed_origins::parse_list(&shown).unwrap_or_default();
    if shown != origins {
        return err_response(
            StatusCode::CONFLICT,
            format!("the page is on {} now, not what you were shown; look again", origins.join(", ")),
        );
    }
    let minutes = bypass::clamp_minutes(body.get("minutes").and_then(|v| v.as_u64()));
    let g = bypass::grant(&state, &pane, &owner, origins, minutes);
    (
        StatusCode::OK,
        Json(json!({ "ok": true, "data": { "origins": g.origins, "minutes": g.minutes, "expires_at": g.expires_at_ms } })),
    )
        .into_response()
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_pane_holds_its_own_grant() {
        // A popup pane (or any pane) is its own: only a popup window, which
        // has no header, is covered by its opener's grant.
        assert_eq!(super::bypass_pane("a-pane-id"), "a-pane-id");
    }
}
