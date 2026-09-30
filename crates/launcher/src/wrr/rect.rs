// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase B.9.1 — rectangle math for monitor-membership classification.
//
// Win32 `RECT` semantics: `right` and `bottom` are one past the
// last included pixel, so two rects intersect iff they overlap on
// both axes after the half-open boundary check. Empty rects
// (zero area) never intersect anything by definition.

use agentmux_common::ipc::Rect;

/// Phase B.9.1 — does `r` overlap with `m`? Half-open semantics.
pub fn intersects(r: &Rect, m: &Rect) -> bool {
    if r.left >= r.right || r.top >= r.bottom {
        return false;
    }
    if m.left >= m.right || m.top >= m.bottom {
        return false;
    }
    r.left < m.right && r.right > m.left && r.top < m.bottom && r.bottom > m.top
}

/// Phase B.9.1 — does `r` overlap with at least one monitor in
/// `monitors`? Empty `monitors` returns `false` — no monitors,
/// nothing to be "on".
pub fn intersects_any(r: &Rect, monitors: &[Rect]) -> bool {
    monitors.iter().any(|m| intersects(r, m))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(l: i32, t: i32, r: i32, b: i32) -> Rect {
        Rect { left: l, top: t, right: r, bottom: b }
    }

    #[test]
    fn fully_inside_intersects() {
        let mon = rect(0, 0, 1920, 1080);
        let win = rect(100, 100, 800, 600);
        assert!(intersects(&win, &mon));
    }

    #[test]
    fn touching_edge_does_not_intersect() {
        // Half-open: `right == left` of the next means no overlap.
        let mon = rect(0, 0, 1920, 1080);
        let win = rect(1920, 100, 2920, 600);
        assert!(!intersects(&win, &mon));
    }

    #[test]
    fn fully_off_screen_no_intersect() {
        let mon = rect(0, 0, 1920, 1080);
        let off = rect(-9999, -9999, -9000, -9000);
        assert!(!intersects(&off, &mon));
    }

    #[test]
    fn intersects_any_picks_correct_monitor() {
        let monitors = vec![
            rect(0, 0, 1920, 1080),       // primary
            rect(1920, 0, 3840, 1080),    // secondary right
        ];
        let win = rect(2000, 100, 2500, 600);
        assert!(intersects_any(&win, &monitors));
    }

    #[test]
    fn empty_monitors_returns_false() {
        let win = rect(100, 100, 200, 200);
        assert!(!intersects_any(&win, &[]));
    }

    #[test]
    fn empty_rect_never_intersects() {
        let mon = rect(0, 0, 1920, 1080);
        let zero = rect(100, 100, 100, 100);
        assert!(!intersects(&zero, &mon));
    }

    #[test]
    fn cross_monitor_window_intersects_one() {
        // Window straddling two monitors picks up at least one.
        let monitors = vec![
            rect(0, 0, 1920, 1080),
            rect(1920, 0, 3840, 1080),
        ];
        let straddle = rect(1800, 100, 2200, 600);
        assert!(intersects_any(&straddle, &monitors));
    }
}
