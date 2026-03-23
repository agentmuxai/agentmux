// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Cross-window drag-and-drop Tauri commands.
//!
//! These commands coordinate drag sessions that span multiple windows.
//! The source window escalates a local react-dnd drag to a cross-window
//! drag when the cursor leaves the window. Position updates are broadcast
//! to all windows via Tauri events so target windows can show drop overlays.

use tauri::{Emitter, Manager};

use crate::state::{AppState, DragPayload, DragSession, DragType};

/// Start a cross-window drag session.
/// Called by the source window when a drag leaves the window.
/// Returns the unique drag ID.
#[tauri::command]
pub async fn start_cross_drag(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    drag_type: DragType,
    source_window: String,
    source_workspace_id: String,
    source_tab_id: String,
    payload: DragPayload,
) -> Result<String, String> {
    let drag_id = uuid::Uuid::new_v4().to_string();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    tracing::info!(
        drag_id = %drag_id,
        drag_type = ?drag_type,
        source_window = %source_window,
        source_ws = %source_workspace_id,
        source_tab = %source_tab_id,
        payload = ?payload,
        "[dnd:tauri] start_cross_drag"
    );

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

    // Notify all windows that a cross-window drag has started
    let _ = app.emit("cross-drag-start", &session);

    Ok(drag_id)
}

/// Update cross-window drag with current cursor position.
/// Performs window hit-testing and broadcasts the result to all windows.
/// Returns the label of the window under the cursor, or None for tear-off.
#[tauri::command]
pub async fn update_cross_drag(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    drag_id: String,
    screen_x: f64,
    screen_y: f64,
) -> Result<Option<String>, String> {
    let session = {
        let guard = state.active_drag.lock().unwrap();
        match guard.as_ref() {
            Some(s) if s.drag_id == drag_id => s.clone(),
            Some(s) => {
                tracing::warn!(
                    expected = %drag_id,
                    actual = %s.drag_id,
                    "[dnd:tauri] update_cross_drag: drag_id mismatch"
                );
                return Err("drag_id mismatch".to_string());
            }
            None => {
                tracing::warn!(drag_id = %drag_id, "[dnd:tauri] update_cross_drag: no active session");
                return Err("no active drag session".to_string());
            }
        }
    };

    // Hit-test all windows to find which one the cursor is over
    let target_window = hit_test_windows(&app, screen_x, screen_y);

    tracing::info!(
        drag_id = %drag_id,
        screen_x = %screen_x,
        screen_y = %screen_y,
        target_window = ?target_window,
        source_window = %session.source_window,
        "[dnd:tauri] update_cross_drag hit-test"
    );

    // Broadcast position update to all windows
    let _ = app.emit(
        "cross-drag-update",
        serde_json::json!({
            "dragId": drag_id,
            "dragType": session.drag_type,
            "payload": session.payload,
            "targetWindow": target_window,
            "sourceWindow": session.source_window,
            "screenX": screen_x,
            "screenY": screen_y,
        }),
    );

    Ok(target_window)
}

/// Complete a cross-window drag by committing the drop.
/// If `target_window` is Some, the drop happened on a specific window.
/// If None, the drop happened outside all windows (tear-off).
#[tauri::command]
pub async fn complete_cross_drag(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    drag_id: String,
    target_window: Option<String>,
    screen_x: f64,
    screen_y: f64,
) -> Result<(), String> {
    let session = {
        let mut guard = state.active_drag.lock().unwrap();
        match guard.take() {
            Some(s) if s.drag_id == drag_id => s,
            Some(s) => {
                tracing::warn!(
                    expected = %drag_id,
                    actual = %s.drag_id,
                    "[dnd:tauri] complete_cross_drag: drag_id mismatch"
                );
                // Put it back if ID doesn't match
                *guard = Some(s);
                return Err("drag_id mismatch".to_string());
            }
            None => {
                tracing::warn!(drag_id = %drag_id, "[dnd:tauri] complete_cross_drag: no active session");
                return Err("no active drag session".to_string());
            }
        }
    };

    let result = if target_window.is_some() {
        "drop"
    } else {
        "tearoff"
    };

    tracing::info!(
        drag_id = %drag_id,
        result = %result,
        target_window = ?target_window,
        source_window = %session.source_window,
        drag_type = ?session.drag_type,
        payload = ?session.payload,
        screen_x = %screen_x,
        screen_y = %screen_y,
        "[dnd:tauri] complete_cross_drag"
    );

    let _ = app.emit(
        "cross-drag-end",
        serde_json::json!({
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
        }),
    );

    Ok(())
}

/// Cancel an active cross-window drag session.
#[tauri::command]
pub async fn cancel_cross_drag(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    drag_id: String,
) -> Result<(), String> {
    let mut guard = state.active_drag.lock().unwrap();
    match guard.as_ref() {
        Some(s) if s.drag_id != drag_id => {
            tracing::warn!(
                expected = %drag_id,
                actual = %s.drag_id,
                "[dnd:tauri] cancel_cross_drag: drag_id mismatch"
            );
            return Err("drag_id mismatch".to_string());
        }
        None => {
            tracing::warn!(drag_id = %drag_id, "[dnd:tauri] cancel_cross_drag: no active session");
            return Err("no active drag session".to_string());
        }
        _ => {}
    }
    *guard = None;
    drop(guard);

    let _ = app.emit(
        "cross-drag-end",
        serde_json::json!({
            "dragId": drag_id,
            "result": "cancel",
        }),
    );

    tracing::info!(drag_id = %drag_id, "[dnd:tauri] cancel_cross_drag complete");
    Ok(())
}

/// Open a new window at a specific screen position.
/// Used for tear-off operations where the pane/tab becomes a new window.
/// Returns the new window label.
#[tauri::command]
pub async fn open_window_at_position(
    app: tauri::AppHandle,
    state: tauri::State<'_, AppState>,
    screen_x: f64,
    screen_y: f64,
    workspace_id: String,
) -> Result<String, String> {
    tracing::info!(screen_x = %screen_x, screen_y = %screen_y, workspace_id = %workspace_id, "[dnd:tauri] open_window_at_position");

    let window_id = uuid::Uuid::new_v4();
    let label = format!("window-{}", window_id.simple());
    let version = env!("CARGO_PKG_VERSION");
    let title = format!("AgentMux {}", version);

    let win_w = 1200.0_f64;
    let win_h = 800.0_f64;

    // Position the window so the cursor lands near the top-center of the title bar.
    // Offset left by half the width so the cursor is horizontally centered.
    // Offset up by 16px so the cursor lands in the middle of the 33px window header —
    // this way the user can immediately drag the new window from where their cursor is.
    let pos_x = (screen_x - win_w / 2.0).max(0.0);
    let pos_y = (screen_y - 16.0).max(0.0);

    tracing::info!(
        pos_x = %pos_x, pos_y = %pos_y,
        "[dnd:tauri] open_window_at_position: adjusted for cursor centering"
    );

    // Embed the tear-off workspace ID in the URL so the new window's JS can
    // call CreateWindow(workspaceId) to reuse the existing workspace.
    let url_path = if workspace_id.is_empty() {
        "index.html".to_string()
    } else {
        format!("index.html?workspaceId={}", workspace_id)
    };

    let builder = tauri::WebviewWindowBuilder::new(
        &app,
        &label,
        tauri::WebviewUrl::App(url_path.into()),
    )
    .title(&title)
    .inner_size(win_w, win_h)
    .min_inner_size(400.0, 300.0)
    .decorations(false)
    .transparent(true)
    // Required for HTML5 drag-and-drop (pragmatic-dnd) to work on Windows.
    // Without this, WebView2 intercepts drag events for OS file drops,
    // preventing dragend from firing. Mirrors "dragDropEnabled": false in tauri.conf.json.
    .disable_drag_drop_handler()
    .visible(false)
    .position(pos_x, pos_y);

    let _new_window = builder
        .build()
        .map_err(|e| {
            tracing::error!(error = %e, "[dnd:tauri] open_window_at_position: window creation failed");
            format!("Failed to create window: {}", e)
        })?;

    // Platform-specific window setup (macOS styleMask + traffic lights,
    // Linux GTK drag + centering + show fallback, etc.)
    crate::platform::setup_window(&_new_window);

    // Register instance number and notify all windows
    let count = {
        let mut reg = state.window_instance_registry.lock().unwrap();
        let num = reg.register(&label);
        tracing::info!(
            label = %label,
            instance = %num,
            screen_x = %screen_x,
            screen_y = %screen_y,
            "[dnd:tauri] tear-off window registered"
        );
        reg.count()
    };
    let _ = app.emit("window-instances-changed", count);

    Ok(label)
}

/// Hit-test all open windows to find which one contains the given screen coordinates.
/// Returns the window label if found, or None if cursor is outside all windows.
fn hit_test_windows(app: &tauri::AppHandle, screen_x: f64, screen_y: f64) -> Option<String> {
    let windows = app.webview_windows();
    tracing::debug!(
        window_count = %windows.len(),
        screen_x = %screen_x,
        screen_y = %screen_y,
        "[dnd:tauri] hit_test_windows"
    );
    for (label, window) in &windows {
        let pos = match window.outer_position() {
            Ok(p) => p,
            Err(e) => {
                tracing::debug!(label = %label, error = %e, "[dnd:tauri] hit_test: failed to get position");
                continue;
            }
        };
        let size = match window.outer_size() {
            Ok(s) => s,
            Err(e) => {
                tracing::debug!(label = %label, error = %e, "[dnd:tauri] hit_test: failed to get size");
                continue;
            }
        };
        let x = pos.x as f64;
        let y = pos.y as f64;
        let w = size.width as f64;
        let h = size.height as f64;
        tracing::debug!(
            label = %label,
            win_x = %x, win_y = %y, win_w = %w, win_h = %h,
            "[dnd:tauri] hit_test: checking window bounds"
        );
        if screen_x >= x && screen_x <= x + w && screen_y >= y && screen_y <= y + h {
            tracing::debug!(label = %label, "[dnd:tauri] hit_test: HIT");
            return Some(label.clone());
        }
    }
    tracing::debug!("[dnd:tauri] hit_test: no window hit (tear-off zone)");
    None
}

/// Replace the system no-drop cursor with a crosshair while a drag is active.
/// This makes the cursor show "+" instead of the circle-slash when dragging
/// outside the webview window.
#[tauri::command]
pub async fn set_drag_cursor() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CopyIcon, LoadCursorW, SetSystemCursor, IDC_CROSS, OCR_NO,
        };
        unsafe {
            let cross = LoadCursorW(std::ptr::null_mut(), IDC_CROSS);
            if cross.is_null() {
                return Err("LoadCursorW(IDC_CROSS) failed".to_string());
            }
            // CopyCursor is a macro that expands to CopyIcon
            let copy = CopyIcon(cross);
            if copy.is_null() {
                return Err("CopyIcon (CopyCursor) failed".to_string());
            }
            let ok = SetSystemCursor(copy, OCR_NO);
            if ok == 0 {
                return Err("SetSystemCursor failed".to_string());
            }
        }
        tracing::debug!("[dnd:tauri] set_drag_cursor: replaced OCR_NO with IDC_CROSS");
    }
    Ok(())
}

/// Release any leftover mouse capture from an HTML5 drag that ended outside the window.
///
/// After an out-of-window HTML5 DnD (WebView2 IDropSource), the OS may not deliver
/// WM_LBUTTONUP to WebView2, leaving Chromium's internal capture active. This prevents
/// Tauri's JS-based drag region (drag.js mousedown → start_dragging) from working
/// because Chromium's pointer state thinks the left button is still pressed.
///
/// Fix strategy (three layers):
/// 1. ReleaseCapture() — releases Win32 mouse capture from any HWND in our process.
/// 2. WM_CANCELMODE to the top-level HWND — cancels modal tracking on our window.
/// 3. EnumChildWindows + WM_CANCELMODE to every child HWND — WebView2 hosts its visual
///    content in child HWNDs within our process. WM_CANCELMODE does NOT automatically
///    cascade to children, so we enumerate them and post to each explicitly.
///    The WebView2 child HWND receiving WM_CANCELMODE will cancel its own capture and
///    reset Chromium's internal pointer state, restoring normal mousedown delivery.
#[tauri::command]
pub async fn release_drag_capture(window: tauri::Window) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::Foundation::{BOOL, HWND, LPARAM};
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            EnumChildWindows, PostMessageW, WM_CANCELMODE,
        };

        let hwnd = window.hwnd().map_err(|e| format!("hwnd error: {e}"))?.0 as HWND;

        unsafe {
            // Layer 1: release any Win32 capture held by a window in our process.
            ReleaseCapture();

            // Layer 2: cancel modal tracking on the top-level window.
            PostMessageW(hwnd, WM_CANCELMODE, 0, 0);

            // Layer 3: enumerate all child HWNDs (WebView2 lives here) and post
            // WM_CANCELMODE to each so the renderer resets its pointer state.
            unsafe extern "system" fn cancel_child(child: HWND, _: LPARAM) -> BOOL {
                PostMessageW(child, WM_CANCELMODE, 0, 0);
                1 // TRUE — continue enumeration
            }
            EnumChildWindows(hwnd, Some(cancel_child), 0);
        }

        tracing::debug!(
            "[dnd:tauri] release_drag_capture: ReleaseCapture + WM_CANCELMODE → hwnd={:?} + all children",
            hwnd
        );
    }
    Ok(())
}

/// Check whether the primary mouse button (left button) is currently pressed.
///
/// Used by the WebView2 drag fallback in CrossWindowDragMonitor: when the cursor leaves
/// the window during an HTML5 drag, we start a timer because OLE may not deliver `dragend`
/// for drops over native apps. When the timer fires, we check if the button is still held
/// before triggering tearoff — if it is, the user is still hovering (not dropping), so
/// we reschedule instead of acting prematurely.
#[tauri::command]
pub async fn get_mouse_button_state() -> Result<bool, String> {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
        // VK_LBUTTON = 0x01
        let state = unsafe { GetAsyncKeyState(0x01) };
        // High-order bit set means the key is currently down.
        return Ok((state as u16 & 0x8000) != 0);
    }
    #[allow(unreachable_code)]
    Ok(false)
}

/// Signal that a JS-level drag (tab or pane via pragmatic-dnd) is starting or ending.
///
/// On Linux, the GTK window-drag handler (`drag.rs`) checks this flag before calling
/// `begin_move_drag`. Without this guard, dragging a tab in the header area causes
/// the GTK motion handler to call `begin_move_drag`, which on Wayland immediately
/// grabs the compositor pointer and crashes WebKitGTK's drag state machine.
///
/// Call with `active = true` on drag start, `active = false` on drag end.
#[tauri::command]
pub async fn set_js_drag_active(active: bool) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        crate::drag::JS_DRAG_ACTIVE.store(active, std::sync::atomic::Ordering::Relaxed);
        tracing::debug!(active = active, "[dnd:tauri] set_js_drag_active");
    }
    Ok(())
}

/// Restore all system cursors to their defaults.
/// Must be called when a drag ends (drop, tear-off, or cancel).
#[tauri::command]
pub async fn restore_drag_cursor() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SystemParametersInfoW, SPI_SETCURSORS,
        };
        unsafe {
            let ok = SystemParametersInfoW(SPI_SETCURSORS, 0, std::ptr::null_mut(), 0);
            if ok == 0 {
                return Err("SystemParametersInfoW(SPI_SETCURSORS) failed".to_string());
            }
        }
        tracing::debug!("[dnd:tauri] restore_drag_cursor: system cursors restored");
    }
    Ok(())
}
