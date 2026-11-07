// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Windows: the AgentMux window the "Restart AgentMux to upgrade?" dialog sits over.
//!
//! The dialog is shown by the launcher, and a window of a process the person has not just
//! clicked in is not brought to the front: it waits as a flashing button on the taskbar. A
//! dialog owned by the host's window is shown over it (an owned window is always above its owner,
//! and the person is looking at that window, having just clicked Upgrade & restart in it).
//! `upgrade_restart::confirm` passes the window found here to `MessageBoxW`.

/// A window of the host that could own the dialog.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Candidate {
    pub hwnd: isize,
    /// The window the person is using at this moment.
    pub foreground: bool,
    /// Its size in pixels.
    pub area: i64,
}

/// The window to own the dialog: the one in use now, else the largest.
pub fn pick_owner(candidates: &[Candidate]) -> Option<isize> {
    candidates
        .iter()
        .find(|c| c.foreground)
        .or_else(|| candidates.iter().max_by_key(|c| c.area))
        .map(|c| c.hwnd)
}

/// The host's window to own the dialog, or `None` when it has none the person can see (all
/// minimized, hidden, on another virtual desktop): the dialog is then shown without an owner.
pub fn host_window(host_pid: u32) -> Option<isize> {
    use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM, RECT};
    use windows_sys::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetForegroundWindow, GetWindow, GetWindowLongW, GetWindowRect, GetWindowThreadProcessId,
        IsIconic, IsWindowVisible, GWL_EXSTYLE, GW_OWNER, WS_EX_TOOLWINDOW,
    };

    struct Search {
        pid: u32,
        foreground: HWND,
        found: Vec<Candidate>,
    }

    unsafe extern "system" fn visit(hwnd: HWND, param: LPARAM) -> BOOL {
        let search = &mut *(param as *mut Search);
        let mut pid = 0u32;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid != search.pid || IsWindowVisible(hwnd) == 0 || IsIconic(hwnd) != 0 {
            return 1;
        }
        // Not an owned window (a dialog, a popup) and not a tool window (a tooltip, a palette).
        if !GetWindow(hwnd, GW_OWNER).is_null() || GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TOOLWINDOW != 0 {
            return 1;
        }
        // On another virtual desktop a window is still "visible", but cloaked.
        let mut cloaked = 0u32;
        let hr = DwmGetWindowAttribute(hwnd, DWMWA_CLOAKED as u32, (&mut cloaked as *mut u32).cast(), 4);
        if hr == 0 && cloaked != 0 {
            return 1;
        }
        let mut rect: RECT = std::mem::zeroed();
        if GetWindowRect(hwnd, &mut rect) == 0 {
            return 1;
        }
        let area = i64::from(rect.right - rect.left).max(0) * i64::from(rect.bottom - rect.top).max(0);
        if area > 0 {
            search.found.push(Candidate { hwnd: hwnd as isize, foreground: hwnd == search.foreground, area });
        }
        1
    }

    let mut search = Search { pid: host_pid, foreground: unsafe { GetForegroundWindow() }, found: Vec::new() };
    // SAFETY: `search` outlives the call, and `visit` is the only reader of the pointer.
    unsafe { EnumWindows(Some(visit), &mut search as *mut Search as LPARAM) };
    pick_owner(&search.found)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn win(hwnd: isize, foreground: bool, area: i64) -> Candidate {
        Candidate { hwnd, foreground, area }
    }

    #[test]
    fn the_window_in_use_wins_over_a_larger_one() {
        assert_eq!(pick_owner(&[win(1, false, 2_000_000), win(2, true, 500_000)]), Some(2));
    }

    #[test]
    fn without_a_foreground_window_the_largest_wins() {
        assert_eq!(pick_owner(&[win(1, false, 500_000), win(2, false, 2_000_000), win(3, false, 10)]), Some(2));
    }

    #[test]
    fn no_window_means_no_owner() {
        assert_eq!(pick_owner(&[]), None);
    }

    #[test]
    fn a_pid_with_no_window_finds_none() {
        // pid 0 is the idle process: it owns no window.
        assert_eq!(host_window(0), None);
    }
}
