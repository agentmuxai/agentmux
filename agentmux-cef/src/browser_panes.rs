// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! BrowserPaneManager: embeds native browser views inside the main window
//! using CefWindow::AddOverlayView with CEF_DOCKING_MODE_CUSTOM.
//!
//! Each browser pane is a CefBrowserView added as an overlay positioned
//! at a specific rect. The OverlayController handles z-order, bounds,
//! and cleanup. All operations dispatched to the CEF UI thread.

use std::collections::HashMap;
use std::sync::Arc;

use cef::*;
use parking_lot::Mutex;

use crate::state::AppState;

struct BrowserPane {
    controller: OverlayController,
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
        // If already exists, just navigate
        {
            let label = format!("browser-pane-{}", block_id);
            let browsers = state.browsers.lock();
            if browsers.contains_key(&label) {
                if let Some(browser) = browsers.get(&label) {
                    if let Some(frame) = browser.main_frame() {
                        frame.load_url(Some(&CefString::from(url)));
                    }
                }
                return Ok(());
            }
        }

        let panes = self.panes_arc();
        let mut task = CreatePaneTask::new(
            state.clone(), panes,
            block_id.to_string(), url.to_string(), rect,
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

    pub fn resize(&self, block_id: &str, rect: Rect) {
        let panes = self.panes.lock();
        if let Some(pane) = panes.get(block_id) {
            pane.controller.set_bounds(Some(&rect));
        }
    }

    pub fn close(&self, block_id: &str) {
        if let Some(pane) = self.panes.lock().remove(block_id) {
            pane.controller.destroy();
            tracing::info!(block_id, "browser pane destroyed");
        }
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

    /// Get an Arc reference to the panes map for passing to UI thread tasks.
    /// Uses a leaked Arc — the map lives for the program lifetime (inside AppState).
    fn panes_arc(&self) -> Arc<Mutex<HashMap<String, BrowserPane>>> {
        let ptr = &self.panes as *const Mutex<HashMap<String, BrowserPane>>;
        let arc = unsafe { Arc::from_raw(ptr) };
        let clone = arc.clone();
        std::mem::forget(arc); // don't drop the original
        clone
    }
}

// ── Create overlay ──────────────────────────────────────────────────────────

wrap_task! {
    pub struct CreatePaneTask {
        state: Arc<AppState>,
        panes: Arc<Mutex<HashMap<String, BrowserPane>>>,
        block_id: String,
        url: String,
        rect: Rect,
    }

    impl Task {
        fn execute(&self) {
            let label = format!("browser-pane-{}", self.block_id);

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

            // Create BrowserView for the URL
            let url_cef = CefString::from(self.url.as_str());
            let settings = BrowserSettings::default();

            let mut settings = settings;
            settings.background_color = 0xFFFFFFFF; // white — visible debug

            // Create a fresh client for this browser pane. Sharing the
            // main browser's client doesn't work — CEF needs a dedicated
            // client per browser for the renderer process to launch.
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

            let has_browser = new_bv.browser().is_some();
            tracing::info!(
                block_id = %self.block_id,
                has_browser,
                "browser_view created, adding overlay"
            );

            // Register browser for navigation (may be None until on_after_created)
            if let Some(browser) = new_bv.browser() {
                self.state.browsers.lock().insert(label.clone(), browser);
            }

            // Add as overlay with custom positioning
            let mut view = View::from(&new_bv);
            let controller = window.add_overlay_view(
                Some(&mut view),
                DockingMode::CUSTOM,
                1, // can_activate — receives keyboard/mouse input
            );

            match controller {
                Some(ctrl) => {
                    ctrl.set_bounds(Some(&self.rect));

                    // CEF issue #3790: overlays with CUSTOM docking may start
                    // hidden. Make the overlay and its contents visible.
                    if let Some(contents) = ctrl.contents_view() {
                        let is_visible = contents.is_visible();
                        tracing::info!(
                            block_id = %self.block_id,
                            contents_visible_before = is_visible,
                            "setting overlay contents visible"
                        );
                        contents.set_visible(1);
                    }

                    // Also make the view itself visible
                    view.set_visible(1);

                    let bounds = ctrl.bounds();
                    tracing::info!(
                        block_id = %self.block_id,
                        ctrl_x = bounds.x, ctrl_y = bounds.y,
                        ctrl_w = bounds.width, ctrl_h = bounds.height,
                        "overlay controller bounds after set_bounds"
                    );

                    tracing::info!(
                        block_id = %self.block_id,
                        url = %self.url,
                        x = self.rect.x, y = self.rect.y,
                        w = self.rect.width, h = self.rect.height,
                        "browser pane overlay created"
                    );
                    self.panes.lock().insert(self.block_id.clone(), BrowserPane {
                        controller: ctrl,
                    });
                }
                None => {
                    tracing::error!("add_overlay_view returned None");
                }
            }
        }
    }
}
