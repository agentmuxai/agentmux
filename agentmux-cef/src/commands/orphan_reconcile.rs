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
/// `window-pool-*` label in `state.browsers` (NOT just classifier
/// candidates — see codex round-15/16 findings about non-candidate
/// hostless labels and IPC→UI race) and feeds that map here.
///
/// The planner classifies each label by intersecting its HwndStatus
/// with shadow / pool.queue / pool.unpromoted membership, then
/// decides drain + close set. No CEF, no Win32, no dispatch.
///
/// `pool_browser_status` keys EVERY pool-prefixed label in
/// `state.browsers` to its HwndStatus. Tests construct this map
/// directly; production builds it from `classify_hwnd`.
///
/// `all_browser_labels` is `state.list_browsers()` keyed; used to
/// compute `live_user_count` (which spans non-pool labels too —
/// regular `main` windows, etc.).
pub(crate) fn plan_reconcile(
    pool_browser_status: &HashMap<String, HwndStatus>,
    all_browser_labels: &[String],
    shadow_keys: &HashSet<String>,
    pool_queue: &HashSet<String>,
    unpromoted_pool: &HashSet<String>,
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

    // Walk every pool-prefixed browser. Each one falls into exactly
    // one bucket:
    //   Hostless        → UnregisterBrowser unconditionally
    //   Dead            → CloseBrowser unconditionally
    //   Live + shadow   → promoted user window, leave alone
    //   Live + queue    → ready warm pool, drainable
    //   Live + unpromoted → spawning pool slot, drainable
    //   Live + (none)   → freshly promoted, blocks drain
    let mut zombies_or_hostless: Vec<(String, CloseAction)> = Vec::new();
    let mut drainable: Vec<String> = Vec::new();
    let mut freshly_promoted: Vec<String> = Vec::new();
    for (label, status) in pool_browser_status {
        match status {
            HwndStatus::Dead => {
                zombies_or_hostless.push((label.clone(), CloseAction::CloseBrowser));
            }
            HwndStatus::Hostless => {
                zombies_or_hostless.push((label.clone(), CloseAction::UnregisterBrowser));
            }
            HwndStatus::Live => {
                if shadow_keys.contains(label.as_str()) {
                    // Promoted user window — leave alone.
                } else if pool_queue.contains(label) || unpromoted_pool.contains(label) {
                    drainable.push(label.clone());
                } else {
                    // Live HWND, not in shadow, not in pool — must be
                    // a freshly-promoted window whose ReportWindowOpened
                    // hasn't echoed yet (Race B).
                    freshly_promoted.push(label.clone());
                }
            }
        }
    }

    let safe_to_drain = live_user_count == 0 && freshly_promoted.is_empty();

    // Sort for determinism (HashMap iteration order is unspecified).
    zombies_or_hostless.sort_by(|a, b| a.0.cmp(&b.0));
    drainable.sort();
    freshly_promoted.sort();

    let mut closes = zombies_or_hostless;
    if safe_to_drain {
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

/// Classify which labels are *candidate* orphans — promoted
/// `window-pool-*` entries not tracked by the launcher's
/// `shadow_window_meta` mirror. Necessary but not sufficient: a
/// freshly-promoted pool window briefly satisfies this (the
/// launcher's `WindowOpened` echo hasn't populated shadow yet),
/// so the UI-thread runner must additionally verify the HWND is
/// dead before dispatching close.
///
/// Pure function over snapshot inputs.
pub(crate) fn classify_candidate_orphans(
    browser_labels: &[String],
    unpromoted_pool: &HashSet<String>,
    shadow_window_meta_keys: &HashSet<String>,
) -> Vec<String> {
    browser_labels
        .iter()
        .filter(|label| {
            label.starts_with("window-pool-")
                && !unpromoted_pool.contains(label.as_str())
                && !shadow_window_meta_keys.contains(label.as_str())
        })
        .cloned()
        .collect()
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

    // Probe HWND status for EVERY `window-pool-*` browser, not just
    // classifier candidates. The classifier excluded labels in
    // unpromoted_pool by design (they're not orphan candidates), but
    // the orchestrator still needs to drain them in shutdown — and
    // they may also be hostless and need UnregisterBrowser. Same
    // applies to labels promoted between the IPC-thread classify and
    // this UI task running.
    let label_to_browser: HashMap<String, Browser> = browser_pairs
        .iter()
        .filter(|(l, _)| l.starts_with("window-pool-"))
        .map(|(l, b)| (l.clone(), b.clone()))
        .collect();
    let pool_browser_status: HashMap<String, HwndStatus> = label_to_browser
        .iter()
        .map(|(label, browser)| (label.clone(), classify_hwnd(browser)))
        .collect();

    let all_labels: Vec<String> = browser_pairs.iter().map(|(l, _)| l.clone()).collect();
    let plan = plan_reconcile(
        &pool_browser_status,
        &all_labels,
        &shadow_keys,
        &pool_queue,
        &unpromoted_pool,
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
        tracing::info!(
            target: "wrr",
            "[orphan-reconcile] nothing to close after classification (begin_drain={})",
            plan.begin_drain
        );
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
                    drive_close_browser(i, label, browser);
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

    // Hostless orphans don't get an `on_before_close` callback to
    // drive Stage-2 `quit_message_loop`. If any unregister fired AND
    // we entered drain AND the reducer's registry is now empty,
    // drive the quit ourselves. Gated on `begin_drain` so a stale
    // `HostShouldQuit` racing with a live session can't terminate
    // it. Same UI thread Stage 2 calls it from.
    if any_hostless
        && plan.begin_drain
        && state.host_state.lock().browsers.is_empty()
    {
        tracing::warn!(
            target: "wrr",
            "[orphan-reconcile] hostless orphans unregistered + browsers map empty — driving quit_message_loop"
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
fn drive_close_browser(idx: usize, label: &str, mut browser: Browser) {
    if let Some(host) = browser.host() {
        host.close_browser(1);
        tracing::debug!(
            target: "wrr-trace",
            "[orphan-reconcile][{}] close_browser(force=1) label={}",
            idx, label
        );
    } else {
        // Edge case: HWND status was Live/Dead at planning but
        // BrowserHost vanished before we got here. Fall through to
        // the same UnregisterBrowser path; the planner will catch
        // this on the next reconcile if it happens.
        tracing::warn!(
            target: "wrr",
            "[orphan-reconcile][{}] browser host=None at execute time label={} — late hostless transition",
            idx, label
        );
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

    #[test]
    fn classify_no_orphans_returns_empty() {
        let labels = vec_of(&["window-pool-aaa"]);
        let unpromoted = set_of(&[]);
        let shadow = set_of(&["window-pool-aaa"]);
        assert_eq!(classify_candidate_orphans(&labels, &unpromoted, &shadow), Vec::<String>::new());
    }

    #[test]
    fn classify_one_orphan_is_returned() {
        let labels = vec_of(&["window-pool-bbb"]);
        let unpromoted = set_of(&[]);
        let shadow = set_of(&[]);
        assert_eq!(
            classify_candidate_orphans(&labels, &unpromoted, &shadow),
            vec_of(&["window-pool-bbb"])
        );
    }

    #[test]
    fn classify_skips_unpromoted_pool_members() {
        let labels = vec_of(&["window-pool-ccc"]);
        let unpromoted = set_of(&["window-pool-ccc"]);
        let shadow = set_of(&[]);
        assert_eq!(classify_candidate_orphans(&labels, &unpromoted, &shadow), Vec::<String>::new());
    }

    #[test]
    fn classify_skips_browser_pane_labels() {
        let labels = vec_of(&["browser-pane-foo"]);
        let unpromoted = set_of(&[]);
        let shadow = set_of(&[]);
        assert_eq!(classify_candidate_orphans(&labels, &unpromoted, &shadow), Vec::<String>::new());
    }

    #[test]
    fn classify_skips_main_label() {
        let labels = vec_of(&["main"]);
        let unpromoted = set_of(&[]);
        let shadow = set_of(&[]);
        assert_eq!(classify_candidate_orphans(&labels, &unpromoted, &shadow), Vec::<String>::new());
    }

    #[test]
    fn classify_returns_multiple_orphans_in_input_order() {
        let labels = vec_of(&[
            "window-pool-aaa", // shadow tracked → not orphan
            "window-pool-bbb", // orphan
            "window-pool-ccc", // unpromoted → not orphan
            "window-pool-ddd", // orphan
            "main",            // not pool prefix
        ]);
        let unpromoted = set_of(&["window-pool-ccc"]);
        let shadow = set_of(&["window-pool-aaa"]);
        assert_eq!(
            classify_candidate_orphans(&labels, &unpromoted, &shadow),
            vec_of(&["window-pool-bbb", "window-pool-ddd"])
        );
    }

    #[test]
    fn classify_returns_freshly_promoted_as_candidate() {
        // In the WindowOpened-echo lag a freshly-promoted pool window
        // is in `browsers` (not in unpromoted) and the launcher's
        // mirror hasn't caught up yet (not in shadow). The classifier
        // returns it as a CANDIDATE; the UI-thread HWND check is
        // what saves it from being closed (live HWND → skip).
        let labels = vec_of(&["window-pool-just-promoted"]);
        let unpromoted = set_of(&[]);
        let shadow = set_of(&[]);
        assert_eq!(
            classify_candidate_orphans(&labels, &unpromoted, &shadow),
            vec_of(&["window-pool-just-promoted"])
        );
    }

    // ── plan_reconcile integration tests ──────────────────────────
    //
    // The classifier above is necessary-but-not-sufficient — every
    // codex P1 since round 1 has been in the orchestrator's decision
    // logic, not the classifier. These tests cover the state cross-
    // product the orchestrator must handle:
    //
    //   axes: HwndStatus (Live / Dead / Hostless),
    //         in shadow_window_meta (yes / no),
    //         in pool.queue (yes / no),
    //         label prefix (window-pool-* / browser-pane-* / other),
    //         and the resulting (live_user_count, freshly_promoted)
    //         derivation that drives `safe_to_drain`.
    //
    // Each test names the scenario from the spec (Race A/B/C/D) it
    // exercises, plus the codex round it would have caught if it
    // had existed earlier.

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
        // needs UnregisterBrowser, not CloseBrowser. Codex round-16
        // P1 case (the warm-pool loop must not blindly emit
        // CloseBrowser for non-probed pool labels).
        let label = "window-pool-hostless-unpromoted";
        let plan = plan_reconcile(
            &map_of(&[(label, HwndStatus::Hostless)]),
            &vec_of(&[label]),
            &set_of(&[]),
            &set_of(&[]),
            &set_of(&[label]),
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
        );
        // freshly_promoted blocks drain → only zombies+hostless close.
        assert_eq!(plan.freshly_promoted, vec![fresh.to_string()]);
        assert!(!plan.begin_drain);
        let actions: HashMap<String, CloseAction> = plan
            .closes
            .iter()
            .map(|(l, a)| (l.clone(), a.clone()))
            .collect();
        assert_eq!(actions.get(zombie), Some(&CloseAction::CloseBrowser));
        assert_eq!(actions.get(hostless), Some(&CloseAction::UnregisterBrowser));
        assert!(!actions.contains_key(unpromoted), "unpromoted spared when drain blocked");
        assert!(!actions.contains_key(ready), "ready spared when drain blocked");
        assert!(!actions.contains_key(fresh), "freshly_promoted never closes");
        assert!(!actions.contains_key(promoted), "promoted user window never closes");
        assert!(!actions.contains_key(pane), "pane never in plan");
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
        let p1 = plan_reconcile(&inputs.0, &inputs.1, &inputs.2, &inputs.3, &inputs.4);
        let p2 = plan_reconcile(&inputs.0, &inputs.1, &inputs.2, &inputs.3, &inputs.4);
        assert_eq!(p1, p2);
    }

    #[test]
    fn classify_handles_zombie_pool_pair() {
        // Two `window-pool-*` labels still in browsers, both promoted
        // (host's pool.queue is empty so unpromoted is empty), and
        // the launcher's mirror has dropped them (shadow is empty).
        let labels = vec_of(&[
            "window-pool-722b6186bb6e42378b48b7068c0d54b0",
            "window-pool-b4e20337180247bdbd7408ddd7754b78",
        ]);
        let unpromoted = set_of(&[]);
        let shadow = set_of(&[]);
        let orphans = classify_candidate_orphans(&labels, &unpromoted, &shadow);
        assert_eq!(orphans.len(), 2);
        assert_eq!(orphans, labels);
    }
}
