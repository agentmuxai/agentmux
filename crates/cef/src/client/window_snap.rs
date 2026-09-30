// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Pure geometry for Chrome-style window snapping —
//! `docs/specs/SPEC_WINDOW_SNAP_MAXIMIZE_2026_09_04.md`.
//!
//! Deliberately **not** `#[cfg(target_os = "windows")]` and deliberately free
//! of any Win32 type: the callers (`client::wndproc`'s `WM_SIZING` arm for
//! border-drag vertical snap, `ui_tasks::drag`'s move loop for
//! drag-to-top maximize) are both native message-loop code that no test
//! harness can drive, so the only way any of this logic gets asserted is by
//! keeping the decisions here, as plain integer math, and letting the
//! platform code do nothing but read/apply the answer.
//!
//! **All coordinates are PHYSICAL screen pixels.** Both call sites work in
//! physical px (`GetWindowRect`/`GetCursorPos`/`MONITORINFO.rcWork` all
//! return physical), and `app::monitor::get_monitor_work_area` is
//! deliberately NOT used by either — it converts to DIP for CEF's
//! `Window::set_bounds`, and mixing the two units is a bug class this
//! codebase has been bitten by before. Nothing here converts anything;
//! callers must pass one consistent unit.

/// How close (physical px) a dragged edge must get before it snaps. Windows'
/// own edge magnetism is in this neighbourhood; small enough that a user
/// deliberately sizing a window near the screen edge isn't fighting it,
/// large enough to be reachable without pixel-hunting.
pub(crate) const SNAP_THRESHOLD_PX: i32 = 12;

/// Which horizontal edge of the window a `WM_SIZING` drag is moving.
/// Corners count as their vertical component — dragging the top-left corner
/// toward the screen top should still offer the vertical snap.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VerticalEdge {
    Top,
    Bottom,
}

/// Map a `WM_SIZING` `wParam` (`WMSZ_*`) to the vertical edge being dragged,
/// or `None` for a purely horizontal drag (left/right edge — no vertical
/// snap applies).
///
/// Values are the raw `WMSZ_*` constants, matching the existing `edge`
/// string mapping in `wndproc::window_edge_resize_wndproc` (which this
/// intentionally mirrors rather than re-deriving from a shared enum — that
/// mapping predates this module and is load-bearing for the
/// `windowresize:tick` payload, so the two are kept side by side and
/// pinned by `wmsz_mapping_matches_the_tick_event_edges`).
pub(crate) fn vertical_edge_for_wmsz(wparam: usize) -> Option<VerticalEdge> {
    match wparam {
        3 | 4 | 5 => Some(VerticalEdge::Top),    // WMSZ_TOP / TOPLEFT / TOPRIGHT
        6 | 7 | 8 => Some(VerticalEdge::Bottom), // WMSZ_BOTTOM / BOTTOMLEFT / BOTTOMRIGHT
        _ => None,                               // WMSZ_LEFT / WMSZ_RIGHT / unknown
    }
}

/// Decide whether an in-progress vertical border drag should snap to fill
/// the monitor vertically, and if so return the `(top, bottom)` the window
/// should take.
///
/// **Fills BOTH edges, not just the dragged one** — this is the behavior the
/// feature request describes ("the top and bottom should snap to the top and
/// bottom") and what Windows/Chrome actually do: dragging the top border to
/// the screen top makes the window full-height, it does not merely stop the
/// top edge there. The horizontal axis is untouched by construction (this
/// returns no x/width), which is exactly what distinguishes this from an
/// accidental full maximize.
///
/// Returns `None` when the drag isn't near the relevant work-area edge, or
/// when the window already exactly fills the work area vertically (nothing
/// to change — avoids rewriting the rect and re-logging on every one of the
/// many `WM_SIZING` ticks a single drag produces).
pub(crate) fn snap_vertical_fill(
    edge: VerticalEdge,
    proposed_top: i32,
    proposed_bottom: i32,
    work_top: i32,
    work_bottom: i32,
    threshold: i32,
) -> Option<(i32, i32)> {
    // Only the edge actually under the cursor arms the snap. Checking both
    // would let a window whose *opposite* edge happens to sit near the
    // screen edge snap while the user drags the other one — surprising, and
    // not what either Windows or Chrome do.
    let near_dragged_edge = match edge {
        VerticalEdge::Top => (proposed_top - work_top).abs() <= threshold,
        VerticalEdge::Bottom => (proposed_bottom - work_bottom).abs() <= threshold,
    };
    if !near_dragged_edge {
        return None;
    }
    if proposed_top == work_top && proposed_bottom == work_bottom {
        return None; // already filled — no-op
    }
    Some((work_top, work_bottom))
}

/// Undo a vertical snap when the drag leaves the snap zone: restore the
/// **non-dragged** edge to where it was before this drag started.
///
/// Without this, backing out of a snap is one-way — [`snap_vertical_fill`]
/// moved the opposite edge to the work-area edge, and simply declining to
/// snap on later ticks leaves it stranded there (the native resize only ever
/// moves the edge under the cursor, so nothing else would ever put it back).
/// Chrome/Windows both restore it, which is the behavior being matched.
///
/// Only the opposite edge is restored — the dragged edge stays wherever the
/// cursor currently puts it.
///
/// Returns the `(top, bottom)` to apply, or `None` when the opposite edge is
/// already at its pre-drag value (the common case: every tick of a drag that
/// never snapped at all, where this must be a no-op rather than a redundant
/// rect write).
pub(crate) fn unsnap_restore_opposite_edge(
    edge: VerticalEdge,
    current_top: i32,
    current_bottom: i32,
    origin_top: i32,
    origin_bottom: i32,
) -> Option<(i32, i32)> {
    match edge {
        VerticalEdge::Top if current_bottom != origin_bottom => {
            Some((current_top, origin_bottom))
        }
        VerticalEdge::Bottom if current_top != origin_top => {
            Some((origin_top, current_bottom))
        }
        _ => None,
    }
}

/// Where to place a window that is being un-maximized because the user
/// started dragging it — the other half of the drag-to-top gesture, and what
/// Chrome/Windows both do when you drag a maximized title bar.
///
/// The window must land *under the cursor*, or the drag would feel like it
/// teleported: the restored window is smaller than the maximized one, so
/// keeping its top-left fixed would leave the cursor somewhere else entirely
/// (often outside the window). Two rules, matching the OS:
///
/// - **Horizontally, proportional.** The cursor keeps the same *fractional*
///   position along the title bar it had while maximized — grab the middle
///   of a maximized title bar and you're still holding the middle of the
///   restored one. Absolute-offset would drop the cursor off the right edge
///   whenever the restored window is much narrower.
/// - **Vertically, same absolute offset.** Title bars are a fixed height, so
///   preserving "N px below the window top" keeps the grab point on the same
///   part of the chrome. Proportional here would slide the grab off the
///   title bar entirely for a short window.
///
/// All inputs/outputs are PHYSICAL px. Returns the restored window's new
/// `(x, y)` top-left.
pub(crate) fn unmaximize_drag_origin(
    cursor_x: i32,
    cursor_y: i32,
    maximized_left: i32,
    maximized_top: i32,
    maximized_width: i32,
    restored_width: i32,
) -> (i32, i32) {
    // Guard the degenerate case rather than dividing by zero — a zero-width
    // maximized rect shouldn't be reachable, but this runs inside a drag
    // loop where a panic would wedge the UI thread mid-gesture.
    let ratio = if maximized_width > 0 {
        (cursor_x - maximized_left) as f64 / maximized_width as f64
    } else {
        0.5
    };
    let new_x = cursor_x - (restored_width as f64 * ratio).round() as i32;
    let new_y = cursor_y - (cursor_y - maximized_top);
    (new_x, new_y)
}

/// Whether a drag-to-top *move* (not resize) is currently in the
/// maximize-offer zone: the cursor at/above the work area's top edge.
///
/// Separate from [`snap_vertical_fill`] because the gestures differ in kind,
/// not just in threshold — a move gesture tracks the CURSOR (the window
/// follows it, so the window's own top edge is wherever the grab offset put
/// it and is not a meaningful signal), whereas a resize gesture tracks the
/// proposed EDGE. Conflating them would make the maximize offer depend on
/// where in the title bar the user happened to grab.
pub(crate) fn cursor_in_top_maximize_zone(
    cursor_y: i32,
    work_top: i32,
    threshold: i32,
) -> bool {
    cursor_y <= work_top + threshold
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wmsz_top_family_maps_to_top_including_corners() {
        assert_eq!(vertical_edge_for_wmsz(3), Some(VerticalEdge::Top)); // WMSZ_TOP
        assert_eq!(vertical_edge_for_wmsz(4), Some(VerticalEdge::Top)); // TOPLEFT
        assert_eq!(vertical_edge_for_wmsz(5), Some(VerticalEdge::Top)); // TOPRIGHT
    }

    #[test]
    fn wmsz_bottom_family_maps_to_bottom_including_corners() {
        assert_eq!(vertical_edge_for_wmsz(6), Some(VerticalEdge::Bottom)); // WMSZ_BOTTOM
        assert_eq!(vertical_edge_for_wmsz(7), Some(VerticalEdge::Bottom)); // BOTTOMLEFT
        assert_eq!(vertical_edge_for_wmsz(8), Some(VerticalEdge::Bottom)); // BOTTOMRIGHT
    }

    /// A purely horizontal drag must never arm the vertical snap — this is
    /// what keeps "drag the left edge near the screen's left" from
    /// unexpectedly changing the window's height.
    #[test]
    fn wmsz_horizontal_and_unknown_map_to_no_vertical_edge() {
        assert_eq!(vertical_edge_for_wmsz(1), None); // WMSZ_LEFT
        assert_eq!(vertical_edge_for_wmsz(2), None); // WMSZ_RIGHT
        assert_eq!(vertical_edge_for_wmsz(0), None);
        assert_eq!(vertical_edge_for_wmsz(99), None);
    }

    /// Mirrors `window_edge_resize_wndproc`'s own WMSZ_* → edge-string map,
    /// so the two can't silently drift apart (the tick event says "topleft"
    /// for 4; this module must agree that 4 is a Top-family drag).
    #[test]
    fn wmsz_mapping_matches_the_tick_event_edges() {
        let tick_edge = |w: usize| match w {
            1 => "left",
            2 => "right",
            3 => "top",
            4 => "topleft",
            5 => "topright",
            6 => "bottom",
            7 => "bottomleft",
            8 => "bottomright",
            _ => "",
        };
        for w in 0..=9usize {
            let name = tick_edge(w);
            let expected = if name.starts_with("top") {
                Some(VerticalEdge::Top)
            } else if name.starts_with("bottom") {
                Some(VerticalEdge::Bottom)
            } else {
                None
            };
            assert_eq!(vertical_edge_for_wmsz(w), expected, "wmsz={w} ({name})");
        }
    }

    #[test]
    fn top_drag_within_threshold_fills_vertically() {
        // Work area 0..1000; window currently 5..600, dragging its top edge.
        let got = snap_vertical_fill(VerticalEdge::Top, 5, 600, 0, 1000, 12);
        assert_eq!(got, Some((0, 1000)), "should fill BOTH edges, not just the dragged one");
    }

    #[test]
    fn bottom_drag_within_threshold_fills_vertically() {
        let got = snap_vertical_fill(VerticalEdge::Bottom, 400, 995, 0, 1000, 12);
        assert_eq!(got, Some((0, 1000)));
    }

    #[test]
    fn snaps_when_the_edge_overshoots_past_the_work_area() {
        // Dragging above the top of the screen — still within |delta| <= threshold.
        let got = snap_vertical_fill(VerticalEdge::Top, -8, 600, 0, 1000, 12);
        assert_eq!(got, Some((0, 1000)));
    }

    #[test]
    fn does_not_snap_when_the_dragged_edge_is_far_from_the_work_area() {
        assert_eq!(snap_vertical_fill(VerticalEdge::Top, 200, 600, 0, 1000, 12), None);
        assert_eq!(snap_vertical_fill(VerticalEdge::Bottom, 200, 600, 0, 1000, 12), None);
    }

    /// The opposite edge being near the screen edge must NOT arm the snap —
    /// only the edge actually being dragged counts.
    #[test]
    fn does_not_snap_on_the_edge_that_is_not_being_dragged() {
        // Bottom is at the work-area bottom, but the user is dragging the TOP,
        // and the top is nowhere near the work-area top.
        assert_eq!(snap_vertical_fill(VerticalEdge::Top, 400, 1000, 0, 1000, 12), None);
        // Mirror: top is at the work-area top, user drags the BOTTOM far away.
        assert_eq!(snap_vertical_fill(VerticalEdge::Bottom, 0, 500, 0, 1000, 12), None);
    }

    #[test]
    fn already_filled_is_a_no_op() {
        assert_eq!(snap_vertical_fill(VerticalEdge::Top, 0, 1000, 0, 1000, 12), None);
        assert_eq!(snap_vertical_fill(VerticalEdge::Bottom, 0, 1000, 0, 1000, 12), None);
    }

    /// Non-zero work-area origin — a secondary monitor placed above/left of
    /// the primary has negative coordinates, and a taskbar makes rcWork.top
    /// non-zero even on the primary. Neither may be assumed to be 0.
    #[test]
    fn handles_a_non_zero_and_negative_work_area_origin() {
        // Secondary monitor above the primary: work area -1080..-40.
        let got = snap_vertical_fill(VerticalEdge::Top, -1075, -500, -1080, -40, 12);
        assert_eq!(got, Some((-1080, -40)));
        // Primary with a 40px taskbar at the top: work area 40..1040.
        let got = snap_vertical_fill(VerticalEdge::Top, 45, 600, 40, 1040, 12);
        assert_eq!(got, Some((40, 1040)));
    }

    #[test]
    fn threshold_boundary_is_inclusive() {
        assert_eq!(snap_vertical_fill(VerticalEdge::Top, 12, 600, 0, 1000, 12), Some((0, 1000)));
        assert_eq!(snap_vertical_fill(VerticalEdge::Top, 13, 600, 0, 1000, 12), None);
    }

    /// The reported bug: drag the top edge up to snap (bottom moves to the
    /// work-area bottom), then drag back down out of the zone — the bottom
    /// must return to where it was before the drag, not stay stranded at the
    /// screen edge.
    #[test]
    fn dragging_back_out_of_a_snap_restores_the_opposite_edge() {
        // Pre-drag window was 300..600. A snap moved it to 0..1000. The user
        // has now dragged the top back down to 250.
        let got = unsnap_restore_opposite_edge(VerticalEdge::Top, 250, 1000, 300, 600);
        assert_eq!(got, Some((250, 600)), "bottom must revert to its pre-drag 600");
    }

    #[test]
    fn dragging_back_out_of_a_bottom_snap_restores_the_top() {
        // Mirror: pre-drag 300..600, snapped to 0..1000, bottom dragged to 700.
        let got = unsnap_restore_opposite_edge(VerticalEdge::Bottom, 0, 700, 300, 600);
        assert_eq!(got, Some((300, 700)), "top must revert to its pre-drag 300");
    }

    /// Every tick of an ordinary drag that never snapped hits this path — it
    /// must be a no-op, not a redundant rect write on every WM_SIZING.
    #[test]
    fn restore_is_a_no_op_when_the_opposite_edge_never_moved() {
        assert_eq!(
            unsnap_restore_opposite_edge(VerticalEdge::Top, 250, 600, 300, 600),
            None,
        );
        assert_eq!(
            unsnap_restore_opposite_edge(VerticalEdge::Bottom, 300, 700, 300, 600),
            None,
        );
    }

    /// The dragged edge is never restored — only the opposite one. A top
    /// drag must keep whatever the cursor currently dictates for `top`, even
    /// though it differs wildly from the origin.
    #[test]
    fn restore_never_touches_the_edge_being_dragged() {
        let (top, _bottom) =
            unsnap_restore_opposite_edge(VerticalEdge::Top, 250, 1000, 300, 600).unwrap();
        assert_eq!(top, 250, "dragged edge must follow the cursor, not the origin");

        let (_top, bottom) =
            unsnap_restore_opposite_edge(VerticalEdge::Bottom, 0, 700, 300, 600).unwrap();
        assert_eq!(bottom, 700);
    }

    #[test]
    fn unmaximize_keeps_the_cursor_proportionally_along_the_title_bar() {
        // Maximized 0..1920, cursor at the midpoint (960). Restored width 800
        // → cursor should still be at the restored window's midpoint, so the
        // window's left edge lands 400px left of the cursor.
        let (x, _y) = unmaximize_drag_origin(960, 10, 0, 0, 1920, 800);
        assert_eq!(x, 560);
        assert_eq!(960 - x, 400, "cursor stays at the restored midpoint");
    }

    #[test]
    fn unmaximize_handles_a_grab_near_the_right_end_of_the_title_bar() {
        // Grabbed at 90% across. An absolute-offset scheme would put the
        // window left edge at 1728-... far off-screen and drop the cursor
        // past the right edge; proportional keeps it at 90% of 800 = 720.
        let (x, _y) = unmaximize_drag_origin(1728, 10, 0, 0, 1920, 800);
        assert_eq!(x, 1008);
        assert_eq!(1728 - x, 720);
    }

    #[test]
    fn unmaximize_preserves_the_absolute_vertical_grab_offset() {
        // Cursor 18px below the maximized window's top → the restored window
        // must also sit 18px above the cursor, keeping the grab on the title bar.
        let (_x, y) = unmaximize_drag_origin(500, 58, 0, 40, 1920, 800);
        assert_eq!(y, 40);
        assert_eq!(58 - y, 18);
    }

    #[test]
    fn unmaximize_respects_a_non_zero_maximized_origin() {
        // Secondary monitor at x=1920, taskbar making top=40.
        let (x, y) = unmaximize_drag_origin(2880, 60, 1920, 40, 1920, 800);
        assert_eq!(x, 2480, "midpoint grab → 400px left of cursor");
        assert_eq!(y, 40);
    }

    /// Must not panic or divide by zero inside a modal drag loop.
    #[test]
    fn unmaximize_survives_a_degenerate_zero_width_maximized_rect() {
        let (x, y) = unmaximize_drag_origin(500, 30, 0, 0, 0, 800);
        assert_eq!(x, 100, "falls back to a centered grab");
        assert_eq!(y, 0);
    }

    #[test]
    fn cursor_zone_covers_at_and_above_the_work_area_top() {
        assert!(cursor_in_top_maximize_zone(0, 0, 12));
        assert!(cursor_in_top_maximize_zone(12, 0, 12));
        assert!(cursor_in_top_maximize_zone(-30, 0, 12), "above the screen still counts");
        assert!(!cursor_in_top_maximize_zone(13, 0, 12));
    }

    #[test]
    fn cursor_zone_respects_a_non_zero_work_area_top() {
        // 40px taskbar at the top: the zone starts at 40, not 0.
        assert!(cursor_in_top_maximize_zone(45, 40, 12));
        assert!(!cursor_in_top_maximize_zone(60, 40, 12));
    }
}
