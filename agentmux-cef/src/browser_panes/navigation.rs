// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Navigation operations for `BrowserPaneManager`: `navigate`, `go_back`,
//! `go_forward`, `reload`, `resize`. Split out of `browser_panes.rs` — see
//! that module's doc comment.

use std::sync::Arc;

use cef::*;

use crate::state::AppState;

use super::BrowserPaneManager;

impl BrowserPaneManager {
    pub fn navigate(&self, block_id: &str, url: &str, state: &Arc<AppState>) -> Result<(), String> {
        if let Some(browser) = self.live_browser(state, block_id) {
            if let Some(frame) = browser.main_frame() {
                frame.load_url(Some(&CefString::from(url)));
            }
        }
        Ok(())
    }

    pub fn resize(&self, block_id: &str, rect: Rect, state: &Arc<AppState>) {
        // Windows: resize the wrapper (its own WM_SIZE handler cascades to
        // CEF's child automatically — see browser_pane::wrapper) instead of
        // SetWindowPos-ing CEF's HWND directly. Still need the live Browser
        // to call notify_move_or_resize_started (a CEF-side hint, unrelated
        // to which HWND owns the actual on-screen rect).
        #[cfg(target_os = "windows")]
        if let Some(browser) = self.live_browser(state, block_id) {
            let resized = state
                .live_browser_pane_label(block_id)
                .and_then(|label| crate::browser_pane::wrapper::peek_wrapper_hwnd(&label))
                .map(|wrapper_hwnd| {
                    crate::browser_pane::wrapper::resize_wrapper(
                        wrapper_hwnd as *mut std::ffi::c_void,
                        &rect,
                    );
                })
                .is_some();
            // Only notify CEF if we actually resized something — matches the
            // original hwnd-null gating this replaced.
            if resized {
                if let Some(host) = browser.host() {
                    host.notify_move_or_resize_started();
                }
            }
        }
        // Linux/macOS — Views path. The pane is a CefBrowserView in the main
        // window's view hierarchy; resizing is `View::set_bounds` in DIP. Must
        // run on the CEF UI thread (set_bounds is UI-thread-only).
        #[cfg(not(target_os = "windows"))]
        {
            let label = match state.live_browser_pane_label(block_id) {
                Some(l) => l,
                None => return, // pane already closed or never created
            };
            let mut task = ResizeBrowserPaneViewTask::new(state.clone(), label, rect);
            cef::post_task(cef::ThreadId::UI, Some(&mut task));
        }
    }

    pub fn go_back(&self, block_id: &str, state: &Arc<AppState>) {
        if let Some(mut b) = self.live_browser(state, block_id) { b.go_back(); }
    }
    pub fn go_forward(&self, block_id: &str, state: &Arc<AppState>) {
        if let Some(mut b) = self.live_browser(state, block_id) { b.go_forward(); }
    }
    pub fn reload(&self, block_id: &str, state: &Arc<AppState>) {
        if let Some(mut b) = self.live_browser(state, block_id) { b.reload(); }
    }
}

// `View::set_bounds` must run on the CEF UI thread; `resize()` is called from
// IPC handler tasks on tokio threads, so we wrap the UI-thread body in a
// `wrap_task!` struct and post it via `post_task(ThreadId::UI, ...)` — same
// pattern as `ui_tasks::CloseWindowTask` / `MaximizeWindowTask` / etc.
#[cfg(not(target_os = "windows"))]
wrap_task! {
    pub struct ResizeBrowserPaneViewTask {
        state: Arc<AppState>,
        label: String,
        rect: Rect,
    }

    impl Task {
        fn execute(&self) {
            crate::browser_pane::creation_views::resize_browser_pane_view(
                &self.state, &self.label, self.rect.clone(),
            );
        }
    }
}
