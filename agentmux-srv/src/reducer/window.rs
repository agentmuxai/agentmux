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
/// Validates the window exists, matching its three sibling meta arms
/// (`handle_update_workspace_meta` / `handle_update_tab_meta` /
/// `handle_update_block_meta`) — this arm used to be the sole outlier
/// with no guard, which made `POST /api/v1/window/name` report success
/// for well-formed-but-nonexistent window ids (the persist subscriber's
/// `apply_window_meta_updated` silently no-ops on a wstore miss, so
/// nothing downstream caught it either). The guard is safe because
/// `state.windows` reliably mirrors real windows: every runtime
/// creation goes through `handle_create_window`, and
/// `persist::bootstrap_state_from_wstore` hydrates pre-existing windows
/// (including the wcore-seeded first-launch window, created before
/// hydration runs) at startup. The old "wcore-direct paths won't appear
/// here" caveat this comment used to carry predates that hydration.
/// SPEC_WINDOW_NAME_API_HARDENING_2026_08_08.md §3.1.
pub(super) fn handle_update_window_meta(
    state: &mut State,
    window_id: String,
    meta_patch: serde_json::Value,
) -> Vec<Event> {
    if !state.windows.contains_key(&window_id) {
        let v = state.bump_version();
        return vec![Event::Error {
            code: ErrorCode::InvalidCommand,
            message: format!("UpdateWindowMeta: window not found: {}", window_id),
            fatal: false,
            version: v,
        }];
    }
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
    fn update_window_meta_errors_on_unknown_window() {
        // Regression for SPEC_WINDOW_NAME_API_HARDENING_2026_08_08.md §2.1:
        // this arm used to emit WindowMetaUpdated unconditionally, so a
        // well-formed-but-nonexistent window id sailed through to a silent
        // persist no-op and POST /api/v1/window/name reported success.
        let mut state = State::default();
        let events = update(
            &mut state,
            Command::UpdateWindowMeta {
                window_id: "00000000-dead-beef-0000-000000000000".into(),
                meta_patch: serde_json::json!({ "window:displayname": "phantom" }),
            },
            &ctx(1),
        );
        assert!(
            matches!(&events[0], Event::Error { message, .. } if message.contains("window not found")),
            "expected window-not-found error, got {:?}",
            events
        );
    }

    #[test]
    fn update_window_meta_emits_event_for_known_window() {
        let mut state = State::default();
        let ws_id = create_workspace(&mut state, "a");
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
            Command::UpdateWindowMeta {
                window_id: "win-1".into(),
                meta_patch: serde_json::json!({ "window:displayname": "named" }),
            },
            &ctx(3),
        );
        assert!(
            matches!(&events[0], Event::WindowMetaUpdated { window_id, .. } if window_id == "win-1"),
            "expected WindowMetaUpdated, got {:?}",
            events
        );
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
