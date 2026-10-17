// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Every window comes back after a quit, an upgrade's included
//! (`docs/specs/SPEC_UPGRADE_FOR_USERS_AND_AGENTS_2026_10_15.md` §6.1).
//!
//! The session record (`session_restore`, `Client.meta["session:last_topology"]`)
//! held one workspace: the last window's, saved by the close that emptied
//! `Client.windowids`. A quit closes the windows one by one and each close
//! deletes its workspace, so with several windows open only the last came back.
//!
//! Now:
//! - a quit asks srv for the whole session before any window closes
//!   (`SaveSessionSnapshot` with `for_quit`, from the host's `quit_app`), and so
//!   does the OS-shutdown flush: one entry per window in `windows`, the main
//!   window first, each the shape `snapshot_workspace` makes plus the window's
//!   position and size under `window`;
//! - the close that then empties `Client.windowids` keeps a quit's record
//!   instead of replacing it with its own workspace ([`save_on_last_close`]);
//! - restore rebuilds the first entry into the window being created, as
//!   before, and every other entry into a workspace of its own, which the main
//!   window then opens each where it was (`TakeRestoredWindows`, then the
//!   host's `open_restored_window`).
//!
//! The record keeps the main window's tabs at its top level too, the shape it
//! always had, so an older AgentMux reading it still restores that window. A
//! record without `windows` reads as one window.

use agentmux_common::ipc::Event;
use serde_json::{json, Value};

use crate::backend::obj::{Client, Window, Workspace};
use crate::backend::service::{WebCallType, WebReturnType};
use crate::backend::storage::store::Store;
use crate::server::AppState;

/// Restored workspaces waiting for the main window to open them.
const WAITING_META_KEY: &str = "session:restored_windows";

/// How long a quit's record outranks the last close's: a quit closes every
/// window within seconds, and one that stalls longer than this was abandoned.
const QUIT_RECORD_KEPT_FOR_MS: i64 = 120_000;

/// The live full windows, the main one first, each with its workspace.
/// Subwindows belong to their parent's window; a window whose workspace is
/// gone, or an id with no window row, is skipped.
fn live_windows(store: &Store) -> Vec<(Window, String)> {
    let Some(client) = store.get_all::<Client>().ok().and_then(|cs| cs.into_iter().next()) else {
        return Vec::new();
    };
    let mut seen = std::collections::HashSet::new();
    let mut windows: Vec<(Window, String)> = client
        .windowids
        .iter()
        .filter_map(|id| store.get::<Window>(id).ok().flatten())
        .filter(|w| w.kind.as_deref() != Some("subwindow"))
        .filter(|w| store.get::<Workspace>(&w.workspaceid).ok().flatten().is_some())
        .filter(|w| seen.insert(w.workspaceid.clone()))
        .map(|w| {
            let ws = w.workspaceid.clone();
            (w, ws)
        })
        .collect();
    // The main window first: by its `host:label`, and otherwise by the order
    // the windows opened, since it opens first (a row can lack the label).
    windows.sort_by_key(|(w, _)| w.meta.get("host:label").and_then(|v| v.as_str()) != Some("main"));
    windows
}

/// Whether any live window has a workspace to capture.
pub(crate) fn has_live_window(store: &Store) -> bool {
    !live_windows(store).is_empty()
}

/// The whole session: every live window's workspace and placement, the main
/// window's tabs also at the top level. `None` when no window has anything to
/// capture. `for_quit` marks it as a quit's, which the closes that follow keep.
pub(crate) fn snapshot_session(store: &Store, for_quit: bool) -> Option<Value> {
    let windows: Vec<Value> = live_windows(store)
        .into_iter()
        .filter_map(|(w, ws)| {
            let mut entry = super::session_restore::snapshot_workspace(store, &ws)?;
            entry["window"] = json!({
                "x": w.pos.x,
                "y": w.pos.y,
                "width": w.winsize.width,
                "height": w.winsize.height,
            });
            Some(entry)
        })
        .collect();
    let mut record = windows.first()?.clone();
    if let Some(obj) = record.as_object_mut() {
        obj.remove("window");
    }
    record["windows"] = Value::Array(windows);
    if for_quit {
        record["saved_for"] = json!("quit");
        record["saved_at_ms"] = json!(agentmux_common::time::now_ms());
    }
    Some(record)
}

/// The windows a record holds: its `windows`, or the record itself for one
/// saved before records held several.
pub(crate) fn windows_of(record: &Value) -> Vec<Value> {
    match record.get("windows").and_then(|v| v.as_array()) {
        Some(windows) if !windows.is_empty() => windows.clone(),
        _ => vec![record.clone()],
    }
}

/// Whether `record` is a quit's, saved within the last two minutes: the
/// closes of that quit are still arriving.
fn is_recent_quit_record(record: &Value, now_ms: i64) -> bool {
    record.get("saved_for").and_then(|v| v.as_str()) == Some("quit")
        && record
            .get("saved_at_ms")
            .and_then(|v| v.as_i64())
            .is_some_and(|at| (0..QUIT_RECORD_KEPT_FOR_MS).contains(&(now_ms - at)))
}

/// The close that empties `Client.windowids`, before `ws_id` is deleted:
/// saves that workspace as the session, unless this close is one of a quit's
/// that already saved every window.
pub(crate) fn save_on_last_close(store: &Store, ws_id: &str) {
    let kept = super::session_restore::load_last_session_snapshot(store)
        .is_some_and(|r| is_recent_quit_record(&r, agentmux_common::time::now_ms()));
    if kept {
        tracing::info!(workspace_id = %ws_id, "session_restore: keeping the record the quit saved for every window");
        return;
    }
    if let Some(snapshot) = super::session_restore::snapshot_workspace(store, ws_id) {
        super::session_restore::save_last_session_snapshot(store, snapshot);
    }
}

/// Set (or, with `None`, remove) one of the client's meta keys.
fn set_client_meta(store: &Store, key: &str, value: Option<Value>) {
    let result = store.with_tx(|tx| {
        let Some(mut client) = tx.get_all::<Client>()?.into_iter().next() else {
            return Ok(());
        };
        match value {
            Some(v) => {
                client.meta.insert(key.to_string(), v);
            }
            None => {
                if client.meta.remove(key).is_none() {
                    return Ok(());
                }
            }
        }
        tx.update(&mut client)?;
        Ok(())
    });
    if let Err(e) = result {
        tracing::warn!(error = %e, key, "session_restore: client meta write failed");
    }
}

/// Restore every window in `record`: the first that comes back is the one
/// being created (returned, as `restore_last_session` always did); each other
/// one gets a workspace of its own, published and left for
/// [`handle_take_restored_windows`].
pub(crate) async fn restore_all(state: &AppState, record: &Value) -> Result<Option<(String, Vec<Event>)>, String> {
    let mut main = None;
    let mut waiting = Vec::new();
    let mut first_err = None;
    for (i, window) in windows_of(record).iter().enumerate() {
        match super::session_restore::restore_workspace(state, window).await {
            Ok(Some(restored)) if main.is_none() => main = Some(restored),
            Ok(Some((ws_id, events))) => {
                super::reducer_helpers::publish_events(state, &events);
                let mut entry = window.get("window").cloned().unwrap_or_else(|| json!({}));
                entry["workspaceid"] = json!(ws_id);
                waiting.push(entry);
            }
            Ok(None) => tracing::warn!(window = i, "session_restore: a window's tabs didn't come back; skipping it"),
            Err(e) => {
                tracing::warn!(window = i, error = %e, "session_restore: a window didn't come back");
                first_err.get_or_insert(e);
            }
        }
    }
    if !waiting.is_empty() {
        tracing::info!(windows = waiting.len() + 1, "session_restore: restored every window of the last session");
    }
    set_client_meta(&state.mstore, WAITING_META_KEY, (!waiting.is_empty()).then(|| Value::Array(waiting)));
    match (main, first_err) {
        (Some(main), _) => Ok(Some(main)),
        (None, Some(e)) => Err(e),
        (None, None) => Ok(None),
    }
}

/// `window.TakeRestoredWindows`: the workspaces a restore rebuilt beside the
/// main window's, each with its saved placement (`workspaceid`, `x`, `y`,
/// `width`, `height`), for the main window to open. Handed out once; one
/// whose workspace is gone since is left out.
pub(crate) fn handle_take_restored_windows(state: &AppState, _call: &WebCallType) -> WebReturnType {
    let store = &state.mstore;
    let waiting = store
        .get_all::<Client>()
        .ok()
        .and_then(|cs| cs.into_iter().next())
        .and_then(|c| c.meta.get(WAITING_META_KEY).cloned())
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default();
    set_client_meta(store, WAITING_META_KEY, None);
    let live: Vec<Value> = waiting
        .into_iter()
        .filter(|w| {
            w.get("workspaceid")
                .and_then(|v| v.as_str())
                .is_some_and(|id| store.get::<Workspace>(id).ok().flatten().is_some())
        })
        .collect();
    WebReturnType::success(Value::Array(live))
}

#[cfg(test)]
#[path = "session_windows_tests.rs"]
mod tests;
