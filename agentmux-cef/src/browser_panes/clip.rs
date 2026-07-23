// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Focus and overlay-clip ("pane airspace") operations for
//! `BrowserPaneManager`: `focus`, `defocus_all`, the Windows `SetWindowRgn`
//! clip workaround and its Linux/macOS Views-based equivalent
//! (`set_pane_overlay_clip`), plus their shared helpers (`clip_sig_whole`,
//! `clip_sig_clipped`, `rects_intersect`, `compute_pane_visible`). Split out
//! of `browser_panes.rs` — see that module's doc comment.

use std::sync::Arc;

use cef::*;

use crate::state::AppState;

use super::BrowserPaneManager;

impl BrowserPaneManager {
    /// Tell every live pane browser it has lost focus, at the Chromium level.
    /// Panes in `Closing` are skipped — their HWND may be mid-destruction and
    /// `set_focus(0)` against it can hit an invalid render widget.
    pub fn defocus_all(&self, state: &Arc<AppState>) {
        // Phase H.1.b + H.2.b — read live labels via reducer-aware helper,
        // then look up each browser via reducer-aware helper. Both with
        // fallback + drift logging.
        let labels = state.live_browser_pane_labels();
        for label in &labels {
            if let Some(browser) = state.get_browser(label) {
                if let Some(host) = browser.host() {
                    host.set_focus(0);
                }
            }
        }
    }

    /// Apply a clip region to every live pane HWND that subtracts the given
    /// overlay rects (in main-window client coordinates). The pane renders
    /// normally outside the overlay region; inside it, the HWND is
    /// transparent so the DOM overlay painted at the same screen position
    /// shows through.
    ///
    /// This is the Win32 "airspace" workaround — native HWNDs always paint
    /// above DOM regardless of CSS z-index, and `SetWindowRgn` is the one
    /// mechanism that lets DOM bleed through a specific region of a child
    /// HWND. Empty `overlay_rects` restores full pane visibility (same as
    /// calling `clear_pane_overlay_clip`).
    ///
    /// No-op on non-Windows: other platforms don't use native child HWNDs
    /// for panes, so there's no airspace to work around.
    ///
    /// `window_label` scopes the clip to panes whose top-level ancestor
    /// matches the requesting window. Without it, a modal opened in
    /// window B would clip panes in window A (see Codex P1 on PR #544).
    /// Empty string matches today's legacy callers that don't know their
    /// window label — falls through to the no-filter behaviour for
    /// back-compat until every caller is updated.
    #[cfg(target_os = "windows")]
    pub fn set_pane_overlay_clip(
        &self,
        state: &Arc<AppState>,
        window_label: &str,
        overlay_rects: &[(i32, i32, i32, i32)],
    ) {
        use windows_sys::Win32::Foundation::{POINT, RECT};
        use windows_sys::Win32::Graphics::Gdi::{
            CombineRgn, CreateRectRgn, DeleteObject, InvalidateRect, MapWindowPoints, SetWindowRgn,
            RGN_DIFF,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetAncestor, GetWindowRect, GA_ROOT,
        };

        // Resolve the requesting window's top-level HWND so we can filter
        // panes by ownership. If the label is unknown we fall through with
        // no filter — matches pre-scoping behaviour rather than silently
        // doing nothing.
        let requesting_top_level: *mut std::ffi::c_void = if window_label.is_empty() {
            std::ptr::null_mut()
        } else {
            // Phase H.2.b — reducer-aware lookup with fallback.
            match state.get_browser(window_label).and_then(|b| b.host()) {
                Some(host) => {
                    let h = host.window_handle();
                    if h.0.is_null() {
                        std::ptr::null_mut()
                    } else {
                        unsafe { GetAncestor(h.0 as _, GA_ROOT) as *mut std::ffi::c_void }
                    }
                }
                None => std::ptr::null_mut(),
            }
        };

        // Phase H.1.b + H.2.b — labels via reducer-aware helper; per-label
        // browser lookup via reducer-aware helper. Drops the held-across-loop
        // legacy lock; each iteration now snapshots independently.
        let labels = state.live_browser_pane_labels();
        for label in &labels {
            let browser = match state.get_browser(label) {
                Some(b) => b,
                None => continue,
            };
            let host = match browser.host() {
                Some(h) => h,
                None => continue,
            };
            let hwnd_raw = host.window_handle();
            if hwnd_raw.0.is_null() {
                continue;
            }
            let pane_hwnd = hwnd_raw.0 as *mut std::ffi::c_void;

            // Window-scope filter. Skip panes whose top-level HWND differs
            // from the requesting window's. `null` requesting = legacy
            // caller / no-op filter (applies to all panes).
            if !requesting_top_level.is_null() {
                let pane_top = unsafe { GetAncestor(pane_hwnd as _, GA_ROOT) as *mut std::ffi::c_void };
                if pane_top != requesting_top_level {
                    continue;
                }
            }

            // App-owned wrapper HWND (browser_pane::wrapper — see its module
            // doc and SPEC_BROWSER_PANE_WINDOWS_TEARDOWN_SPIKE_2026_07_03.md)
            // sits directly between `pane_hwnd` and its host window and is
            // kept congruent with it (same origin, same size — its own
            // `WM_SIZE` handler resizes CEF's child to exactly fill it, see
            // `wrapper.rs`). `SetWindowRgn` only changes hit-testing for the
            // HWND it's called on: clipping `pane_hwnd` alone stops IT from
            // painting/claiming the hole, but Win32 hit-testing for a point
            // inside that hole then resolves to `pane_hwnd`'s own parent —
            // the wrapper — which still claims its full, unclipped
            // rectangle there. The wrapper's window class paints nothing of
            // its own (`hbrBackground: null`) so the DOM underneath still
            // shows through visually (matches "renders fine"), but its
            // `WM_LBUTTONDOWN`/`WM_MOUSEMOVE` fall through to
            // `DefWindowProcW` unhandled — the input is silently absorbed
            // there instead of reaching the DOM (matches "hover/click
            // no-op"). See docs/analysis/ANALYSIS_WINDOWS_PANE_OVERLAY_WRAPPER_HITTEST_GAP_2026_07_13.md.
            // Every region applied to `pane_hwnd` below must therefore also
            // be applied to the wrapper, in the same wrapper-local
            // coordinates (congruent, so no separate coordinate math is
            // needed). `None` only if the wrapper hasn't been created yet /
            // already torn down (a real race, not the common case — every
            // embedded pane creation on Windows creates one unconditionally,
            // `browser_pane/creation.rs`); degrade gracefully by clipping
            // just the pane in that case rather than failing the whole call.
            let wrapper_hwnd = crate::browser_pane::wrapper::peek_wrapper_hwnd(label)
                .map(|h| h as *mut std::ffi::c_void);

            unsafe {
                // Empty overlay list = restore full visibility (region=NULL).
                // bRedraw=FALSE (0) — see #1097 fix #5. With bRedraw=TRUE
                // SetWindowRgn does a SYNCHRONOUS redraw inside the call
                // (it sends WM_WINDOWPOSCHANGING/CHANGED + a paint pass),
                // which serializes against the IPC handler. Passing FALSE
                // updates the region without painting; an explicit
                // InvalidateRect schedules WM_PAINT for the NEXT message
                // pump tick. For a menu hover that re-fires this code path
                // several times per gesture, that's a material win.
                if overlay_rects.is_empty() {
                    // Skip if this pane is already at full visibility (same HWND,
                    // already restored) — avoids a redundant SetWindowRgn +
                    // repaint every call while any overlay is open elsewhere.
                    //
                    // The cache guard is held across check → apply → record so
                    // the sequence is atomic. This handler runs INLINE on a
                    // multi-threaded tokio worker (ipc.rs — no UI-thread marshal,
                    // unlike the non-Windows path), so two concurrent clips for
                    // the same pane could otherwise interleave their SetWindowRgn
                    // vs cache.insert and leave the recorded sig disagreeing with
                    // the live region — a later wrong-skip.
                    let sig = clip_sig_whole(pane_hwnd as isize);
                    let mut cache = state.pane_clip_cache.lock();
                    if cache.get(label).copied() == Some(sig) {
                        continue;
                    }
                    let applied = SetWindowRgn(pane_hwnd as _, std::ptr::null_mut(), 0) != 0;
                    // The pane is being made WHOLE again — the previously
                    // hidden sub-area needs to repaint its content. We
                    // don't track the diff yet (Phase 2 follow-up), so
                    // invalidate the entire pane; CEF will paint only
                    // dirty regions on the next pump.
                    InvalidateRect(pane_hwnd as _, std::ptr::null(), 0);
                    // Restore the wrapper's region too — see the wrapper_hwnd
                    // comment above. Not gated on `applied`/cached separately:
                    // the wrapper and pane_hwnd always change in lockstep, so
                    // the pane's own sig is the correct gate for both.
                    if let Some(wrapper_hwnd) = wrapper_hwnd {
                        SetWindowRgn(wrapper_hwnd as _, std::ptr::null_mut(), 0);
                        InvalidateRect(wrapper_hwnd as _, std::ptr::null(), 0);
                    }
                    // Only record the sig if the region actually applied; on a
                    // (rare) SetWindowRgn failure leave the entry unset so the
                    // next call retries instead of skipping a clip that never
                    // landed.
                    if applied {
                        cache.insert(label.clone(), sig);
                    }
                    continue;
                }

                // Resolve the pane's position in its parent (main window)
                // client coords so we can translate overlay rects (which
                // arrive in main-window client coords from the frontend)
                // into pane-local coords for the region API.
                //
                // GA_ROOT, not single-hop GetParent: since
                // SPEC_BROWSER_PANE_WINDOWS_TEARDOWN_SPIKE_2026_07_03.md,
                // `pane_hwnd` (CEF's own HWND) is a WS_CHILD of our own
                // wrapper HWND, not directly of main — GetParent would
                // resolve to the wrapper (whose client coords are always
                // (0,0)-origin at the pane's own size), not main, silently
                // breaking this translation. GA_ROOT walks past any number
                // of intermediate layers to the actual top-level window,
                // which is what "main window client coords" means here —
                // same pattern already used above for requesting_top_level.
                let parent = GetAncestor(pane_hwnd as _, GA_ROOT);
                if parent.is_null() {
                    continue;
                }
                let mut pane_rect: RECT = std::mem::zeroed();
                if GetWindowRect(pane_hwnd as _, &mut pane_rect) == 0 {
                    continue;
                }
                // Convert pane_rect from screen coords to parent client
                // coords by mapping its two corner points.
                let pts_ptr = &mut pane_rect as *mut RECT as *mut POINT;
                MapWindowPoints(std::ptr::null_mut(), parent, pts_ptr, 2);

                let pane_w = pane_rect.right - pane_rect.left;
                let pane_h = pane_rect.bottom - pane_rect.top;
                if pane_w <= 0 || pane_h <= 0 {
                    continue;
                }

                // Skip if the region we'd compute is identical to the one
                // already applied to this HWND (same geometry + same overlays).
                //
                // As in the whole-visibility branch above, the cache guard is
                // held across check → build → apply → record so a concurrent
                // clip for the same pane (this handler runs inline on a
                // multi-threaded tokio worker) can't interleave SetWindowRgn and
                // cache.insert and desync the recorded sig from the live region.
                let sig = clip_sig_clipped(
                    pane_hwnd as isize,
                    pane_rect.left,
                    pane_rect.top,
                    pane_w,
                    pane_h,
                    overlay_rects,
                );
                let mut cache = state.pane_clip_cache.lock();
                if cache.get(label).copied() == Some(sig) {
                    continue;
                }

                // Build a region in pane-local coords: start with full pane,
                // subtract every overlay rect that intersects it. Factored
                // into a closure because it must run TWICE — once per
                // target HWND — since `SetWindowRgn` takes ownership of the
                // region handle it's given on success, so the same GDI
                // region object can't be handed to two different windows.
                // The wrapper is congruent with `pane_hwnd` (same origin,
                // same size — see the wrapper_hwnd comment above), so the
                // exact same `pane_w`/`pane_h`/`pane_rect` inputs apply to
                // both without any extra coordinate math.
                let build_region = || -> *mut std::ffi::c_void {
                    let region = CreateRectRgn(0, 0, pane_w, pane_h);
                    if region.is_null() {
                        return region;
                    }
                    for (ox, oy, ow, oh) in overlay_rects {
                        // Translate overlay rect (window client coords) →
                        // pane-local coords by subtracting pane's window pos.
                        let left = ox - pane_rect.left;
                        let top = oy - pane_rect.top;
                        let right = left + ow;
                        let bottom = top + oh;
                        // Skip if no intersection with the pane's local bounds.
                        if right <= 0 || bottom <= 0 || left >= pane_w || top >= pane_h {
                            continue;
                        }
                        let overlay_rgn = CreateRectRgn(left, top, right, bottom);
                        if !overlay_rgn.is_null() {
                            CombineRgn(region, region, overlay_rgn, RGN_DIFF);
                            DeleteObject(overlay_rgn as _);
                        }
                    }
                    region
                };

                let region = build_region();
                if region.is_null() {
                    continue;
                }
                // SetWindowRgn takes ownership of the region handle on
                // success; the system frees it when the window is destroyed
                // or a new region is set. On FAILURE ownership stays with us,
                // so we DeleteObject the region below to avoid a GDI leak.
                // bRedraw=FALSE — async paint via InvalidateRect below.
                // See the empty-overlay branch above for the rationale.
                let applied = SetWindowRgn(pane_hwnd as _, region as _, 0) != 0;
                // The clip may have GROWN (more area hidden) or SHRUNK
                // (less area hidden); we don't track the diff yet, so
                // invalidate the whole pane. CEF dirty-region painting
                // keeps the actual GPU work proportional to the change.
                InvalidateRect(pane_hwnd as _, std::ptr::null(), 0);
                // Same clip on the wrapper — see the wrapper_hwnd comment
                // above for why this is required, not optional, for
                // hover/click to reach the DOM inside the punched hole. A
                // second, independently-created region handle (never the
                // same handle passed to pane_hwnd above — see the ownership
                // note). Best-effort: a wrapper-side failure here doesn't
                // block the pane's own clip from being recorded, matching
                // the "degrade gracefully" stance for a missing wrapper_hwnd
                // above.
                if let Some(wrapper_hwnd) = wrapper_hwnd {
                    let wrapper_region = build_region();
                    if !wrapper_region.is_null() {
                        if SetWindowRgn(wrapper_hwnd as _, wrapper_region as _, 0) == 0 {
                            DeleteObject(wrapper_region as _);
                        }
                        InvalidateRect(wrapper_hwnd as _, std::ptr::null(), 0);
                    }
                }
                // Only record the sig when the region actually applied (matches
                // the whole-visibility branch); on failure free the orphaned
                // region and leave the entry unset so the next call retries.
                if applied {
                    cache.insert(label.clone(), sig);
                } else {
                    DeleteObject(region as _);
                }
            }
        }
        // Prune cache entries for panes no longer live (closed/destroyed),
        // bounding the map. Labels are never reused for a different pane, so a
        // dropped entry only means the next clip for a (re)created pane applies
        // fresh — which is correct.
        {
            let mut cache = state.pane_clip_cache.lock();
            cache.retain(|k, _| labels.iter().any(|l| l == k));
        }
        tracing::info!(
            pane_count = labels.len(),
            overlay_count = overlay_rects.len(),
            "[pane-airspace] applied overlay clip to pane HWNDs",
        );
    }
    /// Linux/macOS — equivalent of the Windows SetWindowRgn airspace
    /// workaround, but built on Views instead of HWND clip regions.
    ///
    /// `add_overlay_view` puts the pane on a higher z-layer than the host UI
    /// BrowserView, so any host-side modal/dropdown/contextmenu that overlaps
    /// a pane rect renders UNDERNEATH the pane and becomes unclickable. We
    /// can't punch a clip hole through an Aura View the way Win32 SetWindowRgn
    /// does on an HWND. The pragmatic workaround: when ANY overlay rect
    /// overlaps a pane's bounds, hide that pane (`set_visible(false)`); when
    /// no overlay rect intersects, show it again. The DOM modal renders in
    /// the host UI BrowserView underneath, becomes the topmost paint at that
    /// rect, and the pane's content briefly disappears — same UX trade-off
    /// the Windows path makes (Win32 punches a hole; we hide the whole pane).
    /// (Codex P1 on PR #682.)
    ///
    /// Future improvement: only hide the overlapping fraction of the pane
    /// (would need a per-overlay set_size + position trick or a custom Layout).
    /// Hiding the whole pane is acceptable for now — the airspace problem only
    /// arises when modals open over panes, which is a transient case.
    ///
    /// `_window_label` is currently ignored on this path because we only
    /// support a single primary window for panes (sub-window panes are a
    /// follow-up — see PR #682's "Risks / follow-ups" section).
    /// (See doc comment for the cfg(target_os = "windows") variant.)
    /// Linux/macOS body — marshalled to the CEF UI thread because
    /// `OverlayController::set_visible` and `bounds()` are UI-thread-only.
    /// IPC handler runs on tokio so we post a task and return immediately.
    /// `window_label` filters which panes get visibility-managed: only those
    /// attached to the requesting window are affected.
    #[cfg(not(target_os = "windows"))]
    pub fn set_pane_overlay_clip(
        &self,
        state: &Arc<AppState>,
        window_label: &str,
        overlay_rects: &[(i32, i32, i32, i32)],
    ) {
        // Publish to AppState so resize_browser_pane_view can consult the
        // same authoritative rect list when computing pane visibility on
        // its own code path. Without this, a positive-dimension resize
        // (e.g. user drags a splitter while a DOM modal is open) would
        // call set_visible(1) and re-expose the pane on top of the modal.
        // See state::pane_overlay_rects doc comment.
        state
            .pane_overlay_rects
            .lock()
            .insert(window_label.to_string(), overlay_rects.to_vec());

        let mut task = SetPaneOverlayClipViewsTask::new(
            state.clone(),
            window_label.to_string(),
            overlay_rects.to_vec(),
        );
        cef::post_task(cef::ThreadId::UI, Some(&mut task));
    }

    /// Give keyboard focus to the pane's child HWND so keystrokes reach the
    /// embedded page. Called by the frontend's ViewModel.giveFocus() when the
    /// pane becomes the active layout node — without this, focus falls back to
    /// the main window's invisible "dummy-focus" input and keystrokes vanish.
    ///
    /// No-ops if the pane is `Closing`: a SetFocus against a HWND that CEF is
    /// concurrently tearing down is the exact race documented in
    /// `SPEC_BROWSER_PANE_LIFECYCLE.md` §5 race #2.
    pub fn focus(&self, block_id: &str, state: &Arc<AppState>) {
        if let Some(browser) = self.live_browser(state, block_id) {
            if let Some(host) = browser.host() {
                host.set_focus(1);
                #[cfg(target_os = "windows")]
                {
                    let hwnd = host.window_handle();
                    if !hwnd.0.is_null() {
                        // Tell the subclass this focus request is intentional
                        // (not Chromium's on-load focus steal) so it won't be
                        // redirected back to the parent.
                        crate::browser_pane::ALLOW_BROWSER_PANE_FOCUS_ONCE.store(
                            true,
                            std::sync::atomic::Ordering::Relaxed,
                        );
                        unsafe {
                            windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus(hwnd.0 as _);
                        }
                    }
                }
            }
        }
    }
}

/// Signature of the clip a pane should have, used by `set_pane_overlay_clip`
/// to skip a redundant `SetWindowRgn` when nothing changed. The HWND value is
/// folded in so a recreated pane (new HWND under a reused label) never matches a
/// stale entry — see `AppState::pane_clip_cache`. Within-process only (the hash
/// need not be stable across runs).
#[cfg(target_os = "windows")]
fn clip_sig_whole(hwnd: isize) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    0u8.hash(&mut h); // tag: full visibility (region = NULL)
    hwnd.hash(&mut h);
    h.finish()
}

#[cfg(target_os = "windows")]
fn clip_sig_clipped(
    hwnd: isize,
    pane_left: i32,
    pane_top: i32,
    pane_w: i32,
    pane_h: i32,
    overlay_rects: &[(i32, i32, i32, i32)],
) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    1u8.hash(&mut h); // tag: clipped
    hwnd.hash(&mut h);
    // The applied region is a deterministic function of the pane's screen
    // position/size and the overlay rects, so hashing those inputs is a faithful
    // signature: identical inputs ⇒ identical region (no false cache hit). A
    // non-intersecting rect that changes only yields a false MISS (a harmless
    // redundant re-apply), never a false hit.
    pane_left.hash(&mut h);
    pane_top.hash(&mut h);
    pane_w.hash(&mut h);
    pane_h.hash(&mut h);
    overlay_rects.hash(&mut h);
    h.finish()
}

/// Two axis-aligned rects intersect iff neither is fully to one side of the
/// other. Coordinates: (x, y, width, height). Used by the Linux/macOS
/// pane-airspace logic to decide whether an overlay rect from the frontend
/// covers any part of a pane's bounds.
#[cfg(not(target_os = "windows"))]
fn rects_intersect(a: (i32, i32, i32, i32), b: (i32, i32, i32, i32)) -> bool {
    let (ax, ay, aw, ah) = a;
    let (bx, by, bw, bh) = b;
    let a_right = ax + aw;
    let a_bottom = ay + ah;
    let b_right = bx + bw;
    let b_bottom = by + bh;
    !(a_right <= bx || b_right <= ax || a_bottom <= by || b_bottom <= ay)
}

/// Compute whether a pane with the given bounds should be visible, given
/// the pane's parent window. Both pane-airspace (`SetPaneOverlayClipViewsTask`)
/// and per-pane resize (`resize_browser_pane_view`) call this to converge on
/// the same answer — without it, the two paths fight each other (Codex
/// review on PR #881 caught the dragging-splitter-while-modal-open case
/// where a positive resize re-exposed a pane that airspace had hidden).
///
/// A pane is visible iff BOTH conditions hold:
/// - Its rect has non-zero width and height (frontend places it in a
///   `display:none` placeholder when the tab is inactive → reports 0×0).
/// - It does not intersect any registered overlay-clip rect for its window
///   (e.g. a hamburger menu, tooltip, modal popover).
// Linux-only since the macOS hole-punch mask (ui_tasks/pane_hole_mask.rs):
// macOS paths use rect-only visibility and mask overlays instead of hiding.
//
// `pub(crate)`, not private: called from `browser_pane::creation_views`
// (`resize_browser_pane_view`) via `crate::browser_panes::compute_pane_visible`
// — re-exported from `browser_panes::mod` so that external path still resolves.
#[cfg(all(not(target_os = "windows"), not(target_os = "macos")))]
pub(crate) fn compute_pane_visible(
    state: &Arc<AppState>,
    window_label: &str,
    pane_rect: (i32, i32, i32, i32),
) -> bool {
    let (_, _, w, h) = pane_rect;
    if w <= 0 || h <= 0 {
        return false;
    }
    let rects = state.pane_overlay_rects.lock();
    let overlays = match rects.get(window_label) {
        Some(v) => v.clone(),
        None => return true,
    };
    drop(rects);
    !overlays.iter().any(|or| rects_intersect(*or, pane_rect))
}

/// Linux/macOS pane-airspace task — fired by `set_pane_overlay_clip` for the
/// non-Windows code path. For each live OverlayController, hide it when any
/// overlay rect intersects its current bounds; show it otherwise. See the
/// doc comment on `set_pane_overlay_clip` (non-Windows variant) for why this
/// is the equivalent of the Windows SetWindowRgn airspace dance.
#[cfg(not(target_os = "windows"))]
wrap_task! {
    pub struct SetPaneOverlayClipViewsTask {
        state: Arc<AppState>,
        // Only panes attached to this window get visibility-managed; panes
        // in other windows are unaffected by overlay rects from this window.
        // Mirrors the window_label filtering in the Windows path.
        window_label: String,
        overlay_rects: Vec<(i32, i32, i32, i32)>,
    }

    impl Task {
        fn execute(&self) {
            // Snapshot the (pane label, parent window label, controller) tuples,
            // filter by parent-window-label matching the requesting window,
            // drop the mutex before any FFI call (snapshot-and-drop discipline
            // per docs/specs/SPEC_PHASE_F_HOST_REDUCER §6).
            let live: Vec<(String, cef::OverlayController)> = self
                .state
                .browser_pane_overlays
                .lock()
                .iter()
                .filter(|(_, (win_label, _))| win_label == &self.window_label)
                .map(|(k, (_, c))| (k.clone(), c.clone()))
                .collect();
            if live.is_empty() {
                return;
            }
            for (label, controller) in live {
                use cef::ImplOverlayController;
                // On macOS, `controller.bounds()` cannot be trusted here — CEF
                // Views' own set_size/set_position are permanent no-ops on
                // NativeWidgetMacNSWindow, so bounds() reflects a stale,
                // wrong-scale (DIP, not physical-px) rect rather than the real
                // on-screen frame we committed via raw ObjC setFrame:. Using it
                // made the intersection test silently miss real overlaps —
                // visible stayed `true` while a DOM menu was drawn on top, so
                // the pane kept intercepting clicks meant for the DOM even
                // though it painted correctly underneath. Use the maintained
                // physical-rect cache instead; see
                // `AppState::browser_pane_physical_rects`. Fall back to
                // bounds() if the cache somehow has no entry yet (shouldn't
                // happen — creation seeds it before any overlay-clip can run).
                #[cfg(target_os = "macos")]
                {
                    let pane_rect = self
                        .state
                        .browser_pane_physical_rects
                        .lock()
                        .get(&label)
                        .copied()
                        .unwrap_or_else(|| {
                            let pb = controller.bounds();
                            (pb.x, pb.y, pb.width, pb.height)
                        });
                    let (px, py, pw, ph) = pane_rect;
                    if pw <= 0 || ph <= 0 {
                        // Tab inactive (placeholder collapsed) — keep hidden.
                        controller.set_visible(0);
                        continue;
                    }
                    // Hole punch (Windows SetWindowRgn parity): mask out the
                    // overlay∩pane rects instead of hiding the whole pane. The
                    // pane stays LIVE and visible around the holes; the DOM
                    // overlay shows through them, and transparent pixels
                    // click-through to the main window. Whole-pane hide (and
                    // the frontend freeze-frame that compensated for it) only
                    // remains as the fallback when the overlay NSWindow can't
                    // be found. See ui_tasks/pane_hole_mask.rs.
                    let holes: Vec<(i32, i32, i32, i32)> = self
                        .overlay_rects
                        .iter()
                        .filter(|or| rects_intersect(**or, pane_rect))
                        .map(|&(ox, oy, ow, oh)| {
                            let hx = ox.max(px);
                            let hy = oy.max(py);
                            let hr = (ox + ow).min(px + pw);
                            let hb = (oy + oh).min(py + ph);
                            (hx, hy, hr - hx, hb - hy)
                        })
                        .collect();
                    let wnum = self
                        .state
                        .browser_pane_overlay_wnums
                        .lock()
                        .get(&label)
                        .copied()
                        .unwrap_or(0);
                    let masked = crate::ui_tasks::pane_hole_mask::apply_pane_overlay_hole_mask(
                        wnum, pane_rect, &holes,
                    );
                    if masked {
                        controller.set_visible(1);
                    } else {
                        // Fallback: window lookup failed — old whole-pane hide.
                        let visible = holes.is_empty();
                        controller.set_visible(if visible { 1 } else { 0 });
                    }
                    tracing::debug!(
                        label = %label,
                        window_label = %self.window_label,
                        masked, hole_count = holes.len(),
                        pane_x = px, pane_y = py, pane_w = pw, pane_h = ph,
                        overlay_count = self.overlay_rects.len(),
                        "[pane-airspace] views: applied hole mask"
                    );
                }
                #[cfg(not(target_os = "macos"))]
                {
                    let pane_rect = {
                        let pb = controller.bounds();
                        (pb.x, pb.y, pb.width, pb.height)
                    };
                    // Shared visibility helper consults BOTH the pane's own rect
                    // (zero → hidden because tab inactive) and the latest
                    // overlay-clip rects published in AppState. Resize path uses
                    // the same helper so both decisions converge.
                    let visible = compute_pane_visible(&self.state, &self.window_label, pane_rect);
                    controller.set_visible(if visible { 1 } else { 0 });
                    tracing::debug!(
                        label = %label,
                        window_label = %self.window_label,
                        visible,
                        pane_x = pane_rect.0, pane_y = pane_rect.1, pane_w = pane_rect.2, pane_h = pane_rect.3,
                        overlay_count = self.overlay_rects.len(),
                        "[pane-airspace] views: applied visibility"
                    );
                }
            }
        }
    }
}
