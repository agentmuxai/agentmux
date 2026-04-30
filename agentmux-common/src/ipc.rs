//! Phase B.2 IPC wire protocol — shared between agentmux-launcher
//! (server) and agentmux-cef (client). One source of truth so the
//! Command / Event shapes can't drift between binaries on a
//! version-skew release.
//!
//! See `specs/SPEC_WINDOW_PROCESS_STATE_MACHINE_2026_04_27.md` §5.
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
    /// Phase B.4 follow-up — host reports a pre-warmed pool window
    /// being added (`spawn_pool_window`). Pool windows live in a
    /// SEPARATE map from the user-visible window mirror; the host
    /// transitions them out of the pool with `ReportPoolWindowRemoved`
    /// + `ReportWindowOpened` on promote, or just
    /// `ReportPoolWindowRemoved` on pre-promote destroy.
    ReportPoolWindowAdded {
        label: String,
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
    /// Phase E.3 — create a block inside an existing tab. Reducer
    /// validates parent tab exists, assigns the `block_id` (UUID),
    /// appends to the tab's `block_ids`, emits `Event::BlockCreated`.
    /// Session-only projection (no persist subscriber yet).
    CreateBlock {
        tab_id: String,
    },
    /// Phase E.3 — delete a block from a tab. Idempotent silent no-op
    /// on missing tab or missing block.
    DeleteBlock {
        tab_id: String,
        block_id: String,
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

/// Wire-side enum for `WindowKind`. Mirrors `agentmux-cef::state::WindowKind`
/// — kept here so the launcher can deserialize without depending on the
/// host crate. The host serializes its own type via `serde(rename_all =
/// "snake_case")` so the JSON shape matches exactly.
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
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
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
    },
    /// Phase B.4 follow-up — pool inventory transitioned. Emitted in
    /// response to `ReportPoolWindow{Added,Removed}`. Subscribers
    /// (Tool clients) use this to track pool warmth without polling.
    PoolWindowAdded {
        label: String,
        version: u64,
    },
    PoolWindowRemoved {
        label: String,
        version: u64,
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
    /// Phase E.3 — block was created inside a tab.
    BlockCreated {
        tab_id: String,
        block_id: String,
        version: u64,
    },
    /// Phase E.3 — block was deleted from a tab.
    BlockDeleted {
        tab_id: String,
        block_id: String,
        version: u64,
    },
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
}
