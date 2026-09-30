// Copyright 2024-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// `memory_version_impl_tests`, moved out of server/app_api/mod.rs unchanged
// (SPEC_LARGE_FILE_MODULE_ANALYSIS_2026_09_30.md §4, item 9).

use super::*;

fn agent_def(id: &str, working_directory: &str) -> crate::backend::storage::AgentDefinition {
    crate::backend::storage::AgentDefinition {
        conversation_visibility: crate::backend::storage::agents::default_conversation_visibility(),
        id: id.to_string(),
        slug: id.to_string(),
        name: "Test Agent".to_string(),
        icon: String::new(),
        provider: "claude".to_string(),
        description: String::new(),
        working_directory: working_directory.to_string(),
        shell: String::new(),
        provider_flags: String::new(),
        auto_start: 0,
        restart_on_crash: 0,
        idle_timeout_minutes: 0,
        created_at: 0,
        agent_type: "host".to_string(),
        environment: String::new(),
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
        auto_continue_enabled: 0,
        model_vendor_base_url: String::new(),
        memory_id: String::new(),
    }
}

/// `test_state()` sets `id_store: mstore.clone()`, so both are the same
/// in-memory `Store` (`run_object_schema`) — good enough for these
/// wiring-level tests, since the version-chain logic itself is already
/// covered by `native_memory_handlers.rs`'s tests against the same
/// `Store` methods.
///
/// Sets `CLAUDE_CONFIG_DIR` to the given temp dir explicitly — an empty
/// value would make `memory_dir_for_agent` fall back to the REAL
/// `~/.agentmux/shared/providers/claude/`, writing test fixtures into
/// the developer's actual home directory (the same trap
/// `native_memory_handlers.rs`'s own tests document having hit before).
fn state_with_agent(agent_id: &str, working_directory: &std::path::Path) -> AppState {
    let state = crate::server::tests::test_state();
    let mut def = agent_def(agent_id, &working_directory.to_string_lossy());
    state.mstore.agent_def_insert(&mut def).unwrap();
    state
        .mstore
        .agent_content_set(&crate::backend::storage::AgentContent {
            agent_id: agent_id.to_string(),
            content_type: "env".to_string(),
            content: format!("CLAUDE_CONFIG_DIR={}\n", working_directory.display()),
            updated_at: 0,
        })
        .unwrap();
    state
}

#[tokio::test]
async fn write_then_history_records_a_version_with_default_provenance() {
    let tmp = tempfile::tempdir().unwrap();
    let state = state_with_agent("agent-app-1", tmp.path());

    memory_write_impl(&state, "agent-app-1", "MEMORY.md", "hello", None).unwrap();

    let history = memory_history_impl(&state, "agent-app-1", "MEMORY.md").unwrap();
    let versions = history.get("versions").and_then(|v| v.as_array()).unwrap();
    assert_eq!(versions.len(), 1);
    assert_eq!(versions[0].get("source").and_then(|v| v.as_str()), Some("agent_inferred"));
}

// SPEC_ARMORY_REACTIVE_UPDATES_2026_09_02.md: this is the App API surface
// the `MemoryWrite` MCP tool actually calls — proves the publish keys by
// the resolved canonical UUID (`version_agent_id`), not the raw `agent_id`
// slug parameter this function otherwise takes throughout. In this
// fixture slug == id (agent_def's own convention), so this alone
// wouldn't catch a slug/UUID mixup; see the sibling test below for that.
#[tokio::test]
async fn write_publishes_agent_memory_changed_scoped_to_the_canonical_id() {
    let tmp = tempfile::tempdir().unwrap();
    let mut state = state_with_agent("agent-app-pub", tmp.path());
    let (broker, client) = crate::test_support::broker_recording("agent:memory:changed:agent-app-pub");
    state.broker = std::sync::Arc::new(broker);

    memory_write_impl(&state, "agent-app-pub", "MEMORY.md", "hello", None).unwrap();

    let events = client.received_events();
    assert_eq!(events.len(), 1, "memory_write_impl must publish exactly one agent:memory:changed event");
    assert_eq!(events[0].1.event, "agent:memory:changed:agent-app-pub");
}

// The mixup this test exists to catch: memory_write_impl's own doc
// comment establishes that App-API callers pass a SLUG, which can differ
// from the agent's canonical UUID id. If the publish were still keyed by
// the raw `agent_id` parameter (the bug this spec's own commit fixed),
// this test's event name assertion would read the slug instead of the
// UUID and fail here specifically — a fixture where they genuinely
// differ, unlike every other test in this module.
#[tokio::test]
async fn write_publishes_using_the_resolved_uuid_not_the_raw_slug() {
    let tmp = tempfile::tempdir().unwrap();
    let mut state = crate::server::tests::test_state();
    let mut def = agent_def("agent-real-uuid-pub", &tmp.path().to_string_lossy());
    def.slug = "agent-friendly-slug-pub".to_string();
    state.mstore.agent_def_insert(&mut def).unwrap();
    state
        .mstore
        .agent_content_set(&crate::backend::storage::AgentContent {
            agent_id: def.id.clone(),
            content_type: "env".to_string(),
            content: format!("CLAUDE_CONFIG_DIR={}\n", tmp.path().display()),
            updated_at: 0,
        })
        .unwrap();
    let (broker, client) = crate::test_support::broker_recording("agent:memory:changed:agent-real-uuid-pub");
    state.broker = std::sync::Arc::new(broker);

    // Write via the slug (what the MemoryWrite MCP tool actually sends).
    memory_write_impl(&state, "agent-friendly-slug-pub", "MEMORY.md", "hello", None).unwrap();

    let events = client.received_events();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].1.event,
        "agent:memory:changed:agent-real-uuid-pub",
        "must publish under the resolved canonical UUID, not the raw slug -- \
         a frontend subscriber only ever has AgentDefinition.id (the UUID)"
    );
}

#[tokio::test]
async fn write_honors_explicit_provenance() {
    let tmp = tempfile::tempdir().unwrap();
    let state = state_with_agent("agent-app-2", tmp.path());

    memory_write_impl(
        &state, "agent-app-2", "MEMORY.md", "content",
        Some(MemoryWriteProvenance { source: "jekt", detail: r#"{"TIER":"sensitive"}"# }),
    ).unwrap();

    let history = memory_history_impl(&state, "agent-app-2", "MEMORY.md").unwrap();
    let versions = history.get("versions").and_then(|v| v.as_array()).unwrap();
    assert_eq!(versions[0].get("source").and_then(|v| v.as_str()), Some("jekt"));
}

/// Regression for reagent P1 on PR #2674: memory_write_impl (the
/// App-API path backing the `MemoryWrite` MCP tool) must keep
/// `db_agent_native_memory` in sync, same as its WS-RPC sibling
/// `agent:memory:write_file` already does — otherwise `read_file` in a
/// channel with no live copy of the file falls back to a permanently
/// stale mirror row.
#[tokio::test]
async fn write_updates_the_native_memory_mirror_row() {
    let tmp = tempfile::tempdir().unwrap();
    let state = state_with_agent("agent-app-mirror-1", tmp.path());

    memory_write_impl(&state, "agent-app-mirror-1", "MEMORY.md", "hello mirror", None).unwrap();

    let mirrored = state.id_store.agent_native_memory_read("agent-app-mirror-1", "MEMORY.md").unwrap();
    assert_eq!(mirrored, Some("hello mirror".to_string()));
}

#[tokio::test]
async fn diff_reflects_two_writes() {
    let tmp = tempfile::tempdir().unwrap();
    let state = state_with_agent("agent-app-3", tmp.path());

    memory_write_impl(&state, "agent-app-3", "MEMORY.md", "v1", None).unwrap();
    memory_write_impl(&state, "agent-app-3", "MEMORY.md", "v2", None).unwrap();

    let history = memory_history_impl(&state, "agent-app-3", "MEMORY.md").unwrap();
    let versions = history.get("versions").and_then(|v| v.as_array()).unwrap();
    let newest = versions[0].get("id").and_then(|v| v.as_str()).unwrap();
    let oldest = versions[1].get("id").and_then(|v| v.as_str()).unwrap();

    let diff = memory_diff_impl(&state, "agent-app-3", oldest, newest).unwrap();
    let diff_text = diff.get("diff").and_then(|v| v.as_str()).unwrap();
    assert!(diff_text.contains("- v1"), "unexpected diff: {diff_text}");
    assert!(diff_text.contains("+ v2"), "unexpected diff: {diff_text}");
}

#[tokio::test]
async fn diff_rejects_a_version_from_a_different_agent() {
    let tmp_a = tempfile::tempdir().unwrap();
    let tmp_b = tempfile::tempdir().unwrap();
    let state = crate::server::tests::test_state();
    let mut def_a = agent_def("agent-diff-a", &tmp_a.path().to_string_lossy());
    let mut def_b = agent_def("agent-diff-b", &tmp_b.path().to_string_lossy());
    state.mstore.agent_def_insert(&mut def_a).unwrap();
    state.mstore.agent_def_insert(&mut def_b).unwrap();
    for (id, dir) in [("agent-diff-a", tmp_a.path()), ("agent-diff-b", tmp_b.path())] {
        state
            .mstore
            .agent_content_set(&crate::backend::storage::AgentContent {
                agent_id: id.to_string(),
                content_type: "env".to_string(),
                content: format!("CLAUDE_CONFIG_DIR={}\n", dir.display()),
                updated_at: 0,
            })
            .unwrap();
    }

    memory_write_impl(&state, "agent-diff-a", "MEMORY.md", "v1", None).unwrap();
    memory_write_impl(&state, "agent-diff-a", "MEMORY.md", "v2", None).unwrap();
    let history = memory_history_impl(&state, "agent-diff-a", "MEMORY.md").unwrap();
    let versions = history.get("versions").and_then(|v| v.as_array()).unwrap();
    let newest = versions[0].get("id").and_then(|v| v.as_str()).unwrap().to_string();
    let oldest = versions[1].get("id").and_then(|v| v.as_str()).unwrap().to_string();

    let err = memory_diff_impl(&state, "agent-diff-b", &oldest, &newest).unwrap_err();
    assert!(err.contains("do not belong to"), "unexpected error: {err}");
}

#[tokio::test]
async fn revert_restores_live_content_and_appends_a_new_version() {
    let tmp = tempfile::tempdir().unwrap();
    let state = state_with_agent("agent-app-4", tmp.path());

    memory_write_impl(&state, "agent-app-4", "MEMORY.md", "good", None).unwrap();
    memory_write_impl(&state, "agent-app-4", "MEMORY.md", "fabricated", None).unwrap();

    let history = memory_history_impl(&state, "agent-app-4", "MEMORY.md").unwrap();
    let versions = history.get("versions").and_then(|v| v.as_array()).unwrap();
    let good_id = versions[1].get("id").and_then(|v| v.as_str()).unwrap().to_string();

    let revert = memory_revert_impl(&state, "agent-app-4", "MEMORY.md", &good_id).unwrap();
    assert_eq!(
        revert.get("version").and_then(|v| v.get("source")).and_then(|v| v.as_str()),
        Some("revert")
    );

    let read = memory_read_impl(&state, "agent-app-4", "MEMORY.md").unwrap();
    assert_eq!(read.get("content").and_then(|v| v.as_str()), Some("good"));

    let history_after = memory_history_impl(&state, "agent-app-4", "MEMORY.md").unwrap();
    let versions_after = history_after.get("versions").and_then(|v| v.as_array()).unwrap();
    assert_eq!(versions_after.len(), 3, "revert must not delete or rewrite prior versions");
}

// SPEC_ARMORY_REACTIVE_UPDATES_2026_09_02.md — same rationale as
// write_publishes_agent_memory_changed_scoped_to_the_canonical_id above.
#[tokio::test]
async fn revert_publishes_agent_memory_changed_scoped_to_the_canonical_id() {
    let tmp = tempfile::tempdir().unwrap();
    let mut state = state_with_agent("agent-app-revpub", tmp.path());

    memory_write_impl(&state, "agent-app-revpub", "MEMORY.md", "good", None).unwrap();
    memory_write_impl(&state, "agent-app-revpub", "MEMORY.md", "fabricated", None).unwrap();
    let history = memory_history_impl(&state, "agent-app-revpub", "MEMORY.md").unwrap();
    let versions = history.get("versions").and_then(|v| v.as_array()).unwrap();
    let good_id = versions[1].get("id").and_then(|v| v.as_str()).unwrap().to_string();

    // Only the revert publish is under test — swap in the observed
    // broker after the setup writes above (each of which also
    // published, on the previous default/unobserved broker).
    let (broker, client) = crate::test_support::broker_recording("agent:memory:changed:agent-app-revpub");
    state.broker = std::sync::Arc::new(broker);

    memory_revert_impl(&state, "agent-app-revpub", "MEMORY.md", &good_id).unwrap();

    let events = client.received_events();
    assert_eq!(events.len(), 1, "memory_revert_impl must publish exactly one agent:memory:changed event");
    assert_eq!(events[0].1.event, "agent:memory:changed:agent-app-revpub");
}

/// Regression for reagent P1 on PR #2674: memory_revert_impl (the
/// App-API path backing the `MemoryRevert` MCP tool) reverted the live
/// file but never updated `db_agent_native_memory`, unlike its WS-RPC
/// sibling `agent:memory:revert` — a revert issued through the actual
/// `MemoryRevert` tool silently failed to propagate cross-channel,
/// leaving other channels' `read_file` fallback still showing the
/// pre-revert (fabricated) content indefinitely.
#[tokio::test]
async fn revert_updates_the_native_memory_mirror_row() {
    let tmp = tempfile::tempdir().unwrap();
    let state = state_with_agent("agent-app-mirror-2", tmp.path());

    memory_write_impl(&state, "agent-app-mirror-2", "MEMORY.md", "good", None).unwrap();
    memory_write_impl(&state, "agent-app-mirror-2", "MEMORY.md", "fabricated", None).unwrap();
    let history = memory_history_impl(&state, "agent-app-mirror-2", "MEMORY.md").unwrap();
    let versions = history.get("versions").and_then(|v| v.as_array()).unwrap();
    let good_id = versions[1].get("id").and_then(|v| v.as_str()).unwrap().to_string();

    memory_revert_impl(&state, "agent-app-mirror-2", "MEMORY.md", &good_id).unwrap();

    let mirrored = state.id_store.agent_native_memory_read("agent-app-mirror-2", "MEMORY.md").unwrap();
    assert_eq!(mirrored, Some("good".to_string()), "mirror must reflect the reverted content, not the fabricated one");
}

#[tokio::test]
async fn revert_rejects_a_version_from_a_different_agent() {
    let tmp_a = tempfile::tempdir().unwrap();
    let tmp_b = tempfile::tempdir().unwrap();
    // Same underlying in-memory Store backs both AppState clones here
    // (test_state() re-opens a fresh Store each call) — build one
    // shared state and register two agents on it instead.
    let state = crate::server::tests::test_state();
    let mut def_a = agent_def("agent-app-5a", &tmp_a.path().to_string_lossy());
    let mut def_b = agent_def("agent-app-5b", &tmp_b.path().to_string_lossy());
    state.mstore.agent_def_insert(&mut def_a).unwrap();
    state.mstore.agent_def_insert(&mut def_b).unwrap();
    for (id, dir) in [("agent-app-5a", tmp_a.path()), ("agent-app-5b", tmp_b.path())] {
        state
            .mstore
            .agent_content_set(&crate::backend::storage::AgentContent {
                agent_id: id.to_string(),
                content_type: "env".to_string(),
                content: format!("CLAUDE_CONFIG_DIR={}\n", dir.display()),
                updated_at: 0,
            })
            .unwrap();
    }

    memory_write_impl(&state, "agent-app-5a", "MEMORY.md", "agent a's content", None).unwrap();
    let history = memory_history_impl(&state, "agent-app-5a", "MEMORY.md").unwrap();
    let version_id = history.get("versions").and_then(|v| v.as_array()).unwrap()[0]
        .get("id").and_then(|v| v.as_str()).unwrap().to_string();

    let err = memory_revert_impl(&state, "agent-app-5b", "MEMORY.md", &version_id).unwrap_err();
    assert!(err.contains("does not belong to"), "unexpected error: {err}");
}

/// Regression for reagent P1 (re-review of PR #2674): this App-API
/// surface receives the agent SLUG (per `memory_dir_for_agent`'s own
/// doc), but must key `db_agent_native_memory_versions` by the same
/// canonical `AgentDefinition.id` the WS RPC surface uses — otherwise
/// a version written here is invisible to a WS-RPC-based history/diff/
/// revert call for the same logical agent whenever slug != id. Uses a
/// deliberately DIFFERENT id and slug (existing test fixtures elsewhere
/// in this file always set them equal, so they can't catch this class
/// of bug at all).
#[tokio::test]
async fn write_impl_keys_versions_by_the_resolved_id_not_the_raw_slug() {
    let tmp = tempfile::tempdir().unwrap();
    let state = crate::server::tests::test_state();
    let mut def = agent_def("agent-real-uuid-999", &tmp.path().to_string_lossy());
    def.slug = "agent-friendly-slug".to_string();
    state.mstore.agent_def_insert(&mut def).unwrap();
    state
        .mstore
        .agent_content_set(&crate::backend::storage::AgentContent {
            agent_id: def.id.clone(),
            content_type: "env".to_string(),
            content: format!("CLAUDE_CONFIG_DIR={}\n", tmp.path().display()),
            updated_at: 0,
        })
        .unwrap();

    // Write via the slug (what the MemoryWrite MCP tool actually sends).
    memory_write_impl(&state, "agent-friendly-slug", "MEMORY.md", "content", None).unwrap();

    // The version must be discoverable under the RESOLVED id — the
    // same id a WS-RPC-based history/diff/revert call would use (it
    // receives AgentDefinition.id directly, per the frontend's own
    // contract) — not under the raw slug string.
    let by_resolved_id = state
        .id_store
        .agent_native_memory_version_list("agent-real-uuid-999", "MEMORY.md")
        .unwrap();
    assert_eq!(by_resolved_id.len(), 1, "version must be keyed by the resolved AgentDefinition.id");

    let by_raw_slug = state
        .id_store
        .agent_native_memory_version_list("agent-friendly-slug", "MEMORY.md")
        .unwrap();
    assert_eq!(by_raw_slug.len(), 0, "version must NOT be keyed by the raw, unresolved slug");

    // And memory_history_impl (called with the slug, same as write)
    // must still find it, proving read-your-own-write consistency
    // through this surface's own resolution.
    let history = memory_history_impl(&state, "agent-friendly-slug", "MEMORY.md").unwrap();
    let versions = history.get("versions").and_then(|v| v.as_array()).unwrap();
    assert_eq!(versions.len(), 1);
}

fn add_agent(state: &AppState, id: &str, name: &str, slug: &str, dir: &std::path::Path) {
    let mut def = agent_def(id, &dir.to_string_lossy());
    def.name = name.to_string();
    def.slug = slug.to_string();
    state.mstore.agent_def_insert(&mut def).unwrap();
    state
        .mstore
        .agent_content_set(&crate::backend::storage::AgentContent {
            agent_id: id.to_string(),
            content_type: "env".to_string(),
            content: format!("CLAUDE_CONFIG_DIR={}\n", dir.display()),
            updated_at: 0,
        })
        .unwrap();
}

fn version_count(state: &AppState, owner: &str) -> usize {
    state.id_store.agent_native_memory_version_list(owner, "MEMORY.md").unwrap().len()
}

/// Identity M4c-2b (spec §6.5.9), the colliding-names fixture: the second
/// agent ("AGENTY", `agenty-2`) sends the first one's slug `agenty`. When
/// attributed, its own row is the owner — its directory, its versions,
/// and it cannot read, diff or revert the first agent's; Unattributed,
/// the slug still resolves to the first agent, as before.
#[tokio::test]
async fn an_attributed_memory_owner_is_the_callers_row_not_the_slug() {
    let (tmp_y, tmp_y2) = (tempfile::tempdir().unwrap(), tempfile::tempdir().unwrap());
    let state = crate::server::tests::test_state();
    add_agent(&state, "uid-mo-y", "AgentY", "agenty", tmp_y.path());
    add_agent(&state, "uid-mo-y2", "AGENTY", "agenty-2", tmp_y2.path());
    let as_y2 = SelfOwner::Uid("uid-mo-y2");

    memory_write_impl(&state, "agenty", "MEMORY.md", "y's", None).unwrap();
    memory_write_impl(&state, as_y2, "MEMORY.md", "y2's", None).unwrap();
    assert_eq!(version_count(&state, "uid-mo-y"), 1);
    assert_eq!(version_count(&state, "uid-mo-y2"), 1);

    let read = |owner: SelfOwner<'_>| {
        memory_read_impl(&state, owner, "MEMORY.md").unwrap()["content"]
            .as_str()
            .unwrap()
            .to_string()
    };
    assert_eq!(read(as_y2), "y2's");
    assert_eq!(read("agenty".into()), "y's", "Unattributed: by slug");
    let listed = memory_list_impl(&state, as_y2).unwrap();
    assert_eq!(listed["files"].as_array().unwrap().len(), 1);

    let y_version = state
        .id_store
        .agent_native_memory_version_list("uid-mo-y", "MEMORY.md")
        .unwrap()[0]
        .id
        .clone();
    let err = memory_revert_impl(&state, as_y2, "MEMORY.md", &y_version).unwrap_err();
    assert!(err.contains("does not belong to"), "{err}");
    let err = memory_diff_impl(&state, as_y2, &y_version, &y_version).unwrap_err();
    assert!(err.contains("do not belong to"), "{err}");
    let history = memory_history_impl(&state, as_y2, "MEMORY.md").unwrap();
    assert_eq!(history["versions"].as_array().unwrap().len(), 1);

    // A caller whose row is gone is an error, never another's directory
    // and never an empty list.
    let gone = SelfOwner::Uid("uid-mo-gone");
    for err in [
        memory_list_impl(&state, gone).unwrap_err(),
        memory_history_impl(&state, gone, "MEMORY.md").unwrap_err(),
        memory_diff_impl(&state, gone, &y_version, &y_version).unwrap_err(),
        memory_revert_impl(&state, gone, "MEMORY.md", &y_version).unwrap_err(),
        memory_write_impl(&state, gone, "MEMORY.md", "x", None).unwrap_err(),
    ] {
        assert!(err.contains("not found"), "{err}");
    }
}
