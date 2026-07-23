// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! `workspace` service handler (workspace + tab lifecycle, drag-and-drop moves).
//!
//! The dispatcher below just routes each method to its handler; the
//! handler bodies live in sibling modules:
//! * [`super::workspace_lifecycle`] — `CreateWorkspace` / `GetWorkspace` /
//!   `DeleteWorkspace` / `UpdateWorkspace`.
//! * [`super::tab_lifecycle`] — `CreateTab` / `SetActiveTab` / `CloseTab` /
//!   `UpdateTabIds` / `ReorderTab`.
//! * [`super::tab_move`] — `MoveBlockToTab` / `PromoteBlockToTab` /
//!   `MoveTabToWorkspace` / `RestoreTornOffTab`.
//! * [`super::tear_off`] — `TearOffBlock` / `RedockFloatingPane` /
//!   `TearOffTab`.

use crate::backend::service::{WebCallType, WebReturnType};
use crate::backend::wcore;

use super::super::AppState;
use super::tab_lifecycle::{
    handle_close_tab, handle_create_tab, handle_reorder_tab, handle_set_active_tab,
    handle_update_tab_ids,
};
use super::tab_move::{
    handle_move_block_to_tab, handle_move_tab_to_workspace, handle_promote_block_to_tab,
    handle_restore_torn_off_tab,
};
use super::tear_off::{handle_redock_floating_pane, handle_tear_off_block, handle_tear_off_tab};
use super::workspace_lifecycle::{
    handle_create_workspace, handle_delete_workspace, handle_get_workspace,
    handle_update_workspace,
};

pub(super) async fn handle_workspace_service(state: &AppState, call: &WebCallType) -> WebReturnType {
    let store = &state.wstore;
    // Phase E.2c.2 — workspace lifecycle dispatches through the
    // srv reducer for event emission (sagas / renderer / persist
    // subscriber consume them) AND synchronously applies the
    // emitted events to SQLite via the subscriber's apply path.
    // Synchronous SQLite writes are required during the migration
    // window because tab/block RPC still hits wcore directly and
    // expects workspaces to be present in SQLite by the time the
    // RPC reply returns (e.g., a CreateTab call right after
    // CreateWorkspace would 404 on the workspace lookup if we
    // only relied on the async subscriber). The subscriber later
    // receives the same event on the broadcast bus and re-applies
    // idempotently — safe because each apply arm checks SQLite
    // state before writing. (Both reagent + codex flagged this
    // race as P1 #615.)
    //
    // Reads (`GetWorkspace` / `ListWorkspaces`) stay on wstore
    // until the tab + block RPC layers also migrate (E.2c.3 +
    // E.2c.4). The reducer's `WorkspaceRecord` doesn't track
    // `pinnedtabids` and its `tabids` / `activetabid` go stale
    // immediately after any wcore-direct tab op — reading from
    // it before tabs are migrated returns wrong data.
    match call.method.as_str() {
        "CreateWorkspace" => handle_create_workspace(state, call).await,
        "GetWorkspace" => handle_get_workspace(state, call).await,
        "DeleteWorkspace" => handle_delete_workspace(state, call).await,
        "ListWorkspaces" => match wcore::list_workspaces(store) {
            Ok(list) => WebReturnType::success(serde_json::to_value(&list).unwrap_or_default()),
            Err(e) => WebReturnType::error(e.to_string()),
        },
        "CreateTab" => handle_create_tab(state, call).await,
        "SetActiveTab" => handle_set_active_tab(state, call).await,
        "CloseTab" => handle_close_tab(state, call).await,
        "UpdateWorkspace" => handle_update_workspace(state, call).await,
        "UpdateTabIds" => handle_update_tab_ids(state, call).await,
        "MoveBlockToTab" => handle_move_block_to_tab(state, call).await,
        "PromoteBlockToTab" => handle_promote_block_to_tab(state, call).await,
        "ReorderTab" => handle_reorder_tab(state, call).await,
        "MoveTabToWorkspace" => handle_move_tab_to_workspace(state, call).await,
        "RestoreTornOffTab" => handle_restore_torn_off_tab(state, call).await,
        "TearOffBlock" => handle_tear_off_block(state, call).await,
        "RedockFloatingPane" => handle_redock_floating_pane(state, call).await,
        "TearOffTab" => handle_tear_off_tab(state, call).await,
        _ => WebReturnType::error(format!("unknown workspace method: {}", call.method)),
    }
}
