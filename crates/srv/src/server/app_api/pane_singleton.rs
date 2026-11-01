// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//! A management pane (Settings, Vault, Kanban...) is opened once per window
//! (`REPORT_ACTION_RESULTS_ALWAYS_VISIBLE_ACROSS_WINDOWS_2026_10_31.md` §4, step 6b). A `pane.open` for one
//! that the window of the tab already has returns that pane with `created: false`, so an agent that opens
//! Settings again and again leaves one pane, not a stack of them. `new: true` asks for another on purpose.
//! A pane in another window doesn't count, and existing duplicates are left alone.

use super::*;

/// The built-in management views, as `BUILTIN_SINGLETON_VIEWS` in `frontend/app/store/widget-singleton.ts`
/// has them (a test keeps the two equal). A view that a widget entry flags singleton is the window's to
/// enforce: srv doesn't read `widgets.json`.
pub(super) const SINGLETON_VIEWS: [&str; 12] = [
    "audit",
    "connectors",
    "drone",
    "help",
    "history",
    "kanban",
    "memory",
    "settings",
    "swarm",
    "toolchain",
    "tower",
    "vault",
];

/// Whether this open should bring back the window's pane of the view instead of making one. Floating,
/// stacked and unplaced opens put the block somewhere on purpose, so they always create.
pub(super) fn wants_existing(cmd: &CommandPaneOpenData) -> bool {
    cmd.new != Some(true)
        && cmd.floating != Some(true)
        && cmd.skip_placement != Some(true)
        && cmd.stack_onto_block_id.as_deref().is_none_or(str::is_empty)
        && SINGLETON_VIEWS.contains(&cmd.view.as_str())
}

/// The pane of `view` in the window that shows `tab_id`, as `(block id, its tab id)`.
pub(super) fn in_window(mstore: &Store, tab_id: &str, view: &str) -> Option<(String, String)> {
    let workspaces: Vec<Workspace> = mstore.get_all::<Workspace>().ok()?;
    let ws = workspaces.iter().find(|w| w.tabids.iter().chain(&w.pinnedtabids).any(|t| t == tab_id))?;
    for tab_id in ws.pinnedtabids.iter().chain(&ws.tabids) {
        let Ok(Some(tab)) = mstore.get::<Tab>(tab_id) else { continue };
        for block_id in &tab.blockids {
            let Ok(Some(block)) = mstore.get::<Block>(block_id) else { continue };
            if block.meta.get("view").and_then(|v| v.as_str()) == Some(view) {
                return Some((block_id.clone(), tab_id.clone()));
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
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

    /// With `meta`, as the widget bar opens a view (pane.open builds the meta of only a few views itself).
    fn open(view: &str, tab: &str) -> CommandPaneOpenData {
        let meta = MetaMapType::from([("view".to_string(), serde_json::json!(view))]);
        CommandPaneOpenData { view: view.into(), tab_id: Some(tab.into()), meta: Some(meta), ..Default::default() }
    }

    fn panes(state: &AppState, tab: &str) -> usize {
        state.mstore.must_get::<Tab>(tab).unwrap().blockids.len()
    }

    #[tokio::test]
    async fn opening_a_management_pane_again_brings_back_the_one_the_window_has() {
        let state = test_state();
        let tab = window(&state, "one").await;
        let first = open_pane(&state, open("settings", &tab)).await.unwrap();
        assert!(first.created);
        for _ in 0..2 {
            let again = open_pane(&state, open("settings", &tab)).await.unwrap();
            assert_eq!((again.block_id.as_str(), again.created), (first.block_id.as_str(), false));
            assert_eq!(again.tab_id, tab);
        }
        assert_eq!(panes(&state, &tab), 1, "one Settings pane, not three");
    }

    #[tokio::test]
    async fn the_pane_in_another_tab_of_the_same_window_is_the_one() {
        let state = test_state();
        let first_tab = window(&state, "one").await;
        let ws = state.mstore.get_all::<Workspace>().unwrap().into_iter().find(|w| w.tabids.contains(&first_tab)).unwrap();
        let evs = crate::server::service::dispatch_to_reducer(&state, Command::CreateTab { workspace_id: ws.oid.clone(), name: "two".into() }).await;
        for ev in &evs {
            crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).unwrap();
        }
        let second_tab = evs.iter().find_map(|e| if let Event::TabCreated { tab_id, .. } = e { Some(tab_id.clone()) } else { None }).unwrap();
        let first = open_pane(&state, open("vault", &first_tab)).await.unwrap();
        let again = open_pane(&state, open("vault", &second_tab)).await.unwrap();
        assert_eq!((again.block_id, again.tab_id, again.created), (first.block_id, first_tab, false));
        assert_eq!(panes(&state, &second_tab), 0);
    }

    #[tokio::test]
    async fn new_true_makes_another_on_purpose() {
        let state = test_state();
        let tab = window(&state, "one").await;
        let first = open_pane(&state, open("settings", &tab)).await.unwrap();
        let other = open_pane(&state, CommandPaneOpenData { new: Some(true), ..open("settings", &tab) }).await.unwrap();
        assert!(other.created);
        assert_ne!(other.block_id, first.block_id);
        assert_eq!(panes(&state, &tab), 2);
    }

    #[tokio::test]
    async fn another_window_does_not_count_and_an_ordinary_view_is_never_brought_back() {
        let state = test_state();
        let (here, there) = (window(&state, "here").await, window(&state, "there").await);
        let in_there = open_pane(&state, open("settings", &there)).await.unwrap();
        let in_here = open_pane(&state, open("settings", &here)).await.unwrap();
        assert!(in_here.created, "a Settings pane in another window is not this window's");
        assert_ne!(in_here.block_id, in_there.block_id);
        open_pane(&state, open("sysinfo", &here)).await.unwrap();
        let second = open_pane(&state, open("sysinfo", &here)).await.unwrap();
        assert!(second.created, "a view that is not a management pane opens a pane each time");
        assert_eq!(panes(&state, &here), 3);
    }

    #[tokio::test]
    async fn floating_and_stacked_opens_still_create() {
        let state = test_state();
        let tab = window(&state, "one").await;
        let first = open_pane(&state, open("settings", &tab)).await.unwrap();
        let stacked = open_pane(&state, CommandPaneOpenData { stack_onto_block_id: Some(first.block_id.clone()), ..open("settings", &tab) })
            .await
            .unwrap();
        assert!(stacked.created && stacked.block_id != first.block_id);
        assert!(!wants_existing(&CommandPaneOpenData { floating: Some(true), ..open("settings", &tab) }));
        assert!(!wants_existing(&CommandPaneOpenData { skip_placement: Some(true), ..open("settings", &tab) }));
    }

    /// srv can't read the frontend's list at run time, so a test keeps its copy equal.
    #[test]
    fn the_views_are_the_ones_the_frontend_brings_back() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../frontend/app/store/widget-singleton.ts");
        let text = std::fs::read_to_string(&root).unwrap_or_else(|e| panic!("{}: {e}", root.display()));
        let after = text.split("BUILTIN_SINGLETON_VIEWS").nth(1).and_then(|t| t.split("new Set([").nth(1)).expect("the list");
        let body = after.split("])").next().unwrap();
        let theirs: Vec<&str> = body.split('"').skip(1).step_by(2).collect();
        assert_eq!(theirs, SINGLETON_VIEWS, "keep SINGLETON_VIEWS equal to BUILTIN_SINGLETON_VIEWS in widget-singleton.ts");
    }
}
