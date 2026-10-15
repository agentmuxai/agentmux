// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Bus-lag resync: after `RecvError::Lagged` the dropped events are gone, so the
// reducer-owned columns of every workspace, tab, block, window and layout row
// are rewritten from a reducer snapshot. Insert and update only, never delete
// (#615): a row whose delete was dropped stays on disk, unlinked from its parent.
// Not rebuilt, as the reducer doesn't hold them: meta, a layout's leaforder and
// action queue, and a block row whose `BlockCreated` was dropped. Events still in
// the channel are applied afterwards; every apply is idempotent, so they converge.

use std::sync::Arc;

use tokio::sync::Mutex;

use crate::backend::obj::{Block, LayoutState as PersistedLayoutState, Tab, Workspace};
use crate::backend::storage::store::Store;
use crate::state::{BlockRecord, State, TabRecord, WindowRecord, WorkspaceRecord};

type ResyncResult = Result<(), Box<dyn std::error::Error>>;

/// What the resync copies out of the reducer, taken under its lock.
struct Snapshot {
    workspaces: Vec<WorkspaceRecord>,
    tabs: Vec<TabRecord>,
    blocks: Vec<BlockRecord>,
    windows: Vec<WindowRecord>,
}

impl Snapshot {
    fn take(state: &State) -> Self {
        Self {
            workspaces: state.workspaces.values().cloned().collect(),
            tabs: state.tabs.values().cloned().collect(),
            blocks: state.blocks.values().cloned().collect(),
            windows: state.windows.values().cloned().collect(),
        }
    }
}

/// Rewrite SQLite's reducer-owned rows from the reducer. Keeps going past a
/// failed row so one bad row doesn't leave the rest stale; returns the first
/// error after logging each.
pub(super) async fn resync_from_reducer(state: &Arc<Mutex<State>>, mstore: &Store) -> ResyncResult {
    // Clone under the lock; release it before any I/O.
    let snapshot = Snapshot::take(&*state.lock().await);
    let mut first_err: Option<Box<dyn std::error::Error>> = None;
    let mut note = |what: &str, id: &str, r: ResyncResult| {
        if let Err(e) = r {
            tracing::warn!(
                target: "srv-persist-subscriber",
                "[srv-persist-subscriber] resync: {} {} failed: {}",
                what,
                id,
                e
            );
            first_err.get_or_insert(e);
        }
    };

    // Parents before children, so the create helpers can link each new row.
    for ws in &snapshot.workspaces {
        note("workspace insert", &ws.workspace_id, super::apply_workspace_created(mstore, &ws.workspace_id, &ws.name));
    }
    for tab in &snapshot.tabs {
        note("tab", &tab.tab_id, resync_tab(mstore, tab));
    }
    for ws in &snapshot.workspaces {
        note("workspace", &ws.workspace_id, resync_workspace(mstore, ws));
    }
    for block in &snapshot.blocks {
        note("block", &block.block_id, resync_block_parent(mstore, block));
    }
    for window in &snapshot.windows {
        note(
            "window",
            &window.window_id,
            super::apply_srv_window_opened(mstore, &window.window_id, &window.workspace_id),
        );
    }
    match first_err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Name, ordered `tabids` and `activetabid`. Tabs the reducer lists are
/// drained from the legacy `pinnedtabids`, as `TabsReorderedBulk` does.
fn resync_workspace(mstore: &Store, rec: &WorkspaceRecord) -> ResyncResult {
    let Some(mut ws) = mstore.get::<Workspace>(&rec.workspace_id)? else {
        return Ok(());
    };
    let pinned: Vec<String> = ws
        .pinnedtabids
        .iter()
        .filter(|id| !rec.tab_ids.contains(id))
        .cloned()
        .collect();
    let active = rec.active_tab_id.clone().unwrap_or_default();
    if ws.name == rec.name && ws.tabids == rec.tab_ids && ws.pinnedtabids == pinned && ws.activetabid == active {
        return Ok(());
    }
    ws.name = rec.name.clone();
    ws.tabids = rec.tab_ids.clone();
    ws.pinnedtabids = pinned;
    ws.activetabid = active;
    mstore.update(&mut ws)?;
    Ok(())
}

/// The tab row (inserted with its layout row if missing), its name and
/// `blockids`, and its layout row's tree, focus and magnify.
fn resync_tab(mstore: &Store, rec: &TabRecord) -> ResyncResult {
    super::apply_tab_created(mstore, &rec.workspace_id, &rec.tab_id, &rec.name)?;
    let Some(mut tab) = mstore.get::<Tab>(&rec.tab_id)? else {
        return Ok(());
    };
    // A block row the store never got can't be linked: its meta is unknown.
    let mut block_ids = Vec::with_capacity(rec.block_ids.len());
    for id in &rec.block_ids {
        if mstore.get::<Block>(id)?.is_some() {
            block_ids.push(id.clone());
        }
    }
    if tab.name != rec.name || tab.blockids != block_ids {
        tab.name = rec.name.clone();
        tab.blockids = block_ids;
        mstore.update(&mut tab)?;
    }
    if tab.layoutstate.is_empty() {
        return Ok(());
    }
    let Some(mut layout) = mstore.get::<PersistedLayoutState>(&tab.layoutstate)? else {
        return Ok(());
    };
    // An empty tree clears leaforder too, as `LayoutTreeReplaced` does.
    let leaforder = if rec.rootnode.is_none() { None } else { layout.leaforder.clone() };
    if layout.rootnode == rec.rootnode
        && layout.focusednodeid == rec.focused_node_id
        && layout.magnifiednodeid == rec.magnified_node_id
        && layout.leaforder == leaforder
    {
        return Ok(());
    }
    layout.rootnode = rec.rootnode.clone();
    layout.focusednodeid = rec.focused_node_id.clone();
    layout.magnifiednodeid = rec.magnified_node_id.clone();
    layout.leaforder = leaforder;
    mstore.update(&mut layout)?;
    Ok(())
}

/// A block row's parent, after a dropped `BlockMoved`.
fn resync_block_parent(mstore: &Store, rec: &BlockRecord) -> ResyncResult {
    let Some(mut block) = mstore.get::<Block>(&rec.block_id)? else {
        return Ok(());
    };
    let parent = format!("tab:{}", rec.tab_id);
    if block.parentoref != parent {
        block.parentoref = parent;
        mstore.update(&mut block)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
