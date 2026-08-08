// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Zoom operations for `BrowserPaneManager`: `zoom_in`/`zoom_out`/`step_zoom`/
//! `apply_zoom`/`reapply_zoom`, plus the pure `next_zoom_factor` clamp
//! arithmetic. Split out of `browser_panes.rs` — see that module's doc
//! comment.

use std::sync::Arc;

use cef::*;

use crate::state::AppState;

use super::BrowserPaneManager;

impl BrowserPaneManager {
    /// Ctrl+Wheel step, in the same +/-0.05 units and 0.5-2.0 clamp range
    /// `frontend/app/store/zoom.win32.ts`'s WHEEL_STEP uses for every other
    /// pane type, for a consistent feel even though this path never touches
    /// that frontend module (browser-pane content is unreachable from the
    /// DOM — see the module doc on `AppState::browser_pane_zoom`).
    const ZOOM_STEP: f64 = 0.05;

    pub fn zoom_in(&self, block_id: &str, state: &Arc<AppState>) {
        self.step_zoom(block_id, state, Self::ZOOM_STEP);
    }
    pub fn zoom_out(&self, block_id: &str, state: &Arc<AppState>) {
        self.step_zoom(block_id, state, -Self::ZOOM_STEP);
    }

    fn step_zoom(&self, block_id: &str, state: &Arc<AppState>, delta: f64) {
        let new_factor = {
            let mut map = state.browser_pane_zoom.lock();
            let current = *map.get(block_id).unwrap_or(&1.0);
            let next = next_zoom_factor(current, delta);
            map.insert(block_id.to_string(), next);
            next
        };
        self.apply_zoom(block_id, state, new_factor);
    }

    /// Inject the pane's current zoom factor as CSS `zoom` via
    /// `execute_java_script` on its main frame. Deliberately not Chromium's
    /// native `SetZoomLevel` — that's scoped by `HostZoomMap`, shared across
    /// every browser pane on the same host/RequestContext (the bug this
    /// exists to fix). CSS injection is per-`CefFrame` by construction, no
    /// native zoom system involved at all.
    fn apply_zoom(&self, block_id: &str, state: &Arc<AppState>, factor: f64) {
        if let Some(browser) = self.live_browser(state, block_id) {
            if let Some(frame) = browser.main_frame() {
                let js = format!("document.documentElement.style.zoom = '{factor}';");
                let code = CefString::from(js.as_str());
                let url = CefString::from("");
                frame.execute_java_script(Some(&code), Some(&url), 0);
            }
        }
    }

    /// Re-apply this pane's stored zoom (if it's ever been changed from the
    /// 1.0 default) after a navigation — a fresh page load replaces the
    /// previous page's DOM/inline-style state entirely, so the CSS `zoom`
    /// injected before that navigation is gone along with it. Called from
    /// `browser_pane::callbacks::on_load_end_browser_pane`, which already
    /// fires after every navigation for other per-pane HWND bookkeeping.
    /// A no-op for panes that were never zoomed (no map entry) — every
    /// pane's first ever load doesn't pay for a wasted execute_java_script
    /// call setting zoom to its already-default 1.0.
    pub fn reapply_zoom(&self, block_id: &str, state: &Arc<AppState>) {
        let factor = state.browser_pane_zoom.lock().get(block_id).copied();
        if let Some(factor) = factor {
            self.apply_zoom(block_id, state, factor);
        }
    }
}

/// Pure clamp+step arithmetic for `BrowserPaneManager::step_zoom`, split out
/// so it's testable without a real `AppState`/lock/browser — mirrors why
/// `BrowserPaneCloseOps` exists as its own trait (see that trait's doc
/// comment in `mod.rs`): the CEF/HWND side can't be unit-tested meaningfully,
/// but the logic feeding it can.
fn next_zoom_factor(current: f64, delta: f64) -> f64 {
    (current + delta).clamp(0.5, 2.0)
}

#[cfg(test)]
mod tests {
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
