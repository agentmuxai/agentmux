// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `window` service handlers — window mutation RPCs (`SwitchWorkspace`,
//! `SetWindowPosAndSize`, `SetWindowOpacity`, `SetWindowTopology`). Split
//! out of `window.rs`; see that file's dispatcher for the full method list.

use crate::backend::obj::*;
use crate::backend::service::{self, WebCallType, WebReturnType};

use super::super::AppState;
use super::reducer_helpers::{dispatch_to_reducer, publish_events};

// Phase E.5.8 — SwitchWorkspace migrated to single-step
// reducer dispatch. The reducer validates window + workspace
// both exist + emits SrvWindowWorkspaceChanged; subscriber
// writes Window.workspaceid in SQLite.
pub(crate) async fn handle_switch_workspace(state: &AppState, call: &WebCallType) -> WebReturnType {
    let store = &state.wstore;
    let args = &call.args;
    let window_id: String = match service::get_arg(args, 0) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    let ws_id: String = match service::get_arg(args, 1) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    let events = dispatch_to_reducer(
        state,
        agentmux_common::ipc::Command::SwitchWorkspace {
            window_id: window_id.clone(),
            workspace_id: ws_id.clone(),
        },
    )
    .await;
    if let Some(err_msg) = events.iter().find_map(|e| match e {
        agentmux_common::ipc::Event::Error { message, .. } => Some(message.clone()),
        _ => None,
    }) {
        return WebReturnType::error(err_msg);
    }
    for ev in &events {
        if let Err(e) = crate::persist_subscriber::apply_event_to_wstore(ev, store) {
            return WebReturnType::error(format!(
                "SwitchWorkspace: SQLite write failed: {}",
                e
            ));
        }
    }
    publish_events(state, &events);
    WebReturnType::success_empty()
}

pub(crate) async fn handle_set_window_pos_and_size(state: &AppState, call: &WebCallType) -> WebReturnType {
    let store = &state.wstore;
    let args = &call.args;
    let window_id: String = match service::get_arg(args, 0) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    let pos: Option<Point> = service::get_optional_arg(args, 1).unwrap_or(None);
    let size: Option<WinSize> = service::get_optional_arg(args, 2).unwrap_or(None);
    match store.must_get::<Window>(&window_id) {
        Ok(mut win) => {
            if let Some(p) = pos {
                win.pos = p;
            }
            if let Some(s) = size {
                win.winsize = s;
            }
            match store.update(&mut win) {
                Ok(_) => WebReturnType::success_empty(),
                Err(e) => WebReturnType::error(e.to_string()),
            }
        }
        Err(e) => WebReturnType::error(e.to_string()),
    }
}

// SPEC_PILLAR1_STEP2 (Slice A, Phase 1) — durable mirror of the
// host's per-window opacity so a crashed/restarted host can
// restore it (Phase 2, not yet wired: nothing calls this arm
// today). Direct store read-modify-write, same shape as
// SetWindowPosAndSize just above — window opacity isn't
// reducer-tracked state (see `state::WindowRecord`), so there's
// no split-brain risk to route around the way #864's layout
// tree had. `opacity: None` clears back to fully-opaque/unset —
// that's a real, destructive state change (unlike pos/size's
// `None` in SetWindowPosAndSize, which just means "skip this
// field"), so unlike that handler a malformed (non-null,
// non-f32) argument must surface as an error, not get silently
// treated as an explicit clear. (reagent P1.)
pub(crate) async fn handle_set_window_opacity(state: &AppState, call: &WebCallType) -> WebReturnType {
    let store = &state.wstore;
    let args = &call.args;
    let window_id: String = match service::get_arg(args, 0) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    let opacity: Option<f32> = match service::get_optional_arg(args, 1) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    if let Some(o) = opacity {
        if !(0.0..=1.0).contains(&o) {
            return WebReturnType::error(format!(
                "SetWindowOpacity: opacity must be in 0.0..=1.0, got {o}"
            ));
        }
    }
    match store.must_get::<Window>(&window_id) {
        Ok(mut win) => {
            win.opacity = opacity;
            match store.update(&mut win) {
                Ok(_) => WebReturnType::success_empty(),
                Err(e) => WebReturnType::error(e.to_string()),
            }
        }
        Err(e) => WebReturnType::error(e.to_string()),
    }
}

// SPEC_PILLAR1_STEP3 — durable mirror of a window's kind + parent
// linkage, so a future reproject can tell which native-window
// creation path to drive for each window in `Client.windowids`.
// Direct store read-modify-write, same shape as SetWindowOpacity
// just above — kind/parent aren't reducer-tracked state (see
// `state::WindowRecord`), so there's no split-brain risk to route
// around.
pub(crate) async fn handle_set_window_topology(state: &AppState, call: &WebCallType) -> WebReturnType {
    let store = &state.wstore;
    let args = &call.args;
    let window_id: String = match service::get_arg(args, 0) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    let kind: Option<String> = match service::get_optional_arg(args, 1) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    let parent_window_id: Option<String> = match service::get_optional_arg(args, 2) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    if let Some(k) = &kind {
        if k != "full_instance" && k != "subwindow" {
            return WebReturnType::error(format!(
                "SetWindowTopology: kind must be 'full_instance' or 'subwindow', got '{k}'"
            ));
        }
        if k == "subwindow" && parent_window_id.is_none() {
            return WebReturnType::error(
                "SetWindowTopology: kind='subwindow' requires a non-null parent_window_id"
                    .to_string(),
            );
        }
    }
    // reagent P1: parent_window_id must only ever be set alongside
    // kind='subwindow' — the doc comment's invariant ("None for a
    // full_instance ... or an unset/legacy row") was previously only
    // enforced in one direction (subwindow requires a parent), not
    // the reverse (full_instance/None must NOT carry a parent).
    if parent_window_id.is_some() && kind.as_deref() != Some("subwindow") {
        return WebReturnType::error(
            "SetWindowTopology: parent_window_id must only be set when kind='subwindow'"
                .to_string(),
        );
    }
    // reagent P2: a subwindow's parent must reference a real,
    // DIFFERENT window — otherwise a dangling or self-referential
    // parent_window_id silently persists undetected.
    if let Some(parent_id) = &parent_window_id {
        if parent_id == &window_id {
            return WebReturnType::error(
                "SetWindowTopology: parent_window_id must not equal window_id".to_string(),
            );
        }
        if store.must_get::<Window>(parent_id).is_err() {
            return WebReturnType::error(format!(
                "SetWindowTopology: parent_window_id '{parent_id}' does not reference an existing window"
            ));
        }
    }
    match store.must_get::<Window>(&window_id) {
        Ok(mut win) => {
            win.kind = kind;
            win.parent_window_id = parent_window_id;
            match store.update(&mut win) {
                Ok(_) => WebReturnType::success_empty(),
                Err(e) => WebReturnType::error(e.to_string()),
            }
        }
        Err(e) => WebReturnType::error(e.to_string()),
    }
}

/// SPEC_PILLAR1_STEP2 Slice A Phase 1 — `SetWindowOpacity` unit tests.
#[cfg(test)]
mod set_window_opacity_tests {
    use super::super::window::handle_window_service;
    use crate::backend::obj::Window;
    use crate::backend::service::WebCallType;
    use crate::server::tests::test_state;

    fn call(window_id: &str, opacity: Option<f32>) -> WebCallType {
        WebCallType {
            service: "window".to_string(),
            method: "SetWindowOpacity".to_string(),
            uicontext: None,
            args: vec![
                serde_json::Value::String(window_id.to_string()),
                opacity
                    .map(|o| serde_json::json!(o))
                    .unwrap_or(serde_json::Value::Null),
            ],
        }
    }

    async fn seeded_window_id(state: &crate::server::AppState) -> String {
        let ret = handle_window_service(
            state,
            &WebCallType {
                service: "window".to_string(),
                method: "CreateWindow".to_string(),
                uicontext: None,
                args: vec![
                    serde_json::Value::Null,
                    serde_json::Value::String(String::new()),
                ],
            },
        )
        .await;
        assert!(ret.success, "CreateWindow failed: {:?}", ret.error);
        ret.data
            .expect("CreateWindow returns the Window")
            .get("oid")
            .and_then(|v| v.as_str())
            .expect("window has an oid")
            .to_string()
    }

    #[tokio::test]
    async fn sets_and_persists_opacity() {
        let state = test_state();
        let window_id = seeded_window_id(&state).await;

        let ret = handle_window_service(&state, &call(&window_id, Some(0.85))).await;
        assert!(ret.success, "SetWindowOpacity failed: {:?}", ret.error);

        let win = state.wstore.must_get::<Window>(&window_id).unwrap();
        assert_eq!(win.opacity, Some(0.85));
    }

    #[tokio::test]
    async fn null_opacity_clears_it() {
        let state = test_state();
        let window_id = seeded_window_id(&state).await;

        handle_window_service(&state, &call(&window_id, Some(0.5)))
            .await;
        let ret = handle_window_service(&state, &call(&window_id, None)).await;
        assert!(ret.success, "SetWindowOpacity (clear) failed: {:?}", ret.error);

        let win = state.wstore.must_get::<Window>(&window_id).unwrap();
        assert_eq!(win.opacity, None, "None must clear back to unset/opaque");
    }

    #[tokio::test]
    async fn rejects_out_of_range_opacity() {
        let state = test_state();
        let window_id = seeded_window_id(&state).await;

        for bad in [-0.1_f32, 1.1_f32] {
            let ret = handle_window_service(&state, &call(&window_id, Some(bad))).await;
            assert!(!ret.success, "opacity {bad} must be rejected");
        }
        // Rejected — window's opacity must stay unset.
        let win = state.wstore.must_get::<Window>(&window_id).unwrap();
        assert_eq!(win.opacity, None);
    }

    #[tokio::test]
    async fn errors_on_unknown_window() {
        let state = test_state();
        let ret = handle_window_service(&state, &call("ghost-window", Some(0.5))).await;
        assert!(!ret.success, "unknown window must error");
    }

    /// reagent P1: a malformed (non-null, non-f32) opacity argument must
    /// surface as an error, not be silently swallowed into an explicit
    /// clear — `None` here means "wipe the persisted opacity," a real
    /// state change, so a parse failure must not be conflated with it.
    #[tokio::test]
    async fn malformed_opacity_errors_and_does_not_clear() {
        let state = test_state();
        let window_id = seeded_window_id(&state).await;

        let set_ret = handle_window_service(&state, &call(&window_id, Some(0.6))).await;
        assert!(set_ret.success);

        let bad_call = WebCallType {
            service: "window".to_string(),
            method: "SetWindowOpacity".to_string(),
            uicontext: None,
            args: vec![
                serde_json::Value::String(window_id.clone()),
                serde_json::Value::String("not-a-number".to_string()),
            ],
        };
        let ret = handle_window_service(&state, &bad_call).await;
        assert!(!ret.success, "malformed opacity must error, not be treated as clear");

        let win = state.wstore.must_get::<Window>(&window_id).unwrap();
        assert_eq!(
            win.opacity,
            Some(0.6),
            "a rejected malformed argument must not wipe the previously-set opacity"
        );
    }

    #[tokio::test]
    async fn boundary_values_are_accepted() {
        let state = test_state();
        let window_id = seeded_window_id(&state).await;

        for boundary in [0.0_f32, 1.0_f32] {
            let ret = handle_window_service(&state, &call(&window_id, Some(boundary))).await;
            assert!(ret.success, "boundary opacity {boundary} must be accepted");
            let win = state.wstore.must_get::<Window>(&window_id).unwrap();
            assert_eq!(win.opacity, Some(boundary));
        }
    }
}

/// SPEC_PILLAR1_STEP3 — `SetWindowTopology` unit tests.
#[cfg(test)]
mod set_window_topology_tests {
    use super::super::window::handle_window_service;
    use crate::backend::obj::Window;
    use crate::backend::service::WebCallType;
    use crate::server::tests::test_state;

    fn call(window_id: &str, kind: Option<&str>, parent_window_id: Option<&str>) -> WebCallType {
        WebCallType {
            service: "window".to_string(),
            method: "SetWindowTopology".to_string(),
            uicontext: None,
            args: vec![
                serde_json::Value::String(window_id.to_string()),
                kind.map(|k| serde_json::json!(k)).unwrap_or(serde_json::Value::Null),
                parent_window_id
                    .map(|p| serde_json::json!(p))
                    .unwrap_or(serde_json::Value::Null),
            ],
        }
    }

    async fn seeded_window_id(state: &crate::server::AppState) -> String {
        let ret = handle_window_service(
            state,
            &WebCallType {
                service: "window".to_string(),
                method: "CreateWindow".to_string(),
                uicontext: None,
                args: vec![
                    serde_json::Value::Null,
                    serde_json::Value::String(String::new()),
                ],
            },
        )
        .await;
        assert!(ret.success, "CreateWindow failed: {:?}", ret.error);
        ret.data
            .expect("CreateWindow returns the Window")
            .get("oid")
            .and_then(|v| v.as_str())
            .expect("window has an oid")
            .to_string()
    }

    #[tokio::test]
    async fn sets_and_persists_full_instance() {
        let state = test_state();
        let window_id = seeded_window_id(&state).await;

        let ret = handle_window_service(&state, &call(&window_id, Some("full_instance"), None)).await;
        assert!(ret.success, "SetWindowTopology failed: {:?}", ret.error);

        let win = state.wstore.must_get::<Window>(&window_id).unwrap();
        assert_eq!(win.kind, Some("full_instance".to_string()));
        assert_eq!(win.parent_window_id, None);
    }

    #[tokio::test]
    async fn sets_and_persists_subwindow_with_parent() {
        let state = test_state();
        let parent_id = seeded_window_id(&state).await;
        let window_id = seeded_window_id(&state).await;

        let ret =
            handle_window_service(&state, &call(&window_id, Some("subwindow"), Some(&parent_id))).await;
        assert!(ret.success, "SetWindowTopology failed: {:?}", ret.error);

        let win = state.wstore.must_get::<Window>(&window_id).unwrap();
        assert_eq!(win.kind, Some("subwindow".to_string()));
        assert_eq!(win.parent_window_id, Some(parent_id));
    }

    #[tokio::test]
    async fn rejects_subwindow_without_parent() {
        let state = test_state();
        let window_id = seeded_window_id(&state).await;

        let ret = handle_window_service(&state, &call(&window_id, Some("subwindow"), None)).await;
        assert!(!ret.success, "subwindow with no parent_window_id must be rejected");

        let win = state.wstore.must_get::<Window>(&window_id).unwrap();
        assert_eq!(win.kind, None, "rejected write must not persist");
    }

    #[tokio::test]
    async fn rejects_unknown_kind() {
        let state = test_state();
        let window_id = seeded_window_id(&state).await;

        let ret = handle_window_service(&state, &call(&window_id, Some("floating"), None)).await;
        assert!(!ret.success, "unknown kind must be rejected");
    }

    /// reagent P1: a full_instance (or unset/None kind) must never carry a
    /// parent_window_id — only the reverse (subwindow requires a parent)
    /// was originally enforced.
    #[tokio::test]
    async fn rejects_full_instance_with_parent() {
        let state = test_state();
        let parent_id = seeded_window_id(&state).await;
        let window_id = seeded_window_id(&state).await;

        let ret =
            handle_window_service(&state, &call(&window_id, Some("full_instance"), Some(&parent_id)))
                .await;
        assert!(!ret.success, "full_instance with a parent_window_id must be rejected");

        let win = state.wstore.must_get::<Window>(&window_id).unwrap();
        assert_eq!(win.kind, None, "rejected write must not persist");
    }

    /// reagent P1: same rule when kind is omitted entirely (None) but a
    /// parent_window_id is supplied anyway.
    #[tokio::test]
    async fn rejects_none_kind_with_parent() {
        let state = test_state();
        let parent_id = seeded_window_id(&state).await;
        let window_id = seeded_window_id(&state).await;

        let ret = handle_window_service(&state, &call(&window_id, None, Some(&parent_id))).await;
        assert!(!ret.success, "kind=None with a parent_window_id must be rejected");
    }

    /// reagent P2: parent_window_id must reference a real window row, not a
    /// dangling id.
    #[tokio::test]
    async fn rejects_dangling_parent_window_id() {
        let state = test_state();
        let window_id = seeded_window_id(&state).await;

        let ret =
            handle_window_service(&state, &call(&window_id, Some("subwindow"), Some("ghost-parent")))
                .await;
        assert!(!ret.success, "a parent_window_id that doesn't exist must be rejected");

        let win = state.wstore.must_get::<Window>(&window_id).unwrap();
        assert_eq!(win.kind, None, "rejected write must not persist");
    }

    /// reagent P2: a window cannot be its own parent.
    #[tokio::test]
    async fn rejects_self_referential_parent() {
        let state = test_state();
        let window_id = seeded_window_id(&state).await;

        let ret =
            handle_window_service(&state, &call(&window_id, Some("subwindow"), Some(&window_id))).await;
        assert!(!ret.success, "a window referencing itself as parent must be rejected");
    }

    #[tokio::test]
    async fn null_kind_clears_it() {
        let state = test_state();
        let window_id = seeded_window_id(&state).await;

        handle_window_service(&state, &call(&window_id, Some("full_instance"), None)).await;
        let ret = handle_window_service(&state, &call(&window_id, None, None)).await;
        assert!(ret.success, "SetWindowTopology (clear) failed: {:?}", ret.error);

        let win = state.wstore.must_get::<Window>(&window_id).unwrap();
        assert_eq!(win.kind, None);
        assert_eq!(win.parent_window_id, None);
    }

    #[tokio::test]
    async fn errors_on_unknown_window() {
        let state = test_state();
        let ret = handle_window_service(&state, &call("ghost-window", Some("full_instance"), None)).await;
        assert!(!ret.success, "unknown window must error");
    }
}
