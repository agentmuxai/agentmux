// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! macOS-specific window setup.
//!
//! - Override NSWindow styleMask to restore native resize handles on a frameless window.
//! - Hide traffic light buttons (close/minimize/zoom) — custom buttons handled in frontend
//!   on non-macOS, and macOS users rely on keyboard shortcuts / app menu.
//! - Enable dragging by window background.

use objc2_app_kit::{NSWindow, NSWindowButton, NSWindowStyleMask, NSWindowTitleVisibility};

pub fn setup_window<R: tauri::Runtime>(window: &tauri::WebviewWindow<R>) {
    if let Ok(ns_win_ptr) = window.ns_window() {
        let ns_window: &NSWindow = unsafe { &*(ns_win_ptr as *const NSWindow) };

        // Titled + FullSizeContentView for native resize handles.
        // `decorations:false` gives Borderless with ~1px thin resize edges (unusable).
        // Switching to Titled + FullSizeContentView with a transparent hidden titlebar
        // gives proper native resize handles while keeping the frameless appearance.
        let mask = NSWindowStyleMask::Titled
            | NSWindowStyleMask::Resizable
            | NSWindowStyleMask::Miniaturizable
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::FullSizeContentView;
        ns_window.setStyleMask(mask);
        ns_window.setTitlebarAppearsTransparent(true);
        ns_window.setTitleVisibility(NSWindowTitleVisibility::Hidden);

        // Hide traffic light buttons (close/minimize/zoom).
        // These are visible because we use Titled style; hide them since the
        // frontend provides custom window action buttons on Windows/Linux,
        // and macOS users use Cmd-W / Cmd-M / green-button for these actions.
        for button_type in [
            NSWindowButton::CloseButton,
            NSWindowButton::MiniaturizeButton,
            NSWindowButton::ZoomButton,
        ] {
            if let Some(button) = ns_window.standardWindowButton(button_type) {
                button.setHidden(true);
            }
        }

        // Allow dragging by window background (header area).
        ns_window.setMovableByWindowBackground(true);

        tracing::info!(
            "macOS: applied Titled+FullSizeContentView styleMask, hid traffic lights for window '{}'",
            window.label()
        );
    } else {
        tracing::warn!(
            "macOS: failed to get NSWindow handle for '{}'",
            window.label()
        );
    }
}

pub fn setup_app(_app: &tauri::App) {
    // Nothing needed at app level for macOS currently.
}
