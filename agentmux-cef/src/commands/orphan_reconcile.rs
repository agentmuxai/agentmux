// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Host-side orphan-instance reconciliation. Spec:
//! `docs/specs/SPEC_HOST_ORPHAN_RECONCILIATION_2026_05_05.md`.
//!
//! When the launcher detects that the last user-visible window
//! has closed but the host is still alive, it emits
//! `Event::HostShouldQuit`. The host's handler invokes the
//! reconciler here, which closes any orphan `window-pool-*`
//! browsers (promoted out of the warm pool, but the launcher
//! mirror has dropped them — typically because their HWND was
//! destroyed without the host's `on_before_close` running).
//!
//! Each close is dispatched via `PostMessageW(WM_CLOSE)` (live
//! HWND) or `cef::post_task(close_browser)` (dead/missing HWND).
//! Both routes funnel back through `client::on_before_close`,
//! whose Stage 2 hook fires `quit_message_loop()` once
//! `browser_list` empties — so the reconciler doesn't have to
//! drive UI-thread shutdown directly. Earlier attempts at doing
//! that from this IPC thread all hung CEF (see
//! `launcher_ipc.rs::HostShouldQuit` comment).

use std::collections::HashSet;
use std::sync::Arc;

use crate::state::AppState;

/// Classify which labels are *candidate* orphans — promoted
/// `window-pool-*` entries not tracked by the launcher's
/// `shadow_window_meta` mirror. Necessary but not sufficient: a
/// freshly-promoted pool window briefly satisfies this (the
/// launcher's `WindowOpened` echo hasn't populated shadow yet),
/// so the orchestrator must additionally verify the HWND is dead
/// before dispatching close.
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

/// Snapshot-and-drop entry point. Walks the host's browser map,
/// identifies orphan `window-pool-*` zombies, sets the host into
/// drain mode, and dispatches a close to each.
///
/// Idempotent: a second call before CEF processes the closes will
/// see the same orphan set and re-dispatch; Windows coalesces
/// duplicate `WM_CLOSE` and `BeginDrain` is itself idempotent.
pub fn reconcile_and_drain(state: &Arc<AppState>) {
    let browser_pairs = state.list_browsers();
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
    let labels: Vec<String> = browser_pairs.iter().map(|(l, _)| l.clone()).collect();
    let candidates = classify_candidate_orphans(&labels, &unpromoted, &shadow_keys);

    if candidates.is_empty() {
        tracing::info!(
            target: "wrr",
            "[orphan-reconcile] no candidates — host is consistent; cascade should already be in flight"
        );
        return;
    }

    // Discriminate freshly-promoted (live HWND, must NOT close)
    // from zombie (dead HWND, must close) via Win32 `IsWindow` on
    // the browser's underlying handle. See spec §5.4.
    let candidate_set: HashSet<&str> = candidates.iter().map(|s| s.as_str()).collect();
    let mut to_close: Vec<(String, cef::Browser)> = Vec::new();
    let mut deferred_live: Vec<String> = Vec::new();
    for (label, browser) in browser_pairs.into_iter() {
        if !candidate_set.contains(label.as_str()) {
            continue;
        }
        if hwnd_is_dead_or_missing(&browser) {
            to_close.push((label, browser));
        } else {
            deferred_live.push(label);
        }
    }

    if !deferred_live.is_empty() {
        tracing::info!(
            target: "wrr",
            "[orphan-reconcile] {} candidate(s) have live HWNDs — likely freshly-promoted, skipping: {:?}",
            deferred_live.len(),
            deferred_live
        );
    }

    if to_close.is_empty() {
        tracing::info!(
            target: "wrr",
            "[orphan-reconcile] no zombies after HWND-validity filter — nothing to close"
        );
        return;
    }

    // Set drain state BEFORE posting closes. Each close re-enters
    // `on_before_close → on_pool_window_destroyed`, which checks
    // `quit_state` to decide whether to refill the pool. Without
    // this, the reconciler would close orphans and the pool refill
    // path would immediately spawn fresh `window-pool-*` browsers
    // (see `commands/window_pool.rs` quit-state guard). `BeginDrain`
    // is idempotent — already-draining state is a no-op.
    state.host_dispatch(crate::reducer::HostCommand::BeginDrain {
        reason: crate::state::QuitReason::LastWindowClosed,
    });

    tracing::warn!(
        target: "wrr",
        "[orphan-reconcile] reaping {} zombie window-pool-* browser(s): {:?}",
        to_close.len(),
        to_close.iter().map(|(l, _)| l).collect::<Vec<_>>()
    );

    for (i, (label, browser)) in to_close.into_iter().enumerate() {
        post_close_or_fallback(i, &label, browser);
    }
}

/// True when the browser's underlying HWND is missing or destroyed.
/// Used by the orchestrator to keep freshly-promoted (live-HWND)
/// candidates out of the close set.
///
/// Windows: `IsWindow` returns 0 for destroyed HWNDs even when the
/// stale handle value is non-null — the discriminator we need.
///
/// Non-Windows: only the null-handle check is available. If a
/// platform produces stale-but-non-null HWNDs, a real zombie
/// would be skipped here. The Windows-specific zombie scenario is
/// the one we have evidence for; revisit if it reproduces on
/// macOS/Linux.
fn hwnd_is_dead_or_missing(browser: &cef::Browser) -> bool {
    #[cfg(target_os = "windows")]
    {
        resolve_live_hwnd(browser).is_none()
    }
    #[cfg(not(target_os = "windows"))]
    {
        use cef::*;
        let mut b = browser.clone();
        let Some(host) = b.host() else { return true };
        host.window_handle().0.is_null()
    }
}

#[cfg(target_os = "windows")]
fn post_close_or_fallback(idx: usize, label: &str, browser: cef::Browser) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};
    // Path selection uses the same liveness probe the orchestrator
    // ran, so a non-null but destroyed HWND can't slip onto the
    // `PostMessageW` branch (which Windows silently drops). Dead
    // HWNDs route to `post_task`, which calls
    // `host.close_browser(force=1)` and works regardless of HWND.
    if let Some(hwnd) = resolve_live_hwnd(&browser) {
        let ok = unsafe { PostMessageW(hwnd, WM_CLOSE, 0, 0) };
        tracing::debug!(
            target: "wrr-trace",
            "[orphan-reconcile][{}] PostMessage(hwnd={:p}, WM_CLOSE) label={} ok={}",
            idx, hwnd, label, ok != 0
        );
    } else {
        let mut task = crate::client::ClosePoolBrowserTask::new(browser);
        let posted = cef::post_task(cef::ThreadId::UI, Some(&mut task));
        tracing::warn!(
            target: "wrr",
            "[orphan-reconcile][{}] hwnd dead/missing; fell back to post_task(close_browser) label={} posted={}",
            idx, label, posted != 0
        );
    }
}

/// Returns `Some(hwnd)` only when the browser's underlying HWND is
/// alive (non-null AND `IsWindow == 1`). Single source of truth for
/// "live HWND" — used by both `hwnd_is_dead_or_missing` and the
/// path selection in `post_close_or_fallback`.
#[cfg(target_os = "windows")]
fn resolve_live_hwnd(browser: &cef::Browser) -> Option<windows_sys::Win32::Foundation::HWND> {
    use cef::*;
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::IsWindow;
    let mut b = browser.clone();
    let host = b.host()?;
    let wh = host.window_handle();
    if wh.0.is_null() {
        return None;
    }
    let h = wh.0 as HWND;
    if unsafe { IsWindow(h) } == 0 {
        None
    } else {
        Some(h)
    }
}

#[cfg(not(target_os = "windows"))]
fn post_close_or_fallback(idx: usize, label: &str, browser: cef::Browser) {
    let mut task = crate::client::ClosePoolBrowserTask::new(browser);
    let posted = cef::post_task(cef::ThreadId::UI, Some(&mut task));
    tracing::debug!(
        target: "wrr-trace",
        "[orphan-reconcile][{}] post_task(close_browser) label={} posted={}",
        idx, label, posted != 0
    );
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
        // Promoted user window: in browsers, NOT in unpromoted_pool,
        // IS in shadow_window_meta. Not an orphan.
        let labels = vec_of(&["window-pool-aaa"]);
        let unpromoted = set_of(&[]);
        let shadow = set_of(&["window-pool-aaa"]);
        assert_eq!(classify_candidate_orphans(&labels, &unpromoted, &shadow), Vec::<String>::new());
    }

    #[test]
    fn classify_one_orphan_is_returned() {
        // Promoted (not in unpromoted) but launcher dropped it
        // (not in shadow). Classic orphan candidate.
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
        // Still in unpromoted_pool — warm pool member, not orphan.
        let labels = vec_of(&["window-pool-ccc"]);
        let unpromoted = set_of(&["window-pool-ccc"]);
        let shadow = set_of(&[]);
        assert_eq!(classify_candidate_orphans(&labels, &unpromoted, &shadow), Vec::<String>::new());
    }

    #[test]
    fn classify_skips_browser_pane_labels() {
        // Pane labels are reaped via a different cascade.
        let labels = vec_of(&["browser-pane-foo"]);
        let unpromoted = set_of(&[]);
        let shadow = set_of(&[]);
        assert_eq!(classify_candidate_orphans(&labels, &unpromoted, &shadow), Vec::<String>::new());
    }

    #[test]
    fn classify_skips_main_label() {
        // Only `window-pool-*` is reaped here.
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
        // returns it as a CANDIDATE; the orchestrator's HWND check
        // is what saves it from being closed (live HWND → skip).
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
