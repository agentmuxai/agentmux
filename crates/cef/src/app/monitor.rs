// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Monitor / DPI geometry utilities — work-area lookup and centered-
// window placement math used by `AgentMuxWindowDelegate::on_window_created`
// (see the parent `app` module) and by pool/tear-off window placement
// elsewhere in the crate. Split out of the single-file `app` module (now `app/mod.rs`).

use cef::*;

/// Compute a centered 70% rect for the monitor the window is currently on.
/// Returns (x, y, width, height) or None if the monitor can't be determined.
pub(crate) fn get_monitor_centered_70pct(window: &Window) -> Option<(i32, i32, i32, i32)> {
    let bounds = window.bounds();
    let (work_x, work_y, work_w, work_h) = get_monitor_work_area(bounds.x, bounds.y)?;
    let w = (work_w as f64 * 0.70) as i32;
    let h = (work_h as f64 * 0.70) as i32;
    let x = work_x + (work_w - w) / 2;
    let y = work_y + (work_h - h) / 2;
    Some((x, y, w, h))
}

/// Get the work area (excluding taskbar/dock) of the monitor containing (px, py).
/// Returns (x, y, width, height) of the work area.
#[cfg(target_os = "windows")]
pub fn get_monitor_work_area(px: i32, py: i32) -> Option<(i32, i32, i32, i32)> {
    use windows_sys::Win32::Graphics::Gdi::{
        MonitorFromPoint, GetMonitorInfoW, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
    };
    use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    unsafe {
        let point = windows_sys::Win32::Foundation::POINT { x: px, y: py };
        let hmonitor = MonitorFromPoint(point, MONITOR_DEFAULTTOPRIMARY);
        if hmonitor.is_null() {
            return None;
        }
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(hmonitor, &mut info) == 0 {
            return None;
        }
        // Convert physical pixels → DIP (logical) pixels.
        // CEF Views set_bounds() expects DIP; GetMonitorInfoW returns physical pixels.
        // On Windows 10 @ 100%: dpi_x == 96 → scale == 1.0 (no change).
        // On Windows 11 @ 125%: dpi_x == 120 → divide physical coords by 1.25.
        let mut dpi_x: u32 = 96;
        let mut dpi_y: u32 = 96;
        let _ = GetDpiForMonitor(hmonitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
        let scale = dpi_x as f64 / 96.0;
        let rc = info.rcWork;
        Some((
            (rc.left as f64 / scale).round() as i32,
            (rc.top as f64 / scale).round() as i32,
            ((rc.right - rc.left) as f64 / scale).round() as i32,
            ((rc.bottom - rc.top) as f64 / scale).round() as i32,
        ))
    }
}

/// Like [`get_monitor_work_area`] but returns the work area in **physical**
/// pixels (no DIP division). Win32 `SetWindowPos`/`GetWindowRect` operate in
/// physical pixels, so clamping a physical-pixel window rect must use physical
/// work-area bounds — using the DIP variant over-constrains placement on HiDPI
/// (reagent P1 on PR #1652). Returns `(left, top, width, height)`.
#[cfg(target_os = "windows")]
pub fn get_monitor_work_area_physical(px: i32, py: i32) -> Option<(i32, i32, i32, i32)> {
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTOPRIMARY,
    };
    unsafe {
        let point = windows_sys::Win32::Foundation::POINT { x: px, y: py };
        let hmonitor = MonitorFromPoint(point, MONITOR_DEFAULTTOPRIMARY);
        if hmonitor.is_null() {
            return None;
        }
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(hmonitor, &mut info) == 0 {
            return None;
        }
        let rc = info.rcWork;
        Some((rc.left, rc.top, rc.right - rc.left, rc.bottom - rc.top))
    }
}

/// Effective DPI scale (1.0 == 96 DPI == 100%) of the monitor under `(px, py)`
/// in physical px. Used to convert physical-pixel rects to DIP for CEF Views
/// `set_bounds` (which works in DIP). Returns 1.0 if the monitor can't be found.
#[cfg(target_os = "windows")]
pub fn dpi_scale_at(px: i32, py: i32) -> f32 {
    use windows_sys::Win32::Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTOPRIMARY};
    use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
    unsafe {
        let pt = windows_sys::Win32::Foundation::POINT { x: px, y: py };
        let mon = MonitorFromPoint(pt, MONITOR_DEFAULTTOPRIMARY);
        if mon.is_null() {
            return 1.0;
        }
        let (mut dx, mut dy) = (96u32, 96u32);
        let _ = GetDpiForMonitor(mon, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
        (dx as f32 / 96.0).max(0.1)
    }
}

/// An AppKit rect in Cocoa coordinates: points, origin at the bottom-left of
/// the primary display, y up.
#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Copy, Debug, PartialEq)]
struct CocoaRect {
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

/// One display as `[NSScreen frame]` / `[NSScreen visibleFrame]` report it.
/// `visible` is `frame` minus the menu bar and the Dock.
#[cfg(any(target_os = "macos", test))]
#[derive(Clone, Copy, Debug, PartialEq)]
struct CocoaScreen {
    frame: CocoaRect,
    visible: CocoaRect,
}

/// Work area (`visibleFrame`) of the display containing `(px, py)`, converted
/// to the coordinates CEF Views uses.
///
/// `screens[0]` must be the primary display (the one carrying the menu bar),
/// which is the order `[NSScreen screens]` returns. A point on no display
/// falls back to the primary display, like `MONITOR_DEFAULTTOPRIMARY` does on
/// Windows.
#[cfg(any(target_os = "macos", test))]
fn work_area_at(screens: &[CocoaScreen], px: i32, py: i32) -> Option<(i32, i32, i32, i32)> {
    let primary = screens.first()?;
    // CEF Views puts the origin at the top-left of the primary display with y
    // down; Cocoa puts it at the bottom-left with y up. The primary display's
    // top edge is the flip axis for every display.
    let flip = primary.frame.y + primary.frame.h;
    let top_of = |r: &CocoaRect| flip - (r.y + r.h);
    let (x, y) = (px as f64, py as f64);
    let screen = screens
        .iter()
        .find(|s| {
            let top = top_of(&s.frame);
            x >= s.frame.x && x < s.frame.x + s.frame.w && y >= top && y < top + s.frame.h
        })
        .unwrap_or(primary);
    let v = &screen.visible;
    Some((
        v.x.round() as i32,
        top_of(v).round() as i32,
        v.w.round() as i32,
        v.h.round() as i32,
    ))
}

/// Every attached display, primary first. Must run on the main (UI) thread.
#[cfg(target_os = "macos")]
fn appkit_screens() -> Vec<CocoaScreen> {
    use std::ffi::{c_char, c_void};
    type Id = *mut c_void;
    type Sel = *const c_void;

    extern "C" {
        fn sel_registerName(name: *const c_char) -> Sel;
        fn objc_getClass(name: *const c_char) -> Id;
        fn objc_msgSend();
        #[cfg(target_arch = "x86_64")]
        fn objc_msgSend_stret();
    }

    // Four doubles: returned in registers on arm64 (a homogeneous float
    // aggregate) but through a hidden out-pointer on x86_64, which is why the
    // getter below is arch-specific.
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct NSRect {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    }

    unsafe {
        let sel = |name: &'static [u8]| sel_registerName(name.as_ptr() as *const c_char);
        let get_id: extern "C" fn(Id, Sel) -> Id =
            std::mem::transmute(objc_msgSend as *const c_void);
        let get_usize: extern "C" fn(Id, Sel) -> usize =
            std::mem::transmute(objc_msgSend as *const c_void);
        let obj_at: extern "C" fn(Id, Sel, usize) -> Id =
            std::mem::transmute(objc_msgSend as *const c_void);
        let get_rect = |obj: Id, s: Sel| -> NSRect {
            #[cfg(target_arch = "x86_64")]
            {
                let f: extern "C" fn(*mut NSRect, Id, Sel) =
                    std::mem::transmute(objc_msgSend_stret as *const c_void);
                let mut out = NSRect {
                    x: 0.0,
                    y: 0.0,
                    w: 0.0,
                    h: 0.0,
                };
                f(&mut out, obj, s);
                out
            }
            #[cfg(not(target_arch = "x86_64"))]
            {
                let f: extern "C" fn(Id, Sel) -> NSRect =
                    std::mem::transmute(objc_msgSend as *const c_void);
                f(obj, s)
            }
        };

        let ns_screen = objc_getClass(b"NSScreen\0".as_ptr() as *const c_char);
        if ns_screen.is_null() {
            return Vec::new();
        }
        let screens = get_id(ns_screen, sel(b"screens\0"));
        if screens.is_null() {
            return Vec::new();
        }
        let (sel_frame, sel_visible) = (sel(b"frame\0"), sel(b"visibleFrame\0"));
        let (sel_count, sel_obj_at) = (sel(b"count\0"), sel(b"objectAtIndex:\0"));
        (0..get_usize(screens, sel_count))
            .filter_map(|i| {
                let s = obj_at(screens, sel_obj_at, i);
                if s.is_null() {
                    return None;
                }
                let (f, v) = (get_rect(s, sel_frame), get_rect(s, sel_visible));
                Some(CocoaScreen {
                    frame: CocoaRect {
                        x: f.x,
                        y: f.y,
                        w: f.w,
                        h: f.h,
                    },
                    visible: CocoaRect {
                        x: v.x,
                        y: v.y,
                        w: v.w,
                        h: v.h,
                    },
                })
            })
            .collect()
    }
}

/// Work area (`NSScreen.visibleFrame`, so minus the menu bar and the Dock) of
/// the display containing `(px, py)`, in points. Points are DIP, so unlike the
/// Windows variant there is no scale to divide out.
#[cfg(target_os = "macos")]
pub fn get_monitor_work_area(px: i32, py: i32) -> Option<(i32, i32, i32, i32)> {
    work_area_at(&appkit_screens(), px, py)
}

#[cfg(target_os = "linux")]
pub fn get_monitor_work_area(_px: i32, _py: i32) -> Option<(i32, i32, i32, i32)> {
    // X11: XDisplayWidth/XDisplayHeight on the default screen.
    // This is the full screen, not work area (no taskbar subtraction).
    // TODO: use _NET_WORKAREA from the root window for proper work area.
    None // Falls back to 1200x800 default
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f64, y: f64, w: f64, h: f64) -> CocoaRect {
        CocoaRect { x, y, w, h }
    }

    /// 1440x900 primary with a 24pt menu bar and a 70pt Dock at the bottom.
    fn primary() -> CocoaScreen {
        CocoaScreen {
            frame: rect(0.0, 0.0, 1440.0, 900.0),
            visible: rect(0.0, 70.0, 1440.0, 806.0),
        }
    }

    /// 1920x1080 display to the right of the primary, aligned at the bottom
    /// edge, with a 25pt menu bar and no Dock.
    fn right_of_primary() -> CocoaScreen {
        CocoaScreen {
            frame: rect(1440.0, 0.0, 1920.0, 1080.0),
            visible: rect(1440.0, 0.0, 1920.0, 1055.0),
        }
    }

    /// 1440x900 display stacked above the primary.
    fn above_primary() -> CocoaScreen {
        CocoaScreen {
            frame: rect(0.0, 900.0, 1440.0, 900.0),
            visible: rect(0.0, 900.0, 1440.0, 876.0),
        }
    }

    #[test]
    fn single_display_excludes_menu_bar_and_bottom_dock() {
        // Cocoa visible y=70..876 flips to a top-left origin of y=24 (the menu bar).
        assert_eq!(
            work_area_at(&[primary()], 100, 100),
            Some((0, 24, 1440, 806))
        );
    }

    #[test]
    fn a_dock_on_the_left_shifts_the_origin_right() {
        let s = CocoaScreen {
            frame: rect(0.0, 0.0, 1440.0, 900.0),
            visible: rect(80.0, 0.0, 1360.0, 876.0),
        };
        assert_eq!(work_area_at(&[s], 500, 500), Some((80, 24, 1360, 876)));
    }

    #[test]
    fn point_on_a_display_right_of_primary_picks_that_display() {
        let screens = [primary(), right_of_primary()];
        // Both displays share a bottom edge, so the taller one extends above y=0.
        assert_eq!(
            work_area_at(&screens, 2000, -100),
            Some((1440, -155, 1920, 1055))
        );
    }

    #[test]
    fn point_on_a_display_above_primary_picks_that_display() {
        let screens = [primary(), above_primary()];
        // The display's top edge is y=-900; its work area starts 24pt lower, under the menu bar.
        assert_eq!(
            work_area_at(&screens, 100, -450),
            Some((0, -876, 1440, 876))
        );
    }

    #[test]
    fn shared_edge_belongs_to_the_display_that_starts_there() {
        let screens = [primary(), right_of_primary()];
        assert_eq!(work_area_at(&screens, 1439, 100), Some((0, 24, 1440, 806)));
        assert_eq!(
            work_area_at(&screens, 1440, 100),
            Some((1440, -155, 1920, 1055))
        );
    }

    #[test]
    fn point_on_no_display_falls_back_to_primary() {
        let screens = [primary(), right_of_primary()];
        assert_eq!(work_area_at(&screens, 9000, 9000), Some((0, 24, 1440, 806)));
        assert_eq!(
            work_area_at(&screens, -5000, -5000),
            Some((0, 24, 1440, 806))
        );
    }

    #[test]
    fn fractional_points_round_to_whole_points() {
        let s = CocoaScreen {
            frame: rect(0.0, 0.0, 1512.0, 982.0),
            visible: rect(0.0, 0.0, 1511.6, 950.4),
        };
        assert_eq!(work_area_at(&[s], 10, 10), Some((0, 32, 1512, 950)));
    }

    /// Reads the real displays through AppKit, so a wrong struct-return ABI
    /// (garbage rects) or a wrong selector shows up here.
    #[cfg(target_os = "macos")]
    #[test]
    fn appkit_reports_sane_displays() {
        let screens = appkit_screens();
        assert!(!screens.is_empty(), "NSScreen.screens returned nothing");
        for s in &screens {
            assert!(s.frame.w > 0.0 && s.frame.h > 0.0, "empty frame: {s:?}");
            let (f, v) = (s.frame, s.visible);
            assert!(
                v.w > 0.0
                    && v.h > 0.0
                    && v.x >= f.x
                    && v.y >= f.y
                    && v.x + v.w <= f.x + f.w
                    && v.y + v.h <= f.y + f.h,
                "visibleFrame not inside frame: {s:?}"
            );
        }
        // The primary display sits at the Cocoa origin by definition.
        assert_eq!((screens[0].frame.x, screens[0].frame.y), (0.0, 0.0));
        let (x, y, w, h) =
            get_monitor_work_area(10, 10).expect("work area for a point on the primary display");
        eprintln!("displays: {screens:?}\nwork area at (10,10): ({x},{y},{w},{h})");
        assert!(w > 0 && h > 0 && x >= 0 && y >= 0, "({x},{y},{w},{h})");
    }

    #[test]
    fn no_displays_yields_none() {
        assert_eq!(work_area_at(&[], 0, 0), None);
    }
}
