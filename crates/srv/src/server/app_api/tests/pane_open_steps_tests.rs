// The paths `pane.open` takes (pane_open_steps.rs), driven through `open_pane` and one at a time.

use super::pane_open_steps::{fold_media_into_editor, named_editor_target};
use super::*;
use crate::server::tests::test_state;
use agentmux_common::ipc::{Command, Event};

/// A tab in a workspace of its own: a window.
async fn window(state: &AppState, name: &str) -> String {
    let evs = crate::server::service::dispatch_to_reducer(state, Command::CreateWorkspace { name: name.into() }).await;
    let ws = evs.iter().find_map(|e| match e {
        Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
        _ => None,
    });
    for ev in &evs {
        crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).unwrap();
    }
    let evs = crate::server::service::dispatch_to_reducer(state, Command::CreateTab { workspace_id: ws.unwrap(), name: name.into() }).await;
    for ev in &evs {
        crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).unwrap();
    }
    evs.iter()
        .find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        })
        .unwrap()
}

/// With `meta`, as the widget bar opens a view.
fn open(view: &str, tab: &str) -> CommandPaneOpenData {
    let meta = MetaMapType::from([("view".to_string(), serde_json::json!(view))]);
    CommandPaneOpenData { view: view.into(), tab_id: Some(tab.into()), meta: Some(meta), ..Default::default() }
}

fn editor_target(target: &str) -> CommandPaneOpenData {
    CommandPaneOpenData { view: "editor".into(), target_block_id: Some(target.into()), ..Default::default() }
}

#[test]
fn a_media_open_becomes_an_editor_open_that_follows_the_folder() {
    let mut cmd = CommandPaneOpenData { view: "media".into(), ..Default::default() };
    fold_media_into_editor(&mut cmd).unwrap();
    assert_eq!((cmd.view.as_str(), cmd.follow_folder), ("editor", Some(true)));

    let mut with_meta = open("media", "t");
    fold_media_into_editor(&mut with_meta).unwrap();
    assert_eq!(with_meta.view, "media", "a caller-supplied meta is left alone");
}

#[test]
fn a_target_id_names_an_editor_pane_and_nothing_else() {
    assert_eq!(named_editor_target(&editor_target(" blk-1 ")).unwrap(), Some("blk-1"));
    assert_eq!(named_editor_target(&CommandPaneOpenData { view: "editor".into(), ..Default::default() }).unwrap(), None);

    let not_editor = CommandPaneOpenData { view: "term".into(), ..editor_target("blk-1") };
    assert_eq!(named_editor_target(&not_editor).unwrap_err(), "INVALID_ARG: target_id only names an Editor pane (view editor or media)");
    let floating = CommandPaneOpenData { floating: Some(true), ..editor_target("blk-1") };
    assert_eq!(named_editor_target(&floating).unwrap_err(), "INVALID_ARG: target_id can't be combined with floating");
}

#[tokio::test]
async fn a_docked_open_creates_a_block_in_the_tab_and_reports_it() {
    let state = test_state();
    let tab = window(&state, "docked").await;
    let before = state.mstore.must_get::<Tab>(&tab).unwrap().blockids.len();
    let res = open_pane(&state, open("term", &tab)).await.unwrap();
    assert_eq!((res.tab_id.as_str(), res.view.as_str(), res.created), (tab.as_str(), "term", true));
    assert_eq!(state.mstore.must_get::<Tab>(&tab).unwrap().blockids.len(), before + 1);
    assert!(state.mstore.get::<Block>(&res.block_id).unwrap().is_some());
}

#[tokio::test]
async fn an_open_that_skips_placement_creates_the_block_and_returns_without_a_layout_action() {
    let state = test_state();
    let tab = window(&state, "unplaced").await;
    let before = state.mstore.must_get::<Tab>(&tab).unwrap().blockids.len();
    let res = open_pane(&state, CommandPaneOpenData { skip_placement: Some(true), ..open("term", &tab) }).await.unwrap();
    assert!(res.created);
    assert_eq!((res.tab_id.as_str(), res.view.as_str()), (tab.as_str(), "term"));
    assert!(state.mstore.get::<Block>(&res.block_id).unwrap().is_some(), "the block exists");
    assert_eq!(state.mstore.must_get::<Tab>(&tab).unwrap().blockids.len(), before + 1, "the tab records it, as for any created block");
}

#[tokio::test]
async fn an_open_stacked_onto_a_pane_creates_a_block_in_that_panes_tab() {
    let state = test_state();
    let tab = window(&state, "stacked").await;
    let base = open_pane(&state, open("term", &tab)).await.unwrap();
    let res = open_pane(&state, CommandPaneOpenData { stack_onto_block_id: Some(base.block_id.clone()), ..open("term", &tab) }).await.unwrap();
    assert!(res.created);
    assert_eq!(res.tab_id, tab);
    assert_ne!(res.block_id, base.block_id);
    assert!(state.mstore.get::<Block>(&res.block_id).unwrap().is_some());
}

#[tokio::test]
async fn stacking_onto_a_block_that_does_not_exist_is_refused() {
    let state = test_state();
    let tab = window(&state, "stacked-missing").await;
    let err = open_pane(&state, CommandPaneOpenData { stack_onto_block_id: Some("no-such-block".into()), ..open("term", &tab) }).await.unwrap_err();
    assert!(err.starts_with("pane.open: stack_onto_block_id:"), "{err}");
}
