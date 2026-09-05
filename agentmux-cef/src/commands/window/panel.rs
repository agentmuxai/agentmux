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

/// Where to put the panel, given a monitor work area `(x, y, w, h)` and that
/// monitor's DPI `scale`. **Everything here is PHYSICAL pixels**, including
/// the returned size — `PANEL_WIDTH`/`PANEL_HEIGHT` are logical, so they are
/// scaled on the way in.
///
/// Anchored to the **bottom-right** of the work area: that is where the tray
/// lives on the default Windows layout, so the panel appears next to the thing
/// the user just clicked. The work area (not the full monitor rect) is used so
/// it never lands under the taskbar.
///
/// Clamps rather than assuming it fits: on a small or heavily scaled display
/// the panel can exceed the work area, and a negative-size or off-work-area
/// rect would put the window somewhere unreachable.
pub fn panel_rect(work: (i32, i32, i32, i32), scale: f32) -> (i32, i32, i32, i32) {
    let (wx, wy, ww, wh) = work;
    let scale = if scale > 0.0 { scale } else { 1.0 };
    let margin = (PANEL_MARGIN as f32 * scale).round() as i32;

    let want_w = (PANEL_WIDTH as f32 * scale).round() as i32;
    let want_h = (PANEL_HEIGHT as f32 * scale).round() as i32;

    // Never larger than the work area minus margins; never smaller than 1px,
    // so a pathological work area cannot produce a window with no size.
    let w = want_w.min((ww - 2 * margin).max(1));
    let h = want_h.min((wh - 2 * margin).max(1));

    // Bottom-right, clamped back inside the work area for the case where the
    // panel is wider/taller than the work area itself.
    let x = (wx + ww - w - margin).max(wx);
    let y = (wy + wh - h - margin).max(wy);

    (x, y, w, h)
}

/// Open the tray panel.
///
/// Pool-first (instant), cold-path fallback. Returns the window label.
///
/// The panel is a *normal* top-level window as far as the rest of the host is
/// concerned — it registers, counts toward `count_live_user_windows`, and keeps
/// the instance attended. Deliberate: a panel the user has open is a window
/// they are looking at, so it should satisfy the last-window quit gate and
/// suppress the unattended audit period exactly like any other.
pub fn open_panel(state: &Arc<AppState>) -> Result<serde_json::Value, String> {
    // H.7 invariant. `open_new_window` checks this before ITS pool promote for
    // a reason: creating or promoting a top-level CEF window while a pane is
    // in `Closing` hits a Chromium deadlock that wedges the message loop. The
    // cold path below inherits the guard from `open_window_with_kind`, but the
    // pool promote does not — so without this a tray click arriving mid-close
    // takes the unguarded path (Codex P1 on PR #3002).
    if state.any_browser_pane_closing() {
        tracing::warn!(
            target: "wfr:gate",
            "[wfr:gate] open_panel refused — pane is mid-close (H.7 invariant)"
        );
        return Err("a pane is currently closing; retry shortly".to_string());
    }

    let work = panel_work_area();
    let scale = panel_scale(work);
    let (x, y, w, h) = panel_rect(work, scale);

    // Pool-first. NOTE: `promote_pool_window_for_new_window` cannot be used
    // here — its Windows branch deliberately DISCARDS width/height (to avoid
    // double-DPI-converting the already-physical values its own caller passes)
    // and falls back to POOL_WIDTH/POOL_HEIGHT, which would silently produce a
    // normal 1200x800 window whenever the pool is warm — i.e. in the common
    // case this feature is designed around (ReAgent + Codex P1 on PR #3002).
    // Calling `promote_pool_window` directly lets the panel size survive: it
    // applies `to_physical()` when width/height are `Some`, so they must be
    // handed over as LOGICAL pixels, derived back from the clamped physical
    // rect above.
    let logical_w = ((w as f32) / scale).round().max(1.0) as i32;
    let logical_h = ((h as f32) / scale).round().max(1.0) as i32;
    if let Some(label) = crate::commands::window_pool::promote_pool_window(
        state,
        "", // empty workspace id => the frontend creates a fresh workspace
        x,
        y,
        Some(logical_w),
        Some(logical_h),
        Some(x), // anchor: place the outer top-left exactly here
        Some(y),
        None,
        None,
        true, // this IS a panel — the liveness fallback must recover it as one
    ) {
        tracing::info!(target: "tray:panel", label = %label, "[panel] served from the warm pool");
        crate::ui_tasks::post_set_always_on_top(state, &label);
        return Ok(serde_json::json!(label));
    }

    // Cold path. `explicit_rect` skips the new-window offset/70%-of-monitor
    // placement heuristic and uses the rect verbatim (physical), which is what
    // makes the panel panel-sized rather than main-window-sized.
    tracing::info!(target: "tray:panel", "[panel] pool empty — cold-path window");
    let out = crate::commands::window::open_window_with_kind(
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
    )?;
    if let Some(label) = out.as_str() {
        crate::ui_tasks::post_set_always_on_top(state, label);
    }
    Ok(out)
}

/// Work area to anchor the panel in.
///
/// Resolved from the monitor under the primary origin. A more precise answer
/// would be "the monitor containing the tray icon the user clicked", but the
/// click arrives in the launcher and the position is not plumbed through; the
/// primary work area is correct on the common single-monitor case and always
/// yields an on-screen window otherwise, which is the property that matters.
fn panel_work_area() -> (i32, i32, i32, i32) {
    #[cfg(target_os = "windows")]
    {
        if let Some(wa) = crate::app::get_monitor_work_area_physical(0, 0) {
            return wa;
        }
    }
    // Conservative fallback when the OS will not say (or off-Windows):
    // smaller than essentially any real display, so the clamping above keeps
    // the panel on screen rather than guessing big.
    (0, 0, 1280, 800)
}

/// DPI scale for the monitor the panel will land on.
fn panel_scale(_work: (i32, i32, i32, i32)) -> f32 {
    #[cfg(target_os = "windows")]
    {
        return crate::app::dpi_scale_at(_work.0, _work.1);
    }
    #[allow(unreachable_code)]
    {
        1.0
    }
}

#[cfg(test)]
mod panel_tests {
    use super::*;

    #[test]
    fn panel_sits_inside_the_work_area_bottom_right() {
        let (x, y, w, h) = panel_rect((0, 0, 1920, 1040), 1.0);
        assert_eq!((w, h), (PANEL_WIDTH, PANEL_HEIGHT));
        assert_eq!(x, 1920 - PANEL_WIDTH - PANEL_MARGIN);
        assert_eq!(y, 1040 - PANEL_HEIGHT - PANEL_MARGIN);
    }

    #[test]
    fn panel_respects_a_work_area_that_is_not_at_the_origin() {
        // A secondary monitor, or a primary with a top/left taskbar: anchor
        // to the WORK AREA, not (0,0), or it lands on the wrong screen or
        // under the taskbar.
        let (x, y, w, h) = panel_rect((1920, 100, 1280, 900), 1.0);
        assert_eq!(x, 1920 + 1280 - w - PANEL_MARGIN);
        assert_eq!(y, 100 + 900 - h - PANEL_MARGIN);
        assert!(x >= 1920 && y >= 100);
    }

    /// The returned rect is PHYSICAL, so on a scaled display the panel must
    /// come back proportionally larger — otherwise it renders at half size on
    /// a 200% display.
    #[test]
    fn panel_size_scales_with_display_dpi() {
        let (_, _, w1, h1) = panel_rect((0, 0, 3840, 2100), 1.0);
        let (_, _, w2, h2) = panel_rect((0, 0, 3840, 2100), 2.0);
        assert_eq!((w1, h1), (PANEL_WIDTH, PANEL_HEIGHT));
        assert_eq!((w2, h2), (PANEL_WIDTH * 2, PANEL_HEIGHT * 2));
    }

    /// The logical size handed to `promote_pool_window` is derived by dividing
    /// the clamped physical rect back by the scale — it must round-trip, or
    /// the promoted window is the wrong size on a HiDPI display.
    #[test]
    fn physical_size_round_trips_back_to_the_logical_panel_size() {
        for scale in [1.0f32, 1.25, 1.5, 2.0] {
            let (_, _, w, h) = panel_rect((0, 0, 3840, 2160), scale);
            let logical_w = ((w as f32) / scale).round() as i32;
            let logical_h = ((h as f32) / scale).round() as i32;
            assert_eq!(logical_w, PANEL_WIDTH, "width at scale {}", scale);
            assert_eq!(logical_h, PANEL_HEIGHT, "height at scale {}", scale);
        }
    }

    #[test]
    fn panel_shrinks_to_fit_a_work_area_smaller_than_itself() {
        // Small or heavily scaled display: better a smaller panel than one
        // whose bottom half is off-screen.
        let (x, y, w, h) = panel_rect((0, 0, 300, 400), 1.0);
        assert!(w <= 300 && h <= 400);
        assert!(x >= 0 && y >= 0);
        assert!(x + w <= 300 + PANEL_MARGIN, "stays within the work area");
    }

    #[test]
    fn panel_never_has_a_zero_or_negative_size() {
        // Pathological work areas (a mis-reported monitor, a 1px strip) must
        // not produce a window with no size — CEF would create something the
        // user can neither see nor close.
        for work in [(0, 0, 1, 1), (0, 0, 0, 0), (0, 0, 32, 32)] {
            for scale in [1.0f32, 2.0] {
                let (_, _, w, h) = panel_rect(work, scale);
                assert!(w >= 1 && h >= 1, "degenerate size for {:?} @{}", work, scale);
            }
        }
    }

    #[test]
    fn a_nonsense_scale_does_not_produce_a_degenerate_panel() {
        // GetDpiForMonitor can fail; the caller substitutes a scale that must
        // not divide-by-zero or invert the rect.
        let (x, y, w, h) = panel_rect((0, 0, 1920, 1040), 0.0);
        assert!(w >= 1 && h >= 1 && x >= 0 && y >= 0);
    }

    #[test]
    fn panel_is_smaller_than_a_normal_window() {
        // The whole point: a companion surface, not a second main window.
        // An ordinary new window gets POOL_WIDTH/HEIGHT (1200x800).
        assert!(PANEL_WIDTH < 1200);
        assert!(PANEL_HEIGHT <= 800);
    }
}
