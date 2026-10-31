// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Zoom for `BrowserPaneManager`: Ctrl+Wheel over a pane steps Chromium's own
//! page zoom, the way Chrome's per-site zoom works.
//!
//! The level lives in the profile's `HostZoomMap`, keyed by the page's host, so
//! the browser applies it when a navigation commits: a page never paints at
//! 100% first. It follows the site, not the pane (two panes on one host in one
//! profile share it), and it is stored in the profile, so a named profile keeps
//! it across a restart and a private one forgets it with the session. The
//! earlier version injected CSS `zoom` from `on_load_end`, which painted every
//! page at 100% before jumping.
//!
//! Split out of `browser_panes/mod.rs` -- see that module's doc comment.

use std::sync::Arc;

use cef::*;

use crate::state::AppState;

use super::BrowserPaneManager;

/// Chromium's page zoom: a level `n` scales the page by `1.2^n`.
const ZOOM_BASE: f64 = 1.2;

fn factor_of_level(level: f64) -> f64 {
    ZOOM_BASE.powf(level)
}

fn level_of_factor(factor: f64) -> f64 {
    factor.ln() / ZOOM_BASE.ln()
}

impl BrowserPaneManager {
    /// Ctrl+Wheel step, in the same +/-0.05 units and 0.5-2.0 clamp range
    /// `frontend/app/store/zoom.ts`'s WHEEL_STEP uses for every other
    /// pane type, for a consistent feel even though this path never touches
    /// that frontend module (browser-pane content is unreachable from the
    /// DOM).
    #[cfg(target_os = "windows")]
    const ZOOM_STEP: f64 = 0.05;

    #[cfg(target_os = "windows")]
    pub fn zoom_in(&self, block_id: &str, state: &Arc<AppState>) {
        self.step_zoom(block_id, state, Self::ZOOM_STEP);
    }
    #[cfg(target_os = "windows")]
    pub fn zoom_out(&self, block_id: &str, state: &Arc<AppState>) {
        self.step_zoom(block_id, state, -Self::ZOOM_STEP);
    }

    /// Steps the page's zoom by `delta` of a factor. Runs on the UI thread (the
    /// pane's window procedure), where `BrowserHost`'s zoom calls belong.
    #[cfg(target_os = "windows")]
    fn step_zoom(&self, block_id: &str, state: &Arc<AppState>, delta: f64) {
        let Some(host) = self.live_browser(state, block_id).and_then(|b| b.host()) else { return };
        let level = host.zoom_level();
        // The default zoom level is not always 0; steps are taken from a factor of 1.
        let base = host.default_zoom_level();
        let next = next_zoom_factor(factor_of_level(level - base), delta);
        // Back to the default is a reset (level 0.0 resets the page scale too).
        let target = if (next - 1.0).abs() < 1e-6 { 0.0 } else { level_of_factor(next) + base };
        host.set_zoom_level(target);
    }
}

/// Pure clamp+step arithmetic for `BrowserPaneManager::step_zoom`, split out
/// so it's testable without a real `AppState`/lock/browser — mirrors why
/// `BrowserPaneCloseOps` exists as its own trait (see that trait's doc
/// comment in `mod.rs`): the CEF/HWND side can't be unit-tested meaningfully,
/// but the logic feeding it can.
#[cfg(any(target_os = "windows", test))]
fn next_zoom_factor(current: f64, delta: f64) -> f64 {
    // Rounded to the hundredth: the factor comes back from a level (a power of
    // 1.2), and unrounded steps would drift off the 0.05 grid.
    (((current + delta) * 100.0).round() / 100.0).clamp(0.5, 2.0)
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_factor_round_trips_through_a_level() {
        for f in [0.5, 0.75, 1.0, 1.25, 2.0] {
            assert!((super::factor_of_level(super::level_of_factor(f)) - f).abs() < 1e-9, "{f}");
        }
        assert_eq!(super::level_of_factor(1.0), 0.0);
        assert!((super::level_of_factor(1.2) - 1.0).abs() < 1e-9, "a level is a power of 1.2");
    }

    #[test]
    fn steps_taken_from_a_levels_factor_stay_on_the_grid() {
        let mut f = 1.0;
        for _ in 0..7 {
            f = super::next_zoom_factor(super::factor_of_level(super::level_of_factor(f)), 0.05);
        }
        assert_eq!(f, 1.35);
        for _ in 0..7 {
            f = super::next_zoom_factor(super::factor_of_level(super::level_of_factor(f)), -0.05);
        }
        assert_eq!(f, 1.0);
    }

    #[test]
    fn next_zoom_factor_steps_up_from_default() {
        assert_eq!(super::next_zoom_factor(1.0, 0.05), 1.05);
    }

    #[test]
    fn next_zoom_factor_steps_down_from_default() {
        assert_eq!(super::next_zoom_factor(1.0, -0.05), 0.95);
    }

    #[test]
    fn next_zoom_factor_clamps_at_max() {
        assert_eq!(super::next_zoom_factor(2.0, 0.05), 2.0);
        assert_eq!(super::next_zoom_factor(1.98, 0.05), 2.0);
    }

    #[test]
    fn next_zoom_factor_clamps_at_min() {
        assert_eq!(super::next_zoom_factor(0.5, -0.05), 0.5);
        assert_eq!(super::next_zoom_factor(0.52, -0.05), 0.5);
    }

    #[test]
    fn next_zoom_factor_many_steps_stay_in_bounds() {
        let mut f = 1.0;
        for _ in 0..200 {
            f = super::next_zoom_factor(f, 0.05);
        }
        assert_eq!(f, 2.0);
        for _ in 0..200 {
            f = super::next_zoom_factor(f, -0.05);
        }
        assert_eq!(f, 0.5);
    }
}
