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
    assert!(global_memory_remove_impl(&state, "agent-1", "", "sys-2").is_err());
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

    global_memory_remove_impl(&state, "agent-1", "", "to-remove").unwrap();

    let after = state.id_store.bundle_get("to-remove").unwrap().unwrap();
    assert!(!after.is_global);
}

fn write_entry(state: &AppState, agent: &str, name: &str, content: &str) -> String {
    let wrote = global_memory_write_impl(state, agent, "", None, name, content, None).unwrap();
    wrote["id"].as_str().unwrap().to_string()
}

/// A removal is a version: the content the entry held, written by whoever
/// removed it, so the history shows who and a revert can undo it.
#[tokio::test]
async fn remove_records_a_tombstone_with_the_actor() {
    let state = crate::server::tests::test_state();
    let id = write_entry(&state, "agent-1", "Rules", "the rules");

    let removed = global_memory_remove_impl(&state, "agent-2", "uid-agent-2", &id).unwrap();
    assert_eq!(removed["version"]["source"], "remove");

    let history = state.id_store.bundle_version_list(&id).unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!((history[0].source.as_str(), history[0].written_by.as_str(), history[0].written_by_uid.as_str()), ("remove", "agent-2", "uid-agent-2"));
    assert_eq!(history[0].parent_version_id.as_deref(), Some(history[1].id.as_str()));
    let tombstone = state.id_store.bundle_version_get(&history[0].id).unwrap().unwrap();
    assert_eq!((tombstone.name.as_str(), tombstone.instructions.as_str()), ("Rules", "the rules"));
    assert!(!state.id_store.bundle_get(&id).unwrap().unwrap().is_global);
}

/// A removed entry is listed under `removed` with who removed it, its history
/// and diff still answer, and a revert brings it back as Global Memory, last in
/// the order, recorded under the agent that restored it.
#[tokio::test]
async fn a_removed_entry_can_be_inspected_and_restored() {
    let state = crate::server::tests::test_state();
    let id = write_entry(&state, "agent-1", "Rules", "the rules");
    let other = write_entry(&state, "agent-1", "Other", "other rules");
    let first = state.id_store.bundle_version_list(&id).unwrap()[0].id.clone();
    global_memory_remove_impl(&state, "agent-2", "uid-agent-2", &id).unwrap();

    let list = global_memory_list_impl(&state).unwrap();
    let entry_ids: Vec<&str> = list["entries"].as_array().unwrap().iter().map(|e| e["id"].as_str().unwrap()).collect();
    assert_eq!(entry_ids, [other.as_str()], "a removed entry is no longer Global Memory");
    assert_eq!(list["removed"][0]["id"], id.as_str());
    assert_eq!(list["removed"][0]["removed_by"], "agent-2");

    let history = global_memory_history_impl(&state, &id).unwrap();
    assert_eq!(history["versions"][0]["source"], "remove");
    let tombstone = history["versions"][0]["id"].as_str().unwrap().to_string();
    assert!(global_memory_diff_impl(&state, &id, &first, &tombstone).is_ok());

    let restored = global_memory_revert_impl(&state, "agent-3", "uid-agent-3", &id, &first).unwrap();
    assert_eq!(restored["version"]["source"], "revert");
    assert_eq!(restored["version"]["written_by"], "agent-3");
    let row = state.id_store.bundle_get(&id).unwrap().unwrap();
    assert!(row.is_global);
    assert_eq!(row.instructions, "the rules");
    let after = global_memory_list_impl(&state).unwrap();
    let order: Vec<&str> = after["entries"].as_array().unwrap().iter().map(|e| e["id"].as_str().unwrap()).collect();
    assert_eq!(order, [other.as_str(), id.as_str()], "restored last");
    assert_eq!(after["removed"], serde_json::json!([]));
}

/// Only an entry that was Global Memory can be reached after a removal: a
/// private bundle (no history) stays out of `removed`, `history` and `revert`,
/// and a restored entry leaves `removed`.
#[tokio::test]
async fn a_bundle_that_was_never_global_memory_stays_unreachable() {
    let state = crate::server::tests::test_state();
    state.id_store.bundle_upsert(&ordinary_bundle("private-preset", false)).unwrap();

    assert_eq!(global_memory_list_impl(&state).unwrap()["removed"], serde_json::json!([]));
    let err = global_memory_history_impl(&state, "private-preset").unwrap_err();
    assert!(err.contains("not a Global Memory entry"), "{err}");
    let err = global_memory_revert_impl(&state, "agent-1", "", "private-preset", "any").unwrap_err();
    assert!(err.contains("not a Global Memory entry"), "{err}");
    assert!(!state.id_store.bundle_get("private-preset").unwrap().unwrap().is_global);

    // Restored entries leave `removed` and cannot be "restored" twice from it.
    let id = write_entry(&state, "agent-1", "Rules", "v1");
    let v1 = state.id_store.bundle_version_list(&id).unwrap()[0].id.clone();
    global_memory_remove_impl(&state, "agent-1", "", &id).unwrap();
    global_memory_revert_impl(&state, "agent-1", "", &id, &v1).unwrap();
    assert_eq!(global_memory_list_impl(&state).unwrap()["removed"], serde_json::json!([]));
}

/// The Memory pane's Remove demotes through the versioned upsert; that is a
/// removal too, written by the pane's own writer.
#[tokio::test]
async fn a_pane_removal_leaves_a_tombstone_too() {
    let state = crate::server::tests::test_state();
    let id = write_entry(&state, "agent-1", "Rules", "the rules");
    let mut bundle = state.id_store.bundle_get(&id).unwrap().unwrap();
    bundle.is_global = false;
    state.id_store.bundle_upsert_with_version(&bundle, "armory-ui", "", "human", "{}").unwrap();

    let newest = state.id_store.bundle_version_latest(&id).unwrap().unwrap();
    assert_eq!((newest.source.as_str(), newest.written_by.as_str()), ("remove", "armory-ui"));
    assert_eq!(global_memory_list_impl(&state).unwrap()["removed"][0]["id"], id.as_str());
    // Saving a private bundle that was never global records nothing.
    state.id_store.bundle_upsert_with_version(&ordinary_bundle("plain", false), "armory-ui", "", "human", "{}").unwrap();
    assert!(state.id_store.bundle_version_latest("plain").unwrap().is_none());
}

/// reagent P1, PR #3237 (re-review): remove had the SAME missing-
/// ownership-check bug already fixed for write, left unfixed here.
#[tokio::test]
async fn remove_refuses_to_hijack_the_blank_singleton() {
    let state = crate::server::tests::test_state();
    state.id_store.bundle_upsert(&ordinary_bundle("blank", false)).unwrap();

    let err = global_memory_remove_impl(&state, "agent-1", "", "blank").unwrap_err();
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

    let err = global_memory_remove_impl(&state, "agent-1", "", "someone-elses-preset").unwrap_err();
    assert!(err.contains("not a Global Memory entry"), "unexpected error: {err}");

    let unchanged = state.id_store.bundle_get("someone-elses-preset").unwrap().unwrap();
    assert_eq!(unchanged.updated_at, 0, "must not have been touched at all");
}


// Reach (fleet Global Memory): where an entry goes. The scope table is made at startup, before
// anything is written, so a test makes it first.

fn entry_reach(state: &AppState, id: &str) -> String {
    let list = global_memory_list_impl(state).unwrap();
    let entries = list.get("entries").and_then(|v| v.as_array()).unwrap();
    entries
        .iter()
        .find(|e| e.get("id").and_then(|v| v.as_str()) == Some(id))
        .and_then(|e| e.get("reach").and_then(|v| v.as_str()))
        .unwrap_or_else(|| panic!("no entry {id} in {list}"))
        .to_string()
}

#[tokio::test]
async fn a_new_entry_follows_the_fleet_unless_it_is_written_to_stay_here() {
    let state = crate::server::tests::test_state();
    state.id_store.bundle_scope_ensure().unwrap();

    let shared = global_memory_write_with_reach_impl(&state, "agent-1", "", None, "Shared", "everywhere", None, None).unwrap();
    let shared_id = shared["id"].as_str().unwrap().to_string();
    assert_eq!(entry_reach(&state, &shared_id), "fleet");
    assert!(shared.get("reach").is_none(), "no reach was asked for");

    let local = global_memory_write_with_reach_impl(&state, "agent-1", "", None, "Local", "here only", None, Some("machine")).unwrap();
    assert_eq!(local["reach"], "machine");
    let local_id = local["id"].as_str().unwrap().to_string();
    assert_eq!(entry_reach(&state, &local_id), "machine");
    assert_eq!(global_memory_read_impl(&state, &local_id).unwrap()["reach"], "machine");
}

#[tokio::test]
async fn a_bad_reach_writes_nothing() {
    let state = crate::server::tests::test_state();
    state.id_store.bundle_scope_ensure().unwrap();
    let err = global_memory_write_with_reach_impl(&state, "agent-1", "", None, "Bad", "x", None, Some("everywhere")).unwrap_err();
    assert!(err.contains("reach must be"), "{err}");
    assert!(state.id_store.bundle_list_global().unwrap().is_empty(), "nothing was written");
    assert!(global_memory_reach_impl(&state, "agent-1", "anything", "sideways").unwrap_err().contains("reach must be"));
}

#[tokio::test]
async fn the_reach_of_an_existing_entry_can_be_flipped_without_touching_its_text() {
    let state = crate::server::tests::test_state();
    state.id_store.bundle_upsert(&ordinary_bundle("old-1", true)).unwrap();
    state.id_store.bundle_scope_ensure().unwrap();
    assert_eq!(entry_reach(&state, "old-1"), "machine", "an entry that existed before reach stays here");

    let answer = global_memory_reach_impl(&state, "agent-1", "old-1", "fleet").unwrap();
    assert_eq!((answer["id"].as_str(), answer["reach"].as_str()), (Some("old-1"), Some("fleet")));
    assert_eq!(entry_reach(&state, "old-1"), "fleet");
    let bundle = state.id_store.bundle_get("old-1").unwrap().unwrap();
    assert_eq!((bundle.instructions.as_str(), bundle.updated_at), ("original content", 0), "the text and its time are untouched");

    global_memory_reach_impl(&state, "agent-1", "old-1", "machine").unwrap();
    assert_eq!(entry_reach(&state, "old-1"), "machine");
}

#[tokio::test]
async fn reach_refuses_what_write_and_remove_refuse() {
    let state = crate::server::tests::test_state();
    let mut sys = ordinary_bundle("sys-9", true);
    sys.is_system = true;
    state.id_store.bundle_upsert_system(&sys).unwrap();
    state.id_store.bundle_upsert(&ordinary_bundle("preset-9", false)).unwrap();
    state.id_store.bundle_scope_ensure().unwrap();

    assert!(global_memory_reach_impl(&state, "agent-1", "sys-9", "fleet").unwrap_err().contains("system"));
    assert!(global_memory_reach_impl(&state, "agent-1", "blank", "fleet").unwrap_err().contains("blank"));
    assert!(global_memory_reach_impl(&state, "agent-1", "preset-9", "fleet").unwrap_err().contains("not a Global Memory entry"));
    assert!(global_memory_reach_impl(&state, "agent-1", "missing", "fleet").unwrap_err().contains("not found"));
    assert_eq!(entry_reach(&state, "sys-9"), "machine", "an AgentMux entry never leaves the machine");
}
