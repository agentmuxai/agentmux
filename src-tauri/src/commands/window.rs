use tauri::Emitter;
use tauri::Manager;
use tauri::Runtime;

#[cfg(target_os = "linux")]
use crate::drag;
use crate::state::AppState;

#[cfg(target_os = "macos")]
use window_vibrancy::{apply_vibrancy, NSVisualEffectMaterial};

#[cfg(target_os = "windows")]
use window_vibrancy::{apply_acrylic, apply_mica};

/// Open a new AgentMux window.
/// Replaces: ipcMain.on("open-new-window") in emain/emain.ts
///
/// Creates a new window that will initialize with a new backend Window/Workspace/Tab.
/// The frontend detects it's a new window by checking if it's NOT the "main" window,
/// and triggers backend object creation via initTauriWave().
#[tauri::command]
pub async fn open_new_window<R: Runtime>(app: tauri::AppHandle<R>) -> Result<String, String> {
    let window_id = uuid::Uuid::new_v4();
    let label = format!("window-{}", window_id.simple());
    let version = env!("CARGO_PKG_VERSION");
    let title = format!("AgentMux {}", version);

    // On Linux: new windows have UUID labels so tauri_plugin_window_state never
    // has saved state for them. Center explicitly; macOS/Windows handle this natively.
    #[cfg(not(target_os = "linux"))]
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
    .visible(false);

    #[cfg(target_os = "linux")]
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
    .center();

    let _new_window = builder
        .build()
        .map_err(|e| format!("Failed to create window: {}", e))?;

    // On Linux: attach native GTK drag handler so the header is draggable.
    #[cfg(target_os = "linux")]
    drag::attach_drag_handler(&_new_window);

    // Assign a stable instance number to the new window and notify all windows.
    let state = app.state::<AppState>();
    let count = {
        let mut reg = state.window_instance_registry.lock().unwrap();
        let num = reg.register(&label);
        tracing::info!("New window {} assigned instance #{}", label, num);
        reg.count()
    };
    let _ = app.emit("window-instances-changed", count);

    tracing::info!("Created new window with label: {}", label);
    Ok(label)
}

/// Returns the sequential instance number for the calling window (1-based).
/// Returns 0 if the window is not found in the registry (should not happen in practice).
#[tauri::command]
pub fn get_instance_number(
    window: tauri::Window,
    state: tauri::State<'_, AppState>,
) -> u32 {
    let reg = state.window_instance_registry.lock().unwrap();
    reg.get(window.label()).unwrap_or(0)
}

/// Returns the total number of currently open windows.
#[tauri::command]
pub fn get_window_count(state: tauri::State<'_, AppState>) -> usize {
    let reg = state.window_instance_registry.lock().unwrap();
    reg.count()
}

/// Get the current zoom factor.
/// Replaces: ipcMain.on("get-zoom-factor") in emain/emain.ts
#[tauri::command]
pub fn get_zoom_factor(state: tauri::State<'_, AppState>) -> f64 {
    *state.zoom_factor.lock().unwrap()
}

/// Set the zoom factor.
/// Replaces: webContents.setZoomFactor() calls in emain/menu.ts
#[tauri::command]
pub fn set_zoom_factor<R: Runtime>(
    state: tauri::State<'_, AppState>,
    window: tauri::WebviewWindow<R>,
    factor: f64,
) -> Result<(), String> {
    let factor = factor.clamp(0.5, 3.0);
    *state.zoom_factor.lock().unwrap() = factor;

    // Use cross-platform zoom API
    window
        .set_zoom(factor)
        .map_err(|e| format!("Failed to set zoom: {}", e))?;

    // Notify frontend of zoom change
    let _ = window.emit("zoom-factor-change", factor);

    Ok(())
}

/// Get the cursor position in absolute screen coordinates.
/// Replaces: ipcMain.on("get-cursor-point") in emain/emain.ts
///
/// Uses platform-native APIs to get the true screen position, which is
/// essential for cross-window drag operations where the cursor may be
/// outside any window.
#[tauri::command]
pub fn get_cursor_point(#[allow(unused)] window: tauri::Window) -> Result<serde_json::Value, String> {
    // On Windows, use GetCursorPos for absolute screen coordinates.
    // Tauri's window.cursor_position() returns window-relative coords,
    // which are wrong when the cursor is outside the window (e.g. tear-off).
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::Foundation::POINT;
        use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
        let mut point = POINT { x: 0, y: 0 };
        let ok = unsafe { GetCursorPos(&mut point) };
        if ok == 0 {
            return Err("GetCursorPos failed".to_string());
        }
        tracing::debug!(
            x = point.x, y = point.y,
            "[cursor] get_cursor_point (Win32 GetCursorPos)"
        );
        return Ok(serde_json::json!({
            "x": point.x,
            "y": point.y,
        }));
    }

    // On non-Windows platforms, fall back to Tauri's cursor_position.
    #[cfg(not(target_os = "windows"))]
    {
        let position = window
            .cursor_position()
            .map_err(|e| format!("Failed to get cursor position: {}", e))?;
        Ok(serde_json::json!({
            "x": position.x,
            "y": position.y,
        }))
    }
}

/// Close a specific window by label.
/// If no label provided, closes the calling window.
#[tauri::command]
pub fn close_window(
    app: tauri::AppHandle,
    window: tauri::Window,
    label: Option<String>,
) -> Result<(), String> {
    let target_label = label.unwrap_or_else(|| window.label().to_string());

    if let Some(target_window) = app.get_webview_window(&target_label) {
        tracing::info!("Closing window: {}", target_label);
        target_window
            .close()
            .map_err(|e| format!("Failed to close window: {}", e))?;
    } else {
        return Err(format!("Window not found: {}", target_label));
    }

    Ok(())
}

/// Minimize the current window.
#[tauri::command]
pub fn minimize_window(window: tauri::Window) -> Result<(), String> {
    window
        .minimize()
        .map_err(|e| format!("Failed to minimize window: {}", e))?;
    tracing::info!("Minimized window: {}", window.label());
    Ok(())
}

/// Maximize/unmaximize the current window (toggle).
#[tauri::command]
pub fn maximize_window(window: tauri::Window) -> Result<(), String> {
    let is_maximized = window
        .is_maximized()
        .map_err(|e| format!("Failed to check maximize state: {}", e))?;

    if is_maximized {
        window
            .unmaximize()
            .map_err(|e| format!("Failed to unmaximize window: {}", e))?;
        tracing::info!("Unmaximized window: {}", window.label());
    } else {
        window
            .maximize()
            .map_err(|e| format!("Failed to maximize window: {}", e))?;
        tracing::info!("Maximized window: {}", window.label());
    }

    Ok(())
}

/// Get the current window label.
#[tauri::command]
pub fn get_window_label(window: tauri::Window) -> String {
    window.label().to_string()
}

/// Check if this is the main window.
#[tauri::command]
pub fn is_main_window(window: tauri::Window) -> bool {
    window.label() == "main"
}

/// List all open window labels.
#[tauri::command]
pub fn list_windows(app: tauri::AppHandle) -> Vec<String> {
    app.webview_windows()
        .keys()
        .map(|k| k.to_string())
        .collect()
}

/// Focus a specific window by label.
#[tauri::command]
pub fn focus_window(app: tauri::AppHandle, label: String) -> Result<(), String> {
    if let Some(window) = app.get_webview_window(&label) {
        window
            .set_focus()
            .map_err(|e| format!("Failed to focus window: {}", e))?;
        tracing::info!("Focused window: {}", label);
    } else {
        return Err(format!("Window not found: {}", label));
    }
    Ok(())
}

/// Apply window transparency and platform-specific blur effects.
///
/// When `transparent` is true, the window background becomes see-through
/// at the given `opacity` level. When `blur` is also true, platform-native
/// blur effects are applied (macOS vibrancy, Windows Mica/Acrylic).
#[tauri::command]
pub fn set_window_transparency(
    window: tauri::WebviewWindow,
    transparent: bool,
    blur: bool,
    opacity: f64,
) -> Result<(), String> {
    let opacity = opacity.clamp(0.0, 1.0);
    tracing::info!(
        "set_window_transparency: transparent={}, blur={}, opacity={}",
        transparent, blur, opacity
    );

    if !transparent && !blur {
        // Nothing to apply — frontend CSS handles the opaque case
        return Ok(());
    }

    // Platform-specific blur effects
    #[cfg(target_os = "macos")]
    if blur {
        apply_vibrancy(&window, NSVisualEffectMaterial::HudWindow, None, None)
            .map_err(|e| format!("Failed to apply vibrancy: {}", e))?;
        tracing::info!("Applied macOS vibrancy (HudWindow)");
    }

    #[cfg(target_os = "windows")]
    if blur {
        // Try Mica first (Windows 11), fall back to Acrylic (Windows 10)
        if apply_mica(&window, Some(true)).is_err() {
            let alpha = (opacity * 255.0) as u8;
            apply_acrylic(&window, Some((18, 18, 18, alpha)))
                .map_err(|e| format!("Failed to apply acrylic: {}", e))?;
            tracing::info!("Applied Windows Acrylic (alpha={})", alpha);
        } else {
            tracing::info!("Applied Windows Mica");
        }
    }

    #[cfg(target_os = "linux")]
    if blur {
        // Linux blur is compositor-dependent; CSS backdrop-filter is the primary fallback.
        // No reliable cross-compositor API exists.
        tracing::info!("Linux blur requested — using CSS fallback (no native API)");
    }

    Ok(())
}
