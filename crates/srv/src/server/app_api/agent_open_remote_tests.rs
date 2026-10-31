// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `agents.open` for a remote agent: its own tab, never the person's, and the sender's start limit.

use super::*;
use crate::backend::obj::{Client, Tab, Window, Workspace};
use crate::backend::storage::agents::test_agent_def;
use crate::server::tests::test_state;
use agentmux_common::ipc::{Command, Event};

async fn dispatch_apply(state: &AppState, cmd: Command) -> Vec<Event> {
    let evs = crate::server::service::dispatch_to_reducer(state, cmd).await;
    for ev in &evs {
        crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).unwrap();
    }
    evs
}

/// srv with no window, workspace or tab: `test_state` seeds them straight into the store, which the
/// reducer that makes tabs does not know.
fn bare_state() -> AppState {
    let state = test_state();
    let store = &state.mstore;
    for mut client in store.get_all::<Client>().unwrap() {
        client.windowids.clear();
        store.update(&mut client).unwrap();
    }
    for w in store.get_all::<Window>().unwrap() {
        store.delete::<Window>(&w.oid).unwrap();
    }
    for t in store.get_all::<Tab>().unwrap() {
        store.delete::<Tab>(&t.oid).unwrap();
    }
    for w in store.get_all::<Workspace>().unwrap() {
        store.delete::<Workspace>(&w.oid).unwrap();
    }
    state
}

/// A workspace with the person's own tab, which is the one it shows.
async fn persons_workspace(state: &AppState) -> (String, String) {
    let ws = dispatch_apply(state, Command::CreateWorkspace { name: "w".into() })
        .await
        .iter()
        .find_map(|e| match e {
            Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
            _ => None,
        })
        .unwrap();
    let tab = dispatch_apply(state, Command::CreateTab { workspace_id: ws.clone(), name: "Main".into() })
        .await
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap();
    let mut workspace = state.mstore.must_get::<Workspace>(&ws).unwrap();
    workspace.activetabid = tab.clone();
    state.mstore.update(&mut workspace).unwrap();
    (ws, tab)
}

fn tab_name_of(state: &AppState, tab_id: &str) -> String {
    state.mstore.must_get::<Tab>(tab_id).unwrap().name
}

#[tokio::test]
async fn a_callers_tab_is_made_once_without_changing_the_tab_the_person_is_on() {
    let state = bare_state();
    let (ws, persons_tab) = persons_workspace(&state).await;

    let (first, name) = callers_tab(&state, "agenty@narko").await.unwrap();
    assert_eq!(name, "Team (agenty@narko)");
    assert_eq!(tab_name_of(&state, &first), "Team (agenty@narko)");
    let workspace = state.mstore.must_get::<Workspace>(&ws).unwrap();
    assert!(workspace.tabids.contains(&first));
    assert_eq!(workspace.activetabid, persons_tab, "the person's view stays where it is");

    let (again, _) = callers_tab(&state, "agenty@narko").await.unwrap();
    assert_eq!(again, first, "reused");
    let (other, _) = callers_tab(&state, "agentx@narko").await.unwrap();
    assert_ne!(other, first, "each caller has its own");
    assert_eq!(state.mstore.must_get::<Workspace>(&ws).unwrap().tabids.len(), 3);
}

#[tokio::test]
async fn with_no_workspace_the_tab_goes_in_one_made_for_it_and_no_window_is_opened() {
    let state = bare_state();
    assert!(state.mstore.get_all::<Workspace>().unwrap().is_empty());

    let (tab, _) = callers_tab(&state, "agenty@narko").await.unwrap();
    let workspaces = state.mstore.get_all::<Workspace>().unwrap();
    assert_eq!(workspaces.len(), 1);
    assert_eq!(workspaces[0].tabids, vec![tab.clone()]);
    assert_eq!(tab_name_of(&state, &tab), "Team (agenty@narko)");
    assert!(crate::server::service::live_windows(&state.mstore).is_empty(), "nothing is shown or brought forward");
}

#[tokio::test]
async fn an_agent_that_is_not_here_makes_no_tab() {
    let state = bare_state();
    let (_, persons_tab) = persons_workspace(&state).await;
    let (status, message) = run_remote(&state, "agenty@narko", "nobody-here").await.unwrap_err();
    assert_eq!(status, StatusCode::NOT_FOUND, "{message}");
    let tabs = state.mstore.get_all::<Tab>().unwrap();
    assert_eq!(tabs.iter().map(|t| t.oid.as_str()).collect::<Vec<_>>(), vec![persons_tab.as_str()]);
}

#[tokio::test]
async fn a_start_counts_against_the_senders_limit_and_a_refused_one_makes_no_tab() {
    let state = bare_state();
    persons_workspace(&state).await;
    let id = uuid::Uuid::new_v4().to_string();
    let mut def = test_agent_def(&id, &format!("Open {}", &id[..8]), "claude", "host", 1, "");
    state.mstore.agent_def_insert(&mut def).unwrap();
    for _ in 0..crate::server::start_rate::DEFAULT_PER_MINUTE {
        crate::server::start_rate::allow(&state, "remote:limited@narko").unwrap();
    }
    let (status, message) = run_remote(&state, "limited@narko", &id).await.unwrap_err();
    assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{message}");
    assert_eq!(state.mstore.get_all::<Tab>().unwrap().len(), 1, "no tab for a refused open");
}

#[test]
fn an_opens_own_errors_map_to_statuses() {
    assert_eq!(open_status("AGENT_LIVE_ELSEWHERE: x"), StatusCode::CONFLICT);
    assert_eq!(open_status("CLI_NOT_AVAILABLE: x"), StatusCode::BAD_REQUEST);
    assert_eq!(open_status("agent.open: boom"), StatusCode::INTERNAL_SERVER_ERROR);
}
