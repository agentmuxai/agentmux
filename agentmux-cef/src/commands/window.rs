// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Window management commands for the CEF host.
// Ported from src-tauri/src/commands/window.rs.
//
// Phase 2: Single-window only. Multi-window commands are stubbed.

use std::sync::Arc;

use cef::{ImplBrowser, ImplBrowserHost};

use crate::state::AppState;

/// Get the current zoom factor.
pub fn get_zoom_factor(state: &Arc<AppState>) -> serde_json::Value {
    let factor = *state.zoom_factor.lock();
    serde_json::json!(factor)
}

/// Set the zoom factor.
/// CEF zoom uses a logarithmic scale: zoom_level = log2(zoom_factor)
/// So factor 1.0 = level 0, factor 2.0 = level 1, factor 0.5 = level -1
pub fn set_zoom_factor(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let factor = args
        .get("factor")
        .and_then(|v| v.as_f64())
        .ok_or_else(|| "Missing factor".to_string())?;

    let factor = factor.clamp(0.5, 3.0);
    *state.zoom_factor.lock() = factor;

    // Convert to CEF zoom level (log base 1.2)
    // CEF uses: zoom_factor = 1.2 ^ zoom_level
    // So: zoom_level = log(zoom_factor) / log(1.2)
    let zoom_level = factor.ln() / 1.2_f64.ln();

    // NOTE: host.set_zoom_level() deadlocks from IPC thread, and post_task
    // crashes with current CEF bindings. Zoom is applied via CSS on the frontend.
    // The zoom_factor state is stored for get_zoom_factor queries.

    // Emit zoom-factor-change event
    crate::events::emit_event_from_state(state, "zoom-factor-change", &serde_json::json!(factor));

    Ok(serde_json::Value::Null)
}

/// Close the window. Args: optional `{ "label": string }`; defaults to "main".
/// Routes by label via `resolve_window_hwnd` — without that the floater
/// (owned, drawn above its owner in Z-order) would always swallow the
/// close because `find_own_top_level_window` returns the topmost-Z
/// visible top-level of the process.
pub fn close_window(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("main");

    #[cfg(target_os = "windows")]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let hwnd = resolve_window_hwnd(state, label);
        if !hwnd.is_null() {
            PostMessageW(hwnd, WM_CLOSE, 0, 0);
            return Ok(serde_json::Value::Null);
        }
    }
    #[cfg(not(target_os = "windows"))]
    crate::ui_tasks::post_close_window(state, label);
    let _ = (state, label);
    Ok(serde_json::Value::Null)
}

/// Close a specific window by label. Used by the tear-off Phase 4
/// merge path: after the candidate window pulls the dragged tab into
/// its own workspace via MoveTabToWorkspace, the dragged window is
/// empty and should be destroyed. Posts WM_CLOSE on Win32; uses the
/// existing UI-thread close task on other platforms.
pub fn close_window_by_label(
    state: &Arc<AppState>,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let label = args
        .get("label")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing label".to_string())?
        .to_string();

    #[cfg(target_os = "windows")]
    unsafe {
        use cef::{ImplBrowser, ImplBrowserHost};
        use windows_sys::Win32::UI::WindowsAndMessaging::{PostMessageW, WM_CLOSE};
        // Phase H.2.b — reducer-aware lookup with fallback.
        if let Some(browser) = state.get_browser(&label) {
            if let Some(host) = browser.host() {
                let hwnd = host.window_handle();
                if !hwnd.0.is_null() {
                    PostMessageW(hwnd.0 as *mut std::ffi::c_void, WM_CLOSE, 0, 0);
                    return Ok(serde_json::Value::Null);
                }
            }
        }
        return Err(format!("no top-level HWND for label {}", label));
    }

    #[cfg(not(target_os = "windows"))]
    {
        crate::ui_tasks::post_close_window(state, &label);
        Ok(serde_json::Value::Null)
    }
}

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

/// Resolve a top-level HWND for the given label. Prefer the reducer
/// registry (`state.get_browser(label)` → host → window_handle → walk
/// to root) over the process-wide `find_own_top_level_window` fallback.
///
/// Why the label matters: `find_own_top_level_window` does an
/// `EnumWindows` and returns the *first* visible top-level of the
/// current process. Z-order puts **owned** windows ABOVE their owner,
/// so as soon as a floating-pane window exists, every label-less
/// `get/set_window_position` call (e.g. the main window's
/// `useWindowDrag`) accidentally targets the floater — dragging the
/// main window moves the floater instead.
///
/// `GetAncestor(hwnd, GA_ROOT)` guard handles the case where CEF
/// returns the embedded browser's WS_CHILD HWND rather than our
/// outer top-level — without it, `SetWindowPos` would only shift the
/// child within its parent, not move the outer floater.
/// Class name of the floating-pane outer HWND
/// (`agentmux-cef/src/floating_pane.rs::CLASS_NAME`). Kept in sync so
/// `find_main_window` can EnumWindows-skip floaters when CEF Views
/// hides the main window's HWND.
#[cfg(target_os = "windows")]
const FLOATING_PANE_CLASS_NAME: &str = "AgentMuxFloatingPane";

/// Like `find_own_top_level_window` but skips floating-pane windows.
/// Used when the label points at the main window but the reducer-
/// registry path failed (CEF Views' `BrowserHost::window_handle()`
/// returns NULL on Win32 for Views-based browsers). Without the
/// skip, the floater (owned, drawn ABOVE its owner) would be
/// enumerated first and we'd target it instead.
#[cfg(target_os = "windows")]
unsafe fn find_main_window() -> *mut std::ffi::c_void {
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::*;

    let _pid = GetCurrentProcessId();
    let mut result: *mut std::ffi::c_void = std::ptr::null_mut();

    unsafe extern "system" fn enum_callback(
        hwnd: *mut std::ffi::c_void,
        lparam: isize,
    ) -> i32 {
        use windows_sys::Win32::System::Threading::GetCurrentProcessId;
        let mut window_pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut window_pid);
        if window_pid != GetCurrentProcessId() {
            return 1;
        }
        if IsWindowVisible(hwnd) == 0 {
            return 1;
        }
        // Read the window class name; skip if it matches the floating-
        // pane class. `GetClassNameW` writes up to `cchClassMaxCount`
        // UTF-16 code units (excluding the null terminator).
        let mut buf: [u16; 64] = [0; 64];
        let len = GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        if len > 0 {
            let class = String::from_utf16_lossy(&buf[..len as usize]);
            if class == FLOATING_PANE_CLASS_NAME {
                return 1;
            }
        }
        let result_ptr = lparam as *mut *mut std::ffi::c_void;
        *result_ptr = hwnd;
        0
    }

    EnumWindows(Some(enum_callback), &mut result as *mut _ as isize);
    result
}

#[cfg(target_os = "windows")]
unsafe fn resolve_window_hwnd(state: &Arc<AppState>, label: &str) -> *mut std::ffi::c_void {
    use cef::ImplBrowser;
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetAncestor, GA_ROOT};

    if !label.is_empty() {
        // 1. Try the CEF reducer registry — works for any label whose
        //    `host.window_handle()` exposes the OS HWND (notably our
        //    floating-pane windows created via CreateWindowExW +
        //    set_as_child). For CEF Views (main window) this returns
        //    NULL on Win32 and we fall through.
        if let Some(browser) = state.get_browser(label) {
            if let Some(host) = browser.host() {
                let raw = host.window_handle().0 as *mut std::ffi::c_void;
                if !raw.is_null() {
                    let root = GetAncestor(raw, GA_ROOT);
                    let resolved = if root.is_null() { raw } else { root };
                    tracing::info!(
                        target: "win-resolve",
                        label = %label,
                        host_hwnd = ?raw,
                        root_hwnd = ?resolved,
                        "[win-resolve] resolved via reducer registry"
                    );
                    return resolved;
                }
            }
        }

        // 2. Consult the authoritative per-label HWND cache populated
        //    by `capture_hwnd_for_label` (triggered when the frontend
        //    signals init-complete via `set_window_init_status`). This
        //    is the same source `set_window_transparency` uses
        //    (line 447) and covers the case where `host.window_handle()`
        //    returns NULL but we'd previously stamped the HWND.
        let cached = state.window_hwnds.lock().get(label).copied();
        if let Some(raw_isize) = cached {
            let raw = raw_isize as *mut std::ffi::c_void;
            if !raw.is_null() {
                let root = GetAncestor(raw, GA_ROOT);
                let resolved = if root.is_null() { raw } else { root };
                tracing::info!(
                    target: "win-resolve",
                    label = %label,
                    cache_hwnd = ?raw,
                    root_hwnd = ?resolved,
                    "[win-resolve] resolved via window_hwnds cache"
                );
                return resolved;
            }
        }

        tracing::warn!(
            target: "win-resolve",
            label = %label,
            "[win-resolve] reducer-registry + cache both empty — using class-aware EnumWindows fallback"
        );
    }

    // 3. EnumWindows last resort. CEF Views (main window) hides its
    //    HWND behind a Views container, so this branch fires for
    //    "main" before the user has triggered the init-status path
    //    (e.g. cold-boot drag attempts). Z-order returns the floater
    //    first (owned windows draw ABOVE their owner), so for "main"
    //    we use a class-aware enumerator that skips the floating-pane
    //    window class — deterministic regardless of Z-order. For
    //    non-"main" labels with neither cache nor registry entry,
    //    plain `find_own_top_level_window` is the best we can do.
    let fallback = if label == "main" {
        find_main_window()
    } else {
        find_own_top_level_window()
    };
    tracing::info!(
        target: "win-resolve",
        label = %label,
        fallback_hwnd = ?fallback,
        "[win-resolve] class-aware EnumWindows fallback"
    );
    fallback
}

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

/// Initiate window drag (for frameless windows).
/// Windows: sends WM_NCLBUTTONDOWN/HTCAPTION via Win32 — find_own_top_level_window
/// resolves the per-process HWND so multi-window works without a label.
/// Linux/macOS: dispatches CefWindow::BeginWindowDrag on the UI thread; needs
/// the source window's label so non-main windows drag themselves rather than
/// the main window. Frontend reads `?windowLabel=…` from its URL and passes
/// it here; missing → "main" for backward compatibility.
pub fn start_window_drag(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    #[cfg(target_os = "windows")]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
        let hwnd = find_own_top_level_window();
        if !hwnd.is_null() {
            ReleaseCapture();
            SendMessageW(hwnd, WM_NCLBUTTONDOWN, 2 /* HTCAPTION */, 0);
            return Ok(serde_json::Value::Null);
        }
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
        use windows_sys::Win32::Foundation::{POINT, RECT};
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
                            // Found a window we own but it isn't in
                            // window_hwnds yet (very early startup or
                            // a window we don't track). Treat as "no
                            // agentmux match" and continue Z-order
                            // walk in case a tracked window sits
                            // behind it.
                        }
                    }
                }
            }
            hwnd = GetWindow(hwnd, GW_HWNDNEXT);
            // Bound the walk defensively against EnumWindow loops on
            // pathological window managers (shouldn't happen on Win32
            // but cheap).
            let _ = POINT { x, y };
        }
        let _ = exclude_label;
        Ok(serde_json::json!({ "label": null, "window_id": null }))
    }

    #[cfg(not(target_os = "windows"))]
    {
        let _ = (state, args, x, y, exclude_label);
        Ok(serde_json::json!({ "label": null, "window_id": null }))
    }
}

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

    let _ = state;
    #[cfg(not(target_os = "windows"))]
    let _ = (transparent, opacity);

    Ok(serde_json::Value::Null)
}

/// Find ALL visible top-level windows belonging to this process.
#[cfg(target_os = "windows")]
fn find_all_own_windows() -> Vec<*mut std::ffi::c_void> {
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;

    let mut results: Vec<*mut std::ffi::c_void> = Vec::new();

    unsafe extern "system" fn enum_callback(
        hwnd: *mut std::ffi::c_void,
        lparam: isize,
    ) -> i32 {
        use windows_sys::Win32::System::Threading::GetCurrentProcessId;
        let mut window_pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut window_pid);
        if window_pid == GetCurrentProcessId() && IsWindowVisible(hwnd) != 0 {
            let results = &mut *(lparam as *mut Vec<*mut std::ffi::c_void>);
            results.push(hwnd);
        }
        1 // Continue
    }

    unsafe {
        EnumWindows(Some(enum_callback), &mut results as *mut _ as isize);
    }
    results
}

/// Find the top-level window belonging to this process.
/// In CEF Views mode, browser.host().window_handle() returns NULL,
/// so we enumerate windows and find ours by process ID.
#[cfg(target_os = "windows")]
pub(crate) unsafe fn find_own_top_level_window() -> *mut std::ffi::c_void {
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;

    let pid = GetCurrentProcessId();
    let mut result: *mut std::ffi::c_void = std::ptr::null_mut();

    unsafe extern "system" fn enum_callback(
        hwnd: *mut std::ffi::c_void,
        lparam: isize,
    ) -> i32 {
        use windows_sys::Win32::System::Threading::GetCurrentProcessId;
        let mut window_pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut window_pid);
        if window_pid == GetCurrentProcessId() && IsWindowVisible(hwnd) != 0 {
            // Store the HWND in the pointer passed via lparam
            let result_ptr = lparam as *mut *mut std::ffi::c_void;
            *result_ptr = hwnd;
            return 0; // Stop enumeration
        }
        1 // Continue
    }

    let _ = pid; // Used inside callback via GetCurrentProcessId()
    EnumWindows(
        Some(enum_callback),
        &mut result as *mut _ as isize,
    );
    result
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

/// Get the current window label.
/// The frontend passes its own label (extracted from URL params) as an arg.
pub fn get_window_label(args: &serde_json::Value) -> serde_json::Value {
    let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("main");
    serde_json::json!(label)
}

/// Check if this is the main window.
/// The frontend passes its own label (extracted from URL params) as an arg.
pub fn is_main_window(args: &serde_json::Value) -> serde_json::Value {
    let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("main");
    serde_json::json!(label == "main")
}

/// Return the OS double-click interval in milliseconds.
/// On Windows: `GetDoubleClickTime()` — typically 500ms, user-configurable
/// via Mouse settings. On non-Windows: hardcoded 500ms (the Win32 default,
/// also a common cross-platform default; Phase 7 can refine per platform).
///
/// Used by the InstancePanel to defer single-click focus past the user's
/// dblclick threshold so dblclick-to-rename works for everyone, not just
/// users with the default-or-faster setting. Without this query, a fixed
/// constant would make rename unreliable for slow double-clickers
/// (codex PR #569 round-2 P2).
pub fn get_double_click_time() -> serde_json::Value {
    #[cfg(target_os = "windows")]
    {
        let ms = unsafe { windows_sys::Win32::UI::Input::KeyboardAndMouse::GetDoubleClickTime() };
        serde_json::json!(ms)
    }
    #[cfg(not(target_os = "windows"))]
    {
        serde_json::json!(500u32)
    }
}

/// List all open window instances with their backend window IDs.
/// Same filtering as `list_windows` (excludes unpromoted pool windows
/// and browser-pane child HWNDs), but returns `[{label, windowId}]`
/// pairs so the frontend can resolve per-window backend objects
/// (Window record → meta["window:displayname"], etc.) without an
/// extra round-trip per row.
///
/// `windowId` is `None` for windows that haven't yet completed the
/// `register_backend_window` round-trip — typically a freshly-spawned
/// window before its frontend has finished init. Callers should
/// fall back to label/index-based naming in that case.
pub fn list_window_instances(state: &Arc<AppState>) -> serde_json::Value {
    // Atomic snapshot — pool inventory + browsers under ONE lock.
    // Two-lock variants race against `promote_pool_window` between
    // the reads and would let a just-promoted user window be
    // excluded (or admit a still-hidden pool window).
    let (pool_labels, browsers) = state.user_visibility_snapshot();
    let labels: Vec<String> = browsers
        .into_iter()
        .map(|(l, _)| l)
        .filter(|l| !pool_labels.contains(l.as_str()) && !l.starts_with("browser-pane-"))
        .collect();
    // Read backend window IDs via `state.backend_window_id()`,
    // which queries the launcher-fed `shadow_backend_window_ids`
    // (sole source of truth post-B.5e). Resolve labels OUTSIDE
    // the browsers lock to avoid nesting (browsers + shadow).
    let entries: Vec<serde_json::Value> = labels
        .iter()
        .map(|l| {
            serde_json::json!({
                "label": l,
                "windowId": state.backend_window_id(l),
            })
        })
        .collect();
    serde_json::json!(entries)
}

/// List all open window labels, excluding unpromoted pool windows.
/// Pool windows are pre-warmed tear-off scratch windows kept hidden
/// from the user (WS_EX_TOOLWINDOW, no taskbar entry). Including them
/// in `list_windows` inflates the frontend's InstancePanel row count
/// with phantom entries the user can't see or focus.
///
/// Uses `state.user_visibility_snapshot()` — atomic read of pool
/// inventory (`unpromoted` ∪ `pool.queue`) and the browser registry
/// under one host_state lock. Both `unpromoted` (populated at spawn
/// time) and `pool.queue` (populated after renderer-ready, before
/// promote) are host-internal: the window is hidden off-screen and
/// has no UI a user could see or focus. The atomic read is required
/// because a two-lock variant races against `promote_pool_window`.
pub fn list_windows(state: &Arc<AppState>) -> serde_json::Value {
    let (pool_labels, browsers) = state.user_visibility_snapshot();
    let labels: Vec<String> = browsers
        .into_iter()
        .map(|(l, _)| l)
        .filter(|l| !pool_labels.contains(l.as_str()))
        .collect();
    serde_json::json!(labels)
}

/// Focus a specific window by label.
///
/// Uses the CEF Views `Window::activate()` API on all platforms (via
/// `post_focus_window` → `FocusWindowTask`). On Windows in Views mode,
/// `browser.host().window_handle()` returns NULL — the previous direct
/// SetForegroundWindow path silently failed there. Views' `activate()`
/// resolves the actual top-level HWND through `browser_view_get_for_browser
/// → window()` which is the only correct way to reach it in Views mode.
pub fn focus_window(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("main");
    crate::ui_tasks::post_focus_window(state, label);
    Ok(serde_json::Value::Null)
}

/// Get the instance number for the current window.
///
/// Reads from `state.instance_num()` which queries the launcher-fed
/// `shadow_instance_registry` (B.5e — sole source of truth post-migration).
/// Brief race window for early lookups: see `app-init.ts` retry logic.
pub fn get_instance_number(state: &Arc<AppState>, args: &serde_json::Value) -> serde_json::Value {
    let label = args
        .get("label")
        .and_then(|v| v.as_str())
        .unwrap_or("main");
    serde_json::json!(state.instance_num(label).unwrap_or(1))
}

/// Register the backend window ID for a window label.
/// Called by the frontend after it has initialized its backend Window object.
/// Used by `on_before_close` to notify the backend when a secondary window closes.
pub fn register_backend_window(_state: &Arc<AppState>, args: &serde_json::Value) -> serde_json::Value {
    let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("main");
    let window_id = args.get("window_id").and_then(|v| v.as_str()).unwrap_or("");
    tracing::info!(label = %label, window_id = %window_id, "[window] register_backend_window received");
    crate::client::dlog(&format!("register_backend_window: label={} window_id={}", label, window_id));
    if !window_id.is_empty() {
        // Phase B.5 (window_id_map step d) — host no longer mutates
        // `window_id_map` locally. The launcher's
        // `state.backend_window_ids` (B.5 step a) is sole authority;
        // we just send the report and the shadow update populates
        // the host-side projection.
        tracing::info!(label = %label, window_id = %window_id, "[window] registered backend window ID");
        crate::launcher_ipc::report_backend_window_id_registered(
            label.to_string(),
            window_id.to_string(),
        );
        // Phase B.7.3.3 — the launcher's
        // `Event::BackendWindowIdRegistered` (delivered via the CEF
        // JS bridge) carries the label → windowId mapping change to
        // every renderer's reducer. No sync emit here.
    } else {
        tracing::warn!(label = %label, "[window] register_backend_window called with empty window_id — skipped");
    }
    serde_json::Value::Null
}

/// Toggle DevTools for the main window.
///
/// Uses CEF's native show_dev_tools() API, which triggers
/// BrowserViewDelegate::on_popup_browser_view_created with is_devtools=1.
/// That callback creates a top-level CefWindow with a native title bar,
/// producing a standalone DevTools window — identical to Tauri's open_devtools().
pub fn toggle_devtools(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("main");
    crate::ui_tasks::post_show_dev_tools(state, label);
    Ok(serde_json::Value::Null)
}

/// Open DevTools focused on the element at the given window-relative
/// coordinates. Equivalent to Chrome's right-click → Inspect Element.
/// Used by the pane context menu's Inspect entry.
pub fn inspect_element_at(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("main");
    let x = args.get("x").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let y = args.get("y").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    crate::ui_tasks::post_inspect_element_at(state, label, x, y);
    Ok(serde_json::Value::Null)
}

/// Resolve the dev Vite port. Honors `AGENTMUX_VITE_PORT` (set by
/// `Taskfile.yml`'s `dev:serve` task when the per-clone deterministic
/// port differs from 5173); falls back to 5173 otherwise. Without this,
/// every child window (pool warmups, tab tear-off, floating pane) loads
/// `localhost:5173` and hits `ERR_CONNECTION_REFUSED` on any other port —
/// only the main window survives because the launcher passes `--url=…`
/// on the CLI. See `docs/analyses/ANALYSIS_DEV_VITE_PORT_HARDCODE_2026-05-26.md`.
fn dev_vite_port() -> u16 {
    std::env::var("AGENTMUX_VITE_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(5173)
}

/// Resolve the base URL for the frontend.
/// Production: IPC server serves static files from `frontend/` next to the exe.
/// Dev: Vite dev server at `http://localhost:<AGENTMUX_VITE_PORT or 5173>`.
pub(crate) fn resolve_frontend_base_url(ipc_port: u16) -> String {
    // Detect dev mode. Two reachable scenarios:
    //   a) Launcher-managed: AGENTMUX_RUNTIME_MODE is set, from_env()
    //      returns Some.
    //   b) Standalone `task dev`: env absent. Fall through to
    //      RuntimeMode::current() against the host exe path so the
    //      same `dist/cef-dev/` build dir → Dev classification fires
    //      that the launcher would have used.
    let mode = agentmux_common::RuntimeMode::from_env().or_else(|| {
        std::env::current_exe()
            .ok()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .map(|d| agentmux_common::RuntimeMode::current(&d))
    });
    if matches!(mode, Some(agentmux_common::RuntimeMode::Dev { .. })) {
        return format!("http://localhost:{}", dev_vite_port());
    }
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(|d| d.to_path_buf()));
    let has_frontend = exe_dir
        .as_ref()
        .map(|d| d.join("frontend/index.html").exists())
        .unwrap_or(false);
    if has_frontend {
        format!("http://127.0.0.1:{}", ipc_port)
    } else {
        format!("http://localhost:{}", dev_vite_port())
    }
}

/// Open a new full AgentMux instance (status-bar version click, Ctrl+Shift+N,
/// second `agentmux.exe` launch). Independent top-level window, own taskbar
/// entry, independent lifecycle. See
/// `docs/specs/SPEC_MULTIWINDOW_TASKBAR_GROUPING.md`.
pub fn open_new_window(state: &Arc<AppState>) -> Result<serde_json::Value, String> {
    open_window_with_kind(state, crate::state::WindowKind::FullInstance, None)
}

/// Open a sub-window tied to `parent_instance_id`. **Not exposed to users** —
/// reserved for agent / backend callers that need a transient auxiliary
/// top-level window (tool-spawned panels, diff views, etc.). Sub-windows are
/// hidden from the taskbar via `ITaskbarList::DeleteTab` and close when their
/// parent full instance closes.
pub fn open_subwindow(
    state: &Arc<AppState>,
    parent_instance_id: String,
) -> Result<serde_json::Value, String> {
    // Reject if the parent isn't a known live FullInstance — prevents
    // orphan sub-windows and enforces the lifecycle rule in the spec.
    //
    // Phase B.5 (window_meta step d, refined twice):
    // * Round-1 fix used `state.window_meta()` (shadow-first), which
    //   covered the task-dev-mode regression but allowed a NEW orphan
    //   bug: shadow lags on close, so during the gap between host's
    //   sync `on_before_close` removal and the launcher's async
    //   `WindowClosed` event arrival, this check could still see a
    //   already-closing parent. (codex P2 PR #592 round-2.)
    // * Refined: read host_meta DIRECTLY for this liveness check.
    //   Host_meta is synchronously written in on_after_created and
    //   removed in on_before_close (per the round-2 step-d
    //   refinement keeping host_meta as a sync cache), so it
    //   correctly reflects "is the parent currently open" without
    //   shadow's async lag. Works in `task dev` mode too (host_meta
    //   populated by on_after_created regardless of launcher
    //   presence).
    let parent_ok = state
        .window_meta
        .lock()
        .get(&parent_instance_id)
        .map(|m| m.kind == crate::state::WindowKind::FullInstance)
        .unwrap_or(false);
    if !parent_ok {
        return Err(format!(
            "open_subwindow: unknown or non-full-instance parent label={parent_instance_id}"
        ));
    }
    open_window_with_kind(
        state,
        crate::state::WindowKind::Subwindow,
        Some(parent_instance_id),
    )
}

fn open_window_with_kind(
    state: &Arc<AppState>,
    kind: crate::state::WindowKind,
    parent_instance_id: Option<String>,
) -> Result<serde_json::Value, String> {
    // PR #6 H.7 — refuse top-level creation while any pane is mid-close.
    // See `SPEC_WINDOW_FLEET_REDUCER_2026-05-02.md` and the smoke retro
    // at `docs/retro/smoke-test-0.33.586-and-pr5-plan-2026-05-02.md`:
    // creating a top-level CEF window while a pane is in `Closing` hits
    // a Chromium v146 deadlock (HiddenSinceOpen + IPC backpressure)
    // that wedges the message loop. Frontend should retry on next tick.
    if state.any_browser_pane_closing() {
        tracing::warn!(
            target: "wfr:gate",
            "[wfr:gate] open_window refused — pane is mid-close (H.7 invariant)"
        );
        return Err("a pane is currently closing; retry shortly".to_string());
    }

    let window_id = uuid::Uuid::new_v4();
    let label = format!("window-{}", window_id.simple());

    let ipc_port = *state.ipc_port.lock();
    let ipc_token = &state.ipc_token;
    let base_url = resolve_frontend_base_url(ipc_port);

    let separator = if base_url.contains('?') { "&" } else { "?" };
    let url = format!(
        "{}{}ipc_port={}&ipc_token={}&windowLabel={}",
        base_url, separator, ipc_port, ipc_token, label
    );

    tracing::info!(label = %label, kind = ?kind, parent = ?parent_instance_id, "[window] open window");

    // Phase B.5 (window_meta step d) — push the pre-create handoff
    // (label + kind + parent). Replaces the previous parallel
    // `window_meta.insert` + `pending_window_labels.push` pair.
    let (pos_x, pos_y) = get_offset_position();
    let (win_w, win_h) = get_secondary_window_size(pos_x, pos_y);

    // Phase F.1 — routed through the host reducer.
    state.host_dispatch(
        crate::reducer::HostCommand::EnqueuePendingWindowCreation {
            entry: crate::state::PendingWindowCreation {
                label: label.clone(),
                kind,
                parent_instance_id,
            },
        },
    );

    // Post to CEF UI thread — window_create_top_level must run there.
    // true = frameless: secondary app windows use the same custom title bar as main.
    crate::ui_tasks::post_create_window(
        state, &url, &label, pos_x, pos_y, win_w, win_h,
        true,
    );

    // Phase B.7.3.3 — typed launcher events drive InstancePanel
    // atoms via the CEF JS bridge; no sync emit here.

    Ok(serde_json::json!(label))
}

/// Get an offset position for a new window: 30px right and 30px down from the current window.
fn get_offset_position() -> (i32, i32) {
    #[cfg(target_os = "windows")]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        use windows_sys::Win32::Foundation::RECT;
        let hwnd = find_own_top_level_window();
        if !hwnd.is_null() {
            let mut rect: RECT = std::mem::zeroed();
            GetWindowRect(hwnd, &mut rect);
            return (rect.left + 30, rect.top + 30);
        }
    }
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::CW_USEDEFAULT;
        return (CW_USEDEFAULT, CW_USEDEFAULT);
    }
    #[cfg(not(target_os = "windows"))]
    (100, 100)
}

/// Compute 70% of the monitor's work area for a secondary window at (px, py).
/// Falls back to 1200x800 if the monitor can't be determined.
fn get_secondary_window_size(px: i32, py: i32) -> (i32, i32) {
    #[cfg(target_os = "windows")]
    {
        use crate::app::get_monitor_work_area;
        if let Some((_x, _y, work_w, work_h)) = get_monitor_work_area(px, py) {
            return ((work_w as f64 * 0.70) as i32, (work_h as f64 * 0.70) as i32);
        }
    }
    (1200, 800)
}

// ── Per-window opacity (SPEC_PER_WINDOW_OPACITY_2026-05-14.md) ───────────────

/// Capture and store the HWND for `label` in `AppState::window_hwnds`.
///
/// Called from `set_window_init_status` once the frontend signals "ready".
/// Two-pass approach:
/// 1. Fast path: `browser.host().window_handle()` — may be non-NULL by this
///    point even in CEF Views mode (window fully shown).
/// 2. Fallback: enumerate all process-owned visible HWNDs and pick the one
///    not yet registered in `window_hwnds`. Reliable because windows are
///    opened sequentially (pool windows are hidden before promotion).
#[cfg(target_os = "windows")]
pub(crate) fn capture_hwnd_for_label(state: &Arc<AppState>, label: &str) {
    use cef::ImplBrowserHost;
    // Fast path.
    if let Some(mut browser) = state.get_browser(label) {
        if let Some(host) = browser.host() {
            let hwnd = host.window_handle();
            if !hwnd.0.is_null() {
                state.window_hwnds.lock().insert(label.to_string(), hwnd.0 as isize);
                tracing::debug!("[opacity] captured hwnd fast-path label={} hwnd={:#x}", label, hwnd.0 as isize);
                return;
            }
        }
    }
    // Fallback: pick the first visible HWND not already mapped.
    let known: std::collections::HashSet<isize> = state.window_hwnds.lock().values().cloned().collect();
    for hwnd_raw in find_all_own_windows() {
        let raw = hwnd_raw as isize;
        if !known.contains(&raw) {
            state.window_hwnds.lock().insert(label.to_string(), raw);
            tracing::debug!("[opacity] captured hwnd fallback label={} hwnd={:#x}", label, raw);
            return;
        }
    }
    tracing::warn!("[opacity] capture_hwnd_for_label: no available HWND for label={}", label);
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
