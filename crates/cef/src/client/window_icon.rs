// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The window and taskbar icon follows the Windows theme: the dark icon while Windows is dark, the light
//! one (a light tile with a darker edge, so it shows on a white taskbar) while it is light. It switches
//! live: a thread watches the theme key in the registry and sets the icon again on every window we have
//! given one. Both icons are embedded in the host as resources 1 (dark, also the exe's own icon) and 2
//! (light); `build.rs` embeds them from `resources/win/`.

use std::sync::{Mutex, Once};

/// Resource id of the dark icon (what the exe, the launcher and Explorer show).
const ICON_DARK: u16 = 1;
/// Resource id of the light icon.
const ICON_LIGHT: u16 = 2;

/// The icon resource for a theme.
fn resource_id(light_theme: bool) -> u16 {
    if light_theme { ICON_LIGHT } else { ICON_DARK }
}

/// The windows we have set an icon on (HWNDs as integers), so a theme change can set it again.
static WINDOWS: Mutex<Vec<isize>> = Mutex::new(Vec::new());
static WATCHER: Once = Once::new();

const PERSONALIZE_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize";

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Whether a Personalize value says light: `SystemUsesLightTheme` is the taskbar and Start,
/// `AppsUseLightTheme` the apps' own surfaces (the title bar). An unreadable value counts as dark, the
/// icon every build used before this.
fn uses_light_theme(value: &str) -> bool {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_CURRENT_USER, RRF_RT_REG_DWORD};
    let key = wide(PERSONALIZE_KEY);
    let name = wide(value);
    let mut data: u32 = 0;
    let mut size = std::mem::size_of::<u32>() as u32;
    let rc = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            key.as_ptr(),
            name.as_ptr(),
            RRF_RT_REG_DWORD,
            std::ptr::null_mut(),
            &mut data as *mut u32 as *mut _,
            &mut size,
        )
    };
    rc == 0 && data == 1
}

/// Set the dark or light icon on `hwnd` for the current Windows theme. The taskbar and Alt+Tab take the
/// big icon (`SystemUsesLightTheme`), the title bar the small one (`AppsUseLightTheme`). Posted, not
/// sent, so the watcher thread never waits on a window's thread.
unsafe fn apply(hwnd: isize) {
    use windows_sys::Win32::System::LibraryLoader::{
        GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
        GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    // The module that CONTAINS this code, not the process exe: under the Phase 3 Windows sandbox (#1633)
    // the host process is CEF's bootstrap.exe, which has no AgentMux icon resource, while the
    // winres-embedded icons live in agentmux_cef.dll. `FROM_ADDRESS` gives the DLL (sandbox build) or
    // the exe (non-sandbox build), whichever carries them.
    let mut hinstance: windows_sys::Win32::Foundation::HMODULE = std::ptr::null_mut();
    let got = GetModuleHandleExW(
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
        apply as *const () as *const u16,
        &mut hinstance,
    );
    if got == 0 || hinstance.is_null() {
        tracing::warn!("window icon: GetModuleHandleExW(FROM_ADDRESS) failed");
        return;
    }

    let load = |light: bool, px: i32| {
        LoadImageW(hinstance, resource_id(light) as usize as *const u16, IMAGE_ICON, px, px, LR_SHARED)
    };
    let big = load(uses_light_theme("SystemUsesLightTheme"), 32);
    let small = load(uses_light_theme("AppsUseLightTheme"), 16);
    if !big.is_null() {
        PostMessageW(hwnd as _, WM_SETICON, ICON_BIG as usize, big as isize);
    }
    if !small.is_null() {
        PostMessageW(hwnd as _, WM_SETICON, ICON_SMALL as usize, small as isize);
    }
    if big.is_null() && small.is_null() {
        tracing::warn!("window icon: no icon found in the host's resources");
    }
}

/// Set the icon on every window we know, dropping the ones that are gone.
fn apply_all() {
    use windows_sys::Win32::UI::WindowsAndMessaging::IsWindow;
    let mut windows = WINDOWS.lock().unwrap_or_else(|e| e.into_inner());
    windows.retain(|&h| unsafe { IsWindow(h as _) } != 0);
    for &h in windows.iter() {
        unsafe { apply(h) };
    }
}

/// Block on the Personalize key and set the icons again each time it changes (Windows writes it when the
/// user switches between light and dark). Ends if the key can't be watched.
fn watch_theme() {
    use windows_sys::Win32::System::Registry::{
        RegCloseKey, RegNotifyChangeKeyValue, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, KEY_NOTIFY,
        REG_NOTIFY_CHANGE_LAST_SET,
    };
    let key_name = wide(PERSONALIZE_KEY);
    let mut key: HKEY = std::ptr::null_mut();
    if unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, key_name.as_ptr(), 0, KEY_NOTIFY, &mut key) } != 0 {
        tracing::warn!("window icon: can't watch the Windows theme; the icon stays as set at start");
        return;
    }
    // Synchronous (no event): returns when a value under the key changes.
    while unsafe { RegNotifyChangeKeyValue(key, 0, REG_NOTIFY_CHANGE_LAST_SET, std::ptr::null_mut(), 0) } == 0 {
        tracing::info!("window icon: the Windows theme changed");
        apply_all();
    }
    unsafe { RegCloseKey(key) };
}

/// Give `hwnd` the app icon for the current Windows theme, and keep it matching when the theme changes.
/// Replaces the icon of CEF/Chromium in the taskbar and Alt+Tab.
pub(super) unsafe fn set_window_icon(hwnd: *mut std::ffi::c_void) {
    {
        let mut windows = WINDOWS.lock().unwrap_or_else(|e| e.into_inner());
        if !windows.contains(&(hwnd as isize)) {
            windows.push(hwnd as isize);
        }
    }
    apply(hwnd as isize);
    WATCHER.call_once(|| {
        if let Err(e) = std::thread::Builder::new().name("window-icon-theme".into()).spawn(watch_theme) {
            tracing::warn!("window icon: can't start the theme watcher: {e}");
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dark_theme_gets_the_dark_icon_and_light_the_light_one() {
        assert_eq!(resource_id(false), 1);
        assert_eq!(resource_id(true), 2);
    }

    /// The `.ico` files the resources are built from: both exist, hold the six sizes, and differ.
    #[test]
    fn both_icons_are_in_the_resources() {
        let dark = include_bytes!("../../resources/win/agentmux.ico");
        let light = include_bytes!("../../resources/win/agentmux-light.ico");
        for ico in [&dark[..], &light[..]] {
            assert_eq!(&ico[..4], &[0, 0, 1, 0], "an .ico header");
            assert_eq!(u16::from_le_bytes([ico[4], ico[5]]), 6, "16, 24, 32, 48, 64 and 256 px");
        }
        assert_ne!(&dark[..], &light[..]);
    }

    #[test]
    fn the_theme_values_read_without_panicking() {
        // Whatever this machine's theme is, both values read to a bool.
        let _ = uses_light_theme("SystemUsesLightTheme");
        let _ = uses_light_theme("AppsUseLightTheme");
        assert!(!uses_light_theme("NoSuchValue"));
    }
}
