// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// `deliver`'s forwarding tiers, end to end: the target is not on this instance, so the
// answer comes from the same-host registry, the shared registry, or nowhere.

use super::forward_inject_tests::stub_peer;
use super::*;
use crate::server::tests::test_state;

const TAKEN: &str = r#"{"success":true,"request_id":"from-the-peer"}"#;
const REFUSED: &str = r#"{"success":false,"error":"agent not found"}"#;

fn lock() -> std::sync::MutexGuard<'static, ()> {
    crate::test_support::ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn request(target: &str) -> InjectionRequest {
    InjectionRequest {
        target_agent: target.to_string(),
        message: "hello".to_string(),
        source_agent: None,
        request_id: Some("deliver-tiers".to_string()),
        ..Default::default()
    }
}

fn unknown_agent() -> String {
    format!("nobody-{}", &uuid::Uuid::new_v4().to_string()[..8])
}

async fn deliver_to(target: &str, hops: u8) -> serde_json::Value {
    let mut req = request(target);
    req.forward_hops = hops;
    deliver(&test_state(), crate::server::ReactiveAuthVia::FullAuthKey, "", req).await
}

#[tokio::test]
async fn an_agent_nobody_knows_gets_the_local_error() {
    let _guard = lock();
    let home = tempfile::TempDir::new().unwrap();
    std::env::set_var("AGENTMUX_HOME_OVERRIDE", home.path());
    let out = deliver_to(&unknown_agent(), 0).await;
    std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
    assert_eq!(out["success"], false, "{out}");
    assert!(out["error"].as_str().unwrap().starts_with("agent not found"), "{out}");
}

#[tokio::test]
async fn a_same_host_sidecar_that_runs_the_agent_takes_it() {
    let _guard = lock();
    let home = tempfile::TempDir::new().unwrap();
    std::env::set_var("AGENTMUX_HOME_OVERRIDE", home.path());
    let (url, _peer) = stub_peer(StatusCode::OK, TAKEN).await;
    let target = unknown_agent();
    agent_registry::write(home.path(), &target, &url, "block-sidecar");
    let out = deliver_to(&target, 0).await;
    std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
    assert_eq!(out["success"], true, "{out}");
    assert_eq!(out["request_id"], "from-the-peer", "{out}");
}

#[tokio::test]
async fn another_channels_shared_entry_takes_it_when_there_is_no_sidecar_entry() {
    let _guard = lock();
    let home = tempfile::TempDir::new().unwrap();
    std::env::set_var("AGENTMUX_HOME_OVERRIDE", home.path());
    let (url, _peer) = stub_peer(StatusCode::OK, TAKEN).await;
    let target = unknown_agent();
    let shared = home.path().join("shared").join("agents").join("reactive");
    agent_registry::write_shared(&shared, &target, &url, "block-shared", "dev-a");
    let out = deliver_to(&target, 0).await;
    std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
    assert_eq!(out["success"], true, "{out}");
    assert_eq!(out["request_id"], "from-the-peer", "{out}");
}

#[tokio::test]
async fn a_peer_that_refuses_a_live_entry_leaves_it_and_the_local_error_stands() {
    let _guard = lock();
    let home = tempfile::TempDir::new().unwrap();
    std::env::set_var("AGENTMUX_HOME_OVERRIDE", home.path());
    let (url, _peer) = stub_peer(StatusCode::OK, REFUSED).await;
    let target = unknown_agent();
    // Written by this process, so the entry is fresh and its process alive: not evicted.
    agent_registry::write(home.path(), &target, &url, "block-refusing");
    let out = deliver_to(&target, 0).await;
    let kept = agent_registry::lookup(home.path(), &target).is_some();
    std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
    assert_eq!(out["success"], false, "{out}");
    assert!(out["error"].as_str().unwrap().starts_with("agent not found"), "{out}");
    assert!(kept, "a fresh entry whose process is alive is not evicted");
}

#[tokio::test]
async fn a_request_at_the_hop_limit_is_not_forwarded() {
    let _guard = lock();
    let home = tempfile::TempDir::new().unwrap();
    std::env::set_var("AGENTMUX_HOME_OVERRIDE", home.path());
    let (url, _peer) = stub_peer(StatusCode::OK, TAKEN).await;
    let target = unknown_agent();
    agent_registry::write(home.path(), &target, &url, "block-far");
    let out = deliver_to(&target, MAX_FORWARD_HOPS).await;
    std::env::remove_var("AGENTMUX_HOME_OVERRIDE");
    assert_eq!(out["success"], false, "a peer that would have taken it was not asked: {out}");
    assert!(out["error"].as_str().unwrap().starts_with("agent not found"), "{out}");
}
