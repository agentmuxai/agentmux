// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Host-side orphan-instance reconciliation. Specs:
//! `docs/specs/SPEC_HOST_ORPHAN_RECONCILIATION_2026_05_05.md` (origin),
//! `docs/specs/SPEC_PILLAR2_SANITIZE_THEN_DECIDE_2026_07_11.md` §2.1 (the
//! Phase-1 restructure this module now implements).
//!
//! When the launcher detects that the last user-visible window has closed but
//! the host is still alive, it emits `Event::HostShouldQuit`. The host's
//! handler invokes `reconcile_and_drain` here.
//!
//! **This module is a sanitizer + executor, not a decider.** It probes HWND
//! liveness, repairs the reducer's `browsers` projection (dead/hostless
//! zombies are the one direction the projection goes stale — a crashed window
//! never runs any close flow, so its `TopLevel { is_pool: false }` entry
//! would block `reconcile_quit` forever), and then asks the reducer for the
//! drain verdict via `HostCommand::ReconcileQuit` → `request_drain`. The
//! pre-Phase-1 parallel decision (`ReconcilePlan.begin_drain`, computed from
//! launcher-shadow membership + a Race-B `freshly_promoted` guard) is gone:
//! the reducer already models everything it derived — promotion flips
//! `is_pool:false` synchronously (`reducer/pool.rs`), so a freshly-promoted
//! window is already counted live by `count_live_user_windows`, and floaters
//! are excluded BY TYPE (invariant FP-LIFE) rather than depending on what the
//! launcher mirror happens to contain.
//!
//! Threading: CEF Browser/BrowserHost methods (`host()`, `window_handle()`,
//! `close_browser()`) MUST run on the UI thread per CEF docs. The IPC reader
//! thread that delivers `HostShouldQuit` does no work of its own; it
//! `cef::post_task`s the UI-thread closure.

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

/// Output of the pure planning step — a SANITIZE plan, not a decision (the
/// drain verdict comes from `reconcile_quit` via `HostCommand::ReconcileQuit`
/// after the sanitizes land). The orchestrator executes this against real CEF
/// state. Kept separate so tests can verify classification without standing
/// up a CEF runtime.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct ReconcilePlan {
    /// Dead zombies → `close_browser(force=1)`, in deterministic order.
    /// Self-sanitizing: CEF's cleanup chain runs `on_before_close`, which
    /// dispatches `UnregisterBrowser` AND consumes its own `request_drain`
    /// (PR #1993) — so user-kind zombies that block the verdict at poke time
    /// still drive the drain when their close resolves (the level-trigger
    /// completes asynchronously). Empty when a user creation is in flight:
    /// the close chain itself could race the pending creation (see the
    /// `plan_pending_creation_blocks_zombie_close` test).
    pub zombie_closes: Vec<String>,
    /// Hostless entries whose reducer kind is a live user window
    /// (`TopLevel { is_pool: false }`) — the stale-projection entries that
    /// block `reconcile_quit`. Unregistered UNCONDITIONALLY (projection
    /// repair, correct at any time): there is no `BrowserHost` to close and
    /// no close flow that will ever repair them.
    pub hostless_user: Vec<String>,
    /// Hostless pool-kind entries. They don't block the verdict (not counted
    /// by `count_live_user_windows`), and unregistering them outside drain
    /// bypasses `on_pool_window_destroyed` bookkeeping (pool-reducer drift) —
    /// so their cleanup stays drain-gated, exactly as before Phase 1.
    pub hostless_pool: Vec<String>,
    /// Live HWND, not in shadow, not in a pool set — the Race-B telemetry.
    /// Diagnostic ONLY: the reducer already counts these windows live
    /// (promotion flips `is_pool:false` synchronously), so they block the
    /// verdict through the standard count, not through this list.
    pub freshly_promoted: Vec<String>,
}

/// Pure planning function. The orchestrator HWND-probes every non-pane
/// top-level browser in `state.browsers` and feeds that map here.
///
/// `browser_status` keys every non-pane label in `state.browsers` to its
/// HwndStatus. Includes pool-prefixed labels AND regular top-level windows
/// (e.g. `main`, `window-N`). Pane labels (`browser-pane-*`) are NOT in this
/// map — they drain via a separate cascade and would otherwise pollute
/// classification.
///
/// `user_labels` is the set of labels whose reducer kind is
/// `TopLevel { is_pool: false }` — the authoritative live-user
/// classification, used to split hostless entries into
/// unconditional-repair vs drain-gated buckets.
///
/// Live entries produce no action at all: promoted windows (in shadow) and
/// freshly-promoted windows (in neither shadow nor pool sets) are live user
/// windows the reducer already counts; live pool inventory is Stage 1's job
/// (`ui_tasks::begin_drain_and_cascade` closes `window-pool-*` /
/// `floating-pool-*` when the drain verdict fires) — the pre-Phase-1
/// `drainable` close list duplicated that mechanism and is gone.
pub(crate) fn plan_reconcile(
    browser_status: &HashMap<String, HwndStatus>,
    user_labels: &HashSet<String>,
    shadow_keys: &HashSet<String>,
    pool_queue: &HashSet<String>,
    unpromoted_pool: &HashSet<String>,
    pending_creation_in_flight: bool,
) -> ReconcilePlan {
    let mut zombie_closes: Vec<String> = Vec::new();
    let mut hostless_user: Vec<String> = Vec::new();
    let mut hostless_pool: Vec<String> = Vec::new();
    let mut freshly_promoted: Vec<String> = Vec::new();
    for (label, status) in browser_status {
        match status {
            // Dead zombies close regardless of any verdict —
            // `close_browser(force=1)` drives the host's own cleanup chain
            // (on_before_close → UnregisterBrowser → request_drain
            // consumption). BUT if a window creation is pending, that chain
            // can race the creation: when the zombie is the last live-counted
            // browser, its close-time `request_drain` fires and Stage-2 quit
            // could land before the new window registers. Defer zombie reap
            // until the creation completes; the next `HostShouldQuit`
            // catches it. (Pre-Phase-1 behavior, preserved.)
            HwndStatus::Dead => {
                if !pending_creation_in_flight {
                    zombie_closes.push(label.clone());
                }
            }
            HwndStatus::Hostless => {
                if user_labels.contains(label.as_str()) {
                    hostless_user.push(label.clone());
                } else {
                    hostless_pool.push(label.clone());
                }
            }
            HwndStatus::Live => {
                if shadow_keys.contains(label.as_str()) {
                    // promoted user window — reducer counts it; leave alone
                } else if pool_queue.contains(label) || unpromoted_pool.contains(label) {
                    // warm/spawning pool inventory — Stage 1 closes it if a
                    // drain fires; no per-entry action here
                } else {
                    // Race-B window (promotion echo in flight). Telemetry
                    // only — the reducer already counts it live.
                    freshly_promoted.push(label.clone());
                }
            }
        }
    }

    // Sort for determinism (HashMap iteration order is unspecified).
    zombie_closes.sort();
    hostless_user.sort();
    hostless_pool.sort();
    freshly_promoted.sort();

    ReconcilePlan {
        zombie_closes,
        hostless_user,
        hostless_pool,
        freshly_promoted,
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

/// UI-thread body: **sanitize → decide → execute** (sanitize-then-decide
/// spec §2.1). CEF Browser methods are safe here.
///
/// 1. Probe every non-pane top-level browser's HWND (re-snapshotted on the
///    UI thread — state may have advanced since `HostShouldQuit` fired).
/// 2. Sanitize the projection: unregister hostless live-user entries
///    (unconditional repair — nothing else will ever clean them).
/// 3. Decide: dispatch `ReconcileQuit`; the reducer's `request_drain` is THE
///    verdict. A stale `HostShouldQuit` racing a live session gets `None`
///    here (live windows are counted; `QuitState` never flips; pool refill
///    keeps working) — the same protection the old shadow-count provided,
///    now from the single authority.
/// 4. Execute: on `Some`, run the shared Stage-1 executor
///    (`ui_tasks::begin_drain_and_cascade` — flips `QuitState`, closes pool
///    inventory), then close dead zombies (their `on_before_close` chain
///    self-sanitizes and, per PR #1993, consumes its own `request_drain` —
///    covering the user-kind-zombie case where the poke's verdict was still
///    `None` because the zombie was counted live), then the drain-gated
///    hostless-pool cleanup and the documented direct `quit_message_loop`
///    Stage-2 executions.
///
/// Platform note: `classify_hwnd` hard-codes `Live` on macOS/Linux (#1569),
/// so the zombie/hostless buckets — and every Windows-looking branch below —
/// are empty there; macOS/Linux quit flows run entirely through
/// `on_before_close`'s own consumption + Stage-2 gate. The residual mixed
/// case on Windows (user-kind zombies alongside hostless entries, where
/// Stage 2's `browser_list` gate can never fire) is bounded by the WRR quit
/// watchdog (SPEC_WRR_QUIT_FALSE_POSITIVE Step D), which quits on the OS
/// signal with a loud desync log.
fn ui_thread_reconcile(state: &Arc<AppState>) {
    let browser_pairs = state.list_browsers();
    let shadow_keys: HashSet<String> = state
        .shadow_window_meta
        .lock()
        .keys()
        .cloned()
        .collect();
    let pool_queue: HashSet<String> = {
        let st = state.host_state.lock();
        st.pool
            .queue
            .iter()
            .cloned()
            .chain(st.pane_pool.queue.iter().cloned())
            .collect()
    };
    // unpromoted_pool_labels_snapshot covers both tab pool (window-pool-*) and
    // pane pool (floating-pool-*) unpromoted sets.
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
    // Authoritative live-user classification (BrowserKind, not label prefix)
    // — splits hostless entries into unconditional-repair vs drain-gated.
    let user_labels: HashSet<String> = label_to_browser
        .keys()
        .filter(|l| state.is_live_top_level_browser(l))
        .cloned()
        .collect();

    // Only USER-window pending creates defer the zombie reaps. Pool spawns
    // (`window-pool-*`), pane creates (`browser-pane-*`), and floating panes
    // (`floating-*`, which can leak a pending entry on failure — codex P1 on
    // PR #811) are background; the DRAIN decision itself already accounts for
    // pending user creations inside `reconcile_quit` (its
    // `user_creation_in_flight` read mirrors this exact exclusion — see
    // `reducer/quit.rs::is_background_pending_creation_label`).
    let pending_creation_in_flight = state
        .host_state
        .lock()
        .pending_window_creations
        .iter()
        .any(|p| {
            !p.label.starts_with("window-pool-")
                && !p.label.starts_with("browser-pane-")
                && !p.label.starts_with("floating-")
        });
    let plan = plan_reconcile(
        &browser_status,
        &user_labels,
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

    // ── Sanitize: hostless live-user entries are unconditionally
    // unregistered. This is pure projection repair — the browser is gone at
    // the CEF level; only the stale registration remains, and it is exactly
    // what blocks `reconcile_quit` from ever draining. (Pre-Phase-1 this
    // cleanup was drain-gated, which was circular once the drain decision
    // itself read the registration.)
    let mut any_hostless = false;
    for (i, label) in plan.hostless_user.iter().enumerate() {
        drive_unregister(state, i, label);
        any_hostless = true;
    }

    // ── Decide: the reducer is the single authority. `ReconcileQuit` is a
    // pure poke — the verdict reflects the sanitizes above (dispatched on
    // this same thread, so they've landed).
    let verdict = state
        .host_dispatch(crate::reducer::HostCommand::ReconcileQuit)
        .request_drain;

    // ── Execute: Stage 1 FIRST (flips QuitState to Draining, suppressing
    // pool refill, before any close below can trigger
    // `on_pool_window_destroyed`), mirroring the pre-Phase-1
    // BeginDrain-before-closes ordering.
    if let Some(reason) = verdict.clone() {
        crate::ui_tasks::begin_drain_and_cascade(state, reason);
    } else {
        tracing::info!(
            target: "wrr",
            "[orphan-reconcile] no drain verdict — live user windows, pending user creation, or already draining"
        );
    }

    // ── Mechanism: dead zombies close via CEF's own cleanup chain. For
    // user-kind zombies the poke above answered `None` (they were still
    // counted live); their `on_before_close` unregisters them and consumes
    // its own `request_drain` (PR #1993), so the drain still fires — decided
    // by the same authority, one event later.
    for (i, label) in plan.zombie_closes.iter().enumerate() {
        if let Some(browser) = label_to_browser.get(label).cloned() {
            let late_hostless = drive_close_browser(
                state,
                i,
                label,
                browser,
                user_labels.contains(label.as_str()),
                verdict.is_some(),
            );
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

    if verdict.is_none() {
        return;
    }

    // ── Drain-gated cleanup: hostless pool-kind entries. Outside drain,
    // unregistering them bypasses `on_pool_window_destroyed` bookkeeping and
    // creates pool-reducer drift; the hostless state is already a CEF
    // lifecycle anomaly, so letting it persist until a drain-capable
    // reconcile is preferable (pre-Phase-1 rationale, unchanged).
    for (i, label) in plan.hostless_pool.iter().enumerate() {
        drive_unregister(state, i, label);
        any_hostless = true;
    }

    // Hostless orphans don't get `on_before_close` callbacks. The host's own
    // Stage-2 quit gates on `client::browser_list.is_empty()` — but
    // UnregisterBrowser only touches the reducer's `browsers` map, not
    // CefClient's internal list, so a stale hostless entry there permanently
    // blocks Stage 2. Drive `quit_message_loop` ourselves whenever the drain
    // verdict fired AND any hostless cleanup happened. Zombie closes
    // dispatched alongside are mid-close at this point; their
    // `on_before_close` may not run before the loop terminates, but we're
    // shutting down anyway and OS process exit reclaims their resources.
    // This (and the nothing-will-pump case below) are the two documented
    // direct Stage-2 executions of a reducer-made decision — executor work,
    // not a competing authority (spec §6's grep-gate allowlist).
    if any_hostless {
        tracing::warn!(
            target: "wrr",
            "[orphan-reconcile] hostless orphans unregistered in drain mode — driving quit_message_loop"
        );
        quit_message_loop();
        return;
    }

    // Nothing-will-pump case: drain fired but there are no zombie closes in
    // flight and Stage 1 found no pool inventory to close — no CEF lifecycle
    // event will ever advance Stage 2 (stale `client::browser_list` entries
    // we can't see from here may even be blocking it). Drive quit directly.
    // Type-based tab-pool membership (+ prefix for the pane pool) — same
    // reasoning as Stage 1's sweep: an ADOPTED pool window's foreign label
    // (SPEC_POOL_ADOPTION_AND_WINDOW_ROW_CRUMB Residual 1) would make a
    // prefix-only count read 0 here and drive quit_message_loop with a live
    // pool browser still registered.
    let tab_pool_labels = state.pool_side_top_level_labels();
    let pool_inventory = state
        .list_browsers()
        .iter()
        .filter(|(l, _)| tab_pool_labels.contains(l) || l.starts_with("floating-pool-"))
        .count();
    if plan.zombie_closes.is_empty() && pool_inventory == 0 {
        tracing::warn!(
            target: "wrr",
            "[orphan-reconcile] drain verdict with nothing to close and no pool inventory — driving quit_message_loop"
        );
        quit_message_loop();
    }
}

/// Map a Browser to its `HwndStatus`. Calls CEF + Win32; only safe
/// from the UI thread. Used by the orchestrator to build the input
/// to `plan_reconcile`.
fn classify_hwnd(browser: &Browser) -> HwndStatus {
    let b = browser.clone();
    let Some(host) = b.host() else { return HwndStatus::Hostless };
    #[cfg(target_os = "windows")]
    unsafe {
        use windows_sys::Win32::Foundation::HWND;
        use windows_sys::Win32::UI::WindowsAndMessaging::IsWindow;
        let wh = host.window_handle();
        if wh.0.is_null() {
            return HwndStatus::Dead;
        }
        if IsWindow(wh.0 as HWND) == 0 {
            HwndStatus::Dead
        } else {
            HwndStatus::Live
        }
    }
    // On Linux/macOS `cef_window_handle_t` is `u64` (X11 XID / NSView ptr),
    // not the Win32 HWND tuple-struct, so `wh.0.is_null()` doesn't typecheck.
    // We don't have an `IsWindow` equivalent here either — treat any host
    // with a Browser as Live for orphan-reconcile classification. The Win32
    // path keeps the strict liveness check that landed in #702.
    #[cfg(not(target_os = "windows"))]
    {
        let _ = host;
        HwndStatus::Live
    }
}

/// Execute a zombie close. Already on UI thread.
/// `host.close_browser(force=1)` works regardless of HWND state and
/// triggers the host's `on_before_close` callback chain (which
/// dispatches `UnregisterBrowser`, consumes its own `request_drain`,
/// and drives Stage-2 quit naturally). Returns `true` iff the browser's
/// host had vanished by execute time AND this late hostless transition
/// was repaired here (relevant for the caller's Stage-2 quit drive —
/// same reasoning as the planner's hostless buckets).
fn drive_close_browser(
    state: &Arc<AppState>,
    idx: usize,
    label: &str,
    browser: Browser,
    is_user: bool,
    drain_active: bool,
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
        // Race: HWND status was Dead at planning, but BrowserHost vanished
        // before we got here. Mirror the planner's hostless split: live-user
        // entries are unconditional projection repairs; pool-kind entries
        // are repaired only under an active drain (outside drain,
        // UnregisterBrowser bypasses on_pool_window_destroyed bookkeeping
        // and creates pool-reducer drift).
        if is_user || drain_active {
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
                "[orphan-reconcile][{}] browser host=None at execute time label={} — late hostless transition; deferring UnregisterBrowser (pool-kind, drain not active)",
                idx, label
            );
            false
        }
    }
}

/// Execute an `UnregisterBrowser` repair. Hostless candidates can't
/// be `close_browser`'d (no `BrowserHost` to call it on), so we
/// clean `state.browsers` ourselves. Caller drives `quit_message_loop`
/// after the loop if any unregister fired under an active drain.
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

    // ── plan_reconcile classification tests ───────────────────────
    //
    // Phase 1 (sanitize-then-decide): the planner classifies SANITIZE work
    // only — the drain decision moved to `reconcile_quit` (see the reducer
    // truth-table tests in `reducer/tests.rs`, which pin arming, live-count,
    // pending-creation, and monotonicity behavior). Axes covered here:
    //
    //   HwndStatus (Live / Dead / Hostless) ×
    //   reducer user-kind (in user_labels / not) ×
    //   shadow membership × pool-set membership ×
    //   pending user creation (defers zombie reaps).

    fn map_of(items: &[(&str, HwndStatus)]) -> HashMap<String, HwndStatus> {
        items.iter().map(|(s, st)| (s.to_string(), *st)).collect()
    }

    #[test]
    fn plan_no_browsers_is_empty() {
        let plan = plan_reconcile(
            &map_of(&[]),
            &set_of(&[]),
            &set_of(&[]),
            &set_of(&[]),
            &set_of(&[]),
            false,
        );
        assert_eq!(plan, ReconcilePlan::default());
    }

    #[test]
    fn plan_single_dead_zombie_closes() {
        let label = "window-pool-zombie";
        let plan = plan_reconcile(
            &map_of(&[(label, HwndStatus::Dead)]),
            &set_of(&[]),
            &set_of(&[]),
            &set_of(&[]),
            &set_of(&[]),
            false,
        );
        assert_eq!(plan.zombie_closes, vec_of(&[label]));
        assert!(plan.hostless_user.is_empty());
        assert!(plan.hostless_pool.is_empty());
        assert!(plan.freshly_promoted.is_empty());
    }

    #[test]
    fn plan_hostless_pool_kind_is_drain_gated() {
        // A hostless entry NOT in user_labels (pool-kind) goes to the
        // drain-gated bucket — outside drain, unregistering it bypasses
        // on_pool_window_destroyed bookkeeping.
        let label = "window-pool-hostless";
        let plan = plan_reconcile(
            &map_of(&[(label, HwndStatus::Hostless)]),
            &set_of(&[]),
            &set_of(&[]),
            &set_of(&[]),
            &set_of(&[]),
            false,
        );
        assert_eq!(plan.hostless_pool, vec_of(&[label]));
        assert!(plan.hostless_user.is_empty());
    }

    #[test]
    fn plan_hostless_user_kind_is_unconditional_repair() {
        // A hostless entry whose reducer kind is a live user window is the
        // stale projection that blocks reconcile_quit forever — it goes to
        // the unconditional-repair bucket regardless of anything else.
        let label = "main";
        let plan = plan_reconcile(
            &map_of(&[(label, HwndStatus::Hostless)]),
            &set_of(&[label]),
            &set_of(&[]),
            &set_of(&[]),
            &set_of(&[]),
            false,
        );
        assert_eq!(plan.hostless_user, vec_of(&[label]));
        assert!(plan.hostless_pool.is_empty());
    }

    #[test]
    fn plan_freshly_promoted_is_diagnostic_only() {
        // Race B: live HWND, NOT in pool sets, NOT in shadow —
        // pre-echo-promotion. Diagnostic bucket only; it must not be closed.
        // Drain-blocking is the reducer's job now: promotion flipped its
        // kind to is_pool:false at dispatch time (reducer/pool.rs), so
        // count_live_user_windows counts it and reconcile_quit answers None.
        let label = "window-pool-just-promoted";
        let plan = plan_reconcile(
            &map_of(&[(label, HwndStatus::Live)]),
            &set_of(&[label]), // reducer already counts it as user-kind
            &set_of(&[]),
            &set_of(&[]),
            &set_of(&[]),
            false,
        );
        assert_eq!(plan.freshly_promoted, vec_of(&[label]));
        assert!(plan.zombie_closes.is_empty());
        assert!(plan.hostless_user.is_empty());
        assert!(plan.hostless_pool.is_empty());
    }

    #[test]
    fn plan_live_pool_inventory_produces_no_action() {
        // Warm (queue) and spawning (unpromoted) pool slots are Stage 1's
        // job when a drain fires — the planner no longer duplicates that
        // mechanism with per-entry closes.
        let ready = "window-pool-ready";
        let spawning = "window-pool-spawning";
        let plan = plan_reconcile(
            &map_of(&[(ready, HwndStatus::Live), (spawning, HwndStatus::Live)]),
            &set_of(&[]),
            &set_of(&[]),
            &set_of(&[ready]),
            &set_of(&[spawning]),
            false,
        );
        assert_eq!(plan, ReconcilePlan::default());
    }

    #[test]
    fn plan_promoted_window_in_shadow_is_left_alone() {
        // A `window-pool-*` label that's in shadow is a promoted user
        // window (kept its prefix). Live HWND. No action, no diagnostic.
        let promoted = "window-pool-active";
        let plan = plan_reconcile(
            &map_of(&[(promoted, HwndStatus::Live)]),
            &set_of(&[promoted]),
            &set_of(&[promoted]),
            &set_of(&[]),
            &set_of(&[]),
            false,
        );
        assert_eq!(plan, ReconcilePlan::default());
    }

    #[test]
    fn plan_v0_33_643_reproduction() {
        let z1 = "window-pool-722b6186bb6e42378b48b7068c0d54b0";
        let z2 = "window-pool-b4e20337180247bdbd7408ddd7754b78";
        let plan = plan_reconcile(
            &map_of(&[(z1, HwndStatus::Dead), (z2, HwndStatus::Dead)]),
            &set_of(&[]),
            &set_of(&[]),
            &set_of(&[]),
            &set_of(&[]),
            false,
        );
        assert_eq!(plan.zombie_closes, vec_of(&[z1, z2]));
    }

    #[test]
    fn plan_hostless_takes_precedence_over_pool_membership() {
        // Hostless takes precedence over pool-state classification: an
        // unpromoted-pool slot that lost its BrowserHost needs the
        // UnregisterBrowser repair (drain-gated bucket), not a close.
        let label = "window-pool-hostless-unpromoted";
        let plan = plan_reconcile(
            &map_of(&[(label, HwndStatus::Hostless)]),
            &set_of(&[]),
            &set_of(&[]),
            &set_of(&[]),
            &set_of(&[label]),
            false,
        );
        assert_eq!(plan.hostless_pool, vec_of(&[label]));
        assert!(plan.zombie_closes.is_empty());
    }

    #[test]
    fn plan_non_pool_zombie_closes_too() {
        // A regular `main`/`window-X` top-level can crash. The launcher's
        // apply_hwnd_destroyed emits HostShouldQuit; the planner must reap
        // that zombie too, not just `window-pool-*` ones.
        let main = "main";
        let plan = plan_reconcile(
            &map_of(&[(main, HwndStatus::Dead)]),
            &set_of(&[main]),
            &set_of(&[]),
            &set_of(&[]),
            &set_of(&[]),
            false,
        );
        assert_eq!(plan.zombie_closes, vec_of(&[main]));
    }

    #[test]
    fn plan_zombie_closes_even_alongside_live_user_window() {
        // Dead zombies always plan a close — close_browser(force=1) drives
        // the host's own cleanup chain, whose request_drain consumption
        // correctly answers None while the live window remains.
        let user = "main";
        let zombie = "window-pool-zombie";
        let plan = plan_reconcile(
            &map_of(&[(user, HwndStatus::Live), (zombie, HwndStatus::Dead)]),
            &set_of(&[user]),
            &set_of(&[user]),
            &set_of(&[]),
            &set_of(&[]),
            false,
        );
        assert_eq!(plan.zombie_closes, vec_of(&[zombie]));
        assert!(plan.freshly_promoted.is_empty());
    }

    #[test]
    fn plan_pending_creation_defers_zombie_close() {
        // When a user creation is in flight, even the zombie's
        // on_before_close cleanup chain can race the pending creation: if
        // the zombie is the last live-counted browser, its close-time
        // request_drain fires and Stage-2 quit could land before the new
        // window registers. Defer zombie reap until the next
        // HostShouldQuit. (The DRAIN verdict is separately protected inside
        // reconcile_quit by user_creation_in_flight.)
        let zombie = "window-pool-zombie";
        let plan = plan_reconcile(
            &map_of(&[(zombie, HwndStatus::Dead)]),
            &set_of(&[]),
            &set_of(&[]),
            &set_of(&[]),
            &set_of(&[]),
            true,
        );
        assert!(plan.zombie_closes.is_empty(), "zombie close must defer when creation pending: {:?}", plan.zombie_closes);
    }

    #[test]
    fn plan_pending_creation_does_not_defer_hostless_repair() {
        // Hostless repair is a pure reducer dispatch — it triggers no close
        // chain, so it cannot race a pending creation the way zombie closes
        // can. (The drain verdict itself is still deferred by
        // reconcile_quit's user_creation_in_flight.)
        let hostless = "main";
        let plan = plan_reconcile(
            &map_of(&[(hostless, HwndStatus::Hostless)]),
            &set_of(&[hostless]),
            &set_of(&[]),
            &set_of(&[]),
            &set_of(&[]),
            true,
        );
        assert_eq!(plan.hostless_user, vec_of(&[hostless]));
    }

    #[test]
    fn plan_mixed_full_state_space() {
        // Composite touching every bucket: dead zombie + hostless-pool +
        // hostless-user + unpromoted pool + ready pool + freshly-promoted +
        // promoted user window.
        let zombie = "window-pool-z";
        let hostless_pool = "window-pool-h";
        let hostless_user = "window-h-user";
        let unpromoted = "window-pool-u";
        let ready = "window-pool-r";
        let fresh = "window-pool-f";
        let promoted = "window-pool-p";
        let plan = plan_reconcile(
            &map_of(&[
                (zombie, HwndStatus::Dead),
                (hostless_pool, HwndStatus::Hostless),
                (hostless_user, HwndStatus::Hostless),
                (unpromoted, HwndStatus::Live),
                (ready, HwndStatus::Live),
                (fresh, HwndStatus::Live),
                (promoted, HwndStatus::Live),
            ]),
            &set_of(&[hostless_user, fresh, promoted]),
            &set_of(&[promoted]),
            &set_of(&[ready]),
            &set_of(&[unpromoted]),
            false,
        );
        assert_eq!(plan.zombie_closes, vec_of(&[zombie]));
        assert_eq!(plan.hostless_pool, vec_of(&[hostless_pool]));
        assert_eq!(plan.hostless_user, vec_of(&[hostless_user]));
        assert_eq!(plan.freshly_promoted, vec_of(&[fresh]));
    }

    #[test]
    fn plan_idempotent_under_repeat() {
        let label = "window-pool-zombie";
        let inputs = (
            map_of(&[(label, HwndStatus::Dead)]),
            set_of(&[]),
            set_of(&[]),
            set_of(&[]),
            set_of(&[]),
        );
        let p1 = plan_reconcile(&inputs.0, &inputs.1, &inputs.2, &inputs.3, &inputs.4, false);
        let p2 = plan_reconcile(&inputs.0, &inputs.1, &inputs.2, &inputs.3, &inputs.4, false);
        assert_eq!(p1, p2);
    }
}
