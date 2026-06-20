// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Floating-pane tear-off — the `open_floating_pane_window` IPC command and
//! the `get_pane_debug_state` diagnostic command.
//! Specs: `docs/specs/SPEC_FLOATING_PANE_TEAROFF_2026_05_11.md` (Windows,
//! issue #810) + `docs/specs/SPEC_MACOS_FLOATING_PANE_TEAROFF_2026_05_29.md`
//! (macOS/Linux Phase A).
//!
//! Creates a *subordinate* floating window — a free-positioned, chromeless
//! window showing just the torn-off pane (no tab bar, no widget bar). Unlike
//! the tab tear-off path (which spawns a full new AgentMux instance), a
//! floating pane shares the source instance's sidecar, data dir, and reducer
//! state. The embedded browser loads
//! `<frontend>?floatingPaneId=<id>&windowLabel=floating-<n>&workspaceId=<ws>`
//! and the frontend renders `<FloatingPaneWorkspace>` (chromeless).
//!
//! Platform primitive differs by OS:
//! - **Windows**: an unowned `WS_POPUP + WS_EX_TOOLWINDOW` HWND (no taskbar/
//!   Alt-Tab entry; cascade managed by `install_main_window_floater_cascade_hook`
//!   in `client/wndproc.rs`), CEF browser embedded — see `crate::floating_pane`.
//! - **macOS / Linux** (Phase A): a frameless CEF Views window created via the
//!   same `ui_tasks::post_create_window(frameless=true)` path the regular
//!   tear-off uses, with `&floatingPaneId=` injected into the URL. Owned-window
//!   lifecycle + redock are follow-ups (see the macOS spec, Phase B).

use std::sync::Arc;

use serde::Deserialize;

use crate::state::{AppState, WindowKind};

#[derive(Debug, Deserialize)]
pub struct OpenFloatingPaneArgs {
    /// Reducer-side identifier for the pane being torn off. Threaded
    /// through to the frontend via the query string so the floating-
    /// pane shell knows what to render.
    pub pane_id: String,
    /// Backend workspace id the floating window should attach to.
    /// Threaded through the URL so the floater's `initApp` →
    /// `initHostNewWindow` path picks it up via `?workspaceId=` and
    /// reuses the existing tear-off plumbing (frontend/app-init.ts:236).
    /// Optional for back-compat with Phase 1 callers that didn't pass it.
    /// Issue #1077.
    #[serde(default)]
    pub workspace_id: Option<String>,
    /// Screen-space top-left coordinates where the new floating window
    /// should appear, at the drop point. Coordinate space is per-platform
    /// (the consistent "Windows = physical px, macOS/Linux = DIP" rule):
    /// - **Windows**: PHYSICAL px — the frontend reads `get_cursor_point`
    ///   (`GetCursorPos`).
    /// - **macOS/Linux**: DIP (top-left, CSS px) — the frontend uses the DOM
    ///   `dragend` event's `screenX/Y`, and the host applies no scaling
    ///   (CEF Views positions in DIP).
    pub x: i32,
    pub y: i32,
    /// Initial window size in CSS / DIP pixels (NOT physical).
    /// - **Windows**: the host scales to physical px using the DESTINATION
    ///   monitor's DPI via `MonitorFromPoint(x, y)` + `GetDpiForMonitor`
    ///   (`#[cfg(target_os = "windows")]` below) — the frontend can't, since
    ///   it only knows its source monitor's DPR. This makes cross-DPI handoff
    ///   correct (drag from a 100% monitor onto a 150% one and the floater
    ///   keeps the source pane's visual size). Mirrors
    ///   `commands/window_pool.rs:684-701`.
    /// - **macOS/Linux**: passed through unscaled — CEF Views sizes in DIP.
    pub width: i32,
    pub height: i32,
    /// Window label of the FullInstance window initiating the tear-off.
    /// Used on Windows to bind the new floater to its parent's cascade hook
    /// so that minimize/restore/destroy only affects that window's floaters.
    /// Optional for back-compat; callers should always provide it.
    #[serde(default)]
    pub source_window_label: Option<String>,
}

#[derive(Debug, serde::Serialize)]
pub struct OpenFloatingPaneResponse {
    /// The window label assigned to the floating window. Stable for
    /// the life of the floater; persists into `state.window_meta` like
    /// any other top-level label.
    pub window_label: String,
}

/// IPC handler — called when the frontend or an agent invokes
/// `open_floating_pane_window` on the host. Validates input, allocates
/// a stable label, and posts a UI-thread task to create the owned HWND
/// and embed a CEF browser inside it.
pub fn open_floating_pane_window(
    state: &Arc<AppState>,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let parsed: OpenFloatingPaneArgs = serde_json::from_value(args.clone())
        .map_err(|e| format!("open_floating_pane_window: invalid args: {e}"))?;

    if parsed.pane_id.is_empty() {
        return Err("open_floating_pane_window: pane_id is required".to_string());
    }
    if parsed.width <= 0 || parsed.height <= 0 {
        return Err(format!(
            "open_floating_pane_window: width/height must be positive (got {}×{})",
            parsed.width, parsed.height
        ));
    }

    // The H.7 main-window-creation gate (any pane mid-close → wedged
    // Chromium IPC) applies here too — same Chromium message loop. If
    // a pane is closing, refuse the floating-window creation; the
    // caller retries.
    if state.any_browser_pane_closing() {
        tracing::warn!(
            target: "wfr:gate",
            "[wfr:gate] open_floating_pane_window refused — pane is mid-close (H.7 invariant)"
        );
        return Err("a pane is currently closing; retry shortly".to_string());
    }

    let window_id = uuid::Uuid::new_v4();
    let window_label = format!("floating-{}", window_id.simple());

    // Scale incoming CSS / DIP size to PHYSICAL pixels using the
    // DESTINATION monitor's DPI. The frontend can't do this — it only
    // knows its own (source) monitor's DPR. The destination monitor is
    // wherever (x, y) lands. Mirrors `commands/window_pool.rs:684-701`.
    #[cfg(target_os = "windows")]
    let parsed = {
        use windows_sys::Win32::Foundation::POINT;
        use windows_sys::Win32::Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTONEAREST};
        use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
        let pt = POINT { x: parsed.x, y: parsed.y };
        let dpi_scale: f32 = unsafe {
            let monitor = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
            let mut dpi_x: u32 = 0;
            let mut dpi_y: u32 = 0;
            let hr = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
            if hr != 0 || dpi_x == 0 {
                1.0
            } else {
                dpi_x as f32 / 96.0
            }
        };
        OpenFloatingPaneArgs {
            width: (parsed.width as f32 * dpi_scale).round() as i32,
            height: (parsed.height as f32 * dpi_scale).round() as i32,
            ..parsed
        }
    };

    tracing::info!(
        pane_id = %parsed.pane_id,
        label = %window_label,
        x = parsed.x,
        y = parsed.y,
        w = parsed.width,
        h = parsed.height,
        "[floating-pane] open_floating_pane_window request",
    );

    #[cfg(target_os = "windows")]
    {
        // Resolve source window → parent HWND so the cascade hook restricts
        // min/restore/destroy to only this window's floaters.
        // Primary lookup: label → HWND directly.
        // Fallback: first FullInstance window in window_meta (covers old
        // frontend clients that predate source_window_label, and the rare
        // race where the label isn't in window_hwnds yet).
        let parent_main_hwnd: isize = {
            let direct = parsed
                .source_window_label
                .as_deref()
                .and_then(|lbl| state.window_hwnds.lock().get(lbl).copied())
                .unwrap_or(0);
            if direct != 0 {
                direct
            } else {
                let fallback_label: Option<String> = state
                    .window_meta
                    .lock()
                    .values()
                    .find(|m| m.kind == WindowKind::FullInstance)
                    .map(|m| m.label.clone());
                let fallback = fallback_label
                    .as_deref()
                    .and_then(|lbl| state.window_hwnds.lock().get(lbl).copied())
                    .unwrap_or(0);
                if fallback == 0 {
                    tracing::warn!(
                        source_label = ?parsed.source_window_label,
                        "[floating-pane] could not resolve parent main HWND; \
                         floater will not cascade on minimize/destroy (orphan risk)",
                    );
                } else {
                    tracing::debug!(
                        source_label = ?parsed.source_window_label,
                        fallback,
                        "[floating-pane] source_window_label unresolved; \
                         using FullInstance fallback HWND for cascade binding",
                    );
                }
                fallback
            }
        };
        // Pane pool fast path (Windows): promote a pre-warmed WS_POPUP pane pool
        // window. `width` and `height` are already DPI-converted to physical
        // pixels by the block above. `parent_main_hwnd` is passed so the cascade
        // hook is registered at promote time (deferred from spawn because the
        // parent was unknown when the pool window was pre-warmed).
        if let Some(pool_label) = crate::commands::window_pool::promote_pane_pool_window(
            state,
            &parsed.pane_id,
            parsed.workspace_id.as_deref().unwrap_or(""),
            parsed.x,
            parsed.y,
            parsed.width,
            parsed.height,
            parent_main_hwnd,
        ) {
            return Ok(serde_json::to_value(OpenFloatingPaneResponse {
                window_label: pool_label,
            }).unwrap_or_default());
        }

        crate::floating_pane::post_create_floating_window(state, &parsed, &window_label, parent_main_hwnd);
        Ok(serde_json::to_value(OpenFloatingPaneResponse { window_label }).unwrap_or_default())
    }

    #[cfg(not(target_os = "windows"))]
    {
        // Pane pool fast path — promote a pre-warmed frameless pool window
        // instead of spawning a cold CEF window (~3s → ~30ms).
        if let Some(pool_label) = crate::commands::window_pool::promote_pane_pool_window(
            state,
            &parsed.pane_id,
            parsed.workspace_id.as_deref().unwrap_or(""),
            parsed.x,
            parsed.y,
            parsed.width,
            parsed.height,
            0, // parent_hwnd: unused on non-Windows
        ) {
            return Ok(serde_json::to_value(OpenFloatingPaneResponse {
                window_label: pool_label,
            }).unwrap_or_default());
        }

        // Phase A (SPEC_MACOS_FLOATING_PANE_TEAROFF_2026_05_29.md): create a
        // frameless top-level window via the SAME CEF Views path the regular
        // tear-off uses (`open_window_at_position` →
        // `ui_tasks::post_create_window(..., frameless=true)`), but with a
        // `floatingPaneId` in the URL so the frontend renders
        // `<FloatingPaneWorkspace>` (chromeless: no tab bar, no widget bar)
        // instead of `<Workspace>`. Secondary windows are already frameless on
        // macOS/Linux (window/creation.rs), so this alone produces "just the
        // pane" — the user's request.
        //
        // No DPI scaling on non-Windows: the Windows-only block above is
        // skipped, and CEF Views positions/sizes in DIP (logical px), which is
        // exactly what `width`/`height` (from `getBoundingClientRect`) and
        // `x`/`y` (from the DOM drop event's `screenX/Y`) already are.
        //
        // Phase B adds owned-window lifecycle (follows/minimizes/closes with
        // the source window), JS header-drag, and redock — see the spec.
        let ipc_port = *state.ipc_port.lock();
        let ipc_token = &state.ipc_token;
        let workspace_id = parsed.workspace_id.clone().unwrap_or_default();

        let url = match super::window::resolve_frontend_base_url(ipc_port) {
            Ok(base_url) => {
                let separator = if base_url.contains('?') { "&" } else { "?" };
                let mut u = format!(
                    "{}{}ipc_port={}&ipc_token={}&windowLabel={}&floatingPaneId={}",
                    base_url, separator, ipc_port, ipc_token, window_label, parsed.pane_id
                );
                if !workspace_id.is_empty() {
                    u.push_str(&format!("&workspaceId={}", workspace_id));
                }
                u
            }
            Err(e) => {
                tracing::error!(
                    error = %e,
                    label = %window_label,
                    "[floating-pane] frontend assets unavailable — opening static error page",
                );
                super::window::assets_missing_data_url(&e)
            }
        };

        // Register the pending window (same as the tear-off cold path). The
        // floater shares the source instance's sidecar/data dir — it's not a
        // separate launcher instance — but on non-Windows there is no launcher
        // bookkeeping, so `FullInstance` (matching `open_window_at_position`)
        // is the proven kind for this creation path. Phase B (owned window)
        // may revisit.
        state.host_dispatch(
            crate::reducer::HostCommand::EnqueuePendingWindowCreation {
                entry: crate::state::PendingWindowCreation {
                    label: window_label.clone(),
                    kind: crate::state::WindowKind::FullInstance,
                    parent_instance_id: None,
                },
            },
        );

        crate::ui_tasks::post_create_window(
            state,
            &url,
            &window_label,
            parsed.x,
            parsed.y,
            parsed.width,
            parsed.height,
            true, // frameless — secondary windows use the custom title bar
        );

        Ok(serde_json::to_value(OpenFloatingPaneResponse { window_label }).unwrap_or_default())
    }
}

/// Diagnostic snapshot of active floater state — called by the frontend on
/// every tear-off attempt (before + after) to enable structured logging and
/// to surface intermittent failures without requiring a host-side repro.
///
/// Returns JSON:
/// ```json
/// {
///   "floaters": [{"label": "floating-abc", "hwnd": 12345678}],
///   "any_browser_pane_closing": false,
///   "pool_queue_size": 2,
///   "pending_window_creations": 0
/// }
/// ```
pub fn get_pane_debug_state(state: &Arc<AppState>) -> serde_json::Value {
    #[cfg(target_os = "windows")]
    let floaters: Vec<serde_json::Value> = crate::floating_pane::floater_debug_snapshot()
        .into_iter()
        .map(|(label, hwnd)| serde_json::json!({ "label": label, "hwnd": hwnd }))
        .collect();
    #[cfg(not(target_os = "windows"))]
    let floaters: Vec<serde_json::Value> = vec![];

    let any_closing = state.any_browser_pane_closing();
    let pool_queue = state.pool_queue_size();
    let pending = state
        .host_state
        .lock()
        .pending_window_creations
        .len();

    serde_json::json!({
        "floaters": floaters,
        "any_browser_pane_closing": any_closing,
        "pool_queue_size": pool_queue,
        "pending_window_creations": pending,
    })
}
