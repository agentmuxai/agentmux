// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Linux-specific window setup.
//!
//! - Attach native GTK drag handler for window header dragging.
//! - Center the window if no saved position exists (X11 defaults to 0,0).
//! - Show the window from Rust as a fallback in case JS bundle fails to load.

pub fn setup_window<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>) {
    // Attach native GTK drag handler so the header is draggable.
    crate::drag::attach_drag_handler(window);

    // Center the window if no saved position exists.
    // tauri_plugin_window_state auto-restores saved state at window creation.
    // If no state was saved yet, the window lands at (0,0) on Linux because
    // X11 has no default centering behavior (unlike macOS/Windows). So if
    // the window is still at the origin after plugin restoration, center it.
    if window
        .outer_position()
        .map(|p| p.x == 0 && p.y == 0)
        .unwrap_or(true)
    {
        let _ = window.center();
    }

    // Show the window from Rust as a fallback.
    // The window is created with visible:false to avoid FOUC; JS calls
    // currentWindow.show() after initialization. But if the JS bundle fails
    // to execute, the window would stay invisible forever. Showing it from
    // Rust ensures the window is always visible even if JS fails.
    // JS calling show() again later is harmless (idempotent).
    let _ = window.show();
    tracing::info!("[diag] window.show() called from Rust (Linux fallback)");
}

pub fn setup_app(_app: &tauri::App) {
    // Nothing needed at app level for Linux currently.
}
