// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase E.1b — srv reducer state. Mirrors agentmux-launcher's State
// pattern: plain data, mutated only by the pure reducer
// (`crate::reducer::update`). Held inside Arc<Mutex<State>> by the
// pipe IPC server; mutex held only during reducer dispatch
// (sub-millisecond).
//
// Holds lifecycle, processes, workspaces, tabs (with their layout tree
// and focus/magnify), blocks, and the window→workspace map. SQLite is a
// projection: `persist::bootstrap_state_from_mstore` loads it at startup
// and the persist subscriber writes each reducer event back.

use std::collections::HashMap;

pub use agentmux_common::ipc::LifecyclePhase;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    Running,
    Exited { code: i32 },
}

#[derive(Debug, Clone)]
pub struct ProcessRecord {
    pub state: ProcessState,
    #[cfg(test)]
    pub version: String,
}

/// Phase E.2 — workspace as held by the srv reducer's canonical
/// state. Mirrors the persistent `Workspace` struct in
/// `agentmux_srv::backend::obj::Workspace` but with the reducer-
/// canonical fields the cross-process events care about.
///
/// Phase E.2b extends the record with `tab_ids` (ordered) and
/// `active_tab_id`. Tabs themselves live in `state.tabs` keyed by
/// tab_id; the workspace owns the ordering.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorkspaceRecord {
    pub workspace_id: String,
    pub name: String,
    /// Ordered list of tab ids in this workspace. Mirrors the
    /// persistent `Workspace.tabids` field.
    pub tab_ids: Vec<String>,
    /// The active tab in this workspace, if any. `None` when the
    /// workspace has no tabs or has not yet had one selected.
    pub active_tab_id: Option<String>,
}

/// Phase E.2b — tab as held by the srv reducer's canonical state.
/// Tabs are owned by exactly one workspace; the workspace's
/// `tab_ids` field gives the ordering. E.3 adds `block_ids` so the
/// tab tracks which blocks live inside it.
// `Eq` dropped in Phase E.4.B because `LayoutNode.size: f32` precludes
// it. Nothing in the codebase relies on `TabRecord: Eq` (no HashSet
// usage, no `==` comparisons in the reducer).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TabRecord {
    pub tab_id: String,
    pub workspace_id: String,
    pub name: String,
    /// Phase E.3 — ordered list of block ids in this tab. Mirrors
    /// the persistent `Tab.blockids` field.
    pub block_ids: Vec<String>,
    /// Focused layout node id; empty when no pane in this tab is
    /// focused. Mirrors `LayoutState.focusednodeid`.
    pub focused_node_id: String,
    /// Phase E.4 (Option A) — magnified layout node id. Empty when
    /// no pane is magnified (toggle-off). Mirrors
    /// `LayoutState.magnifiednodeid`.
    pub magnified_node_id: String,
    /// Layout tree root; `None` is an empty tree (no panes). The
    /// reducer is its writer (`LayoutSetTree`, `LayoutDeleteNode*`, the
    /// pane-tab stack arms); `LayoutState.rootnode` is persisted from
    /// their events and bootstrap-loaded at startup.
    pub rootnode: Option<agentmux_common::LayoutNode>,
}

/// Phase E.3 — block as held by the srv reducer's canonical state.
/// Blocks are owned by exactly one tab; the tab's `block_ids`
/// field gives the ordering. Block content (view, meta, runtimeopts)
/// is intentionally not yet tracked — E.3 ships block lifecycle
/// only; metadata + view land in a follow-up.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlockRecord {
    pub block_id: String,
    pub tab_id: String,
}

/// Phase E.5 — window-to-workspace mapping as held by the srv
/// reducer's canonical state. Mirrors the persistent
/// `Window.workspaceid` field. Used by sagas (TearOff/Restore/
/// CreateWindow/CloseWindow) that need to coordinate the
/// window↔workspace lifecycle atomically.
///
/// Note: this is NOT the same as the launcher's `state::Window` —
/// the launcher tracks CEF window ownership (label, kind, hwnd).
/// The srv `WindowRecord` is purely "which workspace does this
/// window currently point at." Both are valid orthogonal projections
/// of the same on-disk Window row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowRecord {
    pub window_id: String,
    pub workspace_id: String,
}

#[derive(Debug)]
pub struct State {
    pub lifecycle: LifecyclePhase,
    pub processes: HashMap<u32, ProcessRecord>,
    pub event_version: u64,
    pub next_client_id: u64,
    /// Workspaces canonical to the srv reducer. Bootstrapped from
    /// SQLite at startup; subsequent transitions flow through `update`
    /// and are written back by the persist subscriber.
    pub workspaces: HashMap<String, WorkspaceRecord>,
    /// Phase E.2b — tabs canonical to the srv reducer. Keyed by
    /// `tab_id`; ordering within a workspace is held in
    /// `WorkspaceRecord.tab_ids`. Bootstrap-loaded from SQLite at
    /// startup alongside workspaces.
    pub tabs: HashMap<String, TabRecord>,
    /// Phase E.3 — blocks canonical to the srv reducer. Keyed by
    /// `block_id`; ordering within a tab is held in
    /// `TabRecord.block_ids`. Bootstrap-loaded from SQLite at startup
    /// alongside workspaces and tabs.
    pub blocks: HashMap<String, BlockRecord>,
    /// Phase E.5 — window→workspace mapping. Bootstrap-loaded from
    /// SQLite Window rows. Mutated by the saga-driven CreateWindow/
    /// CloseWindow/SwitchWorkspace commands; sagas use it to keep
    /// window+workspace lifecycle coherent.
    pub windows: HashMap<String, WindowRecord>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            lifecycle: LifecyclePhase::Starting,
            processes: HashMap::new(),
            event_version: 0,
            next_client_id: 0,
            workspaces: HashMap::new(),
            tabs: HashMap::new(),
            blocks: HashMap::new(),
            windows: HashMap::new(),
        }
    }
}

impl State {
    /// Increment + return the monotonic event-version counter.
    /// Every emitted Event carries a version; subscribers detect
    /// gaps for resync (Phase D.3).
    pub fn bump_version(&mut self) -> u64 {
        self.event_version = self.event_version.saturating_add(1);
        self.event_version
    }

    /// Allocate a fresh client_id for a new Register reply.
    pub fn alloc_client_id(&mut self) -> u64 {
        self.next_client_id = self.next_client_id.saturating_add(1);
        self.next_client_id
    }
}
