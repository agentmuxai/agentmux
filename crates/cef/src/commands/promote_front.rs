// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Bringing a just-promoted tear-off window to the front (Windows).

/// Show a just-promoted window and make it the foreground window. `ShowWindow` alone does not take the
/// foreground: the window the drag started in kept it, so a torn-off window tab could open behind the
/// window it was torn from (the floating panes had the same gap, #6395). The process is the foreground
/// one at this point (the drag ran in its window), so the call is allowed.
#[cfg(target_os = "windows")]
pub(crate) fn show_in_front(hwnd: windows_sys::Win32::Foundation::HWND, label: &str) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{SetForegroundWindow, ShowWindow, SW_SHOW};
    // SAFETY: both calls on a stale HWND fail harmlessly.
    let ok = unsafe {
        ShowWindow(hwnd, SW_SHOW);
        SetForegroundWindow(hwnd) != 0
    };
    tracing::info!(target: "dnd:tearoff:pool", label = %label, ok, "[pool] promoted window shown in front");
}

#[cfg(test)]
mod tests {
    /// The Windows promote shows the window only through `show_in_front`: a plain show does not take the
    /// foreground, and a torn-off window tab opens behind the window it was torn from.
    #[test]
    fn the_window_pool_promote_shows_the_window_only_in_front() {
        let src = include_str!("window_pool.rs");
        let plain_show = ["ShowWindow(raw_hwnd, ", "SW_SHOW)"].concat();
        assert!(!src.contains(&plain_show), "a plain ShowWindow leaves the window behind the source window");
        let in_front = ["promote_front::show_in_front(raw_hwnd", ", &label)"].concat();
        assert!(src.contains(&in_front));
    }

    #[test]
    fn a_floater_released_at_the_screen_edge_is_pulled_inside() {
        use crate::commands::window_pool::clamp_rect_within;
        // A 312-wide floater released at x=850 on a 900x1540 work area (a drop near the right edge).
        assert_eq!(clamp_rect_within(850, 620, 312, 497, 0, 0, 900, 1540), (588, 620, 312, 497));
        // Released below the bottom edge.
        assert_eq!(clamp_rect_within(100, 1500, 312, 497, 0, 0, 900, 1540), (100, 1043, 312, 497));
    }
}
