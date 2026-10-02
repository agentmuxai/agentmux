// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// `global_memory_impl_tests`, moved out of server/app_api/mod.rs unchanged
// (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4, item 9).

use super::*;

// Name derived from `id`, not a fixed literal — db_bundles.name is
// globally UNIQUE (migrations.rs), so two fixtures in the same test
// (e.g. an ordinary entry alongside a system one) would otherwise
// collide on insert.
fn ordinary_bundle(id: &str, is_global: bool) -> Bundle {
    Bundle {
        id: id.to_string(),
        name: format!("Existing ({id})"),
        description: String::new(),
        is_blank: false,
        is_global,
        provider: String::new(),
        model: String::new(),
        instructions: "original content".to_string(),
        instructions_by_provider: "{}".to_string(),
        context_files: "[]".to_string(),
        mcp_servers: "[]".to_string(),
        skills: "[]".to_string(),
        sort_order: 0,
        is_system: false,
        created_at: 0,
        updated_at: 0,
    }
}

/// A slug caller naming an agent that exists only in the registry (no
/// local row) is refused unless the agent's own spawn record proves its
/// directory — the registry's `identity_id` is never used to find it
/// (SPEC_MEMORY_FOLLOWS_THE_AGENT_2026_09_24.md §2.1.2).
#[tokio::test]
async fn memory_write_for_a_registry_only_agent_without_a_spawn_is_refused() {
    let _guard = crate::test_support::ISOLATED_AUTH_ENV_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let prev = std::env::var_os("AGENTMUX_SHARED_DIR");
    let tmp = tempfile::tempdir().unwrap();
    std::env::set_var("AGENTMUX_SHARED_DIR", tmp.path());
    let registry = crate::registry::Registry::open(tmp.path().join("agents").join("registry")).unwrap();
    registry
        .upsert(&crate::registry::NamedAgentRecord {
            schema_version: 1,
            data: crate::registry::NamedAgentRecordV1 {
                instance_id: "inst-reg-only".to_string(),
                instance_name: "RegOnly".to_string(),
                definition_id: "def-reg-only-no-spawn".to_string(),
                identity_id: Some("stale-account".to_string()),
                memory_id: None,
                session_id: None,
                working_dir: "reg-only".to_string(),
                source_agents_base: Some(tmp.path().join("agents").to_string_lossy().to_string()),
                created_at_ms: 1,
                last_launched_at_ms: 1,
                created_by_version: "test".to_string(),
                last_launched_by_version: "test".to_string(),
            },
        })
        .unwrap();

    let state = crate::server::tests::test_state();
    let err = memory_write_impl(&state, "regonly", "MEMORY.md", "x", None).unwrap_err();
    match prev {
        Some(v) => std::env::set_var("AGENTMUX_SHARED_DIR", v),
        None => std::env::remove_var("AGENTMUX_SHARED_DIR"),
    }
    assert!(err.contains("no local row and no spawn on record"), "{err}");
    assert!(!tmp.path().join("identities").join("stale-account").exists(), "nothing written under the stale account");
}

#[tokio::test]
async fn write_new_entry_creates_an_ordinary_global_bundle() {
    let state = crate::server::tests::test_state();
    let result = global_memory_write_impl(&state, "agent-1", "", None, "New Entry", "hello", None).unwrap();
    let id = result.get("id").and_then(|v| v.as_str()).unwrap();
    let saved = state.id_store.bundle_get(id).unwrap().unwrap();
    assert!(saved.is_global);
    assert!(!saved.is_system);
    assert_eq!(saved.name, "New Entry");
    assert_eq!(saved.instructions, "hello");
}

/// reagent P0, PR #3237: `{id: "blank", ...}` must not be able to
/// rename/re-describe the seeded blank Bundle singleton — the exact bug
/// class already fixed once for `upsertmemory`
/// (agent_handlers/bundle.rs, 2026-05-08).
#[tokio::test]
async fn write_refuses_to_hijack_the_blank_singleton() {
    let state = crate::server::tests::test_state();
    state.id_store.bundle_upsert(&ordinary_bundle("blank", false)).unwrap();

    let err = global_memory_write_impl(&state, "agent-1", "", Some("blank"), "evil", "evil content", None)
        .unwrap_err();
    assert!(err.contains("blank"), "error should mention the blank singleton: {err}");

    let unchanged = state.id_store.bundle_get("blank").unwrap().unwrap();
    assert_eq!(unchanged.name, "Existing (blank)");
    assert_eq!(unchanged.instructions, "original content");
}

/// reagent P0, PR #3237: bundle ids are enumerable by any agent via
/// PresetList (unfiltered — every bundle, not just global ones). Without
/// this guard, an agent could target another agent's private,
/// non-global preset by id and hijack it into Global Memory, renaming
/// and replacing its content in the process.
#[tokio::test]
async fn write_refuses_to_hijack_an_existing_non_global_bundle() {
    let state = crate::server::tests::test_state();
    state.id_store.bundle_upsert(&ordinary_bundle("someone-elses-preset", false)).unwrap();

    let err = global_memory_write_impl(
        &state,
        "agent-1",
        "",
        Some("someone-elses-preset"),
        "hijacked",
        "hijacked content",
        None,
    )
    .unwrap_err();
    assert!(err.contains("not a Global Memory entry"), "unexpected error: {err}");

    let unchanged = state.id_store.bundle_get("someone-elses-preset").unwrap().unwrap();
    assert!(!unchanged.is_global, "must not have been promoted to global");
    assert_eq!(unchanged.name, "Existing (someone-elses-preset)");
    assert_eq!(unchanged.instructions, "original content");
}

#[tokio::test]
async fn write_can_edit_an_existing_global_entry() {
    let state = crate::server::tests::test_state();
    state.id_store.bundle_upsert(&ordinary_bundle("already-global", true)).unwrap();

    global_memory_write_impl(&state, "agent-1", "", Some("already-global"), "Renamed", "new content", None)
        .unwrap();

    let updated = state.id_store.bundle_get("already-global").unwrap().unwrap();
    assert_eq!(updated.name, "Renamed");
    assert_eq!(updated.instructions, "new content");
    assert!(updated.is_global);
}

#[tokio::test]
async fn write_refuses_to_touch_a_system_entry() {
    let state = crate::server::tests::test_state();
    let mut sys = ordinary_bundle("sys-1", true);
    sys.is_system = true;
    state.id_store.bundle_upsert_system(&sys).unwrap();

    let err = global_memory_write_impl(&state, "agent-1", "", Some("sys-1"), "hijacked", "x", None).unwrap_err();
    assert!(err.contains("system"), "unexpected error: {err}");
}

/// codex P2, PR #3237: `written_by` must be the TRUSTED `agent_id` this
/// function received, never whatever the caller's own `source` claims
/// (an agent could otherwise falsely label its own write `source:
/// "human"`).
#[tokio::test]
async fn write_records_a_version_with_the_trusted_agent_id_as_written_by() {
    let state = crate::server::tests::test_state();
    let result = global_memory_write_impl(
        &state,
        "the-real-writer",
        "",
        None,
        "Versioned",
        "v1",
        Some(GlobalMemoryWriteProvenance { source: "human", detail: "{}" }),
    )
    .unwrap();
    let id = result.get("id").and_then(|v| v.as_str()).unwrap().to_string();

    let history = state.id_store.bundle_version_list(&id).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].source, "human", "caller-supplied annotation, kept as-is");
    assert_eq!(history[0].written_by, "the-real-writer", "trusted identity, not the caller's own claim");
}

/// codex P2, PR #3237: editing an existing Global Memory entry must
/// also produce exactly one new version, chained onto the entry's
/// creation version.
#[tokio::test]
async fn editing_an_existing_entry_chains_a_second_version() {
    let state = crate::server::tests::test_state();
    let created = global_memory_write_impl(&state, "agent-1", "", None, "V1", "content-1", None).unwrap();
    let id = created.get("id").and_then(|v| v.as_str()).unwrap().to_string();

    global_memory_write_impl(&state, "agent-2", "", Some(&id), "V1", "content-2", None).unwrap();

    let history = state.id_store.bundle_version_list(&id).unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0].written_by, "agent-2", "newest first");
    assert_eq!(history[1].written_by, "agent-1");
    assert_eq!(history[0].parent_version_id.as_deref(), Some(history[1].id.as_str()));
}

#[tokio::test]
async fn list_includes_system_entries_flagged_and_first() {
    let state = crate::server::tests::test_state();
    state.id_store.bundle_upsert(&ordinary_bundle("ordinary-1", true)).unwrap();
    let mut sys = ordinary_bundle("sys-2", true);
    sys.is_system = true;
    state.id_store.bundle_upsert_system(&sys).unwrap();

    let result = global_memory_list_impl(&state).unwrap();
    let entries = result.get("entries").and_then(|v| v.as_array()).unwrap();
    assert_eq!(entries.len(), 2);
    // System rows sort first (bundle_list_global's order) and say so.
    assert_eq!(entries[0].get("id").and_then(|v| v.as_str()), Some("sys-2"));
    assert_eq!(entries[0].get("system").and_then(|v| v.as_bool()), Some(true));
    assert_eq!(entries[1].get("id").and_then(|v| v.as_str()), Some("ordinary-1"));
    assert_eq!(entries[1].get("system").and_then(|v| v.as_bool()), Some(false));
    // Listing is not access: the content never leaves, and every mutation path
    // still refuses a system id.
    assert!(entries[0].get("content").is_none() && entries[0].get("instructions").is_none());
    assert!(global_memory_read_impl(&state, "sys-2").is_err());
    assert!(global_memory_remove_impl(&state, "sys-2").is_err());
    assert!(global_memory_write_impl(&state, "agent-1", "", Some("sys-2"), "x", "y", None).is_err());
}

#[tokio::test]
async fn read_refuses_a_system_entry() {
    let state = crate::server::tests::test_state();
    let mut sys = ordinary_bundle("sys-3", true);
    sys.is_system = true;
    state.id_store.bundle_upsert_system(&sys).unwrap();

    let err = global_memory_read_impl(&state, "sys-3").unwrap_err();
    assert!(err.contains("system"), "unexpected error: {err}");
}

#[tokio::test]
async fn remove_demotes_but_does_not_delete() {
    let state = crate::server::tests::test_state();
    state.id_store.bundle_upsert(&ordinary_bundle("to-remove", true)).unwrap();

    global_memory_remove_impl(&state, "to-remove").unwrap();

    let after = state.id_store.bundle_get("to-remove").unwrap().unwrap();
    assert!(!after.is_global);
}

/// reagent P1, PR #3237 (re-review): remove had the SAME missing-
/// ownership-check bug already fixed for write, left unfixed here.
#[tokio::test]
async fn remove_refuses_to_hijack_the_blank_singleton() {
    let state = crate::server::tests::test_state();
    state.id_store.bundle_upsert(&ordinary_bundle("blank", false)).unwrap();

    let err = global_memory_remove_impl(&state, "blank").unwrap_err();
    assert!(err.contains("blank"), "error should mention the blank singleton: {err}");

    let unchanged = state.id_store.bundle_get("blank").unwrap().unwrap();
    assert_eq!(unchanged.updated_at, 0, "must not have been touched at all");
}

/// reagent P1, PR #3237 (re-review): an agent could call
/// GlobalMemoryRemove with another agent's private preset id
/// (discovered via PresetList) — is_global was already false so the
/// flag write was a no-op, but bundle_upsert still rewrote the row and
/// bumped updated_at on a bundle this API has no business touching.
#[tokio::test]
async fn remove_refuses_to_touch_a_non_global_bundle() {
    let state = crate::server::tests::test_state();
    state.id_store.bundle_upsert(&ordinary_bundle("someone-elses-preset", false)).unwrap();

    let err = global_memory_remove_impl(&state, "someone-elses-preset").unwrap_err();
    assert!(err.contains("not a Global Memory entry"), "unexpected error: {err}");

    let unchanged = state.id_store.bundle_get("someone-elses-preset").unwrap().unwrap();
    assert_eq!(unchanged.updated_at, 0, "must not have been touched at all");
}
