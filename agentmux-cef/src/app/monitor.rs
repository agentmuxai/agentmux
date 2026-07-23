// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Monitor / DPI geometry utilities — work-area lookup and centered-
// window placement math used by `AgentMuxWindowDelegate::on_window_created`
// (see the parent `app` module) and by pool/tear-off window placement
// elsewhere in the crate. Split out of `app.rs` (now `app/mod.rs`).

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

#[cfg(target_os = "macos")]
pub fn get_monitor_work_area(_px: i32, _py: i32) -> Option<(i32, i32, i32, i32)> {
    // TODO: Use NSScreen.main.visibleFrame for proper work area (minus Dock/menu bar).
    // CGMainDisplayID only returns the primary display — doesn't support multi-monitor
    // and hardcoding menu bar height is fragile. Fall back to 1200x800 default.
    None
}

#[cfg(target_os = "linux")]
pub fn get_monitor_work_area(_px: i32, _py: i32) -> Option<(i32, i32, i32, i32)> {
    // X11: XDisplayWidth/XDisplayHeight on the default screen.
    // This is the full screen, not work area (no taskbar subtraction).
    // TODO: use _NET_WORKAREA from the root window for proper work area.
    None // Falls back to 1200x800 default
}
