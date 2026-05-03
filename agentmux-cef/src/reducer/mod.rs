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
use std::time::Instant;

use cef::Browser;

use crate::state::{
    BrowserHandle, BrowserKind, CompletedCreation, CreationPhase, DragSession, EffectKind,
    InFlightCreation, BrowserPaneEntry, BrowserPaneLifecycle, PendingWindowCreation, PoolState, QuitReason,
    QuitState, TopLevelCreationOutcome, TopLevelCreationRequest, TopLevelCreationState,
    TopLevelSource,
};

/// Capacity of `TopLevelCreationState.history` ring buffer. Configurable
/// via `~/.agentmux/config.toml [host.reducer]` once H.5 (config) lands;
/// hard-coded for PR #1.
pub const TOP_LEVEL_CREATION_HISTORY_CAP: usize = 50;

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

    /// H.1 — pane lifecycle map. Replaces `BrowserPaneManager::PaneStateMachine`
    /// (pane/lifecycle.rs:60). Keyed by `block_id`.
    #[allow(dead_code)]
    pub browser_panes: HashMap<String, BrowserPaneEntry>,

    /// H.2 — browser handle registry. Replaces `AppState.browsers:
    /// Mutex<HashMap<String, Browser>>` (state.rs:210). Keyed by label
    /// (e.g., `window-...`, `browser-pane-...`, `window-pool-...`).
    #[allow(dead_code)]
    pub browsers: HashMap<String, BrowserHandle>,

    /// H.3 — active drag session (singleton). Replaces `AppState.active_drag:
    /// Mutex<Option<DragSession>>`.
    #[allow(dead_code)]
    pub active_drag: Option<DragSession>,

    /// H.4 — pool state (queue + unpromoted + in-flight semaphore).
    /// Replaces three separate fields on AppState.
    #[allow(dead_code)]
    pub pool: PoolState,

    /// H.5 — quit lifecycle. Replaces `AppState.is_quitting: AtomicBool`.
    #[allow(dead_code)]
    pub quit_state: QuitState,

    /// H.6 — top-level window creation runner state (queue, in-flight,
    /// history). Event-driven; no watchdog.
    #[allow(dead_code)]
    pub top_level_creation: TopLevelCreationState,

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
            quit_state: QuitState::default(),
            top_level_creation: TopLevelCreationState::default(),
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
static BROWSER_PANE_LABEL_SEQ: AtomicU64 = AtomicU64::new(1);

fn next_browser_pane_label(block_id: &str) -> String {
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
    ///   - `Closing`: caller must reject (old teardown still in flight)
    TryRegisterBrowserPaneLive { block_id: String },

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

    /// PR #5 H.4 — atomic pop+promote front of pool queue. Returns the
    /// popped label via `DispatchOutput::promoted_pool_label`, or None
    /// if the queue is empty. Replaces the legacy
    /// `state.window_pool.lock().pop_front() + state.unpromoted_pool_labels.lock().remove`
    /// pair in `promote_pool_window`.
    PopAndPromoteFrontPoolWindow,

    /// Drain all pool windows on shutdown. Idempotent.
    PoolDrainAll,

    // ── H.5 — quit lifecycle ────────────────────────────────────────────

    /// Transition Running → Draining. Suppresses pool refills, awaits
    /// drain completion.
    BeginDrain { reason: QuitReason },

    /// All drainable resources are gone (pool empty, browsers empty).
    /// Transition Draining → Quit.
    ConfirmDrained,

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
            HostCommand::TryRegisterBrowserPaneLive { block_id } => f
                .debug_struct("TryRegisterBrowserPaneLive")
                .field("block_id", block_id)
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
            HostCommand::PopAndPromoteFrontPoolWindow => f.write_str("PopAndPromoteFrontPoolWindow"),
            HostCommand::PoolDrainAll => f.write_str("PoolDrainAll"),
            HostCommand::BeginDrain { reason } => f
                .debug_struct("BeginDrain")
                .field("reason", reason)
                .finish(),
            HostCommand::ConfirmDrained => f.write_str("ConfirmDrained"),
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
    pub ended_drag_session: Option<DragSession>,
    pub pool_spawn_proceeding: bool,
    pub pool_size_after: Option<usize>,
    pub pool_destroyed_was_unpromoted: bool,
    pub promoted_pool_label: Option<String>,
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
            .finish()
    }
}

/// Pure functional core of the host reducer.
///
/// Returns the events emitted by the command. Side-effecting wiring
/// (logging, future event broadcast) lives in `host_dispatch` — this
/// function takes only `&mut HostState` and produces no I/O.
pub fn update(state: &mut HostState, cmd: HostCommand) -> DispatchOutput {
    match cmd {
        HostCommand::EnqueuePendingWindowCreation { entry } => {
            handle_enqueue_pending_window_creation(state, entry)
        }
        HostCommand::DequeuePendingWindowCreation => {
            handle_dequeue_pending_window_creation(state)
        }
        // H.1 panes
        HostCommand::EnqueueBrowserPaneCreate { block_id, label } => {
            handle_enqueue_browser_pane_create(state, block_id, label)
        }
        HostCommand::TryRegisterBrowserPaneLive { block_id } => {
            handle_try_register_browser_pane_live(state, block_id)
        }
        HostCommand::CompleteBrowserPaneCreate { block_id } => {
            handle_complete_browser_pane_create(state, block_id)
        }
        HostCommand::EnqueueBrowserPaneClose { block_id } => {
            handle_enqueue_browser_pane_close(state, block_id)
        }
        HostCommand::CompleteBrowserPaneClose { block_id } => {
            handle_complete_browser_pane_close(state, block_id)
        }
        HostCommand::DrainBrowserPaneByLabel { label } => {
            handle_drain_browser_pane_by_label(state, label)
        }
        HostCommand::AbortBrowserPaneCreate { block_id, reason } => {
            handle_abort_browser_pane_create(state, block_id, reason)
        }
        // H.2 browsers
        HostCommand::RegisterBrowser { label, browser, kind } => {
            handle_register_browser(state, label, browser, kind)
        }
        HostCommand::UnregisterBrowser { label } => {
            handle_unregister_browser(state, label)
        }
        // H.3 drag
        HostCommand::StartDrag { session } => handle_start_drag(state, session),
        HostCommand::EndDrag { drag_id, outcome } => handle_end_drag(state, drag_id, outcome),
        // H.4 pool
        HostCommand::PoolWindowSpawnStart { label } => handle_pool_spawn_start(state, label),
        HostCommand::PoolWindowReady { label } => handle_pool_ready(state, label),
        HostCommand::PoolWindowDestroyedBeforePromote { label } => {
            handle_pool_destroyed_before_promote(state, label)
        }
        HostCommand::PromotePoolWindow { label } => handle_promote_pool_window(state, label),
        HostCommand::PopAndPromoteFrontPoolWindow => handle_pop_and_promote_front_pool_window(state),
        HostCommand::PoolDrainAll => handle_pool_drain_all(state),
        // H.5 quit
        HostCommand::BeginDrain { reason } => handle_begin_drain(state, reason),
        HostCommand::ConfirmDrained => handle_confirm_drained(state),
        // H.6 top-level runner
        HostCommand::EnqueueTopLevelWindow { request } => {
            handle_enqueue_top_level_window(state, request)
        }
        HostCommand::TopLevelCallbackFired { label } => {
            handle_top_level_callback_fired(state, label)
        }
        HostCommand::TopLevelRendererTerminated { label, status } => {
            handle_top_level_renderer_terminated(state, label, status)
        }
        HostCommand::TopLevelExternallyClosed { label } => {
            handle_top_level_externally_closed(state, label)
        }
    }
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

fn emit_error(state: &mut HostState, message: String) -> DispatchOutput {
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::Error { message, version: v }],
        ..Default::default()
    }
}

// ── H.1 — pane lifecycle ─────────────────────────────────────────────────

fn handle_enqueue_browser_pane_create(
    state: &mut HostState,
    block_id: String,
    label: String,
) -> DispatchOutput {
    if state.lifecycle == HostLifecyclePhase::ShuttingDown {
        return emit_error(state, format!("enqueue_browser_pane_create: shutting down (block_id={})", block_id));
    }
    if state.browser_panes.contains_key(&block_id) {
        return emit_error(state, format!("enqueue_browser_pane_create: block_id {} already has a pane", block_id));
    }
    state.browser_panes.insert(
        block_id.clone(),
        BrowserPaneEntry {
            block_id: block_id.clone(),
            label: label.clone(),
            lifecycle: BrowserPaneLifecycle::Live,
        },
    );
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::BrowserPaneCreateRequested { block_id, label, version: v }],
        ..Default::default()
    }
}

fn handle_complete_browser_pane_create(state: &mut HostState, block_id: String) -> DispatchOutput {
    let entry = match state.browser_panes.get(&block_id) {
        Some(e) => e.clone(),
        None => return DispatchOutput::default(), // late callback for already-removed pane; idempotent no-op
    };
    // Already Live by EnqueueBrowserPaneCreate's invariant; this is a no-op
    // confirmation event for observers.
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::BrowserPaneLive { block_id, label: entry.label, version: v }],
        ..Default::default()
    }
}

fn handle_enqueue_browser_pane_close(state: &mut HostState, block_id: String) -> DispatchOutput {
    let entry = match state.browser_panes.get_mut(&block_id) {
        Some(e) => e,
        None => return DispatchOutput::default(), // close request for already-gone pane; idempotent
    };
    if matches!(entry.lifecycle, BrowserPaneLifecycle::Closing { .. }) {
        return DispatchOutput::default(); // already Closing; idempotent
    }
    entry.lifecycle = BrowserPaneLifecycle::Closing { since: Instant::now() };
    let label = entry.label.clone();
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::BrowserPaneClosing { block_id, version: v }],
        closed_browser_pane_label: Some(label),
        ..Default::default()
    }
}

/// PR #5 — sole pane registration entry point. Replaces
/// `pane::lifecycle::PaneStateMachine::try_register_live`.
///
/// - Live entry exists → `AlreadyLive(label)`
/// - Closing entry exists → `Closing`
/// - No entry → generate label, insert Live, `Fresh(label)` + emit
///   `BrowserPaneCreateRequested`
fn handle_try_register_browser_pane_live(state: &mut HostState, block_id: String) -> DispatchOutput {
    if state.lifecycle == HostLifecyclePhase::ShuttingDown {
        return emit_error(
            state,
            format!("try_register_browser_pane_live: shutting down (block_id={})", block_id),
        );
    }
    if let Some(entry) = state.browser_panes.get(&block_id) {
        let result = match entry.lifecycle {
            BrowserPaneLifecycle::Live => RegisterResult::AlreadyLive(entry.label.clone()),
            BrowserPaneLifecycle::Closing { .. } => RegisterResult::Closing,
        };
        return DispatchOutput {
            browser_pane_register_result: Some(result),
            ..Default::default()
        };
    }
    let label = next_browser_pane_label(&block_id);
    state.browser_panes.insert(
        block_id.clone(),
        BrowserPaneEntry {
            block_id: block_id.clone(),
            label: label.clone(),
            lifecycle: BrowserPaneLifecycle::Live,
        },
    );
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::BrowserPaneCreateRequested {
            block_id,
            label: label.clone(),
            version: v,
        }],
        browser_pane_register_result: Some(RegisterResult::Fresh(label)),
        ..Default::default()
    }
}

/// PR #5 — sole label-keyed drain entry point. Replaces
/// `pane::lifecycle::PaneStateMachine::drain_by_label`.
///
/// Removes whichever pane entry has `label`. Returns the drained block_id
/// in `drained_browser_pane_block_id`. Idempotent — `None` if no entry has that label
/// (e.g., explicit `close()` already cleared it; `on_before_close` arrives
/// later).
fn handle_drain_browser_pane_by_label(state: &mut HostState, label: String) -> DispatchOutput {
    let victim = state
        .browser_panes
        .iter()
        .find(|(_, e)| e.label == label)
        .map(|(k, _)| k.clone());
    let block_id = match victim {
        Some(b) => b,
        None => return DispatchOutput::default(),
    };
    state.browser_panes.remove(&block_id);
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::BrowserPaneClosed {
            block_id: block_id.clone(),
            version: v,
        }],
        drained_browser_pane_block_id: Some(block_id),
        ..Default::default()
    }
}

fn handle_complete_browser_pane_close(state: &mut HostState, block_id: String) -> DispatchOutput {
    if state.browser_panes.remove(&block_id).is_none() {
        return DispatchOutput::default(); // idempotent
    }
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::BrowserPaneClosed { block_id, version: v }],
        ..Default::default()
    }
}

fn handle_abort_browser_pane_create(
    state: &mut HostState,
    block_id: String,
    reason: String,
) -> DispatchOutput {
    state.browser_panes.remove(&block_id);
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::BrowserPaneCreationFailed { block_id, reason, version: v }],
        ..Default::default()
    }
}

// ── H.2 — browser handle registry ────────────────────────────────────────

fn handle_register_browser(
    state: &mut HostState,
    label: String,
    browser: Browser,
    kind: BrowserKind,
) -> DispatchOutput {
    if state.lifecycle == HostLifecyclePhase::ShuttingDown {
        return emit_error(state, format!("register_browser: shutting down (label={})", label));
    }
    if state.browsers.contains_key(&label) {
        return emit_error(state, format!("register_browser: label {} already registered", label));
    }
    state.browsers.insert(
        label.clone(),
        BrowserHandle {
            label: label.clone(),
            browser,
            kind: kind.clone(),
            registered_at: Instant::now(),
        },
    );
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::BrowserRegistered { label, kind, version: v }],
        ..Default::default()
    }
}

fn handle_unregister_browser(state: &mut HostState, label: String) -> DispatchOutput {
    // Atomic remove + return the Browser handle in `removed_browser`
    // (codex P2 PR #660). The pane close path in
    // `browser_panes::AppStateCloseOps::take_browser_hwnd` uses the
    // returned Browser to extract its HWND. Any caller that doesn't
    // need the handle can simply ignore `removed_browser`.
    let removed = state.browsers.remove(&label);
    let removed_browser = removed.map(|h| h.browser);
    if removed_browser.is_none() {
        return DispatchOutput::default(); // idempotent
    }
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::BrowserUnregistered { label, version: v }],
        removed_browser,
        ..Default::default()
    }
}

// ── H.3 — drag state ─────────────────────────────────────────────────────

fn handle_start_drag(state: &mut HostState, session: DragSession) -> DispatchOutput {
    if state.active_drag.is_some() {
        return emit_error(state, "start_drag: drag session already active".to_string());
    }
    let drag_id = session.drag_id.clone();
    let source_window = session.source_window.clone();
    state.active_drag = Some(session);
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::DragStarted { drag_id, source_window, version: v }],
        ..Default::default()
    }
}

fn handle_end_drag(
    state: &mut HostState,
    drag_id: String,
    outcome: DragOutcome,
) -> DispatchOutput {
    let active_id = state.active_drag.as_ref().map(|s| s.drag_id.clone());
    match active_id {
        Some(id) if id == drag_id => {
            // PR #5 H.3 — return the prior session via output so callers
            // (cross-drag complete / cancel) can build the renderer-side
            // event payload without a separate read of state.active_drag.
            let session = state.active_drag.take();
            let v = state.bump_version();
            DispatchOutput {
                events: vec![HostEvent::DragEnded { drag_id, outcome, version: v }],
                ended_drag_session: session,
                ..Default::default()
            }
        }
        _ => DispatchOutput::default(), // mismatched or no drag; idempotent no-op
    }
}

// ── H.4 — pool state ─────────────────────────────────────────────────────

fn handle_pool_spawn_start(state: &mut HostState, label: String) -> DispatchOutput {
    // PR #5 H.4 — single-flight semaphore. The legacy
    // `window_pool_respawn_in_flight.swap(true)` returns the prior
    // value; if true, caller skips. We replicate that here: prior
    // in-flight OR non-Running quit state both suppress the spawn.
    if state.quit_state != QuitState::Running {
        return DispatchOutput {
            pool_spawn_proceeding: false,
            ..Default::default()
        };
    }
    if state.pool.respawn_in_flight {
        return DispatchOutput {
            pool_spawn_proceeding: false,
            ..Default::default()
        };
    }
    state.pool.unpromoted.insert(label);
    state.pool.respawn_in_flight = true;
    DispatchOutput {
        pool_spawn_proceeding: true,
        ..Default::default()
    }
}

fn handle_pool_ready(state: &mut HostState, label: String) -> DispatchOutput {
    if !state.pool.unpromoted.remove(&label) {
        // Not in unpromoted (race or duplicate signal); idempotent.
        return DispatchOutput {
            pool_size_after: Some(state.pool.queue.len()),
            ..Default::default()
        };
    }
    if !state.pool.queue.iter().any(|l| l == &label) {
        state.pool.queue.push_back(label.clone());
    }
    state.pool.respawn_in_flight = false;
    let queue_len_after = state.pool.queue.len();
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::PoolWindowEntered {
            label,
            queue_len_after,
            version: v,
        }],
        pool_size_after: Some(queue_len_after),
        ..Default::default()
    }
}

fn handle_pool_destroyed_before_promote(state: &mut HostState, label: String) -> DispatchOutput {
    // Pool windows can be destroyed in two states (codex P1 PR #654 round 2):
    //   1. Still in `unpromoted` — never reached renderer-ready.
    //   2. Already in `queue` — passed renderer-ready, awaiting promotion,
    //      then closed externally before promote.
    // Both must be cleaned up; otherwise the queue retains a dead label
    // and a later `PromotePoolWindow` operates on stale inventory.
    let was_unpromoted = state.pool.unpromoted.remove(&label);
    let queue_len_before = state.pool.queue.len();
    state.pool.queue.retain(|l| l != &label);
    let was_in_queue = state.pool.queue.len() < queue_len_before;
    state.pool.respawn_in_flight = false;
    let queue_len_after = state.pool.queue.len();
    if !was_unpromoted && !was_in_queue {
        return DispatchOutput {
            pool_size_after: Some(queue_len_after),
            ..Default::default()
        };
    }
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::PoolWindowLeft {
            label,
            queue_len_after,
            reason: PoolLeaveReason::DestroyedBeforePromote,
            version: v,
        }],
        pool_destroyed_was_unpromoted: was_unpromoted,
        pool_size_after: Some(queue_len_after),
        ..Default::default()
    }
}

fn handle_pop_and_promote_front_pool_window(state: &mut HostState) -> DispatchOutput {
    let label = match state.pool.queue.pop_front() {
        Some(l) => l,
        None => return DispatchOutput::default(),
    };
    state.pool.unpromoted.remove(&label);
    if let Some(handle) = state.browsers.get_mut(&label) {
        if let BrowserKind::TopLevel { is_pool } = &mut handle.kind {
            *is_pool = false;
        }
    }
    let queue_len_after = state.pool.queue.len();
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::PoolWindowLeft {
            label: label.clone(),
            queue_len_after,
            reason: PoolLeaveReason::Promoted,
            version: v,
        }],
        promoted_pool_label: Some(label),
        pool_size_after: Some(queue_len_after),
        ..Default::default()
    }
}

fn handle_promote_pool_window(state: &mut HostState, label: String) -> DispatchOutput {
    // Idempotent no-op for truly unknown labels (reagent P2 PR #654 round 3).
    // Symmetric with `handle_pool_destroyed_before_promote`'s pattern: only
    // emit `PoolWindowLeft` if we actually removed the label from one of
    // the pool sets. Without this, a stale promote command (e.g., from a
    // race between PromotePoolWindow and PoolWindowDestroyedBeforePromote)
    // would emit a phantom `PoolWindowLeft` event that observers might act on.
    let queue_len_before = state.pool.queue.len();
    state.pool.queue.retain(|l| l != &label);
    let was_in_queue = state.pool.queue.len() < queue_len_before;
    let was_in_unpromoted = state.pool.unpromoted.remove(&label);
    if !was_in_queue && !was_in_unpromoted {
        return DispatchOutput::default();
    }
    // Mark the corresponding browser handle as no-longer-pool.
    if let Some(handle) = state.browsers.get_mut(&label) {
        if let BrowserKind::TopLevel { is_pool } = &mut handle.kind {
            *is_pool = false;
        }
    }
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::PoolWindowLeft {
            label,
            queue_len_after: state.pool.queue.len(),
            reason: PoolLeaveReason::Promoted,
            version: v,
        }],
        ..Default::default()
    }
}

fn handle_pool_drain_all(state: &mut HostState) -> DispatchOutput {
    let drained: Vec<String> = state
        .pool
        .queue
        .drain(..)
        .chain(state.pool.unpromoted.drain())
        .collect();
    state.pool.respawn_in_flight = false;
    let mut events = Vec::new();
    for label in drained {
        let v = state.bump_version();
        events.push(HostEvent::PoolWindowLeft {
            label,
            queue_len_after: 0,
            reason: PoolLeaveReason::DrainedOnShutdown,
            version: v,
        });
    }
    let v = state.bump_version();
    events.push(HostEvent::PoolEmpty { version: v });
    DispatchOutput {
        events,
        ..Default::default()
    }
}

// ── H.5 — quit lifecycle ─────────────────────────────────────────────────

fn handle_begin_drain(state: &mut HostState, reason: QuitReason) -> DispatchOutput {
    if state.quit_state != QuitState::Running {
        return DispatchOutput::default(); // already draining or quit; idempotent
    }
    state.quit_state = QuitState::Draining { reason: reason.clone() };
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::QuitDraining { reason, version: v }],
        ..Default::default()
    }
}

fn handle_confirm_drained(state: &mut HostState) -> DispatchOutput {
    if !matches!(state.quit_state, QuitState::Draining { .. }) {
        return DispatchOutput::default(); // not draining; idempotent
    }
    state.quit_state = QuitState::Quit;
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::QuitReady { version: v }],
        ..Default::default()
    }
}

// ── H.6 — top-level window creation runner ───────────────────────────────

fn handle_enqueue_top_level_window(
    state: &mut HostState,
    request: TopLevelCreationRequest,
) -> DispatchOutput {
    if state.quit_state != QuitState::Running {
        return emit_error(state, format!("enqueue_top_level_window: not Running (label={})", request.label));
    }

    // Fail-fast for User-initiated requests when in-flight is occupied.
    // Background (pool refill) requests queue silently.
    if state.top_level_creation.in_flight.is_some()
        && request.source == TopLevelSource::User
    {
        return emit_error(state, format!("enqueue_top_level_window: busy in-flight (label={})", request.label));
    }

    state.top_level_creation.queue.push_back(request);
    let queue_len = state.top_level_creation.queue.len();
    let v = state.bump_version();
    let mut out = DispatchOutput {
        events: vec![HostEvent::TopLevelQueueLengthChanged { len: queue_len, version: v }],
        ..Default::default()
    };
    // If idle, start immediately; chain the start arm's events.
    if state.top_level_creation.in_flight.is_none() {
        let started = start_next_top_level_if_idle(state);
        out.events.extend(started.events);
    }
    out
}

/// Internal helper: if in_flight is None and queue has work, pop the front
/// and start it. Emits `TopLevelCreationRequested`, `TopLevelCreationStarted`,
/// `Effect::PostCreateWindow`, and updated queue length.
///
/// **Quit gating** (codex P1 PR #654 round 1): if `quit_state != Running`,
/// don't start anything — queued background requests stay queued but will
/// never fire (host is exiting; in-memory queue dies with the process).
/// Without this guard, an in-flight completion during `Draining` would pop
/// a queued background pool refill and emit `Effect::PostCreateWindow`,
/// creating a new window mid-shutdown and preventing drain completion.
fn start_next_top_level_if_idle(state: &mut HostState) -> DispatchOutput {
    if state.top_level_creation.in_flight.is_some() {
        return DispatchOutput::default();
    }
    if state.quit_state != QuitState::Running {
        return DispatchOutput::default();
    }
    let request = match state.top_level_creation.queue.pop_front() {
        Some(r) => r,
        None => return DispatchOutput::default(),
    };
    state.top_level_creation.next_creation_id =
        state.top_level_creation.next_creation_id.wrapping_add(1);
    let creation_id = state.top_level_creation.next_creation_id;
    let now = Instant::now();
    state.top_level_creation.in_flight = Some(InFlightCreation {
        creation_id,
        label: request.label.clone(),
        started_at: now,
        phase: CreationPhase::Started,
    });
    let label = request.label.clone();
    let source = request.source.clone();
    let queue_len = state.top_level_creation.queue.len();
    let v_req = state.bump_version();
    let v_started = state.bump_version();
    let v_eff = state.bump_version();
    let v_qlen = state.bump_version();
    DispatchOutput {
        events: vec![
            HostEvent::TopLevelCreationRequested {
                creation_id,
                source,
                label: label.clone(),
                version: v_req,
            },
            HostEvent::TopLevelCreationStarted {
                creation_id,
                label: label.clone(),
                version: v_started,
            },
            HostEvent::Effect {
                effect: EffectKind::PostCreateWindow { request, creation_id },
                version: v_eff,
            },
            HostEvent::TopLevelQueueLengthChanged { len: queue_len, version: v_qlen },
        ],
        ..Default::default()
    }
}

fn handle_top_level_callback_fired(state: &mut HostState, label: String) -> DispatchOutput {
    let matches_in_flight = state
        .top_level_creation
        .in_flight
        .as_ref()
        .map(|c| c.label == label)
        .unwrap_or(false);
    if !matches_in_flight {
        // Orphan callback: a CEF browser fired on_after_created with a
        // label we don't have in flight. Could be from a previously-evicted
        // creation (won't happen in PR #1 since we don't evict) or a stale
        // label. Emit an effect to close the orphan, preventing collision.
        let orphan_browser = state.browsers.get(&label).map(|h| h.browser.clone());
        if let Some(browser) = orphan_browser {
            let v = state.bump_version();
            return DispatchOutput {
                events: vec![HostEvent::Effect {
                    effect: EffectKind::CloseOrphanBrowser { browser },
                    version: v,
                }],
                ..Default::default()
            };
        }
        return DispatchOutput::default();
    }
    let inflight = state.top_level_creation.in_flight.take().unwrap();
    let now = Instant::now();
    let latency_ms = now.duration_since(inflight.started_at).as_millis() as u64;
    push_top_level_history(
        state,
        CompletedCreation {
            creation_id: inflight.creation_id,
            label: inflight.label.clone(),
            outcome: TopLevelCreationOutcome::Completed,
            started_at: inflight.started_at,
            finished_at: now,
            last_phase: CreationPhase::BrowserCallbackFired,
        },
    );
    let v_done = state.bump_version();
    let mut out = DispatchOutput {
        events: vec![HostEvent::TopLevelCreationCompleted {
            creation_id: inflight.creation_id,
            label: inflight.label,
            latency_ms,
            version: v_done,
        }],
        ..Default::default()
    };
    let next = start_next_top_level_if_idle(state);
    out.events.extend(next.events);
    out
}

fn handle_top_level_renderer_terminated(
    state: &mut HostState,
    label: String,
    status: String,
) -> DispatchOutput {
    let matches = state
        .top_level_creation
        .in_flight
        .as_ref()
        .map(|c| c.label == label)
        .unwrap_or(false);
    if !matches {
        return DispatchOutput::default();
    }
    let inflight = state.top_level_creation.in_flight.take().unwrap();
    let now = Instant::now();
    let outcome = TopLevelCreationOutcome::RendererTerminated { status };
    push_top_level_history(
        state,
        CompletedCreation {
            creation_id: inflight.creation_id,
            label: inflight.label.clone(),
            outcome: outcome.clone(),
            started_at: inflight.started_at,
            finished_at: now,
            last_phase: inflight.phase,
        },
    );
    let v = state.bump_version();
    let mut out = DispatchOutput {
        events: vec![HostEvent::TopLevelCreationFailed {
            creation_id: inflight.creation_id,
            label: inflight.label,
            outcome,
            version: v,
        }],
        ..Default::default()
    };
    let next = start_next_top_level_if_idle(state);
    out.events.extend(next.events);
    out
}

fn handle_top_level_externally_closed(state: &mut HostState, label: String) -> DispatchOutput {
    let matches = state
        .top_level_creation
        .in_flight
        .as_ref()
        .map(|c| c.label == label)
        .unwrap_or(false);
    if !matches {
        return DispatchOutput::default();
    }
    let inflight = state.top_level_creation.in_flight.take().unwrap();
    let now = Instant::now();
    let outcome = TopLevelCreationOutcome::ExternallyClosed;
    push_top_level_history(
        state,
        CompletedCreation {
            creation_id: inflight.creation_id,
            label: inflight.label.clone(),
            outcome: outcome.clone(),
            started_at: inflight.started_at,
            finished_at: now,
            last_phase: inflight.phase,
        },
    );
    let v = state.bump_version();
    let mut out = DispatchOutput {
        events: vec![HostEvent::TopLevelCreationFailed {
            creation_id: inflight.creation_id,
            label: inflight.label,
            outcome,
            version: v,
        }],
        ..Default::default()
    };
    let next = start_next_top_level_if_idle(state);
    out.events.extend(next.events);
    out
}

fn push_top_level_history(state: &mut HostState, entry: CompletedCreation) {
    if state.top_level_creation.history.len() >= TOP_LEVEL_CREATION_HISTORY_CAP {
        state.top_level_creation.history.pop_front();
    }
    state.top_level_creation.history.push_back(entry);
}


#[cfg(test)]
mod tests;
