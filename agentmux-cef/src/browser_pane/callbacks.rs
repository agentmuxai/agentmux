// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Pane-specific CEF callback bodies.
//!
//! Extracted from `client.rs` in Phase 4 of the modularization split
//! (see `docs/specs/SPEC_BROWSER_PANE_MODULARIZATION.md` §6). `AgentMuxHandler`
//! still owns the CEF callback plumbing; this module holds the pane-branch
//! bodies so pane-specific logic lives in one place instead of threaded
//! through `if self.is_browser_pane` branches in `client.rs`.
//!
//! Notable: this is where `install_browser_pane_focus_redirect` actually gets wired
//! in. Before this phase the function existed in `browser_pane::hwnd` but had zero
//! callers (see `SPEC_BROWSER_PANE_LIFECYCLE.md` §5 race #5). Now
//! `on_after_created_browser_pane` and `on_load_end_browser_pane` both reinstall the focus
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
        // On macOS/Linux, the focus redirect subclass used on Windows doesn't
        // exist. CEF's on_after_created fires from inside add_overlay_view
        // (when the BrowserView is added to the widget hierarchy) and gives
        // focus to the new pane browser. creation_views::create_browser_pane_view
        // immediately returns focus to the main window after add_overlay_view
        // returns — this callback is an additional safety net for any navigation-
        // driven focus steals that bypass that initial return (e.g. cross-origin
        // redirects that recreate the renderer, hitting on_after_created again).
        //
        // Only refocus if we can identify the parent window; don't do a blanket
        // "focus main" that would disrupt multi-window setups.
        if let Some(block_id) = resolve_pane_block_id(state, browser) {
            let parent_window_label = state
                .browser_pane_overlays
                .lock()
                .iter()
                .find(|(lbl, _)| {
                    // label format: browser-pane-<block_id>-<seq>
                    lbl.starts_with(&format!("browser-pane-{}-", block_id))
                })
                .map(|(_, (win_lbl, _))| win_lbl.clone());

            if let Some(win_label) = parent_window_label {
                if let Some(main_browser) = state.get_browser(&win_label) {
                    if let Some(mut host) = main_browser.host() {
                        host.set_focus(1);
                        tracing::info!(
                            block_id = %block_id, window_label = %win_label,
                            "[browser-pane] macOS/Linux on_after_created: returned focus to main window"
                        );
                    }
                }
            }
        }
    }
}

/// Called from `AgentMuxHandler::on_before_close` after the browser has
/// been removed from `state.browsers` and the label has been identified
/// as a pane label (prefix `browser-pane-*`).
///
/// On Linux/macOS, runs the deferred `OverlayController::destroy()` for
/// any controller that `detach_browser_pane_view` stashed — see the long
/// comment on `state.pending_overlay_destroy` for why destroy can't run
/// synchronously with the close request. Drains the reducer entry next so
/// a re-create with the same block_id gets a fresh Live state. Idempotent
/// — if the explicit `close()` path already drained the reducer, the
/// drain is a no-op; if no controller was stashed (Windows or already
/// destroyed), the destroy step is a no-op.
pub fn on_before_close_browser_pane(state: &Arc<AppState>, label: &str) {
    // Step 1 (Linux/macOS): destroy the deferred OverlayController.
    // Safe now because the Browser is fully torn down and Chromium has
    // drained any queued tasks holding `WeakPtr<View>` to its BrowserView.
    #[cfg(not(target_os = "windows"))]
    {
        let stashed = state.pending_overlay_destroy.lock().remove(label);
        if let Some(controller) = stashed {
            controller.destroy();
            tracing::info!(
                label = %label,
                "[browser-pane] views: deferred OverlayController destroyed at on_before_close"
            );
        }
    }

    // Step 2: drain the reducer's pane entry (idempotent).
    state.browser_panes.drain_closed_label(state, label);

    // Labels are `browser-pane-<uuid>-<seq>`; strip prefix + trailing `-<seq>`
    // to recover the block_id.
    let block_id = label
        .strip_prefix("browser-pane-")
        .and_then(|rest| rest.rfind('-').map(|dash| &rest[..dash]));

    // Cross-platform: drop this pane's zoom-factor entry so
    // `browser_pane_zoom` doesn't grow unboundedly as panes are opened and
    // closed over a session. Same for the stashed context-menu frame
    // (SPEC_BROWSER_PANE_UNIFIED_CONTEXT_MENU_2026_08_15.md) — also holds a
    // cloned `Frame` that must not outlive the closing browser.
    if let Some(block_id) = block_id {
        state.browser_pane_zoom.lock().remove(block_id);
        state.browser_pane_context_menu_frame.lock().remove(block_id);
    }

    // Windows only:
    //   1. Restore WndProcs for all subclassed HWNDs (must run first, before
    //      remove_contexts_for_block wipes the outer-HWND lookup).
    //   2. Wipe BROWSER_PANE_HWND_CONTEXT entries for the block.
    #[cfg(target_os = "windows")]
    {
        if let Some(block_id) = block_id {
            crate::browser_pane::hwnd::uninstall_focus_redirect_for_block(block_id);
            crate::browser_pane::hwnd::remove_contexts_for_block(block_id);
        }
    }
}

/// Called from `AgentMuxHandler::on_load_end` when `is_browser_pane` is true.
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
pub fn on_load_end_browser_pane(state: &Arc<AppState>, browser: &Browser) {
    tracing::info!("[pane-load-end] pane page loaded; reinstalling focus subclass");
    if let Some(block_id) = resolve_pane_block_id(state, browser) {
        let url = {
            let mut b: cef::Browser = browser.clone();
            b.main_frame()
                .map(|f| cef::CefString::from(&cef::ImplFrame::url(&f)).to_string())
                .unwrap_or_default()
        };
        crate::browser_pane::trace::pane_trace(&block_id, "load-end", &format!("url={url}"));

        // Every navigation replaces the page's own DOM/inline-style state,
        // so any CSS `zoom` injected before this load is gone with it --
        // re-apply this pane's stored factor (no-op if it's never been
        // zoomed away from the 1.0 default). See BrowserPaneManager::
        // reapply_zoom's own doc comment for why this is CSS injection and
        // not Chromium's native per-host zoom.
        state.browser_panes.reapply_zoom(&block_id, state);
    }

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
    // dedicated `on_loading_state_change_browser_pane` callback below, which CEF
    // provides with correct values as direct parameters.
    if let Some(block_id) = resolve_pane_block_id(state, browser) {
        let url = {
            let mut b: cef::Browser = browser.clone();
            b.main_frame()
                .map(|f| cef::CefString::from(&cef::ImplFrame::url(&f)).to_string())
                .unwrap_or_default()
        };
        let block_id_short: String = block_id.chars().take(7).collect();
        tracing::info!(
            "[browser-pane:diag][{}] emit-nav-state url={:?} url_only=true",
            block_id_short, url,
        );
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
/// `AgentMuxHandler::on_loading_state_change` when `is_browser_pane == true`.
///
/// CEF invokes `on_loading_state_change` whenever the navigation controller's
/// history state changes — navigation start, navigation commit, and after
/// back/forward. `can_go_back` / `can_go_forward` are provided as direct
/// parameters (not queried after the fact), so they're guaranteed to reflect
/// the real committed state rather than the pre-commit race window. Same for
/// `is_loading` — CEF's actual navigation-controller loading state, forwarded
/// verbatim so the frontend's loading indicator reflects real top-level
/// navigations only, not client-side (SPA) route changes, which don't invoke
/// this callback. See SPEC_BROWSER_PANE_LOADING_BRAIN_INDICATOR_2026_07_11.md §4.1.
pub fn on_loading_state_change_browser_pane(
    state: &Arc<AppState>,
    browser: &Browser,
    is_loading: bool,
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
        let block_id_short: String = block_id.chars().take(7).collect();
        tracing::info!(
            "[browser-pane:diag][{}] emit-nav-state url={:?} url_only=false is_loading={} can_back={} can_forward={}",
            block_id_short, url, is_loading, can_go_back, can_go_forward,
        );
        crate::events::emit_event_from_state(
            state,
            "browser-pane-nav-state",
            &serde_json::json!({
                "block_id": block_id,
                "url": url,
                "can_go_back": can_go_back,
                "can_go_forward": can_go_forward,
                "is_loading": is_loading,
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
pub(crate) fn resolve_pane_block_id(state: &Arc<AppState>, browser: &Browser) -> Option<String> {
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
