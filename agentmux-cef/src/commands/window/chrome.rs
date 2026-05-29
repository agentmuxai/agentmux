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

/// Toggle a FLOATING pane's OS-window maximize via the pane-state reducer.
///
/// Args: `{ "label": "floating-<uuid>" }` — the floater's window label (the
/// frontend reads it from its `?windowLabel=` URL param). This is the
/// floating half of the shared maximize button (SPEC_PANE_STATE_REDUCER
/// §3.3a / REVISION 2026-05-29); docked magnify is routed frontend-side to
/// the backend and never reaches here.
///
/// Flow mirrors `set_window_opacity` (transparency.rs): dispatch the reducer
/// command, then apply the Win32 side-effect from the emitted event — never
/// inside the reducer (snapshot-and-drop, no I/O in the pure reducer). The
/// reducer owns the Normal↔Maximized state; we resolve the floater's outer
/// HWND from `window_hwnds[label]` and call `ShowWindow`.
///
/// Returns `{ "placement": "maximized" | "normal" }` so the frontend button
/// can update its icon from the result — there is no host→frontend event
/// push channel on main.
///
/// Maximize is intentionally independent of edge-resize: it does NOT install
/// the HTTRANSPARENT WM_NCHITTEST child-subclass, so it cannot perturb the
/// redock hit-testing the way #1132's resize work did.
pub fn toggle_floating_maximize(
    state: &Arc<AppState>,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let label = args
        .get("label")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "toggle_floating_maximize: label is required".to_string())?
        .to_string();

    // 1. Pure reducer dispatch — flips Normal↔Maximized, emits the event.
    let out = state.host_dispatch(crate::reducer::HostCommand::ToggleFloatingMaximize {
        label: label.clone(),
    });

    // 2. Read the placement the reducer settled on.
    let placement = out.events.iter().find_map(|e| match e {
        crate::reducer::HostEvent::PaneWindowStateChanged { placement, .. } => Some(*placement),
        _ => None,
    });

    // 3. Apply the Win32 side-effect AFTER dispatch (mirrors opacity).
    #[cfg(target_os = "windows")]
    {
        use crate::state::WindowPlacement;
        use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_MAXIMIZE, SW_RESTORE};
        if let Some(p) = placement {
            unsafe {
                // Floaters resolve via the `window_hwnds` cache (label →
                // outer HWND); the GA_ROOT walk would land on the owner.
                let hwnd = super::lifecycle::resolve_window_hwnd(state, &label);
                if !hwnd.is_null() {
                    let show = if matches!(p, WindowPlacement::Maximized) {
                        SW_MAXIMIZE
                    } else {
                        SW_RESTORE
                    };
                    ShowWindow(hwnd, show);
                }
            }
        }
    }

    let placement_str = match placement {
        Some(crate::state::WindowPlacement::Maximized) => "maximized",
        _ => "normal",
    };
    Ok(serde_json::json!({ "placement": placement_str }))
}
