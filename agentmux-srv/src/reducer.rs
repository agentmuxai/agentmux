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
mod window;
mod workspace;

use agentmux_common::ipc::{Command, ErrorCode, Event};
use crate::state::State;
// Test-only imports: tests construct fixtures that mention these types
// directly, but the dispatch+Ctx in this file goes through the
// per-domain submodules.
#[cfg(test)]
use agentmux_common::ipc::{ClientKind, LifecyclePhase};
#[cfg(test)]
use crate::state::ProcessState;

/// Per-dispatch context. Currently just an RFC3339 timestamp + the
/// originating connection's `conn_id` for log correlation.
#[derive(Debug, Clone)]
pub struct Ctx {
    pub now_rfc3339: String,
    pub conn_id: u64,
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
        // Phase E.4.B Phase 5 — layout tree mutation arms. Currently
        // dormant scaffolding (no production callers; Phase 7 migrates
        // the wcore-direct writers to dispatch through these). 4 of 11
        // arms shipped in this PR; remaining 7 (insert_at_index, move,
        // swap, resize, replace, split_horizontal, split_vertical) are
        // structurally identical and follow in subsequent PRs.
        Command::LayoutClear {
            tab_id,
            correlation_id,
        } => layout::handle_layout_clear(state, tab_id, correlation_id),
        Command::LayoutSetTree {
            tab_id,
            new_tree,
            correlation_id,
        } => layout::handle_layout_set_tree(state, tab_id, new_tree, correlation_id),
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

    fn ctx(conn_id: u64) -> Ctx {
        Ctx {
            now_rfc3339: "2026-04-30T00:00:00Z".to_string(),
            conn_id,
            registered_pid: None,
        }
    }

    fn ctx_with_pid(conn_id: u64, pid: u32) -> Ctx {
        Ctx {
            now_rfc3339: "2026-04-30T00:00:00Z".to_string(),
            conn_id,
            registered_pid: Some(pid),
        }
    }

    fn extract_version(e: &Event) -> u64 {
        match e {
            Event::ProcessSpawned { version, .. }
            | Event::ProcessExited { version, .. }
            | Event::LifecyclePhaseChanged { version, .. }
            | Event::Registered { version, .. }
            | Event::Pong { version, .. }
            | Event::WindowOpened { version, .. }
            | Event::WindowClosed { version, .. }
            | Event::PoolWindowAdded { version, .. }
            | Event::PoolWindowRemoved { version, .. }
            | Event::PoolWindowPromoted { version, .. }
            | Event::PanesReaped { version, .. }
            | Event::PoolDrained { version, .. }
            | Event::PoolNotLast { version, .. }
            | Event::WindowInstanceAssigned { version, .. }
            | Event::WindowInstanceReleased { version, .. }
            | Event::BackendWindowIdRegistered { version, .. }
            | Event::BackendWindowIdUnregistered { version, .. }
            | Event::DriftDetected { version, .. }
            | Event::HwndDriftDetected { version, .. }
            | Event::CorrectiveWindowMove { version, .. }
            | Event::HostShouldQuit { version, .. }
            | Event::Snapshot { version, .. }
            | Event::EventList { version, .. }
            | Event::SrvSnapshot { version, .. }
            | Event::SagaStarted { version, .. }
            | Event::SagaCompleted { version, .. }
            | Event::SagaFailed { version, .. }
            | Event::WorkspaceCreated { version, .. }
            | Event::WorkspaceDeleted { version, .. }
            | Event::TabCreated { version, .. }
            | Event::TabDeleted { version, .. }
            | Event::ActiveTabChanged { version, .. }
            | Event::TabReordered { version, .. }
            | Event::BlockCreated { version, .. }
            | Event::BlockDeleted { version, .. }
            | Event::SrvWindowOpened { version, .. }
            | Event::SrvWindowClosed { version, .. }
            | Event::SrvWindowWorkspaceChanged { version, .. }
            | Event::TabsReorderedBulk { version, .. }
            | Event::WorkspaceRenamed { version, .. }
            | Event::TabRenamed { version, .. }
            | Event::WorkspaceMetaUpdated { version, .. }
            | Event::TabMetaUpdated { version, .. }
            | Event::BlockMetaUpdated { version, .. }
            | Event::TabMoved { version, .. }
            | Event::BlockMoved { version, .. }
            | Event::FocusedNodeChanged { version, .. }
            | Event::MagnifiedNodeChanged { version, .. }
            | Event::SagaActionFailed { version, .. }
            | Event::Error { version, .. }
            // Phase E.4.B — layout tree events.
            | Event::LayoutNodeInserted { version, .. }
            | Event::LayoutNodeInsertedAtIndex { version, .. }
            | Event::LayoutNodeDeleted { version, .. }
            | Event::LayoutNodeMoved { version, .. }
            | Event::LayoutNodesSwapped { version, .. }
            | Event::LayoutNodesResized { version, .. }
            | Event::LayoutNodeReplaced { version, .. }
            | Event::LayoutSplitHorizontalApplied { version, .. }
            | Event::LayoutSplitVerticalApplied { version, .. }
            | Event::LayoutCleared { version, .. }
            | Event::LayoutTreeReplaced { version, .. } => *version,
        }
    }

    #[test]
    fn first_register_transitions_to_running() {
        let mut state = State::default();
        let events = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Host,
                pid: 100,
                version: "0.0.0".into(),
            },
            &ctx(1),
        );
        assert_eq!(state.lifecycle, LifecyclePhase::Running);
        assert!(state.processes.contains_key(&100));
        assert_eq!(events.len(), 3);
        assert!(matches!(&events[0], Event::ProcessSpawned { pid: 100, .. }));
        assert!(matches!(
            &events[1],
            Event::LifecyclePhaseChanged {
                from: LifecyclePhase::Starting,
                to: LifecyclePhase::Running,
                ..
            }
        ));
        assert!(matches!(&events[2], Event::Registered { .. }));
    }

    #[test]
    fn second_register_does_not_re_emit_lifecycle_change() {
        let mut state = State::default();
        let _ = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Host,
                pid: 100,
                version: "v1".into(),
            },
            &ctx(1),
        );
        let events = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Tool,
                pid: 200,
                version: "v2".into(),
            },
            &ctx(2),
        );
        assert_eq!(state.lifecycle, LifecyclePhase::Running);
        // ProcessSpawned + Registered, no LifecyclePhaseChanged.
        assert_eq!(events.len(), 2);
        assert!(events
            .iter()
            .all(|e| !matches!(e, Event::LifecyclePhaseChanged { .. })));
    }

    #[test]
    fn duplicate_register_returns_already_registered_error() {
        let mut state = State::default();
        let _ = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Host,
                pid: 100,
                version: "v1".into(),
            },
            &ctx(1),
        );
        let events = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Host,
                pid: 100,
                version: "v2".into(),
            },
            &ctx(2),
        );
        assert!(matches!(
            &events[0],
            Event::Error {
                code: ErrorCode::AlreadyRegistered,
                ..
            }
        ));
        // Original record preserved.
        assert_eq!(&state.processes[&100].version, "v1");
    }

    #[test]
    fn register_replaces_exited_record_for_recycled_pid() {
        let mut state = State::default();
        let _ = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Host,
                pid: 100,
                version: "v1".into(),
            },
            &ctx(1),
        );
        let _ = update(&mut state, Command::Goodbye, &ctx_with_pid(1, 100));
        // Re-register with same PID — allowed because prior is Exited.
        let events = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Host,
                pid: 100,
                version: "v2".into(),
            },
            &ctx(2),
        );
        assert!(matches!(&events[0], Event::ProcessSpawned { .. }));
        assert_eq!(&state.processes[&100].version, "v2");
        assert!(matches!(state.processes[&100].state, ProcessState::Running));
    }

    #[test]
    fn ping_returns_pong_with_same_nonce() {
        let mut state = State::default();
        let events = update(&mut state, Command::Ping { nonce: 42 }, &ctx(1));
        assert!(matches!(&events[0], Event::Pong { nonce: 42, .. }));
    }

    #[test]
    fn get_srv_snapshot_returns_lifecycle_and_bumps_version() {
        let mut state = State::default();
        let v0 = state.event_version;
        let events = update(&mut state, Command::GetSrvSnapshot, &ctx(1));
        assert_eq!(events.len(), 1);
        let Event::SrvSnapshot { version, lifecycle, .. } = events[0].clone() else {
            panic!("expected SrvSnapshot, got {:?}", events[0]);
        };
        assert_eq!(lifecycle, LifecyclePhase::Starting);
        assert!(version > v0);
    }

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

    #[test]
    fn versions_strictly_monotonic_across_sequence() {
        let mut state = State::default();
        let mut versions = vec![];
        for pid in [100, 200, 300] {
            let events = update(
                &mut state,
                Command::Register {
                    kind: ClientKind::Host,
                    pid,
                    version: "v".into(),
                },
                &ctx(pid as u64),
            );
            versions.extend(events.iter().map(extract_version));
        }
        for w in versions.windows(2) {
            assert!(w[1] > w[0], "version regression: {} -> {}", w[0], w[1]);
        }
    }

    #[test]
    fn create_workspace_inserts_record_and_emits_event() {
        let mut state = State::default();
        let events = update(
            &mut state,
            Command::CreateWorkspace {
                name: "myws".into(),
            },
            &ctx(1),
        );
        assert_eq!(events.len(), 1);
        let Event::WorkspaceCreated {
            workspace_id, name, ..
        } = &events[0]
        else {
            panic!("expected WorkspaceCreated, got {:?}", events[0]);
        };
        assert_eq!(name, "myws");
        assert!(state.workspaces.contains_key(workspace_id));
        assert_eq!(state.workspaces[workspace_id].name, "myws");
    }

    #[test]
    fn delete_workspace_removes_record_and_emits_event() {
        let mut state = State::default();
        let events = update(
            &mut state,
            Command::CreateWorkspace {
                name: "to-delete".into(),
            },
            &ctx(1),
        );
        let Event::WorkspaceCreated { workspace_id, .. } = &events[0] else {
            panic!();
        };
        let ws_id = workspace_id.clone();
        let events = update(
            &mut state,
            Command::DeleteWorkspace {
                workspace_id: ws_id.clone(),
                force: false,
            },
            &ctx(2),
        );
        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            Event::WorkspaceDeleted { workspace_id, .. } if workspace_id == &ws_id
        ));
        assert!(!state.workspaces.contains_key(&ws_id));
    }

    #[test]
    fn delete_workspace_unknown_is_silent_no_op() {
        let mut state = State::default();
        let events = update(
            &mut state,
            Command::DeleteWorkspace {
                workspace_id: "does-not-exist".into(),
                force: false,
            },
            &ctx(1),
        );
        assert!(events.is_empty());
    }

    #[test]
    fn snapshot_includes_workspaces_sorted_by_id() {
        let mut state = State::default();
        let _ = update(
            &mut state,
            Command::CreateWorkspace { name: "a".into() },
            &ctx(1),
        );
        let _ = update(
            &mut state,
            Command::CreateWorkspace { name: "b".into() },
            &ctx(2),
        );
        let events = update(&mut state, Command::GetSrvSnapshot, &ctx(3));
        let Event::SrvSnapshot { workspaces, .. } = &events[0] else {
            panic!();
        };
        assert_eq!(workspaces.len(), 2);
        // Sorted by id; verify ordering deterministic (ascending).
        assert!(workspaces[0].0 < workspaces[1].0);
    }

    fn create_workspace(state: &mut State, name: &str) -> String {
        let events = update(
            state,
            Command::CreateWorkspace { name: name.into() },
            &ctx(1),
        );
        match &events[0] {
            Event::WorkspaceCreated { workspace_id, .. } => workspace_id.clone(),
            _ => panic!("expected WorkspaceCreated"),
        }
    }

    #[test]
    fn create_tab_validates_workspace_exists() {
        let mut state = State::default();
        let events = update(
            &mut state,
            Command::CreateTab {
                workspace_id: "no-such-ws".into(),
                name: "t1".into(),
            },
            &ctx(1),
        );
        assert!(matches!(&events[0], Event::Error { .. }));
        assert!(state.tabs.is_empty());
    }

    #[test]
    fn create_tab_first_tab_becomes_active() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let events = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t1".into(),
            },
            &ctx(1),
        );
        // First event: TabCreated; second: ActiveTabChanged.
        assert!(matches!(&events[0], Event::TabCreated { .. }));
        assert!(matches!(&events[1], Event::ActiveTabChanged { .. }));
        let workspace = &state.workspaces[&ws_id];
        assert_eq!(workspace.tab_ids.len(), 1);
        assert_eq!(workspace.active_tab_id, Some(workspace.tab_ids[0].clone()));
        assert_eq!(state.tabs.len(), 1);
    }

    #[test]
    fn create_tab_second_tab_does_not_steal_active() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let _ = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t1".into(),
            },
            &ctx(1),
        );
        let first_active = state.workspaces[&ws_id].active_tab_id.clone();
        let events = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t2".into(),
            },
            &ctx(2),
        );
        // Only TabCreated — second tab does not become active.
        assert!(matches!(&events[0], Event::TabCreated { .. }));
        assert_eq!(events.len(), 1);
        assert_eq!(state.workspaces[&ws_id].active_tab_id, first_active);
        assert_eq!(state.workspaces[&ws_id].tab_ids.len(), 2);
    }

    #[test]
    fn delete_tab_removes_from_state_and_workspace_list() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let _ = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t1".into(),
            },
            &ctx(1),
        );
        let _ = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t2".into(),
            },
            &ctx(2),
        );
        let tab2_id = state.workspaces[&ws_id].tab_ids[1].clone();
        let events = update(
            &mut state,
            Command::DeleteTab {
                workspace_id: ws_id.clone(),
                tab_id: tab2_id.clone(),
                force: false,
            },
            &ctx(3),
        );
        assert!(matches!(&events[0], Event::TabDeleted { .. }));
        // tab2 wasn't active, so no ActiveTabChanged.
        assert_eq!(events.len(), 1);
        assert!(!state.tabs.contains_key(&tab2_id));
        assert_eq!(state.workspaces[&ws_id].tab_ids.len(), 1);
    }

    #[test]
    fn delete_active_tab_promotes_neighbor() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let _ = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t1".into(),
            },
            &ctx(1),
        );
        let _ = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t2".into(),
            },
            &ctx(2),
        );
        let tab1_id = state.workspaces[&ws_id].tab_ids[0].clone();
        let tab2_id = state.workspaces[&ws_id].tab_ids[1].clone();
        // tab1 was created first → it's active. Delete it.
        let events = update(
            &mut state,
            Command::DeleteTab {
                workspace_id: ws_id.clone(),
                tab_id: tab1_id.clone(),
                force: false,
            },
            &ctx(3),
        );
        assert!(matches!(&events[0], Event::TabDeleted { .. }));
        assert!(matches!(
            &events[1],
            Event::ActiveTabChanged { tab_id: Some(_), .. }
        ));
        // tab2 should now be active.
        assert_eq!(state.workspaces[&ws_id].active_tab_id, Some(tab2_id));
    }

    #[test]
    fn delete_last_tab_clears_active_to_none() {
        // Reducer accepts last-tab delete (round 2 of PR #633 walked
        // back the guard). User-facing flows gate at the call site
        // (close button + keymodel both check `tab_ids.len() <= 1`);
        // internal compensation paths rely on this acceptance to
        // roll back failed CreateTab persists.
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let _ = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t1".into(),
            },
            &ctx(1),
        );
        let tab1_id = state.workspaces[&ws_id].tab_ids[0].clone();
        let events = update(
            &mut state,
            Command::DeleteTab {
                workspace_id: ws_id.clone(),
                tab_id: tab1_id,
                // Last-tab delete needs force=true post-round-4
                // (codex P2 #633). Test asserts the
                // ActiveTabChanged-to-None behavior still works
                // when compensation paths force a last-tab delete.
                force: true,
            },
            &ctx(2),
        );
        assert!(matches!(&events[0], Event::TabDeleted { .. }));
        assert!(matches!(
            &events[1],
            Event::ActiveTabChanged { tab_id: None, .. }
        ));
        assert_eq!(state.workspaces[&ws_id].active_tab_id, None);
        assert!(state.tabs.is_empty());
    }

    #[test]
    fn delete_unknown_tab_silent_no_op() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let events = update(
            &mut state,
            Command::DeleteTab {
                workspace_id: ws_id,
                tab_id: "ghost".into(),
                force: false,
            },
            &ctx(1),
        );
        assert!(events.is_empty());
    }

    #[test]
    fn set_active_tab_validates_workspace_and_tab() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let _ = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t1".into(),
            },
            &ctx(1),
        );
        // Wrong workspace.
        let events = update(
            &mut state,
            Command::SetActiveTab {
                workspace_id: "no-such".into(),
                tab_id: "x".into(),
            },
            &ctx(2),
        );
        assert!(matches!(&events[0], Event::Error { .. }));
        // Right workspace, wrong tab.
        let events = update(
            &mut state,
            Command::SetActiveTab {
                workspace_id: ws_id,
                tab_id: "ghost".into(),
            },
            &ctx(3),
        );
        assert!(matches!(&events[0], Event::Error { .. }));
    }

    #[test]
    fn set_active_tab_idempotent_no_event_when_already_active() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let _ = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t1".into(),
            },
            &ctx(1),
        );
        let tab_id = state.workspaces[&ws_id].tab_ids[0].clone();
        // Already active (auto-activated on first tab create).
        let events = update(
            &mut state,
            Command::SetActiveTab {
                workspace_id: ws_id,
                tab_id,
            },
            &ctx(2),
        );
        assert!(events.is_empty());
    }

    #[test]
    fn reorder_tab_moves_to_new_index() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        for i in 1..=3 {
            let _ = update(
                &mut state,
                Command::CreateTab {
                    workspace_id: ws_id.clone(),
                    name: format!("t{}", i),
                },
                &ctx(i),
            );
        }
        let original = state.workspaces[&ws_id].tab_ids.clone();
        // Move first tab to index 2 (last).
        let events = update(
            &mut state,
            Command::ReorderTab {
                workspace_id: ws_id.clone(),
                tab_id: original[0].clone(),
                new_index: 2,
            },
            &ctx(10),
        );
        assert!(matches!(&events[0], Event::TabReordered { .. }));
        let after = &state.workspaces[&ws_id].tab_ids;
        assert_eq!(after[0], original[1]);
        assert_eq!(after[1], original[2]);
        assert_eq!(after[2], original[0]);
    }

    #[test]
    fn reorder_tab_clamps_to_last_index() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let _ = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t1".into(),
            },
            &ctx(1),
        );
        let _ = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t2".into(),
            },
            &ctx(2),
        );
        let original = state.workspaces[&ws_id].tab_ids.clone();
        // Asking for index 99 should clamp to 1 (len-1).
        let events = update(
            &mut state,
            Command::ReorderTab {
                workspace_id: ws_id.clone(),
                tab_id: original[0].clone(),
                new_index: 99,
            },
            &ctx(3),
        );
        if let Event::TabReordered { new_index, .. } = &events[0] {
            assert_eq!(*new_index, 1);
        } else {
            panic!("expected TabReordered");
        }
    }

    #[test]
    fn reorder_tab_already_at_position_no_op() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let _ = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t1".into(),
            },
            &ctx(1),
        );
        let tab_id = state.workspaces[&ws_id].tab_ids[0].clone();
        let events = update(
            &mut state,
            Command::ReorderTab {
                workspace_id: ws_id,
                tab_id,
                new_index: 0,
            },
            &ctx(2),
        );
        assert!(events.is_empty());
    }

    #[test]
    fn reorder_tab_validates_workspace_and_tab() {
        let mut state = State::default();
        let events = update(
            &mut state,
            Command::ReorderTab {
                workspace_id: "no-such-ws".into(),
                tab_id: "x".into(),
                new_index: 0,
            },
            &ctx(1),
        );
        assert!(matches!(&events[0], Event::Error { .. }));
        let ws_id = create_workspace(&mut state, "w");
        let events = update(
            &mut state,
            Command::ReorderTab {
                workspace_id: ws_id,
                tab_id: "ghost".into(),
                new_index: 0,
            },
            &ctx(2),
        );
        assert!(matches!(&events[0], Event::Error { .. }));
    }

    /// codex P1 #620 carryover: `ReorderTabsBulk` must accept a list
    /// containing tab_ids the reducer hasn't seen yet (because they
    /// arrived via wcore-direct paths like `MoveTabToWorkspace`).
    /// Strict permutation validation against the reducer's stale view
    /// would falsely reject the canonical SQLite ordering during the
    /// migration window.
    #[test]
    fn reorder_tabs_bulk_accepts_unknown_ids_during_migration() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let known = create_tab(&mut state, &ws_id, "known");
        // Simulate a wcore-direct move that landed a new tab in this
        // workspace's SQLite list without going through the reducer.
        // The reducer's `workspace.tab_ids` is now stale: it knows
        // about `known` only, but SQLite has both `known` and
        // `imported` (and the latter belongs to an entirely different
        // workspace from the reducer's perspective).
        let imported = "imported-tab".to_string();
        let events = update(
            &mut state,
            Command::ReorderTabsBulk {
                workspace_id: ws_id.clone(),
                tab_ids: vec![imported.clone(), known.clone()],
            },
            &ctx(99),
        );
        assert!(
            matches!(&events[0], Event::TabsReorderedBulk { .. }),
            "expected TabsReorderedBulk, got {:?}",
            events.first()
        );
        let ws = state.workspaces.get(&ws_id).expect("ws still present");
        assert_eq!(ws.tab_ids, vec![imported, known]);
    }

    /// codex P1 #620 carryover: a duplicate tab_id in the new list
    /// is still rejected — that would corrupt the persisted ordering
    /// in a way the subscriber can't recover from.
    #[test]
    fn reorder_tabs_bulk_rejects_duplicates() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let t1 = create_tab(&mut state, &ws_id, "t1");
        let events = update(
            &mut state,
            Command::ReorderTabsBulk {
                workspace_id: ws_id,
                tab_ids: vec![t1.clone(), t1.clone()],
            },
            &ctx(2),
        );
        match &events[0] {
            Event::Error { code, message, .. } => {
                assert_eq!(*code, ErrorCode::InvalidCommand);
                assert!(
                    message.contains("duplicate"),
                    "error should mention duplicate, got: {}",
                    message
                );
            }
            other => panic!("expected Error event, got {:?}", other),
        }
    }

    #[test]
    fn reorder_tabs_bulk_validates_workspace() {
        let mut state = State::default();
        let events = update(
            &mut state,
            Command::ReorderTabsBulk {
                workspace_id: "no-such-ws".into(),
                tab_ids: vec!["a".into()],
            },
            &ctx(1),
        );
        assert!(matches!(&events[0], Event::Error { .. }));
    }

    #[test]
    fn delete_workspace_cascades_tabs() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let _ = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t1".into(),
            },
            &ctx(1),
        );
        let _ = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t2".into(),
            },
            &ctx(2),
        );
        assert_eq!(state.tabs.len(), 2);
        let events = update(
            &mut state,
            Command::DeleteWorkspace {
                workspace_id: ws_id.clone(),
                force: false,
            },
            &ctx(3),
        );
        assert!(matches!(&events[0], Event::WorkspaceDeleted { .. }));
        assert!(state.tabs.is_empty());
        assert!(!state.workspaces.contains_key(&ws_id));
    }

    fn create_tab(state: &mut State, workspace_id: &str, name: &str) -> String {
        let events = update(
            state,
            Command::CreateTab {
                workspace_id: workspace_id.into(),
                name: name.into(),
            },
            &ctx(1),
        );
        match &events[0] {
            Event::TabCreated { tab_id, .. } => tab_id.clone(),
            _ => panic!("expected TabCreated"),
        }
    }

    /// codex P2 #622: empty name auto-generates `tabN`, mirroring
    /// `wcore::create_tab`'s default-naming behaviour. Without this,
    /// CreateWindow's "fresh workspace" path + TearOffBlock's new tab
    /// would land with blank titles — a user-visible regression.
    #[test]
    fn create_tab_auto_generates_tabN_when_name_empty() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let events = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: String::new(),
            },
            &ctx(2),
        );
        match &events[0] {
            Event::TabCreated { name, tab_id, .. } => {
                assert_eq!(name, "tab1", "first tab in fresh workspace");
                assert_eq!(state.tabs[tab_id].name, "tab1");
            }
            other => panic!("expected TabCreated, got {:?}", other),
        }
        // Second empty-name CreateTab → "tab2".
        let events = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: String::new(),
            },
            &ctx(3),
        );
        match &events[0] {
            Event::TabCreated { name, .. } => assert_eq!(name, "tab2"),
            other => panic!("expected TabCreated, got {:?}", other),
        }
        // Explicit non-empty name passes through verbatim.
        let events = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id,
                name: "my custom tab".into(),
            },
            &ctx(4),
        );
        match &events[0] {
            Event::TabCreated { name, .. } => assert_eq!(name, "my custom tab"),
            other => panic!("expected TabCreated, got {:?}", other),
        }
    }

    #[test]
    fn create_block_validates_tab_exists() {
        let mut state = State::default();
        let events = update(
            &mut state,
            Command::CreateBlock { tab_id: "no-such-tab".into(), meta: serde_json::Value::Null },
            &ctx(1),
        );
        assert!(matches!(&events[0], Event::Error { .. }));
        assert!(state.blocks.is_empty());
    }

    #[test]
    fn create_block_appends_to_tab_block_ids() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let tab_id = create_tab(&mut state, &ws_id, "t");
        let events = update(
            &mut state,
            Command::CreateBlock { tab_id: tab_id.clone(), meta: serde_json::Value::Null },
            &ctx(2),
        );
        assert!(matches!(&events[0], Event::BlockCreated { .. }));
        let block_id = match &events[0] {
            Event::BlockCreated { block_id, .. } => block_id.clone(),
            _ => panic!(),
        };
        assert_eq!(state.tabs[&tab_id].block_ids, vec![block_id.clone()]);
        assert_eq!(state.blocks[&block_id].tab_id, tab_id);
    }

    #[test]
    fn delete_block_removes_from_state_and_tab() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let tab_id = create_tab(&mut state, &ws_id, "t");
        let _ = update(
            &mut state,
            Command::CreateBlock { tab_id: tab_id.clone(), meta: serde_json::Value::Null },
            &ctx(2),
        );
        let block_id = state.tabs[&tab_id].block_ids[0].clone();
        let events = update(
            &mut state,
            Command::DeleteBlock {
                tab_id: tab_id.clone(),
                block_id: block_id.clone(),
            },
            &ctx(3),
        );
        assert!(matches!(&events[0], Event::BlockDeleted { .. }));
        assert!(!state.blocks.contains_key(&block_id));
        assert!(state.tabs[&tab_id].block_ids.is_empty());
    }

    #[test]
    fn delete_block_unknown_silent_no_op() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let tab_id = create_tab(&mut state, &ws_id, "t");
        let events = update(
            &mut state,
            Command::DeleteBlock {
                tab_id,
                block_id: "ghost".into(),
            },
            &ctx(2),
        );
        assert!(events.is_empty());
    }

    // ---------------------------------------------------------------
    // Phase E.4 (Option A) — SetFocusedNode / SetMagnifiedNode
    // ---------------------------------------------------------------

    #[test]
    fn set_focused_node_round_trip_emits_event_and_updates_state() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let tab_id = create_tab(&mut state, &ws_id, "t1");
        assert_eq!(state.tabs[&tab_id].focused_node_id, "");
        let events = update(
            &mut state,
            Command::SetFocusedNode {
                tab_id: tab_id.clone(),
                node_id: "node-7".into(),
            },
            &ctx(2),
        );
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::FocusedNodeChanged { tab_id: t, node_id, .. } => {
                assert_eq!(t, &tab_id);
                assert_eq!(node_id, "node-7");
            }
            other => panic!("expected FocusedNodeChanged, got {:?}", other),
        }
        assert_eq!(state.tabs[&tab_id].focused_node_id, "node-7");
    }

    #[test]
    fn set_focused_node_no_op_when_value_unchanged() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let tab_id = create_tab(&mut state, &ws_id, "t1");
        let _ = update(
            &mut state,
            Command::SetFocusedNode {
                tab_id: tab_id.clone(),
                node_id: "node-1".into(),
            },
            &ctx(2),
        );
        let version_before = state.event_version;
        let events = update(
            &mut state,
            Command::SetFocusedNode {
                tab_id,
                node_id: "node-1".into(),
            },
            &ctx(3),
        );
        assert!(events.is_empty(), "no-op should emit no events");
        assert_eq!(state.event_version, version_before, "no version bump on no-op");
    }

    #[test]
    fn set_focused_node_unknown_tab_emits_error() {
        let mut state = State::default();
        let events = update(
            &mut state,
            Command::SetFocusedNode {
                tab_id: "ghost-tab".into(),
                node_id: "node-1".into(),
            },
            &ctx(1),
        );
        assert_eq!(events.len(), 1);
        assert!(
            matches!(&events[0], Event::Error { .. }),
            "expected Event::Error, got {:?}",
            events[0]
        );
    }

    #[test]
    fn set_magnified_node_round_trip_emits_event_and_updates_state() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let tab_id = create_tab(&mut state, &ws_id, "t1");
        let events = update(
            &mut state,
            Command::SetMagnifiedNode {
                tab_id: tab_id.clone(),
                node_id: "node-9".into(),
            },
            &ctx(2),
        );
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::MagnifiedNodeChanged { tab_id: t, node_id, .. } => {
                assert_eq!(t, &tab_id);
                assert_eq!(node_id, "node-9");
            }
            other => panic!("expected MagnifiedNodeChanged, got {:?}", other),
        }
        assert_eq!(state.tabs[&tab_id].magnified_node_id, "node-9");
    }

    #[test]
    fn set_magnified_node_no_op_when_value_unchanged() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let tab_id = create_tab(&mut state, &ws_id, "t1");
        let _ = update(
            &mut state,
            Command::SetMagnifiedNode {
                tab_id: tab_id.clone(),
                node_id: "node-2".into(),
            },
            &ctx(2),
        );
        let version_before = state.event_version;
        let events = update(
            &mut state,
            Command::SetMagnifiedNode {
                tab_id,
                node_id: "node-2".into(),
            },
            &ctx(3),
        );
        assert!(events.is_empty());
        assert_eq!(state.event_version, version_before);
    }

    #[test]
    fn set_magnified_node_clear_with_empty_node_id() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let tab_id = create_tab(&mut state, &ws_id, "t1");
        // Magnify a node first.
        let _ = update(
            &mut state,
            Command::SetMagnifiedNode {
                tab_id: tab_id.clone(),
                node_id: "node-3".into(),
            },
            &ctx(2),
        );
        assert_eq!(state.tabs[&tab_id].magnified_node_id, "node-3");
        // Now clear with empty node_id (toggle-off semantics).
        let events = update(
            &mut state,
            Command::SetMagnifiedNode {
                tab_id: tab_id.clone(),
                node_id: String::new(),
            },
            &ctx(3),
        );
        assert_eq!(events.len(), 1);
        match &events[0] {
            Event::MagnifiedNodeChanged { node_id, .. } => assert_eq!(node_id, ""),
            other => panic!("expected MagnifiedNodeChanged with empty node_id, got {:?}", other),
        }
        assert_eq!(state.tabs[&tab_id].magnified_node_id, "");
    }

    #[test]
    fn set_magnified_node_unknown_tab_emits_error() {
        let mut state = State::default();
        let events = update(
            &mut state,
            Command::SetMagnifiedNode {
                tab_id: "ghost-tab".into(),
                node_id: "node-1".into(),
            },
            &ctx(1),
        );
        assert_eq!(events.len(), 1);
        assert!(matches!(&events[0], Event::Error { .. }));
    }

    #[test]
    fn delete_tab_cascades_blocks() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let tab_id = create_tab(&mut state, &ws_id, "t");
        let _ = update(
            &mut state,
            Command::CreateBlock { tab_id: tab_id.clone(), meta: serde_json::Value::Null },
            &ctx(2),
        );
        let _ = update(
            &mut state,
            Command::CreateBlock { tab_id: tab_id.clone(), meta: serde_json::Value::Null },
            &ctx(3),
        );
        assert_eq!(state.blocks.len(), 2);
        let _ = update(
            &mut state,
            Command::DeleteTab {
                workspace_id: ws_id,
                tab_id,
                // Single-tab workspace; force=true bypasses last-tab
                // guard so we can test the block cascade.
                force: true,
            },
            &ctx(4),
        );
        assert!(state.blocks.is_empty());
    }

    #[test]
    fn delete_workspace_cascades_through_tabs_to_blocks() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let tab_id = create_tab(&mut state, &ws_id, "t");
        let _ = update(
            &mut state,
            Command::CreateBlock { tab_id: tab_id.clone(), meta: serde_json::Value::Null },
            &ctx(2),
        );
        let _ = update(
            &mut state,
            Command::CreateBlock { tab_id, meta: serde_json::Value::Null },
            &ctx(3),
        );
        assert_eq!(state.blocks.len(), 2);
        let _ = update(
            &mut state,
            Command::DeleteWorkspace { workspace_id: ws_id, force: false },
            &ctx(4),
        );
        assert!(state.blocks.is_empty());
        assert!(state.tabs.is_empty());
    }

    #[test]
    fn snapshot_includes_blocks() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let tab_id = create_tab(&mut state, &ws_id, "t");
        let _ = update(
            &mut state,
            Command::CreateBlock { tab_id, meta: serde_json::Value::Null },
            &ctx(2),
        );
        let events = update(&mut state, Command::GetSrvSnapshot, &ctx(3));
        let Event::SrvSnapshot { blocks, .. } = &events[0] else {
            panic!("expected SrvSnapshot");
        };
        assert_eq!(blocks.len(), 1);
    }

    // ---- E.5: window↔workspace mapping arms ----

    #[test]
    fn create_window_validates_workspace_exists() {
        let mut state = State::default();
        let events = update(
            &mut state,
            Command::CreateWindow {
                window_id: "win-1".into(),
                workspace_id: "no-such-ws".into(),
            },
            &ctx(1),
        );
        assert!(matches!(&events[0], Event::Error { .. }));
        assert!(state.windows.is_empty());
    }

    #[test]
    fn create_window_inserts_record_and_emits_event() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let events = update(
            &mut state,
            Command::CreateWindow {
                window_id: "win-1".into(),
                workspace_id: ws_id.clone(),
            },
            &ctx(2),
        );
        assert!(matches!(
            &events[0],
            Event::SrvWindowOpened { window_id, workspace_id, .. }
                if window_id == "win-1" && *workspace_id == ws_id
        ));
        assert_eq!(state.windows["win-1"].workspace_id, ws_id);
    }

    #[test]
    fn create_window_idempotent_on_same_workspace() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let _ = update(
            &mut state,
            Command::CreateWindow {
                window_id: "win-1".into(),
                workspace_id: ws_id.clone(),
            },
            &ctx(2),
        );
        let events = update(
            &mut state,
            Command::CreateWindow {
                window_id: "win-1".into(),
                workspace_id: ws_id,
            },
            &ctx(3),
        );
        assert!(events.is_empty());
    }

    #[test]
    fn close_window_internal_removes_record() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let _ = update(
            &mut state,
            Command::CreateWindow {
                window_id: "win-1".into(),
                workspace_id: ws_id,
            },
            &ctx(2),
        );
        let events = update(
            &mut state,
            Command::CloseWindowInternal {
                window_id: "win-1".into(),
            },
            &ctx(3),
        );
        assert!(matches!(&events[0], Event::SrvWindowClosed { .. }));
        assert!(state.windows.is_empty());
    }

    #[test]
    fn close_window_internal_silent_on_missing() {
        let mut state = State::default();
        let events = update(
            &mut state,
            Command::CloseWindowInternal {
                window_id: "ghost".into(),
            },
            &ctx(1),
        );
        assert!(events.is_empty());
    }

    #[test]
    fn switch_workspace_updates_window_pointer() {
        let mut state = State::default();
        let ws_a = create_workspace(&mut state, "a");
        let ws_b = create_workspace(&mut state, "b");
        let _ = update(
            &mut state,
            Command::CreateWindow {
                window_id: "win-1".into(),
                workspace_id: ws_a.clone(),
            },
            &ctx(2),
        );
        let events = update(
            &mut state,
            Command::SwitchWorkspace {
                window_id: "win-1".into(),
                workspace_id: ws_b.clone(),
            },
            &ctx(3),
        );
        assert!(matches!(
            &events[0],
            Event::SrvWindowWorkspaceChanged { workspace_id, .. } if *workspace_id == ws_b
        ));
        assert_eq!(state.windows["win-1"].workspace_id, ws_b);
    }

    #[test]
    fn switch_workspace_validates_window_and_destination() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "a");
        // Unknown window.
        let events = update(
            &mut state,
            Command::SwitchWorkspace {
                window_id: "ghost".into(),
                workspace_id: ws_id.clone(),
            },
            &ctx(2),
        );
        assert!(matches!(&events[0], Event::Error { .. }));
        // Known window, unknown workspace.
        let _ = update(
            &mut state,
            Command::CreateWindow {
                window_id: "win-1".into(),
                workspace_id: ws_id,
            },
            &ctx(3),
        );
        let events = update(
            &mut state,
            Command::SwitchWorkspace {
                window_id: "win-1".into(),
                workspace_id: "no-such-ws".into(),
            },
            &ctx(4),
        );
        assert!(matches!(&events[0], Event::Error { .. }));
    }

    #[test]
    fn switch_workspace_no_op_when_already_pointing() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "a");
        let _ = update(
            &mut state,
            Command::CreateWindow {
                window_id: "win-1".into(),
                workspace_id: ws_id.clone(),
            },
            &ctx(2),
        );
        let events = update(
            &mut state,
            Command::SwitchWorkspace {
                window_id: "win-1".into(),
                workspace_id: ws_id,
            },
            &ctx(3),
        );
        assert!(events.is_empty());
    }

    #[test]
    fn delete_workspace_drops_pointing_windows_and_emits_events() {
        let mut state = State::default();
        let ws_a = create_workspace(&mut state, "a");
        let ws_b = create_workspace(&mut state, "b");
        let _ = update(
            &mut state,
            Command::CreateWindow {
                window_id: "win-a".into(),
                workspace_id: ws_a.clone(),
            },
            &ctx(2),
        );
        let _ = update(
            &mut state,
            Command::CreateWindow {
                window_id: "win-b".into(),
                workspace_id: ws_b.clone(),
            },
            &ctx(3),
        );
        // Delete workspace A; only win-a should be dropped + emit
        // SrvWindowClosed; win-b survives.
        let events = update(
            &mut state,
            Command::DeleteWorkspace { workspace_id: ws_a, force: false },
            &ctx(4),
        );
        assert!(matches!(&events[0], Event::WorkspaceDeleted { .. }));
        assert!(events.iter().any(|e| matches!(
            e,
            Event::SrvWindowClosed { window_id, .. } if window_id == "win-a"
        )));
        // Verify win-b was NOT closed.
        assert!(!events.iter().any(|e| matches!(
            e,
            Event::SrvWindowClosed { window_id, .. } if window_id == "win-b"
        )));
        assert!(!state.windows.contains_key("win-a"));
        assert_eq!(state.windows["win-b"].workspace_id, ws_b);
    }

    #[test]
    fn snapshot_includes_tabs_and_active_tabs() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let _ = update(
            &mut state,
            Command::CreateTab {
                workspace_id: ws_id.clone(),
                name: "t1".into(),
            },
            &ctx(1),
        );
        let events = update(&mut state, Command::GetSrvSnapshot, &ctx(2));
        let Event::SrvSnapshot {
            tabs, active_tabs, ..
        } = &events[0]
        else {
            panic!("expected SrvSnapshot");
        };
        assert_eq!(tabs.len(), 1);
        assert_eq!(active_tabs.len(), 1);
        assert_eq!(active_tabs[0].0, ws_id);
    }

    #[test]
    fn goodbye_marks_process_exited() {
        let mut state = State::default();
        let _ = update(
            &mut state,
            Command::Register {
                kind: ClientKind::Host,
                pid: 100,
                version: "v".into(),
            },
            &ctx(1),
        );
        let events = update(&mut state, Command::Goodbye, &ctx_with_pid(1, 100));
        assert!(matches!(
            &events[0],
            Event::ProcessExited { pid: 100, code: 0, .. }
        ));
        assert!(matches!(
            state.processes[&100].state,
            ProcessState::Exited { code: 0 }
        ));
    }

    // ---- Phase E.5.5 — MoveTab tests ----

    #[test]
    fn move_tab_cross_workspace_updates_lists_and_parent() {
        let mut state = State::default();
        let src = create_workspace(&mut state, "src");
        let dst = create_workspace(&mut state, "dst");
        let t1 = create_tab(&mut state, &src, "t1");
        let t2 = create_tab(&mut state, &src, "t2");
        let dst_existing = create_tab(&mut state, &dst, "existing");
        let events = update(
            &mut state,
            Command::MoveTab {
                tab_id: t1.clone(),
                src_workspace_id: src.clone(),
                dst_workspace_id: dst.clone(),
                dst_index: 0,
            },
            &ctx(99),
        );
        match &events[0] {
            Event::TabMoved {
                tab_id,
                src_workspace_id,
                dst_workspace_id,
                dst_index,
                new_src_active_tab_id,
                ..
            } => {
                assert_eq!(tab_id, &t1);
                assert_eq!(src_workspace_id, &src);
                assert_eq!(dst_workspace_id, &dst);
                assert_eq!(*dst_index, 0);
                assert_eq!(new_src_active_tab_id, &Some(t2.clone()));
            }
            other => panic!("expected TabMoved, got {:?}", other),
        }
        assert_eq!(state.workspaces[&src].tab_ids, vec![t2.clone()]);
        assert_eq!(state.workspaces[&dst].tab_ids, vec![t1.clone(), dst_existing]);
        assert_eq!(state.tabs[&t1].workspace_id, dst);
        assert_eq!(state.workspaces[&src].active_tab_id, Some(t2));
    }

    #[test]
    fn move_tab_clamps_dst_index_to_dst_length() {
        let mut state = State::default();
        let src = create_workspace(&mut state, "src");
        let dst = create_workspace(&mut state, "dst");
        let t1 = create_tab(&mut state, &src, "t1");
        let _ = create_tab(&mut state, &src, "filler");
        let events = update(
            &mut state,
            Command::MoveTab {
                tab_id: t1.clone(),
                src_workspace_id: src,
                dst_workspace_id: dst.clone(),
                dst_index: 999,
            },
            &ctx(2),
        );
        match &events[0] {
            Event::TabMoved { dst_index, .. } => assert_eq!(*dst_index, 0),
            other => panic!("expected TabMoved, got {:?}", other),
        }
        assert_eq!(state.workspaces[&dst].tab_ids, vec![t1]);
    }

    #[test]
    fn move_tab_src_active_clears_when_workspace_empties() {
        let mut state = State::default();
        let src = create_workspace(&mut state, "src");
        let dst = create_workspace(&mut state, "dst");
        let only_tab = create_tab(&mut state, &src, "only");
        let events = update(
            &mut state,
            Command::MoveTab {
                tab_id: only_tab,
                src_workspace_id: src.clone(),
                dst_workspace_id: dst,
                dst_index: 0,
            },
            &ctx(2),
        );
        match &events[0] {
            Event::TabMoved {
                new_src_active_tab_id,
                ..
            } => assert_eq!(new_src_active_tab_id, &None),
            other => panic!("expected TabMoved, got {:?}", other),
        }
        assert_eq!(state.workspaces[&src].active_tab_id, None);
        assert!(state.workspaces[&src].tab_ids.is_empty());
    }

    #[test]
    fn move_tab_rejects_same_workspace() {
        let mut state = State::default();
        let ws = create_workspace(&mut state, "w");
        let t1 = create_tab(&mut state, &ws, "t1");
        let events = update(
            &mut state,
            Command::MoveTab {
                tab_id: t1,
                src_workspace_id: ws.clone(),
                dst_workspace_id: ws,
                dst_index: 0,
            },
            &ctx(2),
        );
        assert!(matches!(&events[0], Event::Error { .. }));
    }

    #[test]
    fn move_tab_rejects_unknown_src_or_dst_or_tab() {
        let mut state = State::default();
        let src = create_workspace(&mut state, "src");
        let dst = create_workspace(&mut state, "dst");
        let t1 = create_tab(&mut state, &src, "t1");

        let events = update(
            &mut state,
            Command::MoveTab {
                tab_id: t1.clone(),
                src_workspace_id: "no-such-src".into(),
                dst_workspace_id: dst.clone(),
                dst_index: 0,
            },
            &ctx(2),
        );
        assert!(matches!(&events[0], Event::Error { .. }));

        let events = update(
            &mut state,
            Command::MoveTab {
                tab_id: t1.clone(),
                src_workspace_id: src.clone(),
                dst_workspace_id: "no-such-dst".into(),
                dst_index: 0,
            },
            &ctx(3),
        );
        assert!(matches!(&events[0], Event::Error { .. }));

        // Phase E.4 strict-mode flip: unknown tabs are now REJECTED.
        // The migration-tolerant lazy-import fallback was removed once
        // the soak window closed without `lazy-import` warnings being
        // observed in production. See `move_tab_unknown_tab_rejects`
        // for the dedicated test.
        let events = update(
            &mut state,
            Command::MoveTab {
                tab_id: "ghost-tab".into(),
                src_workspace_id: src,
                dst_workspace_id: dst,
                dst_index: 0,
            },
            &ctx(4),
        );
        assert!(matches!(&events[0], Event::Error { .. }));
    }

    /// Phase E.4 strict-mode flip: an unknown tab id (not present in
    /// `state.tabs`) is rejected with a clear "tab not found" error
    /// rather than being lazy-imported. Replaces the migration-window
    /// `move_tab_lazy_imports_unknown_tab` test.
    #[test]
    fn move_tab_unknown_tab_rejects() {
        let mut state = State::default();
        let src = create_workspace(&mut state, "src");
        let dst = create_workspace(&mut state, "dst");
        let unknown_id = "unknown-tab-xyz".to_string();
        assert!(!state.tabs.contains_key(&unknown_id));
        let events = update(
            &mut state,
            Command::MoveTab {
                tab_id: unknown_id.clone(),
                src_workspace_id: src,
                dst_workspace_id: dst,
                dst_index: 0,
            },
            &ctx(2),
        );
        match &events[0] {
            Event::Error { code, message, .. } => {
                assert_eq!(*code, ErrorCode::InvalidCommand);
                assert!(
                    message.contains("tab not found"),
                    "error should mention `tab not found`, got: {}",
                    message
                );
            }
            other => panic!("expected Error event, got {:?}", other),
        }
        // No lazy import side-effect.
        assert!(!state.tabs.contains_key(&unknown_id));
    }

    /// Phase E.4 strict-mode flip: a known tab whose reducer-state
    /// `workspace_id` doesn't match `src_workspace_id` is rejected.
    /// Replaces the migration-window
    /// `move_tab_tolerates_workspace_id_mismatch_during_migration`
    /// test.
    #[test]
    fn move_tab_wrong_workspace_rejects() {
        let mut state = State::default();
        let real_src = create_workspace(&mut state, "real_src");
        let dst = create_workspace(&mut state, "dst");
        let other = create_workspace(&mut state, "other");
        let t1 = create_tab(&mut state, &real_src, "t1");
        let filler = create_tab(&mut state, &other, "filler");
        // Claim the tab lives in `other` even though it actually
        // belongs to `real_src` per reducer state.
        let events = update(
            &mut state,
            Command::MoveTab {
                tab_id: t1.clone(),
                src_workspace_id: other.clone(),
                dst_workspace_id: dst.clone(),
                dst_index: 0,
            },
            &ctx(2),
        );
        match &events[0] {
            Event::Error { code, message, .. } => {
                assert_eq!(*code, ErrorCode::InvalidCommand);
                assert!(
                    message.contains("workspace_id mismatch"),
                    "error should mention `workspace_id mismatch`, got: {}",
                    message
                );
            }
            other => panic!("expected Error event, got {:?}", other),
        }
        // Reducer state untouched: t1 still in real_src, filler still
        // in other, dst empty.
        assert_eq!(state.tabs[&t1].workspace_id, real_src);
        assert_eq!(state.workspaces[&real_src].tab_ids, vec![t1]);
        assert_eq!(state.workspaces[&other].tab_ids, vec![filler]);
        assert!(state.workspaces[&dst].tab_ids.is_empty());
    }

    // ---- Phase E.5.5 — MoveBlock tests ----

    fn create_block(state: &mut State, tab_id: &str) -> String {
        let events = update(
            state,
            Command::CreateBlock {
                tab_id: tab_id.into(),
                meta: serde_json::Value::Null,
            },
            &ctx(1),
        );
        match &events[0] {
            Event::BlockCreated { block_id, .. } => block_id.clone(),
            _ => panic!("expected BlockCreated"),
        }
    }

    #[test]
    fn move_block_cross_tab_updates_lists_and_parent() {
        let mut state = State::default();
        let ws = create_workspace(&mut state, "w");
        let src_tab = create_tab(&mut state, &ws, "src");
        let dst_tab = create_tab(&mut state, &ws, "dst");
        let block = create_block(&mut state, &src_tab);
        let dst_existing = create_block(&mut state, &dst_tab);
        let events = update(
            &mut state,
            Command::MoveBlock {
                block_id: block.clone(),
                src_tab_id: src_tab.clone(),
                dst_tab_id: dst_tab.clone(),
                dst_index: 0,
            },
            &ctx(2),
        );
        assert!(matches!(&events[0], Event::BlockMoved { .. }));
        assert_eq!(state.tabs[&src_tab].block_ids, Vec::<String>::new());
        assert_eq!(state.tabs[&dst_tab].block_ids, vec![block.clone(), dst_existing]);
        assert_eq!(state.blocks[&block].tab_id, dst_tab);
    }

    #[test]
    fn move_block_intra_tab_repositions() {
        let mut state = State::default();
        let ws = create_workspace(&mut state, "w");
        let tab = create_tab(&mut state, &ws, "t");
        let b1 = create_block(&mut state, &tab);
        let b2 = create_block(&mut state, &tab);
        let b3 = create_block(&mut state, &tab);
        // Move b1 to position 2 (end after removal).
        let events = update(
            &mut state,
            Command::MoveBlock {
                block_id: b1.clone(),
                src_tab_id: tab.clone(),
                dst_tab_id: tab.clone(),
                dst_index: 2,
            },
            &ctx(2),
        );
        match &events[0] {
            Event::BlockMoved { dst_index, .. } => assert_eq!(*dst_index, 2),
            other => panic!("expected BlockMoved, got {:?}", other),
        }
        assert_eq!(state.tabs[&tab].block_ids, vec![b2, b3, b1]);
    }

    #[test]
    fn move_block_rejects_unknown_src_or_dst_or_block() {
        let mut state = State::default();
        let ws = create_workspace(&mut state, "w");
        let tab = create_tab(&mut state, &ws, "t");
        let other_tab = create_tab(&mut state, &ws, "other");
        let block = create_block(&mut state, &tab);

        let events = update(
            &mut state,
            Command::MoveBlock {
                block_id: block.clone(),
                src_tab_id: "ghost-src".into(),
                dst_tab_id: other_tab.clone(),
                dst_index: 0,
            },
            &ctx(2),
        );
        assert!(matches!(&events[0], Event::Error { .. }));

        let events = update(
            &mut state,
            Command::MoveBlock {
                block_id: block.clone(),
                src_tab_id: tab.clone(),
                dst_tab_id: "ghost-dst".into(),
                dst_index: 0,
            },
            &ctx(3),
        );
        assert!(matches!(&events[0], Event::Error { .. }));

        let events = update(
            &mut state,
            Command::MoveBlock {
                block_id: "ghost-block".into(),
                src_tab_id: tab,
                dst_tab_id: other_tab,
                dst_index: 0,
            },
            &ctx(4),
        );
        assert!(matches!(&events[0], Event::Error { .. }));
    }

    #[test]
    fn move_block_rejects_when_block_belongs_to_different_tab() {
        let mut state = State::default();
        let ws = create_workspace(&mut state, "w");
        let real_src = create_tab(&mut state, &ws, "real");
        let other = create_tab(&mut state, &ws, "other");
        let dst = create_tab(&mut state, &ws, "dst");
        let block = create_block(&mut state, &real_src);
        let events = update(
            &mut state,
            Command::MoveBlock {
                block_id: block,
                src_tab_id: other,
                dst_tab_id: dst,
                dst_index: 0,
            },
            &ctx(2),
        );
        match &events[0] {
            Event::Error { message, .. } => {
                assert!(message.contains("belongs to tab"), "got: {}", message);
            }
            other => panic!("expected Error, got {:?}", other),
        }
    }

    // ---- Phase E.7 — property tests for reducer arm invariants ----
    //
    // Drives randomized sequences of valid commands through `update`
    // and asserts cross-arm invariants the unit tests above only
    // touch on per-arm. Catches regressions where an individual arm
    // looks correct in isolation but interacts with sibling arms in
    // a way that violates the reducer's whole-state contract.
    //
    // Invariants asserted:
    //   1. Version monotonicity: every event's version strictly
    //      increases across the sequence (no duplicates, no gaps in
    //      the wrong direction).
    //   2. Referential integrity: every tab in `state.tabs` has a
    //      `workspace_id` that exists in `state.workspaces`; every
    //      block in `state.blocks` has a `tab_id` that exists in
    //      `state.tabs`; every workspace's `tab_ids` references real
    //      tabs; every tab's `block_ids` references real blocks.
    //   3. Cascade integrity: after a `DeleteWorkspace`, no tab
    //      remains in `state.tabs` with that workspace_id, and no
    //      block remains in `state.blocks` whose tab was in that
    //      workspace.
    //   4. Active-tab validity: every workspace's `active_tab_id`
    //      is either `None` or points at a tab present in its own
    //      `tab_ids`.

    use proptest::prelude::*;

    /// Higher-level operations the property tests pick from. Each
    /// resolves to one or more `Command` invocations against the
    /// current state. We can't generate `Command`s directly because
    /// IDs are reducer-generated; pick from existing IDs instead.
    #[derive(Debug, Clone)]
    enum PropOp {
        CreateWorkspace,
        CreateTab,
        CreateBlock,
        DeleteTab,
        DeleteBlock,
        DeleteWorkspace,
    }

    fn op_strategy() -> impl Strategy<Value = PropOp> {
        // Bias toward "constructive" ops so sequences accumulate
        // state rather than churn empty. Each Just is one variant;
        // proptest weights via `prop_oneof![weight => strat, …]`.
        prop_oneof![
            4 => Just(PropOp::CreateWorkspace),
            4 => Just(PropOp::CreateTab),
            3 => Just(PropOp::CreateBlock),
            1 => Just(PropOp::DeleteTab),
            1 => Just(PropOp::DeleteBlock),
            1 => Just(PropOp::DeleteWorkspace),
        ]
    }

    /// Apply one PropOp; returns the events produced (which may be
    /// empty if the op was a no-op like "delete from empty pool").
    fn apply_prop_op(state: &mut State, op: PropOp, conn_id: u64) -> Vec<Event> {
        match op {
            PropOp::CreateWorkspace => update(
                state,
                Command::CreateWorkspace { name: format!("ws-{}", conn_id) },
                &ctx(conn_id),
            ),
            PropOp::CreateTab => {
                let target_ws = state.workspaces.keys().next().cloned();
                match target_ws {
                    Some(workspace_id) => update(
                        state,
                        Command::CreateTab {
                            workspace_id,
                            name: format!("tab-{}", conn_id),
                        },
                        &ctx(conn_id),
                    ),
                    None => Vec::new(),
                }
            }
            PropOp::CreateBlock => {
                let target_tab = state.tabs.keys().next().cloned();
                match target_tab {
                    Some(tab_id) => update(
                        state,
                        Command::CreateBlock { tab_id, meta: serde_json::Value::Null },
                        &ctx(conn_id),
                    ),
                    None => Vec::new(),
                }
            }
            PropOp::DeleteTab => {
                if let Some((tab_id, tab)) = state.tabs.iter().next() {
                    let cmd = Command::DeleteTab {
                        workspace_id: tab.workspace_id.clone(),
                        tab_id: tab_id.clone(),
                        // Proptest exercises both guarded + unguarded
                        // paths; force=true here ensures cascade
                        // invariants are tested without the guard
                        // short-circuiting the operation.
                        force: true,
                    };
                    update(state, cmd, &ctx(conn_id))
                } else {
                    Vec::new()
                }
            }
            PropOp::DeleteBlock => {
                if let Some((block_id, block)) = state.blocks.iter().next() {
                    let cmd = Command::DeleteBlock {
                        tab_id: block.tab_id.clone(),
                        block_id: block_id.clone(),
                    };
                    update(state, cmd, &ctx(conn_id))
                } else {
                    Vec::new()
                }
            }
            PropOp::DeleteWorkspace => {
                if let Some(workspace_id) = state.workspaces.keys().next().cloned() {
                    update(
                        state,
                        Command::DeleteWorkspace { workspace_id, force: false },
                        &ctx(conn_id),
                    )
                } else {
                    Vec::new()
                }
            }
        }
    }

    /// Verify all four reducer-state invariants at once. Panics
    /// (proptest catches and shrinks) if any is violated.
    fn assert_invariants(state: &State) {
        // (2) Tabs reference real workspaces.
        for (tab_id, tab) in &state.tabs {
            assert!(
                state.workspaces.contains_key(&tab.workspace_id),
                "tab {} references unknown workspace {}",
                tab_id,
                tab.workspace_id
            );
        }
        // (2) Blocks reference real tabs.
        for (block_id, block) in &state.blocks {
            assert!(
                state.tabs.contains_key(&block.tab_id),
                "block {} references unknown tab {}",
                block_id,
                block.tab_id
            );
        }
        // (2) Workspace.tab_ids references real tabs.
        for (workspace_id, ws) in &state.workspaces {
            for tab_id in &ws.tab_ids {
                assert!(
                    state.tabs.contains_key(tab_id),
                    "workspace {} tab_ids contains unknown tab {}",
                    workspace_id,
                    tab_id
                );
            }
        }
        // (2) Tab.block_ids references real blocks.
        for (tab_id, tab) in &state.tabs {
            for block_id in &tab.block_ids {
                assert!(
                    state.blocks.contains_key(block_id),
                    "tab {} block_ids contains unknown block {}",
                    tab_id,
                    block_id
                );
            }
        }
        // (4) Active-tab validity.
        for (workspace_id, ws) in &state.workspaces {
            if let Some(active) = &ws.active_tab_id {
                assert!(
                    ws.tab_ids.iter().any(|t| t == active),
                    "workspace {} active_tab_id {} not in its tab_ids",
                    workspace_id,
                    active
                );
            }
        }
    }

    proptest! {
        #![proptest_config(ProptestConfig {
            cases: 64,
            ..ProptestConfig::default()
        })]

        /// Apply a random sequence of valid ops. After each op,
        /// referential integrity + active-tab validity hold; across
        /// the whole sequence, version is strictly monotonic for any
        /// emitted events.
        #[test]
        fn invariants_hold_across_random_sequences(ops in prop::collection::vec(op_strategy(), 0..40)) {
            let mut state = State::default();
            let mut last_version: u64 = 0;
            for (i, op) in ops.into_iter().enumerate() {
                let events = apply_prop_op(&mut state, op, (i + 1) as u64);
                for ev in &events {
                    let v = extract_version(ev);
                    prop_assert!(
                        v > last_version,
                        "version {} not strictly greater than previous {} (event {:?})",
                        v,
                        last_version,
                        ev
                    );
                    last_version = v;
                }
                assert_invariants(&state);
            }
        }

        /// Cascade integrity — explicit setup-then-delete pattern.
        /// Build a non-trivial graph (workspace + tabs + blocks),
        /// delete the workspace, assert NO surviving entities
        /// reference the deleted workspace.
        #[test]
        fn delete_workspace_cascades_cleanly(
            tab_count in 1usize..6,
            blocks_per_tab in 0usize..4,
        ) {
            let mut state = State::default();
            // Create workspace.
            let ws_events = update(
                &mut state,
                Command::CreateWorkspace { name: "ws".into() },
                &ctx(1),
            );
            let ws_id = ws_events
                .iter()
                .find_map(|e| match e {
                    Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
                    _ => None,
                })
                .unwrap();
            // Create tabs + blocks under it. We don't keep the tab
            // IDs around — the cascade-after-delete assertions below
            // check the WHOLE-state collections (state.tabs.is_empty()
            // etc.), so per-tab IDs aren't needed. (reagent P2 #627.)
            for _ in 0..tab_count {
                let evs = update(
                    &mut state,
                    Command::CreateTab {
                        workspace_id: ws_id.clone(),
                        name: "t".into(),
                    },
                    &ctx(2),
                );
                let tid = evs
                    .iter()
                    .find_map(|e| match e {
                        Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
                        _ => None,
                    })
                    .unwrap();
                for _ in 0..blocks_per_tab {
                    let _ = update(
                        &mut state,
                        Command::CreateBlock {
                            tab_id: tid.clone(),
                            meta: serde_json::Value::Null,
                        },
                        &ctx(3),
                    );
                }
            }
            // Sanity: counts match.
            prop_assert_eq!(state.workspaces.len(), 1);
            prop_assert_eq!(state.tabs.len(), tab_count);
            prop_assert_eq!(state.blocks.len(), tab_count * blocks_per_tab);
            // Delete the workspace.
            let _ = update(
                &mut state,
                Command::DeleteWorkspace { workspace_id: ws_id.clone(), force: false },
                &ctx(4),
            );
            // Cascade — workspaces, tabs, blocks should all be empty.
            prop_assert!(state.workspaces.is_empty());
            prop_assert!(state.tabs.is_empty());
            prop_assert!(
                state.blocks.is_empty(),
                "blocks should cascade-delete with their tabs; got {} survivors",
                state.blocks.len()
            );
            // And invariants hold on the empty state.
            assert_invariants(&state);
        }
    }

    // ── Phase E.4.B Phase 5 — layout reducer arms ─────────────────
    //
    // Tests for the 4 arms shipped in this PR. All arms share the
    // same shape (lookup tab → mutate `tab.rootnode` via pure helper
    // → emit Event::Layout*); the unit tests below verify state
    // mutation and event shape per arm. The pure helpers themselves
    // have their own ~40 tests in `agentmux-srv/src/backend/layout/`.

    fn leaf_node(id: &str, block_id: &str) -> agentmux_common::LayoutNode {
        agentmux_common::LayoutNode {
            id: id.to_string(),
            size: 1.0,
            data: Some(agentmux_common::LayoutNodeData {
                block_id: block_id.to_string(),
                ..Default::default()
            }),
            ..Default::default()
        }
    }

    fn fresh_tab() -> (State, String) {
        let mut state = State::default();
        let ws = create_workspace(&mut state, "w");
        let tab = create_tab(&mut state, &ws, "t");
        (state, tab)
    }

    #[test]
    fn layout_clear_wipes_rootnode_focus_magnify_and_emits_event() {
        let (mut state, tab_id) = fresh_tab();
        // Pre-load some state.
        state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(leaf_node("n1", "b1"));
        state.tabs.get_mut(&tab_id).unwrap().focused_node_id = "n1".into();
        state.tabs.get_mut(&tab_id).unwrap().magnified_node_id = "n1".into();

        let events = update(
            &mut state,
            Command::LayoutClear {
                tab_id: tab_id.clone(),
                correlation_id: "corr-1".into(),
            },
            &ctx(1),
        );

        assert_eq!(events.len(), 1);
        assert!(matches!(
            &events[0],
            Event::LayoutCleared { correlation_id, .. } if correlation_id == "corr-1"
        ));
        let tab = &state.tabs[&tab_id];
        assert!(tab.rootnode.is_none(), "rootnode wiped");
        assert_eq!(tab.focused_node_id, "");
        assert_eq!(tab.magnified_node_id, "");
    }

    #[test]
    fn layout_clear_unknown_tab_emits_error() {
        let mut state = State::default();
        let events = update(
            &mut state,
            Command::LayoutClear {
                tab_id: "nope".into(),
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        assert!(matches!(&events[0], Event::Error { code: ErrorCode::InvalidCommand, .. }));
    }

    #[test]
    fn layout_set_tree_replaces_rootnode_wholesale() {
        let (mut state, tab_id) = fresh_tab();
        state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(leaf_node("old", "b1"));

        let new_tree = Some(leaf_node("new", "b2"));
        let events = update(
            &mut state,
            Command::LayoutSetTree {
                tab_id: tab_id.clone(),
                new_tree: new_tree.clone(),
                correlation_id: "corr-set".into(),
            },
            &ctx(1),
        );

        assert!(matches!(&events[0], Event::LayoutTreeReplaced { .. }));
        assert_eq!(state.tabs[&tab_id].rootnode, new_tree);
    }

    #[test]
    fn layout_set_tree_to_none_clears_rootnode() {
        let (mut state, tab_id) = fresh_tab();
        state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(leaf_node("n", "b"));

        let _ = update(
            &mut state,
            Command::LayoutSetTree {
                tab_id: tab_id.clone(),
                new_tree: None,
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        assert!(state.tabs[&tab_id].rootnode.is_none());
    }

    #[test]
    fn layout_insert_node_into_empty_tree_promotes_to_root() {
        let (mut state, tab_id) = fresh_tab();
        let node = leaf_node("first", "b1");
        let events = update(
            &mut state,
            Command::LayoutInsertNode {
                tab_id: tab_id.clone(),
                node: node.clone(),
                parent_id: None,
                index: None,
                focus_after: false,
                magnify_after: false,
                correlation_id: "corr-ins".into(),
            },
            &ctx(1),
        );
        assert!(matches!(&events[0], Event::LayoutNodeInserted { .. }));
        assert_eq!(state.tabs[&tab_id].rootnode.as_ref().map(|n| n.id.as_str()), Some("first"));
    }

    #[test]
    fn layout_insert_node_into_existing_tree_uses_helper() {
        let (mut state, tab_id) = fresh_tab();
        // Pre-load a single-leaf tree.
        state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(leaf_node("root", "b1"));
        let new_node = leaf_node("added", "b2");
        let events = update(
            &mut state,
            Command::LayoutInsertNode {
                tab_id: tab_id.clone(),
                node: new_node,
                parent_id: None,
                index: None,
                focus_after: false,
                magnify_after: false,
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        assert!(matches!(&events[0], Event::LayoutNodeInserted { .. }));
        // Helper turned the leaf root into a group with both leaves;
        // exact shape is the helper's contract — we just assert the
        // tree changed and contains both block ids.
        let root = state.tabs[&tab_id].rootnode.as_ref().expect("rootnode set");
        let collected = collect_block_ids(root);
        assert!(collected.contains(&"b1".to_string()));
        assert!(collected.contains(&"b2".to_string()));
    }

    fn collect_block_ids(node: &agentmux_common::LayoutNode) -> Vec<String> {
        let mut ids = Vec::new();
        if let Some(d) = &node.data {
            if !d.block_id.is_empty() {
                ids.push(d.block_id.clone());
            }
        }
        for c in &node.children {
            ids.extend(collect_block_ids(c));
        }
        ids
    }

    #[test]
    fn layout_delete_node_on_empty_tree_is_noop() {
        let (mut state, tab_id) = fresh_tab();
        let events = update(
            &mut state,
            Command::LayoutDeleteNode {
                tab_id: tab_id.clone(),
                node_id: "ghost".into(),
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        assert!(events.is_empty(), "no event for delete on empty tree");
        assert!(state.tabs[&tab_id].rootnode.is_none());
    }

    #[test]
    fn layout_set_tree_to_none_also_clears_focused_and_magnified() {
        // empty-tree set must match
        // `LayoutClear`'s contract — focused/magnified ids would
        // otherwise dangle past the wipe.
        let (mut state, tab_id) = fresh_tab();
        state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(leaf_node("n", "b"));
        state.tabs.get_mut(&tab_id).unwrap().focused_node_id = "n".into();
        state.tabs.get_mut(&tab_id).unwrap().magnified_node_id = "n".into();
        let _ = update(
            &mut state,
            Command::LayoutSetTree {
                tab_id: tab_id.clone(),
                new_tree: None,
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        let tab = &state.tabs[&tab_id];
        assert!(tab.rootnode.is_none());
        assert_eq!(tab.focused_node_id, "");
        assert_eq!(tab.magnified_node_id, "");
    }

    #[test]
    fn layout_set_tree_with_some_preserves_focused_and_magnified() {
        // Symmetry guard: Some(new_tree) must NOT clear focused/
        // magnified — caller may have set them deliberately.
        let (mut state, tab_id) = fresh_tab();
        state.tabs.get_mut(&tab_id).unwrap().focused_node_id = "n".into();
        state.tabs.get_mut(&tab_id).unwrap().magnified_node_id = "n".into();
        let _ = update(
            &mut state,
            Command::LayoutSetTree {
                tab_id: tab_id.clone(),
                new_tree: Some(leaf_node("n", "b")),
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        let tab = &state.tabs[&tab_id];
        assert_eq!(tab.focused_node_id, "n");
        assert_eq!(tab.magnified_node_id, "n");
    }

    #[test]
    fn layout_insert_node_honours_focus_after() {
        // focus_after=true must update
        // focused_node_id so the snapshot matches the event.
        let (mut state, tab_id) = fresh_tab();
        let node = leaf_node("new", "b1");
        let _ = update(
            &mut state,
            Command::LayoutInsertNode {
                tab_id: tab_id.clone(),
                node,
                parent_id: None,
                index: None,
                focus_after: true,
                magnify_after: false,
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        assert_eq!(state.tabs[&tab_id].focused_node_id, "new");
        assert_eq!(state.tabs[&tab_id].magnified_node_id, "");
    }

    #[test]
    fn layout_insert_node_magnify_after_implies_focus() {
        // magnify-implies-focus. Even when
        // focus_after=false, setting magnify_after=true must also
        // update focused_node_id so it doesn't dangle on the prior
        // pane (UI invariant: a magnified pane is the focused pane).
        let (mut state, tab_id) = fresh_tab();
        state.tabs.get_mut(&tab_id).unwrap().focused_node_id = "prev".into();
        let node = leaf_node("new", "b1");
        let _ = update(
            &mut state,
            Command::LayoutInsertNode {
                tab_id: tab_id.clone(),
                node,
                parent_id: None,
                index: None,
                focus_after: false,
                magnify_after: true,
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        assert_eq!(
            state.tabs[&tab_id].focused_node_id, "new",
            "magnify_after must imply focus_after"
        );
        assert_eq!(state.tabs[&tab_id].magnified_node_id, "new");
    }

    #[test]
    fn layout_insert_node_honours_explicit_parent_id_and_index() {
        // with parent_id given, insert at
        // that node at the requested index instead of running the
        // heuristic helper.
        let (mut state, tab_id) = fresh_tab();
        let mut group = leaf_node("group", "");
        group.data = None;
        group.children = vec![leaf_node("a", "ba"), leaf_node("c", "bc")];
        state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(group);

        let _ = update(
            &mut state,
            Command::LayoutInsertNode {
                tab_id: tab_id.clone(),
                node: leaf_node("b", "bb"),
                parent_id: Some("group".into()),
                index: Some(1), // between a and c
                focus_after: false,
                magnify_after: false,
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );

        let root = state.tabs[&tab_id].rootnode.as_ref().expect("root");
        let ids: Vec<_> = root.children.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b", "c"], "explicit index honoured");
    }

    #[test]
    fn layout_insert_node_index_clamps_when_out_of_range() {
        // Out-of-range index clamps to the end (matches frontend
        // `findNextInsertLocation` defensive semantics).
        let (mut state, tab_id) = fresh_tab();
        let mut group = leaf_node("group", "");
        group.data = None;
        group.children = vec![leaf_node("a", "ba")];
        state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(group);

        let _ = update(
            &mut state,
            Command::LayoutInsertNode {
                tab_id: tab_id.clone(),
                node: leaf_node("b", "bb"),
                parent_id: Some("group".into()),
                index: Some(99),
                focus_after: false,
                magnify_after: false,
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        let root = state.tabs[&tab_id].rootnode.as_ref().expect("root");
        let ids: Vec<_> = root.children.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, vec!["a", "b"], "out-of-range index clamps to end");
    }

    #[test]
    fn layout_insert_node_into_empty_tree_with_explicit_parent_id_emits_error() {
        // empty-tree
        // promotion must reject explicit parent_id — otherwise the
        // event echoes a target that subscribers can't resolve.
        let (mut state, tab_id) = fresh_tab();
        let events = update(
            &mut state,
            Command::LayoutInsertNode {
                tab_id: tab_id.clone(),
                node: leaf_node("first", "b1"),
                parent_id: Some("does-not-exist".into()),
                index: None,
                focus_after: false,
                magnify_after: false,
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        assert!(matches!(
            &events[0],
            Event::Error { code: ErrorCode::InvalidCommand, .. }
        ));
        assert!(
            state.tabs[&tab_id].rootnode.is_none(),
            "tree must stay empty on rejection"
        );
    }

    #[test]
    fn layout_insert_node_into_empty_tree_with_explicit_index_emits_error() {
        // Same rationale but with `index` only — the spec §7.1
        // requires both fields be `None` for empty-tree promote.
        let (mut state, tab_id) = fresh_tab();
        let events = update(
            &mut state,
            Command::LayoutInsertNode {
                tab_id: tab_id.clone(),
                node: leaf_node("first", "b1"),
                parent_id: None,
                index: Some(0),
                focus_after: false,
                magnify_after: false,
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        assert!(matches!(
            &events[0],
            Event::Error { code: ErrorCode::InvalidCommand, .. }
        ));
        assert!(state.tabs[&tab_id].rootnode.is_none());
    }

    #[test]
    fn layout_insert_node_with_unknown_parent_id_emits_error() {
        // silent fallback to heuristic
        // diverges the event from the actual mutation. Reject
        // explicit-but-invalid parent_id with Event::Error so
        // subscribers (especially the persist subscriber, future)
        // see a consistent record.
        let (mut state, tab_id) = fresh_tab();
        state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(leaf_node("only", "b1"));

        let events = update(
            &mut state,
            Command::LayoutInsertNode {
                tab_id: tab_id.clone(),
                node: leaf_node("added", "b2"),
                parent_id: Some("does-not-exist".into()),
                index: None,
                focus_after: false,
                magnify_after: false,
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        assert!(matches!(
            &events[0],
            Event::Error { code: ErrorCode::InvalidCommand, .. }
        ));
        // Tree must be unchanged.
        let root = state.tabs[&tab_id].rootnode.as_ref().expect("root");
        assert_eq!(root.id, "only");
        assert!(root.children.is_empty());
    }

    #[test]
    fn layout_insert_node_with_leaf_parent_id_emits_error() {
        // parent_id resolves to a leaf (has data) — leaf can't host
        // children, so treat as invalid the same as a missing parent.
        let (mut state, tab_id) = fresh_tab();
        let mut root = leaf_node("group", "");
        root.data = None;
        root.children = vec![leaf_node("a", "ba")];
        state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(root);

        let events = update(
            &mut state,
            Command::LayoutInsertNode {
                tab_id: tab_id.clone(),
                node: leaf_node("b", "bb"),
                parent_id: Some("a".into()), // leaf, not a group
                index: None,
                focus_after: false,
                magnify_after: false,
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        assert!(matches!(
            &events[0],
            Event::Error { code: ErrorCode::InvalidCommand, .. }
        ));
    }

    #[test]
    fn layout_insert_node_with_neither_flag_leaves_state_alone() {
        // Anti-vacuity guard: confirm the false-flag path is the
        // baseline (otherwise the focus_after/magnify_after tests
        // wouldn't be measuring anything).
        let (mut state, tab_id) = fresh_tab();
        state.tabs.get_mut(&tab_id).unwrap().focused_node_id = "prev".into();
        let _ = update(
            &mut state,
            Command::LayoutInsertNode {
                tab_id: tab_id.clone(),
                node: leaf_node("new", "b1"),
                parent_id: None,
                index: None,
                focus_after: false,
                magnify_after: false,
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        assert_eq!(state.tabs[&tab_id].focused_node_id, "prev");
    }

    #[test]
    fn layout_delete_node_on_root_clears_the_tree() {
        // backend::layout::delete_node leaves
        // root deletion to the caller. Without the root-detection
        // branch, we'd emit LayoutNodeDeleted while rootnode still
        // contains the supposedly-deleted tree.
        let (mut state, tab_id) = fresh_tab();
        state.tabs.get_mut(&tab_id).unwrap().rootnode =
            Some(leaf_node("solitary-root", "b1"));
        let events = update(
            &mut state,
            Command::LayoutDeleteNode {
                tab_id: tab_id.clone(),
                node_id: "solitary-root".into(),
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        assert!(matches!(&events[0], Event::LayoutNodeDeleted { .. }));
        assert!(
            state.tabs[&tab_id].rootnode.is_none(),
            "root deletion must wipe the tree"
        );
    }

    #[test]
    fn layout_delete_node_clears_magnified_when_target_was_magnified() {
        // magnified must be cleared
        // alongside focused; same staleness concern.
        let (mut state, tab_id) = fresh_tab();
        let mut root = leaf_node("group", "");
        root.data = None;
        root.children = vec![leaf_node("a", "b1"), leaf_node("b", "b2")];
        state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(root);
        state.tabs.get_mut(&tab_id).unwrap().magnified_node_id = "a".into();
        let _ = update(
            &mut state,
            Command::LayoutDeleteNode {
                tab_id: tab_id.clone(),
                node_id: "a".into(),
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        assert_eq!(state.tabs[&tab_id].magnified_node_id, "");
    }

    #[test]
    fn layout_node_deleted_event_carries_was_magnified() {
        // subscribers need
        // the was_magnified field to refresh their UI when the
        // magnified node is deleted.
        let (mut state, tab_id) = fresh_tab();
        let mut root = leaf_node("group", "");
        root.data = None;
        root.children = vec![leaf_node("a", "b1"), leaf_node("b", "b2")];
        state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(root);
        state.tabs.get_mut(&tab_id).unwrap().magnified_node_id = "a".into();

        let events = update(
            &mut state,
            Command::LayoutDeleteNode {
                tab_id: tab_id.clone(),
                node_id: "a".into(),
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        match &events[0] {
            Event::LayoutNodeDeleted { was_magnified, was_focused, .. } => {
                assert!(*was_magnified, "was_magnified must be true");
                assert!(!*was_focused, "was_focused stays false");
            }
            other => panic!("expected LayoutNodeDeleted, got {:?}", other),
        }
    }

    #[test]
    fn layout_delete_node_clears_focused_when_collapse_replaces_parent_id() {
        // `backend::layout::delete_node`'s collapse-sole-child path
        // promotes the surviving child and rewrites the parent's id
        // to the child's id. If focused/magnified pointed at the
        // ORIGINAL parent id, that id is gone from the tree even
        // though the same physical layout slot exists. Reducer must
        // clear the dangling reference.
        let (mut state, tab_id) = fresh_tab();
        let mut group = leaf_node("group-id", "");
        group.data = None;
        group.children = vec![leaf_node("only-child", "b1"), leaf_node("sibling", "b2")];
        state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(group);
        // Focus the group (the parent that will get its id rewritten
        // when "sibling" is deleted and "only-child" is the sole
        // survivor of the now-1-child group).
        state.tabs.get_mut(&tab_id).unwrap().focused_node_id = "group-id".into();

        let events = update(
            &mut state,
            Command::LayoutDeleteNode {
                tab_id: tab_id.clone(),
                node_id: "sibling".into(),
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        match &events[0] {
            Event::LayoutNodeDeleted { was_focused, .. } => {
                assert!(
                    *was_focused,
                    "collapse rewrote parent id; reducer must report focus loss"
                );
            }
            other => panic!("expected LayoutNodeDeleted, got {:?}", other),
        }
        assert_eq!(
            state.tabs[&tab_id].focused_node_id, "",
            "stale focus cleared post-collapse"
        );
    }

    #[test]
    fn layout_delete_node_clears_focused_when_target_subtree_contains_focus() {
        // deleting a container
        // wipes its descendants, but a direct-id-match check on
        // focused/magnified misses descendants — they stay dangling.
        let (mut state, tab_id) = fresh_tab();
        // Tree:
        //   root-group (children: group-A, leaf-z)
        //     group-A (children: leaf-x, leaf-y)
        let mut leaf_x = leaf_node("leaf-x", "bx");
        leaf_x.data = Some(agentmux_common::LayoutNodeData {
            block_id: "bx".into(),
            ..Default::default()
        });
        let mut group_a = leaf_node("group-A", "");
        group_a.data = None;
        group_a.children = vec![leaf_x, leaf_node("leaf-y", "by")];
        let mut root = leaf_node("root-group", "");
        root.data = None;
        root.children = vec![group_a, leaf_node("leaf-z", "bz")];
        state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(root);
        // Focus a descendant of group-A, then delete group-A.
        state.tabs.get_mut(&tab_id).unwrap().focused_node_id = "leaf-x".into();
        state.tabs.get_mut(&tab_id).unwrap().magnified_node_id = "leaf-y".into();

        let events = update(
            &mut state,
            Command::LayoutDeleteNode {
                tab_id: tab_id.clone(),
                node_id: "group-A".into(),
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        match &events[0] {
            Event::LayoutNodeDeleted { was_focused, was_magnified, .. } => {
                assert!(*was_focused, "descendant focus must be cleared");
                assert!(*was_magnified, "descendant magnify must be cleared");
            }
            other => panic!("expected LayoutNodeDeleted, got {:?}", other),
        }
        assert_eq!(state.tabs[&tab_id].focused_node_id, "");
        assert_eq!(state.tabs[&tab_id].magnified_node_id, "");
    }

    #[test]
    fn layout_insert_node_event_echoes_parent_id_and_index() {
        // The emitted event must echo the command's parent_id /
        // index so subscribers see what was requested. Tree pre-
        // populated with a group so the explicit-parent path
        // doesn't take the empty-tree rejection branch.
        let (mut state, tab_id) = fresh_tab();
        let mut group = leaf_node("group", "");
        group.data = None;
        group.children = vec![leaf_node("a", "ba")];
        state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(group);

        let events = update(
            &mut state,
            Command::LayoutInsertNode {
                tab_id: tab_id.clone(),
                node: leaf_node("new", "b1"),
                parent_id: Some("group".into()),
                index: Some(0),
                focus_after: false,
                magnify_after: false,
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        match &events[0] {
            Event::LayoutNodeInserted { parent_id, index, .. } => {
                assert_eq!(parent_id.as_deref(), Some("group"));
                assert_eq!(*index, Some(0));
            }
            other => panic!("expected LayoutNodeInserted, got {:?}", other),
        }
    }

    #[test]
    fn layout_delete_node_clears_focused_when_target_was_focused() {
        let (mut state, tab_id) = fresh_tab();
        // Tree: group with two leaves.
        let mut root = leaf_node("root-group", "");
        root.data = None;
        root.children = vec![leaf_node("a", "b1"), leaf_node("b", "b2")];
        state.tabs.get_mut(&tab_id).unwrap().rootnode = Some(root);
        state.tabs.get_mut(&tab_id).unwrap().focused_node_id = "a".into();

        let events = update(
            &mut state,
            Command::LayoutDeleteNode {
                tab_id: tab_id.clone(),
                node_id: "a".into(),
                correlation_id: "corr-del".into(),
            },
            &ctx(1),
        );
        assert!(matches!(
            &events[0],
            Event::LayoutNodeDeleted { was_focused: true, .. }
        ));
        assert_eq!(state.tabs[&tab_id].focused_node_id, "");
    }
}

