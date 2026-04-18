// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Pane-specific CEF callback bodies.
//!
//! Extracted from `client.rs` in Phase 4 of the modularization split
//! (see `docs/specs/SPEC_BROWSER_PANE_MODULARIZATION.md` §6). `AgentMuxHandler`
//! still owns the CEF callback plumbing; this module holds the pane-branch
//! bodies so pane-specific logic lives in one place instead of threaded
//! through `if self.is_pane` branches in `client.rs`.
//!
//! Notable: this is where `install_pane_focus_redirect` actually gets wired
//! in. Before this phase the function existed in `pane::hwnd` but had zero
//! callers (see `SPEC_BROWSER_PANE_LIFECYCLE.md` §5 race #5). Now
//! `on_after_created_pane` and `on_load_end_pane` both reinstall the focus
//! subclass — required because Chromium recreates the
//! `Chrome_RenderWidgetHostHWND` child on every navigation, stranding the
//! old subclass on a destroyed HWND.

use std::sync::Arc;

use cef::*;

use crate::state::AppState;

/// Called from `AgentMuxHandler::on_after_created` when the browser being
/// registered is a pane (label prefix `browser-pane-*`).
///
/// Responsibilities:
/// 1. Raise the pane's outer HWND to the top of its parent's Z-order so
///    mouse-wheel events reach the pane renderer rather than main's.
/// 2. Install the WM_SETFOCUS redirect subclass on the pane's HWND tree so
///    Chromium's internal focus-steals on page load don't yank keyboard
///    focus away from the main window.
pub fn on_after_created_pane(_state: &Arc<AppState>, browser: &Browser) {
    #[cfg(target_os = "windows")]
    {
        if let Some(host) = browser.host() {
            let wh = host.window_handle();
            if !wh.0.is_null() {
                let hwnd = wh.0 as *mut std::ffi::c_void;

                // Z-order: bring pane above main's widget.
                unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::SetWindowPos(
                        hwnd as _,
                        std::ptr::null_mut(), // HWND_TOP
                        0, 0, 0, 0,
                        0x0001 | 0x0002 | 0x0010, // SWP_NOSIZE | SWP_NOMOVE | SWP_NOACTIVATE
                    );
                }
                tracing::info!("[pane-zorder] raised pane to top of Z-order");

                // Subclass the pane HWND + its descendants so WM_SETFOCUS
                // from Chromium gets redirected to the parent.
                unsafe {
                    crate::pane::hwnd::install_pane_focus_redirect(hwnd);
                }
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = browser;
    }
}

/// Called from `AgentMuxHandler::on_before_close` after the browser has
/// been removed from `state.browsers` and the label has been identified
/// as a pane label (prefix `browser-pane-*`).
///
/// Drains the lifecycle entry from `BrowserPaneManager` so a re-create
/// with the same block_id gets a fresh Live state. Idempotent — if the
/// explicit `close()` path already drained it, this is a no-op.
pub fn on_before_close_pane(state: &Arc<AppState>, label: &str) {
    state.browser_panes.drain_closed_label(label);
}

/// Called from `AgentMuxHandler::on_load_end` when `is_pane` is true.
///
/// Chromium creates a fresh `Chrome_RenderWidgetHostHWND` on every
/// navigation. The subclass installed at `on_after_created` is on the
/// OLD widget HWND, which was destroyed during navigation — so without
/// reinstalling here, keyboard focus steals by the new page bypass our
/// redirect and end up stuck on the pane.
///
/// Does NOT force focus back to main. `WM_MOUSEWHEEL` is routed to the
/// focused HWND; stealing focus away from the pane breaks scrolling.
/// The FocusHandler cancel + WndProc redirect already keep focus off
/// the pane during the *initial* navigation focus steal.
pub fn on_load_end_pane(_state: &Arc<AppState>, browser: &Browser) {
    tracing::info!("[pane-load-end] pane page loaded; reinstalling focus subclass");

    #[cfg(target_os = "windows")]
    {
        if let Some(host) = browser.host() {
            let wh = host.window_handle();
            if !wh.0.is_null() {
                unsafe {
                    crate::pane::hwnd::install_pane_focus_redirect(wh.0 as *mut std::ffi::c_void);
                }
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = browser;
    }
}
