// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Host-side orphan-instance reconciliation. Spec:
//! `docs/specs/SPEC_HOST_ORPHAN_RECONCILIATION_2026_05_05.md`.
//!
//! When the launcher detects that the last user-visible window
//! has closed but the host is still alive, it emits
//! `Event::HostShouldQuit`. Pre-this-module, the host's handler
//! was log-only because three prior attempts at making it deliver
//! UI-thread work all failed (post_task drops, direct call UB,
//! PostThreadMessage(WM_QUIT) ignored — see comment at
//! `launcher_ipc.rs:421`).
//!
//! This module wires the event to a real corrective action without
//! reintroducing those hazards: snapshot host state, drop the lock,
//! `PostMessageW(WM_CLOSE)` (or `post_task(close_browser)` fallback)
//! to each orphan `window-pool-*` browser. The two-stage cascade in
//! `client/mod.rs::on_before_close` then funnels each close back
//! through the existing path that empties `browser_list` and fires
//! `quit_message_loop`.

use std::collections::HashSet;
use std::sync::Arc;

use crate::state::AppState;

/// Classify which labels are *candidate* orphan `window-pool-*`
/// entries — promoted out of the pool but not tracked by the
/// launcher's `shadow_window_meta` mirror.
///
/// This is a NECESSARY but not SUFFICIENT condition. The orchestrator
/// must additionally check HWND validity before dispatching close,
/// because a freshly-promoted pool window will briefly satisfy this
/// classification (label removed from `unpromoted_pool`, but the
/// launcher's `WindowOpened` echo hasn't yet populated shadow). Codex
/// #702 round 1 flagged the missing post-classify check; round 2's
/// attempt to fix it via local `window_meta` union failed because
/// local meta is also stale in the actual zombie case (host's
/// `on_before_close` never runs, so the entry is never cleared) —
/// codex round 2 P1. The HWND-validity check at the orchestrator
/// level is the right discriminator.
///
/// Pure function over snapshot inputs, so tests don't need CEF.
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
/// classifies orphans, and dispatches a close to each via the same
/// PostMessageW / post_task channel used by the two-stage cascade.
///
/// Idempotent: a second call after the first has dispatched closes
/// will see the same orphan set until CEF actually destroys those
/// browsers, then will see an empty set and return early. Duplicate
/// `WM_CLOSE` messages are coalesced by Windows.
pub fn reconcile_and_drain(state: &Arc<AppState>) {
    let browser_pairs = state.list_browsers();
    let unpromoted = state.unpromoted_pool_labels_snapshot();
    // Shadow alone is the source of truth for "launcher tracks this
    // as live". Local `window_meta` is unreliable here because it's
    // populated in `on_after_created` and only cleared in
    // `on_before_close` — the v0.33.643 zombie case, by definition,
    // is the one where `on_before_close` never ran, so a local-meta
    // union would skip exactly the labels we need to reap (codex #702
    // round 2 P1).
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

    // Discriminate Race B (just-promoted, launcher echo lag — HWND
    // valid, must NOT close) from Race C (zombie, on_before_close
    // never ran — HWND destroyed, must close) via Win32 IsWindow on
    // the browser's underlying handle. See spec §5.4. Each iteration
    // takes one HWND check; cheap.
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
/// Used to distinguish Race B (live HWND — freshly-promoted pool
/// window, must NOT close) from Race C (zombie — `on_before_close`
/// never ran, HWND was destroyed without clearing host state, must
/// close).
///
/// Windows: `IsWindow` returns 0 for destroyed HWNDs, even when the
/// stale handle value is still around. This is the discriminator.
///
/// Non-Windows: we only have the null-handle check. Race B/C
/// distinction is best-effort — if the platform ever produces a
/// stale-but-non-null HWND, we'd skip a real zombie. The v0.33.643
/// case is Windows-specific so this is acceptable for now; revisit
/// if the bug reproduces on macOS/Linux.
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
    // Use the same liveness probe `hwnd_is_dead_or_missing` uses so
    // path selection here agrees with the orchestrator's discriminator.
    // A non-null but destroyed HWND must NOT take the PostMessageW
    // branch — Windows returns 0 for that and the message goes
    // nowhere, leaving the Race C zombie unclosed (codex + reagent
    // round-3 P1). Force those onto the post_task path, which calls
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
/// alive (non-null AND `IsWindow == 1`). Used by both the
/// orchestrator's discriminator and the path selection inside
/// `post_close_or_fallback` so they agree on what "live" means.
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
        // One promoted user window: in browsers, NOT in unpromoted_pool,
        // IS in shadow_window_meta. Not an orphan.
        let labels = vec_of(&["window-pool-aaa"]);
        let unpromoted = set_of(&[]);
        let shadow = set_of(&["window-pool-aaa"]);
        assert_eq!(classify_candidate_orphans(&labels, &unpromoted, &shadow), Vec::<String>::new());
    }

    #[test]
    fn classify_one_orphan_is_returned() {
        // Promoted (not in unpromoted) but launcher dropped it
        // (not in shadow). Classic orphan.
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
        // Pane labels never qualify; pane drain runs through a
        // different cascade entirely.
        let labels = vec_of(&["browser-pane-foo"]);
        let unpromoted = set_of(&[]);
        let shadow = set_of(&[]);
        assert_eq!(classify_candidate_orphans(&labels, &unpromoted, &shadow), Vec::<String>::new());
    }

    #[test]
    fn classify_skips_main_label() {
        // Plain `main` (or any non-pool prefix) is excluded — only
        // promoted `window-pool-*` entries are reaped here.
        let labels = vec_of(&["main"]);
        let unpromoted = set_of(&[]);
        let shadow = set_of(&[]);
        assert_eq!(classify_candidate_orphans(&labels, &unpromoted, &shadow), Vec::<String>::new());
    }

    #[test]
    fn classify_returns_multiple_orphans_in_input_order() {
        let labels = vec_of(&[
            "window-pool-aaa",   // shadow tracked → not orphan
            "window-pool-bbb",   // orphan
            "window-pool-ccc",   // unpromoted → not orphan
            "window-pool-ddd",   // orphan
            "main",              // not pool prefix
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
        // codex #702 round 1: in the WindowOpened-echo lag, a freshly-
        // promoted pool window is in `browsers` (not in unpromoted)
        // and the launcher's mirror hasn't caught up yet (not in
        // shadow). The classifier returns it as a CANDIDATE; the
        // orchestrator's HWND-validity check is what saves it from
        // being closed (live HWND → skip). We test the negative side
        // here — that we do NOT silently skip it at the classifier
        // layer, because round-2's local-window-meta union (codex
        // round 2 P1) would also skip the actual v0.33.643 zombie.
        let labels = vec_of(&["window-pool-just-promoted"]);
        let unpromoted = set_of(&[]);
        let shadow = set_of(&[]);
        assert_eq!(
            classify_candidate_orphans(&labels, &unpromoted, &shadow),
            vec_of(&["window-pool-just-promoted"])
        );
    }

    #[test]
    fn classify_handles_user_v0_33_643_scenario() {
        // Drift snapshot from the launcher log:
        //   DRIFT Pool: host=0 mirror=2
        // Two `window-pool-*` labels still in browsers, both promoted
        // (host's pool.queue is empty so unpromoted is empty), and the
        // launcher's mirror has dropped them (shadow is empty).
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
