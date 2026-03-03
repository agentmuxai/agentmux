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
    create_tab_with_opts(store, &ws.oid, "", true)?;

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
/// If `tab_name` is empty, auto-generates "T1", "T2", etc. (matching Go).
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

    // Auto-generate tab name if not provided (matches Go: "T" + count)
    let name = if tab_name.is_empty() {
        format!("T{}", ws.tabids.len() + ws.pinnedtabids.len() + 1)
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
        // Tab should be named "T1" (matching Go's auto-naming)
        assert_eq!(tabs[0].name, "T1");
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
}
