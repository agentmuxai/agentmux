// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `window` service handler — `CreateWindow`. Split out of `window.rs`;
//! see that file's dispatcher for the full method list.

use agentmux_common::ipc;
use crate::backend::obj::*;
use crate::backend::service::{self, WebCallType, WebReturnType};

use super::super::AppState;
use super::reducer_helpers::{dispatch_to_reducer, publish_events};

/// How long a restore asks for waits for a window close still on its way.
pub(crate) const RESTORE_WAITS_FOR_CLOSE: std::time::Duration = std::time::Duration::from_secs(3);

/// Windows asking to restore decide one at a time, so exactly one restores
/// when several open together (a double click on the tray icon) (#5363).
static RESTORE_ELECTION: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

/// When a window last asked for and got a restore (epoch ms, client meta).
const RESTORED_AT_META_KEY: &str = "session:restored_at_ms";

/// An opener arriving this soon after a restore, beside another window still
/// opening, opens the default panes at once: the window it would wait for is
/// the one just restored, not one closing.
const JUST_RESTORED_MS: i64 = 10_000;

/// The reply for a window that was made. Every window has a slot in the panel's list from the day it is
/// made: the next one, at the end.
fn created_window_reply(store: &crate::backend::storage::store::Store, win: &Window) -> WebReturnType {
    super::session_slots::place(store, std::slice::from_ref(&win.workspaceid));
    WebReturnType::success(serde_json::to_value(win).unwrap_or_default())
}

#[path = "window_create_steps.rs"]
mod steps;

// Phase E.5.8 — CreateWindow migrated through the reducer.
// Two paths: (1) empty workspace_id → CreateWorkspace +
// CreateTab + CreateWindow as a multi-step dispatch (mirrors
// wcore::create_window_full's "fresh workspace" path); (2)
// existing workspace_id → just CreateWindow. The subscriber's
// apply_srv_window_opened handles `Client.windowids` updates
// and Window-row creation. Layout setup for the new tab uses
// the apply_tab_created provisioning (E.4 layout migration is
// separate; default rootnode = None matches wcore behaviour).
pub(crate) async fn handle_create_window(state: &AppState, call: &WebCallType) -> WebReturnType {
    let args = &call.args;
    let requested_ws_id: String = service::get_arg(args, 1).unwrap_or_default();
    let restore = steps::restore_request(state, args).await;
    // "Open in New Window" from a widget's context menu (arg 4/5, optional,
    // same additive pattern as restoreIfAvailable) — when present,
    // seed the fresh tab with ONLY this one block instead of the default
    // 3-pane agent/swarm/sysinfo layout. Ignored on the tear-off / existing-
    // workspace path (`resolve_workspace`'s existing-workspace case), same as the default seed is.
    let seed_view: Option<String> = service::get_optional_arg(args, 4).unwrap_or(None);
    let seed_meta: Option<serde_json::Value> = service::get_optional_arg(args, 5).unwrap_or(None);
    let mut choice = match steps::resolve_workspace(state, requested_ws_id, restore.restore_if_available, seed_view, seed_meta).await {
        Ok(choice) => choice,
        Err(reply) => return reply,
    };

    // Step 3: register the window in the reducer.
    let (window_id, win_events) = match steps::register_window(state, &choice.ws_id, &choice.fresh_workspace_events).await {
        Ok(registered) => registered,
        Err(reply) => return reply,
    };
    let host_label: String = service::get_arg(args, 2).unwrap_or_default();
    let crumb_events = steps::write_label_crumb(state, &window_id, &host_label).await;
    let name_events = steps::restored_name_events(state, &window_id, &choice.ws_id).await;
    let inherit_events = steps::inherited_events(state, args, &window_id, &host_label).await;
    // Publish all events from this multi-step (workspace + tab + window).
    let mut all_events = std::mem::take(&mut choice.fresh_workspace_events);
    all_events.extend(win_events);
    all_events.extend(crumb_events);
    all_events.extend(name_events);
    all_events.extend(inherit_events);
    publish_events(state, &all_events);
    // Return the Window struct (matches the prior RPC contract).
    steps::created_reply(state, &window_id, &choice)
}

/// The message of the first error among `events`, if any.
fn first_error(events: &[ipc::Event]) -> Option<String> {
    events.iter().find_map(|e| match e {
        ipc::Event::Error { message, .. } => Some(message.clone()),
        _ => None,
    })
}

/// Writes `events` through to the store, stopping at the first failure.
fn persist(events: &[ipc::Event], store: &crate::backend::storage::store::Store) -> Result<(), String> {
    for ev in events {
        crate::persist_subscriber::apply_event_to_mstore(ev, store).map_err(|e| format!("CreateWindow: SQLite write failed: {}", e))?;
    }
    Ok(())
}

/// Step 1: the new workspace's id and the events that made it.
async fn create_workspace(state: &AppState) -> Result<(String, Vec<ipc::Event>), String> {
    let events = dispatch_to_reducer(state, ipc::Command::CreateWorkspace { name: String::new() }).await;
    if let Some(err_msg) = first_error(&events) {
        return Err(err_msg);
    }
    persist(&events, &state.mstore)?;
    let ws_id = events
        .iter()
        .find_map(|e| match e {
            ipc::Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
            _ => None,
        })
        .unwrap_or_default();
    Ok((ws_id, events))
}

/// Step 2: the workspace's tab, and the events that made it. When the tab can't be made, the empty
/// workspace is deleted again.
async fn create_tab(state: &AppState, ws_id: &str) -> Result<Vec<ipc::Event>, String> {
    let events = dispatch_to_reducer(state, ipc::Command::CreateTab { workspace_id: ws_id.to_string(), name: String::new() }).await;
    if let Some(err_msg) = first_error(&events) {
        // Compensate: delete the empty workspace.
        let comp = dispatch_to_reducer(
            state,
            ipc::Command::DeleteWorkspace {
                workspace_id: ws_id.to_string(),
                // Internal compensation path — not
                // saga-driven (Step 5 PR 2 added the
                // `force` flag for saga provenance).
                force: false,
            },
        )
        .await;
        for ev in &comp {
            let _ = crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore);
        }
        publish_events(state, &comp);
        return Err(err_msg);
    }
    persist(&events, &state.mstore)?;
    Ok(events)
}

/// Dispatches the seed block(s) through the reducer: the `seed_view` alone (with `seed_meta` when
/// given), or the default agent, swarm and sysinfo. For the default 3-pane case, order is the one
/// `default_three_pane_tree` expects positionally (agent, swarm, sysinfo) — NOT top-to-bottom
/// display order, since sysinfo renders above swarm. Returns the ids made and their events; a block
/// that can't be made ends the seeding (the tab opens with what there is).
async fn seed_blocks(
    state: &AppState,
    tab_id: &str,
    seed_view: Option<&str>,
    seed_meta: Option<&serde_json::Value>,
) -> (Vec<String>, Vec<ipc::Event>) {
    let views: Vec<&str> = match seed_view {
        Some(v) => vec![v],
        None => vec!["agent", "swarm", "sysinfo"],
    };
    let mut seeded_ids: Vec<String> = Vec::new();
    let mut seed_events: Vec<ipc::Event> = Vec::new();
    for view in views {
        let meta = match (seed_view, seed_meta) {
            (Some(_), Some(meta)) => meta.clone(),
            _ => serde_json::json!({ "view": view }),
        };
        let evs = dispatch_to_reducer(state, ipc::Command::CreateBlock { tab_id: tab_id.to_string(), meta }).await;
        if let Some(err_msg) = first_error(&evs) {
            tracing::warn!(tab_id = %tab_id, view = %view, error = %err_msg, "CreateWindow: seed block create failed — opening blank tab");
            break;
        }
        for ev in &evs {
            if let Err(e) = crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore) {
                tracing::warn!(tab_id = %tab_id, error = %e, "CreateWindow: seed block SQLite write failed");
            }
        }
        seeded_ids.extend(evs.iter().find_map(|e| match e {
            ipc::Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
            _ => None,
        }));
        seed_events.extend(evs);
    }
    (seeded_ids, seed_events)
}

/// Writes the layout for the seeded panes: a single leaf for a widget's "Open in New Window" (see
/// `single_leaf_tree`), the shared three-pane tree for the default. A cold start with nothing of
/// its own to restore (`offer_other_build`) also notes what it seeded, for a layout another build
/// saved to be offered. A seeding that came out short leaves the tab blank.
async fn seed_layout(state: &AppState, ws_id: &str, tab_id: &str, seed_view: Option<&str>, seeded_ids: &[String], offer_other_build: bool) {
    let tree_and_focus = match (seed_view, seeded_ids) {
        (Some(_), [only]) => Some(crate::backend::wcore::single_leaf_tree(only)),
        (None, [agent, swarm, sysinfo]) => {
            if offer_other_build {
                super::session_other_builds::remember_seeded_default(&state.mstore, ws_id, tab_id, seeded_ids);
            }
            // SPEC_864 Phase 3 — post-bootstrap seed routes through the reducer (single writer of
            // db_layout); the tree shape is shared with the pre-bootstrap store-direct first-launch
            // seed via `default_three_pane_tree`.
            Some(crate::backend::wcore::default_three_pane_tree(agent, swarm, sysinfo))
        }
        _ => None,
    };
    if let Some((tree, focused)) = tree_and_focus {
        if let Err(e) = super::reducer_helpers::seed_layout_via_reducer(state, tab_id, tree, focused, String::new()).await {
            tracing::warn!(tab_id = %tab_id, error = %e, "CreateWindow: default layout write failed — opening blank tab");
        }
    }
}

/// A new window's contents: a workspace with one tab seeded with the default
/// panes (agent | sysinfo / swarm), or with the single `seed_view` pane a
/// widget's "Open in New Window" asks for. No window shows it yet. Also what
/// a new user workspace's first window opens (`user_workspace_switch`).
/// `offer_other_build`: a cold start with nothing to restore, which may offer
/// a layout another build saved. Returns the workspace and the applied events,
/// unpublished.
///
/// Seeding is non-fatal: a failure leaves an empty tab (the prior behaviour) rather than failing
/// window creation. Only this fresh-workspace path seeds; tear-off (an existing workspace)
/// reattaches its populated workspace as-is. See docs/retro/retro-blank-new-window-2026-06-21.md.
///
/// 2nd-window-tear-off desync fix (#1681): the seed blocks MUST be created through the reducer
/// (CreateBlock command), NOT via the store-only `seed_default_layout`/`create_block` path. This
/// runs AFTER bootstrap, so anything written straight to SQLite is invisible to the in-memory
/// reducer `srv_state` until the next restart: a later `TearOffBlock` from that window would be
/// rejected "block not found". Dispatching CreateBlock per pane lands the blocks in BOTH srv_state
/// and (via the subscriber) SQLite, then the shared layout is written referencing them.
pub(crate) async fn create_default_contents(
    state: &AppState,
    seed_view: Option<String>,
    seed_meta: Option<serde_json::Value>,
    offer_other_build: bool,
) -> Result<(String, Vec<ipc::Event>), String> {
    let (new_ws_id, mut combined) = create_workspace(state).await?;
    let tab_events = create_tab(state, &new_ws_id).await?;
    let new_tab_id = tab_events.iter().find_map(|e| match e {
        ipc::Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
        _ => None,
    });
    combined.extend(tab_events);
    if let Some(tab_id) = new_tab_id {
        let (seeded_ids, seed_events) = seed_blocks(state, &tab_id, seed_view.as_deref(), seed_meta.as_ref()).await;
        seed_layout(state, &new_ws_id, &tab_id, seed_view.as_deref(), &seeded_ids, offer_other_build).await;
        combined.extend(seed_events);
    }
    Ok((new_ws_id, combined))
}

#[cfg(test)]
#[path = "window_create_tests.rs"]
mod create_tests;

#[cfg(test)]
#[path = "window_create_contents_tests.rs"]
mod contents_tests;

#[cfg(test)]
mod create_window_seed_tests {
    use super::super::window::handle_window_service;
    use crate::backend::obj::{Block, LayoutState, Tab};
    use crate::backend::service::WebCallType;
    use crate::server::tests::test_state;

    fn create_window_call(workspace_id: &str) -> WebCallType {
        WebCallType {
            service: "window".to_string(),
            method: "CreateWindow".to_string(),
            uicontext: None,
            // arg0 is ignored by the handler; arg1 is the (optional) workspace
            // to reattach. Empty → fresh-workspace seed path.
            args: vec![
                serde_json::Value::Null,
                serde_json::Value::String(workspace_id.to_string()),
            ],
        }
    }

    /// Regression for the 2nd-window-tear-off desync (#1681).
    ///
    /// "Open another window" used to seed its default blocks straight
    /// into SQLite (`seed_default_layout` → `create_block`), bypassing the
    /// reducer. The handler runs after bootstrap, so those blocks never
    /// reached the in-memory `srv_state`. The frontend rendered them (it reads
    /// SQLite) but a subsequent `TearOffBlock` from that window was rejected
    /// "block not found" — the workspace/tab existed (they went through the
    /// reducer) but the block did not. This asserts the seed blocks now land in
    /// `srv_state` AND that a tear-off from the new window succeeds end-to-end.
    #[tokio::test]
    async fn new_window_seed_blocks_are_in_reducer_state_and_tear_off_succeeds() {
        let state = test_state();
        let blocks_before = state.srv_state.lock().await.blocks.len();

        let ret = handle_window_service(&state, &create_window_call("")).await;
        assert!(ret.success, "CreateWindow failed: {:?}", ret.error);

        let win = ret.data.expect("CreateWindow returns the Window");
        let workspace_id = win
            .get("workspaceid")
            .and_then(|v| v.as_str())
            .expect("window has a workspaceid")
            .to_string();

        // The fix: the seed blocks are present in the in-memory reducer
        // state, attached to the new window's tab.
        let (tab_id, block_id) = {
            let s = state.srv_state.lock().await;
            assert_eq!(
                s.blocks.len(),
                blocks_before + 3,
                "the 3 seed blocks must be tracked in srv_state, not only SQLite"
            );
            let ws = s
                .workspaces
                .get(&workspace_id)
                .expect("new workspace is in the reducer");
            let tab_id = ws.tab_ids.first().expect("new workspace has a tab").clone();
            let tab = s.tabs.get(&tab_id).expect("new tab is in the reducer");
            assert_eq!(
                tab.block_ids.len(),
                3,
                "the new window's tab must hold its 3 seed blocks in the reducer"
            );
            (tab_id, tab.block_ids[0].clone())
        };

        // End-to-end: tearing a block off the freshly-created window no longer
        // hits the "block not found" pre-condition.
        let result =
            crate::sagas::tear_off_block::run(&state, block_id, tab_id, workspace_id).await;
        assert!(
            result.is_ok(),
            "tear-off from a freshly-created window must succeed, got: {:?}",
            result.err()
        );
    }

    /// A widget's "Open in New Window" (arg 4 `seed_view` / arg 5
    /// `seed_meta`) must seed the new tab with ONLY that one pane — not the
    /// default 3-pane layout plus the widget as a 4th split. Regression: the
    /// pre-existing frontend-only `initial_view` mechanism opened the
    /// default 3-pane layout via CreateWindow and then called `pane.open`
    /// on top of it, which only ever ADDED a pane rather than replacing the
    /// seed.
    #[tokio::test]
    async fn new_window_with_seed_view_seeds_a_single_pane_not_the_default_three() {
        let state = test_state();

        let call = WebCallType {
            service: "window".to_string(),
            method: "CreateWindow".to_string(),
            uicontext: None,
            args: vec![
                serde_json::Value::Null,                        // winSize
                serde_json::Value::String(String::new()),       // workspaceId — fresh
                serde_json::Value::Null,                        // hostLabel
                serde_json::Value::Bool(false),                 // restoreIfAvailable
                serde_json::Value::String("browser".to_string()), // seedView
                serde_json::json!({ "view": "browser", "url": "https://example.com" }), // seedMeta
            ],
        };

        let ret = handle_window_service(&state, &call).await;
        assert!(ret.success, "CreateWindow failed: {:?}", ret.error);

        let win = ret.data.expect("CreateWindow returns the Window");
        let workspace_id = win
            .get("workspaceid")
            .and_then(|v| v.as_str())
            .expect("window has a workspaceid")
            .to_string();

        let tab_id = {
            let s = state.srv_state.lock().await;
            let ws = s.workspaces.get(&workspace_id).expect("new workspace is in the reducer");
            assert_eq!(ws.tab_ids.len(), 1, "one tab");
            let tab_id = ws.tab_ids.first().unwrap().clone();
            let tab = s.tabs.get(&tab_id).expect("new tab is in the reducer");
            assert_eq!(
                tab.block_ids.len(),
                1,
                "seeded with exactly ONE block, not the default 3-pane layout"
            );
            tab_id
        };

        let store = &state.mstore;
        let tab = store.must_get::<Tab>(&tab_id).unwrap();
        assert_eq!(tab.blockids.len(), 1);
        let block = store.must_get::<Block>(&tab.blockids[0]).unwrap();
        assert_eq!(block.meta.get("view").and_then(|v| v.as_str()), Some("browser"));
        assert_eq!(
            block.meta.get("url").and_then(|v| v.as_str()),
            Some("https://example.com"),
            "the full seedMeta must be used, not just a bare {{view}} object"
        );

        // Single-leaf tree — no split, root node itself wraps the block.
        let layout = store.must_get::<LayoutState>(&tab.layoutstate).unwrap();
        let root = layout.rootnode.as_ref().expect("layout must be populated");
        assert!(root.children.is_empty(), "single pane — no split children");
        assert_eq!(root.data.as_ref().unwrap().block_id, tab.blockids[0]);
    }
}
