// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What the UI tools that talk to srv send and what they tell the agent back, pinned against a
//! stand-in srv, so `call` can be split into one function per tool without changing either.
//! CaptureWindow and DiscoverWindows read the real desktop and are not run here.

use super::super::test_stub::{stub_srv, Fixture};
use super::*;

/// One call: the request it sends (path, and the body fields named here) and the text it returns.
struct Case {
    tool: &'static str,
    args: Value,
    reply: Value,
    path: &'static str,
    sent: Value,
    says: &'static [&'static str],
}

fn cases() -> Vec<Case> {
    let c = |tool, args, reply, path, sent, says| Case { tool, args, reply, path, sent, says };
    vec![
        c(
            "UIScreenshot",
            json!({ "pane": "p1" }),
            json!({ "path": "/tmp/shot.png", "png_base64": "" }),
            "/api/v1/ui/screenshot",
            json!({ "pane": "p1", "agent_id": "tester" }),
            &["Screenshot saved to /tmp/shot.png — use Read on that path"],
        ),
        c(
            "UIRecord",
            json!({ "op": "start", "pane": "p1", "fps": 5, "seconds": 9 }),
            json!({ "status": "started", "message": "Recording started." }),
            "/api/v1/ui/record",
            json!({ "op": "start", "pane": "p1", "fps": 5, "seconds": 9 }),
            &["Recording started."],
        ),
        c("UIClick", json!({ "selector": "#go" }), json!({}), "/api/v1/ui/click", json!({ "selector": "#go" }), &["Clicked \"#go\""]),
        c(
            "UIQuery",
            json!({ "selector": ".row", "limit": 3 }),
            json!({ "data": { "matches": [{ "tag": "div" }] } }),
            "/api/v1/ui/query",
            json!({ "selector": ".row", "limit": 3 }),
            &["\"tag\": \"div\""],
        ),
        c("UIQuery", json!({ "selector": ".row" }), json!({}), "/api/v1/ui/query", json!({}), &["[]"]),
        c(
            "ListShortcuts",
            json!({}),
            json!({ "ok": true, "data": { "shortcuts": [{ "keys": "Ctrl+K" }] } }),
            "/api/v1/ui/shortcuts/list",
            json!({ "agent_id": "tester" }),
            &["\"keys\": \"Ctrl+K\""],
        ),
        c(
            "RunCommand",
            json!({ "command": "app:newtab", "target": " t1 " }),
            json!({ "ok": true, "data": { "ran": true } }),
            "/api/v1/ui/shortcuts/run",
            json!({ "command": "app:newtab", "target": "t1" }),
            &["Ran app:newtab."],
        ),
        c(
            "RunCommand",
            json!({ "command": "app:quit" }),
            json!({ "ok": true, "data": { "pending": { "request_id": "r9" }, "reason": "the user must allow it" } }),
            "/api/v1/ui/shortcuts/run",
            json!({}),
            &["app:quit is pending: the user must allow it (request r9)."],
        ),
        c(
            "RunCommand",
            json!({ "command": "app:nope" }),
            json!({ "ok": true, "data": { "ran": false, "reason": "unknown command" } }),
            "/api/v1/ui/shortcuts/run",
            json!({}),
            &["Didn't run app:nope: unknown command"],
        ),
        c(
            "PressKeys",
            json!({ "keys": "Ctrl+T" }),
            json!({ "ok": true, "data": { "pressed": true } }),
            "/api/v1/ui/shortcuts/press",
            json!({ "keys": "Ctrl+T" }),
            &["\"pressed\": true"],
        ),
    ]
}

#[tokio::test]
async fn every_ui_tool_sends_the_request_it_did_and_says_what_it_said() {
    for case in cases() {
        let srv = stub_srv("200 OK", case.reply.clone());
        let fx = Fixture::new(&srv);
        let said = call(case.tool, &case.args, &fx.cx()).await.unwrap_or_else(|e| panic!("{} {}: {e}", case.tool, case.args));
        let seen = srv.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 1, "{} {}: requests {seen:?}", case.tool, case.args);
        let (path, body) = &seen[0];
        assert_eq!(path, case.path, "{} {}", case.tool, case.args);
        for (key, want) in case.sent.as_object().unwrap() {
            assert_eq!(body.get(key), Some(want), "{} {}: body {body}", case.tool, case.args);
        }
        for want in case.says {
            assert!(said.contains(want), "{} {}: said {said:?}, wanted {want:?}", case.tool, case.args);
        }
    }
}

#[tokio::test]
async fn a_tool_missing_what_it_needs_refuses_before_it_asks_srv() {
    let refusals = [
        ("UIRecord", json!({}), "record needs op"),
        ("UIRecord", json!({ "op": "pause" }), "record needs op"),
        ("UIClick", json!({}), "missing required parameter: selector"),
        ("UIQuery", json!({}), "missing required parameter: selector"),
        ("RunCommand", json!({}), "missing required parameter: command"),
        ("PressKeys", json!({}), "missing required parameter: keys"),
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
        ("UIScreenshot", json!({}), "screenshot failed: HTTP 500 Internal Server Error — \"boom\""),
        ("UIRecord", json!({ "op": "stop" }), "record failed: HTTP 500 Internal Server Error — \"boom\""),
        ("UIClick", json!({ "selector": "#a" }), "click failed: HTTP 500 Internal Server Error — \"boom\""),
        ("UIQuery", json!({ "selector": "#a" }), "query failed: HTTP 500 Internal Server Error — \"boom\""),
        ("ListShortcuts", json!({}), "shortcuts/list failed: HTTP 500 Internal Server Error — no error message"),
        ("RunCommand", json!({ "command": "c" }), "shortcuts/run failed: HTTP 500 Internal Server Error — no error message"),
        ("PressKeys", json!({ "keys": "k" }), "shortcuts/press failed: HTTP 500 Internal Server Error — no error message"),
    ];
    for (tool, args, wanted) in failures {
        let srv = stub_srv("500 Internal Server Error", json!("boom"));
        let fx = Fixture::new(&srv);
        let err = call(tool, &args, &fx.cx()).await.expect_err(&format!("{tool} should fail")).to_string();
        assert!(err.contains(wanted), "{tool} {args}: {err:?}, wanted {wanted:?}");
    }
}

#[tokio::test]
async fn a_shortcut_answer_that_is_not_ok_is_a_failure_even_with_a_200() {
    let srv = stub_srv("200 OK", json!({ "ok": false, "error": "no window" }));
    let fx = Fixture::new(&srv);
    let err = call("ListShortcuts", &json!({}), &fx.cx()).await.unwrap_err().to_string();
    assert!(err.contains("shortcuts/list failed: HTTP 200 OK — no window"), "{err}");
}

#[tokio::test]
async fn a_name_that_is_not_a_ui_tool_is_not_in_the_family() {
    let srv = stub_srv("200 OK", json!({}));
    let fx = Fixture::new(&srv);
    assert!(call("UINope", &json!({}), &fx.cx()).await.is_err());
    assert!(srv.seen.lock().unwrap().is_empty());
}
