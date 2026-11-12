// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What a Views overlay pane shows while something of the app page is over it (a menu, a tooltip, a
//! modal), on Windows and Linux: the pane minus those rects. CEF takes it as a clip on the overlay
//! (`set_clip_rects`, an AgentMux CEF patch in the 156 r2 runtimes): outside the clip the pane is
//! neither drawn nor hit-tested, so the app page shows and takes the pointer there, and the rest of the
//! pane's page stays live. Before the patch such a pane was hidden whole and the page showed a frozen
//! picture of it (`use-freeze-frame.ts`).
//!
//! The pane is still hidden whole when nothing of it is left, when the runtime has no such field (an
//! older libcef), and when `AGENTMUX_PANE_CLIP=off` asks for the old way. macOS masks its own overlay
//! window (`ui_tasks/pane_hole_mask.rs`) and does not come here.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, OnceLock};

use cef::ImplOverlayController;
use parking_lot::Mutex;

use crate::state::AppState;

/// `(x, y, width, height)`: a pane or an overlay in its window, in the page's physical pixels, or a
/// piece of a pane in the pane's own DIPs.
type Rect = (i32, i32, i32, i32);

/// What is shown of a pane.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum PaneCut {
    /// All of it: nothing is over it.
    Whole,
    /// Nothing: it has no area, or it is covered.
    Hidden,
    /// These pieces, in the pane's own DIPs from its top-left corner.
    Rects(Vec<Rect>),
}

/// Whether panes are cut around what is over them (the default) or hidden whole
/// (`AGENTMUX_PANE_CLIP=off`). Read once: the page is told at startup (`paneClip` in the about details)
/// and freezes a frame only for a pane that hides whole.
pub(crate) fn enabled() -> bool {
    static CHOSEN: OnceLock<bool> = OnceLock::new();
    *CHOSEN.get_or_init(|| !matches!(std::env::var("AGENTMUX_PANE_CLIP").as_deref(), Ok("off" | "0" | "hide")))
}

/// What is left of a `width` × `height` pane outside `holes` (`(left, top, right, bottom)`, inside the
/// pane), as rects that do not overlap. One row of rects per band between two hole edges, and a band
/// that repeats the one above it makes its rects taller instead of adding more, so a hole in the
/// middle of a pane leaves four rects and holes side by side share their bands.
fn uncovered(width: i32, height: i32, holes: &[(i32, i32, i32, i32)]) -> Vec<Rect> {
    let mut edges: Vec<i32> = holes.iter().flat_map(|&(_, top, _, bottom)| [top, bottom]).chain([0, height]).collect();
    edges.sort_unstable();
    edges.dedup();
    let mut out: Vec<Rect> = Vec::new();
    let mut above: Vec<(i32, i32)> = Vec::new();
    for band in edges.windows(2) {
        let (top, bottom) = (band[0], band[1]);
        let mut cuts: Vec<(i32, i32)> =
            holes.iter().filter(|h| h.1 <= top && h.3 >= bottom).map(|h| (h.0, h.2)).collect();
        cuts.sort_unstable();
        let mut spans: Vec<(i32, i32)> = Vec::new();
        let mut x = 0;
        for (left, right) in cuts {
            if left > x {
                spans.push((x, left));
            }
            x = x.max(right);
        }
        if x < width {
            spans.push((x, width));
        }
        if spans == above {
            let first = out.len() - spans.len();
            out[first..].iter_mut().for_each(|rect| rect.3 += bottom - top);
        } else {
            out.extend(spans.iter().map(|&(left, right)| (left, top, right - left, bottom - top)));
            above = spans;
        }
    }
    out
}

/// What to show of the pane at `pane` with `overlays` over its window, both in the page's physical
/// pixels; `scale` is the window's display scale where Views works in DIPs (Windows) and 1 elsewhere.
///
/// The pane's own DIP rect is rounded as its overlay was placed (`creation_views::views_rect`). An
/// overlay is grown to whole DIPs, so at 125% or 150% no line of the pane's page is left over the edge
/// of a menu. The cut reaches one DIP past the pane's right and bottom edges: at 125% the overlay is
/// drawn a fraction of a DIP larger than its rounded size, and a clip that stopped at the rounded size
/// took the last row and column of the page with it.
pub(crate) fn pane_cut(pane: Rect, overlays: &[Rect], scale: f64) -> PaneCut {
    if pane.2 <= 0 || pane.3 <= 0 {
        return PaneCut::Hidden;
    }
    let near = |v: i32| (v as f64 / scale).round() as i32;
    let (left, top, width, height) = (near(pane.0), near(pane.1), near(pane.2) + 1, near(pane.3) + 1);
    let down = |v: i32| (v as f64 / scale).floor() as i32;
    let up = |v: i32| (v as f64 / scale).ceil() as i32;
    let holes: Vec<(i32, i32, i32, i32)> = overlays
        .iter()
        .filter(|o| o.2 > 0 && o.3 > 0)
        .map(|&(x, y, w, h)| {
            ((down(x) - left).max(0), (down(y) - top).max(0), (up(x + w) - left).min(width), (up(y + h) - top).min(height))
        })
        .filter(|hole| hole.0 < hole.2 && hole.1 < hole.3)
        .collect();
    if holes.is_empty() {
        return PaneCut::Whole;
    }
    let rects = uncovered(width, height, &holes);
    if rects.is_empty() { PaneCut::Hidden } else { PaneCut::Rects(rects) }
}

/// The old way, for a runtime without the clip or with it turned off: a pane with anything over it is
/// hidden whole.
fn whole_or_hidden(pane: Rect, overlays: &[Rect]) -> PaneCut {
    let (x, y, w, h) = pane;
    let over = |o: &Rect| o.0 < x + w && x < o.0 + o.2 && o.1 < y + h && y < o.1 + o.3;
    if w <= 0 || h <= 0 || overlays.iter().any(over) { PaneCut::Hidden } else { PaneCut::Whole }
}

type SetClipRects = unsafe extern "C" fn(*mut cef::sys::_cef_overlay_controller_t, usize, *const cef::sys::cef_rect_t);

/// The runtime's `set_clip_rects`, or none in a libcef built without the patch. The field is past the
/// end of the upstream struct, so it is read only when the struct the runtime made is large enough
/// (as `ui_tasks/drag.rs` does for the window drag).
fn clip_fn(controller: &cef::OverlayController) -> Option<SetClipRects> {
    let raw = ImplOverlayController::get_raw(controller);
    // SAFETY: `raw` is the live controller's own struct; `base.size` is the size libcef allocated.
    let found = unsafe {
        if (*raw).base.size >= std::mem::size_of::<cef::sys::_cef_overlay_controller_t>() {
            (*raw).set_clip_rects
        } else {
            None
        }
    };
    static WARNED: AtomicBool = AtomicBool::new(false);
    if found.is_none() && !WARNED.swap(true, Ordering::Relaxed) {
        tracing::warn!("[pane-airspace] views: this libcef has no overlay clip; a pane under a menu is hidden whole");
    }
    found
}

/// The clip each pane has now, by label. A pane with none has no entry.
static SENT: LazyLock<Mutex<HashMap<String, Vec<Rect>>>> = LazyLock::new(Default::default);

/// Give pane `label` the clip `rects` (none: remove it), unless it has it already. `resend` sends a
/// clip that is unchanged too: CEF does not carry one over a change of the overlay's size or scale.
fn send(label: &str, controller: &cef::OverlayController, set: SetClipRects, rects: &[Rect], resend: bool) {
    let mut sent = SENT.lock();
    let same = sent.get(label).map_or(rects.is_empty(), |last| last.as_slice() == rects);
    if same && !(resend && !rects.is_empty()) {
        return;
    }
    let raw: Vec<cef::sys::cef_rect_t> =
        rects.iter().map(|&(x, y, width, height)| cef::sys::cef_rect_t { x, y, width, height }).collect();
    // SAFETY: `set` was read from this controller's struct (`clip_fn`); the rects outlive the call.
    unsafe { set(ImplOverlayController::get_raw(controller), raw.len(), raw.as_ptr()) };
    if rects.is_empty() {
        sent.remove(label);
    } else {
        sent.insert(label.to_string(), rects.to_vec());
    }
}

/// Pane `label` is gone: forget its clip.
pub(crate) fn forget(label: &str) {
    SENT.lock().remove(label);
}

/// Show of pane `label` what its window's overlays leave of it, and say whether anything is shown.
/// `pane` is the page's last rect for it and `scale` its window's (`creation_views::views_scale`).
/// `resend` after the overlay itself was moved or resized. UI thread.
pub(crate) fn apply(
    state: &Arc<AppState>,
    label: &str,
    window_label: &str,
    controller: &cef::OverlayController,
    pane: Rect,
    scale: f64,
    resend: bool,
) -> bool {
    let overlays = state.pane_overlay_rects.lock().get(window_label).cloned().unwrap_or_default();
    let set = if enabled() { clip_fn(controller) } else { None };
    let cut = match set {
        Some(_) => pane_cut(pane, &overlays, scale),
        None => whole_or_hidden(pane, &overlays),
    };
    let shown = cut != PaneCut::Hidden;
    if let (Some(set), true) = (set, shown) {
        let rects: &[Rect] = if let PaneCut::Rects(rects) = &cut { rects } else { &[] };
        send(label, controller, set, rects, resend);
    }
    // Only on a change: showing a shown pane again is not free.
    if controller.is_visible() != shown as i32 {
        controller.set_visible(shown as i32);
    }
    shown
}

/// `apply` where the caller has neither the pane's rect nor its window: when the window's overlays
/// change (`pane` none: the page's last rect for it, not `bounds()`, which is full-size for an
/// inactive stack tab whose rect is 0×0, see `AppState::browser_pane_physical_rects`), and when a new
/// pane is first shown at `pane`. UI thread.
pub(crate) fn refresh(
    state: &Arc<AppState>,
    label: &str,
    window_label: &str,
    controller: &cef::OverlayController,
    pane: Option<Rect>,
) -> bool {
    let at = pane.or_else(|| state.browser_pane_physical_rects.lock().get(label).copied()).unwrap_or_else(|| {
        let bounds = controller.bounds();
        (bounds.x, bounds.y, bounds.width, bounds.height)
    });
    let window = state.windows.lock().get(window_label).cloned();
    let scale = crate::browser_pane::creation_views::views_scale(window.as_ref());
    apply(state, label, window_label, controller, at, scale, pane.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(rects: &[Rect]) -> i64 {
        rects.iter().map(|r| r.2 as i64 * r.3 as i64).sum()
    }

    fn overlap(a: &Rect, b: &Rect) -> bool {
        a.0 < b.0 + b.2 && b.0 < a.0 + a.2 && a.1 < b.1 + b.3 && b.1 < a.1 + a.3
    }

    #[test]
    fn a_pane_with_nothing_over_it_is_whole() {
        assert_eq!(pane_cut((100, 50, 400, 300), &[], 1.0), PaneCut::Whole);
        // Beside it, and touching its edge.
        assert_eq!(pane_cut((100, 50, 400, 300), &[(0, 0, 100, 600), (500, 0, 50, 50)], 1.0), PaneCut::Whole);
        // An overlay with no area.
        assert_eq!(pane_cut((100, 50, 400, 300), &[(200, 100, 0, 40)], 1.0), PaneCut::Whole);
    }

    #[test]
    fn a_pane_with_no_area_or_fully_covered_is_hidden() {
        assert_eq!(pane_cut((100, 50, 0, 0), &[], 1.0), PaneCut::Hidden);
        assert_eq!(pane_cut((100, 50, 400, 300), &[(0, 0, 2000, 1000)], 1.0), PaneCut::Hidden);
        // Two overlays that cover it between them.
        assert_eq!(pane_cut((100, 50, 400, 300), &[(100, 50, 250, 400), (300, 0, 400, 400)], 1.0), PaneCut::Hidden);
    }

    #[test]
    fn a_hole_in_the_middle_leaves_four_rects_in_the_panes_own_coordinates() {
        let cut = pane_cut((100, 50, 400, 300), &[(200, 100, 100, 80)], 1.0);
        assert_eq!(
            cut,
            PaneCut::Rects(vec![(0, 0, 401, 50), (0, 50, 100, 80), (200, 50, 201, 80), (0, 130, 401, 171)])
        );
    }

    #[test]
    fn a_menu_hanging_into_the_pane_from_its_corner_is_cut_to_the_pane() {
        // From above and left of the pane into it: the hole is the part inside.
        let cut = pane_cut((100, 50, 400, 300), &[(40, 10, 160, 90)], 1.0);
        assert_eq!(cut, PaneCut::Rects(vec![(100, 0, 301, 50), (0, 50, 401, 251)]));
    }

    #[test]
    fn rects_never_overlap_and_cover_exactly_what_the_holes_leave() {
        let pane = (0, 0, 300, 200);
        let overlays = [(20, 20, 100, 60), (80, 50, 120, 90), (250, 0, 100, 30), (0, 190, 300, 40)];
        let PaneCut::Rects(rects) = pane_cut(pane, &overlays, 1.0) else { panic!("expected rects") };
        for (i, a) in rects.iter().enumerate() {
            assert!(a.2 > 0 && a.3 > 0 && a.0 >= 0 && a.1 >= 0 && a.0 + a.2 <= 301 && a.1 + a.3 <= 201, "{a:?}");
            assert!(overlays.iter().all(|o| !overlap(a, o)), "{a:?} is under an overlay");
            assert!(rects[i + 1..].iter().all(|b| !overlap(a, b)), "{a:?} overlaps another rect");
        }
        let covered = (0..201).flat_map(|y| (0..301).map(move |x| (x, y, 1, 1)));
        let free = covered.filter(|p| overlays.iter().all(|o| !overlap(p, o))).count() as i64;
        assert_eq!(area(&rects), free);
    }

    #[test]
    fn bands_that_repeat_are_one_rect() {
        // Two holes with the same top and bottom: three bands, and the middle one has three rects.
        let cut = pane_cut((0, 0, 300, 200), &[(50, 40, 50, 60), (200, 40, 50, 60)], 1.0);
        assert_eq!(
            cut,
            PaneCut::Rects(vec![
                (0, 0, 301, 40),
                (0, 40, 50, 60),
                (100, 40, 100, 60),
                (250, 40, 51, 60),
                (0, 100, 301, 101)
            ])
        );
        // A hole split in two stacked overlays leaves the same rects as one.
        let one = pane_cut((0, 0, 300, 200), &[(50, 40, 50, 60)], 1.0);
        assert_eq!(pane_cut((0, 0, 300, 200), &[(50, 40, 50, 30), (50, 70, 50, 30)], 1.0), one);
    }

    #[test]
    fn at_a_fractional_scale_the_hole_grows_to_whole_dips() {
        // 125%: the pane at 125,63 500x375 px is 100,50 400x300 DIP. An overlay at 251..377 px across is
        // 200.8..301.6 DIP: the hole is 200..302, so 100..202 in the pane.
        let cut = pane_cut((125, 63, 500, 375), &[(251, 0, 126, 2000)], 1.25);
        assert_eq!(cut, PaneCut::Rects(vec![(0, 0, 100, 301), (202, 0, 199, 301)]));
    }

    #[test]
    fn the_cut_reaches_a_dip_past_the_right_and_bottom_edges() {
        // A tooltip that hangs out of the pane's bottom: what is left reaches a DIP past the right
        // edge beside it, and nothing is left under it.
        let cut = pane_cut((0, 0, 400, 300), &[(100, 270, 200, 60)], 1.0);
        assert_eq!(cut, PaneCut::Rects(vec![(0, 0, 401, 270), (0, 270, 100, 31), (300, 270, 101, 31)]));
        // One that ends at the pane's own edge leaves the DIP past it.
        let cut = pane_cut((0, 0, 400, 300), &[(0, 270, 400, 30)], 1.0);
        assert_eq!(cut, PaneCut::Rects(vec![(0, 0, 401, 270), (400, 270, 1, 30), (0, 300, 401, 1)]));
    }

    #[test]
    fn without_the_clip_a_pane_under_anything_is_hidden_whole() {
        assert_eq!(whole_or_hidden((100, 50, 400, 300), &[]), PaneCut::Whole);
        assert_eq!(whole_or_hidden((100, 50, 400, 300), &[(0, 0, 100, 600)]), PaneCut::Whole);
        assert_eq!(whole_or_hidden((100, 50, 400, 300), &[(200, 100, 10, 10)]), PaneCut::Hidden);
        assert_eq!(whole_or_hidden((100, 50, 0, 0), &[]), PaneCut::Hidden);
    }
}
