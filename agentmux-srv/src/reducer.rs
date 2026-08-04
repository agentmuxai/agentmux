// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase E — srv reducer.
//
// Pure functional core: `update(&mut State, Command, &Ctx) -> Vec<Event>`.
// Never blocks, never awaits, never does I/O. Same discipline as
// `agentmux-launcher::reducer`. Mutex held only during dispatch
// (sub-millisecond).
//
// Arms by phase:
//   * E.1b — Register / Goodbye / Ping / GetSrvSnapshot / GetEvents
//   * E.2  — CreateWorkspace / DeleteWorkspace
//   * E.2b — CreateTab / DeleteTab / SetActiveTab / ReorderTab
//   * E.3  — CreateBlock / DeleteBlock
//   * E.5  — CreateWindow / CloseWindowInternal / SwitchWorkspace
//             (window↔workspace mapping for sagas)
//   * E.5+ — saga-driven multi-step commands (TearOff/Restore/Move)
//             land via the saga coordinator dispatching atomic arms
//
// `Command::GetEvents` is intercepted by the IPC server before
// reaching the reducer (server queries the event log; reducer
// stays pure). The reducer's arm exists only for match
// exhaustiveness; same pattern as the launcher reducer.


mod block;
mod layout;
mod lifecycle;
mod snapshot;
mod tab;
#[cfg(test)]
mod test_support;
mod window;
mod workspace;

use agentmux_common::ipc::{Command, ErrorCode, Event};
use crate::state::State;

/// Per-dispatch context. Currently just an RFC3339 timestamp.
#[derive(Debug, Clone)]
pub struct Ctx {
    pub now_rfc3339: String,
    pub registered_pid: Option<u32>,
}

pub fn update(state: &mut State, cmd: Command, ctx: &Ctx) -> Vec<Event> {
    match cmd {
        Command::Register { kind, pid, version } => lifecycle::handle_register(state, ctx, kind, pid, version),
        Command::Goodbye => lifecycle::handle_goodbye(state, ctx),
        Command::Ping { nonce } => {
            let v = state.bump_version();
            vec![Event::Pong { nonce, version: v }]
        }
        Command::GetSrvSnapshot => snapshot::handle_get_srv_snapshot(state),
        Command::GetEvents { .. } => Vec::new(), // intercepted by server; unreachable
        Command::CreateWorkspace { name } => workspace::handle_create_workspace(state, name),
        Command::DeleteWorkspace { workspace_id, force } => {
            workspace::handle_delete_workspace(state, workspace_id, force)
        }
        Command::CreateTab { workspace_id, name } => tab::handle_create_tab(state, workspace_id, name),
        Command::DeleteTab { workspace_id, tab_id, force } => tab::handle_delete_tab(state, workspace_id, tab_id, force),
        Command::SetActiveTab { workspace_id, tab_id } => {
            tab::handle_set_active_tab(state, workspace_id, tab_id)
        }
        Command::ReorderTab {
            workspace_id,
            tab_id,
            new_index,
        } => tab::handle_reorder_tab(state, workspace_id, tab_id, new_index),
        Command::CreateBlock { tab_id, meta } => block::handle_create_block(state, tab_id, meta),
        Command::DeleteBlock { tab_id, block_id } => block::handle_delete_block(state, tab_id, block_id),
        Command::SetFocusedNode { tab_id, node_id } => {
            layout::handle_set_focused_node(state, tab_id, node_id)
        }
        Command::SetMagnifiedNode { tab_id, node_id } => {
            layout::handle_set_magnified_node(state, tab_id, node_id)
        }
        // Phase E.4.B Phase 5 — layout tree mutation arms. All 11 arms are
        // wired. First production dispatcher: the `UpdateObject`→
        // `LayoutSetTree` reroute (SPEC_864 Phase 2, `object.rs`); the
        // remaining wcore-direct writers migrate in SPEC_864 Phases 3–5.
        Command::LayoutClear {
            tab_id,
            correlation_id,
        } => layout::handle_layout_clear(state, tab_id, correlation_id),
        Command::LayoutSetTree {
            tab_id,
            new_tree,
            correlation_id,
            slices,
        } => layout::handle_layout_set_tree(state, tab_id, new_tree, correlation_id, slices),
        Command::LayoutInsertNode {
            tab_id,
            node,
            parent_id,
            index,
            focus_after,
            magnify_after,
            correlation_id,
        } => layout::handle_layout_insert_node(
            state,
            tab_id,
            node,
            parent_id,
            index,
            focus_after,
            magnify_after,
            correlation_id,
        ),
        Command::LayoutDeleteNode {
            tab_id,
            node_id,
            correlation_id,
        } => layout::handle_layout_delete_node(state, tab_id, node_id, correlation_id),
        // SPEC_864 site #6 — block→node resolution in the arm; silent
        // no-op when the block has no layout node (see the handler doc).
        Command::LayoutDeleteNodeByBlock {
            tab_id,
            block_id,
            correlation_id,
        } => layout::handle_layout_delete_node_by_block(state, tab_id, block_id, correlation_id),
        // SPEC_864 Phase 4 — queue-append pass-through (the reducer does
        // not model pendingbackendactions in TabRecord; the persist
        // subscriber appends to db_layout from the event).
        Command::LayoutQueueBackendActions {
            tab_id,
            actions,
            correlation_id,
        } => layout::handle_layout_queue_backend_actions(state, tab_id, actions, correlation_id),
        // Phase 3 — the remaining 7 layout-tree arms. Each resolves the
        // tab, calls the existing pure fn in `backend::layout`, runs
        // `balance_node` (matching the frontend's post-action normalize),
        // reconciles dangling focus/magnify, and emits the granular event.
        Command::LayoutMoveNode {
            tab_id,
            node_id,
            new_parent_id,
            index,
            correlation_id,
        } => layout::handle_layout_move_node(
            state,
            tab_id,
            node_id,
            new_parent_id,
            index,
            correlation_id,
        ),
        Command::LayoutSwapNodes {
            tab_id,
            node1_id,
            node2_id,
            correlation_id,
        } => layout::handle_layout_swap_nodes(state, tab_id, node1_id, node2_id, correlation_id),
        Command::LayoutResizeNodes {
            tab_id,
            ops,
            correlation_id,
        } => layout::handle_layout_resize_nodes(state, tab_id, ops, correlation_id),
        Command::LayoutReplaceNode {
            tab_id,
            target_id,
            new_node,
            focus_after,
            correlation_id,
        } => layout::handle_layout_replace_node(
            state,
            tab_id,
            target_id,
            new_node,
            focus_after,
            correlation_id,
        ),
        Command::LayoutSplitHorizontal {
            tab_id,
            target_id,
            new_node,
            position,
            focus_after,
            correlation_id,
        } => layout::handle_layout_split_horizontal(
            state,
            tab_id,
            target_id,
            new_node,
            position,
            focus_after,
            correlation_id,
        ),
        Command::LayoutSplitVertical {
            tab_id,
            target_id,
            new_node,
            position,
            focus_after,
            correlation_id,
        } => layout::handle_layout_split_vertical(
            state,
            tab_id,
            target_id,
            new_node,
            position,
            focus_after,
            correlation_id,
        ),
        Command::LayoutInsertNodeAtIndex {
            tab_id,
            node,
            index_arr,
            focus_after,
            magnify_after,
            correlation_id,
        } => layout::handle_layout_insert_node_at_index(
            state,
            tab_id,
            node,
            index_arr,
            focus_after,
            magnify_after,
            correlation_id,
        ),
        Command::CreateWindow {
            window_id,
            workspace_id,
        } => window::handle_create_window(state, window_id, workspace_id),
        Command::CloseWindowInternal { window_id } => {
            window::handle_close_window_internal(state, window_id)
        }
        Command::SwitchWorkspace {
            window_id,
            workspace_id,
        } => window::handle_switch_workspace(state, window_id, workspace_id),
        Command::ReorderTabsBulk {
            workspace_id,
            tab_ids,
        } => tab::handle_reorder_tabs_bulk(state, workspace_id, tab_ids),
        Command::RenameWorkspace { workspace_id, name } => {
            workspace::handle_rename_workspace(state, workspace_id, name)
        }
        Command::RenameTab { tab_id, name } => tab::handle_rename_tab(state, tab_id, name),
        Command::UpdateWorkspaceMeta {
            workspace_id,
            meta_patch,
        } => workspace::handle_update_workspace_meta(state, workspace_id, meta_patch),
        Command::UpdateTabMeta {
            tab_id,
            meta_patch,
        } => tab::handle_update_tab_meta(state, tab_id, meta_patch),
        Command::UpdateBlockMeta {
            block_id,
            meta_patch,
        } => block::handle_update_block_meta(state, block_id, meta_patch),
        Command::UpdateWindowMeta {
            window_id,
            meta_patch,
        } => window::handle_update_window_meta(state, window_id, meta_patch),
        Command::MoveTab {
            tab_id,
            src_workspace_id,
            dst_workspace_id,
            dst_index,
        } => tab::handle_move_tab(state, tab_id, src_workspace_id, dst_workspace_id, dst_index),
        Command::MoveBlock {
            block_id,
            src_tab_id,
            dst_tab_id,
            dst_index,
        } => block::handle_move_block(state, block_id, src_tab_id, dst_tab_id, dst_index),
        // Anything else is a non-fatal protocol error. Future
        // phases (E.2b tabs, E.3 blocks, E.4 layouts) extend this
        // match by adding new arms above.
        other => {
            let v = state.bump_version();
            vec![Event::Error {
                code: ErrorCode::InvalidCommand,
                message: format!("srv reducer does not accept: {:?}", other),
                fatal: false,
                version: v,
            }]
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reducer::test_support::*;

    // This test exercises the `update()` dispatch fallback itself (the
    // catch-all `other => { ... InvalidCommand ... }` arm above) rather
    // than any single domain submodule's logic, so it stays here instead
    // of moving to a `reducer::*` submodule.
    #[test]
    fn unaccepted_command_returns_invalid_command_error() {
        let mut state = State::default();
        let events = update(
            &mut state,
            Command::ReportWindowOpened {
                label: "main".into(),
                kind: agentmux_common::ipc::WindowKind::FullInstance,
                parent_label: None,
            },
            &ctx(1),
        );
        assert!(matches!(
            &events[0],
            Event::Error {
                code: ErrorCode::InvalidCommand,
                fatal: false,
                ..
            }
        ));
    }
}
