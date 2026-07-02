// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Agent-facing introspection: `AgentContext` resolution plus the read-only
//! window → workspace → tab → pane snapshots that back `/api/v1/self`,
//! `/api/v1/layout`, and the naming verbs. Pure wstore reads — no reducer.

use serde_json::json;

use crate::backend::obj::*;
use crate::backend::storage::store::Store;

/// The agent pane's place in the object tree, resolved from its block id.
/// Powers `GET /api/v1/self` and lets naming verbs default to "my own X".
#[derive(Debug, serde::Serialize)]
pub(crate) struct AgentContext {
    pub block_id: String,
    pub block_title: String,
    pub tab_id: String,
    pub tab_name: String,
    pub window_id: Option<String>,
    pub window_name: String,
    pub workspace_id: Option<String>,
    pub workspace_name: String,
}

/// Find the id of the workspace that owns `tab_id` (in `tabids` or
/// `pinnedtabids`), or `None` if no workspace references it.
pub(crate) fn workspace_id_for_tab(store: &Store, tab_id: &str) -> Option<String> {
    store
        .get_all::<Workspace>()
        .unwrap_or_default()
        .into_iter()
        .find(|w| {
            w.tabids.iter().any(|t| t == tab_id) || w.pinnedtabids.iter().any(|t| t == tab_id)
        })
        .map(|w| w.oid)
}

/// Read-only snapshot of the window → workspace → tab → pane tree, for agent
/// introspection (`GET /api/v1/layout`). Pure wstore reads — no reducer, so
/// it's hermetic and safe. Lookups use linear scans (a handful of objects).
pub(crate) fn agent_layout(store: &Store) -> serde_json::Value {
    let windows = store.get_all::<Window>().unwrap_or_default();
    let workspaces = store.get_all::<Workspace>().unwrap_or_default();
    let tabs = store.get_all::<Tab>().unwrap_or_default();
    let blocks = store.get_all::<Block>().unwrap_or_default();

    let panes_of = |tab: &Tab| -> Vec<serde_json::Value> {
        tab.blockids
            .iter()
            .filter_map(|bid| blocks.iter().find(|b| &b.oid == bid))
            .map(|b| {
                json!({
                    "block_id": b.oid,
                    "view": meta_get_string(&b.meta, "view", ""),
                    "title": meta_get_string(&b.meta, "frame:title", ""),
                })
            })
            .collect()
    };

    let windows_json: Vec<serde_json::Value> = windows
        .iter()
        .map(|w| {
            let ws = workspaces.iter().find(|x| x.oid == w.workspaceid);
            let tabs_json: Vec<serde_json::Value> = ws
                .map(|ws| {
                    // Pinned tabs render first, then the regular tab order.
                    ws.pinnedtabids
                        .iter()
                        .chain(ws.tabids.iter())
                        .filter_map(|tid| tabs.iter().find(|t| &t.oid == tid))
                        .map(|t| {
                            json!({
                                "tab_id": t.oid,
                                "name": t.name,
                                "active": ws.activetabid == t.oid,
                                "panes": panes_of(t),
                            })
                        })
                        .collect()
                })
                .unwrap_or_default();
            json!({
                "window_id": w.oid,
                "name": meta_get_string(&w.meta, "window:displayname", ""),
                "workspace_id": w.workspaceid,
                "workspace_name": ws.map(|x| x.name.clone()).unwrap_or_default(),
                "tabs": tabs_json,
            })
        })
        .collect();

    json!({ "windows": windows_json })
}

/// Flat list of all windows (id, display name, assigned workspace).
pub(crate) fn agent_windows(store: &Store) -> serde_json::Value {
    let workspaces = store.get_all::<Workspace>().unwrap_or_default();
    let windows: Vec<serde_json::Value> = store
        .get_all::<Window>()
        .unwrap_or_default()
        .iter()
        .map(|w| {
            let ws_name = workspaces
                .iter()
                .find(|x| x.oid == w.workspaceid)
                .map(|x| x.name.clone())
                .unwrap_or_default();
            json!({
                "window_id": w.oid,
                "name": meta_get_string(&w.meta, "window:displayname", ""),
                "workspace_id": w.workspaceid,
                "workspace_name": ws_name,
            })
        })
        .collect();
    json!({ "windows": windows })
}

/// Flat list of all workspaces (id, name, tab count, active tab).
pub(crate) fn agent_workspaces(store: &Store) -> serde_json::Value {
    let workspaces: Vec<serde_json::Value> = store
        .get_all::<Workspace>()
        .unwrap_or_default()
        .iter()
        .map(|w| {
            json!({
                "workspace_id": w.oid,
                "name": w.name,
                "tab_count": w.tabids.len() + w.pinnedtabids.len(),
                "active_tab_id": w.activetabid,
            })
        })
        .collect();
    json!({ "workspaces": workspaces })
}

/// Flat list of tabs (id, name, pane count). When `workspace_id` is given,
/// only that workspace's tabs are returned.
pub(crate) fn agent_tabs(store: &Store, workspace_id: Option<&str>) -> serde_json::Value {
    let workspaces = store.get_all::<Workspace>().unwrap_or_default();
    let scope: Option<Vec<String>> = workspace_id.map(|wid| {
        workspaces
            .iter()
            .find(|w| w.oid == wid)
            .map(|w| w.pinnedtabids.iter().chain(w.tabids.iter()).cloned().collect())
            .unwrap_or_default()
    });
    let tabs: Vec<serde_json::Value> = store
        .get_all::<Tab>()
        .unwrap_or_default()
        .iter()
        .filter(|t| scope.as_ref().map(|ids| ids.contains(&t.oid)).unwrap_or(true))
        .map(|t| {
            json!({
                "tab_id": t.oid,
                "name": t.name,
                "pane_count": t.blockids.len(),
            })
        })
        .collect();
    json!({ "tabs": tabs })
}

/// Walk block → tab → workspace → window from an agent pane's block id.
///
/// Tabs carry no parent reference, so the workspace and window are found by
/// reverse lookup: the workspace whose `tabids`/`pinnedtabids` contains the
/// tab, then the window assigned that workspace. `window_id`/`workspace_id`
/// are `None` when the tab isn't attached to a live window (e.g. a torn-off
/// tab mid-transition) — callers should treat that as "no window to name".
pub(crate) fn resolve_agent_context(store: &Store, block_id: &str) -> Result<AgentContext, String> {
    let block = store
        .get::<Block>(block_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("block not found: {block_id}"))?;
    let block_title = meta_get_string(&block.meta, "frame:title", "");
    let tab_id = block.parentoref.strip_prefix("tab:").unwrap_or("").to_string();
    let tab = store
        .get::<Tab>(&tab_id)
        .map_err(|e| e.to_string())?
        .ok_or_else(|| format!("tab not found for block {block_id}"))?;

    let mut workspace_id = None;
    let mut workspace_name = String::new();
    let mut window_id = None;
    let mut window_name = String::new();

    if let Ok(workspaces) = store.get_all::<Workspace>() {
        if let Some(ws) = workspaces
            .into_iter()
            .find(|w| w.tabids.contains(&tab_id) || w.pinnedtabids.contains(&tab_id))
        {
            workspace_name = ws.name.clone();
            workspace_id = Some(ws.oid.clone());
            if let Ok(windows) = store.get_all::<Window>() {
                if let Some(win) = windows.into_iter().find(|w| w.workspaceid == ws.oid) {
                    window_name = meta_get_string(&win.meta, "window:displayname", "");
                    window_id = Some(win.oid);
                }
            }
        }
    }

    Ok(AgentContext {
        block_id: block_id.to_string(),
        block_title,
        tab_id,
        tab_name: tab.name,
        window_id,
        window_name,
        workspace_id,
        workspace_name,
    })
}

#[cfg(test)]
mod agent_context_tests {
    use super::{resolve_agent_context, workspace_id_for_tab};
    use crate::backend::obj::Tab;
    use crate::backend::storage::store::Store;
    use crate::backend::wcore;

    #[test]
    fn workspace_id_for_tab_finds_owner_and_misses_cleanly() {
        let store = Store::open_in_memory().unwrap();
        wcore::ensure_initial_data(&store).unwrap();
        let tab = store
            .get_all::<Tab>()
            .unwrap()
            .into_iter()
            .next()
            .expect("seeded tab");
        assert!(workspace_id_for_tab(&store, &tab.oid).is_some(), "owner found");
        assert!(workspace_id_for_tab(&store, "nope").is_none(), "miss is None");
    }

    // `ensure_initial_data` seeds one workspace ("Starter workspace") with a
    // window and an initial tab holding a default agent block. The resolver
    // must walk that agent block back up to its tab, workspace, and window.
    #[test]
    fn resolves_block_to_tab_workspace_and_window() {
        let store = Store::open_in_memory().unwrap();
        wcore::ensure_initial_data(&store).unwrap();

        let tab = store
            .get_all::<Tab>()
            .unwrap()
            .into_iter()
            .next()
            .expect("seeded tab");
        let block_id = tab.blockids.first().expect("seeded agent block").clone();

        let ctx = resolve_agent_context(&store, &block_id).expect("resolves context");
        assert_eq!(ctx.tab_id, tab.oid);
        assert_eq!(ctx.workspace_name, "Starter workspace");
        assert!(ctx.workspace_id.is_some(), "workspace should resolve");
        assert!(ctx.window_id.is_some(), "window should resolve via reverse lookup");
    }

    #[test]
    fn unknown_block_errors() {
        let store = Store::open_in_memory().unwrap();
        wcore::ensure_initial_data(&store).unwrap();
        let err = resolve_agent_context(&store, "does-not-exist").unwrap_err();
        assert!(err.contains("block not found"), "unexpected error: {err}");
    }
}
