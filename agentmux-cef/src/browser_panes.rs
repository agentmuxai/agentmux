// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! BrowserPaneManager: creates native CefBrowserView instances as child
//! views of the main window. Each browser pane loads a URL directly in
//! Chromium — no iframes, no popups, no separate windows.

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
        let label = format!("browser-pane-{}", block_id);

        // If pane already exists, just navigate
        {
            let browsers = state.browsers.lock();
            if let Some(browser) = browsers.get(&label) {
                if let Some(frame) = browser.main_frame() {
                    frame.load_url(Some(&CefString::from(url)));
                }
                return Ok(());
            }
        }

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

    pub fn navigate(&self, block_id: &str, url: &str, state: &Arc<AppState>) -> Result<(), String> {
        let label = format!("browser-pane-{}", block_id);
        let browsers = state.browsers.lock();
        if let Some(browser) = browsers.get(&label) {
            if let Some(frame) = browser.main_frame() {
                frame.load_url(Some(&CefString::from(url)));
            }
        }
        Ok(())
    }

    pub fn resize(&self, block_id: &str, rect: Rect, state: &Arc<AppState>) {
        let label = format!("browser-pane-{}", block_id);
        let mut task = ResizePaneTask::new(state.clone(), label, rect);
        post_task(ThreadId::UI, Some(&mut task));
    }

    pub fn close(&self, block_id: &str, state: &Arc<AppState>) {
        let label = format!("browser-pane-{}", block_id);
        let mut task = ClosePaneTask::new(state.clone(), label);
        post_task(ThreadId::UI, Some(&mut task));
    }

    pub fn go_back(&self, block_id: &str, state: &Arc<AppState>) {
        let label = format!("browser-pane-{}", block_id);
        let browsers = state.browsers.lock();
        if let Some(b) = browsers.get(&label) { let mut b = b.clone(); b.go_back(); }
    }

    pub fn go_forward(&self, block_id: &str, state: &Arc<AppState>) {
        let label = format!("browser-pane-{}", block_id);
        let browsers = state.browsers.lock();
        if let Some(b) = browsers.get(&label) { let mut b = b.clone(); b.go_forward(); }
    }

    pub fn reload(&self, block_id: &str, state: &Arc<AppState>) {
        let label = format!("browser-pane-{}", block_id);
        let browsers = state.browsers.lock();
        if let Some(b) = browsers.get(&label) { let mut b = b.clone(); b.reload(); }
    }
}

// ── Create ──────────────────────────────────────────────────────────────────

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
            // Get the main window
            let mut main_browser = {
                let browsers = self.state.browsers.lock();
                match browsers.get("main") {
                    Some(b) => b.clone(),
                    None => { tracing::error!("no main browser"); return; }
                }
            };

            let main_bv = match browser_view_get_for_browser(Some(&mut main_browser)) {
                Some(bv) => bv,
                None => { tracing::error!("no main BrowserView"); return; }
            };

            let window = match main_bv.window() {
                Some(w) => w,
                None => { tracing::error!("no main Window"); return; }
            };

            // Get client from the main browser for proper rendering
            let client = main_browser.host()
                .and_then(|h| h.client());

            let url_cef = CefString::from(self.url.as_str());
            let settings = BrowserSettings::default();

            // Create a BrowserView with the same client as the main browser
            let mut delegate = crate::app::AgentMuxBrowserViewDelegate::new(
                RuntimeStyle::ALLOY,
            );
            let new_bv = match browser_view_create(
                client.as_ref().map(|c| c as &Client).cloned().as_mut(),
                Some(&url_cef),
                Some(&settings),
                None,
                None,
                Some(&mut delegate),
            ) {
                Some(bv) => bv,
                None => { tracing::error!("browser_view_create failed"); return; }
            };

            // Register the browser so navigation/close can find it
            if let Some(browser) = new_bv.browser() {
                self.state.browsers.lock().insert(self.label.clone(), browser);
            }

            // Add as child of the window — CEF Views paints children
            // in order, so last-added renders on top of earlier children
            let mut view = View::from(&new_bv);
            view.set_bounds(Some(&self.rect));
            view.set_visible(1);
            window.add_child_view(Some(&mut view));

            tracing::info!(
                block_id = %self.block_id,
                url = %self.url,
                x = self.rect.x, y = self.rect.y,
                w = self.rect.width, h = self.rect.height,
                "browser pane created"
            );
        }
    }
}

// ── Resize ──────────────────────────────────────────────────────────────────

wrap_task! {
    pub struct ResizePaneTask {
        state: Arc<AppState>,
        label: String,
        rect: Rect,
    }

    impl Task {
        fn execute(&self) {
            let browsers = self.state.browsers.lock();
            let mut browser = match browsers.get(&self.label) {
                Some(b) => b.clone(),
                None => return,
            };
            drop(browsers);

            if let Some(bv) = browser_view_get_for_browser(Some(&mut browser)) {
                let mut view = View::from(&bv);
                view.set_bounds(Some(&self.rect));
            }
        }
    }
}

// ── Close ───────────────────────────────────────────────────────────────────

wrap_task! {
    pub struct ClosePaneTask {
        state: Arc<AppState>,
        label: String,
    }

    impl Task {
        fn execute(&self) {
            let browser = self.state.browsers.lock().remove(&self.label);
            if let Some(browser) = browser {
                if let Some(host) = browser.host() {
                    host.close_browser(1);
                }
                tracing::info!(label = %self.label, "browser pane closed");
            }
        }
    }
}
