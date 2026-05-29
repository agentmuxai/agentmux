// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Window management commands for the CEF host.
// Ported from src-tauri/src/commands/window.rs.
//
// Phase 2: Single-window only. Multi-window commands are stubbed.
//
// Modularization (docs/analysis/ANALYSIS_LARGE_FILE_MODULARIZATION_CANDIDATES_2026_05_28.md,
// Plan 1): the close path + HWND-resolution helpers live in `lifecycle.rs`.
// Motion / chrome / transparency / meta / creation handlers remain here
// pending later carves.

use std::path::PathBuf;
use std::sync::Arc;

use crate::client::helpers::{html_escape, js_string_literal};
use crate::state::AppState;

mod lifecycle;
// Cross-platform command handlers dispatched by ipc.rs.
pub use lifecycle::{close_window, close_window_by_label};
// Windows-only helpers other modules resolve as `commands::window::<name>`
// (browser_pane / client / backend call sites are all `#[cfg(windows)]`).
#[cfg(target_os = "windows")]
pub(crate) use lifecycle::{capture_hwnd_for_label, find_own_top_level_window};

mod motion;
// Position / drag / redock-hover command handlers, all dispatched by ipc.rs.
pub use motion::*;

mod chrome;
// Minimize / maximize command handlers, dispatched by ipc.rs.
pub use chrome::*;

mod transparency;
// Transparency + per-window opacity command handlers, dispatched by ipc.rs.
pub use transparency::*;

/// Get the current zoom factor.
pub fn get_zoom_factor(state: &Arc<AppState>) -> serde_json::Value {
    let factor = *state.zoom_factor.lock();
    serde_json::json!(factor)
}

/// Set the zoom factor.
/// CEF zoom uses a logarithmic scale: zoom_level = log2(zoom_factor)
/// So factor 1.0 = level 0, factor 2.0 = level 1, factor 0.5 = level -1
pub fn set_zoom_factor(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let factor = args
        .get("factor")
        .and_then(|v| v.as_f64())
        .ok_or_else(|| "Missing factor".to_string())?;

    let factor = factor.clamp(0.5, 3.0);
    *state.zoom_factor.lock() = factor;

    // Convert to CEF zoom level (log base 1.2)
    // CEF uses: zoom_factor = 1.2 ^ zoom_level
    // So: zoom_level = log(zoom_factor) / log(1.2)
    let zoom_level = factor.ln() / 1.2_f64.ln();

    // NOTE: host.set_zoom_level() deadlocks from IPC thread, and post_task
    // crashes with current CEF bindings. Zoom is applied via CSS on the frontend.
    // The zoom_factor state is stored for get_zoom_factor queries.

    // Emit zoom-factor-change event
    crate::events::emit_event_from_state(state, "zoom-factor-change", &serde_json::json!(factor));

    Ok(serde_json::Value::Null)
}





/// Get the current window label.
/// The frontend passes its own label (extracted from URL params) as an arg.
pub fn get_window_label(args: &serde_json::Value) -> serde_json::Value {
    let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("main");
    serde_json::json!(label)
}

/// Check if this is the main window.
/// The frontend passes its own label (extracted from URL params) as an arg.
pub fn is_main_window(args: &serde_json::Value) -> serde_json::Value {
    let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("main");
    serde_json::json!(label == "main")
}

/// Return the OS double-click interval in milliseconds.
/// On Windows: `GetDoubleClickTime()` — typically 500ms, user-configurable
/// via Mouse settings. On non-Windows: hardcoded 500ms (the Win32 default,
/// also a common cross-platform default; Phase 7 can refine per platform).
///
/// Used by the InstancePanel to defer single-click focus past the user's
/// dblclick threshold so dblclick-to-rename works for everyone, not just
/// users with the default-or-faster setting. Without this query, a fixed
/// constant would make rename unreliable for slow double-clickers
/// (codex PR #569 round-2 P2).
pub fn get_double_click_time() -> serde_json::Value {
    #[cfg(target_os = "windows")]
    {
        let ms = unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime() };
        serde_json::json!(ms)
    }
    #[cfg(not(target_os = "windows"))]
    {
        serde_json::json!(500u32)
    }
}

/// List all open window instances with their backend window IDs.
/// Same filtering as `list_windows` (excludes unpromoted pool windows
/// and browser-pane child HWNDs), but returns `[{label, windowId}]`
/// pairs so the frontend can resolve per-window backend objects
/// (Window record → meta["window:displayname"], etc.) without an
/// extra round-trip per row.
///
/// `windowId` is `None` for windows that haven't yet completed the
/// `register_backend_window` round-trip — typically a freshly-spawned
/// window before its frontend has finished init. Callers should
/// fall back to label/index-based naming in that case.
pub fn list_window_instances(state: &Arc<AppState>) -> serde_json::Value {
    // Atomic snapshot — pool inventory + browsers under ONE lock.
    // Two-lock variants race against `promote_pool_window` between
    // the reads and would let a just-promoted user window be
    // excluded (or admit a still-hidden pool window).
    let (pool_labels, browsers) = state.user_visibility_snapshot();
    let labels: Vec<String> = browsers
        .into_iter()
        .map(|(l, _)| l)
        .filter(|l| !pool_labels.contains(l.as_str()) && !l.starts_with("browser-pane-"))
        .collect();
    // Read backend window IDs via `state.backend_window_id()`,
    // which queries the launcher-fed `shadow_backend_window_ids`
    // (sole source of truth post-B.5e). Resolve labels OUTSIDE
    // the browsers lock to avoid nesting (browsers + shadow).
    let entries: Vec<serde_json::Value> = labels
        .iter()
        .map(|l| {
            serde_json::json!({
                "label": l,
                "windowId": state.backend_window_id(l),
            })
        })
        .collect();
    serde_json::json!(entries)
}

/// List all open window labels, excluding unpromoted pool windows.
/// Pool windows are pre-warmed tear-off scratch windows kept hidden
/// from the user (WS_EX_TOOLWINDOW, no taskbar entry). Including them
/// in `list_windows` inflates the frontend's InstancePanel row count
/// with phantom entries the user can't see or focus.
///
/// Uses `state.user_visibility_snapshot()` — atomic read of pool
/// inventory (`unpromoted` ∪ `pool.queue`) and the browser registry
/// under one host_state lock. Both `unpromoted` (populated at spawn
/// time) and `pool.queue` (populated after renderer-ready, before
/// promote) are host-internal: the window is hidden off-screen and
/// has no UI a user could see or focus. The atomic read is required
/// because a two-lock variant races against `promote_pool_window`.
pub fn list_windows(state: &Arc<AppState>) -> serde_json::Value {
    let (pool_labels, browsers) = state.user_visibility_snapshot();
    let labels: Vec<String> = browsers
        .into_iter()
        .map(|(l, _)| l)
        .filter(|l| !pool_labels.contains(l.as_str()))
        .collect();
    serde_json::json!(labels)
}

/// Focus a specific window by label.
///
/// Uses the CEF Views `Window::activate()` API on all platforms (via
/// `post_focus_window` → `FocusWindowTask`). On Windows in Views mode,
/// `browser.host().window_handle()` returns NULL — the previous direct
/// SetForegroundWindow path silently failed there. Views' `activate()`
/// resolves the actual top-level HWND through `browser_view_get_for_browser
/// → window()` which is the only correct way to reach it in Views mode.
pub fn focus_window(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("main");
    crate::ui_tasks::post_focus_window(state, label);
    Ok(serde_json::Value::Null)
}

/// Get the instance number for the current window.
///
/// Reads from `state.instance_num()` which queries the launcher-fed
/// `shadow_instance_registry` (B.5e — sole source of truth post-migration).
/// Brief race window for early lookups: see `app-init.ts` retry logic.
pub fn get_instance_number(state: &Arc<AppState>, args: &serde_json::Value) -> serde_json::Value {
    let label = args
        .get("label")
        .and_then(|v| v.as_str())
        .unwrap_or("main");
    serde_json::json!(state.instance_num(label).unwrap_or(1))
}

/// Register the backend window ID for a window label.
/// Called by the frontend after it has initialized its backend Window object.
/// Used by `on_before_close` to notify the backend when a secondary window closes.
pub fn register_backend_window(_state: &Arc<AppState>, args: &serde_json::Value) -> serde_json::Value {
    let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("main");
    let window_id = args.get("window_id").and_then(|v| v.as_str()).unwrap_or("");
    tracing::info!(label = %label, window_id = %window_id, "[window] register_backend_window received");
    crate::client::dlog(&format!("register_backend_window: label={} window_id={}", label, window_id));
    if !window_id.is_empty() {
        // Phase B.5 (window_id_map step d) — host no longer mutates
        // `window_id_map` locally. The launcher's
        // `state.backend_window_ids` (B.5 step a) is sole authority;
        // we just send the report and the shadow update populates
        // the host-side projection.
        tracing::info!(label = %label, window_id = %window_id, "[window] registered backend window ID");
        crate::launcher_ipc::report_backend_window_id_registered(
            label.to_string(),
            window_id.to_string(),
        );
        // Phase B.7.3.3 — the launcher's
        // `Event::BackendWindowIdRegistered` (delivered via the CEF
        // JS bridge) carries the label → windowId mapping change to
        // every renderer's reducer. No sync emit here.
    } else {
        tracing::warn!(label = %label, "[window] register_backend_window called with empty window_id — skipped");
    }
    serde_json::Value::Null
}

/// Toggle DevTools for the main window.
///
/// Uses CEF's native show_dev_tools() API, which triggers
/// BrowserViewDelegate::on_popup_browser_view_created with is_devtools=1.
/// That callback creates a top-level CefWindow with a native title bar,
/// producing a standalone DevTools window — identical to Tauri's open_devtools().
pub fn toggle_devtools(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("main");
    crate::ui_tasks::post_show_dev_tools(state, label);
    Ok(serde_json::Value::Null)
}

/// Open DevTools focused on the element at the given window-relative
/// coordinates. Equivalent to Chrome's right-click → Inspect Element.
/// Used by the pane context menu's Inspect entry.
pub fn inspect_element_at(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("main");
    let x = args.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let y = args.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    crate::ui_tasks::post_inspect_element_at(state, label, x, y);
    Ok(serde_json::Value::Null)
}

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
    // Detect dev mode. Two reachable scenarios:
    //   a) Launcher-managed: AGENTMUX_RUNTIME_MODE is set, from_env()
    //      returns Some.
    //   b) Standalone `task dev`: env absent. Fall through to
    //      RuntimeMode::current() against the host exe path so the
    //      same `dist/cef-dev/` build dir → Dev classification fires
    //      that the launcher would have used.
    let mode = agentmux_common::RuntimeMode::from_env().or_else(|| {
        resolve_host_runtime_dir()
            .ok()
            .map(|d| agentmux_common::RuntimeMode::current(&d))
    });
    if matches!(mode, Some(agentmux_common::RuntimeMode::Dev { .. })) {
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
    let html = format!(
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
<button onclick="navigator.clipboard &amp;&amp; navigator.clipboard.writeText({detail_js})">Copy path</button>
<button onclick="window.close()">Quit</button>
</div></div></body></html>"#,
        reason_safe = html_escape(&reason),
        detail_safe = html_escape(&detail),
        detail_js = js_string_literal(&detail),
    );
    let b64 = cef::base64_encode(Some(html.as_bytes()));
    let b64_str = cef::CefString::from(&b64).to_string();
    format!("data:text/html;base64,{}", b64_str)
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

// ── Per-window opacity (SPEC_PER_WINDOW_OPACITY_2026-05-14.md) ───────────────


