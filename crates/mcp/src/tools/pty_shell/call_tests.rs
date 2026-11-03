// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What each PtyShell tool sends to srv and what it tells the agent back, pinned against a
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
            "PtyShell",
            json!({ "cwd": "/w", "rows": 24, "cols": 80, "connection": "me@box" }),
            json!({ "shell_id": "sh1" }),
            "/api/v1/ptyshell/create",
            json!({ "agent_block_id": "blk", "cwd": "/w", "rows": 24, "cols": 80, "connection": "me@box" }),
            &["sh1"],
        ),
        c(
            "PtyShellInput",
            json!({ "shell_id": "sh1", "text": "ls\n" }),
            json!({ "written": true }),
            "/api/v1/ptyshell/input",
            json!({ "shell_id": "sh1", "agent_block_id": "blk", "text": "ls\n" }),
            &["wrote to shell sh1"],
        ),
        c(
            "PtyShellInput",
            json!({ "shell_id": "sh1", "text": "x" }),
            json!({ "written": false, "error": "not yours" }),
            "/api/v1/ptyshell/input",
            json!({}),
            &["shell sh1: write failed — not yours"],
        ),
        c(
            "PtyShellInput",
            json!({ "shell_id": "sh1", "text": "x" }),
            json!({ "written": false }),
            "/api/v1/ptyshell/input",
            json!({}),
            &["write failed — unknown reason"],
        ),
        c(
            "PtyShellResize",
            json!({ "shell_id": "sh1", "rows": 30, "cols": 100 }),
            json!({ "resized": true }),
            "/api/v1/ptyshell/resize",
            json!({ "shell_id": "sh1", "agent_block_id": "blk", "rows": 30, "cols": 100 }),
            &["resized shell sh1 to 30x100"],
        ),
        c(
            "PtyShellResize",
            json!({ "shell_id": "sh1", "rows": 30, "cols": 100 }),
            json!({ "resized": false, "error": "gone" }),
            "/api/v1/ptyshell/resize",
            json!({}),
            &["shell sh1: resize failed — gone"],
        ),
        c(
            "PtyShellRead",
            json!({ "shell_id": "sh1", "tail_lines": 20 }),
            json!({ "content": "out", "truncated": true, "end": 3 }),
            "/api/v1/ptyshell/read",
            json!({ "shell_id": "sh1", "agent_block_id": "blk", "tail_lines": 20 }),
            &["[...output truncated...]\nout"],
        ),
        c(
            "PtyShellRead",
            json!({ "shell_id": "sh1" }),
            json!({ "content": "out", "truncated": false, "end": 3 }),
            "/api/v1/ptyshell/read",
            json!({}),
            &["out"],
        ),
        c(
            "PtyShellStatus",
            json!({ "shell_id": "sh1" }),
            json!({ "running": true }),
            "/api/v1/ptyshell/status",
            json!({ "shell_id": "sh1", "agent_block_id": "blk" }),
            &["shell sh1 is running"],
        ),
        c(
            "PtyShellStatus",
            json!({ "shell_id": "sh1" }),
            json!({ "running": false, "exit_code": 2 }),
            "/api/v1/ptyshell/status",
            json!({}),
            &["shell sh1 is not running — exit_code: 2"],
        ),
        c(
            "PtyShellStop",
            json!({ "shell_id": "sh1" }),
            json!({ "released": true }),
            "/api/v1/ptyshell/stop",
            json!({ "shell_id": "sh1", "agent_block_id": "blk" }),
            &["released the agent lock on shell sh1 (the shell itself keeps running)"],
        ),
        c(
            "PtyShellStop",
            json!({ "shell_id": "sh1" }),
            json!({ "released": false }),
            "/api/v1/ptyshell/stop",
            json!({}),
            &["no active agent lock to release"],
        ),
    ]
}

#[tokio::test]
async fn every_pty_shell_tool_sends_the_request_it_did_and_says_what_it_said() {
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
        ("PtyShellInput", json!({}), "missing required parameter: shell_id"),
        ("PtyShellInput", json!({ "shell_id": "s" }), "missing required parameter: text"),
        ("PtyShellResize", json!({ "shell_id": "s" }), "missing required parameter: rows"),
        ("PtyShellResize", json!({ "shell_id": "s", "rows": 1 }), "missing required parameter: cols"),
        ("PtyShellRead", json!({}), "missing required parameter: shell_id"),
        ("PtyShellStatus", json!({}), "missing required parameter: shell_id"),
        ("PtyShellStop", json!({}), "missing required parameter: shell_id"),
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
        ("PtyShell", json!({}), "ptyshell/create failed: HTTP 500 Internal Server Error — \"boom\""),
        ("PtyShellInput", json!({ "shell_id": "s", "text": "x" }), "ptyshell/input failed: HTTP 500 Internal Server Error — \"boom\""),
        ("PtyShellResize", json!({ "shell_id": "s", "rows": 1, "cols": 1 }), "ptyshell/resize failed: HTTP 500 Internal Server Error — \"boom\""),
        ("PtyShellRead", json!({ "shell_id": "s" }), "ptyshell/read failed: HTTP 500 Internal Server Error — \"boom\""),
        ("PtyShellStatus", json!({ "shell_id": "s" }), "ptyshell/status failed: HTTP 500 Internal Server Error — \"boom\""),
        ("PtyShellStop", json!({ "shell_id": "s" }), "ptyshell/stop failed: HTTP 500 Internal Server Error — \"boom\""),
    ];
    for (tool, args, wanted) in failures {
        let srv = stub_srv("500 Internal Server Error", json!("boom"));
        let fx = Fixture::new(&srv);
        let err = call(tool, &args, &fx.cx()).await.expect_err(&format!("{tool} should fail")).to_string();
        assert!(err.contains(wanted), "{tool} {args}: {err:?}, wanted {wanted:?}");
    }
}

#[tokio::test]
async fn a_name_that_is_not_a_pty_shell_tool_is_not_in_the_family() {
    let srv = stub_srv("200 OK", json!({}));
    let fx = Fixture::new(&srv);
    assert!(call("PtyShellNope", &json!({}), &fx.cx()).await.is_err());
    assert!(srv.seen.lock().unwrap().is_empty());
}
