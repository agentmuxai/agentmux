// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

// Where a pane an agent opens goes when it names no place: a tab of its own pane group (`agent_tab_default`).

use super::agent_tab_default::{apply, browser_target, target_for, BROWSER_FIRST_SPLIT};
use super::*;
use crate::backend::rpc_types::CommandPaneOpenData;
use crate::server::tests::test_state;
use agentmux_common::ipc::{Command, Event};
use serde_json::json;

async fn dispatch_apply(state: &AppState, cmd: Command) -> Vec<Event> {
    let evs = crate::server::service::dispatch_to_reducer(state, cmd).await;
    for ev in &evs {
        crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).unwrap();
    }
    evs
}

/// A tab with one pane in it (the agent's own): `(tab, pane)`.
async fn tab_with_a_pane(state: &AppState) -> (String, String) {
    let ws = dispatch_apply(state, Command::CreateWorkspace { name: "w".into() }).await;
    let ws_id = ws
        .iter()
        .find_map(|e| match e {
            Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
            _ => None,
        })
        .unwrap();
    let tab = dispatch_apply(state, Command::CreateTab { workspace_id: ws_id, name: "t".into() }).await;
    let tab_id = tab
        .iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap();
    let block = dispatch_apply(state, Command::CreateBlock { tab_id: tab_id.clone(), meta: json!({ "view": "agent" }) }).await;
    let block_id = block
        .iter()
        .find_map(|e| match e {
            Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
            _ => None,
        })
        .unwrap();
    // The pane is the tab's tree: one leaf.
    dispatch_apply(
        state,
        Command::LayoutSetTree {
            tab_id: tab_id.clone(),
            new_tree: Some(agentmux_common::LayoutNode {
                id: "pane".into(),
                data: Some(agentmux_common::LayoutNodeData { block_id: block_id.clone(), ..Default::default() }),
                ..Default::default()
            }),
            correlation_id: String::new(),
            slices: None,
        },
    )
    .await;
    (tab_id, block_id)
}

fn an_agent() -> String {
    format!("tabs-{}", uuid::Uuid::new_v4().simple())
}

/// What an agent's route does for an open that names no place, here without the HTTP layer: the default, the open,
/// and the note of who opened it.
async fn agent_opens(state: &AppState, agent: &str, own: &str, view: &str) -> crate::backend::rpc_types::PaneOpenResult {
    let meta = MetaMapType::from([("view".to_string(), json!(view))]);
    let mut cmd = CommandPaneOpenData { view: view.into(), split_reference_block_id: Some(own.into()), focus: Some(false), meta: Some(meta), ..Default::default() };
    apply(state, true, Some(agent), &mut cmd);
    let res = open_pane(state, cmd).await.expect("an open");
    crate::server::pane_openers::note_docked(Some(agent), &res, false);
    res
}

/// The panes of the tab's one pane group, in order.
async fn group_of(state: &AppState, tab: &str) -> Vec<String> {
    let s = state.srv_state.lock().await;
    s.tabs[tab].rootnode.as_ref().unwrap().data.clone().unwrap().block_stack
}

#[tokio::test]
async fn open_twice_from_one_agent_is_two_tabs_of_one_pane_not_two_splits() {
    let state = test_state();
    let (tab, own) = tab_with_a_pane(&state).await;
    let agent = an_agent();

    let first = agent_opens(&state, &agent, &own, "files").await;
    let second = agent_opens(&state, &agent, &own, "editor").await;

    assert_eq!((first.tab_id.as_str(), second.tab_id.as_str()), (tab.as_str(), tab.as_str()));
    assert_eq!(group_of(&state, &tab).await, vec![own.clone(), first.block_id.clone(), second.block_id.clone()], "one pane group, three tabs");
    let s = state.srv_state.lock().await;
    assert!(s.tabs[&tab].rootnode.as_ref().unwrap().children.is_empty(), "no split was made");
}

#[tokio::test]
async fn the_next_open_stacks_onto_the_pane_the_agent_opened_last() {
    let state = test_state();
    let (_tab, own) = tab_with_a_pane(&state).await;
    let agent = an_agent();
    assert_eq!(target_for(&state.mstore, &[], &own).as_deref(), Some(own.as_str()), "with nothing open yet, its own pane");

    let opened = agent_opens(&state, &agent, &own, "files").await;
    let mut cmd = CommandPaneOpenData { view: "editor".into(), split_reference_block_id: Some(own.clone()), ..Default::default() };
    apply(&state, true, Some(&agent), &mut cmd);
    assert_eq!(cmd.stack_onto_block_id.as_deref(), Some(opened.block_id.as_str()));

    // Another agent's panes are not this agent's group.
    let mut other = CommandPaneOpenData { view: "editor".into(), split_reference_block_id: Some(own.clone()), ..Default::default() };
    apply(&state, true, Some(&an_agent()), &mut other);
    assert_eq!(other.stack_onto_block_id.as_deref(), Some(own.as_str()));
}

#[tokio::test]
async fn a_request_that_asks_for_a_split_or_a_window_is_not_changed() {
    let state = test_state();
    let (tab, own) = tab_with_a_pane(&state).await;
    let agent = an_agent();
    for ask in [json!({ "split_direction": "right" }), json!({ "floating": true }), json!({ "stack_onto_block_id": "elsewhere" }), json!({ "skip_placement": true })] {
        let mut cmd: CommandPaneOpenData = serde_json::from_value(json!({ "view": "files", "split_reference_block_id": own })).unwrap();
        let before = serde_json::to_value(&cmd).unwrap();
        for (k, v) in ask.as_object().unwrap() {
            cmd = serde_json::from_value({
                let mut j = serde_json::to_value(&cmd).unwrap();
                j[k] = v.clone();
                j
            })
            .unwrap();
        }
        let asked = serde_json::to_value(&cmd).unwrap();
        apply(&state, true, Some(&agent), &mut cmd);
        assert_eq!(serde_json::to_value(&cmd).unwrap(), asked, "{ask}: left as it asked (it started as {before})");
    }

    // An explicit split still splits: the new pane is a pane of its own, not a tab of the agent's group.
    let mut split: CommandPaneOpenData = serde_json::from_value(json!({
        "view": "files", "split_direction": "right", "split_reference_block_id": own, "meta": { "view": "files" },
    }))
    .unwrap();
    apply(&state, true, Some(&agent), &mut split);
    let res = open_pane(&state, split).await.expect("a split open");
    assert_eq!(group_of_pane(&state, &tab, &own).await, vec![own.clone()], "the agent's own group did not take it");
    assert_ne!(res.block_id, own);
}

/// The block stack of the pane that holds `pane`, found anywhere in the tab's tree.
async fn group_of_pane(state: &AppState, tab: &str, pane: &str) -> Vec<String> {
    fn find(node: &agentmux_common::layout_types::LayoutNode, pane: &str) -> Option<Vec<String>> {
        if let Some(d) = &node.data {
            if d.block_stack.iter().any(|b| b == pane) || d.block_id == pane {
                return Some(if d.block_stack.is_empty() { vec![d.block_id.clone()] } else { d.block_stack.clone() });
            }
        }
        node.children.iter().find_map(|c| find(c, pane))
    }
    let s = state.srv_state.lock().await;
    find(s.tabs[tab].rootnode.as_ref().unwrap(), pane).expect("the pane is in the tree")
}

#[tokio::test]
async fn a_pane_that_is_gone_or_in_another_tab_is_not_a_group_to_join() {
    let state = test_state();
    let (_tab, own) = tab_with_a_pane(&state).await;
    let (_elsewhere_tab, elsewhere) = tab_with_a_pane(&state).await;
    let opened = vec!["a-pane-that-was-closed".to_string(), elsewhere.clone()];
    assert_eq!(target_for(&state.mstore, &opened, &own).as_deref(), Some(own.as_str()), "falls back to its own pane, never another tab's");
    // A pane still there in its own tab is found past the ones that are not.
    let (_t2, mine) = (&_tab, own.clone());
    let live = vec!["a-pane-that-was-closed".to_string(), elsewhere, mine.clone()];
    assert_eq!(target_for(&state.mstore, &live, &own).as_deref(), Some(mine.as_str()));
    // A block made with no placement is in the tab but in no pane: not a group to join (it would fail the open).
    let (tab, own) = tab_with_a_pane(&state).await;
    let unplaced = dispatch_apply(&state, Command::CreateBlock { tab_id: tab, meta: json!({ "view": "files" }) }).await;
    let unplaced = unplaced
        .iter()
        .find_map(|e| match e {
            Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
            _ => None,
        })
        .unwrap();
    assert_eq!(target_for(&state.mstore, &[unplaced], &own).as_deref(), Some(own.as_str()));
    // Its own pane is not in any tab: nothing to join.
    assert_eq!(target_for(&state.mstore, &[], "no-such-pane"), None);
}

#[tokio::test]
async fn a_container_agent_with_no_history_stacks_onto_its_own_pane() {
    let state = test_state();
    let (tab, own) = tab_with_a_pane(&state).await;
    let mut cmd = CommandPaneOpenData {
        view: "files".into(),
        split_reference_block_id: Some(own.clone()),
        meta: Some(MetaMapType::from([("view".to_string(), json!("files"))])),
        ..Default::default()
    };
    apply(&state, true, None, &mut cmd);
    assert_eq!(cmd.stack_onto_block_id.as_deref(), Some(own.as_str()));
    let res = open_pane(&state, cmd).await.expect("an open");
    assert_eq!(group_of(&state, &tab).await, vec![own, res.block_id]);
}

/// What `apply` made of an agent's browser open that named no place.
fn browser_cmd(own: &str) -> CommandPaneOpenData {
    CommandPaneOpenData { view: "browser".into(), split_reference_block_id: Some(own.into()), ..Default::default() }
}

/// What the open will do: split below the agent, or stack onto a browser pane.
#[derive(Debug, PartialEq)]
enum Browser {
    Split(String),
    Stack(String),
}

fn browser_plan(state: &AppState, agent: &str, own: &str) -> Browser {
    let mut cmd = browser_cmd(own);
    apply(state, true, Some(agent), &mut cmd);
    match (cmd.stack_onto_block_id, cmd.split_direction) {
        (Some(t), None) => Browser::Stack(t),
        (None, Some(d)) => Browser::Split(d),
        other => panic!("a browser open is a split or a tab, not {other:?}"),
    }
}

#[tokio::test]
async fn an_agents_first_browser_gets_a_pane_of_its_own_and_the_next_stack_onto_it() {
    let state = test_state();
    let (tab, own) = tab_with_a_pane(&state).await;
    let agent = an_agent();
    assert_eq!(browser_plan(&state, &agent, &own), Browser::Split(BROWSER_FIRST_SPLIT.into()), "the first browser splits, as before");

    let first = agent_opens(&state, &agent, &own, "browser").await;
    assert_eq!(group_of_pane(&state, &tab, &own).await, vec![own.clone()], "the agent's own pane did not take it");
    assert_eq!(group_of_pane(&state, &tab, &first.block_id).await, vec![first.block_id.clone()], "a pane of its own");

    assert_eq!(browser_plan(&state, &agent, &own), Browser::Stack(first.block_id.clone()));
    let second = agent_opens(&state, &agent, &own, "browser").await;
    assert_eq!(group_of_pane(&state, &tab, &second.block_id).await, vec![first.block_id.clone(), second.block_id.clone()], "a tab of the first");
    assert_eq!(group_of_pane(&state, &tab, &own).await, vec![own.clone()]);
    // The third joins the same pane, whichever of its tabs is the newest.
    assert_eq!(browser_plan(&state, &agent, &own), Browser::Stack(second.block_id.clone()));
    let third = agent_opens(&state, &agent, &own, "browser").await;
    assert_eq!(group_of_pane(&state, &tab, &first.block_id).await, vec![first.block_id, second.block_id, third.block_id]);
}

#[tokio::test]
async fn the_new_browser_tab_is_the_one_shown_in_the_browser_pane() {
    let state = test_state();
    let (_tab, own) = tab_with_a_pane(&state).await;
    let agent = an_agent();
    let first = agent_opens(&state, &agent, &own, "browser").await;
    let mut cmd = browser_cmd(&own);
    cmd.focus = Some(false);
    apply(&state, true, Some(&agent), &mut cmd);
    assert_eq!(cmd.focus, Some(true), "the new tab shows in the browser pane");
    cmd.meta = Some(MetaMapType::from([("view".to_string(), json!("browser"))]));
    let second = open_pane(&state, cmd).await.expect("an open");
    assert_eq!(shown_in_pane_of(&state, &first.block_id).await, second.block_id, "the new tab is the shown one");
    assert_eq!(shown_in_pane_of(&state, &own).await, own, "the agent's own pane keeps what it shows");
}

/// The block a pane shows (its active tab), for the pane that holds `pane`.
async fn shown_in_pane_of(state: &AppState, pane: &str) -> String {
    fn leaf<'a>(n: &'a agentmux_common::layout_types::LayoutNode, b: &str) -> Option<&'a agentmux_common::layout_types::LayoutNodeData> {
        n.data.as_ref().filter(|d| d.block_id == b || d.block_stack.iter().any(|x| x == b)).or_else(|| n.children.iter().find_map(|c| leaf(c, b)))
    }
    let s = state.srv_state.lock().await;
    s.tabs.values().find_map(|t| t.rootnode.as_ref().and_then(|r| leaf(r, pane))).expect("the pane is in a tree").block_id.clone()
}

#[tokio::test]
async fn only_the_agents_own_open_browser_in_its_own_tab_is_a_target() {
    let state = test_state();
    let (_tab, own) = tab_with_a_pane(&state).await;
    let (_other_tab, elsewhere) = tab_with_a_pane(&state).await;
    let (agent, other_agent) = (an_agent(), an_agent());

    // An editor opened in between is not a target.
    let first = agent_opens(&state, &agent, &own, "browser").await;
    let _editor = agent_opens(&state, &agent, &own, "editor").await;
    assert_eq!(browser_plan(&state, &agent, &own), Browser::Stack(first.block_id.clone()));

    // Another agent's browser is not a target: an agent with no browser of its own yet splits.
    let _theirs = agent_opens(&state, &other_agent, &own, "browser").await;
    assert_eq!(browser_plan(&state, &an_agent(), &own), Browser::Split(BROWSER_FIRST_SPLIT.into()));

    // A browser that was closed, or moved to another window tab, makes the next one a first again.
    assert_eq!(browser_target(&state.mstore, &["a-browser-that-was-closed".to_string()], &own), None);
    assert_eq!(browser_target(&state.mstore, &[elsewhere], &own), None);
    assert_eq!(browser_target(&state.mstore, &[], &own), None);
}

#[tokio::test]
async fn a_browser_that_asks_for_a_place_gets_it_and_a_persons_browser_keeps_its_split() {
    let state = test_state();
    let (_tab, own) = tab_with_a_pane(&state).await;
    let agent = an_agent();
    let _first = agent_opens(&state, &agent, &own, "browser").await;
    for ask in [json!({ "split_direction": "right" }), json!({ "floating": true }), json!({ "stack_onto_block_id": "elsewhere" })] {
        let mut cmd = browser_cmd(&own);
        for (k, v) in ask.as_object().unwrap() {
            let mut j = serde_json::to_value(&cmd).unwrap();
            j[k] = v.clone();
            cmd = serde_json::from_value(j).unwrap();
        }
        let asked = serde_json::to_value(&cmd).unwrap();
        apply(&state, true, Some(&agent), &mut cmd);
        assert_eq!(serde_json::to_value(&cmd).unwrap(), asked, "{ask}");
    }
    let mut person = browser_cmd(&own);
    apply(&state, false, None, &mut person);
    assert_eq!((person.stack_onto_block_id, person.split_direction), (None, None), "a person's own open is not touched");
}

#[tokio::test]
async fn a_container_agent_follows_the_same_browser_rule() {
    let state = test_state();
    let (tab, own) = tab_with_a_pane(&state).await;
    // The container's route knows its agent by the signed id, as a host agent's route does.
    let container = an_agent();
    let first = agent_opens(&state, &container, &own, "browser").await;
    assert_eq!(group_of_pane(&state, &tab, &first.block_id).await, vec![first.block_id.clone()], "its first browser has a pane of its own");
    assert_eq!(browser_plan(&state, &container, &own), Browser::Stack(first.block_id));
}

#[tokio::test]
async fn a_persons_own_open_keeps_its_place() {
    let state = test_state();
    let (_tab, own) = tab_with_a_pane(&state).await;
    let mut cmd = CommandPaneOpenData { view: "files".into(), split_reference_block_id: Some(own), ..Default::default() };
    apply(&state, false, None, &mut cmd);
    assert_eq!(cmd.stack_onto_block_id, None);
}

#[test]
fn the_routes_agents_open_through_apply_the_default() {
    for (name, whole) in [("pane.open", include_str!("../../http_open.rs")), ("browser open", include_str!("../../ui_handlers.rs"))] {
        assert!(whole.contains("agent_tab_default::apply(&state, "), "{name} must give an agent's open the tab default");
    }
}
