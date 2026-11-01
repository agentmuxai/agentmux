// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// An agent has one pane (task #5419): every path that would add an agent pane reuses the one it has, and a
// layout that already holds two is reconciled to one.

use super::*;
use crate::sagas::park_agents_tests::{in_tab, seed_agent_block, seed_block, seed_workspace, set_always_available};

fn open_cmd(agent: &str, tab: Option<&str>) -> CommandAgentOpenData {
    CommandAgentOpenData { agent_id: agent.to_string(), tab_id: tab.map(str::to_string), split_direction: None, split_reference_block_id: None, focus: Some(false) }
}

fn blocks_of(state: &AppState, agent: &str) -> Vec<String> {
    let mut blocks: Vec<String> = state
        .mstore
        .get_all::<Block>()
        .unwrap()
        .into_iter()
        .filter(|b| obj::meta_get_string(&b.meta, "agentId", "") == agent)
        .map(|b| b.oid)
        .collect();
    blocks.sort();
    blocks
}

/// Agent `agent` with its pane in the first of two tabs, and a second block of it in the second: a layout an
/// older build restored.
async fn two_panes(state: &AppState, agent: &str) -> (String, String, Vec<String>) {
    let (_ws, tabs) = seed_workspace(state, 2).await;
    let first = seed_agent_block(state, &tabs[0], agent).await;
    let second = seed_block(state, &tabs[1], serde_json::json!({ "view": "agent", "agentId": agent })).await;
    (first, second, tabs)
}

/// A window shows workspace `ws`, so its tabs are on screen.
fn show_workspace(state: &AppState, ws: &str) {
    let mut window = obj::Window { oid: uuid::Uuid::new_v4().to_string(), workspaceid: ws.to_string(), ..Default::default() };
    state.mstore.insert(&mut window).unwrap();
    let mut client = state.mstore.get_all::<obj::Client>().unwrap().into_iter().next().unwrap_or_else(|| {
        let mut c = obj::Client { oid: uuid::Uuid::new_v4().to_string(), ..Default::default() };
        state.mstore.insert(&mut c).unwrap();
        c
    });
    client.windowids.push(window.oid);
    state.mstore.update(&mut client).unwrap();
}

// ---- the paths that add an agent pane ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_open_reuses_a_pane_in_another_tab_even_with_always_available_off() {
    let state = crate::server::tests::test_state();
    set_always_available(&state, false);
    let (_ws, tabs) = seed_workspace(&state, 2).await;
    let block = seed_agent_block(&state, &tabs[0], "ag-off").await;

    let opened = open_agent_impl(&state, open_cmd("ag-off", Some(&tabs[1]))).await.unwrap();

    assert_eq!((opened.block_id.as_str(), opened.tab_id.as_str(), opened.created), (block.as_str(), tabs[0].as_str(), false));
    assert_eq!(blocks_of(&state, "ag-off"), vec![block], "no second pane in the other tab");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn agent_open_reuses_the_pane_the_agent_last_launched_when_old_layouts_left_two() {
    let state = crate::server::tests::test_state();
    let (first, second, tabs) = two_panes(&state, "ag-two").await;
    state.mstore.instance_record_launch("ag-two", &second, 5000).unwrap();

    let opened = open_agent_impl(&state, open_cmd("ag-two", Some(&tabs[0]))).await.unwrap();

    assert_eq!(opened.block_id, second, "the newest pane, not the one in the tab asked for ({first} is the other)");
    assert_eq!(blocks_of(&state, "ag-two").len(), 2, "nothing was made or removed");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_start_with_a_pane_in_a_tab_makes_no_second_block() {
    let state = crate::server::tests::test_state();
    set_always_available(&state, true);
    let (_ws, tabs) = seed_workspace(&state, 1).await;
    let block = seed_agent_block(&state, &tabs[0], "ag-st").await;

    let started = agent_attach::start_agent(&state, "ag-st").await.unwrap();

    assert_eq!(started, block);
    assert_eq!(blocks_of(&state, "ag-st"), vec![block.clone()]);
    assert_eq!(in_tab(&state, &block).await.as_deref(), Some(tabs[0].as_str()), "the pane stays where it is");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_layout_import_into_a_placeholder_keeps_the_agents_pane_elsewhere_and_makes_no_second() {
    let state = crate::server::tests::test_state();
    let (ws, tabs) = seed_workspace(&state, 2).await;
    show_workspace(&state, &ws);
    let block = seed_agent_block(&state, &tabs[0], "ag-imp").await;
    let placeholder = seed_block(&state, &tabs[1], serde_json::json!({ "view": "agent" })).await;

    let opened = agent_open::open_agent_into_block(&state, "ag-imp", &tabs[1], &placeholder).await.unwrap();

    assert_eq!((opened.block_id.as_str(), opened.tab_id.as_str(), opened.created), (block.as_str(), tabs[0].as_str(), false));
    let stored = state.mstore.get::<Block>(&placeholder).unwrap().unwrap();
    assert_eq!(obj::meta_get_string(&stored.meta, "agentId", ""), "", "the placeholder is still a picker");
    assert_eq!(blocks_of(&state, "ag-imp"), vec![block]);
}

/// A pane in a workspace no window shows is saved contents (the one an import replaced): the import opens the
/// agent in its own pane, and the reconciler then closes the saved one rather than the one on screen.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_pane_in_a_closed_workspace_is_not_kept_over_an_imports_own() {
    let state = crate::server::tests::test_state();
    let (_old_ws, old_tabs) = seed_workspace(&state, 1).await;
    let (new_ws, new_tabs) = seed_workspace(&state, 1).await;
    show_workspace(&state, &new_ws);
    let saved = seed_agent_block(&state, &old_tabs[0], "ag-saved").await;
    let agent = state.mstore.agent_def_get("ag-saved").unwrap().unwrap();
    let provider = providers::get_provider("claude").unwrap();

    let reused = agent_attach::reuse_existing_pane(&state, &agent, provider, &open_cmd("ag-saved", Some(&new_tabs[0])), &new_tabs[0], false, Some("placeholder")).await.unwrap();
    assert!(reused.is_none(), "the import fills its placeholder");

    // The import started it there; the saved pane was launched first, so the on-screen one ranks first anyway.
    let on_screen = seed_block(&state, &new_tabs[0], serde_json::json!({ "view": "agent", "agentId": "ag-saved" })).await;
    let found = agent_reconcile::find_duplicates(&state.mstore).unwrap();
    assert_eq!(found.iter().map(|d| (d.block_id.as_str(), d.keeper.as_str())).collect::<Vec<_>>(), vec![(saved.as_str(), on_screen.as_str())]);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn an_agents_open_that_names_no_tab_goes_to_the_callers_own_tab() {
    let state = crate::server::tests::test_state();
    let (_ws, tabs) = seed_workspace(&state, 2).await;
    seed_agent_block(&state, &tabs[1], "ag-leader").await;
    let caller = crate::server::caller::Caller::Agent { uid: "ag-leader".to_string() };

    let mut cmd = open_cmd("ag-other", None);
    agent_block::default_to_callers_tab(&state.mstore, Some(&caller), &mut cmd);
    assert_eq!(cmd.tab_id.as_deref(), Some(tabs[1].as_str()));

    let mut named = open_cmd("ag-other", Some(&tabs[0]));
    agent_block::default_to_callers_tab(&state.mstore, Some(&caller), &mut named);
    assert_eq!(named.tab_id.as_deref(), Some(tabs[0].as_str()), "a tab the caller names is kept");

    let mut by_ui = open_cmd("ag-other", None);
    agent_block::default_to_callers_tab(&state.mstore, Some(&crate::server::caller::Caller::Unattributed), &mut by_ui);
    assert_eq!(by_ui.tab_id, None, "a person's open keeps the active tab");
}

// ---- the reconciler ----

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn a_restored_layout_with_two_panes_of_one_agent_keeps_the_newest_and_closes_the_other() {
    let state = crate::server::tests::test_state();
    let (first, second, _tabs) = two_panes(&state, "ag-rec").await;
    state.mstore.instance_record_launch("ag-rec", &second, 5000).unwrap();

    let found = agent_reconcile::find_duplicates(&state.mstore).unwrap();
    assert_eq!(found.iter().map(|d| (d.block_id.as_str(), d.keeper.as_str())).collect::<Vec<_>>(), vec![(first.as_str(), second.as_str())]);

    assert_eq!(agent_reconcile::reconcile_duplicate_panes(&state), 1);

    let closed = state.mstore.get::<Block>(&first).unwrap().unwrap();
    let failure = closed.meta.get(blockcontroller::core::META_LAST_FAILURE).expect("the duplicate says why it is closed");
    assert_eq!(failure["code"], "open_in_other_pane");
    assert_eq!(failure["holderBlockId"], second.as_str());
    assert!(failure["title"].as_str().unwrap().contains("moved to"), "{failure}");
    assert_eq!(agent_attach::desired_of(&closed).as_deref(), Some("stopped"));
    let kept = state.mstore.get::<Block>(&second).unwrap().unwrap();
    assert!(kept.meta.get(blockcontroller::core::META_LAST_FAILURE).is_none(), "the kept pane is untouched");
    assert!(
        crate::backend::reactive::get_global_handler().get_audit_log(500).iter().any(|e| e.outcome.as_deref() == Some(agent_reconcile::AUDIT_COMMAND) && e.block_id == first),
        "Warden's Audit has the entry"
    );

    assert_eq!(agent_reconcile::reconcile_duplicate_panes(&state), 0, "a second run changes nothing");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn one_pane_per_agent_is_left_alone() {
    let state = crate::server::tests::test_state();
    let (_ws, tabs) = seed_workspace(&state, 2).await;
    seed_agent_block(&state, &tabs[0], "ag-a").await;
    seed_agent_block(&state, &tabs[1], "ag-b").await;

    assert!(agent_reconcile::find_duplicates(&state.mstore).unwrap().is_empty());
    assert_eq!(agent_reconcile::reconcile_duplicate_panes(&state), 0);
}
