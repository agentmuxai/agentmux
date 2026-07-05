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
use std::time::{Duration, Instant};
use parking_lot::Mutex;

use crate::state::AppState;

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

mod handlers;
pub(crate) mod helpers;
mod lifecycle;
mod display;
mod navigation;
mod crash_recovery;
mod recovery_pages;
#[cfg(target_os = "windows")]
mod wndproc;
#[cfg(target_os = "windows")]
pub(crate) use wndproc::install_main_window_floater_cascade_hook;
// Round 6 (pool demote) — the imperative srv-cleanup path in
// `commands/window_pool.rs::demote_srv_cleanup` replicates
// `on_before_close`'s backend cleanup (which never fires for parked
// pool-window browsers).
pub(crate) use helpers::backend_close_window;
pub(crate) use lifecycle::{
    retry_backend_window_id_lookup, BACKEND_WINDOW_ID_RETRY_ATTEMPTS,
    BACKEND_WINDOW_ID_RETRY_DELAY,
};

pub use handlers::AgentMuxClient;

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
            .filter(|u| recovery_pages::url_on_origin(u, base_url));
        if let Some(u) = pre_crash {
            return u;
        }
        let label = self.window_label_for(owned);
        recovery_pages::recovery_navigation_url(base_url, self.ipc_port, &self.state.ipc_token, label.as_deref())
    }
}
