// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `POST /api/v1/ui/browser/retake` and `/handoff_status`: an agent drives again a pane the person took over once they
//! gave it back, and asks what became of a hand-off whose call it lost
//! (docs/specs/SPEC_BROWSER_RETAKE_AND_COMMANDEER_2026_11_04.md §1). Neither goes through `target_block_id`: a retake is
//! asked for a pane the agent no longer owns, so the rule is `browser_owner::may_retake`, and the caller's identity is
//! checked the same way every agent browser call checks it.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde_json::json;

use crate::server::browser_owner;
use crate::server::ui_handlers::{err_response, verified_block_id};
use crate::server::AppState;

fn trimmed(v: &Option<String>) -> Option<&str> {
    v.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

pub(crate) async fn handle_ui_browser_retake(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<agentmux_common::api_types::UiBrowserRetakeRequest>,
) -> impl IntoResponse {
    if let Err(e) = verified_block_id(&state, caller.as_deref(), &req.auth) {
        return err_response(StatusCode::UNAUTHORIZED, e);
    }
    let agent = req.auth.agent_id.as_str();
    let Some(pane) = trimmed(&req.pane) else {
        return err_response(StatusCode::BAD_REQUEST, "Browser retake needs pane: the id of the pane the user took over".to_string());
    };
    let block = match state.mstore.get::<crate::backend::obj::Block>(pane) {
        Ok(Some(b)) => b,
        Ok(None) => return err_response(StatusCode::NOT_FOUND, format!("no pane {pane:?} (it may have been closed)")),
        Err(e) => return err_response(StatusCode::INTERNAL_SERVER_ERROR, format!("load pane {pane:?}: {e}")),
    };
    if block.meta.get("view").and_then(|v| v.as_str()) != Some("browser") {
        return err_response(StatusCode::BAD_REQUEST, format!("pane {pane:?} is not a browser pane"));
    }
    match browser_owner::owner_of(pane) {
        Some(owner) if owner.eq_ignore_ascii_case(agent) => {
            return (StatusCode::OK, Json(json!({ "ok": true, "data": { "pane": pane, "already": true } }))).into_response();
        }
        Some(_) => {
            return err_response(
                StatusCode::FORBIDDEN,
                format!("pane {pane:?} is a browser pane another agent drives; you can retake only a pane you lost to the user"),
            );
        }
        None => {}
    }
    if let Err(e) = browser_owner::may_retake(pane, agent) {
        return err_response(StatusCode::CONFLICT, e);
    }
    // The header reads these keys: written before the agent drives, so the person never sees an agent driving unlabelled.
    if let Err(e) = crate::server::http_shell::broadcast_meta_update(&state, pane, &browser_owner::driver_meta(agent)) {
        return err_response(StatusCode::INTERNAL_SERVER_ERROR, format!("couldn't show you as the pane's driver: {e}"));
    }
    browser_owner::record(pane, agent);
    tracing::info!(agent_id = %agent, pane = %pane, "[browser-owner] agent retook the pane");
    (StatusCode::OK, Json(json!({ "ok": true, "data": { "pane": pane, "already": false } }))).into_response()
}

pub(crate) async fn handle_ui_browser_handoff_status(
    State(state): State<AppState>,
    caller: Option<axum::Extension<crate::server::caller::Caller>>,
    Json(req): Json<agentmux_common::api_types::UiBrowserHandoffStatusRequest>,
) -> impl IntoResponse {
    if let Err(e) = verified_block_id(&state, caller.as_deref(), &req.auth) {
        return err_response(StatusCode::UNAUTHORIZED, e);
    }
    let (id, pane) = (trimmed(&req.handoff_id), trimmed(&req.pane));
    if id.is_none() && pane.is_none() {
        return err_response(
            StatusCode::BAD_REQUEST,
            "Browser handoff_status needs handoff_id (the hand-off's answer carries it) or pane".to_string(),
        );
    }
    match crate::server::browser_handoffs::status(&req.auth.agent_id, id, pane) {
        Some(s) => {
            let data = json!({
                "handoff_id": s.id,
                "pane": s.pane,
                "status": s.state.as_str(),
                "started_secs_ago": s.started_secs_ago,
                "ended_secs_ago": s.ended_secs_ago,
            });
            (StatusCode::OK, Json(json!({ "ok": true, "data": data }))).into_response()
        }
        None => err_response(StatusCode::NOT_FOUND, "no hand-off of yours matches that (they are kept for 24 hours)".to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::obj::MetaMapType;
    use crate::server::tests::test_state;
    use agentmux_common::api_types::{UiAutomationAuth, UiBrowserRetakeRequest};

    fn auth(state: &AppState, agent_id: &str) -> UiAutomationAuth {
        let key = state.mstore.agent_jekt_key_ensure(agent_id).unwrap();
        let ts_secs = agentmux_common::time::now_secs();
        let sig = agentmux_common::jekt_sign::sign_jekt(&key, "ui-automation-identity", agent_id, "__srv__", ts_secs, "");
        UiAutomationAuth { agent_id: agent_id.to_string(), ts_secs, sig }
    }

    async fn retake_status(state: &AppState, agent: &str, pane: &str) -> StatusCode {
        let req = UiBrowserRetakeRequest { auth: auth(state, agent), pane: Some(pane.to_string()) };
        handle_ui_browser_retake(State(state.clone()), None, Json(req)).await.into_response().status()
    }

    fn clear(key: &str) -> MetaMapType {
        let mut m = MetaMapType::new();
        m.insert(key.to_string(), serde_json::Value::Null);
        m
    }

    #[tokio::test]
    async fn a_taken_over_pane_is_retaken_only_after_give_back() {
        let state = test_state();
        let own = format!("own-{}", uuid::Uuid::new_v4());
        let pane = format!("pane-{}", uuid::Uuid::new_v4());
        crate::server::browser_attention::tests::browser_block(&state, &own);
        crate::server::browser_attention::tests::browser_block(&state, &pane);
        let agent = format!("retake-{}", uuid::Uuid::new_v4());
        state.reactive_handler.register_agent(&agent, &own, None).unwrap();

        // Never driven by this agent: refused.
        assert_eq!(retake_status(&state, &agent, &pane).await, StatusCode::CONFLICT);

        // The agent drives it, the person takes it over: refused until they give it back.
        browser_owner::record(&pane, &agent);
        let oref = format!("block:{pane}");
        browser_owner::guard_client_meta_write(&oref, &clear(browser_owner::OWNER_META_KEY)).unwrap();
        assert_eq!(retake_status(&state, &agent, &pane).await, StatusCode::CONFLICT, "the person is still using it");
        assert!(browser_owner::owner_of(&pane).is_none());

        browser_owner::guard_client_meta_write(&oref, &clear(browser_owner::LAST_DRIVER_META_KEY)).unwrap();
        assert_eq!(retake_status(&state, &agent, &pane).await, StatusCode::OK);
        assert_eq!(browser_owner::owner_of(&pane).as_deref(), Some(agent.as_str()));
        let block = state.mstore.must_get::<crate::backend::obj::Block>(&pane).unwrap();
        assert_eq!(block.meta.get(browser_owner::OWNER_META_KEY).and_then(|v| v.as_str()), Some(agent.as_str()));
        assert_eq!(block.meta.get(browser_owner::LAST_DRIVER_META_KEY).and_then(|v| v.as_str()), Some(agent.as_str()));

        // Already driving it: fine, and nothing changes.
        assert_eq!(retake_status(&state, &agent, &pane).await, StatusCode::OK);
        browser_owner::forget(&pane);
    }

    #[tokio::test]
    async fn another_agent_cannot_retake_and_a_driven_pane_is_not_retaken() {
        let state = test_state();
        let (own_a, own_b) = (format!("own-{}", uuid::Uuid::new_v4()), format!("own-{}", uuid::Uuid::new_v4()));
        let pane = format!("pane-{}", uuid::Uuid::new_v4());
        for b in [&own_a, &own_b, &pane] {
            crate::server::browser_attention::tests::browser_block(&state, b);
        }
        let (a, b) = (format!("retake-a-{}", uuid::Uuid::new_v4()), format!("retake-b-{}", uuid::Uuid::new_v4()));
        state.reactive_handler.register_agent(&a, &own_a, None).unwrap();
        state.reactive_handler.register_agent(&b, &own_b, None).unwrap();

        browser_owner::record(&pane, &a);
        assert_eq!(retake_status(&state, &b, &pane).await, StatusCode::FORBIDDEN, "a pane another agent drives");
        let oref = format!("block:{pane}");
        browser_owner::guard_client_meta_write(&oref, &clear(browser_owner::OWNER_META_KEY)).unwrap();
        browser_owner::guard_client_meta_write(&oref, &clear(browser_owner::LAST_DRIVER_META_KEY)).unwrap();
        assert_eq!(retake_status(&state, &b, &pane).await, StatusCode::CONFLICT, "given back to a, not to b");
        assert_eq!(retake_status(&state, &a, "no-such-pane").await, StatusCode::NOT_FOUND);
    }

    #[test]
    fn a_client_cannot_set_the_last_driver() {
        let mut m = MetaMapType::new();
        m.insert(browser_owner::LAST_DRIVER_META_KEY.to_string(), json!("someone"));
        assert!(browser_owner::guard_client_meta_write("block:b1", &m).is_err());
    }
}
