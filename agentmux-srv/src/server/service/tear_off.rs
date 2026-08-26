// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `workspace` service handlers — floating-pane / torn-off tab flows
//! (`TearOffBlock`, `RedockFloatingPane`, `TearOffTab`). Split out of
//! `workspace.rs`; see that file's dispatcher for the full method list.

use crate::backend::obj::*;
use crate::backend::service::{self, WebCallType, WebReturnType};

use super::super::AppState;
use super::layout_helpers::{
    queue_source_layout_delete, queue_target_layout_insert, queue_target_layout_split,
    setup_torn_off_block_layout,
};
use super::reducer_helpers::{dispatch_to_reducer, publish_events};

// Phase E.5.5 — TearOffBlock migrated to saga (reducer-state
// portion: CreateWorkspace + CreateTab + MoveBlock). Layout
// tree setup on the new tab + queueing the source tab's
// layout-delete action stay wcore-direct here — layout state
// is E.4 work, separately scoped. The saga's atomicity is
// limited to the reducer-state portion; layout writes are
// best-effort and can leave a torn-off block with a malformed
// layout if the post-saga step fails. Acceptable trade-off
// for the smoke regression fix; full atomicity is a Phase F+
// gap (see saga-coordinator-location-analysis-2026-04-30.md
// §4.2).
pub(crate) async fn handle_tear_off_block(state: &AppState, call: &WebCallType) -> WebReturnType {
    let store = &state.wstore;
    let args = &call.args;
    let block_id: String = match service::get_arg(args, 0) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    let source_tab_id: String = match service::get_arg(args, 1) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    let source_ws_id: String = match service::get_arg(args, 2) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    let auto_close: bool = service::get_arg(args, 3).unwrap_or(true);
    tracing::info!(block_id = %block_id, source_tab = %source_tab_id, source_ws = %source_ws_id, "[dnd:svc] TearOffBlock via saga");
    let saga_result = crate::sagas::tear_off_block::run(
        state,
        block_id.clone(),
        source_tab_id.clone(),
        source_ws_id.clone(),
    )
    .await;
    let (new_ws_oid, new_tab_oid) = match saga_result {
        Ok(value) => {
            let new_ws_oid = value
                .get("new_workspace_id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let new_tab_oid = value
                .get("new_tab_id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            (new_ws_oid, new_tab_oid)
        }
        Err(reason) => return WebReturnType::error(reason),
    };

    // Layout setup for the new tab — make the moved block its
    // single root node so the frontend renders it. Best-effort;
    // reducer-routed (SPEC_864 Phase 3 seed / Phase 4 queue).
    if let Err(e) = setup_torn_off_block_layout(state, &new_tab_oid, &block_id).await {
        tracing::warn!(new_tab = %new_tab_oid, "TearOffBlock: layout setup failed: {} (block in tab but layout malformed)", e);
    }
    // Source tab: queue a layout-delete action so the source
    // window's frontend removes the node from its tree.
    if let Err(e) = queue_source_layout_delete(state, &source_tab_id, &block_id).await {
        tracing::warn!(source_tab = %source_tab_id, "TearOffBlock: source layout delete-action enqueue failed: {}", e);
    }

    // Auto-close empty source tab. Route through the reducer
    // (DeleteTab cascade is built in; the tab has no blocks
    // at this point — we just moved the only one out). Skip
    // when source workspace would become empty.
    if auto_close {
        let should_close = match store.must_get::<Tab>(&source_tab_id) {
            Ok(t) => t.blockids.is_empty(),
            Err(_) => false,
        };
        if should_close {
            let total_tabs = match store.must_get::<Workspace>(&source_ws_id) {
                Ok(ws) => ws.tabids.len() + ws.pinnedtabids.len(),
                Err(_) => 0,
            };
            if total_tabs > 1 {
                tracing::info!(source_tab = %source_tab_id, "[dnd:svc] auto-closing empty source tab after TearOffBlock");
                let close_events = dispatch_to_reducer(
                    state,
                    agentmux_common::ipc::Command::DeleteTab {
                        workspace_id: source_ws_id.clone(),
                        tab_id: source_tab_id.clone(),
                        // Auto-close already gated on
                        // total_tabs > 1.
                        force: false,
                    },
                )
                .await;
                for ev in &close_events {
                    let _ = crate::persist_subscriber::apply_event_to_wstore(ev, store);
                }
                publish_events(state, &close_events);
            }
        }
    }

    let mut updates = Vec::new();
    if let Ok(src_tab) = store.must_get::<Tab>(&source_tab_id) {
        updates.push(WaveObjUpdate {
            updatetype: "update".into(),
            otype: OTYPE_TAB.to_string(),
            oid: source_tab_id.clone(),
            obj: Some(wave_obj_to_value(&src_tab)),
        });
    }
    if let Ok(src_ws) = store.must_get::<Workspace>(&source_ws_id) {
        updates.push(WaveObjUpdate {
            updatetype: "update".into(),
            otype: OTYPE_WORKSPACE.to_string(),
            oid: source_ws_id.clone(),
            obj: Some(wave_obj_to_value(&src_ws)),
        });
    }
    if let Ok(new_ws) = store.must_get::<Workspace>(&new_ws_oid) {
        updates.push(WaveObjUpdate {
            updatetype: "update".into(),
            otype: OTYPE_WORKSPACE.to_string(),
            oid: new_ws_oid.clone(),
            obj: Some(wave_obj_to_value(&new_ws)),
        });
    }
    WebReturnType::success_data_updates(
        serde_json::to_value(&new_ws_oid).unwrap_or_default(),
        updates,
    )
}

// Floating-pane Phase 4a — re-dock a floater's block back into
// an existing tab in another workspace. Same shape as
// TearOffBlock's RPC handler: saga handles the reducer-state
// move (MoveBlock); layout writes are wcore-direct (the
// target's layout grows a leaf; the source's layout enqueues
// a delete action). Source floater closes via PR #1089's
// empty-tab watcher once its tab.blockids is empty.
// Spec: docs/specs/SPEC_FLOATING_PANE_REDOCK_2026-05-27.md
pub(crate) async fn handle_redock_floating_pane(state: &AppState, call: &WebCallType) -> WebReturnType {
    let store = &state.wstore;
    let args = &call.args;
    let block_id: String = match service::get_arg(args, 0) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    let source_tab_id: String = match service::get_arg(args, 1) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    let source_ws_id: String = match service::get_arg(args, 2) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    let target_tab_id: String = match service::get_arg(args, 3) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    let target_ws_id: String = match service::get_arg(args, 4) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    // Phase 4b — optional direction hint from the ghost overlay.
    // Both must be present; if either is absent fall back to InsertNode.
    let target_block_id: Option<String> = service::get_optional_arg(args, 5)
        .unwrap_or(None);
    let direction: Option<u8> = service::get_optional_arg(args, 6)
        .unwrap_or(None);
    tracing::info!(
        block_id = %block_id,
        source_tab = %source_tab_id,
        source_ws = %source_ws_id,
        target_tab = %target_tab_id,
        target_ws = %target_ws_id,
        target_block_id = ?target_block_id,
        direction = ?direction,
        "[dnd:svc] RedockFloatingPane via saga"
    );
    let saga_result = crate::sagas::redock_floating_pane::run(
        state,
        block_id.clone(),
        source_tab_id.clone(),
        source_ws_id.clone(),
        target_tab_id.clone(),
        target_ws_id.clone(),
        None,
    )
    .await;
    if let Err(reason) = saga_result {
        return WebReturnType::error(reason);
    }

    // Target layout: enqueue the appropriate action on its
    // `pendingbackendactions`. Phase 4b: when a direction hint is
    // present, queue SplitHorizontal/SplitVertical so the block lands
    // in the exact slot the ghost previewed. Otherwise fall back to
    // the generic InsertNode (Phase 4a behavior).
    // Layout writes are required before the Tab broadcast — if either
    // fails the block becomes invisible. Return error so the caller can
    // retry; no visible change has propagated to the renderers yet.
    let target_layout_result = match (target_block_id.as_deref(), direction) {
        (Some(tbid), Some(dir)) => {
            queue_target_layout_split(state, &target_tab_id, &block_id, tbid, dir).await
        }
        _ => queue_target_layout_insert(state, &target_tab_id, &block_id).await,
    };
    if let Err(e) = target_layout_result {
        tracing::error!(
            target_tab = %target_tab_id,
            "RedockFloatingPane: target layout action failed — aborting broadcast: {}",
            e
        );
        return WebReturnType::error(format!(
            "redock layout action failed: {e}"
        ));
    }
    if let Err(e) = queue_source_layout_delete(state, &source_tab_id, &block_id).await {
        tracing::error!(
            source_tab = %source_tab_id,
            "RedockFloatingPane: source layout delete failed — aborting broadcast: {}",
            e
        );
        return WebReturnType::error(format!(
            "redock layout delete failed: {e}"
        ));
    }

    let mut updates = Vec::new();
    // Layout updates MUST come first — the queue helpers persist
    // via the reducer route (SPEC_864 Phase 4), whose SQLite write
    // is NOT auto-broadcast (only the row gets a new version).
    // Without these entries in the response, the target window's
    // frontend never sees the new leaf and renders nothing; the
    // source's pending delete action never gets pulled either.
    // Both layouts are read AFTER the helpers return — the
    // helpers persist synchronously inside the reducer lock, so
    // the re-read always sees the appended actions.
    if let Ok(src_tab) = store.must_get::<Tab>(&source_tab_id) {
        if let Ok(src_layout) = store.must_get::<LayoutState>(&src_tab.layoutstate) {
            updates.push(WaveObjUpdate {
                updatetype: "update".into(),
                otype: OTYPE_LAYOUT.to_string(),
                oid: src_tab.layoutstate.clone(),
                obj: Some(wave_obj_to_value(&src_layout)),
            });
        }
        updates.push(WaveObjUpdate {
            updatetype: "update".into(),
            otype: OTYPE_TAB.to_string(),
            oid: source_tab_id.clone(),
            obj: Some(wave_obj_to_value(&src_tab)),
        });
    }
    if let Ok(dst_tab) = store.must_get::<Tab>(&target_tab_id) {
        if let Ok(dst_layout) = store.must_get::<LayoutState>(&dst_tab.layoutstate) {
            updates.push(WaveObjUpdate {
                updatetype: "update".into(),
                otype: OTYPE_LAYOUT.to_string(),
                oid: dst_tab.layoutstate.clone(),
                obj: Some(wave_obj_to_value(&dst_layout)),
            });
        }
        updates.push(WaveObjUpdate {
            updatetype: "update".into(),
            otype: OTYPE_TAB.to_string(),
            oid: target_tab_id.clone(),
            obj: Some(wave_obj_to_value(&dst_tab)),
        });
    }
    // CRITICAL: WaveObjUpdates in the response only reach the
    // CALLING renderer (the floater that's about to close). The
    // TARGET window's renderer is a different process and won't
    // see the layout change unless we explicitly broadcast on
    // the event bus. Mirrors the pattern in `app_api.rs:399-410`.
    // Without this the target tab.blockids includes the new
    // block but its layout.leaforder doesn't → block invisible.
    // One batched frame so the renderer applies all of them in a single
    // reactive flush — see EventBus::broadcast_wave_obj_updates.
    state.event_bus.broadcast_wave_obj_updates(&updates);

    WebReturnType::success_data_updates(
        serde_json::json!({
            "redocked": true,
            "block_id": block_id,
            "target_tab_id": target_tab_id,
        }),
        updates,
    )
}

// Phase E.5.5 — TearOffTab migrated to saga. Closes the
// smoke regression where wcore::tear_off_tab created the new
// workspace bypassing the reducer, leaving the new window's
// CreateTab/etc. calls failing on "workspace not found"
// checks against the reducer's stale view.
pub(crate) async fn handle_tear_off_tab(state: &AppState, call: &WebCallType) -> WebReturnType {
    let store = &state.wstore;
    let args = &call.args;
    let tab_id: String = match service::get_arg(args, 0) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    let source_ws_id: String = match service::get_arg(args, 1) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    tracing::info!(tab_id = %tab_id, source_ws = %source_ws_id, "[dnd:svc] TearOffTab via saga");
    match crate::sagas::tear_off_tab::run(state, tab_id, source_ws_id.clone()).await {
        Ok(saga_result) => {
            let new_ws_oid = saga_result
                .get("new_workspace_id")
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string();
            let mut updates = Vec::new();
            if let Ok(src_ws) = store.must_get::<Workspace>(&source_ws_id) {
                updates.push(WaveObjUpdate {
                    updatetype: "update".into(),
                    otype: OTYPE_WORKSPACE.to_string(),
                    oid: source_ws_id.clone(),
                    obj: Some(wave_obj_to_value(&src_ws)),
                });
            }
            if let Ok(new_ws) = store.must_get::<Workspace>(&new_ws_oid) {
                updates.push(WaveObjUpdate {
                    updatetype: "update".into(),
                    otype: OTYPE_WORKSPACE.to_string(),
                    oid: new_ws_oid.clone(),
                    obj: Some(wave_obj_to_value(&new_ws)),
                });
            }
            WebReturnType::success_data_updates(
                serde_json::to_value(&new_ws_oid).unwrap_or_default(),
                updates,
            )
        }
        Err(reason) => WebReturnType::error(reason),
    }
}
