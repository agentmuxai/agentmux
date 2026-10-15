// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase E — srv reducer.
//
// The "same discipline as `agentmux-launcher::reducer`" below means exactly
// that: same purity rules, same `agentmux_common::ipc::{Command, Event}` wire
// types. Only `State` differs (each crate's own) and which variants this
// `update` actually handles — workspaces/tabs/blocks, not window/process.
// Two reducers by design; see
// docs/architecture/DECISION_SAGA_REDUCER_TWO_FRAMEWORKS_2026_09_07.md.
//
// Pure functional core: `update(&mut State, Command, &Ctx) -> Vec<Event>`.
// Never blocks, never awaits, never does I/O. Same discipline as
// `agentmux-launcher::reducer`. Mutex held only during dispatch
// (sub-millisecond).
//
// Arms, by domain module: lifecycle (Register / Goodbye / Ping /
// GetSrvSnapshot), workspace, tab, block, window (window↔workspace
// mapping), and layout (tree, focus/magnify, pane-tab stacks, backend
// action queue). Multi-step flows (tear-off, restore, move) are sagas
// that dispatch these atomic arms.
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


/// Text in the `MoveTab` last-tab refusal; `MoveTabToWorkspace` matches on
/// it to return its long-standing error string unchanged.
pub const MOVE_TAB_LAST_TAB_ERROR: &str = "cannot move last tab out of workspace";

/// Per-dispatch context, built by the caller so `update` stays pure.
#[derive(Debug, Clone)]
pub struct Ctx {
    /// The pid the dispatching connection registered.
    pub registered_pid: Option<u32>,
    /// The id a `CreateWorkspace`, `CreateTab` or `CreateBlock` arm gives the
    /// new entity. Other arms ignore it.
    pub new_id: String,
}

impl Ctx {
    /// A context for one dispatch, with a freshly minted `new_id`. Build one
    /// per dispatch: a reused context makes a second create refuse the id.
    pub fn new(registered_pid: Option<u32>) -> Self {
        Self {
            registered_pid,
            new_id: uuid::Uuid::new_v4().to_string(),
        }
    }
}

/// The non-fatal `Event::Error` reply an arm gives a command it refuses,
/// leaving state untouched.
fn error_event(state: &mut State, code: ErrorCode, message: impl Into<String>) -> Vec<Event> {
    let version = state.bump_version();
    vec![Event::Error {
        code,
        message: message.into(),
        fatal: false,
        version,
    }]
}

/// `error_event` with `ErrorCode::InvalidCommand`, the code nearly every
/// refusal uses.
fn invalid_command(state: &mut State, message: impl Into<String>) -> Vec<Event> {
    error_event(state, ErrorCode::InvalidCommand, message)
}

pub fn update(state: &mut State, cmd: Command, ctx: &Ctx) -> Vec<Event> {
    match cmd {
        Command::Register { kind, pid, version } => lifecycle::handle_register(state, kind, pid, version),
        Command::Goodbye => lifecycle::handle_goodbye(state, ctx),
        Command::Ping { nonce } => {
            let v = state.bump_version();
            vec![Event::Pong { nonce, version: v }]
        }
        Command::GetSrvSnapshot => snapshot::handle_get_srv_snapshot(state),
        Command::GetEvents { .. } => Vec::new(), // intercepted by server; unreachable
        Command::CreateWorkspace { name } => {
            workspace::handle_create_workspace(state, ctx.new_id.clone(), name)
        }
        Command::DeleteWorkspace { workspace_id, force } => {
            workspace::handle_delete_workspace(state, workspace_id, force)
        }
        Command::CreateTab { workspace_id, name } => {
            tab::handle_create_tab(state, workspace_id, ctx.new_id.clone(), name)
        }
        Command::DeleteTab { workspace_id, tab_id, force } => tab::handle_delete_tab(state, workspace_id, tab_id, force),
        Command::SetActiveTab { workspace_id, tab_id } => {
            tab::handle_set_active_tab(state, workspace_id, tab_id)
        }
        Command::ReorderTab {
            workspace_id,
            tab_id,
            new_index,
        } => tab::handle_reorder_tab(state, workspace_id, tab_id, new_index),
        Command::CreateBlock { tab_id, meta } => {
            block::create_block_with_id(state, tab_id, ctx.new_id.clone(), meta)
        }
        Command::DeleteBlock { tab_id, block_id } => block::handle_delete_block(state, tab_id, block_id),
        Command::SetFocusedNode { tab_id, node_id } => {
            layout::handle_set_focused_node(state, tab_id, node_id)
        }
        Command::SetMagnifiedNode { tab_id, node_id } => {
            layout::handle_set_magnified_node(state, tab_id, node_id)
        }
        // Layout tree arms. Every one has a producer and is persisted from
        // its event.
        Command::LayoutSetTree {
            tab_id,
            new_tree,
            correlation_id,
            slices,
        } => layout::handle_layout_set_tree(state, tab_id, new_tree, correlation_id, slices),
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
        // SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md §3.2 — pane tabs.
        Command::LayoutStackPush {
            tab_id,
            target_block_id,
            block_id,
            activate,
            correlation_id,
        } => layout::handle_layout_stack_push(state, tab_id, target_block_id, block_id, activate, correlation_id),
        Command::LayoutStackActivate {
            tab_id,
            block_id,
            correlation_id,
        } => layout::handle_layout_stack_activate(state, tab_id, block_id, correlation_id),
        Command::CreateBlockInStack {
            tab_id,
            target_block_id,
            meta,
            activate,
        } => layout::handle_create_block_in_stack(state, tab_id, target_block_id, ctx.new_id.clone(), meta, activate),
        // SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §4.1 — reorder within a
        // pane or move a tab into a different pane.
        Command::LayoutStackMove {
            tab_id,
            block_id,
            target_block_id,
            position,
            activate,
            correlation_id,
        } => layout::handle_layout_stack_move(state, tab_id, block_id, target_block_id, position, activate, correlation_id),
        // SPEC_STRONG_REDUCER_AUTHORITY_LAYOUT_2026_06_30.md §4 — every
        // layout edit the frontend makes, computed here.
        Command::LayoutApplyIntent {
            tab_id,
            intent,
            action_id,
            view,
        } => layout::handle_layout_apply_intent(state, tab_id, intent, action_id, view),
        // SPEC_864 Phase 4 — queue-append pass-through (the reducer does
        // not model pendingbackendactions in TabRecord; the persist
        // subscriber appends to db_layout from the event).
        Command::LayoutQueueBackendActions {
            tab_id,
            actions,
            correlation_id,
        } => layout::handle_layout_queue_backend_actions(state, tab_id, actions, correlation_id),
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
            allow_empty_source,
        } => tab::handle_move_tab(
            state,
            tab_id,
            src_workspace_id,
            dst_workspace_id,
            dst_index,
            allow_empty_source,
        ),
        Command::MoveBlock {
            block_id,
            src_tab_id,
            dst_tab_id,
            dst_index,
        } => block::handle_move_block(state, block_id, src_tab_id, dst_tab_id, dst_index),
        // Anything else (launcher-domain commands, unimplemented layout
        // ops) is a non-fatal protocol error that changes no state.
        other => invalid_command(state, format!("srv reducer does not accept: {:?}", other)),
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
