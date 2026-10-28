// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The record of what each HTTP caller did: one line in the audit log for every call of a
//! state-changing route (`POST`, `PUT`, `PATCH`, `DELETE`), saying who made it, which route, what
//! it acted on (the route's path parameters) and how it went (the response's status). The websocket
//! RPC has the same record (`backend::rpc::action_record`); both end in the same audit line.
//!
//! It is made in one place, a layer under the authentication layers, so a handler cannot forget it
//! and a route nobody decided is recorded ([`super::http_action_routes`], with a test that every
//! registered route is decided). The caller is whoever the request's agent token names
//! (`Caller::Agent`); a request without one is `unproven`, never the operator: nothing on an HTTP
//! request proves the window made it. A request's contents are never recorded; the one body read is
//! the service call's `service` and `method` names.

use axum::body::Body;
use axum::extract::{MatchedPath, State};
use axum::http::{Method, Request, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

use super::caller::Caller;
use super::http_action_routes::{policy, Policy};
use super::AppState;
use crate::backend::rpc::action_record::{ActionRecord, Target};
use crate::backend::rpc::actor::Actor;

/// The generic service call: its `service` and `method` say what it does.
const SERVICE_ROUTE: &str = "/agentmux/service";
/// The largest service call read for its two names; a bigger one is refused with 413, as the
/// route's own body limit would, and the refusal is recorded.
const SERVICE_BODY_MAX: usize = 2 * 1024 * 1024;
/// The largest service reply read for its verdict.
const SERVICE_REPLY_MAX: usize = 32 * 1024 * 1024;
const MAX_FIELD: usize = 200;

/// The values a request's path gives for the route's `:name` parameters, as `name=value`.
pub(crate) fn path_params(template: &str, path: &str) -> Vec<String> {
    template
        .split('/')
        .zip(path.split('/'))
        .filter_map(|(t, p)| t.strip_prefix(':').map(|name| (name, p)))
        .filter(|(_, value)| !value.is_empty())
        .map(|(name, value)| format!("{name}={}", value.chars().take(MAX_FIELD).collect::<String>()))
        .collect()
}

/// `service.method` from a service call's body, when it names both.
fn service_names(body: &[u8]) -> Option<String> {
    let serde_json::Value::Object(fields) = serde_json::from_slice(body).ok()? else { return None };
    let name = |key: &str| fields.get(key)?.as_str().map(|s| s.chars().take(MAX_FIELD).collect::<String>());
    Some(format!("{}.{}", name("service")?, name("method")?))
}

/// Whether a service reply says the call failed. The reply's own `error` text is not kept: it can
/// quote the request's values. (`success` is left out of a failed reply, so a reply without it failed.)
fn service_failure(reply: &[u8]) -> Option<String> {
    let serde_json::Value::Object(fields) = serde_json::from_slice(reply).ok()? else { return None };
    let ok = fields.get("success").and_then(|v| v.as_bool()).unwrap_or(false);
    (!ok).then(|| "service call failed".to_string())
}

fn is_write(method: &Method) -> bool {
    !matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS | Method::TRACE | Method::CONNECT)
}

/// Layered inside `caller_middleware`: records the call once the route has answered.
pub(super) async fn record_action(State(state): State<AppState>, mut req: Request<Body>, next: Next) -> Response {
    if !is_write(req.method()) {
        return next.run(req).await;
    }
    let Some(template) = req.extensions().get::<MatchedPath>().map(|m| m.as_str().to_string()) else {
        return next.run(req).await;
    };
    let command = format!("{} {template}", req.method());
    let policy = policy(&command);
    let caller = req.extensions().get::<Caller>().cloned();
    if policy == Some(Policy::Never) {
        return next.run(req).await;
    }
    // The window's own calls and the host's cannot be told from a process that merely holds the
    // key, so these routes are recorded when an agent's token is on the request.
    if policy == Some(Policy::Plumbing) && caller.as_ref().and_then(Caller::uid).is_none() {
        return next.run(req).await;
    }
    let mut subject = path_params(&template, req.uri().path());
    let mut refused = None;
    if template == SERVICE_ROUTE {
        let (parts, body) = req.into_parts();
        match axum::body::to_bytes(body, SERVICE_BODY_MAX).await {
            Ok(bytes) => {
                subject.extend(service_names(&bytes));
                req = Request::from_parts(parts, Body::from(bytes));
            }
            Err(_) => {
                refused = Some((StatusCode::PAYLOAD_TOO_LARGE, "request body too large").into_response());
                req = Request::from_parts(parts, Body::empty());
            }
        }
    }
    let request_id = req.headers().get("x-request-id").and_then(|v| v.to_str().ok()).unwrap_or_default().to_string();
    let response = match refused {
        Some(r) => r,
        None => next.run(req).await,
    };
    let status = response.status();
    let mut error = (status.as_u16() >= 400).then(|| format!("HTTP {}", status.as_u16()));
    // A route that records its own successes still has its refusals recorded here, since it may
    // refuse before it gets to its own record.
    if policy == Some(Policy::Handler) && error.is_none() {
        return response;
    }
    // A service call answers 200 with its own verdict in the body.
    let mut response = response;
    if template == SERVICE_ROUTE && error.is_none() {
        let (parts, body) = response.into_parts();
        match axum::body::to_bytes(body, SERVICE_REPLY_MAX).await {
            Ok(bytes) => {
                error = service_failure(&bytes);
                response = Response::from_parts(parts, Body::from(bytes));
            }
            Err(_) => {
                error = Some("unreadable reply".to_string());
                response = (StatusCode::INTERNAL_SERVER_ERROR, "reply too large").into_response();
            }
        }
    }
    let store = state.mstore.clone();
    // Off the reply path: naming the agent reads the store.
    tokio::task::spawn_blocking(move || {
        let actor = match caller.as_ref().and_then(Caller::uid) {
            Some(uid) => Actor::Agent(store.agent_names_by_id(uid).ok().flatten().map(|row| row.slug).unwrap_or_else(|| uid.to_string())),
            None => Actor::Unproven,
        };
        let subject = (!subject.is_empty()).then(|| subject.join(" ").chars().take(MAX_FIELD * 2).collect());
        let record = ActionRecord { actor, command, target: Target { agent: None, block_id: None, subject }, request_id, error };
        crate::backend::reactive::get_global_handler().audit_action(&record);
    });
    response
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_routes_path_parameters_are_read_from_the_path() {
        assert_eq!(path_params("/agentmux/work/:id/heartbeat", "/agentmux/work/t-9/heartbeat"), vec!["id=t-9"]);
        assert_eq!(path_params("/a/:x/b/:y", "/a/1/b/2"), vec!["x=1", "y=2"]);
        assert!(path_params("/api/v1/tab/new", "/api/v1/tab/new").is_empty());
        assert_eq!(path_params("/a/:id", "/a/").len(), 0);
    }

    #[test]
    fn only_the_service_and_method_names_are_read_from_a_service_call() {
        assert_eq!(service_names(br#"{"service":"window","method":"CloseWindow","args":["secret"]}"#).as_deref(), Some("window.CloseWindow"));
        assert_eq!(service_names(br#"{"service":"window"}"#), None);
        assert_eq!(service_names(b"not json"), None);
    }

    #[test]
    fn a_service_reply_without_success_is_a_failure_and_its_text_is_not_kept() {
        let failed = service_failure(br#"{"error":"invalid type: string \"s3cret\""}"#).unwrap();
        assert!(!failed.contains("s3cret"), "{failed}");
        assert!(service_failure(br#"{"success":false}"#).is_some());
        assert_eq!(service_failure(br#"{"success":true,"data":1}"#), None);
        assert_eq!(service_failure(b"not json"), None);
    }

    #[test]
    fn reads_are_not_writes() {
        assert!(!is_write(&Method::GET) && !is_write(&Method::OPTIONS) && !is_write(&Method::HEAD));
        assert!(is_write(&Method::POST) && is_write(&Method::PUT) && is_write(&Method::PATCH) && is_write(&Method::DELETE));
    }
}
