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

/// Pure planning function. Given the current state snapshot +
/// candidate classifications, compute what the orchestrator should
/// do. No CEF, no Win32, no dispatch — just the decision.
///
/// `candidates` is the set already produced by
/// `classify_candidate_orphans` (window-pool-* labels not in
/// unpromoted_pool, not in shadow). `hwnd_status` maps each
/// candidate label to its HWND state — production builds this map
/// from real CEF calls; tests build it directly.
///
/// `all_browser_labels` is `state.list_browsers()` keyed; needed for
/// the warm-pool close loop (which must include
/// not-classified-as-candidate labels like `pool.unpromoted`
/// browsers that aren't candidates by classifier rule but still
/// need to drain in shutdown).
pub(crate) fn plan_reconcile(
    candidates: &[String],
    hwnd_status: &HashMap<String, HwndStatus>,
    all_browser_labels: &[String],
    shadow_keys: &HashSet<String>,
    pool_queue: &HashSet<String>,
) -> ReconcilePlan {
    let candidate_set: HashSet<&str> = candidates.iter().map(|s| s.as_str()).collect();

    // Live user count = labels in shadow, not pane-prefixed. Zombies
    // are absent from shadow because `apply_hwnd_destroyed` prunes
    // `state.windows`, so they don't need to be subtracted.
    let live_user_count = all_browser_labels
        .iter()
        .filter(|label| {
            shadow_keys.contains(label.as_str()) && !label.starts_with("browser-pane-")
        })
        .count();

    let mut zombie_close: Vec<(String, CloseAction)> = Vec::new();
    let mut freshly_promoted: Vec<String> = Vec::new();
    for label in candidates {
        let status = hwnd_status.get(label).copied().unwrap_or(HwndStatus::Live);
        match status {
            HwndStatus::Dead => zombie_close.push((label.clone(), CloseAction::CloseBrowser)),
            HwndStatus::Hostless => zombie_close.push((label.clone(), CloseAction::UnregisterBrowser)),
            HwndStatus::Live => {
                if pool_queue.contains(label) {
                    // Ready warm pool — handled below in drain branch
                    // via the warm-pool close loop. No action here.
                } else {
                    freshly_promoted.push(label.clone());
                }
            }
        }
    }

    let safe_to_drain = live_user_count == 0 && freshly_promoted.is_empty();

    let mut closes = zombie_close;

    if safe_to_drain {
        // Add every `window-pool-*` browser still in
        // `all_browser_labels` that isn't already scheduled for
        // close and isn't freshly_promoted. Picks up ready warm pool
        // (in `pool.queue`) AND unpromoted pool inventory (which the
        // classifier filtered out).
        let already_close: HashSet<String> = closes.iter().map(|(l, _)| l.clone()).collect();
        let freshly_set: HashSet<String> = freshly_promoted.iter().cloned().collect();
        for label in all_browser_labels {
            if !label.starts_with("window-pool-") {
                continue;
            }
            if already_close.contains(label) {
                continue;
            }
            if freshly_set.contains(label) {
                continue;
            }
            // Default action: CloseBrowser. If the label is hostless
            // it must already be in `closes` from the candidate loop
            // (a hostless candidate produces UnregisterBrowser there);
            // labels reached here are necessarily live or unknown,
            // so close_browser is the right call.
            closes.push((label.clone(), CloseAction::CloseBrowser));
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

/// IPC-thread entry point. Snapshots host state under locks (no
/// CEF calls), classifies candidate orphans, and posts the
/// CEF-touching work to the UI thread.
pub fn reconcile_and_drain(state: &Arc<AppState>) {
    let labels: Vec<String> = state
        .list_browsers()
        .into_iter()
        .map(|(l, _)| l)
        .collect();
    let unpromoted = state.unpromoted_pool_labels_snapshot();
    // Shadow alone is the source of truth for "launcher tracks this
    // as live". Local `window_meta` is unreliable here because it's
    // populated in `on_after_created` and only cleared in
    // `on_before_close` — exactly the callback that doesn't run for
    // the zombies we're trying to reap.
    let shadow_keys: HashSet<String> = state
        .shadow_window_meta
        .lock()
        .keys()
        .cloned()
        .collect();
    let candidates = classify_candidate_orphans(&labels, &unpromoted, &shadow_keys);

    // Don't early-return when candidates is empty: the warm-pool
    // close loop in the UI-thread planner ALSO needs to fire when
    // the only remaining browsers are unpromoted-pool inventory
    // (which the classifier filters out by design — they're not
    // orphan candidates, but they DO need to drain on shutdown when
    // the host's normal Stage-1 cascade was skipped).

    // Marshal CEF Browser/BrowserHost calls to the UI thread. The
    // existing two-stage cascade (`client/mod.rs::on_before_close`)
    // gets to make these calls inline because it already runs on
    // the UI thread; we don't, so we hand the work to CEF's task
    // queue. Three v0.33.491–v0.33.494 attempts at driving UI work
    // *directly* from the IPC handler all hung CEF — this path
    // avoids that by using CEF's own scheduler.
    let mut task = OrphanReconcileTask::new(state.clone(), candidates);
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
        candidates: Vec<String>,
    }

    impl Task {
        fn execute(&self) {
            ui_thread_reconcile(&self.state, &self.candidates);
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
fn ui_thread_reconcile(state: &Arc<AppState>, candidates: &[String]) {
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

    // Build hwnd_status map for each candidate via real CEF calls.
    // The planner consumes this directly; the production status
    // function is the only piece that can't be unit-tested.
    let label_to_browser: HashMap<String, Browser> = browser_pairs
        .iter()
        .map(|(l, b)| (l.clone(), b.clone()))
        .collect();
    let hwnd_status: HashMap<String, HwndStatus> = candidates
        .iter()
        .map(|label| {
            let status = label_to_browser
                .get(label)
                .map(|b| classify_hwnd(b))
                .unwrap_or(HwndStatus::Hostless); // missing = treat as gone
            (label.clone(), status)
        })
        .collect();

    let all_labels: Vec<String> = browser_pairs.iter().map(|(l, _)| l.clone()).collect();
    let plan = plan_reconcile(
        candidates,
        &hwnd_status,
        &all_labels,
        &shadow_keys,
        &pool_queue,
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
    fn plan_no_candidates_no_browsers_is_empty_no_drain() {
        let plan = plan_reconcile(
            &vec_of(&[]),
            &map_of(&[]),
            &vec_of(&[]),
            &set_of(&[]),
            &set_of(&[]),
        );
        assert!(plan.closes.is_empty());
        assert!(plan.freshly_promoted.is_empty());
        // No live user windows AND no freshly_promoted → safe_to_drain
        // is technically true, but begin_drain semantics carry the
        // intent. With no closes scheduled, the orchestrator early-
        // returns; begin_drain=true is harmless metadata in that case.
        assert!(plan.begin_drain);
    }

    #[test]
    fn plan_single_dead_zombie_drains_and_closes() {
        // The user's v0.33.643 case: one zombie window-pool-* in
        // browsers, no other live browsers, no shadow.
        let label = "window-pool-zombie";
        let plan = plan_reconcile(
            &vec_of(&[label]),
            &map_of(&[(label, HwndStatus::Dead)]),
            &vec_of(&[label]),
            &set_of(&[]), // shadow empty
            &set_of(&[]), // pool.queue empty
        );
        assert_eq!(plan.closes, vec![(label.to_string(), CloseAction::CloseBrowser)]);
        assert!(plan.freshly_promoted.is_empty());
        assert!(plan.begin_drain);
    }

    #[test]
    fn plan_hostless_zombie_unregisters_and_drains() {
        // Hostless zombie (BrowserHost gone) — needs UnregisterBrowser
        // not CloseBrowser, since there's no host to call.
        let label = "window-pool-hostless";
        let plan = plan_reconcile(
            &vec_of(&[label]),
            &map_of(&[(label, HwndStatus::Hostless)]),
            &vec_of(&[label]),
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
        // Race B: live HWND, NOT in pool.queue (popped during
        // promotion). The shadow echo hasn't returned yet so it's
        // not in shadow either. Codex round 1 + 13 case: must NOT
        // be closed AND must block drain.
        let label = "window-pool-just-promoted";
        let plan = plan_reconcile(
            &vec_of(&[label]),
            &map_of(&[(label, HwndStatus::Live)]),
            &vec_of(&[label]),
            &set_of(&[]),
            &set_of(&[]), // pool.queue empty (label was popped)
        );
        assert!(plan.closes.is_empty(), "freshly promoted must not be closed: {:?}", plan.closes);
        assert_eq!(plan.freshly_promoted, vec![label.to_string()]);
        assert!(!plan.begin_drain, "freshly_promoted must block drain");
    }

    #[test]
    fn plan_ready_warm_pool_drains_via_warmpool_loop() {
        // Race D + codex round 12 case: live HWND IN pool.queue.
        // Common shutdown path — last user window closes, only ready
        // warm-pool browsers remain. The classifier returns them as
        // candidates, the planner classifies them as "ready pool"
        // (handled implicitly via the warm-pool close loop), drain
        // fires and they're closed.
        let label = "window-pool-ready";
        let plan = plan_reconcile(
            &vec_of(&[label]),
            &map_of(&[(label, HwndStatus::Live)]),
            &vec_of(&[label]),
            &set_of(&[]),
            &set_of(&[label]), // pool.queue contains it
        );
        assert_eq!(plan.closes, vec![(label.to_string(), CloseAction::CloseBrowser)]);
        assert!(plan.freshly_promoted.is_empty());
        assert!(plan.begin_drain);
    }

    #[test]
    fn plan_unpromoted_pool_browsers_drain_via_warmpool_loop() {
        // Unpromoted pool browsers aren't candidates (the classifier
        // filters them), but the warm-pool close loop in the drain
        // branch must still close them — otherwise they stay alive
        // in browser_list and Stage 2 quit never fires.
        // Codex round 7 case.
        let unpromoted_label = "window-pool-unpromoted";
        let plan = plan_reconcile(
            &vec_of(&[]),                   // no candidates
            &map_of(&[]),
            &vec_of(&[unpromoted_label]),   // but it IS in browser_pairs
            &set_of(&[]),
            &set_of(&[]),
        );
        assert_eq!(
            plan.closes,
            vec![(unpromoted_label.to_string(), CloseAction::CloseBrowser)]
        );
        assert!(plan.begin_drain);
    }

    #[test]
    fn plan_live_user_window_blocks_drain() {
        // A live user window in shadow → live_user_count > 0 →
        // safe_to_drain == false. A stale HostShouldQuit racing with
        // an active session must NOT enter drain (no transition back
        // to Running). Only zombies close; warm pool stays.
        let user_label = "window-pool-promoted-user";
        let zombie_label = "window-pool-zombie";
        let plan = plan_reconcile(
            &vec_of(&[zombie_label]),
            &map_of(&[(zombie_label, HwndStatus::Dead)]),
            &vec_of(&[user_label, zombie_label]),
            &set_of(&[user_label]), // user window is in shadow
            &set_of(&[]),
        );
        assert_eq!(
            plan.closes,
            vec![(zombie_label.to_string(), CloseAction::CloseBrowser)]
        );
        assert!(!plan.begin_drain, "live user window must block drain");
    }

    #[test]
    fn plan_zombie_plus_freshly_promoted_skips_drain() {
        // Zombie + Race B together: zombie should be closed regardless
        // of drain decision; freshly_promoted must block drain so the
        // pre-shadow-echo live window isn't killed. Codex round 13.
        let zombie = "window-pool-zombie";
        let promoted = "window-pool-promoted";
        let plan = plan_reconcile(
            &vec_of(&[zombie, promoted]),
            &map_of(&[
                (zombie, HwndStatus::Dead),
                (promoted, HwndStatus::Live),
            ]),
            &vec_of(&[zombie, promoted]),
            &set_of(&[]),
            &set_of(&[]), // promoted not in pool.queue → freshly_promoted
        );
        assert_eq!(
            plan.closes,
            vec![(zombie.to_string(), CloseAction::CloseBrowser)]
        );
        assert_eq!(plan.freshly_promoted, vec![promoted.to_string()]);
        assert!(!plan.begin_drain, "freshly_promoted blocks drain even with zombies present");
    }

    #[test]
    fn plan_zombie_plus_ready_pool_drains_both() {
        // Zombie + Race D: drain fires (no live user, no freshly
        // promoted). Both close.
        let zombie = "window-pool-zombie";
        let ready = "window-pool-ready";
        let plan = plan_reconcile(
            &vec_of(&[zombie, ready]),
            &map_of(&[
                (zombie, HwndStatus::Dead),
                (ready, HwndStatus::Live),
            ]),
            &vec_of(&[zombie, ready]),
            &set_of(&[]),
            &set_of(&[ready]),
        );
        let close_labels: Vec<&str> = plan.closes.iter().map(|(l, _)| l.as_str()).collect();
        assert!(close_labels.contains(&zombie));
        assert!(close_labels.contains(&ready));
        assert_eq!(plan.closes.len(), 2);
        assert!(plan.begin_drain);
    }

    #[test]
    fn plan_drain_excludes_freshly_promoted_from_warmpool_loop() {
        // Drain branch's warm-pool loop closes every window-pool-*,
        // EXCEPT freshly_promoted entries. Verifies the gate prevents
        // the warm-pool loop from killing a live new window.
        // (This shouldn't happen normally because freshly_promoted
        // also blocks drain, but the gate is defense in depth.)
        let promoted = "window-pool-promoted";
        let unpromoted = "window-pool-unpromoted";
        // Construct a state where drain WOULD fire (no freshly_promoted)
        // but the warm-pool loop sees both labels — verify only
        // unpromoted closes.
        let plan = plan_reconcile(
            &vec_of(&[]),
            &map_of(&[]),
            &vec_of(&[promoted, unpromoted]),
            &set_of(&[promoted]), // promoted is in shadow → user window
            &set_of(&[]),
        );
        // promoted is in shadow → live_user_count = 1 → no drain
        // → no warm-pool loop runs → no closes.
        assert!(plan.closes.is_empty());
        assert!(!plan.begin_drain);
    }

    #[test]
    fn plan_browser_pane_labels_dont_count_toward_live_user() {
        // Pane labels are excluded from live_user_count so they don't
        // keep the host alive when only panes + zombies remain.
        let pane = "browser-pane-foo";
        let zombie = "window-pool-zombie";
        let plan = plan_reconcile(
            &vec_of(&[zombie]),
            &map_of(&[(zombie, HwndStatus::Dead)]),
            &vec_of(&[pane, zombie]),
            &set_of(&[pane]), // pane in shadow but excluded by prefix
            &set_of(&[]),
        );
        assert!(plan.begin_drain, "pane in shadow shouldn't block drain");
        let close_labels: Vec<&str> = plan.closes.iter().map(|(l, _)| l.as_str()).collect();
        assert!(close_labels.contains(&zombie));
    }

    #[test]
    fn plan_v0_33_643_reproduction() {
        // Verbatim reproduction of the user's reported bug:
        //   DRIFT Pool: host=0 mirror=2
        // Two `window-pool-*` labels in browsers, both promoted (host's
        // pool empty), both dropped from launcher mirror (HWNDs were
        // destroyed without on_before_close firing). Both have dead
        // HWNDs.
        let z1 = "window-pool-722b6186bb6e42378b48b7068c0d54b0";
        let z2 = "window-pool-b4e20337180247bdbd7408ddd7754b78";
        let plan = plan_reconcile(
            &vec_of(&[z1, z2]),
            &map_of(&[(z1, HwndStatus::Dead), (z2, HwndStatus::Dead)]),
            &vec_of(&[z1, z2]),
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
    fn plan_mixed_zombie_hostless_unpromoted_drains_all() {
        // Composite: dead zombie + hostless zombie + unpromoted pool
        // (not a candidate but in browsers) + a regular pane (excluded
        // from drain decision). Drain fires; zombies use their
        // respective actions; unpromoted joins via warm-pool loop;
        // pane is left alone.
        let dead = "window-pool-dead";
        let hostless = "window-pool-hostless";
        let unpromoted = "window-pool-unpromoted";
        let pane = "browser-pane-bar";
        let plan = plan_reconcile(
            &vec_of(&[dead, hostless]),
            &map_of(&[
                (dead, HwndStatus::Dead),
                (hostless, HwndStatus::Hostless),
            ]),
            &vec_of(&[dead, hostless, unpromoted, pane]),
            &set_of(&[]),
            &set_of(&[]),
        );
        assert!(plan.begin_drain);
        let actions: HashMap<String, CloseAction> = plan
            .closes
            .iter()
            .map(|(l, a)| (l.clone(), a.clone()))
            .collect();
        assert_eq!(actions.get(dead), Some(&CloseAction::CloseBrowser));
        assert_eq!(actions.get(hostless), Some(&CloseAction::UnregisterBrowser));
        assert_eq!(actions.get(unpromoted), Some(&CloseAction::CloseBrowser));
        assert!(!actions.contains_key(pane), "pane should not be closed");
    }

    #[test]
    fn plan_idempotent_under_repeat() {
        // Calling the planner twice with identical state produces
        // identical plans. Critical for `HostShouldQuit` idempotency.
        let label = "window-pool-zombie";
        let inputs = (
            vec_of(&[label]),
            map_of(&[(label, HwndStatus::Dead)]),
            vec_of(&[label]),
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
