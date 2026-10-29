// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tests of `jekt_start`: what a message to a stopped agent starts.

use super::*;
use crate::sagas::park_agents_tests::{dispatch_apply, in_tab, seed_agent_block, seed_workspace, set_always_available};
use crate::server::tests::test_state;
use agentmux_common::ipc::Command;

#[test]
fn one_start_per_agent_is_in_flight_and_the_slot_frees_on_drop() {
    let first = StartSlot::claim("uid-slot").expect("free");
    assert!(StartSlot::claim("uid-slot").is_none(), "a second start of the same agent waits for the first");
    assert!(StartSlot::claim("uid-other").is_some(), "another agent is not blocked");
    drop(first);
    assert!(StartSlot::claim("uid-slot").is_some());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_start_for_a_stopped_agent_runs_its_parked_block() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    let block = seed_agent_block(&state, &tabs[0], "ag-msg").await;
    dispatch_apply(&state, Command::ParkBlock { tab_id: tabs[0].clone(), block_id: block.clone() }).await;

    start_once(&state, "ag-msg", "sender-msg").await.unwrap();

    let stored = state.mstore.get::<crate::backend::obj::Block>(&block).unwrap().unwrap();
    assert_eq!(crate::server::app_api::agent_attach::desired_of(&stored).as_deref(), Some("running"));
    assert_eq!(in_tab(&state, &block).await.as_deref(), Some(""), "no pane was made");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_fire_to_a_missing_agent_is_answered_as_before_with_or_without_the_setting() {
    let state = test_state();
    let req = || InjectionRequest { target_agent: "no-such-agent".into(), message: "m".into(), source_agent: Some("cron".into()), ..Default::default() };
    let off = deliver_cron(&state, "creator", req()).await;
    set_always_available(&state, true);
    let on = deliver_cron(&state, "creator", req()).await;
    assert_eq!(off["success"], serde_json::json!(false));
    assert_eq!(on["success"], serde_json::json!(false), "an agent that is defined nowhere is not started");
}

async fn hold_for(state: &AppState, agent: &str, auth_via: crate::server::ReactiveAuthVia) -> Option<serde_json::Value> {
    use crate::server::reactive::{hold_for_absent_target, replay, HoldReason};
    let id = format!("req-{agent}");
    let req = InjectionRequest {
        target_agent: agent.to_string(),
        message: "hi".to_string(),
        source_agent: Some("sender".to_string()),
        request_id: Some(id.clone()),
        delivery_tier: Some("host".to_string()),
        ..Default::default()
    };
    let resp: crate::backend::reactive::types::InjectionResponse = serde_json::from_value(serde_json::json!({
        "success": false, "request_id": id, "error": format!("agent not found: {agent}"), "timestamp": 0,
    }))
    .unwrap();
    hold_for_absent_target(state, auth_via, &req, &resp, HoldReason::NotRunning, &mut replay::reserve(&mut req.clone(), 0)).await
}

async fn desired_of(state: &AppState, block: &str) -> Option<String> {
    let stored = state.mstore.get::<crate::backend::obj::Block>(block).unwrap().unwrap();
    crate::server::app_api::agent_attach::desired_of(&stored)
}

/// A host-tier message to a stopped agent is held and starts it, with the setting on; with it off, and
/// for a LAN caller, nothing starts.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_held_message_starts_the_stopped_agent_only_for_a_host_sender_with_the_setting_on() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    let off = seed_agent_block(&state, &tabs[0], "ag-off").await;
    let on = seed_agent_block(&state, &tabs[0], "ag-on").await;
    let lan = seed_agent_block(&state, &tabs[0], "ag-lan").await;
    for b in [&off, &on, &lan] {
        dispatch_apply(&state, Command::ParkBlock { tab_id: tabs[0].clone(), block_id: b.clone() }).await;
    }

    set_always_available(&state, false);
    let held = hold_for(&state, "ag-off", crate::server::ReactiveAuthVia::FullAuthKey).await.expect("held");
    assert_eq!(held["held"], true, "{held}");
    assert!(held.get("starting").is_none(), "setting off: nothing is starting, so the sender is not told it is: {held}");
    set_always_available(&state, true);
    assert!(hold_for(&state, "ag-lan", crate::server::ReactiveAuthVia::LanKey).await.is_none(), "a LAN caller is never held, so never starts one");
    let held = hold_for(&state, "ag-on", crate::server::ReactiveAuthVia::FullAuthKey).await.expect("held");
    assert_eq!(held["held"], true, "{held}");
    assert_eq!(held["starting"], true, "the sender is told the agent is being started: {held}");
    assert!(held["error"].as_str().unwrap().contains("starting it"), "{held}");

    for _ in 0..100 {
        if desired_of(&state, &on).await.as_deref() == Some("running") {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    assert_eq!(desired_of(&state, &on).await.as_deref(), Some("running"), "the held message started its agent");
    assert!(state.reactive_handler.has_uid_registration("ag-on"), "and it can be delivered to");
    assert_eq!(desired_of(&state, &off).await, None, "setting off: held, not started");
    assert_eq!(desired_of(&state, &lan).await, None, "LAN: not started");
}

#[test]
fn a_sender_whose_signature_failed_does_not_start_an_agent() {
    let req = |sig_verified| InjectionRequest { sig_verified, ..Default::default() };
    assert!(may_start(&req(None)), "a sender with no key on file is as before");
    assert!(may_start(&req(Some(true))));
    assert!(!may_start(&req(Some(false))));
}

/// A fire to an agent defined here that runs nowhere starts it first; one that is already registered is left alone.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_fire_starts_a_stopped_local_agent_and_leaves_a_running_one_alone() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    let stopped = seed_agent_block(&state, &tabs[0], "ag-cron-stopped").await;
    let running = seed_agent_block(&state, &tabs[0], "ag-cron-running").await;
    for b in [&stopped, &running] {
        dispatch_apply(&state, Command::ParkBlock { tab_id: tabs[0].clone(), block_id: b.clone() }).await;
    }
    state.reactive_handler.register_agent_full("ag-cron-running", &running, None, 0, None, Some("ag-cron-running"), "test").unwrap();

    start_stopped_local_target(&state, "ag-cron-stopped", "job:t1").await;
    start_stopped_local_target(&state, "ag-cron-running", "job:t1").await;
    start_stopped_local_target(&state, "no-such-agent", "job:t1").await;

    assert_eq!(desired_of(&state, &stopped).await.as_deref(), Some("running"));
    assert!(state.reactive_handler.has_uid_registration("ag-cron-stopped"));
    assert_eq!(desired_of(&state, &running).await, None, "an agent that is running is not started again");
}

/// One sender gets its starts per minute; the next is refused and nothing starts, and a start that is not
/// needed (the agent already runs) spends none.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_sender_over_its_starts_per_minute_starts_nothing_more() {
    let state = test_state();
    state.config_watcher.update_settings(serde_json::from_value(serde_json::json!({ "agents:start_per_minute": 1 })).unwrap());
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    let first = seed_agent_block(&state, &tabs[0], "ag-rate-1").await;
    let second = seed_agent_block(&state, &tabs[0], "ag-rate-2").await;
    for b in [&first, &second] {
        dispatch_apply(&state, Command::ParkBlock { tab_id: tabs[0].clone(), block_id: b.clone() }).await;
    }

    start_once(&state, "ag-rate-1", "rate-sender").await.unwrap();
    start_once(&state, "ag-rate-1", "rate-sender").await.expect("it already runs: a no-op, not a second start");
    let err = start_once(&state, "ag-rate-2", "rate-sender").await.unwrap_err();

    assert!(err.starts_with("START_RATE_LIMIT:"), "{err}");
    assert_eq!(desired_of(&state, &second).await, None, "the refused start started nothing");
    start_once(&state, "ag-rate-2", "another-sender").await.expect("another sender has its own minute");
}

/// A message counts against the sender's uid when its token attributed one (as `Agents start` does), else
/// the name it sent as.
#[test]
fn a_message_start_counts_against_the_senders_uid_not_its_name() {
    let mut req = InjectionRequest::default();
    req.source_agent = Some("qtest".into());
    assert_eq!(sender_key(&req), "qtest", "no attributed uid: the name it sent as");
    req.audit_source_uid = "uid-qtest".into();
    assert_eq!(sender_key(&req), "uid-qtest");
    req.source_agent = None;
    req.audit_source_uid.clear();
    assert_eq!(sender_key(&req), "unknown");
}
