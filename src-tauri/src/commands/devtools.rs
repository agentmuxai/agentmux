/// Toggle devtools for the current window.
/// Maps to Ctrl+Shift+I / Cmd+Option+I keyboard shortcut.
#[tauri::command]
pub fn toggle_devtools(window: tauri::WebviewWindow) {
    #[cfg(debug_assertions)]
    {
        if window.is_devtools_open() {
            tracing::info!("Closing devtools for window: {}", window.label());
            window.close_devtools();
        } else {
            tracing::info!("Opening devtools for window: {}", window.label());
            window.open_devtools();
        }
    }

    #[cfg(not(debug_assertions))]
    {
        // Devtools in release builds is intentional for debugging support
        if window.is_devtools_open() {
            tracing::info!("Closing devtools for window: {} (release build)", window.label());
            window.close_devtools();
        } else {
            tracing::info!("Opening devtools for window: {} (release build)", window.label());
            window.open_devtools();
        }
    }
}

/// Check if devtools are currently open.
#[tauri::command]
pub fn is_devtools_open(window: tauri::WebviewWindow) -> bool {
    window.is_devtools_open()
}
