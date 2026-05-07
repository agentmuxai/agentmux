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

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use agentmux_common::ipc::Event;
use cef::{CefString, ImplBrowser, ImplFrame};

/// Phase F.7 host-bridge dedup cache. Bounded FIFO map keyed by
/// `"{event_kind}|{label}|{hwnd}"` → max version dispatched.
///
/// FIFO insertion order is tracked explicitly via `insertion_order`
/// because std `HashMap::keys().next()` iteration order is undefined
/// per-rebuild — the previous implementation could evict the
/// just-inserted key (reagent P1 PR #722 round 1).
///
/// Reset on launcher restart sentinel (codex P1 PR #722 round 1):
/// when the launcher's `event_version` resets to 1, any cached key
/// holding a higher version blocks the v=1 event. Mirror the
/// renderer-side guard's heuristic — clear the cache when we see a
/// v=1 event AND any cached entry has a version > 0.
#[derive(Default)]
pub struct DedupCache {
    seen: HashMap<String, u64>,
    insertion_order: VecDeque<String>,
}

impl DedupCache {
    pub fn new() -> Self {
        Self {
            seen: HashMap::new(),
            insertion_order: VecDeque::new(),
        }
    }

    /// Returns true if the event should be dispatched (strictly newer
    /// for its key). Updates the cache as a side-effect when admitted.
    /// Bounded at `cap`; on overflow, evicts the oldest insertion in
    /// FIFO order.
    pub fn check_and_record(&mut self, key: String, version: u64, cap: usize) -> bool {
        if let Some(&seen) = self.seen.get(&key) {
            if version <= seen {
                return false;
            }
            // Update existing entry — don't reorder for this case;
            // FIFO ordering is "first inserted, first evicted" not
            // "least recently used".
            self.seen.insert(key, version);
            return true;
        }
        self.seen.insert(key.clone(), version);
        self.insertion_order.push_back(key);
        if self.seen.len() > cap {
            if let Some(victim) = self.insertion_order.pop_front() {
                self.seen.remove(&victim);
            }
        }
        true
    }

    /// Clear the cache. Called on launcher-restart sentinel.
    pub fn clear(&mut self) {
        self.seen.clear();
        self.insertion_order.clear();
    }

    /// True if any cached entry has a version above 0 — used as the
    /// guard for the v=1 restart sentinel.
    pub fn has_any_versioned_entry(&self) -> bool {
        self.seen.values().any(|&v| v > 0)
    }

    pub fn len(&self) -> usize {
        self.seen.len()
    }
}

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
/// newer for its key and should be dispatched; `false` if a
/// higher-or-equal version was already sent.
///
/// Mirrors the renderer-side `shouldDispatchLauncherEvent`. Bounded
/// at 4096 keys with FIFO eviction so a long-running host can't
/// leak unbounded state. Re-arrival of an evicted key bypasses the
/// host gate but the renderer guard still catches it.
///
/// Restart sentinel (codex P1 PR #722 round 1): clear the cache
/// when the launcher's event_version resets to 1 and any cached
/// entry holds a higher version. Mirrors the renderer guard.
fn should_dispatch(state: &Arc<crate::state::AppState>, event: &Event) -> bool {
    const MAX_DEDUP_KEYS: usize = 4096;
    let (key, version) = dedup_key(event);
    let mut cache = state.launcher_bridge_dedup.lock();
    if version == 1 && cache.has_any_versioned_entry() {
        cache.clear();
    }
    cache.check_and_record(key, version, MAX_DEDUP_KEYS)
}

/// Build the dedup cache key + extract the version for an event.
/// Returns `("{kind}|{label}|{hwnd}", version)`.
///
/// `kind` for `HwndDriftDetected` is `"hwnd_drift_detected:{drift_kind}"`
/// so HiddenSinceOpen and OffMonitor for the same `(label, hwnd)`
/// don't collide (reagent P2 PR #722 round 2).
///
/// Unhandled variants are tagged with their serde discriminant
/// (the `event` field of the JSON tagged-union) so different
/// variants don't share the same `__catchall__` key (reagent P1
/// PR #722 round 1).
fn dedup_key(event: &Event) -> (String, u64) {
    use Event::*;
    let (kind, label, hwnd, version) = match event {
        WindowOpened { label, version, .. } => ("window_opened".to_string(), label.as_str(), 0u64, *version),
        WindowClosed { label, version, .. } => ("window_closed".to_string(), label.as_str(), 0, *version),
        WindowInstanceAssigned { label, version, .. } => ("window_instance_assigned".to_string(), label.as_str(), 0, *version),
        WindowInstanceReleased { label, version, .. } => ("window_instance_released".to_string(), label.as_str(), 0, *version),
        BackendWindowIdRegistered { label, version, .. } => ("backend_window_id_registered".to_string(), label.as_str(), 0, *version),
        BackendWindowIdUnregistered { label, version, .. } => ("backend_window_id_unregistered".to_string(), label.as_str(), 0, *version),
        PoolWindowAdded { label, version, .. } => ("pool_window_added".to_string(), label.as_str(), 0, *version),
        PoolWindowRemoved { label, version, .. } => ("pool_window_removed".to_string(), label.as_str(), 0, *version),
        PoolWindowPromoted { label, version, .. } => ("pool_window_promoted".to_string(), label.as_str(), 0, *version),
        HwndDriftDetected { kind: drift_kind, label, hwnd, version, .. } => (
            format!("hwnd_drift_detected:{:?}", drift_kind),
            label.as_deref().unwrap_or(""),
            hwnd.unwrap_or(0),
            *version,
        ),
        DriftDetected { kind: drift_kind, version, .. } => (
            format!("drift_detected:{:?}", drift_kind),
            "",
            0,
            *version,
        ),
        CorrectiveWindowMove { hwnd, version, .. } => ("corrective_window_move".to_string(), "", *hwnd, *version),
        HostShouldQuit { version, .. } => ("host_should_quit".to_string(), "", 0, *version),
        // Catchall: extract the serde discriminant ("event" tag in
        // the JSON tagged-union) so different unhandled variants
        // don't collide on the same `__catchall__` key.
        other => {
            let value = serde_json::to_value(other).ok();
            let event_tag = value
                .as_ref()
                .and_then(|v| v.get("event"))
                .and_then(|v| v.as_str())
                .unwrap_or("__unknown__")
                .to_string();
            let v = value
                .as_ref()
                .and_then(|v| v.get("version"))
                .and_then(|x| x.as_u64())
                .unwrap_or(0);
            (event_tag, "", 0, v)
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

    #[test]
    fn distinct_drift_kinds_dont_collide() {
        // Reagent P2 PR #722 round 2: HwndDriftDetected key must
        // include the drift kind, otherwise HiddenSinceOpen and
        // OffMonitor for the same (label, hwnd) collide.
        let state = fresh_state();
        let label = "window-x";
        let hwnd = 100u64;
        let hidden = Event::HwndDriftDetected {
            kind: HwndDriftKind::HiddenSinceOpen,
            label: Some(label.to_string()),
            hwnd: Some(hwnd),
            detail: "h".into(),
            severity: Severity::Warn,
            version: 10,
        };
        let off = Event::HwndDriftDetected {
            kind: HwndDriftKind::OffMonitor,
            label: Some(label.to_string()),
            hwnd: Some(hwnd),
            detail: "o".into(),
            severity: Severity::Warn,
            version: 10,
        };
        assert!(should_dispatch(&state, &hidden));
        assert!(
            should_dispatch(&state, &off),
            "different drift kinds at same (label, hwnd, version) must NOT collide"
        );
    }

    #[test]
    fn launcher_restart_sentinel_clears_cache() {
        // Codex P1 PR #722 round 1: when launcher restarts and
        // event_version resets to 1, prior cached entries with
        // higher versions block the v=1 sentinel and subsequent
        // low-version events. Cache must clear on the sentinel.
        let state = fresh_state();
        // Establish some prior-incarnation cache entries.
        assert!(should_dispatch(&state, &drift_event(10, "window-a", 100)));
        assert!(should_dispatch(&state, &drift_event(15, "window-b", 200)));
        assert!(state.launcher_bridge_dedup.lock().has_any_versioned_entry());

        // Launcher restarts: emits v=1 for some new window. Without
        // the cache reset, the v=1 wouldn't necessarily collide
        // (different label) but SUBSEQUENT low-version events for
        // pre-existing labels would block. The reset is keyed off
        // "v=1 with prior versions cached" — fires once, clears all.
        let sentinel = Event::WindowOpened {
            label: "main".into(),
            kind: agentmux_common::ipc::WindowKind::FullInstance,
            parent_label: None,
            version: 1,
        };
        assert!(should_dispatch(&state, &sentinel));
        // Cache should now contain only the sentinel + nothing else.
        assert_eq!(
            state.launcher_bridge_dedup.lock().len(),
            1,
            "restart sentinel clears cache before recording new entry"
        );

        // Subsequent low-version events for the same key must admit
        // (cache no longer holds the stale higher version).
        assert!(should_dispatch(
            &state,
            &Event::WindowOpened {
                label: "window-a".into(),
                kind: agentmux_common::ipc::WindowKind::FullInstance,
                parent_label: None,
                version: 2,
            }
        ));
    }

    #[test]
    fn cold_v1_event_into_empty_cache_admits() {
        // Anti-vacuity guard: a cold v=1 event with no prior cache
        // is the first event of a fresh launcher and admits cleanly.
        // The sentinel logic only fires when v=1 arrives WITH a
        // pre-existing cache entry (renderer-side mirror), so a
        // truly-empty-cache v=1 just admits without ceremony.
        let state = fresh_state();
        let evt = Event::WindowOpened {
            label: "main".into(),
            kind: agentmux_common::ipc::WindowKind::FullInstance,
            parent_label: None,
            version: 1,
        };
        assert!(should_dispatch(&state, &evt));
        assert_eq!(state.launcher_bridge_dedup.lock().len(), 1);
    }

    #[test]
    fn fifo_eviction_drops_oldest_not_newest() {
        // Reagent P1 PR #722 round 1: HashMap iteration order is
        // not insertion order, so the previous implementation could
        // evict the just-inserted key. Now: VecDeque tracks insert
        // order; pop_front drops the oldest.
        let state = fresh_state();
        // Fill cache with 5000 unique keys.
        for i in 0..5000 {
            let label = format!("window-{}", i);
            assert!(should_dispatch(&state, &drift_event(1, &label, i as u64)));
        }
        let cache = state.launcher_bridge_dedup.lock();
        assert!(cache.len() <= 4096, "cache bounded at 4096");
        // First 904 keys (5000 - 4096) should have been evicted.
        // Verify a few from the start are gone:
        assert!(!cache.seen.contains_key("hwnd_drift_detected:HiddenSinceOpen|window-0|0"));
        assert!(!cache.seen.contains_key("hwnd_drift_detected:HiddenSinceOpen|window-100|100"));
        // Verify recent keys are retained:
        assert!(cache.seen.contains_key("hwnd_drift_detected:HiddenSinceOpen|window-4999|4999"));
    }
}
