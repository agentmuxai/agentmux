// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Behaviour tests for `agent_define_core`'s validation, `if_exists` policies and stub handling
//! (the vendor-URL, provider-lock and bundle tests sit next to the function).

use super::*;
use crate::server::tests::test_state;

fn cmd(value: serde_json::Value) -> CommandAgentDefineData {
    serde_json::from_value(value).unwrap()
}

async fn define(state: &AppState, value: serde_json::Value) -> Result<AgentDefineResult, String> {
    agent_define_core(state.mstore.clone(), state.broker.clone(), cmd(value)).await
}

/// The name on the definition's instance row: empty until a stub names it.
fn stub_name(state: &AppState, def_id: &str) -> String {
    state.mstore.instance_get(def_id).unwrap().map(|i| i.instance_name).unwrap_or_default()
}

fn names(state: &AppState) -> Vec<String> {
    state.mstore.agent_def_list().unwrap().into_iter().map(|d| d.name).collect()
}

#[tokio::test]
async fn a_blank_name_is_refused() {
    let state = test_state();
    let err = define(&state, json!({ "name": "   " })).await.unwrap_err();
    assert_eq!(err, "agent.define: name is required");
}

#[tokio::test]
async fn an_unknown_if_exists_is_refused_even_for_a_new_name() {
    let state = test_state();
    let err = define(&state, json!({ "name": "typo-agent", "if_exists": "overwrite" })).await.unwrap_err();
    assert_eq!(err, "agent.define: unknown if_exists value 'overwrite'; valid: skip, update, error");
    assert!(!names(&state).contains(&"typo-agent".to_string()));
}

#[tokio::test]
async fn an_unknown_provider_is_refused_and_nothing_is_saved() {
    let state = test_state();
    let err = define(&state, json!({ "name": "no-provider-agent", "provider": "nonesuch" })).await.unwrap_err();
    assert!(err.starts_with("agent.define: unknown provider 'nonesuch'; valid: claude, codex"), "{err}");
    assert!(!names(&state).contains(&"no-provider-agent".to_string()));
}

#[tokio::test]
async fn with_no_provider_and_no_model_the_agent_is_a_claude_agent_with_no_model_flag() {
    let state = test_state();
    let created = define(&state, json!({ "name": "default-provider-agent" })).await.unwrap();
    let def = state.mstore.agent_def_get(&created.definition_id).unwrap().unwrap();
    assert_eq!(def.provider, "claude");
    assert_eq!(def.provider_flags, "");
}

#[tokio::test]
async fn the_provider_is_inferred_from_the_model_and_the_model_becomes_a_cli_flag() {
    let state = test_state();
    let created = define(&state, json!({ "name": "inferred-agent", "model": "gemini-2.5-pro" })).await.unwrap();
    let def = state.mstore.agent_def_get(&created.definition_id).unwrap().unwrap();
    assert_eq!(def.provider, "gemini");
    assert_eq!(def.provider_flags, "--model gemini-2.5-pro");
}

#[tokio::test]
async fn a_vendor_url_the_provider_cannot_take_is_refused_on_a_fresh_definition() {
    let state = test_state();
    let err = define(
        &state,
        json!({ "name": "codex-proxy-agent", "provider": "codex", "model_vendor_base_url": "https://proxy.example.com" }),
    )
    .await
    .unwrap_err();
    assert!(err.contains("does not support a custom model vendor base URL"), "{err}");
    assert!(!names(&state).contains(&"codex-proxy-agent".to_string()));
}

#[tokio::test]
async fn a_fresh_definition_is_created_with_its_fields_and_an_instance_stub() {
    let state = test_state();
    let created = define(
        &state,
        json!({ "name": "fresh-agent", "provider": "claude", "icon": "star", "description": "d", "working_directory": "/w", "shell": "bash", "environment": "A=1" }),
    )
    .await
    .unwrap();
    assert_eq!(created.action, "created");
    let def = state.mstore.agent_def_get(&created.definition_id).unwrap().unwrap();
    assert_eq!(def.slug, created.slug);
    assert_eq!(
        (def.name.as_str(), def.icon.as_str(), def.description.as_str(), def.working_directory.as_str(), def.shell.as_str(), def.environment.as_str()),
        ("fresh-agent", "star", "d", "/w", "bash", "A=1")
    );
    assert_eq!(created.instance_stub_id, Some(created.definition_id.clone()), "the stub is the definition's own agent row");
    let stub = state.mstore.instance_get(&created.definition_id).unwrap().unwrap();
    assert_eq!(stub.instance_name, "fresh-agent");
}

#[tokio::test]
async fn create_instance_stub_false_creates_no_stub() {
    let state = test_state();
    let created = define(&state, json!({ "name": "no-stub-agent", "create_instance_stub": false })).await.unwrap();
    assert_eq!(created.action, "created");
    assert_eq!(created.instance_stub_id, None);
    assert!(stub_name(&state, &created.definition_id).is_empty());
}

#[tokio::test]
async fn if_exists_error_refuses_a_second_definition_of_the_same_name() {
    let state = test_state();
    define(&state, json!({ "name": "dup-agent" })).await.unwrap();
    let err = define(&state, json!({ "name": "dup-agent", "if_exists": "error" })).await.unwrap_err();
    assert_eq!(err, "agent.define: definition 'dup-agent' already exists (if_exists=error)");
}

#[tokio::test]
async fn skip_adds_a_missing_stub_when_asked_and_not_when_it_is_not() {
    let state = test_state();
    let first = define(&state, json!({ "name": "stub-later-agent", "create_instance_stub": false })).await.unwrap();
    assert_eq!(first.instance_stub_id, None);

    let no_stub = define(&state, json!({ "name": "stub-later-agent", "create_instance_stub": false })).await.unwrap();
    assert_eq!((no_stub.action.as_str(), no_stub.instance_stub_id.clone()), ("skipped", None));
    assert!(stub_name(&state, &first.definition_id).is_empty());

    let with_stub = define(&state, json!({ "name": "stub-later-agent" })).await.unwrap();
    assert_eq!(with_stub.action, "skipped");
    assert_eq!(with_stub.definition_id, first.definition_id);
    assert!(with_stub.instance_stub_id.is_some());
    assert_eq!(stub_name(&state, &first.definition_id), "stub-later-agent");
}

#[tokio::test]
async fn skip_leaves_the_existing_definition_untouched() {
    let state = test_state();
    let first = define(&state, json!({ "name": "keep-agent", "description": "original" })).await.unwrap();
    let again = define(&state, json!({ "name": "keep-agent", "description": "changed" })).await.unwrap();
    assert_eq!(again.action, "skipped");
    assert_eq!(state.mstore.agent_def_get(&first.definition_id).unwrap().unwrap().description, "original");
}

#[tokio::test]
async fn update_overwrites_only_the_fields_that_are_given() {
    let state = test_state();
    let created = define(
        &state,
        json!({ "name": "update-agent", "provider": "claude", "icon": "old", "description": "old", "working_directory": "/old", "shell": "sh", "environment": "A=1" }),
    )
    .await
    .unwrap();

    let updated = define(
        &state,
        json!({ "name": "update-agent", "if_exists": "update", "icon": "new", "working_directory": "/new", "environment": "B=2" }),
    )
    .await
    .unwrap();
    assert_eq!(updated.action, "updated");
    assert_eq!((updated.definition_id.as_str(), updated.slug.as_str()), (created.definition_id.as_str(), created.slug.as_str()));
    let def = state.mstore.agent_def_get(&created.definition_id).unwrap().unwrap();
    assert_eq!(
        (def.icon.as_str(), def.description.as_str(), def.working_directory.as_str(), def.shell.as_str(), def.environment.as_str()),
        ("new", "old", "/new", "sh", "B=2")
    );
}

#[tokio::test]
async fn update_sets_the_model_flag_and_a_provider_without_a_model_clears_it() {
    let state = test_state();
    let created = define(&state, json!({ "name": "flag-agent", "provider": "claude", "model": "claude-opus-4" })).await.unwrap();
    let flags = |state: &AppState| state.mstore.agent_def_get(&created.definition_id).unwrap().unwrap().provider_flags;
    assert_eq!(flags(&state), "--model claude-opus-4");

    define(&state, json!({ "name": "flag-agent", "if_exists": "update", "model": "claude-sonnet-4" })).await.unwrap();
    assert_eq!(flags(&state), "--model claude-sonnet-4");

    define(&state, json!({ "name": "flag-agent", "if_exists": "update", "description": "x" })).await.unwrap();
    assert_eq!(flags(&state), "--model claude-sonnet-4", "no provider and no model leaves the flag alone");

    define(&state, json!({ "name": "flag-agent", "if_exists": "update", "provider": "claude" })).await.unwrap();
    assert_eq!(flags(&state), "", "naming the provider without a model clears the old flag");
}

#[tokio::test]
async fn update_refuses_a_vendor_url_the_existing_provider_cannot_take() {
    let state = test_state();
    define(&state, json!({ "name": "codex-update-agent", "provider": "codex" })).await.unwrap();
    let err = define(
        &state,
        json!({ "name": "codex-update-agent", "if_exists": "update", "model_vendor_base_url": "https://proxy.example.com" }),
    )
    .await
    .unwrap_err();
    assert!(err.contains("does not support a custom model vendor base URL"), "{err}");
}

#[tokio::test]
async fn update_can_add_the_stub_a_definition_lacks() {
    let state = test_state();
    let created = define(&state, json!({ "name": "late-stub-agent", "create_instance_stub": false })).await.unwrap();
    let updated = define(&state, json!({ "name": "late-stub-agent", "if_exists": "update" })).await.unwrap();
    assert_eq!(updated.action, "updated");
    assert!(updated.instance_stub_id.is_some());
    assert_eq!(stub_name(&state, &created.definition_id), "late-stub-agent");
}

#[tokio::test]
async fn a_reserved_name_is_refused_for_a_new_definition_and_an_existing_one_is_left_alone() {
    let state = test_state();
    for name in ["operator", "User", "ADMIN", "System", "agentmux", "unproven", "unknown", "window", "host", "github-consumer", "muxreview", "Op3rator"] {
        let err = define(&state, json!({ "name": name })).await.unwrap_err();
        assert!(err.starts_with("agent.define: the name") && err.contains("reserved") && err.contains("nickname"), "{name}: {err}");
        assert!(!names(&state).contains(&name.to_string()), "{name} was made");
    }
    assert!(define(&state, json!({ "name": "Winny" })).await.is_ok());
    // One that has the name already (from before the rule) is not refused when it is defined again.
    let mut old = crate::backend::storage::agents::test_agent_def("old-admin", "Admin", "claude", "agent", 1, "");
    old.slug = "admin".to_string();
    state.mstore.agent_def_insert(&mut old).unwrap();
    assert!(define(&state, json!({ "name": "Admin" })).await.is_ok());
}
