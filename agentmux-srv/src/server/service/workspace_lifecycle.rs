// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `workspace` service handlers — workspace CRUD (`CreateWorkspace`,
//! `GetWorkspace`, `DeleteWorkspace`, `UpdateWorkspace`). Split out of
//! `workspace.rs`; see that file's dispatcher for the full method list.

use crate::backend::service::{self, WebCallType, WebReturnType};
use crate::backend::wcore;

use super::super::AppState;
use super::reducer_helpers::{
    compensate_via_reducer, dispatch_to_reducer, publish_events, wstore_workspace_exists,
};

pub(crate) async fn handle_create_workspace(state: &AppState, call: &WebCallType) -> WebReturnType {
    let store = &state.wstore;
    let args = &call.args;
    let name: String = service::get_arg(args, 0).unwrap_or_default();
    let events = dispatch_to_reducer(
        state,
        agentmux_common::ipc::Command::CreateWorkspace { name: name.clone() },
    )
    .await;
    let workspace_id = events.iter().find_map(|e| match e {
        agentmux_common::ipc::Event::WorkspaceCreated { workspace_id, .. } => {
            Some(workspace_id.clone())
        }
        _ => None,
    });
    // Apply synchronously to wstore BEFORE publishing or
    // returning. On SQLite failure, dispatch a compensating
    // `DeleteWorkspace` so the reducer's session-only state
    // doesn't carry a ghost workspace that was never
    // persisted (codex P2 #615).
    let mut apply_err: Option<String> = None;
    for ev in &events {
        if let Err(e) = crate::persist_subscriber::apply_event_to_wstore(ev, store) {
            apply_err = Some(e.to_string());
            break;
        }
    }
    if let Some(err) = apply_err {
        if let Some(id) = workspace_id.as_ref() {
            compensate_via_reducer(
                state,
                agentmux_common::ipc::Command::DeleteWorkspace {
                    workspace_id: id.clone(),
                    // Internal compensation path for failed
                    // CreateWorkspace SQLite apply — not the
                    // saga (Step 5 PR 2).
                    force: false,
                },
                store,
            )
            .await;
        }
        return WebReturnType::error(format!(
            "CreateWorkspace: SQLite write failed: {}",
            err
        ));
    }
    publish_events(state, &events);
    match workspace_id {
        Some(id) => match wcore::get_workspace(store, &id) {
            Ok(ws) => {
                WebReturnType::success(serde_json::to_value(&ws).unwrap_or_default())
            }
            Err(e) => WebReturnType::error(format!(
                "CreateWorkspace: post-write read failed: {}",
                e
            )),
        },
        None => WebReturnType::error(
            "CreateWorkspace: reducer did not emit WorkspaceCreated".to_string(),
        ),
    }
}

pub(crate) async fn handle_get_workspace(state: &AppState, call: &WebCallType) -> WebReturnType {
    let store = &state.wstore;
    let args = &call.args;
    let ws_id: String = match service::get_arg(args, 0) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    // wstore-direct during the migration window (see
    // ("workspace", ...) header comment above for the
    // rationale). Reducer-state reads return on E.2c.3+ once
    // tabs (and pinned tabs) live in the reducer.
    match wcore::get_workspace(store, &ws_id) {
        Ok(ws) => WebReturnType::success(serde_json::to_value(&ws).unwrap_or_default()),
        Err(e) => WebReturnType::error(e.to_string()),
    }
}

pub(crate) async fn handle_delete_workspace(state: &AppState, call: &WebCallType) -> WebReturnType {
    let store = &state.wstore;
    let args = &call.args;
    let ws_id: String = match service::get_arg(args, 0) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    // Step 5 PR 2 — route the user-initiated DeleteWorkspace
    // through the `delete_workspace` saga. The saga:
    //   1. Snapshots the workspace's tabs+blocks for
    //      provenance in the durable saga log.
    //   2. Dispatches per-tab `DeleteTab { force: true }`
    //      through the reducer (cascades blocks; persist
    //      subscriber writes SQLite + kills controllers via
    //      `wcore::delete_tab_inner`).
    //   3. Dispatches the final
    //      `DeleteWorkspace { force: true }` which removes
    //      the (now-empty) workspace + window mappings.
    //
    // The legacy SQLite-first path here (wcore::delete_workspace
    // followed by Command::DeleteWorkspace dispatch) is replaced
    // by the saga because the durable lifecycle bracket gives
    // crash-recovery a chance to retry/compensate via
    // `recovery::compensate_unresolved` if the cascade is
    // interrupted. Cascade behaviour is preserved 1:1.
    //
    // Pre-condition: workspace must exist (in reducer or
    // SQLite). The saga runs its own existence check; we mirror
    // the legacy NotFound semantics here for backward-compat
    // error messages.
    let exists_in_wstore = match wstore_workspace_exists(store, &ws_id) {
        Ok(v) => v,
        Err(e) => {
            return WebReturnType::error(format!(
                "DeleteWorkspace: SQLite read failed: {}",
                e
            ))
        }
    };
    if !exists_in_wstore {
        let exists_in_state = state
            .srv_state
            .lock()
            .await
            .workspaces
            .contains_key(&ws_id);
        if !exists_in_state {
            return WebReturnType::error(format!(
                "DeleteWorkspace: workspace not found: {}",
                ws_id
            ));
        }
    }
    match crate::sagas::delete_workspace::run(state, ws_id.clone()).await {
        Ok(_) => WebReturnType::success_empty(),
        Err(e) => WebReturnType::error(format!("DeleteWorkspace failed: {}", e)),
    }
}

// Phase E.5.3 — UpdateWorkspace migrated through the reducer.
// Currently only handles rename (the only field this RPC ever
// mutated). Meta-only updates are dispatched as
// UpdateWorkspaceMeta separately by frontends.
pub(crate) async fn handle_update_workspace(state: &AppState, call: &WebCallType) -> WebReturnType {
    let store = &state.wstore;
    let args = &call.args;
    let ws_id: String = match service::get_arg(args, 0) {
        Ok(v) => v,
        Err(e) => return WebReturnType::error(e),
    };
    let name: Option<String> = service::get_optional_arg(args, 1).unwrap_or(None);
    let Some(name) = name else {
        return WebReturnType::success_empty();
    };
    let events = dispatch_to_reducer(
        state,
        agentmux_common::ipc::Command::RenameWorkspace {
            workspace_id: ws_id.clone(),
            name,
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
                "UpdateWorkspace: SQLite write failed: {}",
                e
            ));
        }
    }
    publish_events(state, &events);
    WebReturnType::success_empty()
}
