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

// Canonical label→top-level-HWND resolver from the sibling `lifecycle`
// module. We resolve by LABEL (not `find_own_top_level_window`, which returns
// the process's first visible top-level = the floater when one exists, so
// minimize/maximize would act on the wrong window). See P1 in
// docs/architecture/ARCHITECTURE_FLOATING_PANE_DOCKING_2026_05_30.md.
#[cfg(target_os = "windows")]
use super::lifecycle::resolve_window_hwnd;

/// Minimize the window. Args: optional `{ "label": string }`; defaults to "main".
pub fn minimize_window(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("main");
    #[cfg(target_os = "windows")]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let hwnd = resolve_window_hwnd(state, label);
        if !hwnd.is_null() {
            ShowWindow(hwnd, SW_MINIMIZE);
            return Ok(serde_json::Value::Null);
        }
    }
    #[cfg(not(target_os = "windows"))]
    crate::ui_tasks::post_minimize_window(state, label);
    let _ = (state, args, label);
    Ok(serde_json::Value::Null)
}

/// Maximize/unmaximize the window (toggle).
///
/// Args: `{ "label": string | null }` — optional window label. When omitted,
/// defaults to "main" (preserves single-window-build behavior). The frontend
/// reads its own label from the `?windowLabel=…` URL query and passes it
/// here so non-main windows act on the right CEF window.
pub fn maximize_window(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("main");
    #[cfg(target_os = "windows")]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let hwnd = resolve_window_hwnd(state, label);
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
    crate::ui_tasks::post_maximize_window(state, label);
    let _ = (state, args, label);
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
/// reducer owns the Normal↔Maximized state (and the restore rect); we
/// resolve the floater's outer HWND from `window_hwnds[label]` and, via
/// `SetWindowPos`, size it to the monitor work area on maximize or back to
/// the reducer-supplied normal rect on restore. We do NOT use
/// `ShowWindow(SW_MAXIMIZE)` — borderless `WS_POPUP` floaters have no usable
/// native maximize placement (it parks them top-left at current size).
///
/// Returns `{ "placement": "maximized" | "normal" }` for callers that want
/// the settled placement. The floating button itself is intentionally a
/// FIXED "Maximize" button (no icon flip), so it ignores the result — the
/// reducer is the single source of truth for placement.
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

    // Read the floater's CURRENT (pre-toggle) screen rect so the reducer can
    // stash it as the restore target. Win32 physical pixels — the same space
    // `SetWindowPos` and `GetMonitorInfoW().rcWork` use, so no DPI conversion.
    #[cfg(target_os = "windows")]
    let current_rect = unsafe {
        use windows_sys::Win32::Foundation::RECT;
        use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect;
        let hwnd = super::lifecycle::resolve_window_hwnd(state, &label);
        if hwnd.is_null() {
            None
        } else {
            let mut r: RECT = std::mem::zeroed();
            // Only stash a sane, non-degenerate rect. On GetWindowRect failure
            // or a zero/negative-area rect (e.g. a window mid-teardown), keep
            // `None` so the reducer records no restore target — a missing
            // restore rect makes the later restore a safe no-op rather than
            // sizing the floater to a 0×0 / inverted rect.
            if GetWindowRect(hwnd, &mut r) != 0 && r.right > r.left && r.bottom > r.top {
                Some(crate::state::PaneRect { left: r.left, top: r.top, right: r.right, bottom: r.bottom })
            } else {
                None
            }
        }
    };
    #[cfg(not(target_os = "windows"))]
    let current_rect: Option<crate::state::PaneRect> = None;

    // 1. Pure reducer dispatch — flips Normal↔Maximized, stashes/returns rect.
    let out = state.host_dispatch(crate::reducer::HostCommand::ToggleFloatingMaximize {
        label: label.clone(),
        current_rect,
    });

    // 2. Read the placement + restore rect the reducer settled on.
    let (placement, restore_rect) = out
        .events
        .iter()
        .find_map(|e| match e {
            crate::reducer::HostEvent::PaneWindowStateChanged { placement, restore_rect, .. } => {
                Some((*placement, *restore_rect))
            }
            _ => None,
        })
        .unwrap_or((crate::state::WindowPlacement::Normal, None));

    // 3. Apply the Win32 geometry AFTER dispatch (snapshot-and-drop, mirrors
    //    set_window_opacity). Borderless WS_POPUP floaters have no usable
    //    native maximize placement — `ShowWindow(SW_MAXIMIZE)` parks them at
    //    the top-left at current size — so we size to the monitor work area
    //    on maximize and back to the captured rect on restore.
    #[cfg(target_os = "windows")]
    {
        use crate::state::WindowPlacement;
        use windows_sys::Win32::Foundation::RECT;
        use windows_sys::Win32::Graphics::Gdi::{
            GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::SetWindowPos;
        unsafe {
            // Floaters resolve via the `window_hwnds` cache (label → outer
            // HWND); the GA_ROOT walk would land on the owner.
            let hwnd = super::lifecycle::resolve_window_hwnd(state, &label);
            if !hwnd.is_null() {
                let target: Option<RECT> = match placement {
                    WindowPlacement::Maximized => {
                        // Work area (excludes taskbar) of the floater's monitor.
                        let hmon = MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST);
                        if hmon.is_null() {
                            None
                        } else {
                            let mut mi: MONITORINFO = std::mem::zeroed();
                            mi.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
                            if GetMonitorInfoW(hmon, &mut mi) != 0 {
                                Some(mi.rcWork)
                            } else {
                                None
                            }
                        }
                    }
                    // Normal/Minimized → restore to the captured rect (if any).
                    _ => restore_rect
                        .map(|r| RECT { left: r.left, top: r.top, right: r.right, bottom: r.bottom }),
                };
                if let Some(rc) = target {
                    SetWindowPos(
                        hwnd,
                        std::ptr::null_mut(),
                        rc.left,
                        rc.top,
                        rc.right - rc.left,
                        rc.bottom - rc.top,
                        // 0x0014 = SWP_NOZORDER (0x0004) | SWP_NOACTIVATE (0x0010).
                        0x0014,
                    );
                }
            }
        }
    }

    let placement_str = match placement {
        crate::state::WindowPlacement::Maximized => "maximized",
        _ => "normal",
    };
    Ok(serde_json::json!({ "placement": placement_str }))
}
