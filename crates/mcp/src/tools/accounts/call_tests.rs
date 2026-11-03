// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What each Identity account tool sends to srv, here and on another machine, and what it tells
//! the agent back, pinned against a stand-in srv, so `call` can be split into one function per
//! tool without changing either.

use super::super::test_stub::{stub_srv, Fixture};
use super::*;

/// One call: the requests it makes (their paths), the last one's body fields named here, and
/// the text it returns.
struct Case {
    tool: &'static str,
    args: Value,
    reply: Value,
    paths: &'static [&'static str],
    sent: Value,
    says: &'static [&'static str],
}

fn cases() -> Vec<Case> {
    let c = |tool, args, reply, paths, sent, says| Case { tool, args, reply, paths, sent, says };
    let done = json!({ "status": "done", "result": { "accounts": ["a1"] } });
    vec![
        c(
            "IdentityAccounts",
            json!({}),
            json!({ "accounts": ["a1"] }),
            &["/api/v1/agent/identity/accounts?agent_id=tester&scope="],
            Value::Null,
            &["\"accounts\"", "\"a1\""],
        ),
        c(
            "IdentityAccounts",
            json!({ "scope": "all" }),
            json!({}),
            &["/api/v1/agent/identity/accounts?agent_id=tester&scope=all"],
            Value::Null,
            &["{}"],
        ),
        c(
            "IdentityValidate",
            json!({ "account_id": "acc-1" }),
            json!({ "valid": true }),
            &["/api/v1/agent/identity/validate"],
            json!({ "agent_id": "tester", "account_id": "acc-1" }),
            &["\"valid\": true"],
        ),
        c(
            "IdentitySwitch",
            json!({ "account": "acc-2", "agent": "Opaz", "when": "now" }),
            json!({ "switched": true }),
            &["/api/v1/agent/identity/switch"],
            json!({ "agent_id": "tester", "account": "acc-2", "target": "Opaz", "when": "now" }),
            &["\"switched\": true"],
        ),
        c(
            "IdentitySwitch",
            json!({ "account": "acc-2", "agent": "", "when": "" }),
            json!({}),
            &["/api/v1/agent/identity/switch"],
            json!({ "agent_id": "tester", "account": "acc-2", "target": null, "when": null }),
            &["{}"],
        ),
        // On another machine, the same tools go through that machine's srv as an operation.
        c(
            "IdentityAccounts",
            json!({ "host": " charlie " }),
            done.clone(),
            &["/api/v1/agent/remote/op"],
            json!({ "agent_id": "tester", "host": "charlie", "op": "identity.accounts", "params": {}, "reason": "Identity accounts" }),
            &["From charlie:\n{\n  \"accounts\": [\n    \"a1\"\n  ]\n}"],
        ),
        c(
            "IdentityAccounts",
            json!({ "host": "charlie", "reason": "checking quota" }),
            done.clone(),
            &["/api/v1/agent/remote/op"],
            json!({ "reason": "checking quota" }),
            &["From charlie:"],
        ),
        c(
            "IdentitySwitch",
            json!({ "host": "charlie", "agent": "Opaz", "account": "any with quota", "when": "now" }),
            json!({ "status": "queued", "result": { "queued": true } }),
            &["/api/v1/agent/remote/op"],
            json!({ "host": "charlie", "op": "identity.switch", "params": { "agent_id": "Opaz", "account": "any with quota", "when": "now" }, "reason": "Identity switch" }),
            &["From charlie [queued]:"],
        ),
        c(
            "IdentitySignin",
            json!({ "host": "charlie", "provider": "codex", "account": "me" }),
            json!({ "status": "done", "result": { "session": "s1" } }),
            &["/api/v1/agent/remote/op"],
            json!({ "op": "identity.signin.start", "params": { "provider": "codex", "account": "me" }, "reason": "Identity signin.start" }),
            &["From charlie:", "\"session\": \"s1\""],
        ),
        c(
            "IdentitySignin",
            json!({ "host": "charlie", "session": "s1", "code": "abc" }),
            json!({ "status": "done", "result": { "signed_in": true } }),
            &["/api/v1/agent/remote/op"],
            json!({ "op": "identity.signin.finish", "params": { "session": "s1", "code": "abc" } }),
            &["\"signed_in\": true"],
        ),
        // A started sign-in's link is also shown where the person is, when this machine's srv says it was.
        c(
            "IdentitySignin",
            json!({ "host": "charlie" }),
            json!({ "status": "done", "result": { "url": "https://x/y", "device_code": "dc-1" }, "shown_on": "narko" }),
            &["/api/v1/agent/remote/op", "/api/v1/agent/identity/signin/notice"],
            json!({ "agent_id": "tester", "provider": "claude", "machine": "charlie", "url": "https://x/y", "code": "dc-1" }),
            &["\"url\": \"https://x/y\"", "\n\nThe link is also on narko as a notice with an Open button."],
        ),
    ]
}

#[tokio::test]
async fn every_account_tool_sends_the_requests_it_did_and_says_what_it_said() {
    for case in cases() {
        let srv = stub_srv("200 OK", case.reply.clone());
        let fx = Fixture::new(&srv);
        let said = call(case.tool, &case.args, &fx.cx()).await.unwrap_or_else(|e| panic!("{} {}: {e}", case.tool, case.args));
        let seen = srv.seen.lock().unwrap().clone();
        let paths: Vec<&str> = seen.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(paths, case.paths, "{} {}", case.tool, case.args);
        let body = &seen.last().unwrap().1;
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
async fn a_sign_in_whose_link_is_not_shown_says_nothing_more() {
    // No url in the answer, or srv shows it nowhere: the answer alone.
    for reply in [json!({ "status": "done", "result": { "session": "s" } }), json!({ "status": "done", "result": { "url": "https://x" } })] {
        let srv = stub_srv("200 OK", reply);
        let fx = Fixture::new(&srv);
        let said = call("IdentitySignin", &json!({ "host": "charlie" }), &fx.cx()).await.unwrap();
        assert!(!said.contains("notice"), "{said}");
    }
}

#[tokio::test]
async fn a_tool_missing_what_it_needs_refuses_before_it_asks_srv() {
    let srv = stub_srv("200 OK", json!({}));
    let fx = Fixture::new(&srv);
    let no_url = ToolCtx { local_url: "", ..fx.cx() };
    let no_key = ToolCtx { auth_key: "", ..fx.cx() };
    let unset = "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set";
    let refusals = [
        ("IdentityValidate", json!({}), fx.cx(), "missing required parameter: account_id"),
        ("IdentityValidate", json!({ "account_id": "" }), fx.cx(), "missing required parameter: account_id"),
        ("IdentitySwitch", json!({}), fx.cx(), "missing required parameter: account"),
        ("IdentitySignin", json!({}), fx.cx(), "signin needs a host: it signs an account in on another machine"),
        ("IdentitySignin", json!({ "host": "  " }), fx.cx(), "signin needs a host"),
        ("IdentitySwitch", json!({ "host": "charlie", "agent": "Opaz" }), fx.cx(), "missing required parameter: account"),
        ("IdentitySwitch", json!({ "host": "charlie", "account": "x" }), fx.cx(), "with a host, name the agent on that machine"),
        ("IdentityAccounts", json!({}), no_url, unset),
        ("IdentityAccounts", json!({}), no_key, unset),
        ("IdentityValidate", json!({ "account_id": "a" }), no_url, unset),
        ("IdentitySwitch", json!({ "account": "a" }), no_key, unset),
        ("IdentityAccounts", json!({ "host": "charlie" }), no_url, unset),
        // The arguments are checked before the environment.
        ("IdentityValidate", json!({}), no_url, "missing required parameter: account_id"),
    ];
    for (tool, args, cx, wanted) in refusals {
        let err = call(tool, &args, &cx).await.expect_err(&format!("{tool} {args} should refuse")).to_string();
        assert!(err.contains(wanted), "{tool} {args}: {err:?}, wanted {wanted:?}");
    }
    assert!(srv.seen.lock().unwrap().is_empty(), "a refused call asked srv");
}

#[tokio::test]
async fn a_refusal_from_srv_names_the_endpoint_and_the_status() {
    let failures = [
        ("IdentityAccounts", json!({}), "identity/accounts failed: HTTP 500 Internal Server Error — \"boom\""),
        ("IdentityValidate", json!({ "account_id": "a" }), "identity/validate failed: HTTP 500 Internal Server Error — \"boom\""),
        ("IdentitySwitch", json!({ "account": "a" }), "identity/switch failed: HTTP 500 Internal Server Error — \"boom\""),
        ("IdentityAccounts", json!({ "host": "charlie" }), "identity.accounts on charlie: srv answered 500 Internal Server Error: \"boom\""),
    ];
    for (tool, args, wanted) in failures {
        let srv = stub_srv("500 Internal Server Error", json!("boom"));
        let fx = Fixture::new(&srv);
        let err = call(tool, &args, &fx.cx()).await.unwrap_err().to_string();
        assert_eq!(err, wanted, "{tool}");
    }
}

#[tokio::test]
async fn a_machine_that_refuses_or_fails_says_so_instead_of_returning_its_answer() {
    let refused = json!({ "status": "refused", "refused_reason": "remote_ops_off", "result": { "message": "off" } });
    let failed = json!({ "status": "failed", "result": { "error": "disk full" } });
    for (reply, wanted) in [
        (refused, "charlie refused identity.accounts (remote_ops_off): off. That machine has turned remote operations off"),
        (failed, "identity.accounts on charlie failed: disk full"),
    ] {
        let srv = stub_srv("200 OK", reply);
        let fx = Fixture::new(&srv);
        let err = call("IdentityAccounts", &json!({ "host": "charlie" }), &fx.cx()).await.unwrap_err().to_string();
        assert!(err.starts_with(wanted), "{err}");
    }
}

#[tokio::test]
async fn a_name_that_is_not_an_account_tool_is_not_in_the_family() {
    let srv = stub_srv("200 OK", json!({}));
    let fx = Fixture::new(&srv);
    assert!(call("IdentityNope", &json!({}), &fx.cx()).await.is_err());
    assert!(srv.seen.lock().unwrap().is_empty());
}
