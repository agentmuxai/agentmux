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

    /// "Print" from the unified browser-pane context menu
    /// (SPEC_BROWSER_PANE_UNIFIED_CONTEXT_MENU_2026_08_15.md) — CEF's own
    /// print UI for the pane's current page, same as `CefBrowserHost::Print`
    /// used anywhere else in Chromium.
    pub fn print(&self, block_id: &str, state: &Arc<AppState>) {
        if let Some(mut b) = self.live_browser(state, block_id) {
            if let Some(host) = b.host() { host.print(); }
        }
    }

    /// "View Page Source" — navigates via CEF's built-in `view-source:`
    /// scheme (added to `PANE_ALLOWED_NAV_SCHEMES` alongside this feature;
    /// Chromium renders it internally, same as `chrome-devtools:`, so it
    /// carries none of the OS-handoff risk that allowlist exists to block).
    pub fn view_source(&self, block_id: &str, state: &Arc<AppState>) {
        let Some(mut b) = self.live_browser(state, block_id) else { return };
        let Some(frame) = b.main_frame() else { return };
        let current_url = CefString::from(&ImplFrame::url(&frame)).to_string();
        if current_url.is_empty() {
            return;
        }
        // Already viewing source (e.g. the menu item was clicked again, or
        // the pane's history is currently on a view-source: page) — reuse
        // the URL as-is instead of stacking another prefix onto it
        // (reagentx P2 on PR #2599: view-source:view-source:https://... is
        // not a real page).
        let src_url = if current_url.starts_with("view-source:") {
            current_url
        } else {
            format!("view-source:{current_url}")
        };
        frame.load_url(Some(&CefString::from(src_url.as_str())));
    }

    /// "Inspect Element" — opens CEF's own DevTools window, jumping straight
    /// to the element under the right-click point (mirrors Chrome's
    /// behavior). `x`/`y` are view-local coordinates, the same ones CEF's
    /// `ContextMenuParams::xcoord/ycoord` reported when the menu was
    /// suppressed — see `client::context_menu`. `None` for
    /// window_info/client/settings uses CEF's default popup DevTools window.
    pub fn inspect_element(&self, block_id: &str, state: &Arc<AppState>, x: i32, y: i32) {
        if let Some(mut b) = self.live_browser(state, block_id) {
            if let Some(host) = b.host() {
                host.show_dev_tools(None, None, None, Some(&Point { x, y }));
            }
        }
    }

    /// "Copy" / "Cut" / "Paste" from the unified browser-pane context menu.
    /// Added because suppressing CEF's native menu (SPEC_BROWSER_PANE_UNIFIED_CONTEXT_MENU_2026_08_15.md)
    /// also removed its built-in Cut/Copy/Paste for a text selection or an
    /// editable web form field, with nothing replacing them (reagentx P2 on
    /// PR #2599). `Frame::copy/cut/paste` operate on whatever is currently
    /// selected/focused in that frame, exactly like CEF's own native menu
    /// commands would have.
    ///
    /// Uses `state.browser_pane_context_menu_frame`'s stashed frame — the
    /// ACTUAL frame `client::context_menu::run_context_menu` saw the
    /// right-click land on — instead of unconditionally
    /// `b.main_frame()`. For a page with sub-frames/iframes (ads, embeds,
    /// widgets), the selection/focus can live in a sub-frame; acting on
    /// `main_frame()` there would silently no-op or paste into the wrong
    /// place (reagentx P1, second finding on PR #2599). Falls back to
    /// `main_frame()` only if nothing was stashed (e.g. this pane never had
    /// its context menu invoked in this session).
    fn edit_target_frame(&self, block_id: &str, state: &Arc<AppState>, browser: &mut Browser) -> Option<Frame> {
        state
            .browser_pane_context_menu_frame
            .lock()
            .get(block_id)
            .cloned()
            .or_else(|| browser.main_frame())
    }
    pub fn copy(&self, block_id: &str, state: &Arc<AppState>) {
        if let Some(mut b) = self.live_browser(state, block_id) {
            if let Some(frame) = self.edit_target_frame(block_id, state, &mut b) { frame.copy(); }
        }
    }
    pub fn cut(&self, block_id: &str, state: &Arc<AppState>) {
        if let Some(mut b) = self.live_browser(state, block_id) {
            if let Some(frame) = self.edit_target_frame(block_id, state, &mut b) { frame.cut(); }
        }
    }
    pub fn paste(&self, block_id: &str, state: &Arc<AppState>) {
        if let Some(mut b) = self.live_browser(state, block_id) {
            if let Some(frame) = self.edit_target_frame(block_id, state, &mut b) { frame.paste(); }
        }
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
