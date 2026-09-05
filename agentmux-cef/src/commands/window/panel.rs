// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Tray panel — issue #2977 Workstream 3,
//! `docs/specs/SPEC_TRAY_OPTIONAL_BACKGROUND_SERVICE_2026_09_04.md` §2.
//!
//! A small top-level window, opened from the tray, that reuses the ordinary
//! window machinery rather than being a native OS menu or a second process.
//! The spec rejects both alternatives explicitly: a native menu is "too
//! limited for agent chat", and a separate process would need its own srv
//! connection, update path, and crash supervision.
//!
//! ## Why this is small
//!
//! The panel deliberately reuses the pool promote path first, so it opens
//! instantly (a pre-warmed window repositioned and resized) and only falls
//! back to a cold-path window when the pool is empty. That is WS3's stated
//! requirement — "reusing the existing pool-window hide/show/promote
//! mechanism" — and it costs nothing extra: `promote_pool_window` already
//! accepts explicit width/height and DPI-converts them.
//!
//! ## Scope, stated honestly
//!
//! This opens the normal UI at panel dimensions. A *visually simplified*
//! panel view is frontend work (a new React surface) that cannot be designed
//! or verified without a display, so it is deliberately not attempted here;
//! the plumbing accepts an `initial_view`, so a dedicated view drops in later
//! without touching this file.

use std::sync::Arc;

use crate::state::AppState;

/// Panel size in logical pixels. Tall and narrow — a companion surface beside
/// whatever the user is actually working in, not a second main window (which
/// is what `open_new_window` is for).
pub const PANEL_WIDTH: i32 = 420;
pub const PANEL_HEIGHT: i32 = 640;

/// Gap from the work-area edges, so the panel doesn't sit flush against the
/// taskbar or screen edge.
const PANEL_MARGIN: i32 = 16;

/// Where to put the panel, given a monitor work area `(x, y, w, h)`.
///
/// Anchored to the **bottom-right** of the work area: that is where the tray
/// lives on the default Windows layout, so the panel appears next to the
/// thing the user just clicked rather than somewhere unrelated. The work area
/// (not the full monitor rect) is used so the panel never lands under the
/// taskbar.
///
/// Clamps rather than assuming it fits: on a small or scaled display the
/// panel can be larger than the work area, and a negative-size or
/// off-work-area rect would put the window somewhere the user cannot reach.
/// Pure so all of that is testable without a display.
pub fn panel_rect(work: (i32, i32, i32, i32)) -> (i32, i32, i32, i32) {
    let (wx, wy, ww, wh) = work;

    // Never larger than the work area minus margins; never smaller than
    // something usable, so a pathological work area can't produce a 0-size
    // or negative-size window.
    let w = PANEL_WIDTH.min((ww - 2 * PANEL_MARGIN).max(1));
    let h = PANEL_HEIGHT.min((wh - 2 * PANEL_MARGIN).max(1));

    // Bottom-right, then clamp back inside the work area. `max(wx/wy)` covers
    // the case where the panel is wider/taller than the work area itself.
    let x = (wx + ww - w - PANEL_MARGIN).max(wx);
    let y = (wy + wh - h - PANEL_MARGIN).max(wy);

    (x, y, w, h)
}

/// Open the tray panel.
///
/// Pool-first (instant), cold-path fallback. Returns the window label.
///
/// Note this is a *normal* top-level window as far as the rest of the host is
/// concerned — it registers, counts toward `count_live_user_windows`, and
/// keeps the instance attended. That is deliberate: a panel the user has open
/// is a window they are looking at, so it should suppress the "unattended"
/// audit period and satisfy the last-window quit gate exactly like any other.
pub fn open_panel(state: &Arc<AppState>) -> Result<serde_json::Value, String> {
    let (x, y, w, h) = panel_rect(panel_work_area());

    // Pool-first. `promote_pool_window_for_new_window` emits the
    // `pool:new-window` promote (no workspaceId), so the frontend creates a
    // fresh workspace — the right semantics for a panel, which is not
    // reattaching an existing one.
    if let Some(label) =
        crate::commands::window_pool::promote_pool_window_for_new_window(state, x, y, w, h, None, None)
    {
        tracing::info!(
            target: "tray:panel",
            label = %label,
            "[panel] served from the warm pool"
        );
        return Ok(serde_json::json!(label));
    }

    // Cold path. `explicit_rect` exists for exactly this — it skips the
    // new-window offset/70%-of-monitor placement heuristic and uses the rect
    // verbatim, which is what makes the panel panel-sized instead of
    // main-window-sized.
    tracing::info!(target: "tray:panel", "[panel] pool empty — cold-path window");
    crate::commands::window::open_window_with_kind(
        state,
        crate::state::WindowKind::FullInstance,
        None,
        None,
        None,
        Some(agentmux_common::ipc::Rect {
            left: x,
            top: y,
            right: x + w,
            bottom: y + h,
        }),
        false,
    )
}

/// Work area to anchor the panel in.
///
/// Resolved from the monitor under the *primary* origin. A more precise
/// answer would be "the monitor containing the tray icon the user clicked",
/// but the click arrives in the launcher and the position is not plumbed
/// through; anchoring to the primary work area is correct on the common
/// single-monitor case and always produces an on-screen window on the rest,
/// which is the property that actually matters.
fn panel_work_area() -> (i32, i32, i32, i32) {
    #[cfg(target_os = "windows")]
    {
        if let Some(wa) = crate::app::get_monitor_work_area_physical(0, 0) {
            return wa;
        }
    }
    // Conservative fallback when the OS won't say (or off-Windows): a
    // 1280x800 area, which is smaller than essentially any real display, so
    // the clamping above keeps the panel on screen rather than guessing big.
    (0, 0, 1280, 800)
}

#[cfg(test)]
mod panel_tests {
    use super::*;

    #[test]
    fn panel_sits_inside_the_work_area_bottom_right() {
        let (x, y, w, h) = panel_rect((0, 0, 1920, 1040));
        assert_eq!((w, h), (PANEL_WIDTH, PANEL_HEIGHT));
        assert_eq!(x, 1920 - PANEL_WIDTH - PANEL_MARGIN);
        assert_eq!(y, 1040 - PANEL_HEIGHT - PANEL_MARGIN);
    }

    #[test]
    fn panel_respects_a_work_area_that_is_not_at_the_origin() {
        // A secondary monitor, or a primary with a top/left taskbar: the
        // panel must anchor to the WORK AREA, not to (0,0), or it lands on
        // the wrong screen (or under the taskbar).
        let (x, y, w, h) = panel_rect((1920, 100, 1280, 900));
        assert_eq!(x, 1920 + 1280 - w - PANEL_MARGIN);
        assert_eq!(y, 100 + 900 - h - PANEL_MARGIN);
        assert!(x >= 1920 && y >= 100);
    }

    #[test]
    fn panel_shrinks_to_fit_a_work_area_smaller_than_itself() {
        // Small or heavily scaled display: better a smaller panel than one
        // whose bottom half is off-screen.
        let (x, y, w, h) = panel_rect((0, 0, 300, 400));
        assert!(w <= 300 && h <= 400);
        assert!(x >= 0 && y >= 0);
        assert!(x + w <= 300 + PANEL_MARGIN, "stays within the work area");
    }

    #[test]
    fn panel_never_has_a_zero_or_negative_size() {
        // Pathological work areas (a mis-reported monitor, a 1px strip)
        // must not produce a window with no size — CEF would create
        // something the user can neither see nor close.
        for work in [(0, 0, 1, 1), (0, 0, 0, 0), (0, 0, 32, 32)] {
            let (_, _, w, h) = panel_rect(work);
            assert!(w >= 1 && h >= 1, "degenerate size for work area {:?}", work);
        }
    }

    #[test]
    fn panel_is_smaller_than_a_normal_window() {
        // The whole point: a companion surface, not a second main window.
        // POOL_WIDTH/HEIGHT (1200x800) is what an ordinary new window gets.
        assert!(PANEL_WIDTH < 1200);
        assert!(PANEL_HEIGHT <= 800);
    }
}
