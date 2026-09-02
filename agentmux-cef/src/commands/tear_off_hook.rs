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
//   * On WM_LBUTTONUP, emits a finalisation event: `tearoff:merge` to
//     the candidate window (which pulls the tab in and closes the
//     dragged window), `tearoff:cancel-back` to the source (drop on
//     source window / ESC), or `tearoff:standalone` to the source
//     (released over nothing).
//
// Spec: docs/specs/SPEC_TAB_TEAR_OFF_SIZE_PRESERVATION_2026_04_26 §4.3-§4.4
//
// Cross-window tab remount (docs/specs/SPEC_CROSS_WINDOW_TAB_REMOUNT_2026_07_11):
// the same hook now also runs in `HookMode::TabDrag` for EVERY tab drag
// (installed at drag start via `start_tab_drag_tracking`, before any
// tear-off). In that mode the hover events are identical, but button-up
// over another window emits `tabdrag:merge-direct` (the tab still lives
// in its original multi-tab workspace — no temporary tear-off ws exists),
// and every other outcome emits nothing: the source window's normal
// in-window reorder / tear-off / HTML5 cross-drag paths own those. If a
// tear-off fires mid-drag, `start_tear_off_tracking` takes over the
// session (the previous hook thread is stopped via WM_QUIT).

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

#[cfg(target_os = "macos")]
pub use macos::stop_active_hook_session;

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn stop_active_hook_session() {}

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

#[cfg(target_os = "macos")]
pub use macos::start_tab_drag_tracking;

/// windowNumber → label registration, called from `app/mod.rs`'s
/// `on_window_created`/`on_window_destroyed` on the CEF UI thread. See
/// the macOS module's doc comment below for why this cache exists.
#[cfg(target_os = "macos")]
pub(crate) use macos::register_window_number as macos_register_window_number;
#[cfg(target_os = "macos")]
pub(crate) use macos::unregister_window_label as macos_unregister_window_label;

#[cfg(not(any(target_os = "windows", target_os = "macos")))]
pub fn start_tab_drag_tracking(
    _state: std::sync::Arc<crate::state::AppState>,
    _source_label: String,
    _tab_id: String,
    _source_ws_id: String,
    _is_last_tab: bool,
) -> Result<(), String> {
    Ok(())
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

    for (label, browser) in browsers.iter() {
        if label == &ctx.dragged_label {
            continue;
        }
        // TabDrag mode: the source window is NOT a candidate. Its own
        // pragmatic-dnd reorder owns the strip while the cursor is over
        // it — emitting tearoff:hover-changed at it on every mouse move
        // would race that (two writers, differently-timed and
        // differently-converted, on one insertionPoint signal), and
        // button-up over the source is owned by the in-window reorder
        // anyway. TearOff mode keeps the source as a candidate: that's
        // the cancel-back drop target. (reagent PR #2086 P1)
        if matches!(ctx.mode, HookMode::TabDrag { .. }) && label == &ctx.source_label {
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

// ═══════════════════════════════════════════════════════════════════════
// macOS — CGEventTap-based cross-window cursor tracking for the in-strip
// tab-drag "redock" gesture (Windows' HookMode::TabDrag equivalent).
//
// See docs/specs/SPEC_MACOS_TAB_REDOCK_PARITY_2026_07_24.md for the full
// design writeup. Summary of the scope decision (§0.1 of that spec):
// Windows' TearOff mode / SC_MOVE-handshake live-follow tear-off is dead
// code on every platform today (superseded by a commit-on-release model —
// requestTearOff's skipScMove is always true from its one call site), so
// there is no live-follow tear-off window to track here — only an
// ordinary in-strip HTML5 drag whose cursor may cross into another
// AgentMux window. `start_tear_off_tracking` (TearOff mode) is
// deliberately NOT given a macOS body; it keeps using the shared
// not-Windows no-op stub above.
//
// Threading discipline: the CGEventTap callback runs on a dedicated
// thread with its own CFRunLoop (mirrors the Windows hook thread's
// GetMessage pump). It must NEVER touch AppKit/NSWindow/CEF Views objects
// directly — those require the main thread. This is not a theoretical
// concern in this codebase: docs/investigations/
// tab-drag-tearoff-crash-macos.md documents a real (pre-CEF-migration)
// crash from exactly this mistake (AppKit calls off the main thread).
// Cross-window hit-testing therefore uses `CGWindowListCopyWindowInfo`, a
// Core Graphics *window-server query* API that never touches our own
// NSWindow objects and is thread-safe by design — the macOS analogue of
// Windows' WindowFromPoint (also a system query, not an app-object call).
// windowNumber→label resolution uses a small Mutex<HashMap> cache
// populated on the CEF UI thread at window-creation time (a one-time,
// main-thread-safe read of NSWindow.windowNumber via the existing
// objc_msgSend idiom — see app/mod.rs), read-only from the hook thread
// thereafter.
// ═══════════════════════════════════════════════════════════════════════

#[cfg(target_os = "macos")]
mod macos {
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::sync::mpsc;
    use std::sync::{Arc, Mutex};

    use core_foundation::base::{CFType, TCFType};
    use core_foundation::boolean::CFBoolean;
    use core_foundation::dictionary::{CFDictionary, CFDictionaryRef};
    use core_foundation::number::CFNumber;
    use core_foundation::runloop::{kCFRunLoopCommonModes, CFRunLoop};
    use core_foundation::string::{CFString, CFStringRef};
    use core_graphics::event::{
        CGEvent, CGEventTap, CGEventTapLocation, CGEventTapOptions, CGEventTapPlacement,
        CGEventTapProxy, CGEventType, EventField,
    };
    use core_graphics::window::{
        copy_window_info, kCGWindowBounds, kCGWindowListOptionOnScreenOnly, kCGWindowNumber,
    };

    use crate::state::AppState;

    /// macOS virtual keycode for Escape (`kVK_Escape`). Not exposed by
    /// core-graphics — this is Apple's own stable HIToolbox constant.
    const KVK_ESCAPE: i64 = 0x35;

    thread_local! {
        static HOOK_CTX: RefCell<Option<MacHookContext>> = const { RefCell::new(None) };
    }

    /// TabDrag-mode-only context — see this module's doc comment for why
    /// TearOff mode isn't ported. Field meanings mirror the Windows
    /// `HookContext` fields of the same name.
    struct MacHookContext {
        state: Arc<AppState>,
        source_label: String,
        tab_id: String,
        source_ws_id: String,
        is_last_tab: bool,
        current_target: RefCell<Option<String>>,
        finalized: RefCell<bool>,
        /// Throttle for `candidate_label_under_cursor`'s
        /// `CGWindowListCopyWindowInfo` call — see that function's doc
        /// comment for why this exists. `(when the cached result was
        /// computed, that result)`.
        last_hit_test: RefCell<(std::time::Instant, Option<String>)>,
    }

    /// The currently-running hook session's run loop, if any — mirrors
    /// Windows' `ACTIVE_HOOK_THREAD`. `CFRunLoopStop` is documented safe
    /// to call from any thread, which is exactly how this is used (from
    /// `stop_active_hook_session`, potentially called from a Tokio worker
    /// thread via the IPC handler).
    static ACTIVE_HOOK_RUNLOOP: Mutex<Option<CFRunLoop>> = Mutex::new(None);

    /// Serializes `start_tab_drag_tracking` and `stop_active_hook_session`
    /// against each other — distinct from `ACTIVE_HOOK_RUNLOOP` above,
    /// which only guards concurrent *access* to the state, not the
    /// *ordering* of start-vs-stop. Without this, `start_tab_drag_tracking`
    /// (IPC-dispatched via `spawn_blocking`, taking real time to spawn a
    /// thread and complete `CGEventTapCreate`) and `stop_active_hook_session`
    /// (was dispatched synchronously, so much faster) had no ordering
    /// guarantee relative to each other: a fast drag-then-immediate-release
    /// could have stop's fast path run — and no-op, since
    /// `ACTIVE_HOOK_RUNLOOP` isn't populated yet — before start's hook
    /// thread finished installing, leaving a zombie hook alive to
    /// misattribute a later, unrelated mouseup/Escape to this drag
    /// (reagent PR #2310 P1, found by Codex). `start_tab_drag_tracking`
    /// holds this for its entire critical section (session-takeover stop +
    /// spawn + ready-wait); `stop_active_hook_session` blocks on it too, so
    /// a stop that arrives mid-install simply waits for the install to
    /// finish (and then correctly stops the now-installed hook) instead of
    /// racing ahead of it.
    static HOOK_LIFECYCLE_LOCK: Mutex<()> = Mutex::new(());

    /// windowNumber → AgentMux window label, populated on the CEF UI
    /// thread (`app/mod.rs`'s `on_window_created`/`on_window_destroyed`)
    /// and read-only from the hook thread. `kCGWindowNumber` in a
    /// `CGWindowListCopyWindowInfo` result is documented by Apple to
    /// equal the corresponding `NSWindow`'s `windowNumber` — the same
    /// value cached here at window-creation time.
    static WINDOW_LABELS_BY_NUMBER: Mutex<Option<HashMap<i64, String>>> = Mutex::new(None);

    pub(crate) fn register_window_number(number: i64, label: String) {
        let mut g = WINDOW_LABELS_BY_NUMBER.lock().unwrap();
        g.get_or_insert_with(HashMap::new).insert(number, label);
    }

    pub(crate) fn unregister_window_label(label: &str) {
        if let Some(map) = WINDOW_LABELS_BY_NUMBER.lock().unwrap().as_mut() {
            map.retain(|_, v| v != label);
        }
    }

    #[allow(non_snake_case)]
    #[link(name = "ApplicationServices", kind = "framework")]
    extern "C" {
        fn AXIsProcessTrustedWithOptions(options: CFDictionaryRef) -> bool;
        static kAXTrustedCheckOptionPrompt: CFStringRef;
    }

    /// Silent Accessibility-permission check — no OS prompt (the prompt
    /// option key is explicitly set to `false`). Gate this before ever
    /// attempting `CGEventTapCreate`: an unauthorized tap can be created
    /// successfully but never fire, which would otherwise manifest as a
    /// silent, undebuggable "redock just doesn't work" bug.
    /// See SPEC_MACOS_TAB_REDOCK_PARITY_2026_07_24.md §2.4.
    fn accessibility_trusted_silent() -> bool {
        unsafe {
            let key = CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt);
            let opts = CFDictionary::from_CFType_pairs(&[(
                key.as_CFType(),
                CFBoolean::false_value().as_CFType(),
            )]);
            AXIsProcessTrustedWithOptions(opts.as_concrete_TypeRef())
        }
    }

    /// Same check, but with the OS prompt enabled — triggers the real
    /// System Settings → Privacy & Security → Accessibility dialog if not
    /// already trusted. Only called once per process lifetime (guarded by
    /// `PROMPTED_THIS_SESSION` below): calling this on every single drag
    /// attempt while the user hasn't granted it yet would re-pop the OS
    /// dialog on every drag, which is worse than not prompting at all.
    ///
    /// This is a deliberately minimal stand-in for the full Phase 7c UX
    /// (in-app explanation before the OS prompt, "already asked" persisted
    /// across app launches, settings deep-link) — see
    /// SPEC_MACOS_TAB_REDOCK_PARITY_2026_07_24.md §2.4/§4. Built now,
    /// ahead of that phase, because without SOME request path the feature
    /// is silently inert and un-discoverable: `accessibility_trusted_silent`
    /// alone never shows the user any way to grant the permission, so the
    /// hook just never installs and nothing visibly differs from before.
    fn accessibility_trusted_prompting() -> bool {
        unsafe {
            let key = CFString::wrap_under_get_rule(kAXTrustedCheckOptionPrompt);
            let opts = CFDictionary::from_CFType_pairs(&[(
                key.as_CFType(),
                CFBoolean::true_value().as_CFType(),
            )]);
            AXIsProcessTrustedWithOptions(opts.as_concrete_TypeRef())
        }
    }

    static PROMPTED_THIS_SESSION: std::sync::atomic::AtomicBool =
        std::sync::atomic::AtomicBool::new(false);

    /// The check `start_tab_drag_tracking` actually calls: silent first
    /// (cheap, no dialog), and if untrusted, prompt exactly once per
    /// process lifetime so the user has a real way to grant the
    /// permission and try again on their next drag.
    pub fn accessibility_trusted() -> bool {
        if accessibility_trusted_silent() {
            return true;
        }
        use std::sync::atomic::Ordering;
        if PROMPTED_THIS_SESSION.swap(true, Ordering::SeqCst) {
            // Already prompted this run — don't re-pop the dialog on
            // every subsequent drag while the user hasn't acted on it
            // (or has it open) yet.
            return false;
        }
        tracing::info!(
            target: "dnd:tabdrag:macos",
            "[dnd:tabdrag:macos] Accessibility not yet granted — triggering the OS permission prompt (first attempt this session)"
        );
        accessibility_trusted_prompting()
    }

    /// Does the actual stop, without acquiring `HOOK_LIFECYCLE_LOCK` itself
    /// — callers that already hold it (namely `start_tab_drag_tracking`'s
    /// session-takeover step) call this directly to avoid deadlocking on
    /// their own lock. The public `stop_active_hook_session` below is a
    /// thin wrapper that acquires the lock first.
    fn stop_active_hook_session_locked() {
        let rl = { ACTIVE_HOOK_RUNLOOP.lock().map(|mut g| g.take()).unwrap_or(None) };
        if let Some(rl) = rl {
            rl.stop();
        }
    }

    /// Stop the active hook session, if any. Idempotent — mirrors the
    /// Windows function of the same name. Called from the frontend's
    /// dragend belt-and-suspenders `stop_tab_drag_tracking` IPC call.
    /// Blocks on `HOOK_LIFECYCLE_LOCK` — see that static's doc comment for
    /// why this matters (reagent PR #2310 P1): if a `start_tab_drag_tracking`
    /// is mid-install (holding the lock), this waits for it to finish
    /// rather than racing ahead and no-oping against not-yet-populated
    /// state.
    pub fn stop_active_hook_session() {
        let _guard = HOOK_LIFECYCLE_LOCK.lock();
        stop_active_hook_session_locked();
    }

    /// Install the CGEventTap for an ordinary in-strip tab drag (cross-
    /// window tab remount). Mirrors Windows'
    /// `start_tab_drag_tracking`/`HookMode::TabDrag` exactly at the IPC
    /// contract level: same event names, same payload shapes, so the
    /// frontend (`droppable-tab.tsx`, `tab-tearoff-events.ts`) needs no
    /// changes.
    ///
    /// Falls back to a silent no-op when Accessibility isn't granted —
    /// the existing `DragOverlay` append-only cross-window drag path
    /// (already shipped, works on macOS today) keeps working exactly as
    /// it does now; this hook is a pure upgrade on top of it, never a
    /// replacement it depends on.
    pub fn start_tab_drag_tracking(
        state: Arc<AppState>,
        source_label: String,
        tab_id: String,
        source_ws_id: String,
        is_last_tab: bool,
    ) -> Result<(), String> {
        // Held for this entire function — see HOOK_LIFECYCLE_LOCK's doc
        // comment (reagent PR #2310 P1). Uses the _locked variant for the
        // session-takeover stop below to avoid deadlocking on this same
        // lock; stop_active_hook_session (the public one, called from the
        // separate stop_tab_drag_tracking IPC command) acquires it itself
        // and will correctly block here until this function returns.
        let _lifecycle_guard = HOOK_LIFECYCLE_LOCK.lock();

        // One session at a time, same as Windows.
        stop_active_hook_session_locked();

        if !accessibility_trusted() {
            tracing::warn!(
                target: "dnd:tabdrag:macos",
                "[dnd:tabdrag:macos] Accessibility permission not granted — skipping CGEventTap install; falling back to append-only cross-window drag"
            );
            return Ok(());
        }

        // Oneshot channel so the caller only returns once the tap is
        // actually installed and enabled — mirrors Windows' ready_tx/
        // ready_rx handshake, for the same reason (don't miss the first
        // few mouse events of the drag).
        let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();

        std::thread::Builder::new()
            .name("tab-drag-hook-macos".to_string())
            .spawn(move || {
                let ctx = MacHookContext {
                    state,
                    source_label,
                    tab_id,
                    source_ws_id,
                    is_last_tab,
                    current_target: RefCell::new(None),
                    finalized: RefCell::new(false),
                    // Backdated so the very first hit test always runs
                    // immediately rather than waiting out the throttle.
                    last_hit_test: RefCell::new((
                        std::time::Instant::now() - HIT_TEST_MIN_INTERVAL,
                        None,
                    )),
                };
                HOOK_CTX.with(|cell| *cell.borrow_mut() = Some(ctx));

                // ListenOnly: this hook only observes, never intercepts —
                // returning None from the callback (below) always passes
                // the event through untouched, so the tab strip's own
                // HTML5 drag session and the OS's own event delivery are
                // completely unaffected by this tap's presence.
                let tap_result = CGEventTap::new(
                    CGEventTapLocation::HID,
                    CGEventTapPlacement::HeadInsertEventTap,
                    CGEventTapOptions::ListenOnly,
                    vec![
                        CGEventType::MouseMoved,
                        // Quartz reports pointer motion as LeftMouseDragged,
                        // not MouseMoved, whenever the left button is held —
                        // i.e. for the ENTIRE duration of a tab drag. Without
                        // this, handle_mouse_move never fired during the
                        // drag itself: only the final LeftMouseUp hit-test
                        // worked, so the live hover indicator this hook is
                        // supposed to drive never actually tracked the
                        // cursor (found by Codex, reagent PR #2310 P1).
                        CGEventType::LeftMouseDragged,
                        CGEventType::LeftMouseUp,
                        CGEventType::KeyDown,
                    ],
                    |_proxy: CGEventTapProxy, etype: CGEventType, event: &CGEvent| {
                        handle_tap_event(etype, event);
                        None
                    },
                );

                let tap = match tap_result {
                    Ok(t) => t,
                    Err(_) => {
                        HOOK_CTX.with(|cell| *cell.borrow_mut() = None);
                        let _ = ready_tx.send(Err(
                            "CGEventTapCreate failed (unexpected — Accessibility was already \
                             confirmed granted)"
                                .to_string(),
                        ));
                        return;
                    }
                };

                let loop_source = match tap.mach_port.create_runloop_source(0) {
                    Ok(s) => s,
                    Err(_) => {
                        HOOK_CTX.with(|cell| *cell.borrow_mut() = None);
                        let _ = ready_tx
                            .send(Err("CFMachPort create_runloop_source failed".to_string()));
                        return;
                    }
                };

                let run_loop = CFRunLoop::get_current();
                run_loop.add_source(&loop_source, unsafe { kCFRunLoopCommonModes });
                tap.enable();

                if let Ok(mut g) = ACTIVE_HOOK_RUNLOOP.lock() {
                    *g = Some(run_loop.clone());
                }

                let _ = ready_tx.send(Ok(()));

                tracing::info!(
                    target: "dnd:tabdrag:macos",
                    "[dnd:tabdrag:macos] CGEventTap installed, entering run loop"
                );

                // Blocks until CFRunLoopStop is called — either by this
                // thread's own tap callback (mouseup / ESC) or by
                // stop_active_hook_session (session takeover / dragend
                // belt-and-suspenders).
                CFRunLoop::run_current();

                HOOK_CTX.with(|cell| *cell.borrow_mut() = None);
                // Vacate the active-session slot — but only if it still
                // points at us (a superseding session may have already
                // replaced it). Mirrors the Windows thread-id comparison.
                if let Ok(mut g) = ACTIVE_HOOK_RUNLOOP.lock() {
                    if g.as_ref() == Some(&run_loop) {
                        *g = None;
                    }
                }

                tracing::info!(
                    target: "dnd:tabdrag:macos",
                    "[dnd:tabdrag:macos] run loop exited, thread exiting"
                );
            })
            .map_err(|e| format!("failed to spawn hook thread: {}", e))?;

        ready_rx
            .recv()
            .map_err(|e| format!("hook ready channel closed: {}", e))?
    }

    fn handle_tap_event(etype: CGEventType, event: &CGEvent) {
        match etype {
            // MouseMoved fires when no button is held; LeftMouseDragged
            // fires when the left button IS held — i.e. for a tab drag's
            // entire duration. Both drive the same hover hit-test.
            CGEventType::MouseMoved | CGEventType::LeftMouseDragged => handle_mouse_move(event),
            CGEventType::LeftMouseUp => {
                handle_button_up(event);
                CFRunLoop::get_current().stop();
            }
            CGEventType::KeyDown => {
                let keycode = event.get_integer_value_field(EventField::KEYBOARD_EVENT_KEYCODE);
                if keycode == KVK_ESCAPE {
                    // TabDrag mode: this hook doesn't own the underlying
                    // native HTML5 drag session (pragmatic-drag-and-drop,
                    // owned by the renderer) — it can't stop that drag by
                    // itself. Originally this just retired the hook
                    // session silently, on the assumption the native drag
                    // would cancel itself on Escape. It doesn't: web DnD
                    // gives browsers no such obligation, and empirically
                    // (live testing) the tab still tore off / reordered
                    // normally on release regardless of Escape.
                    //
                    // Fix: tell the SOURCE renderer explicitly via IPC
                    // event, so its drop handler can skip the tear-off/
                    // reorder decision at release time. A DOM-level
                    // `keydown` listener was tried first and didn't work
                    // either — Chromium's internal native-drag handling
                    // appears to suppress normal input dispatch to the
                    // page for the drag's duration. This CGEventTap sees
                    // the raw OS-level HID keystroke instead, entirely
                    // outside the renderer's own event pipeline, so it
                    // isn't subject to that suppression.
                    // See SPEC_MACOS_TAB_REDOCK_PARITY_2026_07_24.md §5.
                    HOOK_CTX.with(|cell| {
                        let ctx_ref = cell.borrow();
                        if let Some(ctx) = ctx_ref.as_ref() {
                            if *ctx.finalized.borrow() {
                                return;
                            }
                            *ctx.finalized.borrow_mut() = true;
                            tracing::info!(
                                target: "dnd:tabdrag:macos",
                                tab_id = %ctx.tab_id,
                                "[dnd:tabdrag:macos] ESC pressed — session aborted"
                            );
                            crate::events::emit_event_to_window(
                                &ctx.state,
                                &ctx.source_label,
                                "tabdrag:escape-pressed",
                                &serde_json::json!({ "tabId": ctx.tab_id }),
                            );
                            if let Some(target_label) = ctx.current_target.borrow().as_ref() {
                                crate::events::emit_event_to_window(
                                    &ctx.state,
                                    target_label,
                                    "tearoff:hover-cleared",
                                    &serde_json::json!({}),
                                );
                            }
                        }
                    });
                    CFRunLoop::get_current().stop();
                }
            }
            _ => {}
        }
    }

    fn handle_mouse_move(event: &CGEvent) {
        HOOK_CTX.with(|cell| {
            let ctx_ref = cell.borrow();
            let Some(ctx) = ctx_ref.as_ref() else {
                return;
            };
            let loc = event.location();
            let (cursor_x, cursor_y) = (loc.x, loc.y);

            // Throttled — see candidate_label_under_cursor_throttled's doc
            // comment. A few-ms-stale hover target is imperceptible for a
            // visual indicator; querying CGWindowListCopyWindowInfo on
            // every single MouseMoved tap event (up to ~120 Hz) is not
            // free, and doing so was a genuine, user-reported performance
            // regression during initial live testing.
            let candidate = candidate_label_under_cursor_throttled(ctx, cursor_x, cursor_y);
            let prev = ctx.current_target.borrow().clone();
            let candidate_changed = prev != candidate;

            if candidate_changed {
                if let Some(prev_label) = prev.as_ref() {
                    crate::events::emit_event_to_window(
                        &ctx.state,
                        prev_label,
                        "tearoff:hover-cleared",
                        &serde_json::json!({}),
                    );
                }
            }
            // Always emit hover-changed when over a candidate, not just
            // on candidate-change — the destination's insertion
            // indicator tracks cursor X continuously. Mirrors Windows'
            // handle_mouse_move exactly (reagent PR #565 P1 there).
            if let Some(cur_label) = candidate.as_ref() {
                crate::events::emit_event_to_window(
                    &ctx.state,
                    cur_label,
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

    fn handle_button_up(event: &CGEvent) {
        HOOK_CTX.with(|cell| {
            let ctx_ref = cell.borrow();
            let Some(ctx) = ctx_ref.as_ref() else {
                return;
            };
            if *ctx.finalized.borrow() {
                return;
            }
            *ctx.finalized.borrow_mut() = true;

            let loc = event.location();
            let (cursor_x, cursor_y) = (loc.x, loc.y);
            // Fresh, not throttled — this is a single one-off call (not a
            // per-move hot path) and it decides the actual merge outcome,
            // so correctness beats the sub-millisecond cost saved by
            // reusing a possibly-stale cached candidate.
            let candidate = candidate_label_under_cursor_uncached(ctx, cursor_x, cursor_y);

            tracing::info!(
                target: "dnd:tabdrag:macos",
                tab_id = %ctx.tab_id,
                cursor_x = %cursor_x,
                cursor_y = %cursor_y,
                target = ?candidate,
                "[dnd:tabdrag:macos] mouseup — finalize"
            );

            // The only outcome this session owns is a release over
            // another AgentMux window — emit tabdrag:merge-direct and
            // let that window strip-hit-test and move the tab. Release
            // over the source window (in-window reorder) or over
            // nothing (existing DragOverlay cross-window append path) is
            // owned by the existing pipelines; emitting nothing here
            // keeps them un-double-processed. Mirrors Windows'
            // handle_button_up TabDrag branch exactly.
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
                            "isLastTab": ctx.is_last_tab,
                            "cursorX": cursor_x,
                            "cursorY": cursor_y,
                        }),
                    );
                }
            }
        });
    }

    /// Minimum interval between real `CGWindowListCopyWindowInfo` calls
    /// from the mouse-move hot path (~30 Hz). Unlike Windows'
    /// `WindowFromPoint` (an O(1) OS-maintained spatial index lookup —
    /// genuinely cheap per call), `CGWindowListCopyWindowInfo` enumerates
    /// and builds a full CFArray/CFDictionary description of every
    /// on-screen window system-wide. Calling it unthrottled on every
    /// `MouseMoved` tap event (up to ~120 Hz) was a real, user-reported
    /// performance regression found during initial live testing — this
    /// throttle is the fix, not a compromise: a stale-by-at-most-33ms
    /// hover target is imperceptible for a visual indicator, so there is
    /// no user-visible cost, only the CPU saved.
    const HIT_TEST_MIN_INTERVAL: std::time::Duration = std::time::Duration::from_millis(33);

    /// Throttled hit test for the mouse-move hot path — reuses the last
    /// result if it's fresher than `HIT_TEST_MIN_INTERVAL`, otherwise
    /// re-runs `candidate_label_under_cursor_uncached` and refreshes the
    /// cache. Do NOT use this for the mouseup finalize decision — see
    /// `handle_button_up`'s call site for why that one stays uncached.
    fn candidate_label_under_cursor_throttled(
        ctx: &MacHookContext,
        x: f64,
        y: f64,
    ) -> Option<String> {
        {
            let cached = ctx.last_hit_test.borrow();
            if cached.0.elapsed() < HIT_TEST_MIN_INTERVAL {
                return cached.1.clone();
            }
        }
        let fresh = candidate_label_under_cursor_uncached(ctx, x, y);
        *ctx.last_hit_test.borrow_mut() = (std::time::Instant::now(), fresh.clone());
        fresh
    }

    /// Point-in-rect hit test against our own on-screen windows via
    /// `CGWindowListCopyWindowInfo` — see this module's doc comment for
    /// why this is the thread-safe choice over any NSWindow-touching
    /// API, and `HIT_TEST_MIN_INTERVAL`'s doc comment for why callers on
    /// the mouse-move hot path go through the throttled wrapper instead
    /// of calling this directly. Excludes the source window (mirrors
    /// Windows' TabDrag-mode exclusion — its own pragmatic-dnd reorder
    /// owns the strip while the cursor is over it; see the Windows
    /// `candidate_label_under_cursor_locked`'s comment).
    fn candidate_label_under_cursor_uncached(
        ctx: &MacHookContext,
        x: f64,
        y: f64,
    ) -> Option<String> {
        let labels_guard = WINDOW_LABELS_BY_NUMBER.lock().ok()?;
        let labels = labels_guard.as_ref()?;
        if labels.is_empty() {
            return None;
        }

        // `CGWindowListCopyWindowInfo(kCGWindowListOptionOnScreenOnly, ...)`
        // returns windows in FRONT-TO-BACK z-order (Apple's documented
        // behavior for this option). This matters: we need the TOPMOST
        // window whose bounds contain the cursor — any app, not just ours
        // — and only treat it as a redock candidate if that topmost window
        // happens to be one of our own. Originally this loop skipped
        // straight past any entry not in our `labels` map before ever
        // checking its bounds, which meant an AgentMux window whose bounds
        // contained the cursor but was visually COVERED by some other app's
        // window on top of it at that exact point would still be reported
        // as the candidate — occlusion was never considered. Fixed by
        // checking bounds for every entry, in z-order, and stopping at the
        // first (frontmost) one that contains the point, exactly mirroring
        // how Windows' `WindowFromPoint` inherently only ever returns the
        // single topmost HWND at a point (reagent PR #2310 P2).
        let info = copy_window_info(kCGWindowListOptionOnScreenOnly, 0)?;
        let count = info.len();
        for i in 0..count {
            let Some(item) = info.get(i) else { continue };
            // `copy_window_info` returns an untyped CFArray (element type
            // `*const c_void`); each element is actually a CFDictionary —
            // wrap it under the "get" rule (borrowed from the array, not
            // owned) to read it safely and typed.
            let dict: CFDictionary<CFType, CFType> =
                unsafe { CFDictionary::wrap_under_get_rule(*item as CFDictionaryRef) };

            let Some(bounds_ref) = dict.find(unsafe { CFString::wrap_under_get_rule(kCGWindowBounds) }.as_CFType()) else {
                continue;
            };
            // `CFDictionary<CFType, CFType>` isn't `ConcreteCFType` (only
            // the fully-untyped `CFDictionary<*const c_void, *const
            // c_void>` is), so `.downcast()` isn't available here —
            // reinterpret the raw ref directly instead. Safe: Apple
            // documents `kCGWindowBounds`'s value as itself a
            // CFDictionary (X/Y/Width/Height), and `wrap_under_get_rule`
            // borrows (retains without taking ownership) exactly like
            // `.downcast()` would have.
            let bounds_dict: CFDictionary<CFType, CFType> = unsafe {
                CFDictionary::wrap_under_get_rule(
                    bounds_ref.as_concrete_TypeRef() as CFDictionaryRef
                )
            };
            let (Some(bx), Some(by), Some(bw), Some(bh)) = (
                cf_dict_number(&bounds_dict, "X"),
                cf_dict_number(&bounds_dict, "Y"),
                cf_dict_number(&bounds_dict, "Width"),
                cf_dict_number(&bounds_dict, "Height"),
            ) else {
                continue;
            };
            if !(x >= bx && x <= bx + bw && y >= by && y <= by + bh) {
                // This entry's bounds don't contain the cursor at all —
                // irrelevant regardless of z-order, keep scanning.
                continue;
            }

            // Found the FRONTMOST window (any app) whose bounds contain
            // the cursor. This is the one and only candidate check for
            // this hit test — if it isn't one of our own (non-source)
            // windows, some other window is occluding us here and there
            // is no valid redock candidate, full stop (do NOT keep
            // scanning further back for an AgentMux window that's
            // actually hidden behind this one).
            let Some(number_ref) = dict.find(unsafe { CFString::wrap_under_get_rule(kCGWindowNumber) }.as_CFType()) else {
                return None;
            };
            let Some(number) = number_ref.downcast::<CFNumber>().and_then(|n| n.to_i64()) else {
                return None;
            };
            let Some(label) = labels.get(&number) else {
                return None;
            };
            if label == &ctx.source_label {
                return None;
            }
            return Some(label.clone());
        }
        None
    }

    fn cf_dict_number(dict: &CFDictionary<CFType, CFType>, key: &str) -> Option<f64> {
        dict.find(CFString::new(key).as_CFType())?
            .downcast::<CFNumber>()?
            .to_f64()
    }
}
