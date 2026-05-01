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
        HostEvent::Error { message, version } => tracing::warn!(
            target: "host-reducer",
            event = "Error",
            version,
            "[host-reducer] {}", message,
        ),
    }
}
