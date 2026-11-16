// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a DOM overlay cut out of a pane's native window means for the page's keyboard and mouse.
//!
//! On macOS a pane is its own window floating above the app's, so an overlay over it is shown through a hole in
//! the pane (`ui_tasks/pane_hole_mask.rs`) and every event for that window has to be sent somewhere else while
//! the hole is open. For a menu or a dialog that is the app, for the whole window: they take the keyboard and
//! the mouse. A toast is not that: the page keeps the keyboard, and the mouse goes to the toast over the toast's
//! pixels and to the page everywhere else (`ui_tasks/pane_hole_mouse.rs`). The frontend marks the second kind
//! `passive` in the clip it sends (`pane-overlay.ts`).
//!
//! The rules live here, free of AppKit, so every platform's tests check them; only macOS applies them.
#![allow(dead_code)]

/// `(x, y, width, height)`.
pub(crate) type Rect = (i32, i32, i32, i32);

/// A rect in points of a window, origin at its bottom-left corner (AppKit's).
pub(crate) type LocalRect = (f64, f64, f64, f64);

/// The part of one overlay that is over a pane.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Hole {
    pub rect: Rect,
    /// Shown over the page without taking it over (a toast).
    pub passive: bool,
}

/// Where the mouse goes for the pane's window while its holes are open.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MouseRouting {
    /// No hole: the page has the mouse.
    Page,
    /// A menu or a dialog is open over it: the whole window passes the mouse to the app.
    App,
    /// Only toasts are over it: the mouse goes to the app over a toast's pixels and to the page elsewhere.
    ByPixel,
}

/// Two axis-aligned rects intersect iff neither is fully to one side of the other.
pub(crate) fn rects_intersect(a: Rect, b: Rect) -> bool {
    let (ax, ay, aw, ah) = a;
    let (bx, by, bw, bh) = b;
    !(ax + aw <= bx || bx + bw <= ax || ay + ah <= by || by + bh <= ay)
}

/// The holes `overlays` cut in the pane at `pane`: each overlay that covers part of it, cut to the pane.
/// `passive[i]` says whether `overlays[i]` is a passive one; one with no entry is not (an older frontend
/// sends none).
pub(crate) fn holes_in_pane(pane: Rect, overlays: &[Rect], passive: &[bool]) -> Vec<Hole> {
    let (px, py, pw, ph) = pane;
    overlays
        .iter()
        .enumerate()
        .filter(|(_, o)| rects_intersect(**o, pane))
        .map(|(i, &(ox, oy, ow, oh))| {
            let hx = ox.max(px);
            let hy = oy.max(py);
            let hr = (ox + ow).min(px + pw);
            let hb = (oy + oh).min(py + ph);
            Hole { rect: (hx, hy, hr - hx, hb - hy), passive: passive.get(i).copied().unwrap_or(false) }
        })
        .collect()
}

/// Whether the holes take the keyboard from the page: one that is not passive does (Escape closes a menu,
/// typing goes into a popover's field), and only when the pane held it.
pub(crate) fn takes_keys(holes: &[Hole]) -> bool {
    holes.iter().any(|h| !h.passive)
}

/// Where the mouse goes for a pane with these holes.
pub(crate) fn mouse_routing(holes: &[Hole]) -> MouseRouting {
    if holes.is_empty() {
        MouseRouting::Page
    } else if takes_keys(holes) {
        MouseRouting::App
    } else {
        MouseRouting::ByPixel
    }
}

/// Whether the point is inside one of the holes (edges: left and bottom in, right and top out).
pub(crate) fn point_in_any(holes: &[LocalRect], (px, py): (f64, f64)) -> bool {
    holes.iter().any(|&(x, y, w, h)| w > 0.0 && h > 0.0 && px >= x && px < x + w && py >= y && py < y + h)
}

/// Whether the pane's window should ignore the mouse now, for a cursor `inside` a toast's hole or not. A button
/// held down keeps things as they are: a drag that began in the page goes on in the page when it crosses a toast,
/// and a press that began on a toast is finished there.
pub(crate) fn ignore_mouse(inside: bool, button_down: bool, ignoring_now: bool) -> bool {
    if button_down {
        ignoring_now
    } else {
        inside
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const PANE: Rect = (100, 100, 400, 300);

    #[test]
    fn an_overlay_that_does_not_touch_the_pane_makes_no_hole() {
        assert!(holes_in_pane(PANE, &[(0, 0, 50, 50), (500, 100, 20, 20), (100, 400, 40, 40)], &[]).is_empty());
    }

    #[test]
    fn a_hole_is_the_overlay_cut_to_the_pane() {
        let h = holes_in_pane(PANE, &[(450, 50, 200, 100)], &[true]);
        assert_eq!(h, vec![Hole { rect: (450, 100, 50, 50), passive: true }]);
    }

    #[test]
    fn the_passive_mark_goes_with_its_own_overlay_not_its_place_in_the_list_of_holes() {
        // The first overlay is off the pane: the second one's mark must not slip onto the third's hole.
        let o = [(0, 0, 10, 10), (120, 120, 30, 30), (200, 200, 30, 30)];
        let h = holes_in_pane(PANE, &o, &[false, true, false]);
        assert_eq!(h.iter().map(|h| h.passive).collect::<Vec<_>>(), vec![true, false]);
    }

    #[test]
    fn an_overlay_with_no_mark_is_not_passive() {
        // An older frontend sends no marks; a short list leaves the rest unmarked too.
        let o = [(120, 120, 30, 30), (200, 200, 30, 30)];
        assert!(holes_in_pane(PANE, &o, &[]).iter().all(|h| !h.passive));
        assert_eq!(holes_in_pane(PANE, &o, &[true]).iter().map(|h| h.passive).collect::<Vec<_>>(), vec![true, false]);
    }

    #[test]
    fn only_a_hole_that_is_not_passive_takes_the_keyboard() {
        let toast = Hole { rect: (0, 0, 10, 10), passive: true };
        let menu = Hole { rect: (0, 0, 10, 10), passive: false };
        assert!(!takes_keys(&[]));
        assert!(!takes_keys(&[toast, toast]));
        assert!(takes_keys(&[menu]));
        // A menu opened over a toast takes it, and gives the pane back only when it closes.
        assert!(takes_keys(&[toast, menu]));
    }

    #[test]
    fn the_mouse_goes_by_pixel_only_for_toasts_alone() {
        let toast = Hole { rect: (0, 0, 10, 10), passive: true };
        let menu = Hole { rect: (0, 0, 10, 10), passive: false };
        assert_eq!(mouse_routing(&[]), MouseRouting::Page);
        assert_eq!(mouse_routing(&[toast]), MouseRouting::ByPixel);
        assert_eq!(mouse_routing(&[toast, toast]), MouseRouting::ByPixel);
        assert_eq!(mouse_routing(&[menu]), MouseRouting::App);
        assert_eq!(mouse_routing(&[toast, menu]), MouseRouting::App);
    }

    #[test]
    fn a_point_is_in_a_hole_from_its_left_and_bottom_edge_to_before_its_right_and_top_one() {
        let holes = [(10.0, 20.0, 30.0, 40.0)];
        assert!(point_in_any(&holes, (10.0, 20.0)));
        assert!(point_in_any(&holes, (39.9, 59.9)));
        assert!(!point_in_any(&holes, (40.0, 30.0)));
        assert!(!point_in_any(&holes, (20.0, 60.0)));
        assert!(!point_in_any(&holes, (9.9, 30.0)));
        assert!(!point_in_any(&[], (1.0, 1.0)));
        assert!(!point_in_any(&[(5.0, 5.0, 0.0, 10.0)], (5.0, 6.0)));
    }

    #[test]
    fn the_cursor_decides_unless_a_button_is_down() {
        assert!(ignore_mouse(true, false, false));
        assert!(!ignore_mouse(false, false, true));
        // Pressed in the page and dragged over a toast: still the page's.
        assert!(!ignore_mouse(true, true, false));
        // Pressed on a toast and dragged off it: still the toast's.
        assert!(ignore_mouse(false, true, true));
    }
}
