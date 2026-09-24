// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Taskbar attention surfaces — docs/specs/SPEC_OS_NOTIFICATIONS_SYSTEM_2026_09_24.md
// §10 Phase 4 (absorbs the attention half of the 2026-05-23 taskbar-indicator
// draft).
//
// `set_taskbar_attention { label, count }` — sent by each window's frontend
// when the srv Router's `notification:state` changes:
//
// - count > 0 → an overlay badge on the window's taskbar button
//   (`ITaskbarList3::SetOverlayIcon`), with an accessible description
//   ("2 agents need you") that screen readers announce;
// - count rose and the window isn't foreground → flash the taskbar button
//   (`FLASHW_TRAY | FLASHW_TIMERNOFG`: stops by itself once the window is
//   focused). Only on a *rise* — Microsoft: "the more often you use the flash
//   capability, the less likely it will be effective";
// - count == 0 → clear the overlay.
//
// Windows only; other platforms accept the call and do nothing (the macOS
// Dock badge is Phase 3 work that needs a Mac to verify).

use std::sync::Arc;

use crate::state::AppState;

pub fn set_taskbar_attention(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let label = args.get("label").and_then(|v| v.as_str()).ok_or("set_taskbar_attention: label required")?;
    let count = args.get("count").and_then(|v| v.as_u64()).ok_or("set_taskbar_attention: count required")?;
    #[cfg(target_os = "windows")]
    {
        let hwnd = state.window_hwnds.lock().get(label).copied();
        match hwnd {
            Some(h) => win::post(h, count.min(99) as u32),
            None => tracing::debug!(label, "set_taskbar_attention: no HWND for label"),
        }
    }
    #[cfg(not(target_os = "windows"))]
    let _ = (state, label, count);
    Ok(serde_json::Value::Null)
}

/// The registered `TaskbarButtonCreated` message id (0 if unavailable).
#[cfg(target_os = "windows")]
pub fn taskbar_button_created_msg() -> u32 {
    use std::sync::OnceLock;
    static ID: OnceLock<u32> = OnceLock::new();
    *ID.get_or_init(|| {
        let name: Vec<u16> = "TaskbarButtonCreated".encode_utf16().chain(std::iter::once(0)).collect();
        unsafe { windows_sys::Win32::UI::WindowsAndMessaging::RegisterWindowMessageW(name.as_ptr()) }
    })
}

/// Re-apply the last badge for `hwnd` after its taskbar button was recreated.
/// Never flashes: the count hasn't risen.
#[cfg(target_os = "windows")]
pub fn reapply(hwnd: isize) {
    if let Some(count) = win::last_count(hwnd) {
        if count > 0 {
            win::post(hwnd, count);
        }
    }
}

/// Accessible description for the overlay (also used as its tooltip-ish label).
pub fn description(count: u32) -> String {
    if count == 1 {
        "1 agent needs you".to_string()
    } else {
        format!("{count} agents need you")
    }
}

/// Flash only when attention grew and the window isn't already in front.
pub fn should_flash(prev: u32, next: u32, is_foreground: bool) -> bool {
    next > prev && !is_foreground
}

#[cfg(target_os = "windows")]
mod win {
    use std::collections::HashMap;
    use std::sync::Mutex;

    use cef::*;
    use windows_sys::core::GUID;

    /// Last count applied per HWND (for the flash-on-rise rule).
    static LAST: Mutex<Option<HashMap<isize, u32>>> = Mutex::new(None);

    wrap_task! {
        pub struct SetAttentionTask {
            hwnd: isize,
            count: u32,
        }

        impl Task {
            fn execute(&self) {
                unsafe { apply(self.hwnd, self.count) }
            }
        }
    }

    pub fn last_count(hwnd: isize) -> Option<u32> {
        LAST.lock().unwrap_or_else(|e| e.into_inner()).as_ref().and_then(|m| m.get(&hwnd).copied())
    }

    pub fn post(hwnd: isize, count: u32) {
        let mut task = SetAttentionTask::new(hwnd, count);
        post_task(ThreadId::UI, Some(&mut task));
    }

    // CLSID_TaskbarList / IID_ITaskbarList3.
    const CLSID_TASKBAR_LIST: GUID = GUID {
        data1: 0x56FDF344,
        data2: 0xFD6D,
        data3: 0x11D0,
        data4: [0x95, 0x8A, 0x00, 0x60, 0x97, 0xC9, 0xA0, 0x90],
    };
    const IID_TASKBAR_LIST3: GUID = GUID {
        data1: 0xEA1AFB91,
        data2: 0x9E28,
        data3: 0x4B86,
        data4: [0x90, 0xE9, 0x9E, 0x9F, 0x8A, 0x5E, 0xEF, 0xAF],
    };

    type Ptr = *mut core::ffi::c_void;

    /// Hand-rolled vtable, same approach as `client/wndproc.rs::skip_taskbar`
    /// (windows-sys has no COM method wrappers). Order is ITaskbarList →
    /// ITaskbarList2 → ITaskbarList3; only the slots we call are typed.
    #[repr(C)]
    struct ITaskbarList3 {
        vtbl: *const Vtbl,
    }
    #[repr(C)]
    struct Vtbl {
        query_interface: usize,
        add_ref: usize,
        release: unsafe extern "system" fn(*mut ITaskbarList3) -> u32,
        hr_init: unsafe extern "system" fn(*mut ITaskbarList3) -> i32,
        add_tab: usize,
        delete_tab: usize,
        activate_tab: usize,
        set_active_alt: usize,
        mark_fullscreen_window: usize,
        set_progress_value: usize,
        set_progress_state: usize,
        register_tab: usize,
        unregister_tab: usize,
        set_tab_order: usize,
        set_tab_active: usize,
        thumb_bar_add_buttons: usize,
        thumb_bar_update_buttons: usize,
        thumb_bar_set_image_list: usize,
        /// `SetOverlayIcon(HWND hwnd, HICON hIcon, LPCWSTR pszDescription)`.
        set_overlay_icon: unsafe extern "system" fn(*mut ITaskbarList3, Ptr, Ptr, *const u16) -> i32,
    }

    unsafe fn apply(hwnd: isize, count: u32) {
        use windows_sys::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            FlashWindowEx, GetForegroundWindow, FLASHWINFO, FLASHW_TIMERNOFG, FLASHW_TRAY,
        };

        let prev = {
            let mut g = LAST.lock().unwrap_or_else(|e| e.into_inner());
            g.get_or_insert_with(HashMap::new).insert(hwnd, count).unwrap_or(0)
        };

        let mut tbl: *mut ITaskbarList3 = std::ptr::null_mut();
        let hr = CoCreateInstance(
            &CLSID_TASKBAR_LIST,
            std::ptr::null_mut(),
            CLSCTX_INPROC_SERVER,
            &IID_TASKBAR_LIST3,
            &mut tbl as *mut _ as *mut _,
        );
        if hr < 0 || tbl.is_null() {
            tracing::warn!("[taskbar-attention] CoCreateInstance(TaskbarList3) failed: hr=0x{:x}", hr);
        } else {
            let vt = &*(*tbl).vtbl;
            (vt.hr_init)(tbl);
            if count > 0 {
                let desc: Vec<u16> = super::description(count).encode_utf16().chain(std::iter::once(0)).collect();
                let hr = (vt.set_overlay_icon)(tbl, hwnd as Ptr, badge_icon(), desc.as_ptr());
                if hr < 0 {
                    tracing::warn!("[taskbar-attention] SetOverlayIcon failed: hr=0x{:x}", hr);
                }
            } else {
                (vt.set_overlay_icon)(tbl, hwnd as Ptr, std::ptr::null_mut(), std::ptr::null());
            }
            (vt.release)(tbl);
        }

        let foreground = GetForegroundWindow() as isize == hwnd;
        if super::should_flash(prev, count, foreground) {
            let fi = FLASHWINFO {
                cbSize: std::mem::size_of::<FLASHWINFO>() as u32,
                hwnd: hwnd as _,
                dwFlags: FLASHW_TRAY | FLASHW_TIMERNOFG,
                uCount: 0,
                dwTimeout: 0,
            };
            FlashWindowEx(&fi);
        }
    }

    /// 16×16 orange dot with a dark ring, built once. Leaked on purpose: the
    /// shell keeps using the overlay icon for as long as it is set.
    fn badge_icon() -> Ptr {
        use std::sync::OnceLock;
        static ICON: OnceLock<isize> = OnceLock::new();
        *ICON.get_or_init(|| unsafe { make_badge() as isize }) as Ptr
    }

    unsafe fn make_badge() -> Ptr {
        use windows_sys::Win32::Graphics::Gdi::{
            CreateBitmap, CreateDIBSection, DeleteObject, BITMAPINFO, BITMAPINFOHEADER, BI_RGB, DIB_RGB_COLORS,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::{CreateIconIndirect, ICONINFO};
        const N: i32 = 16;
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: N,
            biHeight: -N, // top-down
            biPlanes: 1,
            biBitCount: 32,
            biCompression: BI_RGB,
            ..std::mem::zeroed()
        };
        let mut bits: Ptr = std::ptr::null_mut();
        let color = CreateDIBSection(std::ptr::null_mut(), &bmi, DIB_RGB_COLORS, &mut bits, std::ptr::null_mut(), 0);
        if color.is_null() || bits.is_null() {
            return std::ptr::null_mut();
        }
        let px = std::slice::from_raw_parts_mut(bits as *mut u32, (N * N) as usize);
        let (c, r) = (7.5f32, 7.5f32);
        for y in 0..N {
            for x in 0..N {
                let d = ((x as f32 - c).powi(2) + (y as f32 - c).powi(2)).sqrt();
                // BGRA, premultiplied (opaque pixels only, so trivially).
                px[(y * N + x) as usize] = if d <= r - 1.2 {
                    0xFF_FF_8A_00
                } else if d <= r {
                    0xFF_1A_1A_1A
                } else {
                    0
                };
            }
        }
        let mask = CreateBitmap(N, N, 1, 1, std::ptr::null());
        let info = ICONINFO { fIcon: 1, xHotspot: 0, yHotspot: 0, hbmMask: mask, hbmColor: color };
        let icon = CreateIconIndirect(&info);
        DeleteObject(color);
        DeleteObject(mask);
        icon as Ptr
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn describes_counts() {
        assert_eq!(description(1), "1 agent needs you");
        assert_eq!(description(3), "3 agents need you");
    }

    #[test]
    fn flashes_only_on_rise_while_unfocused() {
        assert!(should_flash(0, 1, false));
        assert!(should_flash(1, 2, false));
        assert!(!should_flash(1, 1, false), "no change");
        assert!(!should_flash(2, 1, false), "went down");
        assert!(!should_flash(0, 1, true), "already looking");
    }
}
