// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The commands a client sends to the launcher, and the connection kinds that send them.

use super::*;
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
    /// Launcher → host: the level of srv's health-probe latency changed —
    /// the rolling average of the launcher's probe round-trip times crossed a
    /// threshold. The host shows it through the memory-pressure banner system
    /// as kind `"backend"`. `level` is `"normal" | "warn" | "critical"`.
    /// See docs/analysis/ANALYSIS_SRV_HTTP_STALL_IO_DRIVER_STARVATION_2026_09_26.md §8.2.
    NotifySrvLatency {
        level: String,
        avg_ms: u64,
    },
    /// Launcher → host: quit gracefully, as the host's `quit_app` command does (tray Quit, agreed upgrade restart).
    QuitApp,
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
    /// via `MOS.makeORef`). The launcher mirrors it for the same
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
    /// Phase E.2 — create a new workspace and emit
    /// `Event::WorkspaceCreated`. Its `oid` is minted by the dispatcher
    /// (srv's `reducer::Ctx::new_id`), not the reducer.
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
    /// Phase E.2b — create a tab inside an existing workspace, append it
    /// to the workspace's ordered tab list, and emit `Event::TabCreated`.
    /// Its `tab_id` is minted by the dispatcher (srv's
    /// `reducer::Ctx::new_id`). Returns `Event::Error` if the parent
    /// workspace doesn't exist.
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
    /// actual merge against mstore. Reducer state does NOT track meta
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
    /// lands on `srv_events_tx` and the MuxObjUpdate broadcast bridge
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
    ///
    /// Errors if the source, dest or tab is missing, the tab isn't in the
    /// source, or it is the source's last tab and `allow_empty_source` is
    /// false. Used by TearOffTab, MoveTabToWorkspace and RestoreTornOffTab.
    MoveTab {
        tab_id: String,
        src_workspace_id: String,
        dst_workspace_id: String,
        dst_index: u32,
        /// Bypass the last-tab guard, for flows that drain a workspace and
        /// then delete it. Defaults to `false`, like `DeleteTab.force`.
        #[serde(default)]
        allow_empty_source: bool,
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
    /// Phase E.3 — create a block inside an existing tab, append it to
    /// the tab's `block_ids`, and emit `Event::BlockCreated`. Its
    /// `block_id` is minted by the dispatcher (srv's
    /// `reducer::Ctx::new_id`). Returns `Event::Error` if the parent tab
    /// doesn't exist.
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
    /// Take an agent's block out of `tab_id` and keep it, in no tab. No-op when it isn't in that
    /// tab. The layout node goes separately (`LayoutDeleteNodeByBlock`).
    ParkBlock {
        tab_id: String,
        block_id: String,
    },
    /// Put a parked block into `dst_tab_id` at `dst_index` (clamped); errors when it is in a tab.
    AttachBlock {
        block_id: String,
        dst_tab_id: String,
        dst_index: u32,
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
    /// rest of `LayoutState` (rootnode/pendingbackendactions)
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
    // The frontend's own tree edits arrive as `LayoutApplyIntent`; the
    // rest are srv's (deletes by block, pane tabs, seeding, the queue).
    // See docs/specs/srv-phase-e4b-formal-spec-2026-05-03.md §5.

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
    /// blocks may legitimately have no layout node (already removed by a
    /// layout intent, or never laid out).
    LayoutDeleteNodeByBlock {
        tab_id: String,
        block_id: String,
        correlation_id: String,
    },
    /// Add `block_id` to the tab stack of the pane holding
    /// `target_block_id`, optionally as the visible tab.
    /// SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md §3.2.
    LayoutStackPush {
        tab_id: String,
        target_block_id: String,
        block_id: String,
        activate: bool,
        correlation_id: String,
    },
    /// Make `block_id` the visible tab of the pane whose stack holds it.
    LayoutStackActivate {
        tab_id: String,
        block_id: String,
        correlation_id: String,
    },
    /// Create a block AND place it in the tab stack of the pane holding
    /// `target_block_id`, in one reducer step — the block never exists
    /// without a pane (the two-step `pane.open { skip_placement }` + frontend
    /// push could be interrupted in between, orphaning it).
    /// SPEC_PANE_TABS_REDUCER_COMMANDS_2026_09_18.md §3.2.
    CreateBlockInStack {
        tab_id: String,
        target_block_id: String,
        #[serde(default)]
        meta: serde_json::Value,
        activate: bool,
    },
    /// Move `block_id` to `position` relative to `target_block_id` — a
    /// same-leaf reorder when both are already members of the same pane's
    /// stack, otherwise a cross-pane move (removed from its current leaf,
    /// placed into `target_block_id`'s leaf, promoting a single-block leaf
    /// into a stack if needed). Refuses (silent no-op-with-error) if
    /// `block_id` is its source leaf's only member — removing the last
    /// member is "close the pane," a different command. A same-leaf reorder
    /// never changes which member is visible unless `activate` is set,
    /// even when the reordered member was already the visible one.
    /// SPEC_PANE_TAB_DRAG_AND_DROP_2026_09_19.md §4.1.
    LayoutStackMove {
        tab_id: String,
        block_id: String,
        target_block_id: String,
        position: crate::StackMovePosition,
        activate: bool,
        correlation_id: String,
    },
    /// Move several Pane Tabs into ONE pane in a single edit: every block in
    /// `block_ids` not already in the pane holding `target_block_id` is
    /// appended to it, in the order given, as `LayoutStackMove` with
    /// `position: End` would for each. `activate_block_id`, when present,
    /// becomes that pane's visible tab. All or nothing: a block or target
    /// that is not in the tree leaves the tree untouched. A pane emptied by
    /// the move is removed, as for `LayoutStackMove`.
    /// SPEC_PANE_TAB_MULTI_SELECT_MOVE_AND_COLLAPSED_EXPAND_2026_10_24.md §5.
    LayoutStackMoveMany {
        tab_id: String,
        block_ids: Vec<String>,
        target_block_id: String,
        #[serde(default)]
        activate_block_id: Option<String>,
        correlation_id: String,
    },
    /// Apply a layout intent to the tab's tree: the frontend's own tree
    /// action, computed and normalized by the reducer.
    /// SPEC_STRONG_REDUCER_AUTHORITY_LAYOUT_2026_06_30.md §4.
    ///
    /// `action_id` names it: branches it creates get ids derived from it,
    /// and a repeat of a recent id is a no-op. `view` is the sender's
    /// focused and magnified pane after the intent (`None`: the intent's own
    /// `focused`/`magnified` flags decide, as for srv's own placements).
    LayoutApplyIntent {
        tab_id: String,
        intent: crate::LayoutIntent,
        action_id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        view: Option<crate::LayoutViewState>,
    },
    /// Bulk-replace the tree. Used during Phase 7a writer migration and
    /// for tear-off where the whole subtree changes atomically.
    ///
    /// `slices` (srv's seeders) carries the rest of the `LayoutState` row
    /// (focus / magnify / pendingbackendactions) with REPLACE
    /// semantics. `slices: None` (tree-only callers) leaves those columns
    /// untouched.
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
    /// (`crates/launcher/src/host_pipe/`) and waits on the
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
    /// (`crates/launcher/src/startup_events.rs`), just sourced from
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
