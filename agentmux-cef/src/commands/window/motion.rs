// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Window motion handlers for the CEF host — position, drag, and the
// floating-pane redock-hover feedback.
//
// Second carve of the commands/window.rs modularization
// (docs/analysis/ANALYSIS_LARGE_FILE_MODULARIZATION_CANDIDATES_2026_05_28.md,
// Plan 1). All handlers are `pub` and dispatched by ipc.rs (re-exported
// `pub use motion::*` from the parent). Pure move — no behavior change.

use std::sync::Arc;

use crate::state::AppState;

// HWND resolution lives in the sibling `lifecycle` module. The position
// handlers call these only inside `#[cfg(windows)]` blocks, so the import
// is gated to match (avoids an unused-import error on other targets).
#[cfg(target_os = "windows")]
use super::lifecycle::{find_main_window, find_own_top_level_window, resolve_window_hwnd};

/// Get the current window position on screen.
///
/// Accepts `{ "label": string }` to disambiguate when the process has
/// multiple top-level windows (e.g. main + floating panes). Without a
/// label we fall back to `find_own_top_level_window`, which returns
/// the topmost-Z top-level of the process — wrong when the caller is
/// the main window but a floater (owned, drawn above) exists.
pub fn get_window_position(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("");

    #[cfg(target_os = "windows")]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        use windows_sys::Win32::Foundation::RECT;
        let hwnd = resolve_window_hwnd(state, label);
        if !hwnd.is_null() {
            let mut rect: RECT = std::mem::zeroed();
            GetWindowRect(hwnd, &mut rect);
            return Ok(serde_json::json!({ "x": rect.left, "y": rect.top }));
        }
    }
    let _ = (state, label);
    Ok(serde_json::json!({ "x": 0, "y": 0 }))
}

/// Move the window by a delta (dx, dy) from its current position.
///
/// **Prefer `set_window_position` for drag flows.** This function reads
/// the current rect via `GetWindowRect` then SetWindowPos with the new
/// origin — under rapid concurrent calls (mousemove drag), in-flight
/// IPCs all read the same stale rect and only one delta gets applied.
/// `set_window_position` is self-contained (no read-modify-write) and
/// idempotent under concurrency.
pub fn move_window_by(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let dx = args.get("dx").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let dy = args.get("dy").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("main");

    #[cfg(target_os = "windows")]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        use windows_sys::Win32::Foundation::RECT;
        let hwnd = find_own_top_level_window();
        if !hwnd.is_null() {
            let mut rect: RECT = std::mem::zeroed();
            GetWindowRect(hwnd, &mut rect);
            let width = rect.right - rect.left;
            let height = rect.bottom - rect.top;
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                rect.left + dx,
                rect.top + dy,
                width,
                height,
                // 0x0014 = SWP_NOZORDER (0x0004) | SWP_NOACTIVATE (0x0010).
                0x0014,
            );
            return Ok(serde_json::Value::Null);
        }
    }
    #[cfg(not(target_os = "windows"))]
    crate::ui_tasks::post_move_window(state, label, dx, dy);
    let _ = (state, label);
    Ok(serde_json::Value::Null)
}

/// Move the window to an absolute screen position (x, y).
/// Each call is self-contained — no read-modify-write — so concurrent in-flight
/// calls are idempotent: the last write wins, which is exactly correct for drag.
pub fn set_window_position(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let x = args.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let y = args.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("main");

    #[cfg(target_os = "windows")]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        use windows_sys::Win32::Foundation::RECT;
        let hwnd = resolve_window_hwnd(state, label);
        if !hwnd.is_null() {
            let mut rect: RECT = std::mem::zeroed();
            GetWindowRect(hwnd, &mut rect);
            let width = rect.right - rect.left;
            let height = rect.bottom - rect.top;
            SetWindowPos(
                hwnd,
                std::ptr::null_mut(),
                x,
                y,
                width,
                height,
                // 0x0014 = SWP_NOZORDER (0x0004) | SWP_NOACTIVATE (0x0010).
                // (Width/height are still passed explicitly above so size
                // is preserved without needing SWP_NOSIZE.)
                0x0014,
            );
            return Ok(serde_json::Value::Null);
        }
    }
    #[cfg(not(target_os = "windows"))]
    crate::ui_tasks::post_set_window_position(state, label, x, y);
    let _ = (state, label);
    Ok(serde_json::Value::Null)
}

/// Get the window's full screen rect `{ x, y, width, height }` (physical px).
/// Used by the floater JS-driven edge-resize to capture the start rect.
pub fn get_window_rect(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("main");
    #[cfg(target_os = "windows")]
    unsafe {
        use windows_sys::Win32::Foundation::RECT;
        use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect;
        let hwnd = resolve_window_hwnd(state, label);
        if !hwnd.is_null() {
            let mut rect: RECT = std::mem::zeroed();
            GetWindowRect(hwnd, &mut rect);
            return Ok(serde_json::json!({
                "x": rect.left,
                "y": rect.top,
                "width": rect.right - rect.left,
                "height": rect.bottom - rect.top,
            }));
        }
    }
    let _ = (state, label);
    Ok(serde_json::json!({ "x": 0, "y": 0, "width": 0, "height": 0 }))
}

/// Set the window's full screen rect (absolute position AND size), physical px.
/// Self-contained (no read-modify-write) so concurrent in-flight calls are
/// idempotent — last write wins, exactly right for a live edge-resize drag.
/// This is the floater edge-resize primitive: the frontend captures the start
/// rect (`get_window_rect`) on edge pointer-down (with pointer capture so it
/// keeps receiving moves), computes the new rect per cursor delta + edge, and
/// calls this on each move. Sidesteps WM_NCHITTEST / native SC_SIZE (Chromium
/// holds the DOM mouse capture). See SPEC_FLOATING_PANE_EDGE_RESIZE.
pub fn set_window_rect(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let x = args.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let y = args.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let width = args.get("width").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let height = args.get("height").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("main");

    #[cfg(target_os = "windows")]
    if width > 0 && height > 0 {
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::SetWindowPos;
            let hwnd = resolve_window_hwnd(state, label);
            if !hwnd.is_null() {
                SetWindowPos(
                    hwnd,
                    std::ptr::null_mut(),
                    x,
                    y,
                    width,
                    height,
                    0x0014, // SWP_NOZORDER | SWP_NOACTIVATE
                );
                return Ok(serde_json::Value::Null);
            }
        }
    }
    let _ = (state, label, x, y, width, height);
    Ok(serde_json::Value::Null)
}

/// Initiate window drag (for frameless windows).
/// Windows: sends WM_NCLBUTTONDOWN/HTCAPTION via Win32 — find_own_top_level_window
/// resolves the per-process HWND so multi-window works without a label.
/// Linux/macOS: dispatches CefWindow::BeginWindowDrag on the UI thread; needs
/// the source window's label so non-main windows drag themselves rather than
/// the main window. Frontend reads `?windowLabel=…` from its URL and passes
/// it here; missing → "main" for backward compatibility.
pub fn start_window_drag(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    #[cfg(target_os = "windows")]
    {
        // ReleaseCapture() + WM_NCLBUTTONDOWN(HTCAPTION) starts the OS modal
        // move loop — but BOTH must run on the CEF UI thread, the thread that
        // OWNS the renderer's mouse capture. This handler runs on a tokio IPC
        // worker, where `ReleaseCapture()` is a NO-OP (it releases only the
        // calling thread's capture), so the renderer keeps capture and the move
        // loop never engages. That is the historical "WM_NCLBUTTONDOWN loses
        // mouse state". HWND lookup is thread-safe, so resolve here (label-aware,
        // so a floating pane drags itself, not main) and marshal the begin-move
        // onto the UI thread.
        let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("main");
        let hwnd = unsafe {
            let h = resolve_window_hwnd(state, label);
            if h.is_null() {
                find_own_top_level_window()
            } else {
                h
            }
        };
        if !hwnd.is_null() {
            crate::ui_tasks::post_win32_begin_move(hwnd as usize as u64);
        } else {
            tracing::warn!("[start_window_drag] no HWND resolved for label={}", label);
        }
        return Ok(serde_json::Value::Null);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("main");
        crate::ui_tasks::post_start_drag(state, label);
    }
    let _ = (state, args);
    Ok(serde_json::Value::Null)
}

/// Resolve which agentmux window is under the cursor. Used by the
/// floating-pane re-dock flow: on the floater's mouseup, the
/// frontend asks "what window is the cursor over now?" and, if
/// the answer is another agentmux window in the same process,
/// kicks off `RedockFloatingPane`.
///
/// Args: `{ "x": int, "y": int, "exclude_label": string|null }` —
/// cursor position in physical px (whatever `get_cursor_point`
/// returned). `exclude_label` is the source floater's label; without
/// it, `WindowFromPoint` returns the floater itself (the floater
/// follows the cursor during drag, so it's always at the cursor in
/// Z-order). We walk top-levels in Z-order from front to back and
/// return the first match that ISN'T the excluded source.
///
/// Returns: `{ "label": string|null, "window_id": string|null }`. Both
/// null means no agentmux window of this process is under the cursor
/// (cursor is over the desktop, an external app, etc.). `window_id`
/// is the backend windowId mapped via the launcher's
/// `BackendWindowIdRegistered` projection — frontend uses it to
/// load the WaveWindow / Workspace and figure out the active tab.
///
/// Instance / version isolation is free: another agentmux instance's
/// HWNDs are NOT in this process's `window_hwnds` map, so cross-
/// process drag-over-then-drop won't accidentally re-dock there.
pub fn resolve_window_at_cursor(
    state: &Arc<AppState>,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let x = args.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let y = args.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let exclude_label = args
        .get("exclude_label")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    #[cfg(target_os = "windows")]
    unsafe {
        use windows_sys::Win32::Foundation::RECT;
        use windows_sys::Win32::System::Threading::GetCurrentProcessId;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetTopWindow, GetWindow, GetWindowLongW, GetWindowRect, GetWindowThreadProcessId,
            IsWindowVisible, GWL_EXSTYLE, GW_HWNDNEXT, WS_EX_TRANSPARENT,
        };

        // Snapshot the label↔HWND map once, then walk top-level
        // windows in Z-order (front to back) until we find a visible
        // same-process window whose rect contains the cursor AND
        // whose label isn't the excluded source.
        let hwnds_by_label: std::collections::HashMap<String, isize> = {
            state.window_hwnds.lock().clone()
        };
        // Reverse map for O(1) HWND → label lookup.
        let label_by_hwnd: std::collections::HashMap<isize, String> = hwnds_by_label
            .iter()
            .map(|(k, v)| (*v, k.clone()))
            .collect();
        let exclude_hwnd: Option<isize> = if exclude_label.is_empty() {
            None
        } else {
            hwnds_by_label.get(exclude_label).copied()
        };

        // Deterministic "main" fallback. `window_hwnds["main"]` is populated
        // by a startup capture that races the main window becoming
        // enumerable — when it loses, "main" is absent from the map and the
        // reverse lookup below can't recognise the main frame, so a floater
        // dropped onto the main window silently fails to re-dock (the redock
        // intermittency). `find_main_window()` resolves the real on-screen
        // main frame independently of the cache (it skips floaters and
        // off-screen pool windows), so we can recognise main by HWND even
        // when it never made it into `window_hwnds`. Resolved once here, not
        // per-iteration. `null` if unresolved → the comparison just never
        // matches (no regression).
        let main_hwnd: isize = find_main_window() as isize;

        let our_pid = GetCurrentProcessId();
        let mut hwnd = GetTopWindow(std::ptr::null_mut());
        while !hwnd.is_null() {
            if IsWindowVisible(hwnd) != 0 {
                let mut window_pid: u32 = 0;
                GetWindowThreadProcessId(hwnd, &mut window_pid);
                if window_pid == our_pid {
                    let h_isize = hwnd as isize;
                    let is_excluded = exclude_hwnd == Some(h_isize);
                    // Skip click-through / transparent windows
                    // (defensive — agentmux doesn't currently create
                    // any, but we don't want a hypothetical overlay
                    // to swallow the hit-test).
                    let ex_style = GetWindowLongW(hwnd, GWL_EXSTYLE) as u32;
                    let is_transparent = (ex_style & WS_EX_TRANSPARENT) != 0;
                    if !is_excluded && !is_transparent {
                        let mut rect: RECT = std::mem::zeroed();
                        if GetWindowRect(hwnd, &mut rect) != 0
                            && x >= rect.left
                            && x < rect.right
                            && y >= rect.top
                            && y < rect.bottom
                        {
                            if let Some(label) = label_by_hwnd.get(&h_isize) {
                                let wid = state.backend_window_id(label);
                                return Ok(serde_json::json!({
                                    "label": label,
                                    "window_id": wid,
                                }));
                            }
                            // Not in window_hwnds — but if this IS the main
                            // frame (per the cache-independent resolver),
                            // recognise it as "main" anyway. This is the
                            // deterministic redock-onto-main fix: it no longer
                            // depends on the startup capture having won the
                            // race to register "main".
                            if main_hwnd != 0 && h_isize == main_hwnd {
                                let wid = state.backend_window_id("main");
                                return Ok(serde_json::json!({
                                    "label": "main",
                                    "window_id": wid,
                                }));
                            }
                            // Found a window we own but it isn't in
                            // window_hwnds and isn't the main frame (very
                            // early startup or a window we don't track).
                            // Treat as "no agentmux match" and continue the
                            // Z-order walk in case a tracked window sits
                            // behind it.
                        }
                    }
                }
            }
            hwnd = GetWindow(hwnd, GW_HWNDNEXT);
        }
        Ok(serde_json::json!({ "label": null, "window_id": null }))
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (state, args, x, y, exclude_label);
        Ok(serde_json::json!({ "label": null, "window_id": null }))
    }
}

/// Update the host's "active floating-pane redock hover" state.
/// Called from the dragged floater's mousemove (throttled ~50ms upstream)
/// so target windows can render a per-tile drop preview while the floater
/// hovers over them. The host resolves the target window via the same
/// Z-order walk as `resolve_window_at_cursor` (with the dragged floater
/// excluded) and emits `floating-redock:hover-state` on every call —
/// the target renderer needs the live cursor position to pick which
/// LEAF the cursor is over (not just transitions between windows).
///
/// Args: `{ "source_label": string, "x": int, "y": int }`. Returns
/// `{ "target_label": string|null }` for callers that want to
/// piggyback on this for the resolved target.
pub fn update_floating_redock_hover(
    state: &Arc<AppState>,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let source_label = args
        .get("source_label")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let cursor_x = args.get("x").and_then(|v| v.as_i64()).unwrap_or(0);
    let cursor_y = args.get("y").and_then(|v| v.as_i64()).unwrap_or(0);
    let resolve_args = serde_json::json!({
        "x": cursor_x,
        "y": cursor_y,
        "exclude_label": source_label,
    });
    let resolved = resolve_window_at_cursor(state, &resolve_args)?;
    let new_target = resolved
        .get("label")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());

    // Emit on every call (frontend throttles ~50 ms upstream). The
    // event must carry the cursor position so target renderers can
    // compute which TILE the cursor is over and highlight just that
    // leaf (not the whole window). Cursor is in physical screen px.
    let payload = serde_json::json!({
        "target_label": new_target.clone(),
        "source_label": source_label,
        "cursor_x": cursor_x,
        "cursor_y": cursor_y,
    });
    crate::events::emit_event_to_top_level_windows(
        state,
        "floating-redock:hover-state",
        &payload,
    );

    Ok(serde_json::json!({ "target_label": new_target }))
}

/// Clear the active floating-pane redock hover state. Called from the
/// dragged floater's mouseup (and any drag-cancel path). Emits the
/// `floating-redock:hover-state` event with `target_label: null` so
/// target windows tear down their highlight overlay.
pub fn clear_floating_redock_hover(
    state: &Arc<AppState>,
    _args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    // Always emit, even if no prior hover was active. The frontend
    // listener uses target_label=null as a teardown sentinel and
    // removing a non-existent placeholder is a cheap no-op; emitting
    // unconditionally guarantees cleanup fires even if the last
    // mousemove resolved to None (cursor over the floater itself
    // between in/out boundaries).
    crate::events::emit_event_to_top_level_windows(
        state,
        "floating-redock:hover-state",
        &serde_json::json!({ "target_label": serde_json::Value::Null }),
    );
    Ok(serde_json::Value::Null)
}

