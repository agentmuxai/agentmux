// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Windows WH_MOUSE_LL hook — see the module doc in `mod.rs`.

#[cfg(target_os = "windows")]
use std::cell::RefCell;
#[cfg(target_os = "windows")]
use std::sync::Arc;

#[cfg(target_os = "windows")]
use crate::state::AppState;


/// Which gesture this hook session is tracking. Determines the
/// button-up finalisation behaviour; hover events are identical.
#[cfg(target_os = "windows")]
#[derive(Clone, Copy, PartialEq)]
enum HookMode {
    /// A torn-off window is mid-SC_MOVE; the tab lives in a temporary
    /// single-tab workspace (`dest_ws_id`). Finalisation: merge /
    /// cancel-back / standalone (the shipped Phase 4/5 behaviour).
    TearOff,
    /// An ordinary in-strip tab drag (no tear-off yet). The tab still
    /// lives in its original workspace (`source_ws_id`). Finalisation:
    /// `tabdrag:merge-direct` to a candidate window; every other
    /// outcome is owned by the source window's existing paths.
    TabDrag { is_last_tab: bool },
}

#[cfg(target_os = "windows")]
struct HookContext {
    state: Arc<AppState>,
    mode: HookMode,
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
    /// Index is into `pinnedtabids` if `was_pinned`, else into `tabids`.
    original_tab_index: usize,
    /// Phase 5 — true if the tab was pinned in its source workspace.
    /// Threaded through to the cancel-back payload so the backend can
    /// restore into `pinnedtabids` and preserve pinned status. Without
    /// this, a pinned tab torn off + cancel-backed would silently
    /// come back unpinned. (gemini PR #567 round-6 MEDIUM)
    was_pinned: bool,
    /// Last-known candidate target label, or None when over a non-
    /// AgentMux window or the desktop. Used to emit hover-clear events
    /// when the cursor leaves a candidate.
    current_target: RefCell<Option<String>>,
    /// Set true the moment a finalisation event has been emitted
    /// (cancel-back via ESC, merge, or standalone). Subsequent hook
    /// callbacks bail without emitting — without this guard, a
    /// post-ESC mouseup would fire a second redundant event.
    finalized: RefCell<bool>,
}

#[cfg(target_os = "windows")]
thread_local! {
    static HOOK_CTX: RefCell<Option<HookContext>> = const { RefCell::new(None) };
}

/// Thread id of the currently-running hook session, if any. One hook
/// session runs at a time: starting a new session (e.g. the tear-off
/// handshake taking over from a TabDrag session when the drag crosses
/// TEAR_PAST_PX) posts WM_QUIT to the previous thread first, which
/// unhooks and exits through its normal loop teardown.
#[cfg(target_os = "windows")]
static ACTIVE_HOOK_THREAD: std::sync::Mutex<Option<u32>> = std::sync::Mutex::new(None);

/// Stop the active hook session, if any. Idempotent. Used by the
/// session-takeover path above and by the frontend's dragend
/// belt-and-suspenders `stop_tab_drag_tracking` call — a leaked
/// WH_MOUSE_LL hook degrades every mouse event on the system.
#[cfg(target_os = "windows")]
pub fn stop_active_hook_session() {
    let tid = { ACTIVE_HOOK_THREAD.lock().map(|mut g| g.take()).unwrap_or(None) };
    if let Some(tid) = tid {
        unsafe {
            use windows_sys::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, WM_QUIT};
            PostThreadMessageW(tid, WM_QUIT, 0, 0);
        }
    }
}


/// Spawn a hook thread for the duration of a tear-off gesture.
/// Returns Ok once the hook is installed; the thread runs in the
/// background until WM_LBUTTONUP arrives or the session is stopped.
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
    was_pinned: bool,
) -> Result<(), String> {
    start_hook_session(HookContextSeed {
        state,
        mode: HookMode::TearOff,
        source_label,
        dragged_label,
        tab_id,
        source_ws_id,
        dest_ws_id,
        original_tab_index,
        was_pinned,
    })
}

/// Install the hook for an ordinary in-strip tab drag (cross-window
/// tab remount, docs/specs/SPEC_CROSS_WINDOW_TAB_REMOUNT_2026_07_11 §4.1).
/// No dragged/destination window exists — the tab is still mounted in
/// its source workspace. Button-up over another AgentMux window emits
/// `tabdrag:merge-direct` to it; all other outcomes emit nothing.
#[cfg(target_os = "windows")]
pub fn start_tab_drag_tracking(
    state: Arc<AppState>,
    source_label: String,
    tab_id: String,
    source_ws_id: String,
    is_last_tab: bool,
) -> Result<(), String> {
    start_hook_session(HookContextSeed {
        state,
        mode: HookMode::TabDrag { is_last_tab },
        source_label,
        dragged_label: String::new(),
        tab_id,
        source_ws_id,
        dest_ws_id: String::new(),
        original_tab_index: 0,
        was_pinned: false,
    })
}


/// Owned bag of HookContext fields — lets the two public entry points
/// share one spawn path without threading nine parameters around.
#[cfg(target_os = "windows")]
struct HookContextSeed {
    state: Arc<AppState>,
    mode: HookMode,
    source_label: String,
    dragged_label: String,
    tab_id: String,
    source_ws_id: String,
    dest_ws_id: String,
    original_tab_index: usize,
    was_pinned: bool,
}

#[cfg(target_os = "windows")]
fn start_hook_session(seed: HookContextSeed) -> Result<(), String> {
    use std::sync::mpsc;

    // One session at a time: a TabDrag session is superseded when the
    // drag crosses the tear-off threshold and the SC_MOVE handshake
    // installs a TearOff session.
    stop_active_hook_session();

    // Use a oneshot channel so the spawn returns only after the hook
    // is fully installed. Otherwise PostMessageW(SC_MOVE) could fire
    // before the hook is ready and we'd miss the first few mouse
    // events of the move-loop.
    let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();

    std::thread::Builder::new()
        .name("tear-off-hook".to_string())
        .spawn(move || {
            let ctx = HookContext {
                state: seed.state,
                mode: seed.mode,
                source_label: seed.source_label,
                dragged_label: seed.dragged_label,
                tab_id: seed.tab_id,
                source_ws_id: seed.source_ws_id,
                dest_ws_id: seed.dest_ws_id,
                original_tab_index: seed.original_tab_index,
                was_pinned: seed.was_pinned,
                current_target: RefCell::new(None),
                finalized: RefCell::new(false),
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

                // Register this thread as the active session so a
                // superseding start (or stop_tab_drag_tracking) can
                // post WM_QUIT to it. Registered only after both hooks
                // installed — a failed install never occupies the slot.
                let my_tid =
                    windows_sys::Win32::System::Threading::GetCurrentThreadId();
                if let Ok(mut g) = ACTIVE_HOOK_THREAD.lock() {
                    *g = Some(my_tid);
                }

                let _ = ready_tx.send(Ok(()));

                tracing::info!(
                    target: "dnd:tearoff",
                    "[dnd:tearoff] hooks installed (mouse + keyboard), entering message loop"
                );

                // Standard GetMessage pump. The loop exits when a
                // hook callback posts WM_QUIT after WM_LBUTTONUP or
                // VK_ESCAPE, or when a superseding session / an
                // explicit stop posts WM_QUIT to this thread.
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
                // Vacate the active-session slot — but only if it still
                // points at us (a superseding session may have already
                // replaced it).
                if let Ok(mut g) = ACTIVE_HOOK_THREAD.lock() {
                    if *g == Some(my_tid) {
                        *g = None;
                    }
                }

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
                    if *ctx.finalized.borrow() {
                        return;
                    }
                    *ctx.finalized.borrow_mut() = true;
                    // TabDrag mode: ESC cancels the native OLE drag on
                    // its own; there is no torn-off window to cancel
                    // back. Just retire the session silently.
                    if matches!(ctx.mode, HookMode::TabDrag { .. }) {
                        tracing::info!(
                            target: "dnd:tearoff",
                            tab_id = %ctx.tab_id,
                            "[dnd:tabdrag] ESC pressed — session retired"
                        );
                        return;
                    }
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
                            "originalSourceWsId": ctx.source_ws_id,
                            "draggedWindowLabel": ctx.dragged_label,
                            "originalIndex": ctx.original_tab_index,
                            "wasPinned": ctx.was_pinned,
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
            // Phase H.2.b — reducer-aware browser snapshot. Materializes
            // a HashMap so the existing candidate_label_under_cursor_locked
            // helper signature stays stable. Collected once per hover tick
            // (~16 ms cadence); allocation is negligible.
            let browsers: std::collections::HashMap<String, Browser> = ctx
                .state
                .list_browsers()
                .into_iter()
                .collect();
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
        // If a previous handler already finalized (e.g. ESC fired and
        // posted WM_QUIT, but this mouseup arrived first), bail.
        if *ctx.finalized.borrow() {
            return;
        }
        *ctx.finalized.borrow_mut() = true;

        // Phase H.2.b — reducer-aware browser snapshot for the finalize
        // candidate lookup. Same materialize-into-HashMap pattern as
        // on_mouse_move above.
        let candidate = {
            use cef::Browser;
            let browsers: std::collections::HashMap<String, Browser> = ctx
                .state
                .list_browsers()
                .into_iter()
                .collect();
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

        // TabDrag mode (cross-window tab remount): the ONLY outcome
        // this session owns is a release over another window — emit
        // `tabdrag:merge-direct` and let that window strip-hit-test and
        // move the tab. Release over the source window (in-window
        // reorder) or over nothing (HTML5 cross-drag / standalone
        // paths) is owned by the existing pipelines; emitting nothing
        // here keeps them un-double-processed.
        if let HookMode::TabDrag { is_last_tab } = ctx.mode {
            if let Some(target_label) = &candidate {
                if target_label != &ctx.source_label {
                    crate::events::emit_event_to_window(
                        &ctx.state,
                        target_label,
                        "tabdrag:merge-direct",
                        &serde_json::json!({
                            "tabId": ctx.tab_id,
                            "fromWsId": ctx.source_ws_id,
                            "sourceWindowLabel": ctx.source_label,
                            "isLastTab": is_last_tab,
                            "cursorX": cursor_x,
                            "cursorY": cursor_y,
                        }),
                    );
                }
            }
            return;
        }

        match &candidate {
            Some(target_label) if target_label == &ctx.source_label => {
                // Phase 5 cancel-back path. The cursor is over the
                // source window — but candidate_label_under_cursor only
                // identifies the top-level HWND, not which sub-region
                // (strip vs content vs sidebar). The frontend's
                // cancel-back handler does the same strip hit-test
                // the merge handler does, and falls through to
                // standalone behaviour if the cursor isn't on the
                // strip. We pass cursorY so the frontend can decide.
                tracing::info!(
                    target: "dnd:tearoff",
                    tab_id = %ctx.tab_id,
                    original_index = %ctx.original_tab_index,
                    "[dnd:tearoff] drop on source window — cancel-back candidate"
                );
                crate::events::emit_event_to_window(
                    &ctx.state,
                    &ctx.source_label,
                    "tearoff:cancel-back",
                    &serde_json::json!({
                        "tabId": ctx.tab_id,
                        "fromWsId": ctx.dest_ws_id,
                        "originalSourceWsId": ctx.source_ws_id,
                        "draggedWindowLabel": ctx.dragged_label,
                        "originalIndex": ctx.original_tab_index,
                        "wasPinned": ctx.was_pinned,
                        "cursorX": cursor_x,
                        "cursorY": cursor_y,
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

    let label = label_for_top_level(ctx, browsers, root)?;
    if label == ctx.dragged_label {
        return None;
    }
    // TabDrag mode: the source window is NOT a candidate. Its own
    // pragmatic-dnd reorder owns the strip while the cursor is over
    // it — emitting tearoff:hover-changed at it on every mouse move
    // would race that (two writers, differently-timed and
    // differently-converted, on one insertionPoint signal), and
    // button-up over the source is owned by the in-window reorder
    // anyway. TearOff mode keeps the source as a candidate: that's
    // the cancel-back drop target. (reagent PR #2086 P1)
    if matches!(ctx.mode, HookMode::TabDrag { .. }) && label == ctx.source_label {
        return None;
    }
    if !is_instance_label(&label) {
        return None;
    }
    Some(label)
}

/// The label of the browser whose top-level window is `root`, from the live
/// window under the cursor back to its owner. `BrowserHost::window_handle()`
/// alone can't answer this: in CEF Views mode it is often null, and
/// otherwise a child of the top-level window, so comparing it with the root
/// never matched and every release reported no target. Only window labels
/// count: a browser pane's browser sits inside its window, so its host
/// handle has the same root.
///
/// A cached HWND can be stale, and Windows reuses a destroyed window's HWND
/// value, so a stale entry can equal a live window. Each step guards that:
/// 1. `root` must be a visible CEF Views top-level window of this process,
///    so not another app's window, a menu or a tooltip a reused value could
///    land on.
/// 2. A window's own host handle, when CEF gives one, says for certain.
/// 3. Otherwise `window_hwnds`, bound at Views window creation (app/mod.rs)
///    and evicted on close (client/lifecycle.rs), trusted only when exactly
///    one window label is cached for `root` and that label's browser doesn't
///    place itself in another window. Two labels on one HWND means one is
///    stale and nothing says which: no target. Past that there is no
///    identity to check off the UI thread (a Views window's own handle is
///    only reachable there); the rest of the host trusts `window_hwnds` the
///    same way.
///    A promoted pool window serving the primary keeps its `window-pool-*`
///    entry, while its renderer registered "main" and has no backend window
///    of its own under the pool label; that is "main", the same rule the
///    redock resolver uses (commands/window/motion.rs). It doesn't consult
///    `window_hwnds["main"]`, whose value could be stale or reused; only the
///    live main browser's own host handle can veto it.
#[cfg(target_os = "windows")]
fn label_for_top_level(
    ctx: &HookContext,
    browsers: &std::collections::HashMap<String, cef::Browser>,
    root: *mut std::ffi::c_void,
) -> Option<String> {
    if !is_own_views_top_level(root) {
        return None;
    }

    for (label, browser) in browsers.iter() {
        if is_instance_label(label) && host_top_level(browser) == Some(root) {
            return Some(label.clone());
        }
    }

    let cached = cached_window_labels(ctx, root);
    let [label] = cached.as_slice() else {
        return None;
    };
    if let Some(browser) = browsers.get(label) {
        // Its host handle names a different window: this entry is stale.
        return match host_top_level(browser) {
            Some(_) => None,
            None => Some(label.clone()),
        };
    }
    // A pool window serving the primary: its renderer registered "main",
    // so the pool label has no backend window of its own. The live main
    // browser vetoes it when its own host handle places main elsewhere.
    if label.starts_with("window-pool-") && ctx.state.backend_window_id(label).is_none() {
        let main = browsers.get("main")?;
        return match host_top_level(main) {
            Some(elsewhere) if elsewhere != root => None,
            _ => Some("main".to_string()),
        };
    }
    None
}

/// A visible CEF Views top-level window (class `Chrome_WidgetWin_*`) of
/// this process.
#[cfg(target_os = "windows")]
fn is_own_views_top_level(hwnd: *mut std::ffi::c_void) -> bool {
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetClassNameW, GetWindowThreadProcessId, IsWindowVisible,
    };
    let mut pid: u32 = 0;
    unsafe { GetWindowThreadProcessId(hwnd as _, &mut pid) };
    if pid != unsafe { GetCurrentProcessId() } || unsafe { IsWindowVisible(hwnd as _) } == 0 {
        return false;
    }
    let mut buf = [0u16; 64];
    let n = unsafe { GetClassNameW(hwnd as _, buf.as_mut_ptr(), buf.len() as i32) };
    n > 0 && String::from_utf16_lossy(&buf[..n as usize]).starts_with("Chrome_WidgetWin_")
}

/// The top-level window of the browser's host handle, when CEF gives one.
#[cfg(target_os = "windows")]
fn host_top_level(browser: &cef::Browser) -> Option<*mut std::ffi::c_void> {
    use cef::{ImplBrowser, ImplBrowserHost};
    use windows_sys::Win32::UI::WindowsAndMessaging::{GetAncestor, GA_ROOT};

    let h = browser.host()?.window_handle().0 as *mut std::ffi::c_void;
    if h.is_null() {
        return None;
    }
    let top = unsafe { GetAncestor(h as _, GA_ROOT) } as *mut std::ffi::c_void;
    Some(if top.is_null() { h } else { top })
}

/// The window labels `window_hwnds` has cached for `hwnd`.
#[cfg(target_os = "windows")]
fn cached_window_labels(ctx: &HookContext, hwnd: *mut std::ffi::c_void) -> Vec<String> {
    ctx.state
        .window_hwnds
        .lock()
        .iter()
        .filter(|(l, &h)| h == hwnd as isize && is_instance_label(l))
        .map(|(l, _)| l.clone())
        .collect()
}

#[cfg(target_os = "windows")]
fn is_instance_label(label: &str) -> bool {
    label == "main" || label.starts_with("window-")
}
