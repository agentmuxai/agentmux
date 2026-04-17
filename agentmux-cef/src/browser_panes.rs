// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! BrowserPaneManager: manages native CefBrowserView instances for embedded
//! browser panes. Each pane is a frameless popup CefWindow positioned over
//! the pane's DOM rect in the main window. This solves the z-order problem
//! (add_child_view renders behind the main BrowserView).

use std::cell::RefCell;
use std::sync::Arc;

use cef::*;

use crate::state::AppState;

pub struct BrowserPaneManager;

impl BrowserPaneManager {
    pub fn new() -> Self { Self }

    pub fn create(
        &self,
        state: &Arc<AppState>,
        block_id: &str,
        url: &str,
        rect: Rect,
    ) -> Result<(), String> {
        // Register the popup label so on_after_created can find it
        let label = format!("browser-pane-{}", block_id);
        state.pending_window_labels.lock().push_back(label.clone());

        let mut task = CreateBrowserPaneTask::new(
            state.clone(),
            block_id.to_string(),
            label,
            url.to_string(),
            rect,
        );
        post_task(ThreadId::UI, Some(&mut task));
        Ok(())
    }

    pub fn navigate(&self, block_id: &str, state: &Arc<AppState>) -> Result<(), String> {
        let label = format!("browser-pane-{}", block_id);
        let browsers = state.browsers.lock();
        if let Some(browser) = browsers.get(&label) {
            // Navigate is handled via the frontend sending browser_pane_navigate
            // which re-creates the pane with the new URL
            let _ = browser;
        }
        Ok(())
    }

    pub fn resize(&self, block_id: &str, rect: Rect, state: &Arc<AppState>) {
        let label = format!("browser-pane-{}", block_id);
        let mut task = ResizeBrowserPaneTask::new(
            state.clone(),
            label,
            rect,
        );
        post_task(ThreadId::UI, Some(&mut task));
    }

    pub fn close(&self, block_id: &str, state: &Arc<AppState>) {
        let label = format!("browser-pane-{}", block_id);
        let mut task = crate::ui_tasks::CloseWindowTask::new(
            state.clone(),
            label,
        );
        post_task(ThreadId::UI, Some(&mut task));
    }

    pub fn go_back(&self, block_id: &str, state: &Arc<AppState>) {
        let label = format!("browser-pane-{}", block_id);
        let browsers = state.browsers.lock();
        if let Some(browser) = browsers.get(&label) {
            let mut b = browser.clone();
            b.go_back();
        }
    }

    pub fn go_forward(&self, block_id: &str, state: &Arc<AppState>) {
        let label = format!("browser-pane-{}", block_id);
        let browsers = state.browsers.lock();
        if let Some(browser) = browsers.get(&label) {
            let mut b = browser.clone();
            b.go_forward();
        }
    }

    pub fn reload(&self, block_id: &str, state: &Arc<AppState>) {
        let label = format!("browser-pane-{}", block_id);
        let browsers = state.browsers.lock();
        if let Some(browser) = browsers.get(&label) {
            let mut b = browser.clone();
            b.reload();
        }
    }
}

// ── Create: frameless popup window ──────────────────────────────────────────

wrap_task! {
    pub struct CreateBrowserPaneTask {
        state: Arc<AppState>,
        block_id: String,
        label: String,
        url: String,
        rect: Rect,
    }

    impl Task {
        fn execute(&self) {
            // Get main window position to compute absolute screen coords
            let main_window_origin = {
                let browsers = self.state.browsers.lock();
                let mut main = match browsers.get("main") {
                    Some(b) => b.clone(),
                    None => { tracing::error!("no main browser"); return; }
                };
                drop(browsers);
                browser_view_get_for_browser(Some(&mut main))
                    .and_then(|bv| bv.window())
                    .map(|w| {
                        let b = w.bounds();
                        (b.x, b.y)
                    })
                    .unwrap_or((0, 0))
            };

            let abs_x = main_window_origin.0 + self.rect.x;
            let abs_y = main_window_origin.1 + self.rect.y;

            let url_cef = CefString::from(self.url.as_str());
            let settings = BrowserSettings::default();

            // Get client from main browser
            let browsers = self.state.browsers.lock();
            let client = browsers.values().next()
                .and_then(|b| b.host().map(|h| h.client()));
            drop(browsers);

            let mut client_ref = client.flatten();
            let mut bv_delegate = crate::app::AgentMuxBrowserViewDelegate::new(
                RuntimeStyle::ALLOY,
            );
            let browser_view = browser_view_create(
                client_ref.as_mut(),
                Some(&url_cef),
                Some(&settings),
                None,
                None,
                Some(&mut bv_delegate),
            );

            // Create frameless popup window
            let mut wd = crate::app::AgentMuxWindowDelegate::new(
                RefCell::new(browser_view),
                Some((abs_x, abs_y, self.rect.width, self.rect.height)),
                true, // frameless
                RuntimeStyle::ALLOY,
            );
            window_create_top_level(Some(&mut wd));

            // On Windows: set WS_EX_TOOLWINDOW to hide from taskbar
            #[cfg(target_os = "windows")]
            {
                // The window is created asynchronously by CEF — the style
                // will be set in on_window_created via the label prefix check.
            }

            tracing::info!(
                block_id = %self.block_id,
                label = %self.label,
                url = %self.url,
                abs_x, abs_y,
                w = self.rect.width, h = self.rect.height,
                "browser pane popup created"
            );
        }
    }
}

// ── Resize: reposition popup window ─────────────────────────────────────────

wrap_task! {
    pub struct ResizeBrowserPaneTask {
        state: Arc<AppState>,
        label: String,
        rect: Rect,
    }

    impl Task {
        fn execute(&self) {
            // Get main window origin for absolute positioning
            let main_origin = {
                let browsers = self.state.browsers.lock();
                let mut main = match browsers.get("main") {
                    Some(b) => b.clone(),
                    None => return,
                };
                drop(browsers);
                browser_view_get_for_browser(Some(&mut main))
                    .and_then(|bv| bv.window())
                    .map(|w| { let b = w.bounds(); (b.x, b.y) })
                    .unwrap_or((0, 0))
            };

            let browsers = self.state.browsers.lock();
            let mut browser = match browsers.get(&self.label) {
                Some(b) => b.clone(),
                None => return,
            };
            drop(browsers);

            if let Some(bv) = browser_view_get_for_browser(Some(&mut browser)) {
                if let Some(window) = bv.window() {
                    window.set_bounds(Some(&Rect {
                        x: main_origin.0 + self.rect.x,
                        y: main_origin.1 + self.rect.y,
                        width: self.rect.width,
                        height: self.rect.height,
                    }));
                }
            }
        }
    }
}
