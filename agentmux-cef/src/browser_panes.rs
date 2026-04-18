// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! BrowserPaneManager: embeds browsers as native OS child windows using
//! CefBrowserHost::CreateBrowser. All creation runs on the CEF UI thread.
//!
//! The Browser instance is owned by `state.browsers` (keyed by label). We only
//! store the block_id -> label mapping here; look up the browser when needed.

use std::collections::HashMap;
use std::sync::Arc;

use cef::*;
use parking_lot::Mutex;

use crate::state::AppState;

pub struct BrowserPaneManager {
    // block_id -> browser label (used to find the Browser in state.browsers)
    panes: Mutex<HashMap<String, String>>,
}

impl BrowserPaneManager {
    pub fn new() -> Self {
        Self { panes: Mutex::new(HashMap::new()) }
    }

    fn label_for(&self, block_id: &str) -> Option<String> {
        self.panes.lock().get(block_id).cloned()
    }

    fn browser_for(&self, state: &Arc<AppState>, block_id: &str) -> Option<Browser> {
        let label = self.label_for(block_id)?;
        state.browsers.lock().get(&label).cloned()
    }

    pub fn create(
        &self,
        state: &Arc<AppState>,
        block_id: &str,
        url: &str,
        rect: Rect,
    ) -> Result<(), String> {
        if let Some(browser) = self.browser_for(state, block_id) {
            if let Some(frame) = browser.main_frame() {
                frame.load_url(Some(&CefString::from(url)));
            }
            return Ok(());
        }

        let label = format!("browser-pane-{}", block_id);
        self.panes.lock().insert(block_id.to_string(), label.clone());

        let mut task = CreatePaneTask::new(
            state.clone(),
            block_id.to_string(),
            label,
            url.to_string(),
            rect,
        );
        post_task(ThreadId::UI, Some(&mut task));
        Ok(())
    }

    pub fn navigate(&self, block_id: &str, url: &str, state: &Arc<AppState>) -> Result<(), String> {
        if let Some(browser) = self.browser_for(state, block_id) {
            if let Some(frame) = browser.main_frame() {
                frame.load_url(Some(&CefString::from(url)));
            }
        }
        Ok(())
    }

    pub fn resize(&self, block_id: &str, rect: Rect, state: &Arc<AppState>) {
        if let Some(browser) = self.browser_for(state, block_id) {
            if let Some(host) = browser.host() {
                let hwnd = host.window_handle();
                if !hwnd.0.is_null() {
                    #[cfg(target_os = "windows")]
                    unsafe {
                        // HWND_TOP = 0 brings the window to the top of the Z-order
                        // within its parent. SWP_NOACTIVATE keeps keyboard focus where
                        // it is. We MUST keep the pane on top of the main browser's
                        // Chrome_RenderWidgetHostHWND — otherwise mouse-wheel events
                        // over the visible pane area hit main's widget (higher in
                        // Z-order) instead of the pane, and scrolling is broken.
                        windows_sys::Win32::UI::WindowsAndMessaging::SetWindowPos(
                            hwnd.0 as _,
                            std::ptr::null_mut(), // HWND_TOP
                            rect.x, rect.y, rect.width, rect.height,
                            0x0010, // SWP_NOACTIVATE
                        );
                    }
                    // Tell Chromium the pane has been moved/resized so its cached
                    // screen bounds are updated — without this, hit-tests on
                    // scrollbars and drag operations use stale coordinates and
                    // drags are rejected / misrouted.
                    host.notify_move_or_resize_started();
                }
            }
        }
    }

    pub fn close(&self, block_id: &str, state: &Arc<AppState>) {
        let label = match self.panes.lock().remove(block_id) {
            Some(l) => l,
            None => return,
        };
        // Clone the browser out of state.browsers WITHOUT removing it —
        // removing here made on_before_close's label lookup fail (it uses
        // is_same against entries in state.browsers to find the closing
        // browser), which skipped the per-browser cleanup path and left
        // the main browser's on_before_close as the only one that found a
        // match. Let CEF fire on_before_close and remove the entry
        // naturally there.
        let browser = {
            let browsers = state.browsers.lock();
            browsers.get(&label).cloned()
        };
        if let Some(browser) = browser {
            if let Some(host) = browser.host() {
                // force_close=0 (graceful) — force_close=1 was cascading into
                // the main browser's on_before_close and quitting the whole
                // app. A graceful close closes only this browser.
                host.close_browser(0);
            }
            tracing::info!(block_id, "browser pane close requested");
        }
    }

    pub fn go_back(&self, block_id: &str, state: &Arc<AppState>) {
        if let Some(mut b) = self.browser_for(state, block_id) { b.go_back(); }
    }
    pub fn go_forward(&self, block_id: &str, state: &Arc<AppState>) {
        if let Some(mut b) = self.browser_for(state, block_id) { b.go_forward(); }
    }
    pub fn reload(&self, block_id: &str, state: &Arc<AppState>) {
        if let Some(mut b) = self.browser_for(state, block_id) { b.reload(); }
    }

    /// Tell every registered pane browser it has lost focus, at the Chromium
    /// level. Called by the `main_window_focus` IPC to keep renderer-side
    /// focus in sync with OS-level focus when the user clicks a main-DOM
    /// input (e.g. a URL bar).
    pub fn defocus_all(&self, state: &Arc<AppState>) {
        let labels: Vec<String> = self.panes.lock().values().cloned().collect();
        let browsers = state.browsers.lock();
        for label in &labels {
            if let Some(browser) = browsers.get(label).cloned() {
                if let Some(host) = browser.host() {
                    host.set_focus(0);
                }
            }
        }
    }

    /// Give keyboard focus to the pane's child HWND so keystrokes reach the
    /// embedded page. Called by the frontend's ViewModel.giveFocus() when the
    /// pane becomes the active layout node — without this, focus falls back to
    /// the main window's invisible "dummy-focus" input and keystrokes vanish.
    pub fn focus(&self, block_id: &str, state: &Arc<AppState>) {
        if let Some(browser) = self.browser_for(state, block_id) {
            if let Some(host) = browser.host() {
                host.set_focus(1);
                #[cfg(target_os = "windows")]
                {
                    let hwnd = host.window_handle();
                    if !hwnd.0.is_null() {
                        // Tell the subclass this focus request is intentional
                        // (not Chromium's on-load focus steal) so it won't be
                        // redirected back to the parent.
                        crate::client::ALLOW_PANE_FOCUS_ONCE.store(
                            true,
                            std::sync::atomic::Ordering::Relaxed,
                        );
                        unsafe {
                            windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus(hwnd.0 as _);
                        }
                    }
                }
            }
        }
    }
}

// ── UI thread task: create browser ──────────────────────────────────────────

wrap_task! {
    pub struct CreatePaneTask {
        state: Arc<AppState>,
        block_id: String,
        label: String,
        url: String,
        rect: Rect,
    }

    impl Task {
        fn execute(&self) {
            // Running on the CEF UI thread.

            // Get parent HWND via Win32 enumeration (CEF Views returns null).
            #[cfg(target_os = "windows")]
            let parent_hwnd_raw = unsafe {
                crate::commands::window::find_own_top_level_window()
            };
            #[cfg(not(target_os = "windows"))]
            let parent_hwnd_raw: *mut std::ffi::c_void = std::ptr::null_mut();

            if parent_hwnd_raw.is_null() {
                tracing::error!(block_id = %self.block_id, "cannot find main window HWND — aborting browser pane creation");
                return;
            }

            // Queue label for on_after_created registration (stored in state.browsers)
            self.state.pending_window_labels.lock().push_back(self.label.clone());

            let handler = crate::client::AgentMuxHandler::new_with_pane(self.state.clone(), 0, true);
            let mut client = Some(crate::client::AgentMuxClient::new(handler, true));

            let url_cef = CefString::from(self.url.as_str());
            let settings = BrowserSettings::default();

            #[cfg(target_os = "windows")]
            let window_info = {
                let parent_hwnd = sys::HWND(parent_hwnd_raw as *mut _);
                // Use the clean set_as_child helper — it fills style/parent/bounds
                // correctly and leaves other fields zeroed (in particular `window`
                // which is an OUTPUT field filled by CEF).
                let mut wi = WindowInfo::default().set_as_child(parent_hwnd, &self.rect);
                // Match the main process runtime style (ALLOY throughout the app).
                wi.runtime_style = RuntimeStyle::ALLOY;
                wi
            };

            #[cfg(not(target_os = "windows"))]
            {
                tracing::warn!(block_id = %self.block_id, "browser panes not yet implemented on this platform");
                return;
            }

            #[cfg(target_os = "windows")]
            {
                let result = browser_host_create_browser(
                    Some(&window_info),
                    client.as_mut(),
                    Some(&url_cef),
                    Some(&settings),
                    None, // extra_info
                    None, // request_context
                );

                if result == 0 {
                    tracing::error!(block_id = %self.block_id, "browser_host_create_browser returned 0");
                    return;
                }

                tracing::info!(
                    block_id = %self.block_id,
                    label = %self.label,
                    url = %self.url,
                    x = self.rect.x, y = self.rect.y,
                    w = self.rect.width, h = self.rect.height,
                    "browser pane created on UI thread"
                );
            }
        }
    }
}
