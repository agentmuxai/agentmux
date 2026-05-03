// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Win32 frameless window setup + wndproc subclass. Extracted from
//! client/mod.rs in task #182 PR-G.

/// Set up a native frameless window: extend client area over the thick frame
/// border so the resize handle is invisible, then subclass the window to
/// handle WM_NCHITTEST for edge resize.
///
/// DwmExtendFrameIntoClientArea(-1) makes the entire frame transparent, but
/// it also removes the non-client hit-test region. Without the subclass,
/// Windows can't tell which part of the window edge should be a resize handle.
/// The subclass returns HT{LEFT,RIGHT,TOP,BOTTOM,TOPLEFT,...} when the cursor
/// is within RESIZE_BORDER pixels of the window edge.
#[cfg(target_os = "windows")]
pub(super) unsafe fn setup_native_frameless(hwnd: *mut std::ffi::c_void) {
    use windows_sys::Win32::Graphics::Dwm::DwmExtendFrameIntoClientArea;
    use windows_sys::Win32::UI::Controls::MARGINS;

    let margins = MARGINS {
        cxLeftWidth: -1,
        cxRightWidth: -1,
        cyTopHeight: -1,
        cyBottomHeight: -1,
    };
    let result = DwmExtendFrameIntoClientArea(hwnd, &margins);
    if result == 0 {
        tracing::info!("Applied DwmExtendFrameIntoClientArea to hide resize border");
    } else {
        tracing::warn!("DwmExtendFrameIntoClientArea failed: hr={:#x}", result);
    }
}

/// Map of HWND -> original WndProc for secondary windows with edge resize hooks.
/// Stored here instead of GWLP_USERDATA to avoid clobbering CEF's data.
#[cfg(target_os = "windows")]
static ORIGINAL_WNDPROCS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<usize, isize>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

// Pane Win32 focus-redirect subclass + ALLOW_BROWSER_PANE_FOCUS_ONCE flag moved to
// `crate::browser_pane::hwnd` in Phase 2 of the modularization split. See
// `docs/specs/SPEC_BROWSER_PANE_MODULARIZATION.md`.

/// Install a WndProc hook on a SECONDARY window that handles:
/// - WM_NCCALCSIZE: returns 0 to eliminate the non-client area (removes the
///   wide title bar / top border that WS_THICKFRAME + DWM extension creates)
/// - WM_NCHITTEST: returns HT{LEFT,RIGHT,...} for resize zones at window edges
///
/// MUST NOT be installed on the main CEF Views window — that window handles
/// resize through its delegate, and hooking it clobbers CEF internals.
#[cfg(target_os = "windows")]
pub(super) unsafe fn install_frameless_resize_hook(hwnd: *mut std::ffi::c_void) {
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    const RESIZE_BORDER: i32 = 6;

    unsafe extern "system" fn wndproc_hook(
        hwnd: *mut std::ffi::c_void,
        msg: u32,
        wparam: usize,
        lparam: isize,
    ) -> isize {
        match msg {
            // Remove the non-client area entirely — this eliminates the wide
            // top border that WS_THICKFRAME normally reserves for the title bar.
            WM_NCCALCSIZE if wparam == 1 => {
                // Returning 0 with wparam=1 tells Windows the client area
                // fills the entire window rect. No title bar, no borders.
                return 0;
            }

            // Suppress the DWM activation border — return TRUE without
            // calling DefWindowProc so Windows doesn't repaint the frame.
            WM_NCACTIVATE => {
                return 1; // TRUE = allow activation, but skip default border paint
            }

            WM_NCHITTEST => {
                let x = (lparam & 0xFFFF) as i16 as i32;
                let y = ((lparam >> 16) & 0xFFFF) as i16 as i32;

                let mut rect = std::mem::zeroed::<windows_sys::Win32::Foundation::RECT>();
                GetWindowRect(hwnd, &mut rect);

                let left = x - rect.left < RESIZE_BORDER;
                let right = rect.right - x < RESIZE_BORDER;
                let top = y - rect.top < RESIZE_BORDER;
                let bottom = rect.bottom - y < RESIZE_BORDER;

                if top && left { return HTTOPLEFT as isize; }
                if top && right { return HTTOPRIGHT as isize; }
                if bottom && left { return HTBOTTOMLEFT as isize; }
                if bottom && right { return HTBOTTOMRIGHT as isize; }
                if left { return HTLEFT as isize; }
                if right { return HTRIGHT as isize; }
                if top { return HTTOP as isize; }
                if bottom { return HTBOTTOM as isize; }
                // Not on an edge — fall through to original WndProc.
            }

            _ => {}
        }

        // Delegate to the original WndProc.
        let key = hwnd as usize;
        let original = ORIGINAL_WNDPROCS
            .lock()
            .ok()
            .and_then(|map| map.get(&key).copied())
            .unwrap_or(0);
        if original != 0 {
            CallWindowProcW(Some(std::mem::transmute(original)), hwnd, msg, wparam, lparam)
        } else {
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
    }

    let original = GetWindowLongPtrW(hwnd, GWLP_WNDPROC);
    if let Ok(mut map) = ORIGINAL_WNDPROCS.lock() {
        map.insert(hwnd as usize, original);
    }
    SetWindowLongPtrW(hwnd, GWLP_WNDPROC, wndproc_hook as isize);
    tracing::info!("Installed frameless resize hook (WM_NCCALCSIZE + WM_NCHITTEST)");
}

/// Hide the given top-level HWND from the Windows taskbar via
/// `ITaskbarList::DeleteTab`. The window remains fully usable — Alt-Tab still
/// finds it, it takes focus, repaints, etc. — but the shell paints no taskbar
/// button for it regardless of the user's "Combine taskbar buttons" setting.
///
/// Used only for `WindowKind::Subwindow` top-level windows. Must be called
/// once the HWND exists (post-`on_after_created`) and re-applied on the
/// `TaskbarCreated` broadcast after Explorer restarts.
///
/// Same primitive Electron uses in `NativeWindowViews::SetSkipTaskbar`
/// (`shell/browser/native_window_views.cc`).
#[cfg(target_os = "windows")]
pub(super) unsafe fn skip_taskbar(hwnd: *mut std::ffi::c_void) {
    use windows_sys::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
    use windows_sys::core::GUID;

    // CLSID_TaskbarList
    const CLSID_TASKBAR_LIST: GUID = GUID {
        data1: 0x56FDF344,
        data2: 0xFD6D,
        data3: 0x11D0,
        data4: [0x95, 0x8A, 0x00, 0x60, 0x97, 0xC9, 0xA0, 0x90],
    };
    // IID_ITaskbarList
    const IID_TASKBAR_LIST: GUID = GUID {
        data1: 0x56FDF342,
        data2: 0xFD6D,
        data3: 0x11D0,
        data4: [0x95, 0x8A, 0x00, 0x60, 0x97, 0xC9, 0xA0, 0x90],
    };

    // Hand-rolled vtable — `windows-sys` doesn't expose `ITaskbarList` types
    // at this feature level, and pulling in the full `windows` crate for one
    // COM interface is overkill.
    #[repr(C)]
    struct ITaskbarList {
        lp_vtbl: *const ITaskbarListVtbl,
    }
    #[repr(C)]
    struct ITaskbarListVtbl {
        query_interface: unsafe extern "system" fn(*mut ITaskbarList, *const GUID, *mut *mut core::ffi::c_void) -> i32,
        add_ref: unsafe extern "system" fn(*mut ITaskbarList) -> u32,
        release: unsafe extern "system" fn(*mut ITaskbarList) -> u32,
        hr_init: unsafe extern "system" fn(*mut ITaskbarList) -> i32,
        add_tab: unsafe extern "system" fn(*mut ITaskbarList, *mut core::ffi::c_void) -> i32,
        delete_tab: unsafe extern "system" fn(*mut ITaskbarList, *mut core::ffi::c_void) -> i32,
        activate_tab: unsafe extern "system" fn(*mut ITaskbarList, *mut core::ffi::c_void) -> i32,
        set_active_alt: unsafe extern "system" fn(*mut ITaskbarList, *mut core::ffi::c_void) -> i32,
    }

    let mut tbl: *mut ITaskbarList = std::ptr::null_mut();
    let hr = CoCreateInstance(
        &CLSID_TASKBAR_LIST as *const GUID,
        std::ptr::null_mut(),
        CLSCTX_INPROC_SERVER,
        &IID_TASKBAR_LIST as *const GUID,
        &mut tbl as *mut _ as *mut _,
    );
    if hr < 0 || tbl.is_null() {
        tracing::warn!("[skip_taskbar] CoCreateInstance(TaskbarList) failed: hr=0x{:x}", hr);
        return;
    }

    let vtbl = &*(*tbl).lp_vtbl;
    let hr = (vtbl.hr_init)(tbl);
    if hr < 0 {
        tracing::warn!("[skip_taskbar] HrInit failed: hr=0x{:x}", hr);
        (vtbl.release)(tbl);
        return;
    }
    let hr = (vtbl.delete_tab)(tbl, hwnd);
    if hr < 0 {
        tracing::warn!("[skip_taskbar] DeleteTab failed: hr=0x{:x}", hr);
    } else {
        tracing::info!("[skip_taskbar] hid HWND {:p} from taskbar", hwnd);
    }
    (vtbl.release)(tbl);
}

/// Load the app icon from the exe's embedded resource and set it on the window.
/// This makes the icon appear in the taskbar and Alt+Tab switcher instead of
/// the default CEF/Chromium icon.
#[cfg(target_os = "windows")]
pub(super) unsafe fn set_window_icon(hwnd: *mut std::ffi::c_void) {
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;

    let hinstance = GetModuleHandleW(std::ptr::null());
    if hinstance.is_null() {
        tracing::warn!("set_window_icon: GetModuleHandleW returned null");
        return;
    }

    // Load the big icon (32x32, for Alt+Tab / taskbar)
    let icon_big = LoadImageW(
        hinstance,
        1 as *const u16, // Resource ID 1 (set by winres)
        IMAGE_ICON,
        32, 32,
        LR_SHARED,
    );
    if !icon_big.is_null() {
        SendMessageW(hwnd, WM_SETICON, ICON_BIG as usize, icon_big as isize);
    }

    // Load the small icon (16x16, for title bar)
    let icon_small = LoadImageW(
        hinstance,
        1 as *const u16,
        IMAGE_ICON,
        16, 16,
        LR_SHARED,
    );
    if !icon_small.is_null() {
        SendMessageW(hwnd, WM_SETICON, ICON_SMALL as usize, icon_small as isize);
    }

    if !icon_big.is_null() || !icon_small.is_null() {
        tracing::info!("Set window icon from embedded resource");
    } else {
        tracing::warn!("set_window_icon: no icon found in exe resource");
    }
}
