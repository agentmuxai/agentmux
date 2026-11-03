// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which kind of browser pane this process creates.
//!
//! macOS and Linux panes are CEF Views overlays in the window's own compositor.
//! Windows panes have been child HWNDs that DWM composes with the window, so a
//! resize shows the chrome and the page a frame or more apart
//! (docs/reports/REPORT_RESIZE_LOCKSTEP_CHROME_AND_CEF_PANE_2026_11_03.md, option b).
//! Windows can now create Views overlay panes too. The kind is chosen once, at
//! startup, for the whole process: `AGENTMUX_PANE_BACKEND=views` selects them, and
//! anything else (or nothing) keeps the HWND panes. A pane never changes kind, and
//! the two kinds are never mixed, so each place that differs by kind asks
//! [`views`], not the platform.

/// Whether panes are Views overlays: always off Windows; on Windows only when
/// the process was started with `AGENTMUX_PANE_BACKEND=views`.
#[cfg(not(target_os = "windows"))]
pub const fn views() -> bool {
    true
}

/// See the non-Windows variant.
#[cfg(target_os = "windows")]
pub fn views() -> bool {
    static CHOSEN: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *CHOSEN.get_or_init(|| from_setting(std::env::var("AGENTMUX_PANE_BACKEND").ok().as_deref()))
}

/// The setting's value as a choice: `views` (any case, spaces ignored) selects
/// Views overlays; anything else, including no value, keeps the HWND panes.
#[cfg(any(target_os = "windows", test))]
fn from_setting(value: Option<&str>) -> bool {
    value.is_some_and(|v| v.trim().eq_ignore_ascii_case("views"))
}

/// Frames the window's compositor may wait for a pane's surface at the window's
/// new size before it shows the window's own frame without it (Chromium's
/// `deadline-to-synchronize-surfaces`). Short, so a pane whose page is busy or
/// hung delays a resize by a few frames, not until the page answers.
#[cfg(any(target_os = "windows", test))]
const DEFAULT_DEADLINE_FRAMES: u32 = 4;

/// The deadline to pass to Chromium in Views pane mode: `AGENTMUX_PANE_SURFACE_DEADLINE_FRAMES`
/// as a whole number of frames, `off` to pass none (Chromium's own), and the
/// default for no value or one that isn't a positive number.
#[cfg(any(target_os = "windows", test))]
fn deadline_from_setting(value: Option<&str>) -> Option<u32> {
    match value.map(str::trim) {
        Some(v) if v.eq_ignore_ascii_case("off") => None,
        Some(v) => Some(v.parse::<u32>().ok().filter(|n| *n > 0).unwrap_or(DEFAULT_DEADLINE_FRAMES)),
        None => Some(DEFAULT_DEADLINE_FRAMES),
    }
}

/// Views pane mode on Windows: the window waits a few frames for a pane's
/// surface at the new size, so the chrome and the page arrive in one frame, and
/// the wait is bounded. The other platforms, and the child-window panes, keep
/// Chromium's own.
#[cfg(target_os = "windows")]
pub fn append_surface_deadline(cmd: &mut cef::CommandLine) {
    use cef::ImplCommandLine;
    if !views() {
        return;
    }
    let Some(frames) = deadline_from_setting(std::env::var("AGENTMUX_PANE_SURFACE_DEADLINE_FRAMES").ok().as_deref()) else {
        return;
    };
    cmd.append_switch_with_value(
        Some(&cef::CefString::from("deadline-to-synchronize-surfaces")),
        Some(&cef::CefString::from(frames.to_string().as_str())),
    );
}

#[cfg(test)]
mod tests {
    use super::{deadline_from_setting, from_setting, DEFAULT_DEADLINE_FRAMES};

    #[test]
    fn the_deadline_is_a_positive_number_of_frames_or_off() {
        assert_eq!(deadline_from_setting(None), Some(DEFAULT_DEADLINE_FRAMES));
        assert_eq!(deadline_from_setting(Some("8")), Some(8));
        assert_eq!(deadline_from_setting(Some(" 2 ")), Some(2));
        assert_eq!(deadline_from_setting(Some("0")), Some(DEFAULT_DEADLINE_FRAMES));
        assert_eq!(deadline_from_setting(Some("soon")), Some(DEFAULT_DEADLINE_FRAMES));
        assert_eq!(deadline_from_setting(Some("OFF")), None);
    }

    #[test]
    fn only_views_selects_the_views_panes() {
        assert!(from_setting(Some("views")));
        assert!(from_setting(Some(" Views ")));
        assert!(!from_setting(Some("hwnd")));
        assert!(!from_setting(Some("")));
        assert!(!from_setting(None));
    }
}
