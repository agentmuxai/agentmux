// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Debounced srv write-through for window position/size — the position-side
// counterpart to `transparency.rs`'s opacity write-through. Closes the gap
// documented in SPEC_PILLAR1_STEP4_CRASH_REPROJECT_2026_07_07.md §4:
// `reproject_from_srv` (the slow-path reproject used when the launcher
// process itself also died, not just the main window closing) had no rect
// to restore secondary windows to, because position/size was only ever
// tracked in the launcher's live in-memory `WindowMirror.last_rect` — gone
// the moment the launcher process exits.
//
// `Window.pos`/`Window.winsize` (agentmux-srv/src/backend/obj.rs) already
// existed as fields, and the `SetWindowPosAndSize` RPC
// (agentmux-srv/src/server/service/window_mutate.rs) already existed to
// write them — this file is the first caller of that RPC.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use windows_sys::Win32::Foundation::HWND;

use crate::state::AppState;

/// Trailing-edge debounce for the srv position write-through, keyed by
/// window label — same generation-counter shape as
/// `transparency.rs::OPACITY_WRITE_DEBOUNCE_MS`, with its own map (not
/// shared with opacity's) since the two properties change independently.
///
/// Position changes arrive far more often than opacity changes (every
/// `EVENT_OBJECT_LOCATIONCHANGE` during a drag, already smoothed to ~20Hz
/// by `position_debounce::should_emit` upstream) and are lower-urgency —
/// nothing reads the persisted value until the next full-restart reproject,
/// unlike opacity which a live crash-recovery path can read moments later.
/// A longer window than opacity's 400ms is deliberate: 1500ms collapses an
/// entire drag-then-release gesture to one write instead of firing on every
/// intermediate pause.
const POSITION_WRITE_DEBOUNCE_MS: u64 = 1500;

fn position_write_generations() -> &'static Mutex<HashMap<String, u64>> {
    static MAP: OnceLock<Mutex<HashMap<String, u64>>> = OnceLock::new();
    MAP.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Bump and return the new generation for `label`.
fn next_position_write_generation(label: &str) -> u64 {
    let mut m = position_write_generations()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    let gen = m.entry(label.to_string()).or_insert(0);
    *gen += 1;
    *gen
}

/// True if `generation` is still the latest recorded generation for
/// `label` — i.e. no newer position report for this label has arrived
/// since this write was scheduled.
fn is_current_position_write_generation(label: &str, generation: u64) -> bool {
    let m = position_write_generations()
        .lock()
        .unwrap_or_else(|p| p.into_inner());
    m.get(label).copied() == Some(generation)
}

/// Called from `wrr::win_event`'s `EVENT_OBJECT_LOCATIONCHANGE` handler on
/// every already-20Hz-debounced real (non-pool-move) position report,
/// alongside the existing `launcher_ipc::report_hwnd_position_changed`
/// call. A second, independent consumer of the same position stream — does
/// not touch or replace the launcher-IPC forwarding.
///
/// No-ops (same as opacity's write-through) when the hwnd's label can't be
/// resolved, or when the label has no registered `backend_window_id` yet
/// (e.g. early in window creation, or a floating pane — floating panes
/// have no srv `Window` row, see SPEC_PILLAR1_STEP2_WINDOW_TOPOLOGY_
/// PERSISTENCE_2026_07_06.md §1.B).
pub(crate) fn report_position_for_srv_writethrough(state: &Arc<AppState>, hwnd: HWND, rect: agentmux_common::ipc::Rect) {
    let Some(label) = state.label_for_hwnd(hwnd) else {
        return;
    };
    let Some(window_id) = state.backend_window_id(&label) else {
        return;
    };

    let generation = next_position_write_generation(&label);
    let web_endpoint = state.backend_endpoints.lock().web_endpoint.clone();
    let auth_key = state.auth_key.lock().clone();
    let debounce_label = label.clone();
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_millis(POSITION_WRITE_DEBOUNCE_MS));
        if !is_current_position_write_generation(&debounce_label, generation) {
            // A newer report for this label superseded us during the
            // sleep — this rect is stale, don't persist it.
            return;
        }
        crate::client::backend_set_window_pos_and_size(&web_endpoint, &auth_key, &window_id, rect);
    });
}

#[cfg(test)]
mod position_debounce_tests {
    use super::{is_current_position_write_generation, next_position_write_generation};

    /// Simulates a rapid burst (a drag): only the LAST report's generation
    /// should still be "current" once all bumps have happened — the same
    /// scenario opacity's debounce guards against, applied to position.
    #[test]
    fn burst_only_the_last_generation_is_current() {
        let label = "test-position-burst-only-last-wins";
        let gens: Vec<u64> = (0..5).map(|_| next_position_write_generation(label)).collect();
        for &g in &gens[..gens.len() - 1] {
            assert!(
                !is_current_position_write_generation(label, g),
                "earlier generation {g} must be superseded after a burst"
            );
        }
        assert!(
            is_current_position_write_generation(label, *gens.last().unwrap()),
            "the last generation in the burst must still be current"
        );
    }

    /// A single, deliberate (non-burst) report must still be current.
    #[test]
    fn single_report_is_current() {
        let label = "test-position-single-report-is-current";
        let g = next_position_write_generation(label);
        assert!(is_current_position_write_generation(label, g));
    }

    /// Generations are tracked independently per label — a burst on one
    /// window must not supersede a pending write for a different window.
    #[test]
    fn generations_are_independent_per_label() {
        let a = "test-position-independent-label-a";
        let b = "test-position-independent-label-b";
        let ga = next_position_write_generation(a);
        let gb = next_position_write_generation(b);
        next_position_write_generation(a);
        assert!(!is_current_position_write_generation(a, ga));
        assert!(is_current_position_write_generation(b, gb));
    }
}
