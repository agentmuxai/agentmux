// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Window CRUD, focus, workspace switching, and repair operations.

use uuid::Uuid;

use crate::backend::storage::wstore::WaveStore;
use crate::backend::storage::StoreError;
use crate::backend::obj::*;

use super::{get_client, WORKSPACE_COLORS, WORKSPACE_ICONS};
use super::tab::create_tab;
use super::workspace::create_workspace;

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
pub(super) fn check_and_fix_window(store: &WaveStore, window_id: &str) -> Result<(), StoreError> {
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
