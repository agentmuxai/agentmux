// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Closing is not deleting (`docs/specs/SPEC_OFFLINE_RESILIENCE_2026_10_20.md`
//! G8, §4.7).
//!
//! A window's close stops what runs in its workspace (its agents and shells,
//! the ordinary graceful close) and keeps the workspace: its tabs, its panes
//! with their ids and meta, and their layout. The workspace is then listed in
//! the client's meta (`session:closed_windows`, newest first) with where its
//! window was, its name and its appearance, for:
//! - the next launch, when the session record names it: the restore reopens it
//!   as it was instead of rebuilding it from the record
//!   (`session_windows::restore_all`), which stays the fallback;
//! - ☰ → "Reopen closed window" (`ListClosedWindows`, `TakeClosedWindow`).
//!
//! At most [`CLOSED_WINDOWS_KEPT`] are kept: closing another deletes the
//! oldest, unless the session record still names it. A workspace with nothing
//! of the user's in it (no pane, or only the untouched default layout) is
//! deleted at its close, as every close did before.
//!
//! Only an explicit, confirmed Close workspace deletes one at once: the window
//! asks for it (`DiscardWorkspaceOnClose`), then closes.

use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::backend::obj::{Client, Window, Workspace};
use crate::backend::service::{self, WebCallType, WebReturnType};
use crate::backend::storage::store::Store;
use crate::server::AppState;

/// The closed windows' workspaces, newest first: `{workspaceid, closed_at_ms, window}`.
pub(crate) const CLOSED_META_KEY: &str = "session:closed_windows";

/// How many closed windows are kept for "Reopen closed window".
pub(crate) const CLOSED_WINDOWS_KEPT: usize = 10;

/// How long a Close workspace waits for its window's close to reach srv. A
/// close that never comes (the window stayed open) must not make a later,
/// ordinary close delete the workspace.
const DISCARD_FOR: Duration = Duration::from_secs(60);

static DISCARD: LazyLock<Mutex<HashMap<String, Instant>>> = LazyLock::new(Default::default);

/// Close workspace: the next close of `window_id` deletes its workspace.
pub(crate) fn mark_discard(window_id: &str) {
    let mut marks = DISCARD.lock().unwrap_or_else(|e| e.into_inner());
    marks.retain(|_, at| at.elapsed() < DISCARD_FOR);
    marks.insert(window_id.to_string(), Instant::now());
}

/// Whether this close of `window_id` is a Close workspace's. Asked once.
pub(crate) fn take_discard(window_id: &str) -> bool {
    let mut marks = DISCARD.lock().unwrap_or_else(|e| e.into_inner());
    marks.remove(window_id).is_some_and(|at| at.elapsed() < DISCARD_FOR)
}

/// Where window `w` is, its name and appearance: a record entry's `window`.
pub(crate) fn placement_of(store: &Store, w: &Window) -> Value {
    let mut window = json!({
        "x": w.pos.x,
        "y": w.pos.y,
        "width": w.winsize.width,
        "height": w.winsize.height,
    });
    if let Some(name) = super::session_windows::saved_name(store, w) {
        window["name"] = json!(name);
    }
    let appearance = super::session_windows::saved_appearance(store, w);
    if !appearance.is_empty() {
        window["meta"] = Value::Object(appearance);
    }
    window
}

fn workspace_of(entry: &Value) -> Option<&str> {
    entry.get("workspaceid").and_then(|v| v.as_str()).filter(|s| !s.is_empty())
}

fn client(store: &Store) -> Option<Client> {
    store.get_all::<Client>().ok().and_then(|cs| cs.into_iter().next())
}

fn entries(store: &Store) -> Vec<Value> {
    client(store)
        .and_then(|c| c.meta.get(CLOSED_META_KEY).and_then(|v| v.as_array()).cloned())
        .unwrap_or_default()
}

/// Read, change and write back the list in one transaction: closes and
/// reopens run concurrently. `None` when the store couldn't be written.
fn update<R>(store: &Store, f: impl FnOnce(&mut Vec<Value>) -> R) -> Option<R> {
    let result = store.with_tx(|tx| {
        let Some(mut client) = tx.get_all::<Client>()?.into_iter().next() else {
            return Ok(None);
        };
        let mut list = client.meta.get(CLOSED_META_KEY).and_then(|v| v.as_array()).cloned().unwrap_or_default();
        let before = list.clone();
        let out = f(&mut list);
        if list != before {
            if list.is_empty() {
                client.meta.remove(CLOSED_META_KEY);
            } else {
                client.meta.insert(CLOSED_META_KEY.to_string(), Value::Array(list));
            }
            tx.update(&mut client)?;
        }
        Ok(Some(out))
    });
    result.unwrap_or_else(|e| {
        tracing::warn!(error = %e, "session_closed: couldn't update the closed windows");
        None
    })
}

/// The workspaces a window shows now.
pub(crate) fn open_workspaces(store: &Store) -> HashSet<String> {
    client(store)
        .map(|c| c.windowids)
        .unwrap_or_default()
        .iter()
        .filter_map(|id| store.get::<Window>(id).ok().flatten())
        .map(|w| w.workspaceid)
        .collect()
}

/// The workspaces the session record names: the next launch reopens them.
fn record_workspaces(store: &Store) -> HashSet<String> {
    super::session_restore::load_last_session_snapshot(store)
        .map(|r| super::session_windows::windows_of(&r))
        .unwrap_or_default()
        .iter()
        .filter_map(|w| workspace_of(w).map(str::to_string))
        .collect()
}

/// Whether `ws_id` holds anything of the user's: a tab with a pane that isn't
/// the untouched default layout.
fn worth_keeping(store: &Store, ws_id: &str) -> bool {
    super::session_restore::snapshot_workspace(store, ws_id)
        .and_then(|snap| super::session_other_builds::summarize(&snap, "", "", 0))
        .is_some()
}

/// List `ws_id` as closed just now, its window at `window`.
fn remember(store: &Store, ws_id: &str, window: Value) {
    let entry = json!({
        "workspaceid": ws_id,
        "closed_at_ms": agentmux_common::time::now_ms(),
        "window": window,
    });
    update(store, |list| {
        list.retain(|e| workspace_of(e) != Some(ws_id));
        list.insert(0, entry);
    });
}

/// `ws_id` is open again: drop it from the list. Returns its entry.
pub(crate) fn forget(store: &Store, ws_id: &str) -> Option<Value> {
    update(store, |list| {
        let at = list.iter().position(|e| workspace_of(e) == Some(ws_id))?;
        Some(list.remove(at))
    })
    .flatten()
}

/// The entries to delete so at most [`CLOSED_WINDOWS_KEPT`] stay, oldest
/// first out; dropped from `list`. An entry the session record names
/// (`protected`) is kept and not counted. One whose workspace is gone
/// (`exists` false) or open again (`open`) is dropped and not deleted.
fn trim(
    list: &mut Vec<Value>,
    protected: &HashSet<String>,
    open: &HashSet<String>,
    exists: impl Fn(&str) -> bool,
) -> Vec<String> {
    let mut kept = 0usize;
    let mut out = Vec::new();
    list.retain(|e| {
        let Some(ws) = workspace_of(e) else { return false };
        if !exists(ws) || open.contains(ws) {
            return false;
        }
        if protected.contains(ws) {
            return true;
        }
        if kept < CLOSED_WINDOWS_KEPT {
            kept += 1;
            return true;
        }
        out.push(ws.to_string());
        false
    });
    out
}

/// Delete the closed workspaces past [`CLOSED_WINDOWS_KEPT`].
async fn evict(state: &AppState) {
    let store = &state.mstore;
    let protected = record_workspaces(store);
    let open = open_workspaces(store);
    let existing: HashSet<String> = store.get_all::<Workspace>().unwrap_or_default().into_iter().map(|w| w.oid).collect();
    let evicted = update(store, |list| trim(list, &protected, &open, |ws| existing.contains(ws))).unwrap_or_default();
    for ws_id in evicted {
        tracing::info!(workspace_id = %ws_id, kept = CLOSED_WINDOWS_KEPT, "session_closed: deleting the oldest closed window's workspace");
        super::window_close::close_workspace(state, ws_id).await;
    }
}

/// A window onto `ws_id` closed, and no other window shows it (`window`: where
/// it was): stop what runs in it and keep it. A workspace with nothing of the
/// user's in it is deleted instead.
pub(crate) async fn keep_workspace(state: &AppState, ws_id: String, window: Value) {
    if !worth_keeping(&state.mstore, &ws_id) {
        super::window_close::close_workspace(state, ws_id).await;
        return;
    }
    // Held until its agents have stopped: a reopen waits for it
    // (`TakeClosedWindow`), so it can't start them while they are stopping.
    let guard = crate::sagas::close_pane::workspace_close_lock(&ws_id).await;
    remember(&state.mstore, &ws_id, window);
    tracing::info!(workspace_id = %ws_id, "CloseWindow: keeping the workspace (its agents stop; Reopen closed window and the next launch bring it back)");
    let st = state.clone();
    let done = crate::sagas::agent_teardown::detached(async move {
        let _guard = guard;
        let block_ids = super::window_close::store_block_ids(&st.mstore, &ws_id);
        crate::sagas::agent_teardown::run_many(&st, &block_ids, crate::sagas::agent_teardown::Policy::close()).await;
        // Lifts each block's respawn guard; the blocks stay, so nothing is purged.
        for block_id in &block_ids {
            crate::sagas::close_pane::finish_close(&st, block_id).await;
        }
        evict(&st).await;
    })
    .await;
    if done.is_none() {
        tracing::warn!("CloseWindow: stopping a kept workspace's agents panicked");
    }
}

/// `window.ListClosedWindows`: the closed windows "Reopen closed window"
/// offers, newest first, each `{workspaceid, closed_at_ms, name, tabs, panes,
/// agents}`. One whose workspace is gone or open again is left out.
pub(crate) fn handle_list_closed_windows(state: &AppState, _call: &WebCallType) -> WebReturnType {
    let store = &state.mstore;
    let open = open_workspaces(store);
    let list: Vec<Value> = entries(store)
        .iter()
        .filter_map(|e| {
            let ws_id = workspace_of(e)?;
            if open.contains(ws_id) {
                return None;
            }
            let ws = store.get::<Workspace>(ws_id).ok().flatten()?;
            let snap = super::session_restore::snapshot_workspace(store, ws_id)?;
            let summary = super::session_other_builds::summarize(&snap, "", "", 0)?;
            let name = e
                .get("window")
                .and_then(|w| w.get("name"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or(ws.name);
            Some(json!({
                "workspaceid": ws_id,
                "closed_at_ms": e.get("closed_at_ms").and_then(|v| v.as_i64()).unwrap_or(0),
                "name": name,
                "tabs": summary.tabs,
                "panes": summary.panes,
                "agents": summary.agents,
            }))
        })
        .collect();
    WebReturnType::success(Value::Array(list))
}

/// `window.TakeClosedWindow(workspaceId)`: reopen a closed window. Waits for
/// its agents to finish stopping, drops it from the list and returns where it
/// was (`{workspaceid, x, y, width, height}`) for the host to open it there;
/// its name and appearance wait for that window (`CreateWindow`). Errors when
/// it isn't a closed window any more.
pub(crate) async fn handle_take_closed_window(state: &AppState, call: &WebCallType) -> WebReturnType {
    let ws_id: String = match service::get_arg(&call.args, 0) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    let _stopped = crate::sagas::close_pane::workspace_close_lock(&ws_id).await;
    let store = &state.mstore;
    if open_workspaces(store).contains(&ws_id) {
        forget(store, &ws_id);
        return WebReturnType::error("that window is open");
    }
    let Some(entry) = forget(store, &ws_id) else {
        return WebReturnType::error("that window isn't among the closed windows any more");
    };
    if store.get::<Workspace>(&ws_id).ok().flatten().is_none() {
        return WebReturnType::error("that window's workspace is gone");
    }
    let window = entry.get("window").cloned().unwrap_or_else(|| json!({}));
    let patch = super::session_windows::patch_of(&window);
    if !patch.is_empty() {
        let mut metas = serde_json::Map::new();
        metas.insert(ws_id.clone(), Value::Object(patch));
        super::session_windows::queue_restored_windows(store, Vec::new(), metas);
    }
    let mut out = json!({ "workspaceid": ws_id });
    for key in ["x", "y", "width", "height"] {
        if let Some(v) = window.get(key) {
            out[key] = v.clone();
        }
    }
    WebReturnType::success(out)
}

/// `window.DiscardWorkspaceOnClose(windowId)`: Close workspace, confirmed in
/// the window: its close, which the window starts next, deletes the workspace.
pub(crate) fn handle_discard_workspace_on_close(_state: &AppState, call: &WebCallType) -> WebReturnType {
    let window_id: String = match service::get_arg(&call.args, 0) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    mark_discard(&window_id);
    tracing::info!(window_id = %window_id, "CloseWindow: Close workspace confirmed; the window's close deletes its workspace");
    WebReturnType::success_empty()
}

/// `window.WindowAgents(windowId)`: the agents running in the window's
/// workspace, each `{block_id, name, working}` (`working`: mid-turn), for the
/// window's close warning and Close workspace's confirmation.
pub(crate) fn handle_window_agents(state: &AppState, call: &WebCallType) -> WebReturnType {
    let window_id: String = match service::get_arg(&call.args, 0) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    let store = &state.mstore;
    let Some(window) = store.get::<Window>(&window_id).ok().flatten() else {
        return WebReturnType::success(json!([]));
    };
    let agents: Vec<Value> = super::window_close::store_block_ids(store, &window.workspaceid)
        .into_iter()
        .filter_map(|block_id| {
            let status = crate::backend::blockcontroller::get_controller(&block_id)?.get_runtime_status();
            if !status.is_agent_pane {
                return None;
            }
            let block = store.get::<crate::backend::obj::Block>(&block_id).ok().flatten();
            let meta_str = |key: &str| {
                block
                    .as_ref()
                    .and_then(|b| b.meta.get(key))
                    .and_then(|v| v.as_str())
                    .filter(|s| !s.is_empty())
                    .map(str::to_string)
            };
            let name = meta_str("agentName").or_else(|| meta_str("agentId")).unwrap_or_else(|| block_id.clone());
            Some(json!({ "block_id": block_id, "name": name, "working": status.turn_active }))
        })
        .collect();
    WebReturnType::success(Value::Array(agents))
}

#[cfg(test)]
#[path = "session_closed_tests.rs"]
mod tests;
