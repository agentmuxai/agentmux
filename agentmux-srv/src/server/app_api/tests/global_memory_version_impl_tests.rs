// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// `global_memory_version_impl_tests`, moved out of server/app_api/mod.rs unchanged
// (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4, item 9).

use super::*;

fn system_bundle(id: &str) -> Bundle {
    Bundle {
        id: id.to_string(),
        name: format!("System ({id})"),
        description: String::new(),
        is_blank: false,
        is_global: true,
        provider: String::new(),
        model: String::new(),
        instructions: "system content".to_string(),
        instructions_by_provider: "{}".to_string(),
        context_files: "[]".to_string(),
        mcp_servers: "[]".to_string(),
        skills: "[]".to_string(),
        sort_order: 0,
        is_system: true,
        created_at: 0,
        updated_at: 0,
    }
}

#[tokio::test]
async fn history_lists_versions_newest_first() {
    let state = crate::server::tests::test_state();
    let created = global_memory_write_impl(&state, "agent-1", "", None, "V1", "content-1", None).unwrap();
    let id = created.get("id").and_then(|v| v.as_str()).unwrap().to_string();
    global_memory_write_impl(&state, "agent-2", "", Some(&id), "V1", "content-2", None).unwrap();

    let history = global_memory_history_impl(&state, &id).unwrap();
    let versions = history.get("versions").and_then(|v| v.as_array()).unwrap();
    assert_eq!(versions.len(), 2);
    assert_eq!(versions[0].get("written_by").and_then(|v| v.as_str()), Some("agent-2"), "newest first");
    assert_eq!(versions[1].get("written_by").and_then(|v| v.as_str()), Some("agent-1"));
    // Summary shape only — no name/instructions.
    assert!(versions[0].get("name").is_none());
    assert!(versions[0].get("instructions").is_none());
}

/// Hard invariant, SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md,
/// re-affirmed by SPEC_AGENT_FACING_GLOBAL_MEMORY_API_2026_09_15.md: none
/// of history/diff/revert may ever surface a system-tier bundle's
/// content, the same way write/read/remove already refuse one.
#[tokio::test]
async fn history_refuses_a_system_entry() {
    let state = crate::server::tests::test_state();
    state.id_store.bundle_upsert_system(&system_bundle("sys-history")).unwrap();
    state.id_store.bundle_version_insert("sys-history", "System (sys-history)", "system content", "human", "{}", "armory-ui").unwrap();

    let err = global_memory_history_impl(&state, "sys-history").unwrap_err();
    assert!(err.contains("system"), "unexpected error: {err}");
}

#[tokio::test]
async fn history_refuses_a_non_global_bundle() {
    let state = crate::server::tests::test_state();
    let mut private = system_bundle("private-preset");
    private.is_system = false;
    private.is_global = false;
    state.id_store.bundle_upsert(&private).unwrap();

    let err = global_memory_history_impl(&state, "private-preset").unwrap_err();
    assert!(err.contains("not a Global Memory entry"), "unexpected error: {err}");
}

#[tokio::test]
async fn diff_shows_line_changes_between_two_versions() {
    let state = crate::server::tests::test_state();
    let created = global_memory_write_impl(&state, "agent-1", "", None, "V1", "line one\nline two", None).unwrap();
    let id = created.get("id").and_then(|v| v.as_str()).unwrap().to_string();
    global_memory_write_impl(&state, "agent-1", "", Some(&id), "V1", "line one\nline three", None).unwrap();

    let history = global_memory_history_impl(&state, &id).unwrap();
    let versions = history.get("versions").and_then(|v| v.as_array()).unwrap();
    let newest = versions[0].get("id").and_then(|v| v.as_str()).unwrap();
    let oldest = versions[1].get("id").and_then(|v| v.as_str()).unwrap();

    let diff = global_memory_diff_impl(&state, &id, oldest, newest).unwrap();
    let diff_str = diff.get("diff").and_then(|v| v.as_str()).unwrap();
    assert!(diff_str.contains("- line two"), "diff was: {diff_str}");
    assert!(diff_str.contains("+ line three"), "diff was: {diff_str}");
}

/// A rename with byte-identical body still changes `content_hash`
/// (`bundle_versions.rs`'s own `content_hash` doc comment) — the diff
/// must surface that, not silently report "no differences".
#[tokio::test]
async fn diff_surfaces_a_name_only_change() {
    let state = crate::server::tests::test_state();
    let created = global_memory_write_impl(&state, "agent-1", "", None, "Old Name", "same body", None).unwrap();
    let id = created.get("id").and_then(|v| v.as_str()).unwrap().to_string();
    global_memory_write_impl(&state, "agent-1", "", Some(&id), "New Name", "same body", None).unwrap();

    let history = global_memory_history_impl(&state, &id).unwrap();
    let versions = history.get("versions").and_then(|v| v.as_array()).unwrap();
    let newest = versions[0].get("id").and_then(|v| v.as_str()).unwrap();
    let oldest = versions[1].get("id").and_then(|v| v.as_str()).unwrap();

    let diff = global_memory_diff_impl(&state, &id, oldest, newest).unwrap();
    let diff_str = diff.get("diff").and_then(|v| v.as_str()).unwrap();
    assert!(diff_str.contains("- name: Old Name"), "diff was: {diff_str}");
    assert!(diff_str.contains("+ name: New Name"), "diff was: {diff_str}");
}

/// reagent-P1-equivalent regression for Global Memory: a version id alone
/// does not prove which bundle it belongs to, so diff must refuse to
/// compare versions from two different bundles (mirrors
/// `memory_diff_impl`'s identical ownership check for native memory).
#[tokio::test]
async fn diff_refuses_versions_from_different_bundles() {
    let state = crate::server::tests::test_state();
    let a = global_memory_write_impl(&state, "agent-1", "", None, "A", "content-a", None).unwrap();
    let a_id = a.get("id").and_then(|v| v.as_str()).unwrap().to_string();
    let b = global_memory_write_impl(&state, "agent-1", "", None, "B", "content-b", None).unwrap();
    let b_id = b.get("id").and_then(|v| v.as_str()).unwrap().to_string();

    let a_version = global_memory_history_impl(&state, &a_id).unwrap();
    let a_version_id = a_version["versions"][0]["id"].as_str().unwrap().to_string();
    let b_version = global_memory_history_impl(&state, &b_id).unwrap();
    let b_version_id = b_version["versions"][0]["id"].as_str().unwrap().to_string();

    let err = global_memory_diff_impl(&state, &a_id, &a_version_id, &b_version_id).unwrap_err();
    assert!(err.contains("do not belong to"), "unexpected error: {err}");
}

#[tokio::test]
async fn diff_refuses_a_system_entry() {
    let state = crate::server::tests::test_state();
    state.id_store.bundle_upsert_system(&system_bundle("sys-diff")).unwrap();
    let v = state.id_store.bundle_version_insert("sys-diff", "System (sys-diff)", "system content", "human", "{}", "armory-ui").unwrap();

    let err = global_memory_diff_impl(&state, "sys-diff", &v.id, &v.id).unwrap_err();
    assert!(err.contains("system"), "unexpected error: {err}");
}

#[tokio::test]
async fn revert_restores_content_and_records_a_new_version() {
    let state = crate::server::tests::test_state();
    let created = global_memory_write_impl(&state, "agent-1", "", None, "V1", "original", None).unwrap();
    let id = created.get("id").and_then(|v| v.as_str()).unwrap().to_string();
    global_memory_write_impl(&state, "agent-1", "", Some(&id), "V1", "overwritten", None).unwrap();

    let history_before = global_memory_history_impl(&state, &id).unwrap();
    let good_version_id = history_before["versions"][1]["id"].as_str().unwrap().to_string();

    let result = global_memory_revert_impl(&state, "agent-revert", "", &id, &good_version_id).unwrap();
    assert_eq!(result["version"]["source"].as_str(), Some("revert"));
    assert_eq!(result["version"]["written_by"].as_str(), Some("agent-revert"));

    let live = state.id_store.bundle_get(&id).unwrap().unwrap();
    assert_eq!(live.instructions, "original", "live content must match the reverted-to version");

    // Revert is a NEW event, not a silent rewrite — three versions now
    // exist (write, write, revert), all still present.
    let history_after = global_memory_history_impl(&state, &id).unwrap();
    let versions_after = history_after.get("versions").and_then(|v| v.as_array()).unwrap();
    assert_eq!(versions_after.len(), 3);
    assert_eq!(versions_after[0].get("source").and_then(|v| v.as_str()), Some("revert"), "newest version is the revert event");
}

#[tokio::test]
async fn revert_refuses_to_touch_a_system_entry() {
    let state = crate::server::tests::test_state();
    state.id_store.bundle_upsert_system(&system_bundle("sys-revert")).unwrap();
    let v = state.id_store.bundle_version_insert("sys-revert", "System (sys-revert)", "system content", "human", "{}", "armory-ui").unwrap();

    let err = global_memory_revert_impl(&state, "agent-1", "", "sys-revert", &v.id).unwrap_err();
    assert!(err.contains("system"), "unexpected error: {err}");

    let unchanged = state.id_store.bundle_get("sys-revert").unwrap().unwrap();
    assert_eq!(unchanged.instructions, "system content", "system entry must be untouched");
}

#[tokio::test]
async fn revert_refuses_a_version_from_a_different_bundle() {
    let state = crate::server::tests::test_state();
    let a = global_memory_write_impl(&state, "agent-1", "", None, "A", "content-a", None).unwrap();
    let a_id = a.get("id").and_then(|v| v.as_str()).unwrap().to_string();
    let b = global_memory_write_impl(&state, "agent-1", "", None, "B", "content-b", None).unwrap();
    let b_id = b.get("id").and_then(|v| v.as_str()).unwrap().to_string();
    let b_version_id = global_memory_history_impl(&state, &b_id).unwrap()["versions"][0]["id"].as_str().unwrap().to_string();

    let err = global_memory_revert_impl(&state, "agent-1", "", &a_id, &b_version_id).unwrap_err();
    assert!(err.contains("does not belong to"), "unexpected error: {err}");

    let unchanged = state.id_store.bundle_get(&a_id).unwrap().unwrap();
    assert_eq!(unchanged.instructions, "content-a", "must not have been touched");
}
