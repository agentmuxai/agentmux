// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the fleet tools send to srv and what they tell the agent back, pinned against a stand-in
//! srv, so `call` can be split into one function per tool without changing either. The broadcast
//! file and priority cases are in `broadcast_files_tests`; these cover the rest.

use super::super::test_stub::{stub_srv, Fixture};
use super::*;

#[tokio::test]
async fn a_message_goes_to_inject_signed_and_the_outcome_names_where_it_landed() {
    let srv = stub_srv("200 OK", json!({ "success": true, "block_id": "b1", "request_id": "m-1" }));
    let fx = Fixture::new(&srv);
    let said = call("SendMessage", &json!({ "to": "karl", "message": "hello", "urgent": true, "reply": "none", "in_reply_to": "m-0" }), &fx.cx())
        .await
        .unwrap();
    let seen = srv.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1, "{seen:?}");
    let (path, body) = &seen[0];
    assert_eq!(path, "/agentmux/reactive/inject");
    assert_eq!(body["target_agent"], "karl");
    assert_eq!(body["message"], "hello");
    assert_eq!(body["priority"], "urgent");
    assert_eq!(body["reply"], "none");
    assert_eq!(body["in_reply_to"], "m-0");
    assert!(said.contains("karl") && said.contains("id=m-1"), "{said}");
}

#[tokio::test]
async fn a_message_srv_refused_says_it_was_not_sent() {
    let srv = stub_srv("200 OK", json!({ "refused": true, "error": "not on this machine" }));
    let fx = Fixture::new(&srv);
    let said = call("SendMessage", &json!({ "to": "karl", "message": "hello" }), &fx.cx()).await.unwrap();
    assert_eq!(said, "NOT SENT to karl — not on this machine");
}

#[tokio::test]
async fn discovery_is_read_and_returned_as_it_came() {
    for tool in ["DiscoverAgents", "FleetList"] {
        let srv = stub_srv("200 OK", json!({ "host": { "addressable": [{ "agent_id": "karl" }] } }));
        let fx = Fixture::new(&srv);
        let said = call(tool, &json!({}), &fx.cx()).await.unwrap();
        assert_eq!(srv.seen.lock().unwrap()[0].0, "/agentmux/discovery", "{tool}");
        assert!(said.contains("\"agent_id\": \"karl\""), "{tool}: {said}");
    }
}

#[tokio::test]
async fn a_bulk_stop_sends_what_it_was_given_and_names_the_agent() {
    let srv = stub_srv("200 OK", json!({ "stopped": ["b1"] }));
    let fx = Fixture::new(&srv);
    let said = call("FleetBulkStop", &json!({ "targets": ["b1", "b2"], "signal": "soft", "staged": true }), &fx.cx()).await.unwrap();
    let seen = srv.seen.lock().unwrap().clone();
    assert_eq!(seen.len(), 1, "{seen:?}");
    let (path, body) = &seen[0];
    assert_eq!(path, "/api/v1/fleet/bulk-stop");
    assert_eq!(body["targets"], json!(["b1", "b2"]));
    assert_eq!(body["signal"], "soft");
    assert_eq!(body["staged"], true);
    assert_eq!(body["auth"]["agent_id"], "tester");
    let result: Value = serde_json::from_str(&said).unwrap();
    assert_eq!(result["stopped"], json!(["b1"]));
    assert_eq!(result["kept_by_user"], json!([]), "nothing waited: the same shape as a wait");
}

#[tokio::test]
async fn a_broadcast_reports_each_target_apart() {
    let srv = stub_srv("200 OK", json!({ "success": true, "block_id": "b1" }));
    let fx = Fixture::new(&srv);
    let said = call("FleetBroadcast", &json!({ "targets": ["karl", "ana"], "message": "hi" }), &fx.cx()).await;
    // The stub answers discovery with the inject reply too, which names no one: both targets are used as names.
    let result: Value = serde_json::from_str(&said.unwrap()).unwrap();
    assert_eq!(result["succeeded"], json!(["karl", "ana"]));
    assert_eq!(result["failed"], json!([]));
    let paths: Vec<String> = srv.seen.lock().unwrap().iter().map(|(p, _)| p.clone()).collect();
    assert_eq!(paths, ["/agentmux/discovery", "/agentmux/reactive/inject", "/agentmux/reactive/inject"]);
}

#[tokio::test]
async fn a_tool_missing_what_it_needs_refuses_before_it_asks_srv() {
    let refusals = [
        ("SendMessage", json!({}), "missing required parameter: to"),
        ("SendMessage", json!({ "to": "karl" }), "missing required parameter: message"),
        ("FleetBroadcast", json!({}), "missing required parameter: targets"),
        ("FleetBroadcast", json!({ "targets": [] }), "targets must be a non-empty array"),
        ("FleetBroadcast", json!({ "targets": ["a"] }), "missing required parameter: message"),
        ("FleetBulkStop", json!({}), "missing required parameter: targets"),
        ("FleetBulkStop", json!({ "targets": [] }), "targets must be a non-empty array of block_id strings"),
    ];
    for (tool, args, wanted) in refusals {
        let srv = stub_srv("200 OK", json!({}));
        let fx = Fixture::new(&srv);
        let err = call(tool, &args, &fx.cx()).await.expect_err(&format!("{tool} {args} should refuse")).to_string();
        assert!(err.contains(wanted), "{tool} {args}: {err:?}, wanted {wanted:?}");
        assert!(srv.seen.lock().unwrap().is_empty(), "{tool} {args} asked srv");
    }
}

#[tokio::test]
async fn a_refusal_from_srv_names_the_step_and_the_status() {
    let failures = [
        ("SendMessage", json!({ "to": "karl", "message": "m" }), "inject failed: HTTP 500 Internal Server Error — \"boom\""),
        ("DiscoverAgents", json!({}), "discovery failed: HTTP 500 Internal Server Error — \"boom\""),
        ("FleetList", json!({}), "discovery failed: HTTP 500 Internal Server Error — \"boom\""),
        ("FleetBroadcast", json!({ "targets": ["a"], "message": "m" }), "discovery failed: HTTP 500 Internal Server Error — \"boom\""),
        ("FleetBulkStop", json!({ "targets": ["a"] }), "fleet bulk-stop failed: HTTP 500 Internal Server Error — \"boom\""),
    ];
    for (tool, args, wanted) in failures {
        let srv = stub_srv("500 Internal Server Error", json!("boom"));
        let fx = Fixture::new(&srv);
        let err = call(tool, &args, &fx.cx()).await.expect_err(&format!("{tool} should fail")).to_string();
        assert!(err.contains(wanted), "{tool} {args}: {err:?}, wanted {wanted:?}");
    }
}

#[tokio::test]
async fn a_name_that_is_not_a_fleet_tool_is_not_in_the_family() {
    let srv = stub_srv("200 OK", json!({}));
    let fx = Fixture::new(&srv);
    assert!(call("FleetNope", &json!({}), &fx.cx()).await.is_err());
    assert!(srv.seen.lock().unwrap().is_empty());
}

/// Several targets share srv's one window: the answer is at once, lists the window once, and nothing
/// polls for the user's answer (srv tells the agent by note if it is kept or fails).
#[tokio::test]
async fn a_bulk_stop_answers_pending_at_once_with_one_entry_per_window() {
    let deadline = agentmux_common::time::now_ms() + 30_000;
    let entry = |block: &str, request: &str| json!({ "block_id": block, "request_id": request, "effect": "close_tab", "deadline_ms": deadline });
    let srv = stub_srv(
        "202 Accepted",
        json!({
            "status": "pending_user_override",
            "succeeded": [],
            "failed": [{ "id": "gone", "error": "block not found: gone" }],
            "deadline_ms": deadline,
            "pending": [entry("b1", "r1"), entry("b2", "r1"), entry("b3", "r2")],
        }),
    );
    let fx = Fixture::new(&srv);
    let started = std::time::Instant::now();
    let said = call("FleetBulkStop", &json!({ "targets": ["b1", "b2", "b3", "gone"] }), &fx.cx()).await.unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(1), "no wait for the 30 s window");
    let paths: Vec<String> = srv.seen.lock().unwrap().iter().map(|(p, _)| p.clone()).collect();
    assert_eq!(paths, ["/api/v1/fleet/bulk-stop"], "no status polling");
    let result: Value = serde_json::from_str(&said).unwrap();
    assert_eq!(result["failed"][0]["id"], "gone");
    let windows = result["pending"].as_array().unwrap();
    assert_eq!(windows.len(), 2, "{result}");
    assert_eq!((windows[0]["id"].as_str(), windows[0]["targets"].clone()), (Some("r1"), json!(["b1", "b2"])));
    assert_eq!(windows[0]["closes_at"], deadline);
    assert!(result["note"].as_str().unwrap().contains("one [AgentMux] note"), "{result}");
    assert!(result.get("kept_by_user").is_none(), "nothing is known to be kept yet: {result}");
}
