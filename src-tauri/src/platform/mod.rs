// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Platform-specific window and app setup.
//!
//! Consolidates scattered `#[cfg(target_os = "...")]` blocks from lib.rs,
//! commands/window.rs, and commands/drag.rs into one module per platform.

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "windows")]
mod windows;

/// Called once per window after creation.
/// Applies platform-specific style overrides, drag handlers, etc.
pub fn setup_window<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>) {
    #[cfg(target_os = "macos")]
    macos::setup_window(window);
    #[cfg(target_os = "linux")]
    linux::setup_window(window);
    #[cfg(target_os = "windows")]
    windows::setup_window(window);
}

/// Called on app startup (before any windows).
/// Performs platform-level initialization (console alloc, etc.).
pub fn setup_app(_app: &tauri::App) {
    #[cfg(target_os = "macos")]
    macos::setup_app(_app);
    #[cfg(target_os = "linux")]
    linux::setup_app(_app);
    #[cfg(target_os = "windows")]
    windows::setup_app(_app);
}
