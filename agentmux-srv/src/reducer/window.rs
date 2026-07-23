// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use agentmux_common::ipc::{ErrorCode, Event};

use crate::state::State;


use crate::state::WindowRecord;

/// Phase E.5 — record a new window→workspace mapping. Validates
/// the parent workspace exists; otherwise emits `Event::Error`
/// (non-fatal). Idempotent on duplicate `window_id`: re-issuing
/// for the same window updates the workspace pointer if it
/// changed, or no-ops if identical.
pub(super) fn handle_create_window(
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
pub(super) fn handle_close_window_internal(state: &mut State, window_id: String) -> Vec<Event> {
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
pub(super) fn handle_switch_workspace(
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

/// Phase E.5.x (issue #855) — apply a meta-patch to a window. Pass-
/// through to `Event::WindowMetaUpdated`; the persist subscriber
/// performs the merge against wstore. Same shape as
/// `handle_update_workspace_meta` — reducer state does NOT track
/// window meta, the migration property is "every mutation goes
/// through the reducer's broadcast bus" so the WaveObjUpdate bridge
/// can fan out to the frontend.
///
/// The validation is best-effort: reducer state's `windows` map only
/// holds windows for which a workspace mapping has been registered
/// (via `handle_create_window`). Windows created via wcore-direct
/// paths (legacy bootstrap) won't appear there but still exist in
/// wstore. So we don't error on missing — the persist subscriber's
/// `apply_window_meta_updated` will silently no-op if the window
/// genuinely doesn't exist in wstore either, matching the
/// idempotency contract for the bridge to broadcast (or skip).
pub(super) fn handle_update_window_meta(
    state: &mut State,
    window_id: String,
    meta_patch: serde_json::Value,
) -> Vec<Event> {
    let v = state.bump_version();
    vec![Event::WindowMetaUpdated {
        window_id,
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
}
