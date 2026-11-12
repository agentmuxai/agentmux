// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `listrecentsessions` as the picker sees it, through the RPC engine: how the shared registry and this
//! channel's own instances merge, which rows survive the limit, and in what order. Written against the
//! handler before it was split into named steps; they fill in what the end-to-end tests in `session.rs`
//! (which cover the account line, the preview and the degraded sources) leave open.

use std::sync::Arc;

use super::register;
use crate::backend::rpc::engine::WshRpcEngine;
use crate::backend::rpc_types::{ListRecentSessionsResult, RpcMessage, COMMAND_LIST_RECENT_SESSIONS};
use crate::backend::storage::store::{AgentDefinition, AgentInstance};
use crate::registry::{
    DefinitionRecord, DefinitionRecordV1, DefinitionStore, NamedAgentRecord, NamedAgentRecordV1, Registry,
};
use crate::server::tests::test_state;
use crate::server::AppState;

fn named(instance_id: &str, definition_id: &str, name: &str, launched_ms: i64, identity: Option<&str>) -> NamedAgentRecord {
    NamedAgentRecord {
        schema_version: 1,
        data: NamedAgentRecordV1 {
            instance_id: instance_id.to_string(),
            instance_name: name.to_string(),
            definition_id: definition_id.to_string(),
            identity_id: identity.map(str::to_string),
            memory_id: None,
            session_id: Some("registry-sid".to_string()),
            working_dir: format!("{definition_id}-workdir"),
            source_agents_base: None,
            created_at_ms: 1,
            last_launched_at_ms: launched_ms,
            created_by_version: "test".to_string(),
            last_launched_by_version: "test".to_string(),
        },
    }
}

fn definition(id: &str) -> DefinitionRecord {
    DefinitionRecord {
        schema_version: 1,
        data: DefinitionRecordV1 { id: id.to_string(), name: format!("Agent {id}"), provider: "claude".to_string(), is_seeded: 0, ..Default::default() },
    }
}

fn local_def(id: &str) -> AgentDefinition {
    AgentDefinition {
        id: id.to_string(),
        slug: id.to_string(),
        name: format!("Local {id}"),
        icon: String::new(),
        provider: "claude".to_string(),
        description: String::new(),
        working_directory: String::new(),
        shell: String::new(),
        provider_flags: String::new(),
        auto_start: 0,
        restart_on_crash: 0,
        idle_timeout_minutes: 0,
        created_at: 1,
        agent_type: "standalone".to_string(),
        environment: String::new(),
        agent_bus_id: String::new(),
        is_seeded: 0,
        accounts: String::new(),
        parent_id: String::new(),
        branch_label: String::new(),
        updated_at: 1,
        user_hidden: 0,
        container_image: String::new(),
        container_volumes: "[]".to_string(),
        container_name: String::new(),
        use_ambient_login: 0,
        model_vendor_base_url: String::new(),
        memory_id: String::new(),
        seat: None,
        conversation_visibility: "private".to_string(),
    }
}

fn local_instance(id: &str, definition_id: &str, name: &str, session_id: &str, started_at: i64) -> AgentInstance {
    AgentInstance {
        id: id.to_string(),
        definition_id: definition_id.to_string(),
        parent_instance_id: String::new(),
        block_id: String::new(),
        session_id: session_id.to_string(),
        status: "available".to_string(),
        github_context: String::new(),
        started_at,
        ended_at: 0,
        created_at: 1,
        identity_id: String::new(),
        memory_id: String::new(),
        instance_name: name.to_string(),
        working_directory: format!("/local/{id}"),
        display_hidden: false,
    }
}

/// A state with the given registry records and their definitions attached. The returned dirs keep them alive.
fn state_with(records: &[NamedAgentRecord]) -> (AppState, tempfile::TempDir, tempfile::TempDir) {
    let state = test_state();
    let registry_dir = tempfile::tempdir().unwrap();
    let def_dir = tempfile::tempdir().unwrap();
    let registry = Registry::open(registry_dir.path().to_path_buf()).unwrap();
    let defs = DefinitionStore::open(def_dir.path().to_path_buf()).unwrap();
    for r in records {
        registry.upsert(r).unwrap();
        defs.upsert(&definition(&r.data.definition_id)).unwrap();
    }
    state.mstore.set_registry(Arc::new(registry));
    state.mstore.set_def_registry(Arc::new(defs));
    (state, registry_dir, def_dir)
}

async fn list(state: &AppState, request: serde_json::Value) -> ListRecentSessionsResult {
    let (engine, mut rx) = WshRpcEngine::new();
    register(&engine, state);
    engine.handle_message(RpcMessage {
        command: COMMAND_LIST_RECENT_SESSIONS.to_string(),
        reqid: "req-1".to_string(),
        data: Some(request),
        ..Default::default()
    });
    let resp = tokio::time::timeout(std::time::Duration::from_secs(2), rx.recv()).await.unwrap().unwrap();
    assert!(resp.error.is_empty(), "unexpected error: {}", resp.error);
    serde_json::from_value(resp.data.expect("result data")).unwrap()
}

fn names(result: &ListRecentSessionsResult) -> Vec<&str> {
    result.rows.iter().map(|r| r.instance_name.as_str()).collect()
}

#[tokio::test]
async fn rows_come_newest_launch_first_and_the_limit_cuts_the_tail() {
    let (state, _r, _d) = state_with(&[
        named("i-a", "def-a", "old", 100, None),
        named("i-b", "def-b", "newest", 300, None),
        named("i-c", "def-c", "middle", 200, None),
    ]);

    assert_eq!(names(&list(&state, serde_json::json!({ "limit": 20 })).await), ["newest", "middle", "old"]);
    assert_eq!(names(&list(&state, serde_json::json!({ "limit": 2 })).await), ["newest", "middle"]);
    assert_eq!(list(&state, serde_json::json!({})).await.rows.len(), 3, "no limit means the default of 20");
    assert_eq!(list(&state, serde_json::json!("not an object")).await.rows.len(), 3, "a malformed request means the defaults");
}

#[tokio::test]
async fn two_launches_of_one_named_agent_collapse_to_the_newest() {
    let (state, _r, _d) = state_with(&[
        named("i-1", "def-a", "same", 100, None),
        named("i-2", "def-a", "same", 500, None),
        named("i-3", "def-b", "other", 200, None),
    ]);
    let result = list(&state, serde_json::json!({ "limit": 20 })).await;
    assert_eq!(names(&result), ["same", "other"]);
    assert_eq!(result.rows[0].started_at, 500);
}

#[tokio::test]
async fn the_identity_filter_keeps_only_agents_bound_to_that_identity() {
    let (state, _r, _d) = state_with(&[
        named("i-1", "def-a", "mine", 100, Some("acct-1")),
        named("i-2", "def-b", "theirs", 200, Some("acct-2")),
        named("i-3", "def-c", "unbound", 300, None),
    ]);
    let result = list(&state, serde_json::json!({ "limit": 20, "identity_id": "  acct-1 " })).await;
    assert_eq!(names(&result), ["mine"]);
    let blank = list(&state, serde_json::json!({ "limit": 20, "identity_id": "   " })).await;
    assert_eq!(blank.rows.len(), 3, "a blank filter is no filter");
}

#[tokio::test]
async fn a_local_row_replaces_its_registry_row_and_keeps_its_own_session_id() {
    let (state, _r, _d) = state_with(&[named("i-1", "def-a", "agent-a", 100, None)]);
    let mut def = local_def("def-a");
    state.mstore.agent_def_insert(&mut def).unwrap();
    state.mstore.instance_create(&local_instance("local-1", "def-a", "agent-a", "local-sid", 50)).unwrap();

    let result = list(&state, serde_json::json!({ "limit": 20 })).await;
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0].session_id, "local-sid", "the channel's own instance is the fresher source");
    assert_eq!(result.rows[0].working_directory, "/local/local-1");
}

#[tokio::test]
async fn an_empty_local_session_id_is_filled_from_the_registry() {
    let (state, _r, _d) = state_with(&[named("i-1", "def-a", "agent-a", 100, None)]);
    let mut def = local_def("def-a");
    state.mstore.agent_def_insert(&mut def).unwrap();
    state.mstore.instance_create(&local_instance("local-1", "def-a", "agent-a", "", 50)).unwrap();

    let result = list(&state, serde_json::json!({ "limit": 20 })).await;
    assert_eq!(result.rows[0].working_directory, "/local/local-1", "the row is the channel's own instance");
    assert_eq!(result.rows[0].session_id, "registry-sid");
}

#[tokio::test]
async fn an_agent_only_this_channel_knows_is_listed_beside_the_registry_rows() {
    let (state, _r, _d) = state_with(&[named("i-1", "def-a", "agent-a", 100, None)]);
    let mut def = local_def("def-local");
    state.mstore.agent_def_insert(&mut def).unwrap();
    state.mstore.instance_create(&local_instance("local-2", "def-local", "only-here", "sid", 900)).unwrap();

    let result = list(&state, serde_json::json!({ "limit": 20 })).await;
    let mut got = names(&result);
    got.sort_unstable();
    assert_eq!(got, ["agent-a", "only-here"]);
    let local_only = result.rows.iter().find(|r| r.instance_name == "only-here").unwrap();
    assert_eq!(local_only.definition_name, "Local def-local");
    assert_eq!(local_only.started_at, 900);
}

#[tokio::test]
async fn a_registry_only_row_reports_its_workdir_under_the_agents_base_and_no_block() {
    let (state, _r, _d) = state_with(&[named("i-1", "def-a", "agent-a", 100, None)]);
    let result = list(&state, serde_json::json!({ "limit": 20 })).await;
    let row = &result.rows[0];
    assert_eq!(row.block_id_hint, "");
    assert!(!row.has_snapshot);
    assert_eq!(row.session_id, "registry-sid");
    assert!(row.working_directory.ends_with("def-a-workdir"), "{}", row.working_directory);
    assert_eq!(row.memory_name, "(vanilla CLI)");
    assert_eq!(row.last_active_at, 100, "no history: the launch time stands in");
}

#[tokio::test]
async fn a_row_whose_definition_is_gone_is_still_listed_and_says_so() {
    let (state, _r, _d) = state_with(&[named("i-1", "def-a", "agent-a", 100, None)]);
    // Remove the definition the registry row points at, keeping the row.
    let defs_dir = tempfile::tempdir().unwrap();
    state.mstore.set_def_registry(Arc::new(DefinitionStore::open(defs_dir.path().to_path_buf()).unwrap()));
    let result = list(&state, serde_json::json!({ "limit": 20 })).await;
    assert_eq!(result.rows.len(), 1);
    assert_eq!(result.rows[0].definition_name, "(missing definition)");
    assert_eq!(result.rows[0].provider, "");
}
