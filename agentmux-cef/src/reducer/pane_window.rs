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

use crate::state::{PaneRect, PaneWindowState, WindowPlacement};

use super::{DispatchOutput, HostEvent, HostState};

/// Toggle a floating pane's OS-window maximize: `Maximized → Normal`, or
/// anything else (`Normal` / `Minimized`) `→ Maximized`. Inserts a default
/// `Normal` entry first if the floater has none yet. Keyed by the
/// floating-window `label` (`floating-<uuid>`).
///
/// Pure: flips `pane_window_states[label].placement` and emits
/// `PaneWindowStateChanged`. The `ShowWindow(SW_MAXIMIZE/SW_RESTORE)`
/// side-effect is applied by the IPC handler AFTER `host_dispatch` returns —
/// never inside the reducer (snapshot-and-drop discipline, spec §3.6) — by
/// resolving `window_hwnds[label]`.
pub(super) fn handle_toggle_floating_maximize(
    state: &mut HostState,
    label: String,
    current_rect: Option<PaneRect>,
) -> DispatchOutput {
    // Scope the map borrow so it ends before `bump_version` re-borrows state.
    // `restore_rect` is the rect the IPC handler must `SetWindowPos` the
    // floater back to on a Maximized→Normal flip. Borderless WS_POPUP
    // floaters have no usable native maximize placement, so we capture the
    // pre-maximize rect here and hand it back on restore (the handler sizes
    // to the monitor work area on the way up).
    let (new_placement, restore_rect) = {
        let entry = state
            .pane_window_states
            .entry(label.clone())
            .or_insert(PaneWindowState {
                placement: WindowPlacement::Normal,
                last_known_normal_rect: None,
            });
        match entry.placement {
            WindowPlacement::Maximized => {
                // Maximized → Normal: hand back the rect we stashed when
                // maximizing (if any), then clear it.
                entry.placement = WindowPlacement::Normal;
                let restore = entry.last_known_normal_rect.take();
                (WindowPlacement::Normal, restore)
            }
            // Normal or Minimized both enlarge to Maximized. Stash the
            // floater's current (normal) rect so the next restore can return
            // to it; the handler computes the maximized (work-area) rect.
            WindowPlacement::Normal | WindowPlacement::Minimized => {
                entry.placement = WindowPlacement::Maximized;
                entry.last_known_normal_rect = current_rect;
                (WindowPlacement::Maximized, None)
            }
        }
    };

    let version = state.bump_version();
    DispatchOutput {
        events: vec![HostEvent::PaneWindowStateChanged {
            label,
            placement: new_placement,
            restore_rect,
            version,
        }],
        ..Default::default()
    }
}

/// Evict a floater's window-placement entry on window close. Dispatched from
/// `on_before_close` alongside the `window_hwnds[label]` eviction, so the
/// placement entry can never outlive the floater. Idempotent — no-op if
/// absent (non-floater windows never have an entry). This is the label-keyed
/// cleanup-on-close that the (removed) block_id co-eviction in `panes.rs`
/// could never do for floaters, since floaters aren't in `browser_panes`.
pub(super) fn handle_evict_floating_pane_window_state(
    state: &mut HostState,
    label: String,
) -> DispatchOutput {
    // Idempotent: nothing to do (and no event) if there was no entry.
    state.pane_window_states.remove(&label);
    DispatchOutput::default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reducer::{update, HostCommand};

    const LBL: &str = "floating-abc123";

    fn placement_of(state: &HostState, label: &str) -> Option<WindowPlacement> {
        state.pane_window_states.get(label).map(|e| e.placement)
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
        assert!(placement_of(&state, LBL).is_none());

        let out = update(
            &mut state,
            HostCommand::ToggleFloatingMaximize { label: LBL.into(), current_rect: None },
        );

        assert_eq!(placement_of(&state, LBL), Some(WindowPlacement::Maximized));
        assert_eq!(last_event_placement(&out), Some(WindowPlacement::Maximized));
    }

    #[test]
    fn toggle_is_an_identity_round_trip() {
        let mut state = HostState::default();
        // Normal(implicit) -> Maximized -> Normal.
        update(&mut state, HostCommand::ToggleFloatingMaximize { label: LBL.into(), current_rect: None });
        assert_eq!(placement_of(&state, LBL), Some(WindowPlacement::Maximized));
        update(&mut state, HostCommand::ToggleFloatingMaximize { label: LBL.into(), current_rect: None });
        assert_eq!(placement_of(&state, LBL), Some(WindowPlacement::Normal));
    }

    #[test]
    fn toggle_emits_versioned_event_each_time() {
        let mut state = HostState::default();
        let out1 = update(&mut state, HostCommand::ToggleFloatingMaximize { label: LBL.into(), current_rect: None });
        let out2 = update(&mut state, HostCommand::ToggleFloatingMaximize { label: LBL.into(), current_rect: None });

        let v = |out: &DispatchOutput| out.events.iter().find_map(|e| match e {
            HostEvent::PaneWindowStateChanged { version, .. } => Some(*version),
            _ => None,
        });
        let (v1, v2) = (v(&out1), v(&out2));
        assert!(v1.is_some() && v2.is_some());
        assert!(v2 > v1, "event version must be monotonic across dispatches");
    }

    #[test]
    fn minimized_enlarges_to_maximized_not_normal() {
        let mut state = HostState::default();
        state.pane_window_states.insert(
            LBL.into(),
            PaneWindowState {
                placement: WindowPlacement::Minimized,
                last_known_normal_rect: None,
            },
        );
        update(&mut state, HostCommand::ToggleFloatingMaximize { label: LBL.into(), current_rect: None });
        assert_eq!(placement_of(&state, LBL), Some(WindowPlacement::Maximized));
    }

    fn last_event_restore_rect(out: &DispatchOutput) -> Option<PaneRect> {
        out.events.iter().rev().find_map(|e| match e {
            HostEvent::PaneWindowStateChanged { restore_rect, .. } => Some(*restore_rect),
            _ => None,
        }).flatten()
    }

    #[test]
    fn maximize_captures_current_rect_and_restore_returns_it() {
        let mut state = HostState::default();
        let normal = PaneRect { left: 100, top: 120, right: 740, bottom: 1020 };

        // Normal → Maximized: stash the current rect, emit no restore_rect
        // (the handler computes the work area on the way up).
        let up = update(
            &mut state,
            HostCommand::ToggleFloatingMaximize { label: LBL.into(), current_rect: Some(normal) },
        );
        assert_eq!(placement_of(&state, LBL), Some(WindowPlacement::Maximized));
        assert_eq!(last_event_restore_rect(&up), None, "maximize emits no restore rect");
        assert_eq!(
            state.pane_window_states.get(LBL).and_then(|e| e.last_known_normal_rect),
            Some(normal),
            "the pre-maximize rect must be stashed for the later restore"
        );

        // Maximized → Normal: emit the stashed rect so the handler can
        // SetWindowPos back to it, and clear it from state.
        let down = update(
            &mut state,
            HostCommand::ToggleFloatingMaximize { label: LBL.into(), current_rect: None },
        );
        assert_eq!(placement_of(&state, LBL), Some(WindowPlacement::Normal));
        assert_eq!(last_event_restore_rect(&down), Some(normal), "restore returns the captured rect");
        assert_eq!(
            state.pane_window_states.get(LBL).and_then(|e| e.last_known_normal_rect),
            None,
            "the stashed rect is consumed on restore"
        );
    }

    // -- Cleanup-on-close: EvictFloatingPaneWindowState is dispatched from
    // on_before_close (where window_hwnds[label] is also evicted), keyed by
    // the floating-window label. This is the correct cleanup hook for
    // floaters, which are NOT in browser_panes. --

    #[test]
    fn evict_removes_placement_entry() {
        let mut state = HostState::default();
        state.pane_window_states.insert(
            LBL.into(),
            PaneWindowState { placement: WindowPlacement::Maximized, last_known_normal_rect: None },
        );
        assert!(placement_of(&state, LBL).is_some());

        let out = update(
            &mut state,
            HostCommand::EvictFloatingPaneWindowState { label: LBL.into() },
        );

        assert!(placement_of(&state, LBL).is_none(), "placement must be evicted on close");
        assert!(out.events.is_empty(), "eviction is internal cleanup; emits no event");
    }

    #[test]
    fn evict_absent_label_is_idempotent_noop() {
        // A non-floater window close (or a double close) must not panic.
        let mut state = HostState::default();
        let out = update(
            &mut state,
            HostCommand::EvictFloatingPaneWindowState { label: "main".into() },
        );
        assert!(state.pane_window_states.is_empty());
        assert!(out.events.is_empty());
    }
}
