// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What each Global Memory tool sends to srv and what it tells the agent back, pinned against a
//! stand-in srv, so `call` can be split into one function per tool without changing either.

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
        c("GlobalMemoryList", json!({}), json!({ "entries": [{ "id": "e1" }] }), "/api/v1/agent/globalmemory/list", json!({}), &["\"id\": \"e1\""]),
        c(
            "GlobalMemoryRead",
            json!({ "id": "e1" }),
            json!({ "content": "the text" }),
            "/api/v1/agent/globalmemory/read?id=e1",
            json!({}),
            &["the text"],
        ),
        c(
            "GlobalMemoryRead",
            json!({ "id": "e1" }),
            json!({ "name": "n" }),
            "/api/v1/agent/globalmemory/read?id=e1",
            json!({}),
            &["\"name\": \"n\""],
        ),
        c(
            "GlobalMemoryWrite",
            json!({ "name": "N", "content": "C", "id": "e1", "reach": "fleet", "provenance": { "source": "s" } }),
            json!({ "id": "e1" }),
            "/api/v1/agent/globalmemory/write",
            json!({ "agent_id": "tester", "name": "N", "content": "C", "id": "e1", "reach": "fleet", "provenance": { "source": "s" } }),
            &["Wrote Global Memory entry \"N\" (id: e1)"],
        ),
        c(
            "GlobalMemoryWrite",
            json!({ "name": "N", "content": "" }),
            json!({}),
            "/api/v1/agent/globalmemory/write",
            json!({ "name": "N", "content": "" }),
            &["(id: ?)"],
        ),
        c(
            "GlobalMemoryReach",
            json!({ "id": "e1", "reach": "fleet" }),
            json!({}),
            "/api/v1/agent/globalmemory/reach",
            json!({ "id": "e1", "reach": "fleet", "agent_id": "tester" }),
            &["now goes to every proven machine on the LAN"],
        ),
        c(
            "GlobalMemoryReach",
            json!({ "id": "e1", "reach": "machine" }),
            json!({}),
            "/api/v1/agent/globalmemory/reach",
            json!({}),
            &["now stays on this machine"],
        ),
        c(
            "GlobalMemoryRemove",
            json!({ "id": "e1" }),
            json!({}),
            "/api/v1/agent/globalmemory/remove",
            json!({ "id": "e1", "agent_id": "tester" }),
            &["Removed Global Memory entry e1. It is kept in history"],
        ),
        c(
            "GlobalMemoryHistory",
            json!({ "id": "e1" }),
            json!({ "versions": [1] }),
            "/api/v1/agent/globalmemory/history?id=e1",
            json!({}),
            &["\"versions\""],
        ),
        c(
            "GlobalMemoryDiff",
            json!({ "id": "e1", "from_version_id": "v1", "to_version_id": "v2" }),
            json!({ "diff": "-a\n+b" }),
            "/api/v1/agent/globalmemory/diff?id=e1&from_version_id=v1&to_version_id=v2",
            json!({}),
            &["-a\n+b"],
        ),
        c(
            "GlobalMemoryRevert",
            json!({ "id": "e1", "version_id": "v1" }),
            json!({}),
            "/api/v1/agent/globalmemory/revert",
            json!({ "agent_id": "tester", "id": "e1", "version_id": "v1" }),
            &["Reverted Global Memory entry e1 to version v1"],
        ),
    ]
}

#[tokio::test]
async fn every_global_memory_tool_sends_the_request_it_did_and_says_what_it_said() {
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
        ("GlobalMemoryRead", json!({}), "missing required parameter: id"),
        ("GlobalMemoryWrite", json!({}), "missing required parameter: name"),
        ("GlobalMemoryWrite", json!({ "name": "N" }), "missing required parameter: content"),
        ("GlobalMemoryReach", json!({}), "missing required parameter: id"),
        ("GlobalMemoryReach", json!({ "id": "e" }), "missing required parameter: reach"),
        ("GlobalMemoryRemove", json!({}), "missing required parameter: id"),
        ("GlobalMemoryHistory", json!({}), "missing required parameter: id"),
        ("GlobalMemoryDiff", json!({}), "missing required parameter: id"),
        ("GlobalMemoryDiff", json!({ "id": "e" }), "missing required parameter: from_version_id"),
        ("GlobalMemoryDiff", json!({ "id": "e", "from_version_id": "a" }), "missing required parameter: to_version_id"),
        ("GlobalMemoryRevert", json!({}), "missing required parameter: id"),
        ("GlobalMemoryRevert", json!({ "id": "e" }), "missing required parameter: version_id"),
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
        ("GlobalMemoryList", json!({}), "globalmemory/list"),
        ("GlobalMemoryRead", json!({ "id": "e" }), "globalmemory/read"),
        ("GlobalMemoryWrite", json!({ "name": "N", "content": "C" }), "globalmemory/write"),
        ("GlobalMemoryReach", json!({ "id": "e", "reach": "fleet" }), "globalmemory/reach"),
        ("GlobalMemoryRemove", json!({ "id": "e" }), "globalmemory/remove"),
        ("GlobalMemoryHistory", json!({ "id": "e" }), "globalmemory/history"),
        ("GlobalMemoryDiff", json!({ "id": "e", "from_version_id": "a", "to_version_id": "b" }), "globalmemory/diff"),
        ("GlobalMemoryRevert", json!({ "id": "e", "version_id": "v" }), "globalmemory/revert"),
    ];
    for (tool, args, endpoint) in failures {
        let srv = stub_srv("500 Internal Server Error", json!("boom"));
        let fx = Fixture::new(&srv);
        let err = call(tool, &args, &fx.cx()).await.expect_err(&format!("{tool} should fail")).to_string();
        let wanted = format!("{endpoint} failed: HTTP 500 Internal Server Error — \"boom\"");
        assert!(err.contains(&wanted), "{tool} {args}: {err:?}, wanted {wanted:?}");
    }
}

#[tokio::test]
async fn a_name_that_is_not_a_global_memory_tool_is_not_in_the_family() {
    let srv = stub_srv("200 OK", json!({}));
    let fx = Fixture::new(&srv);
    assert!(call("GlobalMemoryNope", &json!({}), &fx.cx()).await.is_err());
    assert!(srv.seen.lock().unwrap().is_empty());
}
