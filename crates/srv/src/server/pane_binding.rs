// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A pane belongs to one agent. The window registers and unregisters panes
//! over HTTP with no token, so these checks hang on the pane, not on who asks:
//! a registration must name the agent the pane belongs to, and a request that
//! does carry an agent's token acts on that agent's own panes only.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;

use super::actor;
use super::caller::Caller;
use super::AppState;

fn forbidden(error: &str) -> Response {
    (StatusCode::FORBIDDEN, Json(serde_json::json!({ "success": false, "error": error }))).into_response()
}

const WINDOW_ONLY: &str = "FORBIDDEN: the window does this over its own socket; an agent does it for its own panes, with its token (AGENTMUX_AGENT_TOKEN)";

/// `POST /agentmux/reactive/unregister`: an agent, for its own pane. The window unregisters over its socket.
pub(super) async fn handle_unregister(
    State(state): State<AppState>,
    caller: Option<axum::Extension<Caller>>,
    Json(req): Json<super::reactive::UnregisterRequest>,
) -> Response {
    if caller.as_deref().and_then(Caller::uid).is_none() {
        return forbidden(WINDOW_ONLY);
    }
    if let Some(refusal) = unregister_refusal(&state, caller.as_deref(), &req.block_id, &req.agent_id).await {
        return refusal;
    }
    super::reactive::unregister_pane(&state, req).await
}

/// `POST /api/v1/agent/takeover`: an agent, for its own pane. The window takes an agent over through its socket.
pub(super) async fn handle_takeover(
    State(state): State<AppState>,
    caller: Option<axum::Extension<Caller>>,
    Json(req): Json<super::agent_takeover::TakeoverRequest>,
) -> Response {
    if caller.as_deref().and_then(Caller::uid).is_none() {
        return forbidden(WINDOW_ONLY);
    }
    super::agent_takeover::take_over(&state, caller.as_deref(), req).await
}

/// Why a `register` request is refused, if it is: `pane_uid` is the agent the pane belongs to (`None`
/// for a pane with no agent, which any name may register), `sent` the name as sent and `routed` after
/// sanitising.
pub(super) async fn register_refusal(
    state: &AppState,
    caller: Option<&Caller>,
    pane_uid: Option<&str>,
    sent: &str,
    routed: &str,
) -> Option<Response> {
    let uid = pane_uid?;
    let (mstore, uid_owned, sent, routed) = (state.mstore.clone(), uid.to_string(), sent.to_string(), routed.to_string());
    let own = tokio::task::spawn_blocking(move || {
        actor::names_the_caller(&mstore, &uid_owned, &sent) || actor::names_the_caller(&mstore, &uid_owned, &routed)
    })
    .await
    .unwrap_or(true);
    if !own {
        return Some(forbidden("FORBIDDEN: this pane belongs to another agent than the name sent"));
    }
    actor::own_agent_only(caller, Some(uid)).err().map(|e| forbidden(&e))
}

/// Why an `unregister` request is refused, if it is: a caller with a token may unregister its own panes only.
/// The panes a name stands for are the ones the registry itself would tear down for it (`unregister_agent`).
pub(super) async fn unregister_refusal(state: &AppState, caller: Option<&Caller>, block_id: &str, agent: &str) -> Option<Response> {
    caller?.uid()?;
    let blocks: Vec<String> = if block_id.trim().is_empty() {
        state.reactive_handler.blocks_named(agent)
    } else {
        vec![block_id.trim().to_string()]
    };
    let mstore = state.mstore.clone();
    let owners = tokio::task::spawn_blocking(move || {
        blocks.iter().map(|block| crate::backend::agent_resolve::uid_for_block(&mstore, block)).collect::<Vec<_>>()
    })
    .await
    .unwrap_or_default();
    owners.iter().find_map(|owner| actor::own_agent_only(caller, owner.as_deref()).err()).map(|e| forbidden(&e))
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode};
    use serde_json::{json, Value};
    use tower::ServiceExt;

    use crate::backend::storage::agents::test_agent_def;
    use crate::server::routes::build_router;
    use crate::server::tests::test_state;
    use crate::server::AppState;

    /// Panes "pb-block-a" and "pb-block-b" belong to the agents "pb-alpha" and "pb-beta"; alpha's token.
    fn world() -> (AppState, String) {
        let state = test_state();
        for (uid, slug, block) in [("uid-pb-alpha", "pb-alpha", "pb-block-a"), ("uid-pb-beta", "pb-beta", "pb-block-b")] {
            let mut def = test_agent_def(uid, slug, "claude", "agent", 1, "");
            def.slug = slug.to_string();
            state.mstore.agent_def_insert(&mut def).unwrap();
            let mut pane = crate::backend::obj::Block { oid: block.to_string(), ..Default::default() };
            pane.meta.insert("agentId".to_string(), json!(uid));
            state.mstore.insert(&mut pane).unwrap();
        }
        state.mstore.attach_token_index().unwrap();
        let token = state.mstore.agent_token_ensure("uid-pb-alpha").unwrap();
        (state, token)
    }

    async fn post(state: &AppState, token: Option<&str>, uri: &str, body: Value) -> (StatusCode, Value) {
        let mut req = Request::builder().method(Method::POST).uri(uri).header("X-AuthKey", "test-secret-key").header("Content-Type", "application/json");
        if let Some(t) = token {
            req = req.header("X-Agent-Token", t);
        }
        let resp = build_router(state.clone()).oneshot(req.body(Body::from(body.to_string())).unwrap()).await.unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
    }

    #[tokio::test]
    async fn a_pane_registers_only_as_its_own_agent() {
        let (state, token) = world();
        let register = "/agentmux/reactive/register";
        let (status, v) = post(&state, None, register, json!({ "agent_id": "pb-beta", "block_id": "pb-block-a" })).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "another agent's name on this pane: {v}");
        let (status, v) = post(&state, Some(&token), register, json!({ "agent_id": "pb-beta", "block_id": "pb-block-b" })).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "an agent registers its own pane only: {v}");
        let (status, v) = post(&state, None, register, json!({ "agent_id": "pb-alpha", "block_id": "pb-block-a" })).await;
        assert_ne!(status, StatusCode::FORBIDDEN, "the window registers a pane as its agent: {v}");
        let (status, v) = post(&state, Some(&token), register, json!({ "agent_id": "pb-alpha", "block_id": "pb-block-a" })).await;
        assert_ne!(status, StatusCode::FORBIDDEN, "an agent registers its own pane: {v}");
        let _ = state.reactive_handler.unregister_block("pb-block-a");
    }

    #[tokio::test]
    async fn a_token_unregisters_releases_and_takes_over_its_own_agent_only() {
        let (state, token) = world();
        let (status, v) = post(&state, Some(&token), "/agentmux/reactive/unregister", json!({ "agent_id": "", "block_id": "pb-block-b" })).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{v}");
        // By name: the pane is registered under a stable name other than its display name.
        state.reactive_handler.register_agent_full("PB Beta Display", "pb-block-b", None, 0, Some("pb-beta"), Some("uid-pb-beta"), "test").unwrap();
        for name in ["pb-beta", "PB Beta Display", "Pb-Beta"] {
            let (status, v) = post(&state, Some(&token), "/agentmux/reactive/unregister", json!({ "agent_id": name })).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "by name {name}: {v}");
        }
        assert!(state.reactive_handler.get_agent_by_block("pb-block-b").is_some(), "nothing was torn down");
        let _ = state.reactive_handler.unregister_block("pb-block-b");
        // A name that is also another agent's UID still means the pane registered under it.
        state.reactive_handler.register_agent_full("uid-pb-alpha", "pb-block-b", None, 0, Some("pb-beta"), Some("uid-pb-beta"), "test").unwrap();
        state.reactive_handler.register_agent_full("pb-alpha", "pb-block-a", None, 0, Some("pb-alpha"), Some("uid-pb-alpha"), "test").unwrap();
        let (status, v) = post(&state, Some(&token), "/agentmux/reactive/unregister", json!({ "agent_id": "uid-pb-alpha" })).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{v}");
        assert!(state.reactive_handler.get_agent_by_block("pb-block-b").is_some(), "nothing was torn down");
        let _ = state.reactive_handler.unregister_block("pb-block-b");
        let _ = state.reactive_handler.unregister_block("pb-block-a");
        let (status, v) = post(&state, None, "/agentmux/reactive/unregister", json!({ "agent_id": "", "block_id": "pb-block-b" })).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "the window unregisters over its socket, not here: {v}");
        let (status, v) = post(&state, None, "/api/v1/agent/takeover", json!({ "block_id": "pb-block-b" })).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{v}");
        let (status, v) = post(&state, Some(&token), "/agentmux/agent/release", json!({ "uid": "uid-pb-beta" })).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{v}");
        let (status, v) = post(&state, Some(&token), "/api/v1/agent/takeover", json!({ "block_id": "pb-block-b" })).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{v}");
        let (status, v) = post(&state, Some(&token), "/agentmux/agent/release", json!({ "uid": "uid-pb-alpha", "agent": "pb-beta" })).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "its uid but another agent's relay name: {v}");
        let (status, v) = post(&state, Some(&token), "/agentmux/agent/release", json!({ "uid": "uid-pb-alpha", "agent": "pb-alpha" })).await;
        assert_ne!(status, StatusCode::FORBIDDEN, "an agent's own: {v}");
        let (status, v) = post(&state, None, "/agentmux/agent/release", json!({ "uid": "uid-pb-alpha", "agent": "pb-alpha" })).await;
        assert_ne!(status, StatusCode::FORBIDDEN, "a peer instance asks by uid and name: {v}");
    }
}
