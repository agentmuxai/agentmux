// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The Pane Tab edit commands (`pane.moveTab`, `pane.moveTabs`, `pane.activateTab`, `pane.removeTab`):
//! resolve the block's tab, dispatch the reducer command, publish, and queue the matching layout action for
//! other views. Split out of `pane.rs`, which `open_pane` and the other pane commands fill.

use super::*;

/// Core `pane.moveTab` logic — reorder `cmd.block_id` within its own pane, or
/// move it into a different pane (`Command::LayoutStackMove`). Queues a
/// `stackmove` layout action so any OTHER window/tab viewing the same
/// `db_layout` sees the change (mirrors `stack_onto_block_id`'s `stackpush`
/// queue above); the calling frontend applies its own optimistic local edit
/// directly and skips the queued copy by its `action_id`.
/// SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §4.1, Phase 3.
pub(crate) async fn move_tab(state: &AppState, cmd: CommandPaneMoveTabData) -> Result<PaneTabEditResult, String> {
    let position = match cmd.position.as_str() {
        "before" => agentmux_common::StackMovePosition::Before,
        "after" => agentmux_common::StackMovePosition::After,
        "end" => agentmux_common::StackMovePosition::End,
        other => return Err(format!("pane.moveTab: invalid position '{other}' (expected before/after/end)")),
    };
    // Cross-pane moves (Phase 4, SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md
    // §3.4) were rejected here through Phase 3 (Codex P2 on PR #3444) —
    // the frontend's pending-action handler didn't yet mirror a cross-leaf
    // "stackmove" to other windows/tabs watching this layout. That gap is
    // now closed (layoutPersistence.ts's StackMove case, via
    // moveMemberAcrossStacks), so both same-pane and cross-pane targets are
    // accepted here — Command::LayoutStackMove already validates everything
    // else (existence, non-empty source, etc.) via move_stack_member.
    let (tab_id, _) = apply_pane_tab_command(state, "pane.moveTab", &cmd.block_id, |tab_id| {
        agentmux_common::ipc::Command::LayoutStackMove {
            tab_id,
            block_id: cmd.block_id.clone(),
            target_block_id: cmd.target_block_id.clone(),
            position,
            activate: cmd.activate,
            correlation_id: String::new(),
        }
    })
    .await?;
    let queued = crate::server::service::layout_helpers::queue_target_stack_move(
        state,
        &tab_id,
        &cmd.block_id,
        &cmd.target_block_id,
        &cmd.position,
        cmd.activate,
        cmd.action_id.as_deref().unwrap_or_default(),
    )
    .await;
    tracing::info!(block_id = %cmd.block_id, target = %cmd.target_block_id, position = %cmd.position, "pane.moveTab");
    Ok(edit_result(state, &tab_id, "pane.moveTab", &cmd.block_id, queued))
}

/// Core `pane.moveTabs` logic: several tabs into one pane in one reducer edit
/// (`Command::LayoutStackMoveMany`), then one queued `stackmove` per block (and
/// a `stackactivate` when the visible tab is a member that did not move) so any
/// OTHER window viewing the layout follows. The calling frontend has already
/// made the same edit and skips every queued copy by its base `action_id`.
/// SPEC_PANE_TAB_MULTI_SELECT_MOVE_AND_COLLAPSED_EXPAND_2026_10_24.md §5.
pub(crate) async fn move_tabs(state: &AppState, cmd: CommandPaneMoveTabsData) -> Result<PaneTabEditResult, String> {
    if cmd.block_ids.is_empty() {
        return Err("pane.moveTabs: block_ids is empty".to_string());
    }
    let (tab_id, _) = apply_pane_tab_command(state, "pane.moveTabs", &cmd.target_block_id, |tab_id| {
        agentmux_common::ipc::Command::LayoutStackMoveMany {
            tab_id,
            block_ids: cmd.block_ids.clone(),
            target_block_id: cmd.target_block_id.clone(),
            activate_block_id: cmd.activate_block_id.clone(),
            correlation_id: String::new(),
        }
    })
    .await?;
    let queued = crate::server::service::layout_helpers::queue_target_stack_moves(
        state,
        &tab_id,
        &cmd.block_ids,
        &cmd.target_block_id,
        cmd.activate_block_id.as_deref(),
        cmd.action_id.as_deref().unwrap_or_default(),
    )
    .await;
    tracing::info!(count = cmd.block_ids.len(), target = %cmd.target_block_id, "pane.moveTabs");
    Ok(edit_result(state, &tab_id, "pane.moveTabs", &cmd.target_block_id, queued))
}

/// `pane.activateTab`: make `block_id` its pane's visible tab
/// (`Command::LayoutStackActivate`), then queue a `stackactivate` under the
/// caller's action id for any other view of the tab.
pub(crate) async fn activate_tab(state: &AppState, cmd: CommandPaneTabData) -> Result<PaneTabEditResult, String> {
    let block_id = cmd.block_id.clone();
    let (tab_id, _) = apply_pane_tab_command(state, "pane.activateTab", &cmd.block_id, |tab_id| {
        agentmux_common::ipc::Command::LayoutStackActivate { tab_id, block_id, correlation_id: String::new() }
    })
    .await?;
    let action_id = cmd.action_id.as_deref().unwrap_or_default();
    let queued = crate::server::service::layout_helpers::queue_stack_activate(state, &tab_id, &cmd.block_id, action_id).await;
    Ok(edit_result(state, &tab_id, "pane.activateTab", &cmd.block_id, queued))
}

/// `pane.removeTab`: take `block_id` out of its pane — that one tab of a
/// stack, or the pane with its last tab (`Command::LayoutDeleteNodeByBlock`) —
/// then queue the `delete` under the caller's action id. The block itself is
/// closed by `ClosePane`, which the caller sends next. A block in no pane is a
/// no-op: nothing is queued and the revision is 0.
pub(crate) async fn remove_tab(state: &AppState, cmd: CommandPaneTabData) -> Result<PaneTabEditResult, String> {
    let block_id = cmd.block_id.clone();
    let (tab_id, changed) = apply_pane_tab_command(state, "pane.removeTab", &cmd.block_id, |tab_id| {
        agentmux_common::ipc::Command::LayoutDeleteNodeByBlock { tab_id, block_id, correlation_id: String::new() }
    })
    .await?;
    if !changed {
        let version = crate::server::service::layout_intents::layout_row_version(&state.mstore, &tab_id);
        return Ok(PaneTabEditResult { version });
    }
    let action_id = cmd.action_id.as_deref().unwrap_or_default();
    let queued = crate::server::service::layout_helpers::queue_stack_remove(state, &tab_id, &cmd.block_id, action_id).await;
    Ok(edit_result(state, &tab_id, "pane.removeTab", &cmd.block_id, queued))
}

/// Resolve `block_id`'s tab, then dispatch `cmd` → mstore → publish, the
/// shape every pane-tab RPC shares. Returns the tab and whether the reducer
/// changed anything.
async fn apply_pane_tab_command(
    state: &AppState,
    op: &str,
    block_id: &str,
    cmd: impl FnOnce(String) -> agentmux_common::ipc::Command,
) -> Result<(String, bool), String> {
    let tab_id = resolve_tab_id_for_block(&state.mstore, block_id).map_err(|e| format!("{op}: {e}"))?;
    let events = crate::server::service::dispatch_to_reducer(state, cmd(tab_id.clone())).await;
    if let Some(msg) = events.iter().find_map(|e| match e {
        agentmux_common::ipc::Event::Error { message, .. } => Some(message.clone()),
        _ => None,
    }) {
        return Err(format!("{op}: {msg}"));
    }
    for ev in &events {
        if let Err(e) = crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore) {
            tracing::warn!("{op}: mstore apply failed: {e}");
        }
    }
    crate::server::service::publish_events(state, &events);
    Ok((tab_id, !events.is_empty()))
}

/// What a pane-tab RPC answers: the row's version once the change is stored. A
/// failed queue is logged, not returned: the tree change already happened.
fn edit_result(state: &AppState, tab_id: &str, op: &str, block_id: &str, queued: Result<(), String>) -> PaneTabEditResult {
    if let Err(e) = queued {
        tracing::warn!(block_id, "{op}: queueing the frontend action failed: {e}");
    }
    let version = crate::server::service::layout_intents::layout_row_version(&state.mstore, tab_id);
    PaneTabEditResult { version }
}
