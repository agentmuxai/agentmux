// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Which kind of browser pane this process creates.
//!
//! Panes are CEF Views overlays in the window's own compositor on every platform.
//! Windows panes used to be child HWNDs that DWM composes with the window, so a
//! resize showed the chrome and the page a frame or more apart
//! (docs/reports/REPORT_RESIZE_LOCKSTEP_CHROME_AND_CEF_PANE_2026_11_03.md, option b).
//! Windows keeps that kind one switch away. The kind is chosen once, at startup,
//! for the whole process: `AGENTMUX_PANE_BACKEND=hwnd` selects the child HWNDs, and
//! anything else (or nothing) the Views overlays.
//!
//! In a Views process the kind then follows the window a pane is created in. An
//! overlay needs a CEF Views window; a floating pane and a pane pool window on
//! Windows are raw windows (`crate::floating_pane`), so a pane created in one is
//! still a child HWND. A pane is created anew when it moves to another window, so
//! it never changes kind. Each place that differs by kind asks for the pane
//! ([`pane_is_views`]) or the window ([`window_is_views`]), not the platform;
//! [`views`] alone is only the process's choice.

/// Whether panes are Views overlays: always off Windows; on Windows unless the
/// process was started with `AGENTMUX_PANE_BACKEND=hwnd`.
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

/// Whether a pane created in window `label` is a Views overlay: off Windows
/// always; on Windows when the process creates Views panes and `label` is a CEF
/// Views window (`AppState::windows`), which a floating pane's window is not.
#[cfg(not(target_os = "windows"))]
pub fn window_is_views(_state: &crate::state::AppState, _label: &str) -> bool {
    true
}

/// See the non-Windows variant.
#[cfg(target_os = "windows")]
pub fn window_is_views(state: &crate::state::AppState, label: &str) -> bool {
    views() && state.windows.lock().contains_key(label)
}

/// The panes of a Views process that are child HWNDs all the same, by pane label
/// (`browser-pane-<block>-<seq>`, never reused): the ones created in a raw
/// window. Empty off Windows and in an HWND process.
#[cfg(target_os = "windows")]
static HWND_PANES: parking_lot::Mutex<std::collections::BTreeSet<String>> =
    parking_lot::Mutex::new(std::collections::BTreeSet::new());

/// Pane `pane_label` has just been given its entry, to be created in window
/// `window_label`: its kind is decided here, once, and returned (true for a Views
/// overlay). Called where the entry is made (`BrowserPaneManager::create`), not
/// in the creation task, so no one asks for the kind before it is recorded.
#[cfg(target_os = "windows")]
pub fn note_pane_created(state: &crate::state::AppState, pane_label: &str, window_label: &str) -> bool {
    let is_views = window_is_views(state, window_label);
    if views() && !is_views {
        HWND_PANES.lock().insert(pane_label.to_string());
    }
    is_views
}

/// See the Windows variant.
#[cfg(not(target_os = "windows"))]
pub fn note_pane_created(_state: &crate::state::AppState, _pane_label: &str, _window_label: &str) -> bool {
    true
}

/// Pane `pane_label`'s browser is gone (its last callback has run), or none was
/// ever made and the pane is closed: nothing asks for its kind again.
pub fn forget_pane(pane_label: &str) {
    #[cfg(target_os = "windows")]
    HWND_PANES.lock().remove(pane_label);
    #[cfg(not(target_os = "windows"))]
    let _ = pane_label;
}

/// Whether the pane with label `pane_label` is a Views overlay, as decided when
/// it was created ([`note_pane_created`]).
pub fn label_is_views(pane_label: &str) -> bool {
    #[cfg(target_os = "windows")]
    return views() && !HWND_PANES.lock().contains(pane_label);
    #[cfg(not(target_os = "windows"))]
    {
        let _ = pane_label;
        true
    }
}

/// Whether block `block_id`'s pane is a Views overlay. A block with no pane has
/// the process's kind.
pub fn pane_is_views(state: &crate::state::AppState, block_id: &str) -> bool {
    #[cfg(target_os = "windows")]
    {
        let label = state.host_state.lock().browser_panes.get(block_id).map(|e| e.label.clone());
        return label.map_or_else(views, |l| label_is_views(&l));
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (state, block_id);
        true
    }
}

/// The setting's value as a choice: `hwnd` (any case, spaces ignored) selects
/// the child HWND panes; anything else, including no value, the Views overlays.
#[cfg(any(target_os = "windows", test))]
fn from_setting(value: Option<&str>) -> bool {
    !value.is_some_and(|v| v.trim().eq_ignore_ascii_case("hwnd"))
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
    fn only_hwnd_selects_the_child_window_panes() {
        assert!(from_setting(None));
        assert!(from_setting(Some("")));
        assert!(from_setting(Some("views")));
        assert!(from_setting(Some("child")));
        assert!(!from_setting(Some("hwnd")));
        assert!(!from_setting(Some(" HWND ")));
    }
}
