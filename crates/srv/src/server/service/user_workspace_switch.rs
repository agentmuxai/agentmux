// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Several workspaces, one open at a time (docs/specs/SPEC_WORKSPACES_2026_10_23.md
//! §3.2, §6): list them, make a new one, switch, delete one. Not the srv
//! `Workspace` object, which is one window's contents.
//!
//! **Switching** (`workspace.SwitchUserWorkspace`, and the switch that ends
//! `CreateUserWorkspace` and Import…), all in srv:
//! 1. the open workspace is saved, then saves wait (`LayoutWrites::hold_saves`)
//!    until the windows have moved, so no save stores a mix of the two;
//! 2. its windows other than the one the user acted from close here: their
//!    rows go, their contents stay, and their windows stay `open` in its
//!    record. The host closes the OS windows afterwards (the reply names them);
//! 3. the other workspace becomes the open one (`workspace:current`,
//!    `workspace:name`, its `opened_at`), and the closed windows, the restored
//!    windows waiting to open and the session record of the one before are
//!    forgotten: its record holds them;
//! 4. its record is restored as a cold start restores one
//!    (`session_windows::restore_all`): contents still here are reused, those
//!    that are gone are rebuilt from the record, its closed windows are listed
//!    closed. A record with nothing to restore (a new workspace) gets the
//!    default panes;
//! 5. the window the user acted from shows the first window's contents, with
//!    its name and appearance; the rest wait for that window to open them
//!    (`TakeRestoredWindows`);
//! 6. the contents the closed workspace showed have their agents and shells
//!    stopped, after the reply, one switch at a time. The next switch waits
//!    for that.
//!
//! The next launch opens the workspace that was open last: it is the one the
//! client names.

use std::sync::{Arc, LazyLock};

use agentmux_common::ipc::{Command, Event};
use agentmux_common::time::now_ms;
use serde_json::{json, Value};

use crate::backend::obj::{mux_obj_to_value, Client, MuxObjUpdate, Window};
use crate::backend::service::{self, WebCallType, WebReturnType};
use crate::backend::storage::store::Store;
use crate::backend::storage::user_workspaces::UserWorkspaceRow;
use crate::server::AppState;

use super::reducer_helpers::{dispatch_to_reducer, publish_events};
use super::user_workspace::{clean_name, current_id, Saved, CURRENT_META_KEY, NAME_META_KEY};

/// One switch at a time, held until the closed workspace's agents have
/// stopped: the next switch may reopen those contents.
static SWITCH_LOCK: LazyLock<Arc<tokio::sync::Mutex<()>>> = LazyLock::new(Default::default);

/// Wait until no switch is running, its agents' stop included.
#[cfg(test)]
pub(crate) async fn wait_for_switch() {
    drop(SWITCH_LOCK.lock().await);
}

/// Holds saves for as long as it lives.
struct SavesHeld<'a>(&'a Store);

impl<'a> SavesHeld<'a> {
    fn new(store: &'a Store) -> Self {
        store.layout_writes().hold_saves(true);
        SavesHeld(store)
    }
}

impl Drop for SavesHeld<'_> {
    fn drop(&mut self) {
        self.0.layout_writes().hold_saves(false);
    }
}

/// What a switch did, for the window that asked.
#[derive(Debug, Clone)]
pub(crate) struct Switched {
    pub id: String,
    pub name: String,
    /// The windows that closed (`window_id`, host label), for the host to close.
    pub closed: Vec<(String, String)>,
    /// How many windows wait to open beside the one that asked.
    pub beside: usize,
}

impl Switched {
    pub(crate) fn to_json(&self) -> Value {
        json!({
            "id": self.id,
            "name": self.name,
            "closed": self.closed.iter().map(|(w, l)| json!({ "window_id": w, "label": l })).collect::<Vec<_>>(),
            "beside": self.beside,
        })
    }
}

fn client(store: &Store) -> Option<Client> {
    store.get_all::<Client>().ok()?.into_iter().next()
}

/// The client, whose meta names the open workspace, as an object update.
pub(crate) fn client_update(store: &Store) -> Vec<MuxObjUpdate> {
    client(store)
        .map(|c| {
            vec![MuxObjUpdate {
                updatetype: "update".into(),
                otype: "client".into(),
                oid: c.oid.clone(),
                obj: Some(mux_obj_to_value(&c)),
            }]
        })
        .unwrap_or_default()
}

/// Apply `events` to the store and publish them; the first error the reducer
/// returned, if any, instead.
async fn apply(state: &AppState, events: &[Event]) -> Result<(), String> {
    if let Some(err) = events.iter().find_map(|e| match e {
        Event::Error { message, .. } => Some(message.clone()),
        _ => None,
    }) {
        return Err(err);
    }
    for ev in events {
        crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore).map_err(|e| e.to_string())?;
    }
    publish_events(state, events);
    Ok(())
}

/// Make `row` the open workspace: the client names it, and forgets what
/// belonged to the one before (its closed windows, its restored windows
/// waiting to open, the session record), which that one's record holds.
fn set_current(store: &Store, row: &UserWorkspaceRow) -> Result<(), String> {
    store
        .with_tx(|tx| {
            let Some(mut client) = tx.get_all::<Client>()?.into_iter().next() else { return Ok(()) };
            client.meta.insert(CURRENT_META_KEY.to_string(), json!(row.id));
            client.meta.insert(NAME_META_KEY.to_string(), json!(row.name));
            client.meta.remove(super::session_closed::CLOSED_META_KEY);
            super::session_windows::forget_restore_lists(&mut client.meta);
            tx.update(&mut client)?;
            Ok(())
        })
        .map_err(|e| e.to_string())?;
    super::session_restore::clear_last_session_snapshot(store);
    let _ = store.user_workspace_mark_opened(&row.id, now_ms());
    Ok(())
}

/// The meta patch that makes a window the one a record entry describes: its
/// name, theme and opacity, and none of the window's own where the entry has
/// none.
fn window_patch(waiting: serde_json::Map<String, Value>) -> Value {
    let mut patch = serde_json::Map::new();
    for key in std::iter::once("window:displayname").chain(super::window_inherit::INHERITED_KEYS.iter().copied()) {
        patch.insert(key.to_string(), waiting.get(key).cloned().unwrap_or(Value::Null));
    }
    Value::Object(patch)
}

/// Switch to workspace `target_id`, reusing window `window_id` for its first
/// window. See the module doc for the steps.
pub(crate) async fn switch_to(state: &AppState, target_id: &str, window_id: &str) -> Result<Switched, String> {
    let guard = SWITCH_LOCK.clone().lock_owned().await;
    let store = &state.mstore;
    let target = store
        .user_workspace_get(target_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "that workspace doesn't exist any more".to_string())?;
    if current_id(store).as_deref() == Some(target_id) {
        return Err(format!("{} is open already", target.name));
    }
    let acting = store
        .get::<Window>(window_id)
        .map_err(|e| e.to_string())?
        .filter(|w| client(store).is_some_and(|c| c.windowids.contains(&w.oid)))
        .ok_or_else(|| "the window that asked isn't open".to_string())?;

    // 1. Save the open workspace as it is now.
    let saving = Arc::clone(store);
    let saved = tokio::task::spawn_blocking(move || super::user_workspace::save_now(&saving))
        .await
        .map_err(|e| e.to_string())?;
    match saved {
        Saved::Written | Saved::Unchanged | Saved::NoWindow | Saved::NoWorkspace => {}
        Saved::HeldForQuit => return Err("AgentMux is quitting".to_string()),
        Saved::HeldForSwitch | Saved::Failed => return Err("the open workspace couldn't be saved".to_string()),
    }
    let held = SavesHeld::new(store);

    // 2. Its other windows close; their contents stay.
    let mut leaving: Vec<String> = Vec::new();
    let mut closed = Vec::new();
    for (w, _) in super::session_windows::live_windows(store) {
        if w.oid == acting.oid {
            continue;
        }
        let label = w.meta.get("host:label").and_then(|v| v.as_str()).unwrap_or_default().to_string();
        if let Some(ws) = super::window_close::detach_window(state, &w.oid).await? {
            leaving.push(ws);
        }
        closed.push((w.oid.clone(), label));
    }

    // 3. The other workspace is the open one.
    set_current(store, &target)?;

    // 4. Its windows come back, or the default panes for a new one.
    let mut opened = None;
    for (from, record) in super::user_workspace::launch_candidates(None, Some(&target)) {
        match super::session_windows::restore_all(state, &record).await {
            Ok(Some(main)) => {
                opened = Some((main, record, from));
                break;
            }
            Ok(None) => tracing::warn!(copy = ?from, "user_workspace: nothing came back from a saved copy"),
            Err(e) => tracing::warn!(copy = ?from, error = %e, "user_workspace: a saved copy didn't restore"),
        }
    }
    let main_ws = match opened {
        Some(((main_ws, events), record, from)) => {
            publish_events(state, &events);
            super::user_workspace::after_clean_load(store, &record, from);
            main_ws
        }
        None => {
            let (ws, events) = super::window_create::create_default_contents(state, None, None, false).await?;
            publish_events(state, &events);
            ws
        }
    };

    // 5. The window that asked shows its first window.
    apply(state, &dispatch_to_reducer(state, Command::SwitchWorkspace { window_id: acting.oid.clone(), workspace_id: main_ws.clone() }).await)
        .await?;
    let patch = window_patch(super::session_windows::restored_window_meta(store, &main_ws));
    match apply(state, &dispatch_to_reducer(state, Command::UpdateWindowMeta { window_id: acting.oid.clone(), meta_patch: patch }).await).await {
        Ok(()) => {
            super::session_windows::forget_restored_window_meta(store, &main_ws);
        }
        Err(e) => tracing::warn!(error = %e, "user_workspace: the window's name and appearance didn't follow the switch"),
    }
    super::session_windows::restored_window_opened(store, &main_ws);
    super::session_closed::forget(store, &main_ws);
    if acting.workspaceid != main_ws {
        leaving.push(acting.workspaceid.clone());
    }

    drop(held);
    let saving = Arc::clone(store);
    let _ = tokio::task::spawn_blocking(move || super::user_workspace::save_now(&saving)).await;

    // 6. The closed workspace's agents and shells stop, after the reply.
    let st = state.clone();
    tokio::spawn(async move {
        for ws in leaving {
            super::session_closed::keep_workspace(&st, ws, json!({}), true).await;
        }
        drop(guard);
    });

    let beside = super::session_windows::waiting_workspaces(store).len();
    tracing::info!(workspace = %target.id, name = %target.name, closed = closed.len(), beside, "user_workspace: switched");
    Ok(Switched { id: target.id, name: target.name, closed, beside })
}

// ---- Service handlers ----

fn arg_str(call: &WebCallType, i: usize) -> String {
    service::get_arg::<String>(&call.args, i).unwrap_or_default()
}

fn reply(state: &AppState, result: Result<Switched, String>) -> WebReturnType {
    match result {
        Ok(switched) => WebReturnType::success_data_updates(switched.to_json(), client_update(&state.mstore)),
        Err(e) => WebReturnType::error(e),
    }
}

/// The windows a record holds, open and closed.
fn window_counts(record: Option<&str>) -> (usize, usize) {
    let Some(record) = record.and_then(|r| serde_json::from_str::<Value>(r).ok()) else { return (0, 0) };
    let windows = super::session_windows::windows_of(&record);
    let windows: Vec<&Value> = windows.iter().filter(|w| w.get("tabs").and_then(|t| t.as_array()).is_some_and(|t| !t.is_empty())).collect();
    let closed = windows.iter().filter(|w| super::session_closed::is_closed(w)).count();
    (windows.len() - closed, closed)
}

/// `workspace.ListUserWorkspaces`: every workspace, the most recently opened
/// first, each `{id, name, current, opened_at, windows, closed}` (its open
/// and closed windows, as last saved).
pub(crate) fn handle_list(state: &AppState, _call: &WebCallType) -> WebReturnType {
    let store = &state.mstore;
    let current = current_id(store);
    match store.user_workspace_list() {
        Ok(rows) => WebReturnType::success(Value::Array(
            rows.iter()
                .map(|w| {
                    let (windows, closed) = window_counts(w.record.as_deref());
                    json!({
                        "id": w.id,
                        "name": w.name,
                        "current": current.as_deref() == Some(w.id.as_str()),
                        "opened_at": w.opened_at,
                        "windows": windows,
                        "closed": closed,
                    })
                })
                .collect(),
        )),
        Err(e) => WebReturnType::error(e.to_string()),
    }
}

/// `workspace.SwitchUserWorkspace(workspaceId, windowId)`: switch to another
/// workspace from window `windowId`. Returns `{id, name, closed: [{window_id,
/// label}], beside}`: the windows for the host to close, and how many wait to
/// open beside this one (`TakeRestoredWindows`).
pub(crate) async fn handle_switch(state: &AppState, call: &WebCallType) -> WebReturnType {
    reply(state, switch_to(state, &arg_str(call, 0), &arg_str(call, 1)).await)
}

/// A new, empty workspace named `name`.
pub(crate) fn insert_workspace(store: &Store, name: &str) -> Result<UserWorkspaceRow, String> {
    let name = clean_name(name).ok_or_else(|| "a workspace needs a name".to_string())?;
    let now = now_ms();
    let row = UserWorkspaceRow {
        id: uuid::Uuid::new_v4().to_string(),
        name,
        created_at: now,
        updated_at: now,
        ..Default::default()
    };
    store.user_workspace_insert(&row).map_err(|e| e.to_string())?;
    Ok(row)
}

/// `workspace.CreateUserWorkspace(name, windowId)`: a new workspace, switched
/// to at once; its first window shows the default panes. The same reply as
/// `SwitchUserWorkspace`. A switch that fails leaves no workspace behind.
pub(crate) async fn handle_create(state: &AppState, call: &WebCallType) -> WebReturnType {
    let store = &state.mstore;
    let row = match insert_workspace(store, &arg_str(call, 0)) {
        Ok(row) => row,
        Err(e) => return WebReturnType::error(e),
    };
    let result = switch_to(state, &row.id, &arg_str(call, 1)).await;
    if result.is_err() {
        let _ = store.user_workspace_delete(&row.id);
    }
    reply(state, result)
}

/// The window contents workspace `row` keeps (its record's), less any that a
/// window shows, that the open workspace lists as closed, or that wait to open.
fn kept_contents(store: &Store, row: &UserWorkspaceRow) -> Vec<String> {
    let mut busy = super::session_closed::open_workspaces(store);
    busy.extend(super::session_windows::waiting_workspaces(store));
    busy.extend(
        super::session_closed::closed_entries(store)
            .iter()
            .filter_map(|e| e.get("workspaceid").and_then(|v| v.as_str()).map(str::to_string)),
    );
    let mut out = Vec::new();
    for text in [row.record.as_deref(), row.previous.as_deref()].into_iter().flatten() {
        let Ok(record) = serde_json::from_str::<Value>(text) else { continue };
        for w in super::session_windows::windows_of(&record) {
            if let Some(ws) = w.get("workspaceid").and_then(|v| v.as_str()).filter(|s| !s.is_empty()) {
                if !busy.contains(ws) && !out.iter().any(|o| o == ws) {
                    out.push(ws.to_string());
                }
            }
        }
    }
    out
}

/// `workspace.DeleteUserWorkspace(workspaceId)`: delete a workspace that
/// isn't open: its row, and the contents of its windows still kept here. Its
/// agents stay in My Agents.
pub(crate) async fn handle_delete(state: &AppState, call: &WebCallType) -> WebReturnType {
    let store = &state.mstore;
    let id = arg_str(call, 0);
    if current_id(store).as_deref() == Some(id.as_str()) {
        return WebReturnType::error("the open workspace can't be deleted; open another one first");
    }
    let row = match store.user_workspace_get(&id) {
        Ok(Some(row)) => row,
        Ok(None) => return WebReturnType::error("that workspace doesn't exist any more"),
        Err(e) => return WebReturnType::error(e.to_string()),
    };
    // Its contents may still be stopping after the switch away from it.
    let _switching = SWITCH_LOCK.lock().await;
    let contents = kept_contents(store, &row);
    if let Err(e) = store.user_workspace_delete(&id) {
        return WebReturnType::error(e.to_string());
    }
    for ws in &contents {
        if store.get::<crate::backend::obj::Workspace>(ws).ok().flatten().is_some() {
            super::window_close::close_workspace(state, ws.clone()).await;
        }
    }
    tracing::info!(workspace = %id, name = %row.name, contents = contents.len(), "user_workspace: deleted");
    WebReturnType::success_empty()
}

/// `workspace.UserWorkspaceAgents()`: the agents running in the open
/// workspace's windows, each `{block_id, name, working}`, for the warning
/// before a switch closes it.
pub(crate) fn handle_agents(state: &AppState, _call: &WebCallType) -> WebReturnType {
    let store = &state.mstore;
    let agents: Vec<Value> = super::session_windows::live_windows(store)
        .iter()
        .flat_map(|(_, ws)| super::session_closed::agents_in(store, ws))
        .collect();
    WebReturnType::success(Value::Array(agents))
}

#[cfg(test)]
#[path = "user_workspace_switch_tests.rs"]
pub(crate) mod tests;
