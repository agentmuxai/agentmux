// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Tab CRUD and reorder operations.

use uuid::Uuid;

use crate::backend::storage::wstore::WaveStore;
use crate::backend::storage::StoreError;
use crate::backend::obj::*;

/// Create a new tab in a workspace.
/// If `tab_name` is empty, auto-generates "Untitled1", "Untitled2", etc.
/// If `pinned` is true, the tab goes into `pinnedtabids` instead of `tabids`.
pub fn create_tab(store: &WaveStore, ws_id: &str) -> Result<Tab, StoreError> {
    create_tab_with_opts(store, ws_id, "", false)
}

/// Create a new tab with explicit name and pinned options.
pub fn create_tab_with_opts(
    store: &WaveStore,
    ws_id: &str,
    tab_name: &str,
    pinned: bool,
) -> Result<Tab, StoreError> {
    let mut ws = store.must_get::<Workspace>(ws_id)?;

    // Auto-generate tab name if not provided
    let name = if tab_name.is_empty() {
        format!("Untitled{}", ws.tabids.len() + ws.pinnedtabids.len() + 1)
    } else {
        tab_name.to_string()
    };

    // Create layout state for the tab
    let mut layout = LayoutState {
        oid: Uuid::new_v4().to_string(),
        rootnode: None,
        magnifiednodeid: String::new(),
        focusednodeid: String::new(),
        leaforder: None,
        pendingbackendactions: None,
        meta: None,
        ..Default::default()
    };
    store.insert(&mut layout)?;

    let mut tab = Tab {
        oid: Uuid::new_v4().to_string(),
        name,
        layoutstate: layout.oid.clone(),
        blockids: vec![],
        meta: MetaMapType::new(),
        ..Default::default()
    };
    store.insert(&mut tab)?;

    // Add tab to workspace (pinned or unpinned) and set as active
    if pinned {
        ws.pinnedtabids.push(tab.oid.clone());
    } else {
        ws.tabids.push(tab.oid.clone());
    }
    if ws.activetabid.is_empty() {
        ws.activetabid = tab.oid.clone();
    }
    store.update(&mut ws)?;

    Ok(tab)
}

/// Delete a tab and its blocks/layout.
pub fn delete_tab(
    store: &WaveStore,
    ws_id: &str,
    tab_id: &str,
) -> Result<(), StoreError> {
    let mut ws = store.must_get::<Workspace>(ws_id)?;

    // Remove tab from workspace
    ws.tabids.retain(|id| id != tab_id);
    ws.pinnedtabids.retain(|id| id != tab_id);

    // If active tab was deleted, pick a new one
    if ws.activetabid == tab_id {
        ws.activetabid = ws.tabids.first().cloned().unwrap_or_default();
    }
    store.update(&mut ws)?;

    delete_tab_inner(store, tab_id)?;
    Ok(())
}

/// Internal: delete a tab's layout and blocks, then the tab itself.
pub(super) fn delete_tab_inner(store: &WaveStore, tab_id: &str) -> Result<(), StoreError> {
    if let Ok(tab) = store.must_get::<Tab>(tab_id) {
        // Delete layout state
        if !tab.layoutstate.is_empty() {
            let _ = store.delete::<LayoutState>(&tab.layoutstate);
        }
        // Delete all blocks in the tab
        for block_id in &tab.blockids {
            let _ = store.delete::<Block>(block_id);
        }
    }
    let _ = store.delete::<Tab>(tab_id);
    Ok(())
}

/// Set the active tab in a workspace.
pub fn set_active_tab(
    store: &WaveStore,
    ws_id: &str,
    tab_id: &str,
) -> Result<(), StoreError> {
    let mut ws = store.must_get::<Workspace>(ws_id)?;
    let tab_str = tab_id.to_string();
    if !ws.tabids.contains(&tab_str) && !ws.pinnedtabids.contains(&tab_str) {
        return Err(StoreError::NotFound);
    }
    ws.activetabid = tab_str;
    store.update(&mut ws)?;
    Ok(())
}

/// Reorder a tab within a workspace by moving it to a new index.
pub fn reorder_tab(
    store: &WaveStore,
    ws_id: &str,
    tab_id: &str,
    new_index: usize,
) -> Result<(), StoreError> {
    tracing::info!(ws_id = %ws_id, tab_id = %tab_id, new_index = %new_index, "[dnd] reorder_tab");
    let mut ws = store.must_get::<Workspace>(ws_id)?;

    // Determine if tab is in regular or pinned list
    if let Some(pos) = ws.tabids.iter().position(|id| id == tab_id) {
        ws.tabids.remove(pos);
        let insert_at = new_index.min(ws.tabids.len());
        ws.tabids.insert(insert_at, tab_id.to_string());
    } else if let Some(pos) = ws.pinnedtabids.iter().position(|id| id == tab_id) {
        ws.pinnedtabids.remove(pos);
        let insert_at = new_index.min(ws.pinnedtabids.len());
        ws.pinnedtabids.insert(insert_at, tab_id.to_string());
    } else {
        return Err(StoreError::NotFound);
    }

    store.update(&mut ws)?;
    tracing::info!(tab_id = %tab_id, new_index = %new_index, "[dnd] reorder_tab complete");
    Ok(())
}
