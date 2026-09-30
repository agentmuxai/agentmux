// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// `identity_self_accounts_tests`, moved out of server/app_api/mod.rs unchanged
// (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4, item 9).

use super::*;
use crate::backend::storage::identities::SecretRef;
use crate::server::tests::test_state;

fn sample_account(id: &str, provider: &str) -> IdentityAccount {
    IdentityAccount {
        id: id.to_string(),
        name: format!("{provider}-oauth"),
        provider: provider.to_string(),
        kind: "oauth".to_string(),
        display_name: String::new(),
        secret_ref: SecretRef::OAuthConfigDir { dir: format!("/tmp/{id}") },
        context: json!({}),
        status: "ok".to_string(),
        created_at: 0,
        updated_at: 0,
    }
}

/// Regression for reagent P1 on PR #2419 review: one malformed linked
/// account must not hide this agent's other, perfectly readable
/// accounts — the same bug class `identity_list` was fixed for, just
/// reachable through this separate per-agent lookup too.
#[tokio::test]
async fn skips_a_malformed_linked_account_instead_of_failing_the_whole_call() {
    let state = test_state();

    let mut def = AgentDefinition {
        conversation_visibility: crate::backend::storage::agents::default_conversation_visibility(),
        id: uuid::Uuid::new_v4().to_string(),
        slug: String::new(),
        name: "test agent".to_string(),
        icon: String::new(),
        provider: "claude".to_string(),
        description: String::new(),
        working_directory: String::new(),
        shell: String::new(),
        environment: String::new(),
        provider_flags: String::new(),
        auto_start: 0,
        restart_on_crash: 0,
        idle_timeout_minutes: 0,
        created_at: 0,
        agent_type: String::new(),
        agent_bus_id: String::new(),
        is_seeded: 0,
        accounts: String::new(),
        parent_id: String::new(),
        branch_label: String::new(),
        updated_at: 0,
        user_hidden: 0,
        container_image: String::new(),
        container_volumes: "[]".to_string(),
        container_name: String::new(),
        use_ambient_login: 0,
        model_vendor_base_url: String::new(),
        auto_continue_enabled: 0,
        memory_id: String::new(),
    };
    state.mstore.agent_def_insert(&mut def).unwrap();

    state.mstore.identity_upsert(&sample_account("acct-good", "claude")).unwrap();
    {
        let conn = state.mstore.conn().lock().unwrap();
        conn.execute(
            "INSERT INTO db_accounts
                (id, name, provider, kind, display_name, secret_ref, context,
                 status, created_at, updated_at)
             VALUES ('acct-bad', 'broken', 'github', 'oauth', '',
                     '{\"backend\":\"oauth_config_dir\",\"dir\":\"C:\\bad\\path\"}',
                     '{}', 'unknown', 0, 0)",
            [],
        )
        .unwrap();
    }
    state.mstore.agent_identity_link(&def.id, "acct-good", "claude").unwrap();
    state.mstore.agent_identity_link(&def.id, "acct-bad", "github").unwrap();

    let result = identity_self_accounts_impl(&state, &def.id)
        .await
        .expect("must not fail even with a malformed linked account present");
    let accounts = result["accounts"].as_array().unwrap();
    assert_eq!(accounts.len(), 1, "the malformed account must be skipped, not error the whole call");
    assert_eq!(accounts[0]["account_id"], json!("acct-good"));
}

// reagentx P1 on PR #2428 (round 4): `resolve_agent_definition_id`
// (the shared resolver behind `identity.self.*`) only tried
// `instance_get_by_slug` (local `db_agents`) then `agent_def_get`
// (treating the slug as if it were a definition UUID, which it never
// is) — no registry fallback, unlike `bundle_self_get_impl`. So the
// common case (a live agent that only exists in the global
// named-agent registry, per that function's own doc comment) stayed
// broken for `IdentityAccounts` even after the slug/name namespace
// split — the exact tool this PR's own description cited as
// confirmed-live-broken.
#[tokio::test]
async fn resolve_agent_definition_id_falls_back_to_the_registry_for_a_slug_only_agent() {
    use crate::test_support::ISOLATED_AUTH_ENV_LOCK as ENV_LOCK;
    let _guard = ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());

    let state = test_state();
    let mut def = AgentDefinition {
        conversation_visibility: crate::backend::storage::agents::default_conversation_visibility(),
        id: uuid::Uuid::new_v4().to_string(),
        slug: "agenty".to_string(),
        name: "AgentY".to_string(),
        icon: String::new(),
        provider: "claude".to_string(),
        description: String::new(),
        working_directory: String::new(),
        shell: String::new(),
        environment: String::new(),
        provider_flags: String::new(),
        auto_start: 0,
        restart_on_crash: 0,
        idle_timeout_minutes: 0,
        created_at: 0,
        agent_type: String::new(),
        agent_bus_id: String::new(),
        is_seeded: 0,
        accounts: String::new(),
        parent_id: String::new(),
        branch_label: String::new(),
        updated_at: 0,
        user_hidden: 0,
        container_image: String::new(),
        container_volumes: "[]".to_string(),
        container_name: String::new(),
        use_ambient_login: 0,
        model_vendor_base_url: String::new(),
        auto_continue_enabled: 0,
        memory_id: String::new(),
    };
    state.mstore.agent_def_insert(&mut def).unwrap();
    state.mstore.identity_upsert(&sample_account("acct-good", "claude")).unwrap();
    state.mstore.agent_identity_link(&def.id, "acct-good", "claude").unwrap();

    // Deliberately NO `instance_create` call — this agent exists only
    // in the global registry, exactly like a real live-launched agent
    // that never got a local `db_agents` instance row.
    let tmp = tempfile::tempdir().unwrap();
    let prev = std::env::var_os("AGENTMUX_HOME_OVERRIDE");
    std::env::set_var("AGENTMUX_HOME_OVERRIDE", tmp.path());

    let registry_dir = tmp.path().join("shared").join("agents").join("registry");
    let registry = crate::registry::Registry::open(registry_dir).unwrap();
    registry
        .upsert(&crate::registry::NamedAgentRecord {
            schema_version: 1,
            data: crate::registry::NamedAgentRecordV1 {
                instance_id: "inst-agenty".to_string(),
                instance_name: "AgentY".to_string(),
                definition_id: def.id.clone(),
                identity_id: None,
                memory_id: None,
                session_id: None,
                working_dir: "agenty-0629j".to_string(),
                source_agents_base: None,
                created_at_ms: 1,
                last_launched_at_ms: 1,
                created_by_version: "test".to_string(),
                last_launched_by_version: "test".to_string(),
            },
        })
        .unwrap();

    let result = identity_self_accounts_impl(&state, "agenty").await;

    match prev {
        Some(v) => std::env::set_var("AGENTMUX_HOME_OVERRIDE", v),
        None => std::env::remove_var("AGENTMUX_HOME_OVERRIDE"),
    }

    let result = result.expect("must resolve via the registry fallback, not error 'unknown agent'");
    let accounts = result["accounts"].as_array().unwrap();
    assert_eq!(accounts.len(), 1);
    assert_eq!(accounts[0]["account_id"], json!("acct-good"));
}
