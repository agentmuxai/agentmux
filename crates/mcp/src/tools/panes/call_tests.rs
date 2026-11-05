// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What each pane, tab and window tool sends to srv and what it tells the agent back, pinned
//! against a stand-in srv, so `call` can be split into one function per tool without changing
//! either.

use super::super::test_stub::{stub_srv, Fixture};
use super::*;

/// One call: the request it sends (path with query, and the body fields named here) and the
/// text it returns.
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
            "OpenEditor",
            json!({ "file": "/w/a.rs", "title": "T", "target_id": "b9", "collapse_tree": true, "floating": true }),
            json!({ "block_id": "b1", "note": "n" }),
            "/api/v1/pane/open",
            json!({
                "view": "editor", "file": "/w/a.rs", "focus": true, "split_direction": "right", "split_reference_block_id": "blk",
                "title": "T", "tree_expanded": false, "floating": true, "reuse_editor_pane": true, "target_block_id": "b9"
            }),
            &["Opened /w/a.rs in editor pane (block b1). n"],
        ),
        c(
            "OpenMedia",
            json!({ "file": "/w/m.png", "split": "down" }),
            json!({ "block_id": "b2" }),
            "/api/v1/pane/open",
            json!({ "view": "editor", "file": "/w/m.png", "split_direction": "down", "follow_folder": true }),
            &["Opened /w/m.png in the editor (block b2); OpenMedia is now an alias of OpenEditor."],
        ),
        c(
            "OpenFiles",
            json!({ "path": "/w", "select": ["a", "", "b"], "split": "up" }),
            json!({ "block_id": "b3" }),
            "/api/v1/pane/open",
            json!({ "view": "files", "file": "/w", "split_direction": "up", "select": ["a", "b"] }),
            &["Opened /w in a Files pane (block b3)"],
        ),
        c(
            "OpenAgent",
            json!({ "agent_id": "x", "tab_id": "t1", "focus": true }),
            json!({ "block_id": "b4" }),
            "/api/v1/agent/open",
            json!({ "agent_id": "x", "tab_id": "t1", "focus": true }),
            &["\"block_id\": \"b4\""],
        ),
        c("WhoAmI", json!({}), json!({ "tab": "t" }), "/api/v1/self?block_id=blk", json!({}), &["\"tab\": \"t\""]),
        c(
            "SetName",
            json!({ "target": "window", "name": "Home" }),
            json!({ "name": "Home!" }),
            "/api/v1/window/name",
            json!({ "block_id": "blk", "name": "Home" }),
            &["Window name set to \"Home!\""],
        ),
        c(
            "SetName",
            json!({ "target": "tab", "name": "Work", "target_id": "t7" }),
            json!({}),
            "/api/v1/tab/name",
            json!({ "tab_id": "t7", "name": "Work" }),
            &["Tab name set to \"Work\""],
        ),
        c(
            "SetName",
            json!({ "target": "workspace", "name": "Ws" }),
            json!({}),
            "/api/v1/workspace/name",
            json!({ "name": "Ws" }),
            &["Workspace name set to \"Ws\""],
        ),
        c(
            "SetName",
            json!({ "target": "pane", "name": "Term", "target_id": "b8" }),
            json!({ "title": "Term2" }),
            "/api/v1/pane/title",
            json!({ "block_id": "b8", "title": "Term" }),
            &["Pane title set to \"Term2\""],
        ),
        c("Layout", json!({}), json!({ "a": 1 }), "/api/v1/layout", json!({}), &["\"a\": 1"]),
        c("Layout", json!({ "query": "tabs" }), json!({ "a": 1 }), "/api/v1/tabs?block_id=blk", json!({}), &["\"a\": 1"]),
        c("Layout", json!({ "query": "windows" }), json!({ "a": 1 }), "/api/v1/windows", json!({}), &["\"a\": 1"]),
        c("SetActiveTab", json!({ "tab_id": "t1" }), json!({}), "/api/v1/tab/activate", json!({ "tab_id": "t1" }), &["Switched to tab t1"]),
        c("NewTab", json!({ "name": "N" }), json!({}), "/api/v1/tab/new", json!({ "block_id": "blk", "name": "N" }), &["Opened new tab \"N\""]),
        c("NewTab", json!({}), json!({}), "/api/v1/tab/new", json!({ "block_id": "blk" }), &["Opened a new tab"]),
        c("FocusWindow", json!({ "window_id": "w1" }), json!({}), "/api/v1/window/focus", json!({ "block_id": "blk", "window_id": "w1" }), &["Focused window"]),
        c(
            "ClosePane",
            json!({ "block_id": "b7", "reason": "done" }),
            json!({}),
            "/api/v1/agent/pane/close",
            json!({ "block_id": "b7", "reason": "done" }),
            &["Closed pane \"b7\""],
        ),
        c(
            "RegisterDevServer",
            json!({ "project": "p", "port": 3000, "command": "npm run dev" }),
            json!({ "url": "http://p.localhost:8090", "starting": true }),
            "/api/v1/agent/dev_server/register",
            json!({ "project": "p", "port": 3000, "command": "npm run dev" }),
            &["Registered. Browse to: http://p.localhost:8090", "still starting"],
        ),
    ]
}

#[tokio::test]
async fn every_pane_tool_sends_the_request_it_did_and_says_what_it_said() {
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
        ("OpenEditor", json!({}), "missing required parameter: file"),
        ("OpenFiles", json!({}), "missing required parameter: path"),
        ("OpenAgent", json!({}), "missing required parameter: agent_id"),
        ("SetName", json!({}), "missing required parameter: target"),
        ("SetName", json!({ "target": "tab" }), "missing required parameter: name"),
        ("SetName", json!({ "target": "bogus", "name": "x" }), "invalid target 'bogus'"),
        ("Layout", json!({ "query": "bogus" }), "invalid query 'bogus'"),
        ("SetActiveTab", json!({}), "missing required parameter: tab_id"),
        ("QuitSelf", json!({}), "missing required parameter: reason"),
        ("QuitSelf", json!({ "reason": "r" }), "missing required parameter: user_instruction"),
        ("RegisterDevServer", json!({}), "missing required parameter: project"),
        ("RegisterDevServer", json!({ "project": "p" }), "missing or invalid required parameter: port"),
        ("RegisterDevServer", json!({ "project": "p", "port": 70000 }), "missing or invalid required parameter: port"),
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
async fn a_refusal_from_srv_names_the_tool_and_the_status() {
    let failures = [
        ("OpenEditor", json!({ "file": "/a" }), "pane.open failed: HTTP 500 Internal Server Error — \"boom\""),
        ("OpenFiles", json!({ "path": "/a" }), "pane.open failed: HTTP 500 Internal Server Error — \"boom\""),
        ("OpenAgent", json!({ "agent_id": "x" }), "agent open failed: HTTP 500 Internal Server Error — \"boom\""),
        ("WhoAmI", json!({}), "self lookup failed: HTTP 500 Internal Server Error — \"boom\""),
        ("SetName", json!({ "target": "tab", "name": "x" }), "Tab name change failed: HTTP 500 Internal Server Error — \"boom\""),
        ("SetName", json!({ "target": "pane", "name": "x" }), "Pane title change failed: HTTP 500 Internal Server Error — \"boom\""),
        ("Layout", json!({}), "Layout layout failed: HTTP 500 Internal Server Error — \"boom\""),
        ("SetActiveTab", json!({ "tab_id": "t" }), "set active tab failed: HTTP 500 Internal Server Error — \"boom\""),
        ("NewTab", json!({}), "new tab failed: HTTP 500 Internal Server Error — \"boom\""),
        ("FocusWindow", json!({}), "focus window failed: HTTP 500 Internal Server Error — \"boom\""),
        ("ClosePane", json!({ "block_id": "b" }), "close failed: HTTP 500 Internal Server Error — \"boom\""),
        ("RegisterDevServer", json!({ "project": "p", "port": 3000 }), "RegisterDevServer failed: HTTP 500 Internal Server Error — \"boom\""),
    ];
    for (tool, args, wanted) in failures {
        let srv = stub_srv("500 Internal Server Error", json!("boom"));
        let fx = Fixture::new(&srv);
        let err = call(tool, &args, &fx.cx()).await.expect_err(&format!("{tool} should fail")).to_string();
        assert!(err.contains(wanted), "{tool} {args}: {err:?}, wanted {wanted:?}");
    }
}

#[tokio::test]
async fn a_name_that_is_not_a_pane_tool_is_not_in_the_family() {
    let srv = stub_srv("200 OK", json!({}));
    let fx = Fixture::new(&srv);
    assert!(call("PaneNope", &json!({}), &fx.cx()).await.is_err());
    assert!(srv.seen.lock().unwrap().is_empty());
}

/// srv runs the user's window; the call returns with what was opened and polls nothing.
#[tokio::test]
async fn closing_another_pane_answers_pending_at_once_without_polling() {
    let deadline = agentmux_common::time::now_ms() + 30_000;
    let srv = stub_srv("202 Accepted", json!({ "status": "pending_user_override", "request_id": "r1", "by": "tester", "via": "ClosePane", "deadline_ms": deadline }));
    let fx = Fixture::new(&srv);
    let started = std::time::Instant::now();
    let said = call("ClosePane", &json!({ "block_id": "b7" }), &fx.cx()).await.unwrap();
    assert!(started.elapsed() < std::time::Duration::from_secs(1), "no wait for the 30 s window");
    let paths: Vec<String> = srv.seen.lock().unwrap().iter().map(|(p, _)| p.clone()).collect();
    assert_eq!(paths, ["/api/v1/agent/pane/close"], "no status polling");
    let said: Value = serde_json::from_str(&said).unwrap();
    assert_eq!(said["status"], "pending");
    assert_eq!(said["pending"][0]["id"], "r1");
    assert_eq!(said["pending"][0]["targets"], json!(["b7"]));
    assert_eq!(said["pending"][0]["closes_at"], deadline);
    assert!(said["note"].as_str().unwrap().contains("one [AgentMux] note"), "{said}");
}
