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
use std::time::Instant;

use cef::Browser;

use crate::state::{
    BrowserHandle, BrowserKind, CompletedCreation, CreationPhase, DragSession, EffectKind,
    InFlightCreation, PaneEntry, PaneLifecycle, PendingWindowCreation, PoolState, QuitReason,
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
    pub panes: HashMap<String, PaneEntry>,

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
            panes: HashMap::new(),
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
    EnqueuePaneCreate { block_id: String, label: String },

    /// CEF on_after_created fired for a pane browser; confirm it's Live.
    /// No-op if already Live or absent (idempotent against late callbacks).
    CompletePaneCreate { block_id: String },

    /// Caller requests pane close. Reducer flips entry to `Closing`.
    EnqueuePaneClose { block_id: String },

    /// CEF on_before_close fired for a pane; remove entry from map.
    CompletePaneClose { block_id: String },

    /// Pane creation failed before reaching Live (e.g., CEF callback
    /// never fired, browser host returned 0). Reducer removes entry.
    AbortPaneCreate { block_id: String, reason: String },

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
            HostCommand::EnqueuePaneCreate { block_id, label } => f
                .debug_struct("EnqueuePaneCreate")
                .field("block_id", block_id)
                .field("label", label)
                .finish(),
            HostCommand::CompletePaneCreate { block_id } => f
                .debug_struct("CompletePaneCreate")
                .field("block_id", block_id)
                .finish(),
            HostCommand::EnqueuePaneClose { block_id } => f
                .debug_struct("EnqueuePaneClose")
                .field("block_id", block_id)
                .finish(),
            HostCommand::CompletePaneClose { block_id } => f
                .debug_struct("CompletePaneClose")
                .field("block_id", block_id)
                .finish(),
            HostCommand::AbortPaneCreate { block_id, reason } => f
                .debug_struct("AbortPaneCreate")
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

    PaneCreateRequested {
        block_id: String,
        label: String,
        version: u64,
    },
    PaneLive {
        block_id: String,
        label: String,
        version: u64,
    },
    PaneClosing {
        block_id: String,
        version: u64,
    },
    PaneClosed {
        block_id: String,
        version: u64,
    },
    PaneCreationFailed {
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

/// Output bundle returned from the reducer for the dequeue arm.
///
/// The dequeue arm is the only F.1 command whose caller needs the
/// popped value (an event is not sufficient — `client.rs::on_after_created`
/// uses the popped `PendingWindowCreation`'s fields to drive
/// `window_meta.insert` and `ReportWindowOpened`). Other arms can
/// rely on events alone; this struct keeps the dispatch return type
/// uniform.
#[derive(Debug, Default)]
pub struct DispatchOutput {
    pub events: Vec<HostEvent>,
    pub dequeued: Option<PendingWindowCreation>,
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
        HostCommand::EnqueuePaneCreate { block_id, label } => {
            handle_enqueue_pane_create(state, block_id, label)
        }
        HostCommand::CompletePaneCreate { block_id } => {
            handle_complete_pane_create(state, block_id)
        }
        HostCommand::EnqueuePaneClose { block_id } => {
            handle_enqueue_pane_close(state, block_id)
        }
        HostCommand::CompletePaneClose { block_id } => {
            handle_complete_pane_close(state, block_id)
        }
        HostCommand::AbortPaneCreate { block_id, reason } => {
            handle_abort_pane_create(state, block_id, reason)
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
            dequeued: None,
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
        dequeued: None,
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
            }
        }
        None => {
            let v = state.bump_version();
            DispatchOutput {
                events: vec![HostEvent::PendingWindowQueueEmpty { version: v }],
                dequeued: None,
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
        dequeued: None,
    }
}

// ── H.1 — pane lifecycle ─────────────────────────────────────────────────

fn handle_enqueue_pane_create(
    state: &mut HostState,
    block_id: String,
    label: String,
) -> DispatchOutput {
    if state.lifecycle == HostLifecyclePhase::ShuttingDown {
        return emit_error(state, format!("enqueue_pane_create: shutting down (block_id={})", block_id));
    }
    if state.panes.contains_key(&block_id) {
        return emit_error(state, format!("enqueue_pane_create: block_id {} already has a pane", block_id));
    }
    state.panes.insert(
        block_id.clone(),
        PaneEntry {
            block_id: block_id.clone(),
            label: label.clone(),
            lifecycle: PaneLifecycle::Live,
        },
    );
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::PaneCreateRequested { block_id, label, version: v }],
        dequeued: None,
    }
}

fn handle_complete_pane_create(state: &mut HostState, block_id: String) -> DispatchOutput {
    let entry = match state.panes.get(&block_id) {
        Some(e) => e.clone(),
        None => return DispatchOutput::default(), // late callback for already-removed pane; idempotent no-op
    };
    // Already Live by EnqueuePaneCreate's invariant; this is a no-op
    // confirmation event for observers.
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::PaneLive { block_id, label: entry.label, version: v }],
        dequeued: None,
    }
}

fn handle_enqueue_pane_close(state: &mut HostState, block_id: String) -> DispatchOutput {
    let entry = match state.panes.get_mut(&block_id) {
        Some(e) => e,
        None => return DispatchOutput::default(), // close request for already-gone pane; idempotent
    };
    if matches!(entry.lifecycle, PaneLifecycle::Closing { .. }) {
        return DispatchOutput::default(); // already Closing; idempotent
    }
    entry.lifecycle = PaneLifecycle::Closing { since: Instant::now() };
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::PaneClosing { block_id, version: v }],
        dequeued: None,
    }
}

fn handle_complete_pane_close(state: &mut HostState, block_id: String) -> DispatchOutput {
    if state.panes.remove(&block_id).is_none() {
        return DispatchOutput::default(); // idempotent
    }
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::PaneClosed { block_id, version: v }],
        dequeued: None,
    }
}

fn handle_abort_pane_create(
    state: &mut HostState,
    block_id: String,
    reason: String,
) -> DispatchOutput {
    state.panes.remove(&block_id);
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::PaneCreationFailed { block_id, reason, version: v }],
        dequeued: None,
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
        dequeued: None,
    }
}

fn handle_unregister_browser(state: &mut HostState, label: String) -> DispatchOutput {
    if state.browsers.remove(&label).is_none() {
        return DispatchOutput::default(); // idempotent
    }
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::BrowserUnregistered { label, version: v }],
        dequeued: None,
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
        dequeued: None,
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
            state.active_drag = None;
            let v = state.bump_version();
            DispatchOutput {
                events: vec![HostEvent::DragEnded { drag_id, outcome, version: v }],
                dequeued: None,
            }
        }
        _ => DispatchOutput::default(), // mismatched or no drag; idempotent no-op
    }
}

// ── H.4 — pool state ─────────────────────────────────────────────────────

fn handle_pool_spawn_start(state: &mut HostState, label: String) -> DispatchOutput {
    if state.quit_state != QuitState::Running {
        return DispatchOutput::default(); // pool refills suppressed during drain
    }
    state.pool.unpromoted.insert(label);
    state.pool.respawn_in_flight = true;
    DispatchOutput::default()
}

fn handle_pool_ready(state: &mut HostState, label: String) -> DispatchOutput {
    if !state.pool.unpromoted.remove(&label) {
        // Not in unpromoted (race or duplicate signal); idempotent.
        return DispatchOutput::default();
    }
    if !state.pool.queue.iter().any(|l| l == &label) {
        state.pool.queue.push_back(label.clone());
    }
    state.pool.respawn_in_flight = false;
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::PoolWindowEntered {
            label,
            queue_len_after: state.pool.queue.len(),
            version: v,
        }],
        dequeued: None,
    }
}

fn handle_pool_destroyed_before_promote(state: &mut HostState, label: String) -> DispatchOutput {
    let was_unpromoted = state.pool.unpromoted.remove(&label);
    state.pool.respawn_in_flight = false;
    if !was_unpromoted {
        return DispatchOutput::default();
    }
    let v = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::PoolWindowLeft {
            label,
            queue_len_after: state.pool.queue.len(),
            reason: PoolLeaveReason::DestroyedBeforePromote,
            version: v,
        }],
        dequeued: None,
    }
}

fn handle_promote_pool_window(state: &mut HostState, label: String) -> DispatchOutput {
    let in_queue = state.pool.queue.iter().position(|l| l == &label).is_some();
    if in_queue {
        state.pool.queue.retain(|l| l != &label);
    }
    state.pool.unpromoted.remove(&label);
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
        dequeued: None,
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
    DispatchOutput { events, dequeued: None }
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
        dequeued: None,
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
        dequeued: None,
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
        dequeued: None,
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
fn start_next_top_level_if_idle(state: &mut HostState) -> DispatchOutput {
    if state.top_level_creation.in_flight.is_some() {
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
        dequeued: None,
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
                dequeued: None,
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
        dequeued: None,
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
        dequeued: None,
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
        dequeued: None,
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
mod tests {
    use super::*;
    use crate::state::WindowKind;

    fn entry(label: &str) -> PendingWindowCreation {
        PendingWindowCreation {
            label: label.to_string(),
            kind: WindowKind::FullInstance,
            parent_instance_id: None,
        }
    }

    #[test]
    fn enqueue_then_dequeue_round_trips() {
        let mut state = HostState::default();
        let out1 = update(
            &mut state,
            HostCommand::EnqueuePendingWindowCreation { entry: entry("w1") },
        );
        assert!(matches!(
            out1.events.as_slice(),
            [HostEvent::PendingWindowEnqueued { queue_len_after: 1, .. }]
        ));
        assert!(out1.dequeued.is_none());

        let out2 = update(&mut state, HostCommand::DequeuePendingWindowCreation);
        assert!(matches!(
            out2.events.as_slice(),
            [HostEvent::PendingWindowDequeued { queue_len_after: 0, .. }]
        ));
        assert_eq!(out2.dequeued.as_ref().unwrap().label, "w1");
    }

    #[test]
    fn dequeue_on_empty_returns_queue_empty_event() {
        let mut state = HostState::default();
        let out = update(&mut state, HostCommand::DequeuePendingWindowCreation);
        assert!(matches!(
            out.events.as_slice(),
            [HostEvent::PendingWindowQueueEmpty { .. }]
        ));
        assert!(out.dequeued.is_none());
    }

    #[test]
    fn fifo_order_preserved() {
        let mut state = HostState::default();
        for label in ["w1", "w2", "w3"] {
            update(
                &mut state,
                HostCommand::EnqueuePendingWindowCreation { entry: entry(label) },
            );
        }
        for expected in ["w1", "w2", "w3"] {
            let out = update(&mut state, HostCommand::DequeuePendingWindowCreation);
            assert_eq!(out.dequeued.as_ref().unwrap().label, expected);
        }
    }

    #[test]
    fn enqueue_during_shutdown_is_rejected() {
        let mut state = HostState::default();
        state.lifecycle = HostLifecyclePhase::ShuttingDown;
        let out = update(
            &mut state,
            HostCommand::EnqueuePendingWindowCreation { entry: entry("w1") },
        );
        assert!(matches!(
            out.events.as_slice(),
            [HostEvent::Error { .. }]
        ));
        assert_eq!(state.pending_window_creations.len(), 0);
    }

    #[test]
    fn version_increments_monotonically() {
        let mut state = HostState::default();
        let out1 = update(
            &mut state,
            HostCommand::EnqueuePendingWindowCreation { entry: entry("w1") },
        );
        let out2 = update(&mut state, HostCommand::DequeuePendingWindowCreation);
        let out3 = update(&mut state, HostCommand::DequeuePendingWindowCreation);

        // Helper that pulls the `version` field out of any HostEvent.
        // Kept exhaustive so adding a new event variant forces an
        // explicit decision here (vs. silently defaulting to 0).
        let extract_version = |events: &[HostEvent]| match &events[0] {
            HostEvent::PendingWindowEnqueued { version, .. } => *version,
            HostEvent::PendingWindowDequeued { version, .. } => *version,
            HostEvent::PendingWindowQueueEmpty { version } => *version,
            HostEvent::PaneCreateRequested { version, .. } => *version,
            HostEvent::PaneLive { version, .. } => *version,
            HostEvent::PaneClosing { version, .. } => *version,
            HostEvent::PaneClosed { version, .. } => *version,
            HostEvent::PaneCreationFailed { version, .. } => *version,
            HostEvent::BrowserRegistered { version, .. } => *version,
            HostEvent::BrowserUnregistered { version, .. } => *version,
            HostEvent::DragStarted { version, .. } => *version,
            HostEvent::DragEnded { version, .. } => *version,
            HostEvent::PoolWindowEntered { version, .. } => *version,
            HostEvent::PoolWindowLeft { version, .. } => *version,
            HostEvent::PoolEmpty { version } => *version,
            HostEvent::QuitDraining { version, .. } => *version,
            HostEvent::QuitReady { version } => *version,
            HostEvent::TopLevelCreationRequested { version, .. } => *version,
            HostEvent::TopLevelCreationStarted { version, .. } => *version,
            HostEvent::TopLevelCreationCompleted { version, .. } => *version,
            HostEvent::TopLevelCreationFailed { version, .. } => *version,
            HostEvent::TopLevelQueueLengthChanged { version, .. } => *version,
            HostEvent::Effect { version, .. } => *version,
            HostEvent::Error { version, .. } => *version,
        };
        let v1 = extract_version(&out1.events);
        let v2 = extract_version(&out2.events);
        let v3 = extract_version(&out3.events);
        assert!(v1 < v2);
        assert!(v2 < v3);
    }

    // ── Phase H foundations tests ───────────────────────────────────────

    fn pane_request(block_id: &str, label: &str) -> HostCommand {
        HostCommand::EnqueuePaneCreate {
            block_id: block_id.to_string(),
            label: label.to_string(),
        }
    }

    fn top_level_request(label: &str, source: TopLevelSource) -> TopLevelCreationRequest {
        TopLevelCreationRequest {
            label: label.to_string(),
            kind: WindowKind::FullInstance,
            parent_instance_id: None,
            url: format!("https://example.test/{}", label),
            pos: (0, 0),
            size: (800, 600),
            frameless: true,
            source,
        }
    }

    // ── H.1 panes ────────────────────────────────────────────────────────

    #[test]
    fn enqueue_pane_create_inserts_live() {
        let mut state = HostState::default();
        let out = update(&mut state, pane_request("b1", "browser-pane-b1-1"));
        assert!(state.panes.contains_key("b1"));
        assert!(matches!(state.panes["b1"].lifecycle, PaneLifecycle::Live));
        assert!(matches!(out.events[0], HostEvent::PaneCreateRequested { .. }));
    }

    #[test]
    fn enqueue_pane_create_duplicate_rejected() {
        let mut state = HostState::default();
        update(&mut state, pane_request("b1", "browser-pane-b1-1"));
        let out = update(&mut state, pane_request("b1", "browser-pane-b1-2"));
        assert!(matches!(out.events[0], HostEvent::Error { .. }));
        assert_eq!(state.panes.len(), 1);
    }

    #[test]
    fn pane_close_lifecycle() {
        let mut state = HostState::default();
        update(&mut state, pane_request("b1", "browser-pane-b1-1"));
        update(&mut state, HostCommand::EnqueuePaneClose { block_id: "b1".into() });
        assert!(matches!(
            state.panes["b1"].lifecycle,
            PaneLifecycle::Closing { .. }
        ));
        update(&mut state, HostCommand::CompletePaneClose { block_id: "b1".into() });
        assert!(!state.panes.contains_key("b1"));
    }

    #[test]
    fn pane_abort_removes_entry() {
        let mut state = HostState::default();
        update(&mut state, pane_request("b1", "browser-pane-b1-1"));
        let out = update(
            &mut state,
            HostCommand::AbortPaneCreate {
                block_id: "b1".into(),
                reason: "test".into(),
            },
        );
        assert!(!state.panes.contains_key("b1"));
        assert!(matches!(out.events[0], HostEvent::PaneCreationFailed { .. }));
    }

    #[test]
    fn pane_close_idempotent_for_missing() {
        let mut state = HostState::default();
        let out = update(&mut state, HostCommand::EnqueuePaneClose { block_id: "missing".into() });
        assert!(out.events.is_empty()); // idempotent no-op
    }

    // ── H.3 drag (singleton invariant) ───────────────────────────────────

    fn drag_session(id: &str) -> DragSession {
        DragSession {
            drag_id: id.to_string(),
            drag_type: crate::state::DragType::Tab,
            source_window: "main".to_string(),
            source_workspace_id: "ws1".to_string(),
            source_tab_id: "tab1".to_string(),
            payload: crate::state::DragPayload {
                block_id: None,
                tab_id: Some("tab1".to_string()),
            },
            started_at: 0,
        }
    }

    #[test]
    fn drag_singleton_invariant() {
        let mut state = HostState::default();
        update(&mut state, HostCommand::StartDrag { session: drag_session("d1") });
        assert!(state.active_drag.is_some());
        let out = update(&mut state, HostCommand::StartDrag { session: drag_session("d2") });
        assert!(matches!(out.events[0], HostEvent::Error { .. }));
        assert_eq!(state.active_drag.as_ref().unwrap().drag_id, "d1");
    }

    #[test]
    fn drag_end_clears_session() {
        let mut state = HostState::default();
        update(&mut state, HostCommand::StartDrag { session: drag_session("d1") });
        update(
            &mut state,
            HostCommand::EndDrag {
                drag_id: "d1".into(),
                outcome: DragOutcome::Cancelled,
            },
        );
        assert!(state.active_drag.is_none());
    }

    #[test]
    fn drag_end_with_wrong_id_is_noop() {
        let mut state = HostState::default();
        update(&mut state, HostCommand::StartDrag { session: drag_session("d1") });
        let out = update(
            &mut state,
            HostCommand::EndDrag {
                drag_id: "wrong".into(),
                outcome: DragOutcome::Cancelled,
            },
        );
        assert!(out.events.is_empty());
        assert!(state.active_drag.is_some());
    }

    // ── H.5 quit (monotonic transitions) ─────────────────────────────────

    #[test]
    fn quit_state_monotonic() {
        let mut state = HostState::default();
        assert_eq!(state.quit_state, QuitState::Running);
        update(&mut state, HostCommand::BeginDrain { reason: QuitReason::LastWindowClosed });
        assert!(matches!(state.quit_state, QuitState::Draining { .. }));
        update(&mut state, HostCommand::ConfirmDrained);
        assert_eq!(state.quit_state, QuitState::Quit);
        // Subsequent BeginDrain is a no-op (monotonic).
        let out = update(&mut state, HostCommand::BeginDrain { reason: QuitReason::External });
        assert!(out.events.is_empty());
        assert_eq!(state.quit_state, QuitState::Quit);
    }

    // ── H.6 top-level runner (singleton + fail-fast) ─────────────────────

    #[test]
    fn enqueue_top_level_when_idle_starts_immediately() {
        let mut state = HostState::default();
        let out = update(
            &mut state,
            HostCommand::EnqueueTopLevelWindow {
                request: top_level_request("a", TopLevelSource::User),
            },
        );
        assert!(state.top_level_creation.in_flight.is_some());
        assert_eq!(state.top_level_creation.in_flight.as_ref().unwrap().label, "a");
        let begin_count = out
            .events
            .iter()
            .filter(|e| matches!(
                e,
                HostEvent::Effect { effect: EffectKind::PostCreateWindow { .. }, .. }
            ))
            .count();
        assert_eq!(begin_count, 1);
    }

    #[test]
    fn user_initiated_when_busy_fails_fast() {
        let mut state = HostState::default();
        update(
            &mut state,
            HostCommand::EnqueueTopLevelWindow {
                request: top_level_request("a", TopLevelSource::User),
            },
        );
        // Second user-initiated request: in-flight is occupied → error.
        let out = update(
            &mut state,
            HostCommand::EnqueueTopLevelWindow {
                request: top_level_request("b", TopLevelSource::User),
            },
        );
        assert!(matches!(out.events[0], HostEvent::Error { .. }));
        assert_eq!(state.top_level_creation.queue.len(), 0); // not queued
    }

    #[test]
    fn background_when_busy_queues_silently() {
        let mut state = HostState::default();
        update(
            &mut state,
            HostCommand::EnqueueTopLevelWindow {
                request: top_level_request("a", TopLevelSource::User),
            },
        );
        // Background request queues even though in-flight occupied.
        update(
            &mut state,
            HostCommand::EnqueueTopLevelWindow {
                request: top_level_request("b", TopLevelSource::Background),
            },
        );
        assert_eq!(state.top_level_creation.queue.len(), 1);
        assert_eq!(state.top_level_creation.in_flight.as_ref().unwrap().label, "a");
    }

    #[test]
    fn callback_fired_advances_queue() {
        let mut state = HostState::default();
        update(
            &mut state,
            HostCommand::EnqueueTopLevelWindow {
                request: top_level_request("a", TopLevelSource::User),
            },
        );
        update(
            &mut state,
            HostCommand::EnqueueTopLevelWindow {
                request: top_level_request("b", TopLevelSource::Background),
            },
        );
        let out = update(
            &mut state,
            HostCommand::TopLevelCallbackFired { label: "a".into() },
        );
        // a archived to history; b now in-flight.
        assert_eq!(state.top_level_creation.history.len(), 1);
        assert_eq!(state.top_level_creation.in_flight.as_ref().unwrap().label, "b");
        assert!(out.events.iter().any(|e| matches!(e, HostEvent::TopLevelCreationCompleted { .. })));
    }

    #[test]
    fn renderer_terminated_fails_in_flight() {
        let mut state = HostState::default();
        update(
            &mut state,
            HostCommand::EnqueueTopLevelWindow {
                request: top_level_request("a", TopLevelSource::User),
            },
        );
        update(
            &mut state,
            HostCommand::TopLevelRendererTerminated {
                label: "a".into(),
                status: "killed".into(),
            },
        );
        assert!(state.top_level_creation.in_flight.is_none());
        assert_eq!(state.top_level_creation.history.len(), 1);
        assert!(matches!(
            state.top_level_creation.history.back().unwrap().outcome,
            TopLevelCreationOutcome::RendererTerminated { .. }
        ));
    }

    #[test]
    fn callback_fired_with_unknown_label_is_noop_or_orphan_close() {
        let mut state = HostState::default();
        // No in-flight, no browser registered for this label.
        let out = update(
            &mut state,
            HostCommand::TopLevelCallbackFired { label: "ghost".into() },
        );
        assert!(out.events.is_empty()); // pure no-op when no orphan to close
    }

    #[test]
    fn enqueue_top_level_during_quit_rejected() {
        let mut state = HostState::default();
        update(&mut state, HostCommand::BeginDrain { reason: QuitReason::LastWindowClosed });
        let out = update(
            &mut state,
            HostCommand::EnqueueTopLevelWindow {
                request: top_level_request("a", TopLevelSource::User),
            },
        );
        assert!(matches!(out.events[0], HostEvent::Error { .. }));
        assert!(state.top_level_creation.in_flight.is_none());
    }

    #[test]
    fn history_caps_at_50() {
        let mut state = HostState::default();
        for i in 0..60 {
            let label = format!("w{}", i);
            update(
                &mut state,
                HostCommand::EnqueueTopLevelWindow {
                    request: top_level_request(&label, TopLevelSource::Background),
                },
            );
            update(
                &mut state,
                HostCommand::TopLevelCallbackFired { label },
            );
        }
        assert_eq!(state.top_level_creation.history.len(), TOP_LEVEL_CREATION_HISTORY_CAP);
        assert_eq!(state.top_level_creation.history.front().unwrap().label, "w10");
        assert_eq!(state.top_level_creation.history.back().unwrap().label, "w59");
    }

    // ── H.4 pool ─────────────────────────────────────────────────────────

    #[test]
    fn pool_spawn_then_ready_enters_queue() {
        let mut state = HostState::default();
        update(&mut state, HostCommand::PoolWindowSpawnStart { label: "p1".into() });
        assert!(state.pool.unpromoted.contains("p1"));
        assert!(state.pool.respawn_in_flight);
        update(&mut state, HostCommand::PoolWindowReady { label: "p1".into() });
        assert!(!state.pool.unpromoted.contains("p1"));
        assert_eq!(state.pool.queue.len(), 1);
        assert!(!state.pool.respawn_in_flight);
    }

    #[test]
    fn pool_drain_clears_all() {
        let mut state = HostState::default();
        update(&mut state, HostCommand::PoolWindowSpawnStart { label: "p1".into() });
        update(&mut state, HostCommand::PoolWindowReady { label: "p1".into() });
        update(&mut state, HostCommand::PoolWindowSpawnStart { label: "p2".into() });
        let out = update(&mut state, HostCommand::PoolDrainAll);
        assert!(state.pool.queue.is_empty());
        assert!(state.pool.unpromoted.is_empty());
        assert!(out.events.iter().any(|e| matches!(e, HostEvent::PoolEmpty { .. })));
    }

    #[test]
    fn pool_spawn_during_quit_suppressed() {
        let mut state = HostState::default();
        update(&mut state, HostCommand::BeginDrain { reason: QuitReason::LastWindowClosed });
        let out = update(&mut state, HostCommand::PoolWindowSpawnStart { label: "p1".into() });
        assert!(out.events.is_empty()); // suppressed
        assert!(state.pool.unpromoted.is_empty());
    }
}
