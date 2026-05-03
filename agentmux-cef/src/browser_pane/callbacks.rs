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
//! Notable: this is where `install_browser_pane_focus_redirect` actually gets wired
//! in. Before this phase the function existed in `pane::hwnd` but had zero
//! callers (see `SPEC_BROWSER_PANE_LIFECYCLE.md` §5 race #5). Now
//! `on_after_created_browser_pane` and `on_load_end_pane` both reinstall the focus
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
pub fn on_after_created_browser_pane(state: &Arc<AppState>, browser: &Browser) {
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
                // from Chromium gets redirected to the parent. The state
                // and block_id let the subclass emit `browser-pane-clicked`
                // directly on WM_LBUTTONDOWN without relying on CEF focus
                // callbacks (which don't fire for clicks inside an already-
                // focused pane).
                let block_id = resolve_pane_block_id(state, browser).unwrap_or_default();
                unsafe {
                    crate::browser_pane::hwnd::install_browser_pane_focus_redirect(
                        hwnd,
                        state.clone(),
                        block_id,
                    );
                }
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (state, browser);
    }
}

/// Called from `AgentMuxHandler::on_before_close` after the browser has
/// been removed from `state.browsers` and the label has been identified
/// as a pane label (prefix `browser-pane-*`).
///
/// Drains the lifecycle entry from `BrowserPaneManager` so a re-create
/// with the same block_id gets a fresh Live state. Idempotent — if the
/// explicit `close()` path already drained it, this is a no-op.
pub fn on_before_close_browser_pane(state: &Arc<AppState>, label: &str) {
    state.browser_panes.drain_closed_label(state, label);

    // Labels are `browser-pane-<uuid>-<seq>`; strip prefix + trailing `-<seq>`
    // to recover the block_id, then wipe any HWND context entries the
    // WndProc subclass registered for that block.
    #[cfg(target_os = "windows")]
    {
        if let Some(rest) = label.strip_prefix("browser-pane-") {
            if let Some(dash) = rest.rfind('-') {
                let block_id = &rest[..dash];
                crate::browser_pane::hwnd::remove_contexts_for_block(block_id);
            }
        }
    }
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
pub fn on_load_end_pane(state: &Arc<AppState>, browser: &Browser) {
    tracing::info!("[pane-load-end] pane page loaded; reinstalling focus subclass");

    #[cfg(target_os = "windows")]
    {
        if let Some(host) = browser.host() {
            let wh = host.window_handle();
            if !wh.0.is_null() {
                let block_id = resolve_pane_block_id(state, browser).unwrap_or_default();
                unsafe {
                    crate::browser_pane::hwnd::install_browser_pane_focus_redirect(
                        wh.0 as *mut std::ffi::c_void,
                        state.clone(),
                        block_id,
                    );
                }
            }
        }
    }

    // URL-only event emit at load_end so the address bar catches redirects
    // that resolve during frame load (e.g. google.com → www.google.com).
    // `can_go_back` / `can_go_forward` are intentionally not read here —
    // `on_load_end` fires before the navigation controller commits the
    // history entry, so calling `browser.can_go_back()` from this hook
    // can return the pre-navigation state. Those flags flow through the
    // dedicated `on_loading_state_change_pane` callback below, which CEF
    // provides with correct values as direct parameters.
    if let Some(block_id) = resolve_pane_block_id(state, browser) {
        let url = {
            let mut b: cef::Browser = browser.clone();
            b.main_frame()
                .map(|f| cef::CefString::from(&cef::ImplFrame::url(&f)).to_string())
                .unwrap_or_default()
        };
        crate::events::emit_event_from_state(
            state,
            "browser-pane-nav-state",
            &serde_json::json!({
                "block_id": block_id,
                "url": url,
                // can_* omitted on purpose — frontend treats missing
                // fields as "no change" and keeps the last values from
                // on_loading_state_change.
                "url_only": true,
            }),
        );
    } else {
        tracing::warn!("[pane-load-end] couldn't resolve block_id for nav-state emit");
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = browser;
    }
}

/// Pane-specific `on_loading_state_change` body. Called from
/// `AgentMuxHandler::on_loading_state_change` when `is_pane == true`.
///
/// CEF invokes `on_loading_state_change` whenever the navigation controller's
/// history state changes — navigation start, navigation commit, and after
/// back/forward. `can_go_back` / `can_go_forward` are provided as direct
/// parameters (not queried after the fact), so they're guaranteed to reflect
/// the real committed state rather than the pre-commit race window.
pub fn on_loading_state_change_pane(
    state: &Arc<AppState>,
    browser: &Browser,
    can_go_back: bool,
    can_go_forward: bool,
) {
    if let Some(block_id) = resolve_pane_block_id(state, browser) {
        let url = {
            let mut b: cef::Browser = browser.clone();
            b.main_frame()
                .map(|f| cef::CefString::from(&cef::ImplFrame::url(&f)).to_string())
                .unwrap_or_default()
        };
        tracing::info!(
            block_id = %block_id,
            can_back = can_go_back,
            can_forward = can_go_forward,
            url = %url,
            "[pane-nav-state] emitting",
        );
        crate::events::emit_event_from_state(
            state,
            "browser-pane-nav-state",
            &serde_json::json!({
                "block_id": block_id,
                "url": url,
                "can_go_back": can_go_back,
                "can_go_forward": can_go_forward,
            }),
        );
    } else {
        tracing::warn!("[pane-loading-state] couldn't resolve block_id for nav-state emit");
    }
}

/// Resolve the `block_id` for a pane browser. Panes are registered in
/// `state.browsers` under labels like `browser-pane-<uuid>-<seq>`. Find the
/// label whose browser handle matches the given one by `is_same`, then
/// strip the prefix and the trailing `-<seq>` to recover the uuid.
fn resolve_pane_block_id(state: &Arc<AppState>, browser: &Browser) -> Option<String> {
    // Phase H.2.b — reducer-aware iteration with fallback.
    state
        .list_browsers()
        .into_iter()
        .find(|(_k, b)| {
            let mut b_clone = b.clone();
            let mut browser_clone: cef::Browser = browser.clone();
            b_clone.is_same(Some(&mut browser_clone)) != 0
        })
        .and_then(|(label, _)| {
            let rest = label.strip_prefix("browser-pane-")?;
            let dash = rest.rfind('-')?;
            Some(rest[..dash].to_string())
        })
}
