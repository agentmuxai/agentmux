// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The transport's rules (spec §10): who is let in, who is refused and why, and that a
//! repeated request is answered once.

use agentmux_common::jekt_sign::{generate_lan_keypair, WanKeyRecord};
use agentmux_common::remote_op::{lan_audience, OpStatus, RemoteOp, RemoteOpResult, Requester, SignedRemoteOp, REMOTE_OP_VERSION};
use serde_json::json;

use super::receive::receive;
use super::send::resolve_peer;
use crate::backend::lan_discovery::LanInstance;
use crate::backend::storage::wan_identity::WanIdentityStore;
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
    receive(state, via, None, signed, NOW, 0).await
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

/// The certificate an install gives `slug`'s LAN key `public`, as the agent's host answers a lookup.
fn certificate(slug: &str, public: &[u8; 32], issued_at: i64) -> (tempfile::TempDir, WanKeyRecord) {
    let dir = tempfile::tempdir().unwrap();
    let wan = WanIdentityStore::open(&dir.path().join("wan.db")).unwrap();
    let instance = wan.instance_ensure("starpower").unwrap();
    let cert = WanKeyRecord::certify(&instance.private_key, slug, "lan", public, &instance.host_hint, issued_at).unwrap();
    (dir, cert)
}

/// This machine trusts the install `instance_id`: pinned here for some agent already.
fn trust_install(state: &AppState, instance_id: &str) {
    let dir = Box::leak(Box::new(tempfile::tempdir().unwrap()));
    let wan = std::sync::Arc::new(WanIdentityStore::open(&dir.path().join("wan.db")).unwrap());
    wan.instance_ensure("receiver").unwrap();
    state.mstore.set_wan_identity(wan.clone());
    wan.agent_instance_pin_get_or_set("pinned-elsewhere", "stable", instance_id, "host", NOW).unwrap();
}

#[tokio::test]
async fn an_operation_that_changes_state_needs_a_certificate_backed_caller() {
    let (public, private) = keys(20);
    let (state, audience) = state_with_peer("rop-s", &public);
    // The first-seen pin alone is enough to read, and not to change anything.
    let read = run(&state, PEER, request("rop-s", "n-s1", "app.version", &audience, NOW, &private)).await;
    assert_eq!(read.status, OpStatus::Done);
    let write = run(&state, PEER, request("rop-s", "n-s2", "test.change", &audience, NOW, &private)).await;
    assert_eq!(write.refused_reason.as_deref(), Some("needs_certificate"), "{write:?}");
    // A refused request leaves its nonce free: the retry after the certificate arrives is a new request.
    let (_dir, cert) = certificate("rop-s", &public, NOW);
    let instance = cert.instance_id.clone();
    state.lan_discovery.seed_lan_cert_for_test("rop-s", cert);
    trust_install(&state, &instance);
    let ok = run(&state, PEER, request("rop-s", "n-s2", "test.change", &audience, NOW, &private)).await;
    assert_eq!(ok.status, OpStatus::Done, "{ok:?}");
    assert!(ok.result["changed_by"].as_str().unwrap().starts_with("rop-s@"), "{ok:?}");
    assert!(!instance.is_empty());
}

#[tokio::test]
async fn account_data_needs_an_approved_install_like_a_change_and_a_first_seen_peer_is_refused() {
    let (public, private) = keys(23);
    let (state, audience) = state_with_peer("rop-i", &public);
    let listing = run(&state, PEER, request("rop-i", "n-i1", "identity.accounts", &audience, NOW, &private)).await;
    assert_eq!(listing.refused_reason.as_deref(), Some("needs_certificate"), "no email goes to a pin alone: {listing:?}");
    let switch = run(&state, PEER, request("rop-i", "n-i2", "identity.switch", &audience, NOW, &private)).await;
    assert_eq!(switch.refused_reason.as_deref(), Some("needs_certificate"), "{switch:?}");

    // A certificate from an install this machine has not approved backs nothing either.
    let (_dir, cert) = certificate("rop-i", &public, NOW);
    let instance = cert.instance_id.clone();
    state.lan_discovery.seed_lan_cert_for_test("rop-i", cert);
    let untrusted = run(&state, PEER, request("rop-i", "n-i3", "identity.accounts", &audience, NOW, &private)).await;
    assert_eq!(untrusted.refused_reason.as_deref(), Some("install_not_trusted"), "{untrusted:?}");

    trust_install(&state, &instance);
    let ok = run(&state, PEER, request("rop-i", "n-i4", "identity.accounts", &audience, NOW, &private)).await;
    assert_eq!(ok.status, OpStatus::Done, "{ok:?}");
    assert!(ok.result["accounts"].is_array(), "{ok:?}");
}

#[tokio::test]
async fn a_certificate_for_another_key_name_or_time_does_not_back_a_caller() {
    let (public, private) = keys(21);
    let (other_public, _) = keys(22);
    let (state, audience) = state_with_peer("rop-k", &public);
    let cases: Vec<(&str, &str, [u8; 32], i64)> = vec![
        ("another key", "rop-k", other_public, NOW),
        ("another agent", "someone-else", public, NOW),
        ("an old certificate", "rop-k", public, NOW - 601),
    ];
    for (i, (what, name, key, issued)) in cases.into_iter().enumerate() {
        let (_dir, cert) = certificate(name, &key, issued);
        state.lan_discovery.seed_lan_cert_for_test("rop-k", cert);
        let nonce = format!("n-k{i}");
        let result = run(&state, PEER, request("rop-k", &nonce, "test.change", &audience, NOW, &private)).await;
        assert_eq!(result.refused_reason.as_deref(), Some("needs_certificate"), "{what}: {result:?}");
    }
}

#[tokio::test]
async fn a_claimed_install_must_be_the_one_the_certificate_names() {
    let (public, private) = keys(23);
    let (state, audience) = state_with_peer("rop-i", &public);
    let (_dir, cert) = certificate("rop-i", &public, NOW);
    let real = cert.instance_id.clone();
    state.lan_discovery.seed_lan_cert_for_test("rop-i", cert);
    let mut lying = RemoteOp {
        v: REMOTE_OP_VERSION,
        op: "test.change".into(),
        params: json!({}),
        requester: Requester { agent_uid: "uid".into(), slug: "rop-i".into(), instance_id: "some-other-install".into() },
        audience: audience.clone(),
        ts: NOW,
        nonce: "n-i1".into(),
        reason: String::new(),
    };
    let refused = run(&state, PEER, lying.clone().sign(&private).unwrap()).await;
    assert_eq!(refused.refused_reason.as_deref(), Some("install_mismatch"));
    trust_install(&state, &real);
    lying.requester.instance_id = real;
    lying.nonce = "n-i2".into();
    assert_eq!(run(&state, PEER, lying.sign(&private).unwrap()).await.status, OpStatus::Done);
}

#[tokio::test]
async fn a_request_signed_before_this_srv_started_is_refused() {
    let (public, private) = keys(24);
    let (state, audience) = state_with_peer("rop-q", &public);
    let signed = request("rop-q", "n-q", "app.version", &audience, NOW, &private);
    let before = receive(&state, PEER, None, signed.clone(), NOW + 5, NOW + 1).await;
    assert_eq!(before.refused_reason.as_deref(), Some("signed_before_restart"));
    let after = receive(&state, PEER, None, signed, NOW + 5, NOW).await;
    assert_eq!(after.status, OpStatus::Done, "a request made after the start is taken");
}

#[tokio::test]
async fn a_refusal_before_the_signature_names_no_claimed_agent_and_keeps_no_claimed_text() {
    let (_, private) = keys(30);
    let state = test_state();
    let audience = lan_audience(&state.lan_key);
    let mut signed = request("rop-forged-name", "n-forged", "app.version", &audience, NOW, &private);
    signed.op.reason = "FORGED-REASON-TEXT".into();
    let result = run(&state, PEER, signed).await;
    assert_eq!(result.refused_reason.as_deref(), Some("unknown_sender"));
    let rows = state.reactive_handler.get_audit_log(500);
    assert!(rows.iter().any(|r| r.source_agent.as_deref().is_some_and(|s| s.starts_with("unproven peer"))), "an unproven refusal is still recorded");
    let dump = format!("{rows:?}");
    assert!(!dump.contains("rop-forged-name") && !dump.contains("FORGED-REASON-TEXT"), "{dump}");
}

#[test]
fn the_machine_is_named_by_the_address_it_called_from() {
    let mut narko = peer("Narko", "main");
    narko.address = "192.0.2.77".into();
    let from = Some("192.0.2.77:51000".parse().unwrap());
    assert_eq!(super::receive::machine_in(&[peer("charlie", "main"), narko], from), "Narko");
    assert_eq!(super::receive::machine_in(&[peer("charlie", "main")], from), "unknown machine");
    assert_eq!(super::receive::machine_in(&[], None), "unknown machine");
}

/// A request with `params`, signed by `private`.
fn request_params(slug: &str, nonce: &str, op: &str, params: serde_json::Value, audience: &str, private: &[u8; 32]) -> SignedRemoteOp {
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
    // Filing a task changes the board, and the board's text is the workspace's own: the first-seen pin
    // alone is enough for neither, a certificate from an approved install is.
    let refused = run(&state, PEER, request_params("rop-task", "n-task-0", "tasks.enqueue", params.clone(), &audience, &private)).await;
    assert_eq!(refused.refused_reason.as_deref(), Some("needs_certificate"), "{refused:?}");
    let before = run(&state, PEER, request_params("rop-task", "n-task-l0", "tasks.list", json!({}), &audience, &private)).await;
    assert_eq!(before.refused_reason.as_deref(), Some("needs_certificate"), "{before:?}");
    let (_dir, cert) = certificate("rop-task", &public, NOW);
    trust_install(&state, &cert.instance_id);
    state.lan_discovery.seed_lan_cert_for_test("rop-task", cert);
    let result = run(&state, PEER, request_params("rop-task", "n-task", "tasks.enqueue", params, &audience, &private)).await;
    assert_eq!(result.status, OpStatus::Done, "{result:?}");
    let id = result.result["id"].as_str().unwrap().to_string();
    let task = state.work_store.work_board_get(&id).unwrap().unwrap();
    assert!(task.card.requested_by.starts_with("rop-task@"), "{}", task.card.requested_by);
    assert_eq!(task.card.requested_by_kind, "agent", "never the operator");
    assert_eq!(task.item.created_by, task.card.requested_by);
    assert!(!task.item.payload.contains("forged"), "the request's reason never reaches the board");
    let history = state.work_store.work_events(&id, 10).unwrap();
    assert!(history.iter().all(|e| e.actor == task.card.requested_by && e.actor_kind == "agent"), "{history:?}");

    let listed = run(&state, PEER, request_params("rop-task", "n-list", "tasks.list", json!({ "state": "open" }), &audience, &private)).await;
    assert_eq!(listed.status, OpStatus::Done, "{listed:?}");
    assert_eq!(listed.result["items"][0]["title"], "Verify the build");
    assert_eq!(listed.result["items"][0]["num"], result.result["num"]);
    assert_eq!(state.work_store.work_queue_list("", 10).unwrap().len(), 1, "the refused request filed nothing");
}

#[tokio::test]
async fn a_remote_task_that_cannot_be_filed_is_refused_with_a_code_and_nothing_is_on_the_board() {
    let (public, private) = keys(16);
    let (state, audience) = state_with_peer("rop-bad", &public);
    let (_dir, cert) = certificate("rop-bad", &public, NOW);
    trust_install(&state, &cert.instance_id);
    state.lan_discovery.seed_lan_cert_for_test("rop-bad", cert);
    for (nonce, params, code) in [
        ("n-bad1", json!({ "title": "t", "payload": "p", "target_agent": "nobody-here" }), "no_such_agent"),
        ("n-bad2", json!({ "title": "t", "payload": "p", "target_group": "g" }), "bad_params"),
        ("n-bad3", json!({ "title": "t" }), "bad_params"),
    ] {
        let result = run(&state, PEER, request_params("rop-bad", nonce, "tasks.enqueue", params, &audience, &private)).await;
        assert_eq!(result.status, OpStatus::Refused, "{nonce}: {result:?}");
        assert_eq!(result.refused_reason.as_deref(), Some(code), "{nonce}");
    }
    let listed = run(&state, PEER, request_params("rop-bad", "n-bad-list", "tasks.list", json!({}), &audience, &private)).await;
    assert!(listed.result["items"].as_array().unwrap().is_empty());
}

#[tokio::test]
async fn a_certificate_from_an_install_this_machine_does_not_trust_backs_nothing() {
    let (public, private) = keys(25);
    let (state, audience) = state_with_peer("rop-u", &public);
    // A LAN host that won the first-use pin issues a certificate for its own key from its own install.
    let (_dir, cert) = certificate("rop-u", &public, NOW);
    let install = cert.instance_id.clone();
    state.lan_discovery.seed_lan_cert_for_test("rop-u", cert);
    let refused = run(&state, PEER, request("rop-u", "n-u1", "test.change", &audience, NOW, &private)).await;
    assert_eq!(refused.refused_reason.as_deref(), Some("install_not_trusted"), "{refused:?}");
    // It can still read, on the pin alone.
    assert_eq!(run(&state, PEER, request("rop-u", "n-u2", "app.version", &audience, NOW, &private)).await.status, OpStatus::Done);
    // Once this machine trusts that install, the same request goes through.
    trust_install(&state, &install);
    let ok = run(&state, PEER, request("rop-u", "n-u1", "test.change", &audience, NOW, &private)).await;
    assert_eq!(ok.status, OpStatus::Done, "{ok:?}");
}

#[tokio::test]
async fn a_read_gives_no_path_url_email_or_error_text() {
    let (public, private) = keys(31);
    let (state, audience) = state_with_peer("rop-leak", &public);
    for (i, op) in ["app.version", "app.health"].into_iter().enumerate() {
        let result = run(&state, PEER, request("rop-leak", &format!("n-leak{i}"), op, &audience, NOW, &private)).await;
        assert_eq!(result.status, OpStatus::Done, "{result:?}");
        let text = result.result.to_string();
        for banned in ["folder", "installer", "account_email", "last_error", "https://", "http://", "asset", "\\\\", ":/"] {
            assert!(!text.contains(banned), "{op} leaks {banned}: {text}");
        }
    }
}

/// A request `slug` signs for `op` with `params`, up to `max_params` bytes of them.
fn request_with(slug: &str, nonce: &str, op: &str, params: serde_json::Value, audience: &str, private: &[u8; 32], max_params: usize) -> SignedRemoteOp {
    RemoteOp {
        v: REMOTE_OP_VERSION,
        op: op.into(),
        params,
        requester: Requester { agent_uid: format!("uid-{slug}"), slug: slug.into(), instance_id: String::new() },
        audience: audience.into(),
        ts: NOW,
        nonce: nonce.into(),
        reason: "checking".into(),
    }
    .sign_with(private, max_params)
    .unwrap()
}

#[tokio::test]
async fn an_operation_that_declares_a_larger_limit_takes_a_megabyte_and_the_others_still_do_not() {
    let (public, private) = keys(20);
    let (state, audience) = state_with_peer("rop-big", &public);
    let megabyte = json!({ "text": "y".repeat(1_000_000) });

    let big = request_with("rop-big", "n-big", "test.big", megabyte.clone(), &audience, &private, 1_200_000);
    let result = run(&state, PEER, big).await;
    assert_eq!(result.status, OpStatus::Done, "{result:?}");
    assert!(result.result["bytes"].as_u64().unwrap() > 1_000_000);

    // The same params to an operation with the default limit are refused before anything else:
    // the signature is made with the large limit here, as a sender that ignored the op's own would.
    let small = request_with("rop-big", "n-small", "app.version", megabyte.clone(), &audience, &private, 1_200_000);
    assert_eq!(run(&state, PEER, small).await.refused_reason.as_deref(), Some("too_long"));

    // And the large limit is a limit: past it, the same operation refuses too.
    let too_big = json!({ "text": "y".repeat(1_300_000) });
    assert!(RemoteOp {
        v: REMOTE_OP_VERSION,
        op: "test.big".into(),
        params: too_big,
        requester: Requester { agent_uid: "uid-x".into(), slug: "rop-big".into(), instance_id: String::new() },
        audience: audience.clone(),
        ts: NOW,
        nonce: "n-too".into(),
        reason: String::new(),
    }
    .sign_with(&private, super::max_params_of("test.big"))
    .is_none());
}

#[test]
fn only_the_global_memory_put_declares_more_than_the_default() {
    let default = agentmux_common::remote_op::MAX_PARAMS_BYTES;
    assert_eq!(super::max_params_of("memory.global.put"), super::memory::MAX_PUT_PARAMS);
    for op in ["app.version", "app.health", "memory.global.list", "memory.global.remove", "no.such.op"] {
        assert_eq!(super::max_params_of(op), default, "{op}");
    }
}

#[tokio::test]
async fn app_logs_tail_may_hold_personal_data_so_it_needs_an_approved_install_and_refuses_a_log_it_does_not_have() {
    use agentmux_common::remote_op::MAX_PARAMS_BYTES;
    let (public, private) = keys(40);
    let (state, audience) = state_with_peer("rop-log", &public);
    let ask = |nonce: &str| request_with("rop-log", nonce, "app.logs.tail", json!({ "which": "cef" }), &audience, &private, MAX_PARAMS_BYTES);
    // A log line can hold an email or a path: the first-seen pin alone is not enough to read it.
    let refused = run(&state, PEER, ask("n-log1")).await;
    assert_eq!(refused.refused_reason.as_deref(), Some("needs_certificate"), "{refused:?}");
    let (_dir, cert) = certificate("rop-log", &public, NOW);
    let instance = cert.instance_id.clone();
    state.lan_discovery.seed_lan_cert_for_test("rop-log", cert);
    trust_install(&state, &instance);
    let bad = run(&state, PEER, ask("n-log2")).await;
    assert_eq!(bad.status, OpStatus::Failed, "{bad:?}");
    assert!(bad.refused_reason.is_none(), "not a transport refusal: {bad:?}");
    assert!(bad.result["error"].as_str().unwrap_or_default().contains("srv"), "{bad:?}");
}

#[tokio::test]
async fn app_history_reclaim_changes_state_so_it_needs_a_certificate_backed_caller() {
    use agentmux_common::remote_op::MAX_PARAMS_BYTES;
    let (public, private) = keys(41);
    let (state, audience) = state_with_peer("rop-rc", &public);
    let ask = |nonce: &str, action: &str| request_with("rop-rc", nonce, "app.history_reclaim", json!({ "action": action }), &audience, &private, MAX_PARAMS_BYTES);
    let refused = run(&state, PEER, ask("n-rc1", "request")).await;
    assert_eq!(refused.refused_reason.as_deref(), Some("needs_certificate"), "{refused:?}");
    let (_dir, cert) = certificate("rop-rc", &public, NOW);
    let instance = cert.instance_id.clone();
    state.lan_discovery.seed_lan_cert_for_test("rop-rc", cert);
    trust_install(&state, &instance);
    let bad = run(&state, PEER, ask("n-rc2", "wipe")).await;
    assert_eq!(bad.status, OpStatus::Failed, "{bad:?}");
    assert!(bad.result["error"].as_str().unwrap_or_default().contains("request"), "{bad:?}");
}
