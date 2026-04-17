// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! BrowserPaneManager: embeds native browser views inside the main window
//! using add_child_view + deferred bounds. AddOverlayView has a CEF bug
//! where BrowserViews never initialize their renderer (issue #3790).
//!
//! The browser is added as a child view (which triggers renderer creation),
//! then a deferred task sets its bounds to the pane rect. The frontend's
//! ResizeObserver continuously re-sets bounds via IPC.

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

        // If already exists, navigate
        {
            let browsers = state.browsers.lock();
            if let Some(browser) = browsers.get(&label) {
                if let Some(frame) = browser.main_frame() {
                    frame.load_url(Some(&CefString::from(url)));
                }
                return Ok(());
            }
        }

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

// ── Create: add_child_view + deferred bounds ────────────────────────────────

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
            // Get main window
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

            // Queue label for on_after_created registration
            self.state.pending_window_labels.lock().push_back(self.label.clone());

            // Create browser view with a fresh client
            let url_cef = CefString::from(self.url.as_str());
            let settings = BrowserSettings::default();
            let handler = crate::client::AgentMuxHandler::new(self.state.clone(), 0);
            let mut client = Some(crate::client::AgentMuxClient::new(handler));

            let new_bv = match browser_view_create(
                client.as_mut(),
                Some(&url_cef),
                Some(&settings),
                None, None, None,
            ) {
                Some(bv) => bv,
                None => { tracing::error!("browser_view_create failed"); return; }
            };

            // add_child_view triggers browser renderer creation (confirmed working).
            // The window's FillLayout will expand it to full size, but we
            // immediately set bounds and the frontend's resize IPC will
            // continuously re-set bounds on every frame.
            let mut view = View::from(&new_bv);
            window.add_child_view(Some(&mut view));

            // Set bounds immediately after adding
            view.set_bounds(Some(&self.rect));
            view.set_size(Some(&Size {
                width: self.rect.width,
                height: self.rect.height,
            }));

            tracing::info!(
                block_id = %self.block_id,
                label = %self.label,
                url = %self.url,
                x = self.rect.x, y = self.rect.y,
                w = self.rect.width, h = self.rect.height,
                "browser pane added as child view"
            );

            // Post a deferred bounds set — the layout manager may override
            // our bounds during the current layout pass. The deferred task
            // runs after layout completes.
            let mut deferred = DeferredBoundsTask::new(
                self.state.clone(),
                self.label.clone(),
                Rect { x: self.rect.x, y: self.rect.y, width: self.rect.width, height: self.rect.height },
            );
            post_task(ThreadId::UI, Some(&mut deferred));
        }
    }
}

// ── Deferred bounds (runs after layout pass) ────────────────────────────────

wrap_task! {
    pub struct DeferredBoundsTask {
        state: Arc<AppState>,
        label: String,
        rect: Rect,
    }

    impl Task {
        fn execute(&self) {
            let browsers = self.state.browsers.lock();
            let mut browser = match browsers.get(&self.label) {
                Some(b) => b.clone(),
                None => {
                    tracing::warn!(label = %self.label, "deferred bounds: browser not yet registered");
                    return;
                }
            };
            drop(browsers);

            if let Some(bv) = browser_view_get_for_browser(Some(&mut browser)) {
                let mut view = View::from(&bv);
                view.set_bounds(Some(&self.rect));
                view.set_size(Some(&Size {
                    width: self.rect.width,
                    height: self.rect.height,
                }));
                tracing::info!(
                    label = %self.label,
                    x = self.rect.x, y = self.rect.y,
                    w = self.rect.width, h = self.rect.height,
                    "deferred bounds applied"
                );
            }
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
                view.set_size(Some(&Size {
                    width: self.rect.width,
                    height: self.rect.height,
                }));
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
