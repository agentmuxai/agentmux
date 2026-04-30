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
        Command::DeleteWorkspace { workspace_id } => handle_delete_workspace(state, workspace_id),
        Command::CreateTab { workspace_id, name } => handle_create_tab(state, workspace_id, name),
        Command::DeleteTab { workspace_id, tab_id } => handle_delete_tab(state, workspace_id, tab_id),
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
fn handle_delete_workspace(state: &mut State, workspace_id: String) -> Vec<Event> {
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
) -> Vec<Event> {
    let Some(workspace) = state.workspaces.get_mut(&workspace_id) else {
        return Vec::new();
    };
    let Some(pos) = workspace.tab_ids.iter().position(|t| t == &tab_id) else {
        return Vec::new();
    };
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
    // **Migration-tolerant validation** (codex P1 round-2 #621):
    // tab existence + workspace_id checks are dropped during the
    // migration window. Some tab-creating/moving paths
    // (`PromoteBlockToTab`, etc.) are still wcore-direct — their
    // writes don't flow into reducer state, so `state.tabs` and
    // `state.workspaces[*].tab_ids` can lag SQLite. The saga / RPC
    // handler that called us has already validated against SQLite
    // (the source of truth); here we only enforce that both
    // workspaces exist in the reducer (so we can mutate them) and
    // trust the caller for the tab. If the tab isn't in
    // `state.tabs`, lazy-insert a synthetic record stamped with the
    // dst workspace_id (name unset — refilled when later events
    // touch the tab). PR 4 reinstates strict validation once the
    // remaining wcore-direct paths migrate.
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

    // Remove from src. Even if the reducer's view shows the tab
    // isn't in src.tab_ids (stale state), the persist subscriber's
    // apply_tab_moved reads SQLite and removes from there, so the
    // disk-side ordering ends up correct.
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

    // Update the tab's parent. If the reducer's view didn't have
    // this tab (a wcore-direct creation/move slipped past), lazy-
    // insert a TabRecord. Name is left empty — subsequent events
    // (TabRenamed, etc.) will refill it. The launcher's snapshot
    // view tolerates empty names (renderer reads names from SQLite
    // -sourced events).
    match state.tabs.get_mut(&tab_id) {
        Some(tab) => {
            tab.workspace_id = dst_workspace_id.clone();
        }
        None => {
            state.tabs.insert(
                tab_id.clone(),
                crate::state::TabRecord {
                    tab_id: tab_id.clone(),
                    workspace_id: dst_workspace_id.clone(),
                    name: String::new(),
                    block_ids: Vec::new(),
                },
            );
        }
    }

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
            | Event::Error { version, .. } => *version,
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
            Command::DeleteWorkspace { workspace_id: ws_id },
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
            Command::DeleteWorkspace { workspace_id: ws_a },
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

        // codex P1 round-2 #621: unknown tabs are now ACCEPTED
        // (lazy-imported into state.tabs), since wcore-direct paths
        // can create tabs the reducer hasn't seen. Pre-checks live
        // in the saga / RPC layer (against SQLite). The reducer
        // here only validates that both workspaces exist.
        let events = update(
            &mut state,
            Command::MoveTab {
                tab_id: "ghost-tab".into(),
                src_workspace_id: src.clone(),
                dst_workspace_id: dst,
                dst_index: 0,
            },
            &ctx(4),
        );
        assert!(
            matches!(&events[0], Event::TabMoved { .. }),
            "unknown tab should be lazy-imported, got {:?}",
            events.first()
        );
    }

    /// codex P1 round-2 #621: handle_move_tab tolerates a tab whose
    /// reducer-state workspace_id mismatches `src_workspace_id`. The
    /// reducer's view can lag SQLite during the migration window
    /// (wcore-direct paths create/move tabs without dispatching),
    /// so a strict check would reject valid moves. The saga/RPC
    /// reads SQLite (the source of truth) for the membership check;
    /// the reducer here just performs the move.
    #[test]
    fn move_tab_tolerates_workspace_id_mismatch_during_migration() {
        let mut state = State::default();
        let real_src = create_workspace(&mut state, "src");
        let dst = create_workspace(&mut state, "dst");
        let t1 = create_tab(&mut state, &real_src, "t1");
        let other = create_workspace(&mut state, "other");
        let _ = create_tab(&mut state, &other, "filler");
        // Move with src_workspace_id = "other" even though the tab
        // technically belongs to real_src per reducer state.
        let events = update(
            &mut state,
            Command::MoveTab {
                tab_id: t1.clone(),
                src_workspace_id: other,
                dst_workspace_id: dst.clone(),
                dst_index: 0,
            },
            &ctx(2),
        );
        assert!(matches!(&events[0], Event::TabMoved { .. }));
        // Tab's workspace_id should now point at dst.
        assert_eq!(state.tabs[&t1].workspace_id, dst);
    }

    /// codex P1 round-2 #621: lazy-import populates state.tabs for
    /// a tab the reducer hadn't seen. After the move, state.tabs
    /// has an entry with workspace_id = dst.
    #[test]
    fn move_tab_lazy_imports_unknown_tab() {
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
                dst_workspace_id: dst.clone(),
                dst_index: 0,
            },
            &ctx(2),
        );
        assert!(matches!(&events[0], Event::TabMoved { .. }));
        let tab = state
            .tabs
            .get(&unknown_id)
            .expect("unknown tab should be lazy-imported");
        assert_eq!(tab.workspace_id, dst);
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
}
