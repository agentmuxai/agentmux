// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Identity M4d-6, LAN half: the v2 signature is checked through
// `verify_lan_signature`, with the peer's answer seeded into the lookup cache.

use super::super::verify_lan_signature;
use crate::backend::reactive::{Handler, InjectionRequest};
use crate::server::tests::test_state;
use agentmux_common::jekt_sign::{generate_lan_keypair, sign_lan_jekt, sign_lan_jekt_v2};

fn now() -> i64 {
    agentmux_common::time::now_secs()
}

/// A LAN jekt from `korp`, v1-signed with `name_private` and, when given,
/// v2-signed as `uid` with `uid_private`.
fn signed(name_private: &[u8; 32], v2: Option<(&str, &[u8; 32])>) -> InjectionRequest {
    let ts = now();
    let mut req = InjectionRequest {
        target_agent: "lark".into(),
        message: "hello".into(),
        source_agent: Some("korp".into()),
        delivery_tier: Some("lan".into()),
        request_id: Some("req-lan-v2".into()),
        ts_secs: Some(ts),
        ..Default::default()
    };
    req.lan_sig = sign_lan_jekt(name_private, "req-lan-v2", "korp", "lark", ts, "hello");
    if let Some((uid, uid_private)) = v2 {
        req.source_uid = Some(uid.into());
        req.lan_sig_v2 = sign_lan_jekt_v2(uid_private, "req-lan-v2", "korp", uid, "lark", ts, "hello");
    }
    req
}

#[tokio::test]
async fn a_valid_v2_from_the_answering_peers_uid_is_recorded_as_claimed_only() {
    let state = test_state();
    let (name_public, name_private) = generate_lan_keypair([1u8; 32]);
    let (uid_public, uid_private) = generate_lan_keypair([2u8; 32]);
    state.lan_discovery.seed_lan_pubkey_for_test("korp", name_public.to_vec(), Some(("uid-k".into(), uid_public.to_vec())));
    let mut req = signed(&name_private, Some(("uid-k", &uid_private)));
    verify_lan_signature(&state, &mut req).await;
    assert_eq!(req.lan_verified, Some(true));
    assert_eq!(req.lan_claimed_uid, "uid-k");
    assert_eq!(req.audit_source_uid, "", "claimed, never attributed");
}

#[tokio::test]
async fn a_uid_the_answering_peer_did_not_give_is_not_claimed_and_fails_the_jekt() {
    let state = test_state();
    let (name_public, name_private) = generate_lan_keypair([1u8; 32]);
    let (uid_public, uid_private) = generate_lan_keypair([2u8; 32]);
    state.lan_discovery.seed_lan_pubkey_for_test("korp", name_public.to_vec(), Some(("uid-k".into(), uid_public.to_vec())));
    let mut req = signed(&name_private, Some(("uid-other", &uid_private)));
    verify_lan_signature(&state, &mut req).await;
    assert_eq!(req.lan_verified, Some(false), "a UID claim the peer contradicts is a failed signature");
    assert_eq!(req.lan_claimed_uid, "");
}

#[tokio::test]
async fn no_uid_key_to_check_against_leaves_v1_standing() {
    // The peer gave no UID key (an older peer): there is nothing to contradict.
    let state = test_state();
    let (name_public, name_private) = generate_lan_keypair([1u8; 32]);
    let (_, uid_private) = generate_lan_keypair([2u8; 32]);
    state.lan_discovery.seed_lan_pubkey_for_test("korp", name_public.to_vec(), None);
    let mut req = signed(&name_private, Some(("uid-k", &uid_private)));
    verify_lan_signature(&state, &mut req).await;
    assert_eq!(req.lan_verified, Some(true));
    assert_eq!(req.lan_claimed_uid, "");
}

#[tokio::test]
async fn a_peer_that_pinned_a_uid_key_and_now_gives_none_fails_the_jekt() {
    let state = test_state();
    let (name_public, name_private) = generate_lan_keypair([1u8; 32]);
    let (uid_public, uid_private) = generate_lan_keypair([2u8; 32]);
    state.lan_discovery.seed_lan_pubkey_for_test("korp", name_public.to_vec(), Some(("uid-k".into(), uid_public.to_vec())));
    let mut first = signed(&name_private, Some(("uid-k", &uid_private)));
    verify_lan_signature(&state, &mut first).await; // pins uid-k's key under korp
    assert_eq!(first.lan_verified, Some(true));

    // The next answer carries no UID key, and the peers have none to give on a fresh ask.
    state.lan_discovery.seed_lan_pubkey_for_test("korp", name_public.to_vec(), None);
    let mut req = signed(&name_private, Some(("uid-k", &uid_private)));
    verify_lan_signature(&state, &mut req).await;
    assert_eq!(req.lan_verified, Some(false), "a pinned name owes its UID key");
    assert_eq!(req.lan_claimed_uid, "");
}

#[tokio::test]
async fn a_pinned_name_without_a_uid_key_still_passes_a_jekt_that_claims_no_uid() {
    let state = test_state();
    let (name_public, name_private) = generate_lan_keypair([1u8; 32]);
    let (uid_public, uid_private) = generate_lan_keypair([2u8; 32]);
    state.lan_discovery.seed_lan_pubkey_for_test("korp", name_public.to_vec(), Some(("uid-k".into(), uid_public.to_vec())));
    verify_lan_signature(&state, &mut signed(&name_private, Some(("uid-k", &uid_private)))).await;
    state.lan_discovery.seed_lan_pubkey_for_test("korp", name_public.to_vec(), None);
    let mut req = signed(&name_private, None);
    verify_lan_signature(&state, &mut req).await;
    assert_eq!(req.lan_verified, Some(true), "no v2 at all is judged on v1");
}

#[tokio::test]
async fn a_jekt_with_no_v2_at_all_stays_verified_on_v1() {
    let state = test_state();
    let (name_public, name_private) = generate_lan_keypair([1u8; 32]);
    let (uid_public, _) = generate_lan_keypair([2u8; 32]);
    state.lan_discovery.seed_lan_pubkey_for_test("korp", name_public.to_vec(), Some(("uid-k".into(), uid_public.to_vec())));
    let mut req = signed(&name_private, None);
    verify_lan_signature(&state, &mut req).await;
    assert_eq!(req.lan_verified, Some(true));
    assert_eq!(req.lan_claimed_uid, "");
}

#[tokio::test]
async fn a_changed_uid_key_under_the_same_name_is_not_claimed_and_fails_the_jekt() {
    let state = test_state();
    let (name_public, name_private) = generate_lan_keypair([1u8; 32]);
    let (first_public, _) = generate_lan_keypair([2u8; 32]);
    let (second_public, second_private) = generate_lan_keypair([3u8; 32]);
    state.lan_discovery.seed_lan_pubkey_for_test("korp", name_public.to_vec(), Some(("uid-k".into(), first_public.to_vec())));
    let mut first = signed(&name_private, None);
    first.source_uid = Some("uid-k".into());
    first.lan_sig_v2 = Some("unchecked until the pin is set".into());
    verify_lan_signature(&state, &mut first).await; // pins uid-k's first key
    state.lan_discovery.seed_lan_pubkey_for_test("korp", name_public.to_vec(), Some(("uid-k".into(), second_public.to_vec())));
    let mut req = signed(&name_private, Some(("uid-k", &second_private)));
    verify_lan_signature(&state, &mut req).await;
    assert_eq!(req.lan_claimed_uid, "", "the pin holds the first key");
    assert_eq!(req.lan_verified, Some(false), "a key other than the pinned one is a failed signature");
}

#[tokio::test]
async fn a_bad_v2_fails_the_jekt_and_a_failed_v1_claims_nothing() {
    let state = test_state();
    let (name_public, name_private) = generate_lan_keypair([1u8; 32]);
    let (uid_public, _) = generate_lan_keypair([2u8; 32]);
    let (_, wrong_private) = generate_lan_keypair([4u8; 32]);
    state.lan_discovery.seed_lan_pubkey_for_test("korp", name_public.to_vec(), Some(("uid-k".into(), uid_public.to_vec())));
    let mut bad_v2 = signed(&name_private, Some(("uid-k", &wrong_private)));
    verify_lan_signature(&state, &mut bad_v2).await;
    assert_eq!(bad_v2.lan_verified, Some(false), "v1 held, but the UID claim is forged");
    assert_eq!(bad_v2.lan_claimed_uid, "");

    let (_, uid_private) = generate_lan_keypair([2u8; 32]);
    let mut bad_v1 = signed(&wrong_private, Some(("uid-k", &uid_private)));
    verify_lan_signature(&state, &mut bad_v1).await;
    assert_eq!(bad_v1.lan_verified, Some(false));
    assert_eq!(bad_v1.lan_claimed_uid, "", "v2 is checked only after v1 verified");
}

#[tokio::test]
async fn the_claimed_uid_reaches_its_delivery_s_audit_entries_and_never_the_wire() {
    let mut handler = Handler::new();
    let claimed = InjectionRequest {
        target_agent: "m4d6-nobody".into(),
        message: "hi".into(),
        source_agent: Some("korp".into()),
        lan_claimed_uid: "uid-k".into(),
        ..Default::default()
    };
    handler.inject_message(claimed.clone());
    assert_eq!(handler.get_audit_log(1)[0].lan_claimed_uid, "uid-k");
    assert_eq!(handler.get_audit_log(1)[0].audit_source_uid, "");
    handler.inject_message(InjectionRequest { lan_claimed_uid: String::new(), ..claimed.clone() });
    assert_eq!(handler.get_audit_log(1)[0].lan_claimed_uid, "", "restored after the delivery");
    let wire = serde_json::to_value(&claimed).unwrap();
    assert!(wire.get("lan_claimed_uid").is_none(), "never forwarded: {wire}");
}

// ── sign_at_origin: an agent whose own process signed nothing ───────────────
// (SPEC_LAN_JEKT_SIGNED_BY_SRV_2026_10_25.md). A Codex agent's MCP sends no
// signature, so its jekts were `TRUST=network-claimed` on every LAN peer.

mod origin {
    use super::*;
    use crate::backend::storage::store::Store;
    use crate::server::reactive::lan_v2::{receipt_note, sign_at_origin, sign_at_origin_why};
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};

    fn store_with_coddy() -> Store {
        let store = Store::open_in_memory().unwrap();
        let mut def = crate::backend::storage::agents::test_agent_def("uid-coddy", "Coddy", "codex", "agent", 1, "");
        def.slug = "coddy".into();
        store.agent_def_insert(&mut def).unwrap();
        store
    }

    /// What an unsigning agent's request looks like when it reaches srv: a
    /// host-tier request with an id and a time, and nothing else.
    fn unsigned() -> InjectionRequest {
        InjectionRequest {
            target_agent: "agenty".into(),
            message: "hello from a Codex agent".into(),
            source_agent: Some("coddy".into()),
            delivery_tier: Some("host".into()),
            request_id: Some("1792887204449-3664-0".into()),
            ts_secs: Some(now()),
            ..Default::default()
        }
    }

    /// The receiving srv, which learned coddy's keys from the sender's answer
    /// to its peer lookup, as it does for any agent.
    fn receiver_for(store: &Store) -> crate::server::AppState {
        let state = test_state();
        let name_key = BASE64.decode(store.agent_lan_public_key_load("coddy").unwrap().unwrap()).unwrap();
        let (uid_keys, _) = store.agent_uid_keys_ensure("uid-coddy").unwrap().unwrap();
        let uid_key = BASE64.decode(uid_keys.lan.public_key).unwrap();
        state.lan_discovery.seed_lan_pubkey_for_test("coddy", name_key, Some(("uid-coddy".into(), uid_key)));
        state
    }

    #[test]
    fn the_receipt_line_says_whether_nothing_was_signed_or_nothing_could_be_checked() {
        let lan = |sig: Option<&str>, source: Option<&str>, verified: Option<bool>| InjectionRequest {
            delivery_tier: Some("lan".into()),
            lan_sig: sig.map(Into::into),
            source_agent: source.map(Into::into),
            lan_verified: verified,
            ..Default::default()
        };
        assert_eq!(receipt_note(&unsigned()), None, "not a LAN request: no line");
        assert_eq!(receipt_note(&lan(Some("s"), Some("korp"), Some(true))), Some("signature verified"));
        assert_eq!(receipt_note(&lan(Some("s"), Some("korp"), Some(false))), Some("signature failed verification"));
        assert!(receipt_note(&lan(None, Some("korp"), None)).unwrap().contains("no signature came with it"));
        assert!(receipt_note(&lan(Some("s"), None, None)).unwrap().contains("names no sender"));
        assert!(receipt_note(&lan(Some("s"), Some("korp"), None)).unwrap().contains("no key for the sender"));
    }

    #[test]
    fn the_log_line_says_what_became_of_the_signature_and_why() {
        let store = store_with_coddy();
        let why = |caller: &str, req: InjectionRequest| sign_at_origin_why(&store, caller, req).1;
        assert_eq!(why("uid-coddy", unsigned()), "signed by srv at the origin");
        let mut own = unsigned();
        own.lan_sig = Some("the agent's".into());
        assert!(why("uid-coddy", own).starts_with("signed by the agent"));
        let mut hop = unsigned();
        hop.delivery_tier = Some("lan".into());
        assert!(why("uid-coddy", hop).contains("not the origin"));
        assert!(why("", unsigned()).contains("no agent token"), "a hop carries no caller");
        assert!(why("uid-someone-else", unsigned()).contains("no agent token"), "a token that doesn't own the name");
        // The request itself is the same one `sign_at_origin` returns.
        assert_eq!(sign_at_origin_why(&store, "uid-coddy", unsigned()).0.lan_sig.is_some(), sign_at_origin(&store, "uid-coddy", unsigned()).lan_sig.is_some());
    }

    #[tokio::test]
    async fn what_the_origin_signs_verifies_at_the_receiver_like_an_agents_own_signature() {
        let store = store_with_coddy();
        let mut fwd = sign_at_origin(&store, "uid-coddy", unsigned());
        assert!(fwd.lan_sig.is_some() && fwd.lan_sig_v2.is_some());
        assert_eq!(fwd.source_uid.as_deref(), Some("uid-coddy"));
        // The peer forces its own tier on arrival (`resolve_delivery_tier`).
        fwd.delivery_tier = Some("lan".into());
        let state = receiver_for(&store);
        verify_lan_signature(&state, &mut fwd).await;
        assert_eq!(fwd.lan_verified, Some(true));
        assert_eq!(fwd.lan_claimed_uid, "uid-coddy");
    }

    #[tokio::test]
    async fn a_sender_without_an_id_or_time_gets_both_and_still_verifies() {
        let store = store_with_coddy();
        let mut bare = unsigned();
        bare.request_id = None;
        bare.ts_secs = None;
        let mut fwd = sign_at_origin(&store, "uid-coddy", bare);
        assert!(fwd.request_id.as_deref().is_some_and(|id| !id.is_empty()));
        assert!(fwd.ts_secs.is_some_and(|t| t > 0));
        fwd.delivery_tier = Some("lan".into());
        let state = receiver_for(&store);
        verify_lan_signature(&state, &mut fwd).await;
        assert_eq!(fwd.lan_verified, Some(true));
    }

    #[tokio::test]
    async fn a_message_changed_after_signing_fails_at_the_receiver() {
        let store = store_with_coddy();
        let mut fwd = sign_at_origin(&store, "uid-coddy", unsigned());
        fwd.delivery_tier = Some("lan".into());
        fwd.message.push_str(" (and a forged line)");
        let state = receiver_for(&store);
        verify_lan_signature(&state, &mut fwd).await;
        assert_eq!(fwd.lan_verified, Some(false));
    }

    #[test]
    fn an_agents_own_signature_is_never_replaced() {
        let store = store_with_coddy();
        let mut own = unsigned();
        own.lan_sig = Some("the agent's own".into());
        let out = sign_at_origin(&store, "uid-coddy", own);
        assert_eq!(out.lan_sig.as_deref(), Some("the agent's own"));
        assert_eq!(out.lan_sig_v2, None, "nothing is added beside a signature srv did not make");
    }

    #[test]
    fn only_the_origin_signs_and_only_for_the_agent_its_token_names() {
        let store = store_with_coddy();
        let signed = |req: InjectionRequest, caller: &str| sign_at_origin(&store, caller, req).lan_sig.is_some();
        assert!(signed(unsigned(), "uid-coddy"));
        assert!(signed(InjectionRequest { source_agent: Some("Coddy".into()), ..unsigned() }, "uid-coddy"), "the slug, ignoring case");
        // A forwarded hop, a script: no caller.
        assert!(!signed(unsigned(), ""));
        // A token for another agent, or for none this srv knows.
        assert!(!signed(unsigned(), "uid-nobody"));
        assert!(!signed(InjectionRequest { source_agent: Some("agenta".into()), ..unsigned() }, "uid-coddy"), "another agent's name");
        assert!(!signed(InjectionRequest { source_agent: None, ..unsigned() }, "uid-coddy"));
        // Not an origin request: a hop already labelled by an earlier srv.
        for tier in ["lan", "wan", "channel"] {
            assert!(!signed(InjectionRequest { delivery_tier: Some(tier.into()), ..unsigned() }, "uid-coddy"), "{tier}");
        }
    }

    #[test]
    fn a_uid_the_sender_carried_is_not_overwritten() {
        let store = store_with_coddy();
        let mut req = unsigned();
        req.source_uid = Some("uid-claimed-by-the-sender".into());
        let out = sign_at_origin(&store, "uid-coddy", req);
        assert!(out.lan_sig.is_some());
        assert_eq!(out.source_uid.as_deref(), Some("uid-claimed-by-the-sender"));
        assert_eq!(out.lan_sig_v2, None, "a v2 for a UID the token does not name is not made");
    }
}

// ── channel_sign_at_origin: the same-machine, other-instance hop ───────────────
// A container agent's MCP holds no signing key, so the cross-channel signature a receiving instance looks
// for on a published sender is made here.
mod channel_origin {
    use super::*;
    use crate::backend::storage::store::Store;
    use crate::server::reactive::lan_v2::channel_sign_at_origin;
    use agentmux_common::jekt_sign::{decode_key, verify_channel_jekt, verify_channel_jekt_v2};

    fn store_with_coddy() -> Store {
        let store = Store::open_in_memory().unwrap();
        let mut def = crate::backend::storage::agents::test_agent_def("uid-coddy", "Coddy", "codex", "agent", 1, "");
        def.slug = "coddy".into();
        store.agent_def_insert(&mut def).unwrap();
        store
    }

    fn unsigned() -> InjectionRequest {
        InjectionRequest {
            target_agent: "agenty".into(),
            message: "hello across channels".into(),
            source_agent: Some("coddy".into()),
            delivery_tier: Some("host".into()),
            request_id: Some("1792887204449-3664-1".into()),
            ts_secs: Some(now()),
            ..Default::default()
        }
    }

    #[test]
    fn what_the_origin_signs_verifies_against_the_keys_the_agent_publishes() {
        let store = store_with_coddy();
        let out = channel_sign_at_origin(&store, "uid-coddy", unsigned());
        let channel = out.source_channel.clone().expect("the channel the signature binds");
        let (name_pub, uid_pub) = (
            decode_key(&store.agent_lan_public_key_load("coddy").unwrap().unwrap()).unwrap(),
            decode_key(&store.agent_uid_keys_ensure("uid-coddy").unwrap().unwrap().0.lan.public_key).unwrap(),
        );
        let (id, ts) = (out.request_id.as_deref().unwrap(), out.ts_secs.unwrap());
        assert!(verify_channel_jekt(&name_pub, id, "coddy", &channel, "agenty", ts, &out.message, out.channel_sig.as_deref().unwrap()));
        assert_eq!(out.source_uid.as_deref(), Some("uid-coddy"));
        assert!(verify_channel_jekt_v2(&uid_pub, id, "coddy", "uid-coddy", &channel, "agenty", ts, &out.message, out.channel_sig_v2.as_deref().unwrap()));
        let mut changed = out.message.clone();
        changed.push_str(" (forged)");
        assert!(!verify_channel_jekt(&name_pub, id, "coddy", &channel, "agenty", ts, &changed, out.channel_sig.as_deref().unwrap()));
    }

    #[test]
    fn an_agents_own_signature_is_never_replaced_and_only_the_origin_signs() {
        let store = store_with_coddy();
        let mut own = unsigned();
        own.channel_sig = Some("the agent's own".into());
        let out = channel_sign_at_origin(&store, "uid-coddy", own);
        assert_eq!(out.channel_sig.as_deref(), Some("the agent's own"));
        assert_eq!(out.channel_sig_v2, None);

        let signed = |req: InjectionRequest, caller: &str| channel_sign_at_origin(&store, caller, req).channel_sig.is_some();
        assert!(signed(unsigned(), "uid-coddy"));
        assert!(!signed(unsigned(), ""), "a hop carries no caller");
        assert!(!signed(unsigned(), "uid-nobody"));
        assert!(!signed(InjectionRequest { source_agent: Some("agenta".into()), ..unsigned() }, "uid-coddy"), "another agent's name");
        for tier in ["lan", "wan", "channel"] {
            assert!(!signed(InjectionRequest { delivery_tier: Some(tier.into()), ..unsigned() }, "uid-coddy"), "{tier}");
        }
    }
}

// A signed jekt whose sender's key a cached miss hid: asked for again, once.
mod refetch {
    use super::*;
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};

    /// A peer that answers every key lookup with `body`.
    async fn peer_saying(body: String) -> String {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else { return };
                let body = body.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 2048];
                    let _ = stream.read(&mut buf).await;
                    let resp = format!(
                        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                    let _ = stream.write_all(resp.as_bytes()).await;
                });
            }
        });
        format!("127.0.0.1:{}", addr.port())
    }

    fn answer_with(public: &[u8; 32]) -> String {
        format!(r#"{{"lan_public_key":"{}"}}"#, BASE64.encode(public))
    }

    #[tokio::test]
    async fn a_key_found_on_the_second_ask_verifies_the_jekt() {
        let state = test_state();
        let (public, private) = generate_lan_keypair([1u8; 32]);
        state.lan_discovery.seed_peer_at_for_test(&peer_saying(answer_with(&public)).await);
        state.lan_discovery.seed_lan_pubkey_miss_for_test("korp", 30);
        let mut req = signed(&private, None);
        verify_lan_signature(&state, &mut req).await;
        assert_eq!(req.lan_verified, Some(true));
    }

    #[tokio::test]
    async fn a_sender_still_not_found_stays_unverified_not_failed() {
        let state = test_state();
        let (_, private) = generate_lan_keypair([1u8; 32]);
        state.lan_discovery.seed_peer_at_for_test(&peer_saying("{}".into()).await);
        state.lan_discovery.seed_lan_pubkey_miss_for_test("korp", 30);
        let mut req = signed(&private, None);
        verify_lan_signature(&state, &mut req).await;
        assert_eq!(req.lan_verified, None);
    }

    #[tokio::test]
    async fn a_key_that_differs_from_the_pin_is_never_upgraded() {
        let state = test_state();
        let (pinned_public, _) = generate_lan_keypair([9u8; 32]);
        state.mstore.lan_peer_pubkey_pin_get_or_set("korp", &BASE64.encode(pinned_public)).unwrap();
        // The peer now gives another key, and the jekt is signed with it.
        let (other_public, other_private) = generate_lan_keypair([1u8; 32]);
        state.lan_discovery.seed_peer_at_for_test(&peer_saying(answer_with(&other_public)).await);
        state.lan_discovery.seed_lan_pubkey_miss_for_test("korp", 30);
        let mut req = signed(&other_private, None);
        verify_lan_signature(&state, &mut req).await;
        assert_eq!(req.lan_verified, Some(false), "the pin holds the first key");
    }
}
