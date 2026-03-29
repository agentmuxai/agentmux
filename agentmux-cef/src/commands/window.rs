// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Window management commands for the CEF host.
// Ported from src-tauri/src/commands/window.rs.
//
// Phase 2: Single-window only. Multi-window commands are stubbed.

use std::sync::Arc;

use cef::{ImplBrowser, ImplBrowserHost, post_task, ThreadId};

use crate::state::AppState;
use crate::ui_tasks;

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

    // Post to UI thread — host.set_zoom_level() deadlocks from IPC thread
    let mut task = ui_tasks::SetZoomLevelTask::new(state.clone(), zoom_level);
    post_task(ThreadId::UI, Some(&mut task));

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
/// Posted to UI thread — host.show_dev_tools() deadlocks from IPC thread.
pub fn toggle_devtools(state: &Arc<AppState>) -> Result<serde_json::Value, String> {
    let mut task = ui_tasks::ShowDevToolsTask::new(state.clone());
    post_task(ThreadId::UI, Some(&mut task));
    Ok(serde_json::Value::Null)
}
