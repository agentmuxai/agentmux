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
    // `cascaded_block_ids` rides the WorkspaceDeleted event below so the
    // host can tear down any browser-pane renderer whose block was never
    // live/loaded in a window (issue #2218, B.4) — same rationale as
    // TabDeleted's `block_ids` (reducer/tab.rs::handle_delete_tab).
    let mut cascaded_block_ids: Vec<String> = Vec::new();
    for tab_id in &removed.tab_ids {
        if let Some(tab) = state.tabs.remove(tab_id) {
            cascaded_block_ids.extend(tab.block_ids.iter().cloned());
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
        block_ids: cascaded_block_ids,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reducer::test_support::*;
    use crate::reducer::update;
    use agentmux_common::ipc::Command;

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

    #[test]
    fn delete_workspace_emits_cascaded_block_ids_across_all_tabs() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "w");
        let tab1_id = create_tab(&mut state, &ws_id, "t1");
        let tab2_id = create_tab(&mut state, &ws_id, "t2");
        let mut expected = Vec::new();
        for (i, tab_id) in [&tab1_id, &tab2_id].into_iter().enumerate() {
            for j in 0..2 {
                let block_id = match &update(
                    &mut state,
                    Command::CreateBlock { tab_id: tab_id.clone(), meta: serde_json::Value::Null },
                    &ctx(10 + (i * 2 + j) as u64),
                )[0] {
                    Event::BlockCreated { block_id, .. } => block_id.clone(),
                    other => panic!("expected BlockCreated, got {:?}", other),
                };
                expected.push(block_id);
            }
        }
        assert_eq!(expected.len(), 4);
        let events = update(
            &mut state,
            Command::DeleteWorkspace { workspace_id: ws_id.clone(), force: false },
            &ctx(20),
        );
        match &events[0] {
            Event::WorkspaceDeleted { block_ids, .. } => {
                assert_eq!(block_ids.len(), 4);
                for b in &expected {
                    assert!(block_ids.contains(b));
                }
            }
            other => panic!("expected WorkspaceDeleted, got {:?}", other),
        }
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
}
