// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The `identity_id` legacy column: what a blank/empty value means at the
//! inject step, and that the account binding (not that column) supplies the
//! credentials.
//!
//! In its own file because `inject.rs` is over the 1,500-line size ratchet
//! (`scripts/check-file-sizes.baseline`): new tests go in a child module.

use super::super::inject_identity_env;
use super::*;

#[test]
fn inject_blank_identity_is_gated_same_as_any_other_unresolvable_identity() {
    // #2463 Finding 2: this test used to assert the OPPOSITE — that a
    // blank identity_id was NOT gated by layer 3 and silently fell
    // back to ambient creds even with use_ambient_login=0. That let a
    // brand-new agent launch with no account selected spawn on
    // whatever ambient credential happened to be on disk, with
    // nothing bound in Connectors — observably different behavior on two
    // machines running the identical code, purely as a function of
    // which one had a stray credential file. A blank/empty
    // identity_id must be gated exactly like any other unresolvable
    // one (see `spawn_blocked_when_oauth_def_provider_has_no_binding_
    // and_flag_false`, the non-blank equivalent of this test).
    let store = make_store();
    let mut def = gate_def(0);
    store.agent_def_insert(&mut def).unwrap();

    insert_block_for_agent(&store, "block-blank", "def-1");
    let inst = make_instance("block-blank", "blank");
    store.instance_create(&inst).unwrap();

    let mut env: HashMap<String, String> = HashMap::new();
    let res = inject_identity_env(store.clone(), store.clone(), store, "block-blank", &mut env);

    assert_eq!(
        res,
        Err(SpawnGateError::MissingCredentials { provider: "claude".to_string(), linked: false }),
    );
    assert!(env.is_empty());
}

#[test]
fn inject_empty_identity_is_gated_same_as_blank() {
    // Same as the blank case above, but the genuinely-empty string —
    // what a real brand-new-agent launch with no account selected
    // actually produces (#2463 Finding 2's exact repro), as opposed to
    // the legacy "blank" singleton literal.
    let store = make_store();
    let mut def = gate_def(0);
    store.agent_def_insert(&mut def).unwrap();

    insert_block_for_agent(&store, "block-empty", "def-1");
    let inst = make_instance("block-empty", "");
    store.instance_create(&inst).unwrap();

    let mut env: HashMap<String, String> = HashMap::new();
    let res = inject_identity_env(store.clone(), store.clone(), store, "block-empty", &mut env);

    assert_eq!(
        res,
        Err(SpawnGateError::MissingCredentials { provider: "claude".to_string(), linked: false }),
    );
    assert!(env.is_empty());
}

#[test]
fn inject_empty_identity_with_a_linked_account_injects_that_account() {
    // #30077: an empty identity_id is the normal state of a modern launch, not
    // a credential failure. The account's per-provider binding is what supplies
    // the credentials; the live 0.60.21 warnings were this exact case (empty
    // identity_id, an account linked to the definition). It must inject that
    // account, not fall back to ambient creds and not refuse.
    let store = make_store();
    let mut def = gate_def(0);
    store.agent_def_insert(&mut def).unwrap();

    let claude = make_account(
        "acct-linked",
        "claude",
        SecretRef::OAuthConfigDir {
            dir: test_config_dir("empty-identity-linked"),
        },
    );
    store.identity_upsert(&claude).unwrap();
    store.agent_identity_link("def-1", "acct-linked", "claude").unwrap();

    insert_block_for_agent(&store, "block-empty-linked", "def-1");
    let inst = make_instance("block-empty-linked", "");
    store.instance_create(&inst).unwrap();

    let mut env: HashMap<String, String> = HashMap::new();
    inject_identity_env(store.clone(), store.clone(), store, "block-empty-linked", &mut env)
        .expect("an empty identity_id with a bound account must spawn on that account");

    assert_eq!(
        env.get("CLAUDE_CONFIG_DIR").cloned(),
        Some(test_config_dir("empty-identity-linked")),
        "the bound account's dir is what the spawn gets, whatever identity_id holds",
    );
}
