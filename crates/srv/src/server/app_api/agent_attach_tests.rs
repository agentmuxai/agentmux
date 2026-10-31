// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tests of `agent_attach`: `agent.open` and a start for an agent that already has a block.

use super::*;
use crate::sagas::park_agents_tests::{dispatch_apply, in_tab, seed_agent_block, seed_workspace, set_always_available};
use agentmux_common::ipc::Command;

fn open_cmd(agent: &str, tab: Option<&str>) -> CommandAgentOpenData {
    CommandAgentOpenData {
        agent_id: agent.to_string(),
        tab_id: tab.map(str::to_string),
        split_direction: None,
        split_reference_block_id: None,
        focus: Some(false),
    }
}

/// The agent's block, parked in no tab.
async fn parked_agent(state: &AppState, agent: &str) -> (String, Vec<String>) {
    let (_ws, tabs) = seed_workspace(state, 2).await;
    let block = seed_agent_block(state, &tabs[0], agent).await;
    dispatch_apply(state, Command::ParkBlock { tab_id: tabs[0].clone(), block_id: block.clone() }).await;
    assert_eq!(in_tab(state, &block).await.as_deref(), Some(""));
    (block, tabs)
}

fn blocks_of(state: &AppState, agent: &str) -> Vec<String> {
    state
        .mstore
        .get_all::<Block>()
        .unwrap()
        .into_iter()
        .filter(|b| obj::meta_get_string(&b.meta, "agentId", "") == agent)
        .map(|b| b.oid)
        .collect()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn opening_an_agent_with_a_parked_block_attaches_it_and_makes_no_second_block() {
    let state = crate::server::tests::test_state();
    let (block, tabs) = parked_agent(&state, "ag-open").await;

    let opened = open_agent_impl(&state, open_cmd("ag-open", Some(&tabs[1]))).await.unwrap();

    assert_eq!((opened.block_id.as_str(), opened.tab_id.as_str(), opened.created), (block.as_str(), tabs[1].as_str(), false));
    assert_eq!(blocks_of(&state, "ag-open"), vec![block.clone()], "still one block for the agent");
    assert_eq!(in_tab(&state, &block).await.as_deref(), Some(tabs[1].as_str()), "shown in the tab it was opened in");
    let stored = state.mstore.get::<Block>(&block).unwrap().unwrap();
    assert!(!obj::meta_get_bool(&stored.meta, crate::persist_subscriber::parked::PARKED_META_KEY, false));
    assert_eq!(agent_attach::desired_of(&stored).as_deref(), Some("running"));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn opening_an_agent_shown_in_another_tab_focuses_that_pane_with_the_setting_on() {
    let state = crate::server::tests::test_state();
    set_always_available(&state, true);
    let (_ws, tabs) = seed_workspace(&state, 2).await;
    let block = seed_agent_block(&state, &tabs[0], "ag-shown").await;

    let opened = open_agent_impl(&state, open_cmd("ag-shown", Some(&tabs[1]))).await.unwrap();

    assert_eq!((opened.block_id.as_str(), opened.tab_id.as_str(), opened.created), (block.as_str(), tabs[0].as_str(), false));
    assert_eq!(blocks_of(&state, "ag-shown"), vec![block.clone()], "no second block in the other tab");
    assert_eq!(in_tab(&state, &block).await.as_deref(), Some(tabs[0].as_str()));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_start_leaves_a_parked_block_parked_and_wants_it_running() {
    let state = crate::server::tests::test_state();
    let (block, _tabs) = parked_agent(&state, "ag-start").await;

    let started = agent_attach::start_agent(&state, "ag-start").await.unwrap();

    assert_eq!(started, block);
    assert_eq!(in_tab(&state, &block).await.as_deref(), Some(""), "no pane was made");
    let stored = state.mstore.get::<Block>(&block).unwrap().unwrap();
    assert_eq!(agent_attach::desired_of(&stored).as_deref(), Some("running"));
    assert!(obj::meta_get_bool(&stored.meta, crate::persist_subscriber::parked::PARKED_META_KEY, false));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pane_being_filled_in_never_takes_a_block_from_elsewhere() {
    let state = crate::server::tests::test_state();
    let (_block, tabs) = parked_agent(&state, "ag-fill").await;
    let agent = state.mstore.agent_def_get("ag-fill").unwrap().unwrap();
    let provider = providers::get_provider("claude").unwrap();

    let reused = agent_attach::reuse_existing_pane(&state, &agent, provider, &open_cmd("ag-fill", Some(&tabs[1])), &tabs[1], false, false)
        .await
        .unwrap();

    assert!(reused.is_none(), "layout import fills its own placeholder");
}

#[test]
fn a_container_agents_pane_runs_a_subprocess_controller_whatever_its_provider_defaults_to() {
    let provider = providers::get_provider("claude").unwrap();
    let def = |agent_type: &str| crate::backend::storage::agents::test_agent_def("a1", "A One", "claude", agent_type, 1, "");
    assert_eq!(agent_attach::controller_type_of(&def("container"), provider), "subprocess");
    assert_eq!(agent_attach::controller_type_of(&def("host"), provider), provider.controller_type_str());
}

#[test]
fn the_desired_state_is_block_meta_and_a_block_without_it_has_none() {
    let mut block = Block::default();
    assert_eq!(agent_attach::desired_of(&block), None);
    block.meta.insert(agent_attach::META_DESIRED.to_string(), serde_json::json!("stopped"));
    assert_eq!(agent_attach::desired_of(&block).as_deref(), Some("stopped"));
}

/// The last workspace deleted leaves the agents' blocks parked and no tab to put a pane in: a start still
/// runs such an agent, and one with no block yet still needs a tab to make it.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_start_runs_a_parked_agent_when_no_workspace_is_left() {
    let state = crate::server::tests::test_state();
    set_always_available(&state, true);
    let (ws, tabs) = seed_workspace(&state, 1).await;
    let block = seed_agent_block(&state, &tabs[0], "ag-nows").await;
    crate::sagas::delete_workspace::run(&state, ws).await.unwrap();
    assert_eq!(in_tab(&state, &block).await.as_deref(), Some(""));
    // The store keeps no workspace or tab either, as after the delete persists.
    for ws in state.mstore.get_all::<crate::backend::obj::Workspace>().unwrap() {
        state.mstore.delete::<crate::backend::obj::Workspace>(&ws.oid).unwrap();
    }
    for tab in state.mstore.get_all::<crate::backend::obj::Tab>().unwrap() {
        state.mstore.delete::<crate::backend::obj::Tab>(&tab.oid).unwrap();
    }
    assert!(super::resolve_tab_id(&state.mstore, None).is_err(), "no tab is left");

    let started = agent_attach::start_agent(&state, "ag-nows").await.unwrap();

    assert_eq!(started, block);
    let stored = state.mstore.get::<Block>(&block).unwrap().unwrap();
    assert_eq!(agent_attach::desired_of(&stored).as_deref(), Some("running"));
    assert!(agent_attach::start_agent(&state, "no-such-agent").await.is_err());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_start_of_a_terminal_tier_cli_is_refused_and_leaves_no_block() {
    let state = crate::server::tests::test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    let mut def: crate::backend::storage::AgentDefinition = serde_json::from_value(serde_json::json!({
        "id": "ag-term", "slug": "ag-term", "name": "ag-term", "icon": "robot", "provider": "aider", "description": "", "created_at": 1,
    }))
    .unwrap();
    state.mstore.agent_def_insert(&mut def).unwrap();
    assert!(!tabs.is_empty());

    let err = agent_attach::start_agent(&state, "ag-term").await.unwrap_err();

    assert!(err.contains("no headless mode"), "{err}");
    assert!(blocks_of(&state, "ag-term").is_empty(), "no block was made");
}
