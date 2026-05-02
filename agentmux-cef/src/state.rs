// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Shared application state for the CEF host.

use std::collections::{HashMap, VecDeque};
use parking_lot::Mutex;

use cef::Browser;

// ── Cross-window drag types (ported from src-tauri/src/state.rs) ─────────

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DragType {
    Pane,
    Tab,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DragPayload {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub block_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tab_id: Option<String>,
}

#[derive(Debug, Clone, serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DragSession {
    pub drag_id: String,
    pub drag_type: DragType,
    pub source_window: String,
    pub source_workspace_id: String,
    pub source_tab_id: String,
    pub payload: DragPayload,
    pub started_at: u64,
}

// Phase B.5e — `WindowInstanceRegistry` struct deleted. Sequential
// instance numbers are now owned by the launcher's reducer
// (`agentmux-launcher::state::State.instance_registry`). Host's
// projection lives in `AppState.shadow_instance_registry`, fed by
// `Event::WindowInstanceAssigned` / `WindowInstanceReleased`. See
// docs/retro/migration-pattern.md for the a→b→c→d→e ratchet.

// Phase B.1: removed `JobHandle` wrapper. Host no longer owns a Job
// Object on srv; the launcher's J0 covers srv directly via
// AssignProcessToJobObject. The same RAII pattern lives in
// agentmux-launcher/src/main.rs::JobHandle.

/// Backend (agentmux-srv) connection endpoints.
#[derive(Default, Clone, serde::Serialize)]
pub struct BackendEndpoints {
    pub ws_endpoint: String,
    pub web_endpoint: String,
}

/// Window role in the AgentMux multi-window model.
///
/// Two distinct types with different taskbar treatment:
/// - `FullInstance`: independent AgentMux window (like Chrome/VS Code new window).
///   Appears in the Windows taskbar. All user-facing "new window" paths (status-bar
///   version click, second `agentmux.exe` launch, `Ctrl+Shift+N`) create one.
/// - `Subwindow`: hidden from the taskbar via `ITaskbarList::DeleteTab`. Only
///   reachable through the backend `open_subwindow` API — reserved for agent /
///   internal use cases (transient auxiliary views, tool-spawned panels). Closes
///   when its parent full instance closes.
#[derive(Copy, Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WindowKind {
    FullInstance,
    Subwindow,
}

/// Per-window metadata held alongside the CEF `Browser`. See `WindowKind` for
/// the semantics of `kind` and `parent_instance_id`.
#[derive(Clone, Debug)]
pub struct WindowMeta {
    pub label: String,
    pub kind: WindowKind,
    /// For `Subwindow` only: label of the `FullInstance` that owns this window.
    /// `None` for `FullInstance`.
    pub parent_instance_id: Option<String>,
}

/// Phase B.5 (window_meta step d) — pre-create handoff. Caller
/// (`drag.rs::tear_off`, `commands/window.rs::open_new_window`,
/// `window_pool.rs::spawn_pool_window`, `pane/creation.rs`) pushes
/// one entry per window CEF is about to create; `client.rs::on_after_created`
/// pops the head entry and uses `kind` for the Subwindow
/// taskbar-hide branch + as the payload for `ReportWindowOpened`.
///
/// Replaces the previous `pending_window_labels: VecDeque<String>`
/// queue + parallel caller-side `window_meta` writes that used to
/// act as the kind/parent channel. Collapsing them into a single
/// tuple eliminates the parallel-write race; on_after_created
/// performs the single canonical `window_meta.insert` from the
/// popped entry (kept as a synchronous host-side cache for
/// open_subwindow's parent liveness check + cascade-close
/// enumeration in `task dev` mode where launcher IPC is absent).
#[derive(Clone, Debug)]
pub struct PendingWindowCreation {
    pub label: String,
    pub kind: WindowKind,
    pub parent_instance_id: Option<String>,
}

// ── Phase H — host reducer buildout ──────────────────────────────────────
//
// All types below are reducer-only state. PR #1 (h1-foundations) declares
// them; subsequent PRs (#2-#5) wire callers through the reducer per the
// a→b→c→d→e migration ratchet. See:
//   docs/specs/SPEC_HOST_REDUCER_5PR_PLAN_2026-05-02.md
//   docs/specs/SPEC_HOST_REDUCER_PHASE_H_2026-05-02.md
//
// These types intentionally have `#[allow(dead_code)]` because PR #1 ships
// the scaffolding without callers — fields are populated by reducer arms but
// no production code reads them yet. Subsequent PRs lift the allow as they
// wire each migration.

// ── Pane lifecycle (H.1) ─────────────────────────────────────────────────

/// Lifecycle state of a browser pane (the `defwidget@browser` widget). Held
/// inside `HostState.panes` keyed by `block_id`. Mirrors the existing
/// `PaneStateMachine::PaneLifecycle` (pane/lifecycle.rs:28); the existing
/// type stays during PR #2's a→e migration. PR #2 step e deletes the
/// pane/lifecycle.rs version and migrates all readers to this one.
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum PaneLifecycle {
    /// Pane is alive and accepting operations (focus, resize, navigate).
    Live,
    /// Close requested; awaiting CEF on_before_close to fully tear down.
    /// `since` carries the request timestamp for diagnostic purposes only;
    /// nothing in the reducer is timer-driven.
    Closing { since: std::time::Instant },
}

/// Per-pane reducer-managed entry. Replaces `pane::lifecycle::PaneEntry`
/// (lifecycle.rs:42) at PR #2 step e.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub struct PaneEntry {
    pub block_id: String,
    pub label: String,
    pub lifecycle: PaneLifecycle,
}

// ── Browser handle registry (H.2) ────────────────────────────────────────

/// Wrapped CEF Browser handle stored in `HostState.browsers`. Replaces the
/// raw `Mutex<HashMap<String, Browser>>` at `state.rs::AppState.browsers`
/// at PR #2 step e.
///
/// `cef::Browser` is `Clone` (refcounted FFI handle) and safe to store
/// inside the reducer's mutex-guarded state. Doesn't impl Debug, hence
/// the manual `impl Debug` below for `BrowserHandle`.
#[derive(Clone)]
#[allow(dead_code)]
pub struct BrowserHandle {
    pub label: String,
    pub browser: Browser,
    pub kind: BrowserKind,
    pub registered_at: std::time::Instant,
}

impl std::fmt::Debug for BrowserHandle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserHandle")
            .field("label", &self.label)
            .field("kind", &self.kind)
            .field("registered_at", &self.registered_at)
            .field("browser", &"<cef::Browser>")
            .finish()
    }
}

/// Distinguishes top-level CEF Browsers (full-instance windows + pool
/// windows) from pane CEF Browsers (children of a top-level). Determines
/// taskbar treatment, lifecycle ownership, etc.
#[derive(Clone, Debug)]
#[allow(dead_code)]
pub enum BrowserKind {
    /// Top-level window. `is_pool=true` while the window is in the warm
    /// pool; cleared on promote.
    TopLevel { is_pool: bool },
    /// Browser pane child window. `block_id` correlates with the
    /// `HostState.panes` entry.
    Pane { block_id: String },
}

// ── Pool state (H.4) ─────────────────────────────────────────────────────

/// Pre-warmed window pool state. Replaces three separate fields on
/// `AppState`: `window_pool: Mutex<VecDeque<String>>`,
/// `unpromoted_pool_labels: Mutex<HashSet<String>>`, and
/// `window_pool_respawn_in_flight: AtomicBool`. PR #3 migrates each
/// caller through the a→e ratchet.
#[derive(Default, Clone, Debug)]
#[allow(dead_code)]
pub struct PoolState {
    /// Labels of pool windows whose renderer signaled ready (eligible
    /// for promotion).
    pub queue: std::collections::VecDeque<String>,
    /// Labels spawned but not yet renderer-ready (and therefore not yet
    /// in `queue`). Used for taskbar/exclusion filters during the spawn
    /// → ready window.
    pub unpromoted: std::collections::HashSet<String>,
    /// Single-flight semaphore: true while a respawn task is in flight,
    /// preventing stacked refills.
    pub respawn_in_flight: bool,
}

// ── Quit state (H.5) ─────────────────────────────────────────────────────

/// Host process quit lifecycle. Replaces `is_quitting: AtomicBool` at
/// `state.rs::AppState`. Three states; transitions are monotonic
/// (Running → Draining → Quit, no regression).
#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum QuitState {
    /// Normal operation. All commands accepted (subject to per-arm rules).
    Running,
    /// `BeginDrain` dispatched. Pool refills suppressed; awaiting pool +
    /// browsers to drain.
    Draining { reason: QuitReason },
    /// `ConfirmDrained` dispatched. Host quitting; no further commands.
    Quit,
}

#[derive(Clone, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub enum QuitReason {
    /// User closed the last user-visible top-level window. Standard exit.
    LastWindowClosed,
    /// Launcher signaled HostShouldQuit (cross-process shutdown).
    LauncherRequested,
    /// External force-quit (Win32 WM_QUIT, signal, etc.).
    External,
}

impl Default for QuitState {
    fn default() -> Self { QuitState::Running }
}

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

/// Shared application state for the CEF host.
///
/// Unlike the Tauri version, this uses `Arc<AppState>` directly instead of
/// `tauri::State<AppState>`. The sidecar child is `std::process::Child` instead
/// of `tauri_plugin_shell::process::CommandChild`.
pub struct AppState {
    // Phase B.5 (window_id_map step e) — `window_id_map` field
    // deleted. Authoritative copy lives in the launcher's
    // `state.backend_window_ids`. Host's projection is in
    // `shadow_backend_window_ids` (below).

    /// Auth key for backend communication
    pub auth_key: Mutex<String>,

    /// Backend (agentmux-srv) connection endpoints
    pub backend_endpoints: Mutex<BackendEndpoints>,

    /// Handle to the sidecar child process for graceful shutdown
    pub sidecar_child: Mutex<Option<std::process::Child>>,

    /// Backend process PID (set after spawn)
    pub backend_pid: Mutex<Option<u32>>,

    /// Backend process start time as ISO 8601 string
    pub backend_started_at: Mutex<Option<String>>,

    /// Current zoom factor
    pub zoom_factor: Mutex<f64>,

    /// Client ID (set after querying backend on startup)
    pub client_id: Mutex<Option<String>>,

    /// Window ID (set after querying/creating window via backend)
    pub window_id: Mutex<Option<String>>,

    /// Active tab ID (set after querying/creating default tab via backend)
    pub active_tab_id: Mutex<Option<String>>,

    /// Window initialization status ("ready" or "wave-ready")
    pub window_init_status: Mutex<String>,

    /// Phase B.5e — host's projection of the launcher's
    /// authoritative `state.instance_registry`. Fed by
    /// `Event::WindowInstanceAssigned` /
    /// `Event::WindowInstanceReleased` via
    /// `launcher_ipc::apply_event_to_shadow`. Host code never
    /// mutates this directly — reads only. Pre-seeded with
    /// `{"main": 1}` to mirror the launcher's pre-seed (avoids a
    /// spurious first-event mismatch during startup before the
    /// `WindowInstanceAssigned { label: "main", num: 1 }` event
    /// arrives).
    pub shadow_instance_registry: Mutex<HashMap<String, u32>>,

    /// Phase B.5 (window_id_map) — host's projection of the
    /// launcher's authoritative `state.backend_window_ids`. Fed by
    /// `Event::BackendWindowIdRegistered` /
    /// `Event::BackendWindowIdUnregistered` via
    /// `apply_event_to_shadow`. Sole source of truth post-step-e
    /// (host's `window_id_map` was deleted). Read via
    /// `Self::backend_window_id`; never mutated directly by host
    /// code.
    pub shadow_backend_window_ids: Mutex<HashMap<String, String>>,

    /// Phase B.5 (window_meta step b) — host's projection of the
    /// launcher's authoritative `state.windows: HashMap<String,
    /// WindowMirror>` (B.4). The launcher already mirrors all
    /// `WindowMeta` data via `Event::WindowOpened`/`WindowClosed`
    /// (carrying `{label, kind, parent_label}` since B.4); this
    /// host-side cache makes it readable without a launcher
    /// round-trip. Maintained in parallel to `window_meta` until
    /// step c flips reads, step d drops mutations, step e deletes
    /// the host field. Drift logged via
    /// `target = "launcher-ipc:drift"`.
    pub shadow_window_meta: Mutex<HashMap<String, WindowMeta>>,

    /// Cancellation channel for an in-progress CLI login process
    pub cli_login_cancel: Mutex<Option<tokio::sync::oneshot::Sender<()>>>,

    /// Stdin handle for the running CLI login child process.
    /// Written to by `set_provider_auth` to deliver the OAuth device code.
    pub cli_login_stdin: Mutex<Option<tokio::process::ChildStdin>>,

    /// IPC HTTP server port
    pub ipc_port: Mutex<u16>,

    /// IPC bearer token — injected into the page alongside the port.
    /// Verified on every IPC request to prevent unauthorized local access.
    pub ipc_token: String,

    /// CEF Browser handles keyed by window label (multi-window support).
    /// "main" is the primary window; tear-off windows get "window-{UUID}" labels.
    ///
    /// **Phase B / multi-reducer status**: this map is intentionally NOT
    /// migrated to launcher under the standard B.5 a→b→c→d→e ratchet because
    /// `cef::Browser` is an FFI handle to a Chromium browser object in this
    /// process — it can't serialize over IPC and must stay co-located with
    /// the CEF runtime. The KEY-SET role (label set queries) is already
    /// covered by the launcher's `state.windows` mirror (B.4) +
    /// `shadow_window_meta`. This field will be retired into a host-side
    /// reducer in Phase F (see `docs/retro/multi-reducer-proposal-2026-04-28.md`).
    pub browsers: Mutex<HashMap<String, Browser>>,

    /// Per-window metadata (kind, parent linkage).
    ///
    /// **Phase B status**: synchronous host-side cache mirroring the
    /// launcher's `state.windows` mirror. Single canonical mutation site:
    /// inserted in `client.rs::on_after_created` from the popped
    /// `PendingWindowCreation` entry, removed in `on_before_close`.
    /// Required for synchronous lookups that can't tolerate the launcher
    /// round-trip lag (`open_subwindow` parent-liveness check; cascade-close
    /// child enumeration). See `docs/retro/migration-pattern.md` for the
    /// sync-cache exception and `b5-migration-architecture-2026-04-28.md`
    /// for why step e ≠ delete here.
    pub window_meta: Mutex<HashMap<String, WindowMeta>>,

    /// Phase F.1 — host reducer state.
    ///
    /// Owns `pending_window_creations` (formerly a top-level
    /// `Mutex<VecDeque<PendingWindowCreation>>` field on AppState).
    /// All mutations go through `host_dispatch`; reads use the
    /// `peek_back_pending_window_creation` snapshot helper.
    /// Future PRs will migrate `active_drag` and tear-off-hook state
    /// here too. See `agentmux-cef/src/reducer.rs` and
    /// `docs/specs/SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md`.
    pub host_state: Mutex<crate::reducer::HostState>,

    /// Tear-off Phase 6 — pre-warmed pool of hidden CEF windows ready for
    /// instant promotion on tear-off. Each entry is a label of a window
    /// that's already painted, has its renderer connected, and is sitting
    /// in pool-mode (`?pool=1` URL flag) waiting to be assigned a workspace.
    /// On tear-off: pop a label, reposition + show + emit `pool:promote`.
    /// Cold-path (open_window_at_position) remains as defence-in-depth.
    /// See SPEC_TAB_TEAR_OFF_SIZE_PRESERVATION_2026_04_26 §4.5.
    ///
    /// **Phase B / multi-reducer status**: this map is intentionally NOT
    /// migrated to launcher under the B.5 ratchet — pool decisions need
    /// synchronous host-local state (e.g., `window_pool.len() <
    /// POOL_TARGET_SIZE` for refill triggers) that can't tolerate the
    /// launcher round-trip lag. The launcher's `state.pool: HashSet<String>`
    /// (B.4) mirrors the conceptual inventory for cross-process queries.
    /// Will become host-reducer state in Phase F (see
    /// `docs/retro/multi-reducer-proposal-2026-04-28.md`).
    pub window_pool: Mutex<VecDeque<String>>,
    /// Labels of pool windows that have been spawned but NOT yet
    /// promoted. Populated synchronously in `spawn_pool_window` so
    /// callers (list_windows, app-exit decision) can identify pool
    /// windows BEFORE the renderer-ready handshake fires and
    /// `window_pool` gets populated. Removed on promote, on
    /// destroy-before-promote, and during pool teardown.
    /// Without this, list_windows reports unpromoted pool windows
    /// as user-visible instances during the ~100ms gap between
    /// spawn and renderer-ready.
    ///
    /// **Phase B / multi-reducer status**: same as `window_pool` —
    /// host-only synchronous lifecycle scaffolding; not migrated under B.5
    /// ratchet. Phase F (host reducer) will retire it.
    pub unpromoted_pool_labels: Mutex<std::collections::HashSet<String>>,
    /// Single in-flight respawn semaphore — prevents pool refill from
    /// stacking spawns when the user does back-to-back tear-offs faster
    /// than CEF can create windows.
    ///
    /// **Phase B status**: host-only sync primitive; not migrate-able.
    pub window_pool_respawn_in_flight: std::sync::atomic::AtomicBool,

    /// Phase B.9.3 — set true once `on_before_close` decides
    /// "this is the last user-visible window closing". Tells
    /// `spawn_pool_window` to skip refill so the existing pool
    /// can drain. Without this, every pool close triggers a
    /// refill that adds a new pool browser, keeping
    /// `state.browsers` non-empty forever and preventing
    /// `quit_message_loop` from ever reaching idle.
    pub is_quitting: std::sync::atomic::AtomicBool,

    /// Version-specific data directory (e.g. ai.agentmux.cef.v0-32-111/)
    pub version_data_dir: Mutex<Option<String>>,

    /// Version-specific config directory
    pub version_config_dir: Mutex<Option<String>>,

    /// User data home used by the frontend's `agentmuxHome()` helper to
    /// construct per-agent paths (working dir, `GH_CONFIG_DIR`, etc.).
    ///
    /// - Portable: `<portable>/data/` — keeps agent state inside the portable
    ///   folder so two coexisting portables don't clobber each other on the
    ///   same `~/.agentmux/agents/<slug>/` path (slugs are unique per Forge
    ///   DB, not globally; see `docs/specs/portable-agent-working-dirs.md`).
    /// - Installed: `~/.agentmux/` — preserves existing behavior.
    ///
    /// `AGENTMUX_DATA_HOME` env var, if set, overrides both.
    pub user_home_dir: Mutex<Option<String>>,

    /// Active cross-window drag session (at most one at a time).
    pub active_drag: Mutex<Option<DragSession>>,

    /// Embedded browser panes (native CefBrowserView per pane).
    pub browser_panes: crate::browser_panes::BrowserPaneManager,

    /// Browser DOM API state — CDP target cache + future connection
    /// pool. See `crate::browser_api`.
    pub browser_api: crate::browser_api::BrowserApiState,

    /// CEF remote debugging port (9223 dev / 9222 release). Populated
    /// by `main.rs` from the same `is_dev` branch that sets
    /// `Settings.remote_debugging_port`. Used by the browser DOM API
    /// (`/agentmux/browser/*`) to open CDP WebSocket connections to
    /// pane targets. See `docs/specs/SPEC_BROWSER_DOM_API.md` §6.
    pub debug_port: Mutex<u16>,

    // Phase B.1 removed `job_handle` (was Windows-only). Launcher
    // owns J0 wrapping srv now; host no longer needs its own job.
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            auth_key: Mutex::new(uuid::Uuid::new_v4().to_string()),
            backend_endpoints: Mutex::new(BackendEndpoints::default()),
            sidecar_child: Mutex::new(None),
            backend_pid: Mutex::new(None),
            backend_started_at: Mutex::new(None),
            zoom_factor: Mutex::new(1.0),
            client_id: Mutex::new(None),
            window_id: Mutex::new(None),
            active_tab_id: Mutex::new(None),
            window_init_status: Mutex::new(String::new()),
            shadow_backend_window_ids: Mutex::new(HashMap::new()),
            shadow_window_meta: Mutex::new(HashMap::new()),
            shadow_instance_registry: Mutex::new({
                // Pre-seed with main=1 to mirror the launcher's
                // pre-seeded `instance_registry` (B.5a). Without
                // this, the very first drift compare would log a
                // spurious mismatch for "main" before any event
                // arrives.
                let mut m = HashMap::new();
                m.insert("main".to_string(), 1);
                m
            }),
            cli_login_cancel: Mutex::new(None),
            cli_login_stdin: Mutex::new(None),
            ipc_port: Mutex::new(0),
            ipc_token: uuid::Uuid::new_v4().to_string(),
            browsers: Mutex::new(HashMap::new()),
            window_meta: Mutex::new(HashMap::new()),
            host_state: Mutex::new(crate::reducer::HostState::default()),
            window_pool: Mutex::new(VecDeque::new()),
            unpromoted_pool_labels: Mutex::new(std::collections::HashSet::new()),
            window_pool_respawn_in_flight: std::sync::atomic::AtomicBool::new(false),
            is_quitting: std::sync::atomic::AtomicBool::new(false),
            version_data_dir: Mutex::new(None),
            version_config_dir: Mutex::new(None),
            user_home_dir: Mutex::new(None),
            active_drag: Mutex::new(None),
            browser_panes: crate::browser_panes::BrowserPaneManager::new(),
            browser_api: crate::browser_api::BrowserApiState::new(),
            debug_port: Mutex::new(0),
        }
    }
}

impl AppState {
    /// Phase F.1 — dispatch a command through the host reducer.
    ///
    /// Locks `host_state`, applies the command via `reducer::update`,
    /// logs emitted events via tracing, and returns the dispatch
    /// output (which contains the events plus the dequeued entry for
    /// `DequeuePendingWindowCreation`).
    ///
    /// Lock-hold time: pure-function reducer call, no I/O — typically
    /// sub-microsecond. Never held across a `SendMessage`, CEF
    /// callback, or any blocking call (snapshot-and-drop discipline,
    /// see `docs/specs/SPEC_PHASE_F_HOST_REDUCER_2026-05-01.md` §6).
    pub fn host_dispatch(&self, cmd: crate::reducer::HostCommand) -> crate::reducer::DispatchOutput {
        let out = {
            let mut state = self.host_state.lock();
            crate::reducer::update(&mut state, cmd)
        };
        for ev in &out.events {
            log_host_event(ev);
        }
        out
    }

    /// Phase F.1 — non-mutating peek at the back of the
    /// `pending_window_creations` queue.
    ///
    /// Used by `wrr/win_event.rs::handle_event` to label OS-level
    /// `WM_CREATE` events with the upcoming window's label. CEF's
    /// `OnAfterCreated` (which becomes the dequeue) fires AFTER this
    /// OS event, but the host pushed the entry BEFORE calling
    /// `post_create_window`, so back-of-queue is the right answer at
    /// this moment.
    ///
    /// Snapshot-and-drop: takes the lock, clones the entry, drops
    /// the lock. Callers never hold the lock past this call.
    pub fn peek_back_pending_window_creation(&self) -> Option<PendingWindowCreation> {
        self.host_state.lock().pending_window_creations.back().cloned()
    }

    // ── Phase H.2.b — browser read helpers (with fallback) ──────────────
    //
    // These wrap reads against `HostState.browsers` (the new reducer-managed
    // map) with fallback to the legacy `AppState.browsers` mutex. Drift
    // (entry in one but not the other) is logged via
    // `target = "host-reducer:drift"` so production smoke can verify the
    // parallel-write invariant before PR #4 flips reads to reducer-only.
    //
    // Snapshot-and-drop: each helper takes the relevant lock(s) briefly,
    // clones what's needed, drops the lock. Callers never hold either
    // lock across CEF FFI calls.

    /// Get a browser handle by label. Reducer first; legacy fallback.
    pub fn get_browser(&self, label: &str) -> Option<Browser> {
        let from_reducer = self
            .host_state
            .lock()
            .browsers
            .get(label)
            .map(|h| h.browser.clone());
        let from_legacy = self.browsers.lock().get(label).cloned();
        match (&from_reducer, &from_legacy) {
            (Some(_), None) => tracing::warn!(
                target: "host-reducer:drift",
                label = %label,
                "browser in reducer but missing from legacy",
            ),
            (None, Some(_)) => tracing::warn!(
                target: "host-reducer:drift",
                label = %label,
                "browser in legacy but missing from reducer",
            ),
            _ => {}
        }
        // Prefer reducer; fall back to legacy. Both should agree post-H.2.a.
        from_reducer.or(from_legacy)
    }

    /// Check whether a browser is registered under the given label.
    pub fn has_browser(&self, label: &str) -> bool {
        let in_reducer = self.host_state.lock().browsers.contains_key(label);
        if in_reducer {
            return true;
        }
        let in_legacy = self.browsers.lock().contains_key(label);
        if in_legacy {
            tracing::warn!(
                target: "host-reducer:drift",
                label = %label,
                "browser in legacy but missing from reducer",
            );
        }
        in_legacy
    }

    /// Are there any registered browsers?
    pub fn browsers_is_empty(&self) -> bool {
        let reducer_empty = self.host_state.lock().browsers.is_empty();
        let legacy_empty = self.browsers.lock().is_empty();
        if reducer_empty != legacy_empty {
            tracing::warn!(
                target: "host-reducer:drift",
                reducer_empty,
                legacy_empty,
                "browsers is_empty drift",
            );
        }
        // Conservatively report not-empty if either side has entries
        // (preserves caller's "no windows exist yet" decisions).
        reducer_empty && legacy_empty
    }

    /// Snapshot of all registered browser labels (HashMap iteration order;
    /// callers must not assume ordering).
    pub fn list_browser_labels(&self) -> Vec<String> {
        use std::collections::HashSet;
        let reducer_labels: HashSet<String> = self
            .host_state
            .lock()
            .browsers
            .keys()
            .cloned()
            .collect();
        let legacy_labels: HashSet<String> = self.browsers.lock().keys().cloned().collect();
        if reducer_labels != legacy_labels {
            let only_reducer: Vec<&String> =
                reducer_labels.difference(&legacy_labels).collect();
            let only_legacy: Vec<&String> =
                legacy_labels.difference(&reducer_labels).collect();
            tracing::warn!(
                target: "host-reducer:drift",
                only_reducer = ?only_reducer,
                only_legacy = ?only_legacy,
                "browser label set drift",
            );
        }
        // Use legacy as authoritative until H.2.c flips reads.
        legacy_labels.into_iter().collect()
    }

    /// Snapshot of all registered browsers as (label, Browser) pairs.
    /// Ordering is HashMap iteration order; callers must sort if order
    /// matters.
    pub fn list_browsers(&self) -> Vec<(String, Browser)> {
        let from_reducer: Vec<(String, Browser)> = self
            .host_state
            .lock()
            .browsers
            .iter()
            .map(|(k, h)| (k.clone(), h.browser.clone()))
            .collect();
        let from_legacy: Vec<(String, Browser)> = self
            .browsers
            .lock()
            .iter()
            .map(|(k, b)| (k.clone(), b.clone()))
            .collect();
        if from_reducer.len() != from_legacy.len() {
            tracing::warn!(
                target: "host-reducer:drift",
                reducer_count = from_reducer.len(),
                legacy_count = from_legacy.len(),
                "browser count drift in list_browsers",
            );
        }
        // Legacy is authoritative until flip.
        from_legacy
    }

    /// First registered browser (for "any browser" callers like command
    /// palette routing). Returns the label + Browser pair, or None if
    /// the registry is empty.
    pub fn first_browser(&self) -> Option<(String, Browser)> {
        // Prefer the legacy iteration order (matches existing code's
        // `.values().next()` semantics) until flip.
        self.browsers
            .lock()
            .iter()
            .next()
            .map(|(k, b)| (k.clone(), b.clone()))
    }

    // ── Phase H.1.b — pane lifecycle read helpers (with fallback) ───────

    /// Returns the pane's label iff entry is `Live`. Used by op gates
    /// (focus/resize/navigate) — `None` indicates the pane is missing or
    /// in `Closing`, in which case the caller must short-circuit rather
    /// than touch the (possibly destroyed) HWND.
    pub fn live_pane_label(&self, block_id: &str) -> Option<String> {
        let from_reducer = self
            .host_state
            .lock()
            .panes
            .get(block_id)
            .filter(|e| e.lifecycle == PaneLifecycle::Live)
            .map(|e| e.label.clone());
        let from_legacy = self.browser_panes.test_live_label_of(block_id);
        if from_reducer != from_legacy {
            tracing::warn!(
                target: "host-reducer:drift",
                block_id = %block_id,
                reducer = ?from_reducer,
                legacy = ?from_legacy,
                "live_pane_label drift",
            );
        }
        from_legacy.or(from_reducer)
    }

    /// Snapshot of all `Live` pane labels. Used by `defocus_all` etc.
    pub fn live_pane_labels(&self) -> Vec<String> {
        use std::collections::HashSet;
        let reducer_labels: HashSet<String> = self
            .host_state
            .lock()
            .panes
            .values()
            .filter(|e| e.lifecycle == PaneLifecycle::Live)
            .map(|e| e.label.clone())
            .collect();
        let legacy_labels: HashSet<String> =
            self.browser_panes.test_live_labels().into_iter().collect();
        if reducer_labels != legacy_labels {
            let only_reducer: Vec<&String> =
                reducer_labels.difference(&legacy_labels).collect();
            let only_legacy: Vec<&String> =
                legacy_labels.difference(&reducer_labels).collect();
            tracing::warn!(
                target: "host-reducer:drift",
                only_reducer = ?only_reducer,
                only_legacy = ?only_legacy,
                "live_pane_labels set drift",
            );
        }
        legacy_labels.into_iter().collect()
    }

    /// Phase B.5e — authoritative instance-number lookup. Reads
    /// the launcher-fed `shadow_instance_registry`, which is the
    /// sole source of truth post-B.5e (host's
    /// `WindowInstanceRegistry` was deleted). The shadow is
    /// pre-seeded with `{"main": 1}` so the very first lookup
    /// during startup resolves before the launcher's first
    /// `WindowInstanceAssigned` event arrives.
    pub fn instance_num(&self, label: &str) -> Option<u32> {
        self.shadow_instance_registry.lock().get(label).copied()
    }

    /// Phase B.5 (window_id_map step e) — authoritative
    /// label→backend_window_id lookup. Reads from the
    /// launcher-fed `shadow_backend_window_ids`. Sole source of
    /// truth post-step-e (host's `window_id_map` was deleted).
    pub fn backend_window_id(&self, label: &str) -> Option<String> {
        self.shadow_backend_window_ids.lock().get(label).cloned()
    }

    /// Phase B.5 (window_meta step c) — authoritative WindowMeta
    /// lookup. Prefers the launcher-fed `shadow_window_meta`; falls
    /// back to host's local `window_meta` for the race window
    /// where host has just inserted the pre-create handoff but
    /// the launcher's `WindowOpened` event hasn't returned yet.
    /// Same prefer-shadow pattern as `instance_num` and
    /// `backend_window_id`.
    pub fn window_meta(&self, label: &str) -> Option<WindowMeta> {
        if let Some(meta) = self.shadow_window_meta.lock().get(label).cloned() {
            return Some(meta);
        }
        let fallback = self.window_meta.lock().get(label).cloned();
        if fallback.is_some() {
            tracing::debug!(
                target: "launcher-ipc:fallback",
                label = %label,
                "[window_meta] shadow miss — falling back to host's window_meta (B.5c transitional)"
            );
        }
        fallback
    }

    /// Phase B.5 (window_meta step c) — collect labels of Subwindows
    /// whose `parent_instance_id` points to `parent_label`. Used by
    /// `on_before_close`'s cascade-close logic.
    ///
    /// Returns the **union** of matches from `shadow_window_meta`
    /// (the launcher-fed projection) and host's `window_meta` (the
    /// eager pre-create handoff). The union covers a critical race:
    /// a parent may already have one mirrored subwindow AND a newly
    /// opened sibling whose `WindowOpened` event hasn't returned to
    /// host yet. The newer sibling lives in host's `window_meta`
    /// only; the mirrored one lives in shadow only (post-step-d
    /// the shadow becomes the sole source). Cascade-close MUST
    /// catch both — short-circuiting on shadow-non-empty would
    /// leave the race-window sibling orphaned. (codex P1 PR #591
    /// round-1.)
    ///
    /// Dedup is by label; if a label is in both, it's reported
    /// once. Labels collected via a HashSet to dedup, returned as
    /// a Vec.
    pub fn subwindow_children_of(&self, parent_label: &str) -> Vec<String> {
        let mut out: std::collections::HashSet<String> = std::collections::HashSet::new();
        for m in self.shadow_window_meta.lock().values() {
            if m.kind == WindowKind::Subwindow
                && m.parent_instance_id.as_deref() == Some(parent_label)
            {
                out.insert(m.label.clone());
            }
        }
        for m in self.window_meta.lock().values() {
            if m.kind == WindowKind::Subwindow
                && m.parent_instance_id.as_deref() == Some(parent_label)
            {
                out.insert(m.label.clone());
            }
        }
        out.into_iter().collect()
    }
}

/// Phase F.1 — observability hook for host-reducer events.
///
/// Called by `AppState::host_dispatch` after every `update()` call.
/// F.1 logs via tracing only; future PRs may add a broadcast channel
/// here when an event consumer (cross-process saga, frontend
/// dispatcher) appears.
fn log_host_event(ev: &crate::reducer::HostEvent) {
    use crate::reducer::HostEvent;
    match ev {
        HostEvent::PendingWindowEnqueued {
            label,
            queue_len_after,
            version,
        } => tracing::debug!(
            target: "host-reducer",
            event = "PendingWindowEnqueued",
            label = %label,
            queue_len_after,
            version,
        ),
        HostEvent::PendingWindowDequeued {
            label,
            queue_len_after,
            version,
        } => tracing::debug!(
            target: "host-reducer",
            event = "PendingWindowDequeued",
            label = %label,
            queue_len_after,
            version,
        ),
        HostEvent::PendingWindowQueueEmpty { version } => tracing::warn!(
            target: "host-reducer",
            event = "PendingWindowQueueEmpty",
            version,
            "[host-reducer] dequeue on empty queue — caller will fall back",
        ),
        // ── H.1 panes ────────────────────────────────────────────────────
        HostEvent::PaneCreateRequested { block_id, label, version } => tracing::info!(
            target: "host-reducer",
            event = "PaneCreateRequested",
            block_id = %block_id, label = %label, version,
        ),
        HostEvent::PaneLive { block_id, label, version } => tracing::info!(
            target: "host-reducer",
            event = "PaneLive",
            block_id = %block_id, label = %label, version,
        ),
        HostEvent::PaneClosing { block_id, version } => tracing::info!(
            target: "host-reducer",
            event = "PaneClosing",
            block_id = %block_id, version,
        ),
        HostEvent::PaneClosed { block_id, version } => tracing::info!(
            target: "host-reducer",
            event = "PaneClosed",
            block_id = %block_id, version,
        ),
        HostEvent::PaneCreationFailed { block_id, reason, version } => tracing::warn!(
            target: "host-reducer",
            event = "PaneCreationFailed",
            block_id = %block_id, reason = %reason, version,
        ),
        // ── H.2 browsers ─────────────────────────────────────────────────
        HostEvent::BrowserRegistered { label, kind, version } => tracing::info!(
            target: "host-reducer",
            event = "BrowserRegistered",
            label = %label, kind = ?kind, version,
        ),
        HostEvent::BrowserUnregistered { label, version } => tracing::info!(
            target: "host-reducer",
            event = "BrowserUnregistered",
            label = %label, version,
        ),
        // ── H.3 drag ─────────────────────────────────────────────────────
        HostEvent::DragStarted { drag_id, source_window, version } => tracing::info!(
            target: "host-reducer",
            event = "DragStarted",
            drag_id = %drag_id, source_window = %source_window, version,
        ),
        HostEvent::DragEnded { drag_id, outcome, version } => tracing::info!(
            target: "host-reducer",
            event = "DragEnded",
            drag_id = %drag_id, outcome = ?outcome, version,
        ),
        // ── H.4 pool ─────────────────────────────────────────────────────
        HostEvent::PoolWindowEntered { label, queue_len_after, version } => tracing::info!(
            target: "host-reducer",
            event = "PoolWindowEntered",
            label = %label, queue_len_after, version,
        ),
        HostEvent::PoolWindowLeft { label, queue_len_after, reason, version } => tracing::info!(
            target: "host-reducer",
            event = "PoolWindowLeft",
            label = %label, queue_len_after, reason = ?reason, version,
        ),
        HostEvent::PoolEmpty { version } => tracing::info!(
            target: "host-reducer",
            event = "PoolEmpty",
            version,
        ),
        // ── H.5 quit ─────────────────────────────────────────────────────
        HostEvent::QuitDraining { reason, version } => tracing::warn!(
            target: "host-reducer",
            event = "QuitDraining",
            reason = ?reason, version,
            "[host-reducer] entering drain mode",
        ),
        HostEvent::QuitReady { version } => tracing::warn!(
            target: "host-reducer",
            event = "QuitReady",
            version,
            "[host-reducer] drain complete; host quitting",
        ),
        // ── H.6 top-level runner ─────────────────────────────────────────
        HostEvent::TopLevelCreationRequested {
            creation_id, source, label, version,
        } => tracing::info!(
            target: "host-reducer",
            event = "TopLevelCreationRequested",
            creation_id, source = ?source, label = %label, version,
        ),
        HostEvent::TopLevelCreationStarted { creation_id, label, version } => tracing::info!(
            target: "host-reducer",
            event = "TopLevelCreationStarted",
            creation_id, label = %label, version,
        ),
        HostEvent::TopLevelCreationCompleted {
            creation_id, label, latency_ms, version,
        } => tracing::info!(
            target: "host-reducer",
            event = "TopLevelCreationCompleted",
            creation_id, label = %label, latency_ms, version,
        ),
        HostEvent::TopLevelCreationFailed {
            creation_id, label, outcome, version,
        } => tracing::error!(
            target: "host-reducer",
            event = "TopLevelCreationFailed",
            creation_id, label = %label, outcome = ?outcome, version,
        ),
        HostEvent::TopLevelQueueLengthChanged { len, version } => tracing::debug!(
            target: "host-reducer",
            event = "TopLevelQueueLengthChanged",
            len, version,
        ),
        // ── Effect carrier ───────────────────────────────────────────────
        HostEvent::Effect { effect, version } => tracing::debug!(
            target: "host-reducer",
            event = "Effect",
            effect = ?effect, version,
        ),
        HostEvent::Error { message, version } => tracing::warn!(
            target: "host-reducer",
            event = "Error",
            version,
            "[host-reducer] {}", message,
        ),
    }
}
