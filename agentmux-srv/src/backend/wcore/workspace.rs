// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Workspace CRUD operations.

use std::collections::HashMap;
use uuid::Uuid;

use crate::backend::storage::wstore::WaveStore;
use crate::backend::storage::StoreError;
use crate::backend::obj::*;

/// Create a new workspace with name, icon, and color.
pub fn create_workspace(
    store: &WaveStore,
    name: &str,
    icon: &str,
    color: &str,
) -> Result<Workspace, StoreError> {
    let mut ws = Workspace {
        oid: Uuid::new_v4().to_string(),
        name: name.to_string(),
        icon: icon.to_string(),
        color: color.to_string(),
        tabids: vec![],
        pinnedtabids: vec![],
        activetabid: String::new(),
        meta: MetaMapType::new(),
        ..Default::default()
    };
    store.insert(&mut ws)?;
    Ok(ws)
}

/// Delete a workspace and all its tabs/blocks.
pub fn delete_workspace(store: &WaveStore, ws_id: &str) -> Result<(), StoreError> {
    let ws = store.must_get::<Workspace>(ws_id)?;

    // Delete all tabs in the workspace
    for tab_id in &ws.tabids {
        super::tab::delete_tab_inner(store, tab_id)?;
    }

    store.delete::<Workspace>(ws_id)?;
    Ok(())
}

/// Get a workspace by ID.
pub fn get_workspace(store: &WaveStore, ws_id: &str) -> Result<Workspace, StoreError> {
    store.must_get::<Workspace>(ws_id)
}

/// List all workspaces as WorkspaceListEntry (matching Go's behavior).
/// Go returns [{workspaceid, windowid}] — filters out workspaces without name/icon/color.
pub fn list_workspaces(store: &WaveStore) -> Result<Vec<WorkspaceListEntry>, StoreError> {
    let workspaces = store.get_all::<Workspace>()?;
    let windows = store.get_all::<Window>()?;

    // Build workspace -> window mapping
    let mut ws_to_window: HashMap<String, String> = HashMap::new();
    for win in &windows {
        ws_to_window.entry(win.workspaceid.clone()).or_insert_with(|| win.oid.clone());
    }

    let mut entries = Vec::new();
    for ws in &workspaces {
        // Go skips workspaces missing name, icon, or color
        if ws.name.is_empty() || ws.icon.is_empty() || ws.color.is_empty() {
            continue;
        }
        entries.push(WorkspaceListEntry {
            workspaceid: ws.oid.clone(),
            windowid: ws_to_window.get(&ws.oid).cloned().unwrap_or_default(),
        });
    }
    Ok(entries)
}
