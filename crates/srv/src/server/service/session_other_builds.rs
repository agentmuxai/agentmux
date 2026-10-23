// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Restore a layout another build saved.
//!
//! Every channel and every version keeps its own `objects.db`
//! (`channels/<ch>/versions/<v>/data/db/`), and with it its own session record
//! (`session_restore`, `Client.meta["session:last_topology"]`). So a build
//! that has never run here, or a different local build, comes up with the
//! default layout while the record the previous build saved sits unused in
//! that build's database. This finds those records, reading each database
//! read-only, and rebuilds one in this instance with the replay a same-build
//! restore uses (`session_restore::replay_tabs`). The other database is never
//! written.
//!
//! A first launch, or a cold start that seeds the default layout because
//! nothing was restored, remembers it ([`remember_seeded_default`]); while
//! that layout is untouched the window offers the newest record, and
//! restoring replaces it.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use agentmux_common::ipc::Command;
use serde_json::{json, Value};

use crate::backend::obj::{Block, Client, Tab, Window, Workspace};
use crate::backend::rpc_types::{OtherBuildLayout, RestoreOtherBuildLayoutResult};
use crate::backend::storage::store::Store;
use crate::server::AppState;

use super::reducer_helpers::{dispatch_to_reducer, publish_events};
use super::session_restore::{replay_tabs, replay_tabs_of, restore_active_tab, restore_workspace, SNAPSHOT_META_KEY};
use super::session_windows::{patch_of, queue_restored_windows, set_client_meta, windows_of};

/// The default layout a cold start seeded: `{workspace_id, tab_id, block_ids}`.
const SEEDED_META_KEY: &str = "session:seeded_default";

/// Databases opened per scan, the most recently written first: each is one
/// open and one row read, and only recent builds matter.
const MAX_SCANNED: usize = 16;

/// A scan stops opening databases after this long.
pub(crate) const SCAN_BUDGET: Duration = Duration::from_millis(1500);

/// A database another process holds locked is skipped after this long rather
/// than waited for.
const BUSY_TIMEOUT: Duration = Duration::from_millis(200);

/// The views of the default seed (`window_create`): a record holding only
/// these, with no agent chosen, is not worth offering.
const DEFAULT_VIEWS: [&str; 3] = ["agent", "swarm", "sysinfo"];

/// Where to look: the channels root, and this process's own `data/db/`
/// (never offered).
pub(crate) struct ScanRoots {
    pub channels_root: PathBuf,
    pub own_db_dir: Option<PathBuf>,
}

impl ScanRoots {
    /// From the launcher's environment. `None` when it isn't set (tests, a
    /// bare `cargo run`).
    pub(crate) fn live() -> Option<Self> {
        let paths = agentmux_common::DataPaths::from_env()?;
        Some(Self { channels_root: paths.home_dir.join("channels"), own_db_dir: Some(paths.data_dir.join("db")) })
    }
}

struct DbFile {
    channel: String,
    version: String,
    path: PathBuf,
    written_ms: i64,
}

fn modified_ms(path: &Path) -> Option<i64> {
    let at = std::fs::metadata(path).ok()?.modified().ok()?;
    Some(at.duration_since(std::time::UNIX_EPOCH).ok()?.as_millis() as i64)
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// Every `channels/<ch>/versions/<v>/data/db/objects.db` but this process's
/// own. Dev builds keep theirs under `dev/`, outside the channels root.
fn db_files(roots: &ScanRoots) -> Vec<DbFile> {
    let mut out = Vec::new();
    let Ok(channels) = std::fs::read_dir(&roots.channels_root) else {
        return out;
    };
    for channel in channels.flatten() {
        let Ok(versions) = std::fs::read_dir(channel.path().join("versions")) else {
            continue;
        };
        for version in versions.flatten() {
            let db_dir = version.path().join("data").join("db");
            let path = db_dir.join("objects.db");
            let Some(db_ms) = modified_ms(&path) else {
                continue;
            };
            if roots.own_db_dir.as_deref().is_some_and(|own| same_dir(own, &db_dir)) {
                continue;
            }
            // A WAL database's last writes are in its `-wal` until checkpointed.
            // An empty one holds none (a reader may have created it).
            let wal = wal_of(&path);
            let written_ms = if wal_len(&wal) > 0 { modified_ms(&wal).map_or(db_ms, |w| w.max(db_ms)) } else { db_ms };
            out.push(DbFile {
                channel: channel.file_name().to_string_lossy().into_owned(),
                version: version.file_name().to_string_lossy().into_owned(),
                path,
                written_ms,
            });
        }
    }
    out
}

/// The channel this process's own database belongs to: the `<ch>` of
/// `<channels root>/<ch>/versions/<v>/data/db`. `None` for a database
/// elsewhere (a dev build's, under `dev/`).
fn own_channel(roots: &ScanRoots) -> Option<String> {
    let own = roots.own_db_dir.as_deref()?;
    // db → data → <v> → versions → <ch>
    let versions = own.parent()?.parent()?.parent()?;
    if versions.file_name()? != "versions" {
        return None;
    }
    let channel = versions.parent()?;
    if !same_dir(channel.parent()?, &roots.channels_root) {
        return None;
    }
    Some(channel.file_name()?.to_string_lossy().into_owned())
}

/// The most recently written database of another version of this process's
/// own channel: the version a new one follows (`user_workspace`'s upgrade
/// copy, SPEC_WORKSPACES_2026_10_23.md §4.2). `None` outside a channel.
pub(crate) fn newest_other_version_db(roots: &ScanRoots) -> Option<PathBuf> {
    let channel = own_channel(roots)?;
    db_files(roots)
        .into_iter()
        .filter(|f| f.channel == channel)
        .max_by_key(|f| f.written_ms)
        .map(|f| {
            tracing::debug!(version = %f.version, "session_other_builds: newest other version of this channel");
            f.path
        })
}

fn wal_of(db: &Path) -> PathBuf {
    let mut name = db.as_os_str().to_owned();
    name.push("-wal");
    PathBuf::from(name)
}

fn wal_len(wal: &Path) -> u64 {
    std::fs::metadata(wal).map(|m| m.len()).unwrap_or(0)
}

/// `path` as an SQLite URI with `immutable=1`.
fn immutable_uri(path: &Path) -> String {
    let p = path.to_string_lossy().replace('\\', "/").replace('%', "%25").replace('?', "%3f").replace('#', "%23");
    let sep = if p.starts_with('/') { "//" } else { "///" };
    format!("file:{sep}{p}?immutable=1")
}

/// Another build's database at `path`, read-only. A database with nothing in
/// its `-wal` is opened immutable, so not even the reader's `-wal`/`-shm`
/// files are created; one with WAL content is opened as an ordinary reader,
/// which reads that content and may add the `-shm` index.
pub(crate) fn open_other_db(path: &Path) -> Result<rusqlite::Connection, String> {
    use rusqlite::OpenFlags;
    let flags = OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX;
    let conn = if wal_len(&wal_of(path)) == 0 {
        rusqlite::Connection::open_with_flags(immutable_uri(path), flags | OpenFlags::SQLITE_OPEN_URI)
    } else {
        rusqlite::Connection::open_with_flags(path, flags)
    }
    .map_err(|e| format!("open {}: {e}", path.display()))?;
    conn.busy_timeout(BUSY_TIMEOUT).map_err(|e| e.to_string())?;
    Ok(conn)
}

/// The session record in the database at `path`, read-only ([`open_other_db`]).
pub(crate) fn read_record(path: &Path) -> Result<Option<Value>, String> {
    use rusqlite::OptionalExtension;
    let conn = open_other_db(path)?;
    let data: Option<Vec<u8>> = conn
        .query_row("SELECT data FROM db_client LIMIT 1", [], |row| row.get(0))
        .optional()
        .map_err(|e| format!("read {}: {e}", path.display()))?;
    let Some(data) = data else {
        return Ok(None);
    };
    let client: Value = serde_json::from_slice(&data).map_err(|e| e.to_string())?;
    Ok(client.get("meta").and_then(|m| m.get(SNAPSHOT_META_KEY)).filter(|v| !v.is_null()).cloned())
}

fn meta_str<'a>(meta: &'a Value, key: &str) -> &'a str {
    meta.get(key).and_then(|v| v.as_str()).unwrap_or_default()
}

/// Describe a record, or `None` when it holds nothing worth restoring: no
/// tabs, or only the untouched default layout. `written_ms` stands in for
/// the save time a record made before records carried one.
pub(crate) fn summarize(record: &Value, channel: &str, version: &str, written_ms: i64) -> Option<OtherBuildLayout> {
    let windows = windows_of(record);
    let (mut tabs, mut panes) = (0u32, 0u32);
    let mut agents = Vec::new();
    let mut seen = HashSet::new();
    let mut only_default_views = true;
    for window in &windows {
        for tab in window.get("tabs").and_then(|v| v.as_array()).into_iter().flatten() {
            let blocks = tab.get("blocks").and_then(|v| v.as_array()).cloned().unwrap_or_default();
            if blocks.is_empty() {
                continue;
            }
            tabs += 1;
            for block in &blocks {
                panes += 1;
                let meta = block.get("meta").unwrap_or(&Value::Null);
                let view = meta_str(meta, "view");
                only_default_views &= DEFAULT_VIEWS.contains(&view);
                let agent_id = meta_str(meta, "agentId");
                if view == "agent" && !agent_id.is_empty() && seen.insert(agent_id.to_string()) {
                    let name = meta_str(meta, "agentName");
                    agents.push(if name.is_empty() { agent_id } else { name }.to_string());
                }
            }
        }
    }
    if tabs == 0 || (windows.len() == 1 && tabs == 1 && agents.is_empty() && only_default_views) {
        return None;
    }
    let saved_at = record.get("saved_at_ms").and_then(|v| v.as_i64());
    Some(OtherBuildLayout {
        channel: channel.to_string(),
        version: version.to_string(),
        saved_at_ms: saved_at.unwrap_or(written_ms),
        saved_at_exact: saved_at.is_some(),
        windows: windows.len() as u32,
        tabs,
        panes,
        agents,
    })
}

/// The layouts other builds saved, newest first. Reads at most
/// [`MAX_SCANNED`] databases, the most recently written first, for at most
/// `budget`; one that is missing, locked, corrupt or of an unknown shape is
/// skipped. Blocking: run it off the async workers.
pub(crate) fn discover(roots: &ScanRoots, budget: Duration) -> Vec<OtherBuildLayout> {
    let started = Instant::now();
    let mut files = db_files(roots);
    files.sort_by_key(|f| std::cmp::Reverse(f.written_ms));
    files.truncate(MAX_SCANNED);
    let mut found = Vec::new();
    for file in files {
        if started.elapsed() > budget {
            tracing::debug!("session_other_builds: scan budget spent; skipping older builds");
            break;
        }
        match read_record(&file.path) {
            Ok(Some(record)) => found.extend(summarize(&record, &file.channel, &file.version, file.written_ms)),
            Ok(None) => {}
            Err(e) => tracing::debug!(channel = %file.channel, version = %file.version, error = %e, "session_other_builds: skipped a database"),
        }
    }
    found.sort_by_key(|l| std::cmp::Reverse(l.saved_at_ms));
    found
}

/// A cold start seeded the default layout because nothing was restored.
pub(crate) fn remember_seeded_default(store: &Store, ws_id: &str, tab_id: &str, block_ids: &[String]) {
    set_client_meta(
        store,
        SEEDED_META_KEY,
        Some(json!({ "workspace_id": ws_id, "tab_id": tab_id, "block_ids": block_ids })),
    );
}

/// A first launch: the store was empty and `wcore::ensure_initial_data` seeded
/// the default layout, which a new build or a new local build starts with.
pub(crate) fn remember_first_launch_default(store: &Store) {
    let seeded = (|| {
        let client = store.get_all::<Client>().ok()?.into_iter().next()?;
        let window = store.get::<Window>(client.windowids.first()?).ok()??;
        let workspace = store.get::<Workspace>(&window.workspaceid).ok()??;
        let tab_id = workspace.pinnedtabids.iter().chain(workspace.tabids.iter()).next()?.clone();
        let tab = store.get::<Tab>(&tab_id).ok()??;
        Some((workspace.oid, tab_id, tab.blockids))
    })();
    if let Some((ws_id, tab_id, block_ids)) = seeded {
        remember_seeded_default(store, &ws_id, &tab_id, &block_ids);
    }
}

/// The seeded default tab of workspace `ws_id`, if the workspace still holds
/// only it, with the same panes and no agent chosen in its agent pane.
pub(crate) fn untouched_default_tab(store: &Store, ws_id: &str) -> Option<String> {
    let client = store.get_all::<Client>().ok()?.into_iter().next()?;
    let seeded = client.meta.get(SEEDED_META_KEY)?;
    if meta_str(seeded, "workspace_id") != ws_id {
        return None;
    }
    let tab_id = meta_str(seeded, "tab_id").to_string();
    let workspace = store.get::<Workspace>(ws_id).ok()??;
    let tabs: Vec<&String> = workspace.pinnedtabids.iter().chain(workspace.tabids.iter()).collect();
    if tabs != [&tab_id] {
        return None;
    }
    let tab = store.get::<Tab>(&tab_id).ok()??;
    let seeded_blocks: HashSet<&str> =
        seeded.get("block_ids")?.as_array()?.iter().filter_map(|v| v.as_str()).collect();
    let blocks: HashSet<&str> = tab.blockids.iter().map(String::as_str).collect();
    if blocks != seeded_blocks {
        return None;
    }
    let agent_chosen = tab.blockids.iter().any(|id| {
        store
            .get::<Block>(id)
            .ok()
            .flatten()
            .is_some_and(|b| b.meta.get("agentId").and_then(|v| v.as_str()).is_some_and(|s| !s.is_empty()))
    });
    (!agent_chosen).then_some(tab_id)
}

/// A single, plain path segment: what a channel or version directory is named.
fn is_plain_segment(s: &str) -> bool {
    !s.is_empty() && s != "." && s != ".." && !s.contains(['/', '\\', ':', '\0'])
}

/// The database of build `channel`/`version`, never this process's own.
fn locate(roots: &ScanRoots, channel: &str, version: &str) -> Result<PathBuf, String> {
    if !is_plain_segment(channel) || !is_plain_segment(version) {
        return Err("not a build of this AgentMux".to_string());
    }
    let db_dir = roots.channels_root.join(channel).join("versions").join(version).join("data").join("db");
    if roots.own_db_dir.as_deref().is_some_and(|own| same_dir(own, &db_dir)) {
        return Err("that is this build's own layout".to_string());
    }
    let path = db_dir.join("objects.db");
    if !path.is_file() {
        return Err(format!("no build {version} ({channel}) on this computer"));
    }
    Ok(path)
}

async fn apply_publish(state: &AppState, events: &[agentmux_common::ipc::Event]) {
    for ev in events {
        if let Err(e) = crate::persist_subscriber::apply_event_to_mstore(ev, &state.mstore) {
            tracing::warn!(error = %e, "session_other_builds: SQLite write failed");
        }
    }
    publish_events(state, events);
}

/// Rebuild the layout build `channel`/`version` saved into window
/// `window_id`: its first window's tabs into this window, replacing the
/// default tab when it is untouched and otherwise beside what is open; its
/// other windows each into a workspace of its own, queued for this window to
/// open (`window.TakeRestoredWindows`).
pub(crate) async fn restore(
    state: &AppState,
    roots: &ScanRoots,
    window_id: &str,
    channel: &str,
    version: &str,
) -> Result<RestoreOtherBuildLayoutResult, String> {
    let path = locate(roots, channel, version)?;
    let record = tokio::task::spawn_blocking(move || read_record(&path))
        .await
        .map_err(|e| format!("task: {e}"))??
        .ok_or_else(|| format!("{version} has no saved layout"))?;
    let store = &state.mstore;
    let ws_id = store
        .get::<Window>(window_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("no such window: {window_id}"))?
        .workspaceid;
    let seeded_tab = untouched_default_tab(store, &ws_id);
    let windows = windows_of(&record);
    let first = windows.first().ok_or("the saved layout has no windows")?;

    let (events, replayed) = replay_tabs(state, &ws_id, &replay_tabs_of(first), "restore_other_build").await;
    publish_events(state, &events);
    let by_index: HashMap<usize, String> =
        replayed.iter().enumerate().filter_map(|(i, r)| r.as_ref().map(|r| (i, r.tab_id.clone()))).collect();
    let tab_ids: Vec<String> = replayed.iter().flatten().map(|r| r.tab_id.clone()).collect();
    if tab_ids.is_empty() {
        return Err("none of the saved tabs could be created".to_string());
    }
    let mut active = restore_active_tab(state, &ws_id, first, &by_index).await;
    if active.is_empty() {
        // Show what came back, not the tab that was in front before.
        active = dispatch_to_reducer(
            state,
            Command::SetActiveTab { workspace_id: ws_id.clone(), tab_id: tab_ids[0].clone() },
        )
        .await;
        for ev in &active {
            let _ = crate::persist_subscriber::apply_event_to_mstore(ev, store);
        }
    }
    publish_events(state, &active);

    let replaced_default = match &seeded_tab {
        Some(tab_id) => match crate::sagas::delete_tab::run(state, ws_id.clone(), tab_id.clone()).await {
            Ok(_) => true,
            Err(e) => {
                tracing::warn!(error = %e, "session_other_builds: couldn't remove the default tab");
                false
            }
        },
        None => false,
    };
    if replaced_default {
        // The window becomes the saved one: its name and appearance too.
        let patch = first.get("window").map(patch_of).unwrap_or_default();
        if !patch.is_empty() {
            let evs = dispatch_to_reducer(
                state,
                Command::UpdateWindowMeta { window_id: window_id.to_string(), meta_patch: Value::Object(patch) },
            )
            .await;
            apply_publish(state, &evs).await;
        }
    }
    // Restoring answers the offer, whichever way it went.
    set_client_meta(store, SEEDED_META_KEY, None);

    let mut waiting = Vec::new();
    let mut metas = serde_json::Map::new();
    for (i, window) in windows.iter().enumerate().skip(1) {
        match restore_workspace(state, window).await {
            Ok(Some((new_ws, evs))) => {
                publish_events(state, &evs);
                let mut entry = window.get("window").cloned().unwrap_or_else(|| json!({}));
                entry["workspaceid"] = json!(new_ws);
                waiting.push(entry);
                let patch = window.get("window").map(patch_of).unwrap_or_default();
                if !patch.is_empty() {
                    metas.insert(new_ws, Value::Object(patch));
                }
            }
            Ok(None) => tracing::warn!(window = i, "session_other_builds: a window's tabs didn't come back"),
            Err(e) => tracing::warn!(window = i, error = %e, "session_other_builds: a window didn't come back"),
        }
    }
    let other_windows = waiting.len() as u32;
    queue_restored_windows(store, waiting, metas);
    tracing::info!(
        channel,
        version,
        tabs = tab_ids.len(),
        other_windows,
        replaced_default,
        "session_other_builds: restored a layout another build saved"
    );
    Ok(RestoreOtherBuildLayoutResult { workspace_id: ws_id, tab_ids, replaced_default, other_windows })
}

#[cfg(test)]
#[path = "session_other_builds_tests.rs"]
mod tests;
