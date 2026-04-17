// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! BrowserPaneManager: embeds native browser views using CefBrowserHost::CreateBrowser
//! with a parent HWND. This is the production-proven pattern used by CefSharp, QCefView,
//! and Spotify. No CEF Views framework — native OS child window management.

use std::collections::HashMap;
use std::sync::Arc;

use cef::*;
use parking_lot::Mutex;

use crate::state::AppState;

#[cfg(target_os = "windows")]
const WS_CHILD: u32 = 0x4000_0000;
#[cfg(target_os = "windows")]
const WS_VISIBLE: u32 = 0x1000_0000;
#[cfg(target_os = "windows")]
const WS_CLIPCHILDREN: u32 = 0x0200_0000;
#[cfg(target_os = "windows")]
const WS_CLIPSIBLINGS: u32 = 0x0400_0000;

struct BrowserPane {
    browser: Browser,
}

pub struct BrowserPaneManager {
    panes: Mutex<HashMap<String, BrowserPane>>,
}

impl BrowserPaneManager {
    pub fn new() -> Self {
        Self { panes: Mutex::new(HashMap::new()) }
    }

    /// Create a browser pane as a native child window of the main window.
    pub fn create(
        &self,
        state: &Arc<AppState>,
        block_id: &str,
        url: &str,
        rect: Rect,
    ) -> Result<(), String> {
        // If already exists, navigate
        if let Some(pane) = self.panes.lock().get(block_id) {
            if let Some(frame) = pane.browser.main_frame() {
                frame.load_url(Some(&CefString::from(url)));
            }
            return Ok(());
        }

        // Get main window's native handle.
        // CEF Views mode returns NULL from host.window_handle(), so
        // we enumerate Win32 windows by process ID (same as window.rs).
        #[cfg(target_os = "windows")]
        let parent_hwnd_raw = unsafe {
            crate::commands::window::find_own_top_level_window()
        };
        #[cfg(not(target_os = "windows"))]
        let parent_hwnd_raw: *mut std::ffi::c_void = std::ptr::null_mut();

        if parent_hwnd_raw.is_null() {
            return Err("could not find main window HWND".to_string());
        }

        // Wrap raw pointer in CEF's HWND type
        let parent_hwnd = cef::sys::HWND(parent_hwnd_raw as *mut _);

        // Queue label for on_after_created registration
        let label = format!("browser-pane-{}", block_id);
        state.pending_window_labels.lock().push_back(label.clone());

        // Create client for this browser pane
        let handler = crate::client::AgentMuxHandler::new(state.clone(), 0);
        let mut client = Some(crate::client::AgentMuxClient::new(handler));

        let url_cef = CefString::from(url);
        let settings = BrowserSettings::default();

        // Configure as a child window of the main window
        #[cfg(target_os = "windows")]
        let window_info = WindowInfo {
            size: std::mem::size_of::<WindowInfo>(),
            ex_style: 0,
            window_name: CefString::from(""),
            style: WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
            bounds: Rect { x: rect.x, y: rect.y, width: rect.width, height: rect.height },
            parent_window: parent_hwnd,
            menu: std::ptr::null_mut(),
            windowless_rendering_enabled: 0,
            shared_texture_enabled: 0,
            external_begin_frame_enabled: 0,
            window: parent_hwnd,
            runtime_style: RuntimeStyle::DEFAULT,
        };

        #[cfg(not(target_os = "windows"))]
        let window_info = {
            // TODO: macOS/Linux — set parent_window to the main NSView/X11 window
            return Err("browser panes not yet implemented on this platform".to_string());
        };

        let result = browser_host_create_browser(
            Some(&window_info),
            client.as_mut(),
            Some(&url_cef),
            Some(&settings),
            None, // extra_info
            None, // request_context
        );

        if result == 0 {
            return Err("browser_host_create_browser returned 0 (failed)".to_string());
        }

        tracing::info!(
            block_id,
            url,
            x = rect.x, y = rect.y,
            w = rect.width, h = rect.height,
            parent_hwnd = ?parent_hwnd,
            "browser pane creating as native child window"
        );

        // The browser will be registered in on_after_created via pending_window_labels.
        // We'll store it in self.panes when on_after_created fires.
        // For now, spawn a task that waits for registration and stores the ref.
        let panes = self.panes_arc();
        let block_id = block_id.to_string();
        let state_clone = state.clone();
        std::thread::spawn(move || {
            // Poll for the browser to be registered (on_after_created is async)
            for _ in 0..50 {
                std::thread::sleep(std::time::Duration::from_millis(100));
                let browsers = state_clone.browsers.lock();
                if let Some(browser) = browsers.get(&label) {
                    panes.lock().insert(block_id.clone(), BrowserPane {
                        browser: browser.clone(),
                    });
                    tracing::info!(block_id = %block_id, "browser pane registered");
                    return;
                }
            }
            tracing::warn!(block_id = %block_id, "browser pane registration timed out (5s)");
        });

        Ok(())
    }

    pub fn navigate(&self, block_id: &str, url: &str, _state: &Arc<AppState>) -> Result<(), String> {
        let panes = self.panes.lock();
        if let Some(pane) = panes.get(block_id) {
            if let Some(frame) = pane.browser.main_frame() {
                frame.load_url(Some(&CefString::from(url)));
            }
        }
        Ok(())
    }

    pub fn resize(&self, block_id: &str, rect: Rect, _state: &Arc<AppState>) {
        let panes = self.panes.lock();
        if let Some(pane) = panes.get(block_id) {
            if let Some(host) = pane.browser.host() {
                let hwnd = host.window_handle();
                if !hwnd.0.is_null() {
                    #[cfg(target_os = "windows")]
                    unsafe {
                        windows_sys::Win32::UI::WindowsAndMessaging::SetWindowPos(
                            hwnd.0 as _,
                            std::ptr::null_mut(), // HWND_TOP
                            rect.x,
                            rect.y,
                            rect.width,
                            rect.height,
                            0x0004, // SWP_NOZORDER
                        );
                    }
                }
            }
        }
    }

    pub fn close(&self, block_id: &str, _state: &Arc<AppState>) {
        if let Some(pane) = self.panes.lock().remove(block_id) {
            if let Some(host) = pane.browser.host() {
                host.close_browser(1);
            }
            tracing::info!(block_id, "browser pane closed");
        }
    }

    pub fn go_back(&self, block_id: &str, _state: &Arc<AppState>) {
        let panes = self.panes.lock();
        if let Some(pane) = panes.get(block_id) { let mut b = pane.browser.clone(); b.go_back(); }
    }

    pub fn go_forward(&self, block_id: &str, _state: &Arc<AppState>) {
        let panes = self.panes.lock();
        if let Some(pane) = panes.get(block_id) { let mut b = pane.browser.clone(); b.go_forward(); }
    }

    pub fn reload(&self, block_id: &str, _state: &Arc<AppState>) {
        let panes = self.panes.lock();
        if let Some(pane) = panes.get(block_id) { let mut b = pane.browser.clone(); b.reload(); }
    }

    fn panes_arc(&self) -> Arc<Mutex<HashMap<String, BrowserPane>>> {
        let ptr = &self.panes as *const Mutex<HashMap<String, BrowserPane>>;
        let arc = unsafe { Arc::from_raw(ptr) };
        let clone = arc.clone();
        std::mem::forget(arc);
        clone
    }
}
