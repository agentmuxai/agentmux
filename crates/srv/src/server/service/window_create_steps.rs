// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The steps of `CreateWindow`, in the order the handler runs them. A step that can refuse the
//! call answers `Err` with the reply that ends it.

#![allow(clippy::result_large_err)]

use super::*;

type Events = Vec<agentmux_common::ipc::Event>;

/// What a window's restore request came to, and the lock that keeps the next opener behind it.
pub(super) struct RestoreRequest {
    pub(super) restore_if_available: bool,
    /// Held until this window is registered, so the next opener sees it.
    _election: Option<tokio::sync::MutexGuard<'static, ()>>,
}

/// The workspace the window shows, and what building it did.
pub(super) struct WorkspaceChoice {
    pub(super) ws_id: String,
    /// Events of a workspace made for this window, unpublished (empty for an existing one).
    pub(super) fresh_workspace_events: Events,
    /// Set when a restore actually replays the last snapshot — gates clearing
    /// it (see `created_reply`) so a genuinely successful restore
    /// can't be replayed again by a later call, while a crash mid-restore
    /// (this flag set, but the function never reaches that final clear)
    /// leaves the snapshot intact for the next launch to retry.
    pub(super) did_restore: bool,
    /// What that restore loaded, and from which copy: the open workspace keeps
    /// it as its last clean load once this window is up (`user_workspace`).
    pub(super) restored_copy: Option<(serde_json::Value, super::super::user_workspace::LoadedFrom)>,
}

/// Whether this call restores the last session, deciding it under the restore election.
pub(super) async fn restore_request(state: &AppState, args: &[serde_json::Value]) -> RestoreRequest {
    let store = &state.mstore;
    // Restore-on-relaunch (SPEC_SESSION_RESTORE_AND_SAVED_LAYOUTS_2026_08_13
    // Feature 1) — arg 3 is set only by the frontend's genuine cold-start
    // call site (`Client.windowids` was empty). Every other caller
    // (tear-off, "Open New Window", the workspace-missing error-recovery
    // fallback) omits it and gets the pre-existing default-seed behavior,
    // completely unchanged.
    //
    // The client's own claim isn't trusted alone (reagentx P2 on PR #2560):
    // re-check `Client.windowids` server-side too, so a stray/duplicate/
    // misused call with `restoreIfAvailable: true` mid-session (when other
    // windows are legitimately still open) can't resurrect the last snapshot
    // into a spurious extra workspace.
    let windowids_empty = || {
        store.get_all::<Client>().ok().and_then(|cs| cs.into_iter().next()).map(|c| c.windowids.is_empty()).unwrap_or(true)
    };
    let wants_restore: bool = service::get_arg(args, 3).unwrap_or(false);
    let mut client_windowids_empty = windowids_empty();
    // Held until this window is registered, so the next opener sees it.
    let mut _election = None;
    if wants_restore {
        let elected = RESTORE_ELECTION.lock().await;
        client_windowids_empty = windowids_empty();
        let restored_at = store
            .get_all::<Client>()
            .ok()
            .and_then(|cs| cs.into_iter().next())
            .and_then(|c| c.meta.get(RESTORED_AT_META_KEY).and_then(|v| v.as_i64()));
        // The host saw another window still opening (arg 7): the window srv
        // lists is that concurrent opener's, not a restored window closing
        // again, which this waits for like any close (#5363).
        let another_opening: bool = service::get_optional_arg(args, 7).unwrap_or(None).unwrap_or(false);
        let just_restored =
            another_opening && restored_at.is_some_and(|at| agentmux_common::time::now_ms() - at < JUST_RESTORED_MS);
        // A window the tray opens right after the last one closed can arrive
        // before that close does (the host sends it on its own thread): wait
        // briefly for it, rather than open the default panes and lose the
        // window it closed (#5363).
        if !client_windowids_empty && !just_restored {
            let deadline = std::time::Instant::now() + RESTORE_WAITS_FOR_CLOSE;
            while !client_windowids_empty && std::time::Instant::now() < deadline {
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                client_windowids_empty = windowids_empty();
            }
        }
        if client_windowids_empty {
            super::super::session_windows::set_client_meta(
                store,
                RESTORED_AT_META_KEY,
                Some(serde_json::json!(agentmux_common::time::now_ms())),
            );
        }
        _election = Some(elected);
    }
    let restore_if_available: bool = wants_restore && client_windowids_empty;
    RestoreRequest { restore_if_available, _election }
}

/// The workspace for the window: the one asked for, a restored one, or a new one with the default panes.
pub(super) async fn resolve_workspace(
    state: &AppState,
    requested_ws_id: String,
    restore_if_available: bool,
    seed_view: Option<String>,
    seed_meta: Option<serde_json::Value>,
) -> Result<WorkspaceChoice, WebReturnType> {
    let store = &state.mstore;
    let mut did_restore = false;
    let mut restored_copy = None;
    // Resolve / create the workspace.
    let (ws_id, fresh_workspace_events): (String, Events) =
        if requested_ws_id.is_empty() {
            let restored_from_last_session = if restore_if_available {
                // The newer of the session record and the open workspace's
                // record, then that workspace's older copies.
                match super::super::user_workspace::restore_at_launch(state).await {
                    Ok(Some(restored)) => {
                        restored_copy = Some((restored.loaded, restored.from));
                        Some(restored.main)
                    }
                    Ok(None) => None,
                    Err(e) => {
                        tracing::warn!(
                            error = %e,
                            "CreateWindow: session restore failed — falling back to default seed"
                        );
                        None
                    }
                }
            } else {
                None
            };
            if let Some(result) = restored_from_last_session {
                did_restore = true;
                result
            } else {
                match create_default_contents(state, seed_view, seed_meta, restore_if_available).await {
                    Ok(made) => made,
                    Err(e) => return Err(WebReturnType::error(e)),
                }
            }
        } else {
            // Existing workspace — verify it's in the reducer
            // (or SQLite), but no creation needed.
            let exists_in_sqlite = match store.get::<Workspace>(&requested_ws_id) {
                Ok(opt) => opt.is_some(),
                Err(e) => {
                    return Err(WebReturnType::error(format!(
                        "CreateWindow: workspace lookup failed: {}",
                        e
                    )));
                }
            };
            if !exists_in_sqlite {
                return Err(WebReturnType::error(format!(
                    "CreateWindow: workspace not found: {}",
                    requested_ws_id
                )));
            }
            (requested_ws_id, Vec::new())
        };
    Ok(WorkspaceChoice { ws_id, fresh_workspace_events, did_restore, restored_copy })
}

/// Registers the window in the reducer for `ws_id` and marks its row new. On a refusal the workspace
/// made for it is deleted again. Returns the window's id and the events to publish.
pub(super) async fn register_window(state: &AppState, ws_id: &str, fresh_workspace_events: &Events) -> Result<(String, Events), WebReturnType> {
    let store = &state.mstore;
    // Step 3: register the window in the reducer.
    let window_id = uuid::Uuid::new_v4().to_string();
    let win_events = dispatch_to_reducer(
        state,
        agentmux_common::ipc::Command::CreateWindow {
            window_id: window_id.clone(),
            workspace_id: ws_id.to_string(),
        },
    )
    .await;
    if let Some(err_msg) = win_events.iter().find_map(|e| match e {
        agentmux_common::ipc::Event::Error { message, .. } => Some(message.clone()),
        _ => None,
    }) {
        // Compensate the fresh workspace if we created one.
        if !fresh_workspace_events.is_empty() {
            let comp = dispatch_to_reducer(
                state,
                agentmux_common::ipc::Command::DeleteWorkspace {
                    workspace_id: ws_id.to_string(),
                    // Internal compensation path (Step 5 PR 2).
                    force: false,
                },
            )
            .await;
            for ev in &comp {
                let _ = crate::persist_subscriber::apply_event_to_mstore(ev, store);
            }
            publish_events(state, &comp);
        }
        return Err(WebReturnType::error(err_msg));
    }
    for ev in &win_events {
        if let Err(e) = crate::persist_subscriber::apply_event_to_mstore(ev, store) {
            return Err(WebReturnType::error(format!(
                "CreateWindow: SQLite write failed: {}",
                e
            )));
        }
    }
    // Mark the window as `isnew` so the host's first-paint
    // signaling logic still applies — wcore::create_window_full
    // set this; the subscriber's default is `isnew: false`.
    if let Ok(mut win) = store.must_get::<Window>(&window_id) {
        if !win.isnew {
            win.isnew = true;
            let _ = store.update(&mut win);
        }
    }
    Ok((window_id, win_events))
}

/// The host window label as a meta crumb on the new row. Returns the events to publish.
pub(super) async fn write_label_crumb(state: &AppState, window_id: &str, host_label: &str) -> Events {
    let store = &state.mstore;
    // SPEC_POOL_ADOPTION_AND_WINDOW_ROW_CRUMB_2026_07_11 Residual 2 —
    // persist the caller's host window label as a meta crumb,
    // atomically with row creation (optional arg 2; old callers send
    // two args and skip this). The crumb exists so cleanup paths can
    // resolve label → window row WITHOUT the host-side registration
    // chain (`register_backend_window` → launcher shadow), which is
    // exactly what's missing in the early-close race shape (#2088)
    // and after a host crash. It is a HINT, not an identity — labels
    // recur across host restarts — so consumers must treat a miss or
    // duplicate as "fall back to current behavior," never delete on
    // crumb evidence alone. Non-fatal on failure: an unattributed
    // row is the pre-crumb status quo.
    if host_label.is_empty() {
        Vec::new()
    } else {
        let evs = dispatch_to_reducer(
            state,
            agentmux_common::ipc::Command::UpdateWindowMeta {
                window_id: window_id.to_string(),
                meta_patch: serde_json::json!({ "host:label": host_label }),
            },
        )
        .await;
        for ev in &evs {
            if let Err(e) = crate::persist_subscriber::apply_event_to_mstore(ev, store) {
                tracing::warn!(
                    window_id = %window_id,
                    error = %e,
                    "CreateWindow: host:label crumb write failed — row stays unattributed"
                );
            }
        }
        evs
    }
}

/// A window a session restore rebuilt gets back its name, theme and opacity. Returns the events to publish.
pub(super) async fn restored_name_events(state: &AppState, window_id: &str, ws_id: &str) -> Events {
    let store = &state.mstore;
    // A window a session restore rebuilt gets back the name, theme and opacity
    // it had (session_windows.rs): the main one, and each reopened beside it.
    // They are forgotten only once written.
    super::super::session_windows::restored_window_opened(store, ws_id);
    // A closed window reopening (`TakeClosedWindow`) is open now.
    super::super::session_closed::forget(store, ws_id);
    let restored_meta = super::super::session_windows::restored_window_meta(store, ws_id);
    match (!restored_meta.is_empty()).then_some(restored_meta) {
        None => Vec::new(),
        Some(patch) => {
            let evs = dispatch_to_reducer(
                state,
                agentmux_common::ipc::Command::UpdateWindowMeta {
                    window_id: window_id.to_string(),
                    meta_patch: serde_json::Value::Object(patch),
                },
            )
            .await;
            let mut written = !evs.iter().any(|e| matches!(e, agentmux_common::ipc::Event::Error { .. }));
            for ev in &evs {
                if let Err(e) = crate::persist_subscriber::apply_event_to_mstore(ev, store) {
                    tracing::warn!(window_id = %window_id, error = %e, "CreateWindow: restored window name write failed");
                    written = false;
                }
            }
            if written {
                super::super::session_windows::forget_restored_window_meta(store, ws_id);
            }
            evs
        }
    }
}

/// The appearance this window takes from another: the row it replaces on a reload, and a window whose
/// opener asked for it by host label. Returns the events to publish.
pub(super) async fn inherited_events(state: &AppState, args: &[serde_json::Value], window_id: &str, host_label: &str) -> Events {
    // SPEC_PER_WINDOW_THEME_AND_OPACITY_2026_10_12 §4.6 — a reloaded window
    // other than the main one gets a new row each load (optional arg 6 is
    // the row it had before), so its own theme and opacity come across.
    let previous_window_id: Option<String> = service::get_optional_arg(args, 6).unwrap_or(None);
    let mut reload_events = match previous_window_id.filter(|p| !p.is_empty() && *p != window_id) {
        Some(prev) => super::super::window_inherit::apply_inheritance(state, &prev, window_id).await,
        None => Vec::new(),
    };
    // SPEC_PER_WINDOW_THEME_AND_OPACITY_2026_10_12 §6 — the window that
    // opened this one may already have asked for its appearance to be
    // copied here (`InheritWindowAppearance`, keyed by host label). Taken
    // only after the row is recorded, so a request arriving in between
    // finds the row instead (see window_inherit.rs).
    let inherit_events = match (!host_label.is_empty())
        .then(|| {
            let row_seq = super::super::window_inherit::record_created(host_label, window_id);
            super::super::window_inherit::take_pending(host_label, row_seq)
        })
        .flatten()
    {
        Some(source) => super::super::window_inherit::apply_inheritance(state, &source, window_id).await,
        None => Vec::new(),
    };
    reload_events.extend(inherit_events);
    reload_events
}

/// The window row as the reply, once the snapshot a restore replayed is cleared.
pub(super) fn created_reply(state: &AppState, window_id: &str, choice: &WorkspaceChoice) -> WebReturnType {
    let store = &state.mstore;
    match store.must_get::<Window>(window_id) {
        Ok(win) => {
            // Restore-on-relaunch (SPEC_SESSION_RESTORE_AND_SAVED_LAYOUTS_2026_08_13
            // Feature 1) — only clear the snapshot once the restore it
            // produced has fully, successfully completed (this read-back is
            // the last step of that path). A crash between
            // `restore_last_session` returning and reaching this line never
            // executes this clear, so the next launch's `Client.windowids`
            // is still empty and retries against the same intact snapshot —
            // deliberately not cleared any earlier than this. Reviewer-caught
            // (reagentx P2 on PR #2560): without this, a stray repeat call
            // could replay the same snapshot into a second duplicate
            // workspace even after a genuinely successful restore.
            if choice.did_restore {
                super::super::session_restore::clear_last_session_snapshot(store);
                if let Some((loaded, from)) = &choice.restored_copy {
                    super::super::user_workspace::after_clean_load(store, loaded, *from);
                }
            }
            created_window_reply(store, &win)
        }
        Err(e) => WebReturnType::error(format!(
            "CreateWindow: window read-back failed: {}",
            e
        )),
    }
}
