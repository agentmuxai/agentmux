// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The rule of `docs/specs/SPEC_LAN_KEY_CONTINUITY_2026_10_29.md`: a changed key
//! moves a pin only with a certificate from an install this machine trusts.

use std::sync::Arc;

use agentmux_common::jekt_sign::{generate_lan_keypair, sign_lan_jekt, WanKeyRecord};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};

use super::*;
use crate::backend::reactive::InjectionRequest;
use crate::backend::storage::wan_identity::WanIdentityStore;
use crate::server::reactive::lan_refusals;
use crate::server::tests::test_state;

fn now() -> i64 {
    agentmux_common::time::now_secs()
}

/// A WAN identity with an install of its own, as a host has.
fn host(label: &str) -> (tempfile::TempDir, Arc<WanIdentityStore>) {
    let dir = tempfile::tempdir().unwrap();
    let wan = Arc::new(WanIdentityStore::open(&dir.path().join("wan.db")).unwrap());
    wan.instance_ensure(label).unwrap();
    (dir, wan)
}

/// The certificate `wan`'s install gives the key `public`, as an agent's host would answer with.
fn cert(wan: &WanIdentityStore, agent: &str, public: &[u8; 32], issued_at: i64) -> WanKeyRecord {
    let instance = wan.instance_ensure("starpower").unwrap();
    WanKeyRecord::certify(&instance.private_key, agent, CERT_CHANNEL, public, &instance.host_hint, issued_at).unwrap()
}

/// This machine pinned `sender`'s install for some agent already (a WAN message came from it).
fn trust(state: &AppState, receiver: &Arc<WanIdentityStore>, sender: &WanIdentityStore) {
    state.mstore.set_wan_identity(receiver.clone());
    let instance = sender.instance_ensure("starpower").unwrap();
    receiver.agent_instance_pin_get_or_set("clare", "stable", &instance.instance_id, &instance.host_hint, now()).unwrap();
}

fn pin(state: &AppState, agent: &str, public: &[u8; 32]) {
    assert_eq!(state.mstore.lan_peer_pubkey_pin_get_or_set(agent, &BASE64.encode(public)).unwrap(), BASE64.encode(public));
}

fn pinned(state: &AppState, agent: &str) -> String {
    state.mstore.lan_peer_pubkey_pin_load(agent).unwrap().unwrap()
}

fn audited_for(agent: &str) -> Vec<serde_json::Value> {
    AUDITED.lock().unwrap().iter().filter(|e| e["agent"] == agent && e["action"] == "continuity").cloned().collect()
}

#[test]
fn a_certificate_is_checked_for_its_chain_agent_channel_key_age_and_the_pin() {
    let (_dir, wan) = host("starpower");
    let (key, _) = generate_lan_keypair([1u8; 32]);
    let (other, _) = generate_lan_keypair([2u8; 32]);
    let t = now();
    let good = cert(&wan, "maop", &key, t);
    assert_eq!(check_cert(&good, "Maop", &key, t - 5, t), Ok(()));

    assert_eq!(check_cert(&good, "clare", &key, t - 5, t), Err("the certificate names another agent"));
    assert_eq!(check_cert(&good, "maop", &other, t - 5, t), Err("the certificate is for another key"));
    assert_eq!(check_cert(&good, "maop", &key, t - 5, t + CERT_MAX_AGE_SECS + 1), Err("the certificate is not fresh"));
    assert_eq!(check_cert(&good, "maop", &key, t - 5, t - CERT_MAX_AGE_SECS - 1), Err("the certificate is not fresh"));
    assert_eq!(check_cert(&good, "maop", &key, t, t), Err("the certificate is older than the pin it would replace"));

    let mut broken = good.clone();
    broken.cert_sig = cert(&wan, "maop", &other, t).cert_sig; // another key's signature
    assert_eq!(check_cert(&broken, "maop", &key, t - 5, t), Err("the certificate does not verify"));

    let instance = wan.instance_ensure("starpower").unwrap();
    let wrong_channel = WanKeyRecord::certify(&instance.private_key, "maop", "stable", &key, &instance.host_hint, t).unwrap();
    assert_eq!(check_cert(&wrong_channel, "maop", &key, t - 5, t), Err("the certificate is not for a LAN key"));
}

#[tokio::test]
async fn an_impostor_key_with_no_certificate_is_refused_and_the_pin_stays() {
    let state = test_state();
    let (old, _) = generate_lan_keypair([1u8; 32]);
    let (impostor, _) = generate_lan_keypair([9u8; 32]);
    pin(&state, "maop-i", &old);
    let err = move_if_vouched(&state, "maop-i", &impostor, None, "automatic", false).await.unwrap_err();
    assert!(err.contains("no certificate"), "{err}");
    assert_eq!(pinned(&state, "maop-i"), BASE64.encode(old));
    assert!(audited_for("maop-i").is_empty(), "nothing moved, nothing audited");
    // A peer that keeps showing the key is not looked into again for a minute, and the reason stays the real one.
    assert_eq!(move_if_vouched(&state, "maop-i", &impostor, None, "automatic", false).await.unwrap_err(), err);
}

#[tokio::test]
async fn the_memory_of_unvouched_keys_does_not_grow_past_its_cap() {
    let state = test_state();
    let (old, _) = generate_lan_keypair([1u8; 32]);
    pin(&state, "maop-cap", &old);
    for n in 0..(MAX_UNVOUCHED as u32 + 50) {
        let mut seed = [0u8; 32];
        seed[..4].copy_from_slice(&n.to_le_bytes());
        seed[31] = 7;
        let (fresh, _) = generate_lan_keypair(seed);
        let _ = move_if_vouched(&state, "maop-cap", &fresh, None, "automatic", false).await;
    }
    assert!(unvouched().len() <= MAX_UNVOUCHED, "{}", unvouched().len());
}

#[tokio::test]
async fn a_certificate_by_an_install_nobody_here_trusts_is_refused() {
    let state = test_state();
    let (_dir, theirs) = host("attacker");
    let (_rdir, receiver) = host("narko");
    state.mstore.set_wan_identity(receiver); // knows no install but its own
    let (old, _) = generate_lan_keypair([1u8; 32]);
    let (new, _) = generate_lan_keypair([2u8; 32]);
    pin(&state, "maop-u", &old);
    let forged = cert(&theirs, "maop-u", &new, now() + 5);
    let err = move_if_vouched(&state, "maop-u", &new, Some(&forged), "automatic", false).await.unwrap_err();
    assert!(err.contains("not pinned here"), "{err}");
    assert_eq!(pinned(&state, "maop-u"), BASE64.encode(old));
}

#[tokio::test]
async fn a_rotation_vouched_by_a_pinned_install_moves_the_pin_and_writes_the_audit_line() {
    let state = test_state();
    let (_sdir, sender) = host("starpower");
    let (_rdir, receiver) = host("narko");
    trust(&state, &receiver, &sender);
    let (old, _) = generate_lan_keypair([1u8; 32]);
    let (new, _) = generate_lan_keypair([2u8; 32]);
    pin(&state, "maop-v", &old);
    state.mstore.lan_peer_uid_pin_get_or_set("maop-v", "uid-m", "old-uid-key").unwrap();
    lan_refusals::pin_mismatch("maop-v", &BASE64.encode(new));

    let vouch = cert(&sender, "maop-v", &new, now() + 5);
    let path = move_if_vouched(&state, "maop-v", &new, Some(&vouch), "automatic", false).await.unwrap();

    assert_eq!(path, Path::PinnedInstance);
    assert_eq!(pinned(&state, "maop-v"), BASE64.encode(new));
    assert!(!state.mstore.lan_peer_uid_pin_exists("maop-v").unwrap(), "the UID pins went with the old key");
    assert!(lan_refusals::get("maop-v").is_none(), "no longer refused");
    let lines = audited_for("maop-v");
    assert_eq!(lines.len(), 1, "{lines:?}");
    let line = &lines[0];
    assert_eq!(line["by"], "automatic");
    assert_eq!(line["from"], fingerprint(&BASE64.encode(old)));
    assert_eq!(line["to"], fingerprint(&BASE64.encode(new)));
    assert_eq!(line["vouched_by"]["path"], "pinned-instance");
    assert_eq!(line["vouched_by"]["instance"], vouch.instance_id);
    assert!(!line.to_string().contains(&BASE64.encode(new)), "fingerprints only, never a key");
}

#[tokio::test]
async fn a_copy_of_an_older_certificate_cannot_move_a_pin_back_to_a_retired_key() {
    let state = test_state();
    let (_sdir, sender) = host("starpower");
    let (_rdir, receiver) = host("narko");
    trust(&state, &receiver, &sender);
    let (old, _) = generate_lan_keypair([1u8; 32]);
    let (new, _) = generate_lan_keypair([2u8; 32]);
    pin(&state, "maop-b", &old);
    let old_cert = cert(&sender, "maop-b", &old, now() - 100); // issued while the old key was current
    let new_cert = cert(&sender, "maop-b", &new, now() + 5);
    move_if_vouched(&state, "maop-b", &new, Some(&new_cert), "automatic", false).await.unwrap();

    let err = move_if_vouched(&state, "maop-b", &old, Some(&old_cert), "automatic", true).await.unwrap_err();
    assert!(err.contains("older than the pin"), "{err}");
    assert_eq!(pinned(&state, "maop-b"), BASE64.encode(new));
}

#[tokio::test]
async fn with_the_hosts_clock_ahead_an_older_certificate_still_cannot_move_the_pin_back() {
    let state = test_state();
    let (_sdir, sender) = host("starpower");
    let (_rdir, receiver) = host("narko");
    trust(&state, &receiver, &sender);
    let (old, _) = generate_lan_keypair([1u8; 32]);
    let (new, _) = generate_lan_keypair([2u8; 32]);
    pin(&state, "maop-c", &old);
    // The host's clock runs 100 s ahead: both certificates are inside the freshness window,
    // and both are newer than the receiver's own time of pinning.
    let old_cert = cert(&sender, "maop-c", &old, now() + 100);
    let new_cert = cert(&sender, "maop-c", &new, now() + 101);
    move_if_vouched(&state, "maop-c", &new, Some(&new_cert), "automatic", false).await.unwrap();
    let (_, since) = state.mstore.lan_peer_pubkey_pin_since("maop-c").unwrap().unwrap();
    assert_eq!(since, new_cert.issued_at, "the pin is kept from the time of the certificate that moved it");

    let err = move_if_vouched(&state, "maop-c", &old, Some(&old_cert), "automatic", true).await.unwrap_err();
    assert!(err.contains("older than the pin"), "{err}");
    assert_eq!(pinned(&state, "maop-c"), BASE64.encode(new));
}

#[tokio::test]
async fn a_pin_moves_only_if_it_is_still_the_one_that_was_checked() {
    let state = test_state();
    let (a, _) = generate_lan_keypair([1u8; 32]);
    let (b, _) = generate_lan_keypair([2u8; 32]);
    let (c, _) = generate_lan_keypair([3u8; 32]);
    pin(&state, "maop-d", &a);
    let (key, since) = state.mstore.lan_peer_pubkey_pin_since("maop-d").unwrap().unwrap();

    // A newer move (or the user's own change) lands first ...
    assert!(state.mstore.lan_peer_pubkey_pin_move("maop-d", (&key, since), &BASE64.encode(b), since + 50).unwrap());
    // ... so a check made against the pin as it was must not write over it.
    assert!(!state.mstore.lan_peer_pubkey_pin_move("maop-d", (&key, since), &BASE64.encode(c), since + 60).unwrap());
    assert_eq!(pinned(&state, "maop-d"), BASE64.encode(b));
    // An unpinned name is not moved either.
    assert!(!state.mstore.lan_peer_pubkey_pin_move("nobody-d", (&key, since), &BASE64.encode(c), since).unwrap());
    assert!(state.mstore.lan_peer_pubkey_pin_since("nobody-d").unwrap().is_none());
}

#[tokio::test]
async fn an_install_certifies_the_key_it_hosts_and_a_receiver_accepts_the_certificate() {
    let state = test_state();
    let (_dir, wan) = host("starpower");
    state.mstore.set_wan_identity(wan);
    let (public, _) = generate_lan_keypair([3u8; 32]);
    let issued = issue_cert(&state, "Maop-I", &BASE64.encode(public)).expect("an install with a WAN identity vouches");
    assert_eq!(issued.agent_id, "maop-i");
    assert_eq!(issued.channel, CERT_CHANNEL);
    assert_eq!(check_cert(&issued, "maop-i", &public, now() - 5, now()), Ok(()));
    assert!(issue_cert(&state, "maop-i", "not a key").is_none());
}

async fn verified(state: &AppState, agent: &str, signer: &[u8; 32]) -> InjectionRequest {
    let ts = now();
    let mut req = InjectionRequest {
        target_agent: "lark".into(),
        message: "hello".into(),
        source_agent: Some(agent.into()),
        delivery_tier: Some("lan".into()),
        request_id: Some("req-continuity".into()),
        ts_secs: Some(ts),
        ..Default::default()
    };
    req.lan_sig = sign_lan_jekt(signer, "req-continuity", agent, "lark", ts, "hello");
    super::super::verify_lan_signature(state, &mut req).await;
    req
}

#[tokio::test]
async fn a_jekt_from_a_vouched_new_key_verifies_and_one_from_an_impostor_does_not() {
    let state = test_state();
    let (_sdir, sender) = host("starpower");
    let (_rdir, receiver) = host("narko");
    trust(&state, &receiver, &sender);
    let (old_public, _) = generate_lan_keypair([1u8; 32]);
    let (new_public, new_private) = generate_lan_keypair([2u8; 32]);
    let (fake_public, fake_private) = generate_lan_keypair([9u8; 32]);
    pin(&state, "maop-e", &old_public);

    // An impostor on the LAN shows its own key, with a certificate of its own making: refused, and the reason is kept.
    let (_edir, attacker) = host("attacker");
    let forged = cert(&attacker, "maop-e", &fake_public, now() + 5);
    state.lan_discovery.seed_lan_pubkey_with_cert_for_test("maop-e", fake_public.to_vec(), Some(forged));
    let req = verified(&state, "maop-e", &fake_private).await;
    assert_eq!(req.lan_verified, Some(false));
    assert_eq!(pinned(&state, "maop-e"), BASE64.encode(old_public));
    let refusal = lan_refusals::get("maop-e").unwrap();
    assert!(refusal.why.unwrap().contains("not pinned here"), "the screen can say why it stayed refused");

    // The real host shows its new key with its own certificate: the pin moves and the jekt verifies.
    let vouch = cert(&sender, "maop-e", &new_public, now() + 5);
    state.lan_discovery.seed_lan_pubkey_with_cert_for_test("maop-e", new_public.to_vec(), Some(vouch));
    let req = verified(&state, "maop-e", &new_private).await;
    assert_eq!(req.lan_verified, Some(true));
    assert_eq!(pinned(&state, "maop-e"), BASE64.encode(new_public));
}

#[tokio::test]
async fn a_host_that_sends_no_certificate_is_refused_as_before() {
    let state = test_state();
    let (old_public, _) = generate_lan_keypair([1u8; 32]);
    let (new_public, new_private) = generate_lan_keypair([2u8; 32]);
    pin(&state, "maop-n", &old_public);
    state.lan_discovery.seed_lan_pubkey_for_test("maop-n", new_public.to_vec(), None);
    let req = verified(&state, "maop-n", &new_private).await;
    assert_eq!(req.lan_verified, Some(false));
    assert_eq!(pinned(&state, "maop-n"), BASE64.encode(old_public));
}

#[test]
fn the_audit_line_is_appended_to_the_log_with_the_time() {
    let dir = tempfile::tempdir().unwrap();
    write_audit(dir.path(), &json!({ "action": "continuity", "agent": "maop" }));
    write_audit(dir.path(), &json!({ "action": "continuity", "agent": "clare" }));
    let log = std::fs::read_to_string(dir.path().join("pin-changes.log")).unwrap();
    let lines: Vec<serde_json::Value> = log.lines().map(|l| serde_json::from_str(l).unwrap()).collect();
    assert_eq!(lines.len(), 2);
    assert_eq!(lines[0]["change"]["agent"], "maop");
    assert!(lines[0]["at"].as_i64().unwrap() > 0);
}
