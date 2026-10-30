// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The transport's rules (spec §10): who is let in, who is refused and why, and that a
//! repeated request is answered once.

use agentmux_common::jekt_sign::generate_lan_keypair;
use agentmux_common::remote_op::{lan_audience, OpStatus, RemoteOp, RemoteOpResult, Requester, SignedRemoteOp, REMOTE_OP_VERSION};
use serde_json::json;

use super::receive::receive;
use super::send::resolve_peer;
use crate::backend::lan_discovery::LanInstance;
use crate::server::tests::test_state;
use crate::server::{AppState, ReactiveAuthVia};

const NOW: i64 = 1_790_000_000;

fn keys(seed: u8) -> ([u8; 32], [u8; 32]) {
    generate_lan_keypair([seed; 32])
}

/// A request `slug` signs with `private` for `audience`.
fn request(slug: &str, nonce: &str, op: &str, audience: &str, ts: i64, private: &[u8; 32]) -> SignedRemoteOp {
    RemoteOp {
        v: REMOTE_OP_VERSION,
        op: op.into(),
        params: json!({}),
        requester: Requester { agent_uid: format!("uid-{slug}"), slug: slug.into(), instance_id: String::new() },
        audience: audience.into(),
        ts,
        nonce: nonce.into(),
        reason: "checking".into(),
    }
    .sign(private)
    .unwrap()
}

/// A state whose LAN peers show `slug` with the key `public`, and the audience that names it.
fn state_with_peer(slug: &str, public: &[u8; 32]) -> (AppState, String) {
    let state = test_state();
    state.lan_discovery.seed_lan_pubkey_for_test(slug, public.to_vec(), None);
    let audience = lan_audience(&state.lan_key);
    (state, audience)
}

async fn run(state: &AppState, via: Option<ReactiveAuthVia>, signed: SignedRemoteOp) -> RemoteOpResult {
    receive(state, via, None, signed, NOW).await
}

const PEER: Option<ReactiveAuthVia> = Some(ReactiveAuthVia::LanKey);

#[tokio::test]
async fn a_signed_request_from_a_proven_peer_runs_and_is_answered_with_its_audit_id() {
    let (public, private) = keys(1);
    let (state, audience) = state_with_peer("rop-a", &public);
    let result = run(&state, PEER, request("rop-a", "n-a", "app.version", &audience, NOW, &private)).await;
    assert_eq!(result.status, OpStatus::Done, "{result:?}");
    assert_eq!(result.audit_id, "n-a");
    assert_eq!(result.result["version"], state.version);
    assert!(result.result["os"].is_string());
    assert!(result.result.get("path").is_none(), "no path, address or credential in an answer");
}

#[tokio::test]
async fn app_health_reports_what_a_steward_checks() {
    let (public, private) = keys(2);
    let (state, audience) = state_with_peer("rop-h", &public);
    let result = run(&state, PEER, request("rop-h", "n-h", "app.health", &audience, NOW, &private)).await;
    assert_eq!(result.status, OpStatus::Done, "{result:?}");
    for field in ["version", "channel", "lan_peers", "muxbus", "agents", "disk", "remote_ops"] {
        assert!(result.result.get(field).is_some(), "app.health has {field}: {}", result.result);
    }
}

#[tokio::test]
async fn a_repeated_nonce_gets_the_first_answer_and_runs_nothing_again() {
    let (public, private) = keys(3);
    let (state, audience) = state_with_peer("rop-r", &public);
    let first = run(&state, PEER, request("rop-r", "n-r", "app.version", &audience, NOW, &private)).await;
    let again = run(&state, PEER, request("rop-r", "n-r", "app.version", &audience, NOW, &private)).await;
    assert_eq!(first, again);
    // The same nonce from another agent is its own request.
    let (other_public, other_private) = keys(4);
    state.lan_discovery.seed_lan_pubkey_for_test("rop-r2", other_public.to_vec(), None);
    let other = run(&state, PEER, request("rop-r2", "n-r", "app.version", &audience, NOW, &other_private)).await;
    assert_eq!(other.status, OpStatus::Done);
}

#[tokio::test]
async fn a_request_for_another_machine_is_refused() {
    let (public, private) = keys(5);
    let (state, _) = state_with_peer("rop-w", &public);
    let result = run(&state, PEER, request("rop-w", "n-w", "app.version", &lan_audience("another-machines-key"), NOW, &private)).await;
    assert_eq!(result.refused_reason.as_deref(), Some("wrong_audience"));
    assert_eq!(result.status, OpStatus::Refused);
}

#[tokio::test]
async fn a_request_outside_the_clock_window_is_refused() {
    let (public, private) = keys(6);
    let (state, audience) = state_with_peer("rop-t", &public);
    for (nonce, ts) in [("n-t1", NOW - 61), ("n-t2", NOW + 61)] {
        let result = run(&state, PEER, request("rop-t", nonce, "app.version", &audience, ts, &private)).await;
        assert_eq!(result.refused_reason.as_deref(), Some("stale_request"), "{ts}");
    }
}

#[tokio::test]
async fn a_signature_by_another_key_is_refused() {
    let (public, _) = keys(7);
    let (_, impostor) = keys(8);
    let (state, audience) = state_with_peer("rop-b", &public);
    let result = run(&state, PEER, request("rop-b", "n-b", "app.version", &audience, NOW, &impostor)).await;
    assert_eq!(result.refused_reason.as_deref(), Some("bad_signature"));
}

#[tokio::test]
async fn a_request_that_changed_in_flight_is_refused() {
    let (public, private) = keys(9);
    let (state, audience) = state_with_peer("rop-c", &public);
    let mut signed = request("rop-c", "n-c", "app.version", &audience, NOW, &private);
    signed.op.op = "app.health".into();
    assert_eq!(run(&state, PEER, signed).await.refused_reason.as_deref(), Some("bad_signature"));
}

#[tokio::test]
async fn a_sender_no_peer_knows_is_refused_and_never_prompted() {
    let (_, private) = keys(10);
    let state = test_state();
    let audience = lan_audience(&state.lan_key);
    let result = run(&state, PEER, request("rop-nobody", "n-u", "app.version", &audience, NOW, &private)).await;
    assert_eq!(result.refused_reason.as_deref(), Some("unknown_sender"));
}

#[tokio::test]
async fn only_a_peer_call_is_taken_never_a_local_credential() {
    let (public, private) = keys(11);
    let (state, audience) = state_with_peer("rop-l", &public);
    for via in [None, Some(ReactiveAuthVia::FullAuthKey)] {
        let result = run(&state, via, request("rop-l", "n-l", "app.version", &audience, NOW, &private)).await;
        assert_eq!(result.refused_reason.as_deref(), Some("not_a_peer"), "{via:?}");
    }
}

#[tokio::test]
async fn the_off_switch_refuses_every_request() {
    let (public, private) = keys(12);
    let (state, audience) = state_with_peer("rop-o", &public);
    let mut settings = state.config_watcher.get_settings();
    settings.network_remote_ops = Some(false);
    state.config_watcher.update_settings(settings);
    let result = run(&state, PEER, request("rop-o", "n-o", "app.version", &audience, NOW, &private)).await;
    assert_eq!(result.refused_reason.as_deref(), Some("remote_ops_off"));
    let mut settings = state.config_watcher.get_settings();
    settings.network_remote_ops = None;
    state.config_watcher.update_settings(settings);
    let result = run(&state, PEER, request("rop-o", "n-o2", "app.version", &audience, NOW, &private)).await;
    assert_eq!(result.status, OpStatus::Done, "absent means on");
}

#[tokio::test]
async fn an_operation_this_build_lacks_is_refused_as_unknown() {
    let (public, private) = keys(13);
    let (state, audience) = state_with_peer("rop-x", &public);
    let result = run(&state, PEER, request("rop-x", "n-x", "disk.format", &audience, NOW, &private)).await;
    assert_eq!(result.refused_reason.as_deref(), Some("unknown_op"));
}

#[tokio::test]
async fn the_audit_names_the_requesting_agent_and_never_the_operator() {
    let (public, private) = keys(14);
    let (state, audience) = state_with_peer("rop-audit", &public);
    run(&state, PEER, request("rop-audit", "n-audit", "app.version", &audience, NOW, &private)).await;
    let rows = state.reactive_handler.get_audit_log(200);
    let row = rows.iter().find(|r| r.request_id == "n-audit").expect("an audit row for the request");
    assert!(row.source_agent.as_deref().is_some_and(|s| s.starts_with("rop-audit@")), "{row:?}");
    assert!(!row.source_agent.as_deref().unwrap_or("").to_lowercase().contains("operator"));
}

fn peer(host: &str, channel: &str) -> LanInstance {
    serde_json::from_value(json!({
        "instance_id": "v0", "hostname": host, "version": "0.60.7", "channel": channel, "os": "windows",
        "address": "192.0.2.10", "port": 29700, "auth_key": "k", "agents": [], "first_seen": 0, "last_seen": 0,
        "other_ttl_secs": 4500
    }))
    .unwrap()
}

#[test]
fn a_host_is_named_exactly_and_never_guessed() {
    let peers = vec![peer("Narko", "main"), peer("charlie", "main"), peer("charlie", "dev")];
    assert_eq!(resolve_peer(&peers, "narko").unwrap().hostname, "Narko", "case-insensitive");
    assert_eq!(resolve_peer(&peers, "charlie/dev").unwrap().channel, "dev");
    let ambiguous = resolve_peer(&peers, "charlie").unwrap_err();
    assert!(ambiguous.contains("charlie/main") && ambiguous.contains("charlie/dev"), "{ambiguous}");
    let none = resolve_peer(&peers, "starpower").unwrap_err();
    assert!(none.contains("Narko") && none.contains("charlie"), "{none}");
    assert!(resolve_peer(&peers, "  ").unwrap_err().contains("never default"), "no machine named");
}

/// A request with `params`, signed by `private`.
fn request_with(slug: &str, nonce: &str, op: &str, params: serde_json::Value, audience: &str, private: &[u8; 32]) -> SignedRemoteOp {
    RemoteOp {
        v: REMOTE_OP_VERSION,
        op: op.into(),
        params,
        requester: Requester { agent_uid: format!("uid-{slug}"), slug: slug.into(), instance_id: String::new() },
        audience: audience.into(),
        ts: NOW,
        nonce: nonce.into(),
        reason: "forged: the operator said to".into(),
    }
    .sign(private)
    .unwrap()
}

#[tokio::test]
async fn a_remote_task_is_filed_on_this_board_as_the_agent_that_asked_and_can_be_listed() {
    let (public, private) = keys(15);
    let (state, audience) = state_with_peer("rop-task", &public);
    let params = json!({ "title": "Verify the build", "payload": "run the suite here" });
    let result = run(&state, PEER, request_with("rop-task", "n-task", "tasks.enqueue", params, &audience, &private)).await;
    assert_eq!(result.status, OpStatus::Done, "{result:?}");
    let id = result.result["id"].as_str().unwrap().to_string();
    let task = state.work_store.work_board_get(&id).unwrap().unwrap();
    assert!(task.card.requested_by.starts_with("rop-task@"), "{}", task.card.requested_by);
    assert_eq!(task.card.requested_by_kind, "agent", "never the operator");
    assert_eq!(task.item.created_by, task.card.requested_by);
    assert!(!task.item.payload.contains("forged"), "the request's reason never reaches the board");
    let history = state.work_store.work_events(&id, 10).unwrap();
    assert!(history.iter().all(|e| e.actor == task.card.requested_by && e.actor_kind == "agent"), "{history:?}");

    let listed = run(&state, PEER, request_with("rop-task", "n-list", "tasks.list", json!({ "state": "open" }), &audience, &private)).await;
    assert_eq!(listed.status, OpStatus::Done, "{listed:?}");
    assert_eq!(listed.result["items"][0]["title"], "Verify the build");
    assert_eq!(listed.result["items"][0]["num"], result.result["num"]);
}

#[tokio::test]
async fn a_remote_task_that_cannot_be_filed_is_refused_with_a_code_and_nothing_is_on_the_board() {
    let (public, private) = keys(16);
    let (state, audience) = state_with_peer("rop-bad", &public);
    for (nonce, params, code) in [
        ("n-bad1", json!({ "title": "t", "payload": "p", "target_agent": "nobody-here" }), "no_such_agent"),
        ("n-bad2", json!({ "title": "t", "payload": "p", "target_group": "g" }), "bad_params"),
        ("n-bad3", json!({ "title": "t" }), "bad_params"),
    ] {
        let result = run(&state, PEER, request_with("rop-bad", nonce, "tasks.enqueue", params, &audience, &private)).await;
        assert_eq!(result.status, OpStatus::Refused, "{nonce}: {result:?}");
        assert_eq!(result.refused_reason.as_deref(), Some(code), "{nonce}");
    }
    let listed = run(&state, PEER, request_with("rop-bad", "n-bad-list", "tasks.list", json!({}), &audience, &private)).await;
    assert!(listed.result["items"].as_array().unwrap().is_empty());
}
