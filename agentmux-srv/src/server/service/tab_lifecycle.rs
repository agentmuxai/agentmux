// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `workspace` service handlers — tab CRUD (`CreateTab`, `SetActiveTab`,
//! `CloseTab`, `UpdateTabIds`, `ReorderTab`). Split out of `workspace.rs`;
//! see that file's dispatcher for the full method list.

use crate::backend::obj::*;
use crate::backend::service::{self, CloseTabRtnType, WebCallType, WebReturnType};

use super::super::AppState;
use super::reducer_helpers::{compensate_via_reducer, dispatch_to_reducer, publish_events};

// Phase E.2c.3b — CreateTab dispatches through the reducer.
// The `pinned` argument from older clients is ignored:
// pinning was a Waveterm feature removed from AgentMux.
// Legacy SQLite databases may still have entries in
// `Workspace.pinnedtabids`; bootstrap merges them into
// `tab_ids` so they behave as regular tabs.
pub(crate) async fn handle_create_tab(state: &AppState, call: &WebCallType) -> WebReturnType {
    let store = &state.wstore;
    let args = &call.args;
    let ws_id: String = match service::get_arg(args, 0) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    let tab_name: String = service::get_arg(args, 1).unwrap_or_default();
    let activate: bool = service::get_arg(args, 2).unwrap_or(true);
    // args[3] (`pinned`) intentionally ignored.
    // Auto-generate a `Tab {N}` name when the caller passed
    // empty so behaviour matches the prior wcore path. Counts
    // both `tabids` and any leftover `pinnedtabids` from
    // legacy data so the numbering doesn't collide with
    // pre-removal entries that bootstrap will surface as
    // regular tabs.
    let resolved_name = if tab_name.is_empty() {
        match store.get::<Workspace>(&ws_id) {
            Ok(Some(ws)) => {
                format!("Tab {}", ws.tabids.len() + ws.pinnedtabids.len() + 1)
            }
            _ => "Tab 1".to_string(),
        }
    } else {
        tab_name.clone()
    };
    let events = dispatch_to_reducer(
        state,
        agentmux_common::ipc::Command::CreateTab {
            workspace_id: ws_id.clone(),
            name: resolved_name,
        },
    )
    .await;
    // Surface reducer Error events (e.g., workspace not
    // found) before any persistence work — they're not
    // bug events, they're caller-visible failures, and the
    // generic "did not emit TabCreated" message below would
    // mask the real reason. Matches the SetActiveTab pattern.
    // (reagent P1 #616.)
    if let Some(err_msg) = events.iter().find_map(|e| match e {
        agentmux_common::ipc::Event::Error { message, .. } => Some(message.clone()),
        _ => None,
    }) {
        return WebReturnType::error(err_msg);
    }
    let tab_id = events.iter().find_map(|e| match e {
        agentmux_common::ipc::Event::TabCreated { tab_id, .. } => Some(tab_id.clone()),
        _ => None,
    });
    // Apply synchronously to wstore (forward+compensate on
    // failure — same pattern as CreateWorkspace in E.2c.2).
    let mut apply_err: Option<String> = None;
    for ev in &events {
        if let Err(e) = crate::persist_subscriber::apply_event_to_wstore(ev, store) {
            apply_err = Some(e.to_string());
            break;
        }
    }
    if let Some(err) = apply_err {
        if let Some(tid) = tab_id.as_ref() {
            compensate_via_reducer(
                state,
                agentmux_common::ipc::Command::DeleteTab {
                    workspace_id: ws_id.clone(),
                    tab_id: tid.clone(),
                    // Compensation must bypass the last-tab
                    // guard to roll back a just-created sole
                    // tab when its persist failed (codex P1
                    // round 2 + P2 round 4 PR #633).
                    force: true,
                },
                store,
            )
            .await;
        }
        return WebReturnType::error(format!("CreateTab: SQLite write failed: {}", err));
    }
    publish_events(state, &events);
    // If `activate=true` and the reducer didn't auto-activate
    // this as the first tab, dispatch SetActiveTab.
    let auto_activated = events
        .iter()
        .any(|e| matches!(e, agentmux_common::ipc::Event::ActiveTabChanged { .. }));
    if activate && !auto_activated {
        if let Some(tid) = tab_id.as_ref() {
            let active_events = dispatch_to_reducer(
                state,
                agentmux_common::ipc::Command::SetActiveTab {
                    workspace_id: ws_id.clone(),
                    tab_id: tid.clone(),
                },
            )
            .await;
            let mut active_err: Option<String> = None;
            for ev in &active_events {
                if let Err(e) =
                    crate::persist_subscriber::apply_event_to_wstore(ev, store)
                {
                    active_err = Some(e.to_string());
                    break;
                }
            }
            if active_err.is_none() {
                publish_events(state, &active_events);
            }
            // SetActiveTab failure is non-fatal here — the
            // tab exists; activation can be retried by the
            // caller. Log if it happened.
            if let Some(err) = active_err {
                tracing::warn!(
                    "CreateTab: post-create activate failed: {}",
                    err
                );
            }
        }
    }
    match tab_id {
        Some(id) => {
            let mut updates = vec![];
            if let Ok(tab) = store.must_get::<Tab>(&id) {
                updates.push(WaveObjUpdate {
                    updatetype: "update".into(),
                    otype: OTYPE_TAB.to_string(),
                    oid: id.clone(),
                    obj: Some(wave_obj_to_value(&tab)),
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
                serde_json::to_value(&id).unwrap_or_default(),
                updates,
            )
        }
        None => WebReturnType::error(
            "CreateTab: reducer did not emit TabCreated".to_string(),
        ),
    }
}

// Phase E.2c.3 — SetActiveTab routes through the reducer.
// Read-through reads (e.g., GetWorkspace) still hit wstore
// during the migration window, so the synchronous
// apply-to-wstore keeps them consistent.
pub(crate) async fn handle_set_active_tab(state: &AppState, call: &WebCallType) -> WebReturnType {
    let store = &state.wstore;
    let args = &call.args;
    let ws_id: String = match service::get_arg(args, 0) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    let tab_id: String = match service::get_arg(args, 1) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    // Phase E.2c.3b — pinning was removed from AgentMux
    // (Waveterm legacy). All tabs are regular; dispatch
    // straight through the reducer. Bootstrap merges any
    // legacy `pinnedtabids` into the reducer's `tab_ids` so
    // tabs from older databases are reachable as normal tabs.
    let events = dispatch_to_reducer(
        state,
        agentmux_common::ipc::Command::SetActiveTab {
            workspace_id: ws_id.clone(),
            tab_id: tab_id.clone(),
        },
    )
    .await;
    // Reducer emits Event::Error on unknown workspace/tab —
    // surface as RPC error.
    if let Some(err_msg) = events.iter().find_map(|e| match e {
        agentmux_common::ipc::Event::Error { message, .. } => Some(message.clone()),
        _ => None,
    }) {
        return WebReturnType::error(err_msg);
    }
    // Apply synchronously. SetActiveTab is reversible at the
    // reducer level (just write back the previous active id),
    // but we don't track the previous id here; if SQLite
    // fails, return the error and accept short-lived
    // divergence on this RPC path. (Acceptable: SetActiveTab
    // is a UI-driven action; the user can retry.)
    let mut apply_err: Option<String> = None;
    for ev in &events {
        if let Err(e) = crate::persist_subscriber::apply_event_to_wstore(ev, store) {
            apply_err = Some(e.to_string());
            break;
        }
    }
    if let Some(err) = apply_err {
        return WebReturnType::error(format!("SetActiveTab: SQLite write failed: {}", err));
    }
    publish_events(state, &events);
    if let Ok(ws) = store.must_get::<Workspace>(&ws_id) {
        let update = WaveObjUpdate {
            updatetype: "update".into(),
            otype: OTYPE_WORKSPACE.to_string(),
            oid: ws_id.clone(),
            obj: Some(wave_obj_to_value(&ws)),
        };
        WebReturnType::success_with_updates(vec![update])
    } else {
        WebReturnType::success_empty()
    }
}

// Phase E.5.7 (Step 5 PR 1) — CloseTab via DeleteTab saga.
// Replaces the legacy SQLite-first pattern (wcore::delete_tab
// followed by reducer-sync dispatch) with a saga-driven
// reducer + persist-subscriber flow. The saga also enforces
// the not-the-last-tab pre-condition mirrored from
// TearOffTab; user-facing CloseTab now refuses to drain a
// workspace to zero tabs (callers wanting full teardown
// should issue DeleteWorkspace instead — that path migrates
// in Step 5 PR 2).
pub(crate) async fn handle_close_tab(state: &AppState, call: &WebCallType) -> WebReturnType {
    let store = &state.wstore;
    let args = &call.args;
    let ws_id: String = match service::get_arg(args, 0) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    let tab_id: String = match service::get_arg(args, 1) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    if let Err(reason) =
        crate::sagas::delete_tab::run(state, ws_id.clone(), tab_id.clone()).await
    {
        return WebReturnType::error(reason);
    }
    let rtn = CloseTabRtnType {
        closewindow: false,
        newactivetabid: String::new(),
    };
    let mut updates = vec![WaveObjUpdate {
        updatetype: "delete".into(),
        otype: OTYPE_TAB.to_string(),
        oid: tab_id.clone(),
        obj: None,
    }];
    if let Ok(ws) = store.must_get::<Workspace>(&ws_id) {
        updates.push(WaveObjUpdate {
            updatetype: "update".into(),
            otype: OTYPE_WORKSPACE.to_string(),
            oid: ws_id.clone(),
            obj: Some(wave_obj_to_value(&ws)),
        });
    }
    WebReturnType::success_data_updates(
        serde_json::to_value(&rtn).unwrap_or_default(),
        updates,
    )
}

// Phase E.5.3 — UpdateTabIds migrated to ReorderTabsBulk
// through the reducer. The legacy `pinned_tab_ids` arg is
// ignored: pinning was a Waveterm feature removed from
// AgentMux. Bootstrap merged any legacy `pinnedtabids` into
// the reducer's `tab_ids`. The subscriber's
// `apply_tabs_reordered_bulk` rewrites `Workspace.tabids`
// and drains any leftover `Workspace.pinnedtabids` so the
// UI's `[...pinnedtabids, ...tabids]` combine never
// double-counts a tab once a workspace's tabs are
// reordered through the reducer.
pub(crate) async fn handle_update_tab_ids(state: &AppState, call: &WebCallType) -> WebReturnType {
    let store = &state.wstore;
    let args = &call.args;
    let ws_id: String = match service::get_arg(args, 0) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    let tab_ids: Vec<String> = match service::get_arg(args, 1) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    // args[2] (pinned_tab_ids) intentionally ignored.
    let events = dispatch_to_reducer(
        state,
        agentmux_common::ipc::Command::ReorderTabsBulk {
            workspace_id: ws_id.clone(),
            tab_ids,
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
                "UpdateTabIds: SQLite write failed: {}",
                e
            ));
        }
    }
    publish_events(state, &events);
    if let Ok(updated_ws) = store.must_get::<Workspace>(&ws_id) {
        let update = WaveObjUpdate {
            updatetype: "update".into(),
            otype: OTYPE_WORKSPACE.to_string(),
            oid: ws_id.clone(),
            obj: Some(wave_obj_to_value(&updated_ws)),
        };
        return WebReturnType::success_with_updates(vec![update]);
    }
    WebReturnType::success_empty()
}

// Phase E.2c.3b — ReorderTab dispatches through the reducer.
// Forward+compensate isn't useful here (reorder is its own
// inverse), so on SQLite apply failure we just surface the
// error and the reducer state ends up ahead of disk for the
// remainder of the session — converges back at next restart
// via bootstrap.
pub(crate) async fn handle_reorder_tab(state: &AppState, call: &WebCallType) -> WebReturnType {
    let store = &state.wstore;
    let args = &call.args;
    let ws_id: String = match service::get_arg(args, 0) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    let tab_id: String = match service::get_arg(args, 1) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    let new_index: usize = match service::get_arg(args, 2) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    tracing::info!(ws_id = %ws_id, tab_id = %tab_id, new_index = %new_index, "[dnd:svc] ReorderTab");
    // Clamp to u32::MAX rather than truncating via `as u32`.
    // The reducer further clamps to `tab_ids.len() - 1` so an
    // absurd usize ends up at the last position — matching
    // the prior `wcore::reorder_tab` behaviour where any
    // out-of-range usize clamped to the end. (codex P3 #617.)
    let new_index_u32 = u32::try_from(new_index).unwrap_or(u32::MAX);
    let events = dispatch_to_reducer(
        state,
        agentmux_common::ipc::Command::ReorderTab {
            workspace_id: ws_id.clone(),
            tab_id: tab_id.clone(),
            new_index: new_index_u32,
        },
    )
    .await;
    // Surface reducer Error events.
    if let Some(err_msg) = events.iter().find_map(|e| match e {
        agentmux_common::ipc::Event::Error { message, .. } => Some(message.clone()),
        _ => None,
    }) {
        return WebReturnType::error(err_msg);
    }
    let mut apply_err: Option<String> = None;
    for ev in &events {
        if let Err(e) = crate::persist_subscriber::apply_event_to_wstore(ev, store) {
            apply_err = Some(e.to_string());
            break;
        }
    }
    if let Some(err) = apply_err {
        return WebReturnType::error(format!("ReorderTab: SQLite write failed: {}", err));
    }
    publish_events(state, &events);
    if let Ok(ws) = store.must_get::<Workspace>(&ws_id) {
        let update = WaveObjUpdate {
            updatetype: "update".into(),
            otype: OTYPE_WORKSPACE.to_string(),
            oid: ws_id.clone(),
            obj: Some(wave_obj_to_value(&ws)),
        };
        WebReturnType::success_with_updates(vec![update])
    } else {
        WebReturnType::success_empty()
    }
}
