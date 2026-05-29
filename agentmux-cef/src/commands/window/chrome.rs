// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Window chrome handlers for the CEF host — minimize / maximize toggle.
//
// Third carve of the commands/window.rs modularization (Plan 1). Both
// handlers are `pub` and dispatched by ipc.rs (re-exported
// `pub use chrome::*`). Pure move — no behavior change.

use std::sync::Arc;

use crate::state::AppState;

// Resolved per-process top-level HWND lives in the sibling `lifecycle`
// module; used only inside the `#[cfg(windows)]` branches below.
#[cfg(target_os = "windows")]
use super::lifecycle::find_own_top_level_window;

/// Minimize the window. Args: optional `{ "label": string }`; defaults to "main".
pub fn minimize_window(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    #[cfg(target_os = "windows")]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let hwnd = find_own_top_level_window();
        if !hwnd.is_null() {
            ShowWindow(hwnd, SW_MINIMIZE);
            return Ok(serde_json::Value::Null);
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("main");
        crate::ui_tasks::post_minimize_window(state, label);
    }
    let _ = (state, args);
    Ok(serde_json::Value::Null)
}

/// Maximize/unmaximize the window (toggle).
///
/// Args: `{ "label": string | null }` — optional window label. When omitted,
/// defaults to "main" (preserves single-window-build behavior). The frontend
/// reads its own label from the `?windowLabel=…` URL query and passes it
/// here so non-main windows act on the right CEF window.
pub fn maximize_window(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    #[cfg(target_os = "windows")]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let hwnd = find_own_top_level_window();
        if !hwnd.is_null() {
            let mut placement: WINDOWPLACEMENT = std::mem::zeroed();
            placement.length = std::mem::size_of::<WINDOWPLACEMENT>() as u32;
            GetWindowPlacement(hwnd, &mut placement);
            if placement.showCmd == SW_MAXIMIZE as u32 {
                ShowWindow(hwnd, SW_RESTORE);
            } else {
                ShowWindow(hwnd, SW_MAXIMIZE);
            }
            return Ok(serde_json::Value::Null);
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("main");
        crate::ui_tasks::post_maximize_window(state, label);
    }
    let _ = (state, args);
    Ok(serde_json::Value::Null)
}
