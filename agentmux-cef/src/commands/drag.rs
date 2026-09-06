// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Cross-window drag-and-drop commands for the CEF host.
// Ported from src-tauri/src/commands/drag.rs.
//
// These commands coordinate drag sessions that span multiple windows.
// The source window escalates a local pragmatic-dnd drag to a cross-window
// drag when the cursor leaves the window. Position updates are broadcast
// to all windows via CEF execute_javascript events.

use std::sync::Arc;

use cef::{ImplBrowser, ImplBrowserHost};

use crate::events;
use crate::state::{AppState, DragPayload, DragSession, DragType};

/// Sanity bounds for tear-off window dimensions. Frontend caps via
/// `window.outerWidth/Height` (CSS/DIP) but a malformed or hostile
/// arg should not be able to size the new window absurdly.
const TEAROFF_MIN_DIM: i32 = 200;
const TEAROFF_MAX_DIM: i32 = 8192;

/// Start a cross-window drag session.
pub fn start_cross_drag(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let drag_type: DragType = serde_json::from_value(
        args.get("dragType").cloned().unwrap_or_default()
    ).map_err(|e| format!("Invalid dragType: {}", e))?;
    let source_window = args.get("sourceWindow").and_then(|v| v.as_str()).unwrap_or("main").to_string();
    let source_workspace_id = args.get("sourceWorkspaceId").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let source_tab_id = args.get("sourceTabId").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let payload: DragPayload = serde_json::from_value(
        args.get("payload").cloned().unwrap_or_default()
    ).unwrap_or(DragPayload { block_id: None, tab_id: None });

    let drag_id = uuid::Uuid::new_v4().to_string();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;

    tracing::info!(drag_id = %drag_id, drag_type = ?drag_type, source_window = %source_window, "[dnd:cef] start_cross_drag");

    // Self-heal a STALE drag session before starting a new one. The reducer
    // enforces a singleton (StartDrag errors if one is already active), which
    // is correct for a genuinely-concurrent drag — but if a prior drag's
    // end/cancel never reached the host (the renderer threw mid-drop, or the
    // dragged pane/window was destroyed under it), `active_drag` would stay
    // Some forever and reject EVERY future tear-off ("drag session already
    // active"). No legitimate drag is held for tens of seconds, so an active
    // session older than STALE_MS is presumed dead: cancel it (EndDrag) so the
    // new drag can proceed. Belt-and-suspenders behind the frontend's
    // catch-path cancelCrossDrag — recovers even if the renderer never runs it.
    const STALE_MS: u64 = 30_000;
    if let Some(prev) = state.active_drag_snapshot() {
        if now.saturating_sub(prev.started_at) > STALE_MS {
            tracing::warn!(
                stale_drag_id = %prev.drag_id,
                age_ms = now.saturating_sub(prev.started_at),
                "[dnd:cef] clearing stale drag session before new start_cross_drag"
            );
            state.host_dispatch(crate::reducer::HostCommand::EndDrag {
                drag_id: prev.drag_id.clone(),
                outcome: crate::reducer::DragOutcome::Cancelled,
            });
        }
    }

    let session = DragSession {
        drag_id: drag_id.clone(),
        drag_type,
        source_window,
        source_workspace_id,
        source_tab_id,
        payload,
        started_at: now,
    };

    // PR #5 H.3 — sole drag-state mutation entry point. Reducer enforces
    // singleton invariant; if a drag is already active, the dispatch
    // emits a HostEvent::Error and leaves state unchanged. Mirroring the
    // pre-PR semantics, we still proceed with the renderer broadcast —
    // the legacy code unconditionally overwrote, but that masked a
    // genuine bug. Surface the singleton violation by checking the
    // returned event.
    let dispatch = state.host_dispatch(
        crate::reducer::HostCommand::StartDrag { session: session.clone() },
    );
    if dispatch.events.iter().any(|e| matches!(e, crate::reducer::HostEvent::Error { .. })) {
        return Err("a drag session is already active".to_string());
    }
    events::emit_event_all_windows(state, "cross-drag-start", &serde_json::to_value(&session).unwrap());

    Ok(serde_json::json!(drag_id))
}

/// Update cross-window drag with current cursor position.
pub fn update_cross_drag(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let drag_id = args.get("dragId").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let screen_x = args.get("screenX").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let screen_y = args.get("screenY").and_then(|v| v.as_f64()).unwrap_or(0.0);

    // PR #5 H.3 — read via reducer-aware helper.
    let session = state
        .get_drag_session(&drag_id)
        .ok_or_else(|| "no active drag session or drag_id mismatch".to_string())?;

    let target_window = hit_test_windows(state, screen_x, screen_y);

    events::emit_event_all_windows(state, "cross-drag-update", &serde_json::json!({
        "dragId": drag_id,
        "dragType": session.drag_type,
        "payload": session.payload,
        "targetWindow": target_window,
        "sourceWindow": session.source_window,
        "screenX": screen_x,
        "screenY": screen_y,
    }));

    Ok(serde_json::json!(target_window))
}

/// Complete a cross-window drag by committing the drop.
pub fn complete_cross_drag(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let drag_id = args.get("dragId").and_then(|v| v.as_str()).unwrap_or("").to_string();
    let target_window = args.get("targetWindow").and_then(|v| v.as_str()).map(|s| s.to_string());
    let screen_x = args.get("screenX").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let screen_y = args.get("screenY").and_then(|v| v.as_f64()).unwrap_or(0.0);

    // PR #5 H.3 — atomic end-and-return via reducer. EndDrag returns
    // `ended_drag_session: Some(_)` iff the drag_id matched and the
    // session was actually consumed. None means: no session active OR
    // drag_id mismatch — both surface as Err here.
    let outcome = match &target_window {
        Some(t) => crate::reducer::DragOutcome::Dropped { target_label: t.clone() },
        None => crate::reducer::DragOutcome::TornOff { new_label: String::new() },
    };
    let dispatch = state.host_dispatch(
        crate::reducer::HostCommand::EndDrag { drag_id: drag_id.clone(), outcome },
    );
    let session = dispatch
        .ended_drag_session
        .ok_or_else(|| "no active drag session or drag_id mismatch".to_string())?;

    let result = if target_window.is_some() { "drop" } else { "tearoff" };
    tracing::info!(drag_id = %drag_id, result = %result, "[dnd:cef] complete_cross_drag");

    events::emit_event_all_windows(state, "cross-drag-end", &serde_json::json!({
        "dragId": drag_id,
        "result": result,
        "targetWindow": target_window,
        "screenX": screen_x,
        "screenY": screen_y,
        "payload": session.payload,
        "dragType": session.drag_type,
        "sourceWindow": session.source_window,
        "sourceWorkspaceId": session.source_workspace_id,
        "sourceTabId": session.source_tab_id,
    }));

    Ok(serde_json::Value::Null)
}

/// Cancel an active cross-window drag session.
pub fn cancel_cross_drag(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let drag_id = args.get("dragId").and_then(|v| v.as_str()).unwrap_or("").to_string();

    // PR #5 H.3 — atomic cancel via reducer. EndDrag's
    // `ended_drag_session.is_some()` distinguishes "actually ended"
    // from "no session / drag_id mismatch".
    let dispatch = state.host_dispatch(
        crate::reducer::HostCommand::EndDrag {
            drag_id: drag_id.clone(),
            outcome: crate::reducer::DragOutcome::Cancelled,
        },
    );
    if dispatch.ended_drag_session.is_none() {
        return Err("no active drag session or drag_id mismatch".to_string());
    }

    events::emit_event_all_windows(state, "cross-drag-end", &serde_json::json!({
        "dragId": drag_id,
        "result": "cancel",
    }));

    tracing::info!(drag_id = %drag_id, "[dnd:cef] cancel_cross_drag");
    Ok(serde_json::Value::Null)
}

/// Hit-test all open browser windows to find which one contains the cursor.
#[cfg(target_os = "windows")]
fn hit_test_windows(state: &Arc<AppState>, screen_x: f64, screen_y: f64) -> Option<String> {
    use cef::ImplBrowserHost;
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::UI::WindowsAndMessaging::GetWindowRect;

    // Phase H.2.b — reducer-aware iteration with fallback.
    for (label, browser) in state.list_browsers() {
        if let Some(host) = browser.host() {
            let hwnd = host.window_handle();
            if hwnd.0.is_null() { continue; }
            unsafe {
                let mut rect: RECT = std::mem::zeroed();
                GetWindowRect(hwnd.0 as *mut std::ffi::c_void, &mut rect);
                let x = rect.left as f64;
                let y = rect.top as f64;
                let w = (rect.right - rect.left) as f64;
                let h = (rect.bottom - rect.top) as f64;
                if screen_x >= x && screen_x <= x + w && screen_y >= y && screen_y <= y + h {
                    return Some(label.clone());
                }
            }
        }
    }
    None
}

#[cfg(not(target_os = "windows"))]
fn hit_test_windows(_state: &Arc<AppState>, _screen_x: f64, _screen_y: f64) -> Option<String> {
    None
}

/// Get the current cursor position on screen.
pub fn get_cursor_point() -> Result<serde_json::Value, String> {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::Foundation::POINT;
        use windows_sys::Win32::UI::WindowsAndMessaging::GetCursorPos;
        unsafe {
            let mut pt: POINT = std::mem::zeroed();
            GetCursorPos(&mut pt);
            return Ok(serde_json::json!({ "x": pt.x, "y": pt.y }));
        }
    }
    #[allow(unreachable_code)]
    Ok(serde_json::json!({ "x": 0, "y": 0 }))
}

/// Check whether the primary mouse button is currently pressed.
pub fn get_mouse_button_state() -> Result<serde_json::Value, String> {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState;
        let state = unsafe { GetAsyncKeyState(0x01) }; // VK_LBUTTON
        return Ok(serde_json::json!((state as u16 & 0x8000) != 0));
    }
    #[allow(unreachable_code)]
    Ok(serde_json::json!(false))
}

/// Replace the system no-drop cursor with a crosshair during drag.
pub fn set_drag_cursor() -> Result<serde_json::Value, String> {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            CopyIcon, LoadCursorW, SetSystemCursor, IDC_CROSS, OCR_NO,
        };
        unsafe {
            let cross = LoadCursorW(std::ptr::null_mut(), IDC_CROSS);
            if !cross.is_null() {
                let copy = CopyIcon(cross);
                if !copy.is_null() {
                    SetSystemCursor(copy, OCR_NO);
                }
            }
        }
    }
    Ok(serde_json::Value::Null)
}

/// Restore all system cursors to defaults.
pub fn restore_drag_cursor() -> Result<serde_json::Value, String> {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{SystemParametersInfoW, SPI_SETCURSORS};
        unsafe {
            SystemParametersInfoW(SPI_SETCURSORS, 0, std::ptr::null_mut(), 0);
        }
    }
    Ok(serde_json::Value::Null)
}

/// Release mouse capture after an HTML5 drag ends outside the window.
pub fn release_drag_capture(state: &Arc<AppState>) -> Result<serde_json::Value, String> {
    #[cfg(target_os = "windows")]
    {
        use windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            EnumChildWindows, PostMessageW, WM_CANCELMODE,
        };
        use windows_sys::Win32::Foundation::{BOOL, LPARAM};

        // Use the main browser's HWND, or find_own_top_level_window as fallback.
        // Phase H.2.b — reducer-aware lookup with fallback.
        let hwnd = state
            .get_browser("main")
            .and_then(|b| b.host())
            .map(|h| h.window_handle().0 as *mut std::ffi::c_void)
            .unwrap_or_else(|| unsafe { super::window::find_own_top_level_window() });

        if !hwnd.is_null() {
            unsafe {
                ReleaseCapture();
                PostMessageW(hwnd, WM_CANCELMODE, 0, 0);
                unsafe extern "system" fn cancel_child(child: *mut std::ffi::c_void, _: LPARAM) -> BOOL {
                    PostMessageW(child, WM_CANCELMODE, 0, 0);
                    1
                }
                EnumChildWindows(hwnd, Some(cancel_child), 0);
            }
        }
    }
    let _ = state;
    Ok(serde_json::Value::Null)
}

/// Phase 6 — frontend signal that a pool window's renderer is
/// ready to receive `pool:promote`. Called from awaitPoolPromote
/// AFTER the listener is installed. Only after this signal does
/// the window enter the pool queue (otherwise emit_event_to_window
/// would race the listener install and lose promote events).
pub fn pool_window_ready(
    state: &Arc<AppState>,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let label = args
        .get("label")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing label".to_string())?;
    super::window_pool::mark_pool_window_renderer_ready(state, label);
    Ok(serde_json::Value::Null)
}

pub fn pane_pool_window_ready(
    state: &Arc<AppState>,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let label = args
        .get("label")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing label".to_string())?;
    super::window_pool::mark_pane_pool_window_renderer_ready(state, label);
    Ok(serde_json::Value::Null)
}

/// Phase 6 — promote a pre-warmed pool window for tear-off.
/// Returns the promoted window's label, or an error string if the
/// pool was empty (caller should fall back to open_window_at_position).
/// Args: { workspaceId, screenX, screenY }.
pub fn tear_off_pool_promote(
    state: &Arc<AppState>,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let workspace_id = args
        .get("workspaceId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing workspaceId".to_string())?;
    let screen_x = args
        .get("screenX")
        .and_then(|v| v.as_f64())
        .ok_or_else(|| "missing screenX".to_string())? as i32;
    let screen_y = args
        .get("screenY")
        .and_then(|v| v.as_f64())
        .ok_or_else(|| "missing screenY".to_string())? as i32;
    // Optional source-window dimensions for size-matching tear-off.
    let width = args
        .get("width")
        .and_then(|v| v.as_f64())
        .map(|w| (w as i32).clamp(TEAROFF_MIN_DIM, TEAROFF_MAX_DIM));
    let height = args
        .get("height")
        .and_then(|v| v.as_f64())
        .map(|h| (h as i32).clamp(TEAROFF_MIN_DIM, TEAROFF_MAX_DIM));
    // Optional tab anchor — the screen point where the user grabbed
    // the tab. Backend positions the new window so its first tab lands
    // at that point so the cursor stays on the same visual element
    // across the handoff (Chrome-style no-teleport tear-off).
    let tab_anchor_x = args.get("tabAnchorX").and_then(|v| v.as_f64()).map(|n| n as i32);
    let tab_anchor_y = args.get("tabAnchorY").and_then(|v| v.as_f64()).map(|n| n as i32);

    match super::window_pool::promote_pool_window(
        state,
        workspace_id,
        screen_x,
        screen_y,
        width,
        height,
        tab_anchor_x,
        tab_anchor_y,
        None,
        None,
        false, // tab tear-off, not a tray panel
    ) {
        Some(label) => Ok(serde_json::json!(label)),
        None => {
            // Pool exhausted on all platforms (Phase 7 implemented pool on macOS/Linux).
            // Frontend falls back to cold-path window creation.
            tracing::warn!(
                target: "dnd:tearoff:pool",
                workspace_id = %workspace_id,
                "[pool] pool exhausted on tear-off — frontend will cold-path"
            );
            Err("pool_exhausted".to_string())
        }
    }
}

/// Open a new window at a specific screen position (tear-off).
/// Creates a new CEF browser window positioned so the cursor lands in the title bar.
pub fn open_window_at_position(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    // PR #6 H.7 — refuse top-level creation while any pane is mid-close.
    // See `commands/window.rs::open_window_with_kind` for rationale.
    if state.any_browser_pane_closing() {
        tracing::warn!(
            target: "wfr:gate",
            "[wfr:gate] open_window_at_position refused — pane is mid-close (H.7 invariant)"
        );
        return Err("a pane is currently closing; retry shortly".to_string());
    }

    // Same draining guard as `open_window_with_kind` — a tear-off racing an
    // explicit quit would otherwise strand a live window in a draining host
    // (Codex P2 on PR #2996).
    if !matches!(
        state.host_state.lock().quit_state,
        crate::state::QuitState::Running
    ) {
        tracing::warn!(
            target: "wfr:gate",
            "[wfr:gate] open_window_at_position refused — instance is draining/quitting"
        );
        return Err("the app is shutting down".to_string());
    }

    let screen_x = args.get("screenX").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let screen_y = args.get("screenY").and_then(|v| v.as_f64()).unwrap_or(0.0);
    let workspace_id = args.get("workspaceId").and_then(|v| v.as_str()).unwrap_or("").to_string();

    let window_id = uuid::Uuid::new_v4();
    let label = format!("window-{}", window_id.simple());

    // Source-window-matching tear-off size. Frontend captures
    // window.outerWidth/Height of the dragged-from window and passes
    // them through; cold path falls back to the historical default if
    // the args are absent (manual host RPC, etc.).
    let win_w = args
        .get("width")
        .and_then(|v| v.as_f64())
        .map(|w| (w as i32).clamp(TEAROFF_MIN_DIM, TEAROFF_MAX_DIM))
        .unwrap_or(1200);
    let win_h = args
        .get("height")
        .and_then(|v| v.as_f64())
        .map(|h| (h as i32).clamp(TEAROFF_MIN_DIM, TEAROFF_MAX_DIM))
        .unwrap_or(800);

    // Optional tab anchor — see warm-pool path comment.
    let tab_anchor_x = args.get("tabAnchorX").and_then(|v| v.as_f64()).map(|n| n as i32);
    let tab_anchor_y = args.get("tabAnchorY").and_then(|v| v.as_f64()).map(|n| n as i32);

    // Anchor is the new window's outer top-left (frontend pre-computed,
    // chrome inset already subtracted). See window_pool.rs for full
    // rationale. Negative coords valid on multi-monitor.
    let (pos_x, pos_y) = match (tab_anchor_x, tab_anchor_y) {
        (Some(ax), Some(ay)) => (ax, ay),
        _ => (
            ((screen_x - win_w as f64 / 2.0).max(0.0)) as i32,
            ((screen_y - 16.0).max(0.0)) as i32,
        ),
    };

    tracing::info!(
        label = %label, pos_x = %pos_x, pos_y = %pos_y,
        workspace_id = %workspace_id,
        "[dnd:cef] open_window_at_position"
    );

    // Build URL with IPC credentials and tear-off params
    let ipc_port = *state.ipc_port.lock();
    let ipc_token = &state.ipc_token;
    let url = match super::window::resolve_frontend_base_url(ipc_port) {
        Ok(base_url) => {
            let separator = if base_url.contains('?') { "&" } else { "?" };
            let mut u = format!(
                "{}{}ipc_port={}&ipc_token={}&windowLabel={}",
                base_url, separator, ipc_port, ipc_token, label
            );
            if !workspace_id.is_empty() {
                u.push_str(&format!("&workspaceId={}", workspace_id));
            }
            u
        }
        Err(e) => {
            tracing::error!(
                error = %e,
                label = %label,
                "[dnd:cef] frontend assets unavailable — opening static error page on tear-off",
            );
            super::window::assets_missing_data_url(&e)
        }
    };

    // Phase B.5 (window_meta step d) — push the pre-create
    // handoff (label + kind + parent). Tear-offs are FullInstance
    // with no parent. Replaces the previous parallel
    // `window_meta.insert` + `pending_window_labels.push` pair.
    //
    // Phase F.1 — routed through the host reducer.
    state.host_dispatch(
        crate::reducer::HostCommand::EnqueuePendingWindowCreation {
            entry: crate::state::PendingWindowCreation {
                label: label.clone(),
                kind: crate::state::WindowKind::FullInstance,
                parent_instance_id: None,
            },
        },
    );

    // Post to CEF UI thread — window_create_top_level must run there.
    // true = frameless: tear-off windows use the same custom title bar as main.
    crate::ui_tasks::post_create_window(
        state, &url, &label, pos_x, pos_y, win_w, win_h,
        true,
    );

    // Phase B.7.3.3 — the launcher's typed events
    // (`Event::WindowOpened` + `Event::WindowInstanceAssigned` +
    // `Event::BackendWindowIdRegistered`) flow through the CEF JS
    // bridge to drive the InstancePanel atoms. No sync emit here.

    Ok(serde_json::json!(label))
}

/// Signal that a JS-level drag is starting or ending (Linux GTK guard).
pub fn set_js_drag_active(_args: &serde_json::Value) -> Result<serde_json::Value, String> {
    // No-op on Windows/macOS. Linux would need an atomic flag.
    Ok(serde_json::Value::Null)
}

/// Tear-off Phase 2 — the Win32 SC_MOVE handshake.
///
/// Called from `requestTearOff` in tabbar.tsx AFTER the frontend has
/// already (a) called WorkspaceService.TearOffTab to move the tab into
/// a new workspace, and (b) called open_window_at_position to spawn
/// the destination window. This handler waits for the destination
/// window's HWND to register, then issues the Win32 SC_MOVE so the
/// new window enters the OS modal move-loop and follows the cursor
/// at full opacity (no ghost) until mouseup.
///
/// Per spec §0 the cold-path version (no warm pool) accepts a
/// ~150-300 ms first-paint flash for Phase 2 verification only;
/// Phase 6 replaces that with a pre-warmed pool to hit the 0 ms
/// target. The ≤ 8 ms handshake budget from §0 applies from this
/// phase onward and is measured by `tear_off.handshake_ms` —
/// excluded from the budget is only the registration-wait, which
/// goes away with the warm pool.
///
/// See docs/specs/SPEC_TAB_TEAR_OFF_SIZE_PRESERVATION_2026_04_26.
pub fn tear_off_sc_move_handshake(
    state: &Arc<AppState>,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let t_start = std::time::Instant::now();

    let source_label = args
        .get("sourceWindowLabel")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing sourceWindowLabel".to_string())?
        .to_string();
    let dest_label = args
        .get("destWindowLabel")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing destWindowLabel".to_string())?
        .to_string();
    // Error on missing/malformed coords rather than defaulting to (0,0):
    // a silent default would put the new window at screen origin with no
    // diagnostic, looking like a feature bug when it's actually a wire
    // contract bug.
    let cursor_x = args
        .get("cursorX")
        .and_then(|v| v.as_f64())
        .ok_or_else(|| "missing or invalid cursorX".to_string())? as i32;
    let cursor_y = args
        .get("cursorY")
        .and_then(|v| v.as_f64())
        .ok_or_else(|| "missing or invalid cursorY".to_string())? as i32;

    // Phase 4 args — drive the WH_MOUSE_LL hook + finalize event.
    // Optional in case a future call site doesn't need merge detection;
    // if any are missing/empty, we skip the hook install and the
    // dragged window simply ends as a standalone after mouseup.
    let tab_id = args
        .get("tabId")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let source_ws_id = args
        .get("sourceWsId")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let dest_ws_id = args
        .get("destWsId")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    // Phase 5 — original tab index for cancel-back (ESC or drop on
    // source strip). Defaults to 0 if missing — cancel-back will
    // reinsert at start, which is best-effort if the caller didn't
    // provide the real index.
    let original_tab_index = args
        .get("originalTabIndex")
        .and_then(|v| v.as_u64())
        .unwrap_or(0) as usize;
    // Phase 5 — was the tab pinned in its source workspace?
    // Threaded into HookContext so the cancel-back payload can tell
    // the backend to restore into pinnedtabids vs tabids. Defaults
    // to false (regular tab). (gemini PR #567 round-6 MEDIUM)
    let was_pinned = args
        .get("wasPinned")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    // Win32-only path. The HWND poll, ReleaseCapture, and the SC_MOVE
    // post all live inside the cfg block — on macOS / Linux the
    // function returns Ok(null) immediately (Phase 7 adds platform
    // equivalents). Without this gate the 2s HWND-poll would run on
    // every platform with no benefit.
    #[cfg(target_os = "windows")]
    let handshake_ms: f64 = {
        // Wait for the destination browser's HWND to be available —
        // the window-create posts to the CEF UI thread asynchronously
        // and the browser is registered in state.browsers via
        // on_after_created. Poll with the mutex released between
        // checks. 2s deadline is generous; cold-path window creation
        // typically completes in 150-300 ms. Phase 6 (warm pool) drops
        // this to <16 ms.
        let dest_hwnd = wait_for_browser_hwnd(state, &dest_label, std::time::Duration::from_millis(2000))
            .ok_or_else(|| format!("dest window not registered within 2s: {}", dest_label))?;

        // Phase 4 — install the WH_MOUSE_LL hook on a dedicated thread
        // BEFORE PostMessageW(SC_MOVE) so the hook is armed when the
        // user starts moving. Otherwise the first cursor positions of
        // the move-loop would be missed. Skip if any merge-related
        // arg is empty (Phase 2 callers).
        if !tab_id.is_empty() && !source_ws_id.is_empty() && !dest_ws_id.is_empty() {
            crate::commands::tear_off_hook::start_tear_off_tracking(
                state.clone(),
                source_label.clone(),
                dest_label.clone(),
                tab_id.clone(),
                source_ws_id.clone(),
                dest_ws_id.clone(),
                original_tab_index,
                was_pinned,
            )?;
        }

        let t_handshake = std::time::Instant::now();

        unsafe {
            use windows_sys::Win32::UI::Input::KeyboardAndMouse::ReleaseCapture;
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                PostMessageW, SetForegroundWindow, HTCAPTION, SC_MOVE, WM_SYSCOMMAND,
            };

            // Drop whatever capture this thread may hold (defensive —
            // ReleaseCapture only affects the calling thread's
            // capture, so this is a no-op for OLE-owned capture on
            // the source webview's thread; harmless either way).
            ReleaseCapture();

            // Bring the destination forward. Windows grabs capture
            // automatically when entering the SC_MOVE modal loop, so
            // we don't call SetCapture explicitly here — it would
            // fail anyway since `dest_hwnd` doesn't belong to this
            // thread (Win32 SetCapture requires same-thread ownership).
            // If empirically SC_MOVE turns out to need the capture
            // pre-set, we'll post a UI-thread task to do it; for now
            // the simpler path matches Chrome's observed behaviour.
            SetForegroundWindow(dest_hwnd);

            let lparam = ((cursor_y as i32 as u32) << 16) | (cursor_x as i32 as u32 & 0xFFFF);
            // PostMessageW returns BOOL — 0 means the post failed (e.g.
            // dest HWND went invalid between wait_for_browser_hwnd and
            // here, or the message queue rejected the post). Return an
            // error so the frontend doesn't silently treat tear-off as
            // complete; the UI can fall back / log.
            let post_ok = PostMessageW(
                dest_hwnd,
                WM_SYSCOMMAND,
                (SC_MOVE as usize) | (HTCAPTION as usize),
                lparam as isize,
            );
            if post_ok == 0 {
                let last_err = windows_sys::Win32::Foundation::GetLastError();
                return Err(format!(
                    "PostMessageW(SC_MOVE) failed: GetLastError={}",
                    last_err
                ));
            }
        }

        t_handshake.elapsed().as_micros() as f64 / 1000.0
    };

    #[cfg(not(target_os = "windows"))]
    let handshake_ms: f64 = {
        // macOS (host-side manual move loop) and Linux (BeginWindowDrag →
        // _NET_WM_MOVERESIZE / xdg_toplevel.move) have their own drag paths;
        // this handshake-timing value stays a no-op on non-Windows so the IPC
        // contract exists and the rest of the pipeline can be cross-platform.
        let _ = (state, &dest_label);
        0.0
    };

    let total_ms = t_start.elapsed().as_micros() as f64 / 1000.0;

    tracing::info!(
        target: "dnd:tearoff",
        source = %source_label,
        dest = %dest_label,
        cursor_x = %cursor_x,
        cursor_y = %cursor_y,
        handshake_ms = %handshake_ms,
        total_ms = %total_ms,
        "[dnd:tearoff] SC_MOVE handshake complete"
    );

    Ok(serde_json::json!({
        "handshakeMs": handshake_ms,
        "totalMs": total_ms,
    }))
}

/// Install the global mouse hook for an ordinary in-strip tab drag —
/// the cross-window tab remount gesture
/// (docs/specs/SPEC_CROSS_WINDOW_TAB_REMOUNT_2026_07_11 §4.1). Called by the
/// frontend at tab-drag start; the hook self-uninstalls on mouseup/ESC,
/// with stop_tab_drag_tracking as the dragend belt-and-suspenders.
/// No-op on non-Windows (the hook layer is win32-only until the
/// tear-off spec's Phase 7 trackers land).
pub fn start_tab_drag_tracking(
    state: &Arc<AppState>,
    args: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let source_label = args
        .get("sourceWindowLabel")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing sourceWindowLabel".to_string())?
        .to_string();
    let tab_id = args
        .get("tabId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing tabId".to_string())?
        .to_string();
    let source_ws_id = args
        .get("sourceWsId")
        .and_then(|v| v.as_str())
        .ok_or_else(|| "missing sourceWsId".to_string())?
        .to_string();
    let is_last_tab = args
        .get("isLastTab")
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    crate::commands::tear_off_hook::start_tab_drag_tracking(
        state.clone(),
        source_label,
        tab_id,
        source_ws_id,
        is_last_tab,
    )?;
    Ok(serde_json::Value::Null)
}

/// Stop the active tab-drag hook session, if any. Idempotent — also
/// safe to call after the hook already self-uninstalled on mouseup, or
/// after a tear-off handshake superseded the session.
pub fn stop_tab_drag_tracking() -> Result<serde_json::Value, String> {
    crate::commands::tear_off_hook::stop_active_hook_session();
    Ok(serde_json::Value::Null)
}

/// Poll state.browsers for `label` until its host's HWND is non-null
/// or the deadline elapses. Returns the HWND as a raw pointer.
/// Releases the browsers mutex between polls so on_after_created can
/// register on the UI thread.
#[cfg(target_os = "windows")]
fn wait_for_browser_hwnd(
    state: &Arc<AppState>,
    label: &str,
    timeout: std::time::Duration,
) -> Option<*mut std::ffi::c_void> {
    let deadline = std::time::Instant::now() + timeout;
    while std::time::Instant::now() < deadline {
        // Phase H.2.b — reducer-aware lookup with fallback.
        if let Some(browser) = state.get_browser(label) {
            if let Some(host) = browser.host() {
                let h = host.window_handle();
                if !h.0.is_null() {
                    return Some(h.0 as *mut std::ffi::c_void);
                }
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    None
}
