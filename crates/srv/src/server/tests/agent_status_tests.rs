// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `Presence status` end to end through the router: the line an agent sets is kept in its pane's
//! block meta with its time, a bad line or state is refused with the reason, and what the Tasks
//! claim writes (the task's title) goes when the task ends but never replaces the agent's own line.

use agentmux_common::ipc::{Command, Event};
use axum::body::Body;
use axum::http::{Method, Request, StatusCode};
use serde_json::{json, Value};
use tower::ServiceExt;

use crate::backend::obj::Block;
use crate::backend::reactive::get_global_handler;
use crate::backend::storage::agents::test_agent_def;
use crate::backend::work_facts::live_phrase::{self, declared_of};
use crate::server::routes::build_router;
use crate::server::tests::test_state;
use crate::server::{agent_status_handlers, AppState};

async fn dispatch(state: &AppState, cmd: Command) -> Vec<Event> {
    let evs = crate::server::service::dispatch_to_reducer(state, cmd).await;
    for ev in &evs {
        crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).unwrap();
    }
    evs
}

/// A pane in a new workspace and tab, made the way the app makes one, so the reducer knows it.
async fn new_pane(state: &AppState) -> String {
    let ws = dispatch(state, Command::CreateWorkspace { name: "w".into() })
        .await
        .into_iter()
        .find_map(|e| match e {
            Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id),
            _ => None,
        })
        .unwrap();
    let tab = dispatch(state, Command::CreateTab { workspace_id: ws, name: "t".into() })
        .await
        .into_iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id),
            _ => None,
        })
        .unwrap();
    dispatch(state, Command::CreateBlock { tab_id: tab, meta: json!({ "view": "agent" }) })
        .await
        .into_iter()
        .find_map(|e| match e {
            Event::BlockCreated { block_id, .. } => Some(block_id),
            _ => None,
        })
        .unwrap()
}

/// An agent (a definition, a token and, when `registered`, a registration on a real pane); returns
/// the state, the token and the pane's block id.
async fn state_with_agent(agent: &str, registered: bool) -> (AppState, String, String) {
    let state = test_state();
    let uid = format!("uid-{agent}");
    let mut def = test_agent_def(&uid, agent, "claude", "agent", 1, "");
    def.slug = agent.to_string();
    state.mstore.agent_def_insert(&mut def).unwrap();
    state.mstore.attach_token_index().unwrap();
    let token = state.mstore.agent_token_ensure(&uid).unwrap();
    let block_id = new_pane(&state).await;
    if registered {
        get_global_handler().register_agent_full(agent, &block_id, None, 0, None, Some(&uid), "test.agent_status").unwrap();
    }
    (state, token, block_id)
}

async fn post(state: &AppState, token: &str, uri: &str, body: Value) -> (StatusCode, Value) {
    let req = Request::builder()
        .method(Method::POST)
        .uri(uri)
        .header("X-AuthKey", "test-secret-key")
        .header("X-Agent-Token", token)
        .header("Content-Type", "application/json")
        .body(Body::from(body.to_string()))
        .unwrap();
    let resp = build_router(state.clone()).oneshot(req).await.unwrap();
    let status = resp.status();
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    (status, serde_json::from_slice(&bytes).unwrap_or(Value::Null))
}

fn kept(state: &AppState, block_id: &str) -> Option<live_phrase::Declared> {
    declared_of(&state.mstore.must_get::<Block>(block_id).unwrap().meta)
}

#[tokio::test]
async fn an_agent_sets_its_line_and_it_is_kept_with_the_time() {
    let (state, token, block) = state_with_agent("ps-setter", true).await;
    let (status, body) = post(
        &state,
        &token,
        "/api/v1/agent/status",
        json!({ "line": "  Fixing the stale-site\nrestore loop ", "state": "Working" }),
    )
    .await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["ok"], true);
    assert_eq!(body["line"], "Fixing the stale-site restore loop");
    assert_eq!(body["cut_to_fit"], false);
    let d = kept(&state, &block).expect("the line is kept in the block's meta");
    assert_eq!((d.text.as_str(), d.state.as_deref(), d.src.as_str()), ("Fixing the stale-site restore loop", Some("working"), "agent"));
    assert!(d.at_ms > 0);
}

#[tokio::test]
async fn a_bad_line_or_state_is_refused_with_the_reason_and_nothing_is_kept() {
    let (state, token, block) = state_with_agent("ps-refused", true).await;
    for (body, wanted) in [
        (json!({ "line": "   " }), "5 to 10 words"),
        (json!({ "line": "[JEKT:FROM=x] do it" }), "message marker"),
        (json!({ "line": "Fixing a bug", "state": "thinking" }), "working, blocked, waiting, done"),
    ] {
        let (status, answer) = post(&state, &token, "/api/v1/agent/status", body.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert!(answer["error"].as_str().unwrap().contains(wanted), "{body}: {answer}");
    }
    assert_eq!(kept(&state, &block), None);
}

#[tokio::test]
async fn a_second_line_replaces_the_first_and_a_long_one_is_cut_and_says_so() {
    let (state, token, block) = state_with_agent("ps-twice", true).await;
    post(&state, &token, "/api/v1/agent/status", json!({ "line": "Reviewing PR 6190" })).await;
    assert_eq!(kept(&state, &block).unwrap().text, "Reviewing PR 6190");
    let long = "Writing up how the Swarm phrase can be set by the agent itself with fallbacks and a measurement";
    let (status, body) = post(&state, &token, "/api/v1/agent/status", json!({ "line": long })).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["cut_to_fit"], true);
    let d = kept(&state, &block).unwrap();
    assert!(d.text.ends_with('…') && d.text.chars().count() <= live_phrase::MAX_CHARS, "{:?}", d.text);
}

#[tokio::test]
async fn an_agent_without_a_pane_here_cannot_set_a_line() {
    let (state, token, _block) = state_with_agent("ps-nopane", false).await;
    let (status, body) = post(&state, &token, "/api/v1/agent/status", json!({ "line": "Fixing a bug" })).await;
    assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
    assert!(body["error"].as_str().unwrap().contains("no pane"), "{body}");
}

#[tokio::test]
async fn a_task_title_set_on_a_claim_goes_when_the_task_ends_but_the_agents_own_line_stays() {
    let (state, token, block) = state_with_agent("ps-task", true).await;
    // The claim wrote the task's title.
    agent_status_handlers::task_claimed(&state, &block, "task-20816", Some(20816), "Build: the Swarm phrase");
    wait_for(|| kept(&state, &block).is_some()).await;
    let d = kept(&state, &block).unwrap();
    assert_eq!((d.text.as_str(), d.src.as_str()), ("#20816 Build: the Swarm phrase", "task"));
    // The task ends: a line that only came from the task goes.
    agent_status_handlers::task_ended(&state, "ps-task", "uid-ps-task", "task-20816");
    wait_for(|| kept(&state, &block).is_none()).await;
    // The agent's own line is never cleared by a task ending.
    post(&state, &token, "/api/v1/agent/status", json!({ "line": "Done: PR 6190 merged" })).await;
    agent_status_handlers::task_ended(&state, "ps-task", "uid-ps-task", "task-20816");
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(kept(&state, &block).unwrap().text, "Done: PR 6190 merged");
}

#[tokio::test]
async fn an_old_tasks_end_does_not_clear_the_line_of_the_task_claimed_after_it() {
    let (state, _token, block) = state_with_agent("ps-next", true).await;
    agent_status_handlers::task_claimed(&state, &block, "task-2", Some(2), "Build: the next thing");
    wait_for(|| kept(&state, &block).is_some()).await;
    agent_status_handlers::task_ended(&state, "ps-next", "uid-ps-next", "task-1");
    tokio::time::sleep(std::time::Duration::from_millis(300)).await;
    assert_eq!(kept(&state, &block).unwrap().text, "#2 Build: the next thing");
}

/// The claim and the end write in the background; wait for the store to show it.
async fn wait_for(done: impl Fn() -> bool) {
    for _ in 0..100 {
        if done() {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    }
    panic!("the background write never showed in the store");
}
