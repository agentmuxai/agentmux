// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/// See `AppState::ui_thread_gate`.
#[derive(Default)]
pub struct UiThreadGate {
    pub ready: bool,
    pub stashed: Option<Vec<agentmux_common::ipc::WindowSnapshot>>,
    /// SPEC_PILLAR1_STEP4 Phase 3 addendum (reagent P1, PR #2032, 2026-07-08)
    /// — the same `Event::Snapshot`'s sibling `backend_window_ids` field
    /// (label → real srv window_id), stashed alongside `stashed` under the
    /// same lock acquisition so the two never drift apart. Needed so the
    /// fast-path replay can stage a deferred close for each recreated
    /// window via `reproject_from_snapshot_and_stage_closures` — without
    /// this, the fast path had no way to learn the OLD window's real srv
    /// id (`WindowSnapshot.label` alone is only the launcher's in-memory
    /// label), so `Client.windowids` grew unboundedly on every ordinary
    /// (launcher-survives) crash.
    pub stashed_backend_window_ids: Option<Vec<(String, String)>>,
    /// SPEC_PILLAR1_STEP4 Phase 3 — set true the moment EITHER reproject
    /// path (fast, from the launcher's snapshot; or slow, from srv) has
    /// actually been triggered, so only one of them ever creates windows.
    /// Without this, a fast-path snapshot arriving late (after the slow
    /// path already ran, or vice versa) would run a second, redundant
    /// reproject on top of the first.
    pub reprojected: bool,
    /// SPEC_PILLAR1_STEP4 Phase 3 addendum (reagent P0, PR #2017,
    /// 2026-07-08) — set true by `"main"`'s registration when no fast-path
    /// stash was available, meaning the slow path SHOULD run but can't yet:
    /// `reproject_from_srv` needs `"main"`'s own confirmed srv `window_id`
    /// to know which entry in `Client.windowids` to exclude/treat as the
    /// parent-linkage sentinel, and that isn't known until `"main"`'s own
    /// `register_backend_window` call lands (well after native browser
    /// creation — the frontend has to load and bootstrap first). The
    /// original design used `windowids[0]` as a stand-in, positionally —
    /// reagent caught that `windowids` gets reordered by `focus_window`
    /// (`agentmux-srv/.../wcore/window.rs:164`) to put the last-focused
    /// window at index 0, which is not reliably `"main"`. Consumed (cleared)
    /// by whichever of {`register_backend_window`'s `"main"` branch, a
    /// late-arriving real fast-path snapshot} runs first — see both call
    /// sites' comments for why exactly one of them wins.
    pub pending_slow_path: bool,
}

/// What the caller of `UiThreadGate::on_main_ready` should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MainReadyAction {
    /// A real fast-path snapshot was stashed — replay it now.
    ReplayFastPath,
    /// Nothing useful was stashed — the caller should await the slow path
    /// (triggered later, elsewhere, once `"main"`'s srv id is known).
    AwaitSlowPath,
    /// Something already reprojected (shouldn't normally happen — `"main"`
    /// only registers once — kept as a defensive no-op, not a panic).
    Noop,
}

/// What the caller of `UiThreadGate::on_snapshot` should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SnapshotAction {
    /// UI thread isn't ready yet — stash the snapshot for later replay.
    Stash,
    /// Run the fast path now with this snapshot's data.
    RunFastPath,
    /// Something already reprojected — ignore this snapshot.
    Skip,
}

impl UiThreadGate {
    /// SPEC_PILLAR1_STEP4 Phase 3 — reagent P2 (PR #2017, 2026-07-08): this
    /// state machine has had three real bugs found across three review
    /// rounds (a TOCTOU, a "stash exists" vs "stash has data" conflation,
    /// and a positional-vs-value identity bug), each only caught by manual
    /// live-kill verification. Extracted into pure, directly unit-testable
    /// methods so future edits have more than that to lean on. Callers
    /// (`client/lifecycle.rs`, `launcher_ipc.rs`, `commands/window/meta.rs`)
    /// still own all I/O (taking the stash's data, actually calling
    /// `reproject_from_snapshot`/`reproject_from_srv`) — these methods only
    /// decide, they never act.
    ///
    /// Called once, when `"main"` registers. `has_extra` = does the
    /// (already-taken) stash contain anything beyond `"main"` itself? A
    /// stash existing is NOT the same as it having anything useful — see
    /// the call site for why that distinction mattered.
    pub fn on_main_ready(&mut self, has_extra: bool) -> MainReadyAction {
        self.ready = true;
        if has_extra {
            self.reprojected = true;
            MainReadyAction::ReplayFastPath
        } else if !self.reprojected {
            self.pending_slow_path = true;
            MainReadyAction::AwaitSlowPath
        } else {
            MainReadyAction::Noop
        }
    }

    /// Called whenever an `Event::Snapshot` arrives from the launcher.
    pub fn on_snapshot(&mut self) -> SnapshotAction {
        if !self.ready {
            SnapshotAction::Stash
        } else if self.reprojected {
            SnapshotAction::Skip
        } else {
            // ready && !reprojected: a slow path is pending but hasn't
            // fired yet. Real fast-path data is strictly better (has
            // `last_rect`, no srv round-trip) — it wins the race.
            self.reprojected = true;
            self.pending_slow_path = false;
            SnapshotAction::RunFastPath
        }
    }

    /// Called from `register_backend_window`'s `"main"` branch, once
    /// `"main"`'s own confirmed srv `window_id` is known. Returns whether
    /// the caller should now run the slow path.
    pub fn on_main_backend_window_registered(&mut self) -> bool {
        if self.pending_slow_path && !self.reprojected {
            self.pending_slow_path = false;
            self.reprojected = true;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod ui_thread_gate_tests {
    use super::*;

    #[test]
    fn main_ready_with_extra_replays_fast_path_and_sets_reprojected() {
        let mut gate = UiThreadGate::default();
        let action = gate.on_main_ready(true);
        assert_eq!(action, MainReadyAction::ReplayFastPath);
        assert!(gate.ready);
        assert!(gate.reprojected);
        assert!(!gate.pending_slow_path);
    }

    #[test]
    fn main_ready_without_extra_awaits_slow_path() {
        let mut gate = UiThreadGate::default();
        let action = gate.on_main_ready(false);
        assert_eq!(action, MainReadyAction::AwaitSlowPath);
        assert!(gate.ready);
        assert!(!gate.reprojected);
        assert!(gate.pending_slow_path);
    }

    #[test]
    fn main_ready_is_noop_if_already_reprojected() {
        // Defensive case — shouldn't happen in practice ("main" registers
        // once) but must not double-trigger if it somehow did.
        let mut gate = UiThreadGate::default();
        gate.reprojected = true;
        let action = gate.on_main_ready(false);
        assert_eq!(action, MainReadyAction::Noop);
        assert!(!gate.pending_slow_path);
    }

    #[test]
    fn snapshot_before_ready_stashes() {
        let mut gate = UiThreadGate::default();
        assert_eq!(gate.on_snapshot(), SnapshotAction::Stash);
        assert!(!gate.reprojected);
    }

    #[test]
    fn snapshot_after_ready_with_pending_slow_path_wins_and_cancels_it() {
        let mut gate = UiThreadGate::default();
        gate.on_main_ready(false); // ready=true, pending_slow_path=true
        assert!(gate.pending_slow_path);
        let action = gate.on_snapshot();
        assert_eq!(action, SnapshotAction::RunFastPath);
        assert!(gate.reprojected);
        assert!(!gate.pending_slow_path);
    }

    #[test]
    fn snapshot_after_reprojected_is_skipped() {
        let mut gate = UiThreadGate::default();
        gate.on_main_ready(true); // reprojected=true immediately
        let action = gate.on_snapshot();
        assert_eq!(action, SnapshotAction::Skip);
    }

    #[test]
    fn slow_path_runs_when_pending_and_not_yet_reprojected() {
        let mut gate = UiThreadGate::default();
        gate.on_main_ready(false); // pending_slow_path=true
        assert!(gate.on_main_backend_window_registered());
        assert!(gate.reprojected);
        assert!(!gate.pending_slow_path);
    }

    #[test]
    fn slow_path_does_not_run_twice() {
        let mut gate = UiThreadGate::default();
        gate.on_main_ready(false);
        assert!(gate.on_main_backend_window_registered());
        // Second call (e.g. a duplicate register_backend_window) must not
        // re-trigger the slow path.
        assert!(!gate.on_main_backend_window_registered());
    }

    #[test]
    fn slow_path_does_not_run_if_fast_path_already_won() {
        let mut gate = UiThreadGate::default();
        gate.on_main_ready(false); // pending_slow_path=true
        gate.on_snapshot(); // fast path wins, cancels pending_slow_path
        assert!(!gate.on_main_backend_window_registered());
    }

    #[test]
    fn slow_path_does_not_run_if_never_pending() {
        // Fast path had data from the start — pending_slow_path was never set.
        let mut gate = UiThreadGate::default();
        gate.on_main_ready(true);
        assert!(!gate.on_main_backend_window_registered());
    }
}
