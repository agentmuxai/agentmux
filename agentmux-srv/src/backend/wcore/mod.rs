#![allow(dead_code)]
// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Wave Core: application coordinator for storage + pub/sub.
//! Port of Go's pkg/wcore/wcore.go + window.go + workspace.go + block.go.
//!
//! Orchestrates Store mutations with WPS event publishing.

mod block;
mod dnd;
mod event;
mod tab;
mod window;
mod workspace;

// Re-export all public APIs so callers can continue using `wcore::function_name`.
pub use block::*;
pub use dnd::*;
#[allow(unused_imports)]
pub use event::*;
pub use tab::*;
pub use window::*;
pub use workspace::*;

use uuid::Uuid;

use super::storage::store::Store;
use super::storage::StoreError;
use super::obj::*;

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
pub fn ensure_initial_data(store: &Store) -> Result<bool, StoreError> {
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
    let ws = create_workspace(store, "Starter workspace")?;

    // Create window pointing to workspace
    let mut win = create_window(store, &ws.oid)?;

    // Seed the window's display name (drives the OS/taskbar title) from
    // AGENTMUX_WINDOW_NAME if the launcher/caller set it. This lets an agent
    // bring up a recognizable instance, e.g. `AGENTMUX_WINDOW_NAME="repro #1503"
    // task dev`. Only applies to a fresh instance's first window (this
    // initial-data path); the runtime `SetWindowName` verb renames later.
    // See SPEC_AGENT_API_FIRST_CLASS_SURFACE_2026_06_17.md §4.4.
    if let Ok(name) = std::env::var("AGENTMUX_WINDOW_NAME") {
        let name = name.trim();
        if !name.is_empty() {
            win.meta.insert(
                "window:displayname".to_string(),
                serde_json::json!(name.chars().take(64).collect::<String>()),
            );
            store.update(&mut win)?;
        }
    }

    // Update client with window ID
    client.windowids.push(win.oid.clone());
    store.update(&mut client)?;

    // Create initial tab in workspace (pinned, matching Go's isInitialLaunch=true)
    let tab = create_tab_with_opts(store, &ws.oid, "", true)?;

    // Seed the default 4-pane launch layout (agent + swarm + armory +
    // sysinfo). Shared with the new-window path so "Open another window"
    // matches first launch.
    seed_default_layout(store, &tab.oid)?;

    Ok(first_launch)
}

/// Seed a tab with the default 4-pane launch layout (agent + swarm + armory
/// + sysinfo — SPEC_DEFAULT_WIDGETS_REORDER_2026_08_25.md):
///
/// ```text
///   ┌────────────────┬──────────────┐
///   │                │    swarm     │  size 4 of 10 ≈ 40%
///   │     agent      ├──────────────┤
///   │    (tall)      │   armory     │  size 4 of 10 ≈ 40%
///   │                ├──────────────┤
///   │                │   sysinfo    │  size 2 of 10 ≈ 20%
///   └────────────────┴──────────────┘
///        50% width         50% width
/// ```
///
/// Shared by first-launch bootstrap (`ensure_initial_data`) and the new-window
/// path (`server::service`'s `CreateWindow` with an empty workspace) so "Open
/// another window" gets the same starter layout instead of opening blank
/// (regression — see docs/retro/retro-blank-new-window-2026-06-21.md). The tab
/// must already exist with a valid `layoutstate`; both `create_tab_with_opts`
/// and the reducer's `apply_tab_created` provision one.
///
/// `rootnode` is built directly rather than queued via `pendingbackendactions`:
/// the frontend reducer races when draining multiple insert+split actions in one
/// cycle (`layoutPersistence.ts` + `layoutNodeModels.ts::getNodeByBlockId`); a
/// pre-built tree skips the reducer entirely and just renders.
pub(crate) fn seed_default_layout(store: &Store, tab_id: &str) -> Result<(), StoreError> {
    let tab = store.must_get::<Tab>(tab_id)?;

    let mut agent_meta = MetaMapType::new();
    agent_meta.insert("view".to_string(), serde_json::json!("agent"));
    let agent_block = create_block(store, &tab.oid, agent_meta)?;

    let mut swarm_meta = MetaMapType::new();
    swarm_meta.insert("view".to_string(), serde_json::json!("swarm"));
    let swarm_block = create_block(store, &tab.oid, swarm_meta)?;

    let mut armory_meta = MetaMapType::new();
    armory_meta.insert("view".to_string(), serde_json::json!("armory"));
    let armory_block = create_block(store, &tab.oid, armory_meta)?;

    let mut sysinfo_meta = MetaMapType::new();
    sysinfo_meta.insert("view".to_string(), serde_json::json!("sysinfo"));
    let sysinfo_block = create_block(store, &tab.oid, sysinfo_meta)?;

    write_default_four_pane_layout(
        store,
        tab_id,
        &agent_block.oid,
        &swarm_block.oid,
        &armory_block.oid,
        &sysinfo_block.oid,
    )
}

/// Write the default 4-pane launch layout (`agent | [swarm / armory /
/// sysinfo]`) into `tab_id`'s `LayoutState`, wrapping four block IDs that
/// the caller has ALREADY created.
///
/// Split out of `seed_default_layout` for the 2nd-window-tear-off desync fix
/// (#1681). `seed_default_layout` creates its blocks store-only via
/// `create_block`, which is correct for first-launch (`ensure_initial_data`
/// runs before bootstrap loads SQLite into the in-memory reducer `srv_state`).
/// But the post-bootstrap "open another window" path (`service.rs` CreateWindow)
/// must create the blocks THROUGH THE REDUCER so they also land in `srv_state`;
/// otherwise the new window's blocks live only in SQLite, the frontend renders
/// them, and `TearOffBlock` rejects them as "block not found" (the reducer never
/// saw them). Both callers share this one function for the tree shape so the
/// layout can never drift between the two paths.
pub(crate) fn write_default_four_pane_layout(
    store: &Store,
    tab_id: &str,
    agent_block_id: &str,
    swarm_block_id: &str,
    armory_block_id: &str,
    sysinfo_block_id: &str,
) -> Result<(), StoreError> {
    let tab = store.must_get::<Tab>(tab_id)?;
    let (rootnode, focused_node_id, leaforder) =
        default_four_pane_tree(agent_block_id, swarm_block_id, armory_block_id, sysinfo_block_id);
    let mut layout = store.must_get::<LayoutState>(&tab.layoutstate)?;
    layout.rootnode = Some(rootnode);
    layout.focusednodeid = focused_node_id;
    layout.leaforder = Some(leaforder);
    layout.pendingbackendactions = None;
    store.update(&mut layout)?;
    Ok(())
}

/// Pure builder for the default 4-pane tree (`agent | [swarm / armory /
/// sysinfo]`). Returns `(rootnode, focused_node_id, leaforder)`. Shared by
/// the pre-bootstrap store-direct writer above (first launch — reducer not
/// hydrated yet) and the post-bootstrap reducer-routed CreateWindow seed
/// (SPEC_864 Phase 3, `seed_layout_via_reducer`), so the layout shape can
/// never drift between the two paths.
pub(crate) fn default_four_pane_tree(
    agent_block_id: &str,
    swarm_block_id: &str,
    armory_block_id: &str,
    sysinfo_block_id: &str,
) -> (LayoutNode, String, Vec<LeafOrderEntry>) {
    // Node IDs for each tree position. Leaves get their own node IDs distinct
    // from the block IDs they wrap.
    let agent_node_id = Uuid::new_v4().to_string();
    let swarm_node_id = Uuid::new_v4().to_string();
    let armory_node_id = Uuid::new_v4().to_string();
    let sysinfo_node_id = Uuid::new_v4().to_string();
    let right_col_id = Uuid::new_v4().to_string();
    let root_id = Uuid::new_v4().to_string();

    // Phase E.4.B Phase 2 — typed LayoutNode (was inline JSON).
    let rootnode = LayoutNode {
        id: root_id,
        flex_direction: FlexDirection::Row,
        size: 10.0,
        data: None,
        children: vec![
            LayoutNode {
                id: agent_node_id.clone(),
                flex_direction: FlexDirection::Column,
                size: 5.0,
                children: Vec::new(),
                data: Some(LayoutNodeData {
                    block_id: agent_block_id.to_string(),
                    ..Default::default()
                }),
                ..Default::default()
            },
            LayoutNode {
                id: right_col_id,
                flex_direction: FlexDirection::Column,
                size: 5.0,
                children: vec![
                    LayoutNode {
                        id: swarm_node_id.clone(),
                        flex_direction: FlexDirection::Row,
                        size: 4.0,
                        children: Vec::new(),
                        data: Some(LayoutNodeData {
                            block_id: swarm_block_id.to_string(),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    LayoutNode {
                        id: armory_node_id.clone(),
                        flex_direction: FlexDirection::Row,
                        size: 4.0,
                        children: Vec::new(),
                        data: Some(LayoutNodeData {
                            block_id: armory_block_id.to_string(),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    LayoutNode {
                        id: sysinfo_node_id.clone(),
                        flex_direction: FlexDirection::Row,
                        size: 2.0,
                        children: Vec::new(),
                        data: Some(LayoutNodeData {
                            block_id: sysinfo_block_id.to_string(),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                ],
                data: None,
                ..Default::default()
            },
        ],
        ..Default::default()
    };

    let leaforder = vec![
        LeafOrderEntry { nodeid: agent_node_id.clone(), blockid: agent_block_id.to_string() },
        LeafOrderEntry { nodeid: swarm_node_id, blockid: swarm_block_id.to_string() },
        LeafOrderEntry { nodeid: armory_node_id, blockid: armory_block_id.to_string() },
        LeafOrderEntry { nodeid: sysinfo_node_id, blockid: sysinfo_block_id.to_string() },
    ];
    (rootnode, agent_node_id, leaforder)
}

/// Get the singleton client record.
pub fn get_client(store: &Store) -> Result<Client, StoreError> {
    let clients = store.get_all::<Client>()?;
    clients.into_iter().next().ok_or(StoreError::NotFound)
}

// ====================================================================
// Tests
// ====================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn make_store() -> Store {
        Store::open_in_memory().unwrap()
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
        // Tab should be named "tab1" (per SPEC_TAB_GAPS_AND_NAMING_2026_04_25 —
        // auto-generated tabs use the `tabN` convention, not the older
        // `Untitled1` name. The test was asserting against stale-spec naming).
        assert_eq!(tabs[0].name, "tab1");
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
        let ws = create_workspace(&store, "Test WS").unwrap();
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
        let ws = create_workspace(&store, "WS").unwrap();
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
        let ws = create_workspace(&store, "WS").unwrap();
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
        let ws = create_workspace(&store, "WS").unwrap();
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
    fn seed_default_layout_creates_four_pane_layout() {
        // Regression: "Open another window" opened blank because new windows
        // never received the default layout (only first launch did). This is
        // the shared primitive both paths now use.
        // See docs/retro/retro-blank-new-window-2026-06-21.md.
        let store = make_store();
        let ws = create_workspace(&store, "WS").unwrap();
        let tab = create_tab(&store, &ws.oid).unwrap();
        assert!(tab.blockids.is_empty(), "fresh tab starts blank");
        let layout = store.must_get::<LayoutState>(&tab.layoutstate).unwrap();
        assert!(layout.rootnode.is_none(), "fresh tab has no layout");

        seed_default_layout(&store, &tab.oid).unwrap();

        // 4 blocks: agent + swarm + armory + sysinfo (SPEC_DEFAULT_WIDGETS_REORDER_2026_08_25.md).
        let tab = store.must_get::<Tab>(&tab.oid).unwrap();
        assert_eq!(tab.blockids.len(), 4, "agent + swarm + armory + sysinfo");
        let views: Vec<String> = tab
            .blockids
            .iter()
            .map(|bid| {
                store
                    .must_get::<Block>(bid)
                    .unwrap()
                    .meta
                    .get("view")
                    .and_then(|v| v.as_str())
                    .unwrap_or_default()
                    .to_string()
            })
            .collect();
        assert!(views.contains(&"agent".to_string()));
        assert!(views.contains(&"swarm".to_string()));
        assert!(views.contains(&"armory".to_string()));
        assert!(views.contains(&"sysinfo".to_string()));

        // Layout populated (the regression was new windows getting rootnode=None).
        let layout = store.must_get::<LayoutState>(&tab.layoutstate).unwrap();
        assert!(layout.rootnode.is_some(), "rootnode must be populated, not blank");
        assert_eq!(layout.leaforder.as_ref().unwrap().len(), 4);
    }

    #[test]
    fn test_create_and_close_window() {
        let store = make_store();
        ensure_initial_data(&store).unwrap();

        let client = get_client(&store).unwrap();
        let initial_count = client.windowids.len();

        let ws = create_workspace(&store, "WS2").unwrap();
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

        let ws = create_workspace(&store, "WS2").unwrap();
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

        let new_ws = create_workspace(&store, "New WS").unwrap();
        switch_workspace(&store, window_id, &new_ws.oid).unwrap();

        let window = store.must_get::<Window>(window_id).unwrap();
        assert_eq!(window.workspaceid, new_ws.oid);
        assert_ne!(window.workspaceid, old_ws);
    }

    #[test]
    fn test_resolve_block_id_prefix() {
        let store = make_store();
        let ws = create_workspace(&store, "WS").unwrap();
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
        create_workspace(&store, "WS1").unwrap();
        create_workspace(&store, "WS2").unwrap();
        create_workspace(&store, "WS3").unwrap();

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
        let ws = create_workspace(&store, "WS").unwrap();
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
        let ws = create_workspace(&store, "WS").unwrap();
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
        let ws = create_workspace(&store, "WS").unwrap();
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
        let ws = create_workspace(&store, "WS").unwrap();
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
        let ws = create_workspace(&store, "WS").unwrap();
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
        let ws1 = create_workspace(&store, "WS1").unwrap();
        let ws2 = create_workspace(&store, "WS2").unwrap();
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
        let ws1 = create_workspace(&store, "WS1").unwrap();
        let ws2 = create_workspace(&store, "WS2").unwrap();
        let tab1 = create_tab(&store, &ws1.oid).unwrap();
        let _tab2 = create_tab(&store, &ws2.oid).unwrap();

        // Should fail — can't move the only tab out
        let result = move_tab_to_workspace(&store, &tab1.oid, &ws1.oid, &ws2.oid, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_tear_off_tab() {
        let store = make_store();
        let ws = create_workspace(&store, "WS").unwrap();
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
        let ws = create_workspace(&store, "WS").unwrap();
        let tab1 = create_tab(&store, &ws.oid).unwrap();

        // Should fail — can't tear off the only tab
        let result = tear_off_tab(&store, &tab1.oid, &ws.oid);
        assert!(result.is_err());
    }
}
