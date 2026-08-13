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
    // Feature 1) — snapshot this window's workspace BEFORE the destroy
    // cascade below removes it, so the next cold launch has something to
    // restore instead of always reseeding the default layout. Best-effort
    // and independent of the cascade: a failure here must never block the
    // close itself (see `session_restore` module docs for why this is a
    // separate durable record, not a change to the cascade).
    if let Some(ref ws_id_for_snapshot) = ws_id {
        if let Some(snapshot) =
            super::session_restore::snapshot_workspace(store, ws_id_for_snapshot)
        {
            super::session_restore::save_last_session_snapshot(store, snapshot);
        }
    }
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
