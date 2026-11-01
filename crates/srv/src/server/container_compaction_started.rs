// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `POST /api/v1/agent/compaction-started`: a container agent's `agentmux-bashwrap precompact`
//! (Claude Code's `PreCompact` hook) telling srv the CLI is compacting, so the pane shows
//! "Compacting…".
//!
//! `/agentmux/wps/publish` is a generic relay: it republishes an undeclared event to any scope,
//! so a container token is refused there (`container_credential`). This route takes one event,
//! `compaction_started`, for one scope, the container's own `block:<id>`, and only as the typed
//! [`CompactionStarted`]. A request that is not a container's, names another event or scope, or
//! is not a `CompactionStarted` never reaches the broker. It is the host's publish with the
//! same envelope, so the client sends the same body to a different path.

use axum::extract::rejection::JsonRejection;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::{Extension, Json};
use serde_json::json;

use agentmux_common::api_types::WpsPublishRequest;
use agentmux_common::client_events::CompactionStarted;

use crate::backend::container_credential::ContainerGrant;
use crate::server::AppState;

/// The largest request body: the three small fields of a `CompactionStarted`, with its envelope.
pub(crate) const MAX_BODY_BYTES: usize = 4 * 1024;

/// Live subscribers only, never replayed: as the host's `PreCompact` hook asks (`persist: 0`,
/// #2378), because nothing marks the end of a compaction on this event, and a replayed start
/// would bring "Compacting…" back for one that has finished. srv decides it here; the
/// container's value is not read.
const PERSIST: usize = 0;

fn refuse(status: StatusCode, why: impl Into<String>) -> Response {
    (status, Json(json!({ "error": why.into() }))).into_response()
}

/// The event to publish for `block_id`'s `req`, or why the request is refused.
fn check(block_id: &str, req: &WpsPublishRequest) -> Result<CompactionStarted, Response> {
    if req.event != "compaction_started" {
        return Err(refuse(StatusCode::FORBIDDEN, "this route takes compaction_started only"));
    }
    if req.scopes != [format!("block:{block_id}")] {
        return Err(refuse(StatusCode::FORBIDDEN, "a container agent's compaction goes to its own pane only"));
    }
    serde_json::from_value::<CompactionStarted>(req.data.clone())
        .map_err(|e| refuse(StatusCode::BAD_REQUEST, format!("compaction_started: {e}")))
}

pub(crate) async fn handle_container_compaction_started(
    State(state): State<AppState>,
    grant: Option<Extension<ContainerGrant>>,
    body: Result<Json<WpsPublishRequest>, JsonRejection>,
) -> Response {
    let Some(Extension(grant)) = grant else {
        return refuse(StatusCode::FORBIDDEN, "this route is for container agents");
    };
    let req = match body {
        Ok(Json(req)) => req,
        Err(rejection) => return refuse(rejection.status(), rejection.body_text()),
    };
    let started = match check(&grant.block_id, &req) {
        Ok(started) => started,
        Err(refusal) => return refusal,
    };
    state
        .broker
        .publish_event::<crate::backend::mps_events::CompactionStarted>(&started, req.scopes, PERSIST);
    Json(json!({ "ok": true })).into_response()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::Request;
    use tower::ServiceExt;

    fn started() -> serde_json::Value {
        json!({ "trigger": "manual", "sessionId": "sess-1", "startedAt": "2026-10-31T00:00:00Z" })
    }

    fn body(event: &str, scopes: &[&str], data: serde_json::Value) -> serde_json::Value {
        json!({ "event": event, "scopes": scopes, "persist": 999_999, "data": data })
    }

    async fn post(token: &str, body: String) -> (StatusCode, serde_json::Value) {
        let state = crate::server::tests::test_state();
        let app = crate::server::build_router(state);
        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/agent/compaction-started")
            .header("X-AuthKey", token)
            .header("Content-Type", "application/json")
            .body(Body::from(body))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
        (status, serde_json::from_slice(&bytes).unwrap_or_default())
    }

    #[tokio::test]
    async fn a_container_reports_its_own_compaction_only() {
        let mine = crate::backend::container_credential::token_for_block("cs-block-a", None);
        let own = ["block:cs-block-a"];
        let (status, _) = post(&mine, body("compaction_started", &own, started()).to_string()).await;
        assert_eq!(status, StatusCode::OK);

        // Another block's pane, no scope, or more than its own.
        for scopes in [&["block:cs-block-b"][..], &[], &["block:cs-block-a", "block:cs-block-b"], &["tab:cs-block-a"]] {
            let (status, _) = post(&mine, body("compaction_started", scopes, started()).to_string()).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{scopes:?}");
        }
        // Any other event, whatever is in it: this route is not the generic relay.
        for event in ["tool_chunk", "agent_event:cs-block-a", "controllerstatus", "custom:thing"] {
            let (status, _) = post(&mine, body(event, &own, started()).to_string()).await;
            assert_eq!(status, StatusCode::FORBIDDEN, "{event}");
        }
        // Not a CompactionStarted.
        let (status, _) = post(&mine, body("compaction_started", &own, json!({ "trigger": "sideways" })).to_string()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn a_request_over_the_cap_is_413() {
        let mine = crate::backend::container_credential::token_for_block("cs-block-big", None);
        let mut big = started();
        big["sessionId"] = json!("x".repeat(MAX_BODY_BYTES));
        let (status, _) = post(&mine, body("compaction_started", &["block:cs-block-big"], big).to_string()).await;
        assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE);
    }

    #[tokio::test]
    async fn the_route_is_a_container_agents_and_the_generic_relay_stays_refused() {
        // The instance key is not a container: this route is theirs alone.
        let (status, _) = post("test-secret-key", body("compaction_started", &["block:b"], started()).to_string()).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert!(!crate::backend::container_credential::container_route_allowed("/agentmux/wps/publish"));
        assert!(crate::backend::container_credential::container_route_allowed("/api/v1/agent/compaction-started"));
    }

    #[tokio::test]
    async fn the_start_reaches_the_panes_live_subscribers_and_is_not_kept() {
        let state = crate::server::tests::test_state();
        let token = crate::backend::container_credential::token_for_block("cs-block-live", None);
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        state.broker.add_observer(std::sync::Arc::new(move |e: &crate::backend::mps::MuxEvent| {
            if e.scopes == ["block:cs-block-live"] {
                sink.lock().unwrap().push(e.clone());
            }
        }));
        let app = crate::server::build_router(state);
        let req = Request::builder()
            .method("POST")
            .uri("/api/v1/agent/compaction-started")
            .header("X-AuthKey", &token)
            .header("Content-Type", "application/json")
            .body(Body::from(body("compaction_started", &["block:cs-block-live"], started()).to_string()))
            .unwrap();
        assert_eq!(app.oneshot(req).await.unwrap().status(), StatusCode::OK);
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].event, "compaction_started");
        assert_eq!(seen[0].data.as_ref().unwrap()["sessionId"], "sess-1");
        assert_eq!(seen[0].persist, PERSIST, "a start is never replayed");
    }
}
