// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What each history tool sends to srv and what it tells the agent back, pinned against a
//! stand-in srv, so `call` can be split into one function per tool without changing either.

use super::super::test_stub::{stub_srv, Fixture};
use super::*;

/// One call: the request it sends (path with its query) and the text it returns.
struct Case {
    tool: &'static str,
    args: Value,
    reply: Value,
    path: &'static str,
    says: &'static [&'static str],
}

fn cases() -> Vec<Case> {
    let c = |tool, args, reply, path, says| Case { tool, args, reply, path, says };
    vec![
        c(
            "SearchHistory",
            json!({
                "query": "needle", "tool": "Bash", "role": "user", "since": 1, "until": 2,
                "max_sessions": 3, "limit": 4, "include_inferred": true
            }),
            json!({ "hits": ["h1"] }),
            "/agentmux/reactive/history/search?query=needle&agent=tester&tool=Bash&role=user&since=1&until=2&max_sessions=3&limit=4&include_inferred=true",
            &["\"hits\"", "\"h1\""],
        ),
        c("SearchHistory", json!({ "query": "x" }), json!({}), "/agentmux/reactive/history/search?query=x&agent=tester", &["{}"]),
        // A tool filter stands in for the query; a flag that is not true, or not a number, sends nothing.
        c(
            "SearchHistory",
            json!({ "tool": "Bash", "include_inferred": false, "limit": "ten" }),
            json!({}),
            "/agentmux/reactive/history/search?query=&agent=tester&tool=Bash",
            &["{}"],
        ),
        c(
            "GetAgentTranscript",
            json!({ "agent": "agent2", "max_lines": 50 }),
            json!({ "lines": ["l1"] }),
            "/agentmux/reactive/transcript?agent=agent2&max_lines=50",
            &["\"lines\"", "\"l1\""],
        ),
        c("GetAgentTranscript", json!({ "agent": "agent2" }), json!({}), "/agentmux/reactive/transcript?agent=agent2", &["{}"]),
        c(
            "ListConversations",
            json!({}),
            json!({ "conversations": ["c1"] }),
            "/api/v1/muxspect/conversations",
            &["\"conversations\"", "\"c1\""],
        ),
    ]
}

#[tokio::test]
async fn every_history_tool_sends_the_request_it_did_and_says_what_it_said() {
    for case in cases() {
        let srv = stub_srv("200 OK", case.reply.clone());
        let fx = Fixture::new(&srv);
        let said = call(case.tool, &case.args, &fx.cx()).await.unwrap_or_else(|e| panic!("{} {}: {e}", case.tool, case.args));
        let seen = srv.seen.lock().unwrap().clone();
        assert_eq!(seen.len(), 1, "{} {}: requests {seen:?}", case.tool, case.args);
        assert_eq!(seen[0].0, case.path, "{} {}", case.tool, case.args);
        for want in case.says {
            assert!(said.contains(want), "{} {}: said {said:?}, wanted {want:?}", case.tool, case.args);
        }
    }
}

#[tokio::test]
async fn a_search_by_a_process_with_no_agent_name_still_goes_out_without_one() {
    let srv = stub_srv("200 OK", json!({}));
    let fx = Fixture::new(&srv);
    std::env::remove_var("AGENTMUX_AGENT_ID");
    call("SearchHistory", &json!({ "query": "x" }), &fx.cx()).await.unwrap();
    assert_eq!(srv.seen.lock().unwrap()[0].0, "/agentmux/reactive/history/search?query=x");
    std::env::set_var("AGENTMUX_AGENT_ID", "  ");
    call("SearchHistory", &json!({ "query": "x" }), &fx.cx()).await.unwrap();
    assert_eq!(srv.seen.lock().unwrap()[1].0, "/agentmux/reactive/history/search?query=x");
}

#[tokio::test]
async fn a_tool_missing_what_it_needs_refuses_before_it_asks_srv() {
    let srv = stub_srv("200 OK", json!({}));
    let fx = Fixture::new(&srv);
    let no_url = ToolCtx { local_url: "", ..fx.cx() };
    let no_key = ToolCtx { auth_key: "", ..fx.cx() };
    let unset = "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set";
    let query = "query must be non-empty unless `tool` is given";
    let refusals = [
        ("SearchHistory", json!({}), fx.cx(), query),
        ("SearchHistory", json!({ "query": "  " }), fx.cx(), query),
        ("SearchHistory", json!({ "query": "x" }), no_url, unset),
        ("SearchHistory", json!({ "query": "x" }), no_key, unset),
        // The query is checked before the environment.
        ("SearchHistory", json!({}), no_url, query),
        ("GetAgentTranscript", json!({}), fx.cx(), "missing required parameter: agent"),
        ("GetAgentTranscript", json!({ "agent": "" }), fx.cx(), "missing required parameter: agent"),
        ("GetAgentTranscript", json!({ "agent": "a" }), no_url, unset),
        ("GetAgentTranscript", json!({ "agent": "a" }), no_key, unset),
        ("ListConversations", json!({}), no_url, unset),
        ("ListConversations", json!({}), no_key, unset),
    ];
    for (tool, args, cx, wanted) in refusals {
        let err = call(tool, &args, &cx).await.expect_err(&format!("{tool} {args} should refuse")).to_string();
        assert!(err.contains(wanted), "{tool} {args}: {err:?}, wanted {wanted:?}");
    }
    assert!(srv.seen.lock().unwrap().is_empty(), "a refused call asked srv");
}

#[tokio::test]
async fn a_refusal_from_srv_names_the_request_and_the_status() {
    let failures = [
        ("SearchHistory", json!({ "query": "x" }), "history search"),
        ("GetAgentTranscript", json!({ "agent": "a" }), "transcript fetch failed: HTTP 500 Internal Server Error — \"boom\""),
        ("ListConversations", json!({}), "conversations fetch failed: HTTP 500 Internal Server Error — \"boom\""),
    ];
    for (tool, args, wanted) in failures {
        let srv = stub_srv("500 Internal Server Error", json!("boom"));
        let fx = Fixture::new(&srv);
        let err = call(tool, &args, &fx.cx()).await.expect_err(&format!("{tool} should fail")).to_string();
        assert!(err.contains(wanted) && err.contains("500"), "{tool} {args}: {err:?}, wanted {wanted:?}");
    }
}

#[tokio::test]
async fn a_name_that_is_not_a_history_tool_is_not_in_the_family() {
    let srv = stub_srv("200 OK", json!({}));
    let fx = Fixture::new(&srv);
    assert!(call("HistoryNope", &json!({}), &fx.cx()).await.is_err());
    assert!(srv.seen.lock().unwrap().is_empty());
}
