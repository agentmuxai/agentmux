// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase E.2c.1 — persist subscriber. A tokio task that consumes the
// srv reducer's broadcast bus and mirrors workspace/tab/block
// lifecycle events back to SQLite via wcore.
//
// Status in E.2c.1: PLUMBING ONLY.
//
// The subscriber is dead-code in production today because no
// in-process caller currently emits the Workspace/Tab/Block events
// it consumes — RPC handlers still write SQLite directly via wcore;
// the saga coordinator (E.5+) is empty. The subscriber's job in
// E.2c.1 is to establish the persist-back pattern so the saga
// coordinator and the future RPC migration (E.2c.2+) have a target
// to write against. Idempotent applies are exercised by the unit
// tests in this module.
//
// What this module does NOT do (intentionally):
//   * Full-resync on broadcast `Lagged`. Resync would write the
//     reducer's session-only state back to SQLite. That state is
//     FROZEN at bootstrap for any entity the reducer doesn't
//     mutate (i.e., everything RPC writes today). Doing a resync
//     before RPC is migrated would clobber those RPC-driven writes.
//     Resync lands in E.2c.2 alongside the workspace RPC migration,
//     where reducer state actually tracks live changes.
//   * Per-event ACK / sequence-number tracking. Bus is fire-and-
//     forget; subscriber position is whatever tokio broadcast says.
//   * Bus-lag recovery beyond a warning log. With no live event
//     producers in E.2c.1, lag is impossible in practice — adding
//     real recovery before producers exist is YAGNI. E.2c.2 brings
//     the producer (RPC migration) AND the recovery (full-resync).

use std::sync::Arc;

use agentmux_common::ipc::Event;
use tokio::sync::broadcast;

use crate::backend::obj::{Block, Tab, Workspace};
use crate::backend::storage::wstore::WaveStore;
use crate::backend::wcore;

/// Spawn the persist subscriber task. Runs until the broadcast
/// channel closes (i.e., the reducer's bus is dropped at process
/// shutdown).
pub fn spawn_persist_subscriber(
    events_rx: broadcast::Receiver<Event>,
    wstore: Arc<WaveStore>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(run_persist_subscriber(events_rx, wstore))
}

async fn run_persist_subscriber(
    mut events_rx: broadcast::Receiver<Event>,
    wstore: Arc<WaveStore>,
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
                // E.2c.1 — log only. Real recovery (full-resync from
                // reducer state) lands in E.2c.2 once the RPC
                // migration makes resync non-destructive. With no
                // live producers in E.2c.1 (saga coordinator empty,
                // RPC still writes wcore directly), Lagged is
                // unreachable — this arm exists only because the
                // tokio API requires handling it.
                tracing::warn!(
                    target: "srv-persist-subscriber",
                    "[srv-persist-subscriber] dropped {} event(s) — SQLite may diverge from reducer; resync lands in E.2c.2",
                    n
                );
            }
            Err(broadcast::error::RecvError::Closed) => {
                tracing::info!(target: "srv-persist-subscriber", "[srv-persist-subscriber] bus closed — exiting");
                return;
            }
        }
    }
}

/// Apply one reducer event to the on-disk store. Idempotent: each
/// arm checks for the entity's current SQLite state before writing
/// so duplicate events (from at-least-once delivery semantics) don't
/// produce duplicate rows or wcore errors.
fn apply_event_to_wstore(
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
        Event::BlockCreated {
            tab_id, block_id, ..
        } => apply_block_created(wstore, tab_id, block_id),
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
        let mut tab = Tab {
            oid: tab_id.to_string(),
            name: name.to_string(),
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

fn apply_block_created(
    wstore: &WaveStore,
    tab_id: &str,
    block_id: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    if wstore.get::<Block>(block_id)?.is_none() {
        let mut block = Block {
            oid: block_id.to_string(),
            parentoref: format!("tab:{}", tab_id),
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
            &Event::BlockCreated {
                tab_id: "tab-1".into(),
                block_id: "block-1".into(),
                version: 3,
            },
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
            &Event::BlockCreated {
                tab_id: "tab-1".into(),
                block_id: "block-1".into(),
                version: 3,
            },
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
