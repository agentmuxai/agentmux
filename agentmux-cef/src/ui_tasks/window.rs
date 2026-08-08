// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Top-level window tasks: deferred load_url, close, memory-pressure banner,
// minimize / maximize / focus / move / position / rect, window-at-cursor
// resolution, corrective move, window creation, DevTools, and main-focus
// reclaim. Split out of `ui_tasks.rs` unchanged.

use std::sync::Arc;
use cef::*;
use crate::state::AppState;
use super::get_window_on_ui;

// ── Deferred load_url (used by on_before_popup to avoid UI-thread deadlock)
//
// Calling `frame.load_url(url)` synchronously inside a CEF callback that
// holds the handler's inner lock (e.g. `on_before_popup`) deadlocks on
// link clicks: `load_url` kicks a new navigation which triggers
// `on_loading_state_change` on the same thread, which also wants the
// handler's lock. Posting the navigate as a separate UI task lets the
// original callback return, release its lock, and the load starts
// cleanly on the next message-loop turn. ─────────────────────────────────

wrap_task! {
    pub struct DeferredLoadUrlTask {
        browser: Browser,
        url: String,
    }

    impl Task {
        fn execute(&self) {
            let mut browser = self.browser.clone();
            if let Some(frame) = browser.main_frame() {
                frame.load_url(Some(&CefString::from(self.url.as_str())));
            }
        }
    }
}

// ── Deferred quit_message_loop (last-window close fix, 2026-07-16) ────────
//
// `quit_message_loop()` must run on the UI thread (see the Stage-2 comment
// in `client/lifecycle.rs::on_before_close`), but the last-window-close fix
// (docs/retro/retro-last-window-close-quit-race-2026-07-16.md) needs to call
// it AFTER a background thread finishes notifying srv via
// `backend_close_window` — otherwise the process can exit before that
// network call completes, leaking the closing window's srv-side row. This
// task lets the background thread post the quit back to the UI thread once
// its notify work is done, instead of calling it inline (from a background
// thread `quit_message_loop()` is undefined behavior — it's not
// thread-safe).

wrap_task! {
    pub struct QuitMessageLoopTask {}

    impl Task {
        fn execute(&self) {
            tracing::warn!(target: "wrr", "[wrr] QuitMessageLoopTask: calling quit_message_loop");
            quit_message_loop();
            tracing::warn!(target: "wrr", "[wrr] QuitMessageLoopTask: quit_message_loop returned");
        }
    }
}

// ── Close ────────────────────────────────────────────────────────────────

wrap_task! {
    pub struct CloseWindowTask {
        state: Arc<AppState>,
        label: String,
    }

    impl Task {
        fn execute(&self) {
            use cef::ImplWindow;
            // CEF Views: close the WINDOW (CefWindow::close), which routes through
            // WindowDelegate::can_close (app.rs) → try_close_browser → on_before_close
            // → host quit cascade. Calling try_close_browser DIRECTLY on a
            // Views-hosted browser tears the Window down WITHOUT firing
            // on_before_close, so the browser is never unregistered and the host
            // never quits — the orphaned-tree regression (Discussion #1680).
            //
            // The historical reason this used try_close_browser — window.close()'s
            // Widget::Close CHECKs !on_call_stack_ and aborts if the widget is
            // already being destroyed (e.g. macOS windowShouldClose racing this
            // queued IPC task) — is handled by the is_closed() guard below.
            // 2026-07-16 fix (docs/retro/retro-last-window-close-quit-race-2026-07-16.md):
            // "main" was the ONE label structurally excluded from ANY
            // srv-notify call anywhere in this function — every branch below
            // that calls `demote_srv_cleanup` (the actual `backend_close_window`
            // → srv `CloseWindow` → delete_workspace cascade trigger) is
            // explicitly gated on `self.label.starts_with("window-")` or
            // `self.label != "main"`, on the documented assumption that "main's
            // close feeds the tuned wrr last-window quit sequence and process
            // exit reaps everything there." Process exit does NOT reap
            // anything srv-side: srv is a separate process with its own
            // persistent DB, and nothing else ever tells it main's window
            // closed. Confirmed live: closing "main" left its srv window/
            // workspace/tab rows permanently orphaned, resurrected by
            // crash-reproject as a "ghost" window on every subsequent launch,
            // while every OTHER label's close correctly reached srv.
            //
            // MUST be synchronous (block this UI-thread task), not fired on a
            // background thread like `demote_srv_cleanup` does for window-*
            // labels. First attempt used the async version and it lost the
            // race every time: `lib.rs`'s shutdown sequence
            // (`run_message_loop()` returning → "Killing backend sidecar")
            // followed within ~50ms in live testing, killing the very srv
            // process the background thread's HTTP call needed to reach,
            // before it ever got there. CEF's message loop is single-threaded
            // — this task, the WRR win-event callback that eventually calls
            // `quit_message_loop()`, and everything else all run on the same
            // UI thread, so blocking HERE transitively delays all of that
            // until the notify finishes (or its own bounded ~1s retry +
            // ~2s-capped HTTP timeouts are exhausted) — which is the point.
            // Acceptable: this only affects the final moments of an
            // already-closing app, and mirrors the small bounded quit delay
            // the on_before_close-path fix (same retro doc) already accepts
            // for the platforms where that path is the live one.
            #[cfg(target_os = "windows")]
            if self.label == "main" {
                crate::launcher_ipc::report_panes_reaped(self.label.clone());
                let web_endpoint = self.state.backend_endpoints.lock().web_endpoint.clone();
                let auth_key = self.state.auth_key.lock().clone();
                let sleep_fn = |d: std::time::Duration| std::thread::sleep(d);
                match crate::client::retry_backend_window_id_lookup(
                    crate::client::BACKEND_WINDOW_ID_RETRY_ATTEMPTS,
                    crate::client::BACKEND_WINDOW_ID_RETRY_DELAY,
                    || self.state.backend_window_id(&self.label),
                    sleep_fn,
                ) {
                    Some(window_id) => {
                        crate::client::backend_close_window(&web_endpoint, &auth_key, &window_id);
                        crate::launcher_ipc::report_backend_window_id_unregistered(self.label.clone());
                    }
                    None => {
                        tracing::warn!(
                            target: "wrr",
                            label = %self.label,
                            "[close-window] main: no backend window ID after retries — srv state may orphan",
                        );
                        crate::launcher_ipc::report_backend_window_id_unregistered(self.label.clone());
                    }
                }
            }

            if let Some(window) = get_window_on_ui(&self.state, &self.label) {
                // CEF 148 Views: closing the WINDOW FIRST tears down the
                // Window but leaves the hosted browser HIDDEN/RECYCLED —
                // `on_before_close` never fires (the same property the quit
                // path works around in lib.rs with a hard TerminateProcess;
                // Discussion #1680). For MID-SESSION secondary-window closes
                // there was no compensation at all, so every open+close of an
                // extra window permanently leaked a live renderer process
                // (~100MB commit), the reducer `browsers` entry, and — because
                // the `on_before_close` → `backend_close_window` chain never
                // ran — the entire srv-side window/workspace/tab/block state.
                // Confirmed empirically with AGENTMUX_DEBUG_CLOSE=1 tracing:
                // renderer count grew +1 per open/close cycle, no
                // `on_before_close` was ever written, and a round-2 attempt
                // that forced `close_browser(1)` AFTER `window.close()` was
                // ALSO a no-op — once the Views window teardown has detached
                // the browser, no CEF close API reaches it anymore. See
                // docs/retro/retro-window-lifecycle-leak-2026-07-04.md and
                // docs/specs/SPEC_WINDOW_LIFECYCLE_CLOSE_RELIABILITY_2026_07_04.md.
                //
                // Round 5: ARM browser destruction, then FORCE the window
                // death CEF withholds — `close_browser(1)` followed by
                // native `DestroyWindow` on the top-level HWND, for
                // non-main labels (Windows). Empirical trail:
                //   Round 2 (`close_browser` AFTER `window.close()`): no-op —
                //     Views teardown detaches the browser first.
                //   Round 3 (`close_browser` BEFORE `window.close()`):
                //     destruction INITIATES (`do_close` fires) but CEF 148
                //     Views parks the browser instead of completing it.
                //   Round 4 (`DestroyWindow` alone, this session): the
                //     top-level HWND dies (ret=1, verified same-thread
                //     ownership, visible window gone) but NO browser
                //     callbacks fire — unlike #1957's `set_as_child`
                //     browser panes, a Views-hosted browser does NOT tear
                //     down when its HWND is destroyed out from under it;
                //     the renderer still leaks.
                // Round 5 combines the halves that each stalled alone:
                // `close_browser(1)` puts the browser into pending-
                // destruction (round 3's `do_close`), and the immediate
                // native `DestroyWindow` delivers the actual window death
                // Views was deferring, letting the armed destruction
                // complete → `on_before_close` → the §2 cleanup chain.
                // NOT `window.close()` (Views parks the browser), NOT
                // `PostMessage(WM_CLOSE)` (routes back through Views'
                // wndproc — the old #1680 hide-the-frame failure).
                //
                // Safety: uses resolve_window_hwnd_strict (validated cache /
                // registry only — NEVER the EnumWindows fallback, which for
                // an unknown label can return MAIN), plus an explicit
                // main-HWND guard. If strict resolution fails, fall through
                // to the round-3 browser-first sequence below (better a
                // known-leaky close than destroying the wrong window).
                //
                // Scope: NOT for "main" — main's close feeds the tuned wrr
                // last-window quit sequence and process exit reaps everything
                // there.
                //
                // Round 6 — for ALL secondary top-level windows (`window-*`:
                // pool-promoted `window-pool-*`, cold-path and drag-tear-off
                // `window-{uuid}`), the on_before_close cleanup chain never
                // fires on this build (rounds 2–5 evidence), so the srv-side
                // cleanup (backend_close_window → CloseWindow →
                // delete_workspace cascade) runs IMPERATIVELY here for every
                // such close. Scoped to `window-*`: floaters
                // (`floating-*`) DO get a working on_before_close via their
                // owned-popup DestroyWindow path (#1957 mechanism), and
                // browser panes never route through this task.
                //
                // Then, for PROMOTED POOL windows, don't destroy at all:
                // demote back into the warm pool — hidden, reloaded to the
                // pool boot URL, re-enqueued via the normal renderer-ready
                // handshake — so the renderer is REUSED, not leaked. If the
                // pool is at the demote cap (or demote is rejected), fall
                // through to the round-5 destroy: same parked-browser cost
                // as today, srv state still clean.
                //
                // Known residual (tracked in the retro): cold-path and
                // tear-off `window-{uuid}` windows can't re-enter the pool
                // (the pool handshake keys on the `window-pool-` label
                // prefix), so their close still parks the renderer via
                // round 5 — srv state IS cleaned, the ~100MB renderer is
                // not reclaimed. Pool "adoption" for foreign labels is the
                // follow-up. In the default flow this is rare: open_new_window
                // serves from the pool whenever it's non-empty.
                #[cfg(target_os = "windows")]
                if self.label.starts_with("window-") {
                    crate::commands::window_pool::demote_srv_cleanup(&self.state, &self.label);
                    // Residual 1 (SPEC_POOL_ADOPTION_AND_WINDOW_ROW_CRUMB_2026_07_11)
                    // — demote is attempted for EVERY window-* label, not just
                    // window-pool-*: a cold-path / tear-off `window-{uuid}`
                    // window is structurally identical to a promoted pool
                    // window, and adopting it into the warm pool reuses its
                    // renderer instead of stranding ~100MB in a park-and-blank.
                    // Safe because pool membership is tracked by the reducer's
                    // is_pool flag + unpromoted/queue sets, and every quit-path
                    // pool enumeration is type-based (pool_side_top_level_labels)
                    // — the label string is not a pool identity anywhere that
                    // matters post-adoption. Demote's own gates (cap, strict
                    // HWND, reducer accept) still apply; a refusal falls through
                    // to park-and-blank exactly as before.
                    if crate::commands::window_pool::demote_promoted_pool_window(
                        &self.state,
                        &self.label,
                        &window,
                    ) {
                        return;
                    }
                    // SPEC_PARK_AND_BLANK_CLOSE_2026_07_09.md — a non-demotable
                    // window-* close (pool at cap, or a foreign window-{uuid}
                    // the pool prefix gate rejects) used to fall to the round-5
                    // destroy, which parks the browser anyway (no
                    // on_before_close on this build) with the FULL workspace
                    // page still running — ~90MB+ commit leaked per close,
                    // measured. Park deliberately and blank the content
                    // instead; round 5 remains the strict-HWND-failure
                    // fallback (and the working path for floaters, which are
                    // not window-* and never reach this branch).
                    if crate::commands::window_pool::park_and_blank_window(
                        &self.state,
                        &self.label,
                    ) {
                        return;
                    }
                    // fall through to round-5 destroy below
                }
                #[cfg(target_os = "windows")]
                if self.label != "main" {
                    use windows_sys::Win32::UI::WindowsAndMessaging::DestroyWindow;
                    let strict = unsafe {
                        crate::commands::window::resolve_window_hwnd_strict(
                            &self.state,
                            &self.label,
                        )
                    };
                    if let Some(hwnd) = strict {
                        let main_hwnd = self.state.window_hwnds.lock().get("main").copied();
                        if main_hwnd == Some(hwnd as isize) {
                            crate::client::dlog(&format!(
                                "CloseWindowTask({}): strict HWND resolved to MAIN — refusing DestroyWindow, falling back",
                                self.label
                            ));
                        } else {
                            // Step 1 — arm the browser destruction while the
                            // browser is still attached and healthy (round 3;
                            // force=1 skips unload handlers — our own frontend,
                            // no user content).
                            if let Some(mut browser) = self.state.get_browser(&self.label) {
                                if let Some(host) = browser.host() {
                                    crate::client::dlog(&format!(
                                        "CloseWindowTask({}): round 5 — close_browser(1) to arm destruction",
                                        self.label
                                    ));
                                    host.close_browser(1);
                                }
                            }
                            tracing::info!(
                                target: "wrr",
                                label = %self.label,
                                hwnd = ?hwnd,
                                "[close-window] round 5: close_browser(1) + native DestroyWindow"
                            );
                            // Step 2 — deliver the window death Views defers.
                            // Forensics kept from round 4: same-thread
                            // ownership + ret/lasterr/liveness per close.
                            let destroy_confirmed = unsafe {
                                use windows_sys::Win32::Foundation::GetLastError;
                                use windows_sys::Win32::System::Threading::GetCurrentThreadId;
                                use windows_sys::Win32::UI::WindowsAndMessaging::{
                                    GetWindowThreadProcessId, IsWindow,
                                };
                                let mut owner_pid: u32 = 0;
                                let owner_tid = GetWindowThreadProcessId(hwnd, &mut owner_pid);
                                let my_tid = GetCurrentThreadId();
                                crate::client::dlog(&format!(
                                    "CloseWindowTask({}): round 5 — DestroyWindow({:?}) owner_tid={} my_tid={} owner_pid={}",
                                    self.label, hwnd, owner_tid, my_tid, owner_pid
                                ));
                                let ret = DestroyWindow(hwnd);
                                let err = if ret == 0 { GetLastError() } else { 0 };
                                let alive = IsWindow(hwnd);
                                crate::client::dlog(&format!(
                                    "CloseWindowTask({}): round 5 — DestroyWindow ret={} lasterr={} IsWindow_after={}",
                                    self.label, ret, err, alive
                                ));
                                ret != 0 || alive == 0
                            };
                            // Unregister only when the native destroy actually
                            // took (or the HWND is gone regardless) — a failed
                            // DestroyWindow with the window still alive must
                            // keep the registration, else the reducer
                            // undercounts a REAL window and re-opens the exact
                            // false-quit class this PR closes (reagent P2
                            // #2043). The armed close_browser(1) may still
                            // complete asynchronously → on_before_close does
                            // the unregister; if nothing completes, the window
                            // is visibly open and the quit gate correctly
                            // counts it on both sides.
                            if destroy_confirmed {
                                unregister_after_parking_close(&self.state, &self.label);
                            } else {
                                tracing::warn!(
                                    target: "wrr",
                                    label = %self.label,
                                    "[close-window] round 5: DestroyWindow failed with window still alive — keeping reducer registration"
                                );
                            }
                            return;
                        }
                    } else {
                        crate::client::dlog(&format!(
                            "CloseWindowTask({}): strict HWND resolution failed — falling back to round-3 close",
                            self.label
                        ));
                    }
                }
                // Round 3 (fallback / non-Windows): BROWSER-FIRST teardown for
                // non-main labels — force `close_browser(1)` while the browser
                // is still attached and healthy, then close the window. This
                // matches the standard CEF Views pattern (can_close →
                // try_close_browser → browser destruction drives window
                // close), just force-initiated from our side so the
                // destruction is guaranteed to start. force=1 skips unload
                // handlers — correct here: the content is our own frontend,
                // not user web content with unsaved state. On Windows this
                // path is KNOWN INSUFFICIENT (destruction initiates but parks
                // — see retro); it remains only as the no-strict-HWND fallback.
                if self.label != "main" {
                    if let Some(mut browser) = self.state.get_browser(&self.label) {
                        if let Some(host) = browser.host() {
                            tracing::info!(
                                target: "wrr",
                                label = %self.label,
                                "[close-window] browser-first teardown: close_browser(1) before window.close()"
                            );
                            crate::client::dlog(&format!(
                                "CloseWindowTask({}): close_browser(1) BEFORE window.close()",
                                self.label
                            ));
                            host.close_browser(1);
                        }
                    }
                }
                if window.is_closed() == 0 {
                    crate::client::dlog(&format!(
                        "CloseWindowTask({}): window.close()",
                        self.label
                    ));
                    window.close();
                } else {
                    crate::client::dlog(&format!(
                        "CloseWindowTask({}): window already closed — skipping window.close()",
                        self.label
                    ));
                }
                #[cfg(target_os = "windows")]
                unregister_after_parking_close(&self.state, &self.label);
                return;
            }
            crate::client::dlog(&format!(
                "CloseWindowTask({}): no CefWindow resolved — fallback try_close_browser",
                self.label
            ));
            // Fallback: no CefWindow for this label (non-Views path / pre-init
            // teardown) — close the browser handle directly.
            if let Some(mut browser) = self.state.get_browser(&self.label) {
                if let Some(host) = browser.host() {
                    host.try_close_browser();
                }
            }
            #[cfg(target_os = "windows")]
            unregister_after_parking_close(&self.state, &self.label);
        }
    }
}

/// SPEC_WRR_QUIT_FALSE_POSITIVE_2026_07_08.md Step C — on this CEF build
/// (148, Windows), every close path that ends in a PARKED browser fires no
/// `on_before_close`, so the reducer's `browsers` map keeps counting the
/// window as live forever. Live-verified 2026-07-09 for all three parking
/// paths: main's `window.close()` (#1680 park), the round-5
/// `close_browser(1)` + native `DestroyWindow` (browser parks anyway — two
/// round-5-closed windows stayed registered and were caught only by the quit
/// watchdog), and the round-3 no-strict-HWND fallback (documented "KNOWN
/// INSUFFICIENT" above). That staleness was survivable only while the WRR
/// last-window quit ignored the reducer; now that the quit gate requires
/// reducer agreement (`win_event.rs::should_quit_on_last_window`), every
/// parking close must tell the reducer imperatively — here, in the UI-thread
/// executor of the close, where the label is known with certainty. Demoted
/// pool windows are NOT routed here: `DemotePoolWindow` already flips their
/// `is_pool` (excluding them from the live count) and the pool machinery
/// owns their bookkeeping.
///
/// MUST run AFTER the close is initiated, never before: `get_window_on_ui`
/// and the `get_browser` fallback both resolve through `state.browsers`, so a
/// pre-close dispatch removes the registration the close itself needs and
/// turns `CloseWindowTask` into a silent no-op — the window simply stays open
/// (caught live 2026-07-09, first verification pass of this fix).
///
/// Also arms the WRR quit watchdog: a Views park is a *move*, not a
/// hide/destroy, so it can produce ZERO further win-events — with nothing to
/// re-run the quit gate, even a fully-correct registered count would sit
/// unread forever. The watchdog re-check is idempotent and stands down if any
/// window is still visible, so arming it while other windows remain open is
/// harmless.
///
/// Launcher/srv cleanup is NOT duplicated here: the `close_window` RPC
/// already sent `report_window_closed`, and `demote_srv_cleanup` already ran
/// for `window-*` labels.
///
/// Windows-only: on macOS/Linux `window.close()` → `can_close` →
/// `on_before_close` runs the full cleanup chain, and a parallel dispatch
/// would break that chain's label-by-identity lookup.
#[cfg(target_os = "windows")]
pub(crate) fn unregister_after_parking_close(state: &Arc<AppState>, label: &str) {
    tracing::info!(
        target: "wrr",
        "[close-window] {} close initiated — dispatching UnregisterBrowser (parked browser fires no on_before_close)",
        label
    );
    let out = state.host_dispatch(crate::reducer::HostCommand::UnregisterBrowser {
        label: label.to_string(),
    });
    // Pillar 2 Phase 2 — this is the dominant Windows close path; when the
    // unregistration zeroes the live count, the drain cascade fires HERE
    // (QuitState finally transitions on this path) instead of the quit
    // resting solely on WRR's OS-signal gate.
    consume_request_drain(state, &out, "unregister_after_parking_close");
    // 2026-07-16 fix (reagent P1 on PR #2186, docs/retro/retro-last-window-close-quit-race-2026-07-16.md):
    // report_pool_drain_decision is the ONLY source of Event::PoolDrained /
    // Event::PoolNotLast — the launcher's Phase F.6 window_cleanup saga
    // (agentmux-launcher/src/saga/window_cleanup.rs) needs one of those to
    // advance past its DrainingPool step. `on_before_close` reports this
    // (lifecycle.rs, same request_drain.is_some() condition), but this
    // function is the ACTUAL executor for every parking close on Windows —
    // on_before_close never fires for any of them (this whole function
    // exists because of that). Missing here meant every parking close's
    // saga stalled in DrainingPool until its 30s timeout, main's included —
    // silent because nothing else depended on the saga completing promptly,
    // just wasted launcher-side saga slots. Same gate as on_before_close's:
    // skip browser-pane-* (this function isn't reached for those anyway,
    // but mirrors the source of truth exactly rather than assuming).
    if !label.starts_with("browser-pane-") {
        let was_last = out.request_drain.is_some();
        crate::launcher_ipc::report_pool_drain_decision(label.to_string(), was_last);
    }
    if label != "main" {
        // Mirror on_before_close's cache eviction (stale entries break
        // WM_CLOSE routing on label reuse). Main keeps its entry — the
        // process is quitting and WRR still resolves it for logging.
        state.window_hwnds.lock().remove(label);
    }
    crate::wrr::win_event::arm_quit_watchdog(state.count_live_user_windows());
}

/// Pillar 2 Phase 2 (SPEC_PILLAR2_SANITIZE_THEN_DECIDE §2.4) — uniform
/// consumption of `reconcile_quit`'s decision at a dispatch site.
///
/// On the CEF UI thread the Stage-1 executor runs INLINE, matching the
/// `on_before_close` pattern: the cascade must complete before the caller's
/// next statement, because callers (the WRR LOCATIONCHANGE handler in
/// particular) re-run the last-window quit gate synchronously right after —
/// a posted cascade would let `quit_message_loop()` win the race and skip
/// the graceful Stage-1 pool drain entirely (reagent P1 on PR #2082).
/// WINEVENT callbacks run on the UI thread, so this covers the
/// parking-close, LOCATIONCHANGE, and demote sites.
///
/// Off the UI thread (promote-failure cleanup can run from command/IPC
/// threads) the executor is posted as a UI task — a one-loop-turn deferral,
/// harmless there because nothing on those paths quits synchronously.
/// `BeginDrain` is monotonic/idempotent, so racing consumers (e.g.
/// `on_before_close`'s own inline consumption) are benign; first one wins.
///
/// `post_task` can silently drop during teardown (v0.33.492) — but every
/// consumption site runs while the message loop is healthy by construction
/// (a drain verdict means nothing has begun tearing down yet; that is the
/// problem being solved). The entry logs below make any drop visible.
pub(crate) fn consume_request_drain(
    state: &Arc<AppState>,
    out: &crate::reducer::DispatchOutput,
    site: &str,
) {
    let Some(reason) = out.request_drain.clone() else { return };
    if currently_on(ThreadId::UI) != 0 {
        tracing::warn!(
            target: "wrr",
            "[drain-consume] site={} reason={:?} — running begin_drain_and_cascade inline (UI thread)",
            site, reason
        );
        begin_drain_and_cascade(state, reason);
        // Phase 3 — same deterministic-quit re-check as the posted task: a
        // drain with zero pool inventory produces no further OS window
        // events. Idempotent (QUIT_INITIATED); callers that re-run the gate
        // themselves (LOCATIONCHANGE) just hit the guard twice.
        #[cfg(target_os = "windows")]
        crate::wrr::win_event::reevaluate_last_window_quit();
        return;
    }
    let mut task = BeginDrainCascadeTask::new(state.clone(), reason.clone());
    let posted = post_task(ThreadId::UI, Some(&mut task));
    tracing::warn!(
        target: "wrr",
        "[drain-consume] site={} reason={:?} — posting begin_drain_and_cascade posted={}",
        site, reason, posted != 0
    );
}

wrap_task! {
    pub struct BeginDrainCascadeTask {
        state: Arc<AppState>,
        reason: crate::state::QuitReason,
    }

    impl Task {
        fn execute(&self) {
            begin_drain_and_cascade(&self.state, self.reason.clone());
            // Phase 3 — re-run the WRR Stage-2 gate now that QuitState has
            // flipped: a drain with zero pool inventory produces no further
            // OS window events, so without this re-check the quit would wait
            // out the watchdog. Idempotent (QUIT_INITIATED); a no-op when
            // pool closes are still in flight (their own HIDE/LOCATIONCHANGE
            // events re-run the gate as they land).
            #[cfg(target_os = "windows")]
            crate::wrr::win_event::reevaluate_last_window_quit();
        }
    }
}

/// Pillar 2 Stage-1 drain executor (SPEC_PILLAR2_SANITIZE_THEN_DECIDE §1.G) —
/// the ACTION half of the decision/action split (`reducer/quit.rs:49-54`).
/// Callers must have already observed `reconcile_quit`'s decision via
/// `DispatchOutput.request_drain` (the DECISION) before calling this — it does
/// not re-check anything, it just executes. Extracted verbatim from
/// `AgentMuxHandler::begin_drain_and_cascade` (which now delegates here) so it
/// is callable from any UI-thread context that holds `&Arc<AppState>` — the
/// close-edge in `on_before_close`, and (Phase 2) the parking-close /
/// LOCATIONCHANGE / pool-settling dispatch sites.
///
/// UI-THREAD ONLY. Only flips `QuitState` and closes the (already-hidden) pool
/// browsers. Never calls `quit_message_loop()` — that is Stage 2, gated
/// separately (`on_before_close`'s `browser_list.is_empty()` on macOS/Linux;
/// WRR's OS-signal executor on Windows), since calling it from inside another
/// browser's `on_before_close` deadlocks the UI thread (confirmed v0.33.498).
pub(crate) fn begin_drain_and_cascade(state: &Arc<AppState>, reason: crate::state::QuitReason) {
    // PR #5 H.5 — flip QuitState Running → Draining via reducer.
    // Mirrors the pre-PR Phase B.9.3 drain flag: spawn_pool_window
    // checks `quit_state != Running` (in the reducer's spawn arm)
    // and skips refill on every subsequent on_pool_window_destroyed
    // → no new pool browsers added → browsers map can actually
    // drain. BeginDrain is idempotent — safe if a duplicate
    // last-close (or a later reconcile) fires.
    state.host_dispatch(crate::reducer::HostCommand::BeginDrain { reason });
    tracing::warn!(target: "wrr", "[wrr] quit_state=Draining (drain mode)");

    // Phase H.2.b — reducer-aware iteration with fallback + drift logging.
    // Collect ALL background-only browsers: tab pool (window-pool-*)
    // AND pane pool (floating-pool-*). Both live in browser_list
    // (created via CreateWindowTask which clones the main top-level
    // client). Omitting pane pool windows here means browser_list
    // never empties on macOS/Linux (init_pane_pool spawns one at
    // startup), so Stage 2's is_empty() gate never fires and the
    // host hangs on every quit.
    // Tab-pool membership BY TYPE (reducer `is_pool` flag), not label prefix:
    // an ADOPTED pool window (Residual 1, SPEC_POOL_ADOPTION_AND_WINDOW_ROW_
    // CRUMB_2026_07_11) keeps its foreign `window-{uuid}` label while being
    // genuinely pool-side — a prefix filter would skip it here, leaving an
    // unswept browser that blocks Stage 2's `browser_list.is_empty()` gate.
    // The pane pool stays prefix-matched (`floating-pool-*` — pane-pool
    // adoption is out of scope).
    let tab_pool_labels = state.pool_side_top_level_labels();
    let pool_browsers: Vec<cef::Browser> = state
        .list_browsers()
        .into_iter()
        .filter(|(label, _)| {
            tab_pool_labels.contains(label) || label.starts_with("floating-pool-")
        })
        .map(|(_, b)| b)
        .collect();
    tracing::warn!(
        target: "wrr",
        "[wrr] stage 1: draining; closing {} pool browser(s) (tab+pane)",
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
                let mut task = crate::client::ClosePoolBrowserTask::new(b);
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
            let mut task = crate::client::ClosePoolBrowserTask::new(b);
            let posted = cef::post_task(cef::ThreadId::UI, Some(&mut task));
            tracing::debug!(
                target: "wrr-trace",
                "[trace] stage1[{}] post_task(close_browser) posted={}",
                i, posted != 0
            );
        }
    }
}

pub fn post_close_window(state: &Arc<AppState>, label: &str) {
    let mut task = CloseWindowTask::new(state.clone(), label.to_string());
    post_task(ThreadId::UI, Some(&mut task));
}

// ── Memory-pressure → frontend banner event ────────────────────────────────

wrap_task! {
    // `kind` distinguishes RAM pressure from Page File (commit) pressure —
    // SPEC_RAM_PAGEFILE_PRESSURE_SPLIT_2026_08_07. `system_managed`/
    // `disk_free_pct` are only meaningful for `kind == "pagefile"`, and even
    // then only when `has_disk_context` is true — a `GetDiskFreeSpaceExW`/
    // registry read failure (or `kind == "ram"`, where the concept doesn't
    // apply at all) must genuinely OMIT those two keys from the emitted
    // payload rather than substitute a value, or `pagefileGuidance()` on the
    // frontend renders a confident-sounding guess (reagent-caught P1: an
    // earlier version defaulted to "system-managed, disk healthy" on read
    // failure, which produces the reassuring message even while pressure is
    // Critical). Plain `bool`/`f64` fields (not `Option<T>`) because
    // `wrap_task!`'s generated struct is untested against `Option` fields —
    // the extra flag reaches the same "no guess" outcome without that risk.
    pub struct EmitMemoryPressureTask {
        state: Arc<AppState>,
        kind: String,
        level: String,
        free_mb: u64,
        has_disk_context: bool,
        system_managed: bool,
        disk_free_pct: f64,
    }

    impl Task {
        fn execute(&self) {
            let payload = if self.kind == "ram" {
                serde_json::json!({
                    "kind": self.kind,
                    "level": self.level,
                    "phys_free_mb": self.free_mb,
                })
            } else if self.has_disk_context {
                serde_json::json!({
                    "kind": self.kind,
                    "level": self.level,
                    "commit_free_mb": self.free_mb,
                    "system_managed": self.system_managed,
                    "disk_free_pct": self.disk_free_pct,
                })
            } else {
                serde_json::json!({
                    "kind": self.kind,
                    "level": self.level,
                    "commit_free_mb": self.free_mb,
                })
            };
            crate::events::emit_event_to_top_level_windows(
                &self.state,
                "memory-pressure",
                &payload,
            );
        }
    }
}

/// Push a RAM-pressure level transition to the frontend banner. Callable from
/// ANY thread (the memory heartbeat runs on a background std::thread); the
/// emit itself (CEF JS execution) must run on the UI thread, so it's wrapped
/// in a posted task. SPEC_RAM_PAGEFILE_PRESSURE_SPLIT_2026_08_07 §3/§5.
pub fn post_memory_pressure_ram(state: &Arc<AppState>, level: &str, phys_free_mb: u64) {
    let mut task = EmitMemoryPressureTask::new(
        state.clone(),
        "ram".to_string(),
        level.to_string(),
        phys_free_mb,
        false,
        false,
        0.0,
    );
    post_task(ThreadId::UI, Some(&mut task));
}

/// Push a Page File (commit) pressure level transition to the frontend
/// banner. `disk_context`, when `Some`, carries whether Windows can actually
/// grow the page file right now (`system_managed` + `disk_free_pct` on the
/// volume backing it) so the banner can pick the correct guidance —
/// SPEC_WIN10_PAGEFILE_OOM_CRASH_2026_06_29 §5.2 P0 via
/// SPEC_RAM_PAGEFILE_PRESSURE_SPLIT_2026_08_07 §4. `None` means the disk/
/// registry read failed this tick — the payload omits both fields entirely
/// (not a guessed default) so the frontend's own fail-open "no guidance"
/// path (`pagefileGuidance`) is what actually renders, not a false "healthy"
/// claim. Originally `post_memory_pressure`
/// (SPEC_MEMORY_PRESSURE_SUPERVISION_2026_06_16 §5.F); split by `kind`
/// alongside `post_memory_pressure_ram`.
pub fn post_memory_pressure_pagefile(
    state: &Arc<AppState>,
    level: &str,
    commit_free_mb: u64,
    disk_context: Option<(bool, f64)>,
) {
    let (has_disk_context, system_managed, disk_free_pct) = match disk_context {
        Some((system_managed, disk_free_pct)) => (true, system_managed, disk_free_pct),
        None => (false, false, 0.0),
    };
    let mut task = EmitMemoryPressureTask::new(
        state.clone(),
        "pagefile".to_string(),
        level.to_string(),
        commit_free_mb,
        has_disk_context,
        system_managed,
        disk_free_pct,
    );
    post_task(ThreadId::UI, Some(&mut task));
}

// ── Minimize ─────────────────────────────────────────────────────────────

wrap_task! {
    pub struct MinimizeWindowTask {
        state: Arc<AppState>,
        label: String,
    }

    impl Task {
        fn execute(&self) {
            if let Some(window) = get_window_on_ui(&self.state, &self.label) {
                window.minimize();
            }
        }
    }
}

pub fn post_minimize_window(state: &Arc<AppState>, label: &str) {
    let mut task = MinimizeWindowTask::new(state.clone(), label.to_string());
    post_task(ThreadId::UI, Some(&mut task));
}

// ── Maximize (toggle) ────────────────────────────────────────────────────

wrap_task! {
    pub struct MaximizeWindowTask {
        state: Arc<AppState>,
        label: String,
    }

    impl Task {
        fn execute(&self) {
            if let Some(window) = get_window_on_ui(&self.state, &self.label) {
                if window.is_maximized() != 0 {
                    window.restore();
                } else {
                    window.maximize();
                }
            }
        }
    }
}

pub fn post_maximize_window(state: &Arc<AppState>, label: &str) {
    let mut task = MaximizeWindowTask::new(state.clone(), label.to_string());
    post_task(ThreadId::UI, Some(&mut task));
}

// ── Focus/Activate ───────────────────────────────────────────────────────

wrap_task! {
    pub struct FocusWindowTask {
        state: Arc<AppState>,
        label: String,
    }

    impl Task {
        fn execute(&self) {
            // Floating panes are raw WS_POPUP HWNDs with no CEF Views
            // Window — the Views lookup below returns None for them, which
            // made focus a silent no-op (InstancePanel's floating rows did
            // nothing on click). Branch to Win32 via the floater registry
            // first. Spec: docs/specs/instance-panel-floating-panes.md §3.1.
            #[cfg(target_os = "windows")]
            if let Some(hwnd) = crate::floating_pane::floater_hwnd_for_label(&self.label) {
                use windows_sys::Win32::UI::WindowsAndMessaging::{
                    IsIconic, SetForegroundWindow, ShowWindow, SW_RESTORE,
                };
                unsafe {
                    // Restore first — SetForegroundWindow on a minimized
                    // window activates it without un-minimizing.
                    if IsIconic(hwnd as _) != 0 {
                        ShowWindow(hwnd as _, SW_RESTORE);
                    }
                    SetForegroundWindow(hwnd as _);
                }
                return;
            }
            if let Some(window) = get_window_on_ui(&self.state, &self.label) {
                window.activate();
            }
        }
    }
}

pub fn post_focus_window(state: &Arc<AppState>, label: &str) {
    let mut task = FocusWindowTask::new(state.clone(), label.to_string());
    post_task(ThreadId::UI, Some(&mut task));
}

// ── Window alpha (macOS uniform whole-window opacity) ────────────────────
// Track 1 of SPEC_TRANSPARENCY_MACOS_LINUX_2026_07_01: the macOS analogue of
// Windows' WS_EX_LAYERED + SetLayeredWindowAttributes(LWA_ALPHA) — a
// WindowServer-level uniform fade of the finished window over the desktop.
// Applied post-render, so it needs zero CEF/renderer cooperation and works on
// stock and patched frameworks alike. Per-pixel ("glass") transparency is the
// separate patched-libcef track and is orthogonal to this.

#[cfg(target_os = "macos")]
wrap_task! {
    pub struct SetWindowAlphaTask {
        state: Arc<AppState>,
        label: String,
        alpha: f64,
    }

    impl Task {
        fn execute(&self) {
            let Some(window) = get_window_on_ui(&self.state, &self.label) else {
                tracing::warn!(label = %self.label, "[opacity] SetWindowAlphaTask: no window for label");
                return;
            };
            let nsview = window.window_handle() as *mut std::ffi::c_void;
            if nsview.is_null() {
                tracing::warn!(label = %self.label, "[opacity] SetWindowAlphaTask: null NSView handle");
                return;
            }
            if unsafe { macos_set_nswindow_alpha(nsview, self.alpha) } {
                tracing::info!(label = %self.label, alpha = self.alpha, "[opacity] applied NSWindow alphaValue");
            } else {
                // Reagent P2 on #1895: don't claim success when the NSView has
                // no NSWindow yet (window not realized) — nothing was applied.
                tracing::warn!(label = %self.label, "[opacity] SetWindowAlphaTask: NSView has no NSWindow; alpha not applied");
                return;
            }

            // Codex P2 on #1895: browser-pane overlays are separate
            // NativeWidgetMacNSWindow instances layered over this window
            // (browser_pane/creation_views.rs) — child NSWindows do NOT
            // inherit the parent's alphaValue, so without this a faded host
            // window keeps fully-opaque pane rectangles floating on top.
            // Resolve each overlay belonging to this window_label via the
            // cached window numbers and fade it to the same alpha.
            // try_lock (matching the overlay-wnum cache writers): missing a
            // beat here only delays an overlay fade until the next opacity
            // event, which beats risking a UI-thread stall.
            let overlay_wnums: Vec<isize> = {
                let (Some(overlays), Some(wnums)) = (
                    self.state.browser_pane_overlays.try_lock(),
                    self.state.browser_pane_overlay_wnums.try_lock(),
                ) else {
                    tracing::warn!(label = %self.label, "[opacity] overlay maps busy; pane overlays not faded this pass");
                    return;
                };
                overlays
                    .iter()
                    .filter(|(_, (window_label, _))| window_label == &self.label)
                    .filter_map(|(pane_label, _)| wnums.get(pane_label).copied())
                    .collect()
            };
            for wnum in overlay_wnums {
                if unsafe { macos_set_window_alpha_by_number(wnum, self.alpha) } {
                    tracing::info!(label = %self.label, wnum, alpha = self.alpha, "[opacity] applied alphaValue to pane overlay window");
                } else {
                    tracing::warn!(label = %self.label, wnum, "[opacity] pane overlay NSWindow not found for wnum");
                }
            }
        }
    }
}

#[cfg(target_os = "macos")]
pub fn post_set_window_alpha(state: &Arc<AppState>, label: &str, alpha: f64) {
    let mut task = SetWindowAlphaTask::new(state.clone(), label.to_string(), alpha);
    post_task(ThreadId::UI, Some(&mut task));
}

/// `[[nsview window] setAlphaValue:alpha]` — raw libobjc FFI, mirroring
/// `ensure_macos_native_window_buttons` in app.rs. `alphaValue` takes CGFloat
/// (f64 on both arm64 and x86_64, passed in a float register, so plain
/// objc_msgSend is correct). AppKit call — must run on the UI/main thread,
/// which SetWindowAlphaTask guarantees. Returns false when the NSView has no
/// NSWindow (nothing applied).
#[cfg(target_os = "macos")]
unsafe fn macos_set_nswindow_alpha(nsview: *mut std::ffi::c_void, alpha: f64) -> bool {
    use std::ffi::{c_char, c_void};
    type Id = *mut c_void;
    type Sel = *const c_void;
    extern "C" {
        fn sel_registerName(name: *const c_char) -> Sel;
        fn objc_msgSend();
    }

    // nswindow = [nsview window]
    let get_window: extern "C" fn(Id, Sel) -> Id =
        std::mem::transmute(objc_msgSend as *const c_void);
    let nswindow = get_window(nsview, sel_registerName(b"window\0".as_ptr() as _));
    if nswindow.is_null() {
        return false;
    }

    // [nswindow setAlphaValue: alpha]
    let set_alpha: extern "C" fn(Id, Sel, f64) =
        std::mem::transmute(objc_msgSend as *const c_void);
    set_alpha(nswindow, sel_registerName(b"setAlphaValue:\0".as_ptr() as _), alpha);
    true
}

/// `[[NSApp windowWithWindowNumber:wnum] setAlphaValue:alpha]` — fade a
/// window resolved by its WindowServer window number (the form the
/// browser-pane overlay cache stores). Returns false when no window matches
/// (overlay already closed / wnum stale). UI/main thread only.
#[cfg(target_os = "macos")]
unsafe fn macos_set_window_alpha_by_number(wnum: isize, alpha: f64) -> bool {
    use std::ffi::{c_char, c_void};
    type Id = *mut c_void;
    type Sel = *const c_void;
    extern "C" {
        fn sel_registerName(name: *const c_char) -> Sel;
        fn objc_getClass(name: *const c_char) -> Id;
        fn objc_msgSend();
    }

    let msg: extern "C" fn(Id, Sel) -> Id = std::mem::transmute(objc_msgSend as *const c_void);
    let nsapp = msg(
        objc_getClass(b"NSApplication\0".as_ptr() as _),
        sel_registerName(b"sharedApplication\0".as_ptr() as _),
    );
    if nsapp.is_null() {
        return false;
    }

    // [nsapp windowWithWindowNumber: wnum]
    let win_by_num: extern "C" fn(Id, Sel, isize) -> Id =
        std::mem::transmute(objc_msgSend as *const c_void);
    let nswindow = win_by_num(
        nsapp,
        sel_registerName(b"windowWithWindowNumber:\0".as_ptr() as _),
        wnum,
    );
    if nswindow.is_null() {
        return false;
    }

    let set_alpha: extern "C" fn(Id, Sel, f64) =
        std::mem::transmute(objc_msgSend as *const c_void);
    set_alpha(nswindow, sel_registerName(b"setAlphaValue:\0".as_ptr() as _), alpha);
    true
}

// ── Window alpha (Linux/X11 uniform whole-window opacity) ────────────────
// Track 1 of SPEC_TRANSPARENCY_MACOS_LINUX_2026_07_01, Linux arm: the EWMH
// analogue of Win32 LWA_ALPHA / NSWindow.alphaValue. The compositor (Mutter,
// KWin, picom, xfwm4) fades the finished window over the desktop — including
// under XWayland, which is AgentMux's default ozone platform. Post-render:
// needs no CEF/renderer cooperation. Native-Wayland ozone has no equivalent
// protocol; there we log once and no-op (per-pixel Track 2 is the only route).

#[cfg(target_os = "linux")]
wrap_task! {
    pub struct SetWindowAlphaTask {
        state: Arc<AppState>,
        label: String,
        alpha: f64,
    }

    impl Task {
        fn execute(&self) {
            // browser_view.window() returns None post-load on Linux (same
            // CEF behaviour that broke GetWindowPositionTask — see the
            // state.windows fallback there). state.windows is populated at
            // on_window_created and stays valid for the window's lifetime.
            let window = get_window_on_ui(&self.state, &self.label)
                .or_else(|| self.state.windows.lock().get(&self.label).cloned());
            let Some(window) = window else {
                tracing::warn!(label = %self.label, "[opacity] SetWindowAlphaTask: no window for label");
                return;
            };
            // Under ozone-x11 `window_handle()` is the X11 Window XID. Under
            // native Wayland it is not an XID and there is no uniform-alpha
            // protocol at all — the host routes `window:transparent=true`
            // sessions through XWayland (app.rs ozone selection), so this
            // guard only fires on an explicit AGENTMUX_OZONE_PLATFORM=wayland
            // override or when opacity is requested with transparency off.
            if crate::app::SELECTED_OZONE_PLATFORM.get().map(String::as_str) == Some("wayland") {
                tracing::warn!("[opacity] uniform window alpha unsupported on native Wayland (no protocol); set window:transparent=true (XWayland) or use per-pixel transparency");
                return;
            }
            let xid = window.window_handle() as u32;
            if xid == 0 {
                tracing::warn!(label = %self.label, "[opacity] SetWindowAlphaTask: null X11 window handle");
                return;
            }
            match x11_set_window_opacity(xid, self.alpha) {
                Ok(()) => tracing::info!(label = %self.label, alpha = self.alpha, "[opacity] applied _NET_WM_WINDOW_OPACITY"),
                Err(e) => tracing::warn!(label = %self.label, "[opacity] X11 property set failed: {e}"),
            }
        }
    }
}

#[cfg(target_os = "linux")]
pub fn post_set_window_alpha(state: &Arc<AppState>, label: &str, alpha: f64) {
    let mut task = SetWindowAlphaTask::new(state.clone(), label.to_string(), alpha);
    post_task(ThreadId::UI, Some(&mut task));
}

/// Set (or clear, when alpha >= 1.0) the EWMH `_NET_WM_WINDOW_OPACITY`
/// CARDINAL/32 property on the toplevel client window. Value is
/// alpha × 0xFFFFFFFF. Modern compositors read it from the client window.
#[cfg(target_os = "linux")]
fn x11_set_window_opacity(xid: u32, alpha: f64) -> Result<(), Box<dyn std::error::Error>> {
    use std::cell::RefCell;
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{Atom, AtomEnum, ConnectionExt as _, PropMode};
    use x11rb::rust_connection::RustConnection;
    use x11rb::wrapper::ConnectionExt as _;

    // One connection + interned atom, cached per thread — this only ever runs
    // on the CEF UI thread (SetWindowAlphaTask), so thread_local needs no
    // locking. Avoids a fresh socket + intern_atom round-trip per event when
    // the user drags the opacity slider (reagent P1 on #1905). On an X error
    // the cache is dropped and we reconnect once — covers an XWayland restart
    // without reintroducing per-event churn on the happy path.
    thread_local! {
        static X11_OPACITY_CONN: RefCell<Option<(RustConnection, Atom)>> =
            const { RefCell::new(None) };
    }

    fn connect() -> Result<(RustConnection, Atom), Box<dyn std::error::Error>> {
        let (conn, _screen) = x11rb::connect(None)?;
        let atom = conn
            .intern_atom(false, b"_NET_WM_WINDOW_OPACITY")?
            .reply()?
            .atom;
        Ok((conn, atom))
    }

    fn apply(
        conn: &RustConnection,
        atom: Atom,
        xid: u32,
        alpha: f64,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if alpha >= 1.0 {
            conn.delete_property(xid, atom)?.check()?;
        } else {
            let value = (alpha.clamp(0.0, 1.0) * u32::MAX as f64) as u32;
            conn.change_property32(PropMode::REPLACE, xid, atom, AtomEnum::CARDINAL, &[value])?
                .check()?;
        }
        conn.flush()?;
        Ok(())
    }

    X11_OPACITY_CONN.with(|cell| {
        let mut slot = cell.borrow_mut();
        if slot.is_none() {
            *slot = Some(connect()?);
        }
        let (conn, atom) = slot.as_ref().expect("slot populated above");
        match apply(conn, *atom, xid, alpha) {
            Ok(()) => Ok(()),
            Err(_) => {
                // Cached connection may be dead — retry once on a fresh one,
                // and only re-cache it if the retry succeeds (a persistent
                // error like BadWindow shouldn't evict a healthy connection
                // slot with a doomed one… nor cache anything at all).
                *slot = None;
                let (fresh_conn, fresh_atom) = connect()?;
                let out = apply(&fresh_conn, fresh_atom, xid, alpha);
                if out.is_ok() {
                    *slot = Some((fresh_conn, fresh_atom));
                }
                out
            }
        }
    })
}

// ── Move window ───────────────────────────────────────────────────────────

wrap_task! {
    pub struct MoveWindowTask {
        state: Arc<AppState>,
        label: String,
        dx: i32,
        dy: i32,
    }

    impl Task {
        fn execute(&self) {
            if let Some(window) = get_window_on_ui(&self.state, &self.label) {
                let bounds = window.bounds();
                window.set_bounds(Some(&Rect {
                    x: bounds.x + self.dx,
                    y: bounds.y + self.dy,
                    width: bounds.width,
                    height: bounds.height,
                }));
            }
        }
    }
}

pub fn post_move_window(state: &Arc<AppState>, label: &str, dx: i32, dy: i32) {
    let mut task = MoveWindowTask::new(state.clone(), label.to_string(), dx, dy);
    post_task(ThreadId::UI, Some(&mut task));
}

// ── Set window to absolute position ──────────────────────────────────────

wrap_task! {
    pub struct SetWindowPositionTask {
        state: Arc<AppState>,
        label: String,
        x: i32,
        y: i32,
    }

    impl Task {
        fn execute(&self) {
            if let Some(window) = get_window_on_ui(&self.state, &self.label) {
                let bounds = window.bounds();
                window.set_bounds(Some(&Rect {
                    x: self.x,
                    y: self.y,
                    width: bounds.width,
                    height: bounds.height,
                }));
            }
        }
    }
}

pub fn post_set_window_position(state: &Arc<AppState>, label: &str, x: i32, y: i32) {
    let mut task = SetWindowPositionTask::new(state.clone(), label.to_string(), x, y);
    post_task(ThreadId::UI, Some(&mut task));
}

// ── Get window absolute position (DIP) — blocking UI-thread read ──────────
//
// CEF Views `window.bounds()` must run on the UI thread, but
// `get_window_position` is a synchronous IPC command dispatched on the
// (non-UI) IPC thread. Post a task that reads the bounds on the UI thread and
// hand the DIP origin back over a bounded channel. Used by the macOS / Linux
// floating-pane header drag, which needs the window's current position as the
// absolute-move baseline (Windows reads it directly via GetWindowRect, which
// is thread-agnostic).
wrap_task! {
    pub struct GetWindowPositionTask {
        state: Arc<AppState>,
        label: String,
        tx: std::sync::mpsc::SyncSender<Option<(i32, i32)>>,
    }

    impl Task {
        fn execute(&self) {
            // Primary: browser_view.window() — works on Windows and on non-Windows
            // windows that haven't finished loading. On Linux/macOS the Views
            // BrowserView loses its Window reference post-page-load, so
            // browser_view.window() returns None. Fall back to state.windows,
            // which is populated via on_window_created and stays valid for the
            // lifetime of the window (same registry ResolveWindowAtCursorTask uses).
            let pos = if let Some(w) = get_window_on_ui(&self.state, &self.label) {
                let b = w.bounds();
                Some((b.x, b.y))
            } else {
                // Fall back to `state.windows` on Linux/macOS, where the Views
                // BrowserView loses its Window reference post-page-load. That map
                // is `cfg(not(windows))`-only (Windows uses native HWND lookup and
                // never populates it), so on Windows the primary path above is the
                // only source — gate the fallback to keep the Windows build green.
                #[cfg(not(target_os = "windows"))]
                {
                    self.state.windows.lock().get(&self.label).map(|w| {
                        let b = w.bounds();
                        (b.x, b.y)
                    })
                }
                #[cfg(target_os = "windows")]
                {
                    None
                }
            };
            // Capacity-1, freshly created per call → try_send never blocks
            // the UI thread.
            let _ = self.tx.try_send(pos);
        }
    }
}

/// Read a CEF Views window's absolute position (DIP) from the IPC thread by
/// bouncing through the UI thread. `None` if the window isn't found or the UI
/// thread doesn't answer within the timeout (e.g. mid-teardown).
pub fn get_window_position_blocking(state: &Arc<AppState>, label: &str) -> Option<(i32, i32)> {
    let (tx, rx) = std::sync::mpsc::sync_channel::<Option<(i32, i32)>>(1);
    let mut task = GetWindowPositionTask::new(state.clone(), label.to_string(), tx);
    post_task(ThreadId::UI, Some(&mut task));
    rx.recv_timeout(std::time::Duration::from_millis(250)).ok().flatten()
}

// ── Get window full rect (DIP) — blocking UI-thread read ─────────────────
//
// Like GetWindowPositionTask but returns (x, y, width, height). Used by the
// macOS / Linux floater edge-resize path (`get_window_rect` IPC) to capture
// the start rect on pointer-down — Windows reads it directly via GetWindowRect.
wrap_task! {
    pub struct GetWindowRectTask {
        state: Arc<AppState>,
        label: String,
        tx: std::sync::mpsc::SyncSender<Option<(i32, i32, i32, i32)>>,
    }

    impl Task {
        fn execute(&self) {
            let rect = get_window_on_ui(&self.state, &self.label).map(|w| {
                let b = w.bounds();
                (b.x, b.y, b.width, b.height)
            });
            let _ = self.tx.try_send(rect);
        }
    }
}

/// Read a CEF Views window's full rect (DIP) from the IPC thread by bouncing
/// through the UI thread. Returns `None` if the window isn't found or the UI
/// thread doesn't answer within the timeout.
pub fn get_window_rect_blocking(state: &Arc<AppState>, label: &str) -> Option<(i32, i32, i32, i32)> {
    let (tx, rx) = std::sync::mpsc::sync_channel::<Option<(i32, i32, i32, i32)>>(1);
    let mut task = GetWindowRectTask::new(state.clone(), label.to_string(), tx);
    post_task(ThreadId::UI, Some(&mut task));
    // 500ms: the UI thread may still be processing set_bounds tasks queued
    // during a prior drag — give it more headroom before treating as failed.
    rx.recv_timeout(std::time::Duration::from_millis(500)).ok().flatten()
}

// ── Set window rect (position + size, DIP) ───────────────────────────────
//
// Non-Windows analogue of the Windows SetWindowPos call in `set_window_rect`.
// Used by the floater edge-resize drag: the frontend captures the start rect on
// pointer-down, computes a new rect per cursor delta + edge, and calls this on
// each move. `set_bounds` is self-contained (no read-modify-write) so concurrent
// in-flight calls are idempotent — last write wins.
wrap_task! {
    pub struct SetWindowRectTask {
        state: Arc<AppState>,
        label: String,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    }

    impl Task {
        fn execute(&self) {
            if let Some(window) = get_window_on_ui(&self.state, &self.label) {
                window.set_bounds(Some(&cef::Rect {
                    x: self.x,
                    y: self.y,
                    width: self.width,
                    height: self.height,
                }));
            }
        }
    }
}

pub fn post_set_window_rect(state: &Arc<AppState>, label: &str, x: i32, y: i32, width: i32, height: i32) {
    let mut task = SetWindowRectTask::new(state.clone(), label.to_string(), x, y, width, height);
    post_task(ThreadId::UI, Some(&mut task));
}

// ── Resolve which window is under a screen point (DIP) — blocking UI read ──
//
// The macOS/Linux analogue of the Windows HWND Z-order walk in
// `commands/window/motion.rs::resolve_window_at_cursor`. Used by floating-pane
// REDOCK to find the AgentMux window the cursor is over at drop time. CEF Views
// `bounds()` must run on the UI thread, so iterate the registered top-level
// windows there and hit-test the DIP point against each.
//
// Overlap rule (pragmatic first cut — see the redock report): exclude the drag
// source; among the rest, prefer a non-"main" match (a floater/tear-off stacked
// above main is almost always the intended target) over "main"; "main" wins
// only when it's the sole match. True Z-order among multiple overlapping
// non-main windows is a follow-up (would need `[NSApp orderedWindows]` + a
// label↔NSWindow registry).
#[cfg(not(target_os = "windows"))]
wrap_task! {
    pub struct ResolveWindowAtCursorTask {
        state: Arc<AppState>,
        x: i32,
        y: i32,
        exclude_label: String,
        tx: std::sync::mpsc::SyncSender<Option<String>>,
    }

    impl Task {
        fn execute(&self) {
            let windows = self.state.windows.lock();
            let mut main_match = false;
            let mut best_other: Option<String> = None;
            for (label, window) in windows.iter() {
                if label.as_str() == self.exclude_label {
                    continue;
                }
                let b = window.bounds();
                let hit = self.x >= b.x
                    && self.x < b.x + b.width
                    && self.y >= b.y
                    && self.y < b.y + b.height;
                if !hit {
                    continue;
                }
                if label.as_str() == "main" {
                    main_match = true;
                } else if label.starts_with("floating-") {
                    // Floating panes are never valid redock targets — skip
                    // them so a dragged pane hovering over a stacked floater
                    // doesn't ghost the idle floater instead of main.
                } else {
                    // Deterministic pick among overlapping non-main windows:
                    // lexicographically smallest label. (HashMap iteration
                    // order is otherwise nondeterministic.)
                    match &best_other {
                        Some(cur) if cur.as_str() <= label.as_str() => {}
                        _ => best_other = Some(label.clone()),
                    }
                }
            }
            let result = best_other.or(if main_match { Some("main".to_string()) } else { None });
            let _ = self.tx.try_send(result);
        }
    }
}

/// Resolve the label of the top-most AgentMux window containing the DIP screen
/// point `(x, y)`, excluding `exclude_label` (the drag source). `None` if the
/// point is over the desktop / an external app / only the source window, or if
/// the UI thread doesn't answer within the timeout.
#[cfg(not(target_os = "windows"))]
pub fn resolve_window_at_cursor_blocking(
    state: &Arc<AppState>,
    x: i32,
    y: i32,
    exclude_label: &str,
) -> Option<String> {
    let (tx, rx) = std::sync::mpsc::sync_channel::<Option<String>>(1);
    let mut task =
        ResolveWindowAtCursorTask::new(state.clone(), x, y, exclude_label.to_string(), tx);
    post_task(ThreadId::UI, Some(&mut task));
    rx.recv_timeout(std::time::Duration::from_millis(250)).ok().flatten()
}

// ── Phase B.9.2 (WRR) — corrective absolute-position move ─────────────────
//
// Reducer-driven self-heal. Triggered by `Event::CorrectiveWindowMove` when
// the reducer detects an off-monitor / sentinel-parked window that the user
// has never foregrounded. We bypass `state.browsers` lookup-by-label (the
// label might not be registered yet at correction time) and use Win32
// SetWindowPos directly against the HWND. Must run on the UI thread because
// CEF Views' window backing the HWND is owned by the UI thread.

wrap_task! {
    pub struct CorrectiveWindowMoveTask {
        state: Arc<AppState>,
        hwnd: u64,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    }

    impl Task {
        fn execute(&self) {
            #[cfg(target_os = "windows")]
            unsafe {
                use windows_sys::Win32::Foundation::HWND;
                use windows_sys::Win32::UI::WindowsAndMessaging::{
                    SetWindowPos, SWP_NOACTIVATE, SWP_NOZORDER,
                };
                let h = self.hwnd as HWND;
                let ok = SetWindowPos(
                    h,
                    std::ptr::null_mut(),
                    self.x,
                    self.y,
                    self.w,
                    self.h,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                tracing::info!(
                    target: "wrr",
                    "[wrr] corrective SetWindowPos hwnd={:#x} -> ({},{}) {}x{} ok={}",
                    self.hwnd, self.x, self.y, self.w, self.h, ok != 0
                );
            }
            #[cfg(not(target_os = "windows"))]
            {
                let _ = &self.state; // suppress unused on non-Windows
                tracing::warn!(
                    target: "wrr",
                    "[wrr] corrective move requested on non-Windows host: ignored"
                );
            }
        }
    }
}

pub fn post_corrective_window_move(state: &Arc<AppState>, hwnd: u64, x: i32, y: i32, w: i32, h: i32) {
    let mut task = CorrectiveWindowMoveTask::new(state.clone(), hwnd, x, y, w, h);
    post_task(ThreadId::UI, Some(&mut task));
}

// Phase B.9.3 (WRR) — `Event::HostShouldQuit` handling lives in
// `launcher_ipc::apply_event_to_shadow`. After three smoke
// iterations (v0.33.491–v0.33.493) confirmed `cef::post_task`
// silently drops new tasks during the last-window-closed
// teardown window — even when previously-posted tasks still
// run — we bypass CEF entirely and use Win32
// `PostThreadMessage(host_main_tid, WM_QUIT, 0, 0)` via
// `wrr::win_event::post_thread_quit_message`. The UI thread's
// captured TID is stored at `install_hooks` time.

// ── Create new window (CEF Views) ───────────────────────────────────────

wrap_task! {
    pub struct CreateWindowTask {
        state: Arc<AppState>,
        url: String,
        label: String,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        frameless: bool,
    }

    impl Task {
        fn execute(&self) {
            use std::cell::RefCell;

            // Phase 1 diagnostic tracing — see
            // docs/specs/SPEC_HOST_WINDOW_CREATION_RUNNER_2026-05-02.md.
            // Identify which exact CEF call wedges the UI thread under
            // concurrent window creation.
            let t0 = std::time::Instant::now();
            tracing::info!(label = %self.label, "[create-window] task entered UI thread");

            let settings = BrowserSettings {
                // ARGB alpha=0 → transparent, mirroring the MAIN window
                // (app.rs:679) and the global CefSettings.background_color
                // (main.rs). CreateWindowTask builds every secondary window
                // on Linux/macOS — additional windows AND floating-pane
                // tear-offs (open_floating_pane_window routes here on
                // non-Windows; the dedicated post_create_floating_window is
                // Windows-only). Previously hard-coded 0xFF000000 (opaque
                // black), which (a) overrode the transparent global default
                // and (b) gated OFF the BrowserViewImpl transparency cascade
                // (it only fires when default_background_color_ is
                // transparent — see cef/libcef/browser/views/browser_view_impl.cc
                // WebContentsCreated). Result: floaters/secondary windows were
                // fully opaque even when window:transparent=true. 0x00000000
                // lets them inherit the same transparency path as main.
                background_color: 0x00000000,
                ..Default::default()
            };
            let cef_url = CefString::from(self.url.as_str());

            // Get client from an existing TOP-LEVEL browser.
            // Use list_top_level_browsers() rather than list_browsers() +
            // manual filter — the dedicated helper already excludes pane
            // browsers (kind: Pane{..}), removing the label-prefix heuristic.
            //
            // GUARD: if no top-level browser is alive at this point (race
            // between window close → UnregisterBrowser and this task being
            // posted, or all windows closing during a multi-window tear-off),
            // bail early rather than passing None to browser_view_create.
            // CEF's C++ layer CHECK-fails on a null client → SIGABRT on
            // CrBrowserMain. A graceful return here lets the launcher's crash-
            // budget supervisor retry (with --disable-gpu) only on real CEF
            // faults, not on this transient race.
            let client = self
                .state
                .list_top_level_browsers()
                .into_iter()
                .find_map(|(_, b)| {
                    b.host().and_then(|h| h.client())
                });
            tracing::info!(
                label = %self.label,
                elapsed_us = t0.elapsed().as_micros() as u64,
                client_found = client.is_some(),
                "[create-window] got client"
            );

            let mut client_ref = match client {
                Some(c) => c,
                None => {
                    tracing::error!(
                        label = %self.label,
                        elapsed_us = t0.elapsed().as_micros() as u64,
                        "[create-window] no live top-level browser to clone client from \
                         (all windows closing?) — aborting window creation"
                    );
                    return;
                }
            };

            let mut request_context = crate::commands::create_isolated_request_context(
                &self.state, &self.label,
            );
            tracing::info!(
                label = %self.label,
                elapsed_us = t0.elapsed().as_micros() as u64,
                "[create-window] request_context resolved"
            );
            let mut bv_delegate = crate::app::AgentMuxBrowserViewDelegate::new(
                RuntimeStyle::ALLOY,
            );
            let browser_view = browser_view_create(
                Some(&mut client_ref),
                Some(&cef_url),
                Some(&settings),
                None,
                request_context.as_mut(),
                Some(&mut bv_delegate),
            );
            tracing::info!(
                label = %self.label,
                elapsed_us = t0.elapsed().as_micros() as u64,
                "[create-window] browser_view_create returned"
            );

            let mut wd = crate::app::AgentMuxWindowDelegate::new(
                RefCell::new(browser_view),
                Some((self.x, self.y, self.w, self.h)),
                self.frameless,
                RuntimeStyle::ALLOY,
                Some((self.state.clone(), self.label.clone())),
            );
            #[cfg(target_os = "linux")]
            crate::app::install_linux_window_properties_override(&wd);
            window_create_top_level(Some(&mut wd));
            tracing::info!(
                label = %self.label,
                elapsed_us = t0.elapsed().as_micros() as u64,
                "[create-window] window_create_top_level returned"
            );
        }
    }
}

pub fn post_create_window(
    state: &Arc<AppState>,
    url: &str,
    label: &str,
    x: i32, y: i32, w: i32, h: i32,
    frameless: bool,
) {
    let mut task = CreateWindowTask::new(
        state.clone(), url.to_string(), label.to_string(),
        x, y, w, h, frameless,
    );
    tracing::info!(
        label = %label,
        on_ui_thread = currently_on(ThreadId::UI) != 0,
        "[create-window] post_create_window: calling post_task"
    );
    post_task(ThreadId::UI, Some(&mut task));
    tracing::info!(label = %label, "[create-window] post_create_window: post_task returned");
}

// ── DevTools (toggle) ─────────────────────────────────────────────────────

wrap_task! {
    pub struct ShowDevToolsTask {
        state: Arc<AppState>,
        label: String,
    }

    impl Task {
        fn execute(&self) {
            // Phase H.2.b — reducer-aware lookup with fallback.
            let browser = match self.state.get_browser(&self.label) {
                Some(b) => b,
                None => {
                    tracing::warn!("[devtools] browser '{}' not found", self.label);
                    return;
                }
            };

            match browser.host() {
                Some(host) => {
                    // In CEF Views mode, window_info is ignored by show_dev_tools().
                    // CEF routes the DevTools popup through on_popup_browser_view_created
                    // in AgentMuxBrowserViewDelegate, which creates a native window for it.
                    if host.has_dev_tools() != 0 {
                        host.close_dev_tools();
                    } else {
                        host.show_dev_tools(None, None, None, None);
                    }
                }
                None => {
                    tracing::warn!("[devtools] no browser host for '{}'", self.label);
                }
            }
        }
    }
}

pub fn post_show_dev_tools(state: &Arc<AppState>, label: &str) {
    let mut task = ShowDevToolsTask::new(state.clone(), label.to_string());
    post_task(ThreadId::UI, Some(&mut task));
}

// ── DevTools — Inspect Element at coordinates ─────────────────────────────

wrap_task! {
    pub struct InspectElementAtTask {
        state: Arc<AppState>,
        label: String,
        x: i32,
        y: i32,
    }

    impl Task {
        fn execute(&self) {
            let browser = match self.state.get_browser(&self.label) {
                Some(b) => b,
                None => {
                    tracing::warn!("[devtools] inspect-at: browser '{}' not found", self.label);
                    return;
                }
            };

            match browser.host() {
                Some(host) => {
                    // The 4th arg to show_dev_tools is `inspect_element_at: Option<CefPoint>`
                    // in window-relative coords. CEF opens DevTools (creating it if not
                    // already open) and selects the element at that point, equivalent to
                    // Chrome's right-click → Inspect Element flow.
                    let point = Point { x: self.x, y: self.y };
                    host.show_dev_tools(None, None, None, Some(&point));
                }
                None => {
                    tracing::warn!("[devtools] inspect-at: no browser host for '{}'", self.label);
                }
            }
        }
    }
}

pub fn post_inspect_element_at(state: &Arc<AppState>, label: &str, x: i32, y: i32) {
    let mut task = InspectElementAtTask::new(state.clone(), label.to_string(), x, y);
    post_task(ThreadId::UI, Some(&mut task));
}

// ── Main-focus reclaim ────────────────────────────────────────────────────
//
// Reclaim keyboard focus for the main browser when the user clicks a
// main-DOM input (address bar, etc). Runs on the CEF UI thread because:
//   - host.set_focus / browser_view_get_for_browser require the UI thread
//   - walking the HWND tree via EnumChildWindows is safer post-setup when
//     Chromium has published all of its render widgets
//
// On Windows, after the Chromium-level focus flip we also walk the Views
// window for the Chrome_RenderWidgetHostHWND and Win32-SetFocus it — without
// that explicit Win32 SetFocus, keyboard events keep routing to whichever
// pane HWND currently holds Win32 focus even though Chromium "thinks" main
// is focused. Observed on v0.33.264: host.set_focus(1) on main left pane
// keystrokes arriving at the pane HWND for >2 seconds.

wrap_task! {
    pub struct MainFocusReclaimTask {
        state: Arc<AppState>,
        label: String,
    }

    impl Task {
        fn execute(&self) {
            // An empty label means "reclaim the foreground agentmux window" —
            // used by the pane-destroy focus handoff, which can't know the
            // surviving window's label up front (redock vs. in-window close).
            let label: String = if !self.label.is_empty() {
                self.label.clone()
            } else {
                #[cfg(target_os = "windows")]
                {
                    use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
                    let fg = unsafe { GetForegroundWindow() } as isize;
                    let resolved: Option<String> = if fg != 0 {
                        let map = self.state.window_hwnds.lock();
                        map.iter()
                            .find_map(|(k, &h)| if h == fg { Some(k.clone()) } else { None })
                    } else {
                        None
                    };
                    resolved.unwrap_or_else(|| "main".to_string())
                }
                #[cfg(not(target_os = "windows"))]
                {
                    "main".to_string()
                }
            };

            // Phase H.2.b — reducer-aware lookup with fallback.
            let mut browser = match self.state.get_browser(&label) {
                Some(b) => b,
                None => {
                    tracing::warn!("[main-focus-reclaim] no browser for label={}", label);
                    return;
                }
            };

            if let Some(host) = browser.host() {
                host.set_focus(1);
                tracing::info!("[main-focus-reclaim] host.set_focus(1) on label={}", label);
            }

            #[cfg(target_os = "windows")]
            {
                let views_top_hwnd = browser_view_get_for_browser(Some(&mut browser))
                    .and_then(|bv| bv.window())
                    .map(|w| w.window_handle().0 as *mut std::ffi::c_void)
                    .filter(|p| !p.is_null());

                // Collect every pane's outer HWND so we can skip render widgets
                // that descend from them. Panes are siblings of main under the
                // Views top-level, so a naive EnumChildWindows would pick up
                // their Chrome_RenderWidgetHostHWND and SetFocus on the wrong
                // target.
                //
                // Two sources are combined:
                // 1. Live registered panes from state.list_browsers().
                // 2. Pane outer HWNDs still tracked in BROWSER_PANE_HWND_CONTEXT
                //    — covers the window between BrowserUnregistered and CEF's
                //    on_before_close (deferred teardown), during which the HWND
                //    is still live but the label is gone from state.browsers.
                //    Without this, panes_excluded=0 and find_main_render_widget
                //    picks the pane's render widget → infinite focus storm.
                //
                // Phase H.2.b — reducer-aware iteration with fallback.
                let pane_outer_hwnds: Vec<*mut std::ffi::c_void> = {
                    let mut hwnds: Vec<*mut std::ffi::c_void> = self
                        .state
                        .list_browsers()
                        .into_iter()
                        .filter(|(k, _)| k.starts_with("browser-pane-"))
                        .filter_map(|(_, mut b)| {
                            b.host().and_then(|h| {
                                let wh = h.window_handle();
                                if wh.0.is_null() { None } else { Some(wh.0 as *mut std::ffi::c_void) }
                            })
                        })
                        .collect();
                    for h in crate::browser_pane::hwnd::pane_outer_hwnds_from_context() {
                        if !hwnds.contains(&h) {
                            hwnds.push(h);
                        }
                    }
                    hwnds
                };

                match views_top_hwnd {
                    Some(top_hwnd) => unsafe {
                        let render = find_main_render_widget(top_hwnd, &pane_outer_hwnds);
                        let target = render.unwrap_or(top_hwnd);
                        windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus(target as _);
                        crate::browser_pane::hwnd::record_intentional_focus(target);
                        tracing::info!(
                            "[main-focus-reclaim] Win32 SetFocus target={:p} render_found={} panes_excluded={}",
                            target,
                            render.is_some(),
                            pane_outer_hwnds.len(),
                        );
                    },
                    None => {
                        tracing::warn!(
                            "[main-focus-reclaim] could not resolve Views top-level HWND for label={}",
                            label,
                        );
                    }
                }
            }

            // Defocus all live panes at the Chromium level too.
            self.state.browser_panes.defocus_all(&self.state);
        }
    }
}

/// Walk descendants of `root` and return the first Chrome_RenderWidgetHostHWND
/// whose ancestor chain does NOT pass through any of `pane_outer_hwnds`.
/// Panes are siblings of main under the Views top-level, so without this
/// filter the walk would happily pick a pane's render widget.
#[cfg(target_os = "windows")]
unsafe fn find_main_render_widget(
    root: *mut std::ffi::c_void,
    pane_outer_hwnds: &[*mut std::ffi::c_void],
) -> Option<*mut std::ffi::c_void> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumChildWindows, GetClassNameW, GetParent,
    };

    struct Finder<'a> {
        found: *mut std::ffi::c_void,
        panes: &'a [*mut std::ffi::c_void],
    }
    let mut finder = Finder { found: std::ptr::null_mut(), panes: pane_outer_hwnds };

    unsafe extern "system" fn cb(hwnd: *mut std::ffi::c_void, lparam: isize) -> i32 {
        let finder = &mut *(lparam as *mut Finder);
        let mut buf = [0u16; 64];
        let n = GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        if n > 0 {
            let class = String::from_utf16_lossy(&buf[..n as usize]);
            if class == "Chrome_RenderWidgetHostHWND" {
                // Walk ancestors; if we pass through any pane outer HWND,
                // this widget belongs to a pane, not main.
                let mut descends_from_pane = false;
                let mut cursor = GetParent(hwnd);
                while !cursor.is_null() {
                    if finder.panes.iter().any(|p| *p == cursor) {
                        descends_from_pane = true;
                        break;
                    }
                    cursor = GetParent(cursor);
                }
                if !descends_from_pane {
                    finder.found = hwnd;
                    return 0; // stop
                }
            }
        }
        1
    }

    EnumChildWindows(root, Some(cb), &mut finder as *mut _ as isize);
    if finder.found.is_null() { None } else { Some(finder.found) }
}

// ── SPEC_LAUNCHER_TEARDOWN_BACKSTOP_2026_07_11 Phase 1 — UI-thread probe ───

wrap_task! {
    pub struct ProbeUiThreadReplyTask {
        nonce: u64,
    }

    impl Task {
        fn execute(&self) {
            // Executing at all IS the liveness evidence: this task only runs
            // when CEF's UI thread pumps its queue. The reply is sent from
            // here — the UI thread — per `report_ui_thread_alive`'s contract;
            // replying from any other thread would forge the signal.
            crate::launcher_ipc::report_ui_thread_alive(self.nonce);
        }
    }
}

/// SPEC_LAUNCHER_TEARDOWN_BACKSTOP Phase 1 — post the probe reply task.
/// Called from the launcher-ipc reader on `ProbeUiThread`. If the UI thread
/// is wedged — or not yet pumping (the known pre-ready `post_task` silent
/// drop) — the task never executes and no reply is ever sent; the
/// launcher's prober reads that silence as the signal. Deliberately no
/// fallback reply path.
pub fn post_probe_ui_thread_reply(nonce: u64) {
    let mut task = ProbeUiThreadReplyTask::new(nonce);
    post_task(ThreadId::UI, Some(&mut task));
}

// ── SPEC_LAUNCHER_TEARDOWN_BACKSTOP Phase 2 — debug wedge (verification) ──

wrap_task! {
    pub struct HangUiThreadTask {}

    impl Task {
        fn execute(&self) {
            // Park the UI thread. The message loop stops pumping: probe
            // reply tasks queue but never execute, so the launcher's
            // consecutive-miss counter climbs and — once the last user
            // window is closed and the machine arms — the backstop tears
            // the tree down. 1h is effectively forever for the test while
            // still self-recovering if the backstop somehow doesn't fire.
            tracing::warn!("[debug:hang_ui] UI thread parked (1h sleep) — the teardown backstop should reap this process");
            std::thread::sleep(std::time::Duration::from_secs(3600));
        }
    }
}

/// SPEC_LAUNCHER_TEARDOWN_BACKSTOP Phase 2 — verification-only UI-thread
/// wedge. Caller (`ipc.rs "debug:hang_ui"`) gates on `AGENTMUX_DEBUG_HANG=1`.
pub fn post_hang_ui_thread(_state: &Arc<AppState>) {
    let mut task = HangUiThreadTask::new();
    post_task(ThreadId::UI, Some(&mut task));
}
