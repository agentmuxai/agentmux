// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// `Stop agents` closes an agent's tab and `Stop pane` refuses an agent pane
// (docs/specs/SPEC_STOP_AN_AGENT_ONE_WAY_AND_SHUTDOWN_TOAST_2026_10_23.md
// §3.1, §3.2, §3.5). Seeded through the reducer, like `close_pane_tests`.

use super::*;
use crate::server::tests::test_state;
use agentmux_common::api_types::{ClosePaneRequest, UiAutomationAuth};
use agentmux_common::ipc::{Command, Event};
use super::pane::{close_pane_as, handle_close_pane};
use axum::response::IntoResponse;

async fn dispatch_apply(state: &AppState, cmd: Command) -> Vec<Event> {
    let evs = crate::server::service::dispatch_to_reducer(state, cmd).await;
    for ev in &evs {
        crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).unwrap();
    }
    evs
}

/// A workspace, a tab and one plain block (no agent meta).
async fn seed(state: &AppState) -> (String, String) {
    let ws_evs = dispatch_apply(state, Command::CreateWorkspace { name: "w".into() }).await;
    let ws_id = ws_evs
        .iter()
        .find_map(|e| match e {
            Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
            _ => None,
        })
        .unwrap();
    let tab_evs = dispatch_apply(state, Command::CreateTab { workspace_id: ws_id, name: "t".into() }).await;
    let tab_id = tab_evs
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap();
    let blk_evs =
        dispatch_apply(state, Command::CreateBlock { tab_id: tab_id.clone(), meta: serde_json::Value::Null }).await;
    let block_id = blk_evs
        .iter()
        .find_map(|e| match e {
            Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
            _ => None,
        })
        .unwrap();
    (tab_id, block_id)
}

/// Sign a fresh `UiAutomationAuth` for `agent_id`, as agentmux-mcp does.
fn sign_auth(state: &AppState, agent_id: &str) -> UiAutomationAuth {
    let key = state.mstore.agent_jekt_key_ensure(agent_id).unwrap();
    let ts_secs = agentmux_common::time::now_secs();
    let sig = agentmux_common::jekt_sign::sign_jekt(&key, "ui-automation-identity", agent_id, "__srv__", ts_secs, "");
    UiAutomationAuth { agent_id: agent_id.to_string(), ts_secs, sig }
}

async fn body_json(resp: axum::response::Response) -> serde_json::Value {
    let bytes = axum::body::to_bytes(resp.into_body(), usize::MAX).await.unwrap();
    serde_json::from_slice(&bytes).unwrap()
}

/// A block with agent meta, so `is_agent_block` sees an agent pane.
async fn create_agent_block(state: &AppState, tab_id: &str, agent_id: &str) -> String {
    let block = dispatch_apply(
        state,
        Command::CreateBlock {
            tab_id: tab_id.to_string(),
            meta: serde_json::json!({ "view": "agent", "agentId": agent_id }),
        },
    )
    .await
    .iter()
    .find_map(|e| match e {
        Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
        _ => None,
    })
    .unwrap();
    state.reactive_handler.register_agent(agent_id, &block, Some(tab_id)).unwrap();
    block
}

/// Two agents stacked in one pane, `background` behind `visible`.
async fn seed_two_agent_pane(state: &AppState) -> (String, String, String) {
    let (tab_id, _) = seed(state).await;
    let visible = create_agent_block(state, &tab_id, &format!("stack-front-{}", uuid::Uuid::new_v4())).await;
    let background = create_agent_block(state, &tab_id, &format!("stack-back-{}", uuid::Uuid::new_v4())).await;
    dispatch_apply(
        state,
        Command::LayoutSetTree {
            tab_id: tab_id.clone(),
            new_tree: Some(agentmux_common::LayoutNode {
                id: "leaf".into(),
                data: Some(agentmux_common::LayoutNodeData {
                    block_id: visible.clone(),
                    block_stack: vec![visible.clone(), background.clone()],
                    active_block_id: visible.clone(),
                    ..Default::default()
                }),
                ..Default::default()
            }),
            correlation_id: String::new(),
            slices: None,
        },
    )
    .await;
    (tab_id, visible, background)
}

/// `Stop agents` closes the one agent's tab, behind the user's window,
/// and the agent stacked beside it keeps its tab (spec §3.1, §3.5).
#[tokio::test]
async fn stop_agents_closes_one_agent_s_tab_and_leaves_the_agent_beside_it() {
    let state = test_state();
    let (tab_id, visible, background) = seed_two_agent_pane(&state).await;

    let (result, pending, _) = super::fleet::fleet_close_agents_with_override(
        &state,
        "AgentY",
        None,
        vec![background.clone()],
    )
    .await;
    assert!(result.failed.is_empty(), "{:?}", result.failed);
    assert_eq!(pending.len(), 1);
    assert_eq!((pending[0].block_id.as_str(), pending[0].effect, pending[0].by.as_str()), (background.as_str(), "close_tab", "AgentY"));
    assert!(state.srv_state.lock().await.blocks.contains_key(&background), "still there during the window");

    crate::sagas::pending_shutdown::expire_now(&pending[0].request_id);
    assert_eq!(crate::sagas::pending_shutdown::settled(&pending[0].request_id).await.status, "shut_down");

    let s = state.srv_state.lock().await;
    assert!(!s.blocks.contains_key(&background), "its tab is gone");
    assert!(s.blocks.contains_key(&visible), "the agent beside it keeps running");
    let leaf = s.tabs[&tab_id].rootnode.as_ref().expect("the pane stays");
    let members = crate::backend::layout::leaf_members(leaf.data.as_ref().unwrap());
    assert_eq!(members, vec![visible.clone()], "the pane keeps only the other agent");
}

/// Several agents are one window (countdown buttons spec §4): the user keeps one from its own
/// banner and it stays; the other still closes when the window runs out.
#[tokio::test]
async fn stop_agents_is_one_window_and_the_user_can_keep_one() {
    let state = test_state();
    let (_, visible, background) = seed_two_agent_pane(&state).await;
    let (_, pending, _) = super::fleet::fleet_close_agents_with_override(
        &state,
        "AgentY",
        None,
        vec![visible.clone(), background.clone()],
    )
    .await;
    assert_eq!(pending.len(), 2);
    assert_eq!(pending[0].request_id, pending[1].request_id, "two tabs, one window");
    let (keep, close) = if pending[0].block_id == visible { (&pending[0], &pending[1]) } else { (&pending[1], &pending[0]) };
    assert_eq!(crate::sagas::pending_shutdown::keep(&state, &keep.block_id, &keep.request_id), "kept_by_user");
    crate::sagas::pending_shutdown::expire_now(&close.request_id);
    let done = crate::sagas::pending_shutdown::settled(&close.request_id).await;
    assert_eq!(done.status, "shut_down");
    let outcome = |b: &str| done.outcomes.iter().flatten().find(|o| o.block_id == b).map(|o| o.outcome);
    assert_eq!((outcome(&keep.block_id), outcome(&close.block_id)), (Some("kept_by_user"), Some("shut_down")));
    let s = state.srv_state.lock().await;
    assert!(s.blocks.contains_key(&keep.block_id), "the one the user kept");
    assert!(!s.blocks.contains_key(&close.block_id));
}

/// An agent that is already stopped but still has its pane gets the same
/// window and is closed too (spec D2): the case that took two calls.
#[tokio::test]
async fn stop_agents_closes_a_stopped_agent_s_pane_after_the_window() {
    let state = test_state();
    let (tab_id, _) = seed(&state).await;
    let agent = create_agent_block(&state, &tab_id, &format!("stopped-{}", uuid::Uuid::new_v4())).await;
    assert!(crate::backend::blockcontroller::get_controller(&agent).is_none(), "no running session");
    let (_, pending, _) =
        super::fleet::fleet_close_agents_with_override(&state, "AgentY", None, vec![agent.clone()]).await;
    assert_eq!(pending.len(), 1, "a window, not an immediate close");
    crate::sagas::pending_shutdown::expire_now(&pending[0].request_id);
    assert_eq!(crate::sagas::pending_shutdown::settled(&pending[0].request_id).await.status, "shut_down");
    assert!(!state.srv_state.lock().await.blocks.contains_key(&agent));
}

/// Refused before any window opens, each with a pointer (spec §3.4).
#[tokio::test]
async fn stop_agents_refuses_its_own_block_a_non_agent_pane_and_a_missing_block() {
    let state = test_state();
    let (tab_id, plain) = seed(&state).await;
    let own = create_agent_block(&state, &tab_id, &format!("own-{}", uuid::Uuid::new_v4())).await;
    let missing = format!("no-such-block-{}", uuid::Uuid::new_v4());
    let (result, pending, _) = super::fleet::fleet_close_agents_with_override(
        &state,
        "AgentY",
        Some(&own),
        vec![own.clone(), plain.clone(), missing.clone()],
    )
    .await;
    assert!(pending.is_empty(), "nothing waits");
    assert!(result.succeeded.is_empty());
    let error = |id: &str| result.failed.iter().find(|f| f.id == id).map(|f| f.error.clone()).unwrap_or_default();
    assert!(error(&own).contains("Stop with action \"self\""), "{}", error(&own));
    assert!(error(&plain).contains("Stop with action \"pane\""), "{}", error(&plain));
    assert!(error(&missing).starts_with("block not found"), "{}", error(&missing));
    assert!(crate::sagas::pending_shutdown::active_for(&own).is_none());
    assert!(state.srv_state.lock().await.blocks.contains_key(&own), "still open");
}

/// `Stop pane` refuses a pane that holds an agent, and a pane that mixes
/// an agent with something else; no window opens (spec §3.2).
#[tokio::test]
async fn stop_pane_refuses_a_pane_that_holds_an_agent() {
    let state = test_state();
    let (caller_tab, caller_block) = seed(&state).await;
    let caller_id = format!("pane-caller-{}", uuid::Uuid::new_v4());
    state.reactive_handler.register_agent(&caller_id, &caller_block, Some(&caller_tab)).unwrap();
    let (_, visible, background) = seed_two_agent_pane(&state).await;

    let resp = handle_close_pane(
        axum::extract::State(state.clone()),
        None,
        axum::Json(ClosePaneRequest {
            auth: sign_auth(&state, &caller_id),
            block_id: Some(background.clone()),
            reason: None,
        }),
    )
    .await
    .into_response();
    assert_eq!(resp.status(), axum::http::StatusCode::CONFLICT);
    let error = body_json(resp).await["error"].as_str().unwrap_or_default().to_string();
    assert!(error.contains("Stop with action \"agents\""), "{error}");
    assert!(error.contains(&background) && error.contains(&visible), "names the agents in the pane: {error}");
    assert!(crate::sagas::pending_shutdown::active_for(&background).is_none(), "no window opened");
    assert!(state.srv_state.lock().await.blocks.contains_key(&background));
}

/// The `UI run_command pane:close` route shares the refusal: no second way
/// round (spec §3.2).
#[tokio::test]
async fn the_shared_close_refuses_an_agent_pane_for_any_caller() {
    let state = test_state();
    let (_, _, background) = seed_two_agent_pane(&state).await;
    let (code, body) = close_pane_as(&state, "someone", &background, Some("UI command pane:close"), "UI command").await;
    assert_eq!(code, axum::http::StatusCode::CONFLICT);
    assert!(body["error"].as_str().unwrap_or_default().contains("Stop with action \"agents\""));
    assert!(crate::sagas::pending_shutdown::active_for(&background).is_none());
}

/// A caller that doesn't wait for the window (`Stop agents`, `Stop pane`): an agent whose notes can be held.
async fn register_caller(state: &AppState) -> (String, String) {
    let (tab_id, block) = seed(state).await;
    let id = format!("stop-caller-{}", uuid::Uuid::new_v4());
    state.reactive_handler.register_agent(&id, &block, Some(&tab_id)).unwrap();
    (id, block)
}

/// What is held for the caller's next turn, if anything arrives within a moment (the note follows the status).
async fn held_for(state: &AppState, caller: &(String, String), request_id: &str) -> Option<String> {
    let key = format!("shutdown:{request_id}");
    for _ in 0..50 {
        if state.reactive_handler.holds_system_note(&caller.0, &key) {
            let (message, notes) = crate::backend::reactive::held_notes::attach(&caller.1, "next");
            notes.delivered();
            return Some(message);
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    None
}

/// The window is srv's: the call comes back at once, and the user keeping an agent tells the caller by
/// one note, however many of the request's agents were kept.
#[tokio::test]
async fn stop_agents_returns_at_once_and_a_kept_agent_is_one_note_to_the_caller() {
    let state = test_state();
    let caller = register_caller(&state).await;
    let (_, visible, background) = seed_two_agent_pane(&state).await;
    let started = std::time::Instant::now();
    let (_, pending, _) =
        super::fleet::fleet_close_agents_with_override(&state, &caller.0, None, vec![visible.clone(), background.clone()]).await;
    assert!(started.elapsed() < std::time::Duration::from_secs(1), "no wait for the 30 s window");
    let id = pending[0].request_id.clone();
    assert!(pending.iter().all(|p| p.request_id == id), "several closes, one window");
    assert_eq!(crate::sagas::pending_shutdown::keep(&state, &visible, &id), "kept_by_user");
    assert!(!state.reactive_handler.holds_system_note(&caller.0, &format!("shutdown:{id}")), "told when it settles, not before");
    crate::sagas::pending_shutdown::expire_now(&id);
    assert_eq!(crate::sagas::pending_shutdown::settled(&id).await.status, "shut_down");
    let said = held_for(&state, &caller, &id).await.expect("the caller is told the user kept one");
    assert_eq!(said.matches("[AgentMux]").count(), 1, "{said}");
    assert!(said.contains("Stop agents") && said.contains("keep") && said.contains("running"), "{said}");
    assert!(!state.srv_state.lock().await.blocks.contains_key(&background), "the other one closed");
}

/// A close that went ahead as asked says nothing, and neither does asking twice: the second ask joins the first.
#[tokio::test]
async fn stop_agents_says_nothing_when_it_closed_and_asking_twice_shares_the_window() {
    let state = test_state();
    let caller = register_caller(&state).await;
    let (tab_id, _) = seed(&state).await;
    let agent = create_agent_block(&state, &tab_id, &format!("quiet-{}", uuid::Uuid::new_v4())).await;
    let ask = || super::fleet::fleet_close_agents_with_override(&state, &caller.0, None, vec![agent.clone()]);
    let (_, first, _) = ask().await;
    let (_, second, _) = ask().await;
    assert_eq!(first[0].request_id, second[0].request_id, "one window");
    crate::sagas::pending_shutdown::expire_now(&first[0].request_id);
    assert_eq!(crate::sagas::pending_shutdown::settled(&first[0].request_id).await.status, "shut_down");
    assert!(held_for(&state, &caller, &first[0].request_id).await.is_none(), "closed as asked: no note");
}

/// `Stop pane`: 202 at once; kept by the user, the caller gets the one note; a failed close is told too.
#[tokio::test]
async fn stop_pane_returns_at_once_and_a_kept_pane_is_a_note_to_the_caller() {
    let state = test_state();
    let caller = register_caller(&state).await;
    let (_, plain) = seed(&state).await;
    let started = std::time::Instant::now();
    let (code, body) = close_pane_as(&state, &caller.0, &plain, None, "ClosePane").await;
    assert!(started.elapsed() < std::time::Duration::from_secs(1), "no wait for the 30 s window");
    assert_eq!(code, axum::http::StatusCode::ACCEPTED);
    let id = body["request_id"].as_str().unwrap().to_string();
    assert_eq!(crate::sagas::pending_shutdown::keep(&state, &plain, &id), "kept_by_user");
    let said = held_for(&state, &caller, &id).await.expect("the caller is told the user kept it");
    assert!(said.contains("Stop pane") && said.contains("keep"), "{said}");
    assert!(state.srv_state.lock().await.blocks.contains_key(&plain), "still open");
}

/// A caller that asks after its request already ended is told at once rather than never.
#[tokio::test]
async fn a_caller_that_registers_after_the_request_ended_is_still_told() {
    let state = test_state();
    let caller = register_caller(&state).await;
    let (_, plain) = seed(&state).await;
    let (_, body) = close_pane_as(&state, "someone-else", &plain, None, "UI command").await;
    let id = body["request_id"].as_str().unwrap().to_string();
    assert_eq!(crate::sagas::pending_shutdown::keep(&state, &plain, &id), "kept_by_user");
    crate::sagas::pending_shutdown::tell_when_settled(&state, &id, &caller.0, "ClosePane");
    assert!(held_for(&state, &caller, &id).await.is_some());
}
