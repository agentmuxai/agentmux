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

/// Close the window.
pub fn close_window(state: &Arc<AppState>) -> Result<serde_json::Value, String> {
    #[cfg(target_os = "windows")]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let hwnd = find_own_top_level_window();
        if !hwnd.is_null() {
            PostMessageW(hwnd, WM_CLOSE, 0, 0);
            return Ok(serde_json::Value::Null);
        }
    }
    #[cfg(not(target_os = "windows"))]
    crate::ui_tasks::post_close_window(state, "main");
    let _ = state;
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
        let browsers = state.browsers.lock();
        if let Some(browser) = browsers.get(&label) {
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

/// Minimize the window.
pub fn minimize_window(state: &Arc<AppState>) -> Result<serde_json::Value, String> {
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
    crate::ui_tasks::post_minimize_window(state, "main");
    let _ = state;
    Ok(serde_json::Value::Null)
}

/// Maximize/unmaximize the window (toggle).
pub fn maximize_window(state: &Arc<AppState>) -> Result<serde_json::Value, String> {
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
    crate::ui_tasks::post_maximize_window(state, "main");
    let _ = state;
    Ok(serde_json::Value::Null)
}

/// Get the current window position on screen.
pub fn get_window_position(state: &Arc<AppState>) -> Result<serde_json::Value, String> {
    #[cfg(target_os = "windows")]
    unsafe {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        use windows_sys::Win32::Foundation::RECT;
        let hwnd = find_own_top_level_window();
        if !hwnd.is_null() {
            let mut rect: RECT = std::mem::zeroed();
            GetWindowRect(hwnd, &mut rect);
            return Ok(serde_json::json!({ "x": rect.left, "y": rect.top }));
        }
    }
    let _ = state;
    Ok(serde_json::json!({ "x": 0, "y": 0 }))
}

/// Move the window by a delta (dx, dy) from its current position.
pub fn move_window_by(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let dx = args.get("dx").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
    let dy = args.get("dy").and_then(|v| v.as_i64()).unwrap_or(0) as i32;

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
                0x0014, // SWP_NOZORDER | SWP_NOSIZE
            );
            return Ok(serde_json::Value::Null);
        }
    }
    #[cfg(not(target_os = "windows"))]
    crate::ui_tasks::post_move_window(state, "main", dx, dy);
    let _ = state;
    Ok(serde_json::Value::Null)
}

/// Initiate window drag (for frameless windows).
/// Windows: sends WM_NCLBUTTONDOWN/HTCAPTION via Win32.
/// Linux/macOS: delegates to CEF Views Window::drag_move() on the UI thread.
pub fn start_window_drag(state: &Arc<AppState>) -> Result<serde_json::Value, String> {
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
    crate::ui_tasks::post_start_drag(state, "main");
    let _ = state;
    Ok(serde_json::Value::Null)
}

/// Set window transparency/blur effects.
/// Uses DWM Mica/Acrylic on Win11, or SetWindowCompositionAttribute on Win10.
pub fn set_window_transparency(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let transparent = args.get("transparent").and_then(|v| v.as_bool()).unwrap_or(false);
    let opacity = args.get("opacity").and_then(|v| v.as_f64()).unwrap_or(0.8);
    tracing::info!("set_window_transparency: transparent={} opacity={}", transparent, opacity);

    #[cfg(target_os = "windows")]
    {
        // Collect all visible HWNDs for this process, then apply opacity.
        let hwnds = find_all_own_windows();
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
    let pool_labels = state.unpromoted_pool_labels.lock().clone();
    let window_id_map = state.window_id_map.lock();
    let browsers = state.browsers.lock();
    let entries: Vec<serde_json::Value> = browsers
        .keys()
        .filter(|l| !pool_labels.contains(*l) && !l.starts_with("browser-pane-"))
        .map(|l| {
            serde_json::json!({
                "label": l,
                "windowId": window_id_map.get(l),
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
/// Use `state.unpromoted_pool_labels` (NOT `state.window_pool`) as the
/// "is unpromoted pool" oracle — the pool queue is only populated
/// after the renderer-ready handshake (~100 ms after spawn), so it
/// would miss freshly-spawned pool windows during the gap.
/// `unpromoted_pool_labels` is populated synchronously in
/// `spawn_pool_window` and removed in `promote_pool_window` /
/// `on_pool_window_destroyed`.
pub fn list_windows(state: &Arc<AppState>) -> serde_json::Value {
    let pool_labels = state.unpromoted_pool_labels.lock().clone();
    let browsers = state.browsers.lock();
    let labels: Vec<&String> = browsers
        .keys()
        .filter(|l| !pool_labels.contains(*l))
        .collect();
    serde_json::json!(labels)
}

/// Focus a specific window by label.
pub fn focus_window(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("main");

    #[cfg(target_os = "windows")]
    {
        let browsers = state.browsers.lock();
        if let Some(browser) = browsers.get(label) {
            if let Some(host) = browser.host() {
                let hwnd = host.window_handle();
                if !hwnd.0.is_null() {
                    unsafe {
                        windows_sys::Win32::UI::WindowsAndMessaging::SetForegroundWindow(
                            hwnd.0 as *mut std::ffi::c_void,
                        );
                    }
                    return Ok(serde_json::Value::Null);
                }
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    crate::ui_tasks::post_focus_window(state, label);
    let _ = (state, label);
    Ok(serde_json::Value::Null)
}

/// Get the instance number for the current window.
///
/// Phase B.5c — switched from direct `window_instance_registry.lock()`
/// access to `state.instance_num()`, which prefers the
/// launcher-authoritative `shadow_instance_registry` and falls back
/// to host's local registry only for the brief race window where
/// host has registered locally but the launcher's
/// `WindowInstanceAssigned` event hasn't returned yet.
pub fn get_instance_number(state: &Arc<AppState>, args: &serde_json::Value) -> serde_json::Value {
    let label = args
        .get("label")
        .and_then(|v| v.as_str())
        .unwrap_or("main");
    serde_json::json!(state.instance_num(label).unwrap_or(1))
}

/// Get the total window count.
///
/// Phase B.5c — uses `state.instance_count()` which reads from the
/// launcher-authoritative shadow.
pub fn get_window_count(state: &Arc<AppState>) -> serde_json::Value {
    serde_json::json!(state.instance_count())
}

/// Register the backend window ID for a window label.
/// Called by the frontend after it has initialized its backend Window object.
/// Used by `on_before_close` to notify the backend when a secondary window closes.
pub fn register_backend_window(state: &Arc<AppState>, args: &serde_json::Value) -> serde_json::Value {
    let label = args.get("label").and_then(|v| v.as_str()).unwrap_or("main");
    let window_id = args.get("window_id").and_then(|v| v.as_str()).unwrap_or("");
    tracing::info!(label = %label, window_id = %window_id, "[window] register_backend_window received");
    crate::client::dlog(&format!("register_backend_window: label={} window_id={}", label, window_id));
    if !window_id.is_empty() {
        state.window_id_map.lock().insert(label.to_string(), window_id.to_string());
        let keys: Vec<String> = state.window_id_map.lock().keys().cloned().collect();
        crate::client::dlog(&format!("window_id_map now has keys: {:?}", keys));
        tracing::info!(label = %label, window_id = %window_id, "[window] registered backend window ID");
        // Notify listeners that the label→windowId mapping changed.
        // The InstancePanel needs `windowId` to look up the backend
        // Window record (display name in meta, workspace fallback).
        // Without this emit, sibling windows would never re-fetch
        // listWindowInstances after a freshly-opened window's
        // windowId becomes available, leaving its row's name
        // unresolvable and rename disabled. (codex PR #569 P2)
        let count = state.instance_count();
        crate::events::emit_event_all_windows(
            state,
            "window-instances-changed",
            &serde_json::json!(count),
        );
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

/// Resolve the base URL for the frontend.
/// Production: IPC server serves static files from `frontend/` next to the exe.
/// Dev: Vite dev server at `http://localhost:5173`.
pub(crate) fn resolve_frontend_base_url(ipc_port: u16) -> String {
    // In dev mode (AGENTMUX_DEV=1 set by `task dev`), always use the Vite dev
    // server so secondary windows get the latest code and hot reload works.
    // Without this, secondary windows load from dist/cef-dev/frontend/ (the
    // stale production bundle copied at build time) and miss any live changes.
    if std::env::var("AGENTMUX_DEV").is_ok() {
        return "http://localhost:5173".to_string();
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
        "http://localhost:5173".to_string()
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
    // Reject if the parent isn't a known FullInstance — prevents orphan
    // sub-windows and enforces the lifecycle rule in the spec.
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

    // Record WindowMeta BEFORE the browser is created so on_after_created can
    // read it and apply the right taskbar treatment.
    state.window_meta.lock().insert(
        label.clone(),
        crate::state::WindowMeta {
            label: label.clone(),
            kind,
            parent_instance_id,
        },
    );

    // Phase B.5d — host no longer assigns instance numbers locally.
    // The launcher assigns from `ReportWindowOpened` (sent by the
    // host's `on_after_created` callback) and the shadow update
    // emits `window-instances-changed`. Brief race: the new window's
    // frontend may query `get_instance_number` before the launcher's
    // `WindowInstanceAssigned` event has returned, getting a None →
    // `unwrap_or(1)` fallback. Frontend's existing
    // `app-init.ts::refreshLabels(true, retriesLeft)` retry loop
    // catches up within a few hundred ms. B.7 will replace polling
    // with direct event subscription, eliminating the race entirely.
    let (pos_x, pos_y) = get_offset_position();
    let (win_w, win_h) = get_secondary_window_size(pos_x, pos_y);

    // Push label before posting — on_after_created pops it to register the
    // browser under the same label that's baked into the window URL.
    state.pending_window_labels.lock().push_back(label.clone());

    // Post to CEF UI thread — window_create_top_level must run there.
    // true = frameless: secondary app windows use the same custom title bar as main.
    crate::ui_tasks::post_create_window(
        state, &url, &label, pos_x, pos_y, win_w, win_h,
        true,
    );

    // Notify all windows of the count change
    let count = state.instance_count();
    crate::events::emit_event_all_windows(state, "window-instances-changed", &serde_json::json!(count));

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
