// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The Kanban's agent routes end to end through the router (SPEC_WARDEN_KANBAN_2026_10_20.md).

use axum::body::Body;
use axum::http::{Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;

/// A router and the state behind it; a request acts as the agent its body names.
struct Api {
    router: axum::Router,
    state: crate::server::AppState,
}

fn app() -> Api {
    app_and_state().0
}

async fn post(app: &Api, uri: &str, body: Value) -> (StatusCode, Value) {
    let token = crate::server::tests::token_for(&app.state, &crate::server::tests::actor_in(&body));
    let req = Request::builder()
        .uri(uri)
        .method("POST")
        .header("X-AuthKey", "test-secret-key")
        .header("X-Agent-Token", token)
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = app.router.clone().oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

#[tokio::test]
async fn tasks_get_numbers_that_work_wherever_an_id_does() {
    let app = app();
    let (s, v) = post(&app, "/agentmux/work", json!({"title": "t", "payload": "p"})).await;
    assert_eq!(s, StatusCode::OK);
    let num = v["num"].as_i64().expect("a number");
    let (_, c) = post(&app, "/agentmux/work/claim", json!({"agent_id": "a1"})).await;
    assert_eq!(c["num"].as_i64(), Some(num));
    let attempt = c["attempt"].as_i64().unwrap();
    let (s, _) = post(&app, &format!("/agentmux/work/{num}/heartbeat"), json!({"agent_id": "a1", "attempt": attempt})).await;
    assert_eq!(s, StatusCode::OK, "the number is accepted as the id");
}

#[tokio::test]
async fn a_document_link_must_be_a_full_path() {
    let app = app();
    post(&app, "/agentmux/work", json!({"title": "t", "payload": "p"})).await;
    let (_, c) = post(&app, "/agentmux/work/claim", json!({"agent_id": "a1"})).await;
    let (num, attempt) = (c["num"].as_i64().unwrap(), c["attempt"].as_i64().unwrap());
    let uri = format!("/agentmux/work/{num}/update");
    let update = |doc: &str| json!({"agent_id": "a1", "attempt": attempt, "doc": doc});

    let (s, v) = post(&app, &uri, update("docs/specs/SPEC_X.md")).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    assert!(v["error"].as_str().unwrap().contains("relative path"), "{v}");
    for ok in ["C:/Users/a/SPEC_X.md", r"c:\Users\a\SPEC_X.md", "/home/a/SPEC_X.md", "~/work/SPEC_X.md", r"\\host\share\SPEC_X.md"] {
        let (s, v) = post(&app, &uri, update(ok)).await;
        assert_eq!(s, StatusCode::OK, "{ok}: {v}");
    }

    let submit = json!({"agent_id": "a1", "attempt": attempt, "text": "done", "doc": "SPEC_X.md"});
    let (s, v) = post(&app, &format!("/agentmux/work/{num}/submit"), submit).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "submit checks it too: {v}");
}

#[tokio::test]
async fn a_build_claimed_by_anyone_is_shown_its_specs_document() {
    let app = app();
    let (_, pair) = post(&app, "/agentmux/work", json!({"title": "Feature", "payload": "p", "first": "spec"})).await;
    let (spec_num, build_num) = (pair["num"].as_i64().unwrap(), pair["build_num"].as_i64().unwrap());

    let (_, c) = post(&app, "/agentmux/work/claim", json!({"agent_id": "writer"})).await;
    assert_eq!(c["num"].as_i64(), Some(spec_num), "the build waits for its spec");
    assert_eq!(c["after"], json!([]), "a spec starts after nothing");
    let doc = "C:/work/SPEC_FEATURE.md";
    let done = json!({"agent_id": "writer", "attempt": c["attempt"], "text": "approved", "doc": doc});
    let (s, v) = post(&app, &format!("/agentmux/work/{spec_num}/submit"), done).await;
    assert_eq!(s, StatusCode::OK, "{v}");

    // Another agent takes the build; no unlock note went to it, the claim carries the path.
    let (_, c) = post(&app, "/agentmux/work/claim", json!({"agent_id": "builder"})).await;
    assert_eq!(c["num"].as_i64(), Some(build_num), "{c}");
    let after = c["after"].as_array().expect("after");
    assert_eq!(after.len(), 1);
    assert_eq!((after[0]["num"].as_i64(), after[0]["doc"].as_str()), (Some(spec_num), Some(doc)), "{after:?}");
}

#[tokio::test]
async fn finishing_a_task_names_the_next_one_waiting_for_the_agent() {
    let app = app();
    post(&app, "/agentmux/work", json!({"title": "first", "payload": "p", "target_agent": "a1"})).await;
    let (_, c) = post(&app, "/agentmux/work/claim", json!({"agent_id": "a1"})).await;
    let (first, attempt) = (c["num"].as_i64().unwrap(), c["attempt"].as_i64().unwrap());
    let (_, second) = post(&app, "/agentmux/work", json!({"title": "second", "payload": "p", "target_agent": "a1"})).await;
    post(&app, "/agentmux/work", json!({"title": "someone else's", "payload": "p", "target_agent": "a2"})).await;

    let (s, v) =
        post(&app, &format!("/agentmux/work/{first}/complete"), json!({"agent_id": "a1", "attempt": attempt, "result": "done"})).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["state"], "done", "no review asked for: done");
    assert_eq!(v["next"]["num"], second["num"]);
    assert_eq!(v["next"]["title"], "second");
    assert_eq!(v["waiting"], 1);
}

#[tokio::test]
async fn a_task_that_needs_review_is_handed_in_not_done_and_ask_blocks_it() {
    let app = app();
    post(&app, "/agentmux/work", json!({"title": "t", "payload": "p", "review": true})).await;
    let (_, c) = post(&app, "/agentmux/work/claim", json!({"agent_id": "a1"})).await;
    let (num, attempt) = (c["num"].as_i64().unwrap(), c["attempt"].as_i64().unwrap());

    let (s, _) = post(&app, &format!("/agentmux/work/{num}/update"), json!({"agent_id": "a1", "attempt": attempt + 1, "note": "x"})).await;
    assert_eq!(s, StatusCode::CONFLICT, "a stale attempt");
    let (s, _) = post(
        &app,
        &format!("/agentmux/work/{num}/update"),
        json!({"agent_id": "a1", "attempt": attempt, "note": "half way", "checklist": [{"text": "a", "status": "completed"}]}),
    )
    .await;
    assert_eq!(s, StatusCode::OK);

    let (s, v) = post(&app, &format!("/agentmux/work/{num}/ask"), json!({"agent_id": "a1", "attempt": attempt, "text": "which one?"})).await;
    assert_eq!((s, v["state"].as_str()), (StatusCode::OK, Some("blocked")));
    let (s, _) = post(&app, &format!("/agentmux/work/{num}/ask"), json!({"agent_id": "a1", "attempt": attempt, "text": ""})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "a question is required");

    let (s, v) =
        post(&app, &format!("/agentmux/work/{num}/submit"), json!({"agent_id": "a1", "attempt": attempt, "text": "PR #1"})).await;
    assert_eq!(s, StatusCode::OK);
    assert_eq!(v["state"], "review", "the agent can't accept its own work");
    assert!(v["next"].is_null());
}

#[tokio::test]
async fn filing_for_later_can_hold_a_task_out_of_reach() {
    let app = app();
    post(&app, "/agentmux/work", json!({"title": "later", "payload": "p", "hold": true})).await;
    let (_, c) = post(&app, "/agentmux/work/claim", json!({"agent_id": "a1"})).await;
    assert_eq!(c["claimed"], false, "held: nobody takes it");
}

// SPEC_KANBAN_AGENTS_HOLD_AND_UNHOLD_2026_10_22.md: agents hold and unhold.

/// A router and the state behind it, to read a task's history.
fn app_and_state() -> (Api, crate::server::AppState) {
    let state = crate::server::tests::test_state();
    (Api { router: crate::server::build_router(state.clone()), state: state.clone() }, state)
}

fn history(state: &crate::server::AppState, num: i64) -> Vec<(String, String, String)> {
    let id = state.identity_store.work_board_get(&num.to_string()).unwrap().unwrap().item.id;
    state.identity_store.work_events(&id, 100).unwrap().into_iter().map(|e| (e.kind, e.actor, e.body)).collect()
}

#[tokio::test]
async fn any_agent_holds_and_unholds_a_waiting_task_and_the_history_says_who() {
    let (app, state) = app_and_state();
    let (_, v) = post(&app, "/agentmux/work", json!({"title": "t", "payload": "p", "target_agent": "a1"})).await;
    let num = v["num"].as_i64().unwrap();

    let (s, v) = post(&app, &format!("/agentmux/work/{num}/hold"), json!({"agent_id": "a2", "reason": "waiting on a reply"})).await;
    assert_eq!((s, v["state"].as_str(), v["changed"].as_bool()), (StatusCode::OK, Some("held"), Some(true)), "another agent's task: {v}");
    let (_, c) = post(&app, "/agentmux/work/claim", json!({"agent_id": "a1"})).await;
    assert_eq!(c["claimed"], false, "held: nobody takes it");
    let (s, v) = post(&app, &format!("/agentmux/work/{num}/hold"), json!({"agent_id": "a2"})).await;
    assert_eq!((s, v["changed"].as_bool()), (StatusCode::OK, Some(false)), "a repeat is not an error");

    let (s, v) = post(&app, &format!("/agentmux/work/{num}/unhold"), json!({"agent_id": "a3", "reason": "it came"})).await;
    assert_eq!((s, v["state"].as_str(), v["for"].as_str()), (StatusCode::OK, Some("open"), Some("a1")));
    let (s, v) = post(&app, &format!("/agentmux/work/{num}/unhold"), json!({"agent_id": "a3"})).await;
    assert_eq!((s, v["changed"].as_bool()), (StatusCode::OK, Some(false)));

    let rows = history(&state, num);
    let moves: Vec<_> = rows.iter().filter(|(k, _, _)| k == "held" || k == "unheld").collect();
    assert_eq!(moves.len(), 2, "one row per move, none for a repeat: {rows:?}");
    assert!(moves.iter().any(|(k, a, b)| k == "held" && a == "a2" && b == "waiting on a reply"));
    assert!(moves.iter().any(|(k, a, b)| k == "unheld" && a == "a3" && b == "it came"));
    let (_, c) = post(&app, "/agentmux/work/claim", json!({"agent_id": "a1"})).await;
    assert_eq!(c["num"].as_i64(), Some(num));
}

#[tokio::test]
async fn only_its_holder_can_put_a_claimed_task_on_hold_and_the_pause_costs_no_attempt() {
    let app = app();
    post(&app, "/agentmux/work", json!({"title": "t", "payload": "p"})).await;
    let (_, c) = post(&app, "/agentmux/work/claim", json!({"agent_id": "a1"})).await;
    let (num, attempt) = (c["num"].as_i64().unwrap(), c["attempt"].as_i64().unwrap());

    let (s, v) = post(&app, &format!("/agentmux/work/{num}/hold"), json!({"agent_id": "a2"})).await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert!(v["error"].as_str().unwrap().contains("a1"), "names who is working on it: {v}");
    let (s, _) = post(&app, &format!("/agentmux/work/{num}/hold"), json!({"agent_id": "a1"})).await;
    assert_eq!(s, StatusCode::CONFLICT, "its holder, without the attempt");

    let (s, v) = post(&app, &format!("/agentmux/work/{num}/hold"), json!({"agent_id": "a1", "attempt": attempt, "reason": "blocked on X"})).await;
    assert_eq!((s, v["state"].as_str(), v["for"].as_str()), (StatusCode::OK, Some("held"), Some("a1")), "{v}");
    let (_, c) = post(&app, "/agentmux/work/claim", json!({"agent_id": "a1", "id": num.to_string()})).await;
    let again = c["attempt"].as_i64().unwrap();
    assert_eq!((c["claimed"].as_bool(), again), (Some(true), attempt + 1), "a new claim, a new fence");
    assert_eq!(c["item"]["max_attempts"].as_i64(), Some(4), "the pause didn't use up an attempt");
    let (s, _) = post(&app, &format!("/agentmux/work/{num}/update"), json!({"agent_id": "a1", "attempt": attempt, "note": "late"})).await;
    assert_eq!(s, StatusCode::CONFLICT, "the paused claim's attempt no longer works");
}

#[tokio::test]
async fn a_finished_or_questioned_task_cant_be_held_and_an_open_one_cant_be_unheld() {
    let app = app();
    post(&app, "/agentmux/work", json!({"title": "t", "payload": "p", "review": true})).await;
    let (_, c) = post(&app, "/agentmux/work/claim", json!({"agent_id": "a1"})).await;
    let (num, attempt) = (c["num"].as_i64().unwrap(), c["attempt"].as_i64().unwrap());
    post(&app, &format!("/agentmux/work/{num}/ask"), json!({"agent_id": "a1", "attempt": attempt, "text": "which?"})).await;
    let (s, _) = post(&app, &format!("/agentmux/work/{num}/hold"), json!({"agent_id": "a1", "attempt": attempt})).await;
    assert_eq!(s, StatusCode::CONFLICT, "blocked: its question is the pause");
    post(&app, &format!("/agentmux/work/{num}/submit"), json!({"agent_id": "a1", "attempt": attempt, "text": "done"})).await;
    let (s, v) = post(&app, &format!("/agentmux/work/{num}/hold"), json!({"agent_id": "a1"})).await;
    assert_eq!(s, StatusCode::CONFLICT);
    assert!(v["error"].as_str().unwrap().contains("review"), "handed in: {v}");
    let (s, _) = post(&app, &format!("/agentmux/work/{num}/unhold"), json!({"agent_id": "a1"})).await;
    assert_eq!(s, StatusCode::CONFLICT, "not on hold");
    let (s, _) = post(&app, "/agentmux/work/9999/hold", json!({"agent_id": "a1"})).await;
    assert_eq!(s, StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn a_named_claim_takes_a_held_task_and_says_why_when_it_cant() {
    let (app, state) = app_and_state();
    let (_, mine) = post(&app, "/agentmux/work", json!({"title": "mine", "payload": "p", "target_agent": "a1", "hold": true})).await;
    let (_, other) = post(&app, "/agentmux/work", json!({"title": "next", "payload": "p", "target_agent": "a1"})).await;
    let (_, theirs) = post(&app, "/agentmux/work", json!({"title": "theirs", "payload": "p", "target_agent": "a2"})).await;
    let num = mine["num"].as_i64().unwrap();

    let (_, c) = post(&app, "/agentmux/work/claim", json!({"agent_id": "a1", "id": format!("#{num}")})).await;
    assert_eq!((c["claimed"].as_bool(), c["num"].as_i64()), (Some(true), Some(num)), "that task, not the open one: {c}");
    let kinds: Vec<_> = history(&state, num).into_iter().map(|(k, _, _)| k).collect();
    assert!(kinds.contains(&"unheld".to_string()) && kinds.contains(&"claimed".to_string()), "{kinds:?}");

    let theirs = theirs["num"].as_i64().unwrap();
    let (_, c) = post(&app, "/agentmux/work/claim", json!({"agent_id": "a1", "id": theirs.to_string()})).await;
    assert_eq!(c["claimed"], false);
    assert!(c["why"].as_str().unwrap().contains("a2"), "{c}");
    let (_, c) = post(&app, "/agentmux/work/claim", json!({"agent_id": "a1", "id": num.to_string()})).await;
    assert!(c["why"].as_str().unwrap().contains("claimed"), "already taken: {c}");
    let (_, c) = post(&app, "/agentmux/work/claim", json!({"agent_id": "a1"})).await;
    assert_eq!(c["num"], other["num"], "without id, claim is unchanged");
}

#[tokio::test]
async fn a_reason_that_looks_like_a_credential_is_refused_and_nothing_moves() {
    let (app, state) = app_and_state();
    let (_, v) = post(&app, "/agentmux/work", json!({"title": "t", "payload": "p"})).await;
    let num = v["num"].as_i64().unwrap();
    let (s, _) = post(&app, &format!("/agentmux/work/{num}/hold"), json!({"agent_id": "a1", "reason": "waiting: password=hunter2"})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST);
    let id = state.identity_store.work_board_get(&num.to_string()).unwrap().unwrap().item.id;
    assert_eq!(state.identity_store.work_queue_get(&id).unwrap().unwrap().state, "open", "nothing moved");
    assert!(history(&state, num).iter().all(|(k, _, _)| k != "held"));
    post(&app, &format!("/agentmux/work/{num}/hold"), json!({"agent_id": "a1"})).await;
    let (s, _) = post(&app, &format!("/agentmux/work/{num}/unhold"), json!({"agent_id": "a1", "reason": "ran rm -rf on it"})).await;
    assert_eq!(s, StatusCode::BAD_REQUEST, "unhold too");
}

#[tokio::test]
async fn a_group_member_refused_by_name_is_told_the_real_reason() {
    let app = app();
    let (_, first) = post(&app, "/agentmux/work", json!({"title": "first", "payload": "p"})).await;
    let first = first["num"].as_i64().unwrap();
    let (_, t) = post(
        &app,
        "/agentmux/work",
        json!({"title": "group work", "payload": "p", "target_group": "g1", "after": [first.to_string()]}),
    )
    .await;
    let num = t["num"].as_i64().unwrap();
    let (_, c) = post(&app, "/agentmux/work/claim", json!({"agent_id": "a1", "id": num.to_string(), "groups": ["g1"]})).await;
    assert_eq!(c["claimed"], false);
    assert!(c["why"].as_str().unwrap().contains(&format!("starts after #{first}")), "a member: {c}");
    let (_, c) = post(&app, "/agentmux/work/claim", json!({"agent_id": "a2", "id": num.to_string()})).await;
    assert!(c["why"].as_str().unwrap().contains("group g1"), "not a member: {c}");
}

/// A live holder that is out of quota or signed out lets go of a task nobody in
/// particular owns, after 30 minutes with its reset far off; nothing else changes
/// its lease (R5 of SPEC_UNRESPONSIVE_AGENTS_2026_10_23.md).
mod lease_for_an_unavailable_holder {
    use super::super::renews_lease;
    use crate::agents::failure::FailureClass;
    use crate::backend::agent_state::{AgentState, Unavailable};
    use crate::backend::storage::work_queue::WorkItem;

    const MIN: i64 = 60_000;
    const NOW: i64 = 10_000 * MIN;

    fn task(target_agent: &str, target_uid: &str, target_group: &str) -> WorkItem {
        WorkItem {
            id: "t".into(),
            title: "t".into(),
            payload: String::new(),
            kind: String::new(),
            target_agent: target_agent.into(),
            target_group: target_group.into(),
            priority: 0,
            state: "claimed".into(),
            claimed_by: "Naki-2".into(),
            target_agent_uid: target_uid.into(),
            claimed_by_uid: String::new(),
            claim_expires: None,
            attempts: 1,
            max_attempts: 3,
            created_by: "x".into(),
            created_by_uid: String::new(),
            created_at: 0,
            updated_at: 0,
            not_before: None,
            result: String::new(),
        }
    }

    fn down(state: AgentState, since_min_ago: i64, resets_in_min: Option<i64>) -> Unavailable {
        Unavailable {
            state,
            code: FailureClass::UsageLimit,
            title: String::new(),
            resets_at_ms: resets_in_min.map(|m| (NOW + m * MIN) as u64),
            since_ms: (NOW - since_min_ago * MIN) as u64,
            account_id: None,
        }
    }

    #[test]
    fn a_holder_that_can_work_keeps_its_lease() {
        assert!(renews_lease(&task("", "", ""), None, NOW));
    }

    #[test]
    fn a_task_for_anyone_or_a_group_is_let_go_after_30_minutes_with_the_reset_far_off() {
        for item in [task("", "", ""), task("", "", "reviewers")] {
            assert!(renews_lease(&item, Some(&down(AgentState::Limited, 29, Some(300))), NOW), "not yet 30 minutes");
            assert!(!renews_lease(&item, Some(&down(AgentState::Limited, 30, Some(300))), NOW), "30 minutes, reset in 5 h");
            assert!(renews_lease(&item, Some(&down(AgentState::Limited, 90, Some(59))), NOW), "the reset is within the hour: wait it out");
            assert!(!renews_lease(&item, Some(&down(AgentState::Limited, 90, Some(61))), NOW));
            assert!(!renews_lease(&item, Some(&down(AgentState::Limited, 90, None)), NOW), "no reset known counts as far");
            assert!(!renews_lease(&item, Some(&down(AgentState::SignedOut, 45, None)), NOW), "signed out has no reset");
        }
    }

    #[test]
    fn a_task_aimed_at_an_agent_stays_with_it() {
        let long_gone = down(AgentState::Limited, 600, Some(2_000));
        assert!(renews_lease(&task("Naki-2", "", ""), Some(&long_gone), NOW));
        assert!(renews_lease(&task("", "uid-n", ""), Some(&long_gone), NOW));
    }
}

/// A build with a work store of its own (`task dev`) keeps the queue there: a task
/// it creates and claims never reaches the host's identity store, where the
/// operator's real queue is.
#[tokio::test]
async fn a_task_goes_to_the_work_store_not_the_identity_store() {
    use crate::backend::storage::store::Store;
    let mut state = crate::server::tests::test_state();
    let own = Store::open_in_memory().unwrap();
    own.apply_identity_schema_for_tests().unwrap();
    state.work_store = std::sync::Arc::new(own);
    let app = crate::server::build_router(state.clone());

    let (s, v) = post(&app, "/agentmux/work", json!({"title": "t", "payload": "p"})).await;
    assert_eq!(s, StatusCode::OK);
    let id = v["id"].as_str().expect("an id").to_string();
    let (_, c) = post(&app, "/agentmux/work/claim", json!({"agent_id": "a1"})).await;
    assert_eq!(c["num"], v["num"], "claimed from the work store");

    assert!(state.work_store.work_queue_get(&id).unwrap().is_some());
    assert!(state.identity_store.work_queue_get(&id).unwrap().is_none(), "the host's queue is untouched");
    assert!(state.identity_store.work_board_get(&id).unwrap().is_none(), "so is its board");
}
