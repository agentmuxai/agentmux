// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use agentmux_common::ipc::{ErrorCode, Event};

use crate::state::State;


use crate::state::WorkspaceRecord;

/// Phase E.2 — create a new workspace. Reducer assigns the OID
/// (UUID), inserts into canonical state, emits WorkspaceCreated.
/// NOT idempotent on retry: each invocation generates a fresh UUID
/// and inserts a new row, so a saga that double-fires CreateWorkspace
/// would create two distinct workspaces. Saga-side dedup (correlation
/// IDs / saga state machine) is responsible for at-most-once delivery
/// when sagas land in E.5+.
pub(super) fn handle_create_workspace(state: &mut State, name: String) -> Vec<Event> {
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
pub(super) fn handle_delete_workspace(
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

/// Phase E.5.3 — rename a workspace. Errors if missing; no-op if
/// the name is unchanged.
pub(super) fn handle_rename_workspace(state: &mut State, workspace_id: String, name: String) -> Vec<Event> {
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

/// Phase E.5.3 — pass-through validation + emit for workspace
/// meta updates. The reducer does NOT mutate meta in state (it
/// doesn't track meta in WorkspaceRecord); the persist subscriber
/// applies the patch directly to wstore. This keeps the reducer's
/// state shape unchanged while still routing every meta mutation
/// through the broadcast bus for observers.
pub(super) fn handle_update_workspace_meta(
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
