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

/// Map of top-level HWND -> original WndProc for the focus-restore subclass
/// installed by `install_top_level_focus_restore_hook`. Kept separate from
/// `ORIGINAL_WNDPROCS` so the two hooks can coexist on the same HWND in
/// either order (the focus-restore hook always passes through to its own
/// recorded original, which transitively walks back through any other hook).
#[cfg(target_os = "windows")]
static FOCUS_RESTORE_WNDPROCS: std::sync::LazyLock<
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

/// Subclass a top-level window's WndProc to handle `WM_ACTIVATE`: when the
/// window is being activated (`wparam != WA_INACTIVE`), look up the last
/// intentionally-focused child for *this* root in
/// `LAST_FOCUSED_BY_ROOT` and `SetFocus` it. Closes the
/// alt-tab-back-and-input-drops bug.
///
/// Spec: `docs/specs/SPEC_WINDOW_REACTIVATE_FOCUS_RESTORE_2026_05_23.md`
/// §5.1.3.
///
/// SAFE on the main CEF Views window: this hook ONLY observes `WM_ACTIVATE`
/// and ALWAYS passes the message through to the original WndProc. No
/// message is short-circuited. That is the crucial difference from
/// `install_frameless_resize_hook`, which returns early for
/// `WM_NCCALCSIZE` / `WM_NCACTIVATE` and so MUST NOT be installed on main.
///
/// Idempotent: re-calling on an already-hooked HWND is a no-op.
#[cfg(target_os = "windows")]
pub(crate) unsafe fn install_top_level_focus_restore_hook(hwnd: *mut std::ffi::c_void) {
    use std::sync::atomic::Ordering;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallWindowProcW, DefWindowProcW, IsWindow, SetWindowLongPtrW, GWLP_WNDPROC, WM_ACTIVATE,
    };

    const WA_INACTIVE: u32 = 0;

    unsafe extern "system" fn wndproc_hook(
        hwnd: *mut std::ffi::c_void,
        msg: u32,
        wparam: usize,
        lparam: isize,
    ) -> isize {
        if msg == WM_ACTIVATE {
            // The low word of wParam is the activation state; the high word
            // is the minimized-state flag, which we don't care about.
            let activation_state = (wparam & 0xFFFF) as u32;
            if activation_state != WA_INACTIVE {
                // The activating window is `hwnd`, which IS its own
                // top-level root — the map is keyed by root HWND.
                let child = crate::browser_pane::hwnd::LAST_FOCUSED_BY_ROOT
                    .lock()
                    .ok()
                    .and_then(|m| m.get(&(hwnd as usize)).copied())
                    .unwrap_or(0);
                if child != 0 {
                    let child_hwnd = child as *mut std::ffi::c_void;
                    if IsWindow(child_hwnd) != 0 {
                        // Honor the next pane-WM_SETFOCUS instead of redirecting.
                        crate::browser_pane::hwnd::ALLOW_BROWSER_PANE_FOCUS_ONCE
                            .store(true, Ordering::Relaxed);
                        SetFocus(child_hwnd);
                        tracing::info!(
                            "[focus-restore] WM_ACTIVATE root={:p} state={} -> SetFocus child={:p}",
                            hwnd, activation_state, child_hwnd,
                        );
                    } else {
                        tracing::info!(
                            "[focus-restore] WM_ACTIVATE root={:p} stale child={:p} (IsWindow=0) — no-op",
                            hwnd, child_hwnd,
                        );
                    }
                } else {
                    tracing::info!(
                        "[focus-restore] WM_ACTIVATE root={:p} no recorded child — no-op",
                        hwnd,
                    );
                }
            }
        }

        // ALWAYS pass through. We observe WM_ACTIVATE; CEF still owns it.
        let original = FOCUS_RESTORE_WNDPROCS
            .lock()
            .ok()
            .and_then(|m| m.get(&(hwnd as usize)).copied())
            .unwrap_or(0);
        if original != 0 {
            CallWindowProcW(Some(std::mem::transmute(original)), hwnd, msg, wparam, lparam)
        } else {
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
    }

    let already_hooked = FOCUS_RESTORE_WNDPROCS
        .lock()
        .ok()
        .map(|m| m.contains_key(&(hwnd as usize)))
        .unwrap_or(false);
    if already_hooked {
        return;
    }
    let original = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, wndproc_hook as *const () as isize);
    if original != 0 {
        if let Ok(mut map) = FOCUS_RESTORE_WNDPROCS.lock() {
            map.insert(hwnd as usize, original);
        }
        tracing::info!(
            "[focus-restore] installed WM_ACTIVATE observer on top-level HWND {:p}",
            hwnd,
        );
    } else {
        tracing::warn!(
            "[focus-restore] SetWindowLongPtrW returned 0 for HWND {:p} — hook not installed",
            hwnd,
        );
    }
}

/// Original WndProc table for the floater cascade hook.
/// Module-level so `cascade_hook` (an `extern "system" fn`) can access it
/// — inner-function statics are not in scope for nested `extern fn` items.
#[cfg(target_os = "windows")]
static FLOATER_CASCADE_ORIGINALS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<usize, isize>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// Cascade hook body — module-level so it can reference module-level statics.
#[cfg(target_os = "windows")]
unsafe extern "system" fn floater_cascade_wndproc(
    hwnd: *mut std::ffi::c_void,
    msg: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallWindowProcW, DefWindowProcW, WM_DESTROY,
    };

    // FLOATER INDEPENDENCE (supersedes the issue #1560 cascade): a floating
    // pane is now a fully independent top-level window. Closing, minimizing,
    // restoring, or activating a FullInstance window must NOT touch its
    // floaters — the OS already z-orders unowned top-level windows on its own,
    // and a floater torn from one window can be redocked into another, so no
    // window "owns" a floater's lifecycle. The previous WM_ACTIVATE z-order /
    // WM_SIZE hide-show / WM_DESTROY close-cascade handlers are removed.
    //
    // Two bugs they caused: (1) WM_DESTROY cascade-closed any floater whose
    // parent matched OR was parent==0, so closing one window killed unrelated
    // floaters; (2) that surprise close destroyed the block under an in-flight
    // tear-off, which then jammed the drag session and broke tear-off entirely.
    //
    // The subclass itself is kept ONLY as a pure observer-passthrough that
    // self-prunes FLOATER_CASCADE_ORIGINALS on WM_DESTROY (HWND-reuse safety).

    // Always pass through — we observe only, never short-circuit.
    let original = FLOATER_CASCADE_ORIGINALS
        .lock()
        .ok()
        .and_then(|m| m.get(&(hwnd as usize)).copied())
        .unwrap_or(0);
    let result = if original != 0 {
        CallWindowProcW(Some(std::mem::transmute(original)), hwnd, msg, wparam, lparam)
    } else {
        DefWindowProcW(hwnd, msg, wparam, lparam)
    };

    // Prune the map AFTER passthrough so the original proc receives WM_DESTROY.
    // Without this, Windows HWND reuse could make install_main_window_floater_cascade_hook
    // see already_hooked=true for a new window that happens to get the same HWND value.
    if msg == WM_DESTROY {
        if let Ok(mut m) = FLOATER_CASCADE_ORIGINALS.lock() {
            m.remove(&(hwnd as usize));
        }
    }

    result
}

/// Subclass a FullInstance window's WndProc to cascade lifecycle events to all
/// active floating panes (issue #1560 — replaces the owned-window cascade that
/// was removed when we switched to unowned `WS_POPUP` windows).
///
/// Handles:
/// - `WM_ACTIVATE(active)` → z-order floaters below the main HWND
/// - `WM_SIZE(SIZE_MINIMIZED)` → `SW_HIDE` every floater
/// - `WM_SIZE(restored/maximized)` → `SW_SHOWNOACTIVATE` every floater
/// - `WM_DESTROY` → `PostMessage(WM_CLOSE)` every floater
///
/// Idempotent: re-calling on an already-hooked HWND is a no-op.
#[cfg(target_os = "windows")]
pub(crate) unsafe fn install_main_window_floater_cascade_hook(
    hwnd: *mut std::ffi::c_void,
) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{SetWindowLongPtrW, GWLP_WNDPROC};

    let already_hooked = FLOATER_CASCADE_ORIGINALS
        .lock()
        .ok()
        .map(|m| m.contains_key(&(hwnd as usize)))
        .unwrap_or(false);
    if already_hooked {
        return;
    }

    let original = SetWindowLongPtrW(
        hwnd,
        GWLP_WNDPROC,
        floater_cascade_wndproc as *const () as isize,
    );
    if original != 0 {
        if let Ok(mut m) = FLOATER_CASCADE_ORIGINALS.lock() {
            m.insert(hwnd as usize, original);
        }
        tracing::info!(
            "[floater-cascade] installed cascade hook on FullInstance HWND {:p}",
            hwnd,
        );
    } else {
        tracing::warn!(
            "[floater-cascade] SetWindowLongPtrW returned 0 for {:p} — cascade not installed",
            hwnd,
        );
    }
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
    use windows_sys::Win32::System::LibraryLoader::{
        GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
        GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    };

    // Resolve the module that CONTAINS this code, NOT the process exe. Under the
    // Phase 3 Windows sandbox (#1633) the host process is CEF's bootstrap.exe —
    // which has no AgentMux icon resource — while the real host logic + the
    // winres-embedded icon live in agentmux_cef.dll. The old
    // `GetModuleHandleW(null)` returned bootstrap.exe → no resource ID 1 → the
    // window/taskbar fell back to Chrome's icon. `FROM_ADDRESS` on a function in
    // this module returns the DLL (sandbox build) or the exe (non-sandbox build)
    // — whichever actually carries the embedded resource.
    let mut hinstance: windows_sys::Win32::Foundation::HMODULE = std::ptr::null_mut();
    let got = GetModuleHandleExW(
        GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
        set_window_icon as *const () as *const u16,
        &mut hinstance,
    );
    if got == 0 || hinstance.is_null() {
        tracing::warn!("set_window_icon: GetModuleHandleExW(FROM_ADDRESS) failed");
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
