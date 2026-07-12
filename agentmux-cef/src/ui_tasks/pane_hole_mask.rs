// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! macOS pane-airspace hole punch (SPIKE — Windows `SetWindowRgn` parity).
//!
//! The pane is a separate `NativeWidgetMacNSWindow` floating above the main
//! window, so DOM overlays (menus, modals) paint UNDER it. The previous
//! strategy hid the ENTIRE pane while any overlay intersected it
//! (`set_visible(0)`), which required a whole compensation stack on the
//! frontend (freeze-frame capture, prewarm, subpixel alignment) and still
//! produced visible seams at the live→snapshot swap.
//!
//! This module instead punches a REAL hole: a `CAShapeLayer` with an
//! even-odd path (full content bounds + one rect per overlay intersection)
//! is set as the pane contentView layer's mask. Pixels inside the holes
//! composite as fully transparent, so:
//!   * the DOM overlay painted at the same screen position shows through
//!     (the pane around it stays LIVE — no freeze, no flash, no jerk), and
//!   * the window server's per-pixel hit testing routes clicks through the
//!     transparent region to the main window below — this requires the
//!     window to be non-opaque with a clear background, which
//!     `prepare_window_for_click_through` sets (idempotent).
//!
//! Empirical status: rendering hole is CALayer-guaranteed; click-through
//! relies on the window server sampling the composited alpha (the standard
//! shaped-window behavior). `invalidateShadow` after every mask change
//! nudges the server to recompute the window shape.
//!
//! Must run on the CEF UI thread (= AppKit main thread).

#![cfg(target_os = "macos")]

use std::ffi::c_char;

type Id = *mut std::ffi::c_void;
type Sel = *const std::ffi::c_void;

extern "C" {
    fn sel_registerName(name: *const c_char) -> Sel;
    fn objc_msgSend();
    fn objc_getClass(name: *const c_char) -> Id;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGPathCreateMutable() -> *mut std::ffi::c_void;
    fn CGPathAddRect(
        path: *mut std::ffi::c_void,
        transform: *const std::ffi::c_void,
        rect: CGRect,
    );
    fn CGPathRelease(path: *mut std::ffi::c_void);
}

#[repr(C)]
#[derive(Copy, Clone)]
struct CGPoint {
    x: f64,
    y: f64,
}
#[repr(C)]
#[derive(Copy, Clone)]
struct CGSize {
    w: f64,
    h: f64,
}
#[repr(C)]
#[derive(Copy, Clone)]
struct CGRect {
    origin: CGPoint,
    size: CGSize,
}

/// Apply (or clear) the hole mask on the pane overlay NSWindow identified
/// by `overlay_wnum`.
///
/// * `pane_rect` — the pane's on-screen rect in PHYSICAL px (same space as
///   `AppState::browser_pane_physical_rects`).
/// * `holes` — overlay rects ALREADY intersected/clipped against
///   `pane_rect`, physical px, absolute (same origin as `pane_rect`).
///
/// Empty `holes` clears the mask (full pane visible). Returns false when
/// the window could not be found (caller may fall back to whole-pane hide).
pub fn apply_pane_overlay_hole_mask(
    overlay_wnum: isize,
    pane_rect: (i32, i32, i32, i32),
    holes: &[(i32, i32, i32, i32)],
) -> bool {
    if overlay_wnum <= 0 {
        return false;
    }
    unsafe {
        let sel_shared_app = sel_registerName(b"sharedApplication\0".as_ptr() as _);
        let sel_windows = sel_registerName(b"windows\0".as_ptr() as _);
        let sel_count = sel_registerName(b"count\0".as_ptr() as _);
        let sel_obj_at = sel_registerName(b"objectAtIndex:\0".as_ptr() as _);
        let sel_win_number = sel_registerName(b"windowNumber\0".as_ptr() as _);
        let sel_content_view = sel_registerName(b"contentView\0".as_ptr() as _);
        let sel_layer = sel_registerName(b"layer\0".as_ptr() as _);
        let sel_wants_layer = sel_registerName(b"setWantsLayer:\0".as_ptr() as _);
        let sel_bounds = sel_registerName(b"bounds\0".as_ptr() as _);
        let sel_backing_scale = sel_registerName(b"backingScaleFactor\0".as_ptr() as _);
        let sel_set_mask = sel_registerName(b"setMask:\0".as_ptr() as _);
        let sel_layer_cls_new = sel_registerName(b"layer\0".as_ptr() as _);
        let sel_set_path = sel_registerName(b"setPath:\0".as_ptr() as _);
        let sel_set_fill_rule = sel_registerName(b"setFillRule:\0".as_ptr() as _);
        let sel_str_utf8 = sel_registerName(b"stringWithUTF8String:\0".as_ptr() as _);
        let sel_invalidate_shadow = sel_registerName(b"invalidateShadow\0".as_ptr() as _);
        let sel_set_opaque = sel_registerName(b"setOpaque:\0".as_ptr() as _);
        let sel_set_bg = sel_registerName(b"setBackgroundColor:\0".as_ptr() as _);
        let sel_clear_color = sel_registerName(b"clearColor\0".as_ptr() as _);

        let get_id: extern "C" fn(Id, Sel) -> Id =
            std::mem::transmute(objc_msgSend as *const std::ffi::c_void);
        let get_usize: extern "C" fn(Id, Sel) -> usize =
            std::mem::transmute(objc_msgSend as *const std::ffi::c_void);
        let get_isize: extern "C" fn(Id, Sel) -> isize =
            std::mem::transmute(objc_msgSend as *const std::ffi::c_void);
        let get_f64: extern "C" fn(Id, Sel) -> f64 =
            std::mem::transmute(objc_msgSend as *const std::ffi::c_void);
        let obj_at: extern "C" fn(Id, Sel, usize) -> Id =
            std::mem::transmute(objc_msgSend as *const std::ffi::c_void);
        let get_rect: extern "C" fn(Id, Sel) -> CGRect =
            std::mem::transmute(objc_msgSend as *const std::ffi::c_void);
        let set_id: extern "C" fn(Id, Sel, Id) =
            std::mem::transmute(objc_msgSend as *const std::ffi::c_void);
        let set_bool: extern "C" fn(Id, Sel, u8) =
            std::mem::transmute(objc_msgSend as *const std::ffi::c_void);
        let set_ptr: extern "C" fn(Id, Sel, *mut std::ffi::c_void) =
            std::mem::transmute(objc_msgSend as *const std::ffi::c_void);
        let call_void: extern "C" fn(Id, Sel) =
            std::mem::transmute(objc_msgSend as *const std::ffi::c_void);
        let str_utf8: extern "C" fn(Id, Sel, *const c_char) -> Id =
            std::mem::transmute(objc_msgSend as *const std::ffi::c_void);

        // 1. Find the overlay NSWindow by windowNumber.
        let ns_app_cls = objc_getClass(b"NSApplication\0".as_ptr() as _);
        let ns_app = get_id(ns_app_cls, sel_shared_app);
        let all_wins = get_id(ns_app, sel_windows);
        let win_count = if all_wins.is_null() { 0 } else { get_usize(all_wins, sel_count) };
        let mut overlay_win: Id = std::ptr::null_mut();
        for i in 0..win_count {
            let win = obj_at(all_wins, sel_obj_at, i);
            if !win.is_null() && get_isize(win, sel_win_number) == overlay_wnum {
                overlay_win = win;
                break;
            }
        }
        if overlay_win.is_null() {
            tracing::debug!(
                overlay_wnum,
                "[pane-hole-mask] overlay NSWindow not found (closed or not yet created)"
            );
            return false;
        }

        let content_view = get_id(overlay_win, sel_content_view);
        if content_view.is_null() {
            tracing::warn!(overlay_wnum, "[pane-hole-mask] contentView is nil");
            return false;
        }
        set_bool(content_view, sel_wants_layer, 1);
        let layer = get_id(content_view, sel_layer);
        if layer.is_null() {
            tracing::warn!(overlay_wnum, "[pane-hole-mask] contentView.layer is nil");
            return false;
        }

        // 2. Click-through prerequisite: transparent window pixels only pass
        //    events through when the window is non-opaque with a clear
        //    background. Applied ONCE per window — re-setting these forces
        //    the window server to recomposite the whole window, which showed
        //    as a flicker on every menu open when this ran unconditionally.
        {
            use std::sync::Mutex;
            static PREPARED: Mutex<Option<std::collections::HashSet<isize>>> = Mutex::new(None);
            let mut guard = PREPARED.lock().unwrap_or_else(|p| p.into_inner());
            let prepared = guard.get_or_insert_with(Default::default);
            if prepared.insert(overlay_wnum) {
                set_bool(overlay_win, sel_set_opaque, 0);
                let ns_color_cls = objc_getClass(b"NSColor\0".as_ptr() as _);
                let clear_color = get_id(ns_color_cls, sel_clear_color);
                set_id(overlay_win, sel_set_bg, clear_color);
            }
        }

        if holes.is_empty() {
            // Clear the mask — full pane visible again.
            set_ptr(layer, sel_set_mask, std::ptr::null_mut());
            call_void(overlay_win, sel_invalidate_shadow);
            tracing::info!(overlay_wnum, "[pane-hole-mask] mask cleared");
            return true;
        }

        // 3. Build the even-odd path: full bounds + one rect per hole.
        //    Layer coords are points, origin bottom-left (non-flipped view),
        //    while pane/hole rects are physical px with origin top-left.
        let bounds = get_rect(content_view, sel_bounds);
        let scale = {
            let s = get_f64(overlay_win, sel_backing_scale);
            if s > 0.0 { s } else { 1.0 }
        };
        let (px, py, _pw, _ph) = pane_rect;
        let path = CGPathCreateMutable();
        CGPathAddRect(path, std::ptr::null(), bounds);
        for &(hx, hy, hw, hh) in holes {
            if hw <= 0 || hh <= 0 {
                continue;
            }
            let lx = (hx - px) as f64 / scale;
            let ly_top = (hy - py) as f64 / scale;
            let lw = hw as f64 / scale;
            let lh = hh as f64 / scale;
            // Flip: layer origin is bottom-left.
            let ly = bounds.size.h - ly_top - lh;
            CGPathAddRect(
                path,
                std::ptr::null(),
                CGRect {
                    origin: CGPoint { x: lx, y: ly },
                    size: CGSize { w: lw, h: lh },
                },
            );
        }

        // 4. CAShapeLayer mask with even-odd fill: bounds minus holes.
        let shape_cls = objc_getClass(b"CAShapeLayer\0".as_ptr() as _);
        let mask_layer = get_id(shape_cls, sel_layer_cls_new); // +layer (autoreleased)
        if mask_layer.is_null() {
            CGPathRelease(path);
            tracing::warn!(overlay_wnum, "[pane-hole-mask] CAShapeLayer alloc failed");
            return false;
        }
        let ns_string_cls = objc_getClass(b"NSString\0".as_ptr() as _);
        let even_odd = str_utf8(ns_string_cls, sel_str_utf8, b"even-odd\0".as_ptr() as _);
        set_id(mask_layer, sel_set_fill_rule, even_odd);
        set_ptr(mask_layer, sel_set_path, path);
        CGPathRelease(path); // setPath: copies/retains the path
        set_id(layer, sel_set_mask, mask_layer);
        call_void(overlay_win, sel_invalidate_shadow);
        tracing::info!(
            overlay_wnum,
            hole_count = holes.len(),
            "[pane-hole-mask] mask applied"
        );
        true
    }
}
