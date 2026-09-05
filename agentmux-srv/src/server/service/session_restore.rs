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
    // Position of `workspace.activetabid` WITHIN the successfully-captured
    // `tabs` array (reagentx P2 on PR #2560: previously never captured at
    // all, so a restore always focused whichever tab `CreateTab`'s
    // first-tab-wins default left active — see `wcore/tab.rs`'s
    // `ws.activetabid.is_empty()` check). Same "index by position in the
    // list that actually survived" reasoning as blocks below: if the
    // originally-active tab is one that fails to load, this simply stays
    // `None` and restore falls back to whatever `CreateTab` leaves active.
    let mut active_tab_index: Option<usize> = None;
    // `pinnedtabids` is a legacy field (pinning was removed from AgentMux —
    // see `tab_lifecycle.rs`); a workspace the reducer hasn't touched yet
    // (e.g. straight out of `ensure_initial_data`) can still have its one
    // tab sitting there instead of in `tabids` until the next
    // `TabsReordered` event drains it. Read both so a snapshot taken before
    // that drain doesn't silently see zero tabs.
    for tab_id in workspace.pinnedtabids.iter().chain(workspace.tabids.iter()) {
        let Some(tab) = store.get::<Tab>(tab_id).ok().flatten() else {
            continue;
        };
        // Index by position WITHIN `blocks` (the successfully-loaded list),
        // not position within `tab.blockids` (the full original list) —
        // reviewer-caught bug (reagentx P2 on PR #2560): a single unreadable
        // block previously desynced every subsequent placeholder index
        // against restore's `new_block_ids` (built from the same
        // successfully-recreated count), silently misattributing later
        // blocks' layout positions. `blocks.len()` at insert time is always
        // this block's actual final position, skipped entries included or
        // not.
        let mut idx_by_block_id: HashMap<String, usize> = HashMap::new();
        let mut blocks = Vec::new();
        for block_id in &tab.blockids {
            let Some(block) = store.get::<Block>(block_id).ok().flatten() else {
                continue;
            };
            idx_by_block_id.insert(block_id.clone(), blocks.len());
            blocks.push(json!({ "meta": block.meta }));
        }
        if blocks.is_empty() {
            continue;
        }
        let (rootnode, focusednodeid, magnifiednodeid) = if tab.layoutstate.is_empty() {
            (None, String::new(), String::new())
        } else {
            match store.get::<LayoutState>(&tab.layoutstate) {
                Ok(Some(layout)) => {
                    let rootnode = layout.rootnode.map(|mut tree| {
                        placeholderize(&mut tree, &idx_by_block_id);
                        tree
                    });
                    (rootnode, layout.focusednodeid, layout.magnifiednodeid)
                }
                _ => (None, String::new(), String::new()),
            }
        };
        if tab_id == &workspace.activetabid {
            active_tab_index = Some(tabs.len());
        }
        tabs.push(json!({
            "name": tab.name,
            "blocks": blocks,
            "rootnode": rootnode,
            "focusednodeid": focusednodeid,
            "magnifiednodeid": magnifiednodeid,
        }));
    }
    if tabs.is_empty() {
        return None;
    }
    Some(json!({ "tabs": tabs, "active_tab_index": active_tab_index }))
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
///
/// Reads-then-writes the singleton `Client` row inside a single
/// `with_tx` (reagentx P1 on PR #2560): the two used to be separate
/// `store.get_all`/`store.update` calls, unlike `persist_subscriber`'s
/// window handlers which already wrap this exact shape in a transaction.
/// A concurrent write to `Client` between the read and the write here
/// (e.g. another in-flight `CloseWindow` pruning `windowids`, or a second
/// concurrent snapshot save/clear) would silently lose whichever change
/// didn't win the race — a classic lost-update. `with_tx` holds the
/// store's connection lock for the whole read+write, so no other caller
/// can observe or write an intermediate state.
pub(crate) fn save_last_session_snapshot(store: &Store, snapshot: Value) {
    let result = store.with_tx(|tx| {
        let clients = tx.get_all::<Client>()?;
        let Some(mut client) = clients.into_iter().next() else {
            return Ok(());
        };
        client.meta.insert(SNAPSHOT_META_KEY.to_string(), snapshot);
        tx.update(&mut client)?;
        Ok(())
    });
    if let Err(e) = result {
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

/// Consume the durable "last session" record after a fully successful
/// restore (called only once `window_create::handle_create_window`'s final
/// window read-back succeeds — see that call site's comment for why not any
/// earlier). Best-effort, same as `save_last_session_snapshot`: a failure to
/// clear just means a future stray/misused restore call could replay this
/// snapshot again, which `client_windowids_empty`'s server-side check
/// (`handle_create_window`) already independently guards against — this is
/// defense in depth, not the only thing preventing duplicates.
pub(crate) fn clear_last_session_snapshot(store: &Store) {
    // Same `with_tx`-atomicity reasoning as `save_last_session_snapshot`
    // above (reagentx P1 on PR #2560) — this read-then-write was
    // previously two separate store calls.
    let result = store.with_tx(|tx| {
        let clients = tx.get_all::<Client>()?;
        let Some(mut client) = clients.into_iter().next() else {
            return Ok(());
        };
        if client.meta.remove(SNAPSHOT_META_KEY).is_none() {
            return Ok(()); // nothing to clear
        }
        tx.update(&mut client)?;
        Ok(())
    });
    if let Err(e) = result {
        tracing::warn!(
            error = %e,
            "session_restore: failed to clear last-session snapshot after restore"
        );
    }
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
    // Maps this tab's position in the ORIGINAL snapshot (`tabs_json`) to its
    // freshly-created id, for tabs that actually made it through restore —
    // resolves `active_tab_index` below the same way `resolve_placeholders`
    // resolves block placeholders: by position among what actually survived,
    // not raw snapshot index (a tab that fails to restore must not desync
    // this lookup for every tab after it).
    let mut new_tab_ids_by_snapshot_index: HashMap<usize, String> = HashMap::new();

    for (snapshot_idx, tab_json) in tabs_json.iter().enumerate() {
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
        new_tab_ids_by_snapshot_index.insert(snapshot_idx, tab_id.clone());

        if let Some(rootnode_json) = tab_json.get("rootnode").filter(|v| !v.is_null()) {
            if let Ok(mut tree) = serde_json::from_value::<LayoutNode>(rootnode_json.clone()) {
                resolve_placeholders(&mut tree, &new_block_ids);
                let focused = tab_json
                    .get("focusednodeid")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                // `magnifiednodeid` refers to a LayoutNode.id, not a block
                // id — like `focusednodeid`, it survives the snapshot
                // round-trip verbatim (only `data.block_id`/`block_stack`/
                // `active_block_id` get placeholder-remapped, never
                // `LayoutNode.id` itself), so no resolve_placeholders-style
                // step is needed here. Previously never captured/passed at
                // all (reagentx P2 on PR #2560), so a tab closed with a
                // pane magnified always relaunched showing the full split
                // tree.
                let magnified = tab_json
                    .get("magnifiednodeid")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string();
                if let Err(e) = seed_layout_via_reducer(state, &tab_id, tree, focused, Vec::new(), magnified).await {
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

    // Restore which tab was active (reagentx P2 on PR #2560: previously
    // never captured/restored at all — every relaunch focused whichever tab
    // `CreateTab`'s own first-tab-wins default (`wcore/tab.rs`) happened to
    // leave active, i.e. always the first). `None` (no index in the
    // snapshot, or that index's tab didn't survive restore) leaves that
    // default in place rather than erroring — an unset/stale active tab is
    // a cosmetic miss, not a reason to fail the whole restore.
    if let Some(active_snapshot_idx) = snapshot.get("active_tab_index").and_then(|v| v.as_u64()) {
        if let Some(new_active_tab_id) = new_tab_ids_by_snapshot_index.get(&(active_snapshot_idx as usize)) {
            let active_events = dispatch_to_reducer(
                state,
                Command::SetActiveTab {
                    workspace_id: ws_id.clone(),
                    tab_id: new_active_tab_id.clone(),
                },
            )
            .await;
            if let Some(err) = find_error(&active_events) {
                tracing::warn!(error = %err, "restore_last_session: SetActiveTab failed — leaving default active tab");
            } else if apply_and_publish(state, &active_events).await.is_err() {
                tracing::warn!("restore_last_session: SetActiveTab SQLite write failed — leaving default active tab");
            } else {
                all_events.extend(active_events);
            }
        }
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
        seed_layout_via_reducer(&state, &tab_id, tree, "left".into(), Vec::new(), String::new())
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

    // reagentx P2 on PR #2560: `LayoutState.magnifiednodeid` was never
    // captured/restored, so a tab closed with a pane magnified always
    // relaunched showing the full split tree instead of the magnified pane.
    #[tokio::test]
    async fn snapshot_and_restore_preserves_magnifiednodeid() {
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
        // "left" pane is magnified.
        seed_layout_via_reducer(&state, &tab_id, tree, "left".into(), Vec::new(), "left".into())
            .await
            .unwrap();

        let snap = snapshot_workspace(&state.wstore, &ws_id).expect("snapshot should be produced");
        assert_eq!(snap["tabs"][0]["magnifiednodeid"], "left", "magnifiednodeid captured in snapshot");
        save_last_session_snapshot(&state.wstore, snap);

        let (new_ws_id, _events) = restore_last_session(&state).await.unwrap().unwrap();
        let new_workspace = state.wstore.get::<Workspace>(&new_ws_id).unwrap().unwrap();
        let new_tab = state.wstore.get::<Tab>(&new_workspace.tabids[0]).unwrap().unwrap();
        let new_layout = state.wstore.get::<LayoutState>(&new_tab.layoutstate).unwrap().unwrap();
        assert_eq!(
            new_layout.magnifiednodeid, "left",
            "magnifiednodeid restored — LayoutNode.id values aren't placeholder-remapped, \
             only block ids are, so this survives verbatim"
        );
    }

    // reagentx P2 on PR #2560: `Workspace.activetabid` was never
    // captured/restored, so a relaunch always focused the first tab
    // regardless of which was actually active at close.
    #[tokio::test]
    async fn snapshot_and_restore_preserves_active_tab_when_not_the_first() {
        let state = test_state();
        let ws_evs = dispatch_apply(&state, Command::CreateWorkspace { name: "w".into() }).await;
        let ws_id = ws_evs
            .iter()
            .find_map(|e| match e {
                Event::WorkspaceCreated { workspace_id, .. } => Some(workspace_id.clone()),
                _ => None,
            })
            .unwrap();

        // tab1 is created first (and would be the default active tab per
        // wcore/tab.rs's first-tab-wins rule) but tab2 is the one the user
        // actually left active.
        let tab1_evs = dispatch_apply(
            &state,
            Command::CreateTab { workspace_id: ws_id.clone(), name: "tab-one".into() },
        )
        .await;
        let tab1_id = tab1_evs
            .iter()
            .find_map(|e| match e {
                Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
                _ => None,
            })
            .unwrap();
        dispatch_apply(
            &state,
            Command::CreateBlock { tab_id: tab1_id.clone(), meta: json!({"view": "agent"}) },
        )
        .await;

        let tab2_evs = dispatch_apply(
            &state,
            Command::CreateTab { workspace_id: ws_id.clone(), name: "tab-two".into() },
        )
        .await;
        let tab2_id = tab2_evs
            .iter()
            .find_map(|e| match e {
                Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
                _ => None,
            })
            .unwrap();
        dispatch_apply(
            &state,
            Command::CreateBlock { tab_id: tab2_id.clone(), meta: json!({"view": "sysinfo"}) },
        )
        .await;

        dispatch_apply(
            &state,
            Command::SetActiveTab { workspace_id: ws_id.clone(), tab_id: tab2_id.clone() },
        )
        .await;
        let workspace_before = state.wstore.get::<Workspace>(&ws_id).unwrap().unwrap();
        assert_eq!(workspace_before.activetabid, tab2_id, "precondition: tab-two is active");

        let snap = snapshot_workspace(&state.wstore, &ws_id).expect("snapshot should be produced");
        assert_eq!(snap["active_tab_index"], 1, "active tab is the SECOND captured tab");
        save_last_session_snapshot(&state.wstore, snap);

        let (new_ws_id, _events) = restore_last_session(&state).await.unwrap().unwrap();
        let new_workspace = state.wstore.get::<Workspace>(&new_ws_id).unwrap().unwrap();
        assert_eq!(new_workspace.tabids.len(), 2);
        let new_tab2 = state.wstore.get::<Tab>(&new_workspace.tabids[1]).unwrap().unwrap();
        assert_eq!(new_tab2.name, "tab-two");
        assert_eq!(
            new_workspace.activetabid, new_tab2.oid,
            "the restored workspace's active tab is the NEW tab-two, not whichever \
             CreateTab's first-tab-wins default would otherwise leave active"
        );
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
