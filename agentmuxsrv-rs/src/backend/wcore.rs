// Copyright 2025, Command Line Inc.
// SPDX-License-Identifier: Apache-2.0

//! Wave Core: application coordinator for storage + pub/sub.
//! Port of Go's pkg/wcore/wcore.go + window.go + workspace.go + block.go.
//!
//! Orchestrates WaveStore mutations with WPS event publishing.

#![allow(dead_code)]

use std::collections::HashMap;
use uuid::Uuid;

use super::oref::ORef;
use super::storage::wstore::WaveStore;
use super::storage::StoreError;
use super::waveobj::*;
use super::wps::{self, Broker, WaveEvent};

// ---- Workspace defaults (match Go's WorkspaceColors/Icons) ----

pub const WORKSPACE_COLORS: &[&str] = &[
    "#58C142", "#00D1EC", "#FA2D01", "#FBB500", "#8B54FF", "#FF5E8E", "#2B80FF",
];

pub const WORKSPACE_ICONS: &[&str] = &[
    "custom@wave-logo-solid",
    "triangle",
    "star",
    "cube",
    "gem",
    "chess-knight",
    "heart",
    "plane",
    "rocket",
    "flask",
    "bolt",
    "music",
    "globe",
    "leaf",
];

// ---- Layout action types (match Go) ----

pub const LAYOUT_ACTION_INSERT: &str = "insert";
pub const LAYOUT_ACTION_INSERT_AT_INDEX: &str = "insertatindex";
pub const LAYOUT_ACTION_REMOVE: &str = "remove";
pub const LAYOUT_ACTION_CLEAR_TREE: &str = "cleartree";
pub const LAYOUT_ACTION_REPLACE: &str = "replace";
pub const LAYOUT_ACTION_SPLIT_HORIZONTAL: &str = "splithorizontal";
pub const LAYOUT_ACTION_SPLIT_VERTICAL: &str = "splitvertical";

// ---- Core operations ----

/// Ensure initial data is present in the store.
/// Creates a default Client, Window, Workspace, Tab if the store is empty.
/// Returns `true` if this is a first launch (client was just created).
pub fn ensure_initial_data(store: &WaveStore) -> Result<bool, StoreError> {
    let clients = store.get_all::<Client>()?;

    if !clients.is_empty() {
        // Already initialized
        let client = &clients[0];
        if client.tempoid.is_empty() {
            let mut client = client.clone();
            client.tempoid = Uuid::new_v4().to_string();
            store.update(&mut client)?;
        }
        // Check and fix windows
        for window_id in &client.windowids {
            check_and_fix_window(store, window_id)?;
        }
        return Ok(false);
    }

    // First launch: create client + window + workspace + tab
    let first_launch = true;

    // Go inserts client first (version 1), then updates TempOID (version 2).
    // We mirror that to keep the version counter in sync.
    let mut client = Client {
        oid: Uuid::new_v4().to_string(),
        windowids: vec![],
        tempoid: String::new(),
        meta: MetaMapType::new(),
        ..Default::default()
    };

    store.insert(&mut client)?;

    // Separate update for TempOID (matches Go's version 2 update)
    client.tempoid = Uuid::new_v4().to_string();
    store.update(&mut client)?;

    // Create starter workspace
    let ws = create_workspace(
        store,
        "Starter workspace",
        WORKSPACE_ICONS[0],
        WORKSPACE_COLORS[0],
    )?;

    // Create window pointing to workspace
    let window = create_window(store, &ws.oid)?;

    // Update client with window ID
    client.windowids.push(window.oid.clone());
    store.update(&mut client)?;

    // Create initial tab in workspace (pinned, matching Go's isInitialLaunch=true)
    let tab = create_tab_with_opts(store, &ws.oid, "", true)?;

    // Add a default Swarm widget block to the initial tab
    let mut swarm_meta = MetaMapType::new();
    swarm_meta.insert("view".to_string(), serde_json::json!("swarm"));
    let swarm_block = create_block(store, &tab.oid, swarm_meta)?;

    // Queue layout action so the frontend renders the swarm block
    let mut layout = store.must_get::<LayoutState>(&tab.layoutstate)?;
    layout.pendingbackendactions = Some(vec![LayoutActionData {
        actiontype: "insert".to_string(),
        actionid: Uuid::new_v4().to_string(),
        blockid: swarm_block.oid.clone(),
        nodesize: None,
        indexarr: None,
        focused: true,
        magnified: false,
        ephemeral: false,
        targetblockid: String::new(),
        position: String::new(),
    }]);
    store.update(&mut layout)?;

    Ok(first_launch)
}

/// Get the singleton client record.
pub fn get_client(store: &WaveStore) -> Result<Client, StoreError> {
    let clients = store.get_all::<Client>()?;
    clients.into_iter().next().ok_or(StoreError::NotFound)
}

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
        delete_tab_inner(store, tab_id)?;
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
fn delete_tab_inner(store: &WaveStore, tab_id: &str) -> Result<(), StoreError> {
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

/// Create a new block in a tab.
pub fn create_block(
    store: &WaveStore,
    tab_id: &str,
    meta: MetaMapType,
) -> Result<Block, StoreError> {
    let mut tab = store.must_get::<Tab>(tab_id)?;

    let mut block = Block {
        oid: Uuid::new_v4().to_string(),
        parentoref: format!("tab:{}", tab_id),
        meta,
        ..Default::default()
    };
    store.insert(&mut block)?;

    tab.blockids.push(block.oid.clone());
    store.update(&mut tab)?;

    Ok(block)
}

/// Delete a block from its parent tab.
pub fn delete_block(
    store: &WaveStore,
    tab_id: &str,
    block_id: &str,
) -> Result<(), StoreError> {
    let mut tab = store.must_get::<Tab>(tab_id)?;
    tab.blockids.retain(|id| id != block_id);
    store.update(&mut tab)?;
    store.delete::<Block>(block_id)?;
    Ok(())
}

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

    // Remove block from source tab
    let mut source_tab = store.must_get::<Tab>(source_tab_id)?;
    source_tab.blockids.retain(|id| id != block_id);
    store.update(&mut source_tab)?;

    // Create new workspace
    let new_ws = create_workspace(store, "", "", "")?;
    // create_tab adds a tab and sets it as active
    let new_tab = create_tab(store, &new_ws.oid)?;

    // Add block to the new tab
    let mut new_tab = store.must_get::<Tab>(&new_tab.oid)?;
    new_tab.blockids.push(block_id.to_string());
    store.update(&mut new_tab)?;

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

/// Create a new window pointing to a workspace.
/// If workspace_id is empty, auto-creates a new workspace + default tab (matches Go behavior).
pub fn create_window(
    store: &WaveStore,
    workspace_id: &str,
) -> Result<Window, StoreError> {
    let ws_id = if workspace_id.is_empty() {
        let ws = create_workspace(store, "", "", "")?;
        let _tab = create_tab(store, &ws.oid)?;
        ws.oid
    } else {
        let _ws = store.must_get::<Workspace>(workspace_id)?;
        workspace_id.to_string()
    };
    let mut window = Window {
        oid: Uuid::new_v4().to_string(),
        workspaceid: ws_id,
        isnew: true,
        pos: Point { x: 0, y: 0 },
        winsize: WinSize {
            width: 0,
            height: 0,
        },
        meta: MetaMapType::new(),
        ..Default::default()
    };
    store.insert(&mut window)?;
    Ok(window)
}

/// Create a new window with all required objects in a single transaction.
/// If workspace_id is empty, creates workspace + tab + window + updates client
/// all in one BEGIN/COMMIT — reducing 8+ lock acquisitions and fsyncs to 1.
///
/// Returns the created Window.
pub fn create_window_full(
    store: &WaveStore,
    workspace_id: &str,
) -> Result<Window, StoreError> {
    store.with_tx(|tx| {
        let ws_id = if workspace_id.is_empty() {
            // Create workspace
            let mut ws = Workspace {
                oid: Uuid::new_v4().to_string(),
                name: String::new(),
                icon: String::new(),
                color: String::new(),
                tabids: vec![],
                pinnedtabids: vec![],
                activetabid: String::new(),
                meta: MetaMapType::new(),
                ..Default::default()
            };
            tx.insert(&mut ws)?;

            // Create layout state for tab
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
            tx.insert(&mut layout)?;

            // Create tab
            let mut tab = Tab {
                oid: Uuid::new_v4().to_string(),
                name: format!("T{}", ws.tabids.len() + ws.pinnedtabids.len() + 1),
                layoutstate: layout.oid.clone(),
                blockids: vec![],
                meta: MetaMapType::new(),
                ..Default::default()
            };
            tx.insert(&mut tab)?;

            // Link tab to workspace
            ws.tabids.push(tab.oid.clone());
            ws.activetabid = tab.oid.clone();
            tx.update(&mut ws)?;

            ws.oid
        } else {
            let _ws = tx.must_get::<Workspace>(workspace_id)?;
            workspace_id.to_string()
        };

        // Create window
        let mut window = Window {
            oid: Uuid::new_v4().to_string(),
            workspaceid: ws_id,
            isnew: true,
            pos: Point { x: 0, y: 0 },
            winsize: WinSize {
                width: 0,
                height: 0,
            },
            meta: MetaMapType::new(),
            ..Default::default()
        };
        tx.insert(&mut window)?;

        // Update client with new window ID
        let clients = tx.get_all::<Client>()?;
        if let Some(client) = clients.into_iter().next() {
            let mut client = client;
            client.windowids.push(window.oid.clone());
            tx.update(&mut client)?;
        }

        Ok(window)
    })
}

/// Close a window and remove from client's window list.
pub fn close_window(store: &WaveStore, window_id: &str) -> Result<(), StoreError> {
    let mut client = get_client(store)?;
    client.windowids.retain(|id| id != window_id);
    store.update(&mut client)?;
    store.delete::<Window>(window_id)?;
    Ok(())
}

/// Focus a window (move to front of client's window list).
pub fn focus_window(store: &WaveStore, window_id: &str) -> Result<(), StoreError> {
    let mut client = get_client(store)?;
    if let Some(pos) = client.windowids.iter().position(|id| id == window_id) {
        let id = client.windowids.remove(pos);
        client.windowids.insert(0, id);
        store.update(&mut client)?;
    }
    Ok(())
}

/// Switch a window to a different workspace.
pub fn switch_workspace(
    store: &WaveStore,
    window_id: &str,
    ws_id: &str,
) -> Result<(), StoreError> {
    // Verify workspace exists
    let _ = store.must_get::<Workspace>(ws_id)?;

    let mut window = store.must_get::<Window>(window_id)?;
    window.workspaceid = ws_id.to_string();
    store.update(&mut window)?;
    Ok(())
}

/// Check and fix a window — ensure it has a valid workspace with tabs.
fn check_and_fix_window(store: &WaveStore, window_id: &str) -> Result<(), StoreError> {
    let window = match store.get::<Window>(window_id)? {
        Some(w) => w,
        None => return Ok(()), // window doesn't exist, nothing to fix
    };

    // Check workspace exists
    let ws = match store.get::<Workspace>(&window.workspaceid)? {
        Some(ws) => ws,
        None => {
            // Workspace missing — create a new one
            let ws = create_workspace(store, "", WORKSPACE_ICONS[0], WORKSPACE_COLORS[0])?;
            let mut window = window;
            window.workspaceid = ws.oid.clone();
            store.update(&mut window)?;
            ws
        }
    };

    // Ensure workspace has at least one tab (matches Go: checks both tabids and pinnedtabids)
    if ws.tabids.is_empty() && ws.pinnedtabids.is_empty() {
        create_tab(store, &ws.oid)?;
    }

    Ok(())
}

/// Resolve a block ID from an 8-character prefix within a tab.
pub fn resolve_block_id_from_prefix(
    store: &WaveStore,
    tab_id: &str,
    prefix: &str,
) -> Result<String, StoreError> {
    if prefix.len() != 8 {
        return Err(StoreError::Other(
            "block_id prefix must be 8 characters".to_string(),
        ));
    }
    let tab = store.must_get::<Tab>(tab_id)?;
    for block_id in &tab.blockids {
        if block_id.starts_with(prefix) {
            return Ok(block_id.clone());
        }
    }
    Err(StoreError::NotFound)
}

/// Publish a WaveObj update event through the broker.
pub fn send_wave_obj_update(broker: &Broker, store: &WaveStore, oref: &ORef) {
    let obj_json = match oref.otype.as_str() {
        OTYPE_CLIENT => store
            .get::<Client>(&oref.oid)
            .ok()
            .flatten()
            .and_then(|o| serde_json::to_value(&o).ok()),
        OTYPE_WINDOW => store
            .get::<Window>(&oref.oid)
            .ok()
            .flatten()
            .and_then(|o| serde_json::to_value(&o).ok()),
        OTYPE_WORKSPACE => store
            .get::<Workspace>(&oref.oid)
            .ok()
            .flatten()
            .and_then(|o| serde_json::to_value(&o).ok()),
        OTYPE_TAB => store
            .get::<Tab>(&oref.oid)
            .ok()
            .flatten()
            .and_then(|o| serde_json::to_value(&o).ok()),
        OTYPE_LAYOUT => store
            .get::<LayoutState>(&oref.oid)
            .ok()
            .flatten()
            .and_then(|o| serde_json::to_value(&o).ok()),
        OTYPE_BLOCK => store
            .get::<Block>(&oref.oid)
            .ok()
            .flatten()
            .and_then(|o| serde_json::to_value(&o).ok()),
        _ => None,
    };

    if let Some(obj) = obj_json {
        broker.publish(WaveEvent {
            event: wps::EVENT_WAVE_OBJ_UPDATE.to_string(),
            scopes: vec![oref.to_string()],
            sender: String::new(),
            persist: 0,
            data: Some(serde_json::json!({
                "updatetype": UPDATE_TYPE_UPDATE,
                "otype": oref.otype,
                "oid": oref.oid,
                "obj": obj,
            })),
        });
    }
}

// ====================================================================
// Tests
// ====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_store() -> WaveStore {
        WaveStore::open_in_memory().unwrap()
    }

    #[test]
    fn test_ensure_initial_data_first_launch() {
        let store = make_store();
        let first = ensure_initial_data(&store).unwrap();
        assert!(first);

        // Should have created client, window, workspace, tab
        let client = get_client(&store).unwrap();
        assert_eq!(client.windowids.len(), 1);
        assert!(!client.tempoid.is_empty());

        let windows = store.get_all::<Window>().unwrap();
        assert_eq!(windows.len(), 1);
        // Window should have pos:{0,0} and winsize:{0,0} (matching Go)
        assert_eq!(windows[0].pos.x, 0);
        assert_eq!(windows[0].pos.y, 0);
        assert_eq!(windows[0].winsize.width, 0);
        assert_eq!(windows[0].winsize.height, 0);

        let workspaces = store.get_all::<Workspace>().unwrap();
        assert_eq!(workspaces.len(), 1);
        assert_eq!(workspaces[0].name, "Starter workspace");
        // Starter tab should be pinned (matching Go's isInitialLaunch=true)
        assert_eq!(workspaces[0].pinnedtabids.len(), 1);
        assert_eq!(workspaces[0].tabids.len(), 0);

        let tabs = store.get_all::<Tab>().unwrap();
        assert_eq!(tabs.len(), 1);
        // Tab should be named "Untitled1"
        assert_eq!(tabs[0].name, "Untitled1");
    }

    #[test]
    fn test_ensure_initial_data_idempotent() {
        let store = make_store();
        let first = ensure_initial_data(&store).unwrap();
        assert!(first);

        let second = ensure_initial_data(&store).unwrap();
        assert!(!second);

        // Should still have exactly 1 client
        assert_eq!(store.count::<Client>().unwrap(), 1);
    }

    #[test]
    fn test_create_and_delete_workspace() {
        let store = make_store();
        let ws = create_workspace(&store, "Test WS", "star", "#FF0000").unwrap();
        assert_eq!(ws.name, "Test WS");

        // Create tabs in workspace
        let t1 = create_tab(&store, &ws.oid).unwrap();
        let t2 = create_tab(&store, &ws.oid).unwrap();

        let ws = get_workspace(&store, &ws.oid).unwrap();
        assert_eq!(ws.tabids.len(), 2);

        // Delete workspace cascades to tabs
        let t1_oid = t1.oid.clone();
        let t2_oid = t2.oid.clone();
        delete_workspace(&store, &ws.oid).unwrap();
        assert!(store.get::<Workspace>(&ws.oid).unwrap().is_none());
        assert!(store.get::<Tab>(&t1_oid).unwrap().is_none());
        assert!(store.get::<Tab>(&t2_oid).unwrap().is_none());
    }

    #[test]
    fn test_create_and_delete_tab() {
        let store = make_store();
        let ws = create_workspace(&store, "WS", "star", "#000").unwrap();
        let tab1 = create_tab(&store, &ws.oid).unwrap();
        let tab2 = create_tab(&store, &ws.oid).unwrap();

        let ws = get_workspace(&store, &ws.oid).unwrap();
        assert_eq!(ws.tabids.len(), 2);
        assert_eq!(ws.activetabid, tab1.oid);

        // Delete active tab — active should switch to tab2
        delete_tab(&store, &ws.oid, &tab1.oid).unwrap();
        let ws = get_workspace(&store, &ws.oid).unwrap();
        assert_eq!(ws.tabids.len(), 1);
        assert_eq!(ws.activetabid, tab2.oid);
    }

    #[test]
    fn test_set_active_tab() {
        let store = make_store();
        let ws = create_workspace(&store, "WS", "star", "#000").unwrap();
        let _tab1 = create_tab(&store, &ws.oid).unwrap();
        let tab2 = create_tab(&store, &ws.oid).unwrap();

        set_active_tab(&store, &ws.oid, &tab2.oid).unwrap();
        let ws = get_workspace(&store, &ws.oid).unwrap();
        assert_eq!(ws.activetabid, tab2.oid);

        // Setting non-existent tab should fail
        let result = set_active_tab(&store, &ws.oid, "nonexistent");
        assert!(result.is_err());
    }

    #[test]
    fn test_create_and_delete_block() {
        let store = make_store();
        let ws = create_workspace(&store, "WS", "star", "#000").unwrap();
        let tab = create_tab(&store, &ws.oid).unwrap();

        let mut meta = MetaMapType::new();
        meta.insert("view".to_string(), serde_json::json!("term"));
        let block = create_block(&store, &tab.oid, meta).unwrap();

        let tab = store.must_get::<Tab>(&tab.oid).unwrap();
        assert_eq!(tab.blockids.len(), 1);
        assert_eq!(tab.blockids[0], block.oid);

        let loaded = store.must_get::<Block>(&block.oid).unwrap();
        assert_eq!(loaded.parentoref, format!("tab:{}", tab.oid));
        assert_eq!(loaded.meta.get("view").unwrap(), "term");

        delete_block(&store, &tab.oid, &block.oid).unwrap();
        assert!(store.get::<Block>(&block.oid).unwrap().is_none());
        let tab = store.must_get::<Tab>(&tab.oid).unwrap();
        assert!(tab.blockids.is_empty());
    }

    #[test]
    fn test_create_and_close_window() {
        let store = make_store();
        ensure_initial_data(&store).unwrap();

        let client = get_client(&store).unwrap();
        let initial_count = client.windowids.len();

        let ws = create_workspace(&store, "WS2", "star", "#000").unwrap();
        let window = create_window(&store, &ws.oid).unwrap();

        // Add to client
        let mut client = get_client(&store).unwrap();
        client.windowids.push(window.oid.clone());
        store.update(&mut client).unwrap();

        close_window(&store, &window.oid).unwrap();
        let client = get_client(&store).unwrap();
        assert_eq!(client.windowids.len(), initial_count);
        assert!(store.get::<Window>(&window.oid).unwrap().is_none());
    }

    #[test]
    fn test_focus_window() {
        let store = make_store();
        ensure_initial_data(&store).unwrap();

        let ws = create_workspace(&store, "WS2", "star", "#000").unwrap();
        let w2 = create_window(&store, &ws.oid).unwrap();
        let mut client = get_client(&store).unwrap();
        client.windowids.push(w2.oid.clone());
        store.update(&mut client).unwrap();

        // w2 should be last, focus should move it first
        focus_window(&store, &w2.oid).unwrap();
        let client = get_client(&store).unwrap();
        assert_eq!(client.windowids[0], w2.oid);
    }

    #[test]
    fn test_switch_workspace() {
        let store = make_store();
        ensure_initial_data(&store).unwrap();

        let client = get_client(&store).unwrap();
        let window_id = &client.windowids[0];
        let window = store.must_get::<Window>(window_id).unwrap();
        let old_ws = window.workspaceid.clone();

        let new_ws = create_workspace(&store, "New WS", "star", "#000").unwrap();
        switch_workspace(&store, window_id, &new_ws.oid).unwrap();

        let window = store.must_get::<Window>(window_id).unwrap();
        assert_eq!(window.workspaceid, new_ws.oid);
        assert_ne!(window.workspaceid, old_ws);
    }

    #[test]
    fn test_resolve_block_id_prefix() {
        let store = make_store();
        let ws = create_workspace(&store, "WS", "star", "#000").unwrap();
        let tab = create_tab(&store, &ws.oid).unwrap();

        let meta = MetaMapType::new();
        let block = create_block(&store, &tab.oid, meta).unwrap();
        let prefix = &block.oid[..8];

        let resolved = resolve_block_id_from_prefix(&store, &tab.oid, prefix).unwrap();
        assert_eq!(resolved, block.oid);

        // Non-matching prefix
        let result = resolve_block_id_from_prefix(&store, &tab.oid, "00000000");
        assert!(result.is_err());
    }

    #[test]
    fn test_list_workspaces() {
        let store = make_store();
        create_workspace(&store, "WS1", "star", "#000").unwrap();
        create_workspace(&store, "WS2", "star", "#111").unwrap();
        create_workspace(&store, "WS3", "star", "#222").unwrap();

        let all = list_workspaces(&store).unwrap();
        assert_eq!(all.len(), 3);
    }

    #[test]
    fn test_check_and_fix_window_missing_workspace() {
        let store = make_store();
        // Create a window pointing to a non-existent workspace
        let mut window = Window {
            oid: Uuid::new_v4().to_string(),
            workspaceid: "nonexistent".to_string(),
            pos: Point { x: 0, y: 0 },
            winsize: WinSize {
                width: 800,
                height: 600,
            },
            meta: MetaMapType::new(),
            ..Default::default()
        };
        store.insert(&mut window).unwrap();

        check_and_fix_window(&store, &window.oid).unwrap();

        // Should have created a new workspace and pointed window to it
        let fixed = store.must_get::<Window>(&window.oid).unwrap();
        assert_ne!(fixed.workspaceid, "nonexistent");
        let ws = store.must_get::<Workspace>(&fixed.workspaceid).unwrap();
        assert_eq!(ws.tabids.len(), 1); // should have created a tab too
    }

    #[test]
    fn test_move_block_to_tab() {
        let store = make_store();
        let ws = create_workspace(&store, "WS", "star", "#000").unwrap();
        let tab1 = create_tab(&store, &ws.oid).unwrap();
        let tab2 = create_tab(&store, &ws.oid).unwrap();

        let meta = MetaMapType::new();
        let block = create_block(&store, &tab1.oid, meta).unwrap();

        // Verify block is in tab1
        let t1 = store.must_get::<Tab>(&tab1.oid).unwrap();
        assert_eq!(t1.blockids.len(), 1);
        assert_eq!(t1.blockids[0], block.oid);

        // Move block from tab1 to tab2
        move_block_to_tab(&store, &block.oid, &tab1.oid, &tab2.oid, &ws.oid, false).unwrap();

        // tab1 should be empty, tab2 should have the block
        let t1 = store.must_get::<Tab>(&tab1.oid).unwrap();
        let t2 = store.must_get::<Tab>(&tab2.oid).unwrap();
        assert!(t1.blockids.is_empty());
        assert_eq!(t2.blockids.len(), 1);
        assert_eq!(t2.blockids[0], block.oid);

        // Block parentoref should point to tab2
        let b = store.must_get::<Block>(&block.oid).unwrap();
        assert_eq!(b.parentoref, format!("tab:{}", tab2.oid));
    }

    #[test]
    fn test_move_block_to_tab_auto_close() {
        let store = make_store();
        let ws = create_workspace(&store, "WS", "star", "#000").unwrap();
        let tab1 = create_tab(&store, &ws.oid).unwrap();
        let tab2 = create_tab(&store, &ws.oid).unwrap();

        let block = create_block(&store, &tab1.oid, MetaMapType::new()).unwrap();

        // Move with auto_close=true — tab1 should be deleted since it becomes empty
        move_block_to_tab(&store, &block.oid, &tab1.oid, &tab2.oid, &ws.oid, true).unwrap();

        // tab1 should be deleted
        assert!(store.get::<Tab>(&tab1.oid).unwrap().is_none());

        // workspace should only have tab2
        let ws = store.must_get::<Workspace>(&ws.oid).unwrap();
        assert_eq!(ws.tabids.len(), 1);
        assert_eq!(ws.tabids[0], tab2.oid);
    }

    #[test]
    fn test_move_block_same_tab_noop() {
        let store = make_store();
        let ws = create_workspace(&store, "WS", "star", "#000").unwrap();
        let tab = create_tab(&store, &ws.oid).unwrap();
        let block = create_block(&store, &tab.oid, MetaMapType::new()).unwrap();

        // Moving to same tab should be a no-op
        move_block_to_tab(&store, &block.oid, &tab.oid, &tab.oid, &ws.oid, false).unwrap();

        let t = store.must_get::<Tab>(&tab.oid).unwrap();
        assert_eq!(t.blockids.len(), 1);
    }

    #[test]
    fn test_promote_block_to_tab() {
        let store = make_store();
        let ws = create_workspace(&store, "WS", "star", "#000").unwrap();
        let tab = create_tab(&store, &ws.oid).unwrap();
        let block = create_block(&store, &tab.oid, MetaMapType::new()).unwrap();

        // Promote block to new tab
        let new_tab = promote_block_to_tab(&store, &block.oid, &tab.oid, &ws.oid, false).unwrap();

        // Original tab should be empty
        let old_tab = store.must_get::<Tab>(&tab.oid).unwrap();
        assert!(old_tab.blockids.is_empty());

        // New tab should have the block
        let nt = store.must_get::<Tab>(&new_tab.oid).unwrap();
        assert_eq!(nt.blockids.len(), 1);
        assert_eq!(nt.blockids[0], block.oid);

        // Workspace should have both tabs
        let ws = store.must_get::<Workspace>(&ws.oid).unwrap();
        assert_eq!(ws.tabids.len(), 2);

        // New tab should be active
        assert_eq!(ws.activetabid, new_tab.oid);
    }

    #[test]
    fn test_reorder_tab() {
        let store = make_store();
        let ws = create_workspace(&store, "WS", "star", "#000").unwrap();
        let tab1 = create_tab(&store, &ws.oid).unwrap();
        let tab2 = create_tab(&store, &ws.oid).unwrap();
        let tab3 = create_tab(&store, &ws.oid).unwrap();

        // Verify initial order: [tab1, tab2, tab3]
        let ws_data = store.must_get::<Workspace>(&ws.oid).unwrap();
        assert_eq!(ws_data.tabids, vec![tab1.oid.clone(), tab2.oid.clone(), tab3.oid.clone()]);

        // Move tab3 to index 0
        reorder_tab(&store, &ws.oid, &tab3.oid, 0).unwrap();

        let ws_data = store.must_get::<Workspace>(&ws.oid).unwrap();
        assert_eq!(ws_data.tabids, vec![tab3.oid.clone(), tab1.oid.clone(), tab2.oid.clone()]);

        // Move tab1 to end (index 99 should clamp to len)
        reorder_tab(&store, &ws.oid, &tab1.oid, 99).unwrap();

        let ws_data = store.must_get::<Workspace>(&ws.oid).unwrap();
        assert_eq!(ws_data.tabids, vec![tab3.oid.clone(), tab2.oid.clone(), tab1.oid.clone()]);
    }

    #[test]
    fn test_move_tab_to_workspace() {
        let store = make_store();
        let ws1 = create_workspace(&store, "WS1", "star", "#000").unwrap();
        let ws2 = create_workspace(&store, "WS2", "moon", "#fff").unwrap();
        let tab1 = create_tab(&store, &ws1.oid).unwrap();
        let tab2 = create_tab(&store, &ws1.oid).unwrap();
        let tab3 = create_tab(&store, &ws2.oid).unwrap();

        // Set tab1 as active in ws1
        set_active_tab(&store, &ws1.oid, &tab1.oid).unwrap();

        // Move tab2 from ws1 to ws2
        move_tab_to_workspace(&store, &tab2.oid, &ws1.oid, &ws2.oid, None).unwrap();

        let ws1_data = store.must_get::<Workspace>(&ws1.oid).unwrap();
        let ws2_data = store.must_get::<Workspace>(&ws2.oid).unwrap();

        assert_eq!(ws1_data.tabids, vec![tab1.oid.clone()]);
        assert!(ws2_data.tabids.contains(&tab2.oid));
        assert!(ws2_data.tabids.contains(&tab3.oid));
        // Moved tab becomes active in destination
        assert_eq!(ws2_data.activetabid, tab2.oid);
    }

    #[test]
    fn test_move_tab_to_workspace_last_tab_blocked() {
        let store = make_store();
        let ws1 = create_workspace(&store, "WS1", "star", "#000").unwrap();
        let ws2 = create_workspace(&store, "WS2", "moon", "#fff").unwrap();
        let tab1 = create_tab(&store, &ws1.oid).unwrap();
        let _tab2 = create_tab(&store, &ws2.oid).unwrap();

        // Should fail — can't move the only tab out
        let result = move_tab_to_workspace(&store, &tab1.oid, &ws1.oid, &ws2.oid, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_tear_off_block() {
        let store = make_store();
        let ws = create_workspace(&store, "WS", "star", "#000").unwrap();
        let tab = create_tab(&store, &ws.oid).unwrap();
        let block = create_block(&store, &tab.oid, MetaMapType::new()).unwrap();
        let block2 = create_block(&store, &tab.oid, MetaMapType::new()).unwrap();

        // Tear off block (not auto_close because tab still has block2)
        let new_ws = tear_off_block(&store, &block.oid, &tab.oid, &ws.oid, true).unwrap();

        // Block removed from source tab
        let tab_data = store.must_get::<Tab>(&tab.oid).unwrap();
        assert!(!tab_data.blockids.contains(&block.oid));
        assert!(tab_data.blockids.contains(&block2.oid));

        // New workspace created with the block
        assert!(!new_ws.oid.is_empty());
        assert_eq!(new_ws.tabids.len(), 1);
        let new_tab = store.must_get::<Tab>(&new_ws.tabids[0]).unwrap();
        assert!(new_tab.blockids.contains(&block.oid));

        // Block parent ref updated
        let block_data = store.must_get::<Block>(&block.oid).unwrap();
        assert_eq!(block_data.parentoref, format!("tab:{}", new_tab.oid));
    }

    #[test]
    fn test_tear_off_tab() {
        let store = make_store();
        let ws = create_workspace(&store, "WS", "star", "#000").unwrap();
        let tab1 = create_tab(&store, &ws.oid).unwrap();
        let tab2 = create_tab(&store, &ws.oid).unwrap();
        set_active_tab(&store, &ws.oid, &tab1.oid).unwrap();

        // Tear off tab2
        let new_ws = tear_off_tab(&store, &tab2.oid, &ws.oid).unwrap();

        // Source workspace no longer has tab2
        let ws_data = store.must_get::<Workspace>(&ws.oid).unwrap();
        assert_eq!(ws_data.tabids, vec![tab1.oid.clone()]);

        // New workspace has tab2
        assert_eq!(new_ws.tabids, vec![tab2.oid.clone()]);
        assert_eq!(new_ws.activetabid, tab2.oid);
    }

    #[test]
    fn test_tear_off_last_tab_blocked() {
        let store = make_store();
        let ws = create_workspace(&store, "WS", "star", "#000").unwrap();
        let tab1 = create_tab(&store, &ws.oid).unwrap();

        // Should fail — can't tear off the only tab
        let result = tear_off_tab(&store, &tab1.oid, &ws.oid);
        assert!(result.is_err());
    }
}
