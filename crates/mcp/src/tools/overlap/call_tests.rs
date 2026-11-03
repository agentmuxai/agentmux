// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What each coordination tool sends to srv and what it tells the agent back, pinned against a
//! stand-in srv, so `call` can be split into one function per tool without changing either.

use super::super::test_stub::{stub_srv, Fixture};
use super::*;

/// One call: the request it sends (path with its query, and the body fields named here, `null`
/// for a request without a body) and the text it returns.
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
            "WhoIsWorkingOn",
            json!({ "path": " crates/srv ", "repo": "o/r", "branch": "b", "query": "q", "topic": "ignored" }),
            json!({ "agents": [{ "agent": "Maricon" }] }),
            "/api/v1/work-facts/who?path=crates%2Fsrv&repo=o%2Fr&branch=b&query=q&agent=tester&block=blk",
            Value::Null,
            &["\"agents\"", "\"Maricon\""],
        ),
        c("WhoIsWorkingOn", json!({}), json!({}), "/api/v1/work-facts/who?agent=tester&block=blk", Value::Null, &["{}"]),
        // A PR is asked about by number: it needs a repository on GitHub, and a URL names one.
        c(
            "WhoIsWorkingOn",
            json!({ "pr": "https://github.com/o/r/pull/12" }),
            json!({}),
            "/api/v1/work-facts/who?agent=tester&block=blk&pr=12&repo=o%2Fr",
            Value::Null,
            &["\"pr_error\": \"pr needs a repository on GitHub (owner/repo)\""],
        ),
        c(
            "WhoIsWorkingOn",
            json!({ "pr": "#12", "repo": "O/R" }),
            json!({ "asked": { "repo": "local:me" } }),
            "/api/v1/work-facts/who?repo=O%2FR&agent=tester&block=blk&pr=12",
            Value::Null,
            &["\"pr_error\""],
        ),
        c(
            "ClaimWork",
            json!({ "path": " crates/srv ", "topic": "", "note": "presence work", "ttl_minutes": 45, "repo": null }),
            json!({ "id": "c1" }),
            "/api/v1/work-claims",
            json!({ "path": "crates/srv", "note": "presence work", "ttl_minutes": 45, "agent": "tester", "block": "blk" }),
            &["\"id\": \"c1\""],
        ),
        c(
            "ReleaseWork",
            json!({ "id": "c1" }),
            json!({ "released": 1 }),
            "/api/v1/work-claims/release",
            json!({ "id": "c1", "agent": "tester", "block": "blk" }),
            &["\"released\": 1"],
        ),
        c("ReleaseWork", json!({}), json!({ "released": 3 }), "/api/v1/work-claims/release", json!({ "id": null, "agent": "tester" }), &["\"released\": 3"]),
        c("QuotaMode", json!({}), json!({ "mode": "off" }), "/api/v1/quota-mode", Value::Null, &["\"mode\": \"off\""]),
        c(
            "QuotaMode",
            json!({ "mode": "tight", "min_priority": "high", "other": 1 }),
            json!({ "mode": "tight" }),
            "/api/v1/quota-mode",
            json!({ "mode": "tight", "min_priority": "high", "agent": "tester", "block": "blk" }),
            &["\"mode\": \"tight\""],
        ),
        c(
            "PostBulletin",
            json!({ "message": "m", "key": "k", "targets": ["a"], "expires_minutes": 5, "other": 1, "note": null }),
            json!({ "posted": true }),
            "/api/v1/bulletin",
            json!({ "message": "m", "key": "k", "targets": ["a"], "expires_minutes": 5, "agent": "tester", "block": "blk" }),
            &["\"posted\": true"],
        ),
        c("ReadBulletin", json!({}), json!({ "items": [] }), "/api/v1/bulletin?agent=tester&block=blk", Value::Null, &["\"items\""]),
        c(
            "PostToChannel",
            json!({ "message": "regenerate INDEX.md", "path": "docs/specs", "pr": "https://github.com/o/r/pull/12", "topic": "t", "branch": "b" }),
            json!({ "id": "p1" }),
            "/api/v1/coord/posts",
            json!({ "message": "regenerate INDEX.md", "path": "docs/specs", "repo": "o/r", "pr": 12, "topic": "t", "branch": "b", "agent": "tester", "block": "blk" }),
            &["\"id\": \"p1\""],
        ),
        c(
            "ReadChannel",
            json!({ "path": "a.rs", "pr": 7, "since_minutes": 60, "limit": 5, "query": "index" }),
            json!({ "posts": [] }),
            "/api/v1/coord/posts?path=a.rs&query=index&agent=tester&block=blk&pr=7&since_minutes=60&limit=5",
            Value::Null,
            &["\"posts\""],
        ),
    ]
}

#[tokio::test]
async fn every_coordination_tool_sends_the_request_it_did_and_says_what_it_said() {
    for case in cases() {
        let srv = stub_srv("200 OK", case.reply.clone());
        let fx = Fixture::new(&srv);
        let said = call(case.tool, &case.args, &fx.cx()).await.unwrap_or_else(|e| panic!("{} {}: {e}", case.tool, case.args));
        let seen = srv.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 1, "{} {}: requests {seen:?}", case.tool, case.args);
        let (path, body) = &seen[0];
        assert_eq!(path, case.path, "{} {}", case.tool, case.args);
        match case.sent.as_object() {
            Some(fields) => {
                for (key, want) in fields {
                    assert_eq!(body.get(key).unwrap_or(&Value::Null), want, "{} {}: body {body}", case.tool, case.args);
                }
            }
            None => assert!(body.is_null(), "{} {}: sent a body {body}", case.tool, case.args),
        }
        for want in case.says {
            assert!(said.contains(want), "{} {}: said {said:?}, wanted {want:?}", case.tool, case.args);
        }
    }
}

#[tokio::test]
async fn a_tool_that_cannot_be_sent_refuses_before_it_asks_srv() {
    let srv = stub_srv("200 OK", json!({}));
    let fx = Fixture::new(&srv);
    let no_url = ToolCtx { local_url: "", ..fx.cx() };
    let no_key = ToolCtx { auth_key: "", ..fx.cx() };
    let unset = "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set";
    let bad_pr = "pr must be a PR number (12, \"#12\") or its GitHub URL";
    let clash = "pr is a PR of o/r, but repo is o/other: pass one or the other";
    let mut refusals = vec![
        ("WhoIsWorkingOn", json!({ "pr": "nope" }), fx.cx(), bad_pr),
        ("WhoIsWorkingOn", json!({ "pr": "https://github.com/o/r/pull/12", "repo": "o/other" }), fx.cx(), clash),
        ("QuotaMode", json!({ "min_priority": "high" }), fx.cx(), "quota_mode: min_priority only applies together with mode (off or tight)"),
        ("QuotaMode", json!({ "min_priority": "high", "mode": null }), fx.cx(), "min_priority only applies together with mode"),
        ("PostToChannel", json!({ "message": "x", "pr": "nope" }), fx.cx(), bad_pr),
        ("PostToChannel", json!({ "message": "x", "repo": "o/other", "pr": "https://github.com/o/r/pull/12" }), fx.cx(), clash),
        ("ReadChannel", json!({ "pr": "nope" }), fx.cx(), bad_pr),
    ];
    for tool in ["WhoIsWorkingOn", "ClaimWork", "ReleaseWork", "QuotaMode", "PostBulletin", "ReadBulletin", "PostToChannel", "ReadChannel"] {
        refusals.push((tool, json!({ "message": "x" }), no_url, unset));
        refusals.push((tool, json!({ "message": "x" }), no_key, unset));
    }
    for (tool, args, cx, wanted) in refusals {
        let err = call(tool, &args, &cx).await.expect_err(&format!("{tool} {args} should refuse")).to_string();
        assert!(err.contains(wanted), "{tool} {args}: {err:?}, wanted {wanted:?}");
    }
    assert!(srv.seen.lock().unwrap().is_empty(), "a refused call asked srv");
}

#[tokio::test]
async fn a_refusal_from_srv_names_the_tool_and_the_status() {
    let failures = [
        ("WhoIsWorkingOn", json!({}), "Presence who"),
        ("ClaimWork", json!({}), "ClaimWork"),
        ("ReleaseWork", json!({}), "ReleaseWork"),
        ("QuotaMode", json!({}), "QuotaMode"),
        ("QuotaMode", json!({ "mode": "off" }), "QuotaMode"),
        ("PostBulletin", json!({ "message": "m" }), "PostBulletin"),
        ("ReadBulletin", json!({}), "ReadBulletin"),
        ("PostToChannel", json!({ "message": "m" }), "PostToChannel"),
        ("ReadChannel", json!({}), "ReadChannel"),
    ];
    for (tool, args, wanted) in failures {
        let srv = stub_srv("500 Internal Server Error", json!("boom"));
        let fx = Fixture::new(&srv);
        let err = call(tool, &args, &fx.cx()).await.expect_err(&format!("{tool} should fail")).to_string();
        assert!(err.contains(wanted) && err.contains("500"), "{tool} {args}: {err:?}, wanted {wanted:?}");
    }
}

#[tokio::test]
async fn a_name_that_is_not_a_coordination_tool_is_not_in_the_family() {
    let srv = stub_srv("200 OK", json!({}));
    let fx = Fixture::new(&srv);
    assert!(call("OverlapNope", &json!({}), &fx.cx()).await.is_err());
    assert!(srv.seen.lock().unwrap().is_empty());
}
