// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Host-side orphan-instance reconciliation. Spec:
//! `docs/specs/SPEC_HOST_ORPHAN_RECONCILIATION_2026_05_05.md`.
//!
//! When the launcher detects that the last user-visible window
//! has closed but the host is still alive, it emits
//! `Event::HostShouldQuit`. The host's handler invokes
//! `reconcile_and_drain` here, which closes any orphan
//! `window-pool-*` browsers (promoted out of the warm pool, but
//! the launcher mirror has dropped them — typically because their
//! HWND was destroyed without the host's `on_before_close` running).
//!
//! Each close funnels back through `client::on_before_close`,
//! whose Stage 2 hook fires `quit_message_loop()` once
//! `browser_list` empties — so the reconciler doesn't drive
//! UI-thread shutdown directly.
//!
//! Threading: CEF Browser/BrowserHost methods (`host()`,
//! `window_handle()`, `close_browser()`) MUST run on the UI
//! thread per CEF docs. The IPC reader thread that delivers
//! `HostShouldQuit` does only state-snapshot + classification
//! work, then `cef::post_task`s the UI-thread closure.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use cef::*;

use crate::state::AppState;

/// HWND liveness state of a candidate browser. Inputs to the
/// planner; computed in production from real CEF/Win32 calls in
/// `hwnd_is_dead_or_missing` + the `host()` check, supplied
/// directly in tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HwndStatus {
    /// HWND is non-null AND `IsWindow` returns true. Live user window.
    Live,
    /// `BrowserHost` is gone (`browser.host()` returned None).
    Hostless,
    /// HWND is null OR `IsWindow` returns false. Zombie.
    Dead,
}

/// What the planner decided to do with a single label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum CloseAction {
    /// Call `host.close_browser(force=1)` — requires live BrowserHost.
    /// Used for zombies (Dead) and ready warm-pool / unpromoted-pool
    /// labels in drain mode.
    CloseBrowser,
    /// Dispatch `HostCommand::UnregisterBrowser` directly. Used for
    /// hostless zombies — there's no BrowserHost to call
    /// `close_browser` on, so we clean `state.browsers` ourselves and
    /// drive `quit_message_loop` from the orchestrator.
    UnregisterBrowser,
}

/// Output of the pure planning step. The orchestrator executes this
/// against real CEF state. Kept separate so tests can verify
/// decisions without standing up a CEF runtime.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct ReconcilePlan {
    /// Labels to act on, in deterministic order (zombies first, then
    /// drain-mode pool inventory). Each entry pairs a label with the
    /// action the orchestrator should take.
    pub closes: Vec<(String, CloseAction)>,
    /// Live HWND, not in `pool.queue` — Race B. Diagnostic only;
    /// these are intentionally NOT in `closes`.
    pub freshly_promoted: Vec<String>,
    /// Whether to dispatch `BeginDrain` before executing `closes`.
    /// Equivalent to `safe_to_drain`.
    pub begin_drain: bool,
}

/// Pure planning function. The orchestrator HWND-probes every
/// non-pane top-level browser in `state.browsers` and feeds that map
/// here.
///
/// `browser_status` keys every non-pane label in `state.browsers` to
/// its HwndStatus. Includes pool-prefixed labels AND regular
/// top-level windows (e.g. `main`, `window-N`). Pane labels
/// (`browser-pane-*`) are NOT in this map — they drain via a
/// separate cascade and would otherwise pollute classification.
///
/// `all_browser_labels` is `state.list_browsers()` keyed; used only
/// for `live_user_count`.
pub(crate) fn plan_reconcile(
    browser_status: &HashMap<String, HwndStatus>,
    all_browser_labels: &[String],
    shadow_keys: &HashSet<String>,
    pool_queue: &HashSet<String>,
    unpromoted_pool: &HashSet<String>,
    pending_creation_in_flight: bool,
) -> ReconcilePlan {
    // Live user count = labels in shadow, not pane-prefixed. Zombies
    // are absent from shadow (`apply_hwnd_destroyed` prunes
    // `state.windows`), so no subtraction needed.
    let live_user_count = all_browser_labels
        .iter()
        .filter(|label| {
            shadow_keys.contains(label.as_str()) && !label.starts_with("browser-pane-")
        })
        .count();

    // Classify each non-pane top-level browser. Pool-prefixed and
    // regular top-levels share the same shape:
    //   Hostless        → UnregisterBrowser, but ONLY when drain
    //                     fires (otherwise we'd bypass the pool
    //                     reducer's per-destroy bookkeeping).
    //   Dead            → CloseBrowser. on_before_close handles the
    //                     reducer cleanup.
    //   Live + shadow   → promoted user window, leave alone.
    //   Live + pool.queue / pool.unpromoted → drainable pool slot.
    //   Live + (none of above) → freshly opened/promoted, blocks
    //                     drain (Race B).
    let mut dead_zombies: Vec<String> = Vec::new();
    let mut hostless: Vec<String> = Vec::new();
    let mut drainable: Vec<String> = Vec::new();
    let mut freshly_promoted: Vec<String> = Vec::new();
    for (label, status) in browser_status {
        match status {
            HwndStatus::Dead => dead_zombies.push(label.clone()),
            HwndStatus::Hostless => hostless.push(label.clone()),
            HwndStatus::Live => {
                if shadow_keys.contains(label.as_str()) {
                    // promoted user window
                } else if pool_queue.contains(label) || unpromoted_pool.contains(label) {
                    drainable.push(label.clone());
                } else {
                    freshly_promoted.push(label.clone());
                }
            }
        }
    }

    // `pending_creation_in_flight` blocks drain too: a stale
    // `HostShouldQuit` racing with `open_window_with_kind` (which
    // enqueues `PendingWindowCreation` BEFORE `post_create_window`
    // registers the browser) would otherwise close the warm pool
    // and drive Stage-2 quit before the new window is registered,
    // dropping it.
    let safe_to_drain = live_user_count == 0
        && freshly_promoted.is_empty()
        && !pending_creation_in_flight;

    // Sort for determinism (HashMap iteration order is unspecified).
    dead_zombies.sort();
    hostless.sort();
    drainable.sort();
    freshly_promoted.sort();

    let mut closes: Vec<(String, CloseAction)> = Vec::new();
    // Dead zombies always close — `close_browser(force=1)` triggers
    // the host's own on_before_close cleanup chain regardless of
    // drain state, so this is safe whether we're shutting down or
    // recovering from a single-window crash mid-session.
    for label in dead_zombies {
        closes.push((label, CloseAction::CloseBrowser));
    }
    if safe_to_drain {
        // Hostless gets UnregisterBrowser ONLY in drain mode.
        // Outside drain, dispatching UnregisterBrowser bypasses
        // `on_pool_window_destroyed` cleanup, leaving pool reducer
        // state with a stale label that's no longer in `browsers`.
        // The hostless state is already a CEF lifecycle anomaly;
        // letting it persist until the next drain is preferable to
        // creating per-handler-state drift.
        for label in hostless {
            closes.push((label, CloseAction::UnregisterBrowser));
        }
        for label in drainable {
            closes.push((label, CloseAction::CloseBrowser));
        }
    }

    ReconcilePlan {
        closes,
        freshly_promoted,
        begin_drain: safe_to_drain,
    }
}

/// IPC-thread entry point. Posts a UI-thread task that does all
/// state-snapshot + classification + close work. The IPC thread no
/// longer pre-classifies candidates — between IPC and UI execution,
/// labels can move between `pool.unpromoted` / `pool.queue` /
/// promoted-into-shadow. Re-snapshotting on the UI thread avoids
/// stale classification.
pub fn reconcile_and_drain(state: &Arc<AppState>) {
    // Marshal CEF Browser/BrowserHost calls to the UI thread. The
    // existing two-stage cascade (`client/mod.rs::on_before_close`)
    // gets to make these calls inline because it already runs on
    // the UI thread; we don't, so we hand the work to CEF's task
    // queue. Three v0.33.491–v0.33.494 attempts at driving UI work
    // *directly* from the IPC handler all hung CEF — this path
    // avoids that by using CEF's own scheduler.
    let mut task = OrphanReconcileTask::new(state.clone());
    let posted = post_task(ThreadId::UI, Some(&mut task));
    tracing::debug!(
        target: "wrr-trace",
        "[orphan-reconcile] posted UI-thread task posted={}",
        posted != 0
    );
}

wrap_task! {
    pub struct OrphanReconcileTask {
        state: Arc<AppState>,
    }

    impl Task {
        fn execute(&self) {
            ui_thread_reconcile(&self.state);
        }
    }
}

/// UI-thread body. CEF Browser methods are safe here.
///
/// Re-snapshot browsers (state may have advanced since the IPC
/// thread classified candidates — labels that have actually closed
/// are no longer in the map; new candidates may have appeared, but
/// we'll catch them on the next `HostShouldQuit`). For each
/// candidate that's still present, probe its HWND: live → skip
/// (Race B, freshly promoted), dead → close.
///
/// Drain mode is set ONLY if no live user browser remains *after*
/// removing the zombies we're about to close. A stale
/// `HostShouldQuit` racing with a live user session must NOT flip
/// `quit_state` to `Draining`, because there's no transition back
/// to `Running` and `spawn_pool_window` then refuses to refill the
/// pool — silently degrading the live session.
fn ui_thread_reconcile(state: &Arc<AppState>) {
    let browser_pairs = state.list_browsers();
    let shadow_keys: HashSet<String> = state
        .shadow_window_meta
        .lock()
        .keys()
        .cloned()
        .collect();
    let pool_queue: HashSet<String> = state
        .host_state
        .lock()
        .pool
        .queue
        .iter()
        .cloned()
        .collect();
    let unpromoted_pool: HashSet<String> = state.unpromoted_pool_labels_snapshot();

    // Probe HWND status for every non-pane top-level browser. Panes
    // drain via a separate cascade and would otherwise pollute
    // classification. Includes pool-prefixed labels AND regular
    // top-level windows (`main`, `window-N`) — non-pool zombies
    // need to be reaped just like pool zombies do.
    let label_to_browser: HashMap<String, Browser> = browser_pairs
        .iter()
        .filter(|(l, _)| !l.starts_with("browser-pane-"))
        .map(|(l, b)| (l.clone(), b.clone()))
        .collect();
    let browser_status: HashMap<String, HwndStatus> = label_to_browser
        .iter()
        .map(|(label, browser)| (label.clone(), classify_hwnd(browser)))
        .collect();

    let all_labels: Vec<String> = browser_pairs.iter().map(|(l, _)| l.clone()).collect();
    let pending_creation_in_flight = !state
        .host_state
        .lock()
        .pending_window_creations
        .is_empty();
    let plan = plan_reconcile(
        &browser_status,
        &all_labels,
        &shadow_keys,
        &pool_queue,
        &unpromoted_pool,
        pending_creation_in_flight,
    );

    if !plan.freshly_promoted.is_empty() {
        tracing::info!(
            target: "wrr",
            "[orphan-reconcile] {} candidate(s) appear freshly-promoted (live HWND, not in pool queue), skipping: {:?}",
            plan.freshly_promoted.len(),
            plan.freshly_promoted
        );
    }

    if plan.closes.is_empty() {
        if plan.begin_drain {
            // Drain requested AND nothing for us to do (state.browsers
            // is empty). Stage-2 quit may be blocked by stale
            // `client::browser_list` entries we can't see from here.
            // Drive quit ourselves. The pending-creation gate is
            // already enforced by `plan.begin_drain`.
            tracing::warn!(
                target: "wrr",
                "[orphan-reconcile] nothing to close but drain requested — driving quit_message_loop"
            );
            quit_message_loop();
        } else {
            tracing::info!(
                target: "wrr",
                "[orphan-reconcile] nothing to close, drain not requested — host has live work or pending creation"
            );
        }
        return;
    }

    if plan.begin_drain {
        state.host_dispatch(crate::reducer::HostCommand::BeginDrain {
            reason: crate::state::QuitReason::LastWindowClosed,
        });
    } else {
        tracing::info!(
            target: "wrr",
            "[orphan-reconcile] skipping BeginDrain — live user windows or freshly-promoted candidates remain"
        );
    }

    tracing::warn!(
        target: "wrr",
        "[orphan-reconcile] executing {} close action(s): {:?}",
        plan.closes.len(),
        plan.closes
    );

    let mut any_hostless = false;
    for (i, (label, action)) in plan.closes.iter().enumerate() {
        match action {
            CloseAction::CloseBrowser => {
                if let Some(browser) = label_to_browser.get(label).cloned() {
                    let late_hostless =
                        drive_close_browser(state, i, label, browser, plan.begin_drain);
                    if late_hostless {
                        any_hostless = true;
                    }
                } else {
                    tracing::warn!(
                        target: "wrr",
                        "[orphan-reconcile][{}] label={} vanished before close",
                        i, label
                    );
                }
            }
            CloseAction::UnregisterBrowser => {
                drive_unregister(state, i, label);
                any_hostless = true;
            }
        }
    }

    // Hostless orphans don't get `on_before_close` callbacks. The
    // host's own Stage-2 quit gates on `client::browser_list.is_empty()`
    // — but UnregisterBrowser only touches the reducer's `browsers`
    // map, not CefClient's internal list, so a stale Hostless entry
    // there permanently blocks Stage 2.
    //
    // Drive `quit_message_loop` ourselves whenever drain ran AND any
    // Hostless cleanup happened. Closables we dispatched alongside
    // are mid-close at this point; their `on_before_close` may not
    // run before the loop terminates, but we're shutting down anyway
    // and OS process exit reclaims their resources. Gated on
    // `begin_drain` so a stale `HostShouldQuit` racing with a live
    // session can't terminate it.
    if any_hostless && plan.begin_drain {
        tracing::warn!(
            target: "wrr",
            "[orphan-reconcile] hostless orphans unregistered in drain mode — driving quit_message_loop"
        );
        quit_message_loop();
    }
}

/// Map a Browser to its `HwndStatus`. Calls CEF + Win32; only safe
/// from the UI thread. Used by the orchestrator to build the input
/// to `plan_reconcile`.
fn classify_hwnd(browser: &Browser) -> HwndStatus {
    let mut b = browser.clone();
    let Some(host) = b.host() else { return HwndStatus::Hostless };
    let wh = host.window_handle();
    if wh.0.is_null() {
        return HwndStatus::Dead;
    }
    #[cfg(target_os = "windows")]
    unsafe {
        use windows_sys::Win32::Foundation::HWND;
        use windows_sys::Win32::UI::WindowsAndMessaging::IsWindow;
        if IsWindow(wh.0 as HWND) == 0 {
            HwndStatus::Dead
        } else {
            HwndStatus::Live
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        HwndStatus::Live
    }
}

/// Execute a `CloseBrowser` action. Already on UI thread.
/// `host.close_browser(force=1)` works regardless of HWND state and
/// triggers the host's `on_before_close` callback chain (which
/// dispatches `UnregisterBrowser` and drives Stage-2 quit naturally).
/// Returns `true` iff the browser's host had vanished by execute
/// time and the orchestrator should treat this as a late hostless
/// transition (relevant for the Stage-2 quit drive — same reasoning
/// as the planner's Hostless bucket).
fn drive_close_browser(
    state: &Arc<AppState>,
    idx: usize,
    label: &str,
    mut browser: Browser,
    drain_mode: bool,
) -> bool {
    if let Some(host) = browser.host() {
        host.close_browser(1);
        tracing::debug!(
            target: "wrr-trace",
            "[orphan-reconcile][{}] close_browser(force=1) label={}",
            idx, label
        );
        false
    } else {
        // Race: HWND status was Live/Dead at planning, but
        // BrowserHost vanished before we got here. Mirror the
        // planner's Hostless path — but only fall through to
        // UnregisterBrowser if drain is active (same gating as the
        // Hostless bucket — outside drain, dispatching
        // UnregisterBrowser bypasses on_pool_window_destroyed
        // bookkeeping and creates pool-reducer drift).
        if drain_mode {
            tracing::warn!(
                target: "wrr",
                "[orphan-reconcile][{}] browser host=None at execute time label={} — late hostless transition; dispatching UnregisterBrowser",
                idx, label
            );
            state.host_dispatch(crate::reducer::HostCommand::UnregisterBrowser {
                label: label.to_string(),
            });
            true
        } else {
            tracing::warn!(
                target: "wrr",
                "[orphan-reconcile][{}] browser host=None at execute time label={} — late hostless transition; deferring UnregisterBrowser (drain not active)",
                idx, label
            );
            false
        }
    }
}

/// Execute an `UnregisterBrowser` action. Hostless candidates can't
/// be `close_browser`'d (no `BrowserHost` to call it on), so we
/// clean `state.browsers` ourselves. Caller drives `quit_message_loop`
/// after the loop if any unregister fired.
fn drive_unregister(state: &Arc<AppState>, idx: usize, label: &str) {
    tracing::warn!(
        target: "wrr",
        "[orphan-reconcile][{}] hostless label={} — dispatching UnregisterBrowser",
        idx, label
    );
    state.host_dispatch(crate::reducer::HostCommand::UnregisterBrowser {
        label: label.to_string(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set_of(items: &[&str]) -> HashSet<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    fn vec_of(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    // ── plan_reconcile integration tests ──────────────────────────
    //
    // Cover the state cross-product the orchestrator must handle:
    //
    //   axes: HwndStatus (Live / Dead / Hostless),
    //         in shadow_window_meta (yes / no),
    //         in pool.queue (yes / no),
    //         label prefix (window-pool-* / browser-pane-* / other),
    //         and the resulting (live_user_count, freshly_promoted)
    //         derivation that drives `safe_to_drain`.
    //
    // Each test names the spec scenario (Race A/B/C/D) it exercises.

    fn map_of(items: &[(&str, HwndStatus)]) -> HashMap<String, HwndStatus> {
        items.iter().map(|(s, st)| (s.to_string(), *st)).collect()
    }

    #[test]
    fn plan_no_browsers_is_empty_drain_true() {
        let plan = plan_reconcile(
            &map_of(&[]),
            &vec_of(&[]),
            &set_of(&[]),
            &set_of(&[]),
            &set_of(&[]),
            false,
        );
        assert!(plan.closes.is_empty());
        assert!(plan.freshly_promoted.is_empty());
        assert!(plan.begin_drain);
    }

    #[test]
    fn plan_single_dead_zombie_drains_and_closes() {
        let label = "window-pool-zombie";
        let plan = plan_reconcile(
            &map_of(&[(label, HwndStatus::Dead)]),
            &vec_of(&[label]),
            &set_of(&[]),
            &set_of(&[]),
            &set_of(&[]),
            false,
        );
        assert_eq!(plan.closes, vec![(label.to_string(), CloseAction::CloseBrowser)]);
        assert!(plan.freshly_promoted.is_empty());
        assert!(plan.begin_drain);
    }

    #[test]
    fn plan_hostless_zombie_unregisters_and_drains() {
        let label = "window-pool-hostless";
        let plan = plan_reconcile(
            &map_of(&[(label, HwndStatus::Hostless)]),
            &vec_of(&[label]),
            &set_of(&[]),
            &set_of(&[]),
            &set_of(&[]),
            false,
        );
        assert_eq!(
            plan.closes,
            vec![(label.to_string(), CloseAction::UnregisterBrowser)]
        );
        assert!(plan.begin_drain);
    }

    #[test]
    fn plan_freshly_promoted_blocks_drain_and_skips_close() {
        // Race B: live HWND, NOT in pool.queue, NOT in unpromoted,
        // NOT in shadow. Pre-echo-promotion. Must NOT be closed AND
        // must block drain.
        let label = "window-pool-just-promoted";
        let plan = plan_reconcile(
            &map_of(&[(label, HwndStatus::Live)]),
            &vec_of(&[label]),
            &set_of(&[]),
            &set_of(&[]),
            &set_of(&[]),
            false,
        );
        assert!(plan.closes.is_empty(), "freshly promoted must not be closed: {:?}", plan.closes);
        assert_eq!(plan.freshly_promoted, vec![label.to_string()]);
        assert!(!plan.begin_drain);
    }

    #[test]
    fn plan_ready_warm_pool_drains() {
        // Race D: live HWND IN pool.queue. Common shutdown path.
        let label = "window-pool-ready";
        let plan = plan_reconcile(
            &map_of(&[(label, HwndStatus::Live)]),
            &vec_of(&[label]),
            &set_of(&[]),
            &set_of(&[label]),
            &set_of(&[]),
            false,
        );
        assert_eq!(plan.closes, vec![(label.to_string(), CloseAction::CloseBrowser)]);
        assert!(plan.freshly_promoted.is_empty());
        assert!(plan.begin_drain);
    }

    #[test]
    fn plan_unpromoted_pool_drains() {
        // Spawning pool slot — live HWND, IN unpromoted_pool.
        // Drain branch must close it so Stage 2 sees empty browser_list.
        let label = "window-pool-spawning";
        let plan = plan_reconcile(
            &map_of(&[(label, HwndStatus::Live)]),
            &vec_of(&[label]),
            &set_of(&[]),
            &set_of(&[]),
            &set_of(&[label]),
            false,
        );
        assert_eq!(plan.closes, vec![(label.to_string(), CloseAction::CloseBrowser)]);
        assert!(plan.begin_drain);
    }

    #[test]
    fn plan_live_user_window_blocks_drain() {
        let user_label = "window-pool-promoted-user";
        let zombie_label = "window-pool-zombie";
        let plan = plan_reconcile(
            &map_of(&[
                (user_label, HwndStatus::Live),
                (zombie_label, HwndStatus::Dead),
            ]),
            &vec_of(&[user_label, zombie_label]),
            &set_of(&[user_label]),
            &set_of(&[]),
            &set_of(&[]),
            false,
        );
        assert_eq!(
            plan.closes,
            vec![(zombie_label.to_string(), CloseAction::CloseBrowser)]
        );
        assert!(!plan.begin_drain);
    }

    #[test]
    fn plan_zombie_plus_freshly_promoted_skips_drain() {
        // Zombie always closes; freshly_promoted blocks drain.
        let zombie = "window-pool-zombie";
        let promoted = "window-pool-promoted";
        let plan = plan_reconcile(
            &map_of(&[
                (zombie, HwndStatus::Dead),
                (promoted, HwndStatus::Live),
            ]),
            &vec_of(&[zombie, promoted]),
            &set_of(&[]),
            &set_of(&[]),
            &set_of(&[]),
            false,
        );
        assert_eq!(
            plan.closes,
            vec![(zombie.to_string(), CloseAction::CloseBrowser)]
        );
        assert_eq!(plan.freshly_promoted, vec![promoted.to_string()]);
        assert!(!plan.begin_drain);
    }

    #[test]
    fn plan_zombie_plus_ready_pool_drains_both() {
        let zombie = "window-pool-zombie";
        let ready = "window-pool-ready";
        let plan = plan_reconcile(
            &map_of(&[
                (zombie, HwndStatus::Dead),
                (ready, HwndStatus::Live),
            ]),
            &vec_of(&[zombie, ready]),
            &set_of(&[]),
            &set_of(&[ready]),
            &set_of(&[]),
            false,
        );
        let close_labels: Vec<&str> = plan.closes.iter().map(|(l, _)| l.as_str()).collect();
        assert!(close_labels.contains(&zombie));
        assert!(close_labels.contains(&ready));
        assert_eq!(plan.closes.len(), 2);
        assert!(plan.begin_drain);
    }

    #[test]
    fn plan_promoted_pool_window_in_shadow_is_left_alone() {
        // A `window-pool-*` label that's in shadow is a promoted user
        // window (kept its prefix). Live HWND. Must NOT be closed
        // and DOES count toward live_user_count.
        let promoted = "window-pool-active";
        let plan = plan_reconcile(
            &map_of(&[(promoted, HwndStatus::Live)]),
            &vec_of(&[promoted]),
            &set_of(&[promoted]),
            &set_of(&[]),
            &set_of(&[]),
            false,
        );
        assert!(plan.closes.is_empty());
        assert!(plan.freshly_promoted.is_empty());
        assert!(!plan.begin_drain, "promoted pool window must keep host alive");
    }

    #[test]
    fn plan_browser_pane_labels_dont_count_toward_live_user() {
        let pane = "browser-pane-foo";
        let zombie = "window-pool-zombie";
        let plan = plan_reconcile(
            &map_of(&[(zombie, HwndStatus::Dead)]),
            &vec_of(&[pane, zombie]),
            &set_of(&[pane]),
            &set_of(&[]),
            &set_of(&[]),
            false,
        );
        assert!(plan.begin_drain);
        let close_labels: Vec<&str> = plan.closes.iter().map(|(l, _)| l.as_str()).collect();
        assert!(close_labels.contains(&zombie));
    }

    #[test]
    fn plan_v0_33_643_reproduction() {
        let z1 = "window-pool-722b6186bb6e42378b48b7068c0d54b0";
        let z2 = "window-pool-b4e20337180247bdbd7408ddd7754b78";
        let plan = plan_reconcile(
            &map_of(&[(z1, HwndStatus::Dead), (z2, HwndStatus::Dead)]),
            &vec_of(&[z1, z2]),
            &set_of(&[]),
            &set_of(&[]),
            &set_of(&[]),
            false,
        );
        assert_eq!(plan.closes.len(), 2);
        for (label, action) in &plan.closes {
            assert!(label == z1 || label == z2);
            assert_eq!(*action, CloseAction::CloseBrowser);
        }
        assert!(plan.begin_drain);
    }

    #[test]
    fn plan_hostless_unpromoted_pool_unregisters_not_closes() {
        // Hostless takes precedence over pool-state classification:
        // an unpromoted-pool slot that lost its BrowserHost still
        // needs UnregisterBrowser, not CloseBrowser.
        let label = "window-pool-hostless-unpromoted";
        let plan = plan_reconcile(
            &map_of(&[(label, HwndStatus::Hostless)]),
            &vec_of(&[label]),
            &set_of(&[]),
            &set_of(&[]),
            &set_of(&[label]),
            false,
        );
        assert_eq!(
            plan.closes,
            vec![(label.to_string(), CloseAction::UnregisterBrowser)]
        );
        assert!(plan.begin_drain);
    }

    #[test]
    fn plan_mixed_full_state_space() {
        // Composite touching every bucket: zombie + hostless +
        // unpromoted pool + ready pool + freshly_promoted + promoted
        // user window + pane.
        let zombie = "window-pool-z";
        let hostless = "window-pool-h";
        let unpromoted = "window-pool-u";
        let ready = "window-pool-r";
        let fresh = "window-pool-f";
        let promoted = "window-pool-p";
        let pane = "browser-pane-x";
        let plan = plan_reconcile(
            &map_of(&[
                (zombie, HwndStatus::Dead),
                (hostless, HwndStatus::Hostless),
                (unpromoted, HwndStatus::Live),
                (ready, HwndStatus::Live),
                (fresh, HwndStatus::Live),
                (promoted, HwndStatus::Live),
                // pane not in pool_browser_status — it's not pool-prefixed
            ]),
            &vec_of(&[zombie, hostless, unpromoted, ready, fresh, promoted, pane]),
            &set_of(&[promoted]),
            &set_of(&[ready]),
            &set_of(&[unpromoted]),
            false,
        );
        // freshly_promoted blocks drain → only Dead zombies close.
        // Hostless is gated on safe_to_drain (outside drain,
        // dispatching UnregisterBrowser bypasses pool-reducer
        // bookkeeping; it waits for the next reconcile that can
        // drain).
        assert_eq!(plan.freshly_promoted, vec![fresh.to_string()]);
        assert!(!plan.begin_drain);
        let actions: HashMap<String, CloseAction> = plan
            .closes
            .iter()
            .map(|(l, a)| (l.clone(), a.clone()))
            .collect();
        assert_eq!(actions.get(zombie), Some(&CloseAction::CloseBrowser));
        assert!(!actions.contains_key(hostless), "hostless waits for drain mode");
        assert!(!actions.contains_key(unpromoted), "unpromoted spared when drain blocked");
        assert!(!actions.contains_key(ready), "ready spared when drain blocked");
        assert!(!actions.contains_key(fresh), "freshly_promoted never closes");
        assert!(!actions.contains_key(promoted), "promoted user window never closes");
        assert!(!actions.contains_key(pane), "pane never in plan");
    }

    #[test]
    fn plan_non_pool_zombie_closes_too() {
        // A regular `main`/`window-X` top-level can crash. The
        // launcher's apply_hwnd_destroyed emits HostShouldQuit; the
        // reconciler must reap that zombie too, not just
        // `window-pool-*` ones.
        let main = "main";
        let plan = plan_reconcile(
            &map_of(&[(main, HwndStatus::Dead)]),
            &vec_of(&[main]),
            &set_of(&[]),
            &set_of(&[]),
            &set_of(&[]),
            false,
        );
        assert_eq!(plan.closes, vec![(main.to_string(), CloseAction::CloseBrowser)]);
        assert!(plan.begin_drain);
    }

    #[test]
    fn plan_hostless_skipped_when_drain_blocked() {
        // Hostless cleanup outside drain mode bypasses
        // on_pool_window_destroyed and creates pool-reducer drift.
        // When a live user window is present, the hostless entry
        // must NOT be unregistered — wait for the next reconcile
        // that can drain.
        let user = "main";
        let hostless = "window-pool-hostless";
        let plan = plan_reconcile(
            &map_of(&[
                (user, HwndStatus::Live),
                (hostless, HwndStatus::Hostless),
            ]),
            &vec_of(&[user, hostless]),
            &set_of(&[user]),
            &set_of(&[]),
            &set_of(&[]),
            false,
        );
        assert!(!plan.begin_drain);
        assert!(plan.closes.is_empty(), "hostless must not close while drain blocked: {:?}", plan.closes);
    }

    #[test]
    fn plan_dead_zombie_closes_even_when_drain_blocked() {
        // Counterpart to the test above: Dead zombies always close,
        // because close_browser(force=1) drives the host's own
        // cleanup chain (on_before_close → on_pool_window_destroyed →
        // UnregisterBrowser). Drain state is irrelevant.
        let user = "main";
        let zombie = "window-pool-zombie";
        let plan = plan_reconcile(
            &map_of(&[
                (user, HwndStatus::Live),
                (zombie, HwndStatus::Dead),
            ]),
            &vec_of(&[user, zombie]),
            &set_of(&[user]),
            &set_of(&[]),
            &set_of(&[]),
            false,
        );
        assert!(!plan.begin_drain);
        assert_eq!(
            plan.closes,
            vec![(zombie.to_string(), CloseAction::CloseBrowser)]
        );
    }

    #[test]
    fn plan_mixed_hostless_and_closable_in_drain() {
        // When a drain plan contains both Hostless AND CloseBrowser
        // entries, the orchestrator must still drive
        // quit_message_loop. The closables' on_before_close would
        // normally satisfy Stage-2 quit, but the Hostless entries
        // leave stale references in `client::browser_list`
        // (UnregisterBrowser doesn't touch that), so Stage 2 never
        // fires. The reconciler drives quit itself.
        //
        // Plan-level assertion: both actions are scheduled, drain
        // fires. The quit_message_loop drive itself is exercised by
        // the orchestrator and gated on `begin_drain && any_hostless`.
        let dead = "window-pool-dead";
        let hostless = "window-pool-hostless";
        let plan = plan_reconcile(
            &map_of(&[
                (dead, HwndStatus::Dead),
                (hostless, HwndStatus::Hostless),
            ]),
            &vec_of(&[dead, hostless]),
            &set_of(&[]),
            &set_of(&[]),
            &set_of(&[]),
            false,
        );
        assert!(plan.begin_drain);
        let actions: HashMap<String, CloseAction> = plan
            .closes
            .iter()
            .map(|(l, a)| (l.clone(), a.clone()))
            .collect();
        assert_eq!(actions.get(dead), Some(&CloseAction::CloseBrowser));
        assert_eq!(actions.get(hostless), Some(&CloseAction::UnregisterBrowser));
    }

    #[test]
    fn plan_pending_window_creation_blocks_drain() {
        // A stale HostShouldQuit racing with `open_window_with_kind`
        // can land in the gap between `PendingWindowCreation`
        // enqueue and `post_create_window` registering the browser.
        // In that gap the browser doesn't appear in any state, but
        // a creation is in flight. Drain MUST be deferred — closing
        // the warm pool here would let Stage-2 quit fire before the
        // new browser registers, dropping it.
        let ready = "window-pool-ready";
        let plan = plan_reconcile(
            &map_of(&[(ready, HwndStatus::Live)]),
            &vec_of(&[ready]),
            &set_of(&[]),
            &set_of(&[ready]),
            &set_of(&[]),
            true, // pending creation in flight
        );
        assert!(!plan.begin_drain, "pending creation must block drain");
        assert!(plan.closes.is_empty(), "ready warm pool must not close while creation pending: {:?}", plan.closes);
    }

    #[test]
    fn plan_pending_creation_doesnt_block_zombie_close() {
        // Dead zombies always close — they don't depend on drain
        // being safe. A pending creation should not stop us from
        // reaping a zombie.
        let zombie = "window-pool-zombie";
        let plan = plan_reconcile(
            &map_of(&[(zombie, HwndStatus::Dead)]),
            &vec_of(&[zombie]),
            &set_of(&[]),
            &set_of(&[]),
            &set_of(&[]),
            true,
        );
        assert!(!plan.begin_drain);
        assert_eq!(plan.closes, vec![(zombie.to_string(), CloseAction::CloseBrowser)]);
    }

    #[test]
    fn plan_idempotent_under_repeat() {
        let label = "window-pool-zombie";
        let inputs = (
            map_of(&[(label, HwndStatus::Dead)]),
            vec_of(&[label]),
            set_of(&[]),
            set_of(&[]),
            set_of(&[]),
        );
        let p1 = plan_reconcile(&inputs.0, &inputs.1, &inputs.2, &inputs.3, &inputs.4, false);
        let p2 = plan_reconcile(&inputs.0, &inputs.1, &inputs.2, &inputs.3, &inputs.4, false);
        assert_eq!(p1, p2);
    }

}
