// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Phase B.2 IPC wire protocol — shared between agentmux-launcher
//! (server) and agentmux-cef (client). One source of truth so the
//! Command / Event shapes can't drift between binaries on a
//! version-skew release.
//!
//! See `docs/specs/SPEC_WINDOW_PROCESS_STATE_MACHINE_2026_04_27.md` §5.
//!
//! Wire format: newline-delimited JSON. One message per line,
//! parsed via serde_json. Format chosen for debuggability —
//! operators can `cat` / `nc` the named pipe and read traffic
//! without a binary protocol decoder.
//!
//! Backward compat policy (B.2 baseline; harden in Phase D):
//!   * Externally tagged enums (`{"cmd":"register",...}`) so adding
//!     variants is non-breaking; clients send what they know.
//!   * Unknown commands → server replies `Event::Error` rather than
//!     crashing.
//!   * `version: u64` on every Event lets Phase D's GetSnapshot /
//!     resync detect skew. For B.2 it's set but not enforced.

use serde::{Deserialize, Serialize};

/// Stable identifier for a connected client. Tagged so the launcher
/// can route replies + log who said what.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ClientKind {
    /// The CEF host process (one per launcher run).
    Host,
    /// A frontend renderer (proxied via the host's CEF JS bridge — Phase B.7).
    Renderer,
    /// The agentmux-srv backend (proxy connection used for Quit ack
    /// + process-tree facts; the workspace data path stays on
    /// HTTP/WS through host).
    Srv,
    /// External tooling (`agentmux.exe --diag` etc.).
    Tool,
}

/// Commands flow client → launcher.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "cmd", rename_all = "snake_case")]
pub enum Command {
    /// Identifies the connection. MUST be the first command on every
    /// new connection. Server enforces.
    Register {
        kind: ClientKind,
        /// PID of the connecting process — used for cross-checking
        /// against the launcher's `ProcessRecord` map and for
        /// log correlation.
        pid: u32,
        /// Free-form version string of the client binary, for log
        /// correlation across version skew.
        version: String,
    },
    /// Health probe — server replies with `Event::Pong` carrying the
    /// same nonce. NOT a polling heartbeat (per spec §4.3) — sent
    /// only on demand by clients that need round-trip confirmation.
    Ping {
        nonce: u64,
    },
    /// SPEC_LAUNCHER_TEARDOWN_BACKSTOP_2026_07_11 Phase 1 — launcher → host
    /// (over the host pipe, like the saga `IssueCmd::Host` commands): probe
    /// whether the host's CEF **UI thread** is actually pumping. The host
    /// replies with `ReportUiThreadAlive { nonce }` from a posted UI task —
    /// NEVER from the pipe-reader thread (a wedged host's tokio reader keeps
    /// answering; only a UI-thread round-trip is liveness evidence). A host
    /// whose UI thread is hung, or not yet pumping (the known pre-ready
    /// `post_task` silent drop), simply never replies — silence is the
    /// signal; there is no host-side timeout.
    ProbeUiThread {
        nonce: u64,
    },
    /// Reply to `ProbeUiThread`, sent host → launcher via the `Report*`
    /// path once the probe's posted UI task actually executes. Echoes the
    /// nonce; the launcher treats ANY receipt as "UI thread pumped after
    /// the matching probe was sent" (staleness bounded by the probe
    /// interval, which is all the Phase-2 rule consumes).
    ReportUiThreadAlive {
        nonce: u64,
    },
    /// Graceful disconnect. Server logs and closes the connection.
    /// In B.3+ this becomes `Quit { reason }` with shutdown semantics;
    /// for B.2 it's just a polite goodbye.
    Goodbye,
    /// Phase B.4: host reports that a real window has been created
    /// (CEF `on_after_created` fired). Launcher records it in its
    /// read-only mirror and broadcasts `Event::WindowOpened` to other
    /// subscribers. Pool windows do NOT report via this command —
    /// they get their own `ReportPool*` commands in a follow-up so
    /// the mirror can distinguish user-visible windows from pool
    /// inventory.
    ReportWindowOpened {
        /// Stable label assigned by the host (e.g. "main", "window-{uuid}").
        label: String,
        kind: WindowKind,
        /// For `Subwindow` only: label of the `FullInstance` parent.
        /// `None` for `FullInstance`.
        parent_label: Option<String>,
    },
    /// Phase B.4: host reports a window is closing (`on_before_close`).
    /// Launcher removes from mirror, broadcasts `Event::WindowClosed`.
    /// Idempotent: a missing label is logged but not an error (covers
    /// the close-before-launcher-saw-the-open race).
    ReportWindowClosed {
        label: String,
    },
    /// Workstream 0 Phase 1 (`SPEC_TRAY_OPTIONAL_BACKGROUND_SERVICE_2026_09_04.md`
    /// §7) — host reports whether background-service mode
    /// (`AGENTMUX_BACKGROUND_SERVICE`) is enabled for this process. Sent
    /// once, right after connecting (mirrors `GetSnapshot`'s
    /// once-per-connect timing). The launcher's own last-window
    /// orphan-drift detection and the teardown backstop it arms must not
    /// treat an intentionally-resting host (zero windows, by design, for
    /// however long the user leaves it) as a stuck/orphaned one — see
    /// PR #2983 review (Codex P2). Host-only.
    ReportBackgroundServiceEnabled {
        enabled: bool,
    },
    /// Phase B.4 follow-up — host reports a pre-warmed pool window
    /// being added (`spawn_pool_window`). Pool windows live in a
    /// SEPARATE map from the user-visible window mirror; the host
    /// transitions them out of the pool with `ReportPoolWindowRemoved`
    /// + `ReportWindowOpened` on promote, or just
    /// `ReportPoolWindowRemoved` on pre-promote destroy.
    ReportPoolWindowAdded {
        label: String,
        /// Phase CPD-1 (cross-process dispatch) — saga correlation
        /// echo. `Some(N)` when the host is replying to a saga-issued
        /// `Command::SpawnPoolWindow { saga_id: N }`; `None` for
        /// organic refills (e.g. host's implicit `spawn_pool_window`
        /// inside `promote_pool_window`). The launcher reducer
        /// passes the value through to `Event::PoolWindowAdded` so
        /// per-saga correlation (CPD-4) can match the response to
        /// the originating saga.
        ///
        /// `#[serde(default)]` for forward-compat with hosts running
        /// pre-CPD-1 builds — they emit no `saga_id` field, which
        /// deserializes as `None` (organic). Removed once CPD-1+CPD-3
        /// have soaked one release cycle.
        #[serde(default)]
        saga_id: Option<u64>,
    },
    /// Phase B.4 follow-up — host reports a pool window leaving the
    /// pool (promote, destroy, or app exit).
    ReportPoolWindowRemoved {
        label: String,
    },
    /// Phase B.4 follow-up — drift detection (full snapshot). Host
    /// sends its own post-mutation counts after each window-level
    /// transition; the launcher reducer compares both dimensions to
    /// its mirror counts and emits `Event::DriftDetected` per
    /// disagreeing dimension. Sent in a separate command (rather
    /// than embedded in each Report*) so the existing wire shapes
    /// stay unchanged.
    ///
    /// Known limitation (B.4 observe-only): emissions originate
    /// from multiple execution contexts (CEF UI thread for
    /// `on_after_created`/`on_before_close`, IPC handler thread
    /// for `promote_pool_window`). Cross-thread interleaving in
    /// the outbound channel can produce a snapshot whose counts
    /// were taken at a moment that doesn't match the channel
    /// order seen by the reducer, occasionally emitting a
    /// transient false `DriftDetected`. Acceptable for B.4
    /// (drift is diagnostic — false positives are ephemeral and
    /// self-correct on the next stable state). B.5 will tighten
    /// with a transition-ID protocol once the launcher is
    /// authoritative. (codex P2 PR #578 round-4 — accepted as
    /// known limitation.)
    ReportHostCounts {
        /// User-visible top-level windows in the host's
        /// authoritative store (`browsers` minus browser-pane
        /// children minus unpromoted pool labels).
        windows: u32,
        /// Pre-promote pool inventory size.
        pool: u32,
    },
    /// Phase B.4 follow-up — pool-dimension-only drift check. Used
    /// by `spawn_pool_window` (pool transitions) where snapshotting
    /// the windows dimension would produce transient false drift:
    /// pool refill is triggered DURING `on_before_close` BEFORE the
    /// matching `ReportWindowClosed` lands, so the host's window
    /// count is mid-flight relative to the launcher mirror. Pool
    /// count IS stable at that moment (the new pool label was just
    /// added), so checking pool alone preserves the "check every
    /// transition" guarantee for the dimension that actually
    /// changed. (codex P2 PR #578 round-3.)
    ReportHostPoolCount {
        count: u32,
    },
    /// Phase B.5 (window_id_map step a) — host reports the
    /// frontend's `register_backend_window` call: a window's label
    /// → backend window ID (a srv-side UUID the frontend resolves
    /// via `WOS.makeORef`). The launcher mirrors it for the same
    /// reasons it mirrors `instance_registry`: host's authoritative
    /// copy will be retired through the a→b→c→d→e ratchet.
    ReportBackendWindowIdRegistered {
        label: String,
        window_id: String,
    },
    /// Phase B.5 (window_id_map step a) — host reports a window
    /// closing, so the launcher should drop the label→window_id
    /// mapping. Sent from the same close path that emits
    /// `ReportWindowClosed`.
    ReportBackendWindowIdUnregistered {
        label: String,
    },
    /// Phase B.9.1 (WRR — Window Reality Reconciliation) — host
    /// reports a Win32 top-level window was created. Hooked off
    /// `EVENT_OBJECT_CREATE` (objid=`OBJID_WINDOW`) via
    /// `SetWinEventHook(WINEVENT_OUTOFCONTEXT)`. The host filters
    /// CEF subprocess HWNDs (renderer, GPU, plugin) at the hook
    /// callback so the launcher only sees plausible app windows.
    /// `label_hint` is the host's best guess from
    /// `pending_window_creations` if it can disambiguate; `None`
    /// when the create event arrives before the host has linked
    /// the HWND back to a label (caught up later by the reducer's
    /// pending_hwnds).
    ReportHwndOpened {
        hwnd: u64,
        class_name: String,
        title: String,
        label_hint: Option<String>,
    },
    /// Phase B.9.1 — host reports a Win32 top-level window was
    /// destroyed. Hooked off `EVENT_OBJECT_DESTROY`.
    ReportHwndDestroyed {
        hwnd: u64,
    },
    /// Phase B.9.1 — `EVENT_OBJECT_SHOW` / `EVENT_OBJECT_HIDE`.
    ReportHwndVisibilityChanged {
        hwnd: u64,
        visible: bool,
    },
    /// Phase B.9.1 — `EVENT_SYSTEM_FOREGROUND`. Tells the launcher
    /// the user actually saw this window (not just opened it).
    /// Used to distinguish "opened but never foregrounded" (drift)
    /// from "opened and shown".
    ReportHwndForegroundChanged {
        hwnd: u64,
    },
    /// Phase B.9.1 — `EVENT_SYSTEM_MINIMIZESTART` /
    /// `EVENT_SYSTEM_MINIMIZEEND`.
    ReportHwndIconicChanged {
        hwnd: u64,
        iconic: bool,
    },
    /// Phase B.9.1 — `WM_WINDOWPOSCHANGED` from the host's wndproc
    /// wrapper. The host coalesces bursts (50ms debounce per HWND)
    /// before sending so the wire stays light during topology
    /// changes. Reducer compares `rect` against `state.monitors` to
    /// classify off-monitor drift.
    ReportHwndPositionChanged {
        hwnd: u64,
        rect: Rect,
    },
    /// Phase B.9.1 — `WM_DISPLAYCHANGE`. Replaces the launcher's
    /// `state.monitors` wholesale; reducer re-evaluates every
    /// known window's last_rect against the new topology and
    /// emits `OffMonitor` drift for any that newly fall off.
    ReportMonitorTopologyChanged {
        rects: Vec<Rect>,
    },
    /// Phase D.1 — request a `Event::Snapshot` reply containing the
    /// reducer's current canonical state. Used by `--diag wrr` for
    /// state-now visibility, by the frontend reducer for mid-session
    /// resync after disconnect, and by future Tool clients that need
    /// to bootstrap without observing every prior event.
    GetSnapshot,
    /// Phase E.1b — srv-side equivalent of `GetSnapshot`. Routed to
    /// the srv pipe; reducer replies with `Event::SrvSnapshot`.
    /// Separate from `GetSnapshot` (launcher) per spec §4.3 — each
    /// reducer is canonical for its domain and replies on its own
    /// pipe.
    ///
    /// Reply contents grow with each phase: E.1b had lifecycle +
    /// version only; E.2 added `workspaces`; E.2b+ will add tabs /
    /// blocks / layouts. See `Event::SrvSnapshot` for the current
    /// shape.
    GetSrvSnapshot,
    /// Phase E.2 — create a new workspace. The reducer assigns the
    /// `oid` (UUID) and emits `Event::WorkspaceCreated`. In E.2 the
    /// reducer is a session-only projection (no persist subscriber);
    /// E.2c adds the persist subscriber + migrates HTTP/WS RPC
    /// to flow through the reducer.
    CreateWorkspace {
        name: String,
    },
    /// Phase E.2 — delete a workspace. Reducer removes from canonical
    /// state and emits `Event::WorkspaceDeleted`. Cascade-to-tabs +
    /// SQLite write happen via wcore today (RPC path); migrating to
    /// reducer-driven persistence is E.2c.
    DeleteWorkspace {
        workspace_id: String,
        /// Step 5 PR 2 — provenance marker for the `delete_workspace`
        /// saga. `true` means the dispatch is being driven by the
        /// saga coordinator (per-tab DeleteTab dispatches already
        /// happened in saga steps; this final cascade is just the
        /// workspace row + window mappings). `false` for legacy /
        /// internal compensation paths (e.g. `tear_off_tab` /
        /// `tear_off_block` rolling back a freshly-created empty
        /// workspace) which keep the existing cascade behaviour.
        ///
        /// The reducer's `handle_delete_workspace` cascades regardless
        /// of `force` — the reducer is a pure mutator and the cascade
        /// must always execute to keep state consistent. The flag is
        /// recorded in the saga log purely for provenance, mirroring
        /// the saga-as-narrator pattern documented in
        /// `docs/retro/phase-fg-roadmap-2026-05-01.md`.
        ///
        /// Defaults to `false` via `#[serde(default)]` so all existing
        /// producers (RPC, internal compensation) keep working.
        #[serde(default)]
        force: bool,
    },
    /// Phase E.2b — create a tab inside an existing workspace. The
    /// reducer assigns the `tab_id` (UUID), appends to the workspace's
    /// ordered tab list, and emits `Event::TabCreated`. Validates the
    /// parent workspace exists; returns `Event::Error` if not.
    /// Session-only projection (no persist subscriber yet).
    CreateTab {
        workspace_id: String,
        name: String,
    },
    /// Phase E.2b — delete a tab from a workspace. Reducer removes the
    /// tab from canonical state, removes its id from the workspace's
    /// ordered tab list, and emits `Event::TabDeleted`. If the deleted
    /// tab was the active tab, also emits `Event::ActiveTabChanged`
    /// pointing at the new active (next-or-prev tab, or empty if the
    /// workspace has no tabs left).
    DeleteTab {
        workspace_id: String,
        tab_id: String,
        /// (codex P2 PR #633 round 4 + codex P1 round 2.) Bypass the
        /// reducer's atomic last-tab guard. `false` for user-facing
        /// flows (close button, keyboard shortcut) — reducer rejects
        /// last-tab deletes to keep workspaces non-empty. `true` for
        /// internal compensation paths (`CreateTab` rollback,
        /// `PromoteBlockToTab` compensation) where rolling back a
        /// just-created tab requires deleting the only tab.
        ///
        /// Defaults to `false` via `#[serde(default)]` for backwards
        /// compatibility — pre-existing producers that don't set the
        /// field get the safe (guarded) behavior.
        #[serde(default)]
        force: bool,
    },
    /// Phase E.2b — set a workspace's active tab. No-op if already
    /// active. Errors if the workspace doesn't exist or the tab isn't
    /// in that workspace's tab list.
    SetActiveTab {
        workspace_id: String,
        tab_id: String,
    },
    /// Phase E.2c.3b — reorder a tab within its workspace's
    /// `tab_ids`. `new_index` is clamped to the list length; no-op
    /// if the tab is already at that position. Errors if the
    /// workspace doesn't exist or the tab isn't in its tab list.
    ReorderTab {
        workspace_id: String,
        tab_id: String,
        new_index: u32,
    },
    /// Phase E.5 — record a new window in the srv reducer's
    /// `state.windows` map. Caller pre-assigns `window_id` (sagas
    /// use a fresh UUID; RPC migration in PR 4 will likewise mint
    /// the id at the RPC boundary). Validates the parent workspace
    /// exists; errors otherwise. Used by CreateWindow + TearOff
    /// sagas to track window↔workspace association.
    CreateWindow {
        window_id: String,
        workspace_id: String,
    },
    /// Phase E.5 — remove a window's workspace mapping from the
    /// reducer. Called by CloseWindow sagas after the host's CEF
    /// window-close completes. Idempotent silent no-op on missing.
    CloseWindowInternal {
        window_id: String,
    },
    /// Phase E.5 — switch which workspace a window points at.
    /// Errors if the window or destination workspace is unknown.
    SwitchWorkspace {
        window_id: String,
        workspace_id: String,
    },
    /// Phase E.5.3 — replace a workspace's `tab_ids` with the given
    /// list. The new list must be a permutation of the current set
    /// (same elements, possibly different order); reducer errors
    /// otherwise. Used by the drag-reorder UI's bulk-reorder path
    /// (replaces wcore-direct `UpdateTabIds`).
    ReorderTabsBulk {
        workspace_id: String,
        tab_ids: Vec<String>,
    },
    /// Phase E.5.3 — rename a workspace. Errors if the workspace
    /// doesn't exist; no-op if the name is identical.
    RenameWorkspace {
        workspace_id: String,
        name: String,
    },
    /// Phase E.5.3 — rename a tab. Errors if the tab doesn't exist;
    /// no-op if identical.
    RenameTab {
        tab_id: String,
        name: String,
    },
    /// Phase E.5.3 — apply a meta-patch to a workspace. The reducer
    /// validates the entity exists and emits `Event::WorkspaceMetaUpdated`
    /// with the patch payload; the persist subscriber performs the
    /// actual merge against wstore. Reducer state does NOT track meta
    /// in E.5.3 — pass-through preserves the reducer's small footprint
    /// without losing the migration property (every mutation goes
    /// through the reducer's broadcast bus).
    UpdateWorkspaceMeta {
        workspace_id: String,
        meta_patch: serde_json::Value,
    },
    /// Phase E.5.3 — apply a meta-patch to a tab. Same pass-through
    /// shape as `UpdateWorkspaceMeta`.
    UpdateTabMeta {
        tab_id: String,
        meta_patch: serde_json::Value,
    },
    /// Phase E.5.3 — apply a meta-patch to a block. Same pass-through
    /// shape as `UpdateWorkspaceMeta`.
    UpdateBlockMeta {
        block_id: String,
        meta_patch: serde_json::Value,
    },
    /// Phase E.5.x — apply a meta-patch to a window's `meta` map.
    /// Same pass-through shape as `UpdateWorkspaceMeta`. Migrated
    /// through the reducer per issue #855 so `Event::WindowMetaUpdated`
    /// lands on `srv_events_tx` and the WaveObjUpdate broadcast bridge
    /// picks it up — replaces the wcore-direct fallback that bypassed
    /// reducer + bridge entirely.
    UpdateWindowMeta {
        window_id: String,
        meta_patch: serde_json::Value,
    },
    /// Phase E.5.5 — move a tab from one workspace to another.
    /// Reducer:
    /// * Removes `tab_id` from `src_workspace_id.tab_ids`.
    /// * Updates `tab.workspace_id = dst_workspace_id`.
    /// * Inserts `tab_id` at `dst_index` in `dst_workspace_id.tab_ids`,
    ///   clamping to the dst list length.
    /// * If `tab_id` was the source workspace's `active_tab_id`, the
    ///   source's active reverts to its first remaining tab (or
    ///   `None` if the source becomes empty).
    /// Errors if any of: source workspace, dest workspace, or tab is
    /// missing; if `tab.workspace_id != src_workspace_id`; or if
    /// the tab is the source workspace's last tab AND no caller has
    /// arranged a fallback (callers like tear-off should reject the
    /// move at the saga layer if removing the tab would empty the
    /// source — preserving the "workspaces have at least one tab"
    /// invariant most UI paths assume). Used by the TearOffTab,
    /// MoveTabToWorkspace, and RestoreTornOffTab sagas.
    MoveTab {
        tab_id: String,
        src_workspace_id: String,
        dst_workspace_id: String,
        dst_index: u32,
    },
    /// Phase E.5.5 — move a block from one tab to another (or to a
    /// different position in the same tab).
    /// Reducer:
    /// * Removes `block_id` from `src_tab_id.block_ids`.
    /// * Updates `block.tab_id = dst_tab_id`.
    /// * Inserts `block_id` at `dst_index` in `dst_tab_id.block_ids`,
    ///   clamping to the dst list length.
    /// Errors if source tab, dest tab, or block is missing, or if
    /// `block.tab_id != src_tab_id`. Used by TearOffBlock and the
    /// MoveBlockToTab saga.
    MoveBlock {
        block_id: String,
        src_tab_id: String,
        dst_tab_id: String,
        dst_index: u32,
    },
    /// Phase E.3 — create a block inside an existing tab. Reducer
    /// validates parent tab exists, assigns the `block_id` (UUID),
    /// appends to the tab's `block_ids`, emits `Event::BlockCreated`.
    /// Session-only projection (no persist subscriber yet).
    CreateBlock {
        tab_id: String,
        /// Phase E.2c.4 — block metadata (`view`, layout hints, etc.)
        /// passed through to the persisted Block row. The reducer
        /// itself doesn't track meta — it forwards it untouched into
        /// `Event::BlockCreated` so the persist subscriber writes
        /// the Block with the correct meta map. `#[serde(default)]`
        /// for forward-compat with old log entries that pre-date the
        /// meta field.
        #[serde(default)]
        meta: serde_json::Value,
    },
    /// Phase E.3 — delete a block from a tab. Idempotent silent no-op
    /// on missing tab or missing block.
    DeleteBlock {
        tab_id: String,
        block_id: String,
    },
    /// Phase E.4 (Option A) — set a tab's `focusednodeid`. Errors if
    /// the tab is unknown to the reducer; no-op short-circuit when the
    /// value is already current. Empty `node_id` clears the field.
    /// Routes through the reducer so the persist subscriber writes the
    /// new value into `LayoutState.focusednodeid` for the tab. The
    /// rest of `LayoutState` (rootnode/leaforder/pendingbackendactions)
    /// keeps its existing wcore-direct path until Option B lands.
    SetFocusedNode {
        tab_id: String,
        node_id: String,
    },
    /// Phase E.4 (Option A) — set a tab's `magnifiednodeid`. Same
    /// shape as `SetFocusedNode`; empty `node_id` clears (toggle-off).
    SetMagnifiedNode {
        tab_id: String,
        node_id: String,
    },

    // ── Phase E.4.B — Layout tree commands ──────────────────────────────
    //
    // Each command carries a `correlation_id` (UUID string) for the
    // frontend optimistic-confirm pattern: the slice-#8 subscriber uses
    // it to distinguish "my own command echoing back" from "a remote
    // command I must apply locally".
    //
    // `focus_after` / `magnify_after` flags mirror the semantics of the
    // frontend's `LayoutTreeActionType` (focused / magnified side effects
    // on InsertNode, SplitH, SplitV, ReplaceNode).
    //
    // See docs/specs/srv-phase-e4b-formal-spec-2026-05-03.md §5.

    /// Insert a new node into the tree. If `parent_id` is `None`, the
    /// heuristic `findNextInsertLocation` is used (first available slot);
    /// `index` positions within the parent's children (None = append).
    LayoutInsertNode {
        tab_id: String,
        node: crate::LayoutNode,
        parent_id: Option<String>,
        index: Option<usize>,
        focus_after: bool,
        magnify_after: bool,
        correlation_id: String,
    },
    /// Insert at an exact index path through the tree (e.g. `[0, 2]`).
    LayoutInsertNodeAtIndex {
        tab_id: String,
        node: crate::LayoutNode,
        index_arr: Vec<usize>,
        focus_after: bool,
        magnify_after: bool,
        correlation_id: String,
    },
    /// Remove a node by id; collapse empty parents.
    LayoutDeleteNode {
        tab_id: String,
        node_id: String,
        correlation_id: String,
    },
    /// Remove the node holding `block_id`; collapse empty parents.
    /// SPEC_864 site #6 — the block→node resolution happens in the
    /// reducer arm (the reducer owns the tree), so callers that only
    /// know a block id (the `delete_block` saga) don't need tree
    /// access. Resolving to no node is a silent idempotent no-op:
    /// blocks may legitimately have no layout node (already pruned by
    /// a frontend push, or never laid out).
    LayoutDeleteNodeByBlock {
        tab_id: String,
        block_id: String,
        correlation_id: String,
    },
    /// Reparent a node to a new parent at the given child index.
    LayoutMoveNode {
        tab_id: String,
        node_id: String,
        new_parent_id: String,
        index: usize,
        correlation_id: String,
    },
    /// Swap two sibling (or cross-parent) nodes. Sizes travel with nodes.
    LayoutSwapNodes {
        tab_id: String,
        node1_id: String,
        node2_id: String,
        correlation_id: String,
    },
    /// Apply N resize operations atomically. Rejected entirely if any
    /// `size` is out of range (reducer validates; early-return on first
    /// invalid op, matching the frontend's existing semantic).
    LayoutResizeNodes {
        tab_id: String,
        ops: Vec<crate::ResizeOp>,
        correlation_id: String,
    },
    /// Replace a node with a new one, preserving the target's flex size.
    LayoutReplaceNode {
        tab_id: String,
        target_id: String,
        new_node: crate::LayoutNode,
        focus_after: bool,
        correlation_id: String,
    },
    /// Horizontal split: inserts `new_node` before/after `target_id`
    /// in a Row parent (or wraps them in a new Row group if parent is not Row).
    LayoutSplitHorizontal {
        tab_id: String,
        target_id: String,
        new_node: crate::LayoutNode,
        position: crate::SplitPosition,
        focus_after: bool,
        correlation_id: String,
    },
    /// Vertical split: inserts `new_node` before/after `target_id`
    /// in a Column parent (or wraps them in a new Column group).
    LayoutSplitVertical {
        tab_id: String,
        target_id: String,
        new_node: crate::LayoutNode,
        position: crate::SplitPosition,
        focus_after: bool,
        correlation_id: String,
    },
    /// Wipe the entire tree (sets rootnode = None, clears focus/magnify).
    LayoutClear {
        tab_id: String,
        correlation_id: String,
    },
    /// Bulk-replace the tree. Used during Phase 7a writer migration and
    /// for tear-off where the whole subtree changes atomically.
    ///
    /// SPEC_864 Phase 2 — when the command originates from the frontend's
    /// full-row `UpdateObject` push, `slices` carries the remaining
    /// frontend-owned `LayoutState` columns (leaforder / focus / magnify /
    /// pendingbackendactions) with REPLACE semantics, so the single
    /// dispatch fully supersedes the legacy `update_raw` whole-row write.
    /// `slices: None` (tree-only callers) leaves those columns untouched.
    LayoutSetTree {
        tab_id: String,
        new_tree: Option<crate::LayoutNode>,
        correlation_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        slices: Option<crate::LayoutClientSlices>,
    },
    /// SPEC_864 Phase 4 — append actions to a tab's
    /// `pendingbackendactions` queue through the reducer. `actions` is a
    /// JSON array of `LayoutActionData` objects (raw-value pass-through;
    /// the reducer does not model the queue — see
    /// `Event::LayoutBackendActionsQueued`).
    LayoutQueueBackendActions {
        tab_id: String,
        actions: serde_json::Value,
        correlation_id: String,
    },

    /// Phase D.3 — request an `Event::EventList` reply containing the
    /// events the launcher has emitted with version > `since`. Used
    /// by subscribers that hold a snapshot at version V and want to
    /// catch up to the live stream by replaying missed events.
    ///
    /// Typical resync flow:
    ///   1. `Register` → `Registered { version: V0 }`
    ///   2. `GetSnapshot` → `Snapshot { version: V1 }` (V1 > V0)
    ///      — apply the snapshot
    ///   3. `GetEvents { since: V1 }` → `EventList { events, version: V2 }`
    ///      — apply replay events to catch up to V2
    ///   4. live broadcast events flow with version > V2
    ///
    /// Replay is best-effort: the launcher's in-memory ring is
    /// bounded; if `since` is older than the oldest retained event,
    /// the reply still contains all retained events but the
    /// subscriber may have missed some. Caller should treat the
    /// result as "everything I have for you" and re-fetch a snapshot
    /// if state inconsistency is detected.
    GetEvents {
        /// Exclusive lower bound — events with `version > since` are
        /// returned, in ascending order.
        since: u64,
    },
    /// Phase F.5 — host explicitly reports that a pool window was
    /// promoted to a user-visible top-level window (the
    /// `promote_pool_window` flow in `agentmux-cef`). Sent BETWEEN the
    /// `ReportPoolWindowRemoved` + `ReportWindowOpened` pair so the
    /// launcher reducer has unambiguous evidence the transition was a
    /// promote (vs a destroy followed by an unrelated open) and can
    /// emit `Event::PoolWindowPromoted`. The pool-respawn saga
    /// consumes the event to bracket the implicit refill in
    /// `SagaStarted`/`SagaCompleted`.
    ///
    /// Host-only — same gate as `ReportPoolWindowRemoved`.
    ReportPoolWindowPromoted {
        label: String,
    },
    /// Phase F.5 — launcher-side saga coordinator asks the host to
    /// spawn a fresh pool window (refill after promote).
    ///
    /// **Status: live.** Cross-process dispatch (CPD-1 through CPD-5)
    /// shipped; the saga coordinator's `apply_action` for
    /// `PipeTarget::Host` writes this command through `host_pipe`
    /// (`agentmux-launcher/src/host_pipe/`) and waits on the
    /// `Event::PoolWindowAdded { saga_id: Some(N) }` echo. Host's
    /// implicit `spawn_pool_window` call inside `promote_pool_window`
    /// remains the organic refill path for non-saga-driven
    /// promotions (with `saga_id: None`).
    ///
    /// `saga_id`: every host-bound command carries the originating
    /// saga's id so the host can echo it on the corresponding
    /// `Report*` reply. `0` is reserved as "no saga" and treated as a
    /// non-saga dispatch. `#[serde(default)]` retained for
    /// forward-compat with any pre-CPD-1 deserializers (no real-world
    /// consumer today; portable runtime is bundled per release).
    SpawnPoolWindow {
        #[serde(default)]
        saga_id: u64,
    },
    /// Phase F.6 — host reports that all browser-pane HWNDs belonging
    /// to a closing top-level window have been reaped. Emitted from
    /// `client.rs::on_before_close` after the subwindow cascade and
    /// pane lifecycle drain finish for the closing window.
    ///
    /// Distinct from `ReportWindowClosed` — that event marks the
    /// CEF browser leaving the host's `browsers` map; this one marks
    /// the host's pane bookkeeping (lifecycle entries, pane HWND map)
    /// for that label being fully drained. Today both happen in the
    /// same `on_before_close` body so the events arrive back-to-back,
    /// but the saga distinguishes them so future fine-grained
    /// reapers (e.g. async pane teardown for embedded browsers) can
    /// land without rewriting the saga.
    ///
    /// Host-only — same gate as `ReportWindowClosed`.
    ReportPanesReaped {
        label: String,
        /// Phase CPD-1 — saga correlation echo. `Some(N)` when the
        /// host is replying to a saga-issued
        /// `Command::ReapPanes { saga_id: N }`; `None` for organic
        /// reports (e.g. host's existing implicit pane drain inside
        /// `on_before_close` that wasn't saga-driven).
        ///
        /// `#[serde(default)]` for forward-compat with pre-CPD-1
        /// hosts.
        #[serde(default)]
        saga_id: Option<u64>,
    },
    /// Phase F.6 — host reports the result of the post-close
    /// drain-pool-if-last decision. `was_last == true` when the
    /// closing window was the last user-visible window and the host
    /// just kicked off the warm-pool drain (Stage 1 of the two-stage
    /// close cascade in `client.rs::on_before_close`); `false` when
    /// other user-visible windows remain and the pool stays warm.
    ///
    /// The launcher's window-cleanup-cascade saga uses this to close
    /// out its bracket regardless of which branch fires (both are
    /// terminal for the saga).
    ///
    /// Host-only.
    ReportPoolDrainDecision {
        label: String,
        was_last: bool,
        /// Phase CPD-1 — saga correlation echo. `Some(N)` when the
        /// host is replying to a saga-issued
        /// `Command::DrainPoolIfLast { saga_id: N }`; `None` for
        /// organic reports.
        ///
        /// `#[serde(default)]` for forward-compat with pre-CPD-1
        /// hosts.
        #[serde(default)]
        saga_id: Option<u64>,
    },
    /// Phase F.6 — launcher-side saga coordinator asks the host to
    /// reap all browser-pane HWNDs for a window that just closed.
    ///
    /// **Status: live.** Wired through `host_pipe` post-CPD-3. The
    /// saga issues this with `target = PipeTarget::Host`; the
    /// coordinator's `apply_action` writes it through the host pipe
    /// and waits for the `Event::PanesReaped { saga_id: Some(N) }`
    /// echo. Host's organic pane drain inside `on_before_close` still
    /// emits the same Report with `saga_id: None` for non-saga-driven
    /// closes.
    ///
    /// `saga_id`: mandatory on the wire (host echoes back on
    /// `ReportPanesReaped`). `#[serde(default)]` retained for
    /// forward-compat (see `SpawnPoolWindow` for rationale).
    ReapPanes {
        label: String,
        #[serde(default)]
        saga_id: u64,
    },
    /// Phase F.6 — launcher-side saga coordinator asks the host to
    /// drain the warm pool if the just-closed window was the last
    /// user-visible window (i.e. trigger Stage 1 of the close
    /// cascade).
    ///
    /// **Status: live** (same shipping path as `ReapPanes`). Wired
    /// through `host_pipe` post-CPD-3; saga waits on
    /// `Event::PoolDrained { saga_id: Some(N) }` /
    /// `Event::PoolNotLast { saga_id: Some(N) }` echo. Host's
    /// `on_before_close` still runs the equivalent inline check
    /// organically (with `saga_id: None`).
    ///
    /// `saga_id`: mandatory on the wire. `#[serde(default)]` retained
    /// for forward-compat.
    DrainPoolIfLast {
        label: String,
        #[serde(default)]
        saga_id: u64,
    },
    /// Phase CPD-1 — host-emitted report that a saga-issued action
    /// failed (e.g. window not found, IPC error). Carries the
    /// originating `saga_id` and a human-readable `reason`. The
    /// launcher reducer translates this into `Event::SagaActionFailed`
    /// so the saga coordinator can terminate the matching saga as
    /// `SagaFailed`.
    ///
    /// Schema-only in CPD-1: hosts don't yet read commands from the
    /// pipe (CPD-2 wires that), so no producer for this command
    /// exists yet. The shape is added now so launcher reducer arms
    /// + saga coordinator wiring can soak before CPD-3 makes the
    /// dispatch live.
    ReportSagaActionFailed {
        saga_id: u64,
        reason: String,
    },
    /// Host reports the start of a named startup phase, forwarded by
    /// the launcher into its `StartupEventSink` so it renders live in
    /// the splash telemetry panel — same stage/label shape as the
    /// launcher's own internal `saga`/`backend`/`host` stages
    /// (`agentmux-launcher/src/startup_events.rs`), just sourced from
    /// inside the host process instead.
    ///
    /// The host can only send this AFTER `connect_to_launcher`
    /// succeeds, so phases that finish before the IPC connection
    /// exists (e.g. the CEF framework `dlopen` on macOS, which
    /// happens before the connection is even opened) can't be
    /// reported live — the host times those with a local `Instant`
    /// and sends both `ReportStartupStageBegin` + `ReportStartupStageEnd`
    /// back-to-back once connected, with the already-elapsed duration
    /// on the `End` message. Phases that start after the connection
    /// exists (CEF `cef::initialize`, first-paint) are reported live.
    ReportStartupStageBegin {
        /// Stable machine-readable stage key (e.g. `"dlopen"`,
        /// `"cef_init"`, `"first_paint"`) — matches
        /// `StartupEvent::StageBegin`'s `stage` field on the launcher
        /// side.
        stage: String,
        /// Human-readable label shown in the splash panel.
        label: String,
    },
    /// Companion to `ReportStartupStageBegin` — see its doc comment.
    ReportStartupStageEnd {
        stage: String,
        duration_ms: u64,
        /// `"ok"` | `"warn"` | `"error"` — kept as a plain string on
        /// the wire rather than importing `agentmux-launcher`'s
        /// `StartupStatus` (which lives in the launcher binary crate,
        /// not a shared crate); the launcher's IPC handler maps it to
        /// `StartupStatus` when forwarding into the sink.
        status: String,
        detail: Option<String>,
    },
}

/// Phase B.9.1 — rectangle in Win32 screen coordinates (pixels).
/// Matches Windows' `RECT` semantics: `right` and `bottom` are one
/// past the last included pixel, so `right - left == width`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Rect {
    pub left: i32,
    pub top: i32,
    pub right: i32,
    pub bottom: i32,
}

/// Window role in the multi-window model — the single definition, used by
/// the host (`agentmux-cef::state` re-exports it) and deserialized by the
/// launcher. The host used to carry a byte-identical private copy and map
/// between the two variant by variant; that copy is gone
/// (`docs/reports/REPORT_DRY_AND_MODULARITY_AUDIT_2026_09_06.md` §2.2).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowKind {
    /// Independent AgentMux window. Appears in the Windows taskbar.
    FullInstance,
    /// Hidden from the taskbar; closes when its parent FullInstance closes.
    Subwindow,
}

/// Events flow launcher → client. Versioned per spec §5.2 — every
/// event carries a monotonic `version: u64` per launcher run, used
/// by Phase D's resync protocol.
///
/// Phase B.3 introduces the first non-handshake events
/// (ProcessSpawned, ProcessExited, LifecyclePhaseChanged) emitted
/// by the launcher's reducer when commands transition state. B.4+
/// adds the window-state events (WindowAdded, WindowStateChanged,
/// WindowRemoved) per spec §5.2.
/// Note: `Eq` is not derived because layout variants carry `LayoutNode`
/// which contains `f32` (not `Eq`). `PartialEq` is sufficient for all
/// current use-sites.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum Event {
    /// Reply to `Command::Register`. Acknowledges the client kind +
    /// confirms the launcher's view of the world.
    Registered {
        client_id: u64,
        launcher_pid: u32,
        launcher_version: String,
        version: u64,
    },
    /// Reply to `Command::Ping`. Echoes the nonce.
    Pong {
        nonce: u64,
        version: u64,
    },
    /// Sent when an incoming command can't be parsed or violates an
    /// invariant (e.g. Command before Register). Connection stays
    /// open unless `fatal: true`.
    Error {
        code: ErrorCode,
        message: String,
        fatal: bool,
        version: u64,
    },
    /// A process joined the launcher's canonical registry. Emitted
    /// when a client first Registers (B.3) and, in B.4+, when the
    /// launcher itself spawns a child.
    ProcessSpawned {
        pid: u32,
        kind: ClientKind,
        client_version: String,
        version: u64,
    },
    /// A process exited or disconnected gracefully. Emitted on
    /// Goodbye (B.3) and, in B.4+, on detected child exit.
    ProcessExited {
        pid: u32,
        /// Exit code. 0 = clean Goodbye; non-zero = OS-reported
        /// exit code or synthetic value for crashes.
        code: i32,
        version: u64,
    },
    /// The launcher's lifecycle phase changed. Spec §4 defines the
    /// valid transitions: Starting → Running → Quitting → Dead.
    /// Emitted at most once per transition.
    LifecyclePhaseChanged {
        from: LifecyclePhase,
        to: LifecyclePhase,
        version: u64,
    },
    /// Phase B.4: a window joined the launcher's mirror. Emitted in
    /// response to `Command::ReportWindowOpened` from the host. Other
    /// subscribers (Tool clients, eventually srv) receive this to
    /// keep their own views consistent.
    WindowOpened {
        label: String,
        kind: WindowKind,
        parent_label: Option<String>,
        version: u64,
    },
    /// Phase B.4: a window left the launcher's mirror. Emitted on
    /// `Command::ReportWindowClosed`. Cascades for FullInstance
    /// closures are NOT modeled here yet (B.5 tightens) — for now
    /// the host emits one ReportWindowClosed per window even on
    /// cascade closes, so subscribers see the same N events.
    WindowClosed {
        label: String,
        version: u64,
        /// (codex P1 PR #637.) `true` when the close was detected by
        /// `wrr::apply_hwnd_destroyed` after a host/renderer crash —
        /// no clean `on_before_close` ran, so the host did NOT send
        /// the `ReportPanesReaped` / `ReportPoolDrainDecision` reports
        /// the F.6 saga waits for. Subscribers that drive
        /// cleanup-cascade sagas must filter on `!crash_detected` to
        /// avoid spawning an in-flight saga that can never reach a
        /// terminal state.
        ///
        /// `#[serde(default)]` so pre-existing producers default to
        /// `false` (clean close).
        #[serde(default)]
        crash_detected: bool,
    },
    /// Phase B.4 follow-up — pool inventory transitioned. Emitted in
    /// response to `ReportPoolWindow{Added,Removed}`. Subscribers
    /// (Tool clients) use this to track pool warmth without polling.
    PoolWindowAdded {
        label: String,
        version: u64,
        /// Phase CPD-1 — saga correlation. Mirrors the `saga_id` that
        /// arrived on the originating
        /// `Command::ReportPoolWindowAdded { saga_id }`. `None` for
        /// organic refills (no saga in flight). Subscribers (CPD-4
        /// per-saga correlation) match on this to scope events to the
        /// originating saga.
        ///
        /// `#[serde(default)]` for forward-compat with old
        /// `launcher-events.log` entries that pre-date CPD-1.
        #[serde(default)]
        saga_id: Option<u64>,
    },
    PoolWindowRemoved {
        label: String,
        version: u64,
    },
    /// Phase F.5 — emitted by the launcher when the host explicitly
    /// reports a pool window was promoted to a user-visible
    /// top-level window (i.e. the pool→window handoff inside
    /// `agentmux-cef::commands::window_pool::promote_pool_window`).
    /// The pool-respawn saga (launcher-side coordinator) starts on
    /// this event, brackets the implicit refill in
    /// `SagaStarted`/`SagaCompleted`, and waits for the matching
    /// `PoolWindowAdded` for a fresh pool label.
    ///
    /// Distinct from `PoolWindowRemoved` because that event also
    /// fires on pre-promote destroy (closing without promoting), where
    /// no refill saga should run.
    PoolWindowPromoted {
        label: String,
        version: u64,
    },
    /// Phase F.6 — emitted by the launcher when the host reports that
    /// all browser-pane HWNDs belonging to a closing top-level window
    /// have been reaped (`Command::ReportPanesReaped`). Step-1
    /// terminal signal for the window-cleanup-cascade saga.
    PanesReaped {
        label: String,
        version: u64,
        /// Phase CPD-1 — saga correlation. Mirrors `saga_id` from the
        /// originating `ReportPanesReaped`. `None` for organic
        /// reports (non-saga-driven pane drains).
        ///
        /// `#[serde(default)]` for forward-compat with pre-CPD-1
        /// log entries.
        #[serde(default)]
        saga_id: Option<u64>,
    },
    /// Phase F.6 — emitted by the launcher when the host reports it
    /// kicked off Stage 1 of the close-cascade pool drain (i.e. the
    /// just-closed window was the last user-visible window).
    /// Step-2 terminal signal (success branch) for the
    /// window-cleanup-cascade saga.
    ///
    /// "Drained" here means "drain initiated"; the actual pool
    /// teardown is async and surfaces as a series of
    /// `PoolWindowRemoved` events as each pool browser's
    /// `on_before_close` fires. The saga doesn't wait for those —
    /// the bracket closes when drain is *decided*, not when it
    /// completes.
    PoolDrained {
        label: String,
        version: u64,
        /// Phase CPD-1 — saga correlation. Mirrors `saga_id` from the
        /// originating `ReportPoolDrainDecision`. `None` for organic
        /// reports.
        #[serde(default)]
        saga_id: Option<u64>,
    },
    /// Phase F.6 — emitted by the launcher when the host reports a
    /// close that did NOT trigger a pool drain (other user-visible
    /// windows remain). Step-2 terminal signal (no-op branch) for
    /// the window-cleanup-cascade saga; the bracket closes
    /// successfully because nothing further is needed.
    PoolNotLast {
        label: String,
        version: u64,
        /// Phase CPD-1 — saga correlation. Mirrors `saga_id` from the
        /// originating `ReportPoolDrainDecision`. `None` for organic
        /// reports.
        #[serde(default)]
        saga_id: Option<u64>,
    },
    /// Phase B.5 (window_id_map step a) — launcher recorded the
    /// label → backend window ID mapping. Subscribers (host's
    /// shadow, eventually srv-side consumers) update their
    /// projections.
    BackendWindowIdRegistered {
        label: String,
        window_id: String,
        version: u64,
    },
    /// Phase B.5 (window_id_map step a) — launcher dropped the
    /// label → backend window ID mapping (window closed).
    BackendWindowIdUnregistered {
        label: String,
        window_id: String,
        version: u64,
    },
    /// Phase B.5 — sequential instance number assigned to a window
    /// by the launcher's authoritative registry. Numbers start at 1
    /// for "main" and increment for each subsequent open. Never
    /// reused within a launcher run. The host caches these to
    /// display window titles; B.5 step 2 will retire host's own
    /// `WindowInstanceRegistry` in favor of this stream.
    WindowInstanceAssigned {
        label: String,
        num: u32,
        version: u64,
    },
    /// Phase B.5 — instance number released (window closed). The
    /// launcher's authoritative registry drops the label; the slot
    /// is NOT reused (numbers monotonic per spec invariant: stable
    /// instance# across promotions / unregisters).
    WindowInstanceReleased {
        label: String,
        num: u32,
        version: u64,
    },
    /// Phase B.4 follow-up — emitted when the launcher's mirror
    /// disagrees with the host's reported counts. Logged at WARN
    /// level so operators see drift immediately. Drift in B.4 is
    /// a CONTRACT BUG (the host should report every state change);
    /// B.5 will turn drift into a hard failure once the mirror is
    /// authoritative.
    DriftDetected {
        kind: DriftKind,
        host_count: u32,
        mirror_count: u32,
        version: u64,
    },
    /// Phase B.9.1 (WRR) — emitted when the launcher's reducer
    /// detects a divergence between CEF browser identity (tracked
    /// in `state.windows` / `state.pool` via `ReportWindow*`) and
    /// Win32 reality (newly tracked via `ReportHwnd*`). Each variant
    /// of `kind` is emitted at the moment the OS event that
    /// surfaces it is dispatched through the reducer — there is no
    /// timer / heartbeat. See `docs/retro/wrr-design-2026-04-28.md`
    /// for the full classification table.
    HwndDriftDetected {
        kind: HwndDriftKind,
        /// Affected label, when known.
        label: Option<String>,
        /// Affected HWND, when known. `u64` to keep the wire format
        /// stable across pointer-width differences.
        hwnd: Option<u64>,
        /// Human-readable description for the launcher log + `--diag`
        /// output. Free-form; not parsed by any consumer.
        detail: String,
        severity: Severity,
        version: u64,
    },
    /// Phase B.9.2 — pure-reducer self-heal. Emitted alongside an
    /// `HwndDriftDetected` when the reducer is confident the bug
    /// happened at OPEN TIME (not from later user action) and a
    /// safe corrective rect is computable. The host's WRR
    /// subscriber listens for this and applies `SetWindowPos` on
    /// the UI thread. No timers — the trigger is the same OS
    /// event tick that surfaced the bug, so the correction lands
    /// before the user has time to notice the orphan window.
    ///
    /// Guard for emission (per the design, suppress over-correction):
    /// - The window's `mirror.foregrounded_since_open == false`
    ///   (we never auto-move a window the user has already
    ///   touched).
    /// - The reducer can compute a target rect from
    ///   `state.monitors` (i.e., monitors are known).
    CorrectiveWindowMove {
        /// Win32 HWND to move.
        hwnd: u64,
        /// Reducer-computed target rect. Default policy:
        /// primary-monitor-centered at default window size.
        target_rect: Rect,
        /// Why correction fired — surfaces in host log + audit.
        reason: HwndDriftKind,
        version: u64,
    },
    /// Phase B.9.3 — saga-style corrective. Reducer detected the
    /// `OrphanInstance` transition (last user-visible label
    /// removed from `state.windows`, host still Running). Host
    /// subscriber handles by reaping the warm pool and calling
    /// `quit_message_loop()`. ADVISORY, not a hard command —
    /// the host's handler should re-check `state.browsers`
    /// before actually quitting (the user could open a new
    /// window in the same dispatch tick race window). Event is
    /// idempotent: multiple emissions are safe; reaping pool +
    /// quit are themselves idempotent.
    HostShouldQuit {
        version: u64,
    },
    /// Phase D.1 — reply to `Command::GetSnapshot`. Carries the
    /// reducer's current canonical state (the projections subscribers
    /// most commonly need). The `version` field is the reducer's
    /// `event_version` at the moment the snapshot was taken; events
    /// the subscriber receives AFTER this snapshot have monotonically
    /// greater version numbers, letting the subscriber apply them as
    /// deltas without missing or duplicating updates.
    ///
    /// What's included: lifecycle, windows (label + kind + parent +
    /// HWND-observation axis), pool labels, instance numbers,
    /// backend window IDs, monitor topology. What's intentionally
    /// excluded: `processes` (PID metadata is launcher-internal),
    /// `pending_hwnds` (transient reconciliation state), event log
    /// (Phase D.2 adds a separate snapshot-with-replay variant).
    Snapshot {
        version: u64,
        lifecycle: LifecyclePhase,
        windows: Vec<WindowSnapshot>,
        pool: Vec<String>,
        instance_registry: Vec<(String, u32)>,
        backend_window_ids: Vec<(String, String)>,
        monitors: Vec<Rect>,
    },
    /// Phase D.3 — reply to `Command::GetEvents { since }`. Carries
    /// the events the launcher has emitted with `version > since`,
    /// in ascending version order. Subscribers apply them in order
    /// to catch up to the launcher's live stream.
    ///
    /// The `version` field on this event is the launcher's
    /// `event_version` at the moment the reply was assembled — not
    /// the highest version inside `events`. A subscriber treating
    /// this as the "as-of" point for subsequent events should use
    /// `events.last().version` (or fall back to this `version` if
    /// `events` is empty) to know where to resume the live stream.
    ///
    /// Replay is best-effort: if `since` predates the launcher's
    /// in-memory ring, `events` contains everything still retained
    /// but the subscriber should expect potential missed events
    /// before the first one in this reply.
    EventList {
        events: Vec<Event>,
        version: u64,
    },
    /// Phase E.1a — emitted by the saga coordinator when a new saga
    /// starts. `name` is the saga's static name (e.g. "tear_off_block")
    /// for `--diag` output. Subscribers (renderer especially) use
    /// `saga_id` to start buffering subsequent events with that id
    /// until the matching `SagaCompleted` or `SagaFailed` arrives.
    ///
    /// `saga_id` is monotonic per launcher run, allocated by the
    /// coordinator. Persisting saga state across launcher restarts
    /// is deferred (Phase F or beyond); restart abandons in-flight
    /// sagas, and renderer-side timeouts handle the visible
    /// consequence.
    SagaStarted {
        saga_id: u64,
        name: String,
        version: u64,
    },
    /// Phase E.1a — saga ended successfully. All events with this
    /// `saga_id` have been emitted; subscribers can flush their
    /// buffers and apply the changes atomically.
    SagaCompleted {
        saga_id: u64,
        version: u64,
    },
    /// Phase E.1a — saga ended in failure. `reason` is operator-
    /// readable; if compensation actions were issued, they appear
    /// as ordinary commands/events on the bus before this event.
    /// Renderers should discard their buffer for this `saga_id` —
    /// no atomic apply.
    SagaFailed {
        saga_id: u64,
        reason: String,
        version: u64,
    },
    /// Phase E.1b — srv-side snapshot reply. Phase E.2 populates
    /// `workspaces` (canonical Vec); subsequent sub-phases add
    /// `tabs`, `blocks`, `layouts`, etc.
    ///
    /// `version` is the srv reducer's `event_version` at snapshot
    /// time, monotonically distinct from prior srv events so
    /// subscribers know the "as-of" point for delta application.
    SrvSnapshot {
        version: u64,
        lifecycle: LifecyclePhase,
        /// Phase E.2 — sorted list of workspaces in the reducer's
        /// canonical state. (id, name) pairs for compactness; full
        /// state available via per-event subscription. Empty before
        /// E.2 lands.
        ///
        /// `#[serde(default)]` so old `srv-events.log` entries
        /// written by E.1b (which had no `workspaces` field) still
        /// deserialize when later sub-phases add bootstrap-replay
        /// from the on-disk log. Same forward-compat treatment will
        /// apply to E.3's `blocks`, etc. (reagent P2 #611.)
        #[serde(default)]
        workspaces: Vec<(String, String)>,
        /// Phase E.2b — sorted list of tabs in the reducer's canonical
        /// state. `(tab_id, workspace_id, name)` triples for
        /// compactness. `#[serde(default)]` for forward-compat with
        /// pre-E.2b log entries.
        #[serde(default)]
        tabs: Vec<(String, String, String)>,
        /// Phase E.2b — sorted list of `(workspace_id, active_tab_id)`
        /// pairs for workspaces that have an active tab set.
        /// Workspaces with no active tab are omitted.
        #[serde(default)]
        active_tabs: Vec<(String, String)>,
        /// Phase E.3 — sorted list of blocks in the reducer's
        /// canonical state. `(block_id, tab_id)` pairs for
        /// compactness. `#[serde(default)]` for forward-compat with
        /// pre-E.3 log entries.
        #[serde(default)]
        blocks: Vec<(String, String)>,
    },
    /// Phase E.2 — workspace was created. Carries the assigned
    /// `oid` and `name` so subscribers (renderer, persist) can
    /// apply the change without further round-trips.
    WorkspaceCreated {
        workspace_id: String,
        name: String,
        version: u64,
    },
    /// Phase E.2 — workspace was deleted.
    WorkspaceDeleted {
        workspace_id: String,
        /// Every block_id cascaded out with this workspace (every block in
        /// every tab it contained) — `TabDeleted`/`WorkspaceDeleted` never
        /// emitted per-block events (see `reducer/tab.rs::handle_delete_tab`),
        /// so this is the host's only signal to tear down a browser-pane
        /// renderer whose tab/workspace was deleted while unloaded/inactive
        /// (issue #2218, B.4). `#[serde(default)]` so replaying an old
        /// saga-log entry (predating this field) still deserializes.
        #[serde(default)]
        block_ids: Vec<String>,
        version: u64,
    },
    /// Phase E.2b — tab was created inside a workspace. Carries the
    /// assigned `tab_id` and parent `workspace_id` so subscribers can
    /// place it in the correct workspace's tab list.
    TabCreated {
        workspace_id: String,
        tab_id: String,
        name: String,
        version: u64,
    },
    /// Phase E.2b — tab was deleted from a workspace.
    TabDeleted {
        workspace_id: String,
        tab_id: String,
        /// Every block_id cascaded out with this tab. See `WorkspaceDeleted`'s
        /// `block_ids` doc — same rationale (issue #2218, B.4).
        #[serde(default)]
        block_ids: Vec<String>,
        version: u64,
    },
    /// Phase E.2b — a workspace's active tab changed. `tab_id: None`
    /// means the workspace has no active tab (e.g., last tab deleted).
    ActiveTabChanged {
        workspace_id: String,
        tab_id: Option<String>,
        version: u64,
    },
    /// Phase E.2c.3b — a tab was reordered within its workspace's
    /// `tab_ids`. Subscribers should rewrite the workspace's tab
    /// order to match the reducer's authoritative list (which lives
    /// in the snapshot's `tabs` field; subscribers can also recompute
    /// from `tab_id` + `new_index` against their last-known order).
    TabReordered {
        workspace_id: String,
        tab_id: String,
        new_index: u32,
        version: u64,
    },
    /// Phase E.5 — srv-side window→workspace mapping established.
    /// Distinct from launcher's `WindowOpened` (which tracks CEF
    /// window lifecycle). Subscribers update their view of "which
    /// workspace is each window showing."
    SrvWindowOpened {
        window_id: String,
        workspace_id: String,
        version: u64,
    },
    /// Phase E.5 — srv-side window mapping removed. Distinct from
    /// launcher's `WindowClosed`.
    SrvWindowClosed {
        window_id: String,
        version: u64,
    },
    /// Phase E.5 — a window now points at a different workspace
    /// (used by the SwitchWorkspace command + the CloseWindow saga
    /// when reassigning during cleanup).
    SrvWindowWorkspaceChanged {
        window_id: String,
        workspace_id: String,
        version: u64,
    },
    /// Phase E.5.3 — workspace's `tab_ids` was replaced wholesale.
    /// Subscribers should rewrite the persistent `Workspace.tabids`
    /// to match the new list (preserving `pinnedtabids` separately —
    /// pinning is a Waveterm legacy and not in scope here).
    TabsReorderedBulk {
        workspace_id: String,
        tab_ids: Vec<String>,
        version: u64,
    },
    /// Phase E.5.3 — workspace was renamed.
    WorkspaceRenamed {
        workspace_id: String,
        name: String,
        version: u64,
    },
    /// Phase E.5.3 — tab was renamed.
    TabRenamed {
        tab_id: String,
        name: String,
        version: u64,
    },
    /// Phase E.5.3 — meta-patch applied to a workspace. Carries the
    /// patch (NOT the resolved meta map); subscribers merge against
    /// the workspace's existing meta. This shape lets sagas inspect
    /// what changed without needing the prior state.
    WorkspaceMetaUpdated {
        workspace_id: String,
        meta_patch: serde_json::Value,
        version: u64,
    },
    /// Phase E.5.x (issue #855) — meta-patch applied to a window's
    /// `meta` map. Same shape as `WorkspaceMetaUpdated`. Persist
    /// subscriber merges into wstore; WaveObjUpdate bridge translates
    /// to a frontend `waveobj:update` broadcast.
    WindowMetaUpdated {
        window_id: String,
        meta_patch: serde_json::Value,
        version: u64,
    },
    /// Phase E.5.3 — meta-patch applied to a tab.
    TabMetaUpdated {
        tab_id: String,
        meta_patch: serde_json::Value,
        version: u64,
    },
    /// Phase E.5.3 — meta-patch applied to a block.
    BlockMetaUpdated {
        block_id: String,
        meta_patch: serde_json::Value,
        version: u64,
    },
    /// Phase E.5.5 — a tab was moved from one workspace to another.
    /// Subscribers should rewrite both workspaces' `tabids` and the
    /// tab's `parentoref`/`workspaceid` to match the reducer's view.
    /// `dst_index` reflects the position in `dst_workspace_id.tab_ids`
    /// AFTER insertion (already clamped by the reducer).
    /// Carries enough information to re-derive the new state without
    /// reading the reducer (subscribers replay events post-Lagged).
    TabMoved {
        tab_id: String,
        src_workspace_id: String,
        dst_workspace_id: String,
        dst_index: u32,
        /// The source workspace's new `active_tab_id` after the move,
        /// or `None` if the source has no remaining tabs. Subscribers
        /// rewrite the source's `activetabid` to match.
        new_src_active_tab_id: Option<String>,
        /// The destination workspace's new `active_tab_id` after the
        /// move. Wcore behavior (`move_tab_to_workspace`) was to
        /// always set the moved tab as dst's active; the reducer
        /// mirrors that. `None` means "do not change dst.active_tab_id"
        /// — reserved for future flows where the moved tab shouldn't
        /// steal focus. Codex P2 #621.
        ///
        /// `#[serde(default)]` for forward-compat with pre-PR3
        /// `srv-events.log` entries (none in production yet, but the
        /// pattern is established).
        #[serde(default)]
        new_dst_active_tab_id: Option<String>,
        version: u64,
    },
    /// Phase E.5.5 — a block was moved from one tab to another (or
    /// repositioned within the same tab). Subscribers update both
    /// tabs' `blockids` and the block's `parentoref`. `dst_index`
    /// reflects post-insertion position.
    BlockMoved {
        block_id: String,
        src_tab_id: String,
        dst_tab_id: String,
        dst_index: u32,
        version: u64,
    },
    /// Phase E.3 — block was created inside a tab.
    BlockCreated {
        tab_id: String,
        block_id: String,
        /// Phase E.2c.4 — meta carried through from
        /// `Command::CreateBlock`. The persist subscriber writes
        /// the Block row with this meta map. `#[serde(default)]` for
        /// forward-compat with pre-E.2c.4 log entries.
        #[serde(default)]
        meta: serde_json::Value,
        version: u64,
    },
    /// Phase E.3 — block was deleted from a tab.
    BlockDeleted {
        tab_id: String,
        block_id: String,
        version: u64,
    },
    /// Phase E.4 (Option A) — a tab's `focusednodeid` changed via the
    /// reducer. Subscribers (persist, eventually the renderer's E.6
    /// dispatcher) update the tab's layout view. Empty `node_id`
    /// reflects a clear.
    FocusedNodeChanged {
        tab_id: String,
        node_id: String,
        version: u64,
    },
    /// Phase E.4 (Option A) — a tab's `magnifiednodeid` changed via
    /// the reducer. Empty `node_id` reflects a clear (toggle-off).
    MagnifiedNodeChanged {
        tab_id: String,
        node_id: String,
        version: u64,
    },

    // ── Phase E.4.B — Layout tree events ───────────────────────────────
    //
    // Mirror of the 11 layout commands (§6 of the formal spec). Each
    // event carries a `correlation_id` matching its command and a
    // `version` for sequencing. The persist subscriber applies the same
    // tree mutation as the reducer used, making applies idempotent.
    //
    // See docs/specs/srv-phase-e4b-formal-spec-2026-05-03.md §6.

    LayoutNodeInserted {
        tab_id: String,
        node: crate::LayoutNode,
        parent_id: Option<String>,
        index: Option<usize>,
        correlation_id: String,
        version: u64,
    },
    LayoutNodeInsertedAtIndex {
        tab_id: String,
        /// The reducer's resulting tree (post-op, post-balance) for the
        /// persist subscriber to write to `db_layout` — single-writer, no
        /// algebra re-run (SPEC_STRONG_REDUCER_AUTHORITY_LAYOUT). `None` =
        /// empty tree. `#[serde(default)]` for forward-compat with senders
        /// that predate the field.
        #[serde(default)]
        new_tree: Option<crate::LayoutNode>,
        node: crate::LayoutNode,
        index_arr: Vec<usize>,
        correlation_id: String,
        version: u64,
    },
    LayoutNodeDeleted {
        tab_id: String,
        node_id: String,
        /// The reducer's resulting tree (post-delete, post-collapse) for
        /// the persist subscriber to write to `db_layout` — single-writer,
        /// no algebra re-run (SPEC_STRONG_REDUCER_AUTHORITY_LAYOUT /
        /// SPEC_864 site #6). Unlike the other structural events, a delete
        /// CAN legitimately empty the tree (root-orphan case) — `None`
        /// alone is ambiguous with a version-skewed pre-change sender, so
        /// `tree_cleared` below disambiguates. `#[serde(default)]` for
        /// forward-compat.
        #[serde(default)]
        new_tree: Option<crate::LayoutNode>,
        /// True when the delete legitimately emptied the tree (the deleted
        /// node WAS the root). The persist subscriber clears `db_layout`'s
        /// rootnode only when `new_tree.is_none() && tree_cleared` — a bare
        /// `None` (old sender / replayed pre-change JSON) is ignored rather
        /// than erasing the persisted layout (same skew concern as codex
        /// P2 on #1883). `#[serde(default)]` = false for old JSON.
        #[serde(default)]
        tree_cleared: bool,
        /// True if the deleted node was the focused one (subscribers may
        /// need to refocus).
        was_focused: bool,
        /// True if the deleted node was the magnified one (subscribers
        /// may need to re-magnify or clear their magnification UI).
        /// Reagent P1 PR #715 round 3: reducer was clearing
        /// `magnified_node_id` internally but not reporting it.
        ///
        /// `#[serde(default)]` for forward-compat with replay /
        /// version-skewed senders that emit pre-round-3
        /// `LayoutNodeDeleted` JSON without this field (codex P2 PR
        /// #715 round 5).
        #[serde(default)]
        was_magnified: bool,
        correlation_id: String,
        version: u64,
    },
    LayoutNodeMoved {
        tab_id: String,
        /// Resulting tree for the persist subscriber (see LayoutNodeInsertedAtIndex).
        #[serde(default)]
        new_tree: Option<crate::LayoutNode>,
        node_id: String,
        new_parent_id: String,
        index: usize,
        correlation_id: String,
        version: u64,
    },
    LayoutNodesSwapped {
        tab_id: String,
        /// Resulting tree for the persist subscriber (see LayoutNodeInsertedAtIndex).
        #[serde(default)]
        new_tree: Option<crate::LayoutNode>,
        node1_id: String,
        node2_id: String,
        correlation_id: String,
        version: u64,
    },
    LayoutNodesResized {
        tab_id: String,
        /// Resulting tree for the persist subscriber (see LayoutNodeInsertedAtIndex).
        #[serde(default)]
        new_tree: Option<crate::LayoutNode>,
        ops: Vec<crate::ResizeOp>,
        correlation_id: String,
        version: u64,
    },
    LayoutNodeReplaced {
        tab_id: String,
        /// Resulting tree for the persist subscriber (see LayoutNodeInsertedAtIndex).
        #[serde(default)]
        new_tree: Option<crate::LayoutNode>,
        target_id: String,
        new_node: crate::LayoutNode,
        correlation_id: String,
        version: u64,
    },
    LayoutSplitHorizontalApplied {
        tab_id: String,
        /// Resulting tree for the persist subscriber (see LayoutNodeInsertedAtIndex).
        #[serde(default)]
        new_tree: Option<crate::LayoutNode>,
        target_id: String,
        new_node: crate::LayoutNode,
        position: crate::SplitPosition,
        correlation_id: String,
        version: u64,
    },
    LayoutSplitVerticalApplied {
        tab_id: String,
        /// Resulting tree for the persist subscriber (see LayoutNodeInsertedAtIndex).
        #[serde(default)]
        new_tree: Option<crate::LayoutNode>,
        target_id: String,
        new_node: crate::LayoutNode,
        position: crate::SplitPosition,
        correlation_id: String,
        version: u64,
    },
    LayoutCleared {
        tab_id: String,
        correlation_id: String,
        version: u64,
    },
    /// SPEC_864 Phase 2 — `slices` mirrors the command's field: when
    /// present, the persist subscriber also writes leaforder / focus /
    /// magnify / pendingbackendactions (REPLACE semantics). `None` =
    /// tree-only replace; those columns stay untouched.
    LayoutTreeReplaced {
        tab_id: String,
        new_tree: Option<crate::LayoutNode>,
        correlation_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        slices: Option<crate::LayoutClientSlices>,
        version: u64,
    },

    /// SPEC_864 Phase 4 — actions appended to a tab's
    /// `pendingbackendactions` queue through the reducer (the
    /// backend→frontend "please mutate your layout tree" channel).
    /// `actions` is a JSON array of `LayoutActionData` objects, carried
    /// as a raw value the same way `LayoutClientSlices` carries the
    /// queue: the reducer does NOT model the queue in `TabRecord`
    /// (pass-through), and the persist subscriber APPENDS these to the
    /// existing `LayoutState.pendingbackendactions` (append semantics —
    /// unlike the slices REPLACE semantics of a frontend push, which is
    /// how the frontend acks/clears the queue).
    LayoutBackendActionsQueued {
        tab_id: String,
        actions: serde_json::Value,
        correlation_id: String,
        version: u64,
    },

    /// Phase CPD-1 (cross-process dispatch) — emitted by the launcher
    /// when the host reports that a saga-issued action failed
    /// (`Command::ReportSagaActionFailed { saga_id, reason }`). The
    /// saga coordinator's bus loop will (in CPD-3) treat this as a
    /// terminal signal for the matching saga, emitting
    /// `Event::SagaFailed` and removing it from the in-flight
    /// registry. CPD-1 ships the wire shape only; no producer
    /// (host) and no consumer (saga coordinator) are wired yet.
    SagaActionFailed {
        saga_id: u64,
        reason: String,
        version: u64,
    },
}

/// Phase CPD-1 (cross-process dispatch) — envelope enum for frames
/// sent over the launcher → host pipe direction. Today the host's
/// read loop only expects `Event` JSON; CPD-2 extends the read loop
/// to recognize this tagged union and dispatch by `kind`:
///
/// * `event` → existing event-handling code (state sync from
///   launcher reducer broadcasts).
/// * `command` → new command-handling code (saga-issued actions).
///
/// Newline-delimited JSON, one frame per line. `#[serde(tag = "kind",
/// rename_all = "snake_case")]` so the wire shape is e.g.
/// `{"kind":"event","event":"pool_window_added",...}` or
/// `{"kind":"command","cmd":"spawn_pool_window","saga_id":42}`.
///
/// Schema-only in CPD-1: introduced now so the launcher's host-pipe
/// writer (CPD-2) and the host's read loop (CPD-2/CPD-3) can be
/// built against a stable wire envelope without further schema
/// churn.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum HostFrame {
    /// Wraps a launcher-emitted Event being pushed down to the host
    /// (existing fanout path; CPD-2 refactors the writer to go
    /// through this envelope).
    Event(Event),
    /// Wraps a saga-issued Command being dispatched from the launcher
    /// to the host. Carries `saga_id` inside the Command payload.
    Command(Command),
}

/// Phase D.1 — serializable view of one window in the launcher
/// reducer's canonical state. Maps 1:1 to the launcher's internal
/// `WindowMirror` minus `opened_at` (which is launcher-local clock
/// data not meaningful to subscribers).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct WindowSnapshot {
    pub label: String,
    pub kind: WindowKind,
    pub parent_label: Option<String>,
    pub hwnd: Option<u64>,
    pub visible: bool,
    pub iconic: bool,
    pub last_rect: Option<Rect>,
    pub foregrounded_since_open: bool,
}

/// Phase B.9.1 — six classes of CEF↔Win32 disagreement the reducer
/// can detect at event-dispatch time. See the WRR design doc for
/// the per-kind triggering Command and reducer action.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HwndDriftKind {
    /// `state.windows` has a label whose `hwnd` field never got
    /// populated, AND a follow-up event arrived that should have
    /// reconciled it. CEF says open, Win32 has no matching HWND.
    BrowserWithoutHwnd,
    /// HWND in the host's report doesn't map to any
    /// `state.windows` / `state.pool` label. Win32 has it, CEF
    /// didn't open it. The "stray taskbar" case.
    HwndWithoutBrowser,
    /// HWND for a known label was never SHOWN (no foreground
    /// event) since open and a subsequent state transition has
    /// elapsed. User can't see it.
    HiddenSinceOpen,
    /// Window rect doesn't intersect any monitor in
    /// `state.monitors`. Off-screen orphan.
    OffMonitor,
    /// `ReportHwndDestroyed` arrived without a preceding
    /// `ReportWindowClosed` for the matching label. Renderer
    /// crashed, took the HWND with it.
    OrphanDestroy,
    /// `ReportWindowClosed` arrived, but subsequent OS events for
    /// the HWND keep firing (it never went away on the Win32 side).
    LingeringHwnd,
    /// Phase B.9.3 — host process is alive and registered, but
    /// `state.windows` just transitioned to empty. The host's own
    /// close path doesn't reap the warm pool when the last
    /// user-visible window closes (pool windows hold
    /// `state.browsers` non-empty, so `quit_message_loop` never
    /// fires). The launcher's reducer is the only place that knows
    /// "all user-meaningful labels are gone" cleanly, so we
    /// surface the signal here. Paired with `Event::HostShouldQuit`
    /// emitted in the same reducer call (see B.9.3 saga).
    OrphanInstance,
}

/// Phase B.9.1 — drift severity. Operator-tunable severity floor
/// in `WrrConfig.severity_floor` controls which events get
/// broadcast (ones below the floor are still logged at DEBUG so
/// they show up in `--diag wrr` post-mortem).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "lowercase")]
pub enum Severity {
    Info,
    Warn,
    Error,
}

/// Coarse-grained launcher state. Spec §4: Starting → Running →
/// Quitting → Dead, no other transitions allowed. The reducer in
/// agentmux-launcher::reducer enforces this; a violation panics
/// (Job Object reaps via OS).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LifecyclePhase {
    /// Initial state; launcher has not yet seen the host register.
    Starting,
    /// Host has registered and the canonical state is being
    /// maintained. Steady state.
    Running,
    /// Quit { reason } received, ack outstanding to subscribers.
    /// Phase B.3 keeps this state-shape only — the actual Quit
    /// command lands in a later sub-PR.
    Quitting,
    /// Cleanup done; launcher about to exit. Transient.
    Dead,
}

/// Phase B.4 follow-up — which mirror diverged. Tagged so subscribers
/// can route alerts (windows-drift might page; pool-drift is more
/// ephemeral since the pool turns over fast).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DriftKind {
    Windows,
    Pool,
}

/// Discriminant for `Event::Error` — keeps clients structured against
/// failure modes without parsing message text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ErrorCode {
    /// Couldn't deserialize the line into a Command.
    InvalidCommand,
    /// Command sent before Register.
    NotRegistered,
    /// Register sent twice on the same connection.
    AlreadyRegistered,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_register_roundtrip() {
        let c = Command::Register {
            kind: ClientKind::Host,
            pid: 12345,
            version: "0.33.449".into(),
        };
        let json = serde_json::to_string(&c).unwrap();
        assert!(json.contains("\"cmd\":\"register\""));
        assert!(json.contains("\"kind\":\"host\""));
        let back: Command = serde_json::from_str(&json).unwrap();
        if let Command::Register { kind, pid, version } = back {
            assert_eq!(kind, ClientKind::Host);
            assert_eq!(pid, 12345);
            assert_eq!(version, "0.33.449");
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn event_registered_roundtrip() {
        let e = Event::Registered {
            client_id: 1,
            launcher_pid: 9999,
            launcher_version: "0.33.449".into(),
            version: 42,
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"event\":\"registered\""));
        let back: Event = serde_json::from_str(&json).unwrap();
        if let Event::Registered { client_id, version, .. } = back {
            assert_eq!(client_id, 1);
            assert_eq!(version, 42);
        } else {
            panic!("wrong variant");
        }
    }

    #[test]
    fn unknown_cmd_fails_to_deserialize() {
        let json = r#"{"cmd":"banana"}"#;
        let r: Result<Command, _> = serde_json::from_str(json);
        assert!(r.is_err());
    }

    // ---------- Phase CPD-1 — saga_id schema additions ----------

    #[test]
    fn cpd1_spawn_pool_window_round_trip_with_saga_id() {
        let c = Command::SpawnPoolWindow { saga_id: 42 };
        let json = serde_json::to_string(&c).unwrap();
        assert!(json.contains("\"cmd\":\"spawn_pool_window\""));
        assert!(json.contains("\"saga_id\":42"));
        let back: Command = serde_json::from_str(&json).unwrap();
        match back {
            Command::SpawnPoolWindow { saga_id } => assert_eq!(saga_id, 42),
            other => panic!("wrong variant: {:?}", other),
        }
    }

    #[test]
    fn cpd1_spawn_pool_window_forward_compat_default_zero() {
        // Pre-CPD-1 hosts emit the bare command with no `saga_id`
        // field; serde_default fills in 0.
        let json = r#"{"cmd":"spawn_pool_window"}"#;
        let back: Command = serde_json::from_str(json).unwrap();
        match back {
            Command::SpawnPoolWindow { saga_id } => assert_eq!(saga_id, 0),
            other => panic!("wrong variant: {:?}", other),
        }
    }

    #[test]
    fn cpd1_reap_panes_round_trip_with_saga_id() {
        let c = Command::ReapPanes {
            label: "main".into(),
            saga_id: 7,
        };
        let json = serde_json::to_string(&c).unwrap();
        assert!(json.contains("\"cmd\":\"reap_panes\""));
        assert!(json.contains("\"saga_id\":7"));
        let back: Command = serde_json::from_str(&json).unwrap();
        match back {
            Command::ReapPanes { label, saga_id } => {
                assert_eq!(label, "main");
                assert_eq!(saga_id, 7);
            }
            other => panic!("wrong variant: {:?}", other),
        }
    }

    #[test]
    fn cpd1_reap_panes_forward_compat_default_zero() {
        let json = r#"{"cmd":"reap_panes","label":"main"}"#;
        let back: Command = serde_json::from_str(json).unwrap();
        match back {
            Command::ReapPanes { label, saga_id } => {
                assert_eq!(label, "main");
                assert_eq!(saga_id, 0);
            }
            other => panic!("wrong variant: {:?}", other),
        }
    }

    #[test]
    fn cpd1_drain_pool_if_last_round_trip_with_saga_id() {
        let c = Command::DrainPoolIfLast {
            label: "main".into(),
            saga_id: 13,
        };
        let json = serde_json::to_string(&c).unwrap();
        assert!(json.contains("\"cmd\":\"drain_pool_if_last\""));
        assert!(json.contains("\"saga_id\":13"));
        let back: Command = serde_json::from_str(&json).unwrap();
        match back {
            Command::DrainPoolIfLast { label, saga_id } => {
                assert_eq!(label, "main");
                assert_eq!(saga_id, 13);
            }
            other => panic!("wrong variant: {:?}", other),
        }
    }

    #[test]
    fn cpd1_drain_pool_if_last_forward_compat_default_zero() {
        let json = r#"{"cmd":"drain_pool_if_last","label":"main"}"#;
        let back: Command = serde_json::from_str(json).unwrap();
        match back {
            Command::DrainPoolIfLast { label, saga_id } => {
                assert_eq!(label, "main");
                assert_eq!(saga_id, 0);
            }
            other => panic!("wrong variant: {:?}", other),
        }
    }

    #[test]
    fn cpd1_report_pool_window_added_round_trip_with_some() {
        let c = Command::ReportPoolWindowAdded {
            label: "pool-xyz".into(),
            saga_id: Some(99),
        };
        let json = serde_json::to_string(&c).unwrap();
        assert!(json.contains("\"saga_id\":99"));
        let back: Command = serde_json::from_str(&json).unwrap();
        match back {
            Command::ReportPoolWindowAdded { label, saga_id } => {
                assert_eq!(label, "pool-xyz");
                assert_eq!(saga_id, Some(99));
            }
            other => panic!("wrong variant: {:?}", other),
        }
    }

    #[test]
    fn cpd1_report_pool_window_added_forward_compat_default_none() {
        // Pre-CPD-1 hosts omit `saga_id` entirely; deserializes to
        // `None`. JSON with explicit null is also valid.
        let json = r#"{"cmd":"report_pool_window_added","label":"pool-xyz"}"#;
        let back: Command = serde_json::from_str(json).unwrap();
        match back {
            Command::ReportPoolWindowAdded { label, saga_id } => {
                assert_eq!(label, "pool-xyz");
                assert_eq!(saga_id, None);
            }
            other => panic!("wrong variant: {:?}", other),
        }
    }

    #[test]
    fn cpd1_report_panes_reaped_round_trip_with_some() {
        let c = Command::ReportPanesReaped {
            label: "main".into(),
            saga_id: Some(11),
        };
        let json = serde_json::to_string(&c).unwrap();
        let back: Command = serde_json::from_str(&json).unwrap();
        match back {
            Command::ReportPanesReaped { label, saga_id } => {
                assert_eq!(label, "main");
                assert_eq!(saga_id, Some(11));
            }
            other => panic!("wrong variant: {:?}", other),
        }
    }

    #[test]
    fn cpd1_report_panes_reaped_forward_compat_default_none() {
        let json = r#"{"cmd":"report_panes_reaped","label":"main"}"#;
        let back: Command = serde_json::from_str(json).unwrap();
        match back {
            Command::ReportPanesReaped { label, saga_id } => {
                assert_eq!(label, "main");
                assert_eq!(saga_id, None);
            }
            other => panic!("wrong variant: {:?}", other),
        }
    }

    #[test]
    fn cpd1_report_pool_drain_decision_round_trip_with_some() {
        let c = Command::ReportPoolDrainDecision {
            label: "main".into(),
            was_last: true,
            saga_id: Some(101),
        };
        let json = serde_json::to_string(&c).unwrap();
        let back: Command = serde_json::from_str(&json).unwrap();
        match back {
            Command::ReportPoolDrainDecision {
                label,
                was_last,
                saga_id,
            } => {
                assert_eq!(label, "main");
                assert!(was_last);
                assert_eq!(saga_id, Some(101));
            }
            other => panic!("wrong variant: {:?}", other),
        }
    }

    #[test]
    fn cpd1_report_pool_drain_decision_forward_compat_default_none() {
        let json = r#"{"cmd":"report_pool_drain_decision","label":"main","was_last":false}"#;
        let back: Command = serde_json::from_str(json).unwrap();
        match back {
            Command::ReportPoolDrainDecision {
                label,
                was_last,
                saga_id,
            } => {
                assert_eq!(label, "main");
                assert!(!was_last);
                assert_eq!(saga_id, None);
            }
            other => panic!("wrong variant: {:?}", other),
        }
    }

    #[test]
    fn cpd1_report_saga_action_failed_round_trip() {
        let c = Command::ReportSagaActionFailed {
            saga_id: 55,
            reason: "window not found".into(),
        };
        let json = serde_json::to_string(&c).unwrap();
        assert!(json.contains("\"cmd\":\"report_saga_action_failed\""));
        assert!(json.contains("\"saga_id\":55"));
        let back: Command = serde_json::from_str(&json).unwrap();
        match back {
            Command::ReportSagaActionFailed { saga_id, reason } => {
                assert_eq!(saga_id, 55);
                assert_eq!(reason, "window not found");
            }
            other => panic!("wrong variant: {:?}", other),
        }
    }

    #[test]
    fn cpd1_event_pool_window_added_round_trip_with_saga_id() {
        let e = Event::PoolWindowAdded {
            label: "pool-xyz".into(),
            version: 7,
            saga_id: Some(42),
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"saga_id\":42"));
        let back: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn cpd1_event_panes_reaped_forward_compat_default_none() {
        // Pre-CPD-1 launcher-events.log entries lack `saga_id`.
        let json = r#"{"event":"panes_reaped","label":"main","version":1}"#;
        let back: Event = serde_json::from_str(json).unwrap();
        match back {
            Event::PanesReaped {
                label,
                version,
                saga_id,
            } => {
                assert_eq!(label, "main");
                assert_eq!(version, 1);
                assert_eq!(saga_id, None);
            }
            other => panic!("wrong variant: {:?}", other),
        }
    }

    #[test]
    fn cpd1_event_pool_drained_round_trip_with_saga_id() {
        let e = Event::PoolDrained {
            label: "main".into(),
            version: 9,
            saga_id: Some(3),
        };
        let json = serde_json::to_string(&e).unwrap();
        let back: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn cpd1_event_pool_not_last_forward_compat_default_none() {
        let json = r#"{"event":"pool_not_last","label":"main","version":3}"#;
        let back: Event = serde_json::from_str(json).unwrap();
        match back {
            Event::PoolNotLast {
                label,
                version,
                saga_id,
            } => {
                assert_eq!(label, "main");
                assert_eq!(version, 3);
                assert_eq!(saga_id, None);
            }
            other => panic!("wrong variant: {:?}", other),
        }
    }

    #[test]
    fn cpd1_event_saga_action_failed_round_trip() {
        let e = Event::SagaActionFailed {
            saga_id: 12,
            reason: "host pipe broken".into(),
            version: 100,
        };
        let json = serde_json::to_string(&e).unwrap();
        assert!(json.contains("\"event\":\"saga_action_failed\""));
        assert!(json.contains("\"saga_id\":12"));
        let back: Event = serde_json::from_str(&json).unwrap();
        assert_eq!(back, e);
    }

    #[test]
    fn cpd1_host_frame_event_round_trip() {
        let frame = HostFrame::Event(Event::PoolWindowAdded {
            label: "pool-xyz".into(),
            version: 5,
            saga_id: Some(7),
        });
        let json = serde_json::to_string(&frame).unwrap();
        assert!(json.contains("\"kind\":\"event\""));
        assert!(json.contains("\"event\":\"pool_window_added\""));
        let back: HostFrame = serde_json::from_str(&json).unwrap();
        match back {
            HostFrame::Event(Event::PoolWindowAdded {
                label,
                version,
                saga_id,
            }) => {
                assert_eq!(label, "pool-xyz");
                assert_eq!(version, 5);
                assert_eq!(saga_id, Some(7));
            }
            other => panic!("wrong frame: {:?}", other),
        }
    }

    #[test]
    fn cpd1_host_frame_command_round_trip() {
        let frame = HostFrame::Command(Command::SpawnPoolWindow { saga_id: 31 });
        let json = serde_json::to_string(&frame).unwrap();
        assert!(json.contains("\"kind\":\"command\""));
        assert!(json.contains("\"cmd\":\"spawn_pool_window\""));
        assert!(json.contains("\"saga_id\":31"));
        let back: HostFrame = serde_json::from_str(&json).unwrap();
        match back {
            HostFrame::Command(Command::SpawnPoolWindow { saga_id }) => {
                assert_eq!(saga_id, 31);
            }
            other => panic!("wrong frame: {:?}", other),
        }
    }

    #[test]
    fn cpd1_host_frame_unknown_kind_fails() {
        let json = r#"{"kind":"banana"}"#;
        let r: Result<HostFrame, _> = serde_json::from_str(json);
        assert!(r.is_err());
    }
}
