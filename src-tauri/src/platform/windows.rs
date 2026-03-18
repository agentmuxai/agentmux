// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Windows-specific window and app setup.
//!
//! - In dev mode, clear the WebView2 HTTP cache to prevent stale bundle loading.

use tauri::Manager;

pub fn setup_window<R: tauri::Runtime>(_window: &tauri::WebviewWindow<R>) {
    // Nothing extra for Windows window setup currently.
}

pub fn setup_app(app: &tauri::App) {
    // In dev mode, clear the WebView2 cache to prevent stale bundles.
    #[cfg(debug_assertions)]
    {
        if let Ok(data_dir) = app.handle().path().app_local_data_dir() {
            let cache_dir = data_dir.join("EBWebView").join("Default").join("Cache");
            if cache_dir.exists() {
                match std::fs::remove_dir_all(&cache_dir) {
                    Ok(_) => tracing::info!("Cleared WebView2 cache for dev mode"),
                    Err(e) => tracing::debug!("Could not clear WebView2 cache: {}", e),
                }
            }
        }
    }
}
