// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What each Shell tool sends to srv and what it tells the agent back, pinned against a
//! stand-in srv, so `call` can be split into one function per tool without changing either.

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
            "Shell",
            json!({ "cmd": "make", "title": "build", "cwd": "/w", "env": { "A": "1" }, "capture_stdin": true, "connection": "me@box", "stdin_secret": "pw" }),
            json!({ "shell_id": "sh1" }),
            "/api/v1/shell/create",
            json!({ "agent_block_id": "blk", "cmd": "make", "title": "build", "cwd": "/w", "env": { "A": "1" }, "capture_stdin": true, "connection": "me@box", "stdin_secret": "pw" }),
            &["sh1"],
        ),
        c("Shell", json!({ "cmd": "ls" }), json!({ "shell_id": "sh2" }), "/api/v1/shell/create", json!({ "cmd": "ls", "title": "ls" }), &["sh2"]),
        c(
            "ShellStop",
            json!({ "shell_id": "sh1" }),
            json!({ "stopped": true }),
            "/api/v1/shell/stop",
            json!({ "shell_id": "sh1" }),
            &["stopped shell sh1"],
        ),
        c(
            "ShellStop",
            json!({ "shell_id": "sh1" }),
            json!({ "stopped": false }),
            "/api/v1/shell/stop",
            json!({}),
            &["shell sh1 was not running (unknown or already exited)"],
        ),
        c(
            "ShellInput",
            json!({ "shell_id": "sh1", "text": "y\n" }),
            json!({ "written": true }),
            "/api/v1/shell/input",
            json!({ "shell_id": "sh1", "text": "y\n" }),
            &["wrote to shell sh1"],
        ),
        c(
            "ShellInput",
            json!({ "shell_id": "sh1", "text": "y" }),
            json!({ "written": false, "reason": "stdin_not_captured" }),
            "/api/v1/shell/input",
            json!({}),
            &["started without capture_stdin=true"],
        ),
        c(
            "ShellInput",
            json!({ "shell_id": "sh1", "text": "y" }),
            json!({ "written": false, "reason": "write_failed" }),
            "/api/v1/shell/input",
            json!({}),
            &["shell sh1 closed its stdin — input discarded"],
        ),
        c(
            "ShellInput",
            json!({ "shell_id": "sh1", "text": "y" }),
            json!({ "written": false }),
            "/api/v1/shell/input",
            json!({}),
            &["shell sh1 is not running — input discarded"],
        ),
        c(
            "ShellStatus",
            json!({ "shell_id": "sh1" }),
            json!({ "running": true, "line_count": 12 }),
            "/api/v1/shell/status",
            json!({ "shell_id": "sh1" }),
            &["shell sh1 is running (12 lines so far)"],
        ),
        c(
            "ShellStatus",
            json!({ "shell_id": "sh1" }),
            json!({ "running": false, "exit_code": 3, "line_count": 40 }),
            "/api/v1/shell/status",
            json!({}),
            &["shell sh1 has exited — exit_code: 3, 40 lines total"],
        ),
        c("ConnList", json!({}), json!({ "connections": ["a"] }), "/api/v1/conn/list", json!({}), &["\"connections\""]),
    ]
}

#[tokio::test]
async fn every_shell_tool_sends_the_request_it_did_and_says_what_it_said() {
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
        ("Shell", json!({}), "missing required parameter: cmd"),
        ("Shell", json!({ "cmd": "x", "secrets": 3 }), "secrets must be an object"),
        ("Shell", json!({ "cmd": "x", "secret_files": 3 }), "secret_files must be an object"),
        ("ShellStop", json!({}), "missing required parameter: shell_id"),
        ("ShellInput", json!({}), "missing required parameter: shell_id"),
        ("ShellInput", json!({ "shell_id": "s" }), "missing required parameter: text"),
        ("ShellStatus", json!({}), "missing required parameter: shell_id"),
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
async fn a_refusal_from_srv_names_the_endpoint_and_the_status() {
    let failures = [
        ("Shell", json!({ "cmd": "x" }), "shell/create failed: HTTP 500 Internal Server Error — \"boom\""),
        ("ShellStop", json!({ "shell_id": "s" }), "shell/stop failed: HTTP 500 Internal Server Error — \"boom\""),
        ("ShellInput", json!({ "shell_id": "s", "text": "x" }), "shell/input failed: HTTP 500 Internal Server Error — \"boom\""),
        ("ShellStatus", json!({ "shell_id": "s" }), "shell/status failed: HTTP 500 Internal Server Error — \"boom\""),
        ("ConnList", json!({}), "Terminal connections"),
    ];
    for (tool, args, wanted) in failures {
        let srv = stub_srv("500 Internal Server Error", json!("boom"));
        let fx = Fixture::new(&srv);
        let err = call(tool, &args, &fx.cx()).await.expect_err(&format!("{tool} should fail")).to_string();
        assert!(err.contains(wanted), "{tool} {args}: {err:?}, wanted {wanted:?}");
    }
}

#[tokio::test]
async fn a_name_that_is_not_a_shell_tool_is_not_in_the_family() {
    let srv = stub_srv("200 OK", json!({}));
    let fx = Fixture::new(&srv);
    assert!(call("ShellNope", &json!({}), &fx.cx()).await.is_err());
    assert!(srv.seen.lock().unwrap().is_empty());
}
