// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What each Loop tool tells the agent and leaves running, pinned against a stand-in srv, so
//! `call` can be split into one function per tool without changing either.

use super::super::test_stub::{stub_srv, Fixture, StubSrv};
use super::*;
use std::time::Duration;

async fn until(what: &str, mut ok: impl FnMut() -> bool) {
    for _ in 0..200 {
        if ok() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    panic!("{what}");
}

fn injected(srv: &StubSrv) -> Vec<Value> {
    srv.seen.lock().unwrap().iter().filter(|(path, _)| path == "/agentmux/reactive/inject").map(|(_, body)| body.clone()).collect()
}

#[tokio::test]
async fn a_loop_that_fires_at_once_injects_its_prompt_and_stops_at_its_cap() {
    let srv = stub_srv("200 OK", json!({}));
    let fx = Fixture::new(&srv);
    let said = call("Loop", &json!({ "prompt": "check the build", "to": "agent2", "interval": "30s", "immediate": true, "max_iterations": 1 }), &fx.cx())
        .await
        .unwrap();
    assert_eq!(
        said,
        "Started loop-1: injecting to 'agent2' every 30s, auto-stops after 1 fires, first run now. Stop it with Schedule action \"delete\" and id loop-1; Schedule action \"list\" shows them all."
    );
    until("the loop never fired", || !injected(&srv).is_empty()).await;
    let sent = injected(&srv);
    assert_eq!(sent.len(), 1);
    assert_eq!((sent[0]["target_agent"].as_str(), sent[0]["message"].as_str(), sent[0]["source_agent"].as_str()), (Some("agent2"), Some("check the build"), Some("tester")));
    until("the loop's fire was never counted", || fx.cx().loops.lock().unwrap()["loop-1"].fire_count.load(Ordering::Relaxed) == 1).await;
    let listed = call("LoopList", &json!({}), &fx.cx()).await.unwrap();
    assert!(listed.starts_with("1 loop(s) in this session:\n  loop-1  every=30s  to='agent2'  status=DONE (1/1)  age="), "{listed}");
    assert!(listed.ends_with("s  prompt='check the build'"), "{listed}");
}

#[tokio::test]
async fn a_loop_waits_a_whole_interval_before_its_first_fire_and_a_stop_ends_it() {
    let srv = stub_srv("200 OK", json!({}));
    let fx = Fixture::new(&srv);
    let said = call("Loop", &json!({ "prompt": "p" }), &fx.cx()).await.unwrap();
    // No `to` is a loop to the agent itself; no interval is ten minutes; no cap is no limit.
    assert_eq!(
        said,
        "Started loop-1: injecting to 'tester' every 10m. Stop it with Schedule action \"delete\" and id loop-1; Schedule action \"list\" shows them all."
    );
    {
        let reg = fx.cx().loops.lock().unwrap();
        let entry = &reg["loop-1"];
        assert_eq!((entry.prompt.as_str(), entry.target.as_str(), entry.interval_secs, entry.max_iterations), ("p", "tester", 600, None));
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    assert!(injected(&srv).is_empty(), "it fired before its interval");
    assert_eq!(call("LoopStop", &json!({ "loop_id": "loop-1" }), &fx.cx()).await.unwrap(), "stopped loop-1 (fired 0 time(s))");
    assert!(fx.cx().loops.lock().unwrap().is_empty());
    assert_eq!(call("LoopStop", &json!({ "loop_id": "loop-1" }), &fx.cx()).await.unwrap(), "loop-1 was not running (unknown or already stopped)");
}

#[tokio::test]
async fn each_loop_gets_the_next_id_and_an_interval_in_range() {
    let srv = stub_srv("200 OK", json!({}));
    let fx = Fixture::new(&srv);
    // Too short is raised to ten seconds; a bare number is minutes; a cap of zero is no cap.
    let mut said = Vec::new();
    for args in [json!({ "prompt": "a", "interval": "1s" }), json!({ "prompt": "b", "interval": "5" }), json!({ "prompt": "c", "interval": "2h", "max_iterations": 0 })] {
        said.push(call("Loop", &args, &fx.cx()).await.unwrap());
    }
    assert!(said[0].starts_with("Started loop-1: injecting to 'tester' every 10s. "), "{}", said[0]);
    assert!(said[1].starts_with("Started loop-2: injecting to 'tester' every 5m. "), "{}", said[1]);
    assert!(said[2].starts_with("Started loop-3: injecting to 'tester' every 2h. "), "{}", said[2]);
    assert!(!said[2].contains("auto-stops"), "{}", said[2]);
    let listed = call("LoopList", &json!({}), &fx.cx()).await.unwrap();
    assert!(listed.starts_with("3 loop(s) in this session:"), "{listed}");
    for line in ["loop-1  every=10s", "loop-2  every=5m", "loop-3  every=2h"] {
        assert!(listed.contains(line), "{listed}");
    }
    assert_eq!(listed.matches("status=running (0 fired, unlimited)").count(), 3, "{listed}");
}

#[tokio::test]
async fn a_list_shows_the_cap_and_only_the_start_of_a_long_prompt() {
    let srv = stub_srv("200 OK", json!({}));
    let fx = Fixture::new(&srv);
    assert_eq!(call("LoopList", &json!({}), &fx.cx()).await.unwrap(), "No loops running in this session.");
    let long = "x".repeat(80);
    call("Loop", &json!({ "prompt": long, "max_iterations": 3 }), &fx.cx()).await.unwrap();
    let listed = call("LoopList", &json!({}), &fx.cx()).await.unwrap();
    assert!(listed.contains("status=running (0/3)"), "{listed}");
    assert!(listed.ends_with(&format!("prompt='{}'", "x".repeat(60))), "{listed}");
}

#[tokio::test]
async fn a_loop_that_cannot_start_says_why_and_leaves_nothing_running() {
    let srv = stub_srv("200 OK", json!({}));
    let fx = Fixture::new(&srv);
    let no_url = ToolCtx { local_url: "", ..fx.cx() };
    let no_key = ToolCtx { auth_key: "", ..fx.cx() };
    let refusals = [
        (json!({}), fx.cx(), "missing required parameter: prompt"),
        (json!({ "prompt": "" }), fx.cx(), "missing required parameter: prompt"),
        (json!({ "prompt": "p" }), no_url, "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set"),
        (json!({ "prompt": "p" }), no_key, "AGENTMUX_LOCAL_URL and AGENTMUX_AUTH_KEY must be set"),
        (json!({ "prompt": "p", "interval": "soon" }), fx.cx(), "invalid interval 'soon'"),
        (json!({ "prompt": "p", "interval": "0" }), fx.cx(), "interval must be a positive number: '0'"),
        (json!({ "prompt": "p", "interval": "" }), fx.cx(), "interval is empty"),
    ];
    for (args, cx, wanted) in refusals {
        let err = call("Loop", &args, &cx).await.expect_err(&format!("{args} should refuse")).to_string();
        assert!(err.contains(wanted), "{args}: {err:?}, wanted {wanted:?}");
    }
    std::env::remove_var("AGENTMUX_AGENT_ID");
    let err = call("Loop", &json!({ "prompt": "p" }), &fx.cx()).await.unwrap_err().to_string();
    assert_eq!(err, "no loop target: pass `to`, or ensure AGENTMUX_AGENT_ID is set for a self-loop");
    // With a target it needs no agent id of its own.
    assert!(call("Loop", &json!({ "prompt": "p", "to": "agent2" }), &fx.cx()).await.unwrap().starts_with("Started loop-1: injecting to 'agent2'"));
    assert_eq!(fx.cx().loops.lock().unwrap().len(), 1);
    // The refused ones took no id.
    assert!(fx.cx().loops.lock().unwrap().contains_key("loop-1"));
}

#[tokio::test]
async fn a_stop_without_an_id_is_refused() {
    let srv = stub_srv("200 OK", json!({}));
    let fx = Fixture::new(&srv);
    for args in [json!({}), json!({ "loop_id": "" })] {
        let err = call("LoopStop", &args, &fx.cx()).await.unwrap_err().to_string();
        assert!(err.contains("missing required parameter: loop_id"), "{err}");
    }
}

#[tokio::test]
async fn a_name_that_is_not_a_loop_tool_is_not_in_the_family() {
    let srv = stub_srv("200 OK", json!({}));
    let fx = Fixture::new(&srv);
    assert!(call("LoopNope", &json!({}), &fx.cx()).await.is_err());
}
