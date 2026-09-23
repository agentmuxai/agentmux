// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use super::super::*;

fn memory(id: &str, provider: &str, model: &str) -> Bundle {
    Bundle {
        id: id.to_string(),
        name: "T".to_string(),
        description: String::new(),
        is_blank: false,
        is_global: false,
        provider: provider.to_string(),
        model: model.to_string(),
        instructions: String::new(),
        instructions_by_provider: "{}".to_string(),
        context_files: "[]".to_string(),
        mcp_servers: "[]".to_string(),
        skills: "[]".to_string(),
        sort_order: 0,
        created_at: 0,
        updated_at: 0,
        is_system: false,
    }
}

#[test]
fn allows_a_fresh_insert_to_set_provider_and_model() {
    let incoming = memory("b1", "claude", "anthropic");
    assert!(check_provider_model_immutable(None, &incoming).is_ok());
}

#[test]
fn allows_setting_provider_and_model_the_first_time_on_an_existing_empty_row() {
    // Legacy row awaiting backfill, or a bundle created before this
    // field existed — first write that populates them must succeed.
    let existing = memory("b1", "", "");
    let incoming = memory("b1", "claude", "anthropic");
    assert!(check_provider_model_immutable(Some(&existing), &incoming).is_ok());
}

#[test]
fn allows_an_unrelated_field_edit_that_resends_the_same_provider_and_model() {
    let existing = memory("b1", "claude", "anthropic");
    let incoming = memory("b1", "claude", "anthropic");
    assert!(check_provider_model_immutable(Some(&existing), &incoming).is_ok());
}

#[test]
fn rejects_changing_provider_once_set() {
    let existing = memory("b1", "claude", "anthropic");
    let incoming = memory("b1", "codex", "anthropic");
    let err = check_provider_model_immutable(Some(&existing), &incoming).unwrap_err();
    assert!(err.contains("FORBIDDEN"));
    assert!(err.contains("provider"));
}

#[test]
fn rejects_changing_model_once_set() {
    let existing = memory("b1", "claude", "anthropic");
    let incoming = memory("b1", "claude", "custom");
    let err = check_provider_model_immutable(Some(&existing), &incoming).unwrap_err();
    assert!(err.contains("FORBIDDEN"));
    assert!(err.contains("model"));
}
