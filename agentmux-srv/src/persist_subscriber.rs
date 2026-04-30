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

use crate::backend::obj::{Block, LayoutState as PersistedLayoutState, Tab, Workspace};
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

/// Compact textual identifier for an event (debug logging only).
fn event_kind(event: &Event) -> &'static str {
    match event {
        Event::WorkspaceCreated { .. } => "WorkspaceCreated",
        Event::WorkspaceDeleted { .. } => "WorkspaceDeleted",
        Event::TabCreated { .. } => "TabCreated",
        Event::TabDeleted { .. } => "TabDeleted",
        Event::ActiveTabChanged { .. } => "ActiveTabChanged",
        Event::TabReordered { .. } => "TabReordered",
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
}
