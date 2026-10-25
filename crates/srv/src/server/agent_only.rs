// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Routes that act for an agent (its jobs and its work queue) are served only
//! to a request that carries an agent's token, and the agent a request names
//! (`agent_id`, `created_by`) has to be the one that token belongs to. The
//! handlers used to take the acting agent from those body fields, or record
//! the action as the operator's when there was no token. The window reaches
//! the same state over its RPC socket, not these routes.

use axum::body::Body;
use axum::extract::State;
use axum::http::{Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use axum::Json;

use super::caller::Caller;
use super::AppState;

/// More than any task or job a route here takes; a bigger body is refused unread.
const MAX_BODY: usize = 16 * 1024 * 1024;

fn refuse(status: StatusCode, error: &str) -> Response {
    (status, Json(serde_json::json!({ "ok": false, "error": error }))).into_response()
}

/// The agent names a JSON body gives as the one acting.
fn named_actors(body: &[u8]) -> Vec<String> {
    let Ok(serde_json::Value::Object(fields)) = serde_json::from_slice(body) else {
        return Vec::new();
    };
    ["agent_id", "created_by"]
        .iter()
        .filter_map(|key| fields.get(*key)?.as_str())
        .map(|name| name.trim().to_string())
        .filter(|name| !name.is_empty())
        .collect()
}

/// Layered inside `caller_middleware`: refuses a request that isn't [`Caller::Agent`], or that names
/// another agent than its token's as the one acting.
pub(super) async fn require_agent_token(State(state): State<AppState>, req: Request<Body>, next: Next) -> Response {
    let Some(uid) = req.extensions().get::<Caller>().and_then(Caller::uid).map(str::to_string) else {
        return refuse(
            StatusCode::FORBIDDEN,
            "FORBIDDEN: this acts for an agent, so the request must carry that agent's token (AGENTMUX_AGENT_TOKEN)",
        );
    };
    let (parts, body) = req.into_parts();
    let Ok(bytes) = axum::body::to_bytes(body, MAX_BODY).await else {
        return refuse(StatusCode::PAYLOAD_TOO_LARGE, "request body too large");
    };
    let names = named_actors(&bytes);
    if !names.is_empty() {
        let store = state.mstore.clone();
        let own = tokio::task::spawn_blocking(move || names.iter().all(|name| super::actor::proven_slug(&store, &uid, name).is_some())).await;
        if !own.unwrap_or(false) {
            return refuse(
                StatusCode::FORBIDDEN,
                "FORBIDDEN: this acts for the agent the request's token belongs to, and the request names another",
            );
        }
    }
    next.run(Request::from_parts(parts, Body::from(bytes))).await
}

#[cfg(test)]
mod tests {
    use super::named_actors;

    #[test]
    fn the_names_a_body_gives_as_acting() {
        assert_eq!(named_actors(br#"{"agent_id":" a1 ","created_by":"b","title":"x"}"#), vec!["a1", "b"]);
        assert!(named_actors(br#"{"agent_id":"","title":"x"}"#).is_empty());
        assert!(named_actors(b"not json").is_empty());
        assert!(named_actors(br#"["agent_id"]"#).is_empty());
        assert!(named_actors(br#"{"agent_id":7}"#).is_empty());
    }
}
