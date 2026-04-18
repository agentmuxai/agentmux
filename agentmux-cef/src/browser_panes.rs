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
use std::sync::atomic::{AtomicU64, Ordering};

use cef::*;
use parking_lot::Mutex;

use crate::state::AppState;

/// Monotonic counter appended to every pane label so a close-then-recreate of
/// the same block_id doesn't collide: if the old browser's `on_before_close`
/// fires after the new pane's `create()` has already run, `drain_closed_label`
/// would otherwise find and wipe the NEW entry.
static PANE_LABEL_SEQ: AtomicU64 = AtomicU64::new(1);

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

    // ---- test-only helpers ----
    // Exposed for the unit tests below. Kept out of the production surface
    // so real callers can't stumble into manipulating lifecycle state.
    #[cfg(test)]
    fn test_has_entry(&self, block_id: &str) -> bool {
        self.panes.lock().contains_key(block_id)
    }

    #[cfg(test)]
    fn test_entry_state(&self, block_id: &str) -> Option<PaneLifecycle> {
        self.panes.lock().get(block_id).map(|e| e.state)
    }

    #[cfg(test)]
    fn test_entry_label(&self, block_id: &str) -> Option<String> {
        self.panes.lock().get(block_id).map(|e| e.label.clone())
    }

    #[cfg(test)]
    fn test_insert_live(&self, block_id: &str, label: &str) {
        self.panes.lock().insert(
            block_id.to_string(),
            PaneEntry { label: label.to_string(), state: PaneLifecycle::Live },
        );
    }

    #[cfg(test)]
    fn test_mark_closing(&self, block_id: &str) {
        if let Some(entry) = self.panes.lock().get_mut(block_id) {
            entry.state = PaneLifecycle::Closing;
        }
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
        // Existing entry: navigate if Live; reject if Closing. Reject (rather
        // than overwrite) because the old CEF Browser is mid-teardown and its
        // on_before_close will call drain_closed_label — if we let create()
        // overwrite the map entry, the drain would evict the NEW entry. The
        // frontend is expected to retry after a short delay; in practice
        // block_ids are unique-per-create so this race is already rare.
        {
            let panes = self.panes.lock();
            if let Some(entry) = panes.get(block_id) {
                match entry.state {
                    PaneLifecycle::Live => {
                        // drop lock before touching CEF
                        let label = entry.label.clone();
                        drop(panes);
                        if let Some(browser) = state.browsers.lock().get(&label).cloned() {
                            if let Some(frame) = browser.main_frame() {
                                frame.load_url(Some(&CefString::from(url)));
                            }
                        }
                        return Ok(());
                    }
                    PaneLifecycle::Closing => {
                        return Err(format!(
                            "browser pane for block_id={} is still closing; retry after on_before_close",
                            block_id
                        ));
                    }
                }
            }
        }

        // Monotonic seq so close-then-recreate of the same block_id gets a
        // unique label — drain_closed_label on the old pane won't match us.
        let seq = PANE_LABEL_SEQ.fetch_add(1, Ordering::Relaxed);
        let label = format!("browser-pane-{}-{}", block_id, seq);
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

    /// Close a pane by destroying its child HWND directly and dropping the
    /// Browser Arc.
    ///
    /// We deliberately do **not** call `host.close_browser(force)`. Empirically
    /// (host-log trace v0.33.251 and v0.33.252 in `SPEC_BROWSER_PANE_LIFECYCLE.md`
    /// §4), CEF Alloy treats the pane Browser and the main Browser as a single
    /// close unit when the pane's outer HWND is a child of main's top-level:
    /// `close_browser(pane)` fires `do_close` on main too. Previous attempts
    /// (force=0, force=1, a cascade-guard cancelling main's do_close) either
    /// quit the whole app or orphaned the pane's pixels while blocking the
    /// pane's own teardown.
    ///
    /// Instead:
    /// 1. Remove the Browser from `state.browsers` so subsequent lookups miss.
    /// 2. Win32 `DestroyWindow` on the pane's outer HWND. The pane HWND is a
    ///    `WS_CHILD`; `WM_DESTROY` cascades to descendants only, never to the
    ///    parent. Main stays up.
    /// 3. Drop our `Browser` Arc. CEF still holds refs (browser_list etc.);
    ///    `on_before_close` *may* eventually fire on the now-destroyed Browser,
    ///    which is why `drain_closed_label` is idempotent.
    ///
    /// Trade-off: because we bypass `close_browser`, Chromium's `beforeunload`
    /// handler doesn't run. Acceptable for a browser pane (no form data the
    /// user expects to persist across close). If beforeunload becomes
    /// important, revisit.
    pub fn close(&self, block_id: &str, state: &Arc<AppState>) {
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

        // Remove from state.browsers up front so focus/resize/defocus_all
        // lookups miss immediately, in addition to the Closing gate.
        let browser = state.browsers.lock().remove(&label);

        if let Some(browser) = browser {
            #[cfg(target_os = "windows")]
            let hwnd = browser.host().and_then(|h| {
                let wh = h.window_handle();
                if wh.0.is_null() {
                    None
                } else {
                    Some(wh.0 as *mut std::ffi::c_void)
                }
            });

            // Drop our Arc before DestroyWindow so Chromium's refcount doesn't
            // have to wait for this scope to exit.
            drop(browser);

            #[cfg(target_os = "windows")]
            if let Some(hwnd) = hwnd {
                unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::DestroyWindow(hwnd as _);
                }
                tracing::info!(block_id, label, "pane HWND destroyed");
            }
        }

        // Drop the lifecycle entry now so the block_id can be reused. If CEF
        // does eventually fire on_before_close, `drain_closed_label` will find
        // no matching label and no-op.
        self.panes.lock().remove(block_id);
        tracing::info!(block_id, label, "browser pane lifecycle entry cleared");
    }

    /// Called from CEF's `on_before_close` if/when it fires for a pane
    /// browser. The new DestroyWindow-based `close()` usually clears the
    /// lifecycle entry first, so this is a no-op in that case — but
    /// `on_before_close` may still fire async as Chromium's refcount hits
    /// zero, and we need to stay idempotent so the callback doesn't panic.
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

// ── Tests ───────────────────────────────────────────────────────────────────
//
// Covers the non-CEF pieces of `BrowserPaneManager`: the panes map, label
// sequencing, and lifecycle-state transitions. CEF-touching paths (`create`'s
// browser_host_create_browser, `close`'s DestroyWindow, Browser clone/drop)
// need a `PaneCefBridge` trait extraction — deferred to the follow-up PR
// described in SPEC_BROWSER_PANE_LIFECYCLE_TESTS.md §9.

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_manager_is_empty() {
        let m = BrowserPaneManager::new();
        assert!(!m.test_has_entry("any-block"));
    }

    #[test]
    fn drain_closed_label_noop_on_missing() {
        let m = BrowserPaneManager::new();
        // Must not panic, must not corrupt state.
        m.drain_closed_label("nonexistent-label");
        assert!(!m.test_has_entry("nonexistent-block"));
    }

    #[test]
    fn drain_closed_label_removes_matching_entry() {
        let m = BrowserPaneManager::new();
        m.test_insert_live("b1", "browser-pane-b1-1");

        m.drain_closed_label("browser-pane-b1-1");

        assert!(!m.test_has_entry("b1"));
    }

    #[test]
    fn drain_closed_label_ignores_unrelated_labels() {
        let m = BrowserPaneManager::new();
        m.test_insert_live("b1", "browser-pane-b1-1");
        m.test_insert_live("b2", "browser-pane-b2-2");

        m.drain_closed_label("browser-pane-other-99");

        assert!(m.test_has_entry("b1"));
        assert!(m.test_has_entry("b2"));
    }

    #[test]
    fn drain_closed_label_idempotent() {
        // DestroyWindow-based close() removes the entry up front, but CEF may
        // still fire on_before_close afterward. Calling drain twice must not
        // panic.
        let m = BrowserPaneManager::new();
        m.test_insert_live("b1", "browser-pane-b1-1");

        m.drain_closed_label("browser-pane-b1-1");
        m.drain_closed_label("browser-pane-b1-1");

        assert!(!m.test_has_entry("b1"));
    }

    #[test]
    fn label_sequence_is_monotonic() {
        // Three inserts via test_insert_live don't exercise PANE_LABEL_SEQ
        // directly — we inspect the counter by reading PANE_LABEL_SEQ before
        // and after create-like operations. Since PANE_LABEL_SEQ is static
        // and advances monotonically for every create() call in the process,
        // we simply verify the counter keeps growing.
        use std::sync::atomic::Ordering;
        let before = PANE_LABEL_SEQ.load(Ordering::Relaxed);
        let a = PANE_LABEL_SEQ.fetch_add(1, Ordering::Relaxed);
        let b = PANE_LABEL_SEQ.fetch_add(1, Ordering::Relaxed);
        let c = PANE_LABEL_SEQ.fetch_add(1, Ordering::Relaxed);
        assert!(a >= before);
        assert_eq!(b, a + 1);
        assert_eq!(c, b + 1);
    }

    #[test]
    fn state_transitions_live_to_closing() {
        let m = BrowserPaneManager::new();
        m.test_insert_live("b1", "browser-pane-b1-1");
        assert_eq!(m.test_entry_state("b1"), Some(PaneLifecycle::Live));

        m.test_mark_closing("b1");
        assert_eq!(m.test_entry_state("b1"), Some(PaneLifecycle::Closing));
    }

    #[test]
    fn drain_after_transition_still_removes() {
        // close() flips to Closing, then DestroyWindow, then drain. The
        // drain must remove the entry regardless of the Closing flag.
        let m = BrowserPaneManager::new();
        m.test_insert_live("b1", "browser-pane-b1-1");
        m.test_mark_closing("b1");

        m.drain_closed_label("browser-pane-b1-1");

        assert!(!m.test_has_entry("b1"));
    }

    #[test]
    fn entry_label_is_what_we_inserted() {
        let m = BrowserPaneManager::new();
        m.test_insert_live("b1", "custom-label-42");
        assert_eq!(m.test_entry_label("b1").as_deref(), Some("custom-label-42"));
    }
}
