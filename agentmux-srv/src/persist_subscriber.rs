// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase E.2c.1+E.2c.2 — persist subscriber. A tokio task that
// consumes the srv reducer's broadcast bus and mirrors
// workspace/tab/block lifecycle events back to SQLite via wcore.
//
// Status:
//   * E.2c.1 (#614) shipped the per-event apply path. Subscriber was
//     dead-code in production because no producer existed.
//   * E.2c.2 (this) wires the workspace HTTP/WS RPC handlers through
//     the reducer, making the subscriber LIVE for workspace events.
//     Adds full-resync-on-`Lagged` for workspaces (RPC-migrated
//     entities only — tab/block resync lands in E.2c.3 / E.2c.4).
//
// Bus-lag handling:
//   On RecvError::Lagged(n) the subscriber drops `n` events.
//   Naive HWM advancement past dropped events would permanently
//   diverge SQLite from reducer state. Instead we do a workspace-
//   scoped resync: snapshot `state.workspaces` under the reducer
//   mutex, write each workspace into SQLite (idempotent insert /
//   no-op / update). The resync is INSERT/UPDATE only — no deletes —
//   because workspaces can also be created OUTSIDE the reducer
//   during the migration window (e.g., the still-wcore-direct
//   `CreateWindow` flow calls `wcore::create_window_full` which
//   creates a workspace under the hood). Deleting workspaces
//   missing from the reducer snapshot would lose those legitimate
//   rows. Stale workspaces deleted-via-reducer that the subscriber
//   missed on Lagged linger on disk until the next user-driven
//   DeleteWorkspace cleans them up via the wcore fallback in
//   service.rs. (codex P1 #615.)
//
//   IMPORTANT: tab/block resync is NOT done here yet. Tabs and
//   blocks remain RPC-direct via wcore; the reducer's view of them
//   is FROZEN at bootstrap. Doing tab/block resync before E.2c.3
//   and E.2c.4 land would clobber RPC-driven writes the reducer
//   doesn't see. Resync expands as each entity migrates.

use std::sync::Arc;

use agentmux_common::ipc::Event;
use tokio::sync::{broadcast, Mutex};

use crate::backend::obj::{Block, LayoutState as PersistedLayoutState, Tab, Window, Workspace};
use crate::backend::storage::wstore::WaveStore;
use crate::backend::wcore;
use crate::state::State;

/// Spawn the persist subscriber task. Runs until the broadcast
/// channel closes (i.e., the reducer's bus is dropped at process
/// shutdown). The `state` handle is used for workspace-scoped
/// full-resync after a `RecvError::Lagged`.
pub fn spawn_persist_subscriber(
    events_rx: broadcast::Receiver<Event>,
    wstore: Arc<WaveStore>,
    state: Arc<Mutex<State>>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(run_persist_subscriber(events_rx, wstore, state))
}

async fn run_persist_subscriber(
    mut events_rx: broadcast::Receiver<Event>,
    wstore: Arc<WaveStore>,
    state: Arc<Mutex<State>>,
) {
    tracing::info!(target: "srv-persist-subscriber", "[srv-persist-subscriber] started");
    loop {
        match events_rx.recv().await {
            Ok(event) => {
                if let Err(e) = apply_event_to_wstore(&event, &wstore) {
                    tracing::warn!(
                        target: "srv-persist-subscriber",
                        "[srv-persist-subscriber] apply failed for event {:?}: {}",
                        event_kind(&event),
                        e
                    );
                }
            }
            Err(broadcast::error::RecvError::Lagged(n)) => {
                // Workspace-scoped full resync. Subscriber dropped
                // `n` events; rather than guess which entities were
                // affected, snapshot the reducer's workspace map and
                // reconcile SQLite against it. RPC-migrated entities
                // (workspaces in E.2c.2) are correctly recovered;
                // not-yet-migrated entities (tab/block) are left
                // alone — see module-level docs.
                tracing::warn!(
                    target: "srv-persist-subscriber",
                    "[srv-persist-subscriber] dropped {} event(s) — running workspace resync",
                    n
                );
                if let Err(e) = resync_workspaces(&state, &wstore).await {
                    tracing::error!(
                        target: "srv-persist-subscriber",
                        "[srv-persist-subscriber] resync failed: {} — SQLite may diverge from reducer until next event",
                        e
                    );
                }
            }
            Err(broadcast::error::RecvError::Closed) => {
                tracing::info!(target: "srv-persist-subscriber", "[srv-persist-subscriber] bus closed — exiting");
                return;
            }
        }
    }
}

/// Snapshot `state.workspaces` and reconcile against SQLite —
/// INSERT/UPDATE only, no deletes. Workspace-scoped (tab/block
/// resync expands here as each entity's RPC layer migrates).
///
/// Why no delete-phase: during the migration window, workspaces
/// can be created OUTSIDE the reducer (e.g., the still-wcore-direct
/// `CreateWindow` flow calls `wcore::create_window_full` which
/// creates a workspace under the hood). Those workspaces aren't in
/// `state.workspaces`. If the subscriber lagged and we did a
/// delete-not-in-snapshot pass, we'd cascade-delete legitimate
/// workspaces — data loss triggered by bus pressure rather than a
/// user action. Far better for a stale workspace deleted-via-reducer
/// to linger on disk after a Lagged event than to lose a real one.
/// (codex P1 #615.)
///
/// Stale-after-Lagged scenario: reducer fires WorkspaceDeleted, the
/// subscriber drops it on Lagged, resync runs but only insert/updates.
/// SQLite still has the deleted workspace's row. The next user-driven
/// DeleteWorkspace (which falls through to `wcore::delete_workspace`
/// for unknown-to-reducer rows) cleans it up. Acceptable behaviour
/// during the migration; tightens once all RPC is reducer-driven.
///
/// Strategy:
///   1. Lock state, snapshot the workspace map (ids + names),
///      release the lock.
///   2. For each workspace in the snapshot: insert if missing,
///      no-op if name matches, update if name differs.
async fn resync_workspaces(
    state: &Arc<Mutex<State>>,
    wstore: &WaveStore,
) -> Result<(), Box<dyn std::error::Error>> {
    // Snapshot under lock; release before any I/O.
    let snapshot: Vec<(String, String)> = {
        let s = state.lock().await;
        s.workspaces
            .values()
            .map(|w| (w.workspace_id.clone(), w.name.clone()))
            .collect()
    };

    for (workspace_id, name) in &snapshot {
        match wstore.get::<Workspace>(workspace_id)? {
            Some(existing) if existing.name == *name => {
                // Already in sync.
            }
            Some(mut existing) => {
                existing.name = name.clone();
                wstore.update(&mut existing)?;
            }
            None => {
                let mut ws = Workspace {
                    oid: workspace_id.clone(),
                    name: name.clone(),
                    ..Default::default()
                };
                wstore.insert(&mut ws)?;
            }
        }
    }
    Ok(())
}

/// Apply one reducer event to the on-disk store. Idempotent: each
/// arm checks for the entity's current SQLite state before writing
/// so duplicate events (from at-least-once delivery semantics) don't
/// produce duplicate rows or wcore errors.
///
/// Phase E.2c.2 — exposed at crate visibility so RPC handlers in
/// `service.rs` can apply events synchronously after dispatching
/// through the reducer. This closes the race where the async
/// subscriber hadn't yet written SQLite by the time a follow-up
/// RPC (e.g., `CreateTab` against a just-created workspace) tried
/// to read it. Calling this from the RPC handler followed by the
/// subscriber receiving the broadcast event is safe because the
/// arms are idempotent — the subscriber's later apply is a no-op.
pub(crate) fn apply_event_to_wstore(
    event: &Event,
    wstore: &WaveStore,
) -> Result<(), Box<dyn std::error::Error>> {
    match event {
        Event::WorkspaceCreated {
            workspace_id, name, ..
        } => apply_workspace_created(wstore, workspace_id, name),
        Event::WorkspaceDeleted { workspace_id, .. } => {
            apply_workspace_deleted(wstore, workspace_id)
        }
        Event::TabCreated {
            workspace_id,
            tab_id,
            name,
            ..
        } => apply_tab_created(wstore, workspace_id, tab_id, name),
        Event::TabDeleted {
            workspace_id,
            tab_id,
            ..
        } => apply_tab_deleted(wstore, workspace_id, tab_id),
        Event::ActiveTabChanged {
            workspace_id,
            tab_id,
            ..
        } => apply_active_tab_changed(wstore, workspace_id, tab_id.as_deref()),
        Event::TabReordered {
            workspace_id,
            tab_id,
            new_index,
            ..
        } => apply_tab_reordered(wstore, workspace_id, tab_id, *new_index),
        Event::BlockCreated {
            tab_id,
            block_id,
            meta,
            ..
        } => apply_block_created(wstore, tab_id, block_id, meta),
        Event::BlockDeleted {
            tab_id, block_id, ..
        } => apply_block_deleted(wstore, tab_id, block_id),
        Event::SrvWindowOpened {
            window_id,
            workspace_id,
            ..
        } => apply_srv_window_opened(wstore, window_id, workspace_id),
        Event::SrvWindowClosed { window_id, .. } => apply_srv_window_closed(wstore, window_id),
        Event::SrvWindowWorkspaceChanged {
            window_id,
            workspace_id,
            ..
        } => apply_srv_window_workspace_changed(wstore, window_id, workspace_id),
        Event::TabsReorderedBulk {
            workspace_id,
            tab_ids,
            ..
        } => apply_tabs_reordered_bulk(wstore, workspace_id, tab_ids),
        Event::WorkspaceRenamed {
            workspace_id, name, ..
        } => apply_workspace_renamed(wstore, workspace_id, name),
        Event::TabRenamed { tab_id, name, .. } => apply_tab_renamed(wstore, tab_id, name),
        Event::WorkspaceMetaUpdated {
            workspace_id,
            meta_patch,
            ..
        } => apply_workspace_meta_updated(wstore, workspace_id, meta_patch),
        Event::TabMetaUpdated {
            tab_id,
            meta_patch,
            ..
        } => apply_tab_meta_updated(wstore, tab_id, meta_patch),
        Event::BlockMetaUpdated {
            block_id,
            meta_patch,
            ..
        } => apply_block_meta_updated(wstore, block_id, meta_patch),
        Event::TabMoved {
            tab_id,
            src_workspace_id,
            dst_workspace_id,
            dst_index,
            new_src_active_tab_id,
            new_dst_active_tab_id,
            ..
        } => apply_tab_moved(
            wstore,
            tab_id,
            src_workspace_id,
            dst_workspace_id,
            *dst_index,
            new_src_active_tab_id.as_deref(),
            new_dst_active_tab_id.as_deref(),
        ),
        Event::BlockMoved {
            block_id,
            src_tab_id,
            dst_tab_id,
            dst_index,
            ..
        } => apply_block_moved(wstore, block_id, src_tab_id, dst_tab_id, *dst_index),
        // All other event variants are not domain-state mutations
        // (lifecycle, errors, snapshots). The subscriber ignores them.
        _ => Ok(()),
    }
}

fn apply_workspace_created(
    wstore: &WaveStore,
    workspace_id: &str,
    name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if wstore.get::<Workspace>(workspace_id)?.is_some() {
        return Ok(()); // already present
    }
    let mut ws = Workspace {
        oid: workspace_id.to_string(),
        name: name.to_string(),
        ..Default::default()
    };
    wstore.insert(&mut ws)?;
    Ok(())
}

fn apply_workspace_deleted(
    wstore: &WaveStore,
    workspace_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if wstore.get::<Workspace>(workspace_id)?.is_none() {
        return Ok(()); // already gone
    }
    wcore::delete_workspace(wstore, workspace_id)?;
    Ok(())
}

fn apply_tab_created(
    wstore: &WaveStore,
    workspace_id: &str,
    tab_id: &str,
    name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if wstore.get::<Tab>(tab_id)?.is_none() {
        // Phase E.2c.2 — create a LayoutState row alongside the Tab
        // so downstream flows that hard-require `Tab.layoutstate`
        // (e.g., `wcore::tear_off_block`'s
        // `must_get::<LayoutState>(&tab.layoutstate)`) work for
        // reducer-originated tabs. Mirrors the wcore::create_tab
        // pattern. (codex P2 #614.)
        let mut layout = PersistedLayoutState {
            oid: uuid::Uuid::new_v4().to_string(),
            rootnode: None,
            magnifiednodeid: String::new(),
            focusednodeid: String::new(),
            leaforder: None,
            pendingbackendactions: None,
            meta: None,
            ..Default::default()
        };
        wstore.insert(&mut layout)?;
        let mut tab = Tab {
            oid: tab_id.to_string(),
            name: name.to_string(),
            layoutstate: layout.oid.clone(),
            ..Default::default()
        };
        wstore.insert(&mut tab)?;
    }
    // Idempotently link tab into the workspace's tabids.
    if let Some(mut ws) = wstore.get::<Workspace>(workspace_id)? {
        let already_present =
            ws.tabids.iter().any(|t| t == tab_id) || ws.pinnedtabids.iter().any(|t| t == tab_id);
        if !already_present {
            ws.tabids.push(tab_id.to_string());
            wstore.update(&mut ws)?;
        }
    }
    Ok(())
}

fn apply_tab_deleted(
    wstore: &WaveStore,
    workspace_id: &str,
    tab_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if wstore.get::<Tab>(tab_id)?.is_none() {
        return Ok(()); // already gone (or never existed)
    }
    // wcore::delete_tab handles unlinking from the workspace's
    // tabids, deleting the tab's blocks + layout, and the tab row
    // itself. Returns NotFound errors if any of those don't exist —
    // we surface those as the operation having already happened.
    match wcore::delete_tab(wstore, workspace_id, tab_id) {
        Ok(()) => Ok(()),
        Err(crate::backend::storage::StoreError::NotFound) => Ok(()),
        Err(e) => Err(Box::new(e)),
    }
}

fn apply_active_tab_changed(
    wstore: &WaveStore,
    workspace_id: &str,
    tab_id: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(mut ws) = wstore.get::<Workspace>(workspace_id)? else {
        return Ok(()); // workspace already gone — nothing to update
    };
    let new = tab_id.unwrap_or("").to_string();
    if ws.activetabid == new {
        return Ok(());
    }
    ws.activetabid = new;
    wstore.update(&mut ws)?;
    Ok(())
}

fn apply_tab_reordered(
    wstore: &WaveStore,
    workspace_id: &str,
    tab_id: &str,
    new_index: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(mut ws) = wstore.get::<Workspace>(workspace_id)? else {
        return Ok(());
    };
    // Pinning was removed from AgentMux but legacy SQLite databases
    // can still have entries in `Workspace.pinnedtabids`; bootstrap
    // surfaces those as regular tabs in the reducer's `tab_ids`. The
    // reducer can therefore emit a TabReordered for a tab that on
    // disk still lives in `pinnedtabids`. Search both lists so the
    // reorder lands wherever the tab actually is. (codex P2 #617.)
    let target_index = (new_index as usize);
    if let Some(current_pos) = ws.tabids.iter().position(|t| t == tab_id) {
        let len = ws.tabids.len();
        let target = target_index.min(len.saturating_sub(1));
        if current_pos == target {
            return Ok(());
        }
        let id = ws.tabids.remove(current_pos);
        ws.tabids.insert(target, id);
        wstore.update(&mut ws)?;
    } else if let Some(current_pos) = ws.pinnedtabids.iter().position(|t| t == tab_id) {
        let len = ws.pinnedtabids.len();
        let target = target_index.min(len.saturating_sub(1));
        if current_pos == target {
            return Ok(());
        }
        let id = ws.pinnedtabids.remove(current_pos);
        ws.pinnedtabids.insert(target, id);
        wstore.update(&mut ws)?;
    }
    // Tab not in either list — silent no-op (idempotent).
    Ok(())
}

fn apply_block_created(
    wstore: &WaveStore,
    tab_id: &str,
    block_id: &str,
    meta: &serde_json::Value,
) -> Result<(), Box<dyn std::error::Error>> {
    if wstore.get::<Block>(block_id)?.is_none() {
        // Phase E.2c.4 — write the meta map carried in the event
        // (`view`, layout hints, etc.) so reducer-routed CreateBlock
        // RPC produces blocks with valid view/meta in SQLite. Without
        // this, the frontend sees a block with empty meta and renders
        // a blank pane.
        let meta_map: crate::backend::obj::MetaMapType = match meta {
            serde_json::Value::Object(_) => serde_json::from_value(meta.clone())
                .unwrap_or_default(),
            _ => Default::default(),
        };
        let mut block = Block {
            oid: block_id.to_string(),
            parentoref: format!("tab:{}", tab_id),
            meta: meta_map,
            ..Default::default()
        };
        wstore.insert(&mut block)?;
    }
    if let Some(mut tab) = wstore.get::<Tab>(tab_id)? {
        if !tab.blockids.iter().any(|b| b == block_id) {
            tab.blockids.push(block_id.to_string());
            wstore.update(&mut tab)?;
        }
    }
    Ok(())
}

fn apply_block_deleted(
    wstore: &WaveStore,
    tab_id: &str,
    block_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if wstore.get::<Block>(block_id)?.is_none() {
        return Ok(()); // already gone
    }
    match wcore::delete_block(wstore, tab_id, block_id) {
        Ok(()) => Ok(()),
        Err(crate::backend::storage::StoreError::NotFound) => Ok(()),
        Err(e) => Err(Box::new(e)),
    }
}

/// Phase E.5 — record/update a window's workspace pointer on the
/// persisted Window row. Idempotent: inserts if missing, updates
/// if `workspaceid` differs, no-ops if identical.
///
/// Also keeps `Client.windowids` in sync — appends the new window_id
/// when the Window row is freshly created. The legacy
/// `wcore::create_window_full` path does this too; without it, the
/// Window row exists but `GetClientData` / focus-order logic can't
/// see it. (codex P1 #619.)
fn apply_srv_window_opened(
    wstore: &WaveStore,
    window_id: &str,
    workspace_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let was_new = match wstore.get::<Window>(window_id)? {
        Some(existing) if existing.workspaceid == workspace_id => false,
        Some(mut existing) => {
            existing.workspaceid = workspace_id.to_string();
            wstore.update(&mut existing)?;
            false
        }
        None => {
            let mut window = Window {
                oid: window_id.to_string(),
                workspaceid: workspace_id.to_string(),
                ..Default::default()
            };
            wstore.insert(&mut window)?;
            true
        }
    };
    if was_new {
        if let Ok(mut client) = wcore::get_client(wstore) {
            if !client.windowids.iter().any(|id| id == window_id) {
                client.windowids.push(window_id.to_string());
                wstore.update(&mut client)?;
            }
        }
    }
    Ok(())
}

/// Phase E.5 — delete the persisted Window row + remove the id
/// from `Client.windowids`. Mirrors `wcore::close_window`'s order:
/// prune client.windowids FIRST (so any read between the two ops
/// doesn't see a dangling id), then delete the Window row.
/// (codex P1 #619.)
fn apply_srv_window_closed(
    wstore: &WaveStore,
    window_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if wstore.get::<Window>(window_id)?.is_none() {
        // Window already gone, but the client list might still have
        // the id from an earlier divergence — prune defensively.
        if let Ok(mut client) = wcore::get_client(wstore) {
            if client.windowids.iter().any(|id| id == window_id) {
                client.windowids.retain(|id| id != window_id);
                wstore.update(&mut client)?;
            }
        }
        return Ok(());
    }
    if let Ok(mut client) = wcore::get_client(wstore) {
        if client.windowids.iter().any(|id| id == window_id) {
            client.windowids.retain(|id| id != window_id);
            wstore.update(&mut client)?;
        }
    }
    wstore.delete::<Window>(window_id)?;
    Ok(())
}

/// Phase E.5 — same shape as `apply_srv_window_opened` for the
/// upsert behavior; separate function for log-clarity since the
/// emitted event is distinct.
fn apply_srv_window_workspace_changed(
    wstore: &WaveStore,
    window_id: &str,
    workspace_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    apply_srv_window_opened(wstore, window_id, workspace_id)
}

/// Phase E.5.3 — replace the workspace's `tabids` with the new
/// list and drain any legacy `pinnedtabids` rows. Pinning was a
/// Waveterm feature removed from AgentMux; bootstrap merges legacy
/// pinned tabs into the reducer's `tab_ids`, so the next reorder
/// is the canonical full ordering. Leaving stale `pinnedtabids`
/// in SQLite would cause UI double-insertion (`workspace.tsx`
/// builds the displayed tab list as `[...pinnedtabids, ...tabids]`).
fn apply_tabs_reordered_bulk(
    wstore: &WaveStore,
    workspace_id: &str,
    tab_ids: &[String],
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(mut ws) = wstore.get::<Workspace>(workspace_id)? else {
        return Ok(());
    };
    let tabids_match = ws.tabids == tab_ids;
    let pinned_clear = ws.pinnedtabids.is_empty();
    if tabids_match && pinned_clear {
        return Ok(());
    }
    ws.tabids = tab_ids.to_vec();
    ws.pinnedtabids.clear();
    wstore.update(&mut ws)?;
    Ok(())
}

/// Phase E.5.3 — rename a persisted workspace.
fn apply_workspace_renamed(
    wstore: &WaveStore,
    workspace_id: &str,
    name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(mut ws) = wstore.get::<Workspace>(workspace_id)? else {
        return Ok(());
    };
    if ws.name == name {
        return Ok(());
    }
    ws.name = name.to_string();
    wstore.update(&mut ws)?;
    Ok(())
}

/// Phase E.5.3 — rename a persisted tab.
fn apply_tab_renamed(
    wstore: &WaveStore,
    tab_id: &str,
    name: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(mut tab) = wstore.get::<Tab>(tab_id)? else {
        return Ok(());
    };
    if tab.name == name {
        return Ok(());
    }
    tab.name = name.to_string();
    wstore.update(&mut tab)?;
    Ok(())
}

/// Phase E.5.3 — apply a meta-patch to a workspace's `meta` map.
/// Reducer doesn't track meta in `WorkspaceRecord`; this subscriber
/// is the sole authority that mutates persisted meta. Patch is a
/// JSON object that merges shallow-key-by-shallow-key on top of the
/// existing meta. `null` values in the patch delete the key.
fn apply_workspace_meta_updated(
    wstore: &WaveStore,
    workspace_id: &str,
    meta_patch: &serde_json::Value,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(mut ws) = wstore.get::<Workspace>(workspace_id)? else {
        return Ok(());
    };
    if merge_meta_patch(&mut ws.meta, meta_patch) {
        wstore.update(&mut ws)?;
    }
    Ok(())
}

/// Phase E.5.3 — apply a meta-patch to a tab's `meta` map.
fn apply_tab_meta_updated(
    wstore: &WaveStore,
    tab_id: &str,
    meta_patch: &serde_json::Value,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(mut tab) = wstore.get::<Tab>(tab_id)? else {
        return Ok(());
    };
    if merge_meta_patch(&mut tab.meta, meta_patch) {
        wstore.update(&mut tab)?;
    }
    Ok(())
}

/// Phase E.5.3 — apply a meta-patch to a block's `meta` map.
fn apply_block_meta_updated(
    wstore: &WaveStore,
    block_id: &str,
    meta_patch: &serde_json::Value,
) -> Result<(), Box<dyn std::error::Error>> {
    let Some(mut block) = wstore.get::<Block>(block_id)? else {
        return Ok(());
    };
    if merge_meta_patch(&mut block.meta, meta_patch) {
        wstore.update(&mut block)?;
    }
    Ok(())
}

/// Phase E.5.5 — apply a `TabMoved` event to SQLite.
/// Idempotent: detects already-moved state (tab in dst, not in src)
/// and returns `Ok(())` without re-mutating.
///
/// What this writes:
/// * Removes `tab_id` from `src_workspace.tabids` (and `pinnedtabids`
///   for legacy rows — bootstrap merges them into reducer state, so
///   any leftover here is a stray legacy entry that should be drained).
/// * Updates `src_workspace.activetabid` to `new_src_active_tab_id`
///   (which may be empty when the source emptied).
/// * Inserts `tab_id` at `dst_index` in `dst_workspace.tabids`,
///   clamping to the dst list length.
/// * Updates the `Tab` row's parent ref so loaders find it under
///   the new workspace.
fn apply_tab_moved(
    wstore: &WaveStore,
    tab_id: &str,
    src_workspace_id: &str,
    dst_workspace_id: &str,
    dst_index: u32,
    new_src_active_tab_id: Option<&str>,
    new_dst_active_tab_id: Option<&str>,
) -> Result<(), Box<dyn std::error::Error>> {
    // Source workspace: remove the tab and update activetabid.
    if let Some(mut src_ws) = wstore.get::<Workspace>(src_workspace_id)? {
        let len_before_tabids = src_ws.tabids.len();
        let len_before_pinned = src_ws.pinnedtabids.len();
        src_ws.tabids.retain(|id| id != tab_id);
        src_ws.pinnedtabids.retain(|id| id != tab_id);
        let new_active = new_src_active_tab_id.unwrap_or("").to_string();
        let active_changed = src_ws.activetabid != new_active;
        if src_ws.tabids.len() != len_before_tabids
            || src_ws.pinnedtabids.len() != len_before_pinned
            || active_changed
        {
            src_ws.activetabid = new_active;
            wstore.update(&mut src_ws)?;
        }
    }

    // Dest workspace: insert at clamped index + update active_tab_id
    // if the event carries one (codex P2 #621). Skip insert if the
    // tab is already present (idempotent on duplicate delivery).
    if let Some(mut dst_ws) = wstore.get::<Workspace>(dst_workspace_id)? {
        let mut changed = false;
        if !dst_ws.tabids.iter().any(|id| id == tab_id) {
            let clamped = (dst_index as usize).min(dst_ws.tabids.len());
            dst_ws.tabids.insert(clamped, tab_id.to_string());
            changed = true;
        }
        if let Some(new_active) = new_dst_active_tab_id {
            if dst_ws.activetabid != new_active {
                dst_ws.activetabid = new_active.to_string();
                changed = true;
            }
        }
        if changed {
            wstore.update(&mut dst_ws)?;
        }
    }

    // No per-Tab parent ref to update — the workspace owns the
    // parentage relationship via its `tabids` list (no `Tab.workspaceid`
    // or `Tab.parentoref` field exists). Removing from the source
    // workspace's `tabids` and inserting into the dest's is the
    // entirety of the parent change.

    Ok(())
}

/// Phase E.5.5 — apply a `BlockMoved` event to SQLite.
/// Handles both cross-tab moves and intra-tab repositioning.
/// Idempotent on re-delivery (checks current parent before mutating).
fn apply_block_moved(
    wstore: &WaveStore,
    block_id: &str,
    src_tab_id: &str,
    dst_tab_id: &str,
    dst_index: u32,
) -> Result<(), Box<dyn std::error::Error>> {
    if src_tab_id == dst_tab_id {
        // Intra-tab reposition: remove + re-insert in the same tab.
        if let Some(mut tab) = wstore.get::<Tab>(src_tab_id)? {
            tab.blockids.retain(|id| id != block_id);
            let clamped = (dst_index as usize).min(tab.blockids.len());
            tab.blockids.insert(clamped, block_id.to_string());
            wstore.update(&mut tab)?;
        }
        return Ok(());
    }

    // Cross-tab: remove from src.
    if let Some(mut src_tab) = wstore.get::<Tab>(src_tab_id)? {
        let before = src_tab.blockids.len();
        src_tab.blockids.retain(|id| id != block_id);
        if src_tab.blockids.len() != before {
            wstore.update(&mut src_tab)?;
        }
    }

    // Insert into dst (skip if already there).
    if let Some(mut dst_tab) = wstore.get::<Tab>(dst_tab_id)? {
        if !dst_tab.blockids.iter().any(|id| id == block_id) {
            let clamped = (dst_index as usize).min(dst_tab.blockids.len());
            dst_tab.blockids.insert(clamped, block_id.to_string());
            wstore.update(&mut dst_tab)?;
        }
    }

    // Block row: update parent.
    if let Some(mut block) = wstore.get::<Block>(block_id)? {
        let new_parent = format!("tab:{}", dst_tab_id);
        if block.parentoref != new_parent {
            block.parentoref = new_parent;
            wstore.update(&mut block)?;
        }
    }

    Ok(())
}

/// Phase E.5.3 — shallow merge a JSON object patch into a
/// `MetaMapType`. Mirrors `backend::obj::merge_meta` semantics so
/// `UpdateObjectMeta` keeps behaving the same way after the reducer
/// migration:
/// - Keys ending in `:*` with a `true` value clear all keys with
///   that prefix (e.g. `{"term:*": true}` removes every `term*`
///   key) before regular merging.
/// - `null` patch values delete the corresponding key.
/// - Other values replace the key.
/// Returns `true` if anything actually changed.
fn merge_meta_patch(
    meta: &mut crate::backend::obj::MetaMapType,
    patch: &serde_json::Value,
) -> bool {
    let serde_json::Value::Object(patch_map) = patch else {
        return false;
    };
    let mut changed = false;
    // First pass: section-clear keys (`prefix:*` with `true`).
    for (k, v) in patch_map {
        if !k.ends_with(":*") {
            continue;
        }
        if !matches!(v, serde_json::Value::Bool(true)) {
            continue;
        }
        let prefix = k.trim_end_matches(":*");
        if prefix.is_empty() {
            continue;
        }
        let prefix_colon = format!("{prefix}:");
        let before = meta.len();
        meta.retain(|k2, _| k2 != prefix && !k2.starts_with(&prefix_colon));
        if meta.len() != before {
            changed = true;
        }
    }
    // Second pass: regular merges and null deletes.
    for (k, v) in patch_map {
        if k.ends_with(":*") {
            continue;
        }
        if v.is_null() {
            if meta.remove(k).is_some() {
                changed = true;
            }
            continue;
        }
        match meta.get(k) {
            Some(existing) if existing == v => {}
            _ => {
                meta.insert(k.clone(), v.clone());
                changed = true;
            }
        }
    }
    changed
}

/// Compact textual identifier for an event (debug logging only).
fn event_kind(event: &Event) -> &'static str {
    match event {
        Event::WorkspaceCreated { .. } => "WorkspaceCreated",
        Event::WorkspaceDeleted { .. } => "WorkspaceDeleted",
        Event::TabCreated { .. } => "TabCreated",
        Event::TabDeleted { .. } => "TabDeleted",
        Event::ActiveTabChanged { .. } => "ActiveTabChanged",
        Event::TabReordered { .. } => "TabReordered",
        Event::SrvWindowOpened { .. } => "SrvWindowOpened",
        Event::SrvWindowClosed { .. } => "SrvWindowClosed",
        Event::SrvWindowWorkspaceChanged { .. } => "SrvWindowWorkspaceChanged",
        Event::TabsReorderedBulk { .. } => "TabsReorderedBulk",
        Event::WorkspaceRenamed { .. } => "WorkspaceRenamed",
        Event::TabRenamed { .. } => "TabRenamed",
        Event::WorkspaceMetaUpdated { .. } => "WorkspaceMetaUpdated",
        Event::TabMetaUpdated { .. } => "TabMetaUpdated",
        Event::BlockMetaUpdated { .. } => "BlockMetaUpdated",
        Event::TabMoved { .. } => "TabMoved",
        Event::BlockMoved { .. } => "BlockMoved",
        Event::BlockCreated { .. } => "BlockCreated",
        Event::BlockDeleted { .. } => "BlockDeleted",
        _ => "Other",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::storage::wstore::WaveStore;

    fn store() -> Arc<WaveStore> {
        // In-memory SQLite for tests (matches existing wstore test pattern).
        let store = WaveStore::open_in_memory().expect("in-memory wstore");
        Arc::new(store)
    }

    #[test]
    fn workspace_created_inserts_row() {
        let s = store();
        apply_event_to_wstore(
            &Event::WorkspaceCreated {
                workspace_id: "ws-1".into(),
                name: "Alpha".into(),
                version: 1,
            },
            &s,
        )
        .unwrap();
        let ws = s.get::<Workspace>("ws-1").unwrap().unwrap();
        assert_eq!(ws.name, "Alpha");
    }

    #[test]
    fn workspace_created_idempotent_on_duplicate() {
        let s = store();
        let ev = Event::WorkspaceCreated {
            workspace_id: "ws-1".into(),
            name: "Alpha".into(),
            version: 1,
        };
        apply_event_to_wstore(&ev, &s).unwrap();
        // Second application should not error.
        apply_event_to_wstore(&ev, &s).unwrap();
        // Original name preserved (no overwrite).
        let ws = s.get::<Workspace>("ws-1").unwrap().unwrap();
        assert_eq!(ws.name, "Alpha");
    }

    #[test]
    fn workspace_deleted_silent_when_missing() {
        let s = store();
        apply_event_to_wstore(
            &Event::WorkspaceDeleted {
                workspace_id: "ghost".into(),
                version: 1,
            },
            &s,
        )
        .unwrap();
    }

    #[test]
    fn tab_created_links_into_workspace() {
        let s = store();
        apply_event_to_wstore(
            &Event::WorkspaceCreated {
                workspace_id: "ws-1".into(),
                name: "Alpha".into(),
                version: 1,
            },
            &s,
        )
        .unwrap();
        apply_event_to_wstore(
            &Event::TabCreated {
                workspace_id: "ws-1".into(),
                tab_id: "tab-1".into(),
                name: "Tab".into(),
                version: 2,
            },
            &s,
        )
        .unwrap();
        let ws = s.get::<Workspace>("ws-1").unwrap().unwrap();
        assert_eq!(ws.tabids, vec!["tab-1".to_string()]);
        let tab = s.get::<Tab>("tab-1").unwrap().unwrap();
        assert_eq!(tab.name, "Tab");
    }

    #[test]
    fn tab_created_idempotent_on_duplicate_link() {
        let s = store();
        apply_event_to_wstore(
            &Event::WorkspaceCreated {
                workspace_id: "ws-1".into(),
                name: "Alpha".into(),
                version: 1,
            },
            &s,
        )
        .unwrap();
        let ev = Event::TabCreated {
            workspace_id: "ws-1".into(),
            tab_id: "tab-1".into(),
            name: "Tab".into(),
            version: 2,
        };
        apply_event_to_wstore(&ev, &s).unwrap();
        apply_event_to_wstore(&ev, &s).unwrap();
        let ws = s.get::<Workspace>("ws-1").unwrap().unwrap();
        assert_eq!(ws.tabids.len(), 1);
    }

    #[test]
    fn active_tab_changed_updates_workspace() {
        let s = store();
        apply_event_to_wstore(
            &Event::WorkspaceCreated {
                workspace_id: "ws-1".into(),
                name: "Alpha".into(),
                version: 1,
            },
            &s,
        )
        .unwrap();
        apply_event_to_wstore(
            &Event::TabCreated {
                workspace_id: "ws-1".into(),
                tab_id: "tab-1".into(),
                name: "Tab".into(),
                version: 2,
            },
            &s,
        )
        .unwrap();
        apply_event_to_wstore(
            &Event::ActiveTabChanged {
                workspace_id: "ws-1".into(),
                tab_id: Some("tab-1".into()),
                version: 3,
            },
            &s,
        )
        .unwrap();
        let ws = s.get::<Workspace>("ws-1").unwrap().unwrap();
        assert_eq!(ws.activetabid, "tab-1");
    }

    #[test]
    fn active_tab_changed_to_none_clears_activetabid() {
        let s = store();
        apply_event_to_wstore(
            &Event::WorkspaceCreated {
                workspace_id: "ws-1".into(),
                name: "Alpha".into(),
                version: 1,
            },
            &s,
        )
        .unwrap();
        apply_event_to_wstore(
            &Event::TabCreated {
                workspace_id: "ws-1".into(),
                tab_id: "tab-1".into(),
                name: "Tab".into(),
                version: 2,
            },
            &s,
        )
        .unwrap();
        apply_event_to_wstore(
            &Event::ActiveTabChanged {
                workspace_id: "ws-1".into(),
                tab_id: Some("tab-1".into()),
                version: 3,
            },
            &s,
        )
        .unwrap();
        apply_event_to_wstore(
            &Event::ActiveTabChanged {
                workspace_id: "ws-1".into(),
                tab_id: None,
                version: 4,
            },
            &s,
        )
        .unwrap();
        let ws = s.get::<Workspace>("ws-1").unwrap().unwrap();
        assert_eq!(ws.activetabid, "");
    }

    #[test]
    fn tab_created_provisions_layoutstate() {
        let s = store();
        apply_event_to_wstore(
            &Event::WorkspaceCreated {
                workspace_id: "ws-1".into(),
                name: "Alpha".into(),
                version: 1,
            },
            &s,
        )
        .unwrap();
        apply_event_to_wstore(
            &Event::TabCreated {
                workspace_id: "ws-1".into(),
                tab_id: "tab-1".into(),
                name: "Tab".into(),
                version: 2,
            },
            &s,
        )
        .unwrap();
        let tab = s.get::<Tab>("tab-1").unwrap().unwrap();
        // codex P2 #614: tab must reference a real LayoutState row,
        // not be left with empty layoutstate.
        assert!(!tab.layoutstate.is_empty());
        let layout = s
            .get::<PersistedLayoutState>(&tab.layoutstate)
            .unwrap()
            .expect("layout row must exist");
        assert_eq!(layout.oid, tab.layoutstate);
    }

    #[test]
    fn workspace_deleted_cascades_pinned_tabs() {
        let s = store();
        // Build a workspace with a pinned tab via wcore (the bug
        // path: pinned tab created via wcore::create_tab_with_opts).
        let ws = wcore::create_workspace(&s, "Alpha").unwrap();
        let pinned_tab =
            wcore::create_tab_with_opts(&s, &ws.oid, "PinnedTab", true).unwrap();
        // Verify pinned tab is in pinnedtabids (sanity).
        let ws_loaded = s.get::<Workspace>(&ws.oid).unwrap().unwrap();
        assert!(ws_loaded.pinnedtabids.contains(&pinned_tab.oid));
        // Delete via the subscriber's WorkspaceDeleted handler.
        apply_event_to_wstore(
            &Event::WorkspaceDeleted {
                workspace_id: ws.oid.clone(),
                version: 1,
            },
            &s,
        )
        .unwrap();
        // codex P1 #614: pinned tab must be cascade-deleted, not
        // orphaned.
        assert!(s.get::<Tab>(&pinned_tab.oid).unwrap().is_none());
        assert!(s.get::<Workspace>(&ws.oid).unwrap().is_none());
    }

    #[test]
    fn block_created_links_into_tab() {
        let s = store();
        apply_event_to_wstore(
            &Event::WorkspaceCreated {
                workspace_id: "ws-1".into(),
                name: "Alpha".into(),
                version: 1,
            },
            &s,
        )
        .unwrap();
        apply_event_to_wstore(
            &Event::TabCreated {
                workspace_id: "ws-1".into(),
                tab_id: "tab-1".into(),
                name: "Tab".into(),
                version: 2,
            },
            &s,
        )
        .unwrap();
        apply_event_to_wstore(
            &Event::BlockCreated { tab_id: "tab-1".into(), block_id: "block-1".into(), meta: serde_json::Value::Null, version: 3 },
            &s,
        )
        .unwrap();
        let tab = s.get::<Tab>("tab-1").unwrap().unwrap();
        assert_eq!(tab.blockids, vec!["block-1".to_string()]);
        let block = s.get::<Block>("block-1").unwrap().unwrap();
        assert_eq!(block.parentoref, "tab:tab-1");
    }

    #[test]
    fn block_deleted_unlinks_from_tab() {
        let s = store();
        apply_event_to_wstore(
            &Event::WorkspaceCreated {
                workspace_id: "ws-1".into(),
                name: "Alpha".into(),
                version: 1,
            },
            &s,
        )
        .unwrap();
        apply_event_to_wstore(
            &Event::TabCreated {
                workspace_id: "ws-1".into(),
                tab_id: "tab-1".into(),
                name: "Tab".into(),
                version: 2,
            },
            &s,
        )
        .unwrap();
        apply_event_to_wstore(
            &Event::BlockCreated { tab_id: "tab-1".into(), block_id: "block-1".into(), meta: serde_json::Value::Null, version: 3 },
            &s,
        )
        .unwrap();
        apply_event_to_wstore(
            &Event::BlockDeleted {
                tab_id: "tab-1".into(),
                block_id: "block-1".into(),
                version: 4,
            },
            &s,
        )
        .unwrap();
        assert!(s.get::<Block>("block-1").unwrap().is_none());
        let tab = s.get::<Tab>("tab-1").unwrap().unwrap();
        assert!(tab.blockids.is_empty());
    }

    /// codex P1 #620: a workspace with legacy `pinnedtabids` rows must
    /// have those drained the first time `TabsReorderedBulk` writes
    /// the workspace; otherwise the UI's
    /// `[...pinnedtabids, ...tabids]` combine duplicates the pinned
    /// tab IDs once they have been merged into the reducer's
    /// `tab_ids` at bootstrap.
    #[test]
    fn tabs_reordered_bulk_drains_legacy_pinned_tabids() {
        let s = store();
        let ws = wcore::create_workspace(&s, "Alpha").unwrap();
        let pinned_tab =
            wcore::create_tab_with_opts(&s, &ws.oid, "Pinned", true).unwrap();
        let regular_tab =
            wcore::create_tab_with_opts(&s, &ws.oid, "Regular", false).unwrap();
        // Sanity: pinned tab is in `pinnedtabids` on disk.
        let ws_before = s.get::<Workspace>(&ws.oid).unwrap().unwrap();
        assert!(ws_before.pinnedtabids.contains(&pinned_tab.oid));

        // Reducer-driven bulk reorder treating the pinned tab as a
        // regular tab (mirrors what bootstrap-merge produces).
        apply_event_to_wstore(
            &Event::TabsReorderedBulk {
                workspace_id: ws.oid.clone(),
                tab_ids: vec![pinned_tab.oid.clone(), regular_tab.oid.clone()],
                version: ws_before.version as u64 + 1,
            },
            &s,
        )
        .unwrap();
        let ws_after = s.get::<Workspace>(&ws.oid).unwrap().unwrap();
        assert_eq!(
            ws_after.tabids,
            vec![pinned_tab.oid.clone(), regular_tab.oid.clone()]
        );
        assert!(
            ws_after.pinnedtabids.is_empty(),
            "pinnedtabids must be drained, was {:?}",
            ws_after.pinnedtabids
        );
    }

    /// codex P2 #620: `merge_meta_patch` must honour the existing
    /// `section:*` clear-prefix semantics so `UpdateObjectMeta`
    /// behaviour stays the same after the reducer migration.
    #[test]
    fn meta_updated_clears_section_prefix() {
        let s = store();
        apply_event_to_wstore(
            &Event::TabCreated {
                workspace_id: "ws-1".into(),
                tab_id: "tab-1".into(),
                name: "Tab".into(),
                version: 1,
            },
            &s,
        )
        .unwrap();
        apply_event_to_wstore(
            &Event::WorkspaceCreated {
                workspace_id: "ws-1".into(),
                name: "Alpha".into(),
                version: 1,
            },
            &s,
        )
        .unwrap();
        // Seed the tab with grouped meta entries.
        let mut tab = s.get::<Tab>("tab-1").unwrap().unwrap();
        tab.meta
            .insert("term:fontsize".into(), serde_json::json!(14));
        tab.meta
            .insert("term:theme".into(), serde_json::json!("solarized"));
        tab.meta.insert("name".into(), serde_json::json!("keep"));
        s.update(&mut tab).unwrap();
        // Patch with `term:*` clear plus a single replacement key.
        apply_event_to_wstore(
            &Event::TabMetaUpdated {
                tab_id: "tab-1".into(),
                meta_patch: serde_json::json!({
                    "term:*": true,
                    "term:fontsize": 18,
                }),
                version: 2,
            },
            &s,
        )
        .unwrap();
        let after = s.get::<Tab>("tab-1").unwrap().unwrap();
        assert!(!after.meta.contains_key("term:theme"),
            "term:theme should be cleared by `term:*` patch");
        assert_eq!(
            after.meta.get("term:fontsize"),
            Some(&serde_json::json!(18)),
            "term:fontsize replacement must take effect after the section clear"
        );
        assert_eq!(after.meta.get("name"), Some(&serde_json::json!("keep")));
    }

    // ---- Phase E.5.5 — TabMoved / BlockMoved subscriber tests ----

    #[test]
    fn tab_moved_cross_workspace_rewrites_both_tabids() {
        let s = store();
        // Two workspaces, each pre-existing in SQLite.
        for (id, name) in &[("src-ws", "Src"), ("dst-ws", "Dst")] {
            apply_event_to_wstore(
                &Event::WorkspaceCreated {
                    workspace_id: id.to_string(),
                    name: name.to_string(),
                    version: 1,
                },
                &s,
            )
            .unwrap();
        }
        // Tab in src.
        apply_event_to_wstore(
            &Event::TabCreated {
                workspace_id: "src-ws".into(),
                tab_id: "tab-1".into(),
                name: "T".into(),
                version: 2,
            },
            &s,
        )
        .unwrap();
        let src = s.get::<Workspace>("src-ws").unwrap().unwrap();
        assert_eq!(src.tabids, vec!["tab-1".to_string()]);

        // Move it. Set dst active to the moved tab (per reducer
        // semantics + codex P2 #621).
        apply_event_to_wstore(
            &Event::TabMoved {
                tab_id: "tab-1".into(),
                src_workspace_id: "src-ws".into(),
                dst_workspace_id: "dst-ws".into(),
                dst_index: 0,
                new_src_active_tab_id: None,
                new_dst_active_tab_id: Some("tab-1".into()),
                version: 3,
            },
            &s,
        )
        .unwrap();
        let src = s.get::<Workspace>("src-ws").unwrap().unwrap();
        let dst = s.get::<Workspace>("dst-ws").unwrap().unwrap();
        assert!(src.tabids.is_empty());
        assert_eq!(src.activetabid, "");
        assert_eq!(dst.tabids, vec!["tab-1".to_string()]);
        assert_eq!(dst.activetabid, "tab-1", "dst active should be the moved tab");
    }

    #[test]
    fn tab_moved_idempotent_on_re_delivery() {
        let s = store();
        for (id, name) in &[("src-ws", "Src"), ("dst-ws", "Dst")] {
            apply_event_to_wstore(
                &Event::WorkspaceCreated {
                    workspace_id: id.to_string(),
                    name: name.to_string(),
                    version: 1,
                },
                &s,
            )
            .unwrap();
        }
        apply_event_to_wstore(
            &Event::TabCreated {
                workspace_id: "src-ws".into(),
                tab_id: "tab-1".into(),
                name: "T".into(),
                version: 2,
            },
            &s,
        )
        .unwrap();
        let ev = Event::TabMoved {
            tab_id: "tab-1".into(),
            src_workspace_id: "src-ws".into(),
            dst_workspace_id: "dst-ws".into(),
            dst_index: 0,
            new_src_active_tab_id: None,
            new_dst_active_tab_id: Some("tab-1".into()),
            version: 3,
        };
        apply_event_to_wstore(&ev, &s).unwrap();
        // Re-deliver the same event.
        apply_event_to_wstore(&ev, &s).unwrap();
        let dst = s.get::<Workspace>("dst-ws").unwrap().unwrap();
        // Still exactly one entry — no duplicate insert.
        assert_eq!(dst.tabids, vec!["tab-1".to_string()]);
    }

    #[test]
    fn block_moved_cross_tab_updates_block_lists_and_parent() {
        let s = store();
        let ws = wcore::create_workspace(&s, "W").unwrap();
        let src_tab = wcore::create_tab_with_opts(&s, &ws.oid, "src", false).unwrap();
        let dst_tab = wcore::create_tab_with_opts(&s, &ws.oid, "dst", false).unwrap();
        // Create a block in src via the subscriber path.
        apply_event_to_wstore(
            &Event::BlockCreated {
                tab_id: src_tab.oid.clone(),
                block_id: "blk-1".into(),
                meta: serde_json::Value::Null,
                version: 1,
            },
            &s,
        )
        .unwrap();
        let src_before = s.get::<Tab>(&src_tab.oid).unwrap().unwrap();
        assert_eq!(src_before.blockids, vec!["blk-1".to_string()]);

        apply_event_to_wstore(
            &Event::BlockMoved {
                block_id: "blk-1".into(),
                src_tab_id: src_tab.oid.clone(),
                dst_tab_id: dst_tab.oid.clone(),
                dst_index: 0,
                version: 2,
            },
            &s,
        )
        .unwrap();
        let src_after = s.get::<Tab>(&src_tab.oid).unwrap().unwrap();
        let dst_after = s.get::<Tab>(&dst_tab.oid).unwrap().unwrap();
        let block = s.get::<Block>("blk-1").unwrap().unwrap();
        assert!(src_after.blockids.is_empty());
        assert_eq!(dst_after.blockids, vec!["blk-1".to_string()]);
        assert_eq!(block.parentoref, format!("tab:{}", dst_tab.oid));
    }

    #[test]
    fn block_moved_intra_tab_repositions() {
        let s = store();
        let ws = wcore::create_workspace(&s, "W").unwrap();
        let tab = wcore::create_tab_with_opts(&s, &ws.oid, "t", false).unwrap();
        for id in &["b1", "b2", "b3"] {
            apply_event_to_wstore(
                &Event::BlockCreated {
                    tab_id: tab.oid.clone(),
                    block_id: id.to_string(),
                    meta: serde_json::Value::Null,
                    version: 1,
                },
                &s,
            )
            .unwrap();
        }
        // Move b1 to position 2 (post-removal end).
        apply_event_to_wstore(
            &Event::BlockMoved {
                block_id: "b1".into(),
                src_tab_id: tab.oid.clone(),
                dst_tab_id: tab.oid.clone(),
                dst_index: 2,
                version: 2,
            },
            &s,
        )
        .unwrap();
        let tab_after = s.get::<Tab>(&tab.oid).unwrap().unwrap();
        assert_eq!(tab_after.blockids, vec!["b2".to_string(), "b3".to_string(), "b1".to_string()]);
    }
}
