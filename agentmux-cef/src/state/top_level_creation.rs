// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

use cef::Browser;

use super::WindowKind;

// ── Top-level window creation runner (H.6) ───────────────────────────────

/// A request to create a top-level window. Pushed to
/// `HostState.top_level_creation.queue` via `EnqueueTopLevelWindow`.
/// Carries the full spec the effect handler needs to call
/// `ui_tasks::post_create_window`.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct TopLevelCreationRequest {
    pub label: String,
    pub kind: WindowKind,
    pub parent_instance_id: Option<String>,
    pub url: String,
    pub pos: (i32, i32),
    pub size: (i32, i32),
    pub frameless: bool,
    /// `User`-initiated (fail-fast on contention) vs `Background` (pool
    /// refill — may queue silently).
    pub source: TopLevelSource,
}

/// Distinguishes user-facing creation requests (which fail-fast on
/// contention) from background ones (pool refill — may queue indefinitely).
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum TopLevelSource {
    /// Triggered by a user action (click "open new window", tear off a
    /// tab, etc.). Reducer rejects with error if in-flight slot is
    /// occupied — caller propagates a visible error to the frontend.
    User,
    /// Triggered by the runner itself (pool refill, recovery). Queues
    /// behind in-flight; no caller waiting on completion.
    Background,
}

/// The single in-flight top-level creation. Singleton invariant enforced
/// by the reducer (at most one Some across all action sequences).
///
/// **No `deadline` field. No watchdog.** The reducer reacts only to
/// observable CEF callbacks: `on_after_created` (success),
/// `on_render_process_terminated` (renderer crash), `on_before_close`
/// (cancel mid-create). If none fire, the slot stays occupied
/// permanently — user-initiated creates fail-fast with visible error
/// per the no-timer directive.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct InFlightCreation {
    pub creation_id: u64,
    pub label: String,
    pub started_at: std::time::Instant,
    pub phase: CreationPhase,
}

/// Phase progression of an in-flight top-level creation. Monotonic;
/// `AdvanceCreationPhase` (if added later) refuses regression.
#[derive(Clone, Debug, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u8)]
#[allow(dead_code)]
pub enum CreationPhase {
    /// Effect handler has dispatched `post_create_window`; CEF has not
    /// yet fired any callback.
    Started = 0,
    /// CEF `on_after_created` fired — Browser exists, renderer alive.
    BrowserCallbackFired = 1,
}

/// Archived completion record for the runner's history ring buffer.
/// Bounded at `TOP_LEVEL_CREATION_HISTORY_CAP` (50); oldest evicted FIFO.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct CompletedCreation {
    pub creation_id: u64,
    pub label: String,
    pub outcome: TopLevelCreationOutcome,
    pub started_at: std::time::Instant,
    pub finished_at: std::time::Instant,
    pub last_phase: CreationPhase,
}

/// Why a top-level creation completed. `Completed` is happy-path; the
/// other variants are observable failure modes.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum TopLevelCreationOutcome {
    /// CEF `on_after_created` fired; browser registered. Normal completion.
    Completed,
    /// CEF `on_render_process_terminated` fired during creation. Renderer
    /// process died (crash, OOM, killed).
    RendererTerminated { status: String },
    /// CEF `on_before_close` fired during creation. Browser closed
    /// externally (user action, parent close, etc.).
    ExternallyClosed,
}

/// Reducer-managed runner state. Owns the queue, in-flight slot, history,
/// and id allocator.
#[derive(Default, Clone, Debug)]
#[allow(dead_code)]
pub struct TopLevelCreationState {
    pub queue: std::collections::VecDeque<TopLevelCreationRequest>,
    pub in_flight: Option<InFlightCreation>,
    pub history: std::collections::VecDeque<CompletedCreation>,
    pub next_creation_id: u64,
}

// ── Effects (carrier for side-effect-bearing events) ─────────────────────

/// Side-effect descriptor emitted by reducer arms. Carried inside
/// `HostEvent::Effect(EffectKind)`. The effects executor in
/// `AppState::host_dispatch_with_effects` dispatches each kind to the
/// appropriate imperative handler (e.g., posting a CEF UI task).
///
/// Reducer arms emit effects but never execute them; this preserves the
/// pure-functional discipline of `update()`. Manual Debug impl below
/// because `cef::Browser` doesn't impl Debug.
#[derive(Clone)]
#[allow(dead_code)]
pub enum EffectKind {
    /// Begin top-level window creation by posting `ui_tasks::post_create_window`.
    /// Carried by `HostEvent::TopLevelCreationStarted`'s effect path.
    PostCreateWindow { request: TopLevelCreationRequest, creation_id: u64 },
    /// Spawn a pool window (PR #3 wires this when pool drops below TARGET_SIZE).
    SpawnPoolWindow,
    /// Close an orphan CEF browser whose label doesn't match any in-flight
    /// or registered entry. Used by H.6's mismatched-callback handler to
    /// prevent label collision.
    CloseOrphanBrowser { browser: Browser },
}

impl std::fmt::Debug for EffectKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            EffectKind::PostCreateWindow { request, creation_id } => f
                .debug_struct("PostCreateWindow")
                .field("creation_id", creation_id)
                .field("label", &request.label)
                .finish(),
            EffectKind::SpawnPoolWindow => f.write_str("SpawnPoolWindow"),
            EffectKind::CloseOrphanBrowser { .. } => f
                .debug_struct("CloseOrphanBrowser")
                .field("browser", &"<cef::Browser>")
                .finish(),
        }
    }
}

/// A browser-pane create deferred while the block_id was still `Closing`
/// (old CEF Browser mid-teardown). Owned by the reducer's
/// `HostState.pending_browser_pane_creates` (NOT `AppState`) so stash-on-`Closing`
/// and remove-on-close are atomic under the single host_state lock. The
/// close-completion arms (`CompleteBrowserPaneClose`/`DrainBrowserPaneByLabel`)
/// hand it back via `DispatchOutput.pending_browser_pane_create_to_replay`;
/// the IPC handler replays it (now `Fresh`). The deterministic
/// re-create-after-close that fixes the redock "pane sometimes won't load"
/// race (no frontend retry / timer). Rect stored as raw i32s to keep this
/// type independent of `cef::Rect`.
#[derive(Clone, Debug)]
pub struct PendingBrowserPaneCreate {
    pub url: String,
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
    pub window_label: String,
}

/// Phase 4b — ghost state stored per target-window label during a floating-pane
/// redock drag. The target renderer pushes `block_id + dir` when it shows the
/// ghost overlay; the floater reads it at drop time to pass a directional hint
/// to the `RedockFloatingPane` saga.
#[derive(Clone, Debug)]
pub struct FloatingRedockGhostState {
    pub block_id: String,
    pub dir: u8,
}
