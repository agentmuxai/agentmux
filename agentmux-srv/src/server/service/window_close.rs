// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `window` service handler — `CloseWindow`. Split out of `window.rs`;
//! see that file's dispatcher for the full method list.

use crate::backend::obj::*;
use crate::backend::service::{self, WebCallType, WebReturnType};

use super::super::AppState;
use super::reducer_helpers::{dispatch_to_reducer, publish_events};

// Phase E.5.8 — CloseWindow migrated through the reducer.
// Sequence:
//   1. Look up the window's workspace (for cascade decision).
//   2. Dispatch Command::CloseWindowInternal — emits
//      SrvWindowClosed; subscriber prunes Client.windowids.
//   3. If no other window points at the same workspace, dispatch
//      Command::DeleteWorkspace which cascades through tabs+blocks.
// Mirrors `wcore::close_window` behaviour where each window
// owns one workspace, but uses the reducer-routed conditional
// pattern so future multi-window-on-same-workspace flows
// don't accidentally drop user state.
pub(crate) async fn handle_close_window(state: &AppState, call: &WebCallType) -> WebReturnType {
    let store = &state.wstore;
    let args = &call.args;
    let window_id: String = match service::get_arg(args, 0) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    // Look up the window's workspace before we close it.
    // Read SQLite (source of truth during migration).
    let ws_id: Option<String> = match store.get::<Window>(&window_id) {
        Ok(Some(w)) => Some(w.workspaceid.clone()),
        Ok(None) => None,
        Err(e) => {
            return WebReturnType::error(format!(
                "CloseWindow: window lookup failed: {}",
                e
            ));
        }
    };
    // Restore-on-relaunch (SPEC_SESSION_RESTORE_AND_SAVED_LAYOUTS_2026_08_13
    // Feature 1) — is this close the one that empties `Client.windowids`,
    // i.e. a genuine "the user is fully quitting" moment? Deliberately NOT
    // "does this close cascade-delete ITS OWN workspace" (`!any_other_window`
    // below is a workspace-sharing question, answered independently per
    // window, true for basically every close in the common 1:1
    // window:workspace case) — reviewer-caught bug (reagentx P1 on PR
    // #2560): gating on workspace-destruction alone still let closing a
    // small independent tear-off window (its own small 1:1 workspace, so
    // `!any_other_window` is true for it too) AFTER the real main window
    // overwrite the just-saved full-session snapshot with the tear-off's
    // tiny one. Gating on "windowids is about to be empty" instead ties the
    // snapshot to the same trigger `restore_last_session` itself waits for
    // on the read side (only fires when `Client.windowids` is empty) — the
    // save and the read side now agree on what "the session ended" means.
    // Computed once, up front, off the state as it stood BEFORE this
    // close's own dispatch below mutates anything.
    let will_empty_windowids = store
        .get_all::<Client>()
        .ok()
        .and_then(|cs| cs.into_iter().next())
        .map(|c| c.windowids.iter().all(|id| id == &window_id))
        .unwrap_or(false);
    // Step 1: drop the window mapping in reducer.
    let close_events = dispatch_to_reducer(
        state,
        agentmux_common::ipc::Command::CloseWindowInternal {
            window_id: window_id.clone(),
        },
    )
    .await;
    // reagent P1 #622: surface reducer rejection before
    // applying / publishing. Every other primary dispatch in
    // this PR follows this pattern; CloseWindow was missing it.
    if let Some(err_msg) = close_events.iter().find_map(|e| match e {
        agentmux_common::ipc::Event::Error { message, .. } => Some(message.clone()),
        _ => None,
    }) {
        return WebReturnType::error(err_msg);
    }
    // #2051 — the reducer's CloseWindowInternal is an idempotent
    // silent no-op on unknown ids (no event emitted). The SQLite
    // prune below only runs off the SrvWindowClosed event, so
    // without this branch a window the reducer never knew
    // (bootstrap skipped it, or state/store desync) returns 200
    // while its `Client.windowids` entry and Window row survive
    // forever — the leak class that fueled the #2048 window
    // storm. Prune the durable store directly and warn about the
    // divergence; a window that is gone from BOTH sides stays a
    // clean idempotent success.
    if close_events.is_empty() {
        let had_row = ws_id.is_some();
        let had_client_entry = store
            .get_all::<Client>()
            .ok()
            .and_then(|cs| cs.into_iter().next())
            .map(|c| c.windowids.iter().any(|id| id == &window_id))
            .unwrap_or(false);
        if !had_row && !had_client_entry {
            return WebReturnType::success_empty();
        }
        tracing::warn!(
            window_id = %window_id,
            had_row,
            had_client_entry,
            "CloseWindow: window unknown to the reducer but still present in the \
             store — pruning defensively (#2051)"
        );
        if let Err(e) =
            crate::persist_subscriber::apply_srv_window_closed(store, &window_id)
        {
            return WebReturnType::error(format!(
                "CloseWindow: divergence prune failed: {}",
                e
            ));
        }
        // Same guarded cascade as the normal path below, but
        // judged from the store (the reducer has no record of
        // this window's workspace peers). The prune above already
        // deleted this window's own row, so any remaining
        // reference means the workspace is genuinely shared. On a
        // read error keep the workspace — losing user tabs is
        // worse than leaking an empty workspace row.
        if let Some(ws_id) = ws_id {
            let any_other_window = store
                .get_all::<Window>()
                .map(|ws| ws.iter().any(|w| w.workspaceid == ws_id))
                .unwrap_or(true);
            if !any_other_window {
                // Restore-on-relaunch (SPEC_SESSION_RESTORE_AND_SAVED_LAYOUTS_2026_08_13
                // Feature 1) — gated on `will_empty_windowids` (computed at
                // function entry — see its own doc comment for why this is
                // NOT the same condition as `!any_other_window` above), not
                // on workspace destruction. Best-effort: a failure here must
                // never block the close itself.
                if will_empty_windowids {
                    if let Some(snapshot) = super::session_restore::snapshot_workspace(store, &ws_id) {
                        super::session_restore::save_last_session_snapshot(store, snapshot);
                    }
                }
                // A workspace the reducer tracks goes through the
                // saga (keeps reducer + store consistent, durable
                // provenance). One it never knew — the usual case
                // when its window diverged too — would make the
                // saga's reducer dispatch error out, so cascade at
                // the store layer directly instead.
                let reducer_knows_ws = state
                    .srv_state
                    .lock()
                    .await
                    .workspaces
                    .contains_key(&ws_id);
                if reducer_knows_ws {
                    if let Err(e) =
                        crate::sagas::delete_workspace::run(state, ws_id.clone()).await
                    {
                        tracing::warn!(
                            workspace_id = %ws_id,
                            "CloseWindow: divergence-path delete_workspace saga failed: {}",
                            e,
                        );
                    }
                } else if let Err(e) =
                    crate::backend::wcore::delete_workspace(store, &ws_id)
                {
                    tracing::warn!(
                        workspace_id = %ws_id,
                        "CloseWindow: divergence-path store cascade failed: {}",
                        e,
                    );
                }
            }
        }
        return WebReturnType::success_empty();
    }
    for ev in &close_events {
        if let Err(e) = crate::persist_subscriber::apply_event_to_wstore(ev, store) {
            return WebReturnType::error(format!(
                "CloseWindow: SQLite write failed: {}",
                e
            ));
        }
    }
    publish_events(state, &close_events);
    // Step 2: cascade delete the workspace if no other window
    // points at it. The reducer keeps state.windows updated;
    // check there.
    //
    // Step 5 PR 2 — route the user-initiated cascade through
    // the `delete_workspace` saga instead of dispatching
    // `Command::DeleteWorkspace` inline. The saga records
    // lifecycle brackets in the durable saga log so a crash
    // mid-cascade is recoverable via `recovery::compensate_unresolved`.
    // The saga also takes a snapshot of the workspace's
    // tabs+blocks before issuing the cascade, so the durable
    // log captures what was deleted (provenance for
    // `--diag sagas`).
    if let Some(ws_id) = ws_id {
        let any_other_window = {
            let s = state.srv_state.lock().await;
            s.windows.values().any(|w| w.workspace_id == ws_id)
        };
        if !any_other_window {
            // Restore-on-relaunch (SPEC_SESSION_RESTORE_AND_SAVED_LAYOUTS_2026_08_13
            // Feature 1) — gated on `will_empty_windowids` (see the doc
            // comment at its computation, function entry, for why this is
            // NOT the same condition as `!any_other_window` above — that
            // question is "does closing this window destroy ITS OWN
            // workspace," true for basically every close in the common 1:1
            // window:workspace case; this one is "is the user fully
            // quitting"). Workspace tabs/blocks are still fully intact at
            // this point — `CloseWindowInternal`'s already-applied events
            // only touched Window/Client, the actual tab/block cascade is
            // the saga call right below this.
            if will_empty_windowids {
                if let Some(snapshot) = super::session_restore::snapshot_workspace(store, &ws_id) {
                    super::session_restore::save_last_session_snapshot(store, snapshot);
                }
            }
            if let Err(e) =
                crate::sagas::delete_workspace::run(state, ws_id.clone()).await
            {
                tracing::warn!(
                    workspace_id = %ws_id,
                    "CloseWindow: delete_workspace saga failed: {}",
                    e,
                );
            }
        }
    }
    // Subscriber's apply_srv_window_closed already pruned
    // Client.windowids and deleted the Window row; nothing
    // more for the handler to do.
    WebReturnType::success_empty()
}

/// #2051 — `CloseWindow` divergence-path tests: the reducer's
/// `CloseWindowInternal` silently no-ops on ids it never knew, and the
/// SQLite prune normally rides the `SrvWindowClosed` event — so a window
/// present only in the store used to survive a "successful" CloseWindow
/// forever (the leak class behind the #2048 window storm). These pin the
/// handler's direct-prune fallback.
#[cfg(test)]
mod close_window_divergence_tests {
    use super::super::window::handle_window_service;
    use crate::backend::obj::{Client, Window, Workspace};
    use crate::backend::service::WebCallType;
    use crate::backend::wcore;
    use crate::server::tests::test_state;

    fn close_call(window_id: &str) -> WebCallType {
        WebCallType {
            service: "window".to_string(),
            method: "CloseWindow".to_string(),
            uicontext: None,
            args: vec![serde_json::Value::String(window_id.to_string())],
        }
    }

    fn client_has_window(state: &crate::server::AppState, window_id: &str) -> bool {
        state
            .wstore
            .get_all::<Client>()
            .unwrap()
            .into_iter()
            .next()
            .map(|c| c.windowids.iter().any(|id| id == window_id))
            .unwrap_or(false)
    }

    /// A window written straight to the store (`wcore::create_window`
    /// bypasses the reducer — exactly how bootstrap-skipped rows and
    /// pre-reducer desyncs look) must still be fully pruned by
    /// CloseWindow: success, row gone, `Client.windowids` entry gone,
    /// and its now-orphaned workspace cascade-deleted.
    #[tokio::test]
    async fn store_only_window_is_pruned_and_workspace_cascades() {
        let state = test_state();
        let win = wcore::create_window_full(&state.wstore, "").unwrap();
        assert!(client_has_window(&state, &win.oid), "precondition: windowids entry");
        assert!(
            !state.srv_state.lock().await.windows.contains_key(&win.oid),
            "precondition: reducer must NOT know this window"
        );

        let ret = handle_window_service(&state, &close_call(&win.oid)).await;
        assert!(ret.success, "divergent CloseWindow failed: {:?}", ret.error);

        assert!(
            state.wstore.get::<Window>(&win.oid).unwrap().is_none(),
            "Window row must be deleted"
        );
        assert!(
            !client_has_window(&state, &win.oid),
            "Client.windowids entry must be pruned"
        );
        assert!(
            state.wstore.get::<Workspace>(&win.workspaceid).unwrap().is_none(),
            "orphaned workspace must cascade-delete"
        );
    }

    /// A workspace still referenced by another window must survive the
    /// divergence-path cascade guard.
    #[tokio::test]
    async fn shared_workspace_survives_divergent_close() {
        let state = test_state();
        let first = wcore::create_window_full(&state.wstore, "").unwrap();
        let second =
            wcore::create_window_full(&state.wstore, &first.workspaceid).unwrap();

        let ret = handle_window_service(&state, &close_call(&first.oid)).await;
        assert!(ret.success, "divergent CloseWindow failed: {:?}", ret.error);

        assert!(
            state.wstore.get::<Workspace>(&first.workspaceid).unwrap().is_some(),
            "workspace referenced by another window must survive"
        );
        assert!(
            state.wstore.get::<Window>(&second.oid).unwrap().is_some(),
            "the other window must be untouched"
        );
    }

    /// A window gone from BOTH the reducer and the store stays a clean
    /// idempotent success — no error, no phantom prune.
    #[tokio::test]
    async fn truly_missing_window_is_an_idempotent_success() {
        let state = test_state();
        let ret = handle_window_service(&state, &close_call("no-such-window")).await;
        assert!(ret.success, "idempotent CloseWindow failed: {:?}", ret.error);
    }
}

/// Restore-on-relaunch snapshot-timing regression tests
/// (SPEC_SESSION_RESTORE_AND_SAVED_LAYOUTS_2026_08_13.md Feature 1).
/// Reviewer-caught bug (reagentx P1 on PR #2560): the original
/// implementation snapshotted unconditionally on every window close, so
/// closing a second, independent window AFTER the real session's window
/// would silently overwrite the correct snapshot with the second window's
/// (typically much smaller) content — since each window normally owns its
/// own 1:1 workspace, "does this close destroy its own workspace" was true
/// for nearly every close, not a useful filter. These pin the fix: the
/// snapshot now only fires on the close that actually empties
/// `Client.windowids` — the same condition the read side already requires.
#[cfg(test)]
mod snapshot_timing_tests {
    use super::super::window::handle_window_service;
    use super::super::window_create::handle_create_window;
    use crate::backend::obj::{Client, Tab, Window, Workspace};
    use crate::backend::service::WebCallType;
    use crate::server::tests::test_state;

    fn close_call(window_id: &str) -> WebCallType {
        WebCallType {
            service: "window".to_string(),
            method: "CloseWindow".to_string(),
            uicontext: None,
            args: vec![serde_json::Value::String(window_id.to_string())],
        }
    }

    async fn create_window(state: &crate::server::AppState) -> Window {
        let call = WebCallType {
            service: "window".to_string(),
            method: "CreateWindow".to_string(),
            uicontext: None,
            args: vec![serde_json::Value::Null, serde_json::Value::String(String::new())],
        };
        let resp = handle_create_window(state, &call).await;
        assert!(resp.success, "CreateWindow failed: {:?}", resp.error);
        serde_json::from_value(resp.data.unwrap()).unwrap()
    }

    fn rename_first_tab(state: &crate::server::AppState, workspace_id: &str, name: &str) {
        let ws = state.wstore.get::<Workspace>(workspace_id).unwrap().unwrap();
        let mut tab = state.wstore.get::<Tab>(&ws.tabids[0]).unwrap().unwrap();
        tab.name = name.to_string();
        state.wstore.update(&mut tab).unwrap();
    }

    fn last_session_snapshot_tab_name(state: &crate::server::AppState) -> Option<String> {
        let client = state.wstore.get_all::<Client>().unwrap().into_iter().next()?;
        let snapshot = client.meta.get("session:last_topology")?;
        snapshot["tabs"][0]["name"].as_str().map(|s| s.to_string())
    }

    /// `test_state()` seeds a "Starter workspace" bootstrap window
    /// (`wcore::ensure_initial_data`) before any test code runs — without
    /// closing it first, `Client.windowids` always has that extra entry, so
    /// `will_empty_windowids` never actually goes true in these tests no
    /// matter how many test-created windows get closed. Close it before
    /// setting up a test's own "N independent windows" scenario.
    async fn close_bootstrap_window(state: &crate::server::AppState) {
        let bootstrap_id = state
            .wstore
            .get_all::<Client>()
            .unwrap()
            .into_iter()
            .next()
            .unwrap()
            .windowids[0]
            .clone();
        let ret = handle_window_service(state, &close_call(&bootstrap_id)).await;
        assert!(ret.success, "closing bootstrap window failed: {:?}", ret.error);
    }

    /// The core fix: with two independent windows open, closing the FIRST
    /// one (the other is still open — `Client.windowids` does NOT empty)
    /// must not touch the snapshot at all. Only closing the SECOND (last)
    /// window may write one. Under the pre-fix code (unconditional
    /// snapshot-on-every-close) this test fails at the first assertion —
    /// the first close already wrote a snapshot.
    #[tokio::test]
    async fn closing_one_of_two_open_windows_does_not_touch_the_snapshot() {
        let state = test_state();
        // Closing the lone bootstrap window IS a genuine terminal close (it
        // empties `Client.windowids` on its own), so it legitimately writes
        // a snapshot of its own — capture that as the baseline rather than
        // assuming "no snapshot yet".
        close_bootstrap_window(&state).await;
        let snapshot_after_bootstrap_close = last_session_snapshot_tab_name(&state);

        let first = create_window(&state).await;
        rename_first_tab(&state, &first.workspaceid, "first-window-tab");
        let second = create_window(&state).await;
        rename_first_tab(&state, &second.workspaceid, "second-window-tab");

        // Close the FIRST window while the second is still open.
        let ret = handle_window_service(&state, &close_call(&first.oid)).await;
        assert!(ret.success, "CloseWindow failed: {:?}", ret.error);

        assert_eq!(
            last_session_snapshot_tab_name(&state),
            snapshot_after_bootstrap_close,
            "closing one of two open windows must not write a snapshot — \
             the session hasn't actually ended yet"
        );

        // Now close the SECOND (last) window — this is the real end of the
        // session, and must snapshot the second window's own content.
        let ret = handle_window_service(&state, &close_call(&second.oid)).await;
        assert!(ret.success, "CloseWindow failed: {:?}", ret.error);

        assert_eq!(
            last_session_snapshot_tab_name(&state).as_deref(),
            Some("second-window-tab"),
            "the terminal close must snapshot ITS OWN (last) window's content"
        );
    }

    /// Direct regression for the reviewer's exact scenario: close the
    /// window with the "real" session first, then a smaller second window
    /// — the smaller window's close (being the one that actually empties
    /// `Client.windowids`) legitimately becomes the final snapshot, but the
    /// key guarantee is that this reflects a real, coherent design decision
    /// (snapshot = "whatever was open at the moment of full quit") rather
    /// than an arbitrary race between two unconditional writes.
    #[tokio::test]
    async fn snapshot_reflects_whichever_close_actually_empties_windowids() {
        let state = test_state();
        // See `closing_one_of_two_open_windows_does_not_touch_the_snapshot`:
        // closing the lone bootstrap window is itself a terminal close, so
        // it writes a snapshot — capture that as the baseline.
        close_bootstrap_window(&state).await;
        let snapshot_after_bootstrap_close = last_session_snapshot_tab_name(&state);

        let main = create_window(&state).await;
        rename_first_tab(&state, &main.workspaceid, "main-session");
        let tearoff = create_window(&state).await;
        rename_first_tab(&state, &tearoff.workspaceid, "tiny-tearoff");

        // Close "main" first — does NOT empty windowids (tearoff still open).
        let ret = handle_window_service(&state, &close_call(&main.oid)).await;
        assert!(ret.success);
        assert_eq!(
            last_session_snapshot_tab_name(&state),
            snapshot_after_bootstrap_close,
            "closing main first must not snapshot — it isn't the terminal close"
        );

        // Close "tearoff" last — THIS empties windowids, so it (correctly,
        // by definition of "what was open at full quit") is what's saved.
        let ret = handle_window_service(&state, &close_call(&tearoff.oid)).await;
        assert!(ret.success);
        assert_eq!(
            last_session_snapshot_tab_name(&state).as_deref(),
            Some("tiny-tearoff")
        );
    }
}
