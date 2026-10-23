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
//! workspace as it creates it.
//!
//! So does each window's appearance: its own theme and opacity
//! (`window:theme`, `window:opacity`, [`INHERITED_KEYS`]), under `window.meta`.
//! They live only on the Window row, and a quit deletes the row, so without
//! this every quit and relaunch put them back to the defaults
//! (`RCA_UPGRADE_0_59_30_TO_0_59_34_ISSUES_2026_10_19.md` §2). `window:transparent`
//! is not among them: it is a setting of the whole app, in settings.json.
//!
//! A window's name and appearance wait in the client's meta
//! (`session:restored_window_names`, by workspace) for the window that opens
//! onto that workspace, which `CreateWindow` writes onto its row
//! ([`restored_window_meta`]), and is forgotten only once that is written
//! ([`forget_restored_window_meta`]). The value is the window's meta patch;
//! the bare name an earlier version left still reads.
//!
//! The record keeps the main window's tabs at its top level too, the shape it
//! always had, so an older AgentMux reading it still restores that window. A
//! record without `windows` reads as one window.

use agentmux_common::ipc::Event;
use serde_json::{json, Value};

use super::window_inherit::INHERITED_KEYS;
use crate::backend::obj::{Client, Window, Workspace};
use crate::backend::service::{WebCallType, WebReturnType};
use crate::backend::storage::store::Store;
use crate::server::AppState;

/// Restored workspaces waiting for the main window to open them.
const WAITING_META_KEY: &str = "session:restored_windows";

/// Restored workspaces the main window took (`TakeRestoredWindows`) and is
/// opening: kept for saves until a window shows each, so a save in between
/// doesn't drop one that opens slowly or not at all (#5358).
const OPENING_META_KEY: &str = "session:restored_windows_opening";

/// Restored windows' names and appearance, by workspace, waiting for their
/// windows. (Named for what it first held: a name.)
const NAMES_META_KEY: &str = "session:restored_window_names";

/// The Window meta key of the name the user gave the window.
const NAME_KEY: &str = "window:displayname";

/// The name window `w` goes by in a session record: the user's, or one a
/// restore still holds for it, whose write to the window failed: the next
/// save keeps it, so the restore after names it.
pub(crate) fn saved_name(store: &Store, w: &Window) -> Option<String> {
    window_name(w).or_else(|| restored_window_name(store, &w.workspaceid))
}

/// The appearance a session record keeps for window `w`: its own theme and
/// opacity, or those a restore still holds for it, whose write to the window
/// failed (the next save keeps them, as it keeps a name).
pub(crate) fn saved_appearance(store: &Store, w: &Window) -> serde_json::Map<String, Value> {
    let waiting = restored_window_meta(store, &w.workspaceid);
    INHERITED_KEYS
        .iter()
        .filter_map(|k| {
            let v = w.meta.get(*k).filter(|v| !v.is_null()).or_else(|| waiting.get(*k))?;
            Some((k.to_string(), v.clone()))
        })
        .collect()
}

/// The meta patch a record entry's `window` section gives its restored window:
/// the name, and the appearance that is a theme (a string) and an opacity (a
/// number). Nothing else a record says reaches a row.
pub(crate) fn patch_of(window: &Value) -> serde_json::Map<String, Value> {
    let mut patch = serde_json::Map::new();
    if let Some(name) = window.get("name").and_then(|v| v.as_str()).map(str::trim).filter(|n| !n.is_empty()) {
        patch.insert(NAME_KEY.to_string(), json!(name));
    }
    let meta = window.get("meta");
    if let Some(theme) = meta.and_then(|m| m.get("window:theme")).filter(|v| v.is_string()) {
        patch.insert("window:theme".to_string(), theme.clone());
    }
    if let Some(opacity) = meta.and_then(|m| m.get("window:opacity")).filter(|v| v.is_number()) {
        patch.insert("window:opacity".to_string(), opacity.clone());
    }
    patch
}

/// What waits for one workspace, as a patch: an object of meta, or the bare
/// name an earlier version left.
fn waiting_patch(v: &Value) -> serde_json::Map<String, Value> {
    match v {
        Value::String(name) => patch_of(&json!({ "name": name })),
        Value::Object(o) => patch_of(&json!({ "name": o.get(NAME_KEY), "meta": o })),
        _ => serde_json::Map::new(),
    }
}

/// The name the user gave window `w`, if any.
pub(crate) fn window_name(w: &Window) -> Option<String> {
    w.meta
        .get(NAME_KEY)
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
pub(crate) fn live_windows(store: &Store) -> Vec<(Window, String)> {
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
    let live_ws: std::collections::HashSet<String> = live.iter().map(|(_, ws)| ws.clone()).collect();
    let windows: Vec<Value> = live
        .into_iter()
        .filter_map(|(w, ws)| {
            let mut entry = super::session_restore::snapshot_workspace(store, &ws)?;
            entry["window"] = super::session_closed::placement_of(store, &w);
            // A close keeps the window's contents (`session_closed`): the
            // restore reopens them, and rebuilds them from this entry only if
            // they are gone.
            entry["workspaceid"] = json!(ws);
            Some(entry)
        })
        .chain(waiting_windows(store, &live_ws))
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

/// The restored windows still waiting for the main window to open them
/// (`restore_all`), as record entries: their workspaces exist, but no window
/// shows them yet, so a snapshot of the live windows alone would drop them
/// (#5358).
fn waiting_windows(store: &Store, live_ws: &std::collections::HashSet<String>) -> Vec<Value> {
    let client = store.get_all::<Client>().ok().and_then(|cs| cs.into_iter().next());
    let list = |key: &str| client.as_ref().and_then(|c| c.meta.get(key)?.as_array().cloned()).unwrap_or_default();
    let mut seen = std::collections::HashSet::new();
    list(WAITING_META_KEY)
        .into_iter()
        .chain(list(OPENING_META_KEY))
        .filter(|p| p.get("workspaceid").and_then(|v| v.as_str()).is_some_and(|ws| seen.insert(ws.to_string())))
        .filter_map(|mut placement| {
            let ws = placement.get("workspaceid")?.as_str()?.to_string();
            if live_ws.contains(&ws) {
                return None;
            }
            let mut entry = super::session_restore::snapshot_workspace(store, &ws)?;
            placement.as_object_mut()?.remove("workspaceid");
            entry["window"] = placement;
            entry["workspaceid"] = json!(ws);
            Some(entry)
        })
        .collect()
}

/// The contents of the restored windows still waiting for the main window to
/// open them.
pub(crate) fn waiting_workspaces(store: &Store) -> std::collections::HashSet<String> {
    store
        .get_all::<Client>()
        .ok()
        .and_then(|cs| cs.into_iter().next())
        .and_then(|c| c.meta.get(WAITING_META_KEY).and_then(|v| v.as_array().cloned()))
        .unwrap_or_default()
        .iter()
        .filter_map(|w| w.get("workspaceid").and_then(|v| v.as_str()).map(str::to_string))
        .collect()
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
pub(crate) fn is_recent_quit_record(record: &Value, now_ms: i64) -> bool {
    record.get("saved_for").and_then(|v| v.as_str()) == Some("quit")
        && record
            .get("saved_at_ms")
            .and_then(|v| v.as_i64())
            .is_some_and(|at| (0..QUIT_RECORD_KEPT_FOR_MS).contains(&(now_ms - at)))
}

/// The close that empties `Client.windowids`, before `ws_id` is kept or
/// deleted: saves that workspace as the session, with the closing window's `name` and
/// `appearance` (read before its row was pruned), unless this close is one of
/// a quit's that already saved every window.
pub(crate) fn save_on_last_close(
    store: &Store,
    ws_id: &str,
    name: Option<&str>,
    appearance: &serde_json::Map<String, Value>,
) {
    let kept = super::session_restore::load_last_session_snapshot(store)
        .is_some_and(|r| is_recent_quit_record(&r, agentmux_common::time::now_ms()));
    if kept {
        tracing::info!(workspace_id = %ws_id, "session_restore: keeping the record the quit saved for every window");
        return;
    }
    if let Some(mut snapshot) = super::session_restore::snapshot_workspace(store, ws_id) {
        let mut window = serde_json::Map::new();
        if let Some(name) = name.map(str::trim).filter(|n| !n.is_empty()) {
            window.insert("name".to_string(), json!(name));
        }
        if !appearance.is_empty() {
            window.insert("meta".to_string(), Value::Object(appearance.clone()));
        }
        if !window.is_empty() {
            snapshot["window"] = Value::Object(window);
        }
        // When it was closed, for another build offering it (`session_other_builds`).
        snapshot["saved_at_ms"] = json!(agentmux_common::time::now_ms());
        // The close keeps the workspace: the restore reopens it as it was.
        snapshot["workspaceid"] = json!(ws_id);
        super::session_restore::save_last_session_snapshot(store, snapshot.clone());
        // The open workspace keeps it too (`user_workspace`), whole: the saver
        // makes no save with no window open.
        super::user_workspace::write_record_now(store, snapshot);
    }
}

/// At startup, before anything reads the windows: when the last run ended in
/// a quit that saved every window (the record is a quit's), a window still
/// listed is one whose close didn't reach srv before it stopped. The host
/// closes windows concurrently and srv can exit first; seen in a simulated
/// upgrade, where the second window's row survived and the main window then
/// took it instead of restoring the session. The record holds that window, so
/// forget its row: the first window then restores every window as designed.
/// Its workspace stays when the record names it (the restore reopens it), and
/// goes with its tabs and panes when it doesn't (a record an older build
/// wrote, which rebuilds it instead). Only the windows the quit saved (its
/// `window_ids`) are forgotten: a record left from an older quit never costs a
/// later run its windows. Returns how many it forgot.
pub(crate) fn forget_windows_left_by_quit(store: &Store) -> usize {
    let record = super::session_restore::load_last_session_snapshot(store)
        .filter(|r| r.get("saved_for").and_then(|v| v.as_str()) == Some("quit"));
    let saved: std::collections::HashSet<String> = record
        .as_ref()
        .and_then(|r| r.get("window_ids").and_then(|v| v.as_array()).cloned())
        .unwrap_or_default()
        .into_iter()
        .filter_map(|v| v.as_str().map(str::to_string))
        .collect();
    let named: std::collections::HashSet<String> = record
        .as_ref()
        .map(windows_of)
        .unwrap_or_default()
        .iter()
        .filter_map(|w| w.get("workspaceid").and_then(|v| v.as_str()).map(str::to_string))
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
            if !named.contains(&w.workspaceid) && store.get::<Workspace>(&w.workspaceid).ok().flatten().is_some() {
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

/// Forget the restored windows waiting to open, those opening, and their
/// names: they belong to the workspace that was open, which a switch closes
/// (`user_workspace_switch`).
pub(crate) fn forget_restore_lists(meta: &mut crate::backend::obj::MetaMapType) {
    for key in RESTORE_LIST_KEYS {
        meta.remove(key);
    }
}

/// The client meta keys of the restored windows' queues.
pub(crate) const RESTORE_LIST_KEYS: [&str; 3] = [WAITING_META_KEY, OPENING_META_KEY, NAMES_META_KEY];

/// Set (or, with `None`, remove) one of the client's meta keys.
pub(crate) fn set_client_meta(store: &Store, key: &str, value: Option<Value>) {
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
/// [`handle_take_restored_windows`]. A workspace record's closed windows
/// (`"state": "closed"`) stay closed (`session_closed::restore_closed`), once
/// a window came back.
pub(crate) async fn restore_all(state: &AppState, record: &Value) -> Result<Option<(String, Vec<Event>)>, String> {
    let mut main = None;
    let mut waiting = Vec::new();
    let mut waiting_meta = serde_json::Map::new();
    let mut first_err = None;
    let mut restored_count = 0usize;
    let mut reopened = 0usize;
    let mut taken = std::collections::HashSet::new();
    let all = windows_of(record);
    let open: Vec<&Value> = all.iter().filter(|w| !super::session_closed::is_closed(w)).collect();
    for (i, window) in open.iter().copied().enumerate() {
        let restored = match reusable_workspace(state, window, &taken).await {
            Some(ws_id) => {
                reopened += 1;
                taken.insert(ws_id.clone());
                super::session_closed::forget(&state.mstore, &ws_id);
                Ok(Some((ws_id, Vec::new())))
            }
            None => super::session_restore::restore_workspace(state, window).await,
        };
        if let Ok(Some((ws_id, _))) = &restored {
            let patch = window.get("window").map(patch_of).unwrap_or_default();
            if !patch.is_empty() {
                waiting_meta.insert(ws_id.clone(), Value::Object(patch));
            }
        }
        match restored {
            Ok(Some(restored)) if main.is_none() => {
                restored_count += 1;
                main = Some(restored);
            }
            Ok(Some((ws_id, events))) => {
                restored_count += 1;
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
    // One line for every restore, a one-window one included: after a quit the
    // store loads no workspace, tab, block or window, which reads like data loss
    // unless this says the layout came back from the record.
    if restored_count > 0 {
        let from = if record.get("saved_for").and_then(|v| v.as_str()) == Some("quit") { "quit" } else { "last-close" };
        tracing::info!(
            windows = restored_count,
            reopened,
            "session_restore: restored {} window(s) from the {} record ({} kept as they were, the rest rebuilt)",
            restored_count,
            from,
            reopened
        );
    }
    set_client_meta(&state.mstore, WAITING_META_KEY, (!waiting.is_empty()).then(|| Value::Array(waiting)));
    set_client_meta(&state.mstore, OPENING_META_KEY, None);
    set_client_meta(&state.mstore, NAMES_META_KEY, (!waiting_meta.is_empty()).then(|| Value::Object(waiting_meta)));
    if let Some((main_ws, _)) = &main {
        let shown = open
            .iter()
            .filter_map(|w| w.get("workspaceid").and_then(|v| v.as_str()).map(str::to_string))
            .collect();
        super::session_closed::restore_closed(state, record, main_ws, &shown).await;
    }
    match (main, first_err) {
        (Some(main), _) => Ok(Some(main)),
        (None, Some(e)) => Err(e),
        (None, None) => Ok(None),
    }
}

/// The workspace a record entry names, when it is still there to reopen as it
/// was: the reducer and the store have it, it holds a pane, no window shows
/// it, and no earlier entry of this restore took it. `None` means rebuild the
/// entry from the record.
async fn reusable_workspace(
    state: &AppState,
    window: &Value,
    taken: &std::collections::HashSet<String>,
) -> Option<String> {
    let ws_id = window.get("workspaceid").and_then(|v| v.as_str()).filter(|s| !s.is_empty())?;
    if taken.contains(ws_id) || !state.srv_state.lock().await.workspaces.contains_key(ws_id) {
        return None;
    }
    // A close still stopping its agents (srv kept running with no window) finishes first.
    drop(crate::sagas::close_pane::workspace_close_lock(ws_id).await);
    let store = &state.mstore;
    super::session_restore::snapshot_workspace(store, ws_id)?;
    if super::session_closed::open_workspaces(store).contains(ws_id) {
        return None;
    }
    Some(ws_id.to_string())
}

/// Add restored workspaces for the main window to open beside it
/// (`entries`: placement plus `workspaceid`, as [`restore_all`] queues them)
/// and their windows' meta patches, keeping any already waiting.
pub(crate) fn queue_restored_windows(store: &Store, entries: Vec<Value>, metas: serde_json::Map<String, Value>) {
    let client = store.get_all::<Client>().ok().and_then(|cs| cs.into_iter().next());
    let meta_of = |key: &str| client.as_ref().and_then(|c| c.meta.get(key).cloned());
    if !entries.is_empty() {
        let mut waiting = meta_of(WAITING_META_KEY).and_then(|v| v.as_array().cloned()).unwrap_or_default();
        waiting.extend(entries);
        set_client_meta(store, WAITING_META_KEY, Some(Value::Array(waiting)));
    }
    if !metas.is_empty() {
        let mut names = meta_of(NAMES_META_KEY).and_then(|v| v.as_object().cloned()).unwrap_or_default();
        names.extend(metas);
        set_client_meta(store, NAMES_META_KEY, Some(Value::Object(names)));
    }
}

/// The name and appearance a restored window had, as the meta patch for the
/// window now opening onto its workspace `ws_id` (`CreateWindow`). It stays
/// until [`forget_restored_window_meta`]: a window whose meta couldn't be
/// written (srv stopped in between, a failed write) gets it on its next try.
pub(crate) fn restored_window_meta(store: &Store, ws_id: &str) -> serde_json::Map<String, Value> {
    store
        .get_all::<Client>()
        .ok()
        .and_then(|cs| cs.into_iter().next())
        .and_then(|c| c.meta.get(NAMES_META_KEY)?.get(ws_id).map(waiting_patch))
        .unwrap_or_default()
}

/// The name a restored window had, for the window now opening onto `ws_id`.
pub(crate) fn restored_window_name(store: &Store, ws_id: &str) -> Option<String> {
    restored_window_meta(store, ws_id).get(NAME_KEY).and_then(|v| v.as_str()).map(str::to_string)
}

/// `ws_id`'s window has its meta: forget it, once. Read, remove and write
/// back in one transaction: restored windows open together, and two of these
/// must not each write back the entry the other removed.
pub(crate) fn forget_restored_window_meta(store: &Store, ws_id: &str) -> Option<serde_json::Map<String, Value>> {
    let taken = store.with_tx(|tx| {
        let Some(mut client) = tx.get_all::<Client>()?.into_iter().next() else { return Ok(None) };
        let Some(mut waiting) = client.meta.get(NAMES_META_KEY).and_then(|v| v.as_object().cloned()) else {
            return Ok(None);
        };
        let Some(entry) = waiting.remove(ws_id) else { return Ok(None) };
        if waiting.is_empty() {
            client.meta.remove(NAMES_META_KEY);
        } else {
            client.meta.insert(NAMES_META_KEY.to_string(), Value::Object(waiting));
        }
        tx.update(&mut client)?;
        Ok(Some(waiting_patch(&entry)))
    });
    taken.unwrap_or_else(|e| {
        tracing::warn!(error = %e, "session_restore: couldn't forget a restored window's meta");
        None
    })
}

/// `window.TakeRestoredWindows`: the workspaces a restore rebuilt beside the
/// main window's, each with its saved placement (`workspaceid`, `x`, `y`,
/// `width`, `height`), for the main window to open. Handed out once; one
/// whose workspace is gone since is left out.
pub(crate) fn handle_take_restored_windows(state: &AppState, _call: &WebCallType) -> WebReturnType {
    let store = &state.mstore;
    // One transaction moves them from waiting to opening, so no save sees
    // them in neither list (#5358).
    let waiting = store
        .with_tx(|tx| {
            let Some(mut client) = tx.get_all::<Client>()?.into_iter().next() else { return Ok(Vec::new()) };
            let Some(waiting) = client.meta.remove(WAITING_META_KEY).and_then(|v| v.as_array().cloned()) else {
                return Ok(Vec::new());
            };
            let mut opening = client.meta.get(OPENING_META_KEY).and_then(|v| v.as_array().cloned()).unwrap_or_default();
            opening.extend(waiting.iter().cloned());
            if !opening.is_empty() {
                client.meta.insert(OPENING_META_KEY.to_string(), Value::Array(opening));
            }
            tx.update(&mut client)?;
            Ok(waiting)
        })
        .unwrap_or_else(|e| {
            tracing::warn!(error = %e, "session_restore: couldn't take the restored windows");
            Vec::new()
        });
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

/// A window now shows restored workspace `ws_id`: saves take it from the
/// live windows from here on.
pub(crate) fn restored_window_opened(store: &Store, ws_id: &str) {
    let _ = store.with_tx(|tx| {
        let Some(mut client) = tx.get_all::<Client>()?.into_iter().next() else { return Ok(()) };
        let Some(opening) = client.meta.get(OPENING_META_KEY).and_then(|v| v.as_array().cloned()) else { return Ok(()) };
        let left: Vec<Value> =
            opening.into_iter().filter(|e| e.get("workspaceid").and_then(|v| v.as_str()) != Some(ws_id)).collect();
        if left.is_empty() {
            client.meta.remove(OPENING_META_KEY);
        } else {
            client.meta.insert(OPENING_META_KEY.to_string(), Value::Array(left));
        }
        tx.update(&mut client)?;
        Ok(())
    });
}

#[cfg(test)]
#[path = "session_windows_tests.rs"]
mod tests;
