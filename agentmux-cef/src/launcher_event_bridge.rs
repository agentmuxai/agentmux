// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase B.7.3.1 — host outbound JS bridge for launcher typed events.
//
// Single function `dispatch_to_renderers(state, event)` called from
// `launcher_ipc::apply_event_to_shadow` after each event is applied
// to host shadows. Iterates `state.browsers`, calls
// `Frame::ExecuteJavaScript` per top-level browser to invoke
// `window.__agentmux_launcher_event(<json>)` in the renderer.
//
// Filtering: pool windows (`window-pool-*`) and browser-pane child
// HWNDs (`browser-pane-*`) are skipped. They have no UI to react.
//
// Cross-platform: `Frame::ExecuteJavaScript` is portable across
// CEF's Windows / macOS / Linux backends. No platform specifics.
//
// Phase B.7.3.3 — typed events are the SOLE channel for
// InstancePanel state. The bespoke `window-instances-changed`
// event and its 4 sync emit sites in the host are retired.
//
// See `docs/specs/SPEC_B_7_3_LAUNCHER_EVENTS_TO_RENDERER_2026_04_29.md`.

use std::sync::Arc;

use agentmux_common::ipc::Event;
use cef::{CefString, ImplBrowser, ImplFrame};

/// Forward a launcher event to every top-level renderer.
///
/// Excluded:
///   - Pool **inventory** labels (`window-pool-*` in
///     `pool.unpromoted` OR `pool.queue`): no user UI yet. Two
///     sub-states:
///       * `pool.unpromoted` — spawning, renderer not ready.
///       * `pool.queue` — renderer ready, waiting for promote.
///     Both are hidden off-screen and would build stale InstancePanel
///     state from launcher events the user never sees. The bridge
///     uses `state.user_visibility_snapshot()` which atomically
///     reads the pool inventory (unpromoted ∪ queue) and the
///     browser registry under one host_state lock — a two-lock
///     variant would race against `promote_pool_window` and let a
///     just-promoted window slip through (or, worse, count a
///     real user window in the close-cascade gate's exclusion).
///   - Browser-pane labels (`browser-pane-*`): not top-level
///     windows; have no InstancePanel.
///
/// Promoted pool windows (label still has the `window-pool-*`
/// prefix but the entry is in NEITHER pool set) ARE included —
/// they're the user-visible torn-off windows. Pre-fix, a
/// label-prefix-only check excluded them too, so torn-off windows
/// stopped receiving launcher events post-promotion (InstancePanel
/// drift, plus anything else listening to launcher events).
///
/// JSON payload uses `serde_json::to_string`, so any string content
/// from the Event is escaped against quote / backtick injection at
/// the JS-string boundary.
pub fn dispatch_to_renderers(state: &Arc<crate::state::AppState>, event: &Event) {
    // Phase F.7 host-bridge dedup. Mirror of the renderer-side guard
    // (`shouldDispatchLauncherEvent` in launcher-events.ts), but at
    // the host so a fresh V8 context post-crash, multi-context fan-
    // out, or any renderer-side guard failure mode can't amplify the
    // launcher's single emit. Skip the entire dispatch if the event's
    // version is not strictly higher than the per-key max we've
    // already sent.
    if !should_dispatch(state, event) {
        return;
    }

    let json = match serde_json::to_string(event) {
        Ok(s) => s,
        Err(e) => {
            tracing::warn!(
                target: "launcher-event-bridge",
                "[launcher-event-bridge] serialize failed: {}",
                e
            );
            return;
        }
    };

    let script = format!(
        "if (window.__agentmux_launcher_event) {{ try {{ window.__agentmux_launcher_event({}) }} catch(e) {{ console.error('[launcher-event] dispatch failed', e) }} }}",
        json
    );
    let code = CefString::from(script.as_str());
    let url = CefString::from("");

    // Atomic snapshot — pool inventory + browsers under ONE lock.
    // Two-lock variants race against `promote_pool_window` between
    // the reads.
    let (pool_inventory, browsers) = state.user_visibility_snapshot();

    for (label, browser) in browsers {
        if label.starts_with("browser-pane-") {
            continue;
        }
        if pool_inventory.contains(label.as_str()) {
            // Pool inventory (unpromoted or ready-queued) — no user
            // UI yet, skip.
            continue;
        }
        if let Some(frame) = browser.main_frame() {
            frame.execute_java_script(Some(&code), Some(&url), 0);
        }
    }
}

/// Phase F.7 dedup gate. Returns `true` if the event is strictly
/// newer for its `(event_kind, label, hwnd)` key and should be
/// dispatched; `false` if a higher-or-equal version was already
/// sent.
///
/// Mirrors the renderer-side `shouldDispatchLauncherEvent`. Bounded
/// at 4096 keys with FIFO eviction so a long-running host can't
/// leak unbounded state. Re-arrival of an evicted key bypasses the
/// host gate but the renderer guard still catches it.
fn should_dispatch(state: &Arc<crate::state::AppState>, event: &Event) -> bool {
    const MAX_DEDUP_KEYS: usize = 4096;
    let (key, version) = dedup_key(event);
    let mut cache = state.launcher_bridge_dedup.lock();
    if let Some(&seen) = cache.get(&key) {
        if version <= seen {
            return false;
        }
    }
    cache.insert(key, version);
    if cache.len() > MAX_DEDUP_KEYS {
        // FIFO eviction — drop the first key by iteration order. Map
        // iteration order isn't strictly insertion in std HashMap, but
        // for this bounded-leak guard any victim is fine.
        if let Some(victim) = cache.keys().next().cloned() {
            cache.remove(&victim);
        }
    }
    true
}

/// Build the dedup cache key + extract the version for an event.
/// Returns `("{kind}|{label}|{hwnd}", version)`. Missing label/hwnd
/// fields contribute empty strings — kinds without those fields
/// dedup by kind+version alone.
fn dedup_key(event: &Event) -> (String, u64) {
    use Event::*;
    let (kind, label, hwnd, version) = match event {
        WindowOpened { label, version, .. } => ("window_opened", label.as_str(), 0u64, *version),
        WindowClosed { label, version, .. } => ("window_closed", label.as_str(), 0, *version),
        WindowInstanceAssigned { label, version, .. } => ("window_instance_assigned", label.as_str(), 0, *version),
        WindowInstanceReleased { label, version, .. } => ("window_instance_released", label.as_str(), 0, *version),
        BackendWindowIdRegistered { label, version, .. } => ("backend_window_id_registered", label.as_str(), 0, *version),
        BackendWindowIdUnregistered { label, version, .. } => ("backend_window_id_unregistered", label.as_str(), 0, *version),
        PoolWindowAdded { label, version, .. } => ("pool_window_added", label.as_str(), 0, *version),
        PoolWindowRemoved { label, version, .. } => ("pool_window_removed", label.as_str(), 0, *version),
        PoolWindowPromoted { label, version, .. } => ("pool_window_promoted", label.as_str(), 0, *version),
        HwndDriftDetected { kind: _, label, hwnd, version, .. } => (
            "hwnd_drift_detected",
            label.as_deref().unwrap_or(""),
            hwnd.unwrap_or(0),
            *version,
        ),
        DriftDetected { version, .. } => ("drift_detected", "", 0, *version),
        CorrectiveWindowMove { hwnd, version, .. } => ("corrective_window_move", "", *hwnd, *version),
        HostShouldQuit { version, .. } => ("host_should_quit", "", 0, *version),
        // Conservative: events without obvious dedup keys still
        // benefit from the cap. Use the variant name + version with
        // empty label/hwnd; same-version duplicates drop.
        other => {
            let v = match serde_json::to_value(other).ok().and_then(|v| {
                v.get("version").and_then(|x| x.as_u64())
            }) {
                Some(v) => v,
                None => 0,
            };
            ("__catchall__", "", 0, v)
        }
    };
    (format!("{}|{}|{}", kind, label, hwnd), version)
}

#[cfg(test)]
mod bridge_dedup_tests {
    //! Phase F.7 host-bridge dedup. The bridge's job is to amplify-zero —
    //! one launcher event per `(kind, label, hwnd)` key, monotonic by
    //! version. v0.33.688 smoke surfaced a 164× amplification (single
    //! launcher emit at v=78, 164 lines logged by the renderer);
    //! these tests pin the cap.
    use super::*;
    use agentmux_common::ipc::{HwndDriftKind, Severity};
    use std::sync::Arc;

    fn fresh_state() -> Arc<crate::state::AppState> {
        Arc::new(crate::state::AppState::default())
    }

    fn drift_event(version: u64, label: &str, hwnd: u64) -> Event {
        Event::HwndDriftDetected {
            kind: HwndDriftKind::HiddenSinceOpen,
            label: Some(label.to_string()),
            hwnd: Some(hwnd),
            detail: "test".to_string(),
            severity: Severity::Warn,
            version,
        }
    }

    #[test]
    fn first_dispatch_passes_gate() {
        let state = fresh_state();
        assert!(should_dispatch(&state, &drift_event(1, "window-x", 100)));
    }

    #[test]
    fn duplicate_same_version_drops() {
        let state = fresh_state();
        assert!(should_dispatch(&state, &drift_event(78, "window-x", 100)));
        for _ in 0..200 {
            assert!(
                !should_dispatch(&state, &drift_event(78, "window-x", 100)),
                "same (kind,label,hwnd,version) must drop"
            );
        }
    }

    #[test]
    fn higher_version_for_same_key_passes() {
        let state = fresh_state();
        assert!(should_dispatch(&state, &drift_event(5, "window-x", 100)));
        assert!(should_dispatch(&state, &drift_event(6, "window-x", 100)));
    }

    #[test]
    fn lower_version_for_same_key_drops() {
        let state = fresh_state();
        assert!(should_dispatch(&state, &drift_event(10, "window-x", 100)));
        assert!(!should_dispatch(&state, &drift_event(9, "window-x", 100)));
    }

    #[test]
    fn different_labels_dont_collide() {
        let state = fresh_state();
        assert!(should_dispatch(&state, &drift_event(78, "window-a", 100)));
        assert!(should_dispatch(&state, &drift_event(78, "window-b", 200)));
        assert!(should_dispatch(&state, &drift_event(78, "window-c", 300)));
    }

    #[test]
    fn different_hwnds_dont_collide() {
        let state = fresh_state();
        assert!(should_dispatch(&state, &drift_event(78, "window-x", 100)));
        // Same label, different hwnd (e.g. mid-promote re-link).
        assert!(should_dispatch(&state, &drift_event(78, "window-x", 200)));
    }

    #[test]
    fn drift_storm_replay_collapses_to_one() {
        // Reproduce the v0.33.688 smoke pattern: 164 dispatches of
        // an identical HiddenSinceOpen event. The bridge must emit
        // at most ONE through `should_dispatch`.
        let state = fresh_state();
        let evt = drift_event(78, "window-ee4504a143984a4db9a1559f5b66ac21", 6162460);
        let mut admitted = 0;
        for _ in 0..164 {
            if should_dispatch(&state, &evt) {
                admitted += 1;
            }
        }
        assert_eq!(admitted, 1, "bridge must amplify-zero same-version drift");
    }

    #[test]
    fn dedup_cache_bounded() {
        let state = fresh_state();
        // 5000 unique labels — cache must not exceed MAX_DEDUP_KEYS (4096).
        for i in 0..5000 {
            let label = format!("window-{}", i);
            let _ = should_dispatch(&state, &drift_event(1, &label, i as u64));
        }
        let len = state.launcher_bridge_dedup.lock().len();
        assert!(
            len <= 4096,
            "cache size {} exceeds bound 4096 — eviction broken",
            len
        );
    }

    #[test]
    fn distinct_event_kinds_dont_collide() {
        let state = fresh_state();
        let label = "window-x";
        // Both at v=10, different kinds, same label — must both pass.
        assert!(should_dispatch(
            &state,
            &Event::WindowOpened {
                label: label.to_string(),
                kind: agentmux_common::ipc::WindowKind::FullInstance,
                parent_label: None,
                version: 10,
            }
        ));
        assert!(should_dispatch(&state, &drift_event(10, label, 100)));
    }
}
