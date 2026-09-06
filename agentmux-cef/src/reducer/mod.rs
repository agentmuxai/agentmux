// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase F.1 — host reducer.
//
// Third reducer in the multi-reducer architecture, after the launcher
// (Phase B) and srv (Phase E). Same pure-functional shape as the
// other two: `update(&mut HostState, HostCommand) -> Vec<HostEvent>`,
// no I/O, no async, sub-millisecond mutex hold time.
//
// **Scope of F.1 (this PR):** skeleton + the `pending_window_creations`
// arm. Per `docs/specs/SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md` §3.1,
// pending_window_creations is the lowest-risk migration:
// single-producer/single-consumer queue with a clean enqueue/dequeue
// lifecycle and no FFI handles inside.
//
// **NOT in F.1:** drag arms (F.3), tear-off hook arms (F.4 — folds
// into the tear-off spec). The CEF `browsers` map and warm pool stay
// outside the reducer indefinitely (snapshot-and-drop discipline at
// every read site, see spec §3.2 / §6).
//
// **Wire protocol:** F.1's `HostCommand` and `HostEvent` are
// host-internal. They do NOT cross IPC. When a future PR adds a
// command that needs frontend or launcher access, that PR promotes
// the relevant variants to `agentmux-common::ipc::Command` /
// `agentmux-common::ipc::Event` and adds the IPC plumbing. Keeping
// F.1 in-process avoids serializing `PendingWindowCreation` over a
// pipe just to satisfy a pattern that has no current consumer.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};

use cef::Browser;

use crate::state::{
    BrowserHandle, BrowserKind, CompletedCreation, CreationPhase, DragSession, EffectKind,
    InFlightCreation, BrowserPaneEntry, BrowserPaneLifecycle, PanePoolState, PaneWindowState,
    PendingWindowCreation, PoolState, QuitReason, QuitState, TopLevelCreationOutcome,
    TopLevelCreationRequest, TopLevelCreationState, TopLevelSource, WindowPlacement,
};

/// Capacity of `TopLevelCreationState.history` ring buffer. Configurable
/// via `~/.agentmux/config.toml [host.reducer]` once H.5 (config) lands;
/// hard-coded for PR #1.
pub(crate) const TOP_LEVEL_CREATION_HISTORY_CAP: usize = 50;

/// Lifecycle phase of the host reducer. Mirrors the launcher and srv
/// reducers' phase enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostLifecyclePhase {
    /// Pre-init: AppState exists, but no commands accepted yet.
    Bootstrapping,
    /// Normal operation: all commands accepted.
    Running,
    /// Shutting down: only cleanup commands accepted; producers
    /// short-circuit with no-op events.
    ShuttingDown,
}

/// State owned by the host reducer.
///
/// Held inside `AppState.host_state: parking_lot::Mutex<HostState>`.
/// Locked briefly by `host_dispatch`; never held across CEF callbacks
/// or `SendMessage` (snapshot-and-drop discipline — see spec §6).
pub struct HostState {
    /// FIFO queue of pre-create handoffs. Pushed by callers
    /// (`pane/creation.rs`, `commands/window.rs::open_new_window`,
    /// `commands/drag.rs::tear_off`, `commands/window_pool.rs::spawn_pool_window`)
    /// before `post_create_window`. Popped by `client.rs::on_after_created`
    /// when CEF reports a new browser. Peeked at the back by
    /// `wrr/win_event.rs::handle_event` to label OS-level WM_CREATE
    /// events with the upcoming label.
    ///
    /// Invariants:
    /// - At most one entry per (in-flight) browser create.
    /// - `on_after_created` always pops the head it expects to find.
    /// - The "main" window is special-cased in `on_after_created` and
    ///   never has a corresponding entry here.
    pub pending_window_creations: VecDeque<PendingWindowCreation>,

    // ── Phase H — added in PR #1 (h1-foundations); populated by reducer
    // arms below; no production callers yet. PRs #2-#5 wire each through
    // the a→e migration ratchet. See SPEC_HOST_REDUCER_5PR_PLAN_2026-05-02.md.

    /// H.1 — pane lifecycle map. Replaces the deleted
    /// `pane::lifecycle::PaneStateMachine`. Keyed by `block_id`.
    /// Authoritative; `BrowserPaneManager` (browser_panes.rs) is now a
    /// zero-sized handle that delegates all mutations through
    /// `host_dispatch`.
    pub browser_panes: HashMap<String, BrowserPaneEntry>,

    /// H.2 — browser handle registry. Replaces the deleted
    /// `AppState.browsers: Mutex<HashMap<String, Browser>>`. Keyed by
    /// label (e.g., `window-...`, `browser-pane-...`, `window-pool-...`).
    /// Authoritative; read via `AppState::get_browser`, `list_browsers`, etc.
    pub browsers: HashMap<String, BrowserHandle>,

    /// H.3 — active drag session (singleton). Replaces the deleted
    /// `AppState.active_drag: Mutex<Option<DragSession>>`.
    pub active_drag: Option<DragSession>,

    /// H.4 — pool state (queue + unpromoted + in-flight semaphore +
    /// just_promoted_labels bridge from PR #708). Replaces the deleted
    /// `window_pool` / `unpromoted_pool_labels` fields on AppState.
    pub pool: PoolState,

    /// Pane pool state — pre-warmed frameless floating-pane windows
    /// (`floating-pool-{uuid}`). Mirrors `pool` but for the pane pool.
    pub pane_pool: PanePoolState,

    /// H.5 — quit lifecycle. Replaces the deleted
    /// `AppState.is_quitting: AtomicBool`.
    pub quit_state: QuitState,

    /// H.5 — quit arming (SPEC_PILLAR2_SANITIZE_THEN_DECIDE_2026_07_11.md §1.E).
    /// Monotonic: set by `RegisterBrowser` on the first live user window
    /// (`TopLevel { is_pool: false }`), never cleared. `should_begin_drain`
    /// refuses to drain while false — the startup gap between process start and
    /// main's `on_after_created` has zero registered windows AND zero pending
    /// creations (main's creation path never enqueues one), so an unarmed
    /// `reconcile_quit` would otherwise hand `Some(LastWindowClosed)` to any
    /// quit-relevant dispatch in that window. The reducer-side analog of WRR's
    /// `HAD_VISIBLE_USER_WINDOW` (armed earlier — registration precedes SHOW —
    /// which is safe: once registered, the live count itself blocks drain).
    pub saw_live_user_window: bool,

    /// Background-service mode (Workstream 0, `SPEC_TRAY_OPTIONAL_BACKGROUND_SERVICE_2026_09_04.md`
    /// §7 Phase 1). When true, `should_begin_drain` never arms a
    /// `LastWindowClosed` drain — closing the last window hides the app
    /// instead of tearing down `host`/`srv`/`launcher`. Read once at
    /// process start from `AGENTMUX_BACKGROUND_SERVICE` (presence-based,
    /// matching the `AGENTMUX_DEV` idiom elsewhere in this crate); there is
    /// no live-toggle path yet. Defaults off, so today's close-quits-the-app
    /// behavior is unchanged for anyone who hasn't opted in. `wrr::win_event`'s
    /// quit watchdog (Windows) also reads this field (from the same locked
    /// `HostState`, alongside `registered`/`draining`) so it does not treat
    /// the resulting "0 registered, not draining" steady state as a desync
    /// and force-quit a few seconds after the last window closes.
    ///
    /// **Getting a window back (there is no tray icon yet — Workstream 1):**
    /// re-launching the app reopens one **on Windows and Linux**, via the
    /// existing single-instance forward. That keeps working at zero windows,
    /// verified end to end by reading each link: the launcher holds the
    /// single-instance pipe for its whole lifetime (so a second launch
    /// forwards rather than starting fresh); `lib.rs` deletes the
    /// `ipc-port-<hash>` forwarding hint only AFTER `run_message_loop()`
    /// returns, which this mode is precisely what prevents, so the hint
    /// survives; the host's IPC server is bound at startup independent of any
    /// window; and `open_new_window` still finds a warm pool, because the pool
    /// is only cascade-closed by `begin_drain_and_cascade`, which a suppressed
    /// drain never reaches. So the reopen is a pool promote, not a cold start.
    ///
    /// **macOS has a gap here — do not enable this mode there yet.**
    /// LaunchServices delivers a Finder/`open` relaunch as a reopen Apple
    /// Event to the running process instead of starting a second one, so
    /// recovery depends on `splash_mac.rs`'s
    /// `applicationShouldHandleReopen:` delegate, whose only install site is
    /// inside `Splash::show`. With the splash disabled there is no
    /// `NSApplication`, no pump, and no delegate — the reopen event has
    /// nowhere to land and no second process spawns to forward, leaving the
    /// user with no way back. See the design doc §7.5.1; it must be closed
    /// with the Workstream 1 macOS work.
    ///
    /// That is also the recipe for this workstream's own acceptance test,
    /// which has not been run live yet: enable the flag, close the last
    /// window, confirm `srv`/`launcher`/`host` survive in `tasklist`/`ps` and
    /// that an agent turn still completes with no window open, then
    /// re-launch the exe and confirm a window comes back.
    pub background_service_enabled: bool,

    /// Issue #2977 WS4 — is the instance currently running unobserved?
    ///
    /// Lives here, under the `host_state` lock, so the audit transition can be
    /// decided AND recorded atomically with the window-count change that
    /// caused it. Holding it anywhere else meant two concurrent dispatches
    /// could apply out of order and drop a transition.
    ///
    /// Seeded from the persisted log at startup, so an unattended period that
    /// began before a host crash is still correctly open.
    pub background_unattended: bool,

    /// H.6 — top-level window creation runner state (queue, in-flight,
    /// history). Event-driven; no watchdog. **Currently DORMANT** — the
    /// reducer arms (`EnqueueTopLevelWindow`, `TopLevelCallbackFired`,
    /// etc.) exist but no production code dispatches to them. The
    /// `ui_tasks::post_create_window` direct-call path is still
    /// authoritative. Wire-up is a low-priority structural improvement;
    /// see master spec §4.3 and discussion #707.
    #[allow(dead_code)]
    pub top_level_creation: TopLevelCreationState,

    /// Per-window opacity state. Keyed by label, value is clamped [0.0, 1.0].
    /// Absent means fully opaque (1.0). Mutated by `SetWindowOpacity`; read by
    /// `get_window_opacity` and the restore path in app-init. Win32 side-effect
    /// (SetLayeredWindowAttributes) is applied by the IPC handler AFTER dispatch,
    /// not inside the reducer. See SPEC_PER_WINDOW_OPACITY_2026-05-14.md §7.1.
    pub window_opacities: HashMap<String, f32>,

    /// Pane-state reducer — per-floater OS-window placement, keyed by the
    /// floating-window LABEL (`floating-<uuid>`), NOT block_id. Floaters are
    /// tracked by window label everywhere (`window_hwnds`, the frontend
    /// `?windowLabel=` URL, the `on_before_close` teardown) and are NOT in
    /// `browser_panes` — so label is the correct key and the close-time
    /// eviction hangs off `on_before_close` (via
    /// `EvictFloatingPaneWindowState`), not the block_id `browser_panes`
    /// arms. Holds maximize/minimize state + the rect to restore to. Docked
    /// panes have no entry (their magnify is backend-owned). See
    /// SPEC_PANE_STATE_REDUCER_2026-05-28.md (REVISION 2026-05-29).
    pub pane_window_states: HashMap<String, PaneWindowState>,

    /// Browser-pane creates deferred because the block_id was still `Closing`
    /// at register time (old CEF Browser mid-teardown — e.g. redock
    /// re-creating the same block_id the floater is still closing). Keyed by
    /// block_id. Lives HERE in the reducer (not `AppState`) so the
    /// stash-on-`Closing` (in `TryRegisterBrowserPaneLive`) and the
    /// remove-on-close (in `CompleteBrowserPaneClose`/`DrainBrowserPaneByLabel`)
    /// are atomic under the single host_state lock — a separate `Mutex`
    /// allowed a TOCTOU where the close path replayed before the stash landed,
    /// orphaning it (reagent P1 on #1168). See
    /// docs/analysis/ANALYSIS_BROWSER_PANE_REDOCK_LOAD_RACE_2026_05_29.md.
    pub pending_browser_pane_creates: HashMap<String, crate::state::PendingBrowserPaneCreate>,

    /// Lifecycle phase. `Running` is the operating state; the others
    /// gate command acceptance.
    pub lifecycle: HostLifecyclePhase,

    /// Monotonic event-version counter (per host-process run). Same
    /// invariant as launcher / srv reducers.
    pub event_version: u64,
}

impl Default for HostState {
    fn default() -> Self {
        Self {
            pending_window_creations: VecDeque::new(),
            // Phase H foundations (PR #1) — empty defaults; populated as
            // PRs #2-#5 wire callers through the reducer.
            browser_panes: HashMap::new(),
            browsers: HashMap::new(),
            active_drag: None,
            pool: PoolState::default(),
            pane_pool: PanePoolState::default(),
            quit_state: QuitState::default(),
            saw_live_user_window: false,
            background_service_enabled: std::env::var("AGENTMUX_BACKGROUND_SERVICE").is_ok(),
            background_unattended: false,
            top_level_creation: TopLevelCreationState::default(),
            window_opacities: HashMap::new(),
            pane_window_states: HashMap::new(),
            pending_browser_pane_creates: HashMap::new(),
            // Boot directly into Running — nothing in F.1 needs the
            // pre-init guard yet. Future PRs (drag, tear-off hooks)
            // will move boot through Bootstrapping → Running.
            lifecycle: HostLifecyclePhase::Running,
            event_version: 0,
        }
    }
}

impl HostState {
    /// Allocate the next event version. Called inside reducer arms
    /// when emitting an event.
    fn bump_version(&mut self) -> u64 {
        self.event_version += 1;
        self.event_version
    }
}

// ── Pane label generator (replaces pane/lifecycle.rs::BROWSER_PANE_LABEL_SEQ) ──────
//
// Monotonic counter appended to every pane label so a close-then-recreate of
// the same block_id doesn't collide: if the old browser's `on_before_close`
// fires after the new pane's create has already run, `DrainBrowserPaneByLabel`
// would otherwise find and wipe the NEW entry.
pub(super) static BROWSER_PANE_LABEL_SEQ: AtomicU64 = AtomicU64::new(1);

pub(super) fn next_browser_pane_label(block_id: &str) -> String {
    let seq = BROWSER_PANE_LABEL_SEQ.fetch_add(1, Ordering::Relaxed);
    format!("browser-pane-{}-{}", block_id, seq)
}

/// Outcome of `TryRegisterBrowserPaneLive`. Returned via
/// `DispatchOutput::browser_pane_register_result`. Same three-way semantics as the
/// pre-Phase-H `pane::lifecycle::PaneStateMachine::try_register_live` returned
/// — caller decides whether to start a fresh CEF create, re-navigate the
/// existing browser, or reject.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegisterResult {
    /// No prior entry; reducer inserted a new `Live` pane under `label`.
    /// Caller should post `CreateBrowserPaneTask` for this label.
    Fresh(String),
    /// Entry already existed and is `Live`; caller should re-navigate the
    /// existing browser at `label`.
    AlreadyLive(String),
    /// Entry already existed, is `Live`, but in a DIFFERENT window than the
    /// create request targets (tear-off / redock). Carries the existing
    /// `label`. The reducer has stashed the pending create; the caller must
    /// `close()` the old pane — its close-completion replays the stashed
    /// create as `Fresh` in the requested window. Re-navigating in place (the
    /// `AlreadyLive` behavior) would leave the requested window black.
    AlreadyLiveElsewhere(String),
    /// Entry exists and is `Closing`; caller must reject the re-create
    /// because the old browser's `on_before_close` will drain the entry,
    /// and overwriting now would lose the new entry instead of the old.
    Closing,
}

/// Commands handled by the host reducer.
///
/// Manual `Debug` impl below because `RegisterBrowser` carries a
/// `cef::Browser` which doesn't impl Debug.
#[derive(Clone)]
pub enum HostCommand {
    /// Append a pending-window-creation handoff. Producer side of the
    /// `pending_window_creations` queue (replaces the four direct
    /// `state.pending_window_creations.lock().push_back(...)` sites).
    EnqueuePendingWindowCreation { entry: PendingWindowCreation },

    /// Pop the head of `pending_window_creations`. Returns the popped
    /// entry via `HostEvent::PendingWindowDequeued`, or
    /// `HostEvent::PendingWindowQueueEmpty` if the queue was empty.
    /// Consumer side of the queue (replaces
    /// `client.rs::on_after_created`'s direct `pop_front`).
    DequeuePendingWindowCreation,

    // ── H.1 — pane lifecycle ────────────────────────────────────────────

    /// Caller (pane create code) requests a new pane lifecycle entry.
    /// Reducer inserts with `Live`. Reject if `block_id` already present.
    EnqueueBrowserPaneCreate { block_id: String, label: String },

    /// PR #5 — sole pane registration entry point post-H.1.d.
    ///
    /// Replaces `pane::lifecycle::PaneStateMachine::try_register_live`.
    /// Reducer generates the label internally (via `next_browser_pane_label`) so
    /// label assignment is atomic with the entry insert. Returns the
    /// outcome via `DispatchOutput::browser_pane_register_result`:
    ///   - `Fresh(label)`: new `Live` entry inserted; caller posts CreateBrowserPaneTask
    ///   - `AlreadyLive(label)`: caller should re-navigate existing browser
    ///   - `Closing`: old teardown still in flight. The reducer stashes
    ///     `pending` (if `Some`) in `pending_browser_pane_creates` so the
    ///     close-completion arm can replay it — atomically, under this same
    ///     lock (no TOCTOU with a separate map). Caller returns Ok (deferred).
    /// `pending` carries the create params (url/rect/window_label) to stash on
    /// `Closing`; ignored for `Fresh`/`AlreadyLive`.
    TryRegisterBrowserPaneLive {
        block_id: String,
        pending: Option<crate::state::PendingBrowserPaneCreate>,
    },

    /// CEF on_after_created fired for a pane browser; confirm it's Live.
    /// No-op if already Live or absent (idempotent against late callbacks).
    CompleteBrowserPaneCreate { block_id: String },

    /// Caller requests pane close. Reducer flips entry to `Closing` and
    /// returns the entry's label via `DispatchOutput::closed_browser_pane_label`
    /// iff the transition actually fired (was `Live`). Returns `None` for
    /// missing or already-Closing entries (idempotent).
    EnqueueBrowserPaneClose { block_id: String },

    /// CEF on_before_close fired for a pane; remove entry from map.
    CompleteBrowserPaneClose { block_id: String },

    /// PR #5 — sole label-keyed drain entry point post-H.1.d.
    ///
    /// Replaces `pane::lifecycle::PaneStateMachine::drain_by_label`. Used
    /// by `BrowserPaneManager::drain_closed_label` when CEF's
    /// `on_before_close` fires for a pane. Removes the entry whose `label`
    /// matches; returns the drained `block_id` via
    /// `DispatchOutput::drained_browser_pane_block_id` so the caller can also dispatch
    /// any block_id-keyed cleanup. Idempotent (None if no match).
    DrainBrowserPaneByLabel { label: String },

    /// Pane creation failed before reaching Live (e.g., CEF callback
    /// never fired, browser host returned 0). Reducer removes entry.
    AbortBrowserPaneCreate { block_id: String, reason: String },

    // ── H.2 — browser handle registry ───────────────────────────────────

    /// Insert browser into `browsers` map. Caller is on the CEF UI thread
    /// (e.g., client.rs::on_after_created). Reject (with Error) if label
    /// already present (collision indicates a bug).
    RegisterBrowser {
        label: String,
        browser: Browser,
        kind: BrowserKind,
    },

    /// Remove browser from `browsers` map. Idempotent; no-op if absent.
    UnregisterBrowser { label: String },

    /// Rename a browser entry `old_label` → `new_label`, re-keying the
    /// per-label host state that persists for the window's life
    /// (`browsers` + the duplicated `BrowserHandle.label`, and, if present,
    /// `window_opacities` / `pane_window_states`). Used when a pane-pool
    /// window (`floating-pool-*`) is promoted into a user floating pane
    /// (`floating-*`). Errors if `old_label` is absent or `new_label`
    /// already exists. `window_meta` / `window_hwnds` live on `AppState`
    /// and are re-keyed by the caller. See
    /// `SPEC_FLOATING_PANE_POOL_RELABEL_2026_06_30`.
    RelabelBrowser { old_label: String, new_label: String },

    // ── H.3 — drag state ────────────────────────────────────────────────

    /// Begin a cross-window drag session. Reject if one is already
    /// active (singleton invariant).
    StartDrag { session: DragSession },

    /// End the active drag. `drag_id` must match the current session.
    EndDrag { drag_id: String, outcome: DragOutcome },

    // ── H.4 — pool state ────────────────────────────────────────────────

    /// Pool window spawn started. Adds label to `unpromoted` set; sets
    /// `respawn_in_flight = true` if not already.
    PoolWindowSpawnStart { label: String },

    /// Frontend signaled the pool window's renderer is fully initialized.
    /// Move from `unpromoted` to `queue`; clear `respawn_in_flight`.
    PoolWindowReady { label: String },

    /// Pool window was destroyed before renderer-ready (e.g., user
    /// closed it externally during pre-warm). Remove from `unpromoted`,
    /// clear `respawn_in_flight`. Reducer may emit a refill effect.
    PoolWindowDestroyedBeforePromote { label: String },

    /// Promote a pool window into a user-visible top-level. Removes from
    /// `queue` and `unpromoted`, marks the corresponding `BrowserHandle`
    /// as `is_pool: false`.
    PromotePoolWindow { label: String },

    /// Round 6 (pool demote) — a promoted pool window is being closed;
    /// return it to the pool instead of destroying it (CEF 148 parks the
    /// browser on every destroy sequence — see
    /// retro-window-lifecycle-leak-2026-07-04). Flips the browser handle
    /// back to `is_pool: true` and re-inserts the label into `unpromoted`;
    /// the queue re-entry then rides the normal `PoolWindowReady`
    /// handshake after the caller reloads the window to its pool boot URL.
    DemotePoolWindow { label: String },

    /// PR #5 H.4 — atomic pop+promote front of pool queue. Returns the
    /// popped label via `DispatchOutput::promoted_pool_label`, or None
    /// if the queue is empty. Replaces the legacy
    /// `state.window_pool.lock().pop_front() + state.unpromoted_pool_labels.lock().remove`
    /// pair in `promote_pool_window`.
    PopAndPromoteFrontPoolWindow,

    /// Drain all pool windows on shutdown. Idempotent.
    PoolDrainAll,

    // ── Pane pool (floating-pool-{uuid}, frameless=true) ────────────────
    /// Pane pool window spawn started. Adds label to pane_pool.unpromoted
    /// and sets respawn_in_flight=true (single-flight semaphore).
    PanePoolWindowSpawnStart { label: String },
    /// Frontend signalled pane pool window renderer ready. Move from
    /// unpromoted to queue; clear respawn_in_flight.
    PanePoolWindowReady { label: String },
    /// Pane pool window destroyed before promote (renderer crash, OS close).
    PanePoolWindowDestroyedBeforePromote { label: String },
    /// Atomic pop+promote front of pane pool queue.
    /// Returns promoted label via `DispatchOutput::promoted_pane_pool_label`.
    PopAndPromoteFrontPanePoolWindow,
    /// Atomic pop of the front of the pane pool queue for memory-pressure
    /// eviction (issue #2218, B.5 Part 1) — NOT promotion; distinct from
    /// `PopAndPromoteFrontPanePoolWindow` above so eviction and a real
    /// tear-off can never race for the same front label (reagent P2).
    /// Returns the popped label via `DispatchOutput::evicted_pane_pool_label`.
    PopFrontPanePoolWindowForEviction,

    // ── H.5 — quit lifecycle ────────────────────────────────────────────

    /// Transition Running → Draining. Suppresses pool refills, awaits
    /// drain completion.
    BeginDrain { reason: QuitReason },

    /// All drainable resources are gone (pool empty, browsers empty).
    /// Transition Draining → Quit.
    ConfirmDrained,

    /// Pure no-op poke: mutates nothing, but is quit-relevant, so `update`
    /// recomputes `reconcile_quit` and surfaces it via
    /// `DispatchOutput::request_drain`. Exists for executors that reach a
    /// decision point with zero state-changing dispatches to ride (the
    /// level-trigger needs an edge): `orphan_reconcile`'s
    /// nothing-to-close-but-maybe-drain arm. NOT a polling mechanism — never
    /// wire it into hot paths. SPEC_PILLAR2_SANITIZE_THEN_DECIDE §1.H.
    ReconcileQuit,

    // ── H.6 — top-level window creation runner ──────────────────────────

    /// Caller requests a top-level window. Reducer either:
    /// - rejects (User-initiated + busy) with Error; caller propagates
    ///   visible error to frontend.
    /// - queues (Background) for later auto-advance.
    /// - starts immediately (idle slot) and emits `Effect::PostCreateWindow`.
    EnqueueTopLevelWindow { request: TopLevelCreationRequest },

    /// CEF on_after_created fired for `label`. If matches in-flight,
    /// mark Completed and advance queue. If doesn't match (orphan from
    /// stale state), emit `Effect::CloseOrphanBrowser`.
    TopLevelCallbackFired { label: String },

    /// CEF on_render_process_terminated fired for the renderer process
    /// associated with `label`. If matches in-flight, mark Failed and
    /// advance queue.
    TopLevelRendererTerminated { label: String, status: String },

    /// CEF on_before_close fired for `label` while still in-flight.
    /// User or external code closed the window mid-creation. Mark Failed.
    TopLevelExternallyClosed { label: String },

    // ── Opacity ─────────────────────────────────────────────────────────────

    /// Set per-window opacity. Reducer stores the clamped value in
    /// `window_opacities`; the IPC handler applies the Win32 side-effect
    /// after `host_dispatch` returns (pure reducer, no I/O inside).
    SetWindowOpacity { label: String, opacity: f32 },

    // ── Pane window-placement (pane-state reducer) ───────────────────────────

    /// Toggle a FLOATING pane's OS-window maximize (Normal ↔ Maximized),
    /// keyed by the floating-window `label` (`floating-<uuid>`). The
    /// floating half of the shared maximize button (spec §3.3a). The
    /// reducer flips `pane_window_states[label].placement` and emits
    /// `PaneWindowStateChanged`; the IPC handler applies the
    /// `ShowWindow(SW_MAXIMIZE/SW_RESTORE)` side-effect AFTER dispatch
    /// (pure reducer, no I/O) by resolving `window_hwnds[label]`. Docked
    /// panes never reach here — magnify is routed frontend-side to the
    /// backend.
    ///
    /// `current_rect` is the floater's live screen rect at click time (read
    /// by the IPC handler before dispatch). On a Normal→Maximized flip the
    /// reducer stashes it as `last_known_normal_rect` so the later restore
    /// has a rect to return to — borderless `WS_POPUP` floaters have no
    /// usable native `WINDOWPLACEMENT`, so we track the normal rect
    /// ourselves rather than rely on `SW_RESTORE`.
    ToggleFloatingMaximize {
        label: String,
        current_rect: Option<crate::state::PaneRect>,
    },

    /// Evict a floater's window-placement entry. Dispatched from
    /// `on_before_close` (where `window_hwnds[label]` is also evicted) so
    /// placement state can never outlive the window. No-op if absent
    /// (non-floater windows have no entry). This is the label-keyed
    /// cleanup-on-close that replaces the earlier (incorrect) block_id
    /// co-eviction in the `browser_panes` arms — floaters aren't in
    /// `browser_panes`.
    EvictFloatingPaneWindowState { label: String },
}

impl std::fmt::Debug for HostCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostCommand::EnqueuePendingWindowCreation { entry } => f
                .debug_struct("EnqueuePendingWindowCreation")
                .field("entry", entry)
                .finish(),
            HostCommand::DequeuePendingWindowCreation => {
                f.write_str("DequeuePendingWindowCreation")
            }
            HostCommand::EnqueueBrowserPaneCreate { block_id, label } => f
                .debug_struct("EnqueueBrowserPaneCreate")
                .field("block_id", block_id)
                .field("label", label)
                .finish(),
            HostCommand::TryRegisterBrowserPaneLive { block_id, pending } => f
                .debug_struct("TryRegisterBrowserPaneLive")
                .field("block_id", block_id)
                .field("has_pending", &pending.is_some())
                .finish(),
            HostCommand::CompleteBrowserPaneCreate { block_id } => f
                .debug_struct("CompleteBrowserPaneCreate")
                .field("block_id", block_id)
                .finish(),
            HostCommand::EnqueueBrowserPaneClose { block_id } => f
                .debug_struct("EnqueueBrowserPaneClose")
                .field("block_id", block_id)
                .finish(),
            HostCommand::CompleteBrowserPaneClose { block_id } => f
                .debug_struct("CompleteBrowserPaneClose")
                .field("block_id", block_id)
                .finish(),
            HostCommand::DrainBrowserPaneByLabel { label } => f
                .debug_struct("DrainBrowserPaneByLabel")
                .field("label", label)
                .finish(),
            HostCommand::AbortBrowserPaneCreate { block_id, reason } => f
                .debug_struct("AbortBrowserPaneCreate")
                .field("block_id", block_id)
                .field("reason", reason)
                .finish(),
            HostCommand::RegisterBrowser { label, kind, .. } => f
                .debug_struct("RegisterBrowser")
                .field("label", label)
                .field("kind", kind)
                .field("browser", &"<cef::Browser>")
                .finish(),
            HostCommand::UnregisterBrowser { label } => f
                .debug_struct("UnregisterBrowser")
                .field("label", label)
                .finish(),
            HostCommand::RelabelBrowser { old_label, new_label } => f
                .debug_struct("RelabelBrowser")
                .field("old_label", old_label)
                .field("new_label", new_label)
                .finish(),
            HostCommand::StartDrag { session } => f
                .debug_struct("StartDrag")
                .field("drag_id", &session.drag_id)
                .field("source_window", &session.source_window)
                .finish(),
            HostCommand::EndDrag { drag_id, outcome } => f
                .debug_struct("EndDrag")
                .field("drag_id", drag_id)
                .field("outcome", outcome)
                .finish(),
            HostCommand::PoolWindowSpawnStart { label } => f
                .debug_struct("PoolWindowSpawnStart")
                .field("label", label)
                .finish(),
            HostCommand::PoolWindowReady { label } => f
                .debug_struct("PoolWindowReady")
                .field("label", label)
                .finish(),
            HostCommand::PoolWindowDestroyedBeforePromote { label } => f
                .debug_struct("PoolWindowDestroyedBeforePromote")
                .field("label", label)
                .finish(),
            HostCommand::PromotePoolWindow { label } => f
                .debug_struct("PromotePoolWindow")
                .field("label", label)
                .finish(),
            HostCommand::DemotePoolWindow { label } => f
                .debug_struct("DemotePoolWindow")
                .field("label", label)
                .finish(),
            HostCommand::PopAndPromoteFrontPoolWindow => f.write_str("PopAndPromoteFrontPoolWindow"),
            HostCommand::PoolDrainAll => f.write_str("PoolDrainAll"),
            HostCommand::PanePoolWindowSpawnStart { label } => f
                .debug_struct("PanePoolWindowSpawnStart")
                .field("label", label)
                .finish(),
            HostCommand::PanePoolWindowReady { label } => f
                .debug_struct("PanePoolWindowReady")
                .field("label", label)
                .finish(),
            HostCommand::PanePoolWindowDestroyedBeforePromote { label } => f
                .debug_struct("PanePoolWindowDestroyedBeforePromote")
                .field("label", label)
                .finish(),
            HostCommand::PopAndPromoteFrontPanePoolWindow => f.write_str("PopAndPromoteFrontPanePoolWindow"),
            HostCommand::PopFrontPanePoolWindowForEviction => f.write_str("PopFrontPanePoolWindowForEviction"),
            HostCommand::BeginDrain { reason } => f
                .debug_struct("BeginDrain")
                .field("reason", reason)
                .finish(),
            HostCommand::ConfirmDrained => f.write_str("ConfirmDrained"),
            HostCommand::ReconcileQuit => f.write_str("ReconcileQuit"),
            HostCommand::EnqueueTopLevelWindow { request } => f
                .debug_struct("EnqueueTopLevelWindow")
                .field("label", &request.label)
                .field("source", &request.source)
                .finish(),
            HostCommand::TopLevelCallbackFired { label } => f
                .debug_struct("TopLevelCallbackFired")
                .field("label", label)
                .finish(),
            HostCommand::TopLevelRendererTerminated { label, status } => f
                .debug_struct("TopLevelRendererTerminated")
                .field("label", label)
                .field("status", status)
                .finish(),
            HostCommand::TopLevelExternallyClosed { label } => f
                .debug_struct("TopLevelExternallyClosed")
                .field("label", label)
                .finish(),
            HostCommand::SetWindowOpacity { label, opacity } => f
                .debug_struct("SetWindowOpacity")
                .field("label", label)
                .field("opacity", opacity)
                .finish(),
            HostCommand::ToggleFloatingMaximize { label, current_rect } => f
                .debug_struct("ToggleFloatingMaximize")
                .field("label", label)
                .field("current_rect", current_rect)
                .finish(),
            HostCommand::EvictFloatingPaneWindowState { label } => f
                .debug_struct("EvictFloatingPaneWindowState")
                .field("label", label)
                .finish(),
        }
    }
}

/// Outcome of an ended drag session.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum DragOutcome {
    /// Drop completed successfully (block moved to target).
    Dropped { target_label: String },
    /// Drag cancelled by user (e.g., escape key, drop outside any target).
    Cancelled,
    /// Tear-off into a new window completed.
    TornOff { new_label: String },
}

/// Reason a pool window left the pool.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum PoolLeaveReason {
    /// Promoted into a user-visible top-level (tear-off, etc.).
    Promoted,
    /// Destroyed before promote (e.g., user closed externally).
    DestroyedBeforePromote,
    /// Drained on shutdown.
    DrainedOnShutdown,
}

/// Events emitted by the host reducer.
///
/// F.1 keeps these in-host: subscribers log them via tracing for
/// observability, but no IPC propagation. When a future PR adds a
/// wire-level consumer (host→launcher event for cross-process saga
/// observability, frontend dispatcher in E.6), that PR promotes the
/// relevant variants to `agentmux-common::ipc::Event`.
#[derive(Debug, Clone)]
pub enum HostEvent {
    /// A `PendingWindowCreation` was enqueued. Carries a snapshot of
    /// the current queue length so observers can spot pile-ups.
    PendingWindowEnqueued {
        label: String,
        queue_len_after: usize,
        version: u64,
    },

    /// A `PendingWindowCreation` was dequeued. The popped entry
    /// travels back to the caller; observers see only the label and
    /// post-pop queue length.
    PendingWindowDequeued {
        label: String,
        queue_len_after: usize,
        version: u64,
    },

    /// `DequeuePendingWindowCreation` ran on an empty queue. Caller
    /// is responsible for the fallback (the legacy code paths
    /// synthesize a UUID-labelled FullInstance entry).
    PendingWindowQueueEmpty { version: u64 },

    // ── H.1 — pane lifecycle events ─────────────────────────────────────

    BrowserPaneCreateRequested {
        block_id: String,
        label: String,
        version: u64,
    },
    BrowserPaneLive {
        block_id: String,
        label: String,
        version: u64,
    },
    BrowserPaneClosing {
        block_id: String,
        version: u64,
    },
    BrowserPaneClosed {
        block_id: String,
        version: u64,
    },
    BrowserPaneCreationFailed {
        block_id: String,
        reason: String,
        version: u64,
    },

    // ── H.2 — browser registry events ───────────────────────────────────

    BrowserRegistered {
        label: String,
        kind: BrowserKind,
        version: u64,
    },
    BrowserUnregistered {
        label: String,
        version: u64,
    },

    // ── H.3 — drag events ───────────────────────────────────────────────

    DragStarted {
        drag_id: String,
        source_window: String,
        version: u64,
    },
    DragEnded {
        drag_id: String,
        outcome: DragOutcome,
        version: u64,
    },

    // ── H.4 — pool events ───────────────────────────────────────────────

    PoolWindowEntered {
        label: String,
        queue_len_after: usize,
        version: u64,
    },
    PoolWindowLeft {
        label: String,
        queue_len_after: usize,
        reason: PoolLeaveReason,
        version: u64,
    },
    PoolEmpty { version: u64 },

    // ── H.5 — quit events ───────────────────────────────────────────────

    QuitDraining {
        reason: QuitReason,
        version: u64,
    },
    QuitReady { version: u64 },

    // ── H.6 — top-level creation events ─────────────────────────────────

    TopLevelCreationRequested {
        creation_id: u64,
        source: TopLevelSource,
        label: String,
        version: u64,
    },
    TopLevelCreationStarted {
        creation_id: u64,
        label: String,
        version: u64,
    },
    TopLevelCreationCompleted {
        creation_id: u64,
        label: String,
        latency_ms: u64,
        version: u64,
    },
    TopLevelCreationFailed {
        creation_id: u64,
        label: String,
        outcome: TopLevelCreationOutcome,
        version: u64,
    },
    TopLevelQueueLengthChanged {
        len: usize,
        version: u64,
    },

    // ── Opacity events ──────────────────────────────────────────────────

    /// Opacity set successfully. IPC handler applies Win32 side-effect.
    WindowOpacityApplied { label: String, opacity: f32, version: u64 },
    /// Opacity cleared (opacity >= 1.0 → remove WS_EX_LAYERED).
    WindowOpacityCleared { label: String, version: u64 },

    // ── Pane window-placement events (pane-state reducer, Phase 0) ───────

    /// A floating pane's OS-window placement changed (e.g. via
    /// `ToggleFloatingMaximize`). The IPC handler applies the matching Win32
    /// geometry AFTER dispatch — `SetWindowPos` to the monitor work area on
    /// maximize, or back to `restore_rect` on restore. No renderer subscribes:
    /// the floating maximize button is intentionally stateless (fixed icon),
    /// so the reducer is the single source of truth for placement. See
    /// SPEC_PANE_STATE_REDUCER_2026-05-28.md §3.4.
    PaneWindowStateChanged {
        label: String,
        placement: WindowPlacement,
        /// On a Maximized→Normal flip, the rect to restore the floater to
        /// (the `last_known_normal_rect` captured when it was maximized).
        /// `None` for Normal→Maximized (the handler computes the work area)
        /// or when no normal rect was ever recorded.
        restore_rect: Option<crate::state::PaneRect>,
        version: u64,
    },

    // ── Effect carrier ──────────────────────────────────────────────────

    /// Side-effect descriptor. The reducer emits these but never executes
    /// them; `AppState::host_dispatch_with_effects` is responsible for
    /// running each kind. See `EffectKind` for variants.
    Effect {
        effect: EffectKind,
        version: u64,
    },

    /// A command was rejected. Mirrors `Event::Error` in srv/launcher
    /// reducers — kept generic for future arms.
    Error { message: String, version: u64 },
}

/// Output bundle returned from the reducer.
///
/// Most arms communicate via `events` alone, but several arms have callers
/// that need an atomic value-returning op alongside the state mutation:
///
/// - `DequeuePendingWindowCreation` → `dequeued: Option<PendingWindowCreation>`
///   (`client.rs::on_after_created` needs the popped entry's fields to drive
///   `window_meta.insert` + `ReportWindowOpened`).
///
/// - `UnregisterBrowser` → `removed_browser: Option<Browser>` (the close
///   path in `browser_panes::AppStateCloseOps::take_browser_hwnd` needs
///   the Browser handle to extract its HWND for `DestroyWindow`. The
///   atomicity matters: see codex P2 PR #660 — separating get + dispatch
///   creates a window where concurrent readers can also resolve the
///   label and act on the closing handle).
///
/// - `TryRegisterBrowserPaneLive` → `browser_pane_register_result: Option<RegisterResult>`
///   (PR #5 H.1.d: `BrowserPaneManager::create` branches on
///   Fresh/AlreadyLive/Closing).
///
/// - `EnqueueBrowserPaneClose` → `closed_browser_pane_label: Option<String>` (PR #5
///   H.1.d: the close path needs the label to call `take_browser_hwnd`
///   without a separate `live_browser_pane_label` query that could race).
///
/// - `DrainBrowserPaneByLabel` → `drained_browser_pane_block_id: Option<String>` (PR #5
///   H.1.d: `drain_closed_label` needs the block_id to dispatch
///   `CompleteBrowserPaneClose`).
///
/// - `EndDrag` → `ended_drag_session: Option<DragSession>` (PR #5
///   H.3: `complete_cross_drag` / `cancel_cross_drag` need the
///   session payload to emit the renderer-side cross-drag-end event,
///   AND need the .is_some() signal to distinguish actual end vs
///   drag_id mismatch).
///
/// - `PoolWindowSpawnStart` → `pool_spawn_proceeding: bool` (PR #5
///   H.4: spawn_pool_window's single-flight semaphore. true = slot
///   acquired, caller proceeds with CEF spawn; false = suppressed
///   (already in flight, or QuitState != Running)).
///
/// - `PoolWindowReady` / `PoolWindowDestroyedBeforePromote` /
///   `PopAndPromoteFrontPoolWindow` → `pool_size_after: Option<usize>`
///   (PR #5 H.4: caller checks against POOL_TARGET_SIZE to decide
///   whether to trigger a refill).
///
/// - `PoolWindowDestroyedBeforePromote` → `pool_destroyed_was_unpromoted: bool`
///   (PR #5 H.4: caller gates pool-inventory reports on this — the
///   post-promote close path doesn't own that update).
///
/// - `PopAndPromoteFrontPoolWindow` → `promoted_pool_label: Option<String>`
///   (PR #5 H.4: caller needs the popped label to drive the CEF
///   show + emit the pool:promote frontend event).
///
/// Default keeps the dispatch return type uniform across arms that
/// don't populate these fields.
#[derive(Default)]
pub struct DispatchOutput {
    pub events: Vec<HostEvent>,
    pub dequeued: Option<PendingWindowCreation>,
    pub removed_browser: Option<Browser>,
    pub browser_pane_register_result: Option<RegisterResult>,
    pub closed_browser_pane_label: Option<String>,
    pub drained_browser_pane_block_id: Option<String>,
    /// A browser-pane create that was deferred (stashed on `Closing`) and is
    /// now eligible to replay, set by the close-completion arms
    /// (`CompleteBrowserPaneClose` / `DrainBrowserPaneByLabel`) when they
    /// remove the old entry. `(block_id, params)`. The IPC handler re-runs the
    /// create (now `Fresh`) and posts the `CreateBrowserPaneTask`.
    pub pending_browser_pane_create_to_replay:
        Option<(String, crate::state::PendingBrowserPaneCreate)>,
    pub ended_drag_session: Option<DragSession>,
    pub pool_spawn_proceeding: bool,
    pub pool_size_after: Option<usize>,
    pub pool_destroyed_was_unpromoted: bool,
    pub promoted_pool_label: Option<String>,
    /// Round 6 — set by `DemotePoolWindow` when the label was accepted back
    /// into the pool (`is_pool` flipped, inserted into `unpromoted`). False
    /// = already pool-side or unknown browser; caller falls back to the
    /// destroy path.
    pub pool_demote_accepted: bool,
    // Pane pool fields (parallel to tab pool above)
    pub pane_pool_spawn_proceeding: bool,
    pub pane_pool_size_after: Option<usize>,
    pub pane_pool_destroyed_was_unpromoted: bool,
    pub promoted_pane_pool_label: Option<String>,
    /// Set by `PopFrontPanePoolWindowForEviction` — the label atomically
    /// popped for memory-pressure eviction (issue #2218, B.5 Part 1), or
    /// `None` if the queue was already empty. Distinct from
    /// `promoted_pane_pool_label` so eviction and promotion can never be
    /// confused with each other (reagent P2).
    pub evicted_pane_pool_label: Option<String>,
    /// Pillar 2 (level-triggered quit) — set by `update()` after any quit-relevant
    /// command when the host should begin draining NOW (no live user window, no
    /// user creation in flight, still `Running`). The UI-thread drain executor
    /// consumes this and posts the Stage-1 cascade. `None` in the common case.
    /// This is the level-triggered replacement for the edge-triggered gate that
    /// previously lived only in `client::on_before_close` and could miss a
    /// concurrent pool-refill race (SPEC_PILLAR2_WIRE_RECONCILE_QUIT_2026_06_29.md).
    pub request_drain: Option<crate::state::QuitReason>,

    /// Issue #2977 WS4 — the instance crossed the attended/unattended
    /// boundary on this dispatch. `Some(true)` = the last window closed and
    /// the instance is now running unobserved; `Some(false)` = a window
    /// opened and it is observed again. `None` for everything else, including
    /// every dispatch when background-service mode is off (where zero windows
    /// means exiting, not resting).
    ///
    /// Consumed by `AppState::host_dispatch`, which records it into the
    /// background audit log for surfacing when a window next opens.
    pub background_attention: Option<bool>,

    /// Set `true` by `RegisterBrowser` when a live USER window registered
    /// while `QuitState` had already left `Running` — i.e. a window creation
    /// that was already in flight when a quit began (ReAgent P1 on PR #2996).
    ///
    /// The caller (`client::on_after_created`) MUST close that browser
    /// immediately. The pre-checks on the creation paths narrow this race but
    /// cannot close it — registration is the last step, so this is the only
    /// point that cannot be raced. Leaving it open means a live window
    /// stranded in a draining host, which the WRR watchdog then force-kills
    /// seconds later in front of the user.
    pub registered_during_drain: bool,

    /// Set `true` by `RelabelBrowser` when the rename succeeded (or was a
    /// no-op because `old_label == new_label`). Stays `false` on failure
    /// (old label absent, or new label already registered — e.g. a
    /// concurrent close removed the browser between promote and relabel).
    /// The caller MUST gate the AppState `window_hwnds`/`window_meta` re-key
    /// and the `pool:pane-promote` emit on this, else `browsers` and the
    /// AppState maps would disagree and the emit would resolve no browser.
    pub relabel_ok: bool,
}

// Manual Debug — `cef::Browser` doesn't impl Debug.
impl std::fmt::Debug for DispatchOutput {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DispatchOutput")
            .field("events", &self.events)
            .field("dequeued", &self.dequeued)
            .field(
                "removed_browser",
                if self.removed_browser.is_some() {
                    &"Some(<cef::Browser>)"
                } else {
                    &"None"
                },
            )
            .field("browser_pane_register_result", &self.browser_pane_register_result)
            .field("closed_browser_pane_label", &self.closed_browser_pane_label)
            .field("drained_browser_pane_block_id", &self.drained_browser_pane_block_id)
            .field("ended_drag_session", &self.ended_drag_session)
            .field("pool_spawn_proceeding", &self.pool_spawn_proceeding)
            .field("pool_size_after", &self.pool_size_after)
            .field("pool_destroyed_was_unpromoted", &self.pool_destroyed_was_unpromoted)
            .field("promoted_pool_label", &self.promoted_pool_label)
            .field("request_drain", &self.request_drain)
            .finish()
    }
}

/// Pure functional core of the host reducer.
///
/// Returns the events emitted by the command. Side-effecting wiring
/// (logging, future event broadcast) lives in `host_dispatch` — this
/// function takes only `&mut HostState` and produces no I/O.
mod browsers;
mod drag;
mod pane_pool;
mod pane_window;
mod panes;
mod pool;
mod quit;
mod top_level;

/// Shared with `AppState::count_live_user_windows` (the live last-window quit
/// gate) so the count has a single definition. (`is_live_user_window` stays
/// internal to `quit` — used by `count_live_user_windows` and its tests.)
/// Test-only re-export so `background_audit`'s tests can mirror the exact
/// decision `update` makes, rather than duplicating the rule and drifting.
#[cfg(test)]
pub(crate) fn background_attention_transition_for_test(
    enabled: bool,
    currently_unattended: bool,
    live_after: usize,
) -> Option<bool> {
    quit::background_attention_transition(enabled, currently_unattended, live_after)
}

pub(crate) use quit::{count_live_user_windows, live_user_window_labels};

/// Whether a command can change the quit decision's inputs — the live
/// user-window count (`browsers`), pending user-initiated creations, or
/// `quit_state`. **Negative guard:** only known high-frequency / clearly
/// quit-irrelevant commands are excluded, so any command NOT listed here
/// defaults to relevant. That fail-safe is deliberate — silently missing a
/// window/pool transition is exactly the edge-triggered bug Pillar 2 replaces,
/// whereas an extra cheap `reconcile_quit` read on an irrelevant command is
/// harmless (it just returns `None`). The excluded set is the genuine hot path
/// (drag-opacity ticks) plus the browser-pane lifecycle (panes live in the
/// separate `browser_panes` map and never affect `count_live_user_windows`).
fn is_quit_relevant(cmd: &HostCommand) -> bool {
    !matches!(
        cmd,
        HostCommand::SetWindowOpacity { .. }
            | HostCommand::StartDrag { .. }
            | HostCommand::EndDrag { .. }
            | HostCommand::ToggleFloatingMaximize { .. }
            | HostCommand::EvictFloatingPaneWindowState { .. }
            | HostCommand::EnqueueBrowserPaneCreate { .. }
            | HostCommand::TryRegisterBrowserPaneLive { .. }
            | HostCommand::CompleteBrowserPaneCreate { .. }
            | HostCommand::EnqueueBrowserPaneClose { .. }
            | HostCommand::CompleteBrowserPaneClose { .. }
            | HostCommand::DrainBrowserPaneByLabel { .. }
            | HostCommand::AbortBrowserPaneCreate { .. }
            | HostCommand::RelabelBrowser { .. }
    )
}

pub fn update(state: &mut HostState, cmd: HostCommand) -> DispatchOutput {
    let quit_relevant = is_quit_relevant(&cmd);

    let mut out = match cmd {
        HostCommand::EnqueuePendingWindowCreation { entry } => {
            handle_enqueue_pending_window_creation(state, entry)
        }
        HostCommand::DequeuePendingWindowCreation => {
            handle_dequeue_pending_window_creation(state)
        }
        // H.1 panes
        HostCommand::EnqueueBrowserPaneCreate { block_id, label } => {
            panes::handle_enqueue_browser_pane_create(state, block_id, label)
        }
        HostCommand::TryRegisterBrowserPaneLive { block_id, pending } => {
            panes::handle_try_register_browser_pane_live(state, block_id, pending)
        }
        HostCommand::CompleteBrowserPaneCreate { block_id } => {
            panes::handle_complete_browser_pane_create(state, block_id)
        }
        HostCommand::EnqueueBrowserPaneClose { block_id } => {
            panes::handle_enqueue_browser_pane_close(state, block_id)
        }
        HostCommand::CompleteBrowserPaneClose { block_id } => {
            panes::handle_complete_browser_pane_close(state, block_id)
        }
        HostCommand::DrainBrowserPaneByLabel { label } => {
            panes::handle_drain_browser_pane_by_label(state, label)
        }
        HostCommand::AbortBrowserPaneCreate { block_id, reason } => {
            panes::handle_abort_browser_pane_create(state, block_id, reason)
        }
        // H.2 browsers
        HostCommand::RegisterBrowser { label, browser, kind } => {
            browsers::handle_register_browser(state, label, browser, kind)
        }
        HostCommand::UnregisterBrowser { label } => {
            browsers::handle_unregister_browser(state, label)
        }
        HostCommand::RelabelBrowser { old_label, new_label } => {
            browsers::handle_relabel_browser(state, old_label, new_label)
        }
        // H.3 drag
        HostCommand::StartDrag { session } => drag::handle_start_drag(state, session),
        HostCommand::EndDrag { drag_id, outcome } => drag::handle_end_drag(state, drag_id, outcome),
        // H.4 pool
        HostCommand::PoolWindowSpawnStart { label } => pool::handle_pool_spawn_start(state, label),
        HostCommand::PoolWindowReady { label } => pool::handle_pool_ready(state, label),
        HostCommand::DemotePoolWindow { label } => pool::handle_demote_pool_window(state, label),
        HostCommand::PoolWindowDestroyedBeforePromote { label } => {
            pool::handle_pool_destroyed_before_promote(state, label)
        }
        HostCommand::PromotePoolWindow { label } => pool::handle_promote_pool_window(state, label),
        HostCommand::PopAndPromoteFrontPoolWindow => pool::handle_pop_and_promote_front_pool_window(state),
        HostCommand::PoolDrainAll => pool::handle_pool_drain_all(state),
        // Pane pool
        HostCommand::PanePoolWindowSpawnStart { label } => pane_pool::handle_pane_pool_spawn_start(state, label),
        HostCommand::PanePoolWindowReady { label } => pane_pool::handle_pane_pool_ready(state, label),
        HostCommand::PanePoolWindowDestroyedBeforePromote { label } => {
            pane_pool::handle_pane_pool_destroyed_before_promote(state, label)
        }
        HostCommand::PopAndPromoteFrontPanePoolWindow => pane_pool::handle_pop_and_promote_front_pane_pool_window(state),
        HostCommand::PopFrontPanePoolWindowForEviction => pane_pool::handle_pop_front_pane_pool_for_eviction(state),
        // H.5 quit
        HostCommand::BeginDrain { reason } => quit::handle_begin_drain(state, reason),
        HostCommand::ConfirmDrained => quit::handle_confirm_drained(state),
        // Pure poke — no state change; the quit-relevant recomputation below
        // does the only work this command exists for.
        HostCommand::ReconcileQuit => DispatchOutput::default(),
        // H.6 top-level runner
        HostCommand::EnqueueTopLevelWindow { request } => {
            top_level::handle_enqueue_top_level_window(state, request)
        }
        HostCommand::TopLevelCallbackFired { label } => {
            top_level::handle_top_level_callback_fired(state, label)
        }
        HostCommand::TopLevelRendererTerminated { label, status } => {
            top_level::handle_top_level_renderer_terminated(state, label, status)
        }
        HostCommand::TopLevelExternallyClosed { label } => {
            top_level::handle_top_level_externally_closed(state, label)
        }
        // Opacity
        HostCommand::SetWindowOpacity { label, opacity } => {
            handle_set_window_opacity(state, label, opacity)
        }
        // Pane window-placement (pane-state reducer)
        HostCommand::ToggleFloatingMaximize { label, current_rect } => {
            pane_window::handle_toggle_floating_maximize(state, label, current_rect)
        }
        HostCommand::EvictFloatingPaneWindowState { label } => {
            pane_window::handle_evict_floating_pane_window_state(state, label)
        }
    };
    // Pillar 2 — level-triggered quit reconciliation. After any transition that
    // can change the decision's inputs, recompute the pure `reconcile_quit` over
    // the resulting state. A drain that an edge-triggered close gate missed
    // (e.g. raced by a concurrent pool refill) is caught here, on the very next
    // transition that settles the count. Pure read; the actual close cascade is
    // posted to the UI thread by the consumer (never run inline — that would
    // deadlock, see client::on_before_close). `reconcile_quit` is monotonic:
    // once Draining/Quit it returns None, so this can't re-fire or loop.
    if quit_relevant {
        out.request_drain = quit::reconcile_quit(state);
        // WS4 audit log: report crossing the attended/unattended boundary, so
        // the host can record what the user missed and surface it when a
        // window next opens. Computed here rather than at the close/open call
        // sites for the same reason `request_drain` is — one place that sees
        // every transition, instead of N sites that each have to remember.
        out.background_attention = quit::background_attention_transition(
            state.background_service_enabled,
            state.background_unattended,
            quit::count_live_user_windows(state),
        );
        // Update the flag HERE, under the same lock that decided it, so the
        // decision and the state it is based on can never be applied out of
        // order by two concurrent dispatches.
        if let Some(now_unattended) = out.background_attention {
            state.background_unattended = now_unattended;
        }
    }
    out
}

fn handle_enqueue_pending_window_creation(
    state: &mut HostState,
    entry: PendingWindowCreation,
) -> DispatchOutput {
    if state.lifecycle == HostLifecyclePhase::ShuttingDown {
        // No new windows accepted during shutdown. Mirrors the
        // launcher reducer's shutdown gating from B.9.3.
        let v = state.bump_version();
        return DispatchOutput {
            events: vec![HostEvent::Error {
                message: "enqueue_pending_window_creation: host is shutting down".to_string(),
                version: v,
            }],
            ..Default::default()
        };
    }
    let label = entry.label.clone();
    state.pending_window_creations.push_back(entry);
    let queue_len_after = state.pending_window_creations.len();
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::PendingWindowEnqueued {
            label,
            queue_len_after,
            version: v,
        }],
        ..Default::default()
    }
}

fn handle_dequeue_pending_window_creation(state: &mut HostState) -> DispatchOutput {
    match state.pending_window_creations.pop_front() {
        Some(entry) => {
            let queue_len_after = state.pending_window_creations.len();
            let v = state.bump_version();
            let label = entry.label.clone();
            DispatchOutput {
                events: vec![HostEvent::PendingWindowDequeued {
                    label,
                    queue_len_after,
                    version: v,
                }],
                dequeued: Some(entry),
                ..Default::default()
            }
        }
        None => {
            let v = state.bump_version();
            DispatchOutput {
                events: vec![HostEvent::PendingWindowQueueEmpty { version: v }],
                ..Default::default()
            }
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────
// Phase H reducer arms — added in PR #1 (h1-foundations)
//
// All arms are pure: `&mut HostState` in, `DispatchOutput` (events) out.
// No I/O, no async, no logging (logging happens in `state::log_host_event`
// after dispatch returns). Reducer arms emit Effect events; the effect
// handler in `AppState::host_dispatch_with_effects` (added in PR #4)
// dispatches each Effect to its imperative handler.
// ─────────────────────────────────────────────────────────────────────────

pub(super) fn emit_error(state: &mut HostState, message: String) -> DispatchOutput {
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::Error { message, version: v }],
        ..Default::default()
    }
}

fn handle_set_window_opacity(state: &mut HostState, label: String, opacity: f32) -> DispatchOutput {
    let clamped = opacity.clamp(0.0, 1.0);
    let v = state.bump_version();
    if clamped >= 1.0 {
        state.window_opacities.remove(&label);
        DispatchOutput {
            events: vec![HostEvent::WindowOpacityCleared { label, version: v }],
            ..Default::default()
        }
    } else {
        state.window_opacities.insert(label.clone(), clamped);
        DispatchOutput {
            events: vec![HostEvent::WindowOpacityApplied { label, opacity: clamped, version: v }],
            ..Default::default()
        }
    }
}


#[cfg(test)]
mod tests;
