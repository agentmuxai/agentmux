// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! BrowserPaneManager: embeds browsers as native OS child windows using
//! CefBrowserHost::CreateBrowser. All creation runs on the CEF UI thread.

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

    pub fn create(
        &self,
        state: &Arc<AppState>,
        block_id: &str,
        url: &str,
        rect: Rect,
    ) -> Result<(), String> {
        if let Some(pane) = self.panes.lock().get(block_id) {
            if let Some(frame) = pane.browser.main_frame() {
                frame.load_url(Some(&CefString::from(url)));
            }
            return Ok(());
        }

        let label = format!("browser-pane-{}", block_id);
        let panes = self.panes_arc();
        let mut task = CreatePaneTask::new(
            state.clone(), panes,
            block_id.to_string(), label,
            url.to_string(), rect,
        );
        post_task(ThreadId::UI, Some(&mut task));
        Ok(())
    }

    pub fn navigate(&self, block_id: &str, url: &str, _state: &Arc<AppState>) -> Result<(), String> {
        if let Some(pane) = self.panes.lock().get(block_id) {
            if let Some(frame) = pane.browser.main_frame() {
                frame.load_url(Some(&CefString::from(url)));
            }
        }
        Ok(())
    }

    pub fn resize(&self, block_id: &str, rect: Rect, _state: &Arc<AppState>) {
        if let Some(pane) = self.panes.lock().get(block_id) {
            if let Some(host) = pane.browser.host() {
                let hwnd = host.window_handle();
                if !hwnd.0.is_null() {
                    #[cfg(target_os = "windows")]
                    unsafe {
                        windows_sys::Win32::UI::WindowsAndMessaging::SetWindowPos(
                            hwnd.0 as _,
                            std::ptr::null_mut(),
                            rect.x, rect.y, rect.width, rect.height,
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
        if let Some(pane) = self.panes.lock().get(block_id) { let mut b = pane.browser.clone(); b.go_back(); }
    }
    pub fn go_forward(&self, block_id: &str, _state: &Arc<AppState>) {
        if let Some(pane) = self.panes.lock().get(block_id) { let mut b = pane.browser.clone(); b.go_forward(); }
    }
    pub fn reload(&self, block_id: &str, _state: &Arc<AppState>) {
        if let Some(pane) = self.panes.lock().get(block_id) { let mut b = pane.browser.clone(); b.reload(); }
    }

    fn panes_arc(&self) -> Arc<Mutex<HashMap<String, BrowserPane>>> {
        let ptr = &self.panes as *const Mutex<HashMap<String, BrowserPane>>;
        let arc = unsafe { Arc::from_raw(ptr) };
        let clone = arc.clone();
        std::mem::forget(arc);
        clone
    }
}

// ── UI thread task: create browser ──────────────────────────────────────────

wrap_task! {
    pub struct CreatePaneTask {
        state: Arc<AppState>,
        panes: Arc<Mutex<HashMap<String, BrowserPane>>>,
        block_id: String,
        label: String,
        url: String,
        rect: Rect,
    }

    impl Task {
        fn execute(&self) {
            // Running on CEF UI thread

            // Get parent HWND via Win32 enumeration (CEF Views returns null)
            #[cfg(target_os = "windows")]
            let parent_hwnd_raw = unsafe {
                crate::commands::window::find_own_top_level_window()
            };
            #[cfg(not(target_os = "windows"))]
            let parent_hwnd_raw: *mut std::ffi::c_void = std::ptr::null_mut();

            if parent_hwnd_raw.is_null() {
                tracing::error!("cannot find main window HWND");
                return;
            }

            let parent_hwnd = sys::HWND(parent_hwnd_raw as *mut _);

            // Queue label for on_after_created
            self.state.pending_window_labels.lock().push_back(self.label.clone());

            // Create client
            let handler = crate::client::AgentMuxHandler::new(self.state.clone(), 0);
            let mut client = Some(crate::client::AgentMuxClient::new(handler));

            let url_cef = CefString::from(self.url.as_str());
            let settings = BrowserSettings::default();

            #[cfg(target_os = "windows")]
            let window_info = WindowInfo {
                size: std::mem::size_of::<WindowInfo>(),
                ex_style: 0,
                window_name: CefString::from(""),
                style: WS_CHILD | WS_VISIBLE | WS_CLIPCHILDREN | WS_CLIPSIBLINGS,
                bounds: Rect { x: self.rect.x, y: self.rect.y, width: self.rect.width, height: self.rect.height },
                parent_window: parent_hwnd,
                menu: std::ptr::null_mut(),
                windowless_rendering_enabled: 0,
                shared_texture_enabled: 0,
                external_begin_frame_enabled: 0,
                window: parent_hwnd, // CEF will create its own child HWND
                runtime_style: RuntimeStyle::DEFAULT,
            };

            let result = browser_host_create_browser(
                Some(&window_info),
                client.as_mut(),
                Some(&url_cef),
                Some(&settings),
                None, None,
            );

            if result == 0 {
                tracing::error!(block_id = %self.block_id, "browser_host_create_browser failed");
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

            // Wait for on_after_created to register the browser, then store in panes
            let panes = self.panes.clone();
            let block_id = self.block_id.clone();
            let label = self.label.clone();
            let state = self.state.clone();
            std::thread::spawn(move || {
                for _ in 0..50 {
                    std::thread::sleep(std::time::Duration::from_millis(100));
                    let browsers = state.browsers.lock();
                    if let Some(browser) = browsers.get(&label) {
                        panes.lock().insert(block_id.clone(), BrowserPane { browser: browser.clone() });
                        tracing::info!(block_id = %block_id, "browser pane registered");
                        std::mem::forget(panes); // don't drop the Arc (we don't own the Mutex)
                        return;
                    }
                }
                tracing::warn!(block_id = %block_id, "browser pane registration timed out");
                std::mem::forget(panes);
            });
        }
    }
}
