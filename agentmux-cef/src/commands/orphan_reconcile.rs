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

/// Classify which labels in `browser_labels` are orphan
/// `window-pool-*` entries — i.e., promoted out of the pool but no
/// longer tracked by *any* window-metadata source (launcher mirror
/// or host's local handoff cache).
///
/// `tracked_window_meta_keys` MUST be the union of `shadow_window_meta`
/// (launcher-fed projection) AND host's local `window_meta` (the eager
/// pre-create handoff that catches the race window where a pool window
/// has just been promoted but the launcher's `WindowOpened` echo hasn't
/// returned yet, see `state.rs::window_meta` doc + Phase B.5c). Without
/// the union, a `HostShouldQuit` event in flight when the user opens a
/// new window would classify the freshly-promoted pool window as an
/// orphan and post `WM_CLOSE` to it (codex #702 P1).
///
/// Pure function over snapshot inputs, so tests don't need CEF.
pub(crate) fn classify_orphan_labels(
    browser_labels: &[String],
    unpromoted_pool: &HashSet<String>,
    tracked_window_meta_keys: &HashSet<String>,
) -> Vec<String> {
    browser_labels
        .iter()
        .filter(|label| {
            label.starts_with("window-pool-")
                && !unpromoted_pool.contains(label.as_str())
                && !tracked_window_meta_keys.contains(label.as_str())
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
    // Union of launcher-fed mirror + host's local pre-create handoff
    // cache. The local cache covers the race window where a pool window
    // has just been promoted (label removed from `unpromoted_pool`,
    // `ReportWindowOpened` queued) but the launcher's `WindowOpened`
    // echo hasn't returned yet to update `shadow_window_meta`. Without
    // this union, a stale `HostShouldQuit` from the previous last-close
    // would WM_CLOSE the freshly-promoted live window (codex #702 P1).
    let mut tracked: HashSet<String> = state
        .shadow_window_meta
        .lock()
        .keys()
        .cloned()
        .collect();
    tracked.extend(state.window_meta.lock().keys().cloned());
    let labels: Vec<String> = browser_pairs.iter().map(|(l, _)| l.clone()).collect();
    let orphan_labels = classify_orphan_labels(&labels, &unpromoted, &tracked);

    if orphan_labels.is_empty() {
        tracing::info!(
            target: "wrr",
            "[orphan-reconcile] no orphans — host is consistent; cascade should already be in flight"
        );
        return;
    }

    tracing::warn!(
        target: "wrr",
        "[orphan-reconcile] reaping {} orphan window-pool-* browser(s): {:?}",
        orphan_labels.len(),
        orphan_labels
    );

    // Pull just the orphan Browser handles out of the snapshot. The
    // labels list was built from the same snapshot so each orphan
    // label has a corresponding entry; if a parallel `WindowClosed`
    // already removed one between the snapshot and here, the lookup
    // misses and we just skip — the close already happened.
    let orphan_browsers: Vec<(String, cef::Browser)> = browser_pairs
        .into_iter()
        .filter(|(label, _)| orphan_labels.contains(label))
        .collect();
    for (i, (label, browser)) in orphan_browsers.into_iter().enumerate() {
        post_close_or_fallback(i, &label, browser);
    }
}

#[cfg(target_os = "windows")]
fn post_close_or_fallback(idx: usize, label: &str, mut browser: cef::Browser) {
    // Bring CEF wrapper traits into scope so `.host()` and
    // `.window_handle()` resolve on the FFI types — same pattern as
    // `client/mod.rs` (`use cef::*;`).
    use cef::*;
    use windows_sys::Win32::Foundation::HWND;
    use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};
    let hwnd_opt = browser.host().and_then(|h: BrowserHost| {
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
            "[orphan-reconcile][{}] PostMessage(hwnd={:p}, WM_CLOSE) label={} ok={}",
            idx, hwnd, label, ok != 0
        );
    } else {
        let mut task = crate::client::ClosePoolBrowserTask::new(browser);
        let posted = cef::post_task(cef::ThreadId::UI, Some(&mut task));
        tracing::warn!(
            target: "wrr",
            "[orphan-reconcile][{}] hwnd=null; fell back to post_task(close_browser) label={} posted={}",
            idx, label, posted != 0
        );
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
        assert_eq!(classify_orphan_labels(&labels, &unpromoted, &shadow), Vec::<String>::new());
    }

    #[test]
    fn classify_one_orphan_is_returned() {
        // Promoted (not in unpromoted) but launcher dropped it
        // (not in shadow). Classic orphan.
        let labels = vec_of(&["window-pool-bbb"]);
        let unpromoted = set_of(&[]);
        let shadow = set_of(&[]);
        assert_eq!(
            classify_orphan_labels(&labels, &unpromoted, &shadow),
            vec_of(&["window-pool-bbb"])
        );
    }

    #[test]
    fn classify_skips_unpromoted_pool_members() {
        // Still in unpromoted_pool — warm pool member, not orphan.
        let labels = vec_of(&["window-pool-ccc"]);
        let unpromoted = set_of(&["window-pool-ccc"]);
        let shadow = set_of(&[]);
        assert_eq!(classify_orphan_labels(&labels, &unpromoted, &shadow), Vec::<String>::new());
    }

    #[test]
    fn classify_skips_browser_pane_labels() {
        // Pane labels never qualify; pane drain runs through a
        // different cascade entirely.
        let labels = vec_of(&["browser-pane-foo"]);
        let unpromoted = set_of(&[]);
        let shadow = set_of(&[]);
        assert_eq!(classify_orphan_labels(&labels, &unpromoted, &shadow), Vec::<String>::new());
    }

    #[test]
    fn classify_skips_main_label() {
        // Plain `main` (or any non-pool prefix) is excluded — only
        // promoted `window-pool-*` entries are reaped here.
        let labels = vec_of(&["main"]);
        let unpromoted = set_of(&[]);
        let shadow = set_of(&[]);
        assert_eq!(classify_orphan_labels(&labels, &unpromoted, &shadow), Vec::<String>::new());
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
            classify_orphan_labels(&labels, &unpromoted, &shadow),
            vec_of(&["window-pool-bbb", "window-pool-ddd"])
        );
    }

    #[test]
    fn classify_skips_label_in_local_window_meta_only() {
        // codex #702 P1 race: user just promoted a pool window. The
        // host has already removed it from unpromoted_pool and queued
        // ReportWindowOpened, but the launcher's WindowOpened echo
        // hasn't arrived yet, so shadow_window_meta is empty. The
        // host's *local* window_meta has the entry though (eager
        // pre-create handoff). The reconciler MUST treat that as
        // "tracked" so we don't WM_CLOSE a live new window.
        //
        // The caller (`reconcile_and_drain`) builds `tracked` as the
        // union of shadow + local; this test pretends the union is
        // local-only (the freshly-promoted case).
        let labels = vec_of(&["window-pool-newly-promoted"]);
        let unpromoted = set_of(&[]);
        let tracked = set_of(&["window-pool-newly-promoted"]);
        assert_eq!(classify_orphan_labels(&labels, &unpromoted, &tracked), Vec::<String>::new());
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
        let orphans = classify_orphan_labels(&labels, &unpromoted, &shadow);
        assert_eq!(orphans.len(), 2);
        assert_eq!(orphans, labels);
    }
}
