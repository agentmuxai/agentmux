// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Tear-off Phase 4 — WH_MOUSE_LL low-level mouse hook for cross-window
// merge detection during the SC_MOVE modal move-loop.
//
// Background: while Windows runs the modal SC_MOVE loop (entered via
// `commands/drag.rs::tear_off_sc_move_handshake`), AgentMux's normal
// renderer message handlers DON'T fire — Windows owns the cursor
// until mouseup. To detect "is the cursor over another AgentMux
// window's tab strip?" we install a global low-level mouse hook on a
// dedicated thread with its own GetMessage loop.
//
// The hook callback runs on the install thread (not arbitrary
// threads). It uses thread-local storage to access the hook context
// (Arc<AppState>, source/dest labels, tab id, etc.) without risking
// re-entrant locking issues that a global Mutex would have.
//
// Architecture:
//   * `start_tear_off_tracking()` is called from the IPC handler
//     BEFORE the SC_MOVE post. Spawns a thread, installs the hook,
//     returns a `TrackingHandle` that's dropped when the user releases
//     the mouse (the thread's GetMessage loop sees WM_LBUTTONUP and
//     calls PostQuitMessage).
//   * On every WM_MOUSEMOVE, the callback does WindowFromPoint →
//     GetAncestor(GA_ROOT) and looks the HWND up in `state.browsers`
//     (skipping the dragged window itself). If the candidate target
//     changed, emits `tearoff:hover-changed` IPC events to the new
//     and old candidate's renderers.
//   * On WM_LBUTTONUP, emits `tearoff:finalize` to the source window's
//     renderer with the final candidate label + cursor position. The
//     source-side frontend handles the merge (calls
//     MoveTabToWorkspace + closes the dragged window).
//
// Spec: docs/specs/SPEC_TAB_TEAR_OFF_SIZE_PRESERVATION_2026_04_26 §4.3-§4.4
// Phase 5 (cancel-back-to-source) reuses the same finalize event
// with a different source-side handler.

#[cfg(target_os = "windows")]
use std::cell::RefCell;
#[cfg(target_os = "windows")]
use std::sync::Arc;

#[cfg(target_os = "windows")]
use crate::state::AppState;

#[cfg(target_os = "windows")]
struct HookContext {
    state: Arc<AppState>,
    /// Label of the source window (the one the tab originated from).
    /// Used to skip self-detection during cursor tracking, and as the
    /// destination for the `tearoff:finalize` event.
    source_label: String,
    /// Label of the dragged-window-now-following-the-cursor. Also
    /// excluded from candidate detection — landing on the dragged
    /// window's own strip would be a no-op.
    dragged_label: String,
    /// The tab being torn off. Echoed back in the finalize payload so
    /// the source frontend doesn't have to track per-drag state.
    tab_id: String,
    /// Source workspace ID. Echoed back so the frontend can call
    /// `MoveTabToWorkspace` from a different window context if needed.
    source_ws_id: String,
    /// Destination workspace ID (the new workspace TearOffTab created).
    /// Cancel-back uses this as the `fromWsId` when restoring.
    dest_ws_id: String,
    /// Phase 5 — tab's original index in the source workspace at the
    /// moment of tear-off. Used by cancel-back (ESC or drop on source
    /// strip) to reinsert the tab where it was, not at the end.
    original_tab_index: usize,
    /// Last-known candidate target label, or None when over a non-
    /// AgentMux window or the desktop. Used to emit hover-clear events
    /// when the cursor leaves a candidate.
    current_target: RefCell<Option<String>>,
}

#[cfg(target_os = "windows")]
thread_local! {
    static HOOK_CTX: RefCell<Option<HookContext>> = const { RefCell::new(None) };
}

/// Spawn a hook thread for the duration of a tear-off gesture.
/// Returns Ok once the hook is installed; the thread runs in the
/// background until WM_LBUTTONUP arrives or `stop_tear_off_tracking`
/// is called.
///
/// Safe to call from a Tokio worker thread — the hook thread is
/// independent and runs its own message loop.
#[cfg(target_os = "windows")]
pub fn start_tear_off_tracking(
    state: Arc<AppState>,
    source_label: String,
    dragged_label: String,
    tab_id: String,
    source_ws_id: String,
    dest_ws_id: String,
    original_tab_index: usize,
) -> Result<(), String> {
    use std::sync::mpsc;

    // Use a oneshot channel so the spawn returns only after the hook
    // is fully installed. Otherwise PostMessageW(SC_MOVE) could fire
    // before the hook is ready and we'd miss the first few mouse
    // events of the move-loop.
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();

    std::thread::Builder::new()
        .name("tear-off-hook".to_string())
        .spawn(move || {
            let ctx = HookContext {
                state,
                source_label,
                dragged_label,
                tab_id,
                source_ws_id,
                dest_ws_id,
                original_tab_index,
                current_target: RefCell::new(None),
            };
            HOOK_CTX.with(|cell| *cell.borrow_mut() = Some(ctx));

            unsafe {
                use windows_sys::Win32::UI::WindowsAndMessaging::{
                    DispatchMessageW, GetMessageW, SetWindowsHookExW,
                    TranslateMessage, UnhookWindowsHookEx, MSG,
                    WH_KEYBOARD_LL, WH_MOUSE_LL,
                };

                // hMod: pass our own module handle (defensive — the
                // OS accepts NULL for WH_MOUSE_LL/WH_KEYBOARD_LL but
                // the contract technically requires the module
                // containing the hook proc).
                let h_module = windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(
                    std::ptr::null(),
                );

                let mouse_hook = SetWindowsHookExW(
                    WH_MOUSE_LL,
                    Some(low_level_mouse_proc),
                    h_module,
                    0,
                );
                if mouse_hook.is_null() {
                    let err = windows_sys::Win32::Foundation::GetLastError();
                    HOOK_CTX.with(|cell| *cell.borrow_mut() = None);
                    let _ = ready_tx.send(Err(format!(
                        "SetWindowsHookExW(WH_MOUSE_LL) failed: GetLastError={}",
                        err
                    )));
                    return;
                }

                // ESC during the SC_MOVE modal loop cancels the move
                // but Windows sends no WM_LBUTTONUP — without a
                // keyboard hook the mouse hook would survive forever
                // (and worse, the next unrelated WM_LBUTTONUP anywhere
                // on the desktop would fire handle_button_up with
                // stale tear-off context, silently merging the wrong
                // tab into the wrong window). The keyboard hook
                // catches VK_ESCAPE and treats it as a standalone
                // finalisation. (reagent PR #565 P1)
                let kb_hook = SetWindowsHookExW(
                    WH_KEYBOARD_LL,
                    Some(low_level_keyboard_proc),
                    h_module,
                    0,
                );
                if kb_hook.is_null() {
                    let err = windows_sys::Win32::Foundation::GetLastError();
                    UnhookWindowsHookEx(mouse_hook);
                    HOOK_CTX.with(|cell| *cell.borrow_mut() = None);
                    let _ = ready_tx.send(Err(format!(
                        "SetWindowsHookExW(WH_KEYBOARD_LL) failed: GetLastError={}",
                        err
                    )));
                    return;
                }

                let _ = ready_tx.send(Ok(()));

                tracing::info!(
                    target: "dnd:tearoff",
                    "[dnd:tearoff] hooks installed (mouse + keyboard), entering message loop"
                );

                // Standard GetMessage pump. The loop exits when a
                // hook callback posts WM_QUIT after WM_LBUTTONUP or
                // VK_ESCAPE.
                let mut msg: MSG = std::mem::zeroed();
                loop {
                    let r = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
                    if r <= 0 {
                        break; // 0 = WM_QUIT, -1 = error
                    }
                    let _ = TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }

                UnhookWindowsHookEx(kb_hook);
                UnhookWindowsHookEx(mouse_hook);
                HOOK_CTX.with(|cell| *cell.borrow_mut() = None);

                tracing::info!(
                    target: "dnd:tearoff",
                    "[dnd:tearoff] hooks uninstalled, thread exiting"
                );
            }
        })
        .map_err(|e| format!("failed to spawn hook thread: {}", e))?;

    // Block until the hook thread either installs the hook or fails.
    // ~milliseconds latency on success.
    ready_rx
        .recv()
        .map_err(|e| format!("hook ready channel closed: {}", e))?
}

/// No-op stub for non-Windows builds. Phase 7 adds platform
/// equivalents (CGEventTap on macOS, polled XQueryPointer on X11).
#[cfg(not(target_os = "windows"))]
pub fn start_tear_off_tracking(
    _state: std::sync::Arc<crate::state::AppState>,
    _source_label: String,
    _dragged_label: String,
    _tab_id: String,
    _source_ws_id: String,
    _dest_ws_id: String,
    _original_tab_index: usize,
) -> Result<(), String> {
    Ok(())
}

#[cfg(target_os = "windows")]
unsafe extern "system" fn low_level_keyboard_proc(
    n_code: i32,
    w_param: windows_sys::Win32::Foundation::WPARAM,
    l_param: windows_sys::Win32::Foundation::LPARAM,
) -> windows_sys::Win32::Foundation::LRESULT {
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::VK_ESCAPE;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, KBDLLHOOKSTRUCT, PostQuitMessage, WM_KEYDOWN, WM_SYSKEYDOWN,
    };

    if n_code < 0 {
        return CallNextHookEx(std::ptr::null_mut(), n_code, w_param, l_param);
    }

    let msg_id = w_param as u32;
    if msg_id == WM_KEYDOWN || msg_id == WM_SYSKEYDOWN {
        let kb = &*(l_param as *const KBDLLHOOKSTRUCT);
        if kb.vkCode == VK_ESCAPE as u32 {
            // Phase 5: ESC = cancel-back. The dragged window is
            // destroyed and the tab reinserts at its original index
            // in the source workspace.
            HOOK_CTX.with(|cell| {
                let ctx_ref = cell.borrow();
                if let Some(ctx) = ctx_ref.as_ref() {
                    tracing::info!(
                        target: "dnd:tearoff",
                        tab_id = %ctx.tab_id,
                        original_index = %ctx.original_tab_index,
                        "[dnd:tearoff] ESC pressed — cancel-back to source"
                    );
                    crate::events::emit_event_to_window(
                        &ctx.state,
                        &ctx.source_label,
                        "tearoff:cancel-back",
                        &serde_json::json!({
                            "tabId": ctx.tab_id,
                            "fromWsId": ctx.dest_ws_id,
                            "draggedWindowLabel": ctx.dragged_label,
                            "originalIndex": ctx.original_tab_index,
                            "reason": "esc",
                        }),
                    );
                }
            });
            PostQuitMessage(0);
        }
    }

    CallNextHookEx(std::ptr::null_mut(), n_code, w_param, l_param)
}

#[cfg(target_os = "windows")]
unsafe extern "system" fn low_level_mouse_proc(
    n_code: i32,
    w_param: windows_sys::Win32::Foundation::WPARAM,
    l_param: windows_sys::Win32::Foundation::LPARAM,
) -> windows_sys::Win32::Foundation::LRESULT {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallNextHookEx, MSLLHOOKSTRUCT, WM_LBUTTONUP, WM_MOUSEMOVE,
    };

    if n_code < 0 {
        return CallNextHookEx(std::ptr::null_mut(), n_code, w_param, l_param);
    }

    let msg_id = w_param as u32;
    let hook_struct = &*(l_param as *const MSLLHOOKSTRUCT);
    let cursor_x = hook_struct.pt.x;
    let cursor_y = hook_struct.pt.y;

    match msg_id {
        WM_MOUSEMOVE => {
            handle_mouse_move(cursor_x, cursor_y);
        }
        WM_LBUTTONUP => {
            handle_button_up(cursor_x, cursor_y);
            // Tell our message loop to exit — the move-loop is over.
            use windows_sys::Win32::UI::WindowsAndMessaging::PostQuitMessage;
            PostQuitMessage(0);
        }
        _ => {}
    }

    CallNextHookEx(std::ptr::null_mut(), n_code, w_param, l_param)
}

#[cfg(target_os = "windows")]
fn handle_mouse_move(cursor_x: i32, cursor_y: i32) {
    HOOK_CTX.with(|cell| {
        let ctx_ref = cell.borrow();
        let Some(ctx) = ctx_ref.as_ref() else {
            return;
        };

        // Single browsers-lock acquisition: detect candidate AND
        // pre-clone the browser handles for prev/next targets in
        // one critical section. Lock held only here, never spanning
        // emit_event calls below.
        let (candidate, prev_browser, next_browser, candidate_changed) = {
            use cef::Browser;
            let browsers = ctx.state.browsers.lock();
            let candidate = candidate_label_under_cursor_locked(ctx, &browsers, cursor_x, cursor_y);
            let prev_label = ctx.current_target.borrow().clone();
            let candidate_changed = prev_label != candidate;
            let prev_browser: Option<Browser> = if candidate_changed {
                prev_label.as_ref().and_then(|l| browsers.get(l).cloned())
            } else {
                None
            };
            let next_browser: Option<Browser> = candidate
                .as_ref()
                .and_then(|l| browsers.get(l).cloned());
            (candidate, prev_browser, next_browser, candidate_changed)
        };

        // Lock released — emit events without re-locking.
        if let Some(b) = prev_browser.as_ref() {
            crate::events::emit_event(b, "tearoff:hover-cleared", &serde_json::json!({}));
        }
        // Always emit hover-changed when over a candidate, not just on
        // candidate-change. The destination's insertion indicator
        // tracks the cursor X within the strip — without per-move
        // updates the indicator would lock to wherever the cursor
        // entered and never slide as the user traverses the strip.
        // (reagent PR #565 P1)
        if let Some(b) = next_browser.as_ref() {
            crate::events::emit_event(
                b,
                "tearoff:hover-changed",
                &serde_json::json!({
                    "cursorX": cursor_x,
                    "cursorY": cursor_y,
                    "tabId": ctx.tab_id,
                }),
            );
        }
        if candidate_changed {
            *ctx.current_target.borrow_mut() = candidate;
        }
    });
}

#[cfg(target_os = "windows")]
fn handle_button_up(cursor_x: i32, cursor_y: i32) {
    HOOK_CTX.with(|cell| {
        let ctx_ref = cell.borrow();
        let Some(ctx) = ctx_ref.as_ref() else {
            return;
        };

        let candidate = {
            let browsers = ctx.state.browsers.lock();
            candidate_label_under_cursor_locked(ctx, &browsers, cursor_x, cursor_y)
        };

        tracing::info!(
            target: "dnd:tearoff",
            tab_id = %ctx.tab_id,
            cursor_x = %cursor_x,
            cursor_y = %cursor_y,
            target = ?candidate,
            "[dnd:tearoff] mouseup — finalize"
        );

        match &candidate {
            Some(target_label) if target_label == &ctx.source_label => {
                // Phase 5 cancel-back: drop on the source window's own
                // strip. Restore the tab at its original index — not
                // wherever the cursor happens to land — so the user
                // gets back the exact pre-tear state.
                tracing::info!(
                    target: "dnd:tearoff",
                    tab_id = %ctx.tab_id,
                    original_index = %ctx.original_tab_index,
                    "[dnd:tearoff] drop on source strip — cancel-back"
                );
                crate::events::emit_event_to_window(
                    &ctx.state,
                    &ctx.source_label,
                    "tearoff:cancel-back",
                    &serde_json::json!({
                        "tabId": ctx.tab_id,
                        "fromWsId": ctx.dest_ws_id,
                        "draggedWindowLabel": ctx.dragged_label,
                        "originalIndex": ctx.original_tab_index,
                        "reason": "drop-on-source",
                    }),
                );
            }
            Some(target_label) => {
                // Merge path. Tell the candidate's renderer to pull the
                // tab in from `dest_ws_id` (the temporary workspace the
                // dragged window owns). The candidate has its own
                // workspace ID locally and can compute the insertion
                // index from `cursorX` against its tab strip geometry.
                // After the merge the candidate calls closeWindowByLabel
                // on the dragged window to clean up.
                crate::events::emit_event_to_window(
                    &ctx.state,
                    target_label,
                    "tearoff:merge",
                    &serde_json::json!({
                        "tabId": ctx.tab_id,
                        "fromWsId": ctx.dest_ws_id,
                        "draggedWindowLabel": ctx.dragged_label,
                        "cursorX": cursor_x,
                        "cursorY": cursor_y,
                    }),
                );
            }
            None => {
                // Standalone path. The dragged window simply stays
                // where the user released. Inform the source renderer
                // (informational only — no UI state to update on the
                // source side; the tab is already gone).
                crate::events::emit_event_to_window(
                    &ctx.state,
                    &ctx.source_label,
                    "tearoff:standalone",
                    &serde_json::json!({
                        "tabId": ctx.tab_id,
                        "draggedWindowLabel": ctx.dragged_label,
                    }),
                );
            }
        }
    });
}

/// Find the AgentMux window label whose top-level HWND contains the
/// cursor position. Excludes the dragged window itself (landing on
/// the dragged window's strip would be a no-op merge). Takes the
/// browsers lock guard from the caller so we don't re-acquire on
/// the WM_MOUSEMOVE hot path.
#[cfg(target_os = "windows")]
fn candidate_label_under_cursor_locked(
    ctx: &HookContext,
    browsers: &std::collections::HashMap<String, cef::Browser>,
    x: i32,
    y: i32,
) -> Option<String> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetAncestor, WindowFromPoint, GA_ROOT,
    };

    let pt = POINT { x, y };
    let hwnd = unsafe { WindowFromPoint(pt) };
    if hwnd.is_null() {
        return None;
    }
    let root = unsafe { GetAncestor(hwnd, GA_ROOT) };
    let root = if root.is_null() { hwnd } else { root };

    for (label, browser) in browsers.iter() {
        if label == &ctx.dragged_label {
            continue;
        }
        if !is_instance_label(label) {
            continue;
        }
        use cef::{ImplBrowser, ImplBrowserHost};
        if let Some(host) = browser.host() {
            let h = host.window_handle();
            if !h.0.is_null() && h.0 as *mut std::ffi::c_void == root {
                return Some(label.clone());
            }
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn is_instance_label(label: &str) -> bool {
    label == "main" || label.starts_with("window-")
}
