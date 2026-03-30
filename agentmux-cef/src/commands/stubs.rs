// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Stub command handlers for commands deferred to Phase 3+.
// These log the call and return Ok(null) to avoid frontend errors.

/// Handle a stubbed command. Logs the command name and returns null.
pub fn handle_stub(cmd: &str, args: &serde_json::Value) -> serde_json::Value {
    tracing::debug!("stub: {} args={}", cmd, args);
    serde_json::Value::Null
}

/// List of commands that are stubbed (deferred to Phase 3+).
///
/// Cross-window drag commands:
///   start_cross_drag, update_cross_drag, complete_cross_drag,
///   cancel_cross_drag, set_drag_cursor, restore_drag_cursor,
///   release_drag_capture, get_cursor_point, get_mouse_button_state,
///   set_js_drag_active
///
/// Multi-window commands:
///   open_new_window, list_windows, focus_window, open_window_at_position
///
/// Window effects:
///   set_window_transparency
///
/// Legacy claude code stubs:
///   open_claude_code_auth, get_claude_code_auth, disconnect_claude_code
///
/// Existing stubs (already unimplemented in Tauri):
///   download_file, quicklook, update_wco, set_keyboard_chord_mode,
///   create_workspace, switch_workspace, delete_workspace,
///   set_active_tab, create_tab, close_tab
///
/// Update:
///   install_update
pub fn is_stub_command(cmd: &str) -> bool {
    matches!(
        cmd,
        // Legacy stubs
        "open_claude_code_auth"
            | "get_claude_code_auth"
            | "disconnect_claude_code"
            // Existing stubs
            | "download_file"
            | "quicklook"
            | "update_wco"
            | "set_keyboard_chord_mode"
            | "create_workspace"
            | "switch_workspace"
            | "delete_workspace"
            | "set_active_tab"
            | "create_tab"
            | "close_tab"
            // Update
            | "install_update"
            // Devtools (handled separately but stubbed for is_devtools_open)
            | "is_devtools_open"
    )
}
