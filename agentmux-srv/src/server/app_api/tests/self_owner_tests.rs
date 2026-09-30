// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// `self_owner_tests`, moved out of server/app_api/mod.rs unchanged
// (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4, item 9).

use super::*;
use crate::backend::storage::agents::test_agent_def;
use crate::server::tests::test_state;

/// Identity M4c-2c (spec §6.5.9), the colliding-names fixture: "AgentY"
/// (`agenty`, bundle-y, account acct-y) and "AGENTY" (`agenty-2`,
/// bundle-y2, no account). The second agent, attributed, sees its own
/// accounts and preset whatever slug it sends, and cannot validate the
/// first one's account; by slug, `agenty` is still the first agent.
#[tokio::test]
async fn an_attributed_self_call_is_about_the_callers_row() {
    let state = test_state();
    for (id, name, slug, bundle) in [
        ("uid-so-y", "AgentY", "agenty", "bundle-so-y"),
        ("uid-so-y2", "AGENTY", "agenty-2", "bundle-so-y2"),
    ] {
        let mut def = test_agent_def(id, name, "claude", "agent", 1, "");
        def.slug = slug.to_string();
        state.mstore.agent_def_insert(&mut def).unwrap();
        // The launch's bundle (`db_agents.memory_id`), which preset.get
        // self reads — not the definition's default.
        state
            .mstore
            .conn()
            .lock()
            .unwrap()
            .execute("UPDATE db_agents SET memory_id = ?1 WHERE id = ?2", [bundle, id])
            .unwrap();
        let b: crate::backend::storage::bundles::Bundle =
            serde_json::from_value(json!({"id": bundle, "name": bundle})).unwrap();
        state.id_store.bundle_upsert(&b).unwrap();
    }
    let acct = crate::backend::storage::IdentityAccount {
        id: "acct-so-y".to_string(),
        name: "claude-oauth".to_string(),
        provider: "claude".to_string(),
        kind: "oauth".to_string(),
        display_name: String::new(),
        secret_ref: crate::backend::storage::SecretRef::OAuthConfigDir {
            dir: "/tmp/acct-so-y".to_string(),
        },
        context: json!({}),
        status: "ok".to_string(),
        created_at: 0,
        updated_at: 0,
    };
    state.mstore.identity_upsert(&acct).unwrap();
    state.mstore.agent_identity_link("uid-so-y", "acct-so-y", "claude").unwrap();
    let y2 = SelfOwner::Uid("uid-so-y2");

    let accounts = |v: serde_json::Value| v["accounts"].as_array().unwrap().len();
    assert_eq!(accounts(identity_self_accounts_impl(&state, y2).await.unwrap()), 0);
    assert_eq!(accounts(identity_self_accounts_impl(&state, "agenty").await.unwrap()), 1);
    let err = identity_account_validate_stored_impl(&state, y2, "acct-so-y").await.unwrap_err();
    assert!(err.starts_with("FORBIDDEN"), "{err}");

    assert_eq!(bundle_self_get_impl(&state, y2).await.unwrap()["id"], "bundle-so-y2");
    assert_eq!(bundle_self_get_impl(&state, "agenty").await.unwrap()["id"], "bundle-so-y");

    let gone = SelfOwner::Uid("uid-so-gone");
    for err in [
        identity_self_accounts_impl(&state, gone).await.unwrap_err(),
        identity_account_validate_stored_impl(&state, gone, "acct-so-y").await.unwrap_err(),
        bundle_self_get_impl(&state, gone).await.unwrap_err(),
    ] {
        assert!(err.contains("calling agent uid-so-gone not found"), "{err}");
    }
}

/// History for an attributed caller is its own row's: its id keys the
/// identity links, its working directory finds ambient-credential
/// sessions.
#[test]
fn a_history_owner_of_a_row_is_that_rows() {
    let mut def = test_agent_def("uid-so-h", "H", "claude", "agent", 1, "");
    def.working_directory = "/work/h".to_string();
    let owner = crate::backend::history::HistoryOwner::of_row(&def);
    assert_eq!(owner.definition_id, "uid-so-h");
    assert_eq!(owner.working_directory.as_deref(), Some("/work/h"));
}
