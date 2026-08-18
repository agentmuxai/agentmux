// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Shared application state for the CEF host.

use std::collections::HashMap;
use parking_lot::Mutex;

use cef::{Browser, Frame};

mod drag;
mod window_meta;
mod browser_pane;
mod browser_handle;
mod pool;
mod quit;
mod top_level_creation;
mod pending_reproject;
mod ui_thread_gate;

pub use drag::{DragType, DragPayload, DragSession};
pub use window_meta::{WindowKind, WindowMeta, PendingWindowCreation};
pub use browser_pane::{
    BrowserPaneLifecycle, BrowserPaneEntry, PaneRect, WindowPlacement, PaneWindowState,
};
pub use browser_handle::{BrowserHandle, BrowserKind};
pub use pool::{PoolState, PanePoolState};
pub use quit::{QuitState, QuitReason};
pub use top_level_creation::{
    TopLevelCreationRequest, TopLevelSource, InFlightCreation, CreationPhase,
    CompletedCreation, TopLevelCreationOutcome, TopLevelCreationState, EffectKind,
    PendingBrowserPaneCreate, FloatingRedockGhostState,
};
pub use pending_reproject::PendingReprojectClosures;
pub use ui_thread_gate::{UiThreadGate, MainReadyAction, SnapshotAction};

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

    /// Number of data migrations still pending after the in-process startup run,
    /// as reported in ESTART (forwarded via AGENTMUX_PENDING_MIGRATIONS on the
    /// launcher path). Non-zero means run_pending_migrations failed at startup;
    /// the status-bar shows "Migration failed — restart to retry."
    pub pending_migrations: Mutex<usize>,

    /// Guard against concurrent `run_migrations` invocations from the maintenance panel.
    pub migration_running: Mutex<bool>,

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

    /// Linux startup white-flash fix (docs/specs/SPEC_LINUX_STARTUP_PAINT_GATING_2026_07_13.md).
    /// Window labels whose native `window.show()`/focus + splash-ready-file has
    /// been deferred by `on_load_end` pending a real first-paint confirmation
    /// from the frontend (`report_first_paint` IPC command) or a safety-net
    /// timeout, whichever fires first. Value is `(armed_at, epoch)` — `epoch`
    /// guards against a stale safety-net timeout from an earlier `on_load_end`
    /// call for the same label (e.g. a reload/retry mid-startup re-arms the
    /// gate) firing at its original, now-too-early deadline and revealing the
    /// window ahead of the *current* navigation's real paint. Mirrors the
    /// epoch pattern in `browser_pane::auth`. Only populated/consumed on
    /// Linux; the field exists unconditionally to keep `AppState` un-cfg'd.
    pub linux_paint_gate_pending: Mutex<std::collections::HashMap<String, (std::time::Instant, u64)>>,

    /// Linux startup white-flash fix, second-round race (reagent PR #2151
    /// review): `report_first_paint` can reach the UI thread before
    /// `on_load_end` arms `linux_paint_gate_pending` for the same label — CEF's
    /// main-frame load-complete isn't guaranteed to fire after the frontend's
    /// first compositor frame (render-blocking stylesheets can resolve, and a
    /// frame can paint, before other load-blocking resources finish and
    /// `on_load_end` runs). Labels land here when the signal arrives too
    /// early; `on_load_end` checks this set before arming so it can reveal
    /// immediately instead of falling through to the slower safety timeout.
    /// Only populated/consumed on Linux; unconditional field for the same
    /// reason as `linux_paint_gate_pending` above.
    pub linux_first_paint_seen: Mutex<std::collections::HashSet<String>>,

    /// Browser-pane top-level navigation watchdog. Chromium's underlying TCP
    /// connect-timeout ceiling (`net::TransportConnectJob::ConnectionTimeout`)
    /// is 4 minutes — the SAME ceiling real Chrome ships with, since it's the
    /// same net-stack code — which is far too long for a floating pane to sit
    /// on a blank "loading" state. Armed in
    /// `client::lifecycle::on_before_browse` when a pane's main-frame
    /// navigation is about to start, and disarmed in
    /// `browser_pane::callbacks::on_loading_state_change_browser_pane` when it
    /// ends (`is_loading == false`, whether the navigation committed or CEF's
    /// own `on_load_error` already fired). If a navigation is still armed when
    /// its delayed watchdog task runs, we cancel it ourselves and show a
    /// synthetic `ERR_CONNECTION_TIMED_OUT` error page instead of waiting out
    /// Chromium's full ceiling. Keyed by pane `block_id`; value is
    /// `(armed_at, epoch, browser, url)` — `epoch` guards against a stale
    /// timeout firing after a newer navigation re-armed the same pane (mirrors
    /// `linux_paint_gate_pending`'s epoch pattern above); `browser` is the
    /// cloned handle the delayed task navigates when it fires; `url` is the
    /// navigation's OWN target URL, captured from CEF's `Request` at
    /// `on_before_browse` time. Deliberately NOT re-derived from
    /// `frame.main_frame().url()` at fire time — that getter reflects the
    /// frame's last COMMITTED document, which for a navigation that never
    /// committed (the exact case this watchdog exists for) is still the
    /// PREVIOUS page. An earlier version did that and showed the pane's old
    /// URL ("Could not connect to <previous page>") instead of the one the
    /// user actually tried to reach.
    pub browser_pane_load_watchdog:
        Mutex<std::collections::HashMap<String, (std::time::Instant, u64, Browser, String)>>,

    /// Set of pane `block_id`s whose MAIN FRAME is currently between
    /// navigation-start and load-finish. This is the frontend loading
    /// spinner's actual `is_loading` source of truth, deliberately separate
    /// from `browser_pane_load_watchdog` above (which disarms at
    /// navigation COMMIT via `on_load_start_browser_pane` — correct for the
    /// watchdog's own "did this ever commit" purpose, but far too early for
    /// "has the page finished loading") and from CEF's raw
    /// `on_loading_state_change` callback (which aggregates loading state
    /// across the WHOLE frame tree — no frame parameter on that callback at
    /// all — so a sub-frame/subresource load, e.g. an ad iframe or chat
    /// widget, can flip it long after the main document is done). Inserted
    /// in `client::lifecycle::on_before_browse`'s main-frame branch, removed
    /// in `browser_pane::callbacks::on_load_end_browser_pane` (main-frame
    /// load actually finished) or a main-frame, non-`ERR_ABORTED`
    /// `on_load_error`. See
    /// `docs/specs/SPEC_BROWSER_PANE_LOADING_INDICATOR_FLICKER_2026_08_17.md`
    /// layer 1 for the full diagnosis and design.
    pub browser_pane_main_frame_loading: Mutex<std::collections::HashSet<String>>,

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

    /// PID of an in-progress PTY-backed CLI login child. The pipe
    /// path uses `cli_login_cancel` + `tokio::select!` + Tokio's
    /// `kill_on_drop` Child to terminate, but the PTY path moves
    /// the `portable_pty` child into a `spawn_blocking` task that
    /// outlives the outer abort — so cancel needs a PID + platform
    /// kill to actually stop the subprocess. Populated by
    /// `run_cli_login_pty`, cleared when the child exits naturally.
    pub cli_login_pty_pid: Mutex<Option<u32>>,

    /// Stdin handle for the running CLI login child process. Two
    /// variants because some providers (OpenClaw) require an
    /// interactive TTY for their auth subcommand and we spawn them via
    /// `portable_pty` instead of `tokio::process::Command`. Written to
    /// by `set_provider_auth` to deliver an OAuth device code or
    /// pasted token. See `commands::cli_login::CliLoginStdin`.
    pub cli_login_stdin: Mutex<Option<crate::commands::cli_login::CliLoginStdin>>,

    /// Monotonic login-attempt counter. `run_cli_login` bumps it on every
    /// attempt. A login's reaper task captures the value at spawn and only
    /// clears the shared `cli_login_*` slots if it still matches — so a
    /// SUPERSEDED login (whose child is killed by the next attempt) cannot
    /// null out the slots the new attempt just repopulated. Clearing the new
    /// login's stdin handle out from under it is exactly the "stuck login"
    /// bug, where the pasted code has nowhere to be delivered.
    pub cli_login_generation: std::sync::atomic::AtomicU64,

    /// True while a CLI login child (either transport) is alive. Set by
    /// `run_cli_login`/`run_cli_login_pty` right after spawn, cleared by
    /// their reaper tasks once the child has actually exited — the SAME
    /// generation guard `cli_login_stdin`'s clear uses, so a superseded
    /// login's reaper can't falsely mark a newer one inactive.
    ///
    /// Deliberately independent of `cli_login_stdin`: `set_provider_auth`
    /// `.take()`s that slot the instant a pasted code is delivered (it's
    /// single-use), but the child itself keeps running for a bit afterward
    /// while it exchanges the code with the OAuth service. Before this
    /// field existed, `get_cli_login_status` read `cli_login_stdin.is_some()`
    /// directly — reporting `active: false` the moment a code was pasted,
    /// which could make `pollForInAppLoginCompletion` (spec
    /// SPEC_INAPP_CLAUDE_OAUTH_LOGIN_2026_08_03.md §3.1) time out and kill
    /// a login that was still genuinely completing (codex P1 on PR #2410).
    pub cli_login_active: std::sync::atomic::AtomicBool,

    /// Credential-file path + its mtime (`None` = file didn't exist yet)
    /// captured right before the CURRENT login child spawns. `get_cli_
    /// login_status` compares this baseline against the file's live mtime
    /// to report `credential_changed` — required, alongside `!active`, for
    /// `pollForInAppLoginCompletion` to accept a completion.
    ///
    /// Fixes reagent P1 on PR #2410: `active` alone can't distinguish "this
    /// attempt genuinely completed" from "the child crashed/exited early
    /// while a stale-but-still-file-shaped credential from BEFORE this
    /// attempt started" — `CheckCliAuthCommand` only checks local presence,
    /// not server-side validity (a present-but-expired token still reports
    /// authenticated; see force-login.ts's doc comment), so a reconnect
    /// into an account whose isolated dir already holds an old credential
    /// could read `authenticated: true` off that untouched file the instant
    /// the new child dies, before it ever wrote anything.
    pub cli_login_cred_baseline: Mutex<Option<(std::path::PathBuf, Option<std::time::SystemTime>)>>,

    /// IPC HTTP server port
    pub ipc_port: Mutex<u16>,

    /// IPC bearer token — injected into the page alongside the port.
    /// Verified on every IPC request to prevent unauthorized local access.
    pub ipc_token: String,

    // Phase H.2.e (PR #4) — `pub browsers: Mutex<HashMap<String, Browser>>`
    // deleted. Authoritative storage is now `HostState.browsers` (the host
    // reducer's map). Read access goes through `AppState::get_browser`,
    // `list_browsers`, etc. (state.rs:704-742). The H.2 ratchet
    // (a → parallel writes, b → reads with fallback, c → flip reads,
    // d → drop legacy writes, e → delete) is complete for browsers.
    // Pane lifecycle (`PaneStateMachine`) follows in PR #5.

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

    /// Phase F.7 host-bridge dedup. Keyed by `"{event_kind}|{label}|{hwnd}"`,
    /// value is the highest launcher-event version dispatched to renderers
    /// for that key. The bridge skips dispatch if the incoming event's
    /// version is `<=` the cached version — preserving subscriber
    /// idempotency under §8.14 even if the renderer-side guard fails
    /// (multiple V8 contexts, fresh-renderer post-crash, race during
    /// init, etc.).
    ///
    /// Originally proposed in `ANALYSIS_DRIFT_STORM_RENDERER_CRASH_2026-05-06.md`
    /// §4.4 / master spec §9.8 as Phase F.7. Implementation forced when
    /// v0.33.688 smoke surfaced a 164× amplification of a single
    /// `HiddenSinceOpen` drift event v=78 — launcher cap (PR #721)
    /// prevented multi-emit but the bridge fanned the one event into
    /// many V8 contexts, exhausting the renderer.
    ///
    /// Bounded at 4096 keys with FIFO eviction (insertion order). A
    /// re-arrival for an evicted key bypasses dedup once but the
    /// renderer guard still catches it.
    pub launcher_bridge_dedup: Mutex<crate::launcher_event_bridge::DedupCache>,

    /// Linux/macOS only — registry of live top-level `CefWindow` handles,
    /// keyed by window label ("main" for the primary, otherwise the label
    /// CreateWindowTask assigns to a sub-window). Populated by
    /// `AgentMuxWindowDelegate::on_window_created` and cleared by
    /// `on_window_destroyed`. Used by the browser-pane Views path to find
    /// the right parent Window when attaching a pane via
    /// `Window::add_overlay_view` — without this map, panes opened from a
    /// non-main window were silently routed to the main window (PR #682
    /// follow-up: "the browser opens in the wrong window" bug).
    /// The Windows pane path uses native HWND lookup instead and never
    /// reads this field, so it is cfg-gated to avoid carrying unused
    /// state on Windows.
    #[cfg(not(target_os = "windows"))]
    pub windows: Mutex<std::collections::HashMap<String, cef::Window>>,

    /// Linux/macOS only — `CefOverlayController` handles for live browser
    /// panes, keyed by pane label (the same label used in `host_state.browsers`).
    /// Each entry pairs the controller with the `window_label` of the
    /// parent CefWindow it was attached to (looked up from `state.windows`
    /// at create time), so resize / detach / overlay-clip calls can find
    /// the right Window for `layout()` / `remove_child_view`.
    ///
    /// Populated by `browser_pane/creation_views.rs` after the BrowserView
    /// has been added to its parent Window via `add_overlay_view`. We use
    /// AddOverlayView (not AddChildView) because AddChildView cohabitates
    /// poorly with the host UI's BrowserView under the Window's default
    /// FillLayout — both children get full-parent-size bounds and the pane
    /// stacks on top of the host UI at full size with per-frame redraw
    /// flashing (verified empirically during the spike). OverlayController
    /// has its own explicit `set_size` / `set_position` / `set_visible`
    /// /`destroy` that aren't subject to the parent's auto-layout.
    /// Windows panes use HWND ops instead and never touch this map.
    #[cfg(not(target_os = "windows"))]
    pub browser_pane_overlays:
        Mutex<std::collections::HashMap<String, (String, cef::OverlayController)>>,

    /// macOS only — last-known REAL on-screen physical-px rect per pane
    /// label, mirroring whatever was last committed via the raw ObjC
    /// `setFrame:` path in `creation_views.rs` / `pane_geometry.rs`.
    ///
    /// `OverlayController::bounds()` cannot be trusted for this on macOS:
    /// CEF Views' own `set_size`/`set_position` are permanent no-ops on
    /// `NativeWidgetMacNSWindow` (see the extensive comments in
    /// `browser_pane/creation_views.rs`), so `bounds()` reflects a stale,
    /// DIP-scale value from whatever CEF's internal Views layout last
    /// computed — NOT the physical-px frame we forced via ObjC. Comparing
    /// that stale/wrong-scale rect against the overlay-clip rects (which
    /// ARE genuine physical px, matching `browser-view.tsx`'s
    /// `Math.round(v * dpr)` convention) silently fails to detect a real
    /// intersection, so `compute_pane_visible` reports `visible: true`
    /// even while a DOM menu is drawn directly on top of the pane — the
    /// menu displays correctly (occlusion isn't needed for painting, the
    /// DOM already paints over screen pixels) but the pane, still
    /// receiving events, intercepts clicks meant for the DOM underneath.
    /// `SetPaneOverlayClipViewsTask` reads from here instead.
    #[cfg(target_os = "macos")]
    pub browser_pane_physical_rects: Mutex<std::collections::HashMap<String, (i32, i32, i32, i32)>>,

    /// Linux/macOS only — latest overlay-clip rects per window_label.
    ///
    /// `browser_panes_set_overlay_clip` IPC publishes here so that BOTH the
    /// pane-airspace task (`SetPaneOverlayClipViewsTask`) and the per-pane
    /// resize task (`resize_browser_pane_view`) can compute pane visibility
    /// from the same authoritative state. Without this, resize would be the
    /// sole visibility authority on its path and `set_visible(1)` on a
    /// non-zero resize (e.g. user dragging a splitter while a DOM modal is
    /// open) would clobber the airspace's `set_visible(0)` — the
    /// "borderless DOM appears above modal" repro Codex caught on PR #881.
    ///
    /// Empty `Vec` for a key means "no overlays open in this window" →
    /// panes are visible (subject to their own rect being non-zero).
    /// Stale entries for closed windows are harmless: they only affect
    /// panes still attached to that window.
    #[cfg(not(target_os = "windows"))]
    pub pane_overlay_rects:
        Mutex<std::collections::HashMap<String, Vec<(i32, i32, i32, i32)>>>,

    /// Windows only — last-applied clip signature per pane *label*, so
    /// `BrowserPaneManager::set_pane_overlay_clip` can skip a redundant
    /// `SetWindowRgn` + `InvalidateRect` when a pane's computed region is
    /// unchanged since the previous call (the common case: one overlay moves,
    /// every other pane's clip is identical).
    ///
    /// Keyed by label, NOT by HWND: HWND integer values are recycled by the OS,
    /// so an HWND key risks a stale entry colliding with a recreated pane.
    ///
    /// Why a wrong-skip can't happen:
    /// - **Labels are globally unique and never reused** (monotonic
    ///   `BROWSER_PANE_LABEL_SEQ`; see `next_browser_pane_label`). A
    ///   close→recreate registers the new pane under a brand-new label, so the
    ///   label-keyed cache always *misses* and re-applies. This is the primary
    ///   guarantee.
    /// - The HWND value is also folded INTO the signature as cheap
    ///   belt-and-suspenders: even if a label were somehow reused, a different
    ///   (or recycled-but-distinct) HWND yields a different signature → re-apply.
    /// - `set_pane_overlay_clip` holds this lock across check → apply → record,
    ///   so the recorded signature always matches the region that call actually
    ///   applied even though the Windows handler runs on multiple tokio workers.
    ///
    /// We only skip when the same label already has the same region on the same
    /// HWND. Pruned to live labels each call to bound size.
    #[cfg(target_os = "windows")]
    pub pane_clip_cache: Mutex<std::collections::HashMap<String, u64>>,

    /// Linux/macOS only — OverlayControllers awaiting deferred destroy.
    ///
    /// Calling `OverlayController::destroy()` synchronously on the same UI
    /// tick as `BrowserHost::close_browser(force=1)` races CEF/Chromium's
    /// internal Views focus traversal — pending tasks hold
    /// `base::WeakPtr<View>` to the BrowserView, and yanking the view out
    /// of the Window's hierarchy before those tasks drain trips
    /// `weak_ptr.h:250 Check failed: ref_.IsValid()` and FATALs the host.
    /// Two confirmed reproducers: closing a pane while a pool window is
    /// being spawned (PR #743 follow-up) and tearing off a tab (a tear-off
    /// closes the pane in the source workspace then immediately creates a
    /// new top-level window for the torn tab).
    ///
    /// Fix: detach moves the controller out of `browser_pane_overlays`
    /// (so future resize/clip calls miss), calls `close_browser(force=1)`,
    /// and stashes the controller here. `on_before_close_browser_pane`
    /// then drains this map and runs `destroy()` — by that point the
    /// Browser is fully torn down and Chromium has drained the queued
    /// tasks that referenced its View, so destroy can't race anything.
    #[cfg(not(target_os = "windows"))]
    pub pending_overlay_destroy:
        Mutex<std::collections::HashMap<String, cef::OverlayController>>,

    /// macOS only — NSWindow windowNumber for each live browser-pane overlay,
    /// keyed by pane label.  Populated by `SetPaneBoundsViewsTask` after it
    /// resolves the overlay window, consumed by `resize_browser_pane_view` to
    /// pass an exact wnum instead of the highest-wnum fallback (ambiguous when
    /// ≥2 panes are open).  Entries are removed on detach.
    #[cfg(target_os = "macos")]
    pub browser_pane_overlay_wnums: Mutex<std::collections::HashMap<String, isize>>,

    /// Tear-off Phase 6 — pre-warmed pool of hidden CEF windows ready for
    /// instant promotion on tear-off. Each entry is a label of a window
    /// that's already painted, has its renderer connected, and is sitting
    /// in pool-mode (`?pool=1` URL flag) waiting to be assigned a workspace.
    /// On tear-off: pop a label, reposition + show + emit `pool:promote`.
    // PR #5 H.4 — `window_pool`, `unpromoted_pool_labels`, and
    // `window_pool_respawn_in_flight` deleted; the host reducer's
    // `HostState.pool: PoolState` is the sole source of truth. Reads
    // go through `AppState::pool_queue_size`,
    // `unpromoted_pool_labels_snapshot`, `is_unpromoted_pool_label`;
    // mutations through `HostCommand::PoolWindowSpawnStart`,
    // `PoolWindowReady`, `PoolWindowDestroyedBeforePromote`,
    // `PopAndPromoteFrontPoolWindow`, `PoolDrainAll`.

    // PR #5 H.5 — `is_quitting` AtomicBool deleted; the host reducer's
    // `HostState.quit_state: QuitState` is the sole source of truth.
    // Reads go through `AppState::is_quitting()`; transitions through
    // `HostCommand::BeginDrain` and `ConfirmDrained`.

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

    // PR #5 H.3 — `active_drag` field deleted; the host reducer's
    // `HostState.active_drag` is the sole source of truth. Reads go
    // through `AppState::get_drag_session`; mutations through
    // `HostCommand::StartDrag` / `EndDrag`.

    /// Embedded browser panes (native CefBrowserView per pane).
    pub browser_panes: crate::browser_panes::BrowserPaneManager,

    /// Per-pane Ctrl+Wheel zoom factor, keyed by block_id. Applied as CSS
    /// `zoom` via `ExecuteJavaScript` (`BrowserPaneManager::apply_zoom`),
    /// deliberately NOT Chromium's native page zoom — every browser pane
    /// shares its parent window's RequestContext (see
    /// docs/specs/pane-shares-window-request-context-linux-2026-05-13.md),
    /// so native zoom is scoped to HostZoomMap and shared across every pane
    /// on the same host/profile. CSS injection sidesteps that entirely: no
    /// RequestContext/HostZoomMap involvement, so no cookie/session sharing
    /// tradeoff, and it's per-CefFrame by construction. Absent entry means
    /// default (1.0, no injected style). Re-applied on every
    /// `on_load_end_browser_pane` (see browser_pane/callbacks.rs) since a
    /// fresh navigation replaces the page's own DOM/style state.
    pub browser_pane_zoom: Mutex<std::collections::HashMap<String, f64>>,

    /// The specific `Frame` right-clicked to open the unified browser-pane
    /// context menu (SPEC_BROWSER_PANE_UNIFIED_CONTEXT_MENU_2026_08_15.md),
    /// keyed by pane `block_id`. Populated by `client::context_menu`'s
    /// `run_context_menu` from CEF's own `frame` callback param — that's the
    /// actual frame that was right-clicked, which for a page containing
    /// sub-frames/iframes (ads, embeds, widgets) is NOT always
    /// `browser.main_frame()`. Consumed by the Copy/Cut/Paste menu items
    /// (`browser_panes::navigation::copy/cut/paste`) so they operate on
    /// whichever frame actually holds the selection/focus, instead of always
    /// acting on the top-level frame regardless of where the user actually
    /// right-clicked (reagentx P1 on PR #2599). Cleared on pane close
    /// alongside `browser_pane_zoom` (see `browser_pane::callbacks::on_before_close_browser_pane`).
    pub browser_pane_context_menu_frame: Mutex<std::collections::HashMap<String, Frame>>,

    /// Browser DOM API state — CDP target cache + future connection
    /// pool. See `crate::browser_api`.
    pub browser_api: crate::browser_api::BrowserApiState,

    /// CEF remote debugging port (9223 dev / 9222 release). Populated
    /// by `main.rs` from the same `is_dev` branch that sets
    /// `Settings.remote_debugging_port`. Used by the browser DOM API
    /// (`/agentmux/browser/*`) to open CDP WebSocket connections to
    /// pane targets. See `docs/specs/SPEC_BROWSER_DOM_API.md` §6.
    pub debug_port: Mutex<u16>,

    /// CEF `root_cache_path` (the version's `cef-cache` dir). Per-window
    /// RequestContexts are in-memory (unique off-the-record profiles — see
    /// `create_isolated_request_context`) and place nothing under it; the
    /// resolved path is kept for the startup legacy-litter sweep
    /// (`cleanup_legacy_context_dirs`) and any future consumers. See
    /// SPEC_CEF_LOG_ROBUSTNESS_2026_06_20.md §1.6.
    pub cef_cache_dir: Mutex<Option<String>>,

    // Phase B.1 removed `job_handle` (was Windows-only). Launcher
    // owns J0 wrapping srv now; host no longer needs its own job.

    /// Per-window opacity HWND registry. Populated by `set_window_init_status`
    /// once the window is fully shown (CEF Views returns NULL at on_after_created
    /// time). Stored as `isize` (the raw HWND value) so the map is `Send`.
    /// Read by `set_window_opacity` to target exactly one HWND instead of
    /// enumerating all process windows. See SPEC_PER_WINDOW_OPACITY_2026-05-14.md §5.
    #[cfg(target_os = "windows")]
    pub window_hwnds: Mutex<HashMap<String, isize>>,

    /// Phase 4b — per-window ghost state for floating-pane redock.
    /// Keyed by window label. The target renderer pushes its last-computed
    /// `{ block_id, dir }` here on each hover update; the floater queries
    /// it at drop time so the saga can emit a directional split action.
    /// Cleared by `clear_floating_redock_hover` (drag cancel/end).
    pub floating_redock_ghost: Mutex<HashMap<String, FloatingRedockGhostState>>,

    /// `window:transparent` setting read synchronously from settings.json before
    /// CefInitialize. Set once in `on_before_command_line_processing`; read in
    /// `on_context_initialized` to gate the transparent CEF command-line flags
    /// and to pass the value to the frontend via the URL query string.
    pub window_transparent: std::sync::atomic::AtomicBool,

    /// SPEC_PILLAR1_STEP4 Phase 2 — gates `reproject_from_snapshot` on proof
    /// that CEF's UI-thread message loop is actually pumping posted tasks.
    /// `post_task(ThreadId::UI, ...)` silently drops tasks posted before that
    /// point (verified live: `CreateWindowTask::execute` never ran and the
    /// browsers it should have created were never made — see
    /// retro-pillar1-step4-reproject-race). The launcher-ipc reader task can
    /// deliver `Event::Snapshot` before `"main"` registers (the proof point),
    /// since it runs on its own tokio runtime independent of CEF's message
    /// loop — so the event handler and `"main"`'s registration race to
    /// access this state from different threads.
    ///
    /// `ready` and `stashed` are ONE field, not two, deliberately: reagent's
    /// review of the first version (separate `AtomicBool` + `Mutex<Option<_>>`)
    /// caught a TOCTOU hole between them — the reader thread could read
    /// `ready == false`, then `"main"`'s registration could flip it and drain
    /// the (still-empty) stash, then the reader thread would write its
    /// snapshot into the stash *after* the one-time drain had already
    /// happened, so it would never replay. Both the check-then-stash and the
    /// flip-then-drain must happen under the same lock so one always
    /// completes before the other starts.
    pub ui_thread_gate: Mutex<UiThreadGate>,

    /// SPEC_PILLAR1_STEP4 Phase 3 addendum (reagent P1, PR #2032, 2026-07-08)
    /// — `new_label → old_srv_window_id`. The slow path's `reproject_from_srv`
    /// must NOT close the old window_id right after `open_window_with_kind`
    /// returns `Ok`: that only means a `CreateWindowTask` was posted to the
    /// UI thread (fire-and-forget), not that the window actually exists —
    /// this session's own Phase 2 investigation found `post_task` can
    /// silently drop a posted task. Closing on that unconfirmed signal would
    /// delete the old session's window/workspace/tabs with no replacement
    /// and no retry if creation then failed. Instead, the old id is stashed
    /// here keyed by the NEW label, and only closed once that label's own
    /// `register_backend_window` call fires — proof the new window's
    /// frontend actually loaded and round-tripped IPC. If creation silently
    /// fails, the entry just sits here unclosed forever, which is the same
    /// (safe, if imperfect) fallback behavior as before this whole cleanup
    /// existed: the old data lingers, but is never lost.
    pub pending_reproject_closures: Mutex<PendingReprojectClosures>,
}

impl Default for AppState {
    fn default() -> Self {
        Self {
            auth_key: Mutex::new(uuid::Uuid::new_v4().to_string()),
            backend_endpoints: Mutex::new(BackendEndpoints::default()),
            sidecar_child: Mutex::new(None),
            backend_pid: Mutex::new(None),
            backend_started_at: Mutex::new(None),
            pending_migrations: Mutex::new(
                std::env::var("AGENTMUX_PENDING_MIGRATIONS")
                    .ok()
                    .and_then(|v| v.parse::<usize>().ok())
                    .unwrap_or(0)
            ),
            migration_running: Mutex::new(false),
            zoom_factor: Mutex::new(1.0),
            client_id: Mutex::new(None),
            window_id: Mutex::new(None),
            active_tab_id: Mutex::new(None),
            window_init_status: Mutex::new(String::new()),
            linux_paint_gate_pending: Mutex::new(std::collections::HashMap::new()),
            linux_first_paint_seen: Mutex::new(std::collections::HashSet::new()),
            browser_pane_load_watchdog: Mutex::new(std::collections::HashMap::new()),
            browser_pane_main_frame_loading: Mutex::new(std::collections::HashSet::new()),
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
            cli_login_pty_pid: Mutex::new(None),
            cli_login_stdin: Mutex::new(None),
            cli_login_generation: std::sync::atomic::AtomicU64::new(0),
            cli_login_active: std::sync::atomic::AtomicBool::new(false),
            cli_login_cred_baseline: Mutex::new(None),
            ipc_port: Mutex::new(0),
            ipc_token: uuid::Uuid::new_v4().to_string(),
            // browsers field removed in H.2.e — see comment near struct decl.
            window_meta: Mutex::new(HashMap::new()),
            host_state: Mutex::new(crate::reducer::HostState::default()),
            launcher_bridge_dedup: Mutex::new(crate::launcher_event_bridge::DedupCache::new()),
            #[cfg(not(target_os = "windows"))]
            windows: Mutex::new(HashMap::new()),
            #[cfg(not(target_os = "windows"))]
            browser_pane_overlays: Mutex::new(HashMap::new()),
            #[cfg(target_os = "macos")]
            browser_pane_physical_rects: Mutex::new(HashMap::new()),
            #[cfg(not(target_os = "windows"))]
            pane_overlay_rects: Mutex::new(HashMap::new()),
            #[cfg(not(target_os = "windows"))]
            pending_overlay_destroy: Mutex::new(HashMap::new()),
            #[cfg(target_os = "macos")]
            browser_pane_overlay_wnums: Mutex::new(HashMap::new()),
            #[cfg(target_os = "windows")]
            pane_clip_cache: Mutex::new(HashMap::new()),
            // window_pool / unpromoted_pool_labels / window_pool_respawn_in_flight
            // deleted (PR #5 H.4) — see HostState.pool.
            // is_quitting deleted (PR #5 H.5) — see HostState.quit_state.
            version_data_dir: Mutex::new(None),
            version_config_dir: Mutex::new(None),
            user_home_dir: Mutex::new(None),
            // active_drag deleted (PR #5 H.3) — see HostState.active_drag.
            browser_panes: crate::browser_panes::BrowserPaneManager::new(),
            browser_pane_zoom: Mutex::new(std::collections::HashMap::new()),
            browser_pane_context_menu_frame: Mutex::new(std::collections::HashMap::new()),
            browser_api: crate::browser_api::BrowserApiState::new(),
            debug_port: Mutex::new(0),
            cef_cache_dir: Mutex::new(None),
            #[cfg(target_os = "windows")]
            window_hwnds: Mutex::new(HashMap::new()),
            floating_redock_ghost: Mutex::new(HashMap::new()),
            window_transparent: std::sync::atomic::AtomicBool::new(false),
            ui_thread_gate: Mutex::new(UiThreadGate::default()),
            pending_reproject_closures: Mutex::new(PendingReprojectClosures::default()),
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

    // ── Phase H.2 — browser read helpers (reducer-only, post-flip) ──────
    //
    // After PR #4's flip step (H.1.c + H.2.c), `HostState.browsers` /
    // `HostState.browser_panes` are the sole source of truth. Legacy reads,
    // fallback paths, and drift logging removed — production smoke
    // verified zero drift across 50 reducer events with a balanced
    // BrowserRegistered/Unregistered count (18/18) and BrowserPaneCreate/
    // BrowserPaneClosed count (7/7) before this flip.
    //
    // Snapshot-and-drop: each helper takes the relevant lock briefly,
    // clones what's needed, drops the lock. Callers never hold the
    // lock across CEF FFI calls.

    /// Get a browser handle by label.
    pub fn get_browser(&self, label: &str) -> Option<Browser> {
        self.host_state
            .lock()
            .browsers
            .get(label)
            .map(|h| h.browser.clone())
    }

    /// Are there any registered browsers?
    pub fn browsers_is_empty(&self) -> bool {
        self.host_state.lock().browsers.is_empty()
    }

    /// Snapshot of all registered browsers as (label, Browser) pairs.
    /// Ordering is HashMap iteration order; callers must sort if order
    /// matters.
    pub fn list_browsers(&self) -> Vec<(String, Browser)> {
        self.host_state
            .lock()
            .browsers
            .iter()
            .map(|(k, h)| (k.clone(), h.browser.clone()))
            .collect()
    }

    /// Snapshot of top-level + floater browsers — excludes only `BrowserKind::Pane`
    /// child browsers whose main frame is loading untrusted remote content.
    /// Floaters render the trusted frontend (`FloatingPaneWorkspace`), so they
    /// ARE included (they need host JS events); only `Pane` children are excluded.
    /// Callers emitting JS-injected host events must use this (or
    /// `emit_event_to_window`) so a hostile page in one pane can't observe events
    /// meant for the host frontend.
    pub fn list_top_level_browsers(&self) -> Vec<(String, Browser)> {
        self.host_state
            .lock()
            .browsers
            .iter()
            .filter(|(_, h)| {
                matches!(
                    h.kind,
                    BrowserKind::TopLevel { .. } | BrowserKind::Floater { .. }
                )
            })
            .map(|(k, h)| (k.clone(), h.browser.clone()))
            .collect()
    }

    /// Count of live, user-visible top-level windows — the authoritative
    /// last-window quit gate (`client::on_before_close` +
    /// `wrr::win_event::maybe_quit_on_last_user_window`). Delegates to
    /// `reducer::count_live_user_windows`, which counts registered
    /// `BrowserKind::TopLevel { is_pool: false }` browsers ONLY. Floaters are a
    /// distinct `BrowserKind::Floater` and do NOT count (invariant FP-LIFE — they
    /// die with the last top-level window); warm window-pool windows are
    /// `is_pool: true` and don't count; promoted `window-pool-*` windows
    /// (`is_pool: false`, still `window-pool-` labelled) correctly count. No
    /// label-prefix strings — excluded purely by type. Single host_state lock.
    /// See SPEC_INSTANCE_LIFECYCLE_CONSOLIDATION_2026_06_21.md §5.1/§10.1 +
    /// SPEC_REDUCER_SSOT_CONSOLIDATION_2026_06_22.md (L4).
    pub fn count_live_user_windows(&self) -> usize {
        // Delegate to the reducer's pure counter under one lock (deref-coerces
        // the guard to &HostState) — single counting implementation.
        crate::reducer::count_live_user_windows(&self.host_state.lock())
    }

    /// Labels of registered TAB-pool-side top-level browsers — decided
    /// PURELY BY TYPE (`BrowserKind::TopLevel { is_pool: true }`), never by
    /// label prefix, the same doctrine as `count_live_user_windows` above.
    /// SPEC_POOL_ADOPTION_AND_WINDOW_ROW_CRUMB_2026_07_11 Residual 1: an
    /// ADOPTED pool window keeps its foreign `window-{uuid}` label while
    /// being genuinely pool-side, so any pool enumeration still filtering on
    /// the `window-pool-` prefix (the pre-adoption shortcut) silently skips
    /// it — in the quit-drain sweeps that meant Stage 2 could hang on an
    /// unswept adopted browser. Does NOT include the pane pool
    /// (`floating-pool-*`, `BrowserKind::Floater`) — pane-pool adoption is
    /// out of scope; callers that sweep both compose this with the
    /// `floating-pool-` prefix as before. Single host_state lock.
    pub fn pool_side_top_level_labels(&self) -> std::collections::HashSet<String> {
        self.host_state
            .lock()
            .browsers
            .iter()
            .filter(|(_, h)| matches!(h.kind, BrowserKind::TopLevel { is_pool: true }))
            .map(|(l, _)| l.clone())
            .collect()
    }

    /// Reverse lookup: find the label whose cached HWND matches `hwnd`.
    /// Used by the win-event HIDE handler to map an OS-level window hide to a
    /// logical `report_window_closed` call, covering close paths (Alt+F4,
    /// taskbar) that bypass the `close_window()` RPC.
    /// O(n) over `window_hwnds` — n is bounded by the number of open windows
    /// (typically 1–5). Returns `None` if `hwnd` is not in the cache.
    #[cfg(target_os = "windows")]
    pub fn label_for_hwnd(&self, hwnd: windows_sys::Win32::Foundation::HWND) -> Option<String> {
        let raw = hwnd as isize;
        let guard = self.window_hwnds.lock();
        guard
            .iter()
            .find(|(_k, v): &(&String, &isize)| **v == raw)
            .map(|(k, _)| k.clone())
    }

    /// Returns `true` iff the browser registered under `label` is a
    /// `TopLevel { is_pool: false }` — a promoted, user-visible instance
    /// window.
    ///
    /// Used by `wrr/win_event.rs` to gate LOCATIONCHANGE close reports on
    /// the authoritative `is_pool` flag rather than a label-prefix string.
    /// Reasons each case is excluded when NOT a live top-level:
    ///
    /// - `TopLevel { is_pool: true }` — warm pool window, parks at x=-20000
    ///   during initialization; not a close.
    /// - `Floater { .. }` — floating pane, parks at x=-20000 when unpromoted;
    ///   not a close.
    /// - `Pane { .. }` — child browser, never moves to the offscreen position.
    /// - Not in `browsers` at all — pool HWND registered before
    ///   `OnAfterCreated` fires; treat as pool (safe: unwrap_or(false)).
    ///
    /// A promoted `window-pool-*` window keeps its original label but
    /// acquires `is_pool: false` atomically at promotion — so it IS reported
    /// as closed when the user dismisses it. This is the case that a naïve
    /// `starts_with("window-pool-")` prefix filter would have suppressed,
    /// causing a stale window count (reagentx P1 on PR #1827).
    ///
    /// Cross-platform since Pillar 2 Phase 1: the body is a pure reducer read
    /// (no Win32); the historical `cfg(windows)` gate existed only because
    /// its sole caller was WRR. `orphan_reconcile`'s sanitize planner now
    /// calls it on every platform (reagent P0 on PR #2081).
    pub fn is_live_top_level_browser(&self, label: &str) -> bool {
        self.host_state
            .lock()
            .browsers
            .get(label)
            .map(|h| matches!(h.kind, BrowserKind::TopLevel { is_pool: false }))
            .unwrap_or(false)
    }

    /// First registered browser (for "any browser" callers like command
    /// palette routing). Returns the label + Browser pair, or None if
    /// the registry is empty.
    pub fn first_browser(&self) -> Option<(String, Browser)> {
        self.host_state
            .lock()
            .browsers
            .iter()
            .next()
            .map(|(k, h)| (k.clone(), h.browser.clone()))
    }

    // ── Phase H.1 — pane lifecycle read helpers (reducer-only, post-flip)

    /// Returns the pane's label iff entry is `Live`. Used by op gates
    /// (focus/resize/navigate) — `None` indicates the pane is missing or
    /// in `Closing`, in which case the caller must short-circuit rather
    /// than touch the (possibly destroyed) HWND.
    pub fn live_browser_pane_label(&self, block_id: &str) -> Option<String> {
        self.host_state
            .lock()
            .browser_panes
            .get(block_id)
            .filter(|e| e.lifecycle == BrowserPaneLifecycle::Live)
            .map(|e| e.label.clone())
    }

    /// The window a pane currently lives in (`main`, `floating-<uuid>`, …),
    /// or `None` if there is no entry. Used to make `browser_pane_close`
    /// window-aware: a close from a window that no longer owns the pane (e.g.
    /// the source window's view unmounting after a tear-off moved the pane to a
    /// floating window) must be ignored, else it destroys the pane that just
    /// moved. Returns the label regardless of Live/Closing.
    pub fn browser_pane_window_label(&self, block_id: &str) -> Option<String> {
        self.host_state
            .lock()
            .browser_panes
            .get(block_id)
            .map(|e| e.window_label.clone())
    }

    /// Snapshot of all `Live` pane labels. Used by `defocus_all` etc.
    pub fn live_browser_pane_labels(&self) -> Vec<String> {
        self.host_state
            .lock()
            .browser_panes
            .values()
            .filter(|e| e.lifecycle == BrowserPaneLifecycle::Live)
            .map(|e| e.label.clone())
            .collect()
    }

    /// PR #5 H.3 — read-side helper for `commands::drag::update_cross_drag`.
    /// Returns the active drag session iff its drag_id matches `drag_id`.
    /// Snapshot-and-drop: clones under lock, drops the lock.
    pub fn get_drag_session(&self, drag_id: &str) -> Option<DragSession> {
        self.host_state
            .lock()
            .active_drag
            .as_ref()
            .filter(|s| s.drag_id == drag_id)
            .cloned()
    }

    /// Snapshot the current active drag session regardless of drag_id.
    /// Used by `start_cross_drag` to detect and self-heal a *stale* session
    /// (a prior drag whose end/cancel never reached the host — e.g. the
    /// renderer threw mid-drop, or a window/pane was destroyed under it),
    /// which would otherwise reject every future tear-off forever.
    pub fn active_drag_snapshot(&self) -> Option<DragSession> {
        self.host_state.lock().active_drag.clone()
    }

    // ── PR #5 H.4 — pool read helpers ───────────────────────────────────
    //
    // Replace the legacy `state.unpromoted_pool_labels: Mutex<HashSet>` /
    // `state.window_pool: Mutex<VecDeque>` reads. All under one
    // `host_state` lock, snapshot-and-drop discipline.

    /// Atomic snapshot of (user_window_count, unpromoted_pool_count)
    /// for the launcher drift-detection report. Both reads taken
    /// under ONE `host_state` lock; a two-lock variant races
    /// against `promote_pool_window` and would let queued pool
    /// windows leak into the user-window count.
    ///
    /// Filter rules (mirror `list_windows` / `dispatch_to_renderers`):
    /// - exclude pool inventory (`pool.unpromoted` ∪ `pool.queue`)
    /// - exclude `browser-pane-*` child HWNDs
    ///
    /// `pool_count` is the size of the pool **inventory** (unpromoted
    /// ∪ queue) — NOT just unpromoted. The launcher's `state.pool`
    /// mirror is built from `ReportPoolWindowAdded` / `Removed` /
    /// `Promoted` events. On the host's unpromoted→queue transition
    /// (when `pool_ready` fires) NO event is emitted, so the
    /// launcher mirror retains the queued label. Reporting just
    /// `unpromoted.len()` would under-count and trigger spurious
    /// pool drift while the warm pool is idle and ready.
    pub fn host_counts_snapshot(&self) -> (u32, u32) {
        let st = self.host_state.lock();
        // `host_counts_snapshot` feeds the launcher's event-sourced pool mirror.
        // Pane pool emits NO launcher events (report_pool_window_added/removed/promoted),
        // so including pane_pool.* here would cause permanent DriftDetected{Pool}.
        // Only the tab pool (window-pool-*) participates in launcher mirror accounting.
        // (The last-window app-exit gate is `reducer::count_live_user_windows`, which
        // EXCLUDES pane pool — a separate count from this launcher-mirror one;
        // `user_visibility_snapshot` is the snapshot used for on_before_close logging.)
        let pool_inventory: std::collections::HashSet<&str> = st
            .pool
            .unpromoted
            .iter()
            .map(String::as_str)
            .chain(st.pool.queue.iter().map(String::as_str))
            .collect();
        let pool = pool_inventory.len() as u32;
        // Also exclude floating-pool-* (pane pool windows) from the windows count.
        // They are excluded from report_window_opened (client/mod.rs) on ALL
        // platforms, so the launcher mirror never counts them. This filter keeps
        // host_counts_snapshot in sync; without it host_windows > launcher_windows
        // → DriftDetected{Windows}.
        let windows = st
            .browsers
            .keys()
            .filter(|k| {
                !k.starts_with("browser-pane-")
                    && !k.starts_with("floating-pool-")
                    && !pool_inventory.contains(k.as_str())
            })
            .count() as u32;
        (windows, pool)
    }

    /// Snapshot of unpromoted pool labels — both tab pool (`window-pool-*`)
    /// and pane pool (`floating-pool-*`). Used by orphan reconciliation and
    /// the pool-count report after spawning a new pool slot.
    pub fn unpromoted_pool_labels_snapshot(&self) -> std::collections::HashSet<String> {
        let st = self.host_state.lock();
        st.pool
            .unpromoted
            .iter()
            .cloned()
            .chain(st.pane_pool.unpromoted.iter().cloned())
            .collect()
    }

    /// Single-label check against unpromoted pool set. Used by
    /// `client.rs::on_after_created` BrowserKind classification.
    pub fn is_unpromoted_pool_label(&self, label: &str) -> bool {
        self.host_state.lock().pool.unpromoted.contains(label)
    }

    /// Single-label check against the unpromoted PANE-pool set. Mirror of
    /// `is_unpromoted_pool_label` for `floating-pool-*` windows — used by
    /// `client.rs::on_after_created` to classify a warm pane-pool floater as
    /// `Floater { is_pool: true }` (a promoted one, no longer in the set, is
    /// `is_pool: false`).
    pub fn is_unpromoted_pane_pool_label(&self, label: &str) -> bool {
        self.host_state.lock().pane_pool.unpromoted.contains(label)
    }

    /// Number of pool windows currently in the renderer-ready queue
    /// (NOT including unpromoted). Used by `init_pool` to decide
    /// whether to bootstrap more pool windows.
    pub fn pool_queue_size(&self) -> usize {
        self.host_state.lock().pool.queue.len()
    }

    pub fn pane_pool_queue_size(&self) -> usize {
        self.host_state.lock().pane_pool.queue.len()
    }

    /// Atomic snapshot for user-visibility filtering: pool inventory
    /// (`pool.unpromoted` ∪ `pool.queue`) AND the browser registry,
    /// taken under ONE `host_state` lock acquisition.
    ///
    /// Two-lock variants (one snapshot for the pool, one for
    /// `list_browsers()`) race against `promote_pool_window`:
    /// between the reads, a label can move from pool.queue to
    /// promoted, leaving the stale inventory excluding a now-real
    /// user window. Atomic snapshot eliminates the gap.
    ///
    /// Returns:
    /// - `pool_inventory`: labels in `pool.unpromoted` ∪ `pool.queue` ∪
    ///   `pane_pool.unpromoted` ∪ `pane_pool.queue` — all host-internal
    ///   pool windows (both tab pool `window-pool-*` and pane pool
    ///   `floating-pool-*`) that have no user UI yet; exclude from
    ///   user-visible filters and from launcher-event dispatch.
    /// - `browsers`: every label → Browser pair currently registered
    ///   (cheap clone — `Browser` is a CEF refcounted wrapper).
    pub fn user_visibility_snapshot(&self) -> (
        std::collections::HashSet<String>,
        Vec<(String, Browser)>,
    ) {
        let st = self.host_state.lock();
        let pool_inventory: std::collections::HashSet<String> = st
            .pool
            .unpromoted
            .iter()
            .cloned()
            .chain(st.pool.queue.iter().cloned())
            .chain(st.pane_pool.unpromoted.iter().cloned())
            .chain(st.pane_pool.queue.iter().cloned())
            .collect();
        let browsers: Vec<(String, Browser)> = st
            .browsers
            .iter()
            .map(|(k, h)| (k.clone(), h.browser.clone()))
            .collect();
        (pool_inventory, browsers)
    }

    /// PR #5 H.5 — read-side helper for the legacy `is_quitting` check.
    /// Returns true iff the host has begun draining (BeginDrain
    /// dispatched) OR has fully quit. Replaces the AtomicBool.
    pub fn is_quitting(&self) -> bool {
        !matches!(
            self.host_state.lock().quit_state,
            crate::state::QuitState::Running
        )
    }

    /// PR #6 H.7 — cross-state invariant for the 2026-05-02 freeze.
    ///
    /// Returns true iff ANY pane is in `Closing`. Top-level window
    /// creation paths (open_new_window, open_window_at_position,
    /// spawn_pool_window) MUST refuse while this is true: empirically
    /// (`SPEC_WINDOW_FLEET_REDUCER_2026-05-02.md`), creating a CEF
    /// top-level mid-pane-close hits a Chromium v146 deadlock that
    /// wedges the message loop with HiddenSinceOpen + IPC backpressure
    /// (`pending=N` rising) and never recovers.
    ///
    /// The check is small enough to inline at each call site with no
    /// async surface. If it turns out the gate needs to widen
    /// ("any pane present" rather than "any pane Closing"), that's a
    /// one-line edit. Spec §5 escape hatch.
    pub fn any_browser_pane_closing(&self) -> bool {
        self.host_state
            .lock()
            .browser_panes
            .values()
            .any(|e| matches!(e.lifecycle, BrowserPaneLifecycle::Closing { .. }))
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
        HostEvent::BrowserPaneCreateRequested { block_id, label, version } => tracing::info!(
            target: "host-reducer",
            event = "BrowserPaneCreateRequested",
            block_id = %block_id, label = %label, version,
        ),
        HostEvent::BrowserPaneLive { block_id, label, version } => tracing::info!(
            target: "host-reducer",
            event = "BrowserPaneLive",
            block_id = %block_id, label = %label, version,
        ),
        HostEvent::BrowserPaneClosing { block_id, version } => tracing::info!(
            target: "host-reducer",
            event = "BrowserPaneClosing",
            block_id = %block_id, version,
        ),
        HostEvent::BrowserPaneClosed { block_id, version } => tracing::info!(
            target: "host-reducer",
            event = "BrowserPaneClosed",
            block_id = %block_id, version,
        ),
        HostEvent::BrowserPaneCreationFailed { block_id, reason, version } => tracing::warn!(
            target: "host-reducer",
            event = "BrowserPaneCreationFailed",
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
        // ── Opacity ──────────────────────────────────────────────────────
        HostEvent::WindowOpacityApplied { label, opacity, version } => tracing::debug!(
            target: "host-reducer",
            event = "WindowOpacityApplied",
            label = %label, opacity, version,
        ),
        HostEvent::WindowOpacityCleared { label, version } => tracing::debug!(
            target: "host-reducer",
            event = "WindowOpacityCleared",
            label = %label, version,
        ),
        // ── Pane window-placement (pane-state reducer, Phase 0) ──────────
        HostEvent::PaneWindowStateChanged { label, placement, restore_rect, version } => tracing::info!(
            target: "host-reducer",
            event = "PaneWindowStateChanged",
            label = %label,
            placement = ?placement,
            restore_rect = ?restore_rect,
            version,
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
