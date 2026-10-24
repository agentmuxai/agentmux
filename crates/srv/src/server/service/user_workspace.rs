// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The open workspace: a named set of windows, saved as it changes
//! (docs/specs/SPEC_WORKSPACES_2026_10_23.md §2, §4, §8). Not the srv
//! `Workspace` object, which is one window's contents.
//!
//! - **Which one is open:** `Client.meta["workspace:current"]` (its id) and
//!   `Client.meta["workspace:name"]` (its name, for the status bar panel). A
//!   start with no workspace makes **Default Workspace**; a new version's first
//!   start copies the previous version's instead ([`prepare_at_startup`]).
//! - **Saving:** every write to a window, a window's contents, a tab, a layout
//!   or a block counts as a change (`LayoutWrites`, bumped by the store). The
//!   saver ([`spawn_saver`]) writes the whole session
//!   (`session_windows::snapshot_session`) as the workspace's `record` 1 s after
//!   the last change, and at least every 15 s while changes keep coming. A quit,
//!   an OS shutdown and the last window's close write it at once
//!   ([`write_record_now`]), and so does a window losing focus
//!   (`workspace.Flush`). A write that would store what is already there is
//!   skipped; one with no window open is never made. Each window in the
//!   record has a `state`: `open`, or `closed` for the windows the user closed
//!   (`session_closed`), which a cold start keeps closed.
//! - **Loading:** a cold start restores the newer of the session record and the
//!   workspace's record, then falls back to the last record that loaded
//!   cleanly (`previous`) and the copy made before an upgrade (`pre_upgrade`)
//!   ([`restore_at_launch`]). Only a clean load replaces `previous`.
//!
//! The session record (`session:last_topology`) is still written as before:
//! the version after this one reads it if it has no workspace yet.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use agentmux_common::ipc::Event;
use agentmux_common::time::now_ms;
use serde_json::{json, Value};

use crate::backend::obj::{mux_obj_to_value, Client, MuxObjUpdate};
use crate::backend::service::{self, WebCallType, WebReturnType};
use crate::backend::storage::store::Store;
use crate::backend::storage::user_workspaces::{self, UserWorkspaceRow};
use crate::server::AppState;

use super::session_other_builds::{newest_other_version_db, open_other_db, ScanRoots};
use super::session_windows::{is_recent_quit_record, set_client_meta, windows_of};

/// The open workspace's id.
pub(crate) const CURRENT_META_KEY: &str = "workspace:current";
/// The open workspace's name.
pub(crate) const NAME_META_KEY: &str = "workspace:name";
/// Set when a launch had to load an older copy of the workspace: `previous`
/// or `pre_upgrade`. The window says so once and clears it.
pub(crate) const RESTORED_FROM_META_KEY: &str = "workspace:restored_from";

/// The workspace everyone starts in.
pub(crate) const DEFAULT_NAME: &str = "Default Workspace";
/// The longest name kept, in characters (a window's name has the same cap).
const NAME_MAX_CHARS: usize = 64;

/// How often the saver looks for changes.
const TICK: Duration = Duration::from_millis(250);
/// A change is saved this long after the last one...
pub(crate) const DEBOUNCE_MS: i64 = 1_000;
/// ...and no later than this after the first one not yet saved.
pub(crate) const CEILING_MS: i64 = 15_000;
/// A change to blocks' settings alone (an agent pane writes them as it works)
/// waits this long instead; a close, a quit or focus loss saves it at once.
pub(crate) const SETTINGS_ONLY_MS: i64 = 30_000;

/// One save at a time: the saver, a flush and a boundary write never
/// interleave their read-compare-write.
static WRITE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());



fn client(store: &Store) -> Option<Client> {
    store.get_all::<Client>().ok()?.into_iter().next()
}

/// The open workspace's id, if one is set.
pub(crate) fn current_id(store: &Store) -> Option<String> {
    client(store)?
        .meta
        .get(CURRENT_META_KEY)
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// `name` trimmed and capped, or `None` when nothing is left.
pub(crate) fn clean_name(name: &str) -> Option<String> {
    let trimmed = name.trim();
    (!trimmed.is_empty()).then(|| trimmed.chars().take(NAME_MAX_CHARS).collect())
}

fn new_row(name: &str, now: i64) -> UserWorkspaceRow {
    UserWorkspaceRow {
        id: uuid::Uuid::new_v4().to_string(),
        name: name.to_string(),
        created_at: now,
        updated_at: now,
        ..Default::default()
    }
}

// ---- Startup ----

/// Before the store's first window is made (`wcore::ensure_initial_data`):
/// make sure there is a workspace. With none, copy the previous version's
/// ([`copy_from_previous_version`]), else make Default Workspace. Returns the
/// workspace this start opens when it decided one, for [`adopt_current`].
///
/// When the copied workspace has windows and the store has no client yet (a
/// new version's first start), only the client is made: with no window, the
/// first window's cold start restores them (`window_create`) instead of
/// seeding the default layout, so a build that is new here opens where the
/// last one left off, and offers no other build's layout (nothing was seeded).
pub(crate) fn prepare_at_startup(store: &Store, roots: Option<&ScanRoots>) -> Option<String> {
    match store.user_workspace_list() {
        Ok(rows) if !rows.is_empty() => return None,
        Ok(_) => {}
        Err(e) => {
            tracing::warn!(error = %e, "user_workspace: couldn't read the workspaces");
            return None;
        }
    }
    if let Some(copied) = roots.and_then(|r| copy_from_previous_version(store, r)) {
        let has_windows = store
            .user_workspace_get(&copied)
            .ok()
            .flatten()
            .and_then(|w| w.record)
            .and_then(|r| serde_json::from_str::<Value>(&r).ok())
            .is_some_and(|r| record_has_tabs(&r));
        if has_windows && client(store).is_none() {
            if let Err(e) = create_client_only(store) {
                tracing::warn!(error = %e, "user_workspace: couldn't make the client; the default window opens");
            }
        }
        return Some(copied);
    }
    let row = new_row(DEFAULT_NAME, now_ms());
    match store.user_workspace_insert(&row) {
        Ok(()) => {
            tracing::info!(workspace = %row.id, "user_workspace: made Default Workspace");
            Some(row.id)
        }
        Err(e) => {
            tracing::warn!(error = %e, "user_workspace: couldn't make Default Workspace");
            None
        }
    }
}

/// Whether any window in `record` has a tab to restore.
fn record_has_tabs(record: &Value) -> bool {
    windows_of(record)
        .iter()
        .any(|w| w.get("tabs").and_then(|t| t.as_array()).is_some_and(|t| !t.is_empty()))
}

/// The client row alone, as `ensure_initial_data` makes it (insert, then the
/// temp id), with no window: `ensure_initial_data` then sees a store that is
/// set up and seeds nothing.
fn create_client_only(store: &Store) -> Result<(), crate::backend::storage::StoreError> {
    let mut client = Client { oid: uuid::Uuid::new_v4().to_string(), ..Default::default() };
    store.insert(&mut client)?;
    client.tempoid = uuid::Uuid::new_v4().to_string();
    store.update(&mut client)?;
    Ok(())
}

/// After `ensure_initial_data`: open `decided` (from [`prepare_at_startup`]),
/// or keep the one the client names, or the most recently opened. Writes its
/// id and name onto the client, and when it was opened onto its row.
pub(crate) fn adopt_current(store: &Store, decided: Option<String>) {
    let rows = match store.user_workspace_list() {
        Ok(rows) => rows,
        Err(e) => {
            tracing::warn!(error = %e, "user_workspace: couldn't read the workspaces");
            return;
        }
    };
    let named = current_id(store).filter(|id| rows.iter().any(|w| &w.id == id));
    let Some(row) = decided
        .or(named)
        .and_then(|id| rows.iter().find(|w| w.id == id))
        .or_else(|| rows.first())
        .cloned()
    else {
        return;
    };
    let result = store.with_tx(|tx| {
        let Some(mut client) = tx.get_all::<Client>()?.into_iter().next() else { return Ok(()) };
        let was = client.meta.get(CURRENT_META_KEY).and_then(|v| v.as_str()).map(str::to_string);
        let same = was.as_deref() == Some(row.id.as_str())
            && client.meta.get(NAME_META_KEY).and_then(|v| v.as_str()) == Some(row.name.as_str());
        // The closed windows belong to the workspace that was open.
        if was.is_some_and(|w| w != row.id) {
            client.meta.remove(super::session_closed::CLOSED_META_KEY);
        }
        if !same {
            client.meta.insert(CURRENT_META_KEY.to_string(), json!(row.id));
            client.meta.insert(NAME_META_KEY.to_string(), json!(row.name));
            tx.update(&mut client)?;
        }
        Ok(())
    });
    if let Err(e) = result {
        tracing::warn!(error = %e, "user_workspace: couldn't record the open workspace");
    }
    let _ = store.user_workspace_mark_opened(&row.id, now_ms());
    tracing::info!(workspace = %row.id, name = %row.name, "user_workspace: opened");
}

/// The workspaces in the database at `path`, read-only, with the id its client
/// names as open. `Ok(None)` for a database without workspaces (an older
/// version's has no table).
pub(crate) fn read_other_workspaces(path: &Path) -> Result<Option<(Vec<UserWorkspaceRow>, Option<String>)>, String> {
    use rusqlite::OptionalExtension;
    let conn = open_other_db(path)?;
    let has_table: bool = conn
        .query_row("SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = 'db_workspaces'", [], |_| Ok(true))
        .optional()
        .map_err(|e| e.to_string())?
        .unwrap_or(false);
    if !has_table {
        return Ok(None);
    }
    let rows = user_workspaces::list_in(&conn).map_err(|e| e.to_string())?;
    if rows.is_empty() {
        return Ok(None);
    }
    let client: Option<Vec<u8>> = conn
        .query_row("SELECT data FROM db_client LIMIT 1", [], |row| row.get(0))
        .optional()
        .map_err(|e| e.to_string())?;
    let current = client
        .and_then(|d| serde_json::from_slice::<Value>(&d).ok())
        .and_then(|c| c.get("meta")?.get(CURRENT_META_KEY)?.as_str().map(str::to_string));
    Ok(Some((rows, current)))
}

/// A new version's first start (spec §4.2 "Upgrades"): copy every workspace
/// the channel's most recently written other version has, once, reading its
/// database only. Ids and names are kept; each record is also kept as the
/// row's `pre_upgrade`. Returns the one to open: the one that version had
/// open. `None` when there is nothing to copy.
pub(crate) fn copy_from_previous_version(store: &Store, roots: &ScanRoots) -> Option<String> {
    let path = newest_other_version_db(roots)?;
    let (rows, current) = match read_other_workspaces(&path) {
        Ok(Some(found)) => found,
        Ok(None) => return None,
        Err(e) => {
            tracing::warn!(error = %e, "user_workspace: couldn't read the previous version's workspaces");
            return None;
        }
    };
    let copied: Vec<UserWorkspaceRow> =
        rows.into_iter().map(|w| UserWorkspaceRow { pre_upgrade: w.record.clone(), ..w }).collect();
    let written = store.with_tx(|tx| {
        for w in &copied {
            tx.user_workspace_insert(w)?;
        }
        Ok(())
    });
    if let Err(e) = written {
        tracing::warn!(error = %e, "user_workspace: couldn't copy the previous version's workspaces");
        return None;
    }
    let open = current.filter(|id| copied.iter().any(|w| &w.id == id)).unwrap_or_else(|| copied[0].id.clone());
    tracing::info!(
        workspaces = copied.len(),
        from = %path.display(),
        "user_workspace: copied the previous version's workspaces"
    );
    Some(open)
}

// ---- Saving ----

/// What a save did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Saved {
    Written,
    /// The record already says this.
    Unchanged,
    /// No window is open: nothing to save, and the record is kept.
    NoWindow,
    /// A quit is closing the windows: it saved them all, and a save now
    /// would keep only the ones still open.
    HeldForQuit,
    /// A switch is moving the windows to another workspace
    /// (`user_workspace_switch`): a save now would store a mix of the two.
    HeldForSwitch,
    NoWorkspace,
    Failed,
}

/// A change not yet saved: when the first came, and the last, and whether
/// any changed the arrangement (not only blocks' settings).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Pending {
    pub first_ms: i64,
    pub last_ms: i64,
    pub arranged: bool,
}

impl Pending {
    /// A change seen at `now`, after `prev`; `arranged` when it changed the
    /// arrangement.
    pub(crate) fn changed(prev: Option<Pending>, now: i64, arranged: bool) -> Pending {
        Pending {
            first_ms: prev.map_or(now, |p| p.first_ms),
            last_ms: now,
            arranged: arranged || prev.is_some_and(|p| p.arranged),
        }
    }

    /// Whether to save now. The arrangement: nothing changed for
    /// [`DEBOUNCE_MS`], or the first unsaved change is [`CEILING_MS`] old.
    /// Blocks' settings alone: the first is [`SETTINGS_ONLY_MS`] old.
    pub(crate) fn due(&self, now: i64) -> bool {
        if !self.arranged {
            return now - self.first_ms >= SETTINGS_ONLY_MS;
        }
        now - self.last_ms >= DEBOUNCE_MS || now - self.first_ms >= CEILING_MS
    }
}

/// Whether two records hold the same windows: equal but for `saved_at_ms`.
fn same_windows(a: &Value, b: &Value) -> bool {
    let strip = |v: &Value| {
        let mut v = v.clone();
        if let Some(o) = v.as_object_mut() {
            o.remove("saved_at_ms");
        }
        v
    };
    strip(a) == strip(b)
}

/// Store `record` (the open windows) as the open workspace's windows, with
/// its closed windows (`session_closed::with_closed`), unless that is what is
/// stored already. Takes no lock; callers hold [`WRITE_LOCK`].
fn write_record(store: &Store, record: Value) -> Saved {
    let Some(id) = current_id(store) else { return Saved::NoWorkspace };
    let mut record = super::session_closed::with_closed(store, record);
    let row = match store.user_workspace_get(&id) {
        Ok(Some(row)) => row,
        Ok(None) => return Saved::NoWorkspace,
        Err(e) => {
            tracing::warn!(error = %e, "user_workspace: couldn't read the open workspace");
            return Saved::Failed;
        }
    };
    let stored = row.record.as_deref().and_then(|r| serde_json::from_str::<Value>(r).ok());
    if stored.is_some_and(|s| same_windows(&s, &record)) {
        return Saved::Unchanged;
    }
    let now = now_ms();
    if record.get("saved_at_ms").is_none() {
        record["saved_at_ms"] = json!(now);
    }
    match store.user_workspace_save_record(&id, &record.to_string(), now) {
        Ok(true) => Saved::Written,
        Ok(false) => Saved::NoWorkspace,
        Err(e) => {
            tracing::warn!(error = %e, "user_workspace: saving the workspace failed");
            Saved::Failed
        }
    }
}

/// Write `record` (a session record a boundary just built: a quit's, an OS
/// shutdown's, the last close's) as the open workspace's windows.
pub(crate) fn write_record_now(store: &Store, record: Value) -> Saved {
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    write_record(store, record)
}

/// Whether a quit is under way: its record is the session record and was
/// saved within the last two minutes (`session_windows::is_recent_quit_record`).
fn quit_holds_saves(store: &Store) -> bool {
    super::session_restore::load_last_session_snapshot(store).is_some_and(|r| is_recent_quit_record(&r, now_ms()))
}

/// Save the open workspace from the live windows now.
pub(crate) fn save_now(store: &Store) -> Saved {
    let _guard = WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    // Read before the snapshot: a write after this is a change it may miss.
    let generation = store.layout_writes().changed();
    if store.layout_writes().saves_held() {
        return Saved::HeldForSwitch;
    }
    if quit_holds_saves(store) {
        return Saved::HeldForQuit;
    }
    let saved = match super::session_windows::snapshot_session(store, false) {
        Some(record) => write_record(store, record),
        None => Saved::NoWindow,
    };
    if matches!(saved, Saved::Written | Saved::Unchanged | Saved::NoWindow) {
        store.layout_writes().mark_saved(generation);
    }
    if saved == Saved::Written {
        tracing::debug!("user_workspace: saved");
    }
    saved
}

/// Save the open workspace as it changes, for as long as srv runs. The state
/// it starts with is saved too (§8: the windows open at the first start with
/// this become Default Workspace at once).
pub(crate) fn spawn_saver(store: Arc<Store>) {
    tokio::spawn(async move {
        let mut seen = store.layout_writes().changed();
        let mut seen_arranged = store.layout_writes().arranged();
        let mut pending = Some(Pending::changed(None, now_ms(), true));
        let mut first = true;
        let mut tick = tokio::time::interval(TICK);
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tick.tick().await;
            let now = now_ms();
            let changed = store.layout_writes().changed();
            if changed != seen {
                let arranged = store.layout_writes().arranged();
                pending = Some(Pending::changed(pending, now, arranged != seen_arranged));
                seen = changed;
                seen_arranged = arranged;
            }
            let Some(p) = pending else { continue };
            if !first && !store.layout_writes().dirty() {
                // A flush saved it in between.
                pending = None;
                continue;
            }
            if !p.due(now) {
                continue;
            }
            let s = Arc::clone(&store);
            match tokio::task::spawn_blocking(move || save_now(&s)).await {
                // Try again once the quit or the switch is over.
                Ok(Saved::HeldForQuit | Saved::HeldForSwitch) => {}
                Ok(Saved::Failed) | Err(_) => pending = Some(Pending::changed(None, now, p.arranged)),
                Ok(_) => {
                    pending = None;
                    first = false;
                }
            }
        }
    });
}

/// `workspace.Flush`: save the open workspace now if anything changed since
/// its last save. The window calls it when it loses focus. Returns
/// `{ saved }`.
pub(crate) async fn handle_flush(state: &AppState, _call: &WebCallType) -> WebReturnType {
    if !state.mstore.layout_writes().dirty() {
        return WebReturnType::success(json!({ "saved": false }));
    }
    let store = Arc::clone(&state.mstore);
    match tokio::task::spawn_blocking(move || save_now(&store)).await {
        Ok(saved) => WebReturnType::success(json!({ "saved": saved == Saved::Written })),
        Err(e) => WebReturnType::error(format!("workspace flush: {e}")),
    }
}

/// `workspace.RenameUserWorkspace(workspaceId, name)`: rename a workspace
/// (`""` is the open one). The row and, for the open one, the client's
/// `workspace:name` change in one transaction; the client goes back as an
/// object update so every window's status bar panel shows the new name.
pub(crate) fn handle_rename(state: &AppState, call: &WebCallType) -> WebReturnType {
    let store = &state.mstore;
    let requested: String = service::get_arg(&call.args, 0).unwrap_or_default();
    let name: String = service::get_arg(&call.args, 1).unwrap_or_default();
    let Some(name) = clean_name(&name) else {
        return WebReturnType::error("a workspace needs a name");
    };
    let id = if requested.is_empty() { current_id(store).unwrap_or_default() } else { requested };
    if id.is_empty() {
        return WebReturnType::error("no workspace is open");
    }
    let result = store.with_tx(|tx| {
        if !tx.user_workspace_rename(&id, &name, now_ms())? {
            return Ok(Err(format!("no such workspace: {id}")));
        }
        let Some(mut client) = tx.get_all::<Client>()?.into_iter().next() else { return Ok(Ok(None)) };
        if client.meta.get(CURRENT_META_KEY).and_then(|v| v.as_str()) != Some(id.as_str()) {
            return Ok(Ok(None));
        }
        client.meta.insert(NAME_META_KEY.to_string(), json!(name));
        tx.update(&mut client)?;
        Ok(Ok(Some(client)))
    });
    match result {
        Ok(Ok(Some(client))) => {
            tracing::info!(workspace = %id, name = %name, "user_workspace: renamed");
            WebReturnType::success_with_updates(vec![MuxObjUpdate {
                updatetype: "update".into(),
                otype: "client".into(),
                oid: client.oid.clone(),
                obj: Some(mux_obj_to_value(&client)),
            }])
        }
        Ok(Ok(None)) => WebReturnType::success_empty(),
        Ok(Err(e)) => WebReturnType::error(e),
        Err(e) => WebReturnType::error(format!("rename failed: {e}")),
    }
}

// ---- Loading ----

/// Which copy a launch loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LoadedFrom {
    /// The session record (`session:last_topology`).
    Session,
    /// The workspace's record.
    Record,
    /// The workspace's last clean load.
    Previous,
    /// The copy made before this version's first start.
    PreUpgrade,
}

impl LoadedFrom {
    /// An older copy, which the window tells the user about.
    pub(crate) fn notice(self) -> Option<&'static str> {
        match self {
            LoadedFrom::Previous => Some("previous"),
            LoadedFrom::PreUpgrade => Some("pre_upgrade"),
            LoadedFrom::Session | LoadedFrom::Record => None,
        }
    }
}

fn parse_copy(which: &str, text: Option<&str>) -> Option<Value> {
    let text = text?;
    match serde_json::from_str::<Value>(text) {
        Ok(v) if v.is_object() => Some(v),
        Ok(_) => {
            tracing::warn!(copy = which, "user_workspace: a saved copy is not a record; skipping it");
            None
        }
        Err(e) => {
            tracing::warn!(copy = which, error = %e, "user_workspace: a saved copy doesn't parse; skipping it");
            None
        }
    }
}

fn saved_at(record: &Value) -> i64 {
    record.get("saved_at_ms").and_then(|v| v.as_i64()).unwrap_or(0)
}

/// The records a cold start tries, in order: the newer of the session record
/// and the workspace's record (the session record on a tie: a quit writes
/// both), then the other, then `previous`, then `pre_upgrade`. A copy that
/// doesn't parse is left out.
pub(crate) fn launch_candidates(session: Option<Value>, row: Option<&UserWorkspaceRow>) -> Vec<(LoadedFrom, Value)> {
    let record = row.and_then(|w| parse_copy("record", w.record.as_deref()));
    let mut out = Vec::new();
    match (session.filter(Value::is_object), record) {
        (Some(s), Some(r)) if saved_at(&r) > saved_at(&s) => {
            out.push((LoadedFrom::Record, r));
            out.push((LoadedFrom::Session, s));
        }
        (Some(s), Some(r)) => {
            out.push((LoadedFrom::Session, s));
            out.push((LoadedFrom::Record, r));
        }
        (Some(s), None) => out.push((LoadedFrom::Session, s)),
        (None, Some(r)) => out.push((LoadedFrom::Record, r)),
        (None, None) => {}
    }
    if let Some(p) = row.and_then(|w| parse_copy("previous", w.previous.as_deref())) {
        out.push((LoadedFrom::Previous, p));
    }
    if let Some(p) = row.and_then(|w| parse_copy("pre_upgrade", w.pre_upgrade.as_deref())) {
        out.push((LoadedFrom::PreUpgrade, p));
    }
    out
}

/// What a launch restored: the window being created's workspace and events
/// (as `restore_last_session` returns them), the record and where it came from.
pub(crate) struct Restored {
    pub main: (String, Vec<Event>),
    pub loaded: Value,
    pub from: LoadedFrom,
}

/// A cold start's restore: each of [`launch_candidates`] in turn until one
/// brings a window back. `Ok(None)` when none had anything; the default seed
/// follows.
pub(crate) async fn restore_at_launch(state: &AppState) -> Result<Option<Restored>, String> {
    let store = &state.mstore;
    let row = current_id(store).and_then(|id| store.user_workspace_get(&id).ok().flatten());
    let candidates = launch_candidates(super::session_restore::load_last_session_snapshot(store), row.as_ref());
    let mut first_err = None;
    for (from, record) in candidates {
        match super::session_windows::restore_all(state, &record).await {
            Ok(Some(main)) => {
                if let Some(which) = from.notice() {
                    tracing::warn!(copy = which, "user_workspace: the saved workspace didn't load; restored an older copy");
                }
                // The session record holds only the open windows: the closed
                // ones come from the workspace's record (§5).
                if from == LoadedFrom::Session {
                    if let Some(own) = row.as_ref().and_then(|w| parse_copy("record", w.record.as_deref())) {
                        let shown = super::session_windows::windows_of(&record)
                            .iter()
                            .filter_map(|w| w.get("workspaceid").and_then(|v| v.as_str()).map(str::to_string))
                            .collect();
                        super::session_closed::restore_closed(state, &own, &main.0, &shown).await;
                    }
                }
                return Ok(Some(Restored { main, loaded: record, from }));
            }
            Ok(None) => tracing::warn!(copy = ?from, "user_workspace: nothing came back from a saved copy"),
            Err(e) => {
                tracing::warn!(copy = ?from, error = %e, "user_workspace: a saved copy didn't restore");
                first_err.get_or_insert(e);
            }
        }
    }
    match first_err {
        Some(e) => Err(e),
        None => Ok(None),
    }
}

/// A restore finished cleanly (`window_create`'s final read-back): keep what
/// loaded as the workspace's `previous`, and when it was an older copy, leave
/// the window a note to say so.
pub(crate) fn after_clean_load(store: &Store, loaded: &Value, from: LoadedFrom) {
    if let Some(id) = current_id(store) {
        // A quit's record holds the open windows only: keep the closed ones
        // the restore listed with it, so `previous` recovers them too (#5359).
        let has_closed = super::session_windows::windows_of(loaded).iter().any(super::session_closed::is_closed);
        let loaded = if has_closed || super::session_closed::closed_entries(store).is_empty() {
            loaded.clone()
        } else {
            super::session_closed::with_closed(store, loaded.clone())
        };
        if let Err(e) = store.user_workspace_set_previous(&id, &loaded.to_string()) {
            tracing::warn!(error = %e, "user_workspace: couldn't keep the loaded copy");
        }
    }
    if let Some(which) = from.notice() {
        set_client_meta(store, RESTORED_FROM_META_KEY, Some(json!(which)));
    }
}

#[cfg(test)]
#[path = "user_workspace_tests.rs"]
mod tests;
