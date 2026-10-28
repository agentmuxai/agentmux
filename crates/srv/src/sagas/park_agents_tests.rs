// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tests of `park_agents`: what the close paths do to an agent's block with `agents:always_available`
// off and on.

use super::park_agents::*;
use super::SagaCtx;
use crate::backend::storage::agents::{AgentInstance, InstanceStatus};
use crate::server::tests::test_state;
use crate::server::AppState;
use agentmux_common::ipc::{Command, Event};

pub(crate) async fn dispatch_apply(state: &AppState, cmd: Command) -> Vec<Event> {
    let events = crate::server::service::dispatch_to_reducer(state, cmd).await;
    for ev in &events {
        crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).unwrap();
    }
    events
}

pub(crate) async fn seed_workspace(state: &AppState, tabs: usize) -> (String, Vec<String>) {
    let ws = dispatch_apply(state, Command::CreateWorkspace { name: "w".into() })
        .await
        .iter()
        .find_map(|e| match e {
            Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
            _ => None,
        })
        .unwrap();
    let mut ids = Vec::new();
    for _ in 0..tabs {
        let tab = dispatch_apply(state, Command::CreateTab { workspace_id: ws.clone(), name: "t".into() })
            .await
            .iter()
            .find_map(|e| match e {
                Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
                _ => None,
            })
            .unwrap();
        ids.push(tab);
    }
    (ws, ids)
}

pub(crate) async fn seed_block(state: &AppState, tab_id: &str, meta: serde_json::Value) -> String {
    dispatch_apply(state, Command::CreateBlock { tab_id: tab_id.to_string(), meta })
        .await
        .iter()
        .find_map(|e| match e {
            Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
            _ => None,
        })
        .unwrap()
}

/// An existing agent `id` with a pane block in `tab_id`.
pub(crate) async fn seed_agent_block(state: &AppState, tab_id: &str, id: &str) -> String {
    let block = seed_block(state, tab_id, serde_json::json!({ "view": "agent", "agentId": id })).await;
    let mut def: crate::backend::storage::AgentDefinition = serde_json::from_value(serde_json::json!({
        "id": id, "slug": id, "name": id, "icon": "robot", "provider": "claude", "description": "", "created_at": 1,
    }))
    .unwrap();
    state.mstore.agent_def_insert(&mut def).unwrap();
    state
        .mstore
        .instance_create(&AgentInstance {
            id: id.to_string(),
            definition_id: id.to_string(),
            parent_instance_id: String::new(),
            block_id: block.clone(),
            session_id: "sess".to_string(),
            status: InstanceStatus::Running.as_str().to_string(),
            github_context: String::new(),
            started_at: 1000,
            ended_at: 0,
            created_at: 1000,
            identity_id: String::new(),
            memory_id: String::new(),
            instance_name: String::new(),
            working_directory: String::new(),
            display_hidden: false,
        })
        .unwrap();
    block
}

pub(crate) fn set_always_available(state: &AppState, on: bool) {
    state.config_watcher.update_settings(
        serde_json::from_value(serde_json::json!({ crate::backend::always_available::SETTING_ALWAYS_AVAILABLE: on })).unwrap(),
    );
}

/// The tab the reducer has the block in (empty when parked); `None` when it has no such block.
pub(crate) async fn in_tab(state: &AppState, block_id: &str) -> Option<String> {
    state.srv_state.lock().await.blocks.get(block_id).map(|b| b.tab_id.clone())
}

#[test]
fn what_is_left_to_close_is_what_was_not_parked() {
    let all = vec!["a".to_string(), "b".to_string(), "c".to_string()];
    assert_eq!(not_parked(&all, &["b".to_string()]), vec!["a".to_string(), "c".to_string()]);
    assert_eq!(not_parked(&all, &[]), all);
}

#[tokio::test]
async fn only_an_existing_agents_pane_block_is_parkable() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    let agent = seed_agent_block(&state, &tabs[0], "ag-1").await;
    let term = seed_block(&state, &tabs[0], serde_json::json!({ "view": "term" })).await;
    let deleted = seed_block(&state, &tabs[0], serde_json::json!({ "view": "agent", "agentId": "gone" })).await;
    assert!(is_parkable(&state, &agent));
    assert!(!is_parkable(&state, &term), "a terminal is not an agent");
    assert!(!is_parkable(&state, &deleted), "an agent that no longer exists has no one to open it");
    assert!(!is_parkable(&state, "no-such-block"));
}

/// A block launched from a template keeps the template's id in `agentId` and binds its agent through
/// `agentInstanceId`: the agent is found through the binding, not by that id.
#[tokio::test]
async fn a_template_launched_agents_block_is_parkable_through_its_binding() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    let block = seed_block(
        &state,
        &tabs[0],
        serde_json::json!({ "view": "agent", "agentId": "tmpl-1", "agentInstanceId": "inst-1" }),
    )
    .await;
    assert!(!is_parkable(&state, &block), "no agent is bound yet");

    let mut def: crate::backend::storage::AgentDefinition = serde_json::from_value(serde_json::json!({
        "id": "inst-1", "slug": "inst-1", "name": "inst-1", "icon": "robot", "provider": "claude", "description": "", "created_at": 1,
    }))
    .unwrap();
    state.mstore.agent_def_insert(&mut def).unwrap();
    state
        .mstore
        .instance_create(&AgentInstance {
            id: "inst-1".to_string(),
            definition_id: "inst-1".to_string(),
            parent_instance_id: String::new(),
            block_id: block.clone(),
            session_id: "sess".to_string(),
            status: InstanceStatus::Running.as_str().to_string(),
            github_context: String::new(),
            started_at: 1000,
            ended_at: 0,
            created_at: 1000,
            identity_id: String::new(),
            memory_id: String::new(),
            instance_name: String::new(),
            working_directory: String::new(),
            display_hidden: false,
        })
        .unwrap();
    assert!(is_parkable(&state, &block), "its bound agent exists");
}

#[tokio::test]
async fn closing_parks_an_agents_block_and_nothing_else_with_the_setting_on() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    let agent = seed_agent_block(&state, &tabs[0], "ag-1").await;
    let term = seed_block(&state, &tabs[0], serde_json::json!({ "view": "term" })).await;
    let ids = vec![agent.clone(), term.clone()];

    set_always_available(&state, false);
    let ctx = SagaCtx::new(&state, crate::sagas::alloc_saga_id(&state));
    assert!(park_agent_blocks(&ctx, &ids, false).await.is_empty(), "off: nothing is parked");
    assert_eq!(in_tab(&state, &agent).await.as_deref(), Some(tabs[0].as_str()));

    set_always_available(&state, true);
    let parked = park_agent_blocks(&ctx, &ids, false).await;
    assert_eq!(parked, vec![agent.clone()]);
    assert_eq!(in_tab(&state, &agent).await.as_deref(), Some(""), "kept, in no tab");
    assert_eq!(in_tab(&state, &term).await.as_deref(), Some(tabs[0].as_str()), "the terminal is closed as before");
    assert!(state.mstore.get::<crate::backend::obj::Block>(&agent).unwrap().is_some());
}

/// A detach that cannot park a block does not close it: no stop, no delete.
#[tokio::test]
async fn an_explicit_park_that_cannot_park_closes_nothing() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    let term = seed_block(&state, &tabs[0], serde_json::json!({ "view": "term" })).await;
    let opts = crate::sagas::close_pane::CloseOpts { frontend_waits: true, park_agents: true };
    let err = crate::sagas::close_pane::run_with(&state, vec![term.clone()], opts).await.unwrap_err();
    assert!(err.contains("could not park"), "{err}");
    assert_eq!(in_tab(&state, &term).await.as_deref(), Some(tabs[0].as_str()), "still in its pane");
    assert!(state.mstore.get::<crate::backend::obj::Block>(&term).unwrap().is_some());
}

/// An explicit detach parks the agent's block with the setting off, and still only an agent's.
#[tokio::test]
async fn a_forced_park_keeps_an_agents_block_with_the_setting_off() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    let agent = seed_agent_block(&state, &tabs[0], "ag-force").await;
    let term = seed_block(&state, &tabs[0], serde_json::json!({ "view": "term" })).await;
    let ctx = SagaCtx::new(&state, crate::sagas::alloc_saga_id(&state));
    let parked = park_agent_blocks(&ctx, &[agent.clone(), term.clone()], true).await;
    assert_eq!(parked, vec![agent.clone()]);
    assert_eq!(in_tab(&state, &agent).await.as_deref(), Some(""));
    assert_eq!(in_tab(&state, &term).await.as_deref(), Some(tabs[0].as_str()));
}

#[tokio::test]
async fn a_closed_pane_keeps_its_agents_block_with_the_setting_on_and_deletes_it_off() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;

    set_always_available(&state, false);
    let off = seed_agent_block(&state, &tabs[0], "ag-off").await;
    crate::sagas::close_pane::run(&state, vec![off.clone()]).await.unwrap();
    assert_eq!(in_tab(&state, &off).await, None, "off: deleted, as always");

    set_always_available(&state, true);
    let on = seed_agent_block(&state, &tabs[0], "ag-on").await;
    let term = seed_block(&state, &tabs[0], serde_json::json!({ "view": "term" })).await;
    crate::sagas::close_pane::run(&state, vec![on.clone()]).await.unwrap();
    crate::sagas::close_pane::run(&state, vec![term.clone()]).await.unwrap();
    assert_eq!(in_tab(&state, &on).await.as_deref(), Some(""), "on: parked");
    assert_eq!(in_tab(&state, &term).await, None);
    let tab = state.srv_state.lock().await.tabs.get(&tabs[0]).cloned().unwrap();
    assert!(!tab.block_ids.contains(&on), "the tab no longer holds it");
}

#[tokio::test]
async fn a_deleted_tab_keeps_its_agents_blocks_with_the_setting_on() {
    let state = test_state();
    let (ws, tabs) = seed_workspace(&state, 2).await;
    set_always_available(&state, true);
    let agent = seed_agent_block(&state, &tabs[0], "ag-tab").await;
    let term = seed_block(&state, &tabs[0], serde_json::json!({ "view": "term" })).await;

    crate::sagas::delete_tab::run(&state, ws, tabs[0].clone()).await.unwrap();

    assert_eq!(in_tab(&state, &agent).await.as_deref(), Some(""));
    assert_eq!(in_tab(&state, &term).await, None);
    assert!(state.mstore.get::<crate::backend::obj::Block>(&agent).unwrap().is_some(), "the row outlives its tab");
    assert!(state.mstore.get::<crate::backend::obj::Block>(&term).unwrap().is_none());
    assert!(!state.srv_state.lock().await.tabs.contains_key(&tabs[0]));
}

#[tokio::test]
async fn a_deleted_workspace_keeps_its_agents_blocks_with_the_setting_on() {
    let state = test_state();
    let (ws, tabs) = seed_workspace(&state, 2).await;
    set_always_available(&state, true);
    let a = seed_agent_block(&state, &tabs[0], "ag-ws-1").await;
    let b = seed_agent_block(&state, &tabs[1], "ag-ws-2").await;

    crate::sagas::delete_workspace::run(&state, ws.clone()).await.unwrap();

    assert_eq!(in_tab(&state, &a).await.as_deref(), Some(""));
    assert_eq!(in_tab(&state, &b).await.as_deref(), Some(""));
    assert!(!state.srv_state.lock().await.workspaces.contains_key(&ws));
}

#[tokio::test]
async fn deleting_the_agent_deletes_its_parked_block_and_only_that() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    set_always_available(&state, true);
    let parked = seed_agent_block(&state, &tabs[0], "ag-del").await;
    let shown = seed_agent_block(&state, &tabs[0], "ag-shown").await;
    crate::sagas::close_pane::run(&state, vec![parked.clone()]).await.unwrap();
    assert_eq!(in_tab(&state, &parked).await.as_deref(), Some(""));

    assert_eq!(delete_parked_blocks_of(&state, "ag-shown").await, 0, "a block shown in a tab is not parked");
    assert_eq!(delete_parked_blocks_of(&state, "ag-del").await, 1);

    assert_eq!(in_tab(&state, &parked).await, None);
    assert!(state.mstore.get::<crate::backend::obj::Block>(&parked).unwrap().is_none());
    assert_eq!(in_tab(&state, &shown).await.as_deref(), Some(tabs[0].as_str()));
}

/// Deleting a template must not take the parked blocks of the agents launched from it: they keep the
/// template's id in `agentId`, and only the bound instance names the agent.
#[tokio::test]
async fn deleting_a_template_leaves_the_parked_blocks_of_agents_launched_from_it() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    set_always_available(&state, true);
    let block = seed_block(
        &state,
        &tabs[0],
        serde_json::json!({ "view": "agent", "agentId": "tmpl-2", "agentInstanceId": "inst-2" }),
    )
    .await;
    let mut def: crate::backend::storage::AgentDefinition = serde_json::from_value(serde_json::json!({
        "id": "inst-2", "slug": "inst-2", "name": "inst-2", "icon": "robot", "provider": "claude", "description": "", "created_at": 1,
    }))
    .unwrap();
    state.mstore.agent_def_insert(&mut def).unwrap();
    state
        .mstore
        .instance_create(&AgentInstance {
            id: "inst-2".to_string(),
            definition_id: "inst-2".to_string(),
            parent_instance_id: String::new(),
            block_id: block.clone(),
            session_id: "sess".to_string(),
            status: InstanceStatus::Running.as_str().to_string(),
            github_context: String::new(),
            started_at: 1000,
            ended_at: 0,
            created_at: 1000,
            identity_id: String::new(),
            memory_id: String::new(),
            instance_name: String::new(),
            working_directory: String::new(),
            display_hidden: false,
        })
        .unwrap();
    crate::sagas::close_pane::run(&state, vec![block.clone()]).await.unwrap();
    assert_eq!(in_tab(&state, &block).await.as_deref(), Some(""), "parked");

    assert_eq!(delete_parked_blocks_of(&state, "tmpl-2").await, 0, "the template's id does not name the agent");
    assert_eq!(in_tab(&state, &block).await.as_deref(), Some(""));
    assert_eq!(delete_parked_blocks_of(&state, "inst-2").await, 1);
    assert_eq!(in_tab(&state, &block).await, None);
}

#[tokio::test]
async fn a_process_that_exits_keeps_its_agents_block_with_the_setting_on() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    let agent = seed_agent_block(&state, &tabs[0], "ag-exit").await;
    let term = seed_block(&state, &tabs[0], serde_json::json!({ "view": "term" })).await;
    set_always_available(&state, false);
    assert!(!keeps_block(&state, &agent), "off");
    set_always_available(&state, true);
    assert!(keeps_block(&state, &agent));
    assert!(!keeps_block(&state, &term), "a terminal still closes on exit");
}

/// An old layout with two blocks for one agent: closing the one that does not run it closes it (as
/// before), and closing the only block parks it.
#[tokio::test]
async fn a_second_block_of_the_same_agent_is_closed_not_parked() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    set_always_available(&state, true);
    let first = seed_agent_block(&state, &tabs[0], "ag-dup").await;
    let second = seed_block(&state, &tabs[0], serde_json::json!({ "view": "agent", "agentId": "ag-dup" })).await;
    let ctx = SagaCtx::new(&state, crate::sagas::alloc_saga_id(&state));

    assert!(park_agent_blocks(&ctx, &[second.clone()], false).await.is_empty(), "the agent has another block");
    assert_eq!(in_tab(&state, &second).await.as_deref(), Some(tabs[0].as_str()), "left for the close to delete");

    let parked = park_agent_blocks(&ctx, &[first.clone()], true).await;
    assert_eq!(parked, vec![first], "an explicit detach parks what it is asked to");
}

/// The block that runs the agent's process is the one kept.
#[tokio::test]
async fn of_two_blocks_of_one_agent_the_one_with_the_process_is_parked() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    set_always_available(&state, true);
    let idle = seed_agent_block(&state, &tabs[0], "ag-dup2").await;
    let running = seed_block(&state, &tabs[0], serde_json::json!({ "view": "agent", "agentId": "ag-dup2" })).await;
    crate::backend::agent_state::test_support::register_quiet(&running, 1000);
    let ctx = SagaCtx::new(&state, crate::sagas::alloc_saga_id(&state));

    assert_eq!(park_agent_blocks(&ctx, &[running.clone()], false).await, vec![running.clone()]);
    assert!(park_agent_blocks(&ctx, &[idle.clone()], false).await.is_empty(), "the other is closed as before");
    crate::backend::blockcontroller::delete_controller(&running);
}

/// A running agent whose block has no desired state (opened before it existed) is wanted running once
/// its pane is closed; one that is not running is left alone.
#[tokio::test]
async fn closing_the_pane_of_a_running_agent_records_it_as_wanted_running() {
    use crate::server::app_api::agent_attach::desired_of;
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    set_always_available(&state, true);
    let running = seed_agent_block(&state, &tabs[0], "ag-run").await;
    crate::backend::agent_state::test_support::register_quiet(&running, 1000);
    let exited = seed_agent_block(&state, &tabs[0], "ag-exited").await;
    let ctx = SagaCtx::new(&state, crate::sagas::alloc_saga_id(&state));

    park_agent_blocks(&ctx, &[running.clone(), exited.clone()], false).await;

    let get = |id: &str| desired_of(&state.mstore.get::<crate::backend::obj::Block>(id).unwrap().unwrap());
    assert_eq!(get(&running).as_deref(), Some("running"));
    assert_eq!(get(&exited), None);
    crate::backend::blockcontroller::delete_controller(&running);
}

fn pair(block: &str, agent: &str) -> (String, String) {
    (block.to_string(), agent.to_string())
}

/// One close covering both blocks of an agent keeps one of them; with a sibling outside the close, the
/// running block of the two is the survivor.
#[test]
fn a_close_keeps_one_block_of_each_agent() {
    let none_runs = |_: &str| false;
    let all = vec![pair("a1", "A"), pair("a2", "A"), pair("b1", "B")];

    let both = vec![pair("a1", "A"), pair("a2", "A"), pair("b1", "B")];
    assert_eq!(extras_to_close(&both, &all, &none_runs), ["a2".to_string()].into(), "neither runs: the first is kept");

    let a2_runs = |id: &str| id == "a2";
    assert_eq!(extras_to_close(&both, &all, &a2_runs), ["a1".to_string()].into(), "the running one is kept");

    let both_run = |id: &str| id == "a1" || id == "a2";
    assert_eq!(extras_to_close(&both, &all, &both_run), ["a2".to_string()].into(), "both run: one is kept");

    let only_a1 = vec![pair("a1", "A")];
    assert_eq!(extras_to_close(&only_a1, &all, &none_runs), ["a1".to_string()].into(), "a sibling outside stays: this one is extra");
    assert_eq!(extras_to_close(&only_a1, &all, &|id: &str| id == "a1"), Default::default(), "unless it runs and the sibling does not");
    assert_eq!(extras_to_close(&only_a1, &all, &a2_runs), ["a1".to_string()].into(), "the sibling that runs stays");

    let alone = vec![pair("b1", "B")];
    assert_eq!(extras_to_close(&alone, &all, &none_runs), Default::default(), "an agent's only block is parked");
}

/// A tab or workspace close that covers both blocks of an agent parks exactly one.
#[tokio::test]
async fn closing_both_blocks_of_an_agent_in_one_go_parks_one() {
    let state = test_state();
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    set_always_available(&state, true);
    let first = seed_agent_block(&state, &tabs[0], "ag-both").await;
    let second = seed_block(&state, &tabs[0], serde_json::json!({ "view": "agent", "agentId": "ag-both" })).await;
    let ctx = SagaCtx::new(&state, crate::sagas::alloc_saga_id(&state));

    let parked = park_agent_blocks(&ctx, &[first.clone(), second.clone()], false).await;

    assert_eq!(parked.len(), 1, "one block of the agent survives the close: {parked:?}");
}
