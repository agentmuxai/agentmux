// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What each Memory tool sends to srv and what it tells the agent back, pinned against a
//! stand-in srv, so `call` can be split into one function per tool without changing either.

use super::super::test_stub::{stub_srv, Fixture};
use super::*;

/// One call: the request it sends (path with its query, and the body fields named here) and
/// the text it returns.
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
        c("MemoryList", json!({}), json!({ "files": ["a.md"] }), "/api/v1/agent/memory/list?agent_id=tester", json!({}), &["\"files\"", "\"a.md\""]),
        c(
            "MemoryRead",
            json!({ "filename": "a.md" }),
            json!({ "content": "hello" }),
            "/api/v1/agent/memory/read?agent_id=tester&filename=a.md",
            json!({}),
            &["hello"],
        ),
        c(
            "MemoryRead",
            json!({ "filename": "a.md" }),
            json!({ "other": 1 }),
            "/api/v1/agent/memory/read?agent_id=tester&filename=a.md",
            json!({}),
            &["\"other\": 1"],
        ),
        c(
            "MemoryWrite",
            json!({ "filename": "a.md", "content": "body", "provenance": { "by": "me" } }),
            json!({}),
            "/api/v1/agent/memory/write",
            json!({ "agent_id": "tester", "filename": "a.md", "content": "body", "provenance": { "by": "me" } }),
            &["Wrote memory file \"a.md\""],
        ),
        c(
            "MemoryWrite",
            json!({ "filename": "a.md", "content": "" }),
            json!({}),
            "/api/v1/agent/memory/write",
            json!({ "agent_id": "tester", "filename": "a.md", "content": "" }),
            &["Wrote memory file \"a.md\""],
        ),
        c(
            "MemoryHistory",
            json!({ "filename": "a.md" }),
            json!({ "versions": ["v1"] }),
            "/api/v1/agent/memory/history?agent_id=tester&filename=a.md",
            json!({}),
            &["\"versions\"", "\"v1\""],
        ),
        c(
            "MemoryDiff",
            json!({ "from_version_id": "v1", "to_version_id": "v2" }),
            json!({ "diff": "-a\n+b" }),
            "/api/v1/agent/memory/diff?agent_id=tester&from_version_id=v1&to_version_id=v2",
            json!({}),
            &["-a\n+b"],
        ),
        c(
            "MemoryDiff",
            json!({ "from_version_id": "v1", "to_version_id": "v2" }),
            json!({ "other": 1 }),
            "/api/v1/agent/memory/diff?agent_id=tester&from_version_id=v1&to_version_id=v2",
            json!({}),
            &["\"other\": 1"],
        ),
        c(
            "MemoryRevert",
            json!({ "filename": "a.md", "target_version_id": "v1" }),
            json!({}),
            "/api/v1/agent/memory/revert",
            json!({ "agent_id": "tester", "filename": "a.md", "target_version_id": "v1" }),
            &["Reverted \"a.md\" to version v1"],
        ),
    ]
}

#[tokio::test]
async fn every_memory_tool_sends_the_request_it_did_and_says_what_it_said() {
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
async fn a_read_and_a_diff_give_the_text_itself_not_the_json_around_it() {
    for (tool, args, reply, wanted) in [
        ("MemoryRead", json!({ "filename": "a.md" }), json!({ "content": "hello", "size": 5 }), "hello"),
        ("MemoryDiff", json!({ "from_version_id": "v1", "to_version_id": "v2" }), json!({ "diff": "-a\n+b" }), "-a\n+b"),
    ] {
        let srv = stub_srv("200 OK", reply);
        let fx = Fixture::new(&srv);
        assert_eq!(call(tool, &args, &fx.cx()).await.unwrap(), wanted, "{tool}");
    }
}

#[tokio::test]
async fn a_write_without_provenance_sends_none() {
    let srv = stub_srv("200 OK", json!({}));
    let fx = Fixture::new(&srv);
    call("MemoryWrite", &json!({ "filename": "a.md", "content": "x" }), &fx.cx()).await.unwrap();
    assert!(srv.seen.lock().unwrap()[0].1.get("provenance").is_none());
}

#[tokio::test]
async fn a_tool_missing_what_it_needs_refuses_before_it_asks_srv() {
    let refusals = [
        ("MemoryRead", json!({}), "missing required parameter: filename"),
        ("MemoryRead", json!({ "filename": "" }), "missing required parameter: filename"),
        ("MemoryWrite", json!({}), "missing required parameter: filename"),
        ("MemoryWrite", json!({ "filename": "a.md" }), "missing required parameter: content"),
        ("MemoryHistory", json!({}), "missing required parameter: filename"),
        ("MemoryDiff", json!({}), "missing required parameter: from_version_id"),
        ("MemoryDiff", json!({ "from_version_id": "v1" }), "missing required parameter: to_version_id"),
        ("MemoryRevert", json!({}), "missing required parameter: filename"),
        ("MemoryRevert", json!({ "filename": "a.md" }), "missing required parameter: target_version_id"),
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
        ("MemoryList", json!({}), "memory/list"),
        ("MemoryRead", json!({ "filename": "a.md" }), "memory/read"),
        ("MemoryWrite", json!({ "filename": "a.md", "content": "x" }), "memory/write"),
        ("MemoryHistory", json!({ "filename": "a.md" }), "memory/history"),
        ("MemoryDiff", json!({ "from_version_id": "v1", "to_version_id": "v2" }), "memory/diff"),
        ("MemoryRevert", json!({ "filename": "a.md", "target_version_id": "v1" }), "memory/revert"),
    ];
    for (tool, args, endpoint) in failures {
        let srv = stub_srv("500 Internal Server Error", json!("boom"));
        let fx = Fixture::new(&srv);
        let err = call(tool, &args, &fx.cx()).await.expect_err(&format!("{tool} should fail")).to_string();
        assert_eq!(err, format!("{endpoint} failed: HTTP 500 Internal Server Error — \"boom\""), "{tool}");
    }
}

#[tokio::test]
async fn a_name_that_is_not_a_memory_tool_is_not_in_the_family() {
    let srv = stub_srv("200 OK", json!({}));
    let fx = Fixture::new(&srv);
    assert!(call("MemoryNope", &json!({}), &fx.cx()).await.is_err());
    assert!(srv.seen.lock().unwrap().is_empty());
}
