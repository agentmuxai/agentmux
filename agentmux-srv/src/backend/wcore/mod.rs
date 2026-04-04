// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Wave Core: application coordinator for storage + pub/sub.
//! Port of Go's pkg/wcore/wcore.go + window.go + workspace.go + block.go.
//!
//! Orchestrates WaveStore mutations with WPS event publishing.

mod block;
mod dnd;
mod event;
mod tab;
mod window;
mod workspace;

// Re-export all public APIs so callers can continue using `wcore::function_name`.
pub use block::*;
pub use dnd::*;
pub use event::*;
pub use tab::*;
pub use window::*;
pub use workspace::*;

use uuid::Uuid;

use super::storage::wstore::WaveStore;
use super::storage::StoreError;
use super::obj::*;

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
            window::check_and_fix_window(store, window_id)?;
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
    let win = create_window(store, &ws.oid)?;

    // Update client with window ID
    client.windowids.push(win.oid.clone());
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

        window::check_and_fix_window(&store, &window.oid).unwrap();

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
