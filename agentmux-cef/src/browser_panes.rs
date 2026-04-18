// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! BrowserPaneManager: embeds browsers as native OS child windows using
//! CefBrowserHost::CreateBrowser. All creation runs on the CEF UI thread.
//!
//! The Browser instance is owned by `state.browsers` (keyed by label). We only
//! store the block_id -> label mapping here; look up the browser when needed.
//!
//! Lifecycle states are tracked explicitly via `PaneLifecycle`:
//!   Created → Closing → Closed (removed from `panes`)
//! Every pane-facing op (focus/resize/navigate/…) short-circuits when the
//! entry is already in `Closing`. This drops late IPC that the frontend
//! fires after it has already asked for close but before CEF has destroyed
//! the Browser — stale IPC against a mid-destruction HWND is the shape of
//! the crash described in `docs/specs/SPEC_BROWSER_PANE_LIFECYCLE.md` §4c.

use std::collections::HashMap;
use std::sync::Arc;

use cef::*;
use parking_lot::Mutex;

use crate::state::AppState;

/// Per-pane lifecycle phase. Simplified from the full state machine in
/// `SPEC_BROWSER_PANE_LIFECYCLE.md` §6 — the pre-create states are handled
/// by the panes-map-absent sentinel so we only need to distinguish live
/// from closing here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PaneLifecycle {
    /// Browser requested; CEF may still be creating it. Ops proceed because
    /// the Browser will be present in `state.browsers` by the time the IPC
    /// reaches it (or the lookup miss is harmless).
    Live,
    /// `close()` has been called. All further IPC for this pane must no-op.
    /// The entry stays in `panes` until `on_before_close` drains it via
    /// `drain_closed_label` so concurrent `defocus_all` / `resize` see the
    /// Closing flag and skip, rather than seeing a stale browser ref.
    Closing,
}

struct PaneEntry {
    label: String,
    state: PaneLifecycle,
}

pub struct BrowserPaneManager {
    panes: Mutex<HashMap<String, PaneEntry>>,
}

impl BrowserPaneManager {
    pub fn new() -> Self {
        Self { panes: Mutex::new(HashMap::new()) }
    }

    fn label_for(&self, block_id: &str) -> Option<String> {
        self.panes.lock().get(block_id).map(|e| e.label.clone())
    }

    /// Look up the Browser iff the pane is Live. Returns None when closing
    /// so all ops short-circuit uniformly.
    fn live_browser(&self, state: &Arc<AppState>, block_id: &str) -> Option<Browser> {
        let label = {
            let panes = self.panes.lock();
            let entry = panes.get(block_id)?;
            if entry.state != PaneLifecycle::Live {
                return None;
            }
            entry.label.clone()
        };
        state.browsers.lock().get(&label).cloned()
    }

    pub fn create(
        &self,
        state: &Arc<AppState>,
        block_id: &str,
        url: &str,
        rect: Rect,
    ) -> Result<(), String> {
        if let Some(browser) = self.live_browser(state, block_id) {
            if let Some(frame) = browser.main_frame() {
                frame.load_url(Some(&CefString::from(url)));
            }
            return Ok(());
        }

        let label = format!("browser-pane-{}", block_id);
        self.panes.lock().insert(
            block_id.to_string(),
            PaneEntry { label: label.clone(), state: PaneLifecycle::Live },
        );

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
        if let Some(browser) = self.live_browser(state, block_id) {
            if let Some(frame) = browser.main_frame() {
                frame.load_url(Some(&CefString::from(url)));
            }
        }
        Ok(())
    }

    pub fn resize(&self, block_id: &str, rect: Rect, state: &Arc<AppState>) {
        if let Some(browser) = self.live_browser(state, block_id) {
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
        // Flip to Closing first so any concurrent focus/resize/navigate sees
        // the flag and bails before touching the Browser. The entry stays in
        // `panes` until CEF's on_before_close fires drain_closed_label — the
        // Browser stays in state.browsers during that window so on_before_close
        // can still find it by is_same() and run its unified cleanup path
        // (backend_close_window, window_instance_registry, etc).
        let label = {
            let mut panes = self.panes.lock();
            let entry = match panes.get_mut(block_id) {
                Some(e) => e,
                None => return, // already closed/never created
            };
            if entry.state == PaneLifecycle::Closing {
                return; // close already in flight
            }
            entry.state = PaneLifecycle::Closing;
            entry.label.clone()
        };

        let browser = state.browsers.lock().get(&label).cloned();
        if let Some(browser) = browser {
            if let Some(host) = browser.host() {
                // force_close=0 (graceful) — force_close=1 used to cascade into
                // the main browser's on_before_close and quit the app, which the
                // `!is_pane` guard at client.rs on_before_close now prevents.
                // Graceful close is still preferred so Chromium runs its full
                // teardown (beforeunload, GPU surface release) in order.
                host.close_browser(0);
            }
            tracing::info!(block_id, label, "browser pane close requested");
        } else {
            // Browser never made it into state.browsers (creation failed or
            // still on UI thread). Drop the panes entry; nothing to close.
            self.panes.lock().remove(block_id);
        }
    }

    /// Called from CEF's `on_before_close` once the pane's Browser has been
    /// removed from `state.browsers`. Drops the `panes` entry so the block_id
    /// can be reused if a new pane is created with the same id.
    pub fn drain_closed_label(&self, label: &str) {
        let mut panes = self.panes.lock();
        let victim = panes
            .iter()
            .find(|(_, e)| e.label == label)
            .map(|(k, _)| k.clone());
        if let Some(block_id) = victim {
            panes.remove(&block_id);
            tracing::info!(block_id, label, "browser pane drained from lifecycle map");
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

    /// Tell every live pane browser it has lost focus, at the Chromium level.
    /// Panes in `Closing` are skipped — their HWND may be mid-destruction and
    /// `set_focus(0)` against it can hit an invalid render widget.
    pub fn defocus_all(&self, state: &Arc<AppState>) {
        let labels: Vec<String> = self
            .panes
            .lock()
            .values()
            .filter(|e| e.state == PaneLifecycle::Live)
            .map(|e| e.label.clone())
            .collect();
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
    ///
    /// No-ops if the pane is `Closing`: a SetFocus against a HWND that CEF is
    /// concurrently tearing down is the exact race documented in
    /// `SPEC_BROWSER_PANE_LIFECYCLE.md` §5 race #2.
    pub fn focus(&self, block_id: &str, state: &Arc<AppState>) {
        if let Some(browser) = self.live_browser(state, block_id) {
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
