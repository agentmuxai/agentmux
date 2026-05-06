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
    let unpromoted = state.unpromoted_pool_labels_snapshot();

    let mut to_close: Vec<(String, Browser)> = Vec::new();
    let mut deferred_live_candidates: Vec<String> = Vec::new();
    for (label, browser) in browser_pairs.iter() {
        if !candidate_set.contains(label.as_str()) {
            continue;
        }
        if hwnd_is_dead_or_missing(browser) {
            to_close.push((label.clone(), browser.clone()));
        } else {
            deferred_live_candidates.push(label.clone());
        }
    }

    if !deferred_live_candidates.is_empty() {
        tracing::info!(
            target: "wrr",
            "[orphan-reconcile] {} candidate(s) have live HWNDs — likely freshly-promoted, skipping: {:?}",
            deferred_live_candidates.len(),
            deferred_live_candidates
        );
    }

    if to_close.is_empty() {
        tracing::info!(
            target: "wrr",
            "[orphan-reconcile] no zombies after HWND-validity filter — nothing to close"
        );
        return;
    }

    // Compute remaining live user browsers AFTER removing the zombies
    // we're about to close. If anything remains, a stale
    // `HostShouldQuit` raced with an active user session — close the
    // zombies but leave drain mode untouched.
    let zombie_labels: HashSet<&str> = to_close.iter().map(|(l, _)| l.as_str()).collect();
    let live_user_count = browser_pairs
        .iter()
        .filter(|(label, _)| {
            !zombie_labels.contains(label.as_str())
                && !unpromoted.contains(label.as_str())
                && !label.starts_with("browser-pane-")
        })
        .count();

    if live_user_count == 0 {
        // Set drain BEFORE posting closes. Each close re-enters
        // `on_pool_window_destroyed`, which checks `quit_state` to
        // decide whether to refill. Without this, the reconciler
        // closes zombies and the refill path immediately respawns
        // them. `BeginDrain` is idempotent — a parallel cascade
        // having already dispatched it is a no-op.
        state.host_dispatch(crate::reducer::HostCommand::BeginDrain {
            reason: crate::state::QuitReason::LastWindowClosed,
        });

        // Stage-1 mirror: when the normal cascade in
        // `on_before_close` ran with `user_browser_count > 0` (because
        // zombies inflated the count), it skipped its `window-pool-*`
        // close loop. Those warm-pool browsers are still in
        // `browser_list`; Stage 2 won't fire `quit_message_loop` until
        // they're gone. Append them to `to_close` so the close pass
        // below clears the same set the normal cascade would have.
        let already_in_close: HashSet<String> =
            to_close.iter().map(|(l, _)| l.clone()).collect();
        for (label, browser) in browser_pairs.iter() {
            if !label.starts_with("window-pool-") {
                continue;
            }
            if already_in_close.contains(label) {
                continue;
            }
            // deferred_live can't appear here: live_user_count==0
            // implies deferred_live_candidates is empty (a
            // freshly-promoted pool window is itself a live user
            // window and would have bumped the count).
            to_close.push((label.clone(), browser.clone()));
        }
    } else {
        tracing::info!(
            target: "wrr",
            "[orphan-reconcile] {} live user browser(s) remain after excluding zombies — skipping BeginDrain and warm-pool close",
            live_user_count
        );
    }

    tracing::warn!(
        target: "wrr",
        "[orphan-reconcile] closing {} window-pool-* browser(s): {:?}",
        to_close.len(),
        to_close.iter().map(|(l, _)| l).collect::<Vec<_>>()
    );

    for (i, (label, browser)) in to_close.into_iter().enumerate() {
        close_one(state, i, &label, browser);
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
fn close_one(state: &Arc<AppState>, idx: usize, label: &str, mut browser: Browser) {
    if let Some(host) = browser.host() {
        host.close_browser(1); // force_close = true
        tracing::debug!(
            target: "wrr-trace",
            "[orphan-reconcile][{}] close_browser(force=1) label={}",
            idx, label
        );
    } else {
        tracing::warn!(
            target: "wrr",
            "[orphan-reconcile][{}] browser host=None label={} — already torn down, dispatching UnregisterBrowser",
            idx, label
        );
        state.host_dispatch(crate::reducer::HostCommand::UnregisterBrowser {
            label: label.to_string(),
        });
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
