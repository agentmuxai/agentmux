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
    let mut found = BTreeSet::new();
    for route in crate::server::route_scan::registered_routes() {
        for method in ["post", "put", "patch", "delete"] {
            if route.methods.contains(&format!("{method}(")) || route.methods.contains(&format!(".{method}(")) {
                found.insert(format!("{} {}", method.to_uppercase(), route.path));
            }
        }
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
    let (_, id) = send(&state, Method::POST, "/agentmux/service", Some(&token), body).await;
    let line = recorded(&id).await;
    assert_eq!((line.source_agent.as_deref(), line.reason.as_deref()), (Some("agent:uid-http-act-b"), Some("window.CloseWindow")));
    assert!(!serde_json::to_string(&line).unwrap().contains("w-1"));
    // The host's own routes are recorded only with an agent's token.
    let (_, id) = send(&state, Method::POST, "/api/v1/host/browser_pause", None, "{}").await;
    not_recorded(&id).await;
}

#[tokio::test]
async fn a_service_call_with_no_token_is_recorded_as_unproven_with_its_names_and_never_its_arguments() {
    let state = test_state();
    let body = r#"{"service":"client","method":"AgreeTos","args":["a-secret-value"]}"#;
    let (_, id) = send(&state, Method::POST, "/agentmux/service", None, body).await;
    let line = recorded(&id).await;
    assert_eq!((line.source_agent.as_deref(), line.reason.as_deref()), (Some("unproven"), Some("client.AgreeTos")));
    assert_eq!(line.outcome.as_deref(), Some("POST /agentmux/service"));
    assert!(!serde_json::to_string(&line).unwrap().contains("a-secret-value"));
}

#[tokio::test]
async fn an_agents_service_read_is_recorded_but_the_windows_reads_and_bookkeeping_are_not() {
    let state = test_state();
    let token = token_for(&state, "uid-http-act-f");
    let read = r#"{"service":"object","method":"GetObject","args":["x"]}"#;
    let (_, id) = send(&state, Method::POST, "/agentmux/service", Some(&token), read).await;
    assert_eq!(recorded(&id).await.source_agent.as_deref(), Some("agent:uid-http-act-f"));
    let bookkeeping = r#"{"service":"object","method":"UpdateObjectMeta","args":["block:b1",{"term:osc_title":"build","term:human_turns":2}]}"#;
    for quiet in [read, bookkeeping, r#"{"service":"window","method":"SetWindowOpacity","args":[]}"#] {
        let (_, id) = send(&state, Method::POST, "/agentmux/service", None, quiet).await;
        not_recorded(&id).await;
    }
}

#[tokio::test]
async fn the_hosts_plumbing_is_not_recorded_while_it_succeeds_and_is_when_it_is_refused() {
    let mut state = test_state();
    state.host_reg_secret = Some("test-host-secret".to_string());
    // The host proves itself with the registration secret: nothing is written for what it does all the time.
    let from_host = r#"{"service":"credential","method":"ResolveIdentity","args":[{"block_id":"b-none"},"test-host-secret"]}"#;
    let (_, id) = send(&state, Method::POST, "/agentmux/service", None, from_host).await;
    not_recorded(&id).await;
    // A caller that does not hold the secret is refused, and the refusal is recorded with the names only.
    for (body, names) in [
        (r#"{"service":"credential","method":"Fill","args":[{"block_id":"b-none"},"a-guess"]}"#, "credential.Fill"),
        (r#"{"service":"credential","method":"Lookup","args":[]}"#, "credential.Lookup"),
        (r#"{"service":"host_ipc","method":"Register","args":[1234,"a-token","a-guess"]}"#, "host_ipc.Register"),
    ] {
        let (_, id) = send(&state, Method::POST, "/agentmux/service", None, body).await;
        let line = recorded(&id).await;
        assert_eq!((line.source_agent.as_deref(), line.reason.as_deref(), line.success), (Some("unproven"), Some(names), false), "{body}");
        assert_eq!(line.error_message.as_deref(), Some("service call failed"));
        assert!(!serde_json::to_string(&line).unwrap().contains("a-guess"), "{line:?}");
    }
    // With no secret configured at all the call is refused too, and recorded.
    let state = test_state();
    let (_, id) = send(&state, Method::POST, "/agentmux/service", None, from_host).await;
    assert!(!recorded(&id).await.success);
}

#[tokio::test]
async fn a_meta_write_with_any_other_key_is_recorded_with_the_object_and_the_key_names_and_never_the_values() {
    let state = test_state();
    let body = r#"{"service":"object","method":"UpdateObjectMeta","args":["block:b1",{"term:osc_title":"build","controller":"a-secret-value"}]}"#;
    let (_, id) = send(&state, Method::POST, "/agentmux/service", None, body).await;
    let line = recorded(&id).await;
    assert_eq!(line.source_agent.as_deref(), Some("unproven"));
    let subject = line.reason.clone().unwrap_or_default();
    assert!(subject.starts_with("object.UpdateObjectMeta object=block:b1 keys="), "{subject}");
    assert!(subject.contains("controller") && subject.contains("term:osc_title"), "{subject}");
    assert!(!serde_json::to_string(&line).unwrap().contains("a-secret-value"));
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
    assert_eq!(line.error_message.as_deref(), Some("service call failed"));
    let (_, id) = send(&state, Method::POST, "/agentmux/service", Some(&token), r#"{"service":"window","method":"CloseWindow","args":"s3cret"}"#).await;
    let line = recorded(&id).await;
    assert!(!serde_json::to_string(&line).unwrap().contains("s3cret"), "a service error is not kept: {line:?}");
}

#[tokio::test]
async fn a_refusal_from_a_route_that_records_its_own_successes_is_recorded_here() {
    let state = test_state();
    let (status, id) = send(&state, Method::POST, "/api/v1/agent/seats", None, "{}").await;
    assert!(status >= 400, "{status}");
    let line = recorded(&id).await;
    assert_eq!((line.source_agent.as_deref(), line.success), (Some("unproven"), false));
}
