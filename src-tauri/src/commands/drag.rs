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

    tracing::info!("Cross-drag started: {}", drag_id);
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
            Some(_) => return Err("drag_id mismatch".to_string()),
            None => return Err("no active drag session".to_string()),
        }
    };

    // Hit-test all windows to find which one the cursor is over
    let target_window = hit_test_windows(&app, screen_x, screen_y);

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
                // Put it back if ID doesn't match
                *guard = Some(s);
                return Err("drag_id mismatch".to_string());
            }
            None => return Err("no active drag session".to_string()),
        }
    };

    let result = if target_window.is_some() {
        "drop"
    } else {
        "tearoff"
    };

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

    tracing::info!("Cross-drag completed: {} ({})", drag_id, result);
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
            return Err("drag_id mismatch".to_string());
        }
        None => {
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

    tracing::info!("Cross-drag cancelled: {}", drag_id);
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
) -> Result<String, String> {
    let window_id = uuid::Uuid::new_v4();
    let label = format!("window-{}", window_id.simple());
    let version = env!("CARGO_PKG_VERSION");
    let title = format!("AgentMux {}", version);

    let builder = tauri::WebviewWindowBuilder::new(
        &app,
        &label,
        tauri::WebviewUrl::App("index.html".into()),
    )
    .title(&title)
    .inner_size(1200.0, 800.0)
    .min_inner_size(400.0, 300.0)
    .decorations(false)
    .transparent(true)
    .visible(false)
    .position(screen_x, screen_y);

    let _new_window = builder
        .build()
        .map_err(|e| format!("Failed to create window: {}", e))?;

    // On Linux: attach native GTK drag handler
    #[cfg(target_os = "linux")]
    crate::drag::attach_drag_handler(&_new_window);

    // Register instance number and notify all windows
    let count = {
        let mut reg = state.window_instance_registry.lock().unwrap();
        let num = reg.register(&label);
        tracing::info!(
            "Tear-off window {} assigned instance #{} at ({}, {})",
            label,
            num,
            screen_x,
            screen_y
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
    for (label, window) in &windows {
        let pos = match window.outer_position() {
            Ok(p) => p,
            Err(_) => continue,
        };
        let size = match window.outer_size() {
            Ok(s) => s,
            Err(_) => continue,
        };
        let x = pos.x as f64;
        let y = pos.y as f64;
        let w = size.width as f64;
        let h = size.height as f64;
        if screen_x >= x && screen_x <= x + w && screen_y >= y && screen_y <= y + h {
            return Some(label.clone());
        }
    }
    None
}
