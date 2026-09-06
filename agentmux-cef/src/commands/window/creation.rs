// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Window creation handlers + frontend-URL resolution for the CEF host —
// open_new_window / open_subwindow, the dev-Vite-port + host-runtime-dir
// resolution that feeds the loaded URL, and the FrontendUrlError surface.
//
// Final carve of the commands/window.rs modularization (Plan 1); with it,
// `mod.rs` becomes a pure re-export shim. Pure move — no behavior change.
//
// `open_new_window` / `open_subwindow` are `pub` (dispatched by ipc.rs).
// `resolve_frontend_base_url` / `assets_missing_data_url` are `pub(crate)`
// (client / drag / window_pool / floating_pane resolve them as
// `commands::window::<name>`, so `mod.rs` re-exports them explicitly).

use std::path::PathBuf;
use std::sync::Arc;

use crate::client::helpers::{html_escape, js_string_literal};
use crate::state::AppState;

// The secondary-window offset helper resolves the current top-level HWND
// (Windows only) from the sibling `lifecycle` module.
#[cfg(target_os = "windows")]
use super::lifecycle::find_own_top_level_window;

/// Resolve the dev Vite port. Honors `AGENTMUX_VITE_PORT` (set by
/// `Taskfile.yml`'s `dev:serve` task when the per-clone deterministic
/// port differs from 5173); falls back to 5173 otherwise. Without this,
/// every child window (pool warmups, tab tear-off, floating pane) loads
/// `localhost:5173` and hits `ERR_CONNECTION_REFUSED` on any other port —
/// only the main window survives because the launcher passes `--url=…`
/// on the CLI. See `docs/analysis/ANALYSIS_DEV_VITE_PORT_HARDCODE_2026-05-26.md`.
fn dev_vite_port() -> u16 {
    std::env::var("AGENTMUX_VITE_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5173)
}

/// Why `resolve_frontend_base_url` could not return a usable URL.
///
/// Returned by `resolve_frontend_base_url` instead of the old silent
/// fallback to `http://localhost:<vite_port>`, which masked broken
/// installs as renderer-crash loops in production (see
/// `docs/retro/retro-portable-rm-running-install-2026-05-28.md`).
#[derive(Debug)]
pub(crate) enum FrontendUrlError {
    /// `std::env::current_exe()` failed. Extraordinarily rare; on
    /// Windows it would mean we couldn't even resolve our own module
    /// path, which usually only happens for truly corrupted installs.
    ExeUnresolvable(std::io::Error),
    /// We resolved a production install dir but `frontend/index.html`
    /// is not next to the exe. Either the bundle was never built, was
    /// deleted (e.g. by an external `rm -rf` of a running portable —
    /// see retro), or the install layout is otherwise wrong.
    AssetsMissing { checked_path: PathBuf },
}

impl std::fmt::Display for FrontendUrlError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::ExeUnresolvable(e) => write!(f, "could not resolve current_exe: {}", e),
            Self::AssetsMissing { checked_path } => {
                write!(f, "frontend assets missing at {}", checked_path.display())
            }
        }
    }
}

impl std::error::Error for FrontendUrlError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::ExeUnresolvable(e) => Some(e),
            Self::AssetsMissing { .. } => None,
        }
    }
}

/// Resolve the base URL for the frontend.
///
/// - **Production:** IPC server serves static files from `frontend/`
///   next to the exe → returns `http://127.0.0.1:<ipc_port>`.
/// - **Dev:** Vite dev server at
///   `http://localhost:<AGENTMUX_VITE_PORT or 5173>`.
/// - **Production with missing assets:** returns
///   `Err(FrontendUrlError::AssetsMissing)`. Historically this fell
///   through to `http://localhost:<vite_port>`, which in production
///   points at nothing and produced silent renderer crash loops
///   (issue #1117 / retro 2026-05-28). Callers should now translate
///   the error into a built-in static error page via
///   `assets_missing_data_url` rather than navigating to a network
///   URL.
pub(crate) fn resolve_frontend_base_url(ipc_port: u16) -> Result<String, FrontendUrlError> {
    // Detect dev mode by the host exe PATH (`is_dev_self`), NOT
    // `AGENTMUX_RUNTIME_MODE`. A running `task dev` AgentMux leaks that env
    // into descendants, so an env-first check would route a *packaged*
    // build's secondary windows (new window, window pool, tear-off, floating
    // pane) to the inherited dev Vite server — they return here before the
    // bundled-asset check below, so there's no `has_frontend` safety net.
    // Build identity is a property of the binary on disk: `task dev` (with or
    // without the launcher) runs the host from `dist/cef-dev/` → Dev; a
    // packaged host → not Dev, regardless of any inherited env.
    if agentmux_common::is_dev_self() {
        return Ok(format!("http://localhost:{}", dev_vite_port()));
    }
    let runtime_dir = resolve_host_runtime_dir()?;
    let index = runtime_dir.join("frontend").join("index.html");
    if index.exists() {
        Ok(format!("http://127.0.0.1:{}", ipc_port))
    } else {
        Err(FrontendUrlError::AssetsMissing { checked_path: index })
    }
}

/// Resolve the directory that contains this host build's runtime
/// assets (`frontend/`, the CEF DLLs, locales, etc.).
///
/// Preference order:
///
/// 1. **`AGENTMUX_HOME` env var** — set by `agentmux-launcher` from
///    *its* resolved `real_exe` path at host-spawn time. This is the
///    authoritative anchor: the launcher always finds the runtime
///    dir by walking from its own current_exe, so the path it
///    exports always points at the on-disk files that actually
///    exist, even if those files were moved or unlinked after host
///    process startup. See
///    `docs/retro/retro-portable-rm-running-install-2026-05-28.md`
///    for why current_exe() is not safe to use directly on Windows.
/// 2. **Fallback to `std::env::current_exe().parent()`** — used in
///    dev mode (`task dev` without the launcher) and any other
///    invocation where AGENTMUX_HOME isn't set. Carries the old
///    Windows-rename hazard, but those invocations don't survive
///    a directory move in the same way (dev mode is short-lived).
fn resolve_host_runtime_dir() -> Result<PathBuf, FrontendUrlError> {
    if let Some(val) = std::env::var_os("AGENTMUX_HOME") {
        let path = PathBuf::from(val);
        if !path.as_os_str().is_empty() {
            return Ok(path);
        }
    }
    let exe = std::env::current_exe().map_err(FrontendUrlError::ExeUnresolvable)?;
    exe.parent()
        .map(|p| p.to_path_buf())
        .ok_or_else(|| FrontendUrlError::AssetsMissing { checked_path: exe })
}

/// Build a self-contained `data:` URL that renders a static
/// "AgentMux install is broken" error page. Used by every caller of
/// `resolve_frontend_base_url` as the navigation target when the
/// resolver returns `Err` — instead of falling back to a network URL
/// that almost certainly points at nothing in production.
///
/// The page contains no auto-reload and no link back into the broken
/// install, so navigating to it can never trigger the crash loop the
/// old silent fallback produced. Only buttons are "Quit" and "Copy
/// path" (the missing asset path is shown so the user knows what to
/// reinstall).
pub(crate) fn assets_missing_data_url(err: &FrontendUrlError) -> String {
    let html = assets_missing_html(err);
    let b64 = cef::base64_encode(Some(html.as_bytes()));
    let b64_str = cef::CefString::from(&b64).to_string();
    format!("data:text/html;base64,{}", b64_str)
}

/// HTML for the broken-install error page. Split out from
/// `assets_missing_data_url` so the button wiring is unit-testable without
/// CEF / base64 round-tripping.
fn assets_missing_html(err: &FrontendUrlError) -> String {
    let (reason, detail) = match err {
        FrontendUrlError::ExeUnresolvable(e) => (
            "Could not determine the install directory".to_string(),
            e.to_string(),
        ),
        FrontendUrlError::AssetsMissing { checked_path } => (
            "AgentMux frontend assets are missing".to_string(),
            format!("Expected at: {}", checked_path.display()),
        ),
    };
    format!(
        r#"<!DOCTYPE html>
<html lang="en"><head><meta charset="utf-8"><title>AgentMux — Install broken</title>
<style>
:root {{ color-scheme: dark; }}
body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
       background: #1e1e2e; color: #cdd6f4;
       display: flex; justify-content: center; align-items: center;
       min-height: 100vh; margin: 0; padding: 24px; box-sizing: border-box; }}
.box {{ text-align: center; max-width: 560px; padding: 36px;
       background: #181825; border: 1px solid #313244; border-radius: 10px;
       box-shadow: 0 8px 32px rgba(0,0,0,0.4); }}
.icon {{ font-size: 36px; line-height: 1; margin-bottom: 12px; }}
h1 {{ color: #f38ba8; font-size: 22px; margin: 0 0 6px 0; }}
p {{ color: #bac2de; line-height: 1.55; margin: 0 0 12px 0; font-size: 14px; }}
.detail code {{ display: inline-block; background: #313244; color: #f9e2af;
               padding: 6px 10px; border-radius: 4px; font-size: 12px;
               font-family: ui-monospace, 'Cascadia Code', Menlo, Consolas, monospace;
               word-break: break-all; text-align: left; max-width: 100%; }}
.actions {{ display: flex; gap: 10px; justify-content: center; margin-top: 24px; flex-wrap: wrap; }}
button {{ padding: 10px 22px; border: 1px solid #45475a; border-radius: 6px;
         background: #313244; color: #cdd6f4; cursor: pointer;
         font-size: 13px; font-family: inherit; }}
button:hover {{ background: #45475a; border-color: #585b70; }}
</style></head>
<body><div class="box" role="alertdialog">
<div class="icon">⚠</div>
<h1>AgentMux install is broken</h1>
<p>{reason_safe}.</p>
<p class="detail"><code>{detail_safe}</code></p>
<p>Reinstall AgentMux from a fresh portable ZIP to recover. Your saved
sessions and agent state are in <code>~/.agentmux/</code> and are unaffected.</p>
<div class="actions">
<button id="amx-copy">Copy path</button>
<button id="amx-quit">Quit</button>
</div></div>
<script>
(function(){{
  var c = document.getElementById('amx-copy');
  if (c) c.addEventListener('click', function () {{
    if (navigator.clipboard) navigator.clipboard.writeText({detail_js});
  }});
  var q = document.getElementById('amx-quit');
  if (q) q.addEventListener('click', function () {{ window.close(); }});
}})();
</script>
</body></html>"#,
        reason_safe = html_escape(&reason),
        detail_safe = html_escape(&detail),
        detail_js = js_string_literal(&detail),
    )
}

/// Open a new full AgentMux instance (status-bar version click, Ctrl+Shift+N,
/// second `agentmux.exe` launch). Independent top-level window, own taskbar
/// entry, independent lifecycle. See
/// `docs/specs/SPEC_MULTIWINDOW_TASKBAR_GROUPING.md`.
pub fn open_new_window(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let initial_view = args
        .get("initial_view")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    // Opaque JSON string carrying the full blockdef.meta; threaded through pool
    // event payloads and cold-path URL so the new window can call pane.open
    // with the complete meta rather than re-deriving it from the view name.
    let initial_meta = args
        .get("initial_meta")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string());
    // H.7 invariant — also enforced inside open_window_with_kind (cold path).
    if state.any_browser_pane_closing() {
        tracing::warn!(
            target: "wfr:gate",
            "[wfr:gate] open_new_window refused — pane is mid-close (H.7 invariant)"
        );
        return Err("a pane is currently closing; retry shortly".to_string());
    }

    // Pool-first path — instant show with no renderer-spawn latency (~3s saved).
    // On macOS/Linux, emits pool:new-window (no workspaceId → fresh workspace).
    // On Windows, delegates to promote_pool_window with workspace_id="" and physical-pixel
    // anchor (bypassing DIP→physical DPI double-conversion; see promote_pool_window_for_new_window).
    // Probe the monitor from the cascade anchor when we have one, else the
    // primary — `get_secondary_window_size` only uses the point to pick a
    // monitor.
    let (probe_x, probe_y) = cascade_anchor().unwrap_or((0, 0));
    let (win_w, win_h) = get_secondary_window_size(probe_x, probe_y);
    // Centre the dimensions the window will ACTUALLY have. On Windows the pool
    // promote below discards `win_w`/`win_h` and uses POOL_WIDTH/POOL_HEIGHT
    // (see `promote_pool_window_for_new_window`), so centring `win_w`/`win_h`
    // here would offset the window by half the difference.
    // PHYSICAL pixels on both sides: POOL_WIDTH/POOL_HEIGHT reach SetWindowPos
    // unconverted (promote_pool_window only runs to_physical when width is
    // Some), so they must be centred against the physical work area.
    #[cfg(target_os = "windows")]
    let (pos_x, pos_y) = new_window_origin(
        crate::commands::window_pool::POOL_WIDTH,
        crate::commands::window_pool::POOL_HEIGHT,
        crate::app::get_monitor_work_area_physical(0, 0),
    );
    #[cfg(not(target_os = "windows"))]
    let (pos_x, pos_y) = new_window_origin(win_w, win_h, None);
    if let Some(label) = crate::commands::window_pool::promote_pool_window_for_new_window(
        state, pos_x, pos_y, win_w, win_h, initial_view.clone(), initial_meta.clone(),
    ) {
        tracing::info!(
            target: "pool:new-window",
            label = %label,
            "[pool:new-window] served from pool — skipping cold-path window creation"
        );
        return Ok(serde_json::json!(label));
    }

    // Cold path — spin up a fresh CEF window (~2.5–3.5 s).
    open_window_with_kind(
        state,
        crate::state::WindowKind::FullInstance,
        None,
        initial_view.as_deref(),
        initial_meta.as_deref(),
        None,
        false,
    )
}

/// Open a sub-window tied to `parent_instance_id`. **Not exposed to users** —
/// reserved for agent / backend callers that need a transient auxiliary
/// top-level window (tool-spawned panels, diff views, etc.). Sub-windows are
/// hidden from the taskbar via `ITaskbarList::DeleteTab` and close when their
/// parent full instance closes.
pub fn open_subwindow(
    state: &Arc<AppState>,
    parent_instance_id: String,
    initial_view: Option<&str>,
    initial_meta: Option<&str>,
) -> Result<serde_json::Value, String> {
    // Reject if the parent isn't a known live FullInstance — prevents
    // orphan sub-windows and enforces the lifecycle rule in the spec.
    //
    // Phase B.5 (window_meta step d, refined twice):
    // * Round-1 fix used `state.window_meta()` (shadow-first), which
    //   covered the task-dev-mode regression but allowed a NEW orphan
    //   bug: shadow lags on close, so during the gap between host's
    //   sync `on_before_close` removal and the launcher's async
    //   `WindowClosed` event arrival, this check could still see a
    //   already-closing parent. (codex P2 PR #592 round-2.)
    // * Refined: read host_meta DIRECTLY for this liveness check.
    //   Host_meta is synchronously written in on_after_created and
    //   removed in on_before_close (per the round-2 step-d
    //   refinement keeping host_meta as a sync cache), so it
    //   correctly reflects "is the parent currently open" without
    //   shadow's async lag. Works in `task dev` mode too (host_meta
    //   populated by on_after_created regardless of launcher
    //   presence).
    let parent_ok = state
        .window_meta
        .lock()
        .get(&parent_instance_id)
        .map(|m| m.kind == crate::state::WindowKind::FullInstance)
        .unwrap_or(false);
    if !parent_ok {
        return Err(format!(
            "open_subwindow: unknown or non-full-instance parent label={parent_instance_id}"
        ));
    }
    open_window_with_kind(
        state,
        crate::state::WindowKind::Subwindow,
        Some(parent_instance_id),
        initial_view,
        initial_meta,
        None,
        false,
    )
}

/// SPEC_PILLAR1_STEP4 Phase 2 — `pub(crate)` (was private) so the reproject
/// driver (`launcher_ipc::reproject_from_snapshot`) can call it directly,
/// bypassing the IPC-handler wrappers (`open_new_window`/`open_subwindow`)
/// that assume a live, frontend-originated request.
///
/// `explicit_rect`: when `Some`, skips `new_window_origin`/
/// `get_secondary_window_size`'s cascade-or-centre/70%-of-monitor placement
/// and uses the given rect verbatim — the reproject driver's fast path
/// passes the launcher snapshot's `last_rect` here so a recreated window
/// lands roughly where it was, instead of at a new-window default position.
/// `None` preserves today's behavior exactly (both existing callers pass
/// `None`).
pub(crate) fn open_window_with_kind(
    state: &Arc<AppState>,
    kind: crate::state::WindowKind,
    parent_instance_id: Option<String>,
    initial_view: Option<&str>,
    initial_meta: Option<&str>,
    explicit_rect: Option<agentmux_common::ipc::Rect>,
    is_reproject: bool,
) -> Result<serde_json::Value, String> {
    // PR #6 H.7 — refuse top-level creation while any pane is mid-close.
    // See `SPEC_WINDOW_FLEET_REDUCER_2026-05-02.md` and the smoke retro
    // at `docs/retro/smoke-test-0.33.586-and-pr5-plan-2026-05-02.md`:
    // creating a top-level CEF window while a pane is in `Closing` hits
    // a Chromium v146 deadlock (HiddenSinceOpen + IPC backpressure)
    // that wedges the message loop. Frontend should retry on next tick.
    if state.any_browser_pane_closing() {
        tracing::warn!(
            target: "wfr:gate",
            "[wfr:gate] open_window refused — pane is mid-close (H.7 invariant)"
        );
        return Err("a pane is currently closing; retry shortly".to_string());
    }

    // Refuse once the instance has decided to quit. Without this, a window
    // creation racing an explicit `quit_app` would register AFTER the drain
    // began — `handle_register_browser` has no draining guard — leaving a
    // live, visible window in a draining host that the quit's own snapshot
    // never knew to close (Codex P2 on PR #2996). The existing quit watchdog
    // would eventually force the exit, but only by killing a window the user
    // is looking at, several seconds later.
    //
    // Cheap and unconditional: this is not specific to background-service
    // mode. `QuitState` is monotonic, so once it leaves `Running` no new
    // top-level window is ever wanted, by any caller — user, reproject, or
    // pool fallback.
    if !matches!(
        state.host_state.lock().quit_state,
        crate::state::QuitState::Running
    ) {
        tracing::warn!(
            target: "wfr:gate",
            "[wfr:gate] open_window refused — instance is draining/quitting"
        );
        return Err("the app is shutting down".to_string());
    }

    let window_id = uuid::Uuid::new_v4();
    let label = format!("window-{}", window_id.simple());

    let ipc_port = *state.ipc_port.lock();
    let ipc_token = &state.ipc_token;
    let url = match resolve_frontend_base_url(ipc_port) {
        Ok(base_url) => {
            let separator = if base_url.contains('?') { "&" } else { "?" };
            let view_param = initial_view
                .filter(|v| !v.is_empty())
                .map(|v| format!("&initialView={}", v))
                .unwrap_or_default();
            let meta_param = initial_meta
                .filter(|m| !m.is_empty())
                .map(|m| format!("&initialMeta={}", percent_encode(m)))
                .unwrap_or_default();
            // SPEC_PILLAR1_STEP4 Phase 4 — drives index.html's
            // #startup-loading-headline ("Restoring session...") for
            // reprojected windows only; an interactive "New Window"/"New
            // Subwindow" never sets this. Cheap, frontend-only per the
            // spec's §2.4 in-window-phase design.
            let restoring_param = if is_reproject { "&restoring=1" } else { "" };
            format!(
                "{}{}ipc_port={}&ipc_token={}&windowLabel={}{}{}{}",
                base_url, separator, ipc_port, ipc_token, label, view_param, meta_param, restoring_param
            )
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                label = %label,
                "[window] frontend assets unavailable — opening static error page (the new window will display an 'install broken' notice instead of crash-looping)",
            );
            assets_missing_data_url(&e)
        }
    };

    tracing::info!(label = %label, kind = ?kind, parent = ?parent_instance_id, "[window] open window");

    // Phase B.5 (window_meta step d) — push the pre-create handoff
    // (label + kind + parent). Replaces the previous parallel
    // `window_meta.insert` + `pending_window_labels.push` pair.
    let (pos_x, pos_y, win_w, win_h) = match explicit_rect {
        Some(r) => (r.left, r.top, r.right - r.left, r.bottom - r.top),
        None => {
            // Size first (it only needs a point to pick a monitor), then an
            // origin that centres THAT size when nothing is open to cascade
            // from. The cold path really does get `win_w`/`win_h`, unlike the
            // pool path above.
            let (probe_x, probe_y) = cascade_anchor().unwrap_or((0, 0));
            let (win_w, win_h) = get_secondary_window_size(probe_x, probe_y);
            // DIP on both sides: get_secondary_window_size divides by the
            // monitor scale (CEF Views set_bounds expects DIP), so centre
            // against the DIP work area, NOT the physical one.
            #[cfg(target_os = "windows")]
            let work = crate::app::get_monitor_work_area(probe_x, probe_y);
            #[cfg(not(target_os = "windows"))]
            let work = None;
            let (pos_x, pos_y) = new_window_origin(win_w, win_h, work);
            (pos_x, pos_y, win_w, win_h)
        }
    };

    // Phase F.1 — routed through the host reducer.
    state.host_dispatch(
        crate::reducer::HostCommand::EnqueuePendingWindowCreation {
            entry: crate::state::PendingWindowCreation {
                label: label.clone(),
                kind,
                parent_instance_id,
            },
        },
    );

    // Post to CEF UI thread — window_create_top_level must run there.
    // true = frameless: secondary app windows use the same custom title bar as main.
    crate::ui_tasks::post_create_window(
        state, &url, &label, pos_x, pos_y, win_w, win_h,
        true,
    );

    // Phase B.7.3.3 — typed launcher events drive InstancePanel
    // atoms via the CEF JS bridge; no sync emit here.

    Ok(serde_json::json!(label))
}

/// SPEC_PILLAR1_STEP4 Phase 2 — the fast-path reproject driver. Called once
/// per process life from `launcher_ipc::apply_event_to_shadow`'s
/// `Event::Snapshot` arm, with the launcher's live window list (survives a
/// host-only crash — see the spec for why this is preferred over srv's
/// durable-but-poorer-fidelity `Client.windowids`).
///
/// Idempotent by construction, not by an explicit "have we already
/// reprojected" flag: `"main"` is always filtered out (it's created by the
/// separate, unconditional cold-start path regardless of what the snapshot
/// says — recreating it here would double-create). On a genuine first-ever
/// launch the snapshot has zero entries beyond (at most) a stale `"main"`,
/// so this call does nothing.
///
/// Ordering: `FullInstance` windows are recreated before `Subwindow`s, so a
/// subwindow's parent always exists (under its NEW label) by the time the
/// subwindow needs it. This is a stable sort by kind, not a dependency
/// graph — sufficient because every real subwindow's parent is a
/// `FullInstance` (per `WindowKind`'s own doc comment; there is no
/// subwindow-of-subwindow nesting in this codebase).
///
/// Old-label→new-label remapping is mandatory, not optional: every
/// recreated window gets a fresh `window-<uuid>` label
/// (`open_window_with_kind`), so a subwindow's `parent_label` from the
/// snapshot (an OLD label) must be translated to the parent's NEW label
/// before being passed to `open_window_with_kind`. `"main"` is seeded into
/// the remap table as `"main"` → `"main"` since its label is stable across
/// restarts (never regenerated).
///
/// Returns `(old_label, new_label)` for every `WindowSnapshot` whose
/// `open_window_with_kind` call returned `Ok` (not the ones skipped/failed).
///
/// reagent P1 (PR #2032, 2026-07-08): `Ok` here means only that a
/// `CreateWindowTask` was successfully POSTED to the UI thread
/// (`open_window_with_kind` → `post_task`, fire-and-forget) — not that the
/// window actually exists yet. This session's own Phase 2 investigation
/// found `post_task` can silently drop a posted task (the UI-thread-readiness
/// race). The caller MUST NOT treat this return value as "safe to delete the
/// old data" — `reproject_from_srv` instead stashes `new_label → old_id` and
/// waits for `new_label`'s own `register_backend_window` call (proof the new
/// window's frontend actually loaded and round-tripped IPC) before closing
/// the old one. The fast path (launcher snapshot) used to ignore this return
/// value entirely, on the theory that a `WindowSnapshot.label` there is the
/// launcher's own in-memory label, not a real srv window_id, so there was
/// nothing to close either way.
///
/// reagent P1 (PR #2032, 2026-07-08, second finding): that theory was
/// wrong — the launcher's `Event::Snapshot` carries a sibling
/// `backend_window_ids: Vec<(String, String)>` field (that same in-memory
/// label → the real srv window_id it registered), which both fast-path
/// call sites already receive but were discarding. `Client.windowids` grew
/// unboundedly on every ordinary (launcher-survives) crash — the exact
/// scenario this PR's own E2E test exercises — because nothing ever staged
/// a deferred close for it. See `reproject_from_snapshot_and_stage_closures`,
/// which both fast-path call sites (`launcher_ipc.rs`, `client/lifecycle.rs`)
/// now use instead of calling this function directly.
pub(crate) fn reproject_from_snapshot(
    state: &Arc<AppState>,
    windows: &[agentmux_common::ipc::WindowSnapshot],
) -> Vec<(String, String)> {
    use std::collections::HashMap;

    let mut to_create: Vec<&agentmux_common::ipc::WindowSnapshot> =
        windows.iter().filter(|w| w.label != "main").collect();
    if to_create.is_empty() {
        tracing::debug!(target: "reproject", "[reproject] nothing to recreate beyond main");
        return Vec::new();
    }
    // FullInstance before Subwindow — stable sort preserves the snapshot's
    // own relative ordering within each kind. `WindowSnapshot.kind` is the
    // WIRE type (`agentmux_common::ipc::WindowKind`), distinct from the
    // host's own `crate::state::WindowKind` — mapped one-to-one, same
    // conversion `apply_shadow_projection`'s `Event::WindowOpened` arm
    // already does.
    let to_host_kind = |k: agentmux_common::ipc::WindowKind| match k {
        agentmux_common::ipc::WindowKind::FullInstance => crate::state::WindowKind::FullInstance,
        agentmux_common::ipc::WindowKind::Subwindow => crate::state::WindowKind::Subwindow,
    };
    to_create.sort_by_key(|w| match w.kind {
        agentmux_common::ipc::WindowKind::FullInstance => 0,
        agentmux_common::ipc::WindowKind::Subwindow => 1,
    });

    let mut label_remap: HashMap<String, String> = HashMap::new();
    label_remap.insert("main".to_string(), "main".to_string());

    let mut created = 0usize;
    let mut skipped = 0usize;
    let mut recreated_pairs: Vec<(String, String)> = Vec::new();
    for w in to_create {
        let new_parent = match &w.parent_label {
            Some(old_parent) => match label_remap.get(old_parent) {
                Some(new_label) => Some(new_label.clone()),
                None => {
                    // Parent wasn't (re)created — e.g. it was itself
                    // skipped, or the snapshot listed a Subwindow before
                    // its FullInstance parent existed at all (a crash
                    // mid-creation). Skip rather than create an orphan
                    // with a dangling parent link.
                    tracing::warn!(
                        target: "reproject",
                        label = %w.label,
                        parent_label = %old_parent,
                        "[reproject] parent not found in remap table — skipping this window"
                    );
                    skipped += 1;
                    continue;
                }
            },
            None => None,
        };
        match open_window_with_kind(state, to_host_kind(w.kind), new_parent, None, None, w.last_rect, true) {
            Ok(new_label_val) => {
                let new_label = new_label_val.as_str().unwrap_or_default().to_string();
                tracing::info!(
                    target: "reproject",
                    old_label = %w.label,
                    new_label = %new_label,
                    kind = ?w.kind,
                    parent_label = ?w.parent_label,
                    had_rect = w.last_rect.is_some(),
                    "[reproject] recreated window"
                );
                label_remap.insert(w.label.clone(), new_label.clone());
                recreated_pairs.push((w.label.clone(), new_label));
                created += 1;
            }
            Err(e) => {
                tracing::warn!(
                    target: "reproject",
                    label = %w.label,
                    error = %e,
                    "[reproject] failed to recreate window"
                );
                skipped += 1;
            }
        }
    }
    tracing::info!(
        target: "reproject",
        created,
        skipped,
        "[reproject] fast-path reproject complete"
    );
    recreated_pairs
}

/// SPEC_PILLAR1_STEP4 Phase 3 addendum (reagent P1, PR #2032, 2026-07-08,
/// second finding) — fast-path counterpart to `reproject_from_srv`'s
/// deferred-close staging, using the same `PendingReprojectClosures`
/// mechanism so a recreated window's old srv id is only closed once the
/// new one is confirmed live via its own `register_backend_window` call
/// (never on the unconfirmed `Ok` from `open_window_with_kind`).
///
/// `backend_window_ids` is the launcher's `Event::Snapshot.backend_window_ids`
/// — the SAME pre-crash label used in `windows` (`WindowSnapshot.label`),
/// mapped to the real srv window_id the launcher last saw it register. A
/// window the launcher never learned a backend_window_id for (e.g. it
/// crashed before its own `register_backend_window` round trip) has
/// nothing real to close and is silently skipped — same as an unconfirmed
/// `reproject_from_srv` entry, the old (nonexistent-to-us) state just
/// lingers rather than being lost.
///
/// Both fast-path call sites (`launcher_ipc.rs`'s `RunFastPath` arm,
/// `client/lifecycle.rs`'s `ReplayFastPath` arm) use this instead of
/// calling `reproject_from_snapshot` directly, so neither can regress back
/// to silently discarding the recreated pairs.
pub(crate) fn reproject_from_snapshot_and_stage_closures(
    state: &Arc<AppState>,
    windows: &[agentmux_common::ipc::WindowSnapshot],
    backend_window_ids: &[(String, String)],
) {
    let recreated_pairs = reproject_from_snapshot(state, windows);
    if recreated_pairs.is_empty() {
        return;
    }
    let old_ids: std::collections::HashMap<&str, &str> = backend_window_ids
        .iter()
        .map(|(label, id)| (label.as_str(), id.as_str()))
        .collect();
    let mut pending = state.pending_reproject_closures.lock();
    for (old_label, new_label) in recreated_pairs {
        if let Some(old_id) = old_ids.get(old_label.as_str()) {
            pending.stage(new_label, old_id.to_string());
        } else {
            tracing::debug!(
                target: "reproject",
                old_label = %old_label,
                "[reproject] fast path: no known backend_window_id for this old label — nothing to close"
            );
        }
    }
}

/// SPEC_PILLAR1_STEP4 Phase 3 — the slow-path reproject driver. Called once
/// per process life from `commands/window/meta.rs::register_backend_window`'s
/// `"main"` branch, only when no fast-path (launcher-snapshot) data was
/// available — e.g. a full process-tree kill (launcher died too, so it has
/// no in-memory history either) or `task dev` standalone mode (no launcher
/// at all). Reads srv's durable `Client.windowids` + each window's `kind`/
/// `parent_window_id` (SPEC_PILLAR1_STEP3) instead.
///
/// `main_window_id` is `"main"`'s own confirmed srv `window_id`, passed in
/// by the caller — NOT derived positionally from `Client.windowids[0]`.
/// reagent (P0, PR #2017, 2026-07-08) caught that the earlier design did
/// exactly that, on the assumption index 0 was reliably `"main"`; it isn't —
/// `focus_window` (`agentmux-srv/.../wcore/window.rs:164`) reorders
/// `Client.windowids` to put the last-focused window at index 0 on every
/// focus change, so index 0 is "whichever window the user looked at last,"
/// not a stable identity. Filtering `main_window_id` out BY VALUE (wherever
/// it appears in the list, or not at all) is correct regardless of
/// reordering; the caller has it with certainty because this only runs from
/// `register_backend_window`'s own `"main"` branch, i.e. after `"main"`
/// itself resolved and confirmed that id.
///
/// No `last_rect` is available this way — Step 2/3 never persisted window
/// position/size (see the spec's §4 risk) — so every recreated window lands
/// at `open_window_with_kind`'s default offset placement, not where the
/// user left it. Kind/parent/content are still fully correct.
///
/// Does its network I/O on a spawned thread, never the calling (UI) thread —
/// `backend_get_client_window_ids`/`backend_get_window_topology` are
/// blocking calls (same raw-TCP shape as every other `backend_*` read/write
/// helper in this codebase). By the time this runs, `ui_thread_gate.ready`
/// is already true (this fires well after "main"'s own registration), so
/// `open_window_with_kind` posting from this background thread is safe —
/// the same already-verified-safe pattern as any pool-window creation
/// posted after `"main"` registers.
///
/// Converges on the exact same per-window recreation code
/// (`reproject_from_snapshot`) the fast path uses, per the parent design
/// doc's "both tiers converge on the same per-window recreation code path."
pub(crate) fn reproject_from_srv(state: &Arc<AppState>, main_window_id: String) {
    let web_endpoint = state.backend_endpoints.lock().web_endpoint.clone();
    let auth_key = state.auth_key.lock().clone();
    let state = state.clone();
    std::thread::spawn(move || {
        let Some(window_ids) = crate::client::backend_get_client_window_ids(&web_endpoint, &auth_key) else {
            tracing::warn!(target: "reproject", "[reproject] slow path: could not read Client.windowids from srv");
            return;
        };

        // reagent P1 (PR #2017) — `reproject_from_snapshot`'s `label_remap` is
        // seeded only with the string `"main"` → `"main"` (the host's own
        // stable label for it), not srv's UUID for it. A subwindow whose
        // persisted `parent_window_id` equals `main_window_id` must be
        // translated to the `"main"` sentinel here — otherwise the remap
        // lookup finds nothing and the subwindow is silently skipped as if
        // its parent were missing.
        let mut extra_ids: Vec<&String> = window_ids.iter().filter(|id| *id != &main_window_id).collect();
        if extra_ids.is_empty() {
            tracing::debug!(
                target: "reproject",
                window_count = window_ids.len(),
                "[reproject] slow path: nothing beyond main to recreate"
            );
            return;
        }

        // Scan cap — every entry costs one blocking GetWindow round trip
        // (plus one CloseWindow if it turns out to be garbage), all
        // sequential on this thread, so the per-boot work must stay
        // bounded no matter how polluted the store is. Entries beyond the
        // cap are left in srv untouched for the next boot's pass — the
        // store still converges, just across a few launches instead of a
        // single unbounded one.
        let scan_dropped = cap_recreate_list(&mut extra_ids, MAX_SLOW_PATH_SCAN);
        if scan_dropped > 0 {
            tracing::warn!(
                target: "reproject",
                total = extra_ids.len() + scan_dropped,
                scanning = extra_ids.len(),
                dropped = scan_dropped,
                "[reproject] slow path: Client.windowids has more entries than one boot's \
                 scan budget — the rest are left in srv for the next launch"
            );
        }

        let mut snapshots: Vec<agentmux_common::ipc::WindowSnapshot> = Vec::new();
        let mut garbage: Vec<&String> = Vec::new();
        for window_id in extra_ids {
            let Some((kind, parent_window_id)) = crate::client::backend_get_window_topology(&web_endpoint, &auth_key, window_id) else {
                tracing::warn!(target: "reproject", window_id = %window_id, "[reproject] slow path: GetWindow failed — skipping this window");
                continue;
            };
            let kind = match kind.as_deref() {
                Some("subwindow") => agentmux_common::ipc::WindowKind::Subwindow,
                Some("full_instance") => agentmux_common::ipc::WindowKind::FullInstance,
                Some(other) => {
                    tracing::warn!(target: "reproject", window_id = %window_id, kind = %other, "[reproject] slow path: unrecognized persisted kind — defaulting to FullInstance");
                    agentmux_common::ipc::WindowKind::FullInstance
                }
                None => {
                    // No persisted kind means this row never completed a
                    // `register_backend_window` round trip (Step 3's
                    // write-through stamps `kind` on every registration,
                    // main included) — it is an orphan, not a window the
                    // user ever saw: the frontend double-init strands one
                    // such row per window creation, and pre-Step-3 rows
                    // have no restorable identity either. Recreating these
                    // as FullInstance windows is what turned a polluted
                    // `Client.windowids` into a window storm on every
                    // launch. Garbage-collect instead of recreating.
                    garbage.push(window_id);
                    continue;
                }
            };
            let parent_label = parent_window_id.map(|pid| {
                if pid == main_window_id {
                    "main".to_string()
                } else {
                    pid
                }
            });
            // Position/size mirror, added alongside `position_persist`'s
            // write-through — was unconditionally `None` before (see this
            // file's git history / SPEC_PILLAR1_STEP4_CRASH_REPROJECT_
            // 2026_07_07.md §4), the one case the fast path
            // (`reproject_from_snapshot`, which already has `w.last_rect`
            // from the launcher's live snapshot) didn't cover: a full
            // process-tree restart where the launcher died too. `None` here
            // still falls through to `open_window_with_kind`'s existing
            // default-placement heuristic, same as before this existed.
            let last_rect = crate::client::backend_get_window_pos_and_size(&web_endpoint, &auth_key, window_id);
            snapshots.push(agentmux_common::ipc::WindowSnapshot {
                label: window_id.clone(),
                kind,
                parent_label,
                hwnd: None,
                visible: true,
                iconic: false,
                last_rect,
                foregrounded_since_open: true,
            });
        }

        // GC the orphans right now — these ids never had a live window, so
        // there is nothing to confirm before closing (unlike the recreate
        // path's deferred closures below). `CloseWindow` prunes the
        // `Client.windowids` entry, deletes the row, and cascades the
        // orphan's (empty, auto-created) workspace only when no other
        // window references it — a polluted store self-heals here instead
        // of feeding next launch's reproject.
        if !garbage.is_empty() {
            tracing::warn!(
                target: "reproject",
                count = garbage.len(),
                "[reproject] slow path: garbage-collecting never-registered window rows"
            );
            for window_id in garbage {
                crate::client::backend_close_window(&web_endpoint, &auth_key, window_id);
            }
        }

        // Safety cap — found live (2026-07-08): a build force-killed many
        // times over one test session (never a graceful quit) accumulated
        // 30+ stale `Client.windowids` entries, all recreated at once on the
        // next cold boot. Orphan rows are now GC'd above rather than
        // recreated, but genuinely-restorable entries can still pile up
        // (e.g. repeated force-kills of real multi-window sessions). Bound
        // the worst case regardless of root cause: recreate at most this
        // many windows per pass; anything beyond that is left in place (not
        // lost — just not recreated this launch) with a loud warning,
        // rather than silently opening dozens of windows.
        let dropped = cap_recreate_list(&mut snapshots, MAX_SLOW_PATH_RECREATE);
        if dropped > 0 {
            tracing::warn!(
                target: "reproject",
                total = snapshots.len() + dropped,
                recreating = snapshots.len(),
                dropped,
                "[reproject] slow path: Client.windowids has far more restorable entries than \
                 a normal session should — capping recreation to avoid a runaway window-open \
                 storm; the rest are left in srv, uncreated, for now"
            );
        }
        tracing::info!(
            target: "reproject",
            window_count = snapshots.len(),
            "[reproject] slow path: recreating windows from srv"
        );
        let recreated_pairs = reproject_from_snapshot(&state, &snapshots);

        // Stash `new_label → old_window_id` — do NOT close the old id yet.
        // `reagent P1 (PR #2032): `open_window_with_kind`'s `Ok` only means a
        // `CreateWindowTask` was posted to the UI thread (fire-and-forget);
        // it is not proof the window exists. `register_backend_window`
        // (`commands/window/meta.rs`) drains this map and does the actual
        // close once `new_label`'s own registration fires — real
        // confirmation the new window's frontend loaded and round-tripped
        // IPC. Without this deferral (the first version of this fix),
        // closing right here on the unconfirmed `Ok` would delete the old
        // session's window/workspace/tabs with no replacement and no retry
        // if window creation subsequently failed silently (this session's
        // own Phase 2 investigation found `post_task` can do exactly that).
        //
        // This is what actually fixes the unbounded `Client.windowids`
        // growth found live (2026-07-08, 30+ accumulated entries from a test
        // session that only ever force-killed, never gracefully quit) —
        // deferred to a confirmed point rather than an optimistic one.
        if !recreated_pairs.is_empty() {
            let mut pending = state.pending_reproject_closures.lock();
            for (old_id, new_label) in recreated_pairs {
                pending.stage(new_label, old_id);
            }
        }
    });
}

fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3 / 2);
    for byte in s.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9'
            | b'-' | b'_' | b'.' | b'~' => out.push(byte as char),
            b => { let _ = std::fmt::Write::write_fmt(&mut out, format_args!("%{:02X}", b)); }
        }
    }
    out
}

/// Cascade anchor: 30px down-right of an existing top-level window of ours.
///
/// `None` when we have no window open — which is the NORMAL case in
/// background-service mode (issue #2977), where the tray's "New Window" is
/// often the first window of the session. Callers must supply their own origin
/// for that case; see `center_in_work_area`.
fn cascade_anchor() -> Option<(i32, i32)> {
    #[cfg(target_os = "windows")]
    unsafe {
        use windows_sys::Win32::Foundation::RECT;
        use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect;
        let hwnd = find_own_top_level_window();
        if !hwnd.is_null() {
            let mut rect: RECT = std::mem::zeroed();
            GetWindowRect(hwnd, &mut rect);
            return Some((rect.left + 30, rect.top + 30));
        }
    }
    None
}

/// Top-left origin that centres a `win_w` x `win_h` window inside a work area.
///
/// Pure so the arithmetic is testable without a display. Clamped at the work
/// origin: a window LARGER than the work area would otherwise get a negative
/// offset and hang off the top-left, which is the very failure being fixed
/// here — just with a different cause.
fn center_in_work_area(
    wa_x: i32,
    wa_y: i32,
    wa_w: i32,
    wa_h: i32,
    win_w: i32,
    win_h: i32,
) -> (i32, i32) {
    let x = wa_x + ((wa_w - win_w) / 2).max(0);
    let y = wa_y + ((wa_h - win_h) / 2).max(0);
    (x, y)
}

/// Where a new top-level window should go, given the size it will actually be
/// AND the work area to centre it in.
///
/// Cascades from an existing window when there is one. When there is NOT, the
/// window is centred in `work_area`.
///
/// The old behaviour here was to return `CW_USEDEFAULT` in that second case.
/// That is a valid sentinel for `CreateWindow`, which interprets it as "you
/// pick" — but nothing on this path passes it to `CreateWindow`. It travels on
/// as a literal coordinate: `promote_pool_window` takes it as a physical-pixel
/// `SetWindowPos` anchor, and `clamp_rect_within` then pulls that
/// `0x80000000` back inside the work area, landing every window flush in the
/// TOP-LEFT corner. Harmless while some window was always already open (the
/// cascade branch ran instead); very visible once background-service mode made
/// "no window open" the normal state and the tray's "New Window" the first one.
///
/// **`work_area` and `win_w`/`win_h` MUST be in the same unit**, and which unit
/// that is differs per caller — which is why the work area is passed in rather
/// than looked up here:
///
/// - `open_new_window`'s Windows pool path works in **physical** pixels
///   (`POOL_WIDTH`/`POOL_HEIGHT` reach `SetWindowPos` unconverted, so it pairs
///   them with `get_monitor_work_area_physical`).
/// - the `open_window_with_kind` cold path works in **DIP**
///   (`get_secondary_window_size` divides by the monitor scale because CEF
///   Views `set_bounds` expects DIP, so it pairs with `get_monitor_work_area`).
///
/// An earlier revision looked the work area up internally with the physical
/// variant and centred whatever size it was handed. That silently mixed a
/// physical work area with DIP dimensions on the cold path, mis-centring by
/// half the DPI difference on any monitor above 100% scale — i.e. on the
/// 125%/150% that Windows 11 ships by default (ReAgent P1 on #3019).
///
/// **Windows-only.** On macOS/Linux there is no work area lookup here and the
/// origin stays the historical `(100, 100)`; the corner-placement bug this
/// fixes is a Windows path (`promote_pool_window`/`clamp_rect_within`), and
/// those platforms position pool windows from the frontend instead.
fn new_window_origin(
    win_w: i32,
    win_h: i32,
    work_area: Option<(i32, i32, i32, i32)>,
) -> (i32, i32) {
    if let Some(anchor) = cascade_anchor() {
        return anchor;
    }
    if let Some((wa_x, wa_y, wa_w, wa_h)) = work_area {
        return center_in_work_area(wa_x, wa_y, wa_w, wa_h, win_w, win_h);
    }
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::CW_USEDEFAULT;
        (CW_USEDEFAULT, CW_USEDEFAULT)
    }
    #[cfg(not(target_os = "windows"))]
    {
        (100, 100)
    }
}


/// Compute 70% of the monitor's work area for a secondary window at (px, py).
/// Falls back to 1200x800 if the monitor can't be determined.
fn get_secondary_window_size(px: i32, py: i32) -> (i32, i32) {
    #[cfg(target_os = "windows")]
    {
        use crate::app::get_monitor_work_area;
        if let Some((_x, _y, work_w, work_h)) = get_monitor_work_area(px, py) {
            return ((work_w as f64 * 0.70) as i32, (work_h as f64 * 0.70) as i32);
        }
    }
    (1200, 800)
}

/// SPEC_PILLAR1_STEP4 Phase 3 safety cap — see `reproject_from_srv`'s own
/// comment at the call site for why this exists (found live, 2026-07-08:
/// 30+ accumulated `Client.windowids` entries recreated all at once).
const MAX_SLOW_PATH_RECREATE: usize = 20;

/// Bound on how many `Client.windowids` entries one boot's slow path will
/// even LOOK at (one blocking GetWindow round trip each, plus a CloseWindow
/// for each entry classified as garbage — all sequential). Distinct from
/// `MAX_SLOW_PATH_RECREATE`, which bounds windows actually opened: the scan
/// budget is deliberately larger so a badly polluted store (the 2026-07-09
/// storm left 60+ orphan rows per instance) still self-heals in one or two
/// launches, while a pathologically large list can't turn boot into an
/// unbounded sequence of network calls (reagent P1, PR #2048).
const MAX_SLOW_PATH_SCAN: usize = 200;

/// Truncates `items` to at most `max` entries in place; returns how many
/// were dropped. Extracted as a pure function (reagent P2, PR #2032,
/// 2026-07-08) so the cap itself has direct unit test coverage, not just
/// the live-verified end-to-end behavior.
fn cap_recreate_list<T>(items: &mut Vec<T>, max: usize) -> usize {
    if items.len() <= max {
        return 0;
    }
    let dropped = items.len() - max;
    items.truncate(max);
    dropped
}

#[cfg(test)]
mod cap_recreate_list_tests {
    use super::cap_recreate_list;

    #[test]
    fn under_the_cap_is_untouched() {
        let mut items = vec![1, 2, 3];
        assert_eq!(cap_recreate_list(&mut items, 20), 0);
        assert_eq!(items, vec![1, 2, 3]);
    }

    #[test]
    fn exactly_at_the_cap_is_untouched() {
        let mut items: Vec<i32> = (0..20).collect();
        assert_eq!(cap_recreate_list(&mut items, 20), 0);
        assert_eq!(items.len(), 20);
    }

    #[test]
    fn over_the_cap_truncates_and_reports_dropped_count() {
        let mut items: Vec<i32> = (0..44).collect();
        assert_eq!(cap_recreate_list(&mut items, 20), 24);
        assert_eq!(items.len(), 20);
        // Keeps the FIRST `max`, not an arbitrary subset — matters because
        // callers may care about ordering (e.g. FullInstance-before-Subwindow
        // sort already applied upstream).
        assert_eq!(items, (0..20).collect::<Vec<i32>>());
    }

    #[test]
    fn empty_list_is_a_noop() {
        let mut items: Vec<i32> = Vec::new();
        assert_eq!(cap_recreate_list(&mut items, 20), 0);
        assert!(items.is_empty());
    }
}

#[cfg(test)]
mod assets_missing_tests {
    use super::{assets_missing_html, FrontendUrlError};
    use std::path::PathBuf;

    #[test]
    fn copy_path_button_has_working_handler() {
        // Regression: js_string_literal() returns a DOUBLE-quoted JS string;
        // dropping it into a double-quoted onclick="..." attribute terminated
        // the attribute early, leaving the "Copy path" button inert. The
        // handler must be wired in a <script> block instead.
        let err = FrontendUrlError::AssetsMissing {
            checked_path: PathBuf::from("MISSING_ASSETS"),
        };
        let html = assets_missing_html(&err);

        assert!(
            !html.contains("onclick=\"navigator.clipboard"),
            "Copy path must not use an inline onclick with a double-quoted literal"
        );
        assert!(html.contains("addEventListener"), "handler should use addEventListener");
        assert!(html.contains("id=\"amx-copy\""), "Copy button needs its id");
        // The detail string is emitted as a valid JS string literal in script
        // context (js_string_literal wraps in double quotes).
        assert!(
            html.contains("writeText(\"Expected at: MISSING_ASSETS\")"),
            "Copy path must writeText the detail as a valid JS string literal"
        );
    }
}

#[cfg(test)]
mod new_window_origin_tests {
    use super::center_in_work_area;

    /// The reported bug: with no window open, the tray's "New Window" landed
    /// flush in the top-left instead of the middle of the main display.
    #[test]
    fn a_window_is_centred_on_a_standard_work_area() {
        // 1920x1080 minus a 40px taskbar, pool window 1200x800.
        assert_eq!(center_in_work_area(0, 0, 1920, 1040, 1200, 800), (360, 120));
    }

    /// Work areas do not start at (0, 0): a taskbar on the left or top shifts
    /// the origin, and a monitor left of the primary has negative coordinates.
    /// Centring must be relative to the work area, not the screen.
    #[test]
    fn centring_is_relative_to_the_work_area_origin() {
        assert_eq!(center_in_work_area(-1920, 0, 1920, 1040, 1200, 800), (-1560, 120));
        assert_eq!(center_in_work_area(0, 40, 1920, 1000, 1200, 800), (360, 140));
    }

    /// A window at least as large as the work area must sit AT the origin, not
    /// at a negative offset — that would reproduce the top-left-corner bug
    /// this function exists to fix, just by a different route.
    #[test]
    fn an_oversized_window_clamps_to_the_work_origin_rather_than_going_negative() {
        assert_eq!(center_in_work_area(0, 0, 1000, 700, 1200, 800), (0, 0));
        assert_eq!(center_in_work_area(100, 50, 1200, 800, 1200, 800), (100, 50));
    }

    /// Small monitors are the case where a half-pixel of rounding is most
    /// visible; assert the exact expected value so the arithmetic cannot drift.
    #[test]
    fn odd_leftovers_round_down_consistently() {
        assert_eq!(center_in_work_area(0, 0, 1001, 801, 1000, 800), (0, 0));
        assert_eq!(center_in_work_area(0, 0, 1003, 805, 1000, 800), (1, 2));
    }
}
