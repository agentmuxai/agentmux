// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Window motion handlers for the CEF host — position, drag, and the
// floating-pane redock-hover feedback.
//
// Second carve of the `commands::window` modularization
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
    // macOS / Linux: CEF Views windows report DIP bounds; read them on the UI
    // thread. The floating-pane header drag uses this as its absolute-move
    // baseline, adding CSS-px (= DIP) deltas with no DPR scaling — see
    // floating-pane-workspace.tsx `posScale()`. (Windows works in physical px
    // and scales by devicePixelRatio; that path returns above.)
    #[cfg(not(target_os = "windows"))]
    {
        if let Some((x, y)) = crate::ui_tasks::get_window_position_blocking(state, label) {
            return Ok(serde_json::json!({ "x": x, "y": y }));
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

/// Get the window's full screen rect `{ x, y, width, height }`.
/// Windows: physical px via GetWindowRect (thread-agnostic).
/// macOS / Linux: DIP via CEF Views `window.bounds()` (UI-thread read, 500ms timeout).
/// Used by the floater JS-driven edge-resize to capture the start rect on pointer-down.
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
    #[cfg(not(target_os = "windows"))]
    if let Some((x, y, width, height)) = crate::ui_tasks::get_window_rect_blocking(state, label) {
        return Ok(serde_json::json!({ "x": x, "y": y, "width": width, "height": height }));
    }
    let _ = (state, label);
    Ok(serde_json::json!({ "x": 0, "y": 0, "width": 0, "height": 0 }))
}

/// Set the window's full screen rect (absolute position AND size).
/// Self-contained (no read-modify-write) so concurrent in-flight calls are
/// idempotent — last write wins, exactly right for a live edge-resize drag.
/// Windows: physical px via SetWindowPos. macOS / Linux: DIP via CEF Views
/// `window.set_bounds()` posted to the UI thread (1 CSS px == 1 DIP, matching
/// the frontend's `screenX/Y` deltas which are in CSS px). See SPEC_FLOATING_PANE_EDGE_RESIZE.
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
    #[cfg(not(target_os = "windows"))]
    if width > 0 && height > 0 {
        crate::ui_tasks::post_set_window_rect(state, label, x, y, width, height);
    }
    let _ = (state, label, x, y, width, height);
    Ok(serde_json::Value::Null)
}

/// Initiate window drag (for frameless windows).
/// Windows: resolves the top-level HWND label-aware (`resolve_window_hwnd`, so a
/// floating pane drags itself) and posts a host-side manual move loop on the CEF
/// UI thread (`post_win32_begin_move` → `ui_tasks::Win32BeginMoveTask`). The raw
/// WM_NCLBUTTONDOWN/HTCAPTION OS move loop does NOT work for a CEF window
/// (Chromium's frame swallows it), so the host drives the loop itself.
/// Linux: dispatches CefWindow::BeginWindowDrag on the UI thread. macOS: runs a
/// host-side manual move loop (`MacWindowDragTask` → `run_macos_native_drag_loop`,
/// repositioning via `set_bounds`). Both need the source window's label so
/// non-main windows drag themselves rather than the main window. Frontend reads `?windowLabel=…` from its URL and passes
/// it here; missing → "main" for backward compatibility.
pub fn start_window_drag(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("main");
    #[cfg(target_os = "windows")]
    {
        // The native move loop must run on the CEF UI thread (it owns the
        // renderer's mouse capture). This IPC handler is on a tokio worker, so
        // we only do thread-safe HWND lookup here (label-aware, so a floating
        // pane drags itself, not main) and marshal the move loop onto the UI
        // thread via `post_win32_begin_move`. The loop itself is a manual
        // SetCapture + GetMessage + SetWindowPos loop (`ui_tasks::Win32BeginMoveTask`),
        // NOT WM_NCLBUTTONDOWN — Chromium's frame swallows that NC message, so
        // the OS modal move loop never engages for a CEF window.
        let hwnd = unsafe {
            let h = resolve_window_hwnd(state, label);
            if h.is_null() {
                find_own_top_level_window()
            } else {
                h
            }
        };
        if !hwnd.is_null() {
            crate::ui_tasks::post_win32_begin_move(hwnd as usize as u64, state.clone(), Some(label.to_string()));
        } else {
            tracing::warn!("[start_window_drag] no HWND resolved for label={}", label);
        }
    }
    #[cfg(not(target_os = "windows"))]
    crate::ui_tasks::post_start_drag(state, label);
    Ok(serde_json::Value::Null)
}

/// Resolve which agentmux window is under the cursor. Used by the
/// floating-pane re-dock flow: on the floater's mouseup, the
/// frontend asks "what window is the cursor over now?" and, if
/// the answer is another agentmux window in the same process,
/// kicks off `RedockFloatingPane`.
///
/// Args: `{ "x": int, "y": int, "exclude_label": string|null }` —
/// cursor position in the host coordinate space: physical px on Windows
/// (from `get_cursor_point`/`GetCursorPos`), DIP on macOS/Linux (from the
/// DOM drop event's `screenX/Y`; the posScale() rule). `exclude_label` is
/// the source floater's label; without it the hit-test returns the floater
/// itself (it follows the cursor during drag, so it's always topmost at the
/// cursor). Windows walks HWND Z-order front-to-back; macOS/Linux hit-test
/// CEF Views bounds (see `ui_tasks::resolve_window_at_cursor_blocking`).
/// Either way we return the first match that ISN'T the excluded source.
///
/// Returns: `{ "label": string|null, "window_id": string|null }`. Both
/// null means no agentmux window of this process is under the cursor
/// (cursor is over the desktop, an external app, etc.). `window_id`
/// is the backend windowId mapped via the launcher's
/// `BackendWindowIdRegistered` projection — frontend uses it to
/// load the MuxWindow / Workspace and figure out the active tab.
///
/// Instance / version isolation is free: the walk only considers this
/// process's windows, so cross-process drag-over-then-drop won't
/// accidentally re-dock in another instance.
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
        use crate::reducer::DropTarget;
        use windows_sys::Win32::Foundation::RECT;
        use windows_sys::Win32::System::Threading::GetCurrentProcessId;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetClassNameW, GetTopWindow, GetWindow, GetWindowLongW, GetWindowRect,
            GetWindowThreadProcessId, IsWindowVisible, GWL_EXSTYLE, GW_HWNDNEXT, WS_EX_TRANSPARENT,
        };
        // [redock-resolve] permanent diagnostic for the recurring theme-1
        // "main not recognised as a redock target → no landing ghost on main"
        // bug. `muxlog host redock-resolve`. DO NOT REMOVE (see the architecture
        // doc §0 theme 1).
        let class_of = |h: isize| -> String {
            if h == 0 {
                return String::new();
            }
            let mut buf = [0u16; 64];
            let n = GetClassNameW(h as *mut _, buf.as_mut_ptr(), buf.len() as i32);
            if n > 0 {
                String::from_utf16_lossy(&buf[..n as usize])
            } else {
                String::new()
            }
        };

        // The dragged floater follows the cursor, so it is always topmost
        // there: exclude it by HWND. Before its bind lands (a brief race with
        // CreateFloatingWindowTask) fall back to the label-aware lookup
        // start_window_drag uses.
        let exclude_hwnd: Option<isize> = if exclude_label.is_empty() {
            None
        } else {
            state.window_hwnd(exclude_label).or_else(|| {
                let h = resolve_window_hwnd(state, exclude_label);
                if h.is_null() { None } else { Some(h as isize) }
            })
        };

        // The on-screen main frame, independent of the HWND registry (it
        // skips floaters and off-screen pool windows): names a main window
        // whose own bind hasn't landed yet. Resolved once, not per window.
        let main_hwnd: isize = find_main_window() as isize;

        // Walk top-level windows front to back for the first visible window
        // of this process under the cursor that takes a drop, by its
        // registered kind (`AppState::drop_target_at`).
        let our_pid = GetCurrentProcessId();
        let mut hwnd = GetTopWindow(std::ptr::null_mut());
        while !hwnd.is_null() {
            let h_isize = hwnd as isize;
            let mut window_pid: u32 = 0;
            GetWindowThreadProcessId(hwnd, &mut window_pid);
            let mut rect: RECT = std::mem::zeroed();
            // Click-through windows can't take a drop (agentmux creates none
            // today; this keeps a future overlay from swallowing the test).
            let under_cursor = IsWindowVisible(hwnd) != 0
                && window_pid == our_pid
                && exclude_hwnd != Some(h_isize)
                && (GetWindowLongW(hwnd, GWL_EXSTYLE) as u32 & WS_EX_TRANSPARENT) == 0
                && GetWindowRect(hwnd, &mut rect) != 0
                && x >= rect.left
                && x < rect.right
                && y >= rect.top
                && y < rect.bottom;
            if under_cursor {
                let is_main_frame = main_hwnd != 0 && h_isize == main_hwnd;
                let target = state.drop_target_at(h_isize);
                tracing::info!(
                    target: "redock-resolve",
                    hwnd = %format!("{:#x}", h_isize),
                    class = %class_of(h_isize),
                    target = ?target,
                    is_main_hwnd = is_main_frame,
                    main_hwnd = %format!("{:#x}", main_hwnd),
                    "cursor inside window"
                );
                // The label must be the one the target's frontend uses: the
                // redock ghost renders only when the emitted `target_label`
                // equals its `windowLabel` (R3 identity, #1681). A floater, a
                // pool window or an unregistered window is never a target;
                // the walk goes on to the window behind it.
                let label = match target {
                    DropTarget::Window(label) => Some(label),
                    DropTarget::Untracked if is_main_frame => Some("main".to_string()),
                    _ => None,
                };
                if let Some(label) = label {
                    let wid = state.backend_window_id(&label);
                    return Ok(serde_json::json!({
                        "label": label,
                        "window_id": wid,
                    }));
                }
            }
            hwnd = GetWindow(hwnd, GW_HWNDNEXT);
        }
        tracing::info!(
            target: "redock-resolve",
            x, y,
            main_hwnd = %format!("{:#x}", main_hwnd),
            main_class = %class_of(main_hwnd),
            "no agentmux window matched at cursor — no ghost / redock target"
        );
        Ok(serde_json::json!({ "label": null, "window_id": null }))
    }

    // macOS / Linux: CEF Views hit-test. Iterate registered top-level windows
    // on the UI thread, find the top-most one whose DIP bounds contain the
    // (DIP) point — the frontend sends DIP here on macOS/Linux (the posScale()
    // rule), excluding the drag source. Then map label → backend window_id via
    // the same `backend_window_id` projection (populated directly on non-Windows
    // by `register_backend_window`, since there's no launcher). Mirrors the
    // Windows return shape. See
    // docs/analysis/REPORT_MACOS_FLOATING_PANE_REDOCK_2026_05_30.md.
    #[cfg(not(target_os = "windows"))]
    {
        let _ = args;
        match crate::ui_tasks::resolve_window_at_cursor_blocking(state, x, y, exclude_label) {
            Some(label) => {
                let wid = state.backend_window_id(&label);
                Ok(serde_json::json!({ "label": label, "window_id": wid }))
            }
            None => Ok(serde_json::json!({ "label": null, "window_id": null })),
        }
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
    // leaf (not the whole window). The cursor is in the sender's host
    // coordinate space — physical screen px on Windows, DIP on
    // macOS/Linux (the posScale() rule) — and the receiver inverts it
    // accordingly (app-init.ts divides by DPR only on Windows).
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

/// Phase 4b — store the ghost `{ block_id, dir }` for a target window.
///
/// Called by the TARGET window's renderer (`app-init.ts`) each time it
/// computes the drop-zone direction. Passing `block_id: null` (or omitting
/// it) clears the entry for that window so a stale direction is not used
/// if the cursor leaves without a drop.
///
/// Args: `{ "window_label": string, "block_id": string|null, "dir": number|null }`.
pub fn set_floating_redock_target(
    state: &Arc<AppState>,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let window_label = args
        .get("window_label")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    if window_label.is_empty() {
        return Err("set_floating_redock_target: window_label is required".into());
    }
    let block_id = args.get("block_id").and_then(|v| v.as_str()).map(|s| s.to_string());
    let dir = args.get("dir").and_then(|v| v.as_u64()).map(|d| d as u8);

    let mut g = state.floating_redock_ghost.lock();
    match (block_id, dir) {
        (Some(bid), Some(d)) => {
            g.insert(window_label, crate::state::FloatingRedockGhostState { block_id: bid, dir: d });
        }
        _ => {
            g.remove(&window_label);
        }
    }
    Ok(serde_json::Value::Null)
}

/// Phase 4b — consume the ghost state for a target window (read + remove).
///
/// Called by the FLOATER's renderer just before emitting `RedockFloatingPane`
/// so the saga can emit a directional `SplitHorizontal`/`SplitVertical` action
/// instead of a generic `InsertNode`. Consuming (rather than just reading) the
/// entry avoids any stale ghost from a prior drag being applied to a future drop
/// where no new ghost state was set.
///
/// Args: `{ "window_label": string }`.
/// Returns: `{ "block_id": string, "dir": number }` or `{}` if no state stored.
pub fn get_floating_redock_target(
    state: &Arc<AppState>,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let window_label = args
        .get("window_label")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let mut g = state.floating_redock_ghost.lock();
    match g.remove(&window_label) {
        Some(ghost) => Ok(serde_json::json!({
            "block_id": ghost.block_id,
            "dir": ghost.dir,
        })),
        None => Ok(serde_json::json!({})),
    }
}

