// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The paths `pane.open` takes once it knows what to open, one function each. `open_pane` (pane.rs) calls them;
//! the code is moved from it unchanged.

use super::*;

/// The Media pane is folded into the Editor: a media open is an Editor open that follows the folder.
pub(super) fn fold_media_into_editor(cmd: &mut CommandPaneOpenData) -> Result<(), String> {
    // The Media pane is folded into the Editor
    // (SPEC_EDITOR_REVAMP_UNIFIED_VIEWERS_2026_10_15.md §4.11): a media open
    // is an Editor open that follows the folder, and may reuse the Editor
    // pane on screen like OpenEditor does.
    if cmd.view == "media" && cmd.meta.is_none() {
        if let Some(conn) = cmd.connection.as_deref().map(str::trim).filter(|c| !c.is_empty() && *c != "local") {
            if crate::backend::fs_ops::remote::ssh_connection(Some(conn)).is_some() {
                return Err(format!("INVALID_ARG: media files on {conn} can't be shown yet"));
            }
        }
        cmd.view = "editor".to_string();
        cmd.follow_folder.get_or_insert(true);
    }
    Ok(())
}

/// The Editor pane a `target_id` names, if any; it is only valid with an Editor open that is not floating.
pub(super) fn named_editor_target(cmd: &CommandPaneOpenData) -> Result<Option<&str>, String> {
    // A `target_block_id` names the Editor pane to put the file in: it needs no other opt-in.
    let named = cmd.target_block_id.as_deref().map(str::trim).filter(|t| !t.is_empty());
    if named.is_some() {
        if cmd.view != "editor" {
            return Err("INVALID_ARG: target_id only names an Editor pane (view editor or media)".to_string());
        }
        if cmd.floating == Some(true) {
            return Err("INVALID_ARG: target_id can't be combined with floating".to_string());
        }
    }
    Ok(named)
}

/// Add the requested file as a tab of an Editor pane that is already open, when the caller asked for reuse.
pub(super) async fn reuse_editor_pane(state: &AppState, cmd: &CommandPaneOpenData, named: Option<&str>, viewer: Option<&'static str>) -> Result<Option<PaneOpenResult>, String> {
    // Reuse (SPEC_EDITOR_MCP_OPEN_BLANK_PREVIEW_AND_PANE_REUSE_2026_08_03.md Part 2) needs the explicit `reuse_editor_pane`
    // opt-in, never inferred from `split_reference_block_id`: openToTheSide and openInTerminal set it to their own block id
    // and must not reuse (#2404). A floating open always gets its own window. An explicit `tree_expanded` does not bypass
    // reuse: a reused pane keeps its own tree state, and bypassing made a new pane on every open of an agent that always passes it.
    if cmd.view == "editor" && (cmd.reuse_editor_pane == Some(true) || named.is_some()) && cmd.floating != Some(true) {
        if let Some(file) = cmd.file.as_deref() {
            let connection = cmd.connection.as_deref().unwrap_or("");
            let follow = cmd.follow_folder == Some(true);
            let caller = cmd.split_reference_block_id.as_deref();
            if let Some(result) = pane::maybe_reuse_editor_pane(state, caller, named, file, connection, follow, viewer).await? {
                return Ok(Some(result));
            }
        }
    }
    Ok(None)
}

/// Stack path (SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md §3.3): create the block as a new tab of the pane holding
/// `target`, in one reducer step, then queue a `stackpush` action for the frontend.
pub(super) async fn open_stacked(state: &AppState, cmd: &CommandPaneOpenData, target: String, meta: MetaMapType) -> Result<PaneOpenResult, String> {
    let mstore = state.mstore.clone();
    // The new tab is the visible one unless the caller said not to take focus
    // (`focus: false`: an agent's popup must not switch the person's tab).
    let activate = cmd.focus.unwrap_or(true);
    let tab_id = resolve_tab_id_for_block(&mstore, &target)
        .map_err(|e| format!("pane.open: stack_onto_block_id: {e}"))?;
    let meta_val = serde_json::to_value(&meta)
        .map_err(|e| format!("pane.open: stack_onto_block_id: meta serialize: {e}"))?;
    let events = crate::server::service::dispatch_to_reducer(
        state,
        agentmux_common::ipc::Command::CreateBlockInStack {
            tab_id: tab_id.clone(),
            target_block_id: target.clone(),
            meta: meta_val,
            activate,
        },
    )
    .await;
    if let Some(msg) = events.iter().find_map(|e| match e {
        agentmux_common::ipc::Event::Error { message, .. } => Some(message.clone()),
        _ => None,
    }) {
        return Err(format!("pane.open: {msg}"));
    }
    let block_id = events
        .iter()
        .find_map(|e| match e {
            agentmux_common::ipc::Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
            _ => None,
        })
        .ok_or_else(|| "pane.open: CreateBlockInStack emitted no BlockCreated".to_string())?;
    for ev in &events {
        if let Err(e) = crate::persist_subscriber::apply_event_to_mstore(ev, &mstore) {
            tracing::warn!("pane.open: stack_onto_block_id: mstore apply failed: {e}");
        }
    }
    crate::server::service::publish_events(state, &events);
    if let Err(e) =
        crate::server::service::layout_helpers::queue_target_stack_push(state, &tab_id, &block_id, &target, activate).await
    {
        tracing::warn!(block_id = %block_id, "pane.open: stack_onto_block_id: stackpush queue failed: {e}");
    }
    tracing::info!(block_id = %block_id, target = %target, view = %cmd.view, "pane.open: block created as a pane tab");
    Ok(PaneOpenResult {
        block_id,
        tab_id,
        view: cmd.view.clone(),
        created: true,
    })
}

/// Skip-placement path (SPEC_PANE_TAB_STRIP_AGENT_TERMINAL_2026_07_20.md §4.2): create the block through the reducer and
/// return at once, with no layout action; the caller attaches it to an existing pane's block stack.
pub(super) async fn open_unplaced(state: &AppState, view: String, tab_id: String, meta: MetaMapType) -> Result<PaneOpenResult, String> {
    let mstore = state.mstore.clone();
    let meta_val = serde_json::to_value(&meta)
        .map_err(|e| format!("pane.open: skip_placement: meta serialize: {e}"))?;
    let create_events = crate::server::service::dispatch_to_reducer(
        state,
        agentmux_common::ipc::Command::CreateBlock {
            tab_id: tab_id.clone(),
            meta: meta_val,
        },
    )
    .await;
    if let Some(msg) = create_events.iter().find_map(|e| match e {
        agentmux_common::ipc::Event::Error { message, .. } => Some(message.clone()),
        _ => None,
    }) {
        return Err(format!("pane.open: skip_placement: CreateBlock: {msg}"));
    }
    let block_id = create_events
        .iter()
        .find_map(|e| match e {
            agentmux_common::ipc::Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
            _ => None,
        })
        .ok_or_else(|| "pane.open: skip_placement: CreateBlock emitted no BlockCreated".to_string())?;
    for ev in &create_events {
        if let Err(e) = crate::persist_subscriber::apply_event_to_mstore(ev, &mstore) {
            tracing::warn!("pane.open: skip_placement: CreateBlock mstore apply failed: {e}");
        }
    }
    crate::server::service::publish_events(state, &create_events);
    tracing::info!(block_id = %block_id, view = %view, "pane.open: block created, placement skipped");
    Ok(PaneOpenResult {
        block_id,
        tab_id,
        view,
        created: true,
    })
}

/// Docked path: create the block through the reducer, place it (split if asked, else appended), and tell every window.
pub(super) async fn open_docked(state: &AppState, cmd: &CommandPaneOpenData, tab_id: String, meta: MetaMapType) -> Result<PaneOpenResult, String> {
    let mstore = state.mstore.clone();
    let event_bus = state.event_bus.clone();
    // Create the block through the reducer (#1681), not wcore-direct: a store-only `create_block` never reaches the
    // reducer's `state.blocks`, so a later TearOffBlock / RedockFloatingPane is rejected "block not found".
    // Mirrors the open_pane_floating path.
    let meta_val = serde_json::to_value(&meta)
        .map_err(|e| format!("pane.open: meta serialize: {e}"))?;
    let create_events = crate::server::service::dispatch_to_reducer(
        state,
        agentmux_common::ipc::Command::CreateBlock {
            tab_id: tab_id.clone(),
            meta: meta_val,
        },
    )
    .await;
    if let Some(msg) = create_events.iter().find_map(|e| match e {
        agentmux_common::ipc::Event::Error { message, .. } => Some(message.clone()),
        _ => None,
    }) {
        return Err(format!("pane.open: CreateBlock: {msg}"));
    }
    let block_id = create_events
        .iter()
        .find_map(|e| match e {
            agentmux_common::ipc::Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
            _ => None,
        })
        .ok_or_else(|| "pane.open: CreateBlock emitted no BlockCreated".to_string())?;
    for ev in &create_events {
        if let Err(e) = crate::persist_subscriber::apply_event_to_mstore(ev, &mstore) {
            tracing::warn!("pane.open: CreateBlock mstore apply failed: {e}");
        }
    }
    crate::server::service::publish_events(state, &create_events);

    // Place it, split if requested, else append: srv's tree, then the queued
    // action for the window. Best-effort: a failure leaves the block created
    // but not laid out.
    let placement = pane::resolve_placement(
        cmd.split_direction.as_deref(),
        cmd.split_reference_block_id.as_deref(),
        cmd.focus.unwrap_or(true),
    );
    if let Err(e) = crate::server::service::layout_intents::place_block(state, &tab_id, &block_id, placement).await {
        tracing::warn!("pane.open: placing the pane failed: {e}");
    }

    tracing::info!(
        block_id = %block_id,
        view = %cmd.view,
        "pane.open: block created + layout updated"
    );

    // Broadcast block + tab + layout updates
    {
        let mut updates = Vec::new();
        if let Ok(updated_block) = mstore.must_get::<Block>(&block_id) {
            updates.push(obj::MuxObjUpdate {
                updatetype: "update".into(),
                otype: "block".into(),
                oid: block_id.clone(),
                obj: Some(obj::mux_obj_to_value(&updated_block)),
            });
        }
        if let Ok(updated_tab) = mstore.must_get::<Tab>(&tab_id) {
            updates.push(obj::MuxObjUpdate {
                updatetype: "update".into(),
                otype: "tab".into(),
                oid: tab_id.clone(),
                obj: Some(obj::mux_obj_to_value(&updated_tab)),
            });
            if let Ok(updated_layout) = mstore.must_get::<obj::LayoutState>(&updated_tab.layoutstate) {
                updates.push(obj::MuxObjUpdate {
                    updatetype: "update".into(),
                    otype: "layout".into(),
                    oid: updated_tab.layoutstate.clone(),
                    obj: Some(obj::mux_obj_to_value(&updated_layout)),
                });
            }
        }
        // One batched frame so the renderer applies all of them in a single
        // reactive flush — see EventBus::broadcast_mux_obj_updates.
        event_bus.broadcast_mux_obj_updates(&updates);
    }

    Ok(PaneOpenResult {
        block_id,
        tab_id,
        view: cmd.view.clone(),
        created: true,
    })
}
