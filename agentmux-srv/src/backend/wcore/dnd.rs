// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Drag-and-drop operations: moving/promoting/tearing-off blocks and tabs.

use uuid::Uuid;

use crate::backend::storage::wstore::WaveStore;
use crate::backend::storage::StoreError;
use crate::backend::obj::*;

use super::tab::{create_tab, delete_tab, set_active_tab};
use super::workspace::create_workspace;

/// Move a block from one tab to another.
/// Removes the block from `source_tab_id.blockids` and adds it to `dest_tab_id.blockids`.
/// Updates `block.parentoref` to point to the destination tab.
/// If `auto_close_source` is true, deletes the source tab when it becomes empty
/// (only if the workspace has other tabs).
pub fn move_block_to_tab(
    store: &WaveStore,
    block_id: &str,
    source_tab_id: &str,
    dest_tab_id: &str,
    ws_id: &str,
    auto_close_source: bool,
) -> Result<(), StoreError> {
    tracing::info!(
        block_id = %block_id,
        source_tab = %source_tab_id,
        dest_tab = %dest_tab_id,
        ws_id = %ws_id,
        auto_close = %auto_close_source,
        "[dnd] move_block_to_tab"
    );
    if source_tab_id == dest_tab_id {
        tracing::debug!("[dnd] move_block_to_tab: same tab, no-op");
        return Ok(()); // no-op
    }

    // Verify block exists
    let mut block = store.must_get::<Block>(block_id)?;

    // Remove block from source tab
    let mut source_tab = store.must_get::<Tab>(source_tab_id)?;
    source_tab.blockids.retain(|id| id != block_id);
    store.update(&mut source_tab)?;

    // Add block to destination tab
    let mut dest_tab = store.must_get::<Tab>(dest_tab_id)?;
    dest_tab.blockids.push(block_id.to_string());
    store.update(&mut dest_tab)?;

    // Update block's parent reference
    block.parentoref = format!("tab:{}", dest_tab_id);
    store.update(&mut block)?;

    // Auto-close empty source tab if requested
    if auto_close_source && source_tab.blockids.is_empty() {
        let ws = store.must_get::<Workspace>(ws_id)?;
        let total_tabs = ws.tabids.len() + ws.pinnedtabids.len();
        if total_tabs > 1 {
            tracing::info!(source_tab = %source_tab_id, "[dnd] auto-closing empty source tab");
            delete_tab(store, ws_id, source_tab_id)?;
        } else {
            tracing::debug!(source_tab = %source_tab_id, "[dnd] source tab empty but is last tab — keeping");
        }
    }

    tracing::info!(block_id = %block_id, dest_tab = %dest_tab_id, "[dnd] move_block_to_tab complete");
    Ok(())
}

/// Promote a block to a new tab.
/// Removes the block from `source_tab_id`, creates a new tab in `ws_id`,
/// and adds the block to the new tab. Returns the new Tab.
/// If `auto_close_source` is true, deletes the source tab when it becomes empty.
pub fn promote_block_to_tab(
    store: &WaveStore,
    block_id: &str,
    source_tab_id: &str,
    ws_id: &str,
    auto_close_source: bool,
) -> Result<Tab, StoreError> {
    tracing::info!(
        block_id = %block_id,
        source_tab = %source_tab_id,
        ws_id = %ws_id,
        auto_close = %auto_close_source,
        "[dnd] promote_block_to_tab"
    );
    // Verify block exists
    let mut block = store.must_get::<Block>(block_id)?;

    // Remove block from source tab
    let mut source_tab = store.must_get::<Tab>(source_tab_id)?;
    source_tab.blockids.retain(|id| id != block_id);
    store.update(&mut source_tab)?;

    // Create new tab
    let new_tab = create_tab(store, ws_id)?;

    // Add block to new tab
    let mut new_tab = store.must_get::<Tab>(&new_tab.oid)?;
    new_tab.blockids.push(block_id.to_string());
    store.update(&mut new_tab)?;

    // Update block's parent reference
    block.parentoref = format!("tab:{}", new_tab.oid);
    store.update(&mut block)?;

    // Set the new tab as active
    set_active_tab(store, ws_id, &new_tab.oid)?;

    // Auto-close empty source tab if requested
    if auto_close_source && source_tab.blockids.is_empty() {
        let ws = store.must_get::<Workspace>(ws_id)?;
        let total_tabs = ws.tabids.len() + ws.pinnedtabids.len();
        if total_tabs > 1 {
            tracing::info!(source_tab = %source_tab_id, "[dnd] auto-closing empty source tab after promote");
            delete_tab(store, ws_id, source_tab_id)?;
        }
    }

    tracing::info!(block_id = %block_id, new_tab = %new_tab.oid, "[dnd] promote_block_to_tab complete");
    Ok(new_tab)
}

/// Move a tab from one workspace to another.
/// Removes the tab from the source workspace's tabids/pinnedtabids and adds it
/// to the destination workspace's tabids at the specified index.
/// The tab is always added as unpinned in the destination.
/// If the tab was the active tab in the source workspace, a new active tab is chosen.
pub fn move_tab_to_workspace(
    store: &WaveStore,
    tab_id: &str,
    source_ws_id: &str,
    dest_ws_id: &str,
    insert_index: Option<usize>,
) -> Result<(), StoreError> {
    tracing::info!(
        tab_id = %tab_id,
        source_ws = %source_ws_id,
        dest_ws = %dest_ws_id,
        insert_index = ?insert_index,
        "[dnd] move_tab_to_workspace"
    );
    if source_ws_id == dest_ws_id {
        tracing::debug!("[dnd] move_tab_to_workspace: same workspace, no-op");
        return Ok(()); // no-op
    }

    // Verify tab exists
    let _tab = store.must_get::<Tab>(tab_id)?;

    // Remove tab from source workspace
    let mut source_ws = store.must_get::<Workspace>(source_ws_id)?;
    let total_tabs = source_ws.tabids.len() + source_ws.pinnedtabids.len();
    if total_tabs <= 1 {
        tracing::warn!(tab_id = %tab_id, total_tabs = %total_tabs, "[dnd] move_tab_to_workspace blocked: last tab");
        return Err(StoreError::Other(
            "cannot move last tab out of workspace".to_string(),
        ));
    }
    source_ws.tabids.retain(|id| id != tab_id);
    source_ws.pinnedtabids.retain(|id| id != tab_id);
    if source_ws.activetabid == tab_id {
        let new_active = source_ws
            .tabids
            .first()
            .or(source_ws.pinnedtabids.first())
            .cloned()
            .unwrap_or_default();
        tracing::info!(old_active = %tab_id, new_active = %new_active, "[dnd] switching active tab in source workspace");
        source_ws.activetabid = new_active;
    }
    store.update(&mut source_ws)?;

    // Add tab to destination workspace
    let mut dest_ws = store.must_get::<Workspace>(dest_ws_id)?;
    let idx = insert_index.unwrap_or(dest_ws.tabids.len());
    let insert_at = idx.min(dest_ws.tabids.len());
    dest_ws.tabids.insert(insert_at, tab_id.to_string());
    dest_ws.activetabid = tab_id.to_string();
    store.update(&mut dest_ws)?;

    tracing::info!(tab_id = %tab_id, dest_ws = %dest_ws_id, insert_at = %insert_at, "[dnd] move_tab_to_workspace complete");
    Ok(())
}

/// Tear off a block into a new workspace.
/// Removes the block from `source_tab_id`, creates a new workspace with a
/// single tab containing the block. Returns the new workspace.
/// If `auto_close_source` is true, deletes the source tab when it becomes empty.
pub fn tear_off_block(
    store: &WaveStore,
    block_id: &str,
    source_tab_id: &str,
    source_ws_id: &str,
    auto_close_source: bool,
) -> Result<Workspace, StoreError> {
    tracing::info!(
        block_id = %block_id,
        source_tab = %source_tab_id,
        source_ws = %source_ws_id,
        auto_close = %auto_close_source,
        "[dnd] tear_off_block"
    );
    // Verify block exists
    let mut block = store.must_get::<Block>(block_id)?;

    // Remove block from source tab's blockids and queue a layout delete action
    // so the source window's frontend removes the node from its layout tree.
    let mut source_tab = store.must_get::<Tab>(source_tab_id)?;
    source_tab.blockids.retain(|id| id != block_id);
    store.update(&mut source_tab)?;

    let mut source_layout = store.must_get::<LayoutState>(&source_tab.layoutstate)?;
    let mut actions = source_layout.pendingbackendactions.take().unwrap_or_default();
    actions.push(LayoutActionData {
        actiontype: "delete".to_string(),
        actionid: Uuid::new_v4().to_string(),
        blockid: block_id.to_string(),
        nodesize: None,
        indexarr: None,
        focused: false,
        magnified: false,
        ephemeral: false,
        targetblockid: String::new(),
        position: String::new(),
    });
    source_layout.pendingbackendactions = Some(actions);
    store.update(&mut source_layout)?;

    // Create new workspace
    let new_ws = create_workspace(store, "", "", "")?;
    // create_tab adds a tab and sets it as active
    let new_tab = create_tab(store, &new_ws.oid)?;

    // Add block to the new tab
    let mut new_tab = store.must_get::<Tab>(&new_tab.oid)?;
    new_tab.blockids.push(block_id.to_string());
    store.update(&mut new_tab)?;

    // Set up the layout tree for the new tab with the block as the single root node.
    // Without this, the frontend renders an empty layout (rootnode: null).
    let mut layout = store.must_get::<LayoutState>(&new_tab.layoutstate)?;
    layout.rootnode = Some(serde_json::json!({
        "id": Uuid::new_v4().to_string(),
        "data": { "blockId": block_id },
        "flexDirection": "row",
        "size": 1
    }));
    layout.leaforder = Some(vec![LeafOrderEntry {
        nodeid: layout.rootnode.as_ref().unwrap()["id"].as_str().unwrap().to_string(),
        blockid: block_id.to_string(),
    }]);
    store.update(&mut layout)?;

    // Update block's parent reference
    block.parentoref = format!("tab:{}", new_tab.oid);
    store.update(&mut block)?;

    // Auto-close empty source tab if requested
    if auto_close_source && source_tab.blockids.is_empty() {
        let ws = store.must_get::<Workspace>(source_ws_id)?;
        let total_tabs = ws.tabids.len() + ws.pinnedtabids.len();
        if total_tabs > 1 {
            tracing::info!(source_tab = %source_tab_id, "[dnd] auto-closing empty source tab after tear-off");
            delete_tab(store, source_ws_id, source_tab_id)?;
        }
    }

    tracing::info!(
        block_id = %block_id,
        new_ws = %new_ws.oid,
        new_tab = %new_tab.oid,
        "[dnd] tear_off_block complete"
    );
    // Re-fetch workspace to return updated state
    store.must_get::<Workspace>(&new_ws.oid)
}

/// Tear off a tab into a new workspace.
/// Removes the tab from the source workspace and creates a new workspace
/// containing just that tab. Returns the new workspace.
/// The source workspace must have more than one tab.
pub fn tear_off_tab(
    store: &WaveStore,
    tab_id: &str,
    source_ws_id: &str,
) -> Result<Workspace, StoreError> {
    tracing::info!(tab_id = %tab_id, source_ws = %source_ws_id, "[dnd] tear_off_tab");
    // Remove tab from source workspace
    let mut source_ws = store.must_get::<Workspace>(source_ws_id)?;
    let total_tabs = source_ws.tabids.len() + source_ws.pinnedtabids.len();
    if total_tabs <= 1 {
        tracing::warn!(tab_id = %tab_id, total_tabs = %total_tabs, "[dnd] tear_off_tab blocked: last tab");
        return Err(StoreError::Other(
            "cannot tear off last tab from workspace".to_string(),
        ));
    }
    source_ws.tabids.retain(|id| id != tab_id);
    source_ws.pinnedtabids.retain(|id| id != tab_id);
    if source_ws.activetabid == tab_id {
        let new_active = source_ws
            .tabids
            .first()
            .or(source_ws.pinnedtabids.first())
            .cloned()
            .unwrap_or_default();
        tracing::info!(old_active = %tab_id, new_active = %new_active, "[dnd] switching active tab after tear-off");
        source_ws.activetabid = new_active;
    }
    store.update(&mut source_ws)?;

    // Create a new workspace with the tab
    let mut new_ws = Workspace {
        oid: Uuid::new_v4().to_string(),
        name: String::new(),
        icon: String::new(),
        color: String::new(),
        tabids: vec![tab_id.to_string()],
        pinnedtabids: vec![],
        activetabid: tab_id.to_string(),
        meta: MetaMapType::new(),
        ..Default::default()
    };
    store.insert(&mut new_ws)?;

    tracing::info!(tab_id = %tab_id, new_ws = %new_ws.oid, "[dnd] tear_off_tab complete");
    Ok(new_ws)
}
