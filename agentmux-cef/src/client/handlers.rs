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
            Some(AgentMuxKeyboardHandler::new(self.inner.clone()))
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

        fn context_menu_handler(&self) -> Option<ContextMenuHandler> {
            // Browser panes only — see SPEC_BROWSER_PANE_UNIFIED_CONTEXT_MENU_2026_08_15.md.
            // The main app client's own right-click menu is DOM-level
            // (app.tsx's onContextMenu={showTextInputContextMenu}) and needs
            // no CEF-level override.
            if !self.is_browser_pane {
                return None;
            }
            Some(AgentMuxContextMenuHandler::new(self.inner.clone()))
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

        fn permission_handler(&self) -> Option<PermissionHandler> {
            // Microphone (getUserMedia) access for voice input. ONLY the main app
            // client gets a handler — browser panes load arbitrary web content, so
            // auto-granting them media access would hand the mic to any site with no
            // prompt. For panes we return None → CEF's default Alloy handling (deny).
            if self.is_browser_pane {
                return None;
            }
            Some(AgentMuxPermissionHandler::new())
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
// renderer (used on Windows, where native draggable regions work).
//
// NOTE(Linux/macOS): On Linux/Wayland AND macOS we do NOT use
// -webkit-app-region: drag for window-move because Chromium suppresses ALL
// events on drag regions before they reach the renderer (verified
// empirically), making drag mutually exclusive with right-click contextmenu
// on the same element. Both drive drag from JS instead — see
// frontend/app/hook/useWindowDrag.{linux,darwin}.ts and the start_window_drag
// IPC (Linux → CefWindow::BeginWindowDrag(), CEF source patch in
// agentmux/7680-... branch; macOS → host-side run_macos_native_drag_loop).
// Retro: docs/retro/2026-05-02-drag-and-rightclick-coexistence.md.

wrap_drag_handler! {
    struct AgentMuxDragHandler {
        inner: Arc<Mutex<AgentMuxHandler>>,
    }

    impl DragHandler {
        // File drop path-capture. The HTML5 drop event in CEF/Chromium hides
        // full filesystem paths from JS (security model: pages can read `File`
        // objects but not their host paths). CefDragData::get_file_paths
        // exposes the real paths during OnDragEnter; we stash them so the
        // JS-side drop handler can consume them via the consume_drag_paths
        // IPC a few ms later. Returning 0 (don't cancel) lets the JS drop
        // event fire normally — required for the existing DragOverlay UI to
        // run unchanged. Spec: docs/specs/SPEC_PANE_FILE_DROP_2026_05_30.md §3.3.
        fn on_drag_enter(
            &self,
            _browser: Option<&mut Browser>,
            drag_data: Option<&mut DragData>,
            _mask: DragOperationsMask,
        ) -> ::std::os::raw::c_int {
            if let Some(dd) = drag_data {
                let mut list = CefStringList::new();
                let _ = dd.file_paths(Some(&mut list));
                let paths: Vec<String> = list.into_iter().filter(|p| !p.is_empty()).collect();
                if !paths.is_empty() {
                    tracing::info!("[drag] captured {} file path(s) via OnDragEnter", paths.len());
                    crate::drag_stash::put(paths);
                }
            }
            0
        }

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

/// CEF event flag: Shift key is held (cef_types.h `EVENTFLAG_SHIFT_DOWN`).
const EVENTFLAG_SHIFT_DOWN: u32 = 1 << 1;
/// CEF event flag: Ctrl key is held.
const EVENTFLAG_CONTROL_DOWN: u32 = 1 << 2;
/// CEF event flag: Alt key is held (cef_types.h `EVENTFLAG_ALT_DOWN`).
const EVENTFLAG_ALT_DOWN: u32 = 1 << 3;
/// CEF event flag: Cmd key is held (cef_types.h `EVENTFLAG_COMMAND_DOWN`) —
/// macOS's Ctrl-equivalent modifier for the browser-pane shortcuts below
/// (issue #1190's acceptance criteria: "same set with Cmd on macOS"). Not
/// live-verified on macOS hardware — same caveat as issues #2188/#2189.
#[cfg(target_os = "macos")]
const EVENTFLAG_COMMAND_DOWN: u32 = 1 << 7;

/// Windows virtual-key codes for shortcuts we want to forward to JS.
const VK_P: i32 = 0x50; // Ctrl+P — command palette (not print)
const VK_G: i32 = 0x47; // Ctrl+G — (reserve for app use)

/// Windows virtual-key codes for browser-pane-only shortcuts (issue #1190).
const VK_L: i32 = 0x4C; // Ctrl+L — focus address bar
const VK_R: i32 = 0x52; // Ctrl+R — reload
const VK_LEFT: i32 = 0x25; // Alt+Left — back
const VK_RIGHT: i32 = 0x27; // Alt+Right — forward

/// Reserved chrome-style shortcuts intercepted only when the focused CEF
/// browser is a browser pane (see issue #1190 — browser panes are native
/// CEF child windows, so normal app-level keydown handling never sees these
/// keystrokes). Kept independent of any CEF/AppState types so the matching
/// logic itself is unit-testable without a CEF runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BrowserPaneShortcut {
    FocusAddress,
    Reload,
    GoBack,
    GoForward,
}

/// Ctrl+T/Ctrl+W (new tab / close tab) are intentionally not matched here —
/// they depend on browser-pane tabs, which don't exist yet (see the issue's
/// own sequencing note). Ctrl+F (in-page find) is also deferred: it needs a
/// find-bar UI (input + match-count display), not just an intercepted key.
///
/// `shift` must be false to match: codex P2 on #2548 — without this,
/// Ctrl+Shift+R (Chromium's hard-reload chord) was silently downgraded to a
/// plain cached reload, and Ctrl+Shift+L was hijacked as focus-address too.
/// Requiring shift-up leaves those shifted chords alone (falls through to
/// CEF's normal handling) instead of guessing what they should do.
fn browser_pane_shortcut_for(ctrl: bool, alt: bool, shift: bool, vk: i32) -> Option<BrowserPaneShortcut> {
    if shift {
        return None;
    }
    if ctrl && !alt {
        match vk {
            VK_L => Some(BrowserPaneShortcut::FocusAddress),
            VK_R => Some(BrowserPaneShortcut::Reload),
            _ => None,
        }
    } else if alt && !ctrl {
        match vk {
            VK_LEFT => Some(BrowserPaneShortcut::GoBack),
            VK_RIGHT => Some(BrowserPaneShortcut::GoForward),
            _ => None,
        }
    } else {
        None
    }
}

/// Resolve the focused pane's `block_id` and run `shortcut` against it —
/// either by emitting an event the frontend reacts to (`FocusAddress`, which
/// needs to move DOM focus in the host webview) or by calling straight into
/// `BrowserPaneManager` (`Reload`/`GoBack`/`GoForward`, the same methods the
/// nav-bar buttons already use via IPC — see `ipc.rs`'s `browser_pane_go_back`
/// etc.), so this stays consistent with the existing back/forward/reload path
/// instead of poking CEF's `Browser` directly.
fn run_browser_pane_shortcut(
    inner: &Arc<Mutex<AgentMuxHandler>>,
    browser: Option<&mut Browser>,
    shortcut: BrowserPaneShortcut,
) -> bool {
    let handler = inner.lock();
    if !handler.is_browser_pane {
        return false;
    }
    let Some(b) = browser else { return false };
    let Some(block_id) = crate::browser_pane::callbacks::resolve_pane_block_id(&handler.state, b)
    else {
        return false;
    };
    let state = handler.state.clone();
    drop(handler);

    match shortcut {
        BrowserPaneShortcut::FocusAddress => {
            // codex P2 on #2548: emit_event_from_state always targets "main" —
            // wrong when the pane was torn off into a floating window, since
            // that window's own BrowserNavBar (not main's) owns the matching
            // listener. Route to the pane's actual owning window instead.
            match state.browser_pane_window_label(&block_id) {
                Some(label) => {
                    crate::events::emit_event_to_window(
                        &state,
                        &label,
                        "browser-pane-shortcut",
                        &serde_json::json!({ "block_id": block_id, "action": "focus-address" }),
                    );
                }
                None => {
                    tracing::warn!(
                        "[browser-pane-shortcut] no owning window label for block_id={}",
                        block_id
                    );
                }
            }
        }
        BrowserPaneShortcut::Reload => state.browser_panes.reload(&block_id, &state),
        BrowserPaneShortcut::GoBack => state.browser_panes.go_back(&block_id, &state),
        BrowserPaneShortcut::GoForward => state.browser_panes.go_forward(&block_id, &state),
    }
    true
}

// cef-rs declares `on_pre_key_event`'s `os_event` differently per platform:
//   Windows → Option<&mut MSG>     (typed)
//   Linux   → Option<&mut XEvent>  (typed)
//   macOS   → *mut u8              (raw)
// A single signature can't satisfy all three, so we expand the macro per
// target and forward to a shared body.
fn handle_pre_key_event(
    inner: &Arc<Mutex<AgentMuxHandler>>,
    browser: Option<&mut Browser>,
    event: Option<&KeyEvent>,
    is_keyboard_shortcut: Option<&mut ::std::os::raw::c_int>,
) -> ::std::os::raw::c_int {
    let Some(ev) = event else { return 0 };
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
        return 0;
    }

    // Browser-pane shortcuts (issue #1190) are handled entirely here — unlike
    // Ctrl+P/Ctrl+G above, they must NOT reach the pane's own page JS (an
    // arbitrary, possibly untrusted site), so we fully consume them (return 1)
    // instead of forwarding. Gated on RAWKEYDOWN so the action fires once per
    // physical key press — CEF also calls `on_pre_key_event` for the matching
    // KEYUP, which would otherwise double-fire (e.g. two reloads per press).
    if ev.type_ == KeyEventType::RAWKEYDOWN {
        let alt = (ev.modifiers & EVENTFLAG_ALT_DOWN) != 0;
        let shift = (ev.modifiers & EVENTFLAG_SHIFT_DOWN) != 0;
        #[cfg(target_os = "macos")]
        let pane_primary_mod = (ev.modifiers & EVENTFLAG_COMMAND_DOWN) != 0;
        #[cfg(not(target_os = "macos"))]
        let pane_primary_mod = ctrl;
        if let Some(shortcut) = browser_pane_shortcut_for(pane_primary_mod, alt, shift, ev.windows_key_code) {
            if run_browser_pane_shortcut(inner, browser, shortcut) {
                return 1;
            }
        }
    }

    0 // not consumed
}

#[cfg(target_os = "windows")]
wrap_keyboard_handler! {
    struct AgentMuxKeyboardHandler {
        inner: Arc<Mutex<AgentMuxHandler>>,
    }

    impl KeyboardHandler {
        fn on_pre_key_event(
            &self,
            browser: Option<&mut Browser>,
            event: Option<&KeyEvent>,
            _os_event: Option<&mut cef::sys::MSG>,
            is_keyboard_shortcut: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            handle_pre_key_event(&self.inner, browser, event, is_keyboard_shortcut)
        }
    }
}

#[cfg(target_os = "linux")]
wrap_keyboard_handler! {
    struct AgentMuxKeyboardHandler {
        inner: Arc<Mutex<AgentMuxHandler>>,
    }

    impl KeyboardHandler {
        fn on_pre_key_event(
            &self,
            browser: Option<&mut Browser>,
            event: Option<&KeyEvent>,
            _os_event: Option<&mut cef::sys::XEvent>,
            is_keyboard_shortcut: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            handle_pre_key_event(&self.inner, browser, event, is_keyboard_shortcut)
        }
    }
}

#[cfg(target_os = "macos")]
wrap_keyboard_handler! {
    struct AgentMuxKeyboardHandler {
        inner: Arc<Mutex<AgentMuxHandler>>,
    }

    impl KeyboardHandler {
        fn on_pre_key_event(
            &self,
            browser: Option<&mut Browser>,
            event: Option<&KeyEvent>,
            _os_event: *mut u8,
            is_keyboard_shortcut: Option<&mut ::std::os::raw::c_int>,
        ) -> ::std::os::raw::c_int {
            handle_pre_key_event(&self.inner, browser, event, is_keyboard_shortcut)
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

        fn on_favicon_urlchange(
            &self,
            browser: Option<&mut Browser>,
            icon_urls: Option<&mut CefStringList>,
        ) {
            let mut inner = self.inner.lock();
            inner.on_favicon_urlchange(browser, icon_urls);
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

        fn on_load_start(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            transition_type: TransitionType,
        ) {
            let mut inner = self.inner.lock();
            inner.on_load_start(browser, frame, transition_type);
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
// RequestHandler — render-process termination, external-protocol guard, auth
// ---------------------------------------------------------------------------
//
// Overrides here: `on_before_browse` (external-protocol / OS-handoff guard for
// browser panes), `on_render_process_terminated` (white-screen recovery), and
// `auth_credentials` (HTTP Basic/Digest → BrowserAuthModal). Everything else
// inherits the default (no-op) implementations from the cef-rs trait.
// See SPEC_GRACEFUL_CRASH_HANDLING_2026_04_13.md (PR 1).

wrap_request_handler! {
    struct AgentMuxRequestHandler {
        inner: Arc<Mutex<AgentMuxHandler>>,
    }

    impl RequestHandler {
        // External-protocol / OS-handoff guard. See
        // AgentMuxHandler::on_before_browse — cancels browser-pane navigations
        // to non-web schemes so embedded content can't reach an OS protocol
        // handler (and, on Windows, a UAC prompt).
        fn on_before_browse(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            request: Option<&mut Request>,
            user_gesture: ::std::os::raw::c_int,
            is_redirect: ::std::os::raw::c_int,
        ) -> ::std::os::raw::c_int {
            let mut inner = self.inner.lock();
            inner.on_before_browse(browser, frame, request, user_gesture, is_redirect)
        }

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

        // HTTP Basic / Digest auth challenge. Phase α of
        // SPEC_BROWSER_PANE_HTTP_BASIC_AUTH_2026_05_18.md. Returns 1
        // (async) so CEF holds the request open while we surface the
        // credential prompt to the user.
        fn auth_credentials(
            &self,
            browser: Option<&mut Browser>,
            origin_url: Option<&CefString>,
            is_proxy: ::std::os::raw::c_int,
            host: Option<&CefString>,
            port: ::std::os::raw::c_int,
            realm: Option<&CefString>,
            scheme: Option<&CefString>,
            callback: Option<&mut AuthCallback>,
        ) -> ::std::os::raw::c_int {
            let mut inner = self.inner.lock();
            inner.on_auth_credentials(
                browser, origin_url, is_proxy, host, port, realm, scheme, callback,
            )
        }
    }
}

// ---------------------------------------------------------------------------
// ContextMenuHandler — browser panes only: suppress CEF's native menu
// ---------------------------------------------------------------------------
//
// See client::context_menu::AgentMuxHandler::run_context_menu and
// docs/specs/SPEC_BROWSER_PANE_UNIFIED_CONTEXT_MENU_2026_08_15.md. Only
// `run_context_menu` is overridden; `on_before_context_menu` /
// `on_context_menu_command` / `on_context_menu_dismissed` keep their default
// (no-op) implementations — nothing to do there since we never let CEF's
// menu model get used at all.

wrap_context_menu_handler! {
    struct AgentMuxContextMenuHandler {
        inner: Arc<Mutex<AgentMuxHandler>>,
    }

    impl ContextMenuHandler {
        fn run_context_menu(
            &self,
            browser: Option<&mut Browser>,
            frame: Option<&mut Frame>,
            params: Option<&mut ContextMenuParams>,
            _model: Option<&mut MenuModel>,
            callback: Option<&mut RunContextMenuCallback>,
        ) -> ::std::os::raw::c_int {
            let mut inner = self.inner.lock();
            inner.run_context_menu(browser, frame, params, callback)
        }
    }
}

// ---------------------------------------------------------------------------
// PermissionHandler — microphone (getUserMedia) access for voice input
// ---------------------------------------------------------------------------
//
// AgentMux runs the Alloy runtime for all non-devtools windows (app.rs:
// RuntimeStyle::ALLOY). Under Alloy, CEF's DEFAULT handling of a media-access
// request is to DENY — so without this handler every getUserMedia({audio:true})
// (the Web Speech API today, and the MediaRecorder capture for server-side STT
// in #1591) is rejected with NotAllowedError, surfacing as the "Voice input
// unavailable" toast.
//
// We grant only the AUDIO-capture bits the page requested and implicitly deny
// everything else (camera, desktop capture). Per the CEF contract, for a
// getUserMedia request `allowed_permissions` must match `required_permissions`:
// granting only the audio subset means a combined audio+video request is denied
// as a whole (we never want camera), while an audio-only request — AgentMux's
// only case — is granted exactly. Only ever installed on the main app client
// (see `permission_handler` getter above); browser panes never reach here.
//
// OS-level permission (Windows Privacy / macOS TCC) is a SEPARATE layer handled
// in the frontend via getUserMedia error classification + a settings deep-link.
// See SPEC_VOICE_INPUT_PER_PANE_2026_05_19.md §Phase 4 and #1591.

// cef_media_access_permission_types_t bitmask values (ABI-stable in CEF).
const CEF_MEDIA_PERMISSION_DEVICE_AUDIO_CAPTURE: u32 = 1 << 0;
const CEF_MEDIA_PERMISSION_DESKTOP_AUDIO_CAPTURE: u32 = 1 << 2;

wrap_permission_handler! {
    struct AgentMuxPermissionHandler;

    impl PermissionHandler {
        fn on_request_media_access_permission(
            &self,
            _browser: Option<&mut Browser>,
            _frame: Option<&mut Frame>,
            requesting_origin: Option<&CefString>,
            requested_permissions: u32,
            callback: Option<&mut MediaAccessCallback>,
        ) -> ::std::os::raw::c_int {
            let audio_bits = CEF_MEDIA_PERMISSION_DEVICE_AUDIO_CAPTURE
                | CEF_MEDIA_PERMISSION_DESKTOP_AUDIO_CAPTURE;
            let allowed = requested_permissions & audio_bits;
            let origin = requesting_origin.map(|s| s.to_string()).unwrap_or_default();
            match callback {
                Some(cb) => {
                    tracing::info!(
                        target: "voice",
                        %origin,
                        requested = requested_permissions,
                        allowed,
                        "granting audio media-access permission"
                    );
                    cb.cont(allowed);
                    1 // handled
                }
                None => {
                    tracing::warn!(
                        target: "voice",
                        %origin,
                        "media-access request had no callback — deferring to default"
                    );
                    0 // not handled — CEF default (deny under Alloy)
                }
            }
        }
    }
}

#[cfg(test)]
mod browser_pane_shortcut_tests {
    use super::*;

    #[test]
    fn ctrl_l_focuses_address_bar() {
        assert_eq!(
            browser_pane_shortcut_for(true, false, false, VK_L),
            Some(BrowserPaneShortcut::FocusAddress)
        );
    }

    #[test]
    fn ctrl_r_reloads() {
        assert_eq!(
            browser_pane_shortcut_for(true, false, false, VK_R),
            Some(BrowserPaneShortcut::Reload)
        );
    }

    #[test]
    fn alt_left_goes_back() {
        assert_eq!(
            browser_pane_shortcut_for(false, true, false, VK_LEFT),
            Some(BrowserPaneShortcut::GoBack)
        );
    }

    #[test]
    fn alt_right_goes_forward() {
        assert_eq!(
            browser_pane_shortcut_for(false, true, false, VK_RIGHT),
            Some(BrowserPaneShortcut::GoForward)
        );
    }

    #[test]
    fn plain_keypress_without_modifier_is_not_a_shortcut() {
        assert_eq!(browser_pane_shortcut_for(false, false, false, VK_L), None);
        assert_eq!(browser_pane_shortcut_for(false, false, false, VK_LEFT), None);
    }

    #[test]
    fn ctrl_alt_combo_matches_neither_group() {
        // Ctrl+Alt+L / Ctrl+Alt+Left are not reserved shortcuts — only plain
        // Ctrl+<key> and plain Alt+<key> are matched, so a page/site binding
        // Ctrl+Alt+<key> for its own purposes isn't hijacked.
        assert_eq!(browser_pane_shortcut_for(true, true, false, VK_L), None);
        assert_eq!(browser_pane_shortcut_for(true, true, false, VK_LEFT), None);
    }

    #[test]
    fn unrelated_key_is_not_a_shortcut() {
        assert_eq!(browser_pane_shortcut_for(true, false, false, VK_P), None);
    }

    #[test]
    fn ctrl_t_and_ctrl_w_are_not_yet_matched() {
        // Tabs (#1190's Ctrl+T/Ctrl+W) depend on browser-pane tabs, which
        // don't exist yet — asserting the non-match documents that this is
        // deliberate, not an oversight, until that dependency lands.
        const VK_T: i32 = 0x54;
        const VK_W: i32 = 0x57;
        assert_eq!(browser_pane_shortcut_for(true, false, false, VK_T), None);
        assert_eq!(browser_pane_shortcut_for(true, false, false, VK_W), None);
    }

    #[test]
    fn shift_held_excludes_plain_shortcuts() {
        // codex P2 on #2548: Ctrl+Shift+R is Chromium's hard-reload chord and
        // Ctrl+Shift+L isn't a shortcut at all — neither should be silently
        // downgraded/hijacked into the plain Ctrl+R/Ctrl+L actions.
        assert_eq!(browser_pane_shortcut_for(true, false, true, VK_R), None);
        assert_eq!(browser_pane_shortcut_for(true, false, true, VK_L), None);
        assert_eq!(browser_pane_shortcut_for(false, true, true, VK_LEFT), None);
        assert_eq!(browser_pane_shortcut_for(false, true, true, VK_RIGHT), None);
    }
}
