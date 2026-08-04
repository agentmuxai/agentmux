// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase E.1b — srv reducer state. Mirrors agentmux-launcher's State
// pattern: plain data, mutated only by the pure reducer
// (`crate::reducer::update`). Held inside Arc<Mutex<State>> by the
// pipe IPC server; mutex held only during reducer dispatch
// (sub-millisecond).
//
// What's here:
//   * `LifecyclePhase` (re-exported from agentmux-common::ipc) — E.1b
//   * `ProcessRecord` — pid, kind, state, spawned_at — E.1b
//   * `ProcessState` — Running / Exited — E.1b
//   * `WorkspaceRecord` — workspace_id, name — E.2
//   * `State` — top-level: lifecycle + process map + workspaces +
//     monotonic counters
//
// What's intentionally NOT here yet:
//   * Tab / Block / Layout domain state — E.2b+
//   * `persistence_hwm` field — E.2c (lands with the persist
//     subscriber that mirrors pipe-event effects back to SQLite)

use std::collections::HashMap;

use agentmux_common::ipc::ClientKind;
pub use agentmux_common::ipc::LifecyclePhase;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    Running,
    Exited { code: i32 },
}

#[derive(Debug, Clone)]
pub struct ProcessRecord {
    pub pid: u32,
    pub kind: ClientKind,
    pub state: ProcessState,
    pub spawned_at: String,
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
    /// Phase E.4 (Option A) — focused layout node id. Empty when no
    /// pane in this tab is focused. Mirrors
    /// `LayoutState.focusednodeid` for the tab's layout row. Mutated
    /// by `Command::SetFocusedNode` and bootstrap-loaded from the
    /// LayoutState row at startup. The remaining LayoutState fields
    /// (rootnode/leaforder/pendingbackendactions) stay on the
    /// existing wcore-direct path until Option B.
    pub focused_node_id: String,
    /// Phase E.4 (Option A) — magnified layout node id. Empty when
    /// no pane is magnified (toggle-off). Mirrors
    /// `LayoutState.magnifiednodeid`.
    pub magnified_node_id: String,
    /// Phase E.4.B (Option B) — layout tree root.
    ///
    /// Mirrors the persisted `LayoutState.rootnode`. Mutated by the
    /// `LayoutClear` / `LayoutSetTree` / `LayoutInsertNode` /
    /// `LayoutDeleteNode` reducer arms (and the rest of the 11 in
    /// follow-up PRs). `None` represents an empty tree (no panes).
    ///
    /// **Status: scaffolded; bootstrap-loaded.** `persist::
    /// bootstrap_state_from_wstore` populates this from
    /// `LayoutState.rootnode` at startup. Production writers still
    /// go through the wcore-direct path (per
    /// `srv-phase-e4b-implementation-plan-2026-05-03.md` Phase 7);
    /// reducer arms mutate this field but no production code
    /// dispatches to them yet — same "no-callers-yet" discipline H.6
    /// follows in the host reducer.
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
    /// Phase E.2 — workspaces canonical to the srv reducer.
    /// Bootstrapped from SQLite at startup; subsequent transitions
    /// flow through `update`. In E.2 the reducer is a session-only
    /// projection — pipe-originated mutations live only in this
    /// map until the process restarts. E.2c adds the persist
    /// subscriber that mirrors changes back to SQLite (idempotent,
    /// version-gated) and migrates HTTP/WS RPC through the reducer.
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
    // `persistence_hwm` deferred to E.2c when the persist subscriber
    // lands and there's actually something to track.
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
