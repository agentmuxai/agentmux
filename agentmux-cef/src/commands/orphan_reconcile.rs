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

use std::collections::HashSet;
use std::sync::Arc;

use cef::*;

use crate::state::AppState;

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

    if candidates.is_empty() {
        tracing::info!(
            target: "wrr",
            "[orphan-reconcile] no candidates — host is consistent; cascade should already be in flight"
        );
        return;
    }

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
    let candidate_set: HashSet<&str> = candidates.iter().map(|s| s.as_str()).collect();
    let browser_pairs = state.list_browsers();
    // Re-snapshot shadow on the UI thread. `shadow_window_meta` is
    // the launcher's authoritative set of user-visible labels —
    // `ReportWindowOpened` is sent only at promotion time, so pool
    // inventory in any state (`unpromoted` OR `queue`) is absent
    // from shadow, and promoted pool windows (which retain the
    // `window-pool-*` prefix) ARE in shadow.
    let shadow_keys: HashSet<String> = state
        .shadow_window_meta
        .lock()
        .keys()
        .cloned()
        .collect();

    // Live user browsers = shadow members minus pane labels.
    // Zombies (Race C) had their HWND destroyed; the launcher's
    // `apply_hwnd_destroyed` removed them from `state.windows`, so
    // they're absent from shadow already — no need to subtract them
    // separately. Freshly-promoted (Race B) windows whose
    // `WindowOpened` echo hasn't returned yet are also absent from
    // shadow but DO need to keep the host alive: handled below by
    // detecting that case (candidate, live HWND, NOT in pool queue)
    // and skipping drain.
    let live_user_count = browser_pairs
        .iter()
        .filter(|(label, _)| {
            shadow_keys.contains(label) && !label.starts_with("browser-pane-")
        })
        .count();

    // Pool queue membership distinguishes ready warm-pool inventory
    // (in queue, drainable on shutdown) from freshly-promoted user
    // windows (popped from queue + waiting for shadow echo).
    let pool_queue: HashSet<String> = state
        .host_state
        .lock()
        .pool
        .queue
        .iter()
        .cloned()
        .collect();

    let mut zombies: Vec<(String, Browser)> = Vec::new();
    let mut ready_pool: Vec<(String, Browser)> = Vec::new();
    let mut freshly_promoted: Vec<String> = Vec::new();
    for (label, browser) in browser_pairs.iter() {
        if !candidate_set.contains(label.as_str()) {
            continue;
        }
        if hwnd_is_dead_or_missing(browser) {
            zombies.push((label.clone(), browser.clone()));
        } else if pool_queue.contains(label) {
            ready_pool.push((label.clone(), browser.clone()));
        } else {
            freshly_promoted.push(label.clone());
        }
    }

    if !freshly_promoted.is_empty() {
        tracing::info!(
            target: "wrr",
            "[orphan-reconcile] {} candidate(s) appear freshly-promoted (live HWND, not in pool queue), skipping: {:?}",
            freshly_promoted.len(),
            freshly_promoted
        );
    }

    let mut to_close: Vec<(String, Browser)> = zombies;

    // A freshly-promoted candidate (live HWND, not in pool.queue) is
    // a live user window whose `WindowOpened` echo hasn't returned
    // yet — it's not in shadow, so it doesn't bump `live_user_count`,
    // but draining around it would freeze its session (no transition
    // back from Draining to Running). Defer to the next
    // `HostShouldQuit` once shadow catches up.
    let safe_to_drain = live_user_count == 0 && freshly_promoted.is_empty();

    if safe_to_drain {
        // Set drain BEFORE posting closes. Each close re-enters
        // `on_pool_window_destroyed`, which checks `quit_state` to
        // decide whether to refill. Without this, the reconciler
        // closes zombies and the refill path immediately respawns
        // them. `BeginDrain` is idempotent — a parallel cascade
        // having already dispatched it is a no-op.
        state.host_dispatch(crate::reducer::HostCommand::BeginDrain {
            reason: crate::state::QuitReason::LastWindowClosed,
        });

        // Drain branch: also close every other `window-pool-*`
        // browser still in `browser_pairs` (ready pool + unpromoted
        // pool inventory). The normal Stage-1 cascade in
        // `on_before_close` would have done this had it run; in
        // some HostShouldQuit paths it didn't, so the reconciler
        // closes them here. Stage 2's `quit_message_loop` waits on
        // `browser_list.is_empty()`; missing this loop is the most
        // common reason the host stays alive after the user's last
        // window closes.
        let already_in_close: HashSet<String> =
            to_close.iter().map(|(l, _)| l.clone()).collect();
        for (label, browser) in browser_pairs.iter() {
            if !label.starts_with("window-pool-") {
                continue;
            }
            if already_in_close.contains(label) {
                continue;
            }
            // freshly_promoted (live HWND, not in pool.queue) MUST
            // NOT be closed — they're real new user windows whose
            // shadow echo is in flight. Skip them. ready_pool and
            // unpromoted-pool slots fall through to `to_close`.
            if freshly_promoted.iter().any(|l| l == label) {
                continue;
            }
            to_close.push((label.clone(), browser.clone()));
        }

        let _ = ready_pool; // included via the loop above
    } else {
        tracing::info!(
            target: "wrr",
            "[orphan-reconcile] {} live user browser(s) + {} freshly-promoted candidate(s) — skipping BeginDrain and warm-pool close",
            live_user_count,
            freshly_promoted.len()
        );
        // Stale `HostShouldQuit` (or pre-shadow-echo race): any
        // ready-pool / freshly-promoted candidates stay alive; only
        // zombies get closed (already in `to_close`).
        let _ = ready_pool;
    }

    if to_close.is_empty() {
        tracing::info!(
            target: "wrr",
            "[orphan-reconcile] nothing to close after classification"
        );
        return;
    }

    tracing::warn!(
        target: "wrr",
        "[orphan-reconcile] closing {} window-pool-* browser(s): {:?}",
        to_close.len(),
        to_close.iter().map(|(l, _)| l).collect::<Vec<_>>()
    );

    let mut any_hostless_unregistered = false;
    for (i, (label, browser)) in to_close.into_iter().enumerate() {
        if close_one(state, i, &label, browser) {
            any_hostless_unregistered = true;
        }
    }

    // Hostless orphans don't get an on_before_close callback to drive
    // the stage-2 quit_message_loop in client/mod.rs. If we just
    // unregistered such an entry AND the reducer's browser registry
    // is now empty AND we entered drain (`safe_to_drain`), no future
    // CEF callback will satisfy the quit gate. Drive it ourselves on
    // this UI-thread task — same call site, same thread, just no
    // `on_before_close` to ride on. Gated on `safe_to_drain` instead
    // of just `live_user_count == 0` so a freshly-promoted live
    // window isn't terminated mid-handshake.
    if any_hostless_unregistered
        && safe_to_drain
        && state.host_state.lock().browsers.is_empty()
    {
        tracing::warn!(
            target: "wrr",
            "[orphan-reconcile] hostless orphans unregistered + browsers map empty — driving quit_message_loop"
        );
        quit_message_loop();
    }
}

/// Dispatch close for a single browser. Already on UI thread when
/// invoked — calling `BrowserHost::close_browser` directly is safe.
/// We prefer it over PostMessageW because it works regardless of
/// whether the underlying HWND is still alive.
///
/// If `browser.host()` returns None, the BrowserHost has already
/// gone away. We can't drive `close_browser` to trigger
/// `on_before_close` (which would normally dispatch
/// `UnregisterBrowser` to clean up `state.browsers`), so the
/// reconciler dispatches `UnregisterBrowser` itself. Without that,
/// the entry sits in `state.browsers` indefinitely; nothing else
/// will ever remove it because the upstream callback chain is
/// already gone.
/// Returns true iff the hostless-unregister branch was taken — the
/// caller uses that signal to decide whether to drive
/// `quit_message_loop` itself (no `on_before_close` callback will
/// arrive for hostless browsers).
fn close_one(state: &Arc<AppState>, idx: usize, label: &str, mut browser: Browser) -> bool {
    if let Some(host) = browser.host() {
        host.close_browser(1); // force_close = true
        tracing::debug!(
            target: "wrr-trace",
            "[orphan-reconcile][{}] close_browser(force=1) label={}",
            idx, label
        );
        false
    } else {
        tracing::warn!(
            target: "wrr",
            "[orphan-reconcile][{}] browser host=None label={} — already torn down, dispatching UnregisterBrowser",
            idx, label
        );
        state.host_dispatch(crate::reducer::HostCommand::UnregisterBrowser {
            label: label.to_string(),
        });
        true
    }
}

/// True when the browser's underlying HWND is missing or destroyed.
/// Used by the UI-thread runner to keep freshly-promoted (live-HWND)
/// candidates out of the close set.
///
/// Windows: `IsWindow` returns 0 for destroyed HWNDs even when the
/// stale handle value is non-null — the discriminator we need.
///
/// Non-Windows: only the null-handle check is available. If a
/// platform produces stale-but-non-null HWNDs, a real zombie would
/// be skipped here. The Windows-specific zombie scenario is the
/// one we have evidence for; revisit if it reproduces on
/// macOS/Linux.
fn hwnd_is_dead_or_missing(browser: &Browser) -> bool {
    let mut b = browser.clone();
    let Some(host) = b.host() else { return true };
    let wh = host.window_handle();
    if wh.0.is_null() {
        return true;
    }
    #[cfg(target_os = "windows")]
    unsafe {
        use windows_sys::Win32::Foundation::HWND;
        use windows_sys::Win32::UI::WindowsAndMessaging::IsWindow;
        IsWindow(wh.0 as HWND) == 0
    }
    #[cfg(not(target_os = "windows"))]
    {
        false
    }
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
