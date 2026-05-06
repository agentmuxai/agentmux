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

use agentmux_common::ipc::{ClientKind, Command, ErrorCode, Event, LifecyclePhase};

use crate::state::{
    BlockRecord, ProcessRecord, ProcessState, State, TabRecord, WindowRecord, WorkspaceRecord,
};

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
        Command::Register { kind, pid, version } => handle_register(state, ctx, kind, pid, version),
        Command::Goodbye => handle_goodbye(state, ctx),
        Command::Ping { nonce } => {
            let v = state.bump_version();
            vec![Event::Pong { nonce, version: v }]
        }
        Command::GetSrvSnapshot => handle_get_srv_snapshot(state),
        Command::GetEvents { .. } => Vec::new(), // intercepted by server; unreachable
        Command::CreateWorkspace { name } => handle_create_workspace(state, name),
        Command::DeleteWorkspace { workspace_id, force } => {
            handle_delete_workspace(state, workspace_id, force)
        }
        Command::CreateTab { workspace_id, name } => handle_create_tab(state, workspace_id, name),
        Command::DeleteTab { workspace_id, tab_id, force } => handle_delete_tab(state, workspace_id, tab_id, force),
        Command::SetActiveTab { workspace_id, tab_id } => {
            handle_set_active_tab(state, workspace_id, tab_id)
        }
        Command::ReorderTab {
            workspace_id,
            tab_id,
            new_index,
        } => handle_reorder_tab(state, workspace_id, tab_id, new_index),
        Command::CreateBlock { tab_id, meta } => handle_create_block(state, tab_id, meta),
        Command::DeleteBlock { tab_id, block_id } => handle_delete_block(state, tab_id, block_id),
        Command::SetFocusedNode { tab_id, node_id } => {
            handle_set_focused_node(state, tab_id, node_id)
        }
        Command::SetMagnifiedNode { tab_id, node_id } => {
            handle_set_magnified_node(state, tab_id, node_id)
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
        } => handle_layout_clear(state, tab_id, correlation_id),
        Command::LayoutSetTree {
            tab_id,
            new_tree,
            correlation_id,
        } => handle_layout_set_tree(state, tab_id, new_tree, correlation_id),
        Command::LayoutInsertNode {
            tab_id,
            node,
            parent_id,
            index,
            focus_after,
            magnify_after,
            correlation_id,
        } => handle_layout_insert_node(
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
        } => handle_layout_delete_node(state, tab_id, node_id, correlation_id),
        Command::CreateWindow {
            window_id,
            workspace_id,
        } => handle_create_window(state, window_id, workspace_id),
        Command::CloseWindowInternal { window_id } => {
            handle_close_window_internal(state, window_id)
        }
        Command::SwitchWorkspace {
            window_id,
            workspace_id,
        } => handle_switch_workspace(state, window_id, workspace_id),
        Command::ReorderTabsBulk {
            workspace_id,
            tab_ids,
        } => handle_reorder_tabs_bulk(state, workspace_id, tab_ids),
        Command::RenameWorkspace { workspace_id, name } => {
            handle_rename_workspace(state, workspace_id, name)
        }
        Command::RenameTab { tab_id, name } => handle_rename_tab(state, tab_id, name),
        Command::UpdateWorkspaceMeta {
            workspace_id,
            meta_patch,
        } => handle_update_workspace_meta(state, workspace_id, meta_patch),
        Command::UpdateTabMeta {
            tab_id,
            meta_patch,
        } => handle_update_tab_meta(state, tab_id, meta_patch),
        Command::UpdateBlockMeta {
            block_id,
            meta_patch,
        } => handle_update_block_meta(state, block_id, meta_patch),
        Command::MoveTab {
            tab_id,
            src_workspace_id,
            dst_workspace_id,
            dst_index,
        } => handle_move_tab(state, tab_id, src_workspace_id, dst_workspace_id, dst_index),
        Command::MoveBlock {
            block_id,
            src_tab_id,
            dst_tab_id,
            dst_index,
        } => handle_move_block(state, block_id, src_tab_id, dst_tab_id, dst_index),
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

fn handle_register(
    state: &mut State,
    ctx: &Ctx,
    kind: ClientKind,
    pid: u32,
    version: String,
) -> Vec<Event> {
    // Idempotent on duplicate Register from the same PID — preserve
    // the original record (per launcher's pattern). Accept fresh
    // Registers only when the PID has no record OR the existing
    // record is Exited (PID recycled).
    let prior_state = state.processes.get(&pid).map(|r| r.state);
    let allow_register = match prior_state {
        None => true,
        Some(ProcessState::Exited { .. }) => true,
        _ => false,
    };
    if !allow_register {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::AlreadyRegistered,
            message: format!("pid {} is already registered with srv", pid),
            fatal: false,
            version: v,
        }];
    }

    let mut out = Vec::with_capacity(3);

    // Insert the process record.
    state.processes.insert(
        pid,
        ProcessRecord {
            pid,
            kind,
            state: ProcessState::Running,
            spawned_at: ctx.now_rfc3339.clone(),
            version: version.clone(),
        },
    );
    let v = state.bump_version();
    out.push(Event::ProcessSpawned {
        pid,
        kind,
        client_version: version,
        version: v,
    });

    // First Register transitions srv to Running. Same lifecycle
    // pattern as launcher.
    if state.lifecycle == LifecyclePhase::Starting {
        state.lifecycle = LifecyclePhase::Running;
        let v = state.bump_version();
        out.push(Event::LifecyclePhaseChanged {
            from: LifecyclePhase::Starting,
            to: LifecyclePhase::Running,
            version: v,
        });
    }

    let client_id = state.alloc_client_id();
    let v = state.bump_version();
    // Sentinel launcher_pid / launcher_version on Registered — IPC
    // server patches these to the real srv identity before broadcast.
    out.push(Event::Registered {
        client_id,
        launcher_pid: 0,
        launcher_version: String::new(),
        version: v,
    });
    out
}

fn handle_goodbye(state: &mut State, ctx: &Ctx) -> Vec<Event> {
    let Some(pid) = ctx.registered_pid else {
        return Vec::new();
    };
    let Some(record) = state.processes.get_mut(&pid) else {
        return Vec::new();
    };
    if matches!(record.state, ProcessState::Exited { .. }) {
        return Vec::new();
    }
    record.state = ProcessState::Exited { code: 0 };
    let v = state.bump_version();
    vec![Event::ProcessExited {
        pid,
        code: 0,
        version: v,
    }]
}

fn handle_get_srv_snapshot(state: &mut State) -> Vec<Event> {
    let v = state.bump_version();
    let mut workspaces: Vec<(String, String)> = state
        .workspaces
        .values()
        .map(|w| (w.workspace_id.clone(), w.name.clone()))
        .collect();
    // Stable ordering for diffability — reducer state is HashMap so
    // iteration order is non-deterministic.
    workspaces.sort_by(|a, b| a.0.cmp(&b.0));
    let mut tabs: Vec<(String, String, String)> = state
        .tabs
        .values()
        .map(|t| (t.tab_id.clone(), t.workspace_id.clone(), t.name.clone()))
        .collect();
    tabs.sort_by(|a, b| a.0.cmp(&b.0));
    let mut active_tabs: Vec<(String, String)> = state
        .workspaces
        .values()
        .filter_map(|w| {
            w.active_tab_id
                .as_ref()
                .map(|t| (w.workspace_id.clone(), t.clone()))
        })
        .collect();
    active_tabs.sort_by(|a, b| a.0.cmp(&b.0));
    let mut blocks: Vec<(String, String)> = state
        .blocks
        .values()
        .map(|b| (b.block_id.clone(), b.tab_id.clone()))
        .collect();
    blocks.sort_by(|a, b| a.0.cmp(&b.0));
    vec![Event::SrvSnapshot {
        version: v,
        lifecycle: state.lifecycle,
        workspaces,
        tabs,
        active_tabs,
        blocks,
    }]
}

/// Phase E.2 — create a new workspace. Reducer assigns the OID
/// (UUID), inserts into canonical state, emits WorkspaceCreated.
/// NOT idempotent on retry: each invocation generates a fresh UUID
/// and inserts a new row, so a saga that double-fires CreateWorkspace
/// would create two distinct workspaces. Saga-side dedup (correlation
/// IDs / saga state machine) is responsible for at-most-once delivery
/// when sagas land in E.5+.
fn handle_create_workspace(state: &mut State, name: String) -> Vec<Event> {
    let workspace_id = uuid::Uuid::new_v4().to_string();
    state.workspaces.insert(
        workspace_id.clone(),
        WorkspaceRecord {
            workspace_id: workspace_id.clone(),
            name: name.clone(),
            tab_ids: Vec::new(),
            active_tab_id: None,
        },
    );
    let v = state.bump_version();
    vec![Event::WorkspaceCreated {
        workspace_id,
        name,
        version: v,
    }]
}

/// Phase E.2 — delete a workspace from canonical state. Idempotent:
/// deleting a missing workspace is a silent no-op. Cascades to the
/// workspace's tabs (E.2b) and through to each tab's blocks (E.3):
/// every tab whose `workspace_id` matches is removed from
/// `state.tabs`, and each block in those tabs is removed from
/// `state.blocks`, before the workspace itself goes away. Cascade
/// events are NOT emitted individually — subscribers observing
/// `WorkspaceDeleted` are expected to drop dependent state (mirrors
/// how `wcore::delete_workspace` cascades in SQLite).
///
/// The `force` parameter (Step 5 PR 2) is provenance-only: it carries
/// through to the durable saga log when the saga drives this dispatch
/// (`force = true`), and is ignored by the reducer's cascade logic.
/// The reducer is a pure mutator — it must always cascade to keep
/// in-memory state consistent regardless of whether a saga or a
/// legacy/internal path is calling.
fn handle_delete_workspace(
    state: &mut State,
    workspace_id: String,
    _force: bool,
) -> Vec<Event> {
    let Some(removed) = state.workspaces.remove(&workspace_id) else {
        return Vec::new();
    };
    for tab_id in &removed.tab_ids {
        if let Some(tab) = state.tabs.remove(tab_id) {
            for block_id in &tab.block_ids {
                state.blocks.remove(block_id);
            }
        }
    }
    // Phase E.5 — drop window mappings that point at the deleted
    // workspace AND emit `SrvWindowClosed` for each so the persist
    // subscriber prunes the SQLite Window row + Client.windowids.
    // The original cascade (E.5.1+2) was silent, leaving downstream
    // projections out of sync. (codex P1 follow-up to #619.)
    let dropped_window_ids: Vec<String> = state
        .windows
        .iter()
        .filter(|(_, w)| w.workspace_id == workspace_id)
        .map(|(id, _)| id.clone())
        .collect();
    for id in &dropped_window_ids {
        state.windows.remove(id);
    }
    let mut events = Vec::with_capacity(1 + dropped_window_ids.len());
    let v = state.bump_version();
    events.push(Event::WorkspaceDeleted {
        workspace_id,
        version: v,
    });
    for window_id in dropped_window_ids {
        let v = state.bump_version();
        events.push(Event::SrvWindowClosed { window_id, version: v });
    }
    events
}

/// Phase E.2b — create a tab inside a workspace. Validates the
/// parent exists; otherwise emits `Event::Error` (non-fatal). On
/// success: assigns a UUID, appends to the workspace's `tab_ids`,
/// inserts into `state.tabs`, emits `Event::TabCreated`. If the
/// workspace had no active tab, the new tab also becomes active
/// and an `Event::ActiveTabChanged` is emitted alongside.
///
/// NOT idempotent on retry (same UUID-assignment caveat as
/// `handle_create_workspace`).
fn handle_create_tab(state: &mut State, workspace_id: String, name: String) -> Vec<Event> {
    let Some(workspace_record) = state.workspaces.get(&workspace_id) else {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("CreateTab: workspace not found: {}", workspace_id),
            fatal: false,
            version: v,
        }];
    };
    // codex P2 #622: auto-generate `tabN` when name is empty,
    // matching `wcore::create_tab`'s default-naming behaviour. The
    // counter uses the reducer's tab_ids length + 1 (matching the
    // old SQLite-side count: tabids.len() + pinnedtabids.len() + 1
    // — pinnedtabids stays at zero in production since pinning
    // was removed in E.2c.3b, so reducer-only counting matches).
    let resolved_name = if name.is_empty() {
        format!("tab{}", workspace_record.tab_ids.len() + 1)
    } else {
        name
    };
    let tab_id = uuid::Uuid::new_v4().to_string();
    state.tabs.insert(
        tab_id.clone(),
        TabRecord {
            tab_id: tab_id.clone(),
            workspace_id: workspace_id.clone(),
            name: resolved_name.clone(),
            block_ids: Vec::new(),
            focused_node_id: String::new(),
            magnified_node_id: String::new(),
            rootnode: None,
        },
    );
    let workspace = state.workspaces.get_mut(&workspace_id).expect("checked");
    workspace.tab_ids.push(tab_id.clone());
    let activated = if workspace.active_tab_id.is_none() {
        workspace.active_tab_id = Some(tab_id.clone());
        true
    } else {
        false
    };
    let mut events = Vec::with_capacity(2);
    let v = state.bump_version();
    events.push(Event::TabCreated {
        workspace_id: workspace_id.clone(),
        tab_id: tab_id.clone(),
        name: resolved_name,
        version: v,
    });
    if activated {
        let v2 = state.bump_version();
        events.push(Event::ActiveTabChanged {
            workspace_id,
            tab_id: Some(tab_id),
            version: v2,
        });
    }
    events
}

/// Phase E.2b — delete a tab from a workspace. Idempotent: deleting
/// a missing tab is a silent no-op. If the deleted tab was the
/// active tab, the workspace's active tab becomes the next tab in
/// `tab_ids` (or the previous one if the deleted was last; or None
/// if the workspace is now empty), and an `Event::ActiveTabChanged`
/// is emitted alongside `Event::TabDeleted`.
fn handle_delete_tab(
    state: &mut State,
    workspace_id: String,
    tab_id: String,
    force: bool,
) -> Vec<Event> {
    let Some(workspace) = state.workspaces.get_mut(&workspace_id) else {
        return Vec::new();
    };
    let Some(pos) = workspace.tab_ids.iter().position(|t| t == &tab_id) else {
        return Vec::new();
    };
    // Last-tab guard with `force` bypass (round 4 of PR #633).
    // History:
    //   * Round 1: saga pre-check → reagent flagged TOCTOU race.
    //   * Round 2: moved guard to reducer (atomic) → broke CreateTab
    //     compensation (codex P1 round 2) and Cmd+W keyboard flow
    //     (codex P1 round 1).
    //   * Round 3: removed guard entirely; saga keeps soft pre-check
    //     → codex P2 round 4 re-flagged the TOCTOU race.
    //   * Round 4 (this): atomic guard with `force: bool` bypass.
    //     User-facing flows (CloseTab RPC → DeleteTab saga) pass
    //     `force: false`; compensation paths (`CreateTab` rollback,
    //     `PromoteBlockToTab.ctx.compensate`) pass `force: true`.
    //     Frontend keyboard handler `simpleCloseStaticTab` already
    //     gates pre-RPC, so the reducer rejection is a defense-in-
    //     depth backstop that catches automation/race paths.
    if !force && workspace.tab_ids.len() <= 1 {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!(
                "DeleteTab: refusing to delete the last tab in workspace {} (would leave empty workspace; pass force=true for compensation paths)",
                workspace_id,
            ),
            fatal: false,
            version: v,
        }];
    }
    workspace.tab_ids.remove(pos);
    let active_changed = if workspace.active_tab_id.as_deref() == Some(tab_id.as_str()) {
        let new_active = workspace
            .tab_ids
            .get(pos)
            .or_else(|| pos.checked_sub(1).and_then(|i| workspace.tab_ids.get(i)))
            .cloned();
        workspace.active_tab_id = new_active.clone();
        Some(new_active)
    } else {
        None
    };
    let removed_tab = state.tabs.remove(&tab_id);
    // Phase E.3 — cascade to blocks. Subscribers observing TabDeleted
    // are expected to drop dependent block state (no per-block
    // BlockDeleted events emitted; mirrors workspace→tabs cascade
    // semantics).
    if let Some(tab) = &removed_tab {
        for block_id in &tab.block_ids {
            state.blocks.remove(block_id);
        }
    }
    let mut events = Vec::with_capacity(2);
    let v = state.bump_version();
    events.push(Event::TabDeleted {
        workspace_id: workspace_id.clone(),
        tab_id,
        version: v,
    });
    if let Some(new_active) = active_changed {
        let v2 = state.bump_version();
        events.push(Event::ActiveTabChanged {
            workspace_id,
            tab_id: new_active,
            version: v2,
        });
    }
    events
}

/// Phase E.2b — set a workspace's active tab. No-op if already
/// active. Errors (non-fatal) if the workspace doesn't exist or the
/// tab isn't in that workspace's tab list.
fn handle_set_active_tab(
    state: &mut State,
    workspace_id: String,
    tab_id: String,
) -> Vec<Event> {
    let Some(workspace) = state.workspaces.get_mut(&workspace_id) else {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("SetActiveTab: workspace not found: {}", workspace_id),
            fatal: false,
            version: v,
        }];
    };
    if !workspace.tab_ids.iter().any(|t| t == &tab_id) {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!(
                "SetActiveTab: tab {} not in workspace {}",
                tab_id, workspace_id
            ),
            fatal: false,
            version: v,
        }];
    }
    if workspace.active_tab_id.as_deref() == Some(tab_id.as_str()) {
        return Vec::new();
    }
    workspace.active_tab_id = Some(tab_id.clone());
    let v = state.bump_version();
    vec![Event::ActiveTabChanged {
        workspace_id,
        tab_id: Some(tab_id),
        version: v,
    }]
}

/// Phase E.2c.3b — reorder a tab within its workspace's
/// `tab_ids`. `new_index` is clamped to `tab_ids.len()`. No-op
/// if the tab is already at that position. Errors (non-fatal) if
/// the workspace doesn't exist or the tab isn't in its tab list.
fn handle_reorder_tab(
    state: &mut State,
    workspace_id: String,
    tab_id: String,
    new_index: u32,
) -> Vec<Event> {
    let Some(workspace) = state.workspaces.get_mut(&workspace_id) else {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("ReorderTab: workspace not found: {}", workspace_id),
            fatal: false,
            version: v,
        }];
    };
    let Some(current_pos) = workspace.tab_ids.iter().position(|t| t == &tab_id) else {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!(
                "ReorderTab: tab {} not in workspace {}",
                tab_id, workspace_id
            ),
            fatal: false,
            version: v,
        }];
    };
    let len = workspace.tab_ids.len();
    let target = (new_index as usize).min(len.saturating_sub(1));
    if current_pos == target {
        return Vec::new();
    }
    let id = workspace.tab_ids.remove(current_pos);
    workspace.tab_ids.insert(target, id);
    let v = state.bump_version();
    vec![Event::TabReordered {
        workspace_id,
        tab_id,
        new_index: target as u32,
        version: v,
    }]
}

/// Phase E.3 — create a block inside a tab. Validates parent tab
/// exists; otherwise emits `Event::Error` (non-fatal). On success:
/// assigns a UUID, appends to the tab's `block_ids`, inserts into
/// `state.blocks`, emits `Event::BlockCreated`.
///
/// NOT idempotent on retry (UUID assignment per call); saga-side
/// dedup is responsible for at-most-once delivery in E.5+.
fn handle_create_block(
    state: &mut State,
    tab_id: String,
    meta: serde_json::Value,
) -> Vec<Event> {
    if !state.tabs.contains_key(&tab_id) {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("CreateBlock: tab not found: {}", tab_id),
            fatal: false,
            version: v,
        }];
    }
    let block_id = uuid::Uuid::new_v4().to_string();
    state.blocks.insert(
        block_id.clone(),
        BlockRecord {
            block_id: block_id.clone(),
            tab_id: tab_id.clone(),
        },
    );
    let tab = state.tabs.get_mut(&tab_id).expect("checked");
    tab.block_ids.push(block_id.clone());
    let v = state.bump_version();
    vec![Event::BlockCreated {
        tab_id,
        block_id,
        meta,
        version: v,
    }]
}

/// Phase E.3 — delete a block from a tab. Idempotent: deleting a
/// missing tab or missing block is a silent no-op.
fn handle_delete_block(state: &mut State, tab_id: String, block_id: String) -> Vec<Event> {
    let Some(tab) = state.tabs.get_mut(&tab_id) else {
        return Vec::new();
    };
    let Some(pos) = tab.block_ids.iter().position(|b| b == &block_id) else {
        return Vec::new();
    };
    tab.block_ids.remove(pos);
    state.blocks.remove(&block_id);
    let v = state.bump_version();
    vec![Event::BlockDeleted {
        tab_id,
        block_id,
        version: v,
    }]
}

/// Phase E.4 (Option A) — set the focused layout-node id on a tab.
/// Errors (non-fatal) if the tab is unknown to the reducer; no-op
/// short-circuit when the value is already current. Empty `node_id`
/// clears the field. Bumps the version only on real changes so a
/// burst of identical sets doesn't churn the event stream.
fn handle_set_focused_node(state: &mut State, tab_id: String, node_id: String) -> Vec<Event> {
    let Some(tab) = state.tabs.get_mut(&tab_id) else {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("SetFocusedNode: unknown tab {}", tab_id),
            fatal: false,
            version: v,
        }];
    };
    if tab.focused_node_id == node_id {
        return Vec::new();
    }
    tab.focused_node_id = node_id.clone();
    let v = state.bump_version();
    vec![Event::FocusedNodeChanged {
        tab_id,
        node_id,
        version: v,
    }]
}

/// Phase E.4 (Option A) — set the magnified layout-node id on a tab.
/// Same shape as `handle_set_focused_node`. Empty `node_id` is the
/// toggle-off / clear case.
fn handle_set_magnified_node(state: &mut State, tab_id: String, node_id: String) -> Vec<Event> {
    let Some(tab) = state.tabs.get_mut(&tab_id) else {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("SetMagnifiedNode: unknown tab {}", tab_id),
            fatal: false,
            version: v,
        }];
    };
    if tab.magnified_node_id == node_id {
        return Vec::new();
    }
    tab.magnified_node_id = node_id.clone();
    let v = state.bump_version();
    vec![Event::MagnifiedNodeChanged {
        tab_id,
        node_id,
        version: v,
    }]
}

// ── Phase E.4.B Phase 5 — layout tree reducer arms ──────────────────
//
// 4 of 11 arms shipped in this PR (clear, set_tree, insert_node,
// delete_node). Pattern uniform across all 11:
//   1. Look up tab; emit Event::Error on missing.
//   2. Mutate `tab.rootnode` via the pure helpers in
//      `crate::backend::layout::*` (shipped in Phase 4 / PRs #691, #692).
//   3. Emit the matching Event::Layout* variant carrying the
//      correlation_id and a fresh version.
//
// **No production callers yet** — wcore-direct writers continue to be
// authoritative until Phase 7 migrates them. These arms are the
// destination, not the source; tests below exercise them in isolation.

fn handle_layout_clear(
    state: &mut State,
    tab_id: String,
    correlation_id: String,
) -> Vec<Event> {
    let Some(tab) = state.tabs.get_mut(&tab_id) else {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("LayoutClear: unknown tab {}", tab_id),
            fatal: false,
            version: v,
        }];
    };
    tab.rootnode = None;
    tab.focused_node_id = String::new();
    tab.magnified_node_id = String::new();
    let v = state.bump_version();
    vec![Event::LayoutCleared {
        tab_id,
        correlation_id,
        version: v,
    }]
}

fn handle_layout_set_tree(
    state: &mut State,
    tab_id: String,
    new_tree: Option<agentmux_common::LayoutNode>,
    correlation_id: String,
) -> Vec<Event> {
    let Some(tab) = state.tabs.get_mut(&tab_id) else {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("LayoutSetTree: unknown tab {}", tab_id),
            fatal: false,
            version: v,
        }];
    };
    tab.rootnode = new_tree.clone();
    // Reagent P1 PR #715 round 1: when the tree is wiped, focused/
    // magnified ids would point at non-existent nodes. Match
    // `handle_layout_clear`'s contract for the empty-tree case.
    if new_tree.is_none() {
        tab.focused_node_id = String::new();
        tab.magnified_node_id = String::new();
    }
    let v = state.bump_version();
    vec![Event::LayoutTreeReplaced {
        tab_id,
        new_tree,
        correlation_id,
        version: v,
    }]
}

fn handle_layout_insert_node(
    state: &mut State,
    tab_id: String,
    node: agentmux_common::LayoutNode,
    parent_id: Option<String>,
    index: Option<usize>,
    focus_after: bool,
    magnify_after: bool,
    correlation_id: String,
) -> Vec<Event> {
    let Some(tab) = state.tabs.get_mut(&tab_id) else {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("LayoutInsertNode: unknown tab {}", tab_id),
            fatal: false,
            version: v,
        }];
    };
    // Empty tree → promote new node to root (frontend's
    // `findNextInsertLocation` returns the empty case as "make me root").
    // Non-empty tree → delegate to the pure helper, which honours the
    // same heuristic the frontend uses.
    match tab.rootnode.as_mut() {
        None => tab.rootnode = Some(node.clone()),
        Some(root) => crate::backend::layout::insert_node(root, node.clone()),
    }
    // Codex P2 PR #715 round 1: honour focus_after / magnify_after.
    // The schema documents these as the side effects callers rely on
    // for "insert + activate" flows; ignoring them desyncs the snapshot
    // from the event the caller's handler observed.
    if focus_after {
        tab.focused_node_id = node.id.clone();
    }
    if magnify_after {
        tab.magnified_node_id = node.id.clone();
    }
    // `parent_id` and `index` are accepted for forward-compat with the
    // command schema but ignored by the pure-heuristic insert helper.
    // Phase 7 wcore-migration will exercise the parent_id/index path
    // via `LayoutInsertNodeAtIndex` (a separate command); this arm
    // matches `findNextInsertLocation` semantics exactly. The values
    // are still echoed in the emitted event below so subscribers see
    // what the caller asked for.
    let v = state.bump_version();
    vec![Event::LayoutNodeInserted {
        tab_id,
        node,
        // Reagent P2 (kimi) PR #715 round 3: pass the command's
        // parent_id / index through to the event so subscribers see
        // what the caller asked for, not a hardcoded `None, None`.
        // The pure helper currently uses the `findNextInsertLocation`
        // heuristic and ignores these hints — but the event is the
        // record of what was *requested*; subscribers can correlate
        // with the resulting tree by inspecting `node` itself.
        parent_id,
        index,
        correlation_id,
        version: v,
    }]
}

fn handle_layout_delete_node(
    state: &mut State,
    tab_id: String,
    node_id: String,
    correlation_id: String,
) -> Vec<Event> {
    let Some(tab) = state.tabs.get_mut(&tab_id) else {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("LayoutDeleteNode: unknown tab {}", tab_id),
            fatal: false,
            version: v,
        }];
    };
    // Snapshot pre-delete focus/magnify so we can both detect
    // direct-target hits AND post-walk for indirect orphaning.
    let pre_focused = tab.focused_node_id.clone();
    let pre_magnified = tab.magnified_node_id.clone();
    let Some(root) = tab.rootnode.as_mut() else {
        // Empty tree — nothing to delete; idempotent no-op (no event).
        return Vec::new();
    };
    // Codex P2 PR #715 round 1: `backend::layout::delete_node` leaves
    // root deletion to the caller (returns Ok(()) with the root
    // unmodified). Detect the root case here and clear the tree
    // wholesale so the reducer state matches the
    // `LayoutNodeDeleted` event we emit.
    if root.id == node_id {
        tab.rootnode = None;
    } else if let Err(e) = crate::backend::layout::delete_node(root, &node_id) {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("LayoutDeleteNode: {} (tab {})", e, tab_id),
            fatal: false,
            version: v,
        }];
    }

    // Reagent P2 PR #715 round 3 (finding B) and Codex P2 round 3
    // (finding C): both involve focus/magnify ids referencing nodes
    // that no longer exist in the tree post-delete:
    //   - B: `delete_recursive` collapse-sole-child rewrites a
    //        parent's id to the promoted child's id. If
    //        focused/magnified was the parent's original id, that
    //        id is gone from the tree even though the same physical
    //        node remains.
    //   - C: deleting a container removes all descendants. If
    //        focused/magnified was a descendant, it's gone too.
    // Direct-target match (`pre_focused == node_id`) doesn't catch
    // either case. Reconcile by walking the post-delete tree and
    // clearing any focus/magnify id that no longer resolves.
    let id_resolves = |id: &str| -> bool {
        if id.is_empty() {
            return true;
        }
        match tab.rootnode.as_ref() {
            None => false,
            Some(root) => crate::backend::layout::find_node_by_id(root, id).is_some(),
        }
    };
    let was_focused = !pre_focused.is_empty() && !id_resolves(&pre_focused);
    let was_magnified = !pre_magnified.is_empty() && !id_resolves(&pre_magnified);
    if was_focused {
        tab.focused_node_id = String::new();
    }
    if was_magnified {
        tab.magnified_node_id = String::new();
    }

    let v = state.bump_version();
    vec![Event::LayoutNodeDeleted {
        tab_id,
        node_id,
        was_focused,
        was_magnified,
        correlation_id,
        version: v,
    }]
}

/// Phase E.5 — record a new window→workspace mapping. Validates
/// the parent workspace exists; otherwise emits `Event::Error`
/// (non-fatal). Idempotent on duplicate `window_id`: re-issuing
/// for the same window updates the workspace pointer if it
/// changed, or no-ops if identical.
fn handle_create_window(
    state: &mut State,
    window_id: String,
    workspace_id: String,
) -> Vec<Event> {
    if !state.workspaces.contains_key(&workspace_id) {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("CreateWindow: workspace not found: {}", workspace_id),
            fatal: false,
            version: v,
        }];
    }
    if let Some(existing) = state.windows.get(&window_id) {
        if existing.workspace_id == workspace_id {
            return Vec::new();
        }
    }
    state.windows.insert(
        window_id.clone(),
        WindowRecord {
            window_id: window_id.clone(),
            workspace_id: workspace_id.clone(),
        },
    );
    let v = state.bump_version();
    vec![Event::SrvWindowOpened {
        window_id,
        workspace_id,
        version: v,
    }]
}

/// Phase E.5 — remove a window's workspace mapping. Idempotent
/// silent no-op on missing.
fn handle_close_window_internal(state: &mut State, window_id: String) -> Vec<Event> {
    if state.windows.remove(&window_id).is_none() {
        return Vec::new();
    }
    let v = state.bump_version();
    vec![Event::SrvWindowClosed {
        window_id,
        version: v,
    }]
}

/// Phase E.5 — change which workspace a window points at. Errors
/// (non-fatal) if the window or destination workspace is unknown.
/// No-op if the window is already pointing at the destination.
fn handle_switch_workspace(
    state: &mut State,
    window_id: String,
    workspace_id: String,
) -> Vec<Event> {
    if !state.workspaces.contains_key(&workspace_id) {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!(
                "SwitchWorkspace: destination workspace not found: {}",
                workspace_id
            ),
            fatal: false,
            version: v,
        }];
    }
    let Some(window) = state.windows.get_mut(&window_id) else {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("SwitchWorkspace: window not found: {}", window_id),
            fatal: false,
            version: v,
        }];
    };
    if window.workspace_id == workspace_id {
        return Vec::new();
    }
    window.workspace_id = workspace_id.clone();
    let v = state.bump_version();
    vec![Event::SrvWindowWorkspaceChanged {
        window_id,
        workspace_id,
        version: v,
    }]
}

/// Phase E.5.3 — replace a workspace's `tab_ids` with the given
/// list. Validates the workspace exists and the new list is a
/// permutation of the current set (same elements, possibly
/// different order). No-op if identical.
fn handle_reorder_tabs_bulk(
    state: &mut State,
    workspace_id: String,
    tab_ids: Vec<String>,
) -> Vec<Event> {
    // codex P1 #620 carryover: relax membership validation until tab
    // moves are migrated through the reducer. `MoveTabToWorkspace`
    // and `PromoteBlockToTab` (planned for PR 4) still write through
    // wcore without dispatching reducer commands, so the reducer's
    // view of `workspace.tab_ids` can be stale relative to SQLite.
    // A subsequent `UpdateTabIds` (now routed through this command)
    // must not refuse the canonical order just because the reducer
    // hasn't seen the upstream move yet — that would be a
    // user-visible regression vs. the prior wcore-direct path.
    //
    // Treat the caller's `tab_ids` as authoritative. The remaining
    // checks are basic sanity: the workspace must exist in the
    // reducer, and `tab_ids` must not contain duplicates (which would
    // produce a corrupt persisted ordering with no way for the
    // subscriber to recover). Length / set comparison against the
    // reducer's stale view is dropped here; PR 4 reinstates strict
    // validation once tab moves go through the reducer.
    if !state.workspaces.contains_key(&workspace_id) {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("ReorderTabsBulk: workspace not found: {}", workspace_id),
            fatal: false,
            version: v,
        }];
    }
    {
        let mut seen: std::collections::HashSet<&String> =
            std::collections::HashSet::with_capacity(tab_ids.len());
        for id in &tab_ids {
            if !seen.insert(id) {
                let v = state.bump_version();
                return vec![Event::Error {
                    code: ErrorCode::InvalidCommand,
                    message: format!(
                        "ReorderTabsBulk: tab_ids contains duplicate entry: {}",
                        id
                    ),
                    fatal: false,
                    version: v,
                }];
            }
        }
    }
    if state.workspaces.get(&workspace_id).expect("checked").tab_ids == tab_ids {
        return Vec::new();
    }
    state.workspaces.get_mut(&workspace_id).expect("checked").tab_ids = tab_ids.clone();
    let v = state.bump_version();
    vec![Event::TabsReorderedBulk {
        workspace_id,
        tab_ids,
        version: v,
    }]
}

/// Phase E.5.5 — move a tab from `src_workspace_id` to
/// `dst_workspace_id`, inserting at `dst_index` (clamped to dst's
/// length). Updates the tab's `workspace_id`, removes it from src's
/// `tab_ids`, inserts into dst's `tab_ids`. If the tab was src's
/// `active_tab_id`, src's active reverts to its first remaining
/// tab (or `None` when empty).
///
/// Errors when:
/// * source / dest workspace not found,
/// * tab not found,
/// * `tab.workspace_id != src_workspace_id` (caller-side bug),
/// * `src_workspace_id == dst_workspace_id` (use `ReorderTab` for
///   intra-workspace reorders — same-workspace moves through this
///   path would create ambiguity around `dst_index` semantics).
fn handle_move_tab(
    state: &mut State,
    tab_id: String,
    src_workspace_id: String,
    dst_workspace_id: String,
    dst_index: u32,
) -> Vec<Event> {
    if src_workspace_id == dst_workspace_id {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: "MoveTab: src and dst workspaces are identical; use ReorderTab".into(),
            fatal: false,
            version: v,
        }];
    }
    // Strict validation (Phase E.4 strict-mode flip): the
    // migration-tolerant lazy-import fallback (codex P1 round-2
    // #621) was removed once the soak window closed with no
    // `lazy-import` warnings observed in production. All reducer-
    // routed paths now keep `state.tabs` and
    // `state.workspaces[*].tab_ids` consistent with SQLite, so we
    // can reject unknown tabs and workspace_id mismatches outright.
    if !state.workspaces.contains_key(&src_workspace_id) {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("MoveTab: src workspace not found: {}", src_workspace_id),
            fatal: false,
            version: v,
        }];
    }
    if !state.workspaces.contains_key(&dst_workspace_id) {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("MoveTab: dst workspace not found: {}", dst_workspace_id),
            fatal: false,
            version: v,
        }];
    }
    let tab_workspace_id: Option<String> =
        state.tabs.get(&tab_id).map(|t| t.workspace_id.clone());
    match tab_workspace_id {
        None => {
            let v = state.bump_version();
            return vec![Event::Error {
                code: ErrorCode::InvalidCommand,
                message: format!("MoveTab: tab not found in state: {}", tab_id),
                fatal: false,
                version: v,
            }];
        }
        Some(actual_ws) if actual_ws != src_workspace_id => {
            let v = state.bump_version();
            return vec![Event::Error {
                code: ErrorCode::InvalidCommand,
                message: format!(
                    "MoveTab: workspace_id mismatch — tab {} belongs to {}, not {}",
                    tab_id, actual_ws, src_workspace_id
                ),
                fatal: false,
                version: v,
            }];
        }
        Some(_) => {}
    }

    // Remove from src.
    let new_src_active_tab_id: Option<String> = {
        let src = state.workspaces.get_mut(&src_workspace_id).expect("checked");
        src.tab_ids.retain(|id| id != &tab_id);
        if src.active_tab_id.as_deref() == Some(tab_id.as_str()) {
            src.active_tab_id = src.tab_ids.first().cloned();
        }
        src.active_tab_id.clone()
    };

    // Insert into dst at clamped index. Set the moved tab as dst's
    // new active tab — mirrors wcore::move_tab_to_workspace
    // behaviour and addresses codex P2 #621 (dst.active_tab_id was
    // previously left untouched, so a saga-driven tear-off could
    // produce a destination workspace with no active tab selected).
    let final_dst_index: u32 = {
        let dst = state.workspaces.get_mut(&dst_workspace_id).expect("checked");
        let clamped = (dst_index as usize).min(dst.tab_ids.len());
        dst.tab_ids.insert(clamped, tab_id.clone());
        dst.active_tab_id = Some(tab_id.clone());
        clamped as u32
    };

    // Update the tab's parent.
    state
        .tabs
        .get_mut(&tab_id)
        .expect("checked")
        .workspace_id = dst_workspace_id.clone();

    let v = state.bump_version();
    vec![Event::TabMoved {
        tab_id: tab_id.clone(),
        src_workspace_id,
        dst_workspace_id,
        dst_index: final_dst_index,
        new_src_active_tab_id,
        new_dst_active_tab_id: Some(tab_id),
        version: v,
    }]
}

/// Phase E.5.5 — move a block from `src_tab_id` to `dst_tab_id` at
/// `dst_index` (clamped). Updates `block.tab_id`. Cross-tab moves
/// AND intra-tab repositioning both go through this command (the
/// caller specifies the destination index regardless).
///
/// Errors when source / dest tab missing, block missing, or
/// `block.tab_id != src_tab_id`.
fn handle_move_block(
    state: &mut State,
    block_id: String,
    src_tab_id: String,
    dst_tab_id: String,
    dst_index: u32,
) -> Vec<Event> {
    let validation_error: Option<String> = {
        if !state.tabs.contains_key(&src_tab_id) {
            Some(format!("MoveBlock: src tab not found: {}", src_tab_id))
        } else if !state.tabs.contains_key(&dst_tab_id) {
            Some(format!("MoveBlock: dst tab not found: {}", dst_tab_id))
        } else {
            match state.blocks.get(&block_id) {
                None => Some(format!("MoveBlock: block not found: {}", block_id)),
                Some(block) if block.tab_id != src_tab_id => Some(format!(
                    "MoveBlock: block {} belongs to tab {}, not {}",
                    block_id, block.tab_id, src_tab_id
                )),
                _ => None,
            }
        }
    };
    if let Some(message) = validation_error {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message,
            fatal: false,
            version: v,
        }];
    }

    // Special-case intra-tab move: remove and re-insert in the same
    // tab. The clamp is computed AFTER the removal so dst_index
    // refers to the post-removal list (matches the spec's "position
    // in dst.tab_ids AFTER insertion" semantics for cross-tab moves).
    let final_dst_index: u32 = if src_tab_id == dst_tab_id {
        let tab = state.tabs.get_mut(&src_tab_id).expect("checked");
        tab.block_ids.retain(|id| id != &block_id);
        let clamped = (dst_index as usize).min(tab.block_ids.len());
        tab.block_ids.insert(clamped, block_id.clone());
        clamped as u32
    } else {
        // Remove from src.
        state
            .tabs
            .get_mut(&src_tab_id)
            .expect("checked")
            .block_ids
            .retain(|id| id != &block_id);
        // Insert into dst.
        let dst = state.tabs.get_mut(&dst_tab_id).expect("checked");
        let clamped = (dst_index as usize).min(dst.block_ids.len());
        dst.block_ids.insert(clamped, block_id.clone());
        // Update parent.
        state.blocks.get_mut(&block_id).expect("checked").tab_id = dst_tab_id.clone();
        clamped as u32
    };

    let v = state.bump_version();
    vec![Event::BlockMoved {
        block_id,
        src_tab_id,
        dst_tab_id,
        dst_index: final_dst_index,
        version: v,
    }]
}

/// Phase E.5.3 — rename a workspace. Errors if missing; no-op if
/// the name is unchanged.
fn handle_rename_workspace(state: &mut State, workspace_id: String, name: String) -> Vec<Event> {
    let Some(workspace) = state.workspaces.get_mut(&workspace_id) else {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("RenameWorkspace: workspace not found: {}", workspace_id),
            fatal: false,
            version: v,
        }];
    };
    if workspace.name == name {
        return Vec::new();
    }
    workspace.name = name.clone();
    let v = state.bump_version();
    vec![Event::WorkspaceRenamed {
        workspace_id,
        name,
        version: v,
    }]
}

/// Phase E.5.3 — rename a tab. Errors if missing; no-op if the
/// name is unchanged.
fn handle_rename_tab(state: &mut State, tab_id: String, name: String) -> Vec<Event> {
    let Some(tab) = state.tabs.get_mut(&tab_id) else {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("RenameTab: tab not found: {}", tab_id),
            fatal: false,
            version: v,
        }];
    };
    if tab.name == name {
        return Vec::new();
    }
    tab.name = name.clone();
    let v = state.bump_version();
    vec![Event::TabRenamed {
        tab_id,
        name,
        version: v,
    }]
}

/// Phase E.5.3 — pass-through validation + emit for workspace
/// meta updates. The reducer does NOT mutate meta in state (it
/// doesn't track meta in WorkspaceRecord); the persist subscriber
/// applies the patch directly to wstore. This keeps the reducer's
/// state shape unchanged while still routing every meta mutation
/// through the broadcast bus for observers.
fn handle_update_workspace_meta(
    state: &mut State,
    workspace_id: String,
    meta_patch: serde_json::Value,
) -> Vec<Event> {
    if !state.workspaces.contains_key(&workspace_id) {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!(
                "UpdateWorkspaceMeta: workspace not found: {}",
                workspace_id
            ),
            fatal: false,
            version: v,
        }];
    }
    let v = state.bump_version();
    vec![Event::WorkspaceMetaUpdated {
        workspace_id,
        meta_patch,
        version: v,
    }]
}

/// Phase E.5.3 — pass-through for tab meta updates. Same shape as
/// `handle_update_workspace_meta`.
fn handle_update_tab_meta(
    state: &mut State,
    tab_id: String,
    meta_patch: serde_json::Value,
) -> Vec<Event> {
    if !state.tabs.contains_key(&tab_id) {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("UpdateTabMeta: tab not found: {}", tab_id),
            fatal: false,
            version: v,
        }];
    }
    let v = state.bump_version();
    vec![Event::TabMetaUpdated {
        tab_id,
        meta_patch,
        version: v,
    }]
}

/// Phase E.5.3 — pass-through for block meta updates. Same shape.
fn handle_update_block_meta(
    state: &mut State,
    block_id: String,
    meta_patch: serde_json::Value,
) -> Vec<Event> {
    if !state.blocks.contains_key(&block_id) {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("UpdateBlockMeta: block not found: {}", block_id),
            fatal: false,
            version: v,
        }];
    }
    let v = state.bump_version();
    vec![Event::BlockMetaUpdated {
        block_id,
        meta_patch,
        version: v,
    }]
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
        // Reagent P1 PR #715 round 1: empty-tree set must match
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
        // Codex P2 PR #715 round 1: focus_after=true must update
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
    fn layout_insert_node_honours_magnify_after() {
        let (mut state, tab_id) = fresh_tab();
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
        assert_eq!(state.tabs[&tab_id].focused_node_id, "");
        assert_eq!(state.tabs[&tab_id].magnified_node_id, "new");
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
        // Codex P2 PR #715 round 1: backend::layout::delete_node leaves
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
        // Reagent P1 PR #715 round 1: magnified must be cleared
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
        // Reagent P1 (kimi+sonnet) PR #715 round 3: subscribers need
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
        // Reagent P2 (kimi+sonnet) PR #715 round 3 (finding B):
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
        // Codex P2 PR #715 round 3 (finding C): deleting a container
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
        // Reagent P2 (kimi only) PR #715 round 3 (finding D): event
        // hardcoded `parent_id: None, index: None`; subscribers had
        // no record of what the caller asked for.
        let (mut state, tab_id) = fresh_tab();
        let events = update(
            &mut state,
            Command::LayoutInsertNode {
                tab_id: tab_id.clone(),
                node: leaf_node("new", "b1"),
                parent_id: Some("explicit-parent".into()),
                index: Some(3),
                focus_after: false,
                magnify_after: false,
                correlation_id: "corr".into(),
            },
            &ctx(1),
        );
        match &events[0] {
            Event::LayoutNodeInserted { parent_id, index, .. } => {
                assert_eq!(parent_id.as_deref(), Some("explicit-parent"));
                assert_eq!(*index, Some(3));
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
