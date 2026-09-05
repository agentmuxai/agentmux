// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Window-drag tasks: StartWindowDrag (Linux native drag), the macOS host-side
// manual move loop, and the Windows Win32 manual move loop. Split out of
// `ui_tasks.rs` unchanged.

use std::sync::Arc;
use cef::*;
use crate::state::AppState;
#[cfg(not(target_os = "windows"))]
use super::get_window_on_ui;

// ── Drag ─────────────────────────────────────────────────────────────────
// CEF Views does not expose a programmatic drag-initiation API.
// Begin a native window-move via the underlying CefWindow's BeginWindowDrag().
// Posted on the CEF UI thread (BeginWindowDrag must be called there). This is
// the LINUX path: on Wayland it dispatches
// WmMoveResizeHandler::DispatchHostWindowDragMovement → xdg_toplevel.move with
// the most recent input serial; on X11 it dispatches an XEvent for
// _NET_WM_MOVERESIZE. The compositor handles the drag until the user releases
// the mouse button. (macOS uses a separate host-side move loop — see
// MacWindowDragTask below.)
// Note: views::Widget::RunMoveLoop() is the wrong API on Wayland (returns
// immediately with a non-zero result) — see retro for details.
//
// Triggered by `start_window_drag` IPC from the renderer (useWindowDrag.linux.ts).
// The renderer detects a left-button-down + threshold-crossing motion on a
// HTCLIENT header element (NOT -webkit-app-region: drag — that suppresses
// renderer events) and sends this IPC to initiate native drag.
//
// Requires CEF Patch: BeginWindowDrag added to CefWindow API (libcef/browser/views/window_impl.cc).
// Linux: native drag via CefWindow::BeginWindowDrag (Ozone). macOS uses a
// separate host-side move loop (MacWindowDragTask, below) because stock
// libcef has no drag API and the fork's BeginWindowDrag is Ozone-only.
#[cfg(target_os = "linux")]
wrap_task! {
    pub struct StartWindowDragTask {
        state: Arc<AppState>,
        label: String,
    }

    impl Task {
        fn execute(&self) {
            // Call CefWindow::BeginWindowDrag via raw FFI. The cef Rust
            // crate doesn't have a typed wrapper for our extension method
            // (it's appended to _cef_window_t after get_runtime_style),
            // so we get the raw *mut _cef_window_t and invoke the function
            // pointer directly. cef-dll-sys's binding has been patched to
            // include the begin_window_drag field at the end of the struct.
            //
            // Runtime ABI guard: every CEF struct begins with a size field
            // (cef_base_ref_counted_t.size) populated by libcef itself. If
            // libcef wasn't built from the AgentMux a5af/cef branch, the
            // size will be the upstream value (≠ size_of::<_cef_window_t>()
            // here, since our cef-dll-sys binding has begin_window_drag
            // appended) and reading the extension slot would be UB. Bail
            // out cleanly when sizes diverge.
            use cef::ImplWindow;
            if let Some(window) = get_window_on_ui(&self.state, &self.label) {
                let raw_ptr = <cef::Window as ImplWindow>::get_raw(&window);
                unsafe {
                    let runtime_size = (*raw_ptr).base.base.base.size;
                    let expected_size = std::mem::size_of::<cef::sys::_cef_window_t>();
                    if runtime_size != expected_size {
                        tracing::warn!(
                            "[start_window_drag] libcef.so ABI mismatch — runtime _cef_window_t.size={} expected={} (libcef.so was not built from agentmux/7680-...; skipping native drag) label={}",
                            runtime_size, expected_size, self.label
                        );
                        return;
                    }
                    // begin_window_drag is appended to _cef_window_t by the
                    // AgentMux fork of cef-dll-sys (see docs/cef-build/
                    // build-patched-libcef.md). Unpatched builds (default
                    // crates.io cef-dll-sys) lack the field — gated behind
                    // the `patched-libcef` cargo feature so default builds
                    // compile.
                    #[cfg(feature = "patched-libcef")]
                    {
                        if let Some(f) = (*raw_ptr).begin_window_drag {
                            let result = f(raw_ptr);
                            tracing::info!("[start_window_drag] BeginWindowDrag returned {} label={}", result, self.label);
                        } else {
                            tracing::warn!("[start_window_drag] BeginWindowDrag fn ptr is null label={}", self.label);
                        }
                    }
                    #[cfg(not(feature = "patched-libcef"))]
                    {
                        let _ = raw_ptr;
                        tracing::warn!(
                            "[start_window_drag] patched-libcef feature disabled — native drag is a no-op. \
                             Rebuild with --features patched-libcef and a patched libcef.so (a5af/cef agentmux/7680-...) to enable. label={}",
                            self.label
                        );
                    }
                    // Notify the renderer that the OS drag is done so it can
                    // reset its dragging flag. BeginWindowDrag may not re-deliver
                    // a DOM mouseup to the renderer (F3 verification pending).
                    // moved:false — we cannot detect pixel motion from the
                    // BeginWindowDrag return value, so we conservatively emit
                    // false here. On non-Windows, tryRedockAtCursor is driven
                    // directly from onMouseUp (gated on the DOM mousemove-based
                    // hasMoved flag) rather than from window_drag_ended, so this
                    // event is only needed as a dragging-reset safety net.
                    crate::events::emit_event_to_top_level_windows(
                        &self.state,
                        "window_drag_ended",
                        &serde_json::json!({ "label": &self.label, "moved": false }),
                    );
                }
            } else {
                tracing::warn!("[start_window_drag] no window for label={}", self.label);
            }
        }
    }
}

#[cfg(target_os = "linux")]
pub fn post_start_drag(state: &Arc<AppState>, label: &str) {
    let mut task = StartWindowDragTask::new(state.clone(), label.to_string());
    post_task(ThreadId::UI, Some(&mut task));
}

// macOS: drive the drag with a host-side manual move loop (MacWindowDragTask
// → run_macos_native_drag_loop). Stock libcef ships no programmatic drag API
// and the fork's BeginWindowDrag is Ozone-only (no-op on macOS), so rather
// than chain macOS releases to a patched-Chromium framework we pump the drag
// events ourselves and reposition via CEF set_bounds. The header is HTCLIENT
// (useWindowDrag.darwin.ts sends this IPC only on a left-button drag), so
// right-click context menus keep working on the same surface.
#[cfg(target_os = "macos")]
pub fn post_start_drag(state: &Arc<AppState>, label: &str) {
    let mut task = MacWindowDragTask::new(state.clone(), label.to_string());
    post_task(ThreadId::UI, Some(&mut task));
}

#[cfg(target_os = "macos")]
wrap_task! {
    pub struct MacWindowDragTask {
        state: Arc<AppState>,
        label: String,
    }

    impl Task {
        fn execute(&self) {
            if let Some(window) = get_window_on_ui(&self.state, &self.label) {
                // Host-side manual move loop (mirrors the Windows path). We do
                // NOT use performWindowDragWithEvent: — start_window_drag arrives
                // async over IPC, so [NSApp currentEvent] is not the original
                // title-bar mouse-down and the AppKit drag may never start.
                // Instead we pump NSLeftMouseDragged/Up ourselves and reposition
                // via CEF set_bounds, so the window always tracks the cursor.
                unsafe { run_macos_native_drag_loop(&window, &self.label) };
                // Dragging-reset safety net, same as the Linux task — resets the
                // renderer's dragging flag. (On non-Windows, tryRedockAtCursor is
                // driven from the DOM mouseup.)
                crate::events::emit_event_to_top_level_windows(
                    &self.state,
                    "window_drag_ended",
                    &serde_json::json!({ "label": &self.label, "moved": true }),
                );
            } else {
                tracing::warn!("[start_window_drag] no window for label={}", self.label);
            }
        }
    }
}

/// Host-side manual window-drag loop for macOS, run on the CEF UI thread in
/// response to the renderer's `start_window_drag` IPC. Mirrors the Windows
/// `Win32BeginMoveTask` approach instead of AppKit's
/// `performWindowDragWithEvent:`: that API needs the *original* mouse-down
/// event, but our IPC is async (renderer mousemove → IPC → post_task), so
/// `[NSApp currentEvent]` is unreliable and the drag can silently fail to
/// start. Here we pump `NSLeftMouseDragged`/`NSLeftMouseUp` ourselves and
/// reposition with CEF `set_bounds`, so the window always tracks the cursor.
///
/// Coordinates: CEF window bounds are DIP, top-left origin, y-down — the same
/// space the other window commands use (DOM `screenX/Y`). `[NSEvent
/// mouseLocation]` is screen points, bottom-left origin, y-up. Points == DIP on
/// macOS and we track deltas (absolute origin / screen height cancel out), so
/// only the vertical axis is flipped (`origin.y - dy`).
///
/// Raw libobjc FFI, mirroring `ensure_macos_native_window_buttons` in app.rs.
/// Like the Windows loop this blocks the UI thread until mouse-up — the
/// accepted trade-off for a window drag (content freezes briefly, as it does
/// for any native title-bar drag).
#[cfg(target_os = "macos")]
unsafe fn run_macos_native_drag_loop(window: &Window, label: &str) {
    use std::ffi::{c_char, c_void};
    type Id = *mut c_void;
    type Sel = *const c_void;
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct NSPoint {
        x: f64,
        y: f64,
    }
    extern "C" {
        fn sel_registerName(name: *const c_char) -> Sel;
        fn objc_getClass(name: *const c_char) -> Id;
        fn objc_msgSend();
    }
    // objc_msgSend transmuted per call signature (the app.rs idiom). NSPoint is
    // two doubles → returned in registers on both arm64 and x86_64, so plain
    // objc_msgSend is correct (no objc_msgSend_stret); that is why we reposition
    // via CEF set_bounds rather than reading an NSRect frame.
    let msg: extern "C" fn(Id, Sel) -> Id = std::mem::transmute(objc_msgSend as *const c_void);
    let msg_str: extern "C" fn(Id, Sel, *const c_char) -> Id =
        std::mem::transmute(objc_msgSend as *const c_void);
    let msg_point: extern "C" fn(Id, Sel) -> NSPoint =
        std::mem::transmute(objc_msgSend as *const c_void);
    let msg_uint: extern "C" fn(Id, Sel) -> u64 = std::mem::transmute(objc_msgSend as *const c_void);
    let msg_next: extern "C" fn(Id, Sel, u64, Id, Id, i8) -> Id =
        std::mem::transmute(objc_msgSend as *const c_void);
    // -[NSApp sendEvent:] — dispatch a (dequeued) event onward to its window.
    let msg_send_event: extern "C" fn(Id, Sel, Id) =
        std::mem::transmute(objc_msgSend as *const c_void);

    // NSApp = [NSApplication sharedApplication]
    let nsapp = msg(
        objc_getClass(b"NSApplication\0".as_ptr() as _),
        sel_registerName(b"sharedApplication\0".as_ptr() as _),
    );
    if nsapp.is_null() {
        tracing::warn!("[start_window_drag] macOS: NSApp nil label={}", label);
        return;
    }
    let nsevent_cls = objc_getClass(b"NSEvent\0".as_ptr() as _);
    let sel_mouse_location = sel_registerName(b"mouseLocation\0".as_ptr() as _);
    let nsdate_cls = objc_getClass(b"NSDate\0".as_ptr() as _);
    // untilDate: a SHORT (100ms) timeout, NOT distantFuture. start_window_drag
    // is async, so the NSLeftMouseUp can be consumed by the normal run loop
    // before this loop starts pumping; a distantFuture wait would then block
    // the UI (main) thread forever — a whole-app freeze. The timeout wakes the
    // loop ~10x/sec so the live-button check below can end the drag. Mirrors
    // the Windows 100ms SetTimer wake. (Fresh NSDate built each iteration.)
    let sel_date = sel_registerName(b"dateWithTimeIntervalSinceNow:\0".as_ptr() as _);
    let msg_date: extern "C" fn(Id, Sel, f64) -> Id =
        std::mem::transmute(objc_msgSend as *const c_void);
    // [NSEvent pressedMouseButtons] — LIVE hardware bitmask (bit 0 = left),
    // independent of which events have been dispatched; lets us detect a
    // release we never saw as an event. Mirrors Windows GetAsyncKeyState(VK_LBUTTON).
    let sel_pressed = sel_registerName(b"pressedMouseButtons\0".as_ptr() as _);
    // inMode: an NSString equal to the event-tracking run-loop mode (run-loop
    // modes compare by string value, so a fresh NSString is fine).
    let mode = msg_str(
        objc_getClass(b"NSString\0".as_ptr() as _),
        sel_registerName(b"stringWithUTF8String:\0".as_ptr() as _),
        b"NSEventTrackingRunLoopMode\0".as_ptr() as _,
    );
    let sel_next =
        sel_registerName(b"nextEventMatchingMask:untilDate:inMode:dequeue:\0".as_ptr() as _);
    let sel_type = sel_registerName(b"type\0".as_ptr() as _);
    let sel_send_event = sel_registerName(b"sendEvent:\0".as_ptr() as _);

    // NSEventMaskLeftMouseDragged (1<<6) | NSEventMaskLeftMouseUp (1<<2).
    const MASK: u64 = (1 << 6) | (1 << 2);
    const NS_LEFT_MOUSE_UP: u64 = 2; // NSEventTypeLeftMouseUp

    // Bail if the press already ended (fast flick-and-release, or this task was
    // posted late): the loop below would otherwise wait for a drag/up event
    // that will never come, freezing the UI thread. Mirrors the Windows
    // `!lbutton_down()` bail.
    if msg_uint(nsevent_cls, sel_pressed) & 1 == 0 {
        tracing::info!("[start_window_drag] macOS: button already up — skipping drag label={}", label);
        return;
    }

    let origin = window.bounds(); // DIP, top-left, y-down
    let start = msg_point(nsevent_cls, sel_mouse_location); // screen points, y-up

    loop {
        // untilDate: a fresh 100ms deadline each iteration (see above).
        let until = msg_date(nsdate_cls, sel_date, 0.1);
        // dequeue:YES (1) — consume the event, else the same pending event is
        // returned forever. The renderer doesn't need these mid-drag.
        let event = msg_next(nsapp, sel_next, MASK, until, mode, 1);
        if event.is_null() {
            // 100ms timeout, no event. Re-check the live button state: if the
            // left button is up we missed the NSLeftMouseUp (consumed elsewhere
            // during the async gap) — end the drag instead of blocking forever.
            // Otherwise the button is still held; keep tracking.
            if msg_uint(nsevent_cls, sel_pressed) & 1 == 0 {
                break;
            }
            continue;
        }
        if msg_uint(event, sel_type) == NS_LEFT_MOUSE_UP {
            // We dequeued this up (dequeue:YES), so the NSView never saw it.
            // Forward it via -[NSApp sendEvent:] so Chromium balances the
            // renderer's mousedown (the JS drag listener never preventDefault'd
            // it) and the DOM `mouseup` fires; otherwise the renderer is left
            // believing the left button is still down. Mirrors the Windows loop
            // below, which dispatches/synthesizes WM_LBUTTONUP for the same
            // reason. The timeout/button-already-up paths need no forward — the
            // up was already delivered through normal dispatch.
            msg_send_event(nsapp, sel_send_event, event);
            break;
        }
        let cur = msg_point(nsevent_cls, sel_mouse_location);
        let dx = (cur.x - start.x).round() as i32;
        let dy = (cur.y - start.y).round() as i32; // screen y-up
        window.set_bounds(Some(&Rect {
            x: origin.x + dx,
            y: origin.y - dy, // flip screen y-up → CEF y-down
            width: origin.width,
            height: origin.height,
        }));
    }
    tracing::info!("[start_window_drag] macOS: drag loop ended label={}", label);
}

// ── Win32 host-side manual native move loop ───────────────────────────────
//
// The raw `WM_NCLBUTTONDOWN(HTCAPTION)` trick does NOT work with a CEF/Chromium
// window: Chromium's HWNDMessageHandler swallows the non-client message and only
// runs an OS drag for `-webkit-app-region:drag` regions (which we can't use —
// they suppress renderer events). Instrumented proof: on the UI thread with
// `release_ok=true`, SendMessage(WM_NCLBUTTONDOWN) returned in 0.4ms — the move
// loop never engaged. So we run the move loop OURSELVES, on the UI thread (which
// owns the window + capture), driving `SetWindowPos` per mouse-move with zero
// per-move IPC. Full design + risks: SPEC_WINDOW_DRAG_MANUAL_MOVE_LOOP_2026_05_29.
#[cfg(target_os = "windows")]
wrap_task! {
    // `after_unmaximize` is true on the re-posted second half of a drag that
    // had to un-maximize first — see the `unmaximize_for_drag` call site in
    // `execute` for why that is split across two message-loop turns.
    // (Field-level doc comments don't compile inside `wrap_task!`.)
    pub struct Win32BeginMoveTask {
        hwnd: u64,
        state: Arc<AppState>,
        source_label: Option<String>,
        after_unmaximize: bool,
    }

    impl Task {
        fn execute(&self) {
            use windows_sys::Win32::Foundation::{HWND, POINT, RECT};
            use windows_sys::Win32::Graphics::Gdi::ScreenToClient;
            use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
                GetAsyncKeyState, GetCapture, ReleaseCapture, SetCapture, VK_ESCAPE, VK_LBUTTON,
            };
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                DispatchMessageW, GetCursorPos, GetMessageW, GetWindowRect, KillTimer,
                PeekMessageW, PostQuitMessage, SendMessageW, SetTimer, SetWindowPos,
                TranslateMessage, MSG, PM_REMOVE, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER,
                WM_KEYDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_TIMER,
            };

            let h = self.hwnd as HWND;
            // High bit of GetAsyncKeyState = key currently down.
            let lbutton_down = || unsafe {
                (GetAsyncKeyState(VK_LBUTTON as i32) as u16 & 0x8000) != 0
            };

            unsafe {
                // 0. Bail if the press already ended (fast click / late task).
                if !lbutton_down() {
                    tracing::info!(
                        "[start_window_drag] manual move loop skipped — button already up hwnd={:#x}",
                        self.hwnd
                    );
                    return;
                }

                // 0.5. Dragging a MAXIMIZED window restores it down under the
                //      cursor first — Chrome/Windows both do this, and without
                //      it a maximized window would either refuse to move or
                //      slide around at full-screen size.
                //
                //      Done BEFORE taking capture, deliberately: ShowWindow
                //      pumps messages and can disturb activation/capture, so
                //      it must not run between SetCapture and the loop that
                //      depends on that capture.
                //
                //      Then we RETURN and re-post ourselves, so the modal loop
                //      below starts on the NEXT message-loop turn instead of
                //      immediately. This is load-bearing, not tidiness: CEF
                //      runs its own integrated message loop (`run_message_loop`
                //      in lib.rs), and the loop below hijacks the UI thread
                //      with a nested `GetMessageW` pump. Starting it in the
                //      same turn as the resize starves Chromium of the
                //      relayout/compositor work the `WM_SIZE` just queued, so
                //      the window frame shrinks while the CONTENT keeps
                //      painting at its old full-screen size — visibly wrong
                //      for the whole drag, snapping right only on release when
                //      the normal pump resumes (reported live 2026-09-05).
                //      Yielding one turn costs well under a frame and the
                //      button is still down, so the drag is unaffected.
                //
                //      Floaters are excluded for the same reason they're
                //      excluded from the snap itself (SPEC §2.4): they are
                //      borderless WS_POPUPs with no native maximize placement
                //      — `toggle_floating_maximize` manages their geometry
                //      through the reducer instead, so SW_RESTORE here would
                //      desync that state.
                if !self.after_unmaximize
                    && !self
                        .source_label
                        .as_deref()
                        .unwrap_or("main")
                        .starts_with("floating-")
                    && unmaximize_for_drag(h)
                {
                    let mut task = Win32BeginMoveTask::new(
                        self.hwnd,
                        self.state.clone(),
                        self.source_label.clone(),
                        true,
                    );
                    post_task(ThreadId::UI, Some(&mut task));
                    return;
                }

                // 1. Capture the mouse to THIS (UI) thread so WM_MOUSEMOVE /
                //    WM_LBUTTONUP route to our GetMessage loop. ReleaseCapture
                //    first to drop any capture Chromium set on the press.
                ReleaseCapture();
                let prev_capture = SetCapture(h);
                // Confirm we actually hold capture; if not, mouse moves won't
                // reach our loop and the drag would silently no-op (reagent #1).
                if GetCapture() != h {
                    tracing::warn!(
                        "[start_window_drag] SetCapture did not take hwnd={:#x} prev={:#x}",
                        self.hwnd, prev_capture as usize
                    );
                }
                // Wake the loop ~10x/sec even with no mouse input, so a stolen
                // capture or a missed WM_LBUTTONUP can't hang the UI thread —
                // the top-of-loop button-state check then ends the drag
                // (reagent #2; the blocking-GetMessage hang noted in the spec).
                const DRAG_TICK_ID: usize = 0xD9A6;
                SetTimer(h, DRAG_TICK_ID, 100, None);

                // 2. Anchor in physical screen px — no devicePixelRatio math.
                let mut anchor = POINT { x: 0, y: 0 };
                GetCursorPos(&mut anchor);
                let mut r: RECT = std::mem::zeroed();
                GetWindowRect(h, &mut r);
                let (x0, y0) = (r.left, r.top);
                tracing::info!(
                    "[start_window_drag] manual move loop begin hwnd={:#x} anchor=({},{}) origin=({},{})",
                    self.hwnd, anchor.x, anchor.y, x0, y0
                );
                // Throttle state for floater redock-hover emit (§3.2 of the spec).
                // Pre-dated by 50ms so the first WM_MOUSEMOVE emits immediately.
                let mut last_hover_emit = std::time::Instant::now()
                    - std::time::Duration::from_millis(50);

                // Drag-to-top maximize (SPEC_WINDOW_SNAP_MAXIMIZE_2026_09_04 §2).
                // Only for real top-level windows: floaters have their own
                // top-of-screen semantics (redock / tear-back-in) that this
                // must not collide with — see the spec's §2.4 scope note.
                let snap_eligible = !self
                    .source_label
                    .as_deref()
                    .unwrap_or("main")
                    .starts_with("floating-");
                // Tracks zone membership so the preview is shown/hidden on
                // TRANSITIONS only, not re-issued on every mousemove tick.
                let mut in_snap_zone = false;

                // 3. Modal move loop on the UI thread.
                let mut msg: MSG = std::mem::zeroed();
                let mut moves: u32 = 0;
                let mut cancelled = false;
                loop {
                    // Safety net: GetAsyncKeyState reflects PHYSICAL button state,
                    // which can lead the message queue — so a release arriving
                    // just after a WM_MOUSEMOVE/WM_TIMER would trip this check and
                    // break BEFORE the queued WM_LBUTTONUP is dispatched, leaving
                    // the up to land off-window post-ReleaseCapture and Chromium's
                    // mousedown unbalanced (reagent P1 / spec §5.1). So before
                    // breaking, drain a queued WM_LBUTTONUP (capture still held)
                    // and dispatch it; if none is queued yet, synthesize one so
                    // the renderer always sees its balancing up.
                    if !lbutton_down() {
                        let mut up: MSG = std::mem::zeroed();
                        if PeekMessageW(&mut up, h, WM_LBUTTONUP, WM_LBUTTONUP, PM_REMOVE) != 0 {
                            DispatchMessageW(&up);
                        } else {
                            // Synthesize a balancing WM_LBUTTONUP so Chromium's
                            // mousedown is balanced. Encode the ACTUAL cursor
                            // position in lParam (client coords) so the DOM
                            // MouseEvent carries correct screenX/Y for redock.
                            // lParam=0 would encode client (0,0) = floater
                            // top-left, giving tryRedockAtCursor the wrong point.
                            let mut release_pt = POINT { x: 0, y: 0 };
                            GetCursorPos(&mut release_pt);
                            ScreenToClient(h, &mut release_pt);
                            let lp = (release_pt.x as u16 as i32)
                                | ((release_pt.y as u16 as i32) << 16);
                            SendMessageW(h, WM_LBUTTONUP, 0, lp as isize);
                        }
                        break;
                    }
                    let got = GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0);
                    if got <= 0 {
                        if got == 0 {
                            // WM_QUIT — don't swallow app shutdown.
                            PostQuitMessage(msg.wParam as i32);
                        }
                        break;
                    }
                    match msg.message {
                        // Guard both arms against messages sent to other
                        // HWNDs that arrived on this thread (accessibility
                        // tools, test automation). Messages for other windows
                        // fall through to the _ arm for normal dispatch.
                        WM_MOUSEMOVE if msg.hwnd == h => {
                            // After Esc-cancel we keep looping (so the renderer
                            // still gets its balancing WM_LBUTTONUP on release)
                            // but stop moving the window.
                            if !cancelled {
                                let mut cur = POINT { x: 0, y: 0 };
                                GetCursorPos(&mut cur);
                                SetWindowPos(
                                    h,
                                    std::ptr::null_mut(),
                                    x0 + (cur.x - anchor.x),
                                    y0 + (cur.y - anchor.y),
                                    0,
                                    0,
                                    SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                                );
                                moves += 1;
                                // Drag-to-top maximize: offer the snap while the
                                // cursor sits in the work area's top zone. Keyed
                                // off the CURSOR, not the window's own top edge —
                                // the window is following the cursor at whatever
                                // offset the user grabbed it, so its edge says
                                // nothing about intent (see window_snap's
                                // `cursor_in_top_maximize_zone` doc comment).
                                if snap_eligible {
                                    let zone = snap_work_area_for_cursor(cur.x, cur.y)
                                        .map(|(wx, wy, ww, wh)| {
                                            (
                                                crate::client::window_snap::cursor_in_top_maximize_zone(
                                                    cur.y,
                                                    wy,
                                                    crate::client::window_snap::SNAP_THRESHOLD_PX,
                                                ),
                                                (wx, wy, ww, wh),
                                            )
                                        });
                                    if let Some((now_in_zone, work)) = zone {
                                        if now_in_zone != in_snap_zone {
                                            in_snap_zone = now_in_zone;
                                            if now_in_zone {
                                                crate::ui_tasks::snap_preview::show(
                                                    work.0, work.1, work.2, work.3,
                                                );
                                            } else {
                                                crate::ui_tasks::snap_preview::hide();
                                            }
                                        }
                                    }
                                }
                                // Floater: emit redock-hover at 50ms cadence so the
                                // drop-target highlight tracks the cursor while the
                                // renderer's mousemove listener is dark (§2.2 / §3.2).
                                if let Some(sl) = self.source_label.as_deref() {
                                    if sl.starts_with("floating-")
                                        && last_hover_emit.elapsed()
                                            >= std::time::Duration::from_millis(50)
                                    {
                                        let hover_args = serde_json::json!({
                                            "source_label": sl,
                                            "x": cur.x,
                                            "y": cur.y,
                                        });
                                        let _ = crate::commands::window
                                            ::update_floating_redock_hover(
                                                &self.state,
                                                &hover_args,
                                            );
                                        last_hover_emit = std::time::Instant::now();
                                    }
                                }
                            }
                        }
                        WM_LBUTTONUP if msg.hwnd == h => {
                            // Let Chromium see the up so its input state stays
                            // balanced against the mousedown the renderer got
                            // (SPEC §5.1).
                            DispatchMessageW(&msg);
                            break;
                        }
                        WM_KEYDOWN if (msg.wParam as u16) == VK_ESCAPE => {
                            // Cancel: restore the start position, but DON'T break
                            // — keep capture and keep looping so the eventual
                            // WM_LBUTTONUP is still dispatched to balance the
                            // renderer's mousedown (reagent P1 / spec §5.1).
                            // Further moves are ignored via `cancelled`.
                            SetWindowPos(
                                h,
                                std::ptr::null_mut(),
                                x0,
                                y0,
                                0,
                                0,
                                SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
                            );
                            cancelled = true;
                            // Same for the snap preview: an Esc-cancelled drag
                            // must not leave a maximize offer on screen, and
                            // must not maximize on the eventual release (the
                            // commit below re-checks `cancelled` too).
                            in_snap_zone = false;
                            crate::ui_tasks::snap_preview::hide();
                            // Tear down the hover overlay immediately so the
                            // drop-target placeholder doesn't outlive the cancel.
                            let _ = crate::commands::window::clear_floating_redock_hover(
                                &self.state,
                                &serde_json::Value::Null,
                            );
                            // Signal the renderer to suppress redock-on-release
                            // for this drag (§2.3 / §3.2 of the spec).
                            crate::events::emit_event_to_top_level_windows(
                                &self.state,
                                "window_drag_cancelled",
                                &serde_json::json!({
                                    "label": self.source_label.as_deref().unwrap_or("main")
                                }),
                            );
                        }
                        WM_TIMER if msg.wParam == DRAG_TICK_ID => {
                            // Our wake tick — consume ONLY ours; the top-of-loop
                            // button-state check re-runs on the next iteration.
                            // Other timers fall through to the `_` arm so CEF's
                            // own timers aren't dropped during the drag.
                        }
                        _ => {
                            // Keep the app alive (paint, DPI changes, sent msgs).
                            TranslateMessage(&msg);
                            DispatchMessageW(&msg);
                        }
                    }
                }

                // 4. Stop the wake tick and release capture.
                KillTimer(h, DRAG_TICK_ID);

                // Drag-to-top maximize: commit iff the drag ended inside the
                // zone and wasn't cancelled. Unconditionally hide the preview
                // FIRST — this is the single exit path every `break` above
                // funnels through, so clearing here is what guarantees the
                // overlay can never outlive the drag regardless of how the
                // loop ended (release, Esc-then-release, WM_QUIT, stolen
                // capture via the wake-tick safety net).
                crate::ui_tasks::snap_preview::hide();
                if in_snap_zone && !cancelled {
                    tracing::info!(
                        "[start_window_drag] released in top snap zone — maximizing hwnd={:#x}",
                        self.hwnd
                    );
                    // NOT the toggle variant: the gesture means "maximize",
                    // and toggling would restore-down a window that was
                    // already maximized when the drag began.
                    crate::commands::window::maximize_hwnd(h as *mut std::ffi::c_void);
                }
                // Capture cursor position before ReleaseCapture — this is the
                // actual release point in physical screen px. Included in the
                // window_drag_ended payload so the renderer can use these
                // coordinates directly, eliminating the ordering race between
                // the dispatched WM_LBUTTONUP (DOM mouseup) and this event
                // (both travel async CEF IPC, relative arrival is not guaranteed).
                let mut release_cursor = POINT { x: 0, y: 0 };
                GetCursorPos(&mut release_cursor);
                ReleaseCapture();
                // Notify the renderer that the drag is done. cursor_x/cursor_y
                // are physical screen px; renderer divides by posScale() (DPR on
                // Windows, 1 elsewhere) to get CSS px for tryRedockAtCursor.
                crate::events::emit_event_to_top_level_windows(
                    &self.state,
                    "window_drag_ended",
                    &serde_json::json!({
                        "label": self.source_label.as_deref().unwrap_or("main"),
                        "moved": !cancelled && moves > 0,
                        "cursor_x": release_cursor.x,
                        "cursor_y": release_cursor.y,
                    }),
                );
                tracing::info!(
                    "[start_window_drag] manual move loop end hwnd={:#x} moves={}",
                    self.hwnd, moves
                );
            }
        }
    }
}

/// If `hwnd` is maximized, restore it and reposition it under the cursor so
/// the drag can continue naturally — see
/// `window_snap::unmaximize_drag_origin` for the placement rules and
/// `SPEC_WINDOW_SNAP_MAXIMIZE_2026_09_04.md` §2.6.
///
/// No-op for a window that isn't maximized (the overwhelmingly common case),
/// so callers can invoke it unconditionally at drag start.
///
/// Returns `true` only when it actually restored a maximized window — the
/// caller uses that to decide whether it needs to yield a message-loop turn
/// before starting the modal drag loop (see the call site).
#[cfg(target_os = "windows")]
unsafe fn unmaximize_for_drag(h: windows_sys::Win32::Foundation::HWND) -> bool {
    use windows_sys::Win32::Foundation::{POINT, RECT};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetCursorPos, GetWindowPlacement, GetWindowRect, SetWindowPos, ShowWindow, SW_MAXIMIZE,
        SW_RESTORE, SWP_NOACTIVATE, SWP_NOSIZE, SWP_NOZORDER, WINDOWPLACEMENT,
    };

    let mut placement: WINDOWPLACEMENT = std::mem::zeroed();
    placement.length = std::mem::size_of::<WINDOWPLACEMENT>() as u32;
    if GetWindowPlacement(h, &mut placement) == 0 || placement.showCmd != SW_MAXIMIZE as u32 {
        return false;
    }

    let mut maximized: RECT = std::mem::zeroed();
    if GetWindowRect(h, &mut maximized) == 0 {
        return false;
    }
    let mut cursor = POINT { x: 0, y: 0 };
    GetCursorPos(&mut cursor);

    ShowWindow(h, SW_RESTORE);

    // Read the restored size back rather than trusting
    // `placement.rcNormalPosition`: that field is documented in *workspace*
    // coordinates, which diverge from screen coordinates in exactly the
    // multi-monitor / taskbar setups this feature has to work in. The
    // post-restore GetWindowRect is unambiguously screen coords.
    let mut restored: RECT = std::mem::zeroed();
    if GetWindowRect(h, &mut restored) == 0 {
        // Already un-maximized by ShowWindow above, so the caller still needs
        // to yield a turn for the relayout even though placement failed.
        return true;
    }
    let (new_x, new_y) = crate::client::window_snap::unmaximize_drag_origin(
        cursor.x,
        cursor.y,
        maximized.left,
        maximized.top,
        maximized.right - maximized.left,
        restored.right - restored.left,
    );
    SetWindowPos(
        h,
        std::ptr::null_mut(),
        new_x,
        new_y,
        0,
        0,
        SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE,
    );
    tracing::info!(
        "[start_window_drag] un-maximized for drag hwnd={:p} -> ({},{})",
        h, new_x, new_y
    );
    true
}

/// Work area (PHYSICAL px, `(x, y, width, height)`) of the monitor under the
/// cursor, for the drag-to-top snap zone + preview rect.
///
/// `MONITOR_DEFAULTTONEAREST` so a cursor dragged slightly past the top of
/// the screen still resolves to the monitor it just left rather than
/// failing. Deliberately does NOT use `app::monitor::get_monitor_work_area`
/// — that converts to DIP for CEF's `Window::set_bounds`, while this whole
/// move loop is physical px ("no devicePixelRatio math", per its anchor
/// comment above). Mixing them is the unit-confusion bug class called out in
/// `client::window_snap`'s doc comment.
#[cfg(target_os = "windows")]
unsafe fn snap_work_area_for_cursor(x: i32, y: i32) -> Option<(i32, i32, i32, i32)> {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::Graphics::Gdi::{
        GetMonitorInfoW, MonitorFromPoint, MONITORINFO, MONITOR_DEFAULTTONEAREST,
    };

    let hmonitor = MonitorFromPoint(POINT { x, y }, MONITOR_DEFAULTTONEAREST);
    if hmonitor.is_null() {
        return None;
    }
    let mut info: MONITORINFO = std::mem::zeroed();
    info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
    if GetMonitorInfoW(hmonitor, &mut info) == 0 {
        return None;
    }
    let rc = info.rcWork;
    Some((rc.left, rc.top, rc.right - rc.left, rc.bottom - rc.top))
}

#[cfg(target_os = "windows")]
pub fn post_win32_begin_move(hwnd: u64, state: Arc<AppState>, source_label: Option<String>) {
    let mut task = Win32BeginMoveTask::new(hwnd, state, source_label, false);
    post_task(ThreadId::UI, Some(&mut task));
}
