// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Window transparency / per-window opacity handlers for the CEF host.
//
// Fourth carve of the commands/window.rs modularization (Plan 1). Pure
// move — no behavior change.
//
// `set_window_transparency`, `set_window_opacity`, `get_window_opacity`
// are `pub` and dispatched by ipc.rs (re-exported `pub use transparency::*`).
// `apply_window_opacity` / `remove_window_opacity` are private Win32 helpers
// used only by the two setters here. `find_all_own_windows` comes from the
// sibling `lifecycle` module (the fallback when a label's HWND hasn't been
// captured yet).
//
// Platform mechanisms (uniform whole-window alpha, post-render, stock-CEF-safe):
//   Windows — WS_EX_LAYERED + SetLayeredWindowAttributes(LWA_ALPHA), inline
//             (Win32 window-style ops are safe from any thread).
//   macOS   — [NSWindow setAlphaValue:], via ui_tasks::post_set_window_alpha
//             (AppKit → must run on the UI thread).
//   Linux   — not yet implemented (needs _NET_WM_WINDOW_OPACITY; owned by the
//             Linux track of SPEC_TRANSPARENCY_MACOS_LINUX_2026_07_01).

use std::sync::Arc;

use crate::state::AppState;

#[cfg(target_os = "windows")]
use super::lifecycle::find_all_own_windows;

/// Set window transparency/blur effects for a single window.
///
/// Targets exactly the window identified by `label` (from the frontend's URL
/// `windowLabel` param). Uses the `window_hwnds` map populated by
/// `capture_hwnd_for_label`. Falls back to `find_all_own_windows()` only
/// if the label's HWND hasn't been captured yet (e.g. very early startup).
pub fn set_window_transparency(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let transparent = args.get("transparent").and_then(|v| v.as_bool()).unwrap_or(false);
    let opacity = args.get("opacity").and_then(|v| v.as_f64()).unwrap_or(0.8);
    let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("main").to_string();
    tracing::info!("set_window_transparency: label={} transparent={} opacity={}", label, transparent, opacity);

    #[cfg(target_os = "windows")]
    {
        let hwnd_raw = state.window_hwnds.lock().get(label.as_str()).copied();
        let hwnds: Vec<*mut std::ffi::c_void> = if let Some(raw) = hwnd_raw {
            vec![raw as *mut std::ffi::c_void]
        } else {
            // HWND not yet captured (early startup). Fall back to process
            // enumeration as a best-effort. This path is temporary — once
            // set_window_init_status fires, future calls use the map.
            tracing::warn!("set_window_transparency: no hwnd for label={}, falling back to find_all_own_windows", label);
            find_all_own_windows()
        };
        for hwnd in hwnds {
            unsafe {
                if transparent {
                    apply_window_opacity(hwnd, opacity);
                } else {
                    remove_window_opacity(hwnd);
                }
            }
            tracing::info!("set_window_transparency: applied to {:?}", hwnd);
        }
    }

    // macOS — Track 1 of SPEC_TRANSPARENCY_MACOS_LINUX_2026_07_01: the
    // WindowServer analogue of the Win32 layered-window alpha above.
    // [NSWindow setAlphaValue:] fades the entire finished window (content,
    // chrome, and shadow) over the desktop — uniform whole-window alpha,
    // exactly the effect Windows ships. Needs no CEF/renderer cooperation.
    // Per-pixel ("glass") transparency is the separate patched-libcef track.
    #[cfg(target_os = "macos")]
    {
        let alpha = if transparent { opacity.clamp(0.0, 1.0) } else { 1.0 };
        crate::ui_tasks::post_set_window_alpha(state, &label, alpha);
    }

    let _ = state;
    // Linux — still a no-op here: uniform alpha needs _NET_WM_WINDOW_OPACITY
    // (X11/XWayland); owned by the Linux track of the same spec.
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let _ = (transparent, opacity);

    Ok(serde_json::Value::Null)
}


/// Apply window-level opacity via WS_EX_LAYERED + SetLayeredWindowAttributes.
/// This makes the entire window semi-transparent (content + chrome).
#[cfg(target_os = "windows")]
unsafe fn apply_window_opacity(hwnd: *mut std::ffi::c_void, opacity: f64) {
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    let alpha = (opacity.clamp(0.0, 1.0) * 255.0) as u8;

    // Add WS_EX_LAYERED extended style
    let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
    SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style | WS_EX_LAYERED as isize);

    // LWA_ALPHA = 0x02
    let result = SetLayeredWindowAttributes(hwnd, 0, alpha, 0x02);
    if result != 0 {
        tracing::info!("Applied window opacity: {} (alpha={})", opacity, alpha);
    } else {
        tracing::warn!("SetLayeredWindowAttributes failed");
    }
}

/// Remove window opacity — restore to fully opaque by removing WS_EX_LAYERED.
#[cfg(target_os = "windows")]
unsafe fn remove_window_opacity(hwnd: *mut std::ffi::c_void) {
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    let ex_style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
    if (ex_style & WS_EX_LAYERED as isize) != 0 {
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, ex_style & !(WS_EX_LAYERED as isize));
        tracing::info!("Removed window opacity (WS_EX_LAYERED cleared)");
    }
}

/// Set opacity on exactly one window by label.
///
/// Routes through the host reducer (`HostCommand::SetWindowOpacity`) so the
/// change is auditable. The reducer emits `WindowOpacityApplied`; `host_dispatch`
/// reads the event and calls the Win32 helper directly (Win32 window-style ops
/// are safe from any thread). Replaces the global `set_window_transparency` path
/// for per-window calls.
pub fn set_window_opacity(
    state: &Arc<AppState>,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let label = args
        .get("label")
        .and_then(|v| v.as_str())
        .ok_or("set_window_opacity: missing label")?
        .to_string();
    let opacity = args
        .get("opacity")
        .and_then(|v| v.as_f64())
        .unwrap_or(1.0)
        .clamp(0.0, 1.0);

    let out = state.host_dispatch(crate::reducer::HostCommand::SetWindowOpacity {
        label: label.clone(),
        opacity: opacity as f32,
    });

    // Apply Win32 side-effect synchronously based on emitted event.
    // Reagent P1 on #868: the reducer emits `WindowOpacityApplied` for
    // opacity < 1.0 (clamped translucent value) and `WindowOpacityCleared`
    // for opacity >= 1.0. Matching only `WindowOpacityApplied` left
    // windows semi-transparent after the user restored full opacity.
    // Match both arms.
    #[cfg(target_os = "windows")]
    for ev in &out.events {
        match ev {
            crate::reducer::HostEvent::WindowOpacityApplied { label: ev_label, opacity: ev_opacity, .. } => {
                let hwnd_raw = state.window_hwnds.lock().get(ev_label.as_str()).copied();
                if let Some(raw) = hwnd_raw {
                    let hwnd = raw as *mut std::ffi::c_void;
                    unsafe { apply_window_opacity(hwnd, *ev_opacity as f64); }
                } else {
                    tracing::warn!("[opacity] set_window_opacity: no hwnd for label={}", ev_label);
                }
            }
            crate::reducer::HostEvent::WindowOpacityCleared { label: ev_label, .. } => {
                let hwnd_raw = state.window_hwnds.lock().get(ev_label.as_str()).copied();
                if let Some(raw) = hwnd_raw {
                    let hwnd = raw as *mut std::ffi::c_void;
                    unsafe { remove_window_opacity(hwnd); }
                } else {
                    tracing::warn!("[opacity] set_window_opacity: no hwnd for label={} (clear)", ev_label);
                }
            }
            _ => {}
        }
    }

    // macOS mirror of the Windows arms above — same reducer events, same
    // both-arms requirement (reagent P1 on #868: matching only Applied left
    // windows semi-transparent after restore). Applies NSWindow.alphaValue
    // on the UI thread via SetWindowAlphaTask.
    #[cfg(target_os = "macos")]
    for ev in &out.events {
        match ev {
            crate::reducer::HostEvent::WindowOpacityApplied { label: ev_label, opacity: ev_opacity, .. } => {
                crate::ui_tasks::post_set_window_alpha(state, ev_label, *ev_opacity as f64);
            }
            crate::reducer::HostEvent::WindowOpacityCleared { label: ev_label, .. } => {
                crate::ui_tasks::post_set_window_alpha(state, ev_label, 1.0);
            }
            _ => {}
        }
    }

    let _ = out;
    Ok(serde_json::Value::Null)
}

/// Return the currently tracked opacity for a label.
///
/// Reads from `HostState.window_opacities` — reflects the last value applied
/// via `set_window_opacity`, not the Win32 layer. Used by the frontend to
/// restore opacity on window init without an extra IPC round-trip.
pub fn get_window_opacity(
    state: &Arc<AppState>,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let label = args
        .get("label")
        .and_then(|v| v.as_str())
        .unwrap_or("main");
    let opacity = state
        .host_state
        .lock()
        .window_opacities
        .get(label)
        .copied()
        .unwrap_or(1.0);
    Ok(serde_json::json!(opacity))
}
