// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The mouse side buttons (Back / Forward) as the host sees them, sent to the
//! window's frontend as a `side-button` event. Two places call this: a browser
//! pane's windows, whose clicks never reach the page's DOM, and the top-level
//! window's `WM_APPCOMMAND`, which Windows derives from an unhandled
//! `WM_XBUTTONUP` and which would otherwise make Chromium navigate the app's
//! own page. docs/specs/SPEC_MOUSE_SIDE_BUTTONS_PANE_AND_TAB_NAV_2026_10_19.md §5.

#![cfg(target_os = "windows")]

use std::sync::Arc;

use crate::state::AppState;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SideButton {
    Back,
    Forward,
}

impl SideButton {
    pub(crate) fn as_str(self) -> &'static str {
        match self {
            SideButton::Back => "back",
            SideButton::Forward => "forward",
        }
    }
}

const APPCOMMAND_BROWSER_BACKWARD: u32 = 1;
const APPCOMMAND_BROWSER_FORWARD: u32 = 2;
const FAPPCOMMAND_MASK: u32 = 0xF000;
const FAPPCOMMAND_MOUSE: u32 = 0x8000;
const XBUTTON1: u32 = 1;
const XBUTTON2: u32 = 2;
const MK_CONTROL: usize = 0x0008;

/// A `WM_APPCOMMAND`'s browser Back / Forward, and whether it came from the
/// mouse (not a keyboard's browser keys).
pub(crate) fn decode_appcommand(lparam: isize) -> Option<(SideButton, bool)> {
    let hi = (((lparam as usize) >> 16) & 0xFFFF) as u32;
    let from_mouse = hi & FAPPCOMMAND_MASK == FAPPCOMMAND_MOUSE;
    match hi & !FAPPCOMMAND_MASK {
        APPCOMMAND_BROWSER_BACKWARD => Some((SideButton::Back, from_mouse)),
        APPCOMMAND_BROWSER_FORWARD => Some((SideButton::Forward, from_mouse)),
        _ => None,
    }
}

/// A `WM_XBUTTON*`'s button, and whether Ctrl was held (`MK_CONTROL`).
pub(crate) fn decode_xbutton(wparam: usize) -> Option<(SideButton, bool)> {
    let ctrl = wparam & MK_CONTROL != 0;
    match ((wparam >> 16) & 0xFFFF) as u32 {
        XBUTTON1 => Some((SideButton::Back, ctrl)),
        XBUTTON2 => Some((SideButton::Forward, ctrl)),
        _ => None,
    }
}

/// Whether Ctrl is down right now (a `WM_APPCOMMAND` carries no modifiers).
pub(crate) fn ctrl_down() -> bool {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VK_CONTROL};
    unsafe { GetKeyState(VK_CONTROL as i32) < 0 }
}

pub(crate) fn emit(
    state: &Arc<AppState>,
    window: &str,
    button: SideButton,
    ctrl: bool,
    source: &str,
    block_id: Option<&str>,
) {
    tracing::info!(
        "[side-button] {} ctrl={} source={} window={} block={:?}",
        button.as_str(),
        ctrl,
        source,
        window,
        block_id,
    );
    crate::events::emit_event_to_window(
        state,
        window,
        "side-button",
        &serde_json::json!({
            "button": button.as_str(),
            "ctrl": ctrl,
            "source": source,
            "block_id": block_id,
        }),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn appcommand_lparam(cmd_and_device: u32) -> isize {
        ((cmd_and_device as isize) << 16) | 0x0001
    }

    #[test]
    fn appcommand_from_the_mouse() {
        assert_eq!(
            decode_appcommand(appcommand_lparam(FAPPCOMMAND_MOUSE | APPCOMMAND_BROWSER_BACKWARD)),
            Some((SideButton::Back, true))
        );
        assert_eq!(
            decode_appcommand(appcommand_lparam(FAPPCOMMAND_MOUSE | APPCOMMAND_BROWSER_FORWARD)),
            Some((SideButton::Forward, true))
        );
    }

    #[test]
    fn appcommand_from_the_keyboard_is_not_the_mouse() {
        // FAPPCOMMAND_KEY is 0.
        assert_eq!(
            decode_appcommand(appcommand_lparam(APPCOMMAND_BROWSER_BACKWARD)),
            Some((SideButton::Back, false))
        );
    }

    #[test]
    fn other_appcommands_are_ignored() {
        // APPCOMMAND_BROWSER_REFRESH = 3
        assert_eq!(decode_appcommand(appcommand_lparam(FAPPCOMMAND_MOUSE | 3)), None);
    }

    #[test]
    fn xbutton_reads_button_and_ctrl() {
        assert_eq!(decode_xbutton((XBUTTON1 as usize) << 16), Some((SideButton::Back, false)));
        assert_eq!(
            decode_xbutton(((XBUTTON2 as usize) << 16) | MK_CONTROL),
            Some((SideButton::Forward, true))
        );
        assert_eq!(decode_xbutton(3usize << 16), None);
        assert_eq!(decode_xbutton(0), None);
    }
}
