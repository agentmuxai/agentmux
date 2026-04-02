// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Cross-window drag-and-drop commands for the CEF host.
// Ported from src-tauri/src/commands/drag.rs.
//
// These commands coordinate drag sessions that span multiple windows.
// The source window escalates a local pragmatic-dnd drag to a cross-window
// drag when the cursor leaves the window. Position updates are broadcast
// to all windows via CEF execute_javascript events.

use std::sync::Arc;

use cef::{ImplBrowser, ImplBrowserHost};

use crate::events;
use crate::state::{AppState, DragPayload, DragSession, DragType};

/// Start a cross-window drag session.
pub fn start_cross_drag(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let drag_type: DragType = serde_json::from_value(
        args.get("dragType").cloned().unwrap_or_default()
    ).map_err(|e| format!("Invalid dragType: {}", e))?;
    let source_window = args.get("sourceWindow").and_then(|v| v.as_str()).unwrap_or("main").to_string();
    let source_workspace_id = args.get("sourceWorkspaceId").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let source_tab_id = args.get("sourceTabId").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let payload: DragPayload = serde_json::from_value(
        args.get("payload").cloned().unwrap_or_default()
    ).unwrap_or(DragPayload { block_id: None, tab_id: None });

    let drag_id = uuid::Uuid::new_v4().to_string();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    tracing::info!(drag_id = %drag_id, drag_type = ?drag_type, source_window = %source_window, "[dnd:cef] start_cross_drag");

    let session = DragSession {
        drag_id: drag_id.clone(),
        drag_type,
        source_window,
        source_workspace_id,
        source_tab_id,
        payload,
        started_at: now,
    };

    *state.active_drag.lock().unwrap() = Some(session.clone());
    events::emit_event_all_windows(state, "cross-drag-start", &serde_json::to_value(&session).unwrap());

    Ok(serde_json::json!(drag_id))
}

/// Update cross-window drag with current cursor position.
pub fn update_cross_drag(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let drag_id = args.get("dragId").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let screen_x = args.get("screenX").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let screen_y = args.get("screenY").and_then(|v| v.as_f64()).unwrap_or(0.0);

    let session = {
        let guard = state.active_drag.lock().unwrap();
        match guard.as_ref() {
            Some(s) if s.drag_id == drag_id => s.clone(),
            _ => return Err("no active drag session or drag_id mismatch".to_string()),
        }
    };

    let target_window = hit_test_windows(state, screen_x, screen_y);

    events::emit_event_all_windows(state, "cross-drag-update", &serde_json::json!({
        "dragId": drag_id,
        "dragType": session.drag_type,
        "payload": session.payload,
        "targetWindow": target_window,
        "sourceWindow": session.source_window,
        "screenX": screen_x,
        "screenY": screen_y,
    }));

    Ok(serde_json::json!(target_window))
}

/// Complete a cross-window drag by committing the drop.
pub fn complete_cross_drag(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let drag_id = args.get("dragId").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let target_window = args.get("targetWindow").and_then(|v| v.as_str()).map(|s| s.to_string());
    let screen_x = args.get("screenX").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let screen_y = args.get("screenY").and_then(|v| v.as_f64()).unwrap_or(0.0);

    let session = {
        let mut guard = state.active_drag.lock().unwrap();
        match guard.take() {
            Some(s) if s.drag_id == drag_id => s,
            Some(s) => { *guard = Some(s); return Err("drag_id mismatch".to_string()); }
            None => return Err("no active drag session".to_string()),
        }
    };

    let result = if target_window.is_some() { "drop" } else { "tearoff" };
    tracing::info!(drag_id = %drag_id, result = %result, "[dnd:cef] complete_cross_drag");

    events::emit_event_all_windows(state, "cross-drag-end", &serde_json::json!({
        "dragId": drag_id,
        "result": result,
        "targetWindow": target_window,
        "screenX": screen_x,
        "screenY": screen_y,
        "payload": session.payload,
        "dragType": session.drag_type,
        "sourceWindow": session.source_window,
        "sourceWorkspaceId": session.source_workspace_id,
        "sourceTabId": session.source_tab_id,
    }));

    Ok(serde_json::Value::Null)
}

/// Cancel an active cross-window drag session.
pub fn cancel_cross_drag(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let drag_id = args.get("dragId").and_then(|v| v.as_str()).unwrap_or("").to_string();

    {
        let mut guard = state.active_drag.lock().unwrap();
        match guard.as_ref() {
            Some(s) if s.drag_id == drag_id => { *guard = None; }
            _ => return Err("no active drag session or drag_id mismatch".to_string()),
        }
    }

    events::emit_event_all_windows(state, "cross-drag-end", &serde_json::json!({
        "dragId": drag_id,
        "result": "cancel",
    }));

    tracing::info!(drag_id = %drag_id, "[dnd:cef] cancel_cross_drag");
    Ok(serde_json::Value::Null)
}

/// Hit-test all open browser windows to find which one contains the cursor.
#[cfg(target_os = "windows")]
fn hit_test_windows(state: &Arc<AppState>, screen_x: f64, screen_y: f64) -> Option<String> {
    use cef::ImplBrowserHost;
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect;

    let browsers = state.browsers.lock().unwrap();
    for (label, browser) in browsers.iter() {
        if let Some(host) = browser.host() {
            let hwnd = host.window_handle();
            if hwnd.0.is_null() { continue; }
            unsafe {
                let mut rect: RECT = std::mem::zeroed();
                GetWindowRect(hwnd.0 as *mut std::ffi::c_void, &mut rect);
                let x = rect.left as f64;
                let y = rect.top as f64;
                let w = (rect.right - rect.left) as f64;
                let h = (rect.bottom - rect.top) as f64;
                if screen_x >= x && screen_x <= x + w && screen_y >= y && screen_y <= y + h {
                    return Some(label.clone());
                }
            }
        }
    }
    None
}

#[cfg(not(target_os = "windows"))]
fn hit_test_windows(_state: &Arc<AppState>, _screen_x: f64, _screen_y: f64) -> Option<String> {
    None
}

/// Get the current cursor position on screen.
pub fn get_cursor_point() -> Result<serde_json::Value, String> {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::Foundation::POINT;
        use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
        unsafe {
            let mut pt: POINT = std::mem::zeroed();
            GetCursorPos(&mut pt);
            return Ok(serde_json::json!({ "x": pt.x, "y": pt.y }));
        }
    }
    #[allow(unreachable_code)]
    Ok(serde_json::json!({ "x": 0, "y": 0 }))
}

/// Check whether the primary mouse button is currently pressed.
pub fn get_mouse_button_state() -> Result<serde_json::Value, String> {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
        let state = unsafe { GetAsyncKeyState(0x01) }; // VK_LBUTTON
        return Ok(serde_json::json!((state as u16 & 0x8000) != 0));
    }
    #[allow(unreachable_code)]
    Ok(serde_json::json!(false))
}

/// Replace the system no-drop cursor with a crosshair during drag.
pub fn set_drag_cursor() -> Result<serde_json::Value, String> {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CopyIcon, LoadCursorW, SetSystemCursor, IDC_CROSS, OCR_NO,
        };
        unsafe {
            let cross = LoadCursorW(std::ptr::null_mut(), IDC_CROSS);
            if !cross.is_null() {
                let copy = CopyIcon(cross);
                if !copy.is_null() {
                    SetSystemCursor(copy, OCR_NO);
                }
            }
        }
    }
    Ok(serde_json::Value::Null)
}

/// Restore all system cursors to defaults.
pub fn restore_drag_cursor() -> Result<serde_json::Value, String> {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_SETCURSORS};
        unsafe {
            SystemParametersInfoW(SPI_SETCURSORS, 0, std::ptr::null_mut(), 0);
        }
    }
    Ok(serde_json::Value::Null)
}

/// Release mouse capture after an HTML5 drag ends outside the window.
pub fn release_drag_capture(state: &Arc<AppState>) -> Result<serde_json::Value, String> {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            EnumChildWindows, PostMessageW, WM_CANCELMODE,
        };
        use windows_sys::Win32::Foundation::{BOOL, LPARAM};

        // Use the main browser's HWND, or find_own_top_level_window as fallback
        let hwnd = {
            let browsers = state.browsers.lock().unwrap();
            browsers.get("main")
                .and_then(|b| b.host())
                .map(|h| h.window_handle().0 as *mut std::ffi::c_void)
                .unwrap_or_else(|| unsafe { super::window::find_own_top_level_window() })
        };

        if !hwnd.is_null() {
            unsafe {
                ReleaseCapture();
                PostMessageW(hwnd, WM_CANCELMODE, 0, 0);
                unsafe extern "system" fn cancel_child(child: *mut std::ffi::c_void, _: LPARAM) -> BOOL {
                    PostMessageW(child, WM_CANCELMODE, 0, 0);
                    1
                }
                EnumChildWindows(hwnd, Some(cancel_child), 0);
            }
        }
    }
    let _ = state;
    Ok(serde_json::Value::Null)
}

/// Open a new window at a specific screen position (tear-off).
/// Creates a new CEF browser window positioned so the cursor lands in the title bar.
pub fn open_window_at_position(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let screen_x = args.get("screenX").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let screen_y = args.get("screenY").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let workspace_id = args.get("workspaceId").and_then(|v| v.as_str()).unwrap_or("").to_string();

    let window_id = uuid::Uuid::new_v4();
    let label = format!("window-{}", window_id.simple());

    let win_w = 1200i32;
    let win_h = 800i32;

    // Position so cursor lands near top-center of title bar
    let pos_x = ((screen_x - win_w as f64 / 2.0).max(0.0)) as i32;
    let pos_y = ((screen_y - 16.0).max(0.0)) as i32;

    tracing::info!(
        label = %label, pos_x = %pos_x, pos_y = %pos_y,
        workspace_id = %workspace_id,
        "[dnd:cef] open_window_at_position"
    );

    // Build URL with IPC credentials and tear-off params
    let ipc_port = *state.ipc_port.lock().unwrap();
    let ipc_token = &state.ipc_token;
    let base_url = super::window::resolve_frontend_base_url(ipc_port);
    let separator = if base_url.contains('?') { "&" } else { "?" };
    let mut url = format!(
        "{}{}ipc_port={}&ipc_token={}&windowLabel={}",
        base_url, separator, ipc_port, ipc_token, label
    );
    if !workspace_id.is_empty() {
        url.push_str(&format!("&workspaceId={}", workspace_id));
    }

    // Post to CEF UI thread — window_create_top_level must run there.
    crate::ui_tasks::post_create_window(
        state, &url, &label, pos_x, pos_y, win_w, win_h,
    );

    // Register instance number
    {
        let mut reg = state.window_instance_registry.lock().unwrap();
        let num = reg.register(&label);
        tracing::info!(label = %label, instance = %num, "[dnd:cef] tear-off window registered");
    }

    // Notify all windows
    let count = state.window_instance_registry.lock().unwrap().count();
    events::emit_event_all_windows(state, "window-instances-changed", &serde_json::json!(count));

    Ok(serde_json::json!(label))
}

/// Signal that a JS-level drag is starting or ending (Linux GTK guard).
pub fn set_js_drag_active(_args: &serde_json::Value) -> Result<serde_json::Value, String> {
    // No-op on Windows/macOS. Linux would need an atomic flag.
    Ok(serde_json::Value::Null)
}
