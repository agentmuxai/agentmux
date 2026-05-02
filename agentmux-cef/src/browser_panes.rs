// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! BrowserPaneManager: embeds browsers as native OS child windows using
//! CefBrowserHost::CreateBrowser. All creation runs on the CEF UI thread.
//!
//! The Browser instance is owned by the host reducer's `browsers` map (keyed
//! by label, accessed via `AppState::get_browser` etc). We only need the
//! block_id → label mapping, which also lives in the reducer's `panes` map.
//!
//! Lifecycle states are tracked explicitly via the reducer's `PaneLifecycle`:
//!   Created → Closing → Closed (removed from `panes`)
//! Every pane-facing op (focus/resize/navigate/…) short-circuits when the
//! entry is already in `Closing`. This drops late IPC that the frontend
//! fires after it has already asked for close but before CEF has destroyed
//! the Browser — stale IPC against a mid-destruction HWND is the shape of
//! the crash described in `docs/specs/SPEC_BROWSER_PANE_LIFECYCLE.md` §4c.
//!
//! **Phase H.1.d/e (PR #5):** The legacy `pane::lifecycle::PaneStateMachine`
//! is gone; pane state lives only in `HostState.panes`. All mutations go
//! through `HostCommand::TryRegisterPaneLive` / `EnqueuePaneClose` /
//! `CompletePaneClose` / `DrainPaneByLabel` and read back via the reducer's
//! atomic `DispatchOutput` fields.

use std::sync::Arc;

use cef::*;

use crate::pane::CreatePaneTask;
use crate::reducer::RegisterResult;
use crate::state::AppState;

/// Abstraction over the CEF-side operations `close()` performs on the host
/// process. Production implements this over `&Arc<AppState>` and Win32;
/// tests implement it with a recording mock so the close-path state machine
/// can be exercised without real CEF/HWNDs.
///
/// Kept minimal (just two methods) to avoid a dependency graph that has to
/// be updated every time `close()` grows. Other ops (`focus`, `resize`, …)
/// can gain their own traits when they need testing, or graduate to a
/// unified `PaneCefBridge` once the shape is stable.
pub trait PaneCloseOps {
    /// Remove the Browser for this label from the registry. Return its
    /// outer HWND as a pointer-sized value, or `None` if there is no
    /// Browser or no HWND. Dropping the Browser Arc is the implementation's
    /// responsibility — production drops before returning so Chromium's
    /// refcount isn't held by our scope.
    fn take_browser_hwnd(&self, label: &str) -> Option<usize>;

    /// Destroy the given HWND. Production calls Win32 `DestroyWindow`.
    /// Called only with values returned from `take_browser_hwnd`.
    fn destroy_hwnd(&self, hwnd: usize);
}

/// Production implementation of `PaneCloseOps` backed by `AppState.browsers`
/// and Win32 `DestroyWindow`.
struct AppStateCloseOps<'a>(&'a Arc<AppState>);

impl<'a> PaneCloseOps for AppStateCloseOps<'a> {
    fn take_browser_hwnd(&self, label: &str) -> Option<usize> {
        // Atomic take-and-return via reducer (codex P2 PR #660). Earlier
        // round 1 separated `get_browser` + `UnregisterBrowser` dispatch,
        // which left a window for concurrent readers to resolve the same
        // label and act on the closing handle. `UnregisterBrowser` now
        // returns the removed `Browser` via `DispatchOutput.removed_browser`
        // — single host_state lock, single mutation, no race.
        let out = self.0.host_dispatch(
            crate::reducer::HostCommand::UnregisterBrowser {
                label: label.to_string(),
            },
        );
        let browser = out.removed_browser?;

        #[cfg(target_os = "windows")]
        let hwnd = browser.host().and_then(|h| {
            let wh = h.window_handle();
            if wh.0.is_null() {
                None
            } else {
                Some(wh.0 as usize)
            }
        });
        #[cfg(not(target_os = "windows"))]
        let hwnd: Option<usize> = None;

        // Drop our Arc before returning so Chromium's refcount doesn't wait
        // for the caller's scope to unwind.
        drop(browser);
        hwnd
    }

    fn destroy_hwnd(&self, hwnd: usize) {
        #[cfg(target_os = "windows")]
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                DestroyWindow, GetParent, ShowWindow, SW_HIDE,
            };
            use windows_sys::Win32::Graphics::Gdi::{InvalidateRect, UpdateWindow};
            let h = hwnd as *mut std::ffi::c_void;

            // Capture the parent BEFORE we destroy the HWND — GetParent(h)
            // on a destroyed HWND returns null.
            let parent = GetParent(h);

            // Hide first so DWM stops compositing the pane's GPU surface.
            // Without this, even after DestroyWindow the Chromium compositor's
            // last-rendered frame can stay "stuck" on-screen because the GPU
            // process is still alive and DWM was caching that layer. Observed
            // in v0.33.259 with a loaded google.com pane — close fires,
            // lifecycle entry clears, HWND is gone — but the page pixels
            // persist over the main frame until a resize/redraw.
            ShowWindow(h, SW_HIDE);

            DestroyWindow(h);

            // Ask the parent (main's top-level) to repaint the area where the
            // pane used to sit. Without InvalidateRect + UpdateWindow, DWM
            // may keep showing the cached pane surface until unrelated UI
            // activity happens to repaint over it.
            if !parent.is_null() {
                InvalidateRect(parent, std::ptr::null(), 1 /* TRUE erase */);
                UpdateWindow(parent);
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            let _ = hwnd;
        }
    }
}

pub struct BrowserPaneManager;

impl BrowserPaneManager {
    pub fn new() -> Self {
        Self
    }

    /// Look up the Browser iff the pane is Live. Returns None when closing
    /// so all ops short-circuit uniformly.
    fn live_browser(&self, state: &Arc<AppState>, block_id: &str) -> Option<Browser> {
        let label = state.live_pane_label(block_id)?;
        state.get_browser(&label)
    }

    /// Return the current URL of the pane's main frame, if the pane
    /// is Live. Used by the browser DOM API resolver
    /// (`crate::browser_api::resolver`) to match CEF `/json` targets
    /// against block ids without a first-class `browserId` field on
    /// the CEF side.
    pub fn pane_url(&self, state: &Arc<AppState>, block_id: &str) -> Option<String> {
        let browser = self.live_browser(state, block_id)?;
        let frame = browser.main_frame()?;
        Some(CefString::from(&frame.url()).to_string())
    }

    pub fn create(
        &self,
        state: &Arc<AppState>,
        block_id: &str,
        url: &str,
        rect: Rect,
    ) -> Result<(), String> {
        // Phase H.1.d (PR #5) — sole pane-registration entry point. The
        // reducer atomically generates the label and inserts the entry,
        // returning Fresh / AlreadyLive / Closing via DispatchOutput.
        let out = state.host_dispatch(
            crate::reducer::HostCommand::TryRegisterPaneLive {
                block_id: block_id.to_string(),
            },
        );
        let result = out.pane_register_result.ok_or_else(|| {
            format!(
                "try_register_pane_live returned no result (block_id={}); host shutting down?",
                block_id
            )
        })?;
        match result {
            RegisterResult::AlreadyLive(label) => {
                // Existing Live entry — re-navigate the existing browser.
                if let Some(browser) = state.get_browser(&label) {
                    if let Some(frame) = browser.main_frame() {
                        frame.load_url(Some(&CefString::from(url)));
                    }
                }
                Ok(())
            }
            RegisterResult::Closing => {
                // Reject rather than overwrite: the old CEF Browser is
                // mid-teardown and its on_before_close will call
                // DrainPaneByLabel — if we let create overwrite, drain
                // would evict the NEW entry. Frontend retries on next tick.
                Err(format!(
                    "browser pane for block_id={} is still closing; retry after on_before_close",
                    block_id
                ))
            }
            RegisterResult::Fresh(label) => {
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
        }
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
        // Phase H.1.d (PR #5) — sole pane-close entry point. The reducer
        // flips Live→Closing atomically and returns the entry's label iff
        // the transition fired. None means missing or already-Closing —
        // both idempotent no-ops; we don't dispatch CompletePaneClose in
        // those cases (codex P2 PR #655 race), avoiding the entry removal
        // while another in-flight close is still tearing down the HWND.
        let close_out = state.host_dispatch(
            crate::reducer::HostCommand::EnqueuePaneClose {
                block_id: block_id.to_string(),
            },
        );
        let label = match close_out.closed_pane_label {
            Some(l) => l,
            None => return,
        };
        let ops = AppStateCloseOps(state);
        Self::close_with(&label, &ops);
        state.host_dispatch(
            crate::reducer::HostCommand::CompletePaneClose {
                block_id: block_id.to_string(),
            },
        );
        tracing::info!(block_id, label, "browser pane closed");
    }

    /// The testable side-effect body of `close()`. Given a pane's `label`,
    /// remove its Browser handle and destroy its HWND. The state-machine
    /// transition (Live→Closing) and the entry removal (CompletePaneClose)
    /// happen in `close()` via reducer dispatch — `close_with` is purely
    /// the FFI side-effects that follow.
    fn close_with(label: &str, ops: &dyn PaneCloseOps) {
        if let Some(hwnd) = ops.take_browser_hwnd(label) {
            ops.destroy_hwnd(hwnd);
            tracing::info!(label, "pane HWND destroyed");
        }
    }

    /// Called from CEF's `on_before_close` if/when it fires for a pane
    /// browser. The explicit `close()` path usually clears the entry first,
    /// so this is a no-op in that case — but `on_before_close` may still
    /// fire async as Chromium's refcount hits zero, and `DrainPaneByLabel`
    /// is idempotent so the callback is safe.
    pub fn drain_closed_label(&self, state: &Arc<AppState>, label: &str) {
        let out = state.host_dispatch(
            crate::reducer::HostCommand::DrainPaneByLabel {
                label: label.to_string(),
            },
        );
        if let Some(block_id) = out.drained_block_id {
            tracing::info!(label, block_id = %block_id, "browser pane drained via on_before_close");
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
        // Phase H.1.b + H.2.b — read live labels via reducer-aware helper,
        // then look up each browser via reducer-aware helper. Both with
        // fallback + drift logging.
        let labels = state.live_pane_labels();
        for label in &labels {
            if let Some(browser) = state.get_browser(label) {
                if let Some(host) = browser.host() {
                    host.set_focus(0);
                }
            }
        }
    }

    /// Apply a clip region to every live pane HWND that subtracts the given
    /// overlay rects (in main-window client coordinates). The pane renders
    /// normally outside the overlay region; inside it, the HWND is
    /// transparent so the DOM overlay painted at the same screen position
    /// shows through.
    ///
    /// This is the Win32 "airspace" workaround — native HWNDs always paint
    /// above DOM regardless of CSS z-index, and `SetWindowRgn` is the one
    /// mechanism that lets DOM bleed through a specific region of a child
    /// HWND. Empty `overlay_rects` restores full pane visibility (same as
    /// calling `clear_pane_overlay_clip`).
    ///
    /// No-op on non-Windows: other platforms don't use native child HWNDs
    /// for panes, so there's no airspace to work around.
    ///
    /// `window_label` scopes the clip to panes whose top-level ancestor
    /// matches the requesting window. Without it, a modal opened in
    /// window B would clip panes in window A (see Codex P1 on PR #544).
    /// Empty string matches today's legacy callers that don't know their
    /// window label — falls through to the no-filter behaviour for
    /// back-compat until every caller is updated.
    #[cfg(target_os = "windows")]
    pub fn set_pane_overlay_clip(
        &self,
        state: &Arc<AppState>,
        window_label: &str,
        overlay_rects: &[(i32, i32, i32, i32)],
    ) {
        use windows_sys::Win32::Foundation::{POINT, RECT};
        use windows_sys::Win32::Graphics::Gdi::{
            CombineRgn, CreateRectRgn, DeleteObject, MapWindowPoints, SetWindowRgn, RGN_DIFF,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetAncestor, GetParent, GetWindowRect, GA_ROOT,
        };

        // Resolve the requesting window's top-level HWND so we can filter
        // panes by ownership. If the label is unknown we fall through with
        // no filter — matches pre-scoping behaviour rather than silently
        // doing nothing.
        let requesting_top_level: *mut std::ffi::c_void = if window_label.is_empty() {
            std::ptr::null_mut()
        } else {
            // Phase H.2.b — reducer-aware lookup with fallback.
            match state.get_browser(window_label).and_then(|b| b.host()) {
                Some(host) => {
                    let h = host.window_handle();
                    if h.0.is_null() {
                        std::ptr::null_mut()
                    } else {
                        unsafe { GetAncestor(h.0 as _, GA_ROOT) as *mut std::ffi::c_void }
                    }
                }
                None => std::ptr::null_mut(),
            }
        };

        // Phase H.1.b + H.2.b — labels via reducer-aware helper; per-label
        // browser lookup via reducer-aware helper. Drops the held-across-loop
        // legacy lock; each iteration now snapshots independently.
        let labels = state.live_pane_labels();
        for label in &labels {
            let browser = match state.get_browser(label) {
                Some(b) => b,
                None => continue,
            };
            let host = match browser.host() {
                Some(h) => h,
                None => continue,
            };
            let hwnd_raw = host.window_handle();
            if hwnd_raw.0.is_null() {
                continue;
            }
            let pane_hwnd = hwnd_raw.0 as *mut std::ffi::c_void;

            // Window-scope filter. Skip panes whose top-level HWND differs
            // from the requesting window's. `null` requesting = legacy
            // caller / no-op filter (applies to all panes).
            if !requesting_top_level.is_null() {
                let pane_top = unsafe { GetAncestor(pane_hwnd as _, GA_ROOT) as *mut std::ffi::c_void };
                if pane_top != requesting_top_level {
                    continue;
                }
            }

            unsafe {
                // Empty overlay list = restore full visibility (region=NULL).
                if overlay_rects.is_empty() {
                    SetWindowRgn(pane_hwnd as _, std::ptr::null_mut(), 1);
                    continue;
                }

                // Resolve the pane's position in its parent (main window)
                // client coords so we can translate overlay rects (which
                // arrive in main-window client coords from the frontend)
                // into pane-local coords for the region API.
                let parent = GetParent(pane_hwnd as _);
                if parent.is_null() {
                    continue;
                }
                let mut pane_rect: RECT = std::mem::zeroed();
                if GetWindowRect(pane_hwnd as _, &mut pane_rect) == 0 {
                    continue;
                }
                // Convert pane_rect from screen coords to parent client
                // coords by mapping its two corner points.
                let pts_ptr = &mut pane_rect as *mut RECT as *mut POINT;
                MapWindowPoints(std::ptr::null_mut(), parent, pts_ptr, 2);

                let pane_w = pane_rect.right - pane_rect.left;
                let pane_h = pane_rect.bottom - pane_rect.top;
                if pane_w <= 0 || pane_h <= 0 {
                    continue;
                }

                // Build region in pane-local coords: start with full pane,
                // subtract every overlay rect that intersects it.
                let region = CreateRectRgn(0, 0, pane_w, pane_h);
                if region.is_null() {
                    continue;
                }
                for (ox, oy, ow, oh) in overlay_rects {
                    // Translate overlay rect (window client coords) →
                    // pane-local coords by subtracting pane's window pos.
                    let left = ox - pane_rect.left;
                    let top = oy - pane_rect.top;
                    let right = left + ow;
                    let bottom = top + oh;
                    // Skip if no intersection with the pane's local bounds.
                    if right <= 0 || bottom <= 0 || left >= pane_w || top >= pane_h {
                        continue;
                    }
                    let overlay_rgn = CreateRectRgn(left, top, right, bottom);
                    if !overlay_rgn.is_null() {
                        CombineRgn(region, region, overlay_rgn, RGN_DIFF);
                        DeleteObject(overlay_rgn as _);
                    }
                }
                // SetWindowRgn takes ownership of the region handle on
                // success; the system frees it when the window is destroyed
                // or a new region is set.
                SetWindowRgn(pane_hwnd as _, region as _, 1);
            }
        }
        tracing::info!(
            pane_count = labels.len(),
            overlay_count = overlay_rects.len(),
            "[pane-airspace] applied overlay clip to pane HWNDs",
        );
    }
    #[cfg(not(target_os = "windows"))]
    pub fn set_pane_overlay_clip(
        &self,
        _state: &Arc<AppState>,
        _window_label: &str,
        _overlay_rects: &[(i32, i32, i32, i32)],
    ) {
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
                        crate::pane::ALLOW_PANE_FOCUS_ONCE.store(
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

// `CreatePaneTask` moved to `crate::pane::creation` in Phase 3.

// ── Tests ───────────────────────────────────────────────────────────────────
//
// Phase H.1.d/e (PR #5): The pane state machine lives in the host reducer
// (`HostState.panes`). Lifecycle transition tests — Live→Closing, idempotent
// no-ops for missing or already-Closing entries, label sequence monotonicity,
// drain-by-label — are now in `crate::reducer::tests`.
//
// What remains here: the FFI seam. `close_with` only takes a label and
// drives `PaneCloseOps`; tests verify it forwards label → take → destroy
// in order, with a None-returning `take` short-circuiting the destroy.

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    /// Recording mock for `PaneCloseOps`. Tests inspect `taken` and
    /// `destroyed` to assert what close_with did.
    struct MockCloseOps {
        registered: parking_lot::Mutex<HashMap<String, usize>>,
        taken: parking_lot::Mutex<Vec<String>>,
        destroyed: parking_lot::Mutex<Vec<usize>>,
    }

    impl MockCloseOps {
        fn new() -> Self {
            Self {
                registered: parking_lot::Mutex::new(HashMap::new()),
                taken: parking_lot::Mutex::new(Vec::new()),
                destroyed: parking_lot::Mutex::new(Vec::new()),
            }
        }

        fn register(&self, label: &str, hwnd: usize) {
            self.registered.lock().insert(label.to_string(), hwnd);
        }

        fn taken_labels(&self) -> Vec<String> {
            self.taken.lock().clone()
        }

        fn destroyed_hwnds(&self) -> Vec<usize> {
            self.destroyed.lock().clone()
        }
    }

    impl PaneCloseOps for MockCloseOps {
        fn take_browser_hwnd(&self, label: &str) -> Option<usize> {
            self.taken.lock().push(label.to_string());
            self.registered.lock().remove(label)
        }

        fn destroy_hwnd(&self, hwnd: usize) {
            self.destroyed.lock().push(hwnd);
        }
    }

    #[test]
    fn close_with_take_then_destroy_in_order() {
        let ops = MockCloseOps::new();
        ops.register("browser-pane-b1-1", 0xABCD);

        BrowserPaneManager::close_with("browser-pane-b1-1", &ops);

        assert_eq!(ops.taken_labels(), vec!["browser-pane-b1-1"]);
        assert_eq!(ops.destroyed_hwnds(), vec![0xABCD]);
    }

    #[test]
    fn close_with_no_hwnd_skips_destroy() {
        // Browser was already gone (rare race — explicit close raced with
        // an external close). take returns None; destroy must NOT be called.
        let ops = MockCloseOps::new(); // no register() — lookup will miss

        BrowserPaneManager::close_with("browser-pane-missing", &ops);

        assert_eq!(ops.taken_labels(), vec!["browser-pane-missing"]);
        assert!(ops.destroyed_hwnds().is_empty(),
            "destroy_hwnd must not be called when take_browser_hwnd returns None");
    }
}
