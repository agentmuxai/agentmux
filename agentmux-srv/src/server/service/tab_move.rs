// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `workspace` service handlers — drag-and-drop block/tab moves
//! (`MoveBlockToTab`, `PromoteBlockToTab`, `MoveTabToWorkspace`,
//! `RestoreTornOffTab`). Split out of `workspace.rs`; see that file's
//! dispatcher for the full method list.

use crate::backend::obj::*;
use crate::backend::service::{self, WebCallType, WebReturnType};

use super::super::AppState;
use super::layout_helpers::{queue_source_layout_delete, setup_torn_off_block_layout};
use super::reducer_helpers::{dispatch_to_reducer, publish_events};

// Phase E.5.7 — MoveBlockToTab migrated to dispatch
// Command::MoveBlock through the reducer. Auto-close empty
// source tab still uses Command::DeleteTab. ws_id arg kept
// for backward compat — used only for the post-op SQLite
// refresh + auto-close workspace check.
pub(crate) async fn handle_move_block_to_tab(state: &AppState, call: &WebCallType) -> WebReturnType {
    let store = &state.wstore;
    let args = &call.args;
    let ws_id: String = match service::get_arg(args, 0) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    let block_id: String = match service::get_arg(args, 1) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    let source_tab_id: String = match service::get_arg(args, 2) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    let dest_tab_id: String = match service::get_arg(args, 3) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    let auto_close: bool = service::get_arg(args, 4).unwrap_or(true);
    tracing::info!(ws_id = %ws_id, block_id = %block_id, source_tab = %source_tab_id, dest_tab = %dest_tab_id, "[dnd:svc] MoveBlockToTab via reducer");
    // codex P2 #622: same-tab requests were no-ops in the
    // prior wcore handler. The reducer's MoveBlock treats
    // same source = dest as an in-tab reorder; with
    // `dst_index: u32::MAX` it would silently move the block
    // to the end of the list. Short-circuit to preserve the
    // prior contract — a `MoveBlockToTab` whose dest equals
    // the source is a UI quirk (e.g. drop on origin tab),
    // not an intentional reorder.
    if source_tab_id == dest_tab_id {
        return WebReturnType::success_empty();
    }
    // Move the block via the reducer. dst_index 0 to mirror
    // wcore::move_block_to_tab which appended at end... wait,
    // wcore appends, so end-of-list. The reducer's MoveBlock
    // clamps dst_index to dst.block_ids.len(); use u32::MAX
    // to land at the end.
    let events = dispatch_to_reducer(
        state,
        agentmux_common::ipc::Command::MoveBlock {
            block_id: block_id.clone(),
            src_tab_id: source_tab_id.clone(),
            dst_tab_id: dest_tab_id.clone(),
            dst_index: u32::MAX,
        },
    )
    .await;
    if let Some(err_msg) = events.iter().find_map(|e| match e {
        agentmux_common::ipc::Event::Error { message, .. } => Some(message.clone()),
        _ => None,
    }) {
        return WebReturnType::error(err_msg);
    }
    for ev in &events {
        if let Err(e) = crate::persist_subscriber::apply_event_to_wstore(ev, store) {
            return WebReturnType::error(format!(
                "MoveBlockToTab: SQLite write failed: {}",
                e
            ));
        }
    }
    publish_events(state, &events);
    // Auto-close empty source tab (mirrors wcore::move_block_to_tab).
    if auto_close {
        let should_close = match store.must_get::<Tab>(&source_tab_id) {
            Ok(t) => t.blockids.is_empty(),
            Err(_) => false,
        };
        if should_close {
            let total_tabs = match store.must_get::<Workspace>(&ws_id) {
                Ok(ws) => ws.tabids.len() + ws.pinnedtabids.len(),
                Err(_) => 0,
            };
            if total_tabs > 1 {
                let close_events = dispatch_to_reducer(
                    state,
                    agentmux_common::ipc::Command::DeleteTab {
                        workspace_id: ws_id.clone(),
                        tab_id: source_tab_id.clone(),
                        // Auto-close already gated on
                        // `total_tabs > 1` above; reducer's
                        // last-tab guard is defense-in-depth
                        // for the race window.
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
    let mut updates = vec![];
    if let Ok(src) = store.must_get::<Tab>(&source_tab_id) {
        updates.push(WaveObjUpdate {
            updatetype: "update".into(),
            otype: OTYPE_TAB.to_string(),
            oid: source_tab_id.clone(),
            obj: Some(wave_obj_to_value(&src)),
        });
    }
    if let Ok(dst) = store.must_get::<Tab>(&dest_tab_id) {
        updates.push(WaveObjUpdate {
            updatetype: "update".into(),
            otype: OTYPE_TAB.to_string(),
            oid: dest_tab_id.clone(),
            obj: Some(wave_obj_to_value(&dst)),
        });
    }
    if let Ok(ws) = store.must_get::<Workspace>(&ws_id) {
        updates.push(WaveObjUpdate {
            updatetype: "update".into(),
            otype: OTYPE_WORKSPACE.to_string(),
            oid: ws_id.clone(),
            obj: Some(wave_obj_to_value(&ws)),
        });
    }
    WebReturnType::success_with_updates(updates)
}

// Phase E.5.7 — PromoteBlockToTab migrated to saga
// (CreateTab + MoveBlock). Layout setup + SetActiveTab +
// auto-close source tab stay wcore-direct here (E.4 layout
// territory). Same shape as TearOffBlock's RPC handler.
pub(crate) async fn handle_promote_block_to_tab(state: &AppState, call: &WebCallType) -> WebReturnType {
    let store = &state.wstore;
    let args = &call.args;
    let ws_id: String = match service::get_arg(args, 0) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    let block_id: String = match service::get_arg(args, 1) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    let source_tab_id: String = match service::get_arg(args, 2) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    let auto_close: bool = service::get_arg(args, 3).unwrap_or(true);
    tracing::info!(ws_id = %ws_id, block_id = %block_id, source_tab = %source_tab_id, "[dnd:svc] PromoteBlockToTab via saga");
    let saga_result = crate::sagas::promote_block_to_tab::run(
        state,
        block_id.clone(),
        source_tab_id.clone(),
        ws_id.clone(),
    )
    .await;
    let new_tab_oid = match saga_result {
        Ok(v) => v
            .get("new_tab_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string(),
        Err(reason) => return WebReturnType::error(reason),
    };

    // Layout setup: rootnode + leaforder for the new tab so
    // the frontend renders the moved block correctly. Same
    // helper TearOffBlock uses.
    if let Err(e) = setup_torn_off_block_layout(state, &new_tab_oid, &block_id).await {
        tracing::warn!(new_tab = %new_tab_oid, "PromoteBlockToTab: layout setup failed: {}", e);
    }
    // Source tab: queue layout-delete action.
    if let Err(e) = queue_source_layout_delete(state, &source_tab_id, &block_id).await {
        tracing::warn!(source_tab = %source_tab_id, "PromoteBlockToTab: source layout delete-action enqueue failed: {}", e);
    }
    // Set the new tab as active in the workspace via reducer.
    let active_events = dispatch_to_reducer(
        state,
        agentmux_common::ipc::Command::SetActiveTab {
            workspace_id: ws_id.clone(),
            tab_id: new_tab_oid.clone(),
        },
    )
    .await;
    for ev in &active_events {
        if let Err(e) = crate::persist_subscriber::apply_event_to_wstore(ev, store) {
            tracing::warn!("PromoteBlockToTab: SetActiveTab apply failed: {}", e);
        }
    }
    publish_events(state, &active_events);

    // Auto-close empty source tab (mirrors wcore behaviour).
    if auto_close {
        let should_close = match store.must_get::<Tab>(&source_tab_id) {
            Ok(t) => t.blockids.is_empty(),
            Err(_) => false,
        };
        if should_close {
            let total_tabs = match store.must_get::<Workspace>(&ws_id) {
                Ok(ws) => ws.tabids.len() + ws.pinnedtabids.len(),
                Err(_) => 0,
            };
            if total_tabs > 1 {
                let close_events = dispatch_to_reducer(
                    state,
                    agentmux_common::ipc::Command::DeleteTab {
                        workspace_id: ws_id.clone(),
                        tab_id: source_tab_id.clone(),
                        // Auto-close already gated on
                        // `total_tabs > 1` above; reducer's
                        // last-tab guard is defense-in-depth
                        // for the race window.
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

    let mut updates = vec![];
    if let Ok(new_tab) = store.must_get::<Tab>(&new_tab_oid) {
        updates.push(WaveObjUpdate {
            updatetype: "update".into(),
            otype: OTYPE_TAB.to_string(),
            oid: new_tab_oid.clone(),
            obj: Some(wave_obj_to_value(&new_tab)),
        });
    }
    if let Ok(src) = store.must_get::<Tab>(&source_tab_id) {
        updates.push(WaveObjUpdate {
            updatetype: "update".into(),
            otype: OTYPE_TAB.to_string(),
            oid: source_tab_id.clone(),
            obj: Some(wave_obj_to_value(&src)),
        });
    }
    if let Ok(ws) = store.must_get::<Workspace>(&ws_id) {
        updates.push(WaveObjUpdate {
            updatetype: "update".into(),
            otype: OTYPE_WORKSPACE.to_string(),
            oid: ws_id.clone(),
            obj: Some(wave_obj_to_value(&ws)),
        });
    }
    WebReturnType::success_data_updates(
        serde_json::to_value(&new_tab_oid).unwrap_or_default(),
        updates,
    )
}

// Phase E.5.5 — MoveTabToWorkspace migrated to dispatch
// Command::MoveTab through the reducer. Closes codex P1 #621
// (the saga's reducer-state pre-check rejected tear-off after
// a wcore-direct cross-window drag had left state.tabs stale)
// by routing all tab moves through the reducer so its view
// always matches SQLite.
pub(crate) async fn handle_move_tab_to_workspace(state: &AppState, call: &WebCallType) -> WebReturnType {
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
    let dest_ws_id: String = match service::get_arg(args, 2) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    let insert_index: Option<u32> = service::get_arg::<usize>(args, 3)
        .ok()
        .map(|v| v.try_into().unwrap_or(u32::MAX));
    tracing::info!(tab_id = %tab_id, source_ws = %source_ws_id, dest_ws = %dest_ws_id, insert_index = ?insert_index, "[dnd:svc] MoveTabToWorkspace via reducer");
    // Same-workspace short-circuit matches wcore behaviour.
    // The reducer rejects same-workspace moves outright (use
    // ReorderTab instead); for the RPC contract, treat it as
    // a no-op success so existing callers don't see a
    // behavioural regression.
    if source_ws_id == dest_ws_id {
        return WebReturnType::success_empty();
    }
    // Last-tab guard mirrors wcore::move_tab_to_workspace —
    // the reducer's MoveTab doesn't enforce this (intentionally,
    // for sagas that legitimately drain a workspace to delete
    // it). Keep the guard at the RPC layer where the policy
    // belongs. Reads SQLite rather than reducer state — as of
    // SPEC_864 Phase 5 every tab-set mutation (CreateTab, MoveTab,
    // and the PromoteBlockToTab/TearOffBlock/TearOffTab sagas) is
    // reducer-routed, so the two are expected to agree; SQLite is
    // just the existing read path here, not a staleness workaround
    // (codex P1 round-2 #621, which motivated it, predates that).
    match store.get::<Workspace>(&source_ws_id) {
        Ok(Some(src_ws)) => {
            let total_tabs = src_ws.tabids.len() + src_ws.pinnedtabids.len();
            if total_tabs <= 1 {
                return WebReturnType::error(
                    "cannot move last tab out of workspace".to_string(),
                );
            }
        }
        Ok(None) => {
            return WebReturnType::error(format!(
                "MoveTabToWorkspace: source workspace not found: {}",
                source_ws_id
            ));
        }
        Err(e) => {
            return WebReturnType::error(format!(
                "MoveTabToWorkspace: workspace read failed: {}",
                e
            ));
        }
    }
    let dst_index = insert_index.unwrap_or(u32::MAX);
    let events = dispatch_to_reducer(
        state,
        agentmux_common::ipc::Command::MoveTab {
            tab_id: tab_id.clone(),
            src_workspace_id: source_ws_id.clone(),
            dst_workspace_id: dest_ws_id.clone(),
            dst_index,
        },
    )
    .await;
    if let Some(err_msg) = events.iter().find_map(|e| match e {
        agentmux_common::ipc::Event::Error { message, .. } => Some(message.clone()),
        _ => None,
    }) {
        return WebReturnType::error(err_msg);
    }
    for ev in &events {
        if let Err(e) = crate::persist_subscriber::apply_event_to_wstore(ev, store) {
            return WebReturnType::error(format!(
                "MoveTabToWorkspace: SQLite write failed: {}",
                e
            ));
        }
    }
    publish_events(state, &events);
    let mut updates = Vec::new();
    if let Ok(src_ws) = store.must_get::<Workspace>(&source_ws_id) {
        updates.push(WaveObjUpdate {
            updatetype: "update".into(),
            otype: OTYPE_WORKSPACE.to_string(),
            oid: source_ws_id.clone(),
            obj: Some(wave_obj_to_value(&src_ws)),
        });
    }
    if let Ok(dst_ws) = store.must_get::<Workspace>(&dest_ws_id) {
        updates.push(WaveObjUpdate {
            updatetype: "update".into(),
            otype: OTYPE_WORKSPACE.to_string(),
            oid: dest_ws_id.clone(),
            obj: Some(wave_obj_to_value(&dst_ws)),
        });
    }
    WebReturnType::success_with_updates(updates)
}

// Phase E.5.6 — RestoreTornOffTab migrated to saga (MoveTab
// back + conditional DeleteWorkspaceCascade if source becomes
// empty). The legacy `was_pinned` arg is ignored — pinning
// was removed from AgentMux in E.2c.3b; restored tabs always
// land in `tab_ids`.
pub(crate) async fn handle_restore_torn_off_tab(state: &AppState, call: &WebCallType) -> WebReturnType {
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
    let dest_ws_id: String = match service::get_arg(args, 2) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    let insert_index: Option<u32> = service::get_arg::<usize>(args, 3)
        .ok()
        .map(|v| v.try_into().unwrap_or(u32::MAX));
    tracing::info!(tab_id = %tab_id, source_ws = %source_ws_id, dest_ws = %dest_ws_id, insert_index = ?insert_index, "[dnd:svc] RestoreTornOffTab via saga");
    let saga_result = crate::sagas::restore_torn_off_tab::run(
        state,
        tab_id,
        source_ws_id.clone(),
        dest_ws_id.clone(),
        insert_index,
    )
    .await;
    match saga_result {
        Ok(_) => {
            let mut updates = Vec::new();
            match store.get::<Workspace>(&source_ws_id) {
                Ok(Some(src_ws)) => {
                    updates.push(WaveObjUpdate {
                        updatetype: "update".into(),
                        otype: OTYPE_WORKSPACE.to_string(),
                        oid: source_ws_id.clone(),
                        obj: Some(wave_obj_to_value(&src_ws)),
                    });
                }
                Ok(None) => {
                    updates.push(WaveObjUpdate {
                        updatetype: "delete".into(),
                        otype: OTYPE_WORKSPACE.to_string(),
                        oid: source_ws_id.clone(),
                        obj: None,
                    });
                }
                Err(_) => {}
            }
            if let Ok(dst_ws) = store.must_get::<Workspace>(&dest_ws_id) {
                updates.push(WaveObjUpdate {
                    updatetype: "update".into(),
                    otype: OTYPE_WORKSPACE.to_string(),
                    oid: dest_ws_id.clone(),
                    obj: Some(wave_obj_to_value(&dst_ws)),
                });
            }
            WebReturnType::success_with_updates(updates)
        }
        Err(reason) => WebReturnType::error(reason),
    }
}
