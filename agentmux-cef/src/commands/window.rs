// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Window management commands for the CEF host.
// Ported from src-tauri/src/commands/window.rs.
//
// Phase 2: Single-window only. Multi-window commands are stubbed.

use std::sync::Arc;

use cef::{ImplBrowser, ImplBrowserHost};

use crate::state::AppState;

/// Get the current zoom factor.
pub fn get_zoom_factor(state: &Arc<AppState>) -> serde_json::Value {
    let factor = *state.zoom_factor.lock().unwrap();
    serde_json::json!(factor)
}

/// Set the zoom factor.
/// CEF zoom uses a logarithmic scale: zoom_level = log2(zoom_factor)
/// So factor 1.0 = level 0, factor 2.0 = level 1, factor 0.5 = level -1
pub fn set_zoom_factor(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let factor = args
        .get("factor")
        .and_then(|v| v.as_f64())
        .ok_or_else(|| "Missing factor".to_string())?;

    let factor = factor.clamp(0.5, 3.0);
    *state.zoom_factor.lock().unwrap() = factor;

    // Convert to CEF zoom level (log base 1.2)
    // CEF uses: zoom_factor = 1.2 ^ zoom_level
    // So: zoom_level = log(zoom_factor) / log(1.2)
    let zoom_level = factor.ln() / 1.2_f64.ln();

    // NOTE: host.set_zoom_level() deadlocks from IPC thread, and post_task
    // crashes with current CEF bindings. Zoom is applied via CSS on the frontend.
    // The zoom_factor state is stored for get_zoom_factor queries.

    // Emit zoom-factor-change event
    crate::events::emit_event_from_state(state, "zoom-factor-change", &serde_json::json!(factor));

    Ok(serde_json::Value::Null)
}

/// Close the window.
pub fn close_window(state: &Arc<AppState>) -> Result<serde_json::Value, String> {
    let browser = state.browser.lock().unwrap();
    if let Some(ref browser) = *browser {
        if let Some(host) = browser.host() {
            host.try_close_browser();
        }
    }
    Ok(serde_json::Value::Null)
}

/// Minimize the window.
/// Note: CEF Views Window.minimize() would be needed here.
/// For now, use platform-specific APIs.
pub fn minimize_window(_state: &Arc<AppState>) -> Result<serde_json::Value, String> {
    #[cfg(target_os = "windows")]
    {
        // Find our process's main window and minimize it
        let browser = _state.browser.lock().unwrap();
        if let Some(ref browser) = *browser {
            if let Some(host) = browser.host() {
                let hwnd = host.window_handle();
                if !hwnd.0.is_null() {
                    unsafe {
                        windows_sys::Win32::UI::WindowsAndMessaging::ShowWindow(
                            hwnd.0 as *mut std::ffi::c_void,
                            windows_sys::Win32::UI::WindowsAndMessaging::SW_MINIMIZE,
                        );
                    }
                }
            }
        }
    }
    Ok(serde_json::Value::Null)
}

/// Maximize/unmaximize the window (toggle).
pub fn maximize_window(_state: &Arc<AppState>) -> Result<serde_json::Value, String> {
    #[cfg(target_os = "windows")]
    {
        let browser = _state.browser.lock().unwrap();
        if let Some(ref browser) = *browser {
            if let Some(host) = browser.host() {
                let hwnd = host.window_handle();
                if !hwnd.0.is_null() {
                    unsafe {
                        use windows_sys::Win32::UI::WindowsAndMessaging::*;
                        let hwnd_ptr = hwnd.0 as *mut std::ffi::c_void;
                        let mut placement: WINDOWPLACEMENT = std::mem::zeroed();
                        placement.length = std::mem::size_of::<WINDOWPLACEMENT>() as u32;
                        GetWindowPlacement(hwnd_ptr, &mut placement);
                        if placement.showCmd == SW_MAXIMIZE as u32 {
                            ShowWindow(hwnd_ptr, SW_RESTORE);
                        } else {
                            ShowWindow(hwnd_ptr, SW_MAXIMIZE);
                        }
                    }
                }
            }
        }
    }
    Ok(serde_json::Value::Null)
}

/// Set window transparency/blur effects.
/// Uses DWM Mica/Acrylic on Win11, or SetWindowCompositionAttribute on Win10.
pub fn set_window_transparency(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let transparent = args.get("transparent").and_then(|v| v.as_bool()).unwrap_or(false);
    let blur = args.get("blur").and_then(|v| v.as_bool()).unwrap_or(false);
    let _opacity = args.get("opacity").and_then(|v| v.as_f64()).unwrap_or(0.8);
    tracing::info!("set_window_transparency: transparent={} blur={}", transparent, blur);
    #[cfg(not(target_os = "windows"))]
    tracing::info!("set_window_transparency: not windows, skipping");

    #[cfg(target_os = "windows")]
    {
        unsafe {
            let hwnd = find_own_top_level_window();
            if !hwnd.is_null() {
                tracing::info!("set_window_transparency: found hwnd={:?}", hwnd);
                if blur {
                    apply_window_effects(hwnd, true, true);
                }
                if transparent {
                    apply_window_opacity(hwnd, _opacity);
                }
            } else {
                tracing::warn!("set_window_transparency: could not find top-level window");
            }
        }
    }
    let _ = (state, transparent, blur);
    Ok(serde_json::Value::Null)
}

/// Find the top-level window belonging to this process.
/// In CEF Views mode, browser.host().window_handle() returns NULL,
/// so we enumerate windows and find ours by process ID.
#[cfg(target_os = "windows")]
unsafe fn find_own_top_level_window() -> *mut std::ffi::c_void {
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;

    let pid = GetCurrentProcessId();
    let mut result: *mut std::ffi::c_void = std::ptr::null_mut();

    unsafe extern "system" fn enum_callback(
        hwnd: *mut std::ffi::c_void,
        lparam: isize,
    ) -> i32 {
        use windows_sys::Win32::System::Threading::GetCurrentProcessId;
        let mut window_pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut window_pid);
        if window_pid == GetCurrentProcessId() && IsWindowVisible(hwnd) != 0 {
            // Store the HWND in the pointer passed via lparam
            let result_ptr = lparam as *mut *mut std::ffi::c_void;
            *result_ptr = hwnd;
            return 0; // Stop enumeration
        }
        1 // Continue
    }

    let _ = pid; // Used inside callback via GetCurrentProcessId()
    EnumWindows(
        Some(enum_callback),
        &mut result as *mut _ as isize,
    );
    result
}

#[cfg(target_os = "windows")]
unsafe fn apply_window_effects(hwnd: *mut std::ffi::c_void, transparent: bool, blur: bool) {
    use windows_sys::Win32::Graphics::Dwm::*;

    if !transparent && !blur {
        // Disable: set backdrop to NONE
        let backdrop_type: i32 = 1; // DWMSBT_NONE
        DwmSetWindowAttribute(
            hwnd,
            38, // DWMWA_SYSTEMBACKDROP_TYPE
            &backdrop_type as *const _ as *const std::ffi::c_void,
            std::mem::size_of::<i32>() as u32,
        );
        return;
    }

    // Try Win11 DWM backdrop first (Mica or Acrylic)
    // DWMWA_SYSTEMBACKDROP_TYPE = 38
    // DWMSBT_MAINWINDOW (Mica) = 2, DWMSBT_TRANSIENTWINDOW (Acrylic) = 3, DWMSBT_TABBEDWINDOW = 4
    let backdrop_type: i32 = if blur { 3 } else { 2 }; // Acrylic for blur, Mica otherwise

    // Enable immersive dark mode first (required for Mica/Acrylic to look correct on dark themes)
    let dark_mode: i32 = 1;
    DwmSetWindowAttribute(
        hwnd,
        20, // DWMWA_USE_IMMERSIVE_DARK_MODE
        &dark_mode as *const _ as *const std::ffi::c_void,
        std::mem::size_of::<i32>() as u32,
    );

    let result = DwmSetWindowAttribute(
        hwnd,
        38, // DWMWA_SYSTEMBACKDROP_TYPE
        &backdrop_type as *const _ as *const std::ffi::c_void,
        std::mem::size_of::<i32>() as u32,
    );

    if result != 0 {
        // Win11 API failed (probably Win10) — try SetWindowCompositionAttribute
        tracing::debug!("DWM backdrop failed (hr={:#x}), trying Win10 acrylic", result);
        apply_win10_acrylic(hwnd, transparent);
    } else {
        tracing::info!("Applied DWM backdrop type {} to window", backdrop_type);
    }
}

/// Apply window-level opacity via WS_EX_LAYERED + SetLayeredWindowAttributes.
/// This makes the entire window semi-transparent (content + chrome).
#[cfg(target_os = "windows")]
unsafe fn apply_window_opacity(hwnd: *mut std::ffi::c_void, opacity: f64) {
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    let alpha = (opacity.clamp(0.0, 1.0) * 255.0) as u8;

    // Add WS_EX_LAYERED extended style
    let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
    SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style | WS_EX_LAYERED as isize);

    // LWA_ALPHA = 0x02
    let result = SetLayeredWindowAttributes(hwnd, 0, alpha, 0x02);
    if result != 0 {
        tracing::info!("Applied window opacity: {} (alpha={})", opacity, alpha);
    } else {
        tracing::warn!("SetLayeredWindowAttributes failed");
    }
}

/// Win10 acrylic blur via undocumented SetWindowCompositionAttribute API.
#[cfg(target_os = "windows")]
unsafe fn apply_win10_acrylic(hwnd: *mut std::ffi::c_void, enable: bool) {
    #[repr(C)]
    struct AccentPolicy {
        accent_state: u32,
        accent_flags: u32,
        gradient_color: u32,
        animation_id: u32,
    }

    #[repr(C)]
    struct WindowCompositionAttribData {
        attrib: u32, // WCA_ACCENT_POLICY = 19
        data: *mut std::ffi::c_void,
        size: usize,
    }

    // ACCENT_ENABLE_ACRYLICBLURBEHIND = 4, ACCENT_DISABLED = 0
    let mut policy = AccentPolicy {
        accent_state: if enable { 4 } else { 0 },
        accent_flags: 2, // ACCENT_FLAG_DRAW_ALL
        gradient_color: 0x01000000, // Nearly transparent black
        animation_id: 0,
    };

    let mut data = WindowCompositionAttribData {
        attrib: 19, // WCA_ACCENT_POLICY
        data: &mut policy as *mut _ as *mut std::ffi::c_void,
        size: std::mem::size_of::<AccentPolicy>(),
    };

    let user32 = windows_sys::Win32::System::LibraryLoader::LoadLibraryA(b"user32.dll\0".as_ptr());
    if !user32.is_null() {
        let proc = windows_sys::Win32::System::LibraryLoader::GetProcAddress(
            user32,
            b"SetWindowCompositionAttribute\0".as_ptr(),
        );
        if let Some(func) = proc {
            let func: extern "system" fn(*mut std::ffi::c_void, *mut WindowCompositionAttribData) -> i32 =
                std::mem::transmute(func);
            func(hwnd, &mut data);
            tracing::info!("Applied Win10 acrylic blur to window");
        }
    }
}

/// Get the current window label.
/// In Phase 2 (single window), always returns "main".
pub fn get_window_label() -> serde_json::Value {
    serde_json::json!("main")
}

/// Check if this is the main window.
/// In Phase 2 (single window), always true.
pub fn is_main_window() -> serde_json::Value {
    serde_json::json!(true)
}

/// Get the instance number for the current window.
pub fn get_instance_number(state: &Arc<AppState>) -> serde_json::Value {
    let reg = state.window_instance_registry.lock().unwrap();
    serde_json::json!(reg.get("main").unwrap_or(1))
}

/// Get the total window count.
pub fn get_window_count(state: &Arc<AppState>) -> serde_json::Value {
    let reg = state.window_instance_registry.lock().unwrap();
    serde_json::json!(reg.count())
}

/// Toggle devtools.
/// Returns the remote debugging URL — the frontend opens it in a new browser tab.
/// Direct host.show_dev_tools() calls crash with current CEF bindings.
pub fn toggle_devtools(_state: &Arc<AppState>) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({ "remote_debug_url": "http://localhost:9222" }))
}
