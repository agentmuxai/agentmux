// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use serde_json::json;

use super::super::{lookup, RemoteCaller, Verification};
use super::*;
use crate::server::tests::test_state;

fn caller() -> RemoteCaller {
    RemoteCaller {
        agent_uid: "uid-claimed".into(),
        slug: "loap".into(),
        machine: "charlie".into(),
        instance_id: "inst-1".into(),
        install_trusted: true,
        verification: Verification::Lan,
        reason: "sync".into(),
    }
}

fn entry(id: &str, text: &str, updated_at: i64) -> Value {
    json!({ "id": id, "name": format!("Rules {id}"), "instructions": text, "updated_at": updated_at, "edited_by": "armory-ui" })
}

#[tokio::test]
async fn a_put_creates_the_entry_as_the_remote_agent_and_never_as_the_operator() {
    let state = test_state();
    let answer = put(&state, &caller(), entry("f1", "shared rule", 5_000)).await.unwrap();
    assert_eq!(answer["outcome"], "created");

    let b = state.id_store.bundle_get("f1").unwrap().unwrap();
    assert!(b.is_global);
    assert_eq!((b.instructions.as_str(), b.updated_at), ("shared rule", 5_000));
    let v = state.id_store.bundle_version_latest("f1").unwrap().unwrap();
    assert_eq!(v.written_by, "loap@charlie");
    assert_eq!(v.written_by_uid, "", "a claimed uid is not proof; none is recorded");
    assert_eq!(v.source, "fleet");
}

#[tokio::test]
async fn an_older_write_is_skipped_and_says_why() {
    let state = test_state();
    put(&state, &caller(), entry("f1", "new", 5_000)).await.unwrap();
    let answer = put(&state, &caller(), entry("f1", "old", 1_000)).await.unwrap();
    assert_eq!((answer["outcome"].as_str(), answer["reason"].as_str()), (Some("skipped"), Some("older")));
    assert_eq!(state.id_store.bundle_get("f1").unwrap().unwrap().instructions, "new");
}

#[tokio::test]
async fn a_removal_takes_the_entry_out_of_global_memory_and_a_later_put_brings_it_back() {
    let state = test_state();
    put(&state, &caller(), entry("f1", "text", 1_000)).await.unwrap();
    let removed = remove(&state, &caller(), json!({ "id": "f1", "removed_at": 2_000 })).await.unwrap();
    assert_eq!(removed["outcome"], "removed");
    assert!(!state.id_store.bundle_get("f1").unwrap().unwrap().is_global);
    let back = put(&state, &caller(), entry("f1", "again", 3_000)).await.unwrap();
    assert_eq!(back["outcome"], "restored");
}

#[tokio::test]
async fn malformed_params_are_refused_as_bad_params_and_write_nothing() {
    let state = test_state();
    let missing = put(&state, &caller(), json!({ "id": "f1" })).await.unwrap_err();
    assert_eq!(missing.code, "bad_params");
    let blank_name = put(&state, &caller(), json!({ "id": "f2", "name": " ", "instructions": "x", "updated_at": 1 })).await.unwrap_err();
    assert_eq!(blank_name.code, "bad_params");
    assert!(state.id_store.bundle_get("f2").unwrap().is_none());
}

#[tokio::test]
async fn the_listing_has_ids_times_and_hashes_and_no_text() {
    let state = test_state();
    put(&state, &caller(), entry("f1", "secret wording", 1_000)).await.unwrap();
    let answer = list(&state, &caller(), json!({})).await.unwrap();
    let rows = answer["entries"].as_array().unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!((rows[0]["id"].as_str(), rows[0]["updated_at"].as_i64(), rows[0]["removed"].as_bool()), (Some("f1"), Some(1_000), Some(false)));
    assert!(rows[0]["hash"].as_str().is_some_and(|h| h.len() == 64));
    assert!(!answer.to_string().contains("secret wording"));
}

#[test]
fn the_writes_need_a_certificate_backed_caller_and_the_listing_does_not() {
    assert!(lookup("memory.global.put").unwrap().changes_state);
    assert!(lookup("memory.global.remove").unwrap().changes_state);
    assert!(!lookup("memory.global.list").unwrap().changes_state);
}
