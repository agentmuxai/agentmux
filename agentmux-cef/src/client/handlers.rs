// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Cef handler trait wrappers for AgentMuxHandler. Extracted from
//! client/mod.rs in task #182 PR-G.
//!
//! Each block is a macro invocation that generates a small wrapper
//! struct delegating to AgentMuxHandler methods.

use std::sync::Arc;
use cef::*;
use parking_lot::Mutex;

use super::AgentMuxHandler;
use super::dlog;

// ---------------------------------------------------------------------------

wrap_client! {
    pub struct AgentMuxClient {
        inner: Arc<Mutex<AgentMuxHandler>>,
        is_browser_pane: bool,
    }

    impl Client {
        fn display_handler(&self) -> Option<DisplayHandler> {
            Some(AgentMuxDisplayHandler::new(self.inner.clone()))
        }

        fn keyboard_handler(&self) -> Option<KeyboardHandler> {
            Some(AgentMuxKeyboardHandler::new())
        }

        fn life_span_handler(&self) -> Option<LifeSpanHandler> {
            Some(AgentMuxLifeSpanHandler::new(self.inner.clone()))
        }

        fn load_handler(&self) -> Option<LoadHandler> {
            Some(AgentMuxLoadHandler::new(self.inner.clone()))
        }

        fn request_handler(&self) -> Option<RequestHandler> {
            Some(AgentMuxRequestHandler::new(self.inner.clone()))
        }

        fn drag_handler(&self) -> Option<DragHandler> {
            if self.is_browser_pane {
                return None;
            }
            Some(AgentMuxDragHandler::new(self.inner.clone()))
        }

        fn focus_handler(&self) -> Option<FocusHandler> {
            // For browser panes only: cancel CEF's auto-focus on navigation so the
            // child HWND doesn't steal keyboard focus from the main window when the
            // page finishes loading. The user can still click into the pane to focus it.
            if self.is_browser_pane {
                Some(AgentMuxPaneFocusHandler::new())
            } else {
                None
            }
        }
    }
}

// FocusHandler used only by browser-pane clients. Returns 0 for every
// focus source (never cancels at the CEF level) — cancelling NAVIGATION
// focus during the very first navigation of a newly-created pane fires
// CEF's `on_before_close` on that pane ~10ms later. Focus-steal
// protection lives entirely in the Win32 `WndProc` subclass below
// (`browser_pane::hwnd::install_browser_pane_focus_redirect`), which redirects programmatic
// `WM_SETFOCUS` back to the top-level window. User clicks are let through
// because `WM_LBUTTONDOWN` in the subclass arms `ALLOW_BROWSER_PANE_FOCUS_ONCE`.
wrap_focus_handler! {
    struct AgentMuxPaneFocusHandler;

    impl FocusHandler {
        fn on_set_focus(
            &self,
            _browser: Option<&mut Browser>,
            source: FocusSource,
        ) -> ::std::os::raw::c_int {
            // Previously we cancelled FocusSource::NAVIGATION here to
            // stop page-load from stealing focus away from the main
            // window. But cancelling on_set_focus during the very
            // first navigation of a newly-created pane triggered CEF
            // to fire `on_before_close` on that pane ~10ms later —
            // reliably reproducible when creating a 2nd browser pane.
            // The Win32 WndProc subclass below already redirects
            // page-load SetFocus to the top-level window (see
            // `browser_pane::hwnd::install_browser_pane_focus_redirect`), which
            // handles the original focus-steal concern. Returning 0
            // here so CEF proceeds with normal focus handling at the
            // Chromium level; Win32 subclass continues to redirect
            // any resulting Win32 focus change away from the pane.
            tracing::info!("[pane-focus] on_set_focus source={:?} cancel=false", source);
            0
        }
    }
}

// ---------------------------------------------------------------------------
// DragHandler — handles `-webkit-app-region: drag` regions reported by the
// renderer (used on macOS/Windows where native draggable regions work).
//
// NOTE(Linux): On Linux/Wayland we do NOT use -webkit-app-region: drag for
// window-move because Chromium suppresses ALL events on drag regions before
// they reach the renderer (verified empirically), making drag mutually
// exclusive with right-click contextmenu on the same element. Linux drag is
// JS-driven instead — see frontend/app/hook/useWindowDrag.linux.ts and
// the start_window_drag IPC → CefWindow::BeginWindowDrag() (CEF source
// patch in agentmux/7680-... branch). Retro:
// docs/retros/2026-05-02-drag-and-rightclick-coexistence.md.

wrap_drag_handler! {
    struct AgentMuxDragHandler {
        inner: Arc<Mutex<AgentMuxHandler>>,
    }

    impl DragHandler {
        fn on_draggable_regions_changed(
            &self,
            browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            regions: Option<&[DraggableRegion]>,
        ) {
            if let Some(rs) = regions {
                let summary: Vec<String> = rs.iter().map(|r| {
                    format!("{}x{}@{},{} drag={}", r.bounds.width, r.bounds.height, r.bounds.x, r.bounds.y, r.draggable != 0)
                }).collect();
                tracing::info!("[drag_handler] on_draggable_regions_changed: {} regions — {:?}", rs.len(), summary);
            } else {
                tracing::info!("[drag_handler] on_draggable_regions_changed: None");
            }
            let mut browser = browser.cloned();
            let Some(browser_view) = browser_view_get_for_browser(browser.as_mut()) else { return };
            let Some(window) = browser_view.window() else { return };
            window.set_draggable_regions(regions);
        }
    }
}

// KeyboardHandler — intercept Ctrl+<key> shortcuts before CEF/Chromium
// consumes them (e.g., Ctrl+P = print, Ctrl+G = find-next).
// Returning true from on_pre_key_event tells CEF "handled" so it won't
// trigger the built-in action; the key still reaches JavaScript.
// ---------------------------------------------------------------------------

/// CEF event flag: Ctrl key is held.
const EVENTFLAG_CONTROL_DOWN: u32 = 1 << 2;

/// Windows virtual-key codes for shortcuts we want to forward to JS.
const VK_P: i32 = 0x50; // Ctrl+P — command palette (not print)
const VK_G: i32 = 0x47; // Ctrl+G — (reserve for app use)

wrap_keyboard_handler! {
    struct AgentMuxKeyboardHandler;

    impl KeyboardHandler {
        fn on_pre_key_event(
            &self,
            _browser: Option<&mut Browser>,
            event: Option<&KeyEvent>,
            _os_event: Option<&mut crate::OsKeyEvent>,
            is_keyboard_shortcut: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            if let Some(ev) = event {
                let ctrl = (ev.modifiers & EVENTFLAG_CONTROL_DOWN) != 0;
                if ctrl && matches!(ev.windows_key_code, VK_P | VK_G) {
                    // Tell CEF this is a keyboard shortcut so it dispatches
                    // the keydown event to JavaScript instead of handling it
                    // as a built-in browser action (print dialog, etc.).
                    if let Some(flag) = is_keyboard_shortcut {
                        *flag = 1;
                    }
                    // Return 0 = not consumed at pre-key stage; CEF will
                    // still call on_key_event where we return 0 again,
                    // letting JS handle it via the normal keydown path.
                }
            }
            0 // not consumed
        }
    }
}

// ---------------------------------------------------------------------------
// DisplayHandler — title changes
// ---------------------------------------------------------------------------

wrap_display_handler! {
    struct AgentMuxDisplayHandler {
        inner: Arc<Mutex<AgentMuxHandler>>,
    }

    impl DisplayHandler {
        fn on_title_change(&self, browser: Option<&mut Browser>, title: Option<&CefString>) {
            let mut inner = self.inner.lock();
            inner.on_title_change(browser, title);
        }
    }
}

// ---------------------------------------------------------------------------
// LifeSpanHandler — browser creation/destruction
// ---------------------------------------------------------------------------

wrap_life_span_handler! {
    struct AgentMuxLifeSpanHandler {
        inner: Arc<Mutex<AgentMuxHandler>>,
    }

    impl LifeSpanHandler {
        fn on_after_created(&self, browser: Option<&mut Browser>) {
            let mut inner = self.inner.lock();
            inner.on_after_created(browser);
        }

        fn do_close(&self, browser: Option<&mut Browser>) -> i32 {
            let mut inner = self.inner.lock();
            inner.do_close(browser).into()
        }

        fn on_before_close(&self, browser: Option<&mut Browser>) {
            let mut inner = self.inner.lock();
            inner.on_before_close(browser);
        }

        fn on_before_popup(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            _popup_id: ::std::os::raw::c_int,
            target_url: Option<&CefString>,
            _target_frame_name: Option<&CefString>,
            target_disposition: WindowOpenDisposition,
            _user_gesture: ::std::os::raw::c_int,
            _popup_features: Option<&PopupFeatures>,
            _window_info: Option<&mut WindowInfo>,
            _client: Option<&mut Option<Client>>,
            _settings: Option<&mut BrowserSettings>,
            _extra_info: Option<&mut Option<DictionaryValue>>,
            _no_javascript_access: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            let mut inner = self.inner.lock();
            if inner.on_before_popup(browser, frame, target_url, target_disposition) {
                1
            } else {
                0
            }
        }
    }
}

// ---------------------------------------------------------------------------
// LoadHandler — load events and errors
// ---------------------------------------------------------------------------

wrap_load_handler! {
    struct AgentMuxLoadHandler {
        inner: Arc<Mutex<AgentMuxHandler>>,
    }

    impl LoadHandler {
        fn on_loading_state_change(
            &self,
            browser: Option<&mut Browser>,
            is_loading: ::std::os::raw::c_int,
            can_go_back: ::std::os::raw::c_int,
            can_go_forward: ::std::os::raw::c_int,
        ) {
            let mut inner = self.inner.lock();
            inner.on_loading_state_change(browser, is_loading, can_go_back, can_go_forward);
        }

        fn on_load_end(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            http_status_code: i32,
        ) {
            let mut inner = self.inner.lock();
            inner.on_load_end(browser, frame, http_status_code);
        }

        fn on_load_error(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            error_code: Errorcode,
            error_text: Option<&CefString>,
            failed_url: Option<&CefString>,
        ) {
            let mut inner = self.inner.lock();
            inner.on_load_error(browser, frame, error_code, error_text, failed_url);
        }
    }
}

// ---------------------------------------------------------------------------
// RequestHandler — render-process termination (white-screen recovery)
// ---------------------------------------------------------------------------
//
// We only override `on_render_process_terminated` here. Everything else
// inherits the default (no-op) implementations from the cef-rs trait.
// See SPEC_GRACEFUL_CRASH_HANDLING_2026_04_13.md (PR 1).

wrap_request_handler! {
    struct AgentMuxRequestHandler {
        inner: Arc<Mutex<AgentMuxHandler>>,
    }

    impl RequestHandler {
        fn on_render_process_terminated(
            &self,
            browser: Option<&mut Browser>,
            status: TerminationStatus,
            error_code: ::std::os::raw::c_int,
            error_string: Option<&CefString>,
        ) {
            let mut inner = self.inner.lock();
            inner.on_render_process_terminated(browser, status, error_code, error_string);
        }
    }
}
