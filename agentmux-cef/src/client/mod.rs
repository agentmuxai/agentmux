// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// CefClient and associated handler implementations.
// Manages browser lifecycle, display updates, and load errors.
//
// Phase 2: Stores browser ref in AppState and injects IPC port on page load.

use cef::*;
use std::sync::Arc;
use parking_lot::Mutex;

use crate::state::{AppState, WindowKind};

// Phase B.9.3 — close-pool-browser task. Used by Stage 1 to defer
// `close_browser` onto the CEF UI thread via `cef::post_task`, so
// the call runs AFTER the current `on_before_close` unwinds.
// `close_browser` called inline from inside another browser's
// close callback re-enters CEF and hangs the UI thread (smoke
// v0.33.497 confirmed).
//
// Two callers:
// - Windows: HWND lookup fallback. Stage 1 prefers Win32
//   `PostMessage(hwnd, WM_CLOSE)` (bypasses CEF's task queue —
//   useful as a belt-and-suspenders mechanism), but if the
//   window handle is null (early/late lifecycle, e.g. browser
//   created without a Views top-level yet), fall through to this
//   task so the close still happens. Otherwise self.browser_list
//   never empties and Stage 2 never fires. (codex #601 P1.)
// - Non-Windows: canonical path. macOS/Linux don't have
//   `PostMessage(WM_CLOSE)`; defer close_browser via post_task
//   for both correctness and portability. (reagent #601 P1.)
wrap_task! {
    pub struct ClosePoolBrowserTask {
        browser: Browser,
    }

    impl Task {
        fn execute(&self) {
            let mut b = self.browser.clone();
            if let Some(host) = b.host() {
                host.close_browser(1); // force_close = true
            }
        }
    }
}

/// Write a debug line to `%TEMP%\agentmux-close-debug.txt`.
///
/// Only active when `AGENTMUX_DEBUG_CLOSE=1` is set in the environment.
/// In normal production runs the file is never written to.
/// Always emits at tracing::debug level regardless of the env flag.
pub fn dlog(msg: &str) {
    use std::sync::OnceLock;
    static ENABLED: OnceLock<bool> = OnceLock::new();
    let enabled = *ENABLED.get_or_init(|| std::env::var("AGENTMUX_DEBUG_CLOSE").is_ok());

    tracing::debug!("[close-debug] {}", msg);

    if enabled {
        use std::io::Write;
        let path = std::env::temp_dir().join("agentmux-close-debug.txt");
        if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&path) {
            use std::time::{SystemTime, UNIX_EPOCH};
            let ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis();
            let _ = writeln!(f, "[{}] {}", ms, msg);
        }
        tracing::info!("[close-debug] {}", msg);
    }
}

/// Core handler state shared across all CEF callback interfaces.
pub struct AgentMuxHandler {
    browser_list: Vec<Browser>,
    is_closing: bool,
    state: Arc<AppState>,
    ipc_port: u16,
    is_browser_pane: bool,
}

mod handlers;
mod helpers;
#[cfg(target_os = "windows")]
mod wndproc;

pub use handlers::AgentMuxClient;

use helpers::{js_string_literal, html_escape, backend_close_window};
#[cfg(target_os = "windows")]
use wndproc::{set_window_icon, skip_taskbar};

impl AgentMuxHandler {
    pub fn new(state: Arc<AppState>, ipc_port: u16) -> Arc<Mutex<Self>> {
        Self::new_with_browser_pane(state, ipc_port, false)
    }

    pub fn new_with_browser_pane(state: Arc<AppState>, ipc_port: u16, is_browser_pane: bool) -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(Self {
            browser_list: Vec::new(),
            is_closing: false,
            state,
            ipc_port,
            is_browser_pane,
        }))
    }

    fn on_title_change(&mut self, browser: Option<&mut Browser>, title: Option<&CefString>) {
        debug_assert_ne!(currently_on(ThreadId::UI), 0);

        // Update the window title via CEF Views.
        let mut browser = browser.cloned();
        if let Some(browser_view) = browser_view_get_for_browser(browser.as_mut()) {
            if let Some(window) = browser_view.window() {
                window.set_title(title);
            }
        }
        // For Alloy-style native windows on Windows, update via Win32 API.
        #[cfg(target_os = "windows")]
        {
            if let (Some(browser), Some(title)) = (browser.as_ref(), title) {
                if let Some(host) = browser.host() {
                    let hwnd = host.window_handle();
                    if !hwnd.0.is_null() {
                        let title_wide: Vec<u16> = title
                            .to_string()
                            .encode_utf16()
                            .chain(std::iter::once(0))
                            .collect();
                        unsafe {
                            windows_sys::Win32::UI::WindowsAndMessaging::SetWindowTextW(
                                hwnd.0 as *mut std::ffi::c_void,
                                title_wide.as_ptr(),
                            );
                        }
                    }
                }
            }
        }
    }

    fn on_after_created(&mut self, browser: Option<&mut Browser>) {
        debug_assert_ne!(currently_on(ThreadId::UI), 0);

        let browser = browser.cloned().expect("Browser is None");
        tracing::info!("Browser created (total: {})", self.browser_list.len() + 1);

        // Phase 1 diagnostic tracing — find the exact line that silences the
        // UI thread under concurrent window creation. See
        // docs/specs/SPEC_HOST_WINDOW_CREATION_RUNNER_2026-05-02.md.
        let t0 = std::time::Instant::now();

        // Phase B.5 (window_meta step d) — pop the pre-create
        // handoff entry. Pre-step-d this was a label-only queue +
        // separate `window_meta.insert` from the caller; now it's
        // a single `PendingWindowCreation` carrying label + kind +
        // parent_instance_id, eliminating the parallel-write race
        // between caller and on_after_created.
        //
        // First-browser shortcut: "main" never has a pre-create
        // handoff (host startup spawns it directly), so we
        // synthesize a FullInstance entry. Subsequent windows pop
        // their entry; if the queue is empty (legacy paths /
        // unexpected races) fall back to a generated UUID label
        // with FullInstance defaults.
        // Phase H.2.b — reducer-aware emptiness check with fallback.
        let pending = if self.state.browsers_is_empty() {
            crate::state::PendingWindowCreation {
                label: "main".to_string(),
                kind: WindowKind::FullInstance,
                parent_instance_id: None,
            }
        } else {
            // Phase F.1 — dequeue via the host reducer. The reducer
            // emits PendingWindowQueueEmpty on miss; the fallback
            // (synthesize a UUID-labelled FullInstance entry) lives
            // in the legacy code path it always has.
            tracing::info!(
                elapsed_us = t0.elapsed().as_micros() as u64,
                "[on-after-created] dispatching DequeuePendingWindowCreation"
            );
            let out = self
                .state
                .host_dispatch(crate::reducer::HostCommand::DequeuePendingWindowCreation);
            tracing::info!(
                elapsed_us = t0.elapsed().as_micros() as u64,
                dequeued_some = out.dequeued.is_some(),
                "[on-after-created] DequeuePendingWindowCreation returned"
            );
            out.dequeued.unwrap_or_else(|| {
                let lbl = format!("window-{}", uuid::Uuid::new_v4());
                tracing::warn!(label = %lbl, "[on_after_created] no pending creation entry — defaulting to FullInstance");
                crate::state::PendingWindowCreation {
                    label: lbl,
                    kind: WindowKind::FullInstance,
                    parent_instance_id: None,
                }
            })
        };
        let label = pending.label.clone();
        let pending_kind = pending.kind;
        let pending_parent = pending.parent_instance_id.clone();

        // Phase H.2.d — legacy `state.browsers.insert` removed. Reducer's
        // `RegisterBrowser` (dispatched below) is now the sole canonical
        // mutation site. Smoke test on 0.33.585 verified parallel-write
        // parity (zero drift across 18 RegisterBrowser/Unregister pairs).
        let total = self.state.host_state.lock().browsers.len() + 1;
        tracing::info!(
            label = %label,
            elapsed_us = t0.elapsed().as_micros() as u64,
            total,
            "[on-after-created] registering browser via reducer",
        );
        dlog(&format!("on_after_created: registered label={} total={}", label, total));

        let is_top_level_window = !label.starts_with("browser-pane-");

        // Determine BrowserKind from the LABEL prefix, not the
        // AgentMuxClient `is_browser_pane` flag. Smoke test on 0.33.586 found
        // top-level windows misclassified as `Pane { block_id: "" }`
        // because `CreateWindowTask::execute` reuses an existing
        // browser's CEF Client via `first_browser()` — if the iteration
        // happens to pick a pane, the new window inherits `is_browser_pane=true`
        // and the label-stripping in this branch produces an empty
        // block_id (since the label starts with `window-` not
        // `browser-pane-`). LABEL is the source of truth. See
        // docs/retro/smoke-test-0.33.586-and-pr5-plan-2026-05-02.md.
        //
        // Classification:
        //   - label `browser-pane-<uuid>-<seq>` → Pane { block_id: uuid }
        //   - label `window-pool-*` + still in unpromoted_pool_labels →
        //     TopLevel { is_pool: true }
        //   - everything else (main, window-*, promoted pool windows) →
        //     TopLevel { is_pool: false }
        let kind = if let Some(rest) = label.strip_prefix("browser-pane-") {
            let block_id = rest
                .rfind('-')
                .map(|i| rest[..i].to_string())
                .unwrap_or_default();
            crate::state::BrowserKind::Pane { block_id }
        } else if label.starts_with("window-pool-")
            && self.state.is_unpromoted_pool_label(&label)
        {
            crate::state::BrowserKind::TopLevel { is_pool: true }
        } else {
            crate::state::BrowserKind::TopLevel { is_pool: false }
        };
        self.state.host_dispatch(
            crate::reducer::HostCommand::RegisterBrowser {
                label: label.clone(),
                browser: browser.clone(),
                kind,
            },
        );

        // Phase B.5 (window_meta step d, refined) — write host's
        // local `window_meta` ONCE here, synchronously from the
        // popped pending entry. This is no longer the authoritative
        // state (the launcher's `state.windows` is); it's a
        // host-internal cache that covers two scenarios where the
        // launcher-fed shadow can't:
        //
        // 1. `task dev` mode — no launcher IPC at all, shadow stays
        //    empty forever. open_subwindow's parent validation +
        //    cascade-close need a synchronous local source.
        // 2. Cascade-close race — child opens just before parent
        //    closes; on_after_created→ReportWindowOpened→launcher
        //    →WindowOpened→shadow round-trip hasn't completed by
        //    the time parent's on_before_close runs. Without the
        //    local write, `subwindow_children_of` would miss the
        //    child and skip cascade close.
        //
        // The retired piece (step d's intent) is the
        // **caller-side parallel write** — drag/window/window_pool
        // no longer write meta themselves. Single canonical
        // mutation site here. (codex P1 PR #592 round-2.)
        if is_top_level_window {
            let mut metas = self.state.window_meta.lock();
            metas.insert(
                label.clone(),
                crate::state::WindowMeta {
                    label: label.clone(),
                    kind: pending_kind,
                    parent_instance_id: pending_parent.clone(),
                },
            );
        }

        // No DwmExtendFrameIntoClientArea — it causes the white flash.
        // CEF Views handles frameless + resize via its delegate.

        // Set the taskbar/title bar icon from the embedded exe resource, and
        // for `Subwindow` top-levels, hide them from the taskbar via
        // ITaskbarList::DeleteTab.
        #[cfg(target_os = "windows")]
        {
            // Prefer CEF Views' `Window::window_handle()` — it targets the
            // specific top-level window for THIS browser, avoiding the
            // `find_own_top_level_window` fallback's "first visible HWND"
            // ambiguity when multiple windows exist.
            let mut browser_mut = browser.clone();
            let views_top_hwnd = browser_view_get_for_browser(Some(&mut browser_mut))
                .and_then(|bv| bv.window())
                .map(|w| w.window_handle().0 as *mut std::ffi::c_void)
                .filter(|p| !p.is_null());

            let hwnd = views_top_hwnd.unwrap_or_else(|| {
                browser.host()
                    .and_then(|h| {
                        let wh = h.window_handle();
                        if wh.0.is_null() { None } else { Some(wh.0 as *mut std::ffi::c_void) }
                    })
                    .unwrap_or_else(|| unsafe {
                        crate::commands::window::find_own_top_level_window()
                    })
            });

            if !hwnd.is_null() {
                unsafe { set_window_icon(hwnd); }

                // Subwindow? Hide from taskbar. Full instances and browser-pane
                // child HWNDs skip this branch.
                if is_top_level_window {
                    // Phase B.5 (window_meta step d) — read kind from
                    // the pending entry we just popped. No
                    // window_meta lookup, no race window.
                    if pending_kind == WindowKind::Subwindow {
                        unsafe { skip_taskbar(hwnd); }
                    }
                }
            }
        }

        // Pane-specific on_after_created work (Z-order raise + Win32 focus
        // subclass install) lives in `crate::browser_pane::callbacks` after Phase 4
        // of the modularization split.
        if self.is_browser_pane {
            crate::browser_pane::callbacks::on_after_created_browser_pane(&self.state, &browser);
        }

        // Phase B.4 — report top-level windows to the launcher's
        // read-only state mirror. Skips browser-pane child HWNDs and
        // pool windows (they're not user-visible until promoted; the
        // pool->user transition gets its own report in a follow-up).
        // No-op if launcher IPC isn't connected (`task dev` mode).
        if is_top_level_window && !label.starts_with("window-pool-") {
            // Phase B.5 (window_meta step d) — kind/parent come
            // from the pending entry we popped at the top of this
            // fn, not a window_meta lookup.
            let wire_kind = match pending_kind {
                WindowKind::FullInstance => agentmux_common::ipc::WindowKind::FullInstance,
                WindowKind::Subwindow => agentmux_common::ipc::WindowKind::Subwindow,
            };
            crate::launcher_ipc::report_window_opened(label.clone(), wire_kind, pending_parent.clone());

            // Phase B.9.1 (WRR) — authoritative HWND link. We have
            // both the label (popped from PendingWindowCreation
            // above) and the native HWND (computed in the
            // #[cfg(target_os = "windows")] block above as
            // `views_top_hwnd` / `hwnd`). Sending an explicit
            // ReportHwndOpened with `label_hint = Some(label)` here
            // eliminates the race between the OS-driven
            // EVENT_OBJECT_CREATE (which my hook captures with
            // `label_hint = None` because pending_window_creations
            // may already have been popped by the time the OS event
            // bubbles back) and CEF's lifecycle. The OS-event path
            // still runs as belt-and-suspenders for non-CEF windows
            // / future detection of strays. (The prior pending_hwnds
            // entry from the OS event is harmless — it ages out on
            // the next event-driven reconciliation pass.)
            #[cfg(target_os = "windows")]
            {
                // Recompute the HWND here — the prior #[cfg] block
                // computed `hwnd` as a local that's not in scope at
                // this site. The CEF Browser API is cheap to query
                // a second time. Precedence: Views' window handle →
                // host's window handle. NO fallback to
                // `find_own_top_level_window()` — that function uses
                // `EnumWindows` and returns the FIRST visible window
                // belonging to this process, which in a multi-window
                // session is some OTHER window's HWND. Sending that
                // as authoritative `Some(label)` would corrupt the
                // OTHER label's mirror via the `Repaired` arm in
                // `apply_hwnd_opened`. (reagent P1 PR #664 round 3.)
                //
                // If both Views and host return null (transient
                // lifecycle case), skip the explicit dispatch. The
                // launcher's drain-on-WindowOpened fallback links
                // the recent pending HWND from WM_CREATE — that's
                // the sole link path when `hwnd_val=0`. The drain
                // is reliable when WM_CREATE arrived recently (within
                // the launcher's 2s age limit); the only failure mode
                // is no WM_CREATE-pending entry within that window,
                // in which case the mirror stays hwnd=None — same
                // outcome as pre-PR-664 for that edge case, no worse.
                let mut browser_for_wrr = browser.clone();
                let views_hwnd = browser_view_get_for_browser(Some(&mut browser_for_wrr))
                    .and_then(|bv| bv.window())
                    .map(|w| w.window_handle().0 as *mut std::ffi::c_void)
                    .filter(|p| !p.is_null());
                let host_hwnd = browser.host().and_then(|h| {
                    let wh = h.window_handle();
                    if wh.0.is_null() {
                        None
                    } else {
                        Some(wh.0 as *mut std::ffi::c_void)
                    }
                });
                let hwnd_val = views_hwnd.or(host_hwnd).map(|p| p as u64).unwrap_or(0);
                if hwnd_val != 0 {
                    crate::launcher_ipc::report_hwnd_opened(
                        hwnd_val,
                        "Chrome_WidgetWin_1".to_string(),
                        label.clone(),
                        Some(label.clone()),
                    );
                } else {
                    // Both sources null. Launcher's drain-on-WindowOpened
                    // fallback should still link the pending HWND from
                    // WM_CREATE; if that race lost too, the mirror
                    // stays hwnd=None for this window — degraded but
                    // not corrupted. Log at WARN so the regression is
                    // visible if it happens.
                    tracing::warn!(
                        target: "wrr",
                        label = %label,
                        "[wrr] on_after_created: hwnd_val=0 from both Views and host — \
                         relying on launcher's pending_hwnds drain fallback"
                    );
                }
            }
            // Phase B.4 follow-up — drift check after the open.
            crate::launcher_ipc::compute_and_report_host_counts(&self.state);
        }

        self.browser_list.push(browser);

        // Tear-off Phase 6 — pre-warmed window pool.
        // - When the "main" window registers, kick off the initial pool spawn.
        // - When a "window-pool-*" window registers, log only — actual
        //   queue insertion waits for the frontend's renderer-ready IPC
        //   so emit_event_to_window doesn't race the listener install.
        if label == "main" {
            crate::commands::window_pool::init_pool(&self.state);
        } else if label.starts_with("window-pool-") {
            crate::commands::window_pool::register_pool_window(&self.state, &label);
        }
    }

    fn do_close(&mut self, _browser: Option<&mut Browser>) -> bool {
        debug_assert_ne!(currently_on(ThreadId::UI), 0);

        if self.browser_list.len() == 1 {
            self.is_closing = true;
        }
        // Return false to allow the close.
        false
    }

    /// Intercept `target="_blank"` / `window.open()` from embedded pages so
    /// they don't spawn rogue top-level CEF windows. Instead navigate the
    /// **current** frame to the target URL — matches the UX expectation
    /// that AgentMux owns window management, not the page.
    ///
    /// Returning non-zero cancels popup creation. Applies to both main
    /// and pane clients: main's frontend never relies on `window.open`
    /// (link clicks go through `openExternal` IPC), and panes explicitly
    /// don't want popups. See
    /// specs/SPEC_BROWSER_PANE_DEFAULT_URL_AND_POPUP_2026_04_21.md.
    ///
    /// **The `load_url` call is deferred via `post_task`**, not run inline.
    /// Inline `load_url` caused a UI-thread deadlock on link click:
    /// `on_before_popup` runs while `AgentMuxLifeSpanHandler` holds
    /// `self.inner.lock()` (via the wrap macro). Inline `load_url` starts
    /// a new navigation on the same UI thread, which triggers
    /// `on_loading_state_change` on `AgentMuxLoadHandler`, which also
    /// tries to take `self.inner.lock()` → deadlock. The host hung with
    /// backend heartbeats still running but the whole UI frozen. Posting
    /// the `load_url` as a separate UI task lets the popup handler
    /// return, release the lock, then pick up the load on the next loop
    /// iteration.
    fn on_before_popup(
        &mut self,
        browser: Option<&mut Browser>,
        _frame: Option<&mut Frame>,
        target_url: Option<&CefString>,
        _target_disposition: WindowOpenDisposition,
    ) -> bool {
        let url = target_url.map(|s| s.to_string()).unwrap_or_default();
        if url.is_empty() {
            // Nothing useful to navigate to; just cancel the popup.
            return true;
        }
        if let Some(b) = browser {
            let browser_clone = b.clone();
            let mut task = crate::ui_tasks::DeferredLoadUrlTask::new(
                browser_clone,
                url.clone(),
            );
            cef::post_task(cef::ThreadId::UI, Some(&mut task));
        }
        tracing::info!(
            is_browser_pane = %self.is_browser_pane,
            url = %url,
            "popup intercepted — deferred navigation of current frame",
        );
        true // cancel the top-level popup creation
    }

    fn on_before_close(&mut self, browser: Option<&mut Browser>) {
        debug_assert_ne!(currently_on(ThreadId::UI), 0);

        // Phase B.9.3 — diagnostic trace at debug level. Filtered
        // out in production (default RUST_LOG=info). Enable via
        // RUST_LOG="info,wrr-trace=debug" when investigating
        // close-cascade issues.
        tracing::debug!(
            target: "wrr-trace",
            "[trace] on_before_close ENTER; self.browser_list.len()={} is_browser_pane={}",
            self.browser_list.len(), self.is_browser_pane
        );
        dlog(&format!("on_before_close fired; browser_list.len()={}", self.browser_list.len()));

        let mut browser = browser.cloned().expect("Browser is None");

        // Unregister browser from the reducer's `browsers` map and get its
        // label. Phase H.2.d — legacy `state.browsers.lock().remove` removed;
        // reducer is sole source of truth (see PR #4 commit 2 H.2.c flip).
        // Find-by-identity loop now iterates reducer-backed snapshot via
        // `state.list_browsers()`, then dispatches `UnregisterBrowser`.
        let snapshot = self.state.list_browsers();
        let keys: Vec<&String> = snapshot.iter().map(|(k, _)| k).collect();
        dlog(&format!("browsers map keys: {:?}", keys));
        let label = snapshot
            .iter()
            .find(|(_, b)| {
                let mut b = b.clone();
                b.is_same(Some(&mut browser)) != 0
            })
            .map(|(k, _)| k.clone());
        dlog(&format!("label found: {:?}", label));
        if let Some(ref lbl) = label {
            self.state.host_dispatch(
                crate::reducer::HostCommand::UnregisterBrowser { label: lbl.clone() },
            );
            let remaining = self.state.host_state.lock().browsers.len();
            tracing::info!(
                "Unregistered browser: label={} (remaining: {})",
                lbl,
                remaining
            );
        }

        // Pane-specific on_before_close work (drain lifecycle entry) lives
        // in `crate::browser_pane::callbacks` after Phase 4.
        if let Some(ref lbl) = label {
            if lbl.starts_with("browser-pane-") {
                crate::browser_pane::callbacks::on_before_close_browser_pane(&self.state, lbl);
            }
            // Pool-window cleanup — release the respawn semaphore +
            // drop the label from the queue if the window died before
            // promote (renderer crash, OS-level close). Without this
            // the pool would never refill.
            if lbl.starts_with("window-pool-") {
                crate::commands::window_pool::on_pool_window_destroyed(&self.state, lbl);
            }
            // Phase B.4 — mirror the close to the launcher. Skip
            // browser-pane child HWNDs (never reported as open).
            // For everything else, send unconditionally: the launcher
            // reducer silently no-ops on unknown labels (codex P2
            // PR #577 round-2 made `WindowClosed` strictly paired
            // with `WindowOpened`), so pre-promote pool deaths and
            // post-pop / pre-validation orphans are filtered there
            // — no host-side guard needed. Pool inventory updates
            // travel via `ReportPoolWindowRemoved` from
            // `on_pool_window_destroyed` and `promote_pool_window`.
            if !lbl.starts_with("browser-pane-") {
                crate::launcher_ipc::report_window_closed(lbl.clone());
                // Phase B.4 follow-up — drift check after the close.
                crate::launcher_ipc::compute_and_report_host_counts(&self.state);
            }
        }

        // Phase B.5 (window_id_map step d) — host no longer mutates
        // `window_id_map`. The launcher's `state.backend_window_ids`
        // (B.5 step a) is the sole authority; we look up the wid
        // via the shadow-first helper before notifying the launcher
        // to drop it. The wid lookup at close time is safe even
        // without the host fallback because the frontend's original
        // `register_backend_window` ran long before close — shadow
        // has been populated for the entire window lifetime.
        let backend_window_id = label.as_deref().and_then(|lbl| {
            let wid = self.state.backend_window_id(lbl);
            dlog(&format!("backend_window_id({:?}) => {:?}", lbl, wid));
            crate::launcher_ipc::report_backend_window_id_unregistered(lbl.to_string());
            wid
        });

        // Pull and remove the closing window's meta; if it's a FullInstance,
        // cascade-close every Subwindow whose parent_instance_id points to it.
        // See `docs/specs/SPEC_MULTIWINDOW_TASKBAR_GROUPING.md` §2.3.
        //
        // Phase B.5 (window_meta step d, refined) — read closing
        // meta via shadow-first helper, drop the host-side cache
        // entry (single canonical mutation site for window_meta
        // post-refinement: insert in on_after_created, remove here).
        let closing_meta = label
            .as_deref()
            .and_then(|lbl| self.state.window_meta(lbl));
        if let Some(lbl) = label.as_deref() {
            self.state.window_meta.lock().remove(lbl);
        }
        if let Some(meta) = &closing_meta {
            if meta.kind == WindowKind::FullInstance {
                let child_labels = self.state.subwindow_children_of(&meta.label);
                for child_label in child_labels {
                    // Phase H.2.b — reducer-aware lookup with fallback.
                    if let Some(mut child) = self.state.get_browser(&child_label) {
                        if let Some(host) = child.host() {
                            tracing::info!(parent = %meta.label, child = %child_label, "[subwindow-cascade] closing sub-window");
                            host.close_browser(1);
                        }
                    }
                }
            }
        }

        // Phase F.6 — narrate the pane-reap step for the launcher's
        // window-cleanup-cascade saga. By the time we reach here, the
        // pane lifecycle drain (`on_before_close_browser_pane` for browser-
        // pane labels) and the subwindow cascade above have run for
        // this label. The saga uses this signal as the Step 1
        // terminal so it can advance to Step 2 (drain-pool decision).
        //
        // Skip for browser-pane labels: the saga is triggered by
        // `Event::WindowClosed`, which only fires for non-pane
        // top-level windows; emitting `PanesReaped` for pane labels
        // would be a stray report (no in-flight saga to consume it).
        // Same gate as `report_window_closed` above — skip
        // browser-pane-* labels (sub-views, not top-level windows).
        // Don't filter window-pool-* here: filtering on prefix would
        // wrongly suppress promoted pool windows (which keep the
        // `window-pool-*` prefix but ARE tracked windows). Stray
        // events for unpromoted-pool drains are emitted but harmless
        // — no F.6 saga is in flight to consume them.
        if let Some(ref lbl) = label {
            if !lbl.starts_with("browser-pane-") {
                crate::launcher_ipc::report_panes_reaped(lbl.clone());
            }
        }

        dlog(&format!("backend_window_id: {:?}", backend_window_id));

        if let Some(index) = self
            .browser_list
            .iter()
            .position(|elem| elem.is_same(Some(&mut browser)) != 0)
        {
            self.browser_list.remove(index);
        }

        dlog(&format!("browser_list after remove: {}", self.browser_list.len()));

        // App-exit decision: count remaining USER-FACING browsers.
        // Unpromoted pool windows are pre-warmed scratch windows
        // hidden from the user via WS_EX_TOOLWINDOW — they have no
        // taskbar entry, can't be closed by the user, and would
        // otherwise keep the app alive forever after the last
        // visible window closes. Browser-pane child HWNDs
        // (`browser-pane-*`) are sub-views of a parent window, not
        // standalone instances, so they don't count either.
        //
        // Use `pool_inventory_labels_snapshot()` — union of
        // `unpromoted` (spawn-time) and `pool.queue` (renderer-ready,
        // pre-promote). Both states are hidden off-screen with no
        // user UI; counting them as user-visible inflates this
        // gate and prevents the cascade from firing when the user
        // really did close their last visible window.
        //
        // Promoted pool windows ARE counted: they're removed from
        // BOTH pool sets at promote time.
        let (user_browser_count, browsers_keys, pool_keys) = {
            // Phase H.2.b — reducer-aware label snapshot. Single helper call
            // replaces an interleaved pool_labels.lock + browsers.lock pair.
            let pool_labels = self.state.pool_inventory_labels_snapshot();
            let keys = self.state.list_browser_labels();
            let count = keys
                .iter()
                .filter(|label| {
                    !pool_labels.contains(label.as_str()) && !label.starts_with("browser-pane-")
                })
                .count();
            let pool: Vec<String> = pool_labels.iter().cloned().collect();
            (count, keys, pool)
        };

        // Phase B.9.3 diagnostic — fires for every close (incl.
        // pane closes). Demoted to debug for production. Enable
        // via RUST_LOG="info,wrr-trace=debug" to see per-close
        // gate input when investigating close-cascade issues.
        tracing::debug!(
            target: "wrr-trace",
            "[trace] app-exit gate: closing_label={:?} user_count={} is_browser_pane={} browsers={:?} unpromoted_pool={:?}",
            label, user_browser_count, self.is_browser_pane, browsers_keys, pool_keys
        );

        // Phase F.6 — narrate the post-close pool-drain decision for
        // the launcher's window-cleanup-cascade saga. The saga's
        // Step 2 terminal: `was_last == true` → `Event::PoolDrained`
        // (the wrr two-stage cascade below kicked off Stage 1's
        // pool drain); `was_last == false` → `Event::PoolNotLast`
        // (other windows remain; pool stays warm). Both close the
        // saga's `SagaStarted` bracket successfully — the saga's job
        // is to narrate the decision, not enforce a particular
        // outcome.
        //
        // Same skip-pane gate as `report_panes_reaped` above: the
        // saga is triggered by `Event::WindowClosed`, which only
        // fires for non-pane top-level windows. Pane closes don't
        // start a saga, so the report would be a no-op stray on the
        // bus.
        //
        // Computed here (BEFORE the wrr two-stage cascade below) so
        // the same condition the cascade gates on is what gets
        // reported. The boolean flag captures intent — Stage 1 may
        // not have started yet by the time the report is sent, but
        // the decision itself is final.
        if let Some(ref lbl) = label {
            // Same gate as report_panes_reaped above: skip
            // browser-pane-* only. window-pool-* labels (promoted)
            // are tracked windows and need their cleanup events.
            if !lbl.starts_with("browser-pane-") {
                let was_last = user_browser_count == 0 && !self.is_browser_pane;
                crate::launcher_ipc::report_pool_drain_decision(lbl.clone(), was_last);
            }
        }

        // ── Phase B.9.3 — two-stage close cascade ─────────────────
        //
        // Stage 1: If user_browser_count just dropped to 0 (last
        // user-visible window closed), POST WM_CLOSE to every pool
        // browser. Async — the message loop processes the closes on
        // subsequent iterations. We do NOT call quit_message_loop
        // here. Calling it from inside on_before_close DEADLOCKS the
        // UI thread (smoke v0.33.498 confirmed: log line "calling
        // quit_message_loop now" was last; loop never returned).
        //
        // Stage 2: When self.browser_list becomes empty AFTER
        // removing this browser (i.e. every browser this handler
        // ever managed has closed), THEN call quit_message_loop.
        // Matches the canonical cefsimple pattern. By then there
        // are no other in-flight CEF lifecycle events to deadlock
        // against. The MAIN client's handler is the only one that
        // owns top-level windows + pool windows, so this fires
        // exactly when the entire app's CEF browser inventory is
        // gone.
        //
        // Cross-platform note: the Stage 1 PostMessage is the
        // Windows path. macOS uses NSWindow.performClose:; Linux
        // uses X11 WM_DELETE_WINDOW. Same async-close-cascade
        // semantics on all platforms; only the OS API differs.
        if user_browser_count == 0 && !self.is_browser_pane {
            // PR #5 H.5 — flip QuitState Running → Draining via reducer.
            // Mirrors the pre-PR Phase B.9.3 drain flag: spawn_pool_window
            // checks `quit_state != Running` (in the reducer's spawn arm)
            // and skips refill on every subsequent on_pool_window_destroyed
            // → no new pool browsers added → browsers map can actually
            // drain. BeginDrain is idempotent — safe if a duplicate
            // last-close fires.
            self.state.host_dispatch(
                crate::reducer::HostCommand::BeginDrain {
                    reason: crate::state::QuitReason::LastWindowClosed,
                },
            );
            tracing::warn!(target: "wrr", "[wrr] quit_state=Draining (drain mode)");

            // Phase H.2.b — reducer-aware iteration with fallback + drift logging.
            let pool_browsers: Vec<cef::Browser> = self
                .state
                .list_browsers()
                .into_iter()
                .filter(|(label, _)| label.starts_with("window-pool-"))
                .map(|(_, b)| b)
                .collect();
            tracing::warn!(
                target: "wrr",
                "[wrr] stage 1: user_count==0; closing {} pool browser(s)",
                pool_browsers.len()
            );

            // Windows path: prefer Win32 PostMessage(WM_CLOSE) —
            // bypasses CEF's task queue (proven reliable; smoke
            // v0.33.500+). When window_handle() returns null
            // (early/late lifecycle), fall through to the
            // post_task path so the close still happens — without
            // the fallback, self.browser_list never empties and
            // Stage 2 never fires. (codex #601 P1.)
            #[cfg(target_os = "windows")]
            {
                use windows_sys::Win32::Foundation::HWND;
                use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};
                for (i, mut b) in pool_browsers.into_iter().enumerate() {
                    let hwnd_opt = b.host().and_then(|h| {
                        let wh = h.window_handle();
                        if wh.0.is_null() {
                            None
                        } else {
                            Some(wh.0 as HWND)
                        }
                    });
                    if let Some(hwnd) = hwnd_opt {
                        let ok = unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) };
                        tracing::debug!(
                            target: "wrr-trace",
                            "[trace] stage1[{}] PostMessage(hwnd={:p}, WM_CLOSE) ok={}",
                            i, hwnd, ok != 0
                        );
                    } else {
                        // Fallback: defer close_browser via UI task.
                        // Same path as non-Windows so the cascade
                        // still drains.
                        let mut task = ClosePoolBrowserTask::new(b);
                        let posted = cef::post_task(cef::ThreadId::UI, Some(&mut task));
                        tracing::warn!(
                            target: "wrr",
                            "[wrr] stage1[{}] hwnd=null; fell back to post_task(close_browser) posted={}",
                            i, posted != 0
                        );
                    }
                }
            }

            // Non-Windows path: defer `close_browser` to the UI
            // thread via `cef::post_task`. Calling close_browser
            // inline from inside another browser's on_before_close
            // hangs the UI thread (CEF re-entrance, smoke
            // v0.33.497 confirmed on Windows; same constraint on
            // macOS / Linux per CEF docs). Windows prefers
            // PostMessage(WM_CLOSE) as the primary path (bypasses
            // CEF's task queue, which proved unreliable in
            // late-teardown windows — see
            // `docs/retro/b9-3-quit-thread-analysis.md`).
            // (reagent #601 P1.)
            #[cfg(not(target_os = "windows"))]
            {
                for (i, b) in pool_browsers.into_iter().enumerate() {
                    let mut task = ClosePoolBrowserTask::new(b);
                    let posted = cef::post_task(cef::ThreadId::UI, Some(&mut task));
                    tracing::debug!(
                        target: "wrr-trace",
                        "[trace] stage1[{}] post_task(close_browser) posted={}",
                        i, posted != 0
                    );
                }
            }
        }

        // Stage 2: every browser this handler ever managed is now
        // gone. Safe to call quit_message_loop — no other CEF
        // lifecycle is in flight that could deadlock with it.
        if self.browser_list.is_empty() && !self.is_browser_pane {
            tracing::warn!(
                target: "wrr",
                "[wrr] stage 2: self.browser_list.is_empty() — calling quit_message_loop"
            );
            quit_message_loop();
            tracing::warn!(target: "wrr", "[wrr] quit_message_loop returned");
        } else {
            // Phase B.7.3.3 — `Event::WindowClosed` +
            // `Event::WindowInstanceReleased` from the launcher
            // drive remaining renderers' InstancePanel atoms via the
            // CEF JS bridge; no sync emit here.

            // Tell the backend to clean up this window's workspace/tabs/shells.
            // This replaces the JavaScript `beforeunload` handler — running it here
            // ensures shells die after the CEF browser is gone (not while it's still
            // alive), so Task Manager keeps them grouped until they exit.
            if let Some(window_id) = backend_window_id {
                let web_endpoint = self.state.backend_endpoints.lock().web_endpoint.clone();
                let auth_key = self.state.auth_key.lock().clone();
                dlog(&format!("spawning backend_close_window thread for window_id={}", window_id));
                std::thread::spawn(move || {
                    backend_close_window(&web_endpoint, &auth_key, &window_id);
                });
            } else {
                let warn = format!(
                    "[on_before_close] no backend window ID registered for label={:?} — shells may orphan",
                    label
                );
                dlog(&warn);
                tracing::warn!("{}", warn);
            }
        }

        tracing::debug!(
            target: "wrr-trace",
            "[trace] on_before_close EXIT label={:?} self.browser_list.len()={}",
            label, self.browser_list.len()
        );
    }

    /// CEF fires this whenever the browser's loading/history state changes
    /// (navigation started, navigation committed, back/forward enabled).
    /// `can_go_back` / `can_go_forward` come directly from the navigation
    /// controller — no need to query `browser.can_go_back()` (which races
    /// with history commit when called from `on_load_end`).
    ///
    /// For panes: emit `browser-pane-nav-state` so the frontend address
    /// bar + back/forward buttons reflect CEF's real history state.
    fn on_loading_state_change(
        &mut self,
        browser: Option<&mut Browser>,
        _is_loading: i32,
        can_go_back: i32,
        can_go_forward: i32,
    ) {
        if !self.is_browser_pane {
            return;
        }
        if let Some(b) = browser.as_deref() {
            crate::browser_pane::callbacks::on_loading_state_change_browser_pane(
                &self.state,
                b,
                can_go_back != 0,
                can_go_forward != 0,
            );
        }
    }

    fn on_load_end(
        &mut self,
        browser: Option<&mut Browser>,
        frame: Option<&mut Frame>,
        _http_status_code: i32,
    ) {
        // Inject the IPC port into the page after it finishes loading.
        // Only inject into the main frame (not iframes).
        let Some(frame) = frame else { return };

        if frame.is_main() != 1 {
            return;
        }

        // Pane-specific on_load_end work (focus subclass re-install after
        // Chromium rebuilds Chrome_RenderWidgetHostHWND on navigation)
        // lives in `crate::browser_pane::callbacks` after Phase 4. Returning early
        // skips main-only IPC-port injection below.
        if self.is_browser_pane {
            if let Some(b) = browser.as_deref() {
                crate::browser_pane::callbacks::on_load_end_browser_pane(&self.state, b);
            }
            return;
        }

        let ipc_token = &self.state.ipc_token;
        let js = format!(
            "window.__AGENTMUX_IPC_PORT__ = {}; window.__AGENTMUX_IPC_TOKEN__ = '{}';",
            self.ipc_port, ipc_token
        );
        let code = CefString::from(js.as_str());
        let url = CefString::from("");
        frame.execute_java_script(Some(&code), Some(&url), 0);

        let url_str = browser
            .as_ref()
            .and_then(|b| b.main_frame().map(|f| CefString::from(&f.url()).to_string()))
            .unwrap_or_default();
        tracing::info!(
            "Injected IPC port {} into page: {}",
            self.ipc_port,
            url_str
        );

        // Show window via CEF Views API after content paints.
        // All windows (main + secondary) now use CEF Views.
        let mut browser_cloned = browser.cloned();
        if let Some(bv) = browser_view_get_for_browser(browser_cloned.as_mut()) {
            if let Some(window) = bv.window() {
                if window.is_visible() == 0 {
                    window.show();
                    if let Some(ref mut b) = browser_cloned {
                        if let Some(host) = b.host() {
                            host.set_focus(1);
                        }
                    }
                }
            }
        }
    }

    fn on_load_error(
        &mut self,
        _browser: Option<&mut Browser>,
        frame: Option<&mut Frame>,
        error_code: Errorcode,
        error_text: Option<&CefString>,
        failed_url: Option<&CefString>,
    ) {
        debug_assert_ne!(currently_on(ThreadId::UI), 0);

        let error_code_raw = sys::cef_errorcode_t::from(error_code);
        if error_code_raw == sys::cef_errorcode_t::ERR_ABORTED {
            return;
        }

        let frame = frame.expect("Frame is None");

        // Don't show error pages for sub-frames (iframes) — only for
        // the main frame. Without this, an iframe blocked by
        // X-Frame-Options replaces the entire app with an error page.
        if frame.is_main() != 1 {
            return;
        }
        let error_text = error_text.map(CefString::to_string).unwrap_or_default();
        let failed_url = failed_url.map(CefString::to_string).unwrap_or_default();
        let error_code_i32 = error_code_raw as i32;

        tracing::error!(
            "Load error: url={} error={} ({})",
            failed_url,
            error_text,
            error_code_i32
        );

        // Show a user-friendly error page.
        let html = format!(
            r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <style>
        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: #1e1e2e;
            color: #cdd6f4;
            display: flex;
            justify-content: center;
            align-items: center;
            height: 100vh;
            margin: 0;
        }}
        .error-container {{
            text-align: center;
            max-width: 600px;
            padding: 40px;
        }}
        h1 {{ color: #f38ba8; font-size: 24px; }}
        p {{ color: #a6adc8; line-height: 1.6; }}
        code {{
            background: #313244;
            padding: 2px 8px;
            border-radius: 4px;
            font-size: 14px;
        }}
        .retry {{
            margin-top: 20px;
            padding: 10px 24px;
            background: #89b4fa;
            color: #1e1e2e;
            border: none;
            border-radius: 6px;
            cursor: pointer;
            font-size: 14px;
        }}
    </style>
</head>
<body>
    <div class="error-container">
        <h1>Failed to load AgentMux frontend</h1>
        <p>Could not connect to <code>{failed_url}</code></p>
        <p>Error: {error_text} ({error_code_i32})</p>
        <p>Make sure the Vite dev server is running:<br>
           <code>task dev</code> or <code>npx vite</code></p>
        <button class="retry" onclick="location.reload()">Retry</button>
    </div>
</body>
</html>"#
        );

        let b64 = cef::base64_encode(Some(html.as_bytes()));
        let b64_str = CefString::from(&b64).to_string();
        let data_uri = format!("data:text/html;base64,{}", b64_str);
        let uri = CefString::from(data_uri.as_str());
        frame.load_url(Some(&uri));
    }

    /// Render-process terminated — typically OOM, a renderer-side panic, or
    /// some native bug inside CEF/Chromium. Without this hook the window
    /// just turns white. We log the cause and replace the white page with
    /// a recovery HTML page that offers Reload / Quit buttons.
    ///
    /// See specs/SPEC_GRACEFUL_CRASH_HANDLING_2026_04_13.md (PR 1).
    fn on_render_process_terminated(
        &mut self,
        browser: Option<&mut Browser>,
        status: TerminationStatus,
        error_code: i32,
        error_string: Option<&CefString>,
    ) {
        let reason = if status == TerminationStatus::PROCESS_OOM {
            "out of memory"
        } else if status == TerminationStatus::PROCESS_CRASHED {
            "renderer process crashed"
        } else if status == TerminationStatus::ABNORMAL_TERMINATION {
            "abnormal termination"
        } else {
            "renderer process terminated"
        };

        let detail = error_string.map(CefString::to_string).unwrap_or_default();
        tracing::error!(
            target: "crash",
            kind = "renderer_terminated",
            reason,
            error_code,
            detail = %detail,
            "{}", reason,
        );

        // Resolve the real frontend URL so the Reload button can navigate
        // back to the live app instead of reloading the recovery page
        // itself. Matches the format used by
        // commands::window::resolve_frontend_base_url and its callers
        // (see window.rs:400, window.rs:430, drag.rs:294 — all use the
        // same ipc_port / ipc_token query params).
        let base_url = crate::commands::window::resolve_frontend_base_url(self.ipc_port);
        let separator = if base_url.contains('?') { "&" } else { "?" };
        let app_url = format!(
            "{}{}ipc_port={}&ipc_token={}",
            base_url, separator, self.ipc_port, self.state.ipc_token
        );

        let detail_block = if detail.is_empty() {
            String::new()
        } else {
            format!("<p class=\"detail\"><code>{}</code></p>", html_escape(&detail))
        };

        // Build the recovery page. Plain HTML+CSS, no JS dependencies
        // beyond a single click handler, so it renders even if the
        // frontend bundle is dead. The Reload button navigates directly
        // to the real app URL (NOT location.reload() — that would just
        // re-render the same data: URL). CEF will spawn a fresh renderer
        // subprocess for the navigation.
        //
        // NOTE on ipc_token exposure: the token is already present in
        // the live app URL that was loaded before the crash (it's in
        // the location bar for the dead renderer's process). Embedding
        // it in the recovery HTML that runs inside the same browser
        // doesn't extend its reach — the HTML is ephemeral, not
        // persisted to disk, and `window.close()` or the next crash
        // clears it.
        let html = format!(
            r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="utf-8">
    <title>AgentMux — Recovery</title>
    <style>
        :root {{
            color-scheme: dark;
        }}
        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: #1e1e2e;
            color: #cdd6f4;
            display: flex;
            justify-content: center;
            align-items: center;
            min-height: 100vh;
            margin: 0;
            padding: 24px;
            box-sizing: border-box;
        }}
        .recovery {{
            text-align: center;
            max-width: 540px;
            padding: 36px;
            background: #181825;
            border: 1px solid #313244;
            border-radius: 10px;
            box-shadow: 0 8px 32px rgba(0, 0, 0, 0.4);
        }}
        .icon {{
            font-size: 36px;
            line-height: 1;
            margin-bottom: 12px;
        }}
        h1 {{
            color: #f9e2af;
            font-size: 22px;
            margin: 0 0 6px 0;
        }}
        .reason {{
            color: #a6adc8;
            font-size: 14px;
            margin: 0 0 20px 0;
            font-style: italic;
        }}
        p {{
            color: #bac2de;
            line-height: 1.55;
            margin: 0 0 12px 0;
            font-size: 14px;
        }}
        .detail code {{
            display: inline-block;
            background: #313244;
            color: #f38ba8;
            padding: 6px 10px;
            border-radius: 4px;
            font-size: 12px;
            font-family: ui-monospace, 'Cascadia Code', Menlo, Consolas, monospace;
            word-break: break-all;
            text-align: left;
            max-width: 100%;
        }}
        .actions {{
            display: flex;
            gap: 10px;
            justify-content: center;
            margin-top: 24px;
            flex-wrap: wrap;
        }}
        button {{
            padding: 10px 22px;
            border: 1px solid #45475a;
            border-radius: 6px;
            background: #313244;
            color: #cdd6f4;
            cursor: pointer;
            font-size: 13px;
            font-family: inherit;
            transition: background 0.1s, border-color 0.1s;
        }}
        button:hover {{
            background: #45475a;
            border-color: #585b70;
        }}
        button.primary {{
            background: #89b4fa;
            color: #1e1e2e;
            border-color: #89b4fa;
            font-weight: 600;
        }}
        button.primary:hover {{
            background: #74a0f8;
            border-color: #74a0f8;
        }}
        .footer {{
            color: #6c7086;
            font-size: 11px;
            margin-top: 18px;
            font-family: ui-monospace, monospace;
        }}
    </style>
</head>
<body>
    <div class="recovery" role="alertdialog" aria-labelledby="title">
        <div class="icon">⚠</div>
        <h1 id="title">AgentMux hit a problem</h1>
        <p class="reason">Reason: {reason_safe}</p>
        {detail_block}
        <p>Your open sessions are saved on disk. Reloading will bring you back where you left off.</p>
        <div class="actions">
            <button class="primary" id="reload-btn">Reload window</button>
            <button onclick="window.close()">Quit</button>
        </div>
        <div class="footer">error_code={error_code}</div>
    </div>
    <script>
        // The Reload button navigates to the live app URL (not
        // location.reload, which would just re-render this data: page).
        // The URL is injected by the host at HTML-build time.
        document.getElementById('reload-btn').addEventListener('click', function() {{
            location.href = {app_url_js};
        }});
    </script>
</body>
</html>"#,
            reason_safe = html_escape(reason),
            detail_block = detail_block,
            error_code = error_code,
            app_url_js = js_string_literal(&app_url),
        );

        // Load the recovery page in the main frame of the dead browser.
        // The renderer subprocess will be re-spawned by CEF when we
        // navigate, so the new page mounts in a fresh process.
        if let Some(b) = browser {
            if let Some(frame) = b.main_frame() {
                let b64 = cef::base64_encode(Some(html.as_bytes()));
                let b64_str = CefString::from(&b64).to_string();
                let data_uri = format!("data:text/html;base64,{}", b64_str);
                let uri = CefString::from(data_uri.as_str());
                frame.load_url(Some(&uri));
            }
        }
    }
}




