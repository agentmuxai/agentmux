// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Former slugs on the message routes (`SPEC_FORK_SLUG_MIGRATION_2026_10_22.md`
//! Phase 1): they receive, and never speak.

use super::super::verify_jekt_signature;
use super::*;
use crate::backend::storage::agents::test_agent_def;
use crate::server::tests::test_state;

/// An agent `uid` defined as `old`, then renamed to `new`, with a host key
/// left under each name (Phase 2 retires the old one; until then it exists).
fn renamed(state: &AppState, uid: &str, old: &str, new: &str) -> (Vec<u8>, Vec<u8>) {
    let mut def = test_agent_def(uid, uid, "claude", "host", 1, "");
    def.slug = old.to_string();
    state.mstore.agent_def_insert(&mut def).unwrap();
    let old_key = state.mstore.agent_jekt_key_ensure(old).unwrap();
    state.mstore.test_rename_slug(uid, new, agentmux_common::time::now_ms());
    let new_key = state.mstore.agent_jekt_key_ensure(new).unwrap();
    (old_key, new_key)
}

fn signed(key: &[u8], source: &str) -> InjectionRequest {
    let ts = agentmux_common::time::now_secs();
    InjectionRequest {
        target_agent: "agenty".into(),
        message: "hello".into(),
        source_agent: Some(source.into()),
        delivery_tier: Some("host".into()),
        request_id: Some("msg-former".into()),
        ts_secs: Some(ts),
        jekt_sig: Some(agentmux_common::jekt_sign::sign_jekt(key, "msg-former", source, "agenty", ts, "hello")),
        ..Default::default()
    }
}

/// A message signed as a former slug, with the key still filed under it, is
/// refused as a sender: unverified, so forced sensitive and stopped. The
/// agent signing as its current slug verifies.
#[tokio::test]
async fn a_message_signed_as_a_former_slug_is_refused() {
    let state = test_state();
    let (old_key, new_key) = renamed(&state, "uid-fs-sign", "fs-sign-fs-sign-2", "fs-sign-2");

    let mut req = signed(&old_key, "fs-sign-fs-sign-2");
    verify_jekt_signature(&state, &mut req);
    assert_eq!(req.sig_verified, Some(false));

    let mut unsigned = signed(&old_key, "FS-SIGN-FS-SIGN-2");
    unsigned.jekt_sig = None;
    verify_jekt_signature(&state, &mut unsigned);
    assert_eq!(unsigned.sig_verified, Some(false), "a claim of a former slug is unverified, signed or not");

    let mut current = signed(&new_key, "fs-sign-2");
    verify_jekt_signature(&state, &mut current);
    assert_eq!(current.sig_verified, Some(true));
    assert!(!crate::server::actor::uid_owns_name(&state.mstore, "uid-fs-sign", "fs-sign-fs-sign-2"));
    assert!(!crate::server::actor::uid_has_slug(&state.mstore, "uid-fs-sign", "fs-sign-fs-sign-2"));
}

/// `self/keys` serves the current slug's keys only, never the ones left
/// under a former slug.
#[tokio::test]
async fn self_keys_serve_the_current_slug_only() {
    let state = test_state();
    let (old_key, new_key) = renamed(&state, "uid-fs-keys", "fs-keys-fs-keys-2", "fs-keys-2");
    let keys = crate::server::agent_self_keys::self_keys_for(&state.mstore, "uid-fs-keys").unwrap();
    use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
    assert_eq!(keys.slug, "fs-keys-2");
    assert_eq!(keys.jekt_key, Some(BASE64.encode(&new_key)));
    assert_ne!(keys.jekt_key, Some(BASE64.encode(&old_key)));
    let old_lan = state.mstore.agent_lan_key_ensure("fs-keys-fs-keys-2").unwrap();
    assert_ne!(keys.lan_key, Some(old_lan.private_key));
}

/// A LAN peer asking for a former slug gets the agent's current name and the
/// key filed under it, never the one left under the old name.
#[tokio::test]
async fn a_lan_lookup_of_a_former_slug_gives_the_current_name_and_key() {
    let state = test_state();
    renamed(&state, "uid-fs-lan", "fs-lan-fs-lan-2", "fs-lan-2");
    let old_pub = state.mstore.agent_lan_key_ensure("fs-lan-fs-lan-2").unwrap().public_key;
    let new_pub = state.mstore.agent_lan_key_ensure("fs-lan-2").unwrap().public_key;
    // As `handle_reactive_agent` builds it before this runs: the key under
    // the name asked for.
    let mut body = serde_json::json!({ "agent_id": "Fs-Lan-2", "lan_public_key": old_pub });
    renamed_lookup(&state, "fs-lan-fs-lan-2", &mut body);
    assert_eq!(body["current_name"], "fs-lan-2");
    assert_eq!(body["lan_public_key"], serde_json::json!(new_pub));

    // An agent's own name, asked for as itself, is untouched.
    let mut own = serde_json::json!({ "lan_public_key": new_pub });
    renamed_lookup(&state, "fs-lan-2", &mut own);
    assert!(own.get("current_name").is_none());
}

/// When the current name isn't the agent's alone (another row's slug folds
/// to it), no key is served for the former slug at all: it would not be only
/// this agent's.
#[tokio::test]
async fn a_lan_lookup_never_serves_a_key_that_is_not_the_agents_alone() {
    let state = test_state();
    renamed(&state, "uid-fs-own", "fs-own-fs-own-2", "fs-own-2");
    let mut other = test_agent_def("uid-fs-other", "Other", "claude", "host", 1, "");
    other.slug = "fs-own-x".into();
    state.mstore.agent_def_insert(&mut other).unwrap();
    state.mstore.test_force_slug("uid-fs-other", "FS-OWN-2").unwrap();
    state.mstore.agent_lan_key_ensure("fs-own-2").unwrap();
    let mut body = serde_json::json!({ "lan_public_key": "old" });
    renamed_lookup(&state, "fs-own-fs-own-2", &mut body);
    assert_eq!(body["current_name"], "fs-own-2");
    assert!(body.get("lan_public_key").is_none(), "{body}");
}

/// The miss path finds a former slug's registry entries under its current
/// slug, and never evicts one found that way.
#[test]
fn registries_are_searched_under_the_current_slug_after_the_name_as_sent() {
    let dir = tempfile::tempdir().unwrap();
    agent_registry::write(dir.path(), "fs-reg-2", "http://127.0.0.1:9101", "block-r");
    assert!(registry_entry(dir.path(), "fs-reg-fs-reg-2", None).is_none());
    let (found_as, entry) = registry_entry(dir.path(), "fs-reg-fs-reg-2", Some("fs-reg-2")).unwrap();
    assert_eq!((found_as.as_str(), entry.block_id.as_str()), ("fs-reg-2", "block-r"));

    agent_registry::write_shared(dir.path(), "fs-reg-2", "http://127.0.0.1:9102", "block-s", "dev-a");
    let (found_as, entries) = shared_entries(dir.path(), "fs-reg-fs-reg-2", Some("fs-reg-2"));
    assert_eq!((found_as.as_str(), entries.len()), ("fs-reg-2", 1));
    let (found_as, entries) = shared_entries(dir.path(), "fs-reg-2", Some("unused"));
    assert_eq!((found_as.as_str(), entries.len()), ("fs-reg-2", 1), "the name as sent comes first");

    let req = InjectionRequest { target_agent: "fs-reg-fs-reg-2".into(), ..Default::default() };
    assert!(!may_evict("fs-reg-2", &req));
    assert!(may_evict("fs-reg-fs-reg-2", &req));
}

/// UI automation verifies a signature, then acts as its signer's pane: a
/// signer named by a former slug is refused, even with its old key and a
/// valid signature. The same agent under its current slug is accepted.
#[tokio::test]
async fn ui_automation_signed_as_a_former_slug_is_refused() {
    let state = test_state();
    let (old_key, new_key) = renamed(&state, "uid-fs-ui", "fs-ui-fs-ui-2", "fs-ui-2");
    crate::backend::reactive::handler::get_global_handler()
        .register_agent_full("fs-ui-2", "block-fs-ui", None, 0, Some("fs-ui-2"), Some("uid-fs-ui"), "test")
        .unwrap();
    let auth = |key: &[u8], name: &str| {
        let ts = agentmux_common::time::now_secs();
        let sig = agentmux_common::jekt_sign::sign_jekt(key, "ui-automation-identity", name, "__srv__", ts, "");
        agentmux_common::api_types::UiAutomationAuth { agent_id: name.into(), ts_secs: ts, sig }
    };
    let refused = crate::server::ui_handlers::verified_block_id(&state, None, &auth(&old_key, "fs-ui-fs-ui-2"));
    assert!(refused.as_ref().is_err_and(|e| e.contains("renamed")), "{refused:?}");
    let accepted = crate::server::ui_handlers::verified_block_id(&state, None, &auth(&new_key, "fs-ui-2"));
    assert_eq!(accepted.as_deref(), Ok("block-fs-ui"));
}

/// #1459: a message sent from another channel to "Naki-2", the route form of
/// "Naki #2", is looked for under the slug the agent runs as, where the
/// cross-channel registry files it, instead of falling to the cloud relay;
/// the request is forwarded as sent, so the signature over "Naki-2" still
/// verifies at the receiver, and an entry found under the slug isn't evicted
/// for a failure on a name its instance may not know.
#[tokio::test]
async fn a_display_routes_message_is_looked_for_under_the_slug_and_still_verifies() {
    let state = test_state();
    let mut def = test_agent_def("uid-dr-naki", "Dr Naki #2", "claude", "host", 1, "");
    def.slug = "dr-naki-dr-naki-2".into();
    state.mstore.agent_def_insert(&mut def).unwrap();
    assert_eq!(delivery_alias(&state, "Dr-Naki-2").as_deref(), Some("dr-naki-dr-naki-2"));
    assert_eq!(delivery_alias(&state, "dr-naki-dr-naki-2"), None, "a slug is looked for as itself");

    let dir = tempfile::tempdir().unwrap();
    agent_registry::write_shared(dir.path(), "dr-naki-dr-naki-2", "http://127.0.0.1:9103", "block-dr", "dev-b");
    let alias = delivery_alias(&state, "Dr-Naki-2");
    let (found_as, entries) = shared_entries(dir.path(), "Dr-Naki-2", alias.as_deref());
    assert_eq!((found_as.as_str(), entries.len()), ("dr-naki-dr-naki-2", 1));

    // The sender's own key; the request keeps target "Dr-Naki-2" end to end.
    let key = state.mstore.agent_jekt_key_ensure("agenty").unwrap();
    let ts = agentmux_common::time::now_secs();
    let mut req = InjectionRequest {
        target_agent: "Dr-Naki-2".into(),
        message: "hello".into(),
        source_agent: Some("agenty".into()),
        delivery_tier: Some("host".into()),
        request_id: Some("msg-dr".into()),
        ts_secs: Some(ts),
        jekt_sig: Some(agentmux_common::jekt_sign::sign_jekt(&key, "msg-dr", "agenty", "Dr-Naki-2", ts, "hello")),
        ..Default::default()
    };
    assert!(!may_evict(&found_as, &req));
    verify_jekt_signature(&state, &mut req);
    assert_eq!(req.sig_verified, Some(true));
    assert_eq!(req.target_agent, "Dr-Naki-2");
}
