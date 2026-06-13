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
/// on the CLI. See `docs/analyses/ANALYSIS_DEV_VITE_PORT_HARDCODE_2026-05-26.md`.
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
pub fn open_new_window(state: &Arc<AppState>) -> Result<serde_json::Value, String> {
    open_window_with_kind(state, crate::state::WindowKind::FullInstance, None)
}

/// Open a sub-window tied to `parent_instance_id`. **Not exposed to users** —
/// reserved for agent / backend callers that need a transient auxiliary
/// top-level window (tool-spawned panels, diff views, etc.). Sub-windows are
/// hidden from the taskbar via `ITaskbarList::DeleteTab` and close when their
/// parent full instance closes.
pub fn open_subwindow(
    state: &Arc<AppState>,
    parent_instance_id: String,
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
    )
}

fn open_window_with_kind(
    state: &Arc<AppState>,
    kind: crate::state::WindowKind,
    parent_instance_id: Option<String>,
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

    let window_id = uuid::Uuid::new_v4();
    let label = format!("window-{}", window_id.simple());

    let ipc_port = *state.ipc_port.lock();
    let ipc_token = &state.ipc_token;
    let url = match resolve_frontend_base_url(ipc_port) {
        Ok(base_url) => {
            let separator = if base_url.contains('?') { "&" } else { "?" };
            format!(
                "{}{}ipc_port={}&ipc_token={}&windowLabel={}",
                base_url, separator, ipc_port, ipc_token, label
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
    let (pos_x, pos_y) = get_offset_position();
    let (win_w, win_h) = get_secondary_window_size(pos_x, pos_y);

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

/// Get an offset position for a new window: 30px right and 30px down from the current window.
fn get_offset_position() -> (i32, i32) {
    #[cfg(target_os = "windows")]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        use windows_sys::Win32::Foundation::RECT;
        let hwnd = find_own_top_level_window();
        if !hwnd.is_null() {
            let mut rect: RECT = std::mem::zeroed();
            GetWindowRect(hwnd, &mut rect);
            return (rect.left + 30, rect.top + 30);
        }
    }
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::CW_USEDEFAULT;
        return (CW_USEDEFAULT, CW_USEDEFAULT);
    }
    #[cfg(not(target_os = "windows"))]
    (100, 100)
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
