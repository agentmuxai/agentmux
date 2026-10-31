// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Closed windows: closing a window keeps it in the open workspace
//! (`docs/specs/SPEC_WORKSPACES_2026_10_23.md` §2, §5; closing is not
//! deleting, `SPEC_OFFLINE_RESILIENCE_2026_10_20.md` G8).
//!
//! A window's close stops what runs in its contents (its shells, the ordinary
//! graceful close) and keeps them: its tabs, its panes with their ids and meta,
//! and their layout. Its agents keep running (`agents:always_available`): their
//! blocks are parked, and a reopen puts them back in their tabs. The window is then a **closed** window of
//! the workspace, listed in the client's meta (`workspace:closed_windows`,
//! newest first) with when it closed, where it was, its name and appearance.
//! - The workspace record keeps each one as a `"state": "closed"` entry
//!   ([`with_closed`]). A cold start keeps it closed ([`restore_closed`]): its
//!   contents are reused when they are still here and rebuilt from the entry
//!   when not, and no window opens for it.
//! - The status bar panel lists them (`ListClosedWindows`), reopens one where
//!   it was (`TakeClosedWindow`) and deletes one (`DeleteClosedWindow`). An
//!   open window is deleted by marking its close (`DiscardWorkspaceOnClose`),
//!   which then deletes its contents instead of keeping them.
//!
//! They stay until the user deletes them: no cap. Not listed: the close that
//! ends the session (the last window's, or one of a quit's), whose window
//! stays open in the workspace and reopens at the next launch; a workspace
//! switch's closes (`user_workspace_switch`), whose windows stay open in the
//! workspace they leave and open again with it; and a window
//! with nothing of the user's in it (no pane, or only the untouched default
//! layout), whose contents are deleted at its close.

use std::collections::{HashMap, HashSet};
use std::sync::{LazyLock, Mutex};
use std::time::{Duration, Instant};

use agentmux_common::time::now_ms;
use serde_json::{json, Value};

use crate::backend::obj::{Block, Client, Window, Workspace};
use crate::backend::service::{self, WebCallType, WebReturnType};
use crate::backend::storage::store::Store;
use crate::server::AppState;

/// The open workspace's closed windows, newest first:
/// `{workspaceid, closed_at_ms, window}`.
pub(crate) const CLOSED_META_KEY: &str = "workspace:closed_windows";

/// A workspace record entry's `state`: shown now, or reopened at the next launch.
pub(crate) const STATE_OPEN: &str = "open";
/// A workspace record entry's `state`: closed, kept to reopen.
pub(crate) const STATE_CLOSED: &str = "closed";

/// How long a delete waits for its window's close to reach srv. A close that
/// never comes (the window stayed open) must not make a later, ordinary close
/// delete the window's contents.
const DISCARD_FOR: Duration = Duration::from_secs(60);

static DISCARD: LazyLock<Mutex<HashMap<String, Instant>>> = LazyLock::new(Default::default);

/// Delete an open window: the next close of `window_id` deletes its contents.
pub(crate) fn mark_discard(window_id: &str) {
    let mut marks = DISCARD.lock().unwrap_or_else(|e| e.into_inner());
    marks.retain(|_, at| at.elapsed() < DISCARD_FOR);
    marks.insert(window_id.to_string(), Instant::now());
}

/// Whether this close of `window_id` is a delete's. Asked once.
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

fn closed_at(entry: &Value) -> i64 {
    entry.get("closed_at_ms").and_then(|v| v.as_i64()).unwrap_or(0)
}

/// Whether a workspace record entry is a closed window's.
pub(crate) fn is_closed(entry: &Value) -> bool {
    entry.get("state").and_then(|v| v.as_str()) == Some(STATE_CLOSED)
}

fn client(store: &Store) -> Option<Client> {
    store.get_all::<Client>().ok().and_then(|cs| cs.into_iter().next())
}

fn entries(store: &Store) -> Vec<Value> {
    client(store)
        .and_then(|c| c.meta.get(CLOSED_META_KEY).and_then(|v| v.as_array()).cloned())
        .unwrap_or_default()
}

/// Read, change and write back the list in one transaction: closes, reopens
/// and deletes run concurrently. A change counts as one to the workspace's
/// windows, so the saver saves it. `None` when the store couldn't be written.
fn update<R>(store: &Store, f: impl FnOnce(&mut Vec<Value>) -> R) -> Option<R> {
    let result = store.with_tx(|tx| {
        let Some(mut client) = tx.get_all::<Client>()?.into_iter().next() else {
            return Ok(None);
        };
        let mut list = client.meta.get(CLOSED_META_KEY).and_then(|v| v.as_array()).cloned().unwrap_or_default();
        let before = list.clone();
        let out = f(&mut list);
        let changed = list != before;
        if changed {
            if list.is_empty() {
                client.meta.remove(CLOSED_META_KEY);
            } else {
                client.meta.insert(CLOSED_META_KEY.to_string(), Value::Array(list));
            }
            tx.update(&mut client)?;
        }
        Ok(Some((out, changed)))
    });
    match result {
        Ok(Some((out, changed))) => {
            if changed {
                store.layout_writes().wrote("window");
            }
            Some(out)
        }
        Ok(None) => None,
        Err(e) => {
            tracing::warn!(error = %e, "session_closed: couldn't update the closed windows");
            None
        }
    }
}

/// The window contents a window shows now.
pub(crate) fn open_workspaces(store: &Store) -> HashSet<String> {
    client(store)
        .map(|c| c.windowids)
        .unwrap_or_default()
        .iter()
        .filter_map(|id| store.get::<Window>(id).ok().flatten())
        .map(|w| w.workspaceid)
        .collect()
}

/// How many full windows are open (a subwindow belongs to its parent's).
fn open_window_count(store: &Store) -> usize {
    client(store)
        .map(|c| c.windowids)
        .unwrap_or_default()
        .iter()
        .filter_map(|id| store.get::<Window>(id).ok().flatten())
        .filter(|w| w.kind.as_deref() != Some("subwindow"))
        .count()
}

/// Whether `ws_id` holds anything of the user's: a tab with a pane that isn't
/// the untouched default layout.
fn worth_keeping(store: &Store, ws_id: &str) -> bool {
    super::session_restore::snapshot_workspace(store, ws_id)
        .and_then(|snap| super::session_other_builds::summarize(&snap, "", "", 0))
        .is_some()
}

/// List `ws_id` as closed at `closed_at_ms`, its window at `window`, keeping
/// the list newest first.
fn remember(store: &Store, ws_id: &str, window: Value, closed_at_ms: i64) {
    let entry = json!({
        "workspaceid": ws_id,
        "closed_at_ms": closed_at_ms,
        "window": window,
    });
    update(store, |list| {
        list.retain(|e| workspace_of(e) != Some(ws_id));
        list.push(entry);
        list.sort_by_key(|e| std::cmp::Reverse(closed_at(e)));
    });
}

/// Name the agent blocks a closed window parked, on its entry: what a reopen puts back.
fn set_parked(store: &Store, ws_id: &str, parked: &[String]) {
    if parked.is_empty() {
        return;
    }
    update(store, |list| {
        if let Some(entry) = list.iter_mut().find(|e| workspace_of(e) == Some(ws_id)) {
            entry["parked"] = json!(parked);
        }
    });
}

/// The block ids a closed window's entry names as parked at its close.
pub(crate) fn parked_of(entry: &Value) -> Vec<String> {
    entry
        .get("parked")
        .and_then(|v| v.as_array())
        .map(|ids| ids.iter().filter_map(|id| id.as_str().map(str::to_string)).collect())
        .unwrap_or_default()
}

/// The agent blocks a closed window parked that are still parked, each with the tab it was parked
/// from. The window's list entry counts them: parked, they are in none of its tabs.
pub(crate) fn parked_blocks(store: &Store, ws_id: &str) -> Vec<(String, String)> {
    use crate::persist_subscriber::parked::{PARKED_FROM_TAB_META_KEY, PARKED_META_KEY};
    let Some(entry) = entries(store).into_iter().find(|e| workspace_of(e) == Some(ws_id)) else {
        return Vec::new();
    };
    parked_of(&entry)
        .into_iter()
        .filter_map(|id| {
            let block = store.get::<Block>(&id).ok().flatten()?;
            if !crate::backend::obj::meta_get_bool(&block.meta, PARKED_META_KEY, false) {
                return None;
            }
            let tab = crate::backend::obj::meta_get_string(&block.meta, PARKED_FROM_TAB_META_KEY, "");
            (!tab.is_empty()).then_some((id, tab))
        })
        .collect()
}

/// `ws_id` is open again, or deleted: drop it from the list. Returns its entry.
pub(crate) fn forget(store: &Store, ws_id: &str) -> Option<Value> {
    update(store, |list| {
        let at = list.iter().position(|e| workspace_of(e) == Some(ws_id))?;
        Some(list.remove(at))
    })
    .flatten()
}

/// Whether a quit is closing the windows: its record is the session record,
/// saved within the last two minutes.
pub(crate) fn quit_under_way(store: &Store) -> bool {
    super::session_restore::load_last_session_snapshot(store)
        .is_some_and(|r| super::session_windows::is_recent_quit_record(&r, now_ms()))
}

/// A window onto `ws_id` closed, and no other window shows it (`window`: where
/// it was). Stop what runs in it and keep its contents, listed as a closed
/// window unless `unlisted`, or a quit is closing every window: then the
/// window stays open in its workspace's record and opens again with it. A
/// listed window's agents are not stopped but parked (`agents:always_available`),
/// and its record names them, so a reopen puts them back.
/// `unlisted` is the close that ends the session (it emptied
/// `Client.windowids`) and a workspace switch's closes. Contents with nothing
/// of the user's in them are deleted instead.
pub(crate) async fn keep_workspace(state: &AppState, ws_id: String, window: Value, unlisted: bool) {
    if !worth_keeping(&state.mstore, &ws_id) {
        super::window_close::close_workspace(state, ws_id).await;
        return;
    }
    // Held until what runs in it has stopped and its agents are parked: a reopen waits for it
    // (`TakeClosedWindow`), so it can't start them while they are stopping.
    let guard = crate::sagas::close_pane::workspace_close_lock(&ws_id).await;
    let listed = !unlisted && !quit_under_way(&state.mstore);
    if listed {
        remember(&state.mstore, &ws_id, window, now_ms());
    }
    tracing::info!(
        workspace_id = %ws_id,
        listed,
        "CloseWindow: keeping the window's contents (its agents {}; {})",
        if listed { "keep running" } else { "stop" },
        if listed { "a closed window of the workspace" } else { "it opens again with its workspace" }
    );
    let st = state.clone();
    let done = crate::sagas::agent_teardown::detached(async move {
        let _guard = guard;
        let block_ids = super::window_close::store_block_ids(&st.mstore, &ws_id);
        // Only a window that stays listed leaves its agents running: one that ends the session or
        // switches workspace opens again with its panes, and its agents are stopped for that.
        let parked = if listed { crate::sagas::park_agents::park_window_agents(&st, &ws_id, &block_ids).await } else { Vec::new() };
        set_parked(&st.mstore, &ws_id, &parked);
        let stopping = crate::sagas::park_agents::not_parked(&block_ids, &parked);
        crate::sagas::agent_teardown::run_many(&st, &stopping, crate::sagas::agent_teardown::Policy::close()).await;
        // Lifts each block's respawn guard; the blocks stay, so nothing is purged.
        for block_id in &block_ids {
            crate::sagas::close_pane::finish_close(&st, block_id).await;
        }
    })
    .await;
    if done.is_none() {
        tracing::warn!("CloseWindow: stopping a kept window's agents panicked");
    }
}

/// The closed windows as workspace record entries, newest first: each one's
/// contents (`snapshot_workspace`), where its window was (`window`), its
/// `workspaceid`, `"state": "closed"` and `closed_at_ms`. One whose contents
/// are gone or shown by a window is left out.
pub(crate) fn closed_entries(store: &Store) -> Vec<Value> {
    let open = open_workspaces(store);
    entries(store)
        .iter()
        .filter_map(|e| {
            let ws = workspace_of(e)?;
            if open.contains(ws) {
                return None;
            }
            let mut entry = match super::session_restore::snapshot_workspace(store, ws) {
                Some(snap) => snap,
                // A window of agents only has no block in any tab once they are parked: still a closed window.
                None if !parked_blocks(store, ws).is_empty() => json!({ "tabs": [], "active_tab_index": null }),
                None => return None,
            };
            let parked = parked_of(e);
            if !parked.is_empty() {
                entry["parked"] = json!(parked);
            }
            entry["window"] = e.get("window").cloned().unwrap_or_else(|| json!({}));
            entry["workspaceid"] = json!(ws);
            entry["state"] = json!(STATE_CLOSED);
            entry["closed_at_ms"] = json!(closed_at(e));
            Some(entry)
        })
        .collect()
}

/// A session record (the open windows) as the workspace's record: each of its
/// windows marked `"state": "open"`, then the closed windows
/// ([`closed_entries`]) it doesn't already hold. A record with closed entries
/// already comes back the same.
pub(crate) fn with_closed(store: &Store, mut record: Value) -> Value {
    if !record.is_object() {
        return record;
    }
    let mut windows = super::session_windows::windows_of(&record);
    for w in &mut windows {
        if w.get("state").is_none() {
            w["state"] = json!(STATE_OPEN);
        }
    }
    let held: HashSet<String> = windows.iter().filter_map(|w| workspace_of(w).map(str::to_string)).collect();
    windows.extend(
        closed_entries(store)
            .into_iter()
            .filter(|e| !matches!(workspace_of(e), Some(ws) if held.contains(ws))),
    );
    record["windows"] = Value::Array(windows);
    record
}

/// A cold start's closed windows: `record`'s `"state": "closed"` entries,
/// kept closed. An entry whose contents are still here (its `workspaceid`, not
/// shown by a window, not `main_ws`, not waiting to open) is listed as it is;
/// one whose contents are gone (a new version's first start, which copied the
/// record) is rebuilt from the entry, with new ids. No window opens for
/// either. An entry naming contents in `skip` (an open window's in the record
/// that was restored) is left out. Returns how many are listed.
pub(crate) async fn restore_closed(state: &AppState, record: &Value, main_ws: &str, skip: &HashSet<String>) -> usize {
    let store = &state.mstore;
    let mut busy = open_workspaces(store);
    busy.extend(super::session_windows::waiting_workspaces(store));
    busy.insert(main_ws.to_string());
    let mut listed = 0usize;
    let mut rebuilt = 0usize;
    for entry in super::session_windows::windows_of(record).iter().filter(|w| is_closed(w)) {
        if workspace_of(entry).is_some_and(|ws| skip.contains(ws)) {
            continue;
        }
        let here = match workspace_of(entry).filter(|ws| !busy.contains(*ws)) {
            Some(ws) => {
                let known = state.srv_state.lock().await.workspaces.contains_key(ws);
                (known && (super::session_restore::snapshot_workspace(store, ws).is_some() || !parked_of(entry).is_empty()))
                    .then(|| ws.to_string())
            }
            None => None,
        };
        // Contents that are still here still have their parked agents; a rebuilt window has new tabs.
        let carried = if here.is_some() { parked_of(entry) } else { Vec::new() };
        let ws_id = match here {
            Some(ws) => ws,
            None => match super::session_restore::restore_workspace(state, entry).await {
                Ok(Some((ws, events))) => {
                    super::reducer_helpers::publish_events(state, &events);
                    rebuilt += 1;
                    ws
                }
                Ok(None) => {
                    tracing::warn!("session_closed: a closed window's tabs didn't come back; skipping it");
                    continue;
                }
                Err(e) => {
                    tracing::warn!(error = %e, "session_closed: a closed window didn't come back");
                    continue;
                }
            },
        };
        busy.insert(ws_id.clone());
        let window = entry.get("window").cloned().unwrap_or_else(|| json!({}));
        remember(store, &ws_id, window, closed_at(entry));
        set_parked(store, &ws_id, &carried);
        listed += 1;
    }
    if listed > 0 {
        tracing::info!(listed, rebuilt, "session_closed: kept {} closed window(s) closed ({} rebuilt)", listed, rebuilt);
    }
    listed
}

/// The tabs and panes a window snapshot holds.
fn snapshot_counts(snap: &Value) -> (u32, u32) {
    let tabs = snap.get("tabs").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    let panes = tabs.iter().map(|t| t.get("blocks").and_then(|b| b.as_array()).map_or(0, |b| b.len())).sum::<usize>();
    (tabs.len() as u32, panes as u32)
}

/// What a closed window's parked agents add to its list entry: the tabs they alone keep, and their names.
fn parked_summary(store: &Store, parked: &[(String, String)]) -> (u32, Vec<String>) {
    use crate::backend::obj::{meta_get_string, Tab};
    let mut alone = HashSet::new();
    let mut names = Vec::new();
    for (block_id, tab_id) in parked {
        if store.get::<Tab>(tab_id).ok().flatten().is_some_and(|t| t.blockids.is_empty()) {
            alone.insert(tab_id.clone());
        }
        if let Ok(Some(block)) = store.get::<Block>(block_id) {
            let name = meta_get_string(&block.meta, "agentName", "");
            let name = if name.is_empty() { meta_get_string(&block.meta, "agentId", "") } else { name };
            if !name.is_empty() && !names.contains(&name) {
                names.push(name);
            }
        }
    }
    (alone.len() as u32, names)
}

/// `window.ListClosedWindows`: the open workspace's closed windows, newest
/// first, each `{workspaceid, closed_at_ms, name, tabs, panes, agents, running}`
/// for the status bar panel (`running`: its agents that keep running, parked). One whose contents are gone or shown again is left
/// out.
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
            let parked = parked_blocks(store, ws_id);
            let snap = super::session_restore::snapshot_workspace(store, ws_id);
            if snap.is_none() && parked.is_empty() {
                return None;
            }
            let summary = snap.as_ref().and_then(|s| super::session_other_builds::summarize(s, "", "", 0));
            let (parked_tabs, mut agents) = parked_summary(store, &parked);
            // `summarize` says nothing of a window with only its default panes; with parked agents it is
            // not that window, so its tabs and panes are counted as they are.
            let (tabs, panes) = match (&summary, &snap) {
                (Some(s), _) => (s.tabs, s.panes),
                (None, Some(snap)) if !parked.is_empty() => snapshot_counts(snap),
                _ => (0, 0),
            };
            if let Some(s) = &summary {
                agents.splice(0..0, s.agents.iter().cloned());
            }
            let name = e
                .get("window")
                .and_then(|w| w.get("name"))
                .and_then(|v| v.as_str())
                .map(str::to_string)
                .unwrap_or(ws.name);
            Some(json!({
                "workspaceid": ws_id,
                "closed_at_ms": closed_at(e),
                "name": name,
                "tabs": tabs + parked_tabs,
                "panes": panes + parked.len() as u32,
                "agents": agents,
                "running": parked.len(),
            }))
        })
        .collect();
    WebReturnType::success(Value::Array(list))
}

/// `window.TakeClosedWindow(workspaceId)`: reopen a closed window. Waits for
/// its close to finish, puts its parked agents back in their tabs and returns where it was (`{workspaceid, x,
/// y, width, height}`) for the host to open it there; its name and appearance
/// wait for that window (`CreateWindow`). It stays listed until that window
/// registers, so a window that fails to open can be reopened again and is
/// still saved (#5359). Errors when it isn't a closed window any more.
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
    let Some(entry) = entries(store).into_iter().find(|e| workspace_of(e) == Some(ws_id.as_str())) else {
        return WebReturnType::error("that window isn't among the closed windows any more");
    };
    if store.get::<Workspace>(&ws_id).ok().flatten().is_none() {
        return WebReturnType::error("that window's contents are gone");
    }
    // Its agents kept running without it: back into their tabs before the window shows them.
    let back = crate::sagas::park_agents::reattach_window_agents(state, &ws_id, &parked_of(&entry)).await;
    if back > 0 {
        tracing::info!(workspace_id = %ws_id, agents = back, "TakeClosedWindow: the window's agents are back in its tabs");
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

/// `window.DeleteClosedWindow(workspaceId)`: remove a closed window from the
/// workspace without reopening it. Its contents are deleted; its agents stay
/// in My Agents.
pub(crate) async fn handle_delete_closed_window(state: &AppState, call: &WebCallType) -> WebReturnType {
    let ws_id: String = match service::get_arg(&call.args, 0) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    // A close still stopping its agents finishes first.
    drop(crate::sagas::close_pane::workspace_close_lock(&ws_id).await);
    let store = &state.mstore;
    if open_workspaces(store).contains(&ws_id) {
        return WebReturnType::error("that window is open");
    }
    if forget(store, &ws_id).is_none() {
        return WebReturnType::error("that window isn't among the closed windows any more");
    }
    tracing::info!(workspace_id = %ws_id, "session_closed: removing a closed window from the workspace");
    super::window_close::close_workspace(state, ws_id).await;
    WebReturnType::success_empty()
}

/// `window.DiscardWorkspaceOnClose(windowId)`: remove an open window from the
/// workspace. Its close, which the window starts next, deletes its contents
/// instead of keeping them. The only open window can't be removed.
pub(crate) fn handle_discard_workspace_on_close(state: &AppState, call: &WebCallType) -> WebReturnType {
    let window_id: String = match service::get_arg(&call.args, 0) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    if open_window_count(&state.mstore) <= 1 {
        return WebReturnType::error("the last window can't be removed from the workspace");
    }
    mark_discard(&window_id);
    tracing::info!(window_id = %window_id, "CloseWindow: removing the window from the workspace; its close deletes its contents");
    WebReturnType::success_empty()
}

/// `window.WindowAgents(windowId)`: the agents running in the window, each
/// `{block_id, name, working}` (`working`: mid-turn), for the close warning
/// and the panel's delete.
pub(crate) fn handle_window_agents(state: &AppState, call: &WebCallType) -> WebReturnType {
    let window_id: String = match service::get_arg(&call.args, 0) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    let store = &state.mstore;
    let Some(window) = store.get::<Window>(&window_id).ok().flatten() else {
        return WebReturnType::success(json!([]));
    };
    WebReturnType::success(Value::Array(agents_in(store, &window.workspaceid)))
}

/// The agents running in window contents `ws_id`, each `{block_id, name,
/// working}` (`working`: mid-turn).
pub(crate) fn agents_in(store: &Store, ws_id: &str) -> Vec<Value> {
    super::window_close::store_block_ids(store, ws_id)
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
        .collect()
}

#[cfg(test)]
#[path = "session_closed_tests.rs"]
mod tests;

#[cfg(test)]
#[path = "session_closed_agents_tests.rs"]
mod agent_tests;
