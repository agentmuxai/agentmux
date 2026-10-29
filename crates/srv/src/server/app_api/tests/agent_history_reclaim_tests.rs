// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `App history_reclaim` (`/api/v1/agent/history/reclaim`): who a request is recorded as, that the operator's
//! window sees it pending and can withdraw it, and that a call with no agent token is refused.

use super::*;
use crate::server::tests::test_state;

fn db() -> (tempfile::TempDir, std::path::PathBuf) {
    let dir = tempfile::tempdir().unwrap();
    let db = dir.path().join("filestore.db");
    (dir, db)
}

#[test]
fn a_request_is_recorded_as_the_agent_and_shows_as_pending_to_the_window() {
    let (_dir, db) = db();
    let body = change(&db, "request", "loap-3", 1_000).ok().unwrap();
    assert_eq!(body["pending"], true);
    assert_eq!((body["requested_by"].as_str(), body["requested_at_ms"].as_u64()), (Some("loap-3"), Some(1_000)));
    // What the Storage view reads (`history.usage`) is the same marker.
    assert!(history_usage::reclaim_of(&db).pending);
    assert!(reclaim::requested(&db));
    let (marker, _) = reclaim::status(&db);
    assert_eq!(marker.unwrap().requested_by, "loap-3", "never \"operator\"");
}

#[test]
fn asking_again_keeps_the_first_request_and_says_who_made_it() {
    let (_dir, db) = db();
    reclaim::request(&db, "operator", 500).unwrap();
    let body = change(&db, "request", "loap-3", 2_000).ok().unwrap();
    assert_eq!((body["requested_by"].as_str(), body["requested_at_ms"].as_u64()), (Some("operator"), Some(500)));
}

#[test]
fn a_request_can_be_withdrawn_and_withdrawing_nothing_is_fine() {
    let (_dir, db) = db();
    change(&db, "request", "loap-3", 1_000).ok().unwrap();
    let body = change(&db, "cancel", "loap-3", 2_000).ok().unwrap();
    assert_eq!(body["pending"], false);
    assert!(body.get("requested_by").is_none());
    assert!(!history_usage::reclaim_of(&db).pending);
    assert_eq!(change(&db, "cancel", "loap-3", 3_000).ok().unwrap()["pending"], false);
}

#[test]
fn a_cancel_says_whose_request_it_took_back() {
    let (_dir, db) = db();
    reclaim::request(&db, "operator", 500).unwrap();
    let body = change(&db, "cancel", "loap-3", 2_000).ok().unwrap();
    assert_eq!(body["pending"], false);
    assert_eq!(body["withdrew_request_of"], "operator", "the person's request is taken back in the open");
    // With nothing pending, nothing was taken back.
    assert!(change(&db, "cancel", "loap-3", 3_000).ok().unwrap().get("withdrew_request_of").is_none());
}

#[test]
fn an_unknown_action_is_refused() {
    let (_dir, db) = db();
    let err = change(&db, "now", "loap-3", 1_000).err().unwrap();
    assert_eq!(err.status, StatusCode::BAD_REQUEST);
    assert!(err.message.contains("request or cancel"), "{}", err.message);
    assert!(!reclaim::requested(&db));
}

#[tokio::test]
async fn a_call_without_an_agent_token_is_refused_and_changes_nothing() {
    let state = test_state();
    let resp = handle_history_reclaim(State(state.clone()), None, Json(ReclaimRequest { action: "request".into() })).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    let resp = handle_history_reclaim_status(State(state.clone()), None).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    // A token that names no agent on this machine is no better.
    let stranger = Some(Extension(Caller::Agent { uid: "no-such-agent".into() }));
    let resp = handle_history_reclaim(State(state), stranger, Json(ReclaimRequest { action: "request".into() })).await;
    assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
}

/// Host and container agents alike get past the identity check: nothing here runs or writes on the host's
/// side of the agent.
#[tokio::test]
async fn host_and_container_agents_are_both_allowed() {
    use crate::backend::storage::agents::test_agent_def;
    let state = test_state();
    for agent_type in ["host", "container"] {
        let id = uuid::Uuid::new_v4().to_string();
        let mut def = test_agent_def(&id, &format!("Reclaim {}", &id[..8]), "claude", agent_type, 1, "");
        state.mstore.agent_def_insert(&mut def).unwrap();
        let caller = Some(Extension(Caller::Agent { uid: id }));
        let resp = handle_history_reclaim(State(state.clone()), caller, Json(ReclaimRequest { action: "request".into() })).await;
        // Past the identity check: the store isn't open in most tests (409), or another test opened it (200).
        assert!(matches!(resp.status(), StatusCode::CONFLICT | StatusCode::OK), "{agent_type}: {}", resp.status());
    }
}
