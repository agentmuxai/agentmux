// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! BrowserPaneManager: embeds browsers as native OS child windows using
//! CefBrowserHost::CreateBrowser. All creation runs on the CEF UI thread.
//!
//! The Browser instance is owned by the host reducer's `browsers` map (keyed
//! by label, accessed via `AppState::get_browser` etc). We only need the
//! block_id → label mapping, which also lives in the reducer's `panes` map.
//!
//! Lifecycle states are tracked explicitly via the reducer's `BrowserPaneLifecycle`:
//!   Created → Closing → Closed (removed from `panes`)
//! Every pane-facing op (focus/resize/navigate/…) short-circuits when the
//! entry is already in `Closing`. This drops late IPC that the frontend
//! fires after it has already asked for close but before CEF has destroyed
//! the Browser — stale IPC against a mid-destruction HWND is the shape of
//! the crash described in `docs/specs/SPEC_BROWSER_PANE_LIFECYCLE.md` §4c.
//!
//! **Phase H.1.d/e (PR #5):** The legacy `pane::lifecycle::PaneStateMachine`
//! is gone; pane state lives only in `HostState.browser_panes`. All mutations go
//! through `HostCommand::TryRegisterBrowserPaneLive` / `EnqueueBrowserPaneClose` /
//! `CompleteBrowserPaneClose` / `DrainBrowserPaneByLabel` and read back via the reducer's
//! atomic `DispatchOutput` fields.
//!
//! This module is split across files by operation group (mechanical split —
//! `BrowserPaneManager` is a zero-field unit struct, so every method takes
//! `state: &Arc<AppState>` explicitly and there is no shared private struct
//! state to worry about; Rust allows multiple `impl BrowserPaneManager`
//! blocks across files/modules):
//! - `mod.rs` (this file): struct definition, `BrowserPaneCloseOps` trait,
//!   `AppStateCloseOps` impl, and pane creation.
//! - `close`: close/lifecycle (`close`, `close_with`, `drain_closed_label`,
//!   `replay_pending_create`, `reclaim_focus_after_pane_destroy`).
//! - `navigation`: `navigate`, `go_back`, `go_forward`, `reload`, `resize`.
//! - `zoom`: `zoom_in`/`zoom_out`/`step_zoom`/`apply_zoom`/`reapply_zoom`,
//!   `next_zoom_factor`.
//! - `clip`: focus + the Win32/X11 overlay-clip airspace workaround
//!   (`set_pane_overlay_clip`, `focus`, `defocus_all`, `compute_pane_visible`).

use std::sync::Arc;

use cef::*;

use crate::browser_pane::CreateBrowserPaneTask;
use crate::reducer::RegisterResult;
use crate::state::AppState;

mod clip;
mod close;
pub(crate) mod media_grants;
mod navigation;
mod zoom;

// `compute_pane_visible` is called from `browser_pane::creation_views` via
// `crate::browser_panes::compute_pane_visible` — re-export so that external
// path keeps resolving now that the definition lives in the `clip` submodule.
// Same cfg gate as the definition (Linux only — macOS uses the hole-punch
// mask, Windows uses SetWindowRgn; see `clip::compute_pane_visible`'s doc).
#[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
pub(crate) use clip::compute_pane_visible;

/// Abstraction over the CEF-side operations `close()` performs on the host
/// process. Production implements this over `&Arc<AppState>` and Win32;
/// tests implement it with a recording mock so the close-path state machine
/// can be exercised without real CEF/HWNDs.
///
/// Kept minimal (just two methods) to avoid a dependency graph that has to
/// be updated every time `close()` grows. Other ops (`focus`, `resize`, …)
/// can gain their own traits when they need testing, or graduate to a
/// unified `BrowserPaneCefBridge` once the shape is stable.
pub trait BrowserPaneCloseOps {
    /// Remove the Browser for this label from the registry. Return its
    /// outer HWND as a pointer-sized value, or `None` if there is no
    /// Browser or no HWND. Dropping the Browser Arc is the implementation's
    /// responsibility — production drops before returning so Chromium's
    /// refcount isn't held by our scope.
    fn take_browser_hwnd(&self, label: &str) -> Option<usize>;

    /// Destroy the given HWND. Production posts a CEF UI-thread task
    /// (`DestroyPaneWrapperTask`) that runs Win32 `DestroyWindow` on the
    /// wrapper's OWNING thread — DestroyWindow is owner-thread-only, and
    /// calling it directly from the IPC thread silently no-ops
    /// (retro-browser-pane-renderer-leak-2026-07-07). Called only with
    /// values returned from `take_browser_hwnd`.
    fn destroy_hwnd(&self, hwnd: usize);
}

/// Production implementation of `BrowserPaneCloseOps` backed by `AppState.browsers`
/// and Win32 `DestroyWindow`.
///
/// SPEC_BROWSER_PANE_WINDOWS_TEARDOWN_SPIKE_2026_07_03.md: `take_browser_hwnd`
/// now returns the app-owned WRAPPER's HWND (`browser_pane::wrapper`), not
/// CEF's own HWND — `destroy_hwnd` destroys the wrapper (after reparenting it
/// out to top-level; see `destroy_wrapper_hwnd`'s doc for why that step is
/// what actually releases the renderer — retro-browser-pane-renderer-leak-
/// 2026-07-07), never CEF's own window directly and never via
/// `close_browser()`, avoiding the close_browser-cascades-into-main bug three
/// earlier attempts hit.
struct AppStateCloseOps<'a>(&'a Arc<AppState>);

impl<'a> BrowserPaneCloseOps for AppStateCloseOps<'a> {
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
        {
            // Forget any stale focus-tracking entry for CEF's OWN hwnd
            // (LAST_FOCUSED_BY_ROOT is keyed by whatever HWND actually
            // received SetFocus — always CEF's own hwnd/descendants, never
            // our wrapper, which never itself receives focus). Belt-and-
            // suspenders: `on_before_close_browser_pane` also runs a more
            // thorough sweep via `uninstall_focus_redirect_for_block` once
            // CEF's OnBeforeClose fires (which the wrapper's reparent-then-
            // destroy teardown is what makes reliable — see
            // destroy_wrapper_hwnd) — this covers the gap before that.
            if let Some(host) = browser.host() {
                let wh = host.window_handle();
                if !wh.0.is_null() {
                    crate::browser_pane::hwnd::forget_focus_for_child(wh.0 as *mut _);
                }
            }
        }

        // Drop our Arc before returning so Chromium's refcount doesn't wait
        // for the caller's scope to unwind.
        drop(browser);

        #[cfg(target_os = "windows")]
        {
            crate::browser_pane::wrapper::take_wrapper_hwnd(label).map(|h| h as usize)
        }
        #[cfg(not(target_os = "windows"))]
        {
            None
        }
    }

    fn destroy_hwnd(&self, hwnd: usize) {
        #[cfg(target_os = "windows")]
        {
            // Marshal the destroy onto the CEF UI thread — the thread that
            // CREATED the wrapper (CreateBrowserPaneTask::execute). `close()`
            // runs on a tokio IPC thread, and Win32's DestroyWindow hard-fails
            // (ERROR_ACCESS_DENIED) from any thread other than the window's
            // owner — which is exactly how every pane close silently leaked
            // its renderer while the logs looked clean
            // (retro-browser-pane-renderer-leak-2026-07-07: the pixels
            // vanished because ShowWindow(SW_HIDE) works cross-thread; the
            // DestroyWindow after it never did anything, so CEF never saw
            // WM_DESTROY and the browser survived headless). Mirrors the
            // Linux/macOS path, which has always posted
            // DetachBrowserPaneViewTask for the same stated reason (see the
            // marshalling-tasks comment near the bottom of `close.rs`).
            let mut task = close::DestroyPaneWrapperTask::new(hwnd as isize);
            let posted = cef::post_task(cef::ThreadId::UI, Some(&mut task));
            if posted == 0 {
                tracing::error!(
                    hwnd,
                    "[pane-wrapper] post_task(destroy) failed — wrapper + CEF browser will leak"
                );
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
        let label = state.live_browser_pane_label(block_id)?;
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
        window_label: &str,
    ) -> Result<(), String> {
        // Phase H.1.d (PR #5) — sole pane-registration entry point. The
        // reducer atomically generates the label and inserts the entry,
        // returning Fresh / AlreadyLive / Closing via DispatchOutput.
        //
        // We pass the create params as `pending`: if the reducer finds the
        // block_id still `Closing`, it stashes them (under the same host_state
        // lock that observed Closing — atomic, no TOCTOU) for the
        // close-completion arm to replay. Ignored for Fresh/AlreadyLive.
        let out = state.host_dispatch(
            crate::reducer::HostCommand::TryRegisterBrowserPaneLive {
                block_id: block_id.to_string(),
                pending: Some(crate::state::PendingBrowserPaneCreate {
                    url: url.to_string(),
                    x: rect.x,
                    y: rect.y,
                    width: rect.width,
                    height: rect.height,
                    window_label: window_label.to_string(),
                }),
            },
        );
        let result = out.browser_pane_register_result.ok_or_else(|| {
            format!(
                "try_register_browser_pane_live returned no result (block_id={}); host shutting down?",
                block_id
            )
        })?;
        match result {
            RegisterResult::AlreadyLive(label) => {
                // Existing Live entry — re-navigate the existing browser.
                //
                // DIAGNOSTIC (redock black-page triage —
                // ANALYSIS_BROWSER_PANE_REDOCK_BLACK_TYPING_LOCK_2026_06_15.md §2):
                // if a redock's create lands here while the old pane is still
                // Live (the close IPC hasn't flipped it to Closing yet), we
                // only re-navigate the EXISTING browser — which is still
                // parented to the *old* (floater) window — and never create a
                // pane in the requested `window_label`. The target window then
                // shows black until the old pane closes. Logging requested
                // window + rect makes this race visible in a repro; if a redock
                // hits this arm, it is the smoking gun.
                tracing::info!(
                    block_id,
                    label = %label,
                    requested_window = %window_label,
                    rect = ?(rect.x, rect.y, rect.width, rect.height),
                    "browser pane create hit AlreadyLive — re-navigating existing browser (NOT creating in requested window)"
                );
                crate::browser_pane::trace::pane_trace(
                    block_id,
                    "create-already-live",
                    &format!("label={label} requested_win={window_label} url={url}"),
                );
                if let Some(browser) = state.get_browser(&label) {
                    if let Some(frame) = browser.main_frame() {
                        frame.load_url(Some(&CefString::from(url)));
                    }
                }
                Ok(())
            }
            RegisterResult::AlreadyLiveElsewhere(label) => {
                // Cross-window move (tear-off / redock): the block is Live in a
                // DIFFERENT window. The reducer stashed our pending create;
                // close the old pane now. Its close-completion (CompleteBrowserPaneClose
                // / DrainBrowserPaneByLabel) drains the entry and replays the
                // stashed create as `Fresh` in the requested window — so the
                // browser reappears in the new (floating) window instead of the
                // requested window rendering black. See
                // ANALYSIS_BROWSER_PANE_REDOCK_BLACK_TYPING_LOCK_2026_06_15 §2.
                tracing::info!(
                    block_id,
                    old_label = %label,
                    requested_window = %window_label,
                    rect = ?(rect.x, rect.y, rect.width, rect.height),
                    "browser pane create is a cross-window move — closing old pane; reducer will replay create in requested window"
                );
                crate::browser_pane::trace::pane_trace(
                    block_id,
                    "create-cross-window-move",
                    &format!("old_label={label} requested_win={window_label}"),
                );
                self.close(block_id, state);
                Ok(())
            }
            RegisterResult::Closing => {
                // Old CEF Browser mid-teardown — don't overwrite (its
                // on_before_close → DrainBrowserPaneByLabel would evict the NEW
                // entry), and don't drop the request. The reducer already
                // stashed our `pending` create (above) under the host_state
                // lock; the close-completion arm replays it once the old entry
                // is gone (see `drain_closed_label` / `close()`). This is the
                // redock case: the target window re-creates the same block_id
                // while the floater's pane is still Closing. (The old "frontend
                // retries on next tick" never existed — browser-view.tsx::createPane
                // errored out — which is why redocked panes intermittently
                // never loaded.) See
                // docs/analysis/ANALYSIS_BROWSER_PANE_REDOCK_LOAD_RACE_2026_05_29.md.
                tracing::info!(
                    block_id,
                    requested_window = %window_label,
                    rect = ?(rect.x, rect.y, rect.width, rect.height),
                    "browser pane create deferred — block_id still Closing; reducer will replay on close-completion"
                );
                crate::browser_pane::trace::pane_trace(
                    block_id,
                    "create-deferred-closing",
                    "old entry still Closing; reducer will replay on close-completion",
                );
                Ok(())
            }
            RegisterResult::Fresh(label) => {
                crate::browser_pane::trace::pane_trace(
                    block_id,
                    "create-request",
                    &format!(
                        "url={url} label={label} win={window_label} rect=({},{},{},{})",
                        rect.x, rect.y, rect.width, rect.height
                    ),
                );
                let mut task = CreateBrowserPaneTask::new(
                    state.clone(),
                    block_id.to_string(),
                    label,
                    url.to_string(),
                    rect,
                    window_label.to_string(),
                );
                post_task(ThreadId::UI, Some(&mut task));
                Ok(())
            }
        }
    }
}
