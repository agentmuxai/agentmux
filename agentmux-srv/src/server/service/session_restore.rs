// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Restore-on-relaunch (Feature 1 of
//! `docs/specs/SPEC_SESSION_RESTORE_AND_SAVED_LAYOUTS_2026_08_13.md`).
//!
//! A graceful window close today deliberately cascades a full delete of the
//! workspace/tabs/blocks (`e3a6f85c2`, wired to the last-window "main" case
//! by `4cbf856b7` — see `docs/retro/retro-pane-layout-restore-was-a-leak-not-a-feature-2026-08-13.md`),
//! so the next cold launch always finds `Client.windowids` empty and reseeds
//! the hardcoded default 3-pane layout (`window_create::default_three_pane_tree`).
//!
//! This module adds an independent, durable "what was open last" record —
//! written just before that destroy cascade runs (`window_close::handle_close_window`)
//! and consulted by the cold-launch path (`window_create::handle_create_window`)
//! before it falls back to the hardcoded seed. It deliberately does NOT touch
//! the destroy cascade itself (that fix is correct and must stay — orphaned
//! shell processes were the original bug) and does NOT reuse Pillar 1's
//! crash-reproject machinery (that's scoped to crash-only recovery from rows
//! a graceful close is specifically supposed to remove — see
//! `docs/specs/SPEC_PILLAR1_HOST_REPROJECT_DESIGN_2026_06_30.md` §3).
//!
//! Stored as a JSON blob under `Client.meta["session:last_topology"]` —
//! `Client` is already a durable singleton row with a generic `meta` sidecar
//! (the same "per-entity loosely-typed sidecar" pattern Pillar 1 Step 2 used
//! for floating-pane placement on `Block.meta`), so this needs no new
//! `StoreObj` type, table, or migration.

use std::collections::HashMap;

use agentmux_common::ipc::{Command, Event};
use agentmux_common::LayoutNode;
use serde_json::{json, Value};

use crate::backend::obj::{Block, Client, LayoutState, Tab, Workspace};
use crate::backend::storage::store::Store;
use crate::server::AppState;

use super::reducer_helpers::{dispatch_to_reducer, seed_layout_via_reducer};

const SNAPSHOT_META_KEY: &str = "session:last_topology";

fn placeholder(idx: usize) -> String {
    format!("__snap_block_{idx}__")
}

fn placeholder_idx(s: &str) -> Option<usize> {
    s.strip_prefix("__snap_block_")?.strip_suffix("__")?.parse().ok()
}

/// Read the given workspace's tabs/blocks/layout directly from the store and
/// build a durable, block-id-independent snapshot (real block ids are
/// replaced with positional placeholders — `CreateBlock` always assigns a
/// fresh id, so the snapshot can never reference one that will exist again).
/// Returns `None` for an empty/unknown workspace (nothing worth persisting).
pub(crate) fn snapshot_workspace(store: &Store, workspace_id: &str) -> Option<Value> {
    let workspace = store.get::<Workspace>(workspace_id).ok().flatten()?;
    let mut tabs = Vec::new();
    for tab_id in &workspace.tabids {
        let Some(tab) = store.get::<Tab>(tab_id).ok().flatten() else {
            continue;
        };
        let mut idx_by_block_id: HashMap<String, usize> = HashMap::new();
        let mut blocks = Vec::new();
        for (i, block_id) in tab.blockids.iter().enumerate() {
            let Some(block) = store.get::<Block>(block_id).ok().flatten() else {
                continue;
            };
            idx_by_block_id.insert(block_id.clone(), i);
            blocks.push(json!({ "meta": block.meta }));
        }
        if blocks.is_empty() {
            continue;
        }
        let (rootnode, focusednodeid) = if tab.layoutstate.is_empty() {
            (None, String::new())
        } else {
            match store.get::<LayoutState>(&tab.layoutstate) {
                Ok(Some(layout)) => {
                    let rootnode = layout.rootnode.map(|mut tree| {
                        placeholderize(&mut tree, &idx_by_block_id);
                        tree
                    });
                    (rootnode, layout.focusednodeid)
                }
                _ => (None, String::new()),
            }
        };
        tabs.push(json!({
            "name": tab.name,
            "blocks": blocks,
            "rootnode": rootnode,
            "focusednodeid": focusednodeid,
        }));
    }
    if tabs.is_empty() {
        return None;
    }
    Some(json!({ "tabs": tabs }))
}

/// Replace every leaf's real block id (in `data.block_id`, `block_stack`,
/// `active_block_id`) with a positional placeholder, in place.
fn placeholderize(node: &mut LayoutNode, idx_by_block_id: &HashMap<String, usize>) {
    if let Some(data) = node.data.as_mut() {
        if let Some(&idx) = idx_by_block_id.get(&data.block_id) {
            data.block_id = placeholder(idx);
        }
        for id in data.block_stack.iter_mut() {
            if let Some(&idx) = idx_by_block_id.get(id.as_str()) {
                *id = placeholder(idx);
            }
        }
        if let Some(&idx) = idx_by_block_id.get(&data.active_block_id) {
            data.active_block_id = placeholder(idx);
        }
    }
    for child in node.children.iter_mut() {
        placeholderize(child, idx_by_block_id);
    }
}

/// Replace every leaf's positional placeholder with the corresponding freshly
/// created block id, in place. A placeholder with no matching entry (should
/// not happen — `new_ids` is built 1:1 from the same snapshot's `blocks`
/// array — but tolerated defensively) is left as-is; such a leaf simply won't
/// resolve to a real block and the frontend already handles unknown block ids
/// in a layout leaf as an empty pane, matching how a manually-crafted or
/// corrupted layout degrades today.
fn resolve_placeholders(node: &mut LayoutNode, new_ids: &[String]) {
    if let Some(data) = node.data.as_mut() {
        if let Some(idx) = placeholder_idx(&data.block_id) {
            if let Some(real) = new_ids.get(idx) {
                data.block_id = real.clone();
            }
        }
        for id in data.block_stack.iter_mut() {
            if let Some(idx) = placeholder_idx(id) {
                if let Some(real) = new_ids.get(idx) {
                    *id = real.clone();
                }
            }
        }
        if let Some(idx) = placeholder_idx(&data.active_block_id) {
            if let Some(real) = new_ids.get(idx) {
                data.active_block_id = real.clone();
            }
        }
    }
    for child in node.children.iter_mut() {
        resolve_placeholders(child, new_ids);
    }
}

/// Persist a snapshot as the durable "last session" record, overwriting
/// whatever was there before. Best-effort: a write failure is logged, never
/// propagated — this must never block the close it rides alongside.
pub(crate) fn save_last_session_snapshot(store: &Store, snapshot: Value) {
    let Ok(clients) = store.get_all::<Client>() else {
        return;
    };
    let Some(mut client) = clients.into_iter().next() else {
        return;
    };
    client.meta.insert(SNAPSHOT_META_KEY.to_string(), snapshot);
    if let Err(e) = store.update(&mut client) {
        tracing::warn!(
            error = %e,
            "session_restore: failed to persist last-session snapshot"
        );
    }
}

fn load_last_session_snapshot(store: &Store) -> Option<Value> {
    let clients = store.get_all::<Client>().ok()?;
    let client = clients.into_iter().next()?;
    client.meta.get(SNAPSHOT_META_KEY).cloned()
}

fn find_error(events: &[Event]) -> Option<String> {
    events.iter().find_map(|e| match e {
        Event::Error { message, .. } => Some(message.clone()),
        _ => None,
    })
}

async fn apply_and_publish(
    state: &AppState,
    events: &[Event],
) -> Result<(), String> {
    for ev in events {
        if let Err(e) = crate::persist_subscriber::apply_event_to_wstore(ev, &state.wstore) {
            return Err(format!("SQLite write failed: {}", e));
        }
    }
    Ok(())
}

/// Attempt to replay the durable "last session" snapshot (if any) into a
/// fresh workspace: `CreateWorkspace` → per-tab `CreateTab` + per-block
/// `CreateBlock` (reusing each block's saved `meta`, so a restored agent pane
/// relaunches the same way opening a fresh one does) → `LayoutSetTree` with
/// the saved split tree (block ids remapped from placeholders to the newly
/// created ones).
///
/// Returns `Ok(None)` when there's nothing to restore (no snapshot, or every
/// tab in it failed to recreate) — the caller falls back to the hardcoded
/// default seed exactly as it did before this feature existed. Returns
/// `Err` only for a failure in the very first step (creating the workspace
/// itself); per-tab/per-block failures during replay are logged and skipped
/// rather than aborting the whole restore, so a partially-stale snapshot
/// (e.g. one tab's block meta no longer makes sense) still restores
/// everything else instead of failing shut.
pub(crate) async fn restore_last_session(
    state: &AppState,
) -> Result<Option<(String, Vec<Event>)>, String> {
    let Some(snapshot) = load_last_session_snapshot(&state.wstore) else {
        return Ok(None);
    };
    let tabs_json = snapshot
        .get("tabs")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    if tabs_json.is_empty() {
        return Ok(None);
    }

    let ws_events = dispatch_to_reducer(
        state,
        Command::CreateWorkspace { name: String::new() },
    )
    .await;
    if let Some(err) = find_error(&ws_events) {
        return Err(format!("restore_last_session: CreateWorkspace failed: {}", err));
    }
    apply_and_publish(state, &ws_events).await?;
    let Some(ws_id) = ws_events.iter().find_map(|e| match e {
        Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
        _ => None,
    }) else {
        return Err("restore_last_session: CreateWorkspace produced no WorkspaceCreated event".into());
    };

    let mut all_events = ws_events;
    let mut restored_any_tab = false;

    for tab_json in &tabs_json {
        let name = tab_json
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let tab_events = dispatch_to_reducer(
            state,
            Command::CreateTab { workspace_id: ws_id.clone(), name },
        )
        .await;
        if let Some(err) = find_error(&tab_events) {
            tracing::warn!(error = %err, "restore_last_session: CreateTab failed — skipping this tab");
            continue;
        }
        if apply_and_publish(state, &tab_events).await.is_err() {
            tracing::warn!("restore_last_session: CreateTab SQLite write failed — skipping this tab");
            continue;
        }
        let Some(tab_id) = tab_events.iter().find_map(|e| match e {
            Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
            _ => None,
        }) else {
            continue;
        };
        all_events.extend(tab_events);

        let blocks_json = tab_json
            .get("blocks")
            .and_then(|v| v.as_array())
            .cloned()
            .unwrap_or_default();
        let mut new_block_ids: Vec<String> = Vec::new();
        for block_json in &blocks_json {
            let meta = block_json.get("meta").cloned().unwrap_or(Value::Null);
            let blk_events = dispatch_to_reducer(
                state,
                Command::CreateBlock { tab_id: tab_id.clone(), meta },
            )
            .await;
            if let Some(err) = find_error(&blk_events) {
                tracing::warn!(error = %err, "restore_last_session: CreateBlock failed — skipping this block");
                continue;
            }
            if apply_and_publish(state, &blk_events).await.is_err() {
                tracing::warn!("restore_last_session: CreateBlock SQLite write failed — skipping this block");
                continue;
            }
            if let Some(block_id) = blk_events.iter().find_map(|e| match e {
                Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
                _ => None,
            }) {
                new_block_ids.push(block_id);
            }
            all_events.extend(blk_events);
        }

        if new_block_ids.is_empty() {
            continue;
        }
        restored_any_tab = true;

        if let Some(rootnode_json) = tab_json.get("rootnode").filter(|v| !v.is_null()) {
            if let Ok(mut tree) = serde_json::from_value::<LayoutNode>(rootnode_json.clone()) {
                resolve_placeholders(&mut tree, &new_block_ids);
                let focused = tab_json
                    .get("focusednodeid")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                if let Err(e) = seed_layout_via_reducer(state, &tab_id, tree, focused, Vec::new()).await {
                    tracing::warn!(
                        tab_id = %tab_id,
                        error = %e,
                        "restore_last_session: layout write failed — tab restored blank"
                    );
                }
            }
        }
    }

    if !restored_any_tab {
        // Nothing actually came back (every tab's CreateTab/CreateBlock
        // failed) — compensate the now-empty workspace so we don't leave a
        // stray row behind, and let the caller fall back to the default seed.
        let comp = dispatch_to_reducer(
            state,
            Command::DeleteWorkspace { workspace_id: ws_id.clone(), force: false },
        )
        .await;
        let _ = apply_and_publish(state, &comp).await;
        super::reducer_helpers::publish_events(state, &comp);
        return Ok(None);
    }

    Ok(Some((ws_id, all_events)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::server::tests::test_state;

    async fn dispatch_apply(state: &AppState, cmd: Command) -> Vec<Event> {
        let events = dispatch_to_reducer(state, cmd).await;
        for ev in &events {
            crate::persist_subscriber::apply_event_to_wstore(ev, &state.wstore).unwrap();
        }
        events
    }

    #[tokio::test]
    async fn no_snapshot_means_nothing_to_load() {
        let state = test_state();
        assert!(load_last_session_snapshot(&state.wstore).is_none());
    }

    #[tokio::test]
    async fn save_then_load_round_trips() {
        let state = test_state();
        let snap = json!({ "tabs": [{"name": "t", "blocks": [{"meta": {"view": "agent"}}]}] });
        save_last_session_snapshot(&state.wstore, snap.clone());
        assert_eq!(load_last_session_snapshot(&state.wstore), Some(snap));
    }

    #[tokio::test]
    async fn save_overwrites_prior_snapshot() {
        let state = test_state();
        save_last_session_snapshot(&state.wstore, json!({"tabs": [{"name": "old"}]}));
        save_last_session_snapshot(&state.wstore, json!({"tabs": [{"name": "new"}]}));
        let loaded = load_last_session_snapshot(&state.wstore).unwrap();
        assert_eq!(loaded["tabs"][0]["name"], "new");
    }

    #[tokio::test]
    async fn snapshot_workspace_captures_tab_and_block_meta() {
        let state = test_state();
        let ws_evs = dispatch_apply(&state, Command::CreateWorkspace { name: "w".into() }).await;
        let ws_id = ws_evs
            .iter()
            .find_map(|e| match e {
                Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
                _ => None,
            })
            .unwrap();
        let tab_evs = dispatch_apply(
            &state,
            Command::CreateTab { workspace_id: ws_id.clone(), name: "mytab".into() },
        )
        .await;
        let tab_id = tab_evs
            .iter()
            .find_map(|e| match e {
                Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
                _ => None,
            })
            .unwrap();
        dispatch_apply(
            &state,
            Command::CreateBlock {
                tab_id: tab_id.clone(),
                meta: json!({ "view": "agent", "agent:name": "test-agent" }),
            },
        )
        .await;

        let snap = snapshot_workspace(&state.wstore, &ws_id).expect("snapshot should be produced");
        let tabs = snap["tabs"].as_array().unwrap();
        assert_eq!(tabs.len(), 1);
        assert_eq!(tabs[0]["name"], "mytab");
        let blocks = tabs[0]["blocks"].as_array().unwrap();
        assert_eq!(blocks.len(), 1);
        assert_eq!(blocks[0]["meta"]["agent:name"], "test-agent");
    }

    #[tokio::test]
    async fn snapshot_of_unknown_workspace_is_none() {
        let state = test_state();
        assert!(snapshot_workspace(&state.wstore, "no-such-workspace").is_none());
    }

    #[tokio::test]
    async fn restore_with_no_snapshot_returns_none() {
        let state = test_state();
        let result = restore_last_session(&state).await.unwrap();
        assert!(result.is_none());
    }

    #[tokio::test]
    async fn round_trip_snapshot_then_restore_recreates_tabs_and_blocks() {
        let state = test_state();
        let ws_evs = dispatch_apply(&state, Command::CreateWorkspace { name: "w".into() }).await;
        let ws_id = ws_evs
            .iter()
            .find_map(|e| match e {
                Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
                _ => None,
            })
            .unwrap();
        let tab_evs = dispatch_apply(
            &state,
            Command::CreateTab { workspace_id: ws_id.clone(), name: "mytab".into() },
        )
        .await;
        let tab_id = tab_evs
            .iter()
            .find_map(|e| match e {
                Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
                _ => None,
            })
            .unwrap();
        let blk1 = dispatch_apply(
            &state,
            Command::CreateBlock { tab_id: tab_id.clone(), meta: json!({"view": "agent"}) },
        )
        .await
        .iter()
        .find_map(|e| match e {
            Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
            _ => None,
        })
        .unwrap();
        let blk2 = dispatch_apply(
            &state,
            Command::CreateBlock { tab_id: tab_id.clone(), meta: json!({"view": "sysinfo"}) },
        )
        .await
        .iter()
        .find_map(|e| match e {
            Event::BlockCreated { block_id, .. } => Some(block_id.clone()),
            _ => None,
        })
        .unwrap();
        // Give the tab a real split layout referencing both blocks.
        let tree = agentmux_common::LayoutNode {
            id: "root".into(),
            flex_direction: agentmux_common::FlexDirection::Row,
            size: 10.0,
            children: vec![
                agentmux_common::LayoutNode {
                    id: "left".into(),
                    size: 5.0,
                    data: Some(agentmux_common::LayoutNodeData { block_id: blk1.clone(), ..Default::default() }),
                    ..Default::default()
                },
                agentmux_common::LayoutNode {
                    id: "right".into(),
                    size: 5.0,
                    data: Some(agentmux_common::LayoutNodeData { block_id: blk2.clone(), ..Default::default() }),
                    ..Default::default()
                },
            ],
            data: None,
            extra: Default::default(),
        };
        seed_layout_via_reducer(&state, &tab_id, tree, "left".into(), Vec::new())
            .await
            .unwrap();

        // Snapshot + save exactly like the close hook does.
        let snap = snapshot_workspace(&state.wstore, &ws_id).expect("snapshot should be produced");
        save_last_session_snapshot(&state.wstore, snap);

        // Restore into a brand-new workspace.
        let (new_ws_id, _events) = restore_last_session(&state)
            .await
            .unwrap()
            .expect("restore should recreate the tab");
        assert_ne!(new_ws_id, ws_id, "restore creates a NEW workspace, not the deleted one");

        let new_workspace = state.wstore.get::<Workspace>(&new_ws_id).unwrap().unwrap();
        assert_eq!(new_workspace.tabids.len(), 1);
        let new_tab = state.wstore.get::<Tab>(&new_workspace.tabids[0]).unwrap().unwrap();
        assert_eq!(new_tab.name, "mytab");
        assert_eq!(new_tab.blockids.len(), 2);

        let new_blk1 = state.wstore.get::<Block>(&new_tab.blockids[0]).unwrap().unwrap();
        assert_eq!(new_blk1.meta.get("view").and_then(|v| v.as_str()), Some("agent"));
        let new_blk2 = state.wstore.get::<Block>(&new_tab.blockids[1]).unwrap().unwrap();
        assert_eq!(new_blk2.meta.get("view").and_then(|v| v.as_str()), Some("sysinfo"));

        // Layout tree was restored with the NEW block ids, not the old ones
        // or leftover placeholders.
        let new_layout = state
            .wstore
            .get::<LayoutState>(&new_tab.layoutstate)
            .unwrap()
            .unwrap();
        let rootnode = new_layout.rootnode.expect("layout tree restored");
        let leaf_block_ids: Vec<String> = rootnode
            .children
            .iter()
            .filter_map(|c| c.data.as_ref().map(|d| d.block_id.clone()))
            .collect();
        assert_eq!(leaf_block_ids, vec![new_tab.blockids[0].clone(), new_tab.blockids[1].clone()]);
        assert!(!leaf_block_ids.iter().any(|id| id.starts_with("__snap_block_")));
    }

    #[tokio::test]
    async fn restore_is_idempotent_source_snapshot_survives() {
        // A second restore attempt (e.g. the app crashed mid-restore and
        // relaunched again) must still find and replay the same snapshot —
        // restoring does not consume/clear it.
        let state = test_state();
        let ws_evs = dispatch_apply(&state, Command::CreateWorkspace { name: "w".into() }).await;
        let ws_id = ws_evs
            .iter()
            .find_map(|e| match e {
                Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
                _ => None,
            })
            .unwrap();
        let tab_evs = dispatch_apply(
            &state,
            Command::CreateTab { workspace_id: ws_id.clone(), name: "t".into() },
        )
        .await;
        let tab_id = tab_evs
            .iter()
            .find_map(|e| match e {
                Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
                _ => None,
            })
            .unwrap();
        dispatch_apply(
            &state,
            Command::CreateBlock { tab_id: tab_id.clone(), meta: json!({"view": "agent"}) },
        )
        .await;
        let snap = snapshot_workspace(&state.wstore, &ws_id).unwrap();
        save_last_session_snapshot(&state.wstore, snap);

        let (first_ws, _) = restore_last_session(&state).await.unwrap().unwrap();
        let (second_ws, _) = restore_last_session(&state).await.unwrap().unwrap();
        assert_ne!(first_ws, second_ws, "each restore call creates its own fresh workspace");
        assert!(state.wstore.get::<Workspace>(&first_ws).unwrap().is_some());
        assert!(state.wstore.get::<Workspace>(&second_ws).unwrap().is_some());
    }
}
