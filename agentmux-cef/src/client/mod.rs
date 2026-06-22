// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// CefClient and associated handler implementations.
// Manages browser lifecycle, display updates, and load errors.
//
// Phase 2: Stores browser ref in AppState and injects IPC port on page load.

use cef::*;
use std::collections::{HashMap, VecDeque};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use parking_lot::Mutex;

use crate::state::{AppState, WindowKind};

/// Maximum number of renderer crashes per browser within `CRASH_BUDGET_WINDOW`
/// before `on_render_process_terminated` stops auto-recovery and loads a
/// terminal "give up" page instead. Picked at 3 because the normal recovery
/// path (load a `data:` URL → fresh renderer paints it once) reaches steady
/// state in one cycle; multiple consecutive crashes always indicate an
/// unrecoverable state, not transient flakiness. See SPEC_SERVICE_SUPERVISION
/// prime directive — "bounded recovery; never an infinite restart loop."
const CRASH_BUDGET: usize = 3;

/// Rolling window over which the crash budget is enforced. 10 s catches the
/// 2026-05-28 incident pattern (108 crashes/sec for 22 min — would have
/// tripped on the first batch in <30 ms) without false-positiving on a
/// renderer that genuinely needed 2 retries to stabilise across a minute.
const CRASH_BUDGET_WINDOW: Duration = Duration::from_secs(10);

/// Commit-free (available page file) floor, in MB, below which an OOM renderer
/// termination is treated as transient SYSTEM memory pressure rather than a
/// broken renderer. Below this, recovery does NOT consume the wedged-slot crash
/// budget and the browser is shown a recoverable "low memory" page instead of
/// the give-up page. A fresh renderer's initial commit is ~100-200 MB; 512 MB
/// leaves margin so a manual Resume doesn't instantly re-OOM.
/// See docs/specs/SPEC_GATED_RENDERER_RECOVERY_2026_06_01.md §6.B/§7.
const RESUME_FLOOR_MB: u64 = 512;

/// Backstop for the memory-pause path: if a browser enters memory-pause more
/// than this many times within `MEMORY_PAUSE_WINDOW` — i.e. commit is so
/// totally exhausted that even the tiny low-memory page can't render and
/// re-fires this handler — fall through to the give-up page so we stop
/// re-spawning a renderer that instantly dies. Deliberately more lenient than
/// `CRASH_BUDGET`: this path is *expected* to repeat under sustained pressure,
/// so it must not converge as fast. (Native-overlay handling of total
/// exhaustion — rendering the paused UI without a renderer — is Phase 1b.)
const MEMORY_PAUSE_BUDGET: usize = 5;
const MEMORY_PAUSE_WINDOW: Duration = Duration::from_secs(30);

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
    /// Per-browser ring of renderer-crash timestamps used by
    /// `on_render_process_terminated` to enforce `CRASH_BUDGET` within
    /// `CRASH_BUDGET_WINDOW`. Keyed by `Browser::identifier()`. Entries
    /// are pruned in-place on each crash event (entries older than the
    /// window are dropped); when a browser closes cleanly its entry is
    /// removed in `on_before_close`.
    crash_history: HashMap<i32, VecDeque<Instant>>,
    /// Per-browser ring of memory-pause timestamps (OOM under low system
    /// commit). Separate from `crash_history` so transient system-memory
    /// pressure never trips the wedged-slot crash budget. Bounded by
    /// `MEMORY_PAUSE_BUDGET` within `MEMORY_PAUSE_WINDOW`; pruned in place and
    /// removed on clean close, exactly like `crash_history`.
    /// See docs/specs/SPEC_GATED_RENDERER_RECOVERY_2026_06_01.md §6.B.
    memory_pause_history: HashMap<i32, VecDeque<Instant>>,
}

mod handlers;
pub(crate) mod helpers;
#[cfg(target_os = "windows")]
mod wndproc;
#[cfg(target_os = "windows")]
pub(crate) use wndproc::install_main_window_floater_cascade_hook;

pub use handlers::AgentMuxClient;

use helpers::{js_string_literal, html_escape, backend_close_window};
#[cfg(target_os = "windows")]
use wndproc::{
    install_top_level_focus_restore_hook,
    set_window_icon, skip_taskbar,
};

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
            crash_history: HashMap::new(),
            memory_pause_history: HashMap::new(),
        }))
    }

    /// Reverse-lookup this browser's window label from the reducer's browsers
    /// map (label → browser), by object identity. Mirrors the find-by-identity
    /// loop in `on_before_close`. Used by the crash-recovery navigation so a
    /// resumed *secondary* window preserves its `windowLabel` instead of
    /// defaulting to `main` (the frontend treats a missing label as `main`,
    /// which would re-register/route the recovered window as the wrong one).
    /// (codex P2 on #1229.)
    fn window_label_for(&self, browser: &mut Browser) -> Option<String> {
        self.state
            .list_browsers()
            .into_iter()
            .find(|(_, b)| {
                let mut b = b.clone();
                b.is_same(Some(&mut *browser)) != 0
            })
            .map(|(k, _)| k)
    }

    /// Best navigation target for bringing a crashed window back. Prefers the
    /// window's OWN pre-crash URL (read from its main frame before we load any
    /// recovery page over it): it already carries every creation-time param —
    /// `windowLabel` and, for tear-off / floating-pane windows, `workspaceId`
    /// / `floatingPaneId` — plus the still-valid in-process `ipc_port` /
    /// `ipc_token`, so reusing it verbatim re-projects the exact window. Falls
    /// back to a reconstructed `?ipc_port&ipc_token&windowLabel` URL only when
    /// the frame URL isn't a live app URL (renderer died before committing one,
    /// or it's a prior data: recovery page). codex P2 #1229.
    fn recovery_target_url(&self, owned: &mut Browser, base_url: &str) -> String {
        // Only reuse the pre-crash URL if it's on the SAME origin we'd reload
        // from (`base_url`) — that's `http://127.0.0.1:<ipc_port>` for
        // installed/portable and `http://localhost:<vite_port>` for dev, so
        // matching against base_url rather than a hardcoded host covers both
        // (codex P2 #1229: dev tear-off URLs are localhost).
        let pre_crash = owned
            .main_frame()
            .map(|f| CefString::from(&f.url()).to_string())
            .filter(|u| url_on_origin(u, base_url));
        if let Some(u) = pre_crash {
            return u;
        }
        let label = self.window_label_for(owned);
        recovery_navigation_url(base_url, self.ipc_port, &self.state.ipc_token, label.as_deref())
    }

    fn on_title_change(&mut self, browser: Option<&mut Browser>, title: Option<&CefString>) {
        debug_assert_ne!(currently_on(ThreadId::UI), 0);

        let title_str = title.map(|t| t.to_string()).unwrap_or_default();

        // Update the window title via CEF Views.
        let mut browser = browser.cloned();
        if let Some(browser_view) = browser_view_get_for_browser(browser.as_mut()) {
            if let Some(window) = browser_view.window() {
                window.set_title(title);
            }
        }
        // For Alloy-style native windows on Windows, update via Win32 API.
        // Reagent P1 on #876: only call SetWindowTextW when CEF gave us an
        // actual title. CEF fires `on_title_change` with `title = None` in
        // several paths (e.g. about:blank, popup blockers) — passing "" to
        // SetWindowTextW would blank the application window title in those
        // cases. Preserve the existing title by skipping the Win32 update
        // when title is None.
        #[cfg(target_os = "windows")]
        if title.is_some() {
            if let Some(browser) = browser.as_ref() {
                if let Some(host) = browser.host() {
                    let hwnd = host.window_handle();
                    if !hwnd.0.is_null() {
                        let title_wide: Vec<u16> = title_str
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

        // Emit live title to frontend for browser panes.
        if self.is_browser_pane {
            if let Some(b) = browser.as_ref() {
                if let Some(block_id) =
                    crate::browser_pane::callbacks::resolve_pane_block_id(&self.state, b)
                {
                    let block_id_short: String = block_id.chars().take(7).collect();
                    tracing::info!(
                        "[browser-pane:diag][{}] emit-title-change title={:?}",
                        block_id_short,
                        title_str,
                    );
                    crate::events::emit_event_from_state(
                        &self.state,
                        "browser-pane-title-change",
                        &serde_json::json!({ "block_id": block_id, "title": title_str }),
                    );
                }
            }
        }
    }

    fn on_favicon_urlchange(
        &mut self,
        browser: Option<&mut Browser>,
        icon_urls: Option<&mut CefStringList>,
    ) {
        if !self.is_browser_pane {
            return;
        }
        let Some(b) = browser.as_deref() else { return };
        let Some(block_id) =
            crate::browser_pane::callbacks::resolve_pane_block_id(&self.state, b)
        else {
            return;
        };

        // Collect favicon URLs from the CefStringList. The list is an in-param
        // provided by CEF — we read via the raw sys API so we don't need to
        // consume (move) the borrowed reference.
        //
        // Reagent P1 on #876: `cef_string_list_value` writes into a
        // `cef_string_t` whose `str_` field points at a freshly-allocated
        // buffer owned by the list value (with `dtor` set to release it).
        // Dropping `value` as a plain Rust struct would leak that buffer on
        // every favicon URL CEF reports. After reading the string, we must
        // invoke the dtor manually to free the buffer.
        let urls: Vec<String> = if let Some(list) = icon_urls {
            let raw: *mut cef::sys::_cef_string_list_t = list.into();
            if let Some(raw_ref) = unsafe { raw.as_mut() } {
                let count = unsafe { cef::sys::cef_string_list_size(raw_ref) };
                (0..count)
                    .filter_map(|i| unsafe {
                        let mut value: cef::sys::cef_string_t = std::mem::zeroed();
                        if cef::sys::cef_string_list_value(raw_ref, i, &mut value) > 0 {
                            let s = CefString::from(std::ptr::from_ref(&value)).to_string();
                            // Free the buffer CEF allocated into `value.str_`.
                            if let Some(dtor) = value.dtor {
                                dtor(value.str_);
                            }
                            Some(s)
                        } else {
                            None
                        }
                    })
                    .collect()
            } else {
                vec![]
            }
        } else {
            vec![]
        };

        let block_id_short: String = block_id.chars().take(7).collect();
        tracing::info!(
            "[browser-pane:diag][{}] emit-favicon-urls count={} first={:?}",
            block_id_short,
            urls.len(),
            urls.first(),
        );
        crate::events::emit_event_from_state(
            &self.state,
            "browser-pane-favicon-urls",
            &serde_json::json!({ "block_id": block_id, "urls": urls }),
        );
    }

    fn on_after_created(&mut self, browser: Option<&mut Browser>) {
        debug_assert_ne!(currently_on(ThreadId::UI), 0);

        let Some(browser) = browser.cloned() else {
            tracing::error!("[on_after_created] browser is None — skipping registration");
            return;
        };
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

                // Subclass for the focus-restore-on-WM_ACTIVATE behavior
                // (window-reactivate-focus-restore spec §5.1.3). Observes
                // WM_ACTIVATE only; all messages pass through to CEF.
                // Install on every top-level — both `main` and Subwindow.
                if is_top_level_window {
                    unsafe { install_top_level_focus_restore_hook(hwnd); }
                }

                // Floater cascade hook (issue #1560): replaces the Win32
                // owned-window z-order/minimize/destroy invariant now that
                // floaters are unowned WS_POPUP windows. Install on all
                // FullInstance windows (not pool, not subwindow) so that
                // closing or minimizing any main window cascades to its floaters.
                #[cfg(target_os = "windows")]
                if is_top_level_window
                    && pending_kind == WindowKind::FullInstance
                    && !label.starts_with("window-pool-")
                    && !label.starts_with("floating-pool-")
                {
                    unsafe { install_main_window_floater_cascade_hook(hwnd); }
                }

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
        // read-only state mirror. Skips browser-pane child HWNDs,
        // tab pool windows (`window-pool-*`), and pane pool windows
        // (`floating-pool-*`). Pane pool windows are excluded here AND
        // in `host_counts_snapshot` (state.rs) so the launcher mirror's
        // windows count stays in sync with the host count on all platforms.
        // No-op if launcher IPC isn't connected (`task dev` mode).
        if is_top_level_window && !label.starts_with("window-pool-") && !label.starts_with("floating-pool-") {
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
            crate::commands::window_pool::init_pane_pool(&self.state);
        } else if label.starts_with("window-pool-") {
            crate::commands::window_pool::register_pool_window(&self.state, &label);
        } else if label.starts_with("floating-pool-") {
            crate::commands::window_pool::register_pane_pool_window(&self.state, &label);
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
    /// they don't spawn rogue top-level CEF windows.
    ///
    /// Routing depends on who fired the popup and where it points:
    /// * **App UI → external site** (e.g. the Help pane's GitHub / docs /
    ///   Discord links): open in the **system browser** and cancel. Navigating
    ///   the app window itself to an external origin replaces the AgentMux UI
    ///   and strands the window on "Can't reconnect". See
    ///   `SPEC_HELP_EXTERNAL_LINKS_AND_RESTORE_2026_06_17.md`.
    /// * **Browser pane, or internal URL**: navigate the **current** frame to
    ///   the target URL — matches the UX that AgentMux owns window management,
    ///   and in a browser pane following a link IS the point.
    ///
    /// Returning non-zero cancels popup creation. Applies to both main and pane
    /// clients; panes explicitly don't want top-level popups. See
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

        // External links from the app UI (Help pane's "Report Bugs & Issues",
        // docs, Discord, …) must open in the SYSTEM browser — never navigate
        // the current frame. Navigating the app's own window to an external
        // origin replaces the whole AgentMux UI with that site and tears down
        // the host bridge: the window comes back bridge-dead on "Can't
        // reconnect" (the window.api init race), which is exactly the lock-out
        // this fixes. Browser panes are exempt — for them, following a link IS
        // the point, so they keep in-pane navigation.
        // `open_url_in_default_browser` only spawns a child process (rundll32 /
        // open / xdg-open); it never re-enters CEF or `self.inner`, so calling
        // it inline here (under the handler lock) cannot deadlock the way an
        // inline `load_url` would.
        if !self.is_browser_pane && crate::commands::platform::is_external_http_url(&url) {
            match crate::commands::platform::open_url_in_default_browser(&url) {
                Ok(()) => tracing::info!(url = %url, "external link opened in system browser"),
                Err(e) => tracing::warn!(
                    url = %url,
                    error = %e,
                    "failed to open external link in system browser",
                ),
            }
            return true; // cancel popup; do NOT navigate the app frame
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

        let Some(mut browser) = browser.cloned() else {
            // CEF can pass None during emergency teardown (e.g. process
            // shutdown while a browser is still closing). Log and bail —
            // a panic here SIGABRTs CrBrowserMain, which the launcher
            // mistakes for a crash and relaunches the app.
            tracing::error!("[on_before_close] browser is None — skipping close logic");
            return;
        };

        // Drop any crash-history entry for this browser — it's closing
        // cleanly so its budget is reset. Without this the map would
        // accumulate one stale entry per closed browser over a session.
        self.crash_history.remove(&browser.identifier());
        self.memory_pause_history.remove(&browser.identifier());

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

            // Evict this label's HWND from `window_hwnds`. The cache
            // has no other cleanup path, and the resolver's hot-path
            // hits it before walking the registry — without this,
            // a subsequent open of the same label (e.g. main
            // restart) leaves a stale entry that breaks WM_CLOSE
            // routing. See
            // docs/specs/SPEC_WINDOW_HWND_CACHE_STALE_FIX_2026_05_28.md.
            // Windows-only because `AppState::window_hwnds` is itself
            // `#[cfg(target_os = "windows")]` in `state.rs`. Codex P1
            // on PR #1133.
            #[cfg(target_os = "windows")]
            {
                let removed = self.state.window_hwnds.lock().remove(lbl);
                if removed.is_some() {
                    tracing::debug!(
                        target: "win-resolve",
                        label = %lbl,
                        "[win-resolve] evicted on close"
                    );
                }
            }

            // Co-evict the floater's window-placement entry (pane-state
            // reducer). Floaters key `pane_window_states` by window label,
            // and they're NOT in `browser_panes`, so this close hook — the
            // same place `window_hwnds` is evicted — is the correct cleanup
            // site. Gated to `floating-` labels (the only ones that ever
            // hold an entry); the reducer arm is itself idempotent/no-op if
            // absent. See SPEC_PANE_STATE_REDUCER_2026-05-28.md (REVISION
            // 2026-05-29).
            if lbl.starts_with("floating-") {
                self.state.host_dispatch(
                    crate::reducer::HostCommand::EvictFloatingPaneWindowState {
                        label: lbl.to_string(),
                    },
                );
            }
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
            } else if lbl.starts_with("floating-pool-") {
                crate::commands::window_pool::on_pane_pool_window_destroyed(&self.state, lbl);
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

        // App-exit decision (authoritative): count remaining live USER windows
        // by the per-browser `BrowserKind::is_pool` flag, NOT pool
        // set-membership. The closing browser was already removed from the
        // reducer's `browsers` map above (`UnregisterBrowser`), so this reflects
        // what REMAINS.
        //
        // Why the flag, not `user_visibility_snapshot()`'s pool-SET count
        // (which this used to use, and which is kept below for logging only):
        // the snapshot excludes labels found in `pool.unpromoted ∪ pool.queue`.
        // If a pool window left those SETS without its `is_pool` flag clearing
        // (a failed/partial promote, an out-of-band drop), the snapshot counted
        // it as user-visible while it was really a hidden scratch window —
        // `user_browser_count` never hit 0, `BeginDrain` never fired, and the
        // host never quit. That is the orphaned-process-tree regression
        // (confirmed: 9,483-line orphan host log with no drain marker). The
        // `is_pool` flag is the single source of truth, flipped atomically at
        // promote (`pool.rs`) and read here under one lock, so it can't drift or
        // race a concurrent promote the way the two-set read can.
        //
        // A PROMOTED pool window keeps its `window-pool-*` label but is
        // `is_pool: false`, so it still correctly counts; unpromoted pool
        // windows (`is_pool: true`) and `BrowserKind::Pane` children don't. See
        // SPEC_INSTANCE_LIFECYCLE_CONSOLIDATION_2026_06_21.md §5.1/§10.1.
        let user_browser_count = self.state.count_live_user_windows();

        // Snapshot retained for the diagnostic trace below only (label lists);
        // the gate above is the authoritative `is_pool` count.
        let (browsers_keys, pool_keys) = {
            let (pool_labels, browsers) = self.state.user_visibility_snapshot();
            let keys: Vec<String> = browsers.into_iter().map(|(l, _)| l).collect();
            let pool: Vec<String> = pool_labels.into_iter().collect();
            (keys, pool)
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
            // Collect ALL background-only browsers: tab pool (window-pool-*)
            // AND pane pool (floating-pool-*). Both live in browser_list
            // (created via CreateWindowTask which clones the main top-level
            // client). Omitting pane pool windows here means browser_list
            // never empties on macOS/Linux (init_pane_pool spawns one at
            // startup), so Stage 2's is_empty() gate never fires and the
            // host hangs on every quit.
            let pool_browsers: Vec<cef::Browser> = self
                .state
                .list_browsers()
                .into_iter()
                .filter(|(label, _)| {
                    label.starts_with("window-pool-") || label.starts_with("floating-pool-")
                })
                .map(|(_, b)| b)
                .collect();
            tracing::warn!(
                target: "wrr",
                "[wrr] stage 1: user_count==0; closing {} pool browser(s) (tab+pane)",
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

        // Signal the pre-splash to fade out the moment CEF's first frame
        // is ready. The launcher created this named event and forwarded
        // its name via AGENTMUX_SPLASH_EVENT. OpenEventW + SetEvent is
        // fire-and-forget; missing env var means no splash was running.
        #[cfg(target_os = "windows")]
        {
            use windows_sys::Win32::Foundation::CloseHandle;
            use windows_sys::Win32::System::Threading::{
                OpenEventW, SetEvent, EVENT_MODIFY_STATE,
            };
            if let Ok(event_name) = std::env::var("AGENTMUX_SPLASH_EVENT") {
                let nul: Vec<u16> = format!("{}\0", event_name).encode_utf16().collect();
                unsafe {
                    let ev = OpenEventW(EVENT_MODIFY_STATE, 0, nul.as_ptr());
                    if !ev.is_null() {
                        SetEvent(ev);
                        CloseHandle(ev);
                    }
                }
            }
        }

        // macOS/Linux analogue of the Win32 splash signal: the launcher owns the
        // native splash (see agentmux-launcher/src/splash_mac.rs and splash_linux/)
        // and passes a ready-file path via AGENTMUX_SPLASH_READY_FILE. Creating the
        // file is the cross-process "first frame painted" signal the launcher polls
        // for before tearing the splash down. Fire-and-forget; absent var => no
        // launcher splash (e.g. dev:standalone), so this is a no-op.
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            if let Ok(path) = std::env::var("AGENTMUX_SPLASH_READY_FILE") {
                if !path.is_empty() {
                    let _ = std::fs::write(&path, b"ready");
                }
            }
        }

        // Show window via CEF Views API after content paints.
        // All windows (main + secondary) now use CEF Views.
        let mut browser_cloned = browser.cloned();

        // Pool windows are kept hidden until promote_pool_window fires
        // PromotePoolWindowTask, which positions the window with set_bounds()
        // and then calls window.show(). On macOS/Linux, CEF Views Window::Show()
        // activates the widget (no foreground-lock equivalent), so showing an
        // off-screen pool window here — even at (-32000,-32000) — would steal
        // key focus. Instead, pool windows skip the show/focus block entirely
        // and are shown for the first time at the promote-target position.
        let browser_label = browser_cloned.as_mut().and_then(|b| self.window_label_for(b));
        let is_pool_window = browser_label
            .as_deref()
            .map_or(false, |l| l.starts_with("window-pool-") || l.starts_with("floating-pool-"));

        if !is_pool_window {
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
    }

    fn on_load_error(
        &mut self,
        browser: Option<&mut Browser>,
        frame: Option<&mut Frame>,
        error_code: Errorcode,
        error_text: Option<&CefString>,
        failed_url: Option<&CefString>,
    ) {
        debug_assert_ne!(currently_on(ThreadId::UI), 0);

        let error_code_raw = sys::cef_errorcode_t::from(error_code);

        // [DIAG] Unconditional entry log — captures every load error
        // BEFORE the ERR_ABORTED filter, sub-frame filter, or fallback-
        // page render. Pair with `[browser-pane-auth][ENTRY]` to
        // diagnose the auth-modal-doesn't-appear path: if we see a
        // load-error here with no preceding auth-credentials entry,
        // CEF skipped the auth flow entirely.
        let failed_url_dbg = failed_url.map(CefString::to_string).unwrap_or_default();
        let error_text_dbg = error_text.map(CefString::to_string).unwrap_or_default();
        // `as_ref()` + auto-deref on `&&mut Frame` — relies only on
        // `is_main(&self)` resolving through normal method-call deref,
        // not on `Deref` for the `Option::as_deref()` blanket impl.
        let is_main_frame = match frame.as_ref() {
            Some(f) => f.is_main() == 1,
            None => false,
        };
        tracing::info!(
            "[load-error][ENTRY] url={:?} error={:?} ({}) is_main_frame={} aborted={}",
            failed_url_dbg,
            error_text_dbg,
            error_code_raw as i32,
            is_main_frame,
            error_code_raw == sys::cef_errorcode_t::ERR_ABORTED,
        );

        // Persistent pane lifecycle trace (see browser_pane::trace). Recorded
        // BEFORE the ERR_ABORTED early-return below, because ERR_ABORTED on a
        // pane's main frame is exactly the black-render-on-redock signature
        // (the re-created pane's navigation was aborted mid-load).
        if self.is_browser_pane && is_main_frame {
            if let Some(b) = browser.as_deref() {
                if let Some(block_id) =
                    crate::browser_pane::callbacks::resolve_pane_block_id(&self.state, b)
                {
                    crate::browser_pane::trace::pane_trace(
                        &block_id,
                        "load-error",
                        &format!(
                            "url={failed_url_dbg} err={error_text_dbg}({}) aborted={}",
                            error_code_raw as i32,
                            error_code_raw == sys::cef_errorcode_t::ERR_ABORTED,
                        ),
                    );
                }
            }
        }

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

        // JSON-encode the URL so it is a safe JS string literal: a real URL can
        // contain a single quote (e.g. `?q=can't`), which would otherwise break
        // the interpolated JS in the Retry handler below.
        let failed_url_js =
            serde_json::to_string(&failed_url).unwrap_or_else(|_| "\"\"".to_string());
        // Auto-retry ONLY for the dev frontend (the main window), which commonly
        // races the Vite dev server on launch. Browser panes load arbitrary user
        // URLs through this SAME handler — auto-retrying their failures (offline
        // site, DNS error, refused service) would be an unbounded reload loop, so
        // panes get a manual Retry only.
        let auto_retry = if self.is_browser_pane {
            String::new()
        } else {
            "setTimeout(__amxRetry, 1200);".to_string()
        };

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
        <button class="retry" onclick="__amxRetry()">Retry</button>
    <script>
        // This error page is itself a data: URI, so location.reload() would just
        // reload THIS page (and a data: reload aborts) — the original URL would
        // never be re-tried. Navigate to the real failed URL instead.
        var __amxTarget = {failed_url_js};
        function __amxRetry() {{ location.href = __amxTarget; }}
        {auto_retry}
    </script>
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
        // Rate-limit the renderer_terminated event on the `crash`
        // target. The crash budget below caps per-browser crashes at
        // CRASH_BUDGET within CRASH_BUDGET_WINDOW, but if many
        // browsers crash simultaneously the aggregate write rate to
        // the host log can still spike. RENDERER_TERMINATED_LOG_MIN_GAP
        // throttles to at most one full log line per 100 ms across
        // the whole process; suppressed events are counted and the
        // count is emitted as a `suppressed` field on the next
        // un-throttled event so no information is lost. See
        // docs/retro/retro-portable-rm-running-install-2026-05-28.md
        // for the original 884 MB / 22 min log spam that motivated
        // both this and the per-browser budget.
        const RENDERER_TERMINATED_LOG_MIN_GAP: Duration = Duration::from_millis(100);
        static LAST_LOGGED_AT_MS: AtomicU64 = AtomicU64::new(0);
        static SUPPRESSED_SINCE: AtomicU64 = AtomicU64::new(0);
        let now_ms = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        let last_ms = LAST_LOGGED_AT_MS.load(Ordering::Relaxed);
        if now_ms.saturating_sub(last_ms) < RENDERER_TERMINATED_LOG_MIN_GAP.as_millis() as u64 {
            SUPPRESSED_SINCE.fetch_add(1, Ordering::Relaxed);
        } else {
            let suppressed = SUPPRESSED_SINCE.swap(0, Ordering::Relaxed);
            LAST_LOGGED_AT_MS.store(now_ms, Ordering::Relaxed);
            tracing::error!(
                target: "crash",
                kind = "renderer_terminated",
                reason,
                error_code,
                detail = %detail,
                suppressed_since_last = suppressed,
                "{}", reason,
            );
        }

        // ── Gated renderer recovery (SPEC_GATED_RENDERER_RECOVERY §6.B) ──
        // A renderer OOM while the system commit limit is exhausted is
        // transient OS pressure, NOT a broken renderer — no amount of retrying
        // helps until memory frees, and the standard 3-crashes-in-10s budget
        // would wrongly declare "give up" on a window that is fully recoverable
        // once commit drops. So: if the termination is PROCESS_OOM and
        // commit-free is below RESUME_FLOOR_MB, do NOT consume the crash
        // budget; show a recoverable "low memory" page (state is durable in
        // srv, so Resume re-projects everything). Bounded separately by
        // MEMORY_PAUSE_BUDGET so that *total* exhaustion — where even this tiny
        // page can't render and re-fires the handler — still converges on the
        // give-up page rather than looping. (Auto-resume + a renderer-free
        // native overlay for total exhaustion are Phase 1b.)
        if status == TerminationStatus::PROCESS_OOM {
            let commit_free = crate::memory_heartbeat::commit_free_mb();
            if commit_free < RESUME_FLOOR_MB {
                if let Some(bid) = browser.as_ref().map(|b| b.identifier()) {
                    // Resolve the live app URL up front: if the frontend assets
                    // are unavailable that's a *different* failure (the
                    // 2026-05-28 rm-while-running pattern), so leave the
                    // memory-pause history untouched and fall through to the
                    // normal path, which has the assets-missing handling.
                    if let Ok(base_url) =
                        crate::commands::window::resolve_frontend_base_url(self.ipc_port)
                    {
                        let now = Instant::now();
                        let hist = self.memory_pause_history.entry(bid).or_default();
                        let within_budget = record_memory_pause(hist, now);
                        let pauses_in_window = hist.len();
                        if within_budget {
                            // Clone to an owned Browser (mirrors on_before_close)
                            // for the label lookup + navigation; the original
                            // `browser` Option stays intact for the fall-through
                            // paths below. as_deref().cloned() borrows, not moves.
                            if let Some(mut owned) = browser.as_deref().cloned() {
                                let app_url = self.recovery_target_url(&mut owned, &base_url);
                                tracing::warn!(
                                    target: "crash",
                                    kind = "renderer_memory_paused",
                                    browser_id = bid,
                                    commit_free_mb = commit_free,
                                    resume_floor_mb = RESUME_FLOOR_MB,
                                    pauses_in_window,
                                    "renderer OOM under low system commit — paused (NOT counted against the crash budget)",
                                );
                                let html = memory_paused_page(reason, error_code, commit_free, &app_url);
                                let b64 = cef::base64_encode(Some(html.as_bytes()));
                                let b64_str = CefString::from(&b64).to_string();
                                let data_uri = format!("data:text/html;base64,{}", b64_str);
                                let uri = CefString::from(data_uri.as_str());
                                if let Some(frame) = owned.main_frame() {
                                    frame.load_url(Some(&uri));
                                }
                                return;
                            }
                            // browser was None (unusual on this path) — fall
                            // through to the normal crash-budget handling below.
                        } else {
                            tracing::error!(
                                target: "crash",
                                kind = "memory_pause_budget_exceeded",
                                browser_id = bid,
                                pauses_in_window,
                                window_secs = MEMORY_PAUSE_WINDOW.as_secs(),
                                "memory-pause budget exceeded (commit totally exhausted) — falling through to the give-up page",
                            );
                            // fall through to the crash-budget path below.
                        }
                    }
                }
            }
        }

        // Crash budget — if this browser has crashed more than
        // CRASH_BUDGET times in CRASH_BUDGET_WINDOW, abandon
        // auto-recovery and load a terminal "give up" page that does
        // NOT call frame.load_url again. This breaks the loop the
        // 2026-05-28 incident produced: a wedged renderer slot meant
        // every recovery-page load itself terminated, re-firing this
        // handler at ~108 events/sec for 22 minutes (139k crashes,
        // 884 MB log). See SPEC_SERVICE_SUPERVISION prime directive
        // ("bounded recovery; never an infinite restart loop") and
        // docs/retro/retro-portable-rm-running-install-2026-05-28.md.
        //
        // Pre-budget-check is cheap and runs on every crash; the work
        // it gates (resolve_frontend_base_url + format! + base64 +
        // load_url) is several orders of magnitude more expensive,
        // so even N crashes within budget incur no measurable
        // overhead from this block.
        let browser_id = browser.as_ref().map(|b| b.identifier());
        if let Some(bid) = browser_id {
            let now = Instant::now();
            let history = self.crash_history.entry(bid).or_default();
            // Prune entries outside the window before counting.
            while history.front().is_some_and(|t| now.duration_since(*t) > CRASH_BUDGET_WINDOW) {
                history.pop_front();
            }
            history.push_back(now);
            if history.len() > CRASH_BUDGET {
                let crashes_in_window = history.len();
                tracing::error!(
                    target: "crash",
                    kind = "crash_loop_aborted",
                    browser_id = bid,
                    crashes_in_window,
                    window_secs = CRASH_BUDGET_WINDOW.as_secs(),
                    "crash budget exceeded — abandoning auto-recovery for this browser",
                );
                let html = crash_loop_terminal_page(reason, error_code, crashes_in_window);
                let b64 = cef::base64_encode(Some(html.as_bytes()));
                let b64_str = CefString::from(&b64).to_string();
                let data_uri = format!("data:text/html;base64,{}", b64_str);
                let uri = CefString::from(data_uri.as_str());
                if let Some(b) = browser {
                    if let Some(frame) = b.main_frame() {
                        frame.load_url(Some(&uri));
                    }
                }
                return;
            }
        }

        // Resolve the real frontend URL so the Reload button can navigate
        // back to the live app instead of reloading the recovery page
        // itself. Matches the format used by
        // commands::window::resolve_frontend_base_url and its callers
        // (see window.rs:400, window.rs:430, drag.rs:294 — all use the
        // same ipc_port / ipc_token query params).
        //
        // If the resolver returns Err (frontend assets missing — the
        // 2026-05-28 incident pattern where an external `rm -rf` of a
        // running portable left current_exe()'s parent dir empty),
        // short-circuit to the "install broken" static page instead of
        // pointing the Reload button at a URL that would itself crash.
        // See docs/retro/retro-portable-rm-running-install-2026-05-28.md.
        // Reload must bring a recovered window back as ITSELF — preserving
        // windowLabel and (for tear-off / floating-pane windows) workspaceId /
        // floatingPaneId. recovery_target_url reuses the window's own pre-crash
        // URL when possible. as_deref().cloned() borrows browser (a clone),
        // leaving the original intact for the load below. (codex P2 #1229.)
        let mut recovery_owned = browser.as_deref().cloned();
        let app_url = match crate::commands::window::resolve_frontend_base_url(self.ipc_port) {
            Ok(base_url) => match recovery_owned.as_mut() {
                Some(owned) => self.recovery_target_url(owned, &base_url),
                None => recovery_navigation_url(
                    &base_url,
                    self.ipc_port,
                    &self.state.ipc_token,
                    None,
                ),
            },
            Err(e) => {
                tracing::error!(
                    target: "crash",
                    error = %e,
                    "renderer crash recovery: frontend assets unavailable — loading static install-broken page instead of an unresolvable network URL",
                );
                let url = crate::commands::window::assets_missing_data_url(&e);
                if let Some(b) = browser {
                    if let Some(frame) = b.main_frame() {
                        let uri = CefString::from(url.as_str());
                        frame.load_url(Some(&uri));
                    }
                }
                return;
            }
        };

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

    /// CEF asks the embedder for HTTP Basic / Digest credentials on a
    /// 401/407. Browser-pane requests get surfaced to the renderer via
    /// `browser-pane-auth-required` so the user can type credentials;
    /// non-browser-pane requests (the main host window's frontend
    /// load) are declined — those shouldn't hit auth-challenged
    /// resources, and silently failing matches the prior behavior.
    ///
    /// Returns 1 (async — we'll resolve the callback via
    /// `browser_pane_auth_submit` / `browser_pane_auth_cancel`) or 0
    /// (sync no — CEF aborts the request).
    ///
    /// Phase α of SPEC_BROWSER_PANE_HTTP_BASIC_AUTH_2026_05_18.md.
    fn on_auth_credentials(
        &mut self,
        browser: Option<&mut Browser>,
        origin_url: Option<&CefString>,
        is_proxy: ::std::os::raw::c_int,
        host: Option<&CefString>,
        port: ::std::os::raw::c_int,
        realm: Option<&CefString>,
        _scheme: Option<&CefString>,
        callback: Option<&mut AuthCallback>,
    ) -> ::std::os::raw::c_int {
        // [DIAG] Top-of-function entry log — fires unconditionally before
        // every early-return so we can confirm whether CEF is invoking
        // the callback at all for a given URL. The reagent-merged
        // browser-pane auth flow (#906) appeared to fail silently for
        // some sites in dev mode (e.g. https://pulse.asaf.cc returns
        // ERR_INVALID_AUTH_CREDENTIALS without `[browser-pane-auth]`
        // ever logging). This entry log narrows the diagnosis:
        //   - Visible → CEF is calling the callback; early return below
        //     OR downstream path is the problem.
        //   - Not visible → CEF is suppressing the call entirely
        //     (caching, security policy, missing vtable wire-up).
        // Logs `origin/host/port/realm/is_proxy/has_browser/has_callback`
        // so all the discriminators are captured even on the silent-
        // decline branches that follow.
        let origin_dbg = origin_url.map(CefString::to_string).unwrap_or_default();
        let host_dbg = host.map(CefString::to_string).unwrap_or_default();
        let realm_dbg = realm.map(CefString::to_string).unwrap_or_default();
        tracing::info!(
            "[browser-pane-auth][ENTRY] origin={:?} host={:?}:{} realm={:?} \
             is_proxy={} has_browser={} has_callback={}",
            origin_dbg,
            host_dbg,
            port,
            realm_dbg,
            is_proxy != 0,
            browser.is_some(),
            callback.is_some(),
        );

        // Resolve the pane block_id from the browser ref. If this isn't
        // a browser-pane browser (i.e. it's the host frontend's browser),
        // we have no UI to prompt — decline and let CEF fail the
        // request. The host frontend should never hit auth challenges.
        let Some(b) = browser.as_deref() else {
            tracing::warn!("[browser-pane-auth] no browser ref — declining");
            return 0;
        };
        let Some(block_id) =
            crate::browser_pane::callbacks::resolve_pane_block_id(&self.state, b)
        else {
            tracing::info!(
                "[browser-pane-auth] not a browser pane (host frontend?) — declining"
            );
            return 0;
        };
        let Some(cb) = callback else {
            // [DIAG] Previously silent. If we reach here CEF gave us a
            // browser + a resolvable pane block_id but no callback —
            // an unusual combination worth logging so the diagnosis
            // path doesn't have a blind spot.
            tracing::warn!(
                "[browser-pane-auth] callback is None (block={}) — declining",
                block_id,
            );
            return 0;
        };

        let request_id = uuid::Uuid::new_v4().to_string();
        let origin = origin_url.map(CefString::to_string).unwrap_or_default();
        let host_str = host.map(CefString::to_string).unwrap_or_default();
        let realm_str = realm.map(CefString::to_string).unwrap_or_default();
        let is_proxy_bool = is_proxy != 0;

        let block_id_short: String = block_id.chars().take(7).collect();
        tracing::info!(
            "[browser-pane-auth][{}] auth-required origin={:?} host={:?}:{} realm={:?} is_proxy={} request_id={}",
            block_id_short, origin, host_str, port, realm_str, is_proxy_bool, request_id,
        );

        // Park the callback so the renderer's submit/cancel IPC can
        // resolve it. The callback IS reference-counted internally
        // (RefGuard) — `cb.clone()` bumps the refcount so the registry
        // can hold it after CEF's invocation returns. Pass block_id so
        // `cancel_for_block` can clean up when the pane closes.
        crate::browser_pane::auth::register(
            request_id.clone(),
            block_id.clone(),
            cb.clone(),
        );

        // Broadcast to every TOP-LEVEL window — `emit_event_from_state`
        // only dispatches to the "main" browser, so panes hosted in a
        // tear-off / secondary window would never see the event and
        // the CEF callback would wait until TTL/cancel. Filtering to
        // top-level is critical: the payload carries origin/host/realm
        // plus block_id correlation, which must not be visible to
        // remote content loaded inside a sibling pane (whose main
        // frame is an arbitrary URL). The host renderer filters on
        // `payload.block_id` so only the window owning this pane
        // surfaces the prompt.
        crate::events::emit_event_to_top_level_windows(
            &self.state,
            "browser-pane-auth-required",
            &serde_json::json!({
                "block_id": block_id,
                "request_id": request_id,
                "origin": origin,
                "host": host_str,
                "port": port,
                "realm": realm_str,
                "is_proxy": is_proxy_bool,
            }),
        );

        1
    }
}

/// Terminal "give up" page rendered when a browser exceeds `CRASH_BUDGET`
/// renderer crashes within `CRASH_BUDGET_WINDOW`. Unlike the normal recovery
/// page, this one has NO reload button and NO `frame.load_url` target — it
/// only offers Quit. That's the whole point: navigating away from it cannot
/// re-enter `on_render_process_terminated` and restart the loop.
///
/// Auto-closes after `CRASH_LOOP_AUTO_CLOSE_SECS` so the dead window doesn't
/// linger in the host's window registry indefinitely. When `window.close()`
/// fires, the existing `on_before_close` path runs and `ReportWindowClosed`
/// → launcher → `Event::WindowInstanceReleased` chain decrements the
/// user-visible window count (fix for #1117 follow-up "decouple window count
/// from window lifecycle"). A visible countdown gives the user time to read
/// the message; clicking "Close this window" (or any keystroke / mouse
/// activity) cancels the auto-close so it's never surprising.
fn crash_loop_terminal_page(reason: &str, error_code: i32, crashes_in_window: usize) -> String {
    use crate::client::helpers::html_escape;
    const CRASH_LOOP_AUTO_CLOSE_SECS: u32 = 30;
    format!(
        r#"<!DOCTYPE html>
<html lang="en"><head><meta charset="utf-8"><title>AgentMux — Crash loop</title>
<style>
:root {{ color-scheme: dark; }}
body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
       background: #1e1e2e; color: #cdd6f4;
       display: flex; justify-content: center; align-items: center;
       min-height: 100vh; margin: 0; padding: 24px; box-sizing: border-box; }}
.box {{ text-align: center; max-width: 560px; padding: 36px;
       background: #181825; border: 1px solid #313244; border-radius: 10px;
       box-shadow: 0 8px 32px rgba(0,0,0,0.4); }}
.icon {{ font-size: 36px; line-height: 1; margin-bottom: 12px; }}
h1 {{ color: #f38ba8; font-size: 22px; margin: 0 0 6px 0; }}
.reason {{ color: #a6adc8; font-size: 14px; margin: 0 0 20px 0; font-style: italic; }}
p {{ color: #bac2de; line-height: 1.55; margin: 0 0 12px 0; font-size: 14px; }}
.countdown {{ color: #a6adc8; font-size: 13px; margin-top: 16px; }}
.countdown span {{ color: #f9e2af; font-weight: 600; }}
button {{ padding: 10px 22px; border: 1px solid #45475a; border-radius: 6px;
         background: #313244; color: #cdd6f4; cursor: pointer;
         font-size: 13px; font-family: inherit; margin-top: 16px; }}
button:hover {{ background: #45475a; border-color: #585b70; }}
.footer {{ color: #6c7086; font-size: 11px; margin-top: 18px;
          font-family: ui-monospace, monospace; }}
</style></head>
<body><div class="box" role="alertdialog">
<div class="icon">🛑</div>
<h1>Window stopped recovering</h1>
<p class="reason">Reason: {reason_safe}</p>
<p>This window crashed {crashes_in_window} times within {window_secs} seconds.
Auto-recovery is disabled to prevent a crash loop.</p>
<p>Your other AgentMux windows and your saved sessions are not affected —
they remain available. Close this window and open a fresh one to continue.</p>
<button onclick="window.close()">Close this window</button>
<p class="countdown">Auto-closing in <span id="countdown">{auto_close_secs}</span> s
(any keypress or click cancels)</p>
<div class="footer">error_code={error_code}</div>
</div>
<script>
// Auto-close so the dead window doesn't linger in the host window
// registry — when window.close() fires, on_before_close runs the
// normal ReportWindowClosed → WindowInstanceReleased chain and the
// UI window count snaps to the live count. Any user interaction
// cancels (so the message stays readable as long as the user wants).
let secs = {auto_close_secs};
const el = document.getElementById('countdown');
let cancelled = false;
const cancel = () => {{ cancelled = true; el.parentElement.style.display = 'none'; }};
document.addEventListener('keydown', cancel, {{ once: true }});
document.addEventListener('mousedown', cancel, {{ once: true }});
const tick = () => {{
    if (cancelled) return;
    secs -= 1;
    if (secs <= 0) {{ window.close(); return; }}
    el.textContent = String(secs);
    setTimeout(tick, 1000);
}};
setTimeout(tick, 1000);
</script>
</body></html>"#,
        reason_safe = html_escape(reason),
        window_secs = CRASH_BUDGET_WINDOW.as_secs(),
        crashes_in_window = crashes_in_window,
        error_code = error_code,
        auto_close_secs = CRASH_LOOP_AUTO_CLOSE_SECS,
    )
}

/// True when `url` is on the same origin as `base_url` (i.e. a live app URL we
/// can safely reuse for recovery). The boundary check after the prefix guards
/// against a port that is a numeric prefix of another (e.g. `:5173` matching
/// `:51730`): the char following the origin must be a path/query/fragment
/// separator or end-of-string. `base_url` is an origin with no path
/// (`http://127.0.0.1:<port>` or `http://localhost:<port>`).
fn url_on_origin(url: &str, base_url: &str) -> bool {
    url.strip_prefix(base_url).is_some_and(|rest| {
        rest.is_empty()
            || rest.starts_with('/')
            || rest.starts_with('?')
            || rest.starts_with('#')
    })
}

/// Build the navigation URL a crash-recovery / low-memory page sends the user
/// back to. Carries `ipc_port` + `ipc_token` and — critically — the window's
/// `windowLabel` when known, so a recovered secondary window doesn't
/// reinitialize as `main` (creation.rs adds the same `windowLabel` param on
/// first load; the frontend defaults a missing label to `main`). codex P2 #1229.
fn recovery_navigation_url(
    base_url: &str,
    ipc_port: u16,
    ipc_token: &str,
    window_label: Option<&str>,
) -> String {
    let sep = if base_url.contains('?') { "&" } else { "?" };
    match window_label {
        Some(lbl) => format!(
            "{base_url}{sep}ipc_port={ipc_port}&ipc_token={ipc_token}&windowLabel={lbl}"
        ),
        None => format!("{base_url}{sep}ipc_port={ipc_port}&ipc_token={ipc_token}"),
    }
}

/// Record a memory-pause for `now` into `hist`, pruning entries older than
/// `MEMORY_PAUSE_WINDOW` first, and return whether we are still within
/// `MEMORY_PAUSE_BUDGET`. Extracted from `on_render_process_terminated` so the
/// bounded-recovery logic — the part that must converge on the give-up page
/// under *total* commit exhaustion rather than loop forever — is unit-testable
/// without a live CEF browser. (SPEC_GATED_RENDERER_RECOVERY §6.B.)
fn record_memory_pause(hist: &mut VecDeque<Instant>, now: Instant) -> bool {
    while hist.front().is_some_and(|t| now.duration_since(*t) > MEMORY_PAUSE_WINDOW) {
        hist.pop_front();
    }
    hist.push_back(now);
    hist.len() <= MEMORY_PAUSE_BUDGET
}

/// Low-memory "paused" page — shown when a renderer is OOM-terminated while the
/// system commit limit is exhausted (SPEC_GATED_RENDERER_RECOVERY §6.B). Unlike
/// the give-up page this state is RECOVERABLE: all durable state lives in the
/// sidecar, so "Resume" navigates to the live app URL and re-projects
/// everything — losing nothing. The Resume is manual and memory-guided: an
/// automatic retry before commit recovers would just re-OOM, so we tell the
/// user to free memory first (host-driven, memory-gated auto-resume is Phase
/// 1b). `app_url` is the live frontend URL Resume navigates to (spawns a fresh
/// renderer); it already carries the ipc_token, same as the recovery page.
fn memory_paused_page(reason: &str, error_code: i32, commit_free_mb: u64, app_url: &str) -> String {
    use crate::client::helpers::{html_escape, js_string_literal};
    format!(
        r#"<!DOCTYPE html>
<html lang="en"><head><meta charset="utf-8"><title>AgentMux — Low memory</title>
<style>
:root {{ color-scheme: dark; }}
body {{ font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
       background: #1e1e2e; color: #cdd6f4;
       display: flex; justify-content: center; align-items: center;
       min-height: 100vh; margin: 0; padding: 24px; box-sizing: border-box; }}
.box {{ text-align: center; max-width: 560px; padding: 36px;
       background: #181825; border: 1px solid #313244; border-radius: 10px;
       box-shadow: 0 8px 32px rgba(0,0,0,0.4); }}
.icon {{ font-size: 36px; line-height: 1; margin-bottom: 12px; }}
h1 {{ color: #f9e2af; font-size: 22px; margin: 0 0 6px 0; }}
.reason {{ color: #a6adc8; font-size: 14px; margin: 0 0 20px 0; font-style: italic; }}
p {{ color: #bac2de; line-height: 1.55; margin: 0 0 12px 0; font-size: 14px; }}
strong {{ color: #f9e2af; }}
.actions {{ display: flex; gap: 10px; justify-content: center; margin-top: 24px; flex-wrap: wrap; }}
button {{ padding: 10px 22px; border: 1px solid #45475a; border-radius: 6px;
         background: #313244; color: #cdd6f4; cursor: pointer;
         font-size: 13px; font-family: inherit; }}
button:hover {{ background: #45475a; border-color: #585b70; }}
button.primary {{ background: #89b4fa; color: #1e1e2e; border-color: #89b4fa; font-weight: 600; }}
button.primary:hover {{ background: #74a0f8; border-color: #74a0f8; }}
.footer {{ color: #6c7086; font-size: 11px; margin-top: 18px; font-family: ui-monospace, monospace; }}
</style></head>
<body><div class="box" role="alertdialog">
<div class="icon">⏳</div>
<h1>Paused — system memory low</h1>
<p class="reason">Reason: {reason_safe}</p>
<p>This window paused because the system ran out of memory
(only {commit_free_mb} MB of commit was free). <strong>Your work is safe</strong> —
everything is saved in the background and will be restored exactly when this
window resumes.</p>
<p><strong>Free some memory first</strong> — close other AgentMux windows or
other apps — then click Resume. Resuming before memory recovers will just
pause again.</p>
<div class="actions">
<button id="amx-resume" class="primary">Resume</button>
<button id="amx-quit">Quit this window</button>
</div>
<div class="footer">error_code={error_code} · commit_free={commit_free_mb}MB</div>
</div>
<script>
(function(){{
  var r = document.getElementById('amx-resume');
  if (r) r.addEventListener('click', function () {{ location.href = {app_url_js}; }});
  var q = document.getElementById('amx-quit');
  if (q) q.addEventListener('click', function () {{ window.close(); }});
}})();
</script>
</body></html>"#,
        reason_safe = html_escape(reason),
        commit_free_mb = commit_free_mb,
        error_code = error_code,
        app_url_js = js_string_literal(app_url),
    )
}



#[cfg(test)]
mod gated_recovery_tests {
    use super::{
        memory_paused_page, record_memory_pause, url_on_origin, MEMORY_PAUSE_BUDGET,
        MEMORY_PAUSE_WINDOW, RESUME_FLOOR_MB,
    };
    use std::collections::VecDeque;
    use std::time::{Duration, Instant};

    #[test]
    fn memory_paused_page_resume_button_has_working_handler() {
        // Regression for the inert-Resume bug: js_string_literal() returns a
        // DOUBLE-quoted JS string and is a <script>-context literal; embedding it
        // in a double-quoted onclick="..." attribute terminated the attribute
        // early, so Resume did nothing and the window wedged. The handler must be
        // wired in a <script> block instead.
        let url = "http://127.0.0.1:63627/?ipc_port=63627&ipc_token=abc";
        let html = memory_paused_page("out of memory", -1, 100, url);

        // The broken antipattern must be gone.
        assert!(
            !html.contains("onclick=\"location.href"),
            "Resume must not use an inline onclick with a double-quoted URL"
        );
        // The handler is attached by id via addEventListener.
        assert!(html.contains("addEventListener"), "handler should use addEventListener");
        assert!(html.contains("id=\"amx-resume\""), "Resume button needs its id");
        // The app URL must appear as a valid JS string literal in script context
        // (js_string_literal escapes & -> \\u0026 and wraps in double quotes).
        assert!(
            html.contains(
                "location.href = \"http://127.0.0.1:63627/?ipc_port=63627\\u0026ipc_token=abc\""
            ),
            "Resume must navigate to the app URL as a valid JS string literal"
        );
    }

    #[test]
    fn url_on_origin_matches_only_real_same_origin_urls() {
        let base = "http://localhost:5173";
        // Exact origin, origin + path / query / fragment — all reuse.
        assert!(url_on_origin("http://localhost:5173", base));
        assert!(url_on_origin("http://localhost:5173/", base));
        assert!(url_on_origin("http://localhost:5173/?windowLabel=w1&workspaceId=ws", base));
        assert!(url_on_origin("http://localhost:5173?ipc_port=1&ipc_token=t", base));
        assert!(url_on_origin("http://localhost:5173#/route", base));
        // Port that merely *extends* the base port must NOT match.
        assert!(!url_on_origin("http://localhost:51730/?x=1", base));
        // Different origin, and a non-http (data:) recovery page, must not match.
        assert!(!url_on_origin("http://127.0.0.1:5173/?x=1", base));
        assert!(!url_on_origin("data:text/html;base64,abc", base));
        // Prod origin behaves the same.
        let prod = "http://127.0.0.1:8080";
        assert!(url_on_origin("http://127.0.0.1:8080/?ipc_port=8080", prod));
        assert!(!url_on_origin("http://127.0.0.1:80801/", prod));
    }

    #[test]
    fn resume_floor_is_above_a_fresh_renderer_commit() {
        // A fresh renderer commits ~100-200 MB; the floor must leave margin so
        // a resume doesn't instantly re-OOM. Guards against an accidental
        // shrink that would make the gate useless.
        assert!(RESUME_FLOOR_MB >= 256, "RESUME_FLOOR_MB too low to be safe");
    }

    #[test]
    fn within_budget_until_exceeded_then_converges() {
        let mut hist: VecDeque<Instant> = VecDeque::new();
        let now = Instant::now();
        // The first MEMORY_PAUSE_BUDGET pauses stay within budget (pause path).
        for i in 0..MEMORY_PAUSE_BUDGET {
            assert!(
                record_memory_pause(&mut hist, now),
                "pause {} should be within budget",
                i + 1
            );
        }
        // The next one exceeds → falls through to the give-up path. This is the
        // total-exhaustion convergence guarantee (no infinite memory-pause loop).
        assert!(
            !record_memory_pause(&mut hist, now),
            "exceeding the budget must return false (fall through to give-up)"
        );
    }

    #[test]
    fn old_pauses_outside_the_window_are_pruned() {
        let mut hist: VecDeque<Instant> = VecDeque::new();
        let start = Instant::now();
        // Fill the budget at t=start.
        for _ in 0..MEMORY_PAUSE_BUDGET {
            record_memory_pause(&mut hist, start);
        }
        // Later than the window: all prior entries prune, so we're within budget
        // again — a window that recovered then hit pressure again is NOT treated
        // as a wedged loop.
        let later = start + MEMORY_PAUSE_WINDOW + Duration::from_secs(1);
        assert!(
            record_memory_pause(&mut hist, later),
            "pauses older than the window must be pruned, resetting the budget"
        );
        assert_eq!(hist.len(), 1, "only the recent pause should remain");
    }
}
