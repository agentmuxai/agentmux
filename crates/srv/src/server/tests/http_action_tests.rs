// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Who each state-changing HTTP call is recorded as (`server::http_action`), through the real
//! router, and that every registered route is decided.

use std::collections::BTreeSet;

use axum::body::Body;
use axum::http::{Method, Request};
use tower::ServiceExt;

use crate::backend::reactive::AuditLogEntry;
use crate::server::http_action_routes::{ALWAYS, HANDLER, NEVER, PLUMBING};
use crate::server::routes::build_router;
use crate::server::tests::test_state;

/// Every `(METHOD, path)` that `routes.rs` registers with a method that can change state.
fn registered_writes() -> BTreeSet<String> {
    let source: String = include_str!("../routes.rs")
        .lines()
        .filter(|l| !l.trim_start().starts_with("//"))
        .collect::<Vec<_>>()
        .join("\n");
    let mut found = BTreeSet::new();
    let mut rest = source.as_str();
    while let Some(at) = rest.find(".route(") {
        let body_start = at + ".route(".len();
        let mut depth = 1;
        let mut end = body_start;
        for (i, c) in rest[body_start..].char_indices() {
            match c {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {}
            }
            if depth == 0 {
                end = body_start + i;
                break;
            }
        }
        let body = &rest[body_start..end];
        let (path, after) = match body.trim_start().strip_prefix('"') {
            Some(quoted) => {
                let close = quoted.find('"').expect("a closing quote");
                (quoted[..close].to_string(), &quoted[close..])
            }
            None => {
                // The one route whose path is a named constant.
                assert!(body.contains("STOP_RUN_PATH"), "a route path that is neither a literal nor STOP_RUN_PATH: {body}");
                (crate::server::jobs_remote_stop::STOP_RUN_PATH.to_string(), body)
            }
        };
        for method in ["post", "put", "patch", "delete"] {
            if after.contains(&format!("{method}(")) || after.contains(&format!(".{method}(")) {
                found.insert(format!("{} {path}", method.to_uppercase()));
            }
        }
        rest = &rest[end..];
    }
    found
}

#[test]
fn every_registered_write_route_is_decided_and_every_decided_route_is_registered() {
    let registered = registered_writes();
    assert!(registered.len() > 100, "the parser found {} routes", registered.len());
    let lists: [(&str, &[&str]); 4] = [("ALWAYS", ALWAYS), ("PLUMBING", PLUMBING), ("HANDLER", HANDLER), ("NEVER", NEVER)];
    let mut decided = BTreeSet::new();
    for (name, list) in lists {
        for route in list {
            assert!(decided.insert(route.to_string()), "{route} is in more than one list (seen again in {name})");
        }
    }
    let undecided: Vec<_> = registered.difference(&decided).collect();
    let stale: Vec<_> = decided.difference(&registered).collect();
    assert!(undecided.is_empty(), "routes in no list of server/http_action_routes.rs (decide whether they change anything): {undecided:?}");
    assert!(stale.is_empty(), "listed routes that routes.rs no longer registers: {stale:?}");
}

fn token_for(state: &crate::server::AppState, uid: &str) -> String {
    state.mstore.attach_token_index().unwrap();
    state.mstore.agent_token_ensure(uid).unwrap()
}

async fn send(state: &crate::server::AppState, method: Method, uri: &str, token: Option<&str>, body: &str) -> (u16, String) {
    let id = format!("http-act-{}", uuid::Uuid::new_v4());
    let mut req = Request::builder().method(method).uri(uri).header("X-AuthKey", "test-secret-key").header("content-type", "application/json").header("x-request-id", &id);
    if let Some(t) = token {
        req = req.header("X-Agent-Token", t);
    }
    let resp = build_router(state.clone()).oneshot(req.body(Body::from(body.to_string())).unwrap()).await.unwrap();
    (resp.status().as_u16(), id)
}

fn lines(id: &str) -> Vec<AuditLogEntry> {
    crate::backend::reactive::get_global_handler().get_audit_log(usize::MAX).into_iter().filter(|e| e.event_kind == "action" && e.request_id == id).collect()
}

/// The line for call `id`, once it has been written (it is written off the reply path).
async fn recorded(id: &str) -> AuditLogEntry {
    for _ in 0..100 {
        if let Some(e) = lines(id).into_iter().next() {
            return e;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("no line for {id}");
}

/// That nothing is written for call `id`, after giving a line time to appear.
async fn not_recorded(id: &str) {
    tokio::time::sleep(std::time::Duration::from_millis(400)).await;
    assert!(lines(id).is_empty(), "{id} was recorded: {:?}", lines(id));
}

#[tokio::test]
async fn an_agents_call_is_recorded_as_that_agent_with_the_route_and_what_it_acted_on() {
    let state = test_state();
    let token = token_for(&state, "uid-http-act-a");
    let (status, id) = send(&state, Method::POST, "/agentmux/work/t-42/heartbeat", Some(&token), r#"{"note":"s3cret body"}"#).await;
    let line = recorded(&id).await;
    assert_eq!(line.source_agent.as_deref(), Some("agent:uid-http-act-a"));
    assert_eq!(line.outcome.as_deref(), Some("POST /agentmux/work/:id/heartbeat"));
    assert_eq!(line.reason.as_deref(), Some("id=t-42"));
    assert_eq!(line.success, status < 400);
    assert!(!serde_json::to_string(&line).unwrap().contains("s3cret"), "a request's contents are not recorded");
}

#[tokio::test]
async fn a_call_with_no_token_is_unproven_never_the_operator() {
    let state = test_state();
    let (_, id) = send(&state, Method::POST, "/api/v1/window/focus", None, "{}").await;
    let line = recorded(&id).await;
    assert_eq!(line.source_agent.as_deref(), Some("unproven"));
}

#[tokio::test]
async fn a_refused_call_is_recorded_with_why() {
    let state = test_state();
    // Routes that act for an agent refuse a request with no token.
    let (status, id) = send(&state, Method::POST, "/agentmux/work", None, "{}").await;
    assert_eq!(status, 403);
    let line = recorded(&id).await;
    assert_eq!((line.source_agent.as_deref(), line.success, line.error_message.as_deref()), (Some("unproven"), false, Some("HTTP 403")));
}

#[tokio::test]
async fn plumbing_is_recorded_only_with_an_agents_token_and_a_service_call_names_what_it_asked() {
    let state = test_state();
    let token = token_for(&state, "uid-http-act-b");
    let body = r#"{"service":"window","method":"CloseWindow","args":["w-1"]}"#;
    let (_, id) = send(&state, Method::POST, "/agentmux/service", None, body).await;
    not_recorded(&id).await;
    let (_, id) = send(&state, Method::POST, "/agentmux/service", Some(&token), body).await;
    let line = recorded(&id).await;
    assert_eq!((line.source_agent.as_deref(), line.reason.as_deref()), (Some("agent:uid-http-act-b"), Some("window.CloseWindow")));
    assert!(!serde_json::to_string(&line).unwrap().contains("w-1"));
}

#[tokio::test]
async fn reads_and_streams_are_not_recorded() {
    let state = test_state();
    let token = token_for(&state, "uid-http-act-c");
    for (method, uri) in [
        (Method::GET, "/api/v1/layout"),
        (Method::POST, "/api/v1/shell/status"),
    ] {
        let (_, id) = send(&state, method, uri, Some(&token), "{}").await;
        not_recorded(&id).await;
    }
}

#[tokio::test]
async fn an_oversized_service_call_is_refused_and_the_refusal_is_recorded() {
    let state = test_state();
    let token = token_for(&state, "uid-http-act-d");
    let huge = format!(r#"{{"service":"object","method":"x","args":["{}"]}}"#, "a".repeat(3 * 1024 * 1024));
    let (status, id) = send(&state, Method::POST, "/agentmux/service", Some(&token), &huge).await;
    assert_eq!(status, 413);
    let line = recorded(&id).await;
    assert_eq!((line.source_agent.as_deref(), line.success, line.error_message.as_deref()), (Some("agent:uid-http-act-d"), false, Some("HTTP 413")));
}

#[tokio::test]
async fn a_service_call_that_fails_is_recorded_as_failed_though_the_reply_is_200() {
    let state = test_state();
    let token = token_for(&state, "uid-http-act-e");
    let (status, id) = send(&state, Method::POST, "/agentmux/service", Some(&token), "not a service call").await;
    assert_eq!(status, 200);
    let line = recorded(&id).await;
    assert!(!line.success, "{line:?}");
    assert!(line.error_message.as_deref().unwrap_or_default().contains("invalid request body"), "{line:?}");
}

#[tokio::test]
async fn a_refusal_from_a_route_that_records_its_own_successes_is_recorded_here() {
    let state = test_state();
    let (status, id) = send(&state, Method::POST, "/api/v1/agent/seats", None, "{}").await;
    assert!(status >= 400, "{status}");
    let line = recorded(&id).await;
    assert_eq!((line.source_agent.as_deref(), line.success), (Some("unproven"), false));
}
