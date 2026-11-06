// Copyright 2025-2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The events the launcher sends to clients, and the frame that wraps either direction on the
//! host pipe.

use super::*;
use serde::{Deserialize, Serialize};

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
    /// subscriber merges into mstore; MuxObjUpdate bridge translates
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
    /// A block left a tab and was kept (`Command::ParkBlock`); nothing was deleted.
    BlockParked {
        tab_id: String,
        block_id: String,
        version: u64,
    },
    /// A parked block went into a tab (`Command::AttachBlock`).
    BlockAttached {
        block_id: String,
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
    // Each carries the resulting tree (or the queue change), so the
    // persist subscriber writes it without re-running the algebra.
    // See docs/specs/srv-phase-e4b-formal-spec-2026-05-03.md §6.

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
    /// SPEC_864 Phase 2 — `slices` mirrors the command's field: when
    /// present, the persist subscriber also writes focus /
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
    /// `pendingbackendactions` queue through the reducer: what srv did to
    /// the tree, for the window showing the tab to mirror. `actions` is a
    /// JSON array of `LayoutActionData` objects, carried as a raw value the
    /// same way `LayoutClientSlices` carries the queue: the reducer does NOT
    /// model the queue in `TabRecord` (pass-through), and the persist
    /// subscriber APPENDS these to `LayoutState.pendingbackendactions`.
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
