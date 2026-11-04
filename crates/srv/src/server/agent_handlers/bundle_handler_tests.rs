// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Memory bundle handlers, called as plain functions without the engine.

use super::*;
use crate::server::tests::test_state;

fn upsert_req(v: serde_json::Value) -> CommandUpsertBundleData {
    serde_json::from_value(v).unwrap()
}

fn user_ctx() -> RpcContext {
    RpcContext::default()
}

fn agent_ctx(agent_id: &str) -> RpcContext {
    RpcContext { agent_id: agent_id.to_string(), ..Default::default() }
}

/// An upsert mints an id and stamps the times; `get` and `list` then agree, and
/// the version is recorded as the window ("armory-ui") unless an agent wrote it.
#[tokio::test]
async fn upsert_get_and_list_memories_agree() {
    let state = test_state();
    let saved = upsert_memory(&state, &user_ctx(), upsert_req(serde_json::json!({ "id": "", "name": "Direct", "is_global": true }))).unwrap();
    assert!(!saved.id.is_empty());
    assert!(saved.created_at > 0 && saved.updated_at >= saved.created_at);

    let got = get_memory(&state, serde_json::from_value(serde_json::json!({ "id": saved.id })).unwrap()).unwrap();
    assert_eq!(got.name, "Direct");
    let listed = list_memories(&state).unwrap();
    assert!(listed.iter().any(|b| b.id == saved.id));

    let err = get_memory(&state, serde_json::from_value(serde_json::json!({ "id": "nope" })).unwrap()).unwrap_err();
    assert_eq!(err, "getmemory: not found id=nope");
}

/// The blank singleton can't be mutated, by either its flag or its id.
#[tokio::test]
async fn upsert_memory_refuses_the_blank_singleton() {
    let state = test_state();
    for v in [
        serde_json::json!({ "id": "x", "name": "n", "is_blank": true }),
        serde_json::json!({ "id": "blank", "name": "evil", "is_blank": false }),
    ] {
        let err = upsert_memory(&state, &user_ctx(), upsert_req(v)).unwrap_err();
        assert_eq!(err, "upsertmemory: cannot mutate the blank singleton");
    }
}

/// A save is recorded as the window, or as the registered agent that made it.
#[tokio::test]
async fn upsert_memory_records_who_wrote_it() {
    let state = test_state();
    let by_window = upsert_memory(&state, &user_ctx(), upsert_req(serde_json::json!({ "id": "", "name": "W", "is_global": true }))).unwrap();
    let by_agent = upsert_memory(&state, &agent_ctx("agent-w"), upsert_req(serde_json::json!({ "id": "", "name": "A", "is_global": true }))).unwrap();

    let last_writer = |id: &str| {
        state.id_store.bundle_version_list(id).unwrap().first().map(|v| v.written_by.clone()).unwrap_or_default()
    };
    assert_eq!(last_writer(&by_window.id), "armory-ui");
    assert_eq!(last_writer(&by_agent.id), "agent-w");
}

/// `deletememory` reports whether a row went, and a second delete says false.
#[tokio::test]
async fn delete_memory_reports_whether_a_row_went() {
    let state = test_state();
    let saved = upsert_memory(&state, &user_ctx(), upsert_req(serde_json::json!({ "id": "", "name": "Doomed" }))).unwrap();
    let del = |id: &str| delete_memory(&state, serde_json::from_value(serde_json::json!({ "id": id })).unwrap()).unwrap().deleted;
    assert!(del(&saved.id));
    assert!(state.id_store.bundle_get(&saved.id).unwrap().is_none());
    assert!(!del(&saved.id));
}

/// The system tier has its own save and delete: the row it answers with is the
/// stored one, and the ordinary delete refuses to touch it.
#[tokio::test]
async fn system_memory_save_answers_with_the_stored_row_and_ordinary_delete_refuses_it() {
    let state = test_state();
    let saved = upsert_system_memory(&state, &user_ctx(), upsert_req(serde_json::json!({ "id": "sys-direct", "name": "Sys" }))).unwrap();
    assert!(saved.is_system, "the stored row is system-tier, whatever the request carried");

    let err = delete_memory(&state, serde_json::from_value(serde_json::json!({ "id": "sys-direct" })).unwrap()).unwrap_err();
    assert_eq!(err, "deletememory: cannot delete a system Global Memory entry via the generic delete path; use bundle_delete_system");
    assert!(state.id_store.bundle_get("sys-direct").unwrap().is_some(), "the refused delete removed nothing");
    let system = delete_system_memory(&state, serde_json::from_value(serde_json::json!({ "id": "sys-direct" })).unwrap()).unwrap();
    assert!(system.deleted);
}

/// A reorder answers with how many it moved.
#[tokio::test]
async fn reorder_global_bundles_reports_the_count() {
    let state = test_state();
    let a = upsert_memory(&state, &user_ctx(), upsert_req(serde_json::json!({ "id": "", "name": "A", "is_global": true }))).unwrap();
    let b = upsert_memory(&state, &user_ctx(), upsert_req(serde_json::json!({ "id": "", "name": "B", "is_global": true }))).unwrap();
    let done = reorder_global_bundles(&state, serde_json::from_value(serde_json::json!({ "ids": [b.id, a.id] })).unwrap()).unwrap();
    assert!(done.updated >= 2, "{}", done.updated);
}
