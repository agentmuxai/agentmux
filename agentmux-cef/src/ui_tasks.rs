// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// CEF UI thread task dispatch.
//
// All CEF Views operations (Window::close, minimize, maximize, etc.) must run
// on the CEF UI thread. IPC commands arrive on tokio threads. This module
// provides tasks that can be posted to the UI thread via post_task().
//
// Key insight: don't pass Browser/Window handles across threads. Instead,
// pass Arc<AppState> and look up the browser on the UI thread.
//
// Used on Linux (and macOS). On Windows, Win32 APIs are used directly since
// they are safe to call from any thread.

use std::sync::Arc;
use cef::*;
use crate::state::AppState;

/// Get the CEF Views Window for a browser label on the UI thread.
fn get_window_on_ui(state: &Arc<AppState>, label: &str) -> Option<Window> {
    let browsers = state.browsers.lock();
    let mut browser = browsers.get(label)?.clone();
    drop(browsers);
    let browser_view = browser_view_get_for_browser(Some(&mut browser))?;
    browser_view.window()
}

// ── Deferred load_url (used by on_before_popup to avoid UI-thread deadlock)
//
// Calling `frame.load_url(url)` synchronously inside a CEF callback that
// holds the handler's inner lock (e.g. `on_before_popup`) deadlocks on
// link clicks: `load_url` kicks a new navigation which triggers
// `on_loading_state_change` on the same thread, which also wants the
// handler's lock. Posting the navigate as a separate UI task lets the
// original callback return, release its lock, and the load starts
// cleanly on the next message-loop turn. ─────────────────────────────────

wrap_task! {
    pub struct DeferredLoadUrlTask {
        browser: Browser,
        url: String,
    }

    impl Task {
        fn execute(&self) {
            let mut browser = self.browser.clone();
            if let Some(frame) = browser.main_frame() {
                frame.load_url(Some(&CefString::from(self.url.as_str())));
            }
        }
    }
}

// ── Close ────────────────────────────────────────────────────────────────

wrap_task! {
    pub struct CloseWindowTask {
        state: Arc<AppState>,
        label: String,
    }

    impl Task {
        fn execute(&self) {
            if let Some(window) = get_window_on_ui(&self.state, &self.label) {
                window.close();
            }
        }
    }
}

pub fn post_close_window(state: &Arc<AppState>, label: &str) {
    let mut task = CloseWindowTask::new(state.clone(), label.to_string());
    post_task(ThreadId::UI, Some(&mut task));
}

// ── Minimize ─────────────────────────────────────────────────────────────

wrap_task! {
    pub struct MinimizeWindowTask {
        state: Arc<AppState>,
        label: String,
    }

    impl Task {
        fn execute(&self) {
            if let Some(window) = get_window_on_ui(&self.state, &self.label) {
                window.minimize();
            }
        }
    }
}

pub fn post_minimize_window(state: &Arc<AppState>, label: &str) {
    let mut task = MinimizeWindowTask::new(state.clone(), label.to_string());
    post_task(ThreadId::UI, Some(&mut task));
}

// ── Maximize (toggle) ────────────────────────────────────────────────────

wrap_task! {
    pub struct MaximizeWindowTask {
        state: Arc<AppState>,
        label: String,
    }

    impl Task {
        fn execute(&self) {
            if let Some(window) = get_window_on_ui(&self.state, &self.label) {
                if window.is_maximized() != 0 {
                    window.restore();
                } else {
                    window.maximize();
                }
            }
        }
    }
}

pub fn post_maximize_window(state: &Arc<AppState>, label: &str) {
    let mut task = MaximizeWindowTask::new(state.clone(), label.to_string());
    post_task(ThreadId::UI, Some(&mut task));
}

// ── Focus/Activate ───────────────────────────────────────────────────────

wrap_task! {
    pub struct FocusWindowTask {
        state: Arc<AppState>,
        label: String,
    }

    impl Task {
        fn execute(&self) {
            if let Some(window) = get_window_on_ui(&self.state, &self.label) {
                window.activate();
            }
        }
    }
}

pub fn post_focus_window(state: &Arc<AppState>, label: &str) {
    let mut task = FocusWindowTask::new(state.clone(), label.to_string());
    post_task(ThreadId::UI, Some(&mut task));
}

// ── Drag ─────────────────────────────────────────────────────────────────
// CEF Views does not expose a programmatic drag-initiation API.
// Window dragging on Linux/macOS uses the WindowDelegate draggable regions.
// TODO: implement via X11 _NET_WM_MOVERESIZE for programmatic drag.
pub fn post_start_drag(_state: &Arc<AppState>, _label: &str) {}

// ── Move window ───────────────────────────────────────────────────────────

wrap_task! {
    pub struct MoveWindowTask {
        state: Arc<AppState>,
        label: String,
        dx: i32,
        dy: i32,
    }

    impl Task {
        fn execute(&self) {
            if let Some(window) = get_window_on_ui(&self.state, &self.label) {
                let bounds = window.bounds();
                window.set_bounds(Some(&Rect {
                    x: bounds.x + self.dx,
                    y: bounds.y + self.dy,
                    width: bounds.width,
                    height: bounds.height,
                }));
            }
        }
    }
}

pub fn post_move_window(state: &Arc<AppState>, label: &str, dx: i32, dy: i32) {
    let mut task = MoveWindowTask::new(state.clone(), label.to_string(), dx, dy);
    post_task(ThreadId::UI, Some(&mut task));
}

// ── Phase B.9.2 (WRR) — corrective absolute-position move ─────────────────
//
// Reducer-driven self-heal. Triggered by `Event::CorrectiveWindowMove` when
// the reducer detects an off-monitor / sentinel-parked window that the user
// has never foregrounded. We bypass `state.browsers` lookup-by-label (the
// label might not be registered yet at correction time) and use Win32
// SetWindowPos directly against the HWND. Must run on the UI thread because
// CEF Views' window backing the HWND is owned by the UI thread.

wrap_task! {
    pub struct CorrectiveWindowMoveTask {
        state: Arc<AppState>,
        hwnd: u64,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    }

    impl Task {
        fn execute(&self) {
            #[cfg(target_os = "windows")]
            unsafe {
                use windows_sys::Win32::Foundation::HWND;
                use windows_sys::Win32::UI::WindowsAndMessaging::{
                    SetWindowPos, SWP_NOACTIVATE, SWP_NOZORDER,
                };
                let h = self.hwnd as HWND;
                let ok = SetWindowPos(
                    h,
                    std::ptr::null_mut(),
                    self.x,
                    self.y,
                    self.w,
                    self.h,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                tracing::info!(
                    target: "wrr",
                    "[wrr] corrective SetWindowPos hwnd={:#x} -> ({},{}) {}x{} ok={}",
                    self.hwnd, self.x, self.y, self.w, self.h, ok != 0
                );
            }
            #[cfg(not(target_os = "windows"))]
            {
                let _ = &self.state; // suppress unused on non-Windows
                tracing::warn!(
                    target: "wrr",
                    "[wrr] corrective move requested on non-Windows host: ignored"
                );
            }
        }
    }
}

pub fn post_corrective_window_move(state: &Arc<AppState>, hwnd: u64, x: i32, y: i32, w: i32, h: i32) {
    let mut task = CorrectiveWindowMoveTask::new(state.clone(), hwnd, x, y, w, h);
    post_task(ThreadId::UI, Some(&mut task));
}

// Phase B.9.3 (WRR) — `Event::HostShouldQuit` handling lives in
// `launcher_ipc::apply_event_to_shadow`. After three smoke
// iterations (v0.33.491–v0.33.493) confirmed `cef::post_task`
// silently drops new tasks during the last-window-closed
// teardown window — even when previously-posted tasks still
// run — we bypass CEF entirely and use Win32
// `PostThreadMessage(host_main_tid, WM_QUIT, 0, 0)` via
// `wrr::win_event::post_thread_quit_message`. The UI thread's
// captured TID is stored at `install_hooks` time.

// ── Create new window (CEF Views) ───────────────────────────────────────

wrap_task! {
    pub struct CreateWindowTask {
        state: Arc<AppState>,
        url: String,
        label: String,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        frameless: bool,
    }

    impl Task {
        fn execute(&self) {
            use std::cell::RefCell;

            let settings = BrowserSettings {
                background_color: 0xFF000000,
                ..Default::default()
            };
            let cef_url = CefString::from(self.url.as_str());

            // Get client from an existing browser
            let browsers = self.state.browsers.lock();
            let client = browsers.values().next()
                .and_then(|b| b.host().map(|h| h.client()));
            drop(browsers);

            let mut request_context = crate::commands::create_isolated_request_context(
                &self.state, &self.label,
            );

            let mut client_ref = client.flatten();
            let mut bv_delegate = crate::app::AgentMuxBrowserViewDelegate::new(
                RuntimeStyle::ALLOY,
            );
            let browser_view = browser_view_create(
                client_ref.as_mut(),
                Some(&cef_url),
                Some(&settings),
                None,
                request_context.as_mut(),
                Some(&mut bv_delegate),
            );
            let mut wd = crate::app::AgentMuxWindowDelegate::new(
                RefCell::new(browser_view),
                Some((self.x, self.y, self.w, self.h)),
                self.frameless,
                RuntimeStyle::ALLOY,
            );
            window_create_top_level(Some(&mut wd));
        }
    }
}

pub fn post_create_window(
    state: &Arc<AppState>,
    url: &str,
    label: &str,
    x: i32, y: i32, w: i32, h: i32,
    frameless: bool,
) {
    let mut task = CreateWindowTask::new(
        state.clone(), url.to_string(), label.to_string(),
        x, y, w, h, frameless,
    );
    post_task(ThreadId::UI, Some(&mut task));
}

// ── DevTools (toggle) ─────────────────────────────────────────────────────

wrap_task! {
    pub struct ShowDevToolsTask {
        state: Arc<AppState>,
        label: String,
    }

    impl Task {
        fn execute(&self) {
            let browsers = self.state.browsers.lock();
            let browser = match browsers.get(&self.label) {
                Some(b) => b.clone(),
                None => {
                    tracing::warn!("[devtools] browser '{}' not found", self.label);
                    return;
                }
            };
            drop(browsers);

            match browser.host() {
                Some(host) => {
                    // In CEF Views mode, window_info is ignored by show_dev_tools().
                    // CEF routes the DevTools popup through on_popup_browser_view_created
                    // in AgentMuxBrowserViewDelegate, which creates a native window for it.
                    if host.has_dev_tools() != 0 {
                        host.close_dev_tools();
                    } else {
                        host.show_dev_tools(None, None, None, None);
                    }
                }
                None => {
                    tracing::warn!("[devtools] no browser host for '{}'", self.label);
                }
            }
        }
    }
}

pub fn post_show_dev_tools(state: &Arc<AppState>, label: &str) {
    let mut task = ShowDevToolsTask::new(state.clone(), label.to_string());
    post_task(ThreadId::UI, Some(&mut task));
}

// ── Main-focus reclaim ────────────────────────────────────────────────────
//
// Reclaim keyboard focus for the main browser when the user clicks a
// main-DOM input (address bar, etc). Runs on the CEF UI thread because:
//   - host.set_focus / browser_view_get_for_browser require the UI thread
//   - walking the HWND tree via EnumChildWindows is safer post-setup when
//     Chromium has published all of its render widgets
//
// On Windows, after the Chromium-level focus flip we also walk the Views
// window for the Chrome_RenderWidgetHostHWND and Win32-SetFocus it — without
// that explicit Win32 SetFocus, keyboard events keep routing to whichever
// pane HWND currently holds Win32 focus even though Chromium "thinks" main
// is focused. Observed on v0.33.264: host.set_focus(1) on main left pane
// keystrokes arriving at the pane HWND for >2 seconds.

wrap_task! {
    pub struct MainFocusReclaimTask {
        state: Arc<AppState>,
        label: String,
    }

    impl Task {
        fn execute(&self) {
            let mut browser = match self.state.browsers.lock().get(&self.label).cloned() {
                Some(b) => b,
                None => {
                    tracing::warn!("[main-focus-reclaim] no browser for label={}", self.label);
                    return;
                }
            };

            if let Some(host) = browser.host() {
                host.set_focus(1);
                tracing::info!("[main-focus-reclaim] host.set_focus(1) on label={}", self.label);
            }

            #[cfg(target_os = "windows")]
            {
                let views_top_hwnd = browser_view_get_for_browser(Some(&mut browser))
                    .and_then(|bv| bv.window())
                    .map(|w| w.window_handle().0 as *mut std::ffi::c_void)
                    .filter(|p| !p.is_null());

                // Collect every pane's outer HWND so we can skip render widgets
                // that descend from them. Panes are siblings of main under the
                // Views top-level, so a naive EnumChildWindows would pick up
                // their Chrome_RenderWidgetHostHWND and SetFocus on the wrong
                // target.
                let pane_outer_hwnds: Vec<*mut std::ffi::c_void> = {
                    let browsers = self.state.browsers.lock();
                    browsers
                        .iter()
                        .filter(|(k, _)| k.starts_with("browser-pane-"))
                        .filter_map(|(_, b)| {
                            let mut b = b.clone();
                            b.host().and_then(|h| {
                                let wh = h.window_handle();
                                if wh.0.is_null() { None } else { Some(wh.0 as *mut std::ffi::c_void) }
                            })
                        })
                        .collect()
                };

                match views_top_hwnd {
                    Some(top_hwnd) => unsafe {
                        let render = find_main_render_widget(top_hwnd, &pane_outer_hwnds);
                        let target = render.unwrap_or(top_hwnd);
                        windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus(target as _);
                        tracing::info!(
                            "[main-focus-reclaim] Win32 SetFocus target={:p} render_found={} panes_excluded={}",
                            target,
                            render.is_some(),
                            pane_outer_hwnds.len(),
                        );
                    },
                    None => {
                        tracing::warn!(
                            "[main-focus-reclaim] could not resolve Views top-level HWND for label={}",
                            self.label,
                        );
                    }
                }
            }

            // Defocus all live panes at the Chromium level too.
            self.state.browser_panes.defocus_all(&self.state);
        }
    }
}

/// Walk descendants of `root` and return the first Chrome_RenderWidgetHostHWND
/// whose ancestor chain does NOT pass through any of `pane_outer_hwnds`.
/// Panes are siblings of main under the Views top-level, so without this
/// filter the walk would happily pick a pane's render widget.
#[cfg(target_os = "windows")]
unsafe fn find_main_render_widget(
    root: *mut std::ffi::c_void,
    pane_outer_hwnds: &[*mut std::ffi::c_void],
) -> Option<*mut std::ffi::c_void> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumChildWindows, GetClassNameW, GetParent,
    };

    struct Finder<'a> {
        found: *mut std::ffi::c_void,
        panes: &'a [*mut std::ffi::c_void],
    }
    let mut finder = Finder { found: std::ptr::null_mut(), panes: pane_outer_hwnds };

    unsafe extern "system" fn cb(hwnd: *mut std::ffi::c_void, lparam: isize) -> i32 {
        let finder = &mut *(lparam as *mut Finder);
        let mut buf = [0u16; 64];
        let n = GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        if n > 0 {
            let class = String::from_utf16_lossy(&buf[..n as usize]);
            if class == "Chrome_RenderWidgetHostHWND" {
                // Walk ancestors; if we pass through any pane outer HWND,
                // this widget belongs to a pane, not main.
                let mut descends_from_pane = false;
                let mut cursor = GetParent(hwnd);
                while !cursor.is_null() {
                    if finder.panes.iter().any(|p| *p == cursor) {
                        descends_from_pane = true;
                        break;
                    }
                    cursor = GetParent(cursor);
                }
                if !descends_from_pane {
                    finder.found = hwnd;
                    return 0; // stop
                }
            }
        }
        1
    }

    EnumChildWindows(root, Some(cb), &mut finder as *mut _ as isize);
    if finder.found.is_null() { None } else { Some(finder.found) }
}
