// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! LoadHandler methods for `AgentMuxHandler` — loading-state, load-end IPC
//! injection + splash signals, and the load-error fallback page. Extracted
//! verbatim from client/mod.rs.

use cef::*;

use super::AgentMuxHandler;

#[cfg(any(target_os = "linux", target_os = "windows"))]
use std::sync::Arc;
#[cfg(any(target_os = "linux", target_os = "windows"))]
use crate::state::AppState;

/// Startup white-flash fix
/// (docs/specs/REPORT_NEW_WINDOW_STARTUP_COLOR_FLASH_2026_07_14.md,
/// docs/reports/REPORT_SPLASH_TO_FIRST_PAINT_BLANK_WINDOW_GAP_2026_09_03.md).
///
/// Bound on how long the real window + native splash can stay hidden/up
/// waiting for the frontend's first-paint signal before we show anyway.
///
/// Empirically (2026-07-14, verification run on the dev machine, a
/// resource-shared sandbox running several concurrent CEF instances) the real
/// double-rAF signal for the main window landed ~2.08s after `on_load_end`,
/// and pool-window prewarms (unaffected by this gate, so this is a baseline
/// reading of the environment's compositor-first-frame latency, not an
/// artifact of the gate itself) landed 1.36-1.84s out. An earlier 1500ms cap
/// fired *before* that real signal, silently degrading this fix to "always
/// show after a fixed 1.5s delay" — strictly better than the original
/// immediate-show bug, but not the intended "wait for real paint" behavior.
/// Set well above the observed worst case so the real signal wins in
/// practice; this only matters as a backstop against a genuinely stalled
/// renderer (crashed JS, rAF never firing at all).
///
/// Reused verbatim for Windows (ported 2026-09-03) rather than picking a
/// separate constant: this is a backstop, not a target, and the whole point
/// is that the real signal should win in practice on any machine capable of
/// running the app at all. A tighter Windows-specific value would need its
/// own dedicated measurement pass — see
/// `docs/reports/REPORT_SPLASH_TO_FIRST_PAINT_BLANK_WINDOW_GAP_2026_09_03.md`
/// §5.1 for why this report does not claim to have done that. If this timeout
/// is ever observed firing on real Windows hardware (not a crashed-JS case),
/// that is the signal a dedicated pass is now overdue, not a reason to
/// silently raise the number.
#[cfg(any(target_os = "linux", target_os = "windows"))]
const PAINT_GATE_SAFETY_TIMEOUT_MS: i64 = 4000;

/// Monotonic arm counter for the paint gate. Each `on_load_end` call that
/// (re-)arms a label's gate gets a fresh epoch; its safety-net timeout task
/// captures that epoch and only acts if it's still current when the timeout
/// fires (see `reveal_gated_window`). Mirrors the identical stale-timeout
/// guard in `browser_pane::auth` (`NEXT_EPOCH`/`Entry::epoch`).
#[cfg(any(target_os = "linux", target_os = "windows"))]
static PAINT_GATE_NEXT_EPOCH: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Reveal (show + focus the real window, dismiss the native splash) once —
/// whichever of the real `report_first_paint` IPC signal or the safety-net
/// timeout fires first wins; the other becomes a no-op via the
/// `linux_paint_gate_pending` membership check.
///
/// `expected_epoch`: `None` for the real-signal path (always reveal whatever
/// is currently armed for `label` — a stale signal from a torn-down document
/// can't fire; CEF cancels pending rAF/timers on navigation). `Some(epoch)`
/// for the safety-timeout path — reveals only if `label`'s current arm is
/// still the one this specific timeout was scheduled for; if `on_load_end`
/// re-armed the gate since (reload/retry mid-startup), this stale timeout is
/// a no-op instead of revealing the window ahead of the *current*
/// navigation's real paint (docs/specs/REPORT_NEW_WINDOW_STARTUP_COLOR_FLASH_2026_07_14.md,
/// reagent PR #2151 review).
///
/// Must run on the CEF UI thread.
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub(crate) fn reveal_gated_window(
    state: &Arc<AppState>,
    label: &str,
    reason: &'static str,
    expected_epoch: Option<u64>,
) {
    let armed_at = {
        let mut pending = state.linux_paint_gate_pending.lock();
        match pending.get(label) {
            Some(&(armed_at, epoch)) => {
                if expected_epoch.is_some_and(|expected| expected != epoch) {
                    // A newer on_load_end call re-armed this label after this
                    // timeout was scheduled — the newer arm's own timeout (or
                    // the real signal) owns the reveal now.
                    return;
                }
                pending.remove(label);
                armed_at
            }
            // Already revealed by the other path, or never armed (e.g. a pool
            // window — those don't go through this gate at all).
            None => return,
        }
    };
    let elapsed_ms = armed_at.elapsed().as_millis() as u64;
    tracing::info!(
        target: "startup-paint",
        label = %label,
        reason,
        elapsed_ms,
        "[startup-paint] revealing gated window"
    );
    // "paint" stage telemetry (docs/reports/REPORT_SPLASH_TO_FIRST_PAINT_BLANK_WINDOW_GAP_2026_09_03.md
    // §5.3): reports the on_load_end -> real-paint gap this whole gate exists
    // to close, as its own splash row. Paired with the `stage_begin` call at
    // the point this label was armed (`reveal_top_level_window` below).
    // Placed here rather than gated on the fallible browser/window lookups
    // further down, matching the ready-file/SetEvent dismiss signals below:
    // the paint gate resolving is the event worth reporting, independent of
    // whether the low-level show() that follows happens to succeed.
    crate::launcher_ipc::report_startup_stage_end(
        "paint",
        elapsed_ms,
        "ok",
        Some(reason.to_string()),
    );
    if let Ok(path) = std::env::var("AGENTMUX_SPLASH_READY_FILE") {
        if !path.is_empty() {
            let _ = std::fs::write(&path, b"ready");
        }
    }
    // Windows analogue of the macOS/Linux ready-file above: signal the named
    // Win32 event the launcher's splash thread is waiting on. Ported from
    // `on_load_end`'s old unconditional call (2026-09-03) — moving it here
    // means it now fires only once the window has something real to show,
    // instead of at bare document-load-complete. This also fixes a latent
    // bug the move inherits a fix for: the old call ran for EVERY on_load_end
    // including hidden pool-window prewarms (nothing gated it on
    // `is_pool_window`, which is checked later in `on_load_end`), so a
    // background prewarm racing ahead of the real main window during a cold
    // start could dismiss the splash before the real window had loaded at
    // all. Pool windows never reach this function (they skip the whole
    // reveal call — see `on_load_end`), so that race is now closed the same
    // way Linux's ready-file was already immune to it.
    //
    // Also called from `reveal_top_level_window`'s `label: None` fallback
    // branch — that path never arms the paint gate (no label to key it on),
    // so it must fire this signal itself rather than relying on this
    // function, which the fallback never reaches. Reagent PR #2968 review:
    // the signal used to fire unconditionally in `on_load_end` regardless of
    // label resolution; moving it here alone silently dropped that fallback
    // case, leaving the launcher's splash wait (no overall timeout,
    // `agentmux-launcher/src/splash.rs::run_splash`) to hang forever.
    #[cfg(target_os = "windows")]
    signal_windows_splash_dismiss();
    let Some(mut browser) = state.get_browser(label) else { return };
    if let Some(bv) = browser_view_get_for_browser(Some(&mut browser)) {
        if let Some(window) = bv.window() {
            if window.is_visible() == 0 {
                window.show();
                if let Some(host) = browser.host() {
                    host.set_focus(1);
                }
            }
        }
    }
}

/// Signal the named Win32 event the launcher's splash thread is waiting on
/// (`AGENTMUX_SPLASH_EVENT`). Shared by both `reveal_gated_window` (the
/// normal, label-resolved path) and `reveal_top_level_window`'s `label:
/// None` fallback (which never arms the paint gate, so it never reaches
/// `reveal_gated_window` at all) — every code path that can show the
/// top-level window for the first time must dismiss the splash, or
/// `run_splash`'s untimed wait hangs forever.
#[cfg(target_os = "windows")]
fn signal_windows_splash_dismiss() {
    use windows_sys::Win32::Foundation::CloseHandle;
    use windows_sys::Win32::System::Threading::{OpenEventW, SetEvent, EVENT_MODIFY_STATE};
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

#[cfg(any(target_os = "linux", target_os = "windows"))]
wrap_task! {
    struct PaintGateRevealTask {
        state: Arc<AppState>,
        label: String,
        reason: &'static str,
        expected_epoch: Option<u64>,
    }
    impl Task { fn execute(&self) {
        reveal_gated_window(&self.state, &self.label, self.reason, self.expected_epoch);
    }}
}

/// Handle the real `report_first_paint` signal on the UI thread. Two possible
/// orderings against `on_load_end`, both handled:
///
/// - `on_load_end` already armed the gate (the common case) — reveal now.
/// - `on_load_end` hasn't run yet for this label — CEF's main-frame
///   load-complete isn't guaranteed to fire after first paint (render-blocking
///   stylesheets can resolve, and a frame can be presented, before other
///   load-blocking resources finish). Record the label in
///   `linux_first_paint_seen` so `on_load_end` reveals immediately when it
///   does arm, instead of silently dropping this signal and falling through
///   to the slower safety timeout. Reagent PR #2151 second-round review.
///
/// Must run on the CEF UI thread.
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub(crate) fn handle_first_paint_signal(state: &Arc<AppState>, label: &str) {
    let already_armed = state.linux_paint_gate_pending.lock().contains_key(label);
    if already_armed {
        reveal_gated_window(state, label, "signal", None);
    } else {
        state.linux_first_paint_seen.lock().insert(label.to_string());
    }
}

#[cfg(any(target_os = "linux", target_os = "windows"))]
wrap_task! {
    struct FirstPaintSignalTask { state: Arc<AppState>, label: String }
    impl Task { fn execute(&self) {
        handle_first_paint_signal(&self.state, &self.label);
    }}
}

/// Called off the UI thread (IPC handler for `report_first_paint`) once the
/// frontend's double-`requestAnimationFrame` confirms the compositor actually
/// presented a frame. Posts to the UI thread to reveal the window if
/// `on_load_end` already deferred it, or to record the signal for `on_load_end`
/// to consume if it hasn't run yet (see `handle_first_paint_signal`).
#[cfg(any(target_os = "linux", target_os = "windows"))]
pub(crate) fn on_frontend_first_paint(state: Arc<AppState>, label: String) {
    let mut task = FirstPaintSignalTask::new(state, label);
    post_task(ThreadId::UI, Some(&mut task));
}

/// Bound on how many times `on_load_end`'s top-level `window.show()` retries
/// when `browser_view_get_for_browser()`/`BrowserView::window()` return
/// `None` at load-end, instead of the previous silent no-op that left the
/// window permanently hidden with zero diagnostic trace. This is the same
/// class of CEF Views timing quirk already diagnosed and worked around on
/// the pool-window path (`commands/window_pool.rs`'s `POOL_HWND_CACHE`,
/// `SPEC_POOL_WINDOW_HWND_NULL_2026_05_06.md`) but was never ported to the
/// main/top-level window path — see
/// `docs/retro/RETRO_MAIN_WINDOW_SHOW_SILENT_NOOP_2026_08_13.md`.
const SHOW_WINDOW_MAX_RETRIES: u32 = 10;
/// Delay between retries — short enough that a handful still resolves within
/// a user-imperceptible instant if the window becomes resolvable a frame or
/// two later, long enough not to busy-loop the UI thread.
const SHOW_WINDOW_RETRY_DELAY_MS: i64 = 50;

/// Show (or, on Linux, arm the startup-paint gate for) a top-level window's
/// Views `Window`, once confirmed not yet visible. Shared by `on_load_end`'s
/// primary resolution path and `ShowWindowRetryTask`'s retry path so BOTH
/// respect the same Linux paint gating
/// (`linux_paint_gate_pending`/`reveal_gated_window`,
/// `REPORT_NEW_WINDOW_STARTUP_COLOR_FLASH_2026_07_14.md`) — reagentx P1 on this PR:
/// the retry path originally called `window.show()` unconditionally on every
/// platform, bypassing the gate entirely. Whenever a *retry* (not the first
/// `on_load_end` pass) is what actually resolves the window, that would
/// reintroduce the Linux white-flash bug the gate exists to prevent.
///
/// `label` is `None` only when the window label couldn't be resolved at all
/// — matches the pre-existing "can't gate safely on an unknown label"
/// fallback (immediate show, every platform).
///
/// `state`/`label` are only read inside the `#[cfg(any(target_os = "linux",
/// target_os = "windows"))]` branch below — unused on macOS, hence the
/// `allow`. See `docs/reports/REPORT_SPLASH_TO_FIRST_PAINT_BLANK_WINDOW_GAP_2026_09_03.md`
/// §6 for why macOS is deliberately not included here: it needs its own
/// live-verified pass, not a blind extension of the Linux/Windows cfg gate.
#[cfg_attr(target_os = "macos", allow(unused_variables))]
fn reveal_top_level_window(
    state: &std::sync::Arc<crate::state::AppState>,
    label: Option<&str>,
    window: &cef::Window,
    browser: Option<&mut Browser>,
) {
    #[cfg(any(target_os = "linux", target_os = "windows"))]
    if let Some(label) = label {
        // "paint" stage begin — pairs with the stage_end call inside
        // reveal_gated_window, whichever path below resolves it. Fired once
        // per arm regardless of which sub-path (early-signal vs timeout)
        // ends up resolving, by sitting above both.
        crate::launcher_ipc::report_startup_stage_begin("paint", "Window paint");
        // The real paint signal can race ahead of this call (see
        // handle_first_paint_signal) — if it already arrived, reveal
        // immediately instead of arming a gate + safety timeout for a paint
        // that already happened.
        let already_painted = state.linux_first_paint_seen.lock().remove(label);
        // Fresh epoch per arm — see PAINT_GATE_NEXT_EPOCH and
        // reveal_gated_window's doc comment. Guards against a reload/retry
        // re-arming this same label before a timeout fires.
        let epoch = PAINT_GATE_NEXT_EPOCH.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        state
            .linux_paint_gate_pending
            .lock()
            .insert(label.to_string(), (std::time::Instant::now(), epoch));
        if already_painted {
            reveal_gated_window(state, label, "signal-early", None);
        } else {
            let mut timeout_task = PaintGateRevealTask::new(
                state.clone(),
                label.to_string(),
                "timeout",
                Some(epoch),
            );
            post_delayed_task(ThreadId::UI, Some(&mut timeout_task), PAINT_GATE_SAFETY_TIMEOUT_MS);
        }
        return;
    }
    // No resolvable label — the paint gate above never armed, so this is
    // also this window's only chance to dismiss the Windows splash (see
    // `signal_windows_splash_dismiss`'s doc comment).
    #[cfg(target_os = "windows")]
    signal_windows_splash_dismiss();
    window.show();
    if let Some(b) = browser {
        if let Some(host) = b.host() {
            host.set_focus(1);
        }
    }
}

/// Resolve a top-level (non-pool) window by label and reveal it (via
/// `reveal_top_level_window`) if its `BrowserView`/`Window` are resolvable
/// and it isn't already visible. Returns `true` once resolved (shown,
/// gate-armed, or already visible), `false` if the CEF Views objects aren't
/// available yet — caller should retry rather than give up silently.
fn try_show_top_level_window(state: &std::sync::Arc<crate::state::AppState>, label: &str) -> bool {
    let Some(mut browser) = state.get_browser(label) else { return false };
    let Some(bv) = browser_view_get_for_browser(Some(&mut browser)) else { return false };
    let Some(window) = bv.window() else { return false };
    if window.is_visible() == 0 {
        reveal_top_level_window(state, Some(label), &window, Some(&mut browser));
    }
    true
}

wrap_task! {
    struct ShowWindowRetryTask {
        state: std::sync::Arc<crate::state::AppState>,
        label: String,
        retries_left: u32,
    }
    impl Task { fn execute(&self) {
        if try_show_top_level_window(&self.state, &self.label) {
            tracing::info!(
                target: "wrr",
                label = %self.label,
                retries_left = self.retries_left,
                "[on_load_end] show() retry succeeded"
            );
            return;
        }
        if self.retries_left == 0 {
            tracing::error!(
                target: "wrr",
                label = %self.label,
                max_retries = SHOW_WINDOW_MAX_RETRIES,
                "[on_load_end] browser_view/window still not resolvable after all retries — window will stay hidden"
            );
            return;
        }
        let mut next = ShowWindowRetryTask::new(
            self.state.clone(),
            self.label.clone(),
            self.retries_left - 1,
        );
        post_delayed_task(ThreadId::UI, Some(&mut next), SHOW_WINDOW_RETRY_DELAY_MS);
    }}
}

impl AgentMuxHandler {
    /// CEF fires this whenever the browser's loading/history state changes
    /// (navigation started, navigation committed, back/forward enabled).
    /// `can_go_back` / `can_go_forward` come directly from the navigation
    /// controller — no need to query `browser.can_go_back()` (which races
    /// with history commit when called from `on_load_end`).
    ///
    /// For panes: emit `browser-pane-nav-state` so the frontend address
    /// bar + back/forward buttons reflect CEF's real history state, and
    /// `is_loading` so the pane can show a loading indicator while a real
    /// top-level navigation is in flight (SPEC_BROWSER_PANE_LOADING_BRAIN_INDICATOR_2026_07_11.md).
    pub(crate) fn on_loading_state_change(
        &mut self,
        browser: Option<&mut Browser>,
        is_loading: i32,
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
                is_loading != 0,
                can_go_back != 0,
                can_go_forward != 0,
            );
        }
    }

    /// CEF fires this once a navigation has COMMITTED and the specified
    /// frame begins loading its content — i.e. the browser now represents
    /// the new document, even though that document's own subresources
    /// (images/scripts/iframes) may still be in flight. Distinct from
    /// `is_loading` going false (`on_loading_state_change`, which waits for
    /// the ENTIRE load — all subresources too) and from `on_load_end` (whose
    /// own doc comment there notes it can fire before the navigation
    /// controller has finished committing the history entry — not a
    /// reliable "has this committed" signal either).
    ///
    /// For browser panes only: disarms the pane-load-watchdog the moment the
    /// TARGET document commits, same as the existing disarm-on-`!is_loading`
    /// path in `on_loading_state_change_browser_pane`. Without this, a page
    /// that commits and renders successfully but has one slow subresource
    /// could still hit the watchdog's deadline (is_loading stays true until
    /// every subresource finishes) and have `fire_pane_load_watchdog`
    /// replace the already-loaded, visible page with a synthetic
    /// `ERR_CONNECTION_TIMED_OUT` — flagged inline by Codex and by reagentx
    /// P1 on PR #2593 (second pass); the "address reagentx P1s" follow-up
    /// commit only fixed the redirect-timer and pane-close disarm gaps, not
    /// this one.
    pub(crate) fn on_load_start(
        &mut self,
        browser: Option<&mut Browser>,
        frame: Option<&mut Frame>,
        _transition_type: TransitionType,
    ) {
        if !self.is_browser_pane {
            return;
        }
        let Some(frame) = frame else { return };
        if frame.is_main() != 1 {
            return;
        }
        if let Some(b) = browser.as_deref() {
            crate::browser_pane::callbacks::on_load_start_browser_pane(&self.state, b);
        }
    }

    pub(crate) fn on_load_end(
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

        // Re-inject the host IPC creds on EVERY main-frame load of OUR OWN
        // frontend — crucially INCLUDING `is_browser_pane` floating-pane /
        // pool windows, which host our frontend (on our localhost origin)
        // but were starved of creds by the old `!is_browser_pane` gate.
        //
        // Why every load, not just the first: the frontend strips
        // `ipc_port`/`ipc_token` from the URL after reading them once
        // (cef-init.ts token-leak fix), so any reload (Vite HMR, WebGL
        // context-loss reload, or the bridge auto-recover itself) arrives
        // cred-less. Without re-injection, `setupCefApi` can't rebuild
        // `window.api` → the permanent "window.api still undefined after 5s"
        // blank + ~5s reload storm observed on floating-pool windows (#52).
        //
        // Gate on frame ORIGIN, not the pane flag: inject only when the
        // loading main frame is on our frontend origin. A real browser pane
        // rendering a remote site (example.com) is a different origin and
        // still never receives the bearer token — preserving exactly the
        // leak protection the `is_browser_pane` gate was meant to provide.
        let frame_url = browser
            .as_ref()
            .and_then(|b| b.main_frame().map(|f| CefString::from(&f.url()).to_string()))
            .unwrap_or_default();
        // Authoritative port (self.ipc_port is 0 for floating-pane/pool
        // handlers — see resolved_ipc_port). Injecting 0 would make the
        // frontend's invokeCommand reject the creds and time out on
        // get_backend_endpoints (#52).
        let ipc_port = self.resolved_ipc_port();
        // Non-pane windows (main + secondary app windows) always load our
        // frontend → inject unconditionally (unchanged behavior; `||`
        // short-circuits so the common path skips the origin resolve). Only
        // `is_browser_pane` windows need the origin gate to separate our
        // floating-pane/pool windows from real remote browser panes.
        let should_inject = !self.is_browser_pane
            || crate::commands::window::resolve_frontend_base_url(ipc_port)
                .map(|base| super::recovery_pages::url_on_origin(&frame_url, &base))
                .unwrap_or(false);
        if should_inject {
            let ipc_token = &self.state.ipc_token;
            let js = format!(
                "window.__AGENTMUX_IPC_PORT__ = {}; window.__AGENTMUX_IPC_TOKEN__ = '{}';",
                ipc_port, ipc_token
            );
            let code = CefString::from(js.as_str());
            let empty = CefString::from("");
            frame.execute_java_script(Some(&code), Some(&empty), 0);
            tracing::info!("Injected IPC port {} into page: {}", ipc_port, frame_url);
        }

        // Pane-specific on_load_end work (focus subclass re-install after
        // Chromium rebuilds Chrome_RenderWidgetHostHWND on navigation).
        // Runs AFTER cred injection so a floating-pane window gets BOTH the
        // bridge and its focus fix; a real remote browser pane falls through
        // here having received no creds (origin didn't match above).
        if self.is_browser_pane {
            if let Some(b) = browser.as_deref() {
                crate::browser_pane::callbacks::on_load_end_browser_pane(&self.state, b);
            }
            return;
        }

        // Windows: the old unconditional AGENTMUX_SPLASH_EVENT SetEvent that
        // used to live here was moved into `reveal_gated_window`
        // (2026-09-03, docs/reports/REPORT_SPLASH_TO_FIRST_PAINT_BLANK_WINDOW_GAP_2026_09_03.md).
        // Firing it here — at bare `on_load_end`, before anything has visually
        // painted — is exactly the too-early dismiss defect A of that report
        // diagnoses; it also had no `is_pool_window` guard, so a hidden
        // pool-window prewarm racing ahead of the real main window during a
        // cold start could dismiss the splash before the real window had even
        // loaded. See `reveal_top_level_window`/`reveal_gated_window` below,
        // which now gate this on the same real first-paint confirmation Linux
        // already used, reached only via the non-pool-window path.

        // macOS analogue of the Win32 splash signal: the launcher owns the native
        // splash (see agentmux-launcher/src/splash_mac.rs) and passes a ready-file
        // path via AGENTMUX_SPLASH_READY_FILE. Creating the file is the
        // cross-process "first frame painted" signal the launcher polls for before
        // tearing the splash down. Fire-and-forget; absent var => no launcher
        // splash (e.g. dev:standalone), so this is a no-op.
        //
        // Linux does NOT write it here. `on_load_end` fires per-browser on
        // main-frame load-complete, including hidden pool-window prewarms — on
        // Linux that could dismiss the splash before the real (visible) window
        // has painted anything (the white-flash bug this gate fixes). Linux
        // writes the ready-file from `reveal_gated_window` instead, gated on the
        // same first-paint confirmation that unblocks the real window's show()
        // below. See docs/specs/REPORT_NEW_WINDOW_STARTUP_COLOR_FLASH_2026_07_14.md.
        #[cfg(target_os = "macos")]
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
            let mut resolved = false;
            if let Some(bv) = browser_view_get_for_browser(browser_cloned.as_mut()) {
                if let Some(window) = bv.window() {
                    resolved = true;
                    if window.is_visible() == 0 {
                        // Linux: defer the actual show()/focus (and the splash
                        // dismiss above) until the frontend confirms a real
                        // compositor paint, instead of doing it here on mere
                        // load-complete — see reveal_gated_window and
                        // docs/specs/REPORT_NEW_WINDOW_STARTUP_COLOR_FLASH_2026_07_14.md.
                        // Falls back to the old immediate-show behavior if the
                        // window label can't be resolved (can't gate safely on
                        // an unknown label). Shared with ShowWindowRetryTask's
                        // retry path via reveal_top_level_window so both
                        // respect the same gating.
                        reveal_top_level_window(
                            &self.state,
                            browser_label.as_deref(),
                            &window,
                            browser_cloned.as_mut(),
                        );
                    }
                }
            }
            if !resolved {
                // browser_view_get_for_browser()/bv.window() returned None —
                // the CEF Views timing quirk documented on
                // ShowWindowRetryTask above. Previously this silently left
                // the window hidden forever; now retry on a short backoff.
                if let Some(label) = browser_label.clone() {
                    tracing::warn!(
                        target: "wrr",
                        label = %label,
                        "[on_load_end] browser_view/window not resolvable at load-end — scheduling show() retry"
                    );
                    let mut task = ShowWindowRetryTask::new(
                        self.state.clone(),
                        label,
                        SHOW_WINDOW_MAX_RETRIES,
                    );
                    post_delayed_task(ThreadId::UI, Some(&mut task), SHOW_WINDOW_RETRY_DELAY_MS);
                } else {
                    tracing::error!(
                        target: "wrr",
                        "[on_load_end] browser_view/window not resolvable AND label unknown — cannot retry show(), window will stay hidden"
                    );
                }
            }
        }
    }

    pub(crate) fn on_load_error(
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

        tracing::error!(
            "Load error: url={} error={} ({})",
            failed_url,
            error_text,
            error_code_i32
        );

        // Layer 1, SPEC_BROWSER_PANE_LOADING_INDICATOR_FLICKER_2026_08_17.md:
        // a real (non-ERR_ABORTED, already filtered above) main-frame error
        // ends the pane's main-frame-loading state same as a successful
        // on_load_end would — the spinner shouldn't stay up forever on a
        // failed navigation. A navigation that got superseded by a newer
        // one is reported as ERR_ABORTED (filtered above), so this can't
        // race a newer navigation that's already marked loading.
        if self.is_browser_pane {
            if let Some(b) = browser.as_deref() {
                if let Some(block_id) =
                    crate::browser_pane::callbacks::resolve_pane_block_id(&self.state, b)
                {
                    crate::browser_pane::callbacks::set_pane_main_frame_loading(
                        &self.state,
                        &block_id,
                        b,
                        &failed_url,
                        false,
                    );
                }
            }
        }

        show_load_error_page(frame, &failed_url, error_code_i32, &error_text, self.is_browser_pane);
    }
}

/// Build the load-error page HTML for a failed navigation. Always sets an
/// explicit `<title>` — see `error_catalog`'s module doc for why that
/// matters: without one, the window/tab title falls back to this page's own
/// `data:text/html;base64,...` URI. The human title/heading/detail come from
/// `error_catalog::describe`; CEF's own `error_text` (e.g.
/// `ERR_CONNECTION_TIMED_OUT`) and the numeric code are kept as a secondary
/// diagnostic line underneath, not the headline.
///
/// `is_browser_pane` controls two things: the heading copy (a pane failing to
/// load an arbitrary external site must never claim "Failed to load AgentMux
/// frontend" — that copy was previously hardcoded and wrong for panes) and
/// whether the page auto-retries (dev frontend only, see `on_load_error`'s
/// original comment, preserved below).
pub(crate) fn render_load_error_html(
    failed_url: &str,
    error_code_i32: i32,
    error_text: &str,
    is_browser_pane: bool,
) -> String {
    use crate::client::error_catalog::describe;
    use crate::client::helpers::{html_escape, js_string_literal};

    let copy = describe(error_code_i32);
    let failed_url_safe = html_escape(failed_url);
    let error_text_safe = html_escape(error_text);
    let title_safe = html_escape(copy.title);
    // The dev frontend (main window, !is_browser_pane) keeps its own
    // specific heading — restored per reagentx P2 on PR #2593 (this file's
    // own copy silently dropped it when the error page was unified with
    // error_catalog::describe). A browser pane still uses the generic
    // per-error-code heading: that's the actual fix this unification made
    // (see this function's own doc comment) — a pane loading an arbitrary
    // external site must never claim "Failed to load AgentMux frontend".
    let heading_safe = html_escape(if is_browser_pane {
        copy.heading
    } else {
        "Failed to load AgentMux frontend"
    });
    let detail_safe = html_escape(copy.detail);
    // `js_string_literal`, not hand-rolled JSON: a real URL can contain a
    // single quote (e.g. `?q=can't`), which would otherwise break the
    // interpolated JS in the Retry handler below.
    let failed_url_js = js_string_literal(failed_url);

    // Auto-retry ONLY for the dev frontend (the main window), which commonly
    // races the Vite dev server on launch. Browser panes load arbitrary user
    // URLs through this SAME handler — auto-retrying their failures (offline
    // site, DNS error, refused service) would be an unbounded reload loop, so
    // panes get a manual Retry only.
    let auto_retry = if is_browser_pane {
        String::new()
    } else {
        "setTimeout(__amxRetry, 1200);".to_string()
    };
    let dev_hint = if is_browser_pane {
        String::new()
    } else {
        "<p>Make sure the Vite dev server is running:<br><code>task dev</code> or <code>npx vite</code></p>"
            .to_string()
    };

    format!(
        r#"<!DOCTYPE html>
<html>
<head>
    <meta charset="utf-8">
    <title>{title_safe}</title>
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
        <h1>{heading_safe}</h1>
        <p>Could not load <code>{failed_url_safe}</code></p>
        <p>{detail_safe}</p>
        <p style="opacity:0.7;font-size:12px;">{error_text_safe} ({error_code_i32})</p>
        {dev_hint}
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
    )
}

/// Render `render_load_error_html` and navigate `frame` to it as a base64
/// `data:` URI — the same mechanism `on_load_error` has always used. Shared
/// with `browser_pane::callbacks`'s navigation watchdog, which calls this
/// with a synthetic `ERR_CONNECTION_TIMED_OUT` when a pane's top-level
/// navigation is still loading past its watchdog deadline — see that module
/// for why (Chromium's real connect-timeout ceiling is 4 minutes, shared
/// verbatim with Chrome's own net stack, which is too long to leave a pane
/// sitting blank).
pub(crate) fn show_load_error_page(
    frame: &mut Frame,
    failed_url: &str,
    error_code_i32: i32,
    error_text: &str,
    is_browser_pane: bool,
) {
    let html = render_load_error_html(failed_url, error_code_i32, error_text, is_browser_pane);
    let b64 = cef::base64_encode(Some(html.as_bytes()));
    let b64_str = CefString::from(&b64).to_string();
    let data_uri = format!("data:text/html;base64,{}", b64_str);
    let uri = CefString::from(data_uri.as_str());
    frame.load_url(Some(&uri));
}
