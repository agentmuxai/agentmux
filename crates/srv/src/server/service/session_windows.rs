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
//! Names come back too: each window's own (`window:displayname`, under
//! `window.name`) and its workspace's (`name`). The restore names each
//! workspace as it creates it; a window's name waits in the client's meta
//! (`session:restored_window_names`, by workspace) for the window that opens
//! onto that workspace, which `CreateWindow` names
//! ([`take_restored_window_name`]).
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

/// Restored windows' names, by workspace, waiting for their windows.
const NAMES_META_KEY: &str = "session:restored_window_names";

/// The name the user gave window `w`, if any.
pub(crate) fn window_name(w: &Window) -> Option<String> {
    w.meta
        .get("window:displayname")
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|n| !n.is_empty())
        .map(str::to_string)
}

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
    let live = live_windows(store);
    let window_ids: Vec<String> = live.iter().map(|(w, _)| w.oid.clone()).collect();
    let windows: Vec<Value> = live
        .into_iter()
        .filter_map(|(w, ws)| {
            let mut entry = super::session_restore::snapshot_workspace(store, &ws)?;
            entry["window"] = json!({
                "x": w.pos.x,
                "y": w.pos.y,
                "width": w.winsize.width,
                "height": w.winsize.height,
            });
            if let Some(name) = window_name(&w) {
                entry["window"]["name"] = json!(name);
            }
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
        // The windows this quit saved: the only ones startup may forget.
        record["window_ids"] = json!(window_ids);
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
/// saves that workspace as the session, with the closing window's `name`
/// (read before its row was pruned), unless this close is one of a quit's
/// that already saved every window.
pub(crate) fn save_on_last_close(store: &Store, ws_id: &str, name: Option<&str>) {
    let kept = super::session_restore::load_last_session_snapshot(store)
        .is_some_and(|r| is_recent_quit_record(&r, agentmux_common::time::now_ms()));
    if kept {
        tracing::info!(workspace_id = %ws_id, "session_restore: keeping the record the quit saved for every window");
        return;
    }
    if let Some(mut snapshot) = super::session_restore::snapshot_workspace(store, ws_id) {
        if let Some(name) = name.map(str::trim).filter(|n| !n.is_empty()) {
            snapshot["window"] = json!({ "name": name });
        }
        super::session_restore::save_last_session_snapshot(store, snapshot);
    }
}

/// At startup, before anything reads the windows: when the last run ended in
/// a quit that saved every window (the record is a quit's), a window still
/// listed is one whose close didn't reach srv before it stopped. The host
/// closes windows concurrently and srv can exit first; seen in a simulated
/// upgrade, where the second window's row survived and the main window then
/// took it instead of restoring the session. The record holds that window, so
/// forget its row and its workspace (with its tabs and panes): the first
/// window then restores every window as designed. Only the windows the quit
/// saved (its `window_ids`) are forgotten: a record left from an older quit
/// never costs a later run its windows. Returns how many it forgot.
pub(crate) fn forget_windows_left_by_quit(store: &Store) -> usize {
    let saved: std::collections::HashSet<String> = super::session_restore::load_last_session_snapshot(store)
        .filter(|r| r.get("saved_for").and_then(|v| v.as_str()) == Some("quit"))
        .and_then(|r| r.get("window_ids").and_then(|v| v.as_array()).cloned())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    let Some(mut client) = store.get_all::<Client>().ok().and_then(|cs| cs.into_iter().next()) else {
        return 0;
    };
    let (left, kept): (Vec<String>, Vec<String>) = client.windowids.iter().cloned().partition(|id| saved.contains(id));
    if left.is_empty() {
        return 0;
    }
    client.windowids = kept;
    for id in &left {
        if let Some(w) = store.get::<Window>(id).ok().flatten() {
            if store.get::<Workspace>(&w.workspaceid).ok().flatten().is_some() {
                let _ = crate::backend::wcore::delete_workspace(store, &w.workspaceid);
            }
            let _ = store.delete::<Window>(id);
        }
    }
    if let Err(e) = store.update(&mut client) {
        tracing::warn!(error = %e, "session_restore: couldn't forget the windows a quit left behind");
        return 0;
    }
    tracing::info!(windows = left.len(), "session_restore: forgot windows a quit left behind; the session restores them");
    left.len()
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
    let mut names = serde_json::Map::new();
    let mut first_err = None;
    for (i, window) in windows_of(record).iter().enumerate() {
        let restored = super::session_restore::restore_workspace(state, window).await;
        if let (Ok(Some((ws_id, _))), Some(name)) =
            (&restored, window.pointer("/window/name").and_then(|v| v.as_str()).filter(|n| !n.trim().is_empty()))
        {
            names.insert(ws_id.clone(), json!(name));
        }
        match restored {
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
    set_client_meta(&state.mstore, NAMES_META_KEY, (!names.is_empty()).then(|| Value::Object(names)));
    match (main, first_err) {
        (Some(main), _) => Ok(Some(main)),
        (None, Some(e)) => Err(e),
        (None, None) => Ok(None),
    }
}

/// The name a restored window had, for the window now opening onto its
/// workspace `ws_id`; handed out once (`CreateWindow`).
/// Read, remove and write back in one transaction: restored windows open
/// together, and two takes must not each write back the other's name.
pub(crate) fn take_restored_window_name(store: &Store, ws_id: &str) -> Option<String> {
    let taken = store.with_tx(|tx| {
        let Some(mut client) = tx.get_all::<Client>()?.into_iter().next() else { return Ok(None) };
        let Some(mut names) = client.meta.get(NAMES_META_KEY).and_then(|v| v.as_object().cloned()) else {
            return Ok(None);
        };
        let Some(name) = names.remove(ws_id) else { return Ok(None) };
        if names.is_empty() {
            client.meta.remove(NAMES_META_KEY);
        } else {
            client.meta.insert(NAMES_META_KEY.to_string(), Value::Object(names));
        }
        tx.update(&mut client)?;
        Ok(name.as_str().map(str::to_string))
    });
    taken.unwrap_or_else(|e| {
        tracing::warn!(error = %e, "session_restore: couldn't take a restored window's name");
        None
    })
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
