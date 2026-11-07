// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `POST /api/v1/ui/browser/retake` and `/handoff_status`: an agent drives again a pane the person took over once they
//! gave it back, and asks what became of a hand-off whose call it lost
//! (docs/specs/SPEC_BROWSER_RETAKE_AND_COMMANDEER_2026_11_04.md §1). Neither goes through `target_block_id`: a retake is
//! asked for a pane the agent no longer owns, so the rule is `browser_owner::may_retake`, and the caller's identity is
//! checked the same way every agent browser call checks it. The person's Give back arrives from the host
//! (`give_back`, called by `handle_host_browser_attention`), never from a client.

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
    // The person's Give back is not the only way back: a pane they have left alone for a minute is the agent's again.
    // Asked only when that is what decides, so a pane given back or never lost costs no host call.
    let since_input = if browser_owner::takeover_in_use(&block, pane, agent).is_some() {
        since_person_input(&state, pane).await
    } else {
        None
    };
    if let Err(e) = browser_owner::may_retake(&block, pane, agent, since_input) {
        return err_response(StatusCode::CONFLICT, e);
    }
    // The person's Take over ended the pane's site limit; the pane's header still names it, so the limit comes back
    // with the agent, before it drives.
    if let Some(list) = limit_of(&block) {
        crate::server::browser_allowlist::limit_new_pane(&state, pane, Some(list)).await;
    }
    // The header reads these keys: written before the agent drives, so the person never sees an agent driving unlabelled.
    if let Err(e) = crate::server::http_shell::broadcast_meta_update(&state, pane, &browser_owner::retaken_meta(agent)) {
        return err_response(StatusCode::INTERNAL_SERVER_ERROR, format!("couldn't show you as the pane's driver: {e}"));
    }
    browser_owner::record(pane, agent);
    tracing::info!(agent_id = %agent, pane = %pane, "[browser-owner] agent retook the pane");
    (StatusCode::OK, Json(json!({ "ok": true, "data": { "pane": pane, "already": false } }))).into_response()
}

/// How long ago the host last saw the person use `pane`'s page (`Duration::MAX` if it never has), or None when the host
/// cannot say, which keeps the pane the person's until they give it back.
async fn since_person_input(state: &AppState, pane: &str) -> Option<std::time::Duration> {
    let host = crate::server::ui_handlers::get_host_ipc(state).await.ok()?;
    // Not `post_to_host_awake`: a sleeping pane has no page to use, and asking must not wake it.
    let answer = crate::server::host_http::post_to_host(
        &state.http_client,
        &host,
        "last_input",
        json!({ "block_id": pane }),
        Some(std::time::Duration::from_secs(5)),
    )
    .await
    .ok()?;
    if answer.get("ok").and_then(|v| v.as_bool()) != Some(true) {
        return None;
    }
    let idle = answer.pointer("/data/idle_secs")?;
    Some(idle.as_u64().map_or(std::time::Duration::MAX, std::time::Duration::from_secs))
}

/// The sites the pane was limited to, as its header recorded them (`browser:allowed_origins`).
fn limit_of(block: &crate::backend::obj::Block) -> Option<Vec<String>> {
    let list = block.meta.get(crate::server::browser_allowlist::ALLOWED_META_KEY)?.as_array()?;
    Some(list.iter().filter_map(|o| o.as_str().map(str::to_string)).collect())
}

/// The person's Give back on `block_id`, relayed by the host: the agent that drove the pane last may retake it. Written
/// to the block's meta too, so it holds after an srv restart.
pub(crate) fn give_back(state: &AppState, block_id: &str) -> axum::response::Response {
    let block = match state.mstore.get::<crate::backend::obj::Block>(block_id) {
        Ok(Some(b)) => b,
        _ => return err_response(StatusCode::NOT_FOUND, format!("no pane {block_id:?}")),
    };
    let Some(agent) = block.meta.get(browser_owner::LAST_DRIVER_META_KEY).and_then(|v| v.as_str()) else {
        return err_response(StatusCode::NOT_FOUND, "no agent drove that pane".to_string());
    };
    if browser_owner::owner_of(block_id).is_some() {
        return err_response(StatusCode::CONFLICT, "an agent drives that pane: Take over comes first".to_string());
    }
    let mut meta = crate::backend::obj::MetaMapType::new();
    meta.insert(browser_owner::GIVEN_BACK_META_KEY.to_string(), json!(agent));
    if let Err(e) = crate::server::http_shell::broadcast_meta_update(state, block_id, &meta) {
        return err_response(StatusCode::INTERNAL_SERVER_ERROR, format!("couldn't give the pane back: {e}"));
    }
    browser_owner::mark_given_back(block_id);
    tracing::info!(pane = %block_id, agent_id = %agent, "[browser-owner] the person gave the pane back");
    (StatusCode::OK, Json(json!({ "ok": true }))).into_response()
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
        let own = uuid::Uuid::new_v4().to_string();
        let pane = uuid::Uuid::new_v4().to_string();
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

        // The person's Give back arrives from the host (not a client write): it needs an agent that drove the pane.
        let mut driver = MetaMapType::new();
        driver.insert(browser_owner::LAST_DRIVER_META_KEY.to_string(), json!(agent));
        crate::server::http_shell::broadcast_meta_update(&state, &pane, &driver).unwrap();
        assert_eq!(give_back(&state, &pane).status(), StatusCode::OK);
        assert_eq!(retake_status(&state, &agent, &pane).await, StatusCode::OK);
        assert_eq!(browser_owner::owner_of(&pane).as_deref(), Some(agent.as_str()));
        let block = state.mstore.must_get::<crate::backend::obj::Block>(&pane).unwrap();
        assert_eq!(block.meta.get(browser_owner::OWNER_META_KEY).and_then(|v| v.as_str()), Some(agent.as_str()));
        assert_eq!(block.meta.get(browser_owner::LAST_DRIVER_META_KEY).and_then(|v| v.as_str()), Some(agent.as_str()));

        // Already driving it: fine, and nothing changes.
        assert_eq!(retake_status(&state, &agent, &pane).await, StatusCode::OK);
        browser_owner::forget(&pane);
    }

    /// A host that answers `last_input` with `idle_secs` (null: the person never used the page), on a port of its own.
    async fn fake_host(state: &AppState, idle_secs: Option<u64>) {
        use axum::{routing::post, Router};
        let app = Router::new().route(
            "/agentmux/browser/last_input",
            post(move |Json(_b): Json<serde_json::Value>| async move {
                Json(json!({ "ok": true, "data": { "idle_secs": idle_secs } }))
            }),
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        tokio::spawn(async move { axum::serve(listener, app).await.unwrap() });
        *state.host_ipc.lock().await = Some(crate::server::state::HostIpc { port, token: "host-ipc-token".into() });
    }

    #[tokio::test]
    async fn a_pane_the_person_has_left_alone_for_a_minute_is_retaken_without_give_back() {
        let state = test_state();
        let own = uuid::Uuid::new_v4().to_string();
        let pane = uuid::Uuid::new_v4().to_string();
        crate::server::browser_attention::tests::browser_block(&state, &own);
        crate::server::browser_attention::tests::browser_block(&state, &pane);
        let agent = format!("retake-{}", uuid::Uuid::new_v4());
        state.reactive_handler.register_agent(&agent, &own, None).unwrap();
        browser_owner::record(&pane, &agent);
        browser_owner::guard_client_meta_write(&format!("block:{pane}"), &clear(browser_owner::OWNER_META_KEY)).unwrap();

        // No host to ask: only Give back counts.
        assert_eq!(retake_status(&state, &agent, &pane).await, StatusCode::CONFLICT, "the host cannot say");

        // The person typed 5 s ago: still theirs, even though they took it over long ago.
        browser_owner::age_lost(&pane, std::time::Duration::from_secs(600));
        fake_host(&state, Some(5)).await;
        assert_eq!(retake_status(&state, &agent, &pane).await, StatusCode::CONFLICT, "typed 5 s ago");

        // Idle 120 s, and the Take over was longer ago still: the agent's again.
        fake_host(&state, Some(120)).await;
        assert_eq!(retake_status(&state, &agent, &pane).await, StatusCode::OK, "idle 120 s");
        assert_eq!(browser_owner::owner_of(&pane).as_deref(), Some(agent.as_str()));
        browser_owner::forget(&pane);

        // Never used the page since the host started: counts from the Take over click.
        browser_owner::record(&pane, &agent);
        browser_owner::guard_client_meta_write(&format!("block:{pane}"), &clear(browser_owner::OWNER_META_KEY)).unwrap();
        fake_host(&state, None).await;
        assert_eq!(retake_status(&state, &agent, &pane).await, StatusCode::CONFLICT, "took over just now");
        browser_owner::age_lost(&pane, std::time::Duration::from_secs(61));
        assert_eq!(retake_status(&state, &agent, &pane).await, StatusCode::OK, "took over 61 s ago, never touched the page");
        browser_owner::forget(&pane);
    }

    #[tokio::test]
    async fn another_agent_cannot_retake_and_a_driven_pane_is_not_retaken() {
        let state = test_state();
        let (own_a, own_b) = (uuid::Uuid::new_v4().to_string(), uuid::Uuid::new_v4().to_string());
        let pane = uuid::Uuid::new_v4().to_string();
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
        let mut driver = MetaMapType::new();
        driver.insert(browser_owner::LAST_DRIVER_META_KEY.to_string(), json!(a));
        crate::server::http_shell::broadcast_meta_update(&state, &pane, &driver).unwrap();
        assert_eq!(give_back(&state, &pane).status(), StatusCode::OK);
        assert_eq!(retake_status(&state, &b, &pane).await, StatusCode::CONFLICT, "given back to a, not to b");
        assert_eq!(retake_status(&state, &a, "no-such-pane").await, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn a_retake_brings_the_sites_limit_back_and_a_give_back_needs_a_last_driver() {
        let state = test_state();
        let own = uuid::Uuid::new_v4().to_string();
        let pane = uuid::Uuid::new_v4().to_string();
        crate::server::browser_attention::tests::browser_block(&state, &own);
        crate::server::browser_attention::tests::browser_block(&state, &pane);
        let agent = format!("retake-{}", uuid::Uuid::new_v4());
        state.reactive_handler.register_agent(&agent, &own, None).unwrap();
        assert_eq!(give_back(&state, &pane).status(), StatusCode::NOT_FOUND, "no agent drove it");

        let sites = vec!["https://example.com".to_string()];
        let mut meta = browser_owner::driver_meta(&agent);
        meta.insert(crate::server::browser_allowlist::ALLOWED_META_KEY.to_string(), json!(sites));
        crate::server::http_shell::broadcast_meta_update(&state, &pane, &meta).unwrap();
        crate::server::browser_allowlist::set(&pane, sites.clone());
        browser_owner::record(&pane, &agent);
        assert_eq!(give_back(&state, &pane).status(), StatusCode::CONFLICT, "an agent drives it");

        browser_owner::guard_client_meta_write(&format!("block:{pane}"), &clear(browser_owner::OWNER_META_KEY)).unwrap();
        assert_eq!(crate::server::browser_allowlist::list_for(&pane), None, "the Take over ended the limit");
        assert_eq!(give_back(&state, &pane).status(), StatusCode::OK);
        assert_eq!(retake_status(&state, &agent, &pane).await, StatusCode::OK);
        assert_eq!(crate::server::browser_allowlist::list_for(&pane), Some(sites), "the limit came back with the agent");
        let block = state.mstore.must_get::<crate::backend::obj::Block>(&pane).unwrap();
        assert!(block.meta.get(browser_owner::GIVEN_BACK_META_KEY).is_none_or(|v| v.is_null()), "the Give back is spent");
        crate::server::browser_allowlist::drop_pane(&pane);
        browser_owner::forget(&pane);
    }
}
