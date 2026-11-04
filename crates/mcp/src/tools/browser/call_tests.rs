// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What each Browser tool sends to srv and what it tells the agent back, pinned against a
//! stand-in srv, so `call` can be split into one function per tool without changing either.

use super::super::test_stub::{stub_srv, Fixture, StubSrv};
use super::*;

async fn call_tool(srv: &StubSrv, tool: &str, args: Value) -> Result<String> {
    let fx = Fixture::new(srv);
    call(tool, &args, &fx.cx()).await
}

/// One call: the request it sends (path, and the fields of the body named here) and the text it returns.
struct Case {
    tool: &'static str,
    args: Value,
    reply: Value,
    path: &'static str,
    sent: Value,
    says: &'static [&'static str],
}

fn ok(data: Value) -> Value {
    json!({ "ok": true, "data": data })
}

fn cases() -> Vec<Case> {
    let c = |tool, args, reply, path, sent, says| Case { tool, args, reply, path, sent, says };
    vec![
        c(
            "OpenBrowser",
            json!({ "url": "https://a.example", "split": "right", "title": "T" }),
            ok(json!({ "pane": "p1" })),
            "/api/v1/ui/browser/open",
            json!({ "url": "https://a.example", "split": "right", "title": "T" }),
            &["Opened a browser pane at \"https://a.example\".", "Its pane id is p1"],
        ),
        c(
            "OpenBrowser",
            json!({ "url": "https://a.example", "allowed_origins": ["https://a.example"], "profile": "incognito" }),
            ok(json!({ "pane": "p2", "browsing_as": "incognito" })),
            "/api/v1/ui/browser/open",
            json!({ "allowed_origins": ["https://a.example"], "profile": "incognito" }),
            &["limited to allowed_origins", "It browses Incognito"],
        ),
        c(
            "BrowserSnapshot",
            json!({ "pane": "p1" }),
            ok(json!({ "url": "https://x.example", "refs": 3, "snapshot": "page body" })),
            "/api/v1/ui/browser/snapshot",
            json!({ "pane": "p1" }),
            &["Page https://x.example (3 references).", "untrusted", "---\npage body"],
        ),
        c(
            "BrowserSnapshot",
            json!({}),
            ok(json!({ "url": "https://x.example", "refs": 0, "snapshot": "", "woke_from_sleep": true })),
            "/api/v1/ui/browser/snapshot",
            json!({}),
            &["asleep to save memory"],
        ),
        c(
            "BrowserFind",
            json!({ "name": "Save", "role": "button", "exact": true }),
            ok(json!({})),
            "/api/v1/ui/browser/find",
            json!({ "role": "button", "name": "Save", "exact": true }),
            &["Save"],
        ),
        c(
            "BrowserClick",
            json!({ "ref": "e1" }),
            ok(json!({})),
            "/api/v1/ui/browser/act",
            json!({ "ref": "e1", "action": "click" }),
            &["BrowserClick e1"],
        ),
        c(
            "BrowserFill",
            json!({ "ref": "e2", "text": "hi" }),
            ok(json!({})),
            "/api/v1/ui/browser/act",
            json!({ "ref": "e2", "action": "fill", "text": "hi" }),
            &["BrowserFill e2"],
        ),
        c(
            "BrowserSelect",
            json!({ "ref": "e3", "option": "two" }),
            ok(json!({})),
            "/api/v1/ui/browser/act",
            json!({ "ref": "e3", "action": "select", "option": "two" }),
            &["BrowserSelect e3"],
        ),
        c(
            "BrowserCheck",
            json!({ "ref": "e4", "checked": false }),
            ok(json!({})),
            "/api/v1/ui/browser/act",
            json!({ "ref": "e4", "action": "check", "checked": false }),
            &["BrowserCheck e4"],
        ),
        c(
            "BrowserSetFiles",
            json!({ "ref": "e5", "paths": ["/w/a.txt"] }),
            ok(json!({ "after": "a.txt" })),
            "/api/v1/ui/browser/set_files",
            json!({ "ref": "e5", "paths": ["/w/a.txt"] }),
            &["Browser set_files e5: done. The file input now holds: \"a.txt\""],
        ),
        c(
            "BrowserWaitFor",
            json!({ "text": "Done", "timeout_seconds": 2.5 }),
            ok(json!({})),
            "/api/v1/ui/browser/wait_for",
            json!({ "text": "Done", "timeout_ms": 2500 }),
            &[],
        ),
        c(
            "BrowserHandoff",
            json!({ "reason": "log in", "timeout_minutes": 5 }),
            ok(json!({ "answer": "done" })),
            "/api/v1/ui/browser/handoff",
            json!({ "reason": "log in", "timeout_minutes": 5 }),
            &["The user clicked Done."],
        ),
        c(
            "BrowserHandoff",
            json!({ "reason": "log in" }),
            ok(json!({ "answer": "cancelled" })),
            "/api/v1/ui/browser/handoff",
            json!({ "reason": "log in" }),
            &["The user clicked Cancel."],
        ),
        c(
            "BrowserRequestBypass",
            json!({ "reason": "bulk edit", "minutes": 5, "powers": ["click"] }),
            ok(json!({ "answer": "declined" })),
            "/api/v1/ui/browser/request_bypass",
            json!({ "reason": "bulk edit", "minutes": 5, "powers": ["click"] }),
            &["The user said No."],
        ),
        c(
            "BrowserEndBypass",
            json!({}),
            ok(json!({ "ended": true })),
            "/api/v1/ui/browser/end_bypass",
            json!({}),
            &["Auto-approve stopped: committing clicks ask the user again."],
        ),
        c(
            "BrowserEndBypass",
            json!({}),
            ok(json!({ "ended": false })),
            "/api/v1/ui/browser/end_bypass",
            json!({}),
            &["There was no auto-approve on that pane for you to stop."],
        ),
        c("BrowserLog", json!({}), ok(json!({})), "/api/v1/ui/browser/actions", json!({}), &[]),
        c(
            "BrowserNavigate",
            json!({ "url": "https://b.example" }),
            ok(json!({})),
            "/api/v1/ui/browser/navigate",
            json!({ "url": "https://b.example" }),
            &["Navigated to \"https://b.example\""],
        ),
        c("BrowserBack", json!({}), ok(json!({})), "/api/v1/ui/browser/back", json!({}), &["Browser back ok"]),
        c("BrowserForward", json!({}), ok(json!({})), "/api/v1/ui/browser/forward", json!({}), &["Browser forward ok"]),
        c(
            "BrowserReload",
            json!({ "ignore_cache": true }),
            ok(json!({})),
            "/api/v1/ui/browser/reload",
            json!({ "ignore_cache": true }),
            &["Browser reload ok"],
        ),
        c(
            "BrowserEval",
            json!({ "script": "1+1", "await_promise": true }),
            ok(json!({ "result": 2 })),
            "/api/v1/ui/browser/eval",
            json!({ "script": "1+1", "await_promise": true }),
            &["\"result\": 2"],
        ),
        c(
            "BrowserDispatchKey",
            json!({ "key": "Enter", "selector": "#q" }),
            ok(json!({})),
            "/api/v1/ui/browser/dispatch_key",
            json!({ "key": "Enter", "selector": "#q" }),
            &[],
        ),
        c(
            "BrowserTerminal",
            json!({ "text": "ls", "enter": true, "keys": ["Tab"], "until_text": "$", "timeout_seconds": 90 }),
            ok(json!({})),
            "/api/v1/ui/browser/terminal",
            json!({ "text": "ls", "enter": true, "keys": ["Tab"], "until_text": "$", "timeout_seconds": 60 }),
            &[],
        ),
        c(
            "BrowserFocusElement",
            json!({ "selector": "#q" }),
            ok(json!({})),
            "/api/v1/ui/browser/focus_element",
            json!({ "selector": "#q" }),
            &["Focused \"#q\""],
        ),
        c(
            "BrowserFocusInfo",
            json!({}),
            ok(json!({ "focused": { "tag": "input" } })),
            "/api/v1/ui/browser/focus_info",
            json!({}),
            &["\"tag\": \"input\""],
        ),
    ]
}

#[tokio::test]
async fn every_browser_tool_sends_the_request_it_did_and_says_what_it_said() {
    for case in cases() {
        let srv = stub_srv("200 OK", case.reply.clone());
        let said = call_tool(&srv, case.tool, case.args.clone()).await.unwrap_or_else(|e| panic!("{} {}: {e}", case.tool, case.args));
        let seen = srv.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 1, "{} {}: requests {seen:?}", case.tool, case.args);
        let (path, body) = &seen[0];
        assert_eq!(path, case.path, "{} {}", case.tool, case.args);
        for (key, want) in case.sent.as_object().unwrap() {
            assert_eq!(body.get(key), Some(want), "{} {}: body {body}", case.tool, case.args);
        }
        assert_eq!(body["agent_id"], "tester", "{} {}: body {body}", case.tool, case.args);
        for want in case.says {
            assert!(said.contains(want), "{} {}: said {said:?}, wanted {want:?}", case.tool, case.args);
        }
    }
}

#[tokio::test]
async fn a_tool_missing_what_it_needs_refuses_before_it_asks_srv() {
    let refusals = [
        ("OpenBrowser", json!({}), "missing required parameter: url"),
        ("OpenBrowser", json!({ "url": "https://a.example", "allowed_origins": "x" }), "allowed_origins must be a list of strings"),
        ("OpenBrowser", json!({ "url": "https://a.example", "profile": 3 }), "profile must be a string"),
        ("BrowserFind", json!({}), "missing required parameter: name"),
        ("BrowserClick", json!({}), "missing required parameter: ref"),
        ("BrowserFill", json!({ "ref": "e1" }), "missing required parameter: text"),
        ("BrowserSelect", json!({ "ref": "e1" }), "missing required parameter: option"),
        ("BrowserCheck", json!({ "ref": "e1" }), "missing required parameter: checked"),
        ("BrowserFill", json!({ "ref": "e1", "text": "a", "vault": "v" }), "vault is for fill"),
        ("BrowserSetFiles", json!({ "ref": "e1" }), "missing required parameter: paths"),
        ("BrowserWaitFor", json!({ "text": "a", "gone": "b" }), "give at most one of"),
        ("BrowserWaitFor", json!({ "text": "a", "state": "idle" }), "give at most one of"),
        ("BrowserWaitFor", json!({ "state": "soon" }), "until_state is"),
        ("BrowserHandoff", json!({}), "missing required parameter: reason"),
        ("BrowserRequestBypass", json!({}), "missing required parameter: reason"),
        ("BrowserNavigate", json!({}), "missing required parameter: url"),
        ("BrowserEval", json!({}), "missing required parameter: script"),
        ("BrowserDispatchKey", json!({}), "requires exactly one of"),
        ("BrowserDispatchKey", json!({ "text": "a", "key": "b" }), "requires exactly one of"),
        ("BrowserTerminal", json!({}), "needs `text` to type"),
        ("BrowserFocusElement", json!({}), "missing required parameter: selector"),
    ];
    for (tool, args, wanted) in refusals {
        let srv = stub_srv("200 OK", ok(json!({})));
        let err = call_tool(&srv, tool, args.clone()).await.expect_err(&format!("{tool} {args} should refuse")).to_string();
        assert!(err.contains(wanted), "{tool} {args}: {err:?}, wanted {wanted:?}");
        assert!(srv.seen.lock().unwrap().is_empty(), "{tool} {args} asked srv");
    }
}

#[tokio::test]
async fn a_refusal_from_srv_names_the_tool_and_the_status() {
    let failures = [
        ("OpenBrowser", json!({ "url": "https://a.example" }), "Browser open failed: HTTP 500 Internal Server Error — \"boom\""),
        ("BrowserSnapshot", json!({}), "snapshot failed: HTTP 500 Internal Server Error — \"boom\""),
        ("BrowserFind", json!({ "name": "x" }), "find failed: HTTP 500 Internal Server Error — \"boom\""),
        ("BrowserClick", json!({ "ref": "e1" }), "BrowserClick e1 failed: HTTP 500 Internal Server Error — \"boom\""),
        ("BrowserSetFiles", json!({ "ref": "e1", "paths": ["/a"] }), "Browser set_files e1 failed: HTTP 500 Internal Server Error — \"boom\""),
        ("BrowserWaitFor", json!({ "text": "a" }), "Browser wait: HTTP 500 Internal Server Error — \"boom\""),
        ("BrowserHandoff", json!({ "reason": "r" }), "Browser handoff failed: HTTP 500 Internal Server Error — \"boom\""),
        ("BrowserRequestBypass", json!({ "reason": "r" }), "Browser request_auto_approve failed: HTTP 500 Internal Server Error — \"boom\""),
        ("BrowserEndBypass", json!({}), "Browser stop_auto_approve failed: HTTP 500 Internal Server Error — \"boom\""),
        ("BrowserLog", json!({}), "BrowserLog failed: HTTP 500 Internal Server Error — \"boom\""),
        ("BrowserNavigate", json!({ "url": "https://a.example" }), "navigate failed: HTTP 500 Internal Server Error — \"boom\""),
        ("BrowserBack", json!({}), "back failed: HTTP 500 Internal Server Error — \"boom\""),
        ("BrowserReload", json!({}), "reload failed: HTTP 500 Internal Server Error — \"boom\""),
        ("BrowserEval", json!({ "script": "1" }), "eval failed: HTTP 500 Internal Server Error — \"boom\""),
        ("BrowserDispatchKey", json!({ "key": "a" }), "dispatch_key failed: HTTP 500 Internal Server Error — \"boom\""),
        ("BrowserTerminal", json!({ "text": "a" }), "Browser terminal failed: HTTP 500 Internal Server Error — \"boom\""),
        ("BrowserFocusElement", json!({ "selector": "#a" }), "focus_element failed: HTTP 500 Internal Server Error — \"boom\""),
        ("BrowserFocusInfo", json!({}), "focus_info failed: HTTP 500 Internal Server Error — \"boom\""),
    ];
    for (tool, args, wanted) in failures {
        let srv = stub_srv("500 Internal Server Error", json!("boom"));
        let err = call_tool(&srv, tool, args.clone()).await.expect_err(&format!("{tool} should fail")).to_string();
        assert!(err.contains(wanted), "{tool} {args}: {err:?}, wanted {wanted:?}");
    }
}

#[tokio::test]
async fn a_name_that_is_not_a_browser_tool_is_not_in_the_family() {
    let srv = stub_srv("200 OK", ok(json!({})));
    assert!(call_tool(&srv, "BrowserNope", json!({})).await.is_err());
    assert!(srv.seen.lock().unwrap().is_empty());
}
