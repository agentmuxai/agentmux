// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! BrowserPaneManager: manages native CefBrowserView instances for embedded
//! browser panes. Each pane is a full Chromium browser rendered as a child
//! view of the main CefWindow, positioned over a placeholder div in the
//! frontend DOM.
//!
//! See docs/specs/SPEC_NATIVE_BROWSER_PANE_2026_04_17.md

use std::collections::HashMap;
use std::sync::Arc;

use cef::*;
use parking_lot::Mutex;

use crate::state::AppState;

/// A single embedded browser pane.
struct BrowserPane {
    browser_view: BrowserView,
    #[allow(dead_code)]
    block_id: String,
}

/// Manages all active browser panes.
pub struct BrowserPaneManager {
    panes: Mutex<HashMap<String, BrowserPane>>,
}

impl BrowserPaneManager {
    pub fn new() -> Self {
        Self {
            panes: Mutex::new(HashMap::new()),
        }
    }

    /// Create a new browser pane and add it to the window.
    pub fn create(
        &self,
        state: &Arc<AppState>,
        block_id: &str,
        url: &str,
        rect: Rect,
    ) -> Result<(), String> {
        // Don't create duplicates
        if self.panes.lock().contains_key(block_id) {
            tracing::warn!(block_id, "browser pane already exists, navigating instead");
            return self.navigate(block_id, url);
        }

        // Get the main window from the browsers map.
        // Clone the browser handle (bumps CEF ref count), drop the lock,
        // then resolve the window.
        let mut main_browser = {
            let browsers = state.browsers.lock();
            browsers.get("main")
                .ok_or("no main browser registered")?
                .clone()
        };
        let browser_view = browser_view_get_for_browser(Some(&mut main_browser))
            .ok_or("could not get BrowserView for main browser")?;
        let window = browser_view.window()
            .ok_or("could not get Window from main BrowserView")?;

        let url_cef = CefString::from(url);
        let settings = BrowserSettings::default();

        // Create the browser view — no delegate needed for basic browsing
        let browser_view = browser_view_create(
            None,       // client (use default)
            Some(&url_cef),
            Some(&settings),
            None,       // extra_info
            None,       // request_context (shared cookies)
            None,       // delegate
        );

        let Some(browser_view) = browser_view else {
            return Err("browser_view_create returned None".to_string());
        };

        // Position the view and add to window
        let mut view = View::from(&browser_view);
        view.set_bounds(Some(&rect));
        window.add_child_view(Some(&mut view));

        tracing::info!(
            block_id,
            url,
            x = rect.x, y = rect.y, w = rect.width, h = rect.height,
            "browser pane created"
        );

        self.panes.lock().insert(block_id.to_string(), BrowserPane {
            browser_view,
            block_id: block_id.to_string(),
        });

        Ok(())
    }

    /// Navigate an existing pane to a new URL.
    pub fn navigate(&self, block_id: &str, url: &str) -> Result<(), String> {
        let panes = self.panes.lock();
        let pane = panes.get(block_id)
            .ok_or_else(|| format!("browser pane {} not found", block_id))?;

        if let Some(browser) = pane.browser_view.browser() {
            let frame = browser.main_frame();
            if let Some(frame) = frame {
                frame.load_url(Some(&CefString::from(url)));
            }
        }
        Ok(())
    }

    /// Reposition an existing pane (called on scroll/resize).
    pub fn resize(&self, block_id: &str, rect: Rect) {
        let panes = self.panes.lock();
        if let Some(pane) = panes.get(block_id) {
            let mut view = View::from(&pane.browser_view);
            view.set_bounds(Some(&rect));
        }
    }

    /// Close and destroy a browser pane.
    pub fn close(&self, block_id: &str) {
        if let Some(pane) = self.panes.lock().remove(block_id) {
            // Remove from the window's view hierarchy
            let mut view = View::from(&pane.browser_view);
            if let Some(parent) = view.parent_view() {
                // The parent is the CefWindow's content panel
                // Removing the view destroys the browser
                let _ = parent;
            }
            // Close the browser to free resources
            if let Some(browser) = pane.browser_view.browser() {
                let host = browser.host();
                if let Some(host) = host {
                    host.close_browser(1);
                }
            }
            tracing::info!(block_id, "browser pane closed");
        }
    }

    /// Go back in the pane's browser history.
    pub fn go_back(&self, block_id: &str) {
        let panes = self.panes.lock();
        if let Some(pane) = panes.get(block_id) {
            if let Some(mut browser) = pane.browser_view.browser() {
                browser.go_back();
            }
        }
    }

    /// Go forward in the pane's browser history.
    pub fn go_forward(&self, block_id: &str) {
        let panes = self.panes.lock();
        if let Some(pane) = panes.get(block_id) {
            if let Some(mut browser) = pane.browser_view.browser() {
                browser.go_forward();
            }
        }
    }

    /// Reload the pane's current page.
    pub fn reload(&self, block_id: &str) {
        let panes = self.panes.lock();
        if let Some(pane) = panes.get(block_id) {
            if let Some(mut browser) = pane.browser_view.browser() {
                browser.reload();
            }
        }
    }
}
