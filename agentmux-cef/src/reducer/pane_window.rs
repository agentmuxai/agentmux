// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Pane window-placement reducer handlers (pane-state reducer, Phase 0).
//!
//! Owns the OS-window placement of FLOATING panes — the maximize/restore
//! state and the rect to restore to — keyed by `block_id` in
//! `HostState.pane_window_states`. Lifecycle (Live/Closing) is NOT here; it
//! stays in `HostState.browser_panes` (`panes.rs`).
//!
//! Design + rationale: `SPEC_PANE_STATE_REDUCER_2026-05-28.md`
//! (REVISION 2026-05-29 — folded into `HostState` instead of a standalone
//! `PaneStateMachine`, mirroring the Phase-H consolidation that deleted
//! `pane::lifecycle::PaneStateMachine` in commit 151f42e2 because a parallel
//! pane-state store drifted from the reducer's).
//!
//! ## Scope of Phase 0 (this PR)
//!
//! - The `pane_window_states` field on `HostState` + its types in
//!   `crate::state` (`PaneWindowState`, `WindowPlacement`, `PaneRect`).
//! - The `ToggleFloatingMaximize` arm (the floating half of the shared
//!   maximize button, spec §3.3a). It is DORMANT — wired into `update()`
//!   and unit-tested, but no production code dispatches it yet. The IPC
//!   wiring (refactoring `maximize_window` to dispatch this + apply the
//!   `ShowWindow` side-effect from the emitted event) lands in a later
//!   phase, alongside deleting `AppState.floating_restored_rects`.
//!
//! ## Out of scope for Phase 0
//!
//! - `ReportOSPlacementChange` (Win+Down / system menu → reducer) — later phase.
//! - `ReportNormalRect` (WM_WINDOWPOSCHANGED debounced) + the backend
//!   rect-mirror (`block.meta["pane:floating_normal_rect"]`) — later phase.
//! - Cleanup-on-close eviction folded into the `browser_panes`
//!   Closing→Closed transition — later phase.

use crate::state::{PaneWindowState, WindowPlacement};

use super::{DispatchOutput, HostEvent, HostState};

/// Toggle a floating pane's OS-window maximize: `Maximized → Normal`, or
/// anything else (`Normal` / `Minimized`) `→ Maximized`. Inserts a default
/// `Normal` entry first if the pane has none yet.
///
/// Pure: flips `pane_window_states[block_id].placement` and emits
/// `PaneWindowStateChanged`. The `ShowWindow(SW_MAXIMIZE/SW_RESTORE)`
/// side-effect is applied by the IPC handler AFTER `host_dispatch` returns —
/// never inside the reducer (snapshot-and-drop discipline, spec §3.6).
pub(super) fn handle_toggle_floating_maximize(
    state: &mut HostState,
    block_id: String,
) -> DispatchOutput {
    // Scope the map borrow so it ends before `bump_version` re-borrows state.
    let new_placement = {
        let entry = state
            .pane_window_states
            .entry(block_id.clone())
            .or_insert(PaneWindowState {
                placement: WindowPlacement::Normal,
                last_known_normal_rect: None,
            });
        let next = match entry.placement {
            WindowPlacement::Maximized => WindowPlacement::Normal,
            // Normal or Minimized both enlarge to Maximized.
            WindowPlacement::Normal | WindowPlacement::Minimized => WindowPlacement::Maximized,
        };
        entry.placement = next;
        next
    };

    let version = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::PaneWindowStateChanged {
            block_id,
            placement: new_placement,
            version,
        }],
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reducer::{update, HostCommand};
    use crate::state::{BrowserPaneEntry, BrowserPaneLifecycle};

    fn placement_of(state: &HostState, block_id: &str) -> Option<WindowPlacement> {
        state
            .pane_window_states
            .get(block_id)
            .map(|e| e.placement)
    }

    /// Seed a Live lifecycle entry + a Maximized placement entry for the
    /// same block, so close/drain/abort arms have something to co-evict.
    fn seed_pane(state: &mut HostState, block_id: &str, label: &str) {
        state.browser_panes.insert(
            block_id.to_string(),
            BrowserPaneEntry {
                block_id: block_id.to_string(),
                label: label.to_string(),
                lifecycle: BrowserPaneLifecycle::Live,
            },
        );
        state.pane_window_states.insert(
            block_id.to_string(),
            PaneWindowState {
                placement: WindowPlacement::Maximized,
                last_known_normal_rect: None,
            },
        );
    }

    fn last_event_placement(out: &DispatchOutput) -> Option<WindowPlacement> {
        out.events.iter().rev().find_map(|e| match e {
            HostEvent::PaneWindowStateChanged { placement, .. } => Some(*placement),
            _ => None,
        })
    }

    #[test]
    fn toggle_inserts_entry_and_maximizes_when_absent() {
        let mut state = HostState::default();
        assert!(placement_of(&state, "b1").is_none());

        let out = update(
            &mut state,
            HostCommand::ToggleFloatingMaximize { block_id: "b1".into() },
        );

        assert_eq!(placement_of(&state, "b1"), Some(WindowPlacement::Maximized));
        assert_eq!(last_event_placement(&out), Some(WindowPlacement::Maximized));
    }

    #[test]
    fn toggle_is_an_identity_round_trip() {
        let mut state = HostState::default();
        // Normal(implicit) → Maximized → Normal.
        update(&mut state, HostCommand::ToggleFloatingMaximize { block_id: "b1".into() });
        assert_eq!(placement_of(&state, "b1"), Some(WindowPlacement::Maximized));
        update(&mut state, HostCommand::ToggleFloatingMaximize { block_id: "b1".into() });
        assert_eq!(placement_of(&state, "b1"), Some(WindowPlacement::Normal));
    }

    #[test]
    fn toggle_emits_versioned_event_each_time() {
        let mut state = HostState::default();
        let out1 = update(&mut state, HostCommand::ToggleFloatingMaximize { block_id: "b1".into() });
        let out2 = update(&mut state, HostCommand::ToggleFloatingMaximize { block_id: "b1".into() });

        let v1 = out1.events.iter().find_map(|e| match e {
            HostEvent::PaneWindowStateChanged { version, .. } => Some(*version),
            _ => None,
        });
        let v2 = out2.events.iter().find_map(|e| match e {
            HostEvent::PaneWindowStateChanged { version, .. } => Some(*version),
            _ => None,
        });
        assert!(v1.is_some() && v2.is_some());
        assert!(v2 > v1, "event version must be monotonic across dispatches");
    }

    #[test]
    fn minimized_enlarges_to_maximized_not_normal() {
        let mut state = HostState::default();
        state.pane_window_states.insert(
            "b1".into(),
            PaneWindowState {
                placement: WindowPlacement::Minimized,
                last_known_normal_rect: None,
            },
        );
        update(&mut state, HostCommand::ToggleFloatingMaximize { block_id: "b1".into() });
        assert_eq!(placement_of(&state, "b1"), Some(WindowPlacement::Maximized));
    }

    // ── Phase 1: cleanup-on-close — placement is co-evicted wherever a
    // pane's lifecycle ends, so it can never leak. ───────────────────────

    #[test]
    fn complete_close_co_evicts_placement() {
        let mut state = HostState::default();
        seed_pane(&mut state, "b1", "browser-pane-b1-1");
        assert!(placement_of(&state, "b1").is_some());

        update(&mut state, HostCommand::CompleteBrowserPaneClose { block_id: "b1".into() });

        assert!(placement_of(&state, "b1").is_none(), "placement must be evicted on close");
        assert!(!state.browser_panes.contains_key("b1"), "lifecycle entry also gone");
    }

    #[test]
    fn drain_by_label_co_evicts_placement() {
        let mut state = HostState::default();
        seed_pane(&mut state, "b1", "browser-pane-b1-1");

        update(&mut state, HostCommand::DrainBrowserPaneByLabel {
            label: "browser-pane-b1-1".into(),
        });

        assert!(placement_of(&state, "b1").is_none(), "placement must be evicted on drain");
    }

    #[test]
    fn abort_create_co_evicts_placement() {
        let mut state = HostState::default();
        seed_pane(&mut state, "b1", "browser-pane-b1-1");

        update(&mut state, HostCommand::AbortBrowserPaneCreate {
            block_id: "b1".into(),
            reason: "browser_host returned 0".into(),
        });

        assert!(placement_of(&state, "b1").is_none(), "placement must be evicted on abort");
    }

    #[test]
    fn close_without_placement_entry_is_clean() {
        // Docked panes never have a placement entry; closing one must not
        // panic or misbehave — the remove is a no-op.
        let mut state = HostState::default();
        state.browser_panes.insert(
            "docked1".into(),
            BrowserPaneEntry {
                block_id: "docked1".into(),
                label: "browser-pane-docked1-1".into(),
                lifecycle: BrowserPaneLifecycle::Live,
            },
        );
        // No pane_window_states entry.
        let out = update(&mut state, HostCommand::CompleteBrowserPaneClose {
            block_id: "docked1".into(),
        });
        assert!(!state.browser_panes.contains_key("docked1"));
        assert!(state.pane_window_states.is_empty());
        assert!(matches!(
            out.events.as_slice(),
            [HostEvent::BrowserPaneClosed { .. }]
        ));
    }
}
