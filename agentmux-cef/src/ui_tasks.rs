// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// CEF UI thread task dispatch.
//
// All CEF Views operations (Window::close, minimize, maximize, etc.) must run
// on the CEF UI thread. IPC commands arrive on tokio threads. This module
// provides tasks that can be posted to the UI thread via post_task().
//
// Key insight: don't pass Browser/Window handles across threads. Instead,
// pass Arc<AppState> and look up the browser on the UI thread.
//
// Used on Linux (and macOS). On Windows, Win32 APIs are used directly since
// they are safe to call from any thread.

use std::sync::Arc;
use cef::*;
use crate::state::AppState;

/// Get the CEF Views Window for a browser label on the UI thread.
fn get_window_on_ui(state: &Arc<AppState>, label: &str) -> Option<Window> {
    // Phase H.2.b — reducer-aware lookup with fallback.
    let mut browser = state.get_browser(label)?;
    let browser_view = browser_view_get_for_browser(Some(&mut browser))?;
    browser_view.window()
}

// ── Deferred load_url (used by on_before_popup to avoid UI-thread deadlock)
//
// Calling `frame.load_url(url)` synchronously inside a CEF callback that
// holds the handler's inner lock (e.g. `on_before_popup`) deadlocks on
// link clicks: `load_url` kicks a new navigation which triggers
// `on_loading_state_change` on the same thread, which also wants the
// handler's lock. Posting the navigate as a separate UI task lets the
// original callback return, release its lock, and the load starts
// cleanly on the next message-loop turn. ─────────────────────────────────

wrap_task! {
    pub struct DeferredLoadUrlTask {
        browser: Browser,
        url: String,
    }

    impl Task {
        fn execute(&self) {
            let mut browser = self.browser.clone();
            if let Some(frame) = browser.main_frame() {
                frame.load_url(Some(&CefString::from(self.url.as_str())));
            }
        }
    }
}

// ── Close ────────────────────────────────────────────────────────────────

wrap_task! {
    pub struct CloseWindowTask {
        state: Arc<AppState>,
        label: String,
    }

    impl Task {
        fn execute(&self) {
            // Use try_close_browser rather than window.close() — the latter
            // calls Widget::Close directly, which CHECKs !on_call_stack_ and
            // aborts if the widget is already being destroyed (e.g. the OS
            // sent windowShouldClose on macOS while our close_window IPC task
            // was already queued). try_close_browser goes through do_close
            // which sets is_closing and is idempotent on re-entry.
            if let Some(mut browser) = self.state.get_browser(&self.label) {
                if let Some(host) = browser.host() {
                    host.try_close_browser();
                }
            }
        }
    }
}

pub fn post_close_window(state: &Arc<AppState>, label: &str) {
    let mut task = CloseWindowTask::new(state.clone(), label.to_string());
    post_task(ThreadId::UI, Some(&mut task));
}

// ── Minimize ─────────────────────────────────────────────────────────────

wrap_task! {
    pub struct MinimizeWindowTask {
        state: Arc<AppState>,
        label: String,
    }

    impl Task {
        fn execute(&self) {
            if let Some(window) = get_window_on_ui(&self.state, &self.label) {
                window.minimize();
            }
        }
    }
}

pub fn post_minimize_window(state: &Arc<AppState>, label: &str) {
    let mut task = MinimizeWindowTask::new(state.clone(), label.to_string());
    post_task(ThreadId::UI, Some(&mut task));
}

// ── Maximize (toggle) ────────────────────────────────────────────────────

wrap_task! {
    pub struct MaximizeWindowTask {
        state: Arc<AppState>,
        label: String,
    }

    impl Task {
        fn execute(&self) {
            if let Some(window) = get_window_on_ui(&self.state, &self.label) {
                if window.is_maximized() != 0 {
                    window.restore();
                } else {
                    window.maximize();
                }
            }
        }
    }
}

pub fn post_maximize_window(state: &Arc<AppState>, label: &str) {
    let mut task = MaximizeWindowTask::new(state.clone(), label.to_string());
    post_task(ThreadId::UI, Some(&mut task));
}

// ── Focus/Activate ───────────────────────────────────────────────────────

wrap_task! {
    pub struct FocusWindowTask {
        state: Arc<AppState>,
        label: String,
    }

    impl Task {
        fn execute(&self) {
            if let Some(window) = get_window_on_ui(&self.state, &self.label) {
                window.activate();
            }
        }
    }
}

pub fn post_focus_window(state: &Arc<AppState>, label: &str) {
    let mut task = FocusWindowTask::new(state.clone(), label.to_string());
    post_task(ThreadId::UI, Some(&mut task));
}

// ── Drag ─────────────────────────────────────────────────────────────────
// CEF Views does not expose a programmatic drag-initiation API.
// Begin a native window-move via the underlying CefWindow's BeginWindowDrag().
// Posted on the CEF UI thread (BeginWindowDrag must be called there). On
// Linux/Wayland this dispatches WmMoveResizeHandler::DispatchHostWindowDragMovement
// → xdg_toplevel.move with the most recent input serial; on X11 it dispatches
// an XEvent for _NET_WM_MOVERESIZE; on macOS it begins a system move loop.
// The compositor handles the drag until the user releases the mouse button.
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

#[cfg(target_os = "windows")]
pub fn post_start_drag(_state: &Arc<AppState>, _label: &str) {}

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
    pub struct Win32BeginMoveTask {
        hwnd: u64,
        state: Arc<AppState>,
        source_label: Option<String>,
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

#[cfg(target_os = "windows")]
pub fn post_win32_begin_move(hwnd: u64, state: Arc<AppState>, source_label: Option<String>) {
    let mut task = Win32BeginMoveTask::new(hwnd, state, source_label);
    post_task(ThreadId::UI, Some(&mut task));
}

// ── Move window ───────────────────────────────────────────────────────────

wrap_task! {
    pub struct MoveWindowTask {
        state: Arc<AppState>,
        label: String,
        dx: i32,
        dy: i32,
    }

    impl Task {
        fn execute(&self) {
            if let Some(window) = get_window_on_ui(&self.state, &self.label) {
                let bounds = window.bounds();
                window.set_bounds(Some(&Rect {
                    x: bounds.x + self.dx,
                    y: bounds.y + self.dy,
                    width: bounds.width,
                    height: bounds.height,
                }));
            }
        }
    }
}

pub fn post_move_window(state: &Arc<AppState>, label: &str, dx: i32, dy: i32) {
    let mut task = MoveWindowTask::new(state.clone(), label.to_string(), dx, dy);
    post_task(ThreadId::UI, Some(&mut task));
}

// ── Set window to absolute position ──────────────────────────────────────

wrap_task! {
    pub struct SetWindowPositionTask {
        state: Arc<AppState>,
        label: String,
        x: i32,
        y: i32,
    }

    impl Task {
        fn execute(&self) {
            if let Some(window) = get_window_on_ui(&self.state, &self.label) {
                let bounds = window.bounds();
                window.set_bounds(Some(&Rect {
                    x: self.x,
                    y: self.y,
                    width: bounds.width,
                    height: bounds.height,
                }));
            }
        }
    }
}

pub fn post_set_window_position(state: &Arc<AppState>, label: &str, x: i32, y: i32) {
    let mut task = SetWindowPositionTask::new(state.clone(), label.to_string(), x, y);
    post_task(ThreadId::UI, Some(&mut task));
}

// ── Get window absolute position (DIP) — blocking UI-thread read ──────────
//
// CEF Views `window.bounds()` must run on the UI thread, but
// `get_window_position` is a synchronous IPC command dispatched on the
// (non-UI) IPC thread. Post a task that reads the bounds on the UI thread and
// hand the DIP origin back over a bounded channel. Used by the macOS / Linux
// floating-pane header drag, which needs the window's current position as the
// absolute-move baseline (Windows reads it directly via GetWindowRect, which
// is thread-agnostic).
wrap_task! {
    pub struct GetWindowPositionTask {
        state: Arc<AppState>,
        label: String,
        tx: std::sync::mpsc::SyncSender<Option<(i32, i32)>>,
    }

    impl Task {
        fn execute(&self) {
            let pos = get_window_on_ui(&self.state, &self.label).map(|w| {
                let b = w.bounds();
                (b.x, b.y)
            });
            // Capacity-1, freshly created per call → try_send never blocks
            // the UI thread.
            let _ = self.tx.try_send(pos);
        }
    }
}

/// Read a CEF Views window's absolute position (DIP) from the IPC thread by
/// bouncing through the UI thread. `None` if the window isn't found or the UI
/// thread doesn't answer within the timeout (e.g. mid-teardown).
pub fn get_window_position_blocking(state: &Arc<AppState>, label: &str) -> Option<(i32, i32)> {
    let (tx, rx) = std::sync::mpsc::sync_channel::<Option<(i32, i32)>>(1);
    let mut task = GetWindowPositionTask::new(state.clone(), label.to_string(), tx);
    post_task(ThreadId::UI, Some(&mut task));
    rx.recv_timeout(std::time::Duration::from_millis(250)).ok().flatten()
}

// ── Resolve which window is under a screen point (DIP) — blocking UI read ──
//
// The macOS/Linux analogue of the Windows HWND Z-order walk in
// `commands/window/motion.rs::resolve_window_at_cursor`. Used by floating-pane
// REDOCK to find the AgentMux window the cursor is over at drop time. CEF Views
// `bounds()` must run on the UI thread, so iterate the registered top-level
// windows there and hit-test the DIP point against each.
//
// Overlap rule (pragmatic first cut — see the redock report): exclude the drag
// source; among the rest, prefer a non-"main" match (a floater/tear-off stacked
// above main is almost always the intended target) over "main"; "main" wins
// only when it's the sole match. True Z-order among multiple overlapping
// non-main windows is a follow-up (would need `[NSApp orderedWindows]` + a
// label↔NSWindow registry).
#[cfg(not(target_os = "windows"))]
wrap_task! {
    pub struct ResolveWindowAtCursorTask {
        state: Arc<AppState>,
        x: i32,
        y: i32,
        exclude_label: String,
        tx: std::sync::mpsc::SyncSender<Option<String>>,
    }

    impl Task {
        fn execute(&self) {
            let windows = self.state.windows.lock();
            let mut main_match = false;
            let mut best_other: Option<String> = None;
            for (label, window) in windows.iter() {
                if label.as_str() == self.exclude_label {
                    continue;
                }
                let b = window.bounds();
                let hit = self.x >= b.x
                    && self.x < b.x + b.width
                    && self.y >= b.y
                    && self.y < b.y + b.height;
                if !hit {
                    continue;
                }
                if label.as_str() == "main" {
                    main_match = true;
                } else {
                    // Deterministic pick among overlapping non-main windows:
                    // lexicographically smallest label. (HashMap iteration
                    // order is otherwise nondeterministic.)
                    match &best_other {
                        Some(cur) if cur.as_str() <= label.as_str() => {}
                        _ => best_other = Some(label.clone()),
                    }
                }
            }
            let result = best_other.or(if main_match { Some("main".to_string()) } else { None });
            let _ = self.tx.try_send(result);
        }
    }
}

/// Resolve the label of the top-most AgentMux window containing the DIP screen
/// point `(x, y)`, excluding `exclude_label` (the drag source). `None` if the
/// point is over the desktop / an external app / only the source window, or if
/// the UI thread doesn't answer within the timeout.
#[cfg(not(target_os = "windows"))]
pub fn resolve_window_at_cursor_blocking(
    state: &Arc<AppState>,
    x: i32,
    y: i32,
    exclude_label: &str,
) -> Option<String> {
    let (tx, rx) = std::sync::mpsc::sync_channel::<Option<String>>(1);
    let mut task =
        ResolveWindowAtCursorTask::new(state.clone(), x, y, exclude_label.to_string(), tx);
    post_task(ThreadId::UI, Some(&mut task));
    rx.recv_timeout(std::time::Duration::from_millis(250)).ok().flatten()
}

// ── Phase B.9.2 (WRR) — corrective absolute-position move ─────────────────
//
// Reducer-driven self-heal. Triggered by `Event::CorrectiveWindowMove` when
// the reducer detects an off-monitor / sentinel-parked window that the user
// has never foregrounded. We bypass `state.browsers` lookup-by-label (the
// label might not be registered yet at correction time) and use Win32
// SetWindowPos directly against the HWND. Must run on the UI thread because
// CEF Views' window backing the HWND is owned by the UI thread.

wrap_task! {
    pub struct CorrectiveWindowMoveTask {
        state: Arc<AppState>,
        hwnd: u64,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
    }

    impl Task {
        fn execute(&self) {
            #[cfg(target_os = "windows")]
            unsafe {
                use windows_sys::Win32::Foundation::HWND;
                use windows_sys::Win32::UI::WindowsAndMessaging::{
                    SetWindowPos, SWP_NOACTIVATE, SWP_NOZORDER,
                };
                let h = self.hwnd as HWND;
                let ok = SetWindowPos(
                    h,
                    std::ptr::null_mut(),
                    self.x,
                    self.y,
                    self.w,
                    self.h,
                    SWP_NOZORDER | SWP_NOACTIVATE,
                );
                tracing::info!(
                    target: "wrr",
                    "[wrr] corrective SetWindowPos hwnd={:#x} -> ({},{}) {}x{} ok={}",
                    self.hwnd, self.x, self.y, self.w, self.h, ok != 0
                );
            }
            #[cfg(not(target_os = "windows"))]
            {
                let _ = &self.state; // suppress unused on non-Windows
                tracing::warn!(
                    target: "wrr",
                    "[wrr] corrective move requested on non-Windows host: ignored"
                );
            }
        }
    }
}

pub fn post_corrective_window_move(state: &Arc<AppState>, hwnd: u64, x: i32, y: i32, w: i32, h: i32) {
    let mut task = CorrectiveWindowMoveTask::new(state.clone(), hwnd, x, y, w, h);
    post_task(ThreadId::UI, Some(&mut task));
}

// Phase B.9.3 (WRR) — `Event::HostShouldQuit` handling lives in
// `launcher_ipc::apply_event_to_shadow`. After three smoke
// iterations (v0.33.491–v0.33.493) confirmed `cef::post_task`
// silently drops new tasks during the last-window-closed
// teardown window — even when previously-posted tasks still
// run — we bypass CEF entirely and use Win32
// `PostThreadMessage(host_main_tid, WM_QUIT, 0, 0)` via
// `wrr::win_event::post_thread_quit_message`. The UI thread's
// captured TID is stored at `install_hooks` time.

// ── Create new window (CEF Views) ───────────────────────────────────────

wrap_task! {
    pub struct CreateWindowTask {
        state: Arc<AppState>,
        url: String,
        label: String,
        x: i32,
        y: i32,
        w: i32,
        h: i32,
        frameless: bool,
    }

    impl Task {
        fn execute(&self) {
            use std::cell::RefCell;

            // Phase 1 diagnostic tracing — see
            // docs/specs/SPEC_HOST_WINDOW_CREATION_RUNNER_2026-05-02.md.
            // Identify which exact CEF call wedges the UI thread under
            // concurrent window creation.
            let t0 = std::time::Instant::now();
            tracing::info!(label = %self.label, "[create-window] task entered UI thread");

            let settings = BrowserSettings {
                // ARGB alpha=0 → transparent, mirroring the MAIN window
                // (app.rs:679) and the global CefSettings.background_color
                // (main.rs). CreateWindowTask builds every secondary window
                // on Linux/macOS — additional windows AND floating-pane
                // tear-offs (open_floating_pane_window routes here on
                // non-Windows; the dedicated post_create_floating_window is
                // Windows-only). Previously hard-coded 0xFF000000 (opaque
                // black), which (a) overrode the transparent global default
                // and (b) gated OFF the BrowserViewImpl transparency cascade
                // (it only fires when default_background_color_ is
                // transparent — see cef/libcef/browser/views/browser_view_impl.cc
                // WebContentsCreated). Result: floaters/secondary windows were
                // fully opaque even when window:transparent=true. 0x00000000
                // lets them inherit the same transparency path as main.
                background_color: 0x00000000,
                ..Default::default()
            };
            let cef_url = CefString::from(self.url.as_str());

            // Get client from an existing TOP-LEVEL browser.
            // Use list_top_level_browsers() rather than list_browsers() +
            // manual filter — the dedicated helper already excludes pane
            // browsers (kind: Pane{..}), removing the label-prefix heuristic.
            //
            // GUARD: if no top-level browser is alive at this point (race
            // between window close → UnregisterBrowser and this task being
            // posted, or all windows closing during a multi-window tear-off),
            // bail early rather than passing None to browser_view_create.
            // CEF's C++ layer CHECK-fails on a null client → SIGABRT on
            // CrBrowserMain. A graceful return here lets the launcher's crash-
            // budget supervisor retry (with --disable-gpu) only on real CEF
            // faults, not on this transient race.
            let client = self
                .state
                .list_top_level_browsers()
                .into_iter()
                .find_map(|(_, b)| {
                    b.host().and_then(|h| h.client())
                });
            tracing::info!(
                label = %self.label,
                elapsed_us = t0.elapsed().as_micros() as u64,
                client_found = client.is_some(),
                "[create-window] got client"
            );

            let mut client_ref = match client {
                Some(c) => c,
                None => {
                    tracing::error!(
                        label = %self.label,
                        elapsed_us = t0.elapsed().as_micros() as u64,
                        "[create-window] no live top-level browser to clone client from \
                         (all windows closing?) — aborting window creation"
                    );
                    return;
                }
            };

            let mut request_context = crate::commands::create_isolated_request_context(
                &self.state, &self.label,
            );
            tracing::info!(
                label = %self.label,
                elapsed_us = t0.elapsed().as_micros() as u64,
                "[create-window] request_context resolved"
            );
            let mut bv_delegate = crate::app::AgentMuxBrowserViewDelegate::new(
                RuntimeStyle::ALLOY,
            );
            let browser_view = browser_view_create(
                Some(&mut client_ref),
                Some(&cef_url),
                Some(&settings),
                None,
                request_context.as_mut(),
                Some(&mut bv_delegate),
            );
            tracing::info!(
                label = %self.label,
                elapsed_us = t0.elapsed().as_micros() as u64,
                "[create-window] browser_view_create returned"
            );

            let mut wd = crate::app::AgentMuxWindowDelegate::new(
                RefCell::new(browser_view),
                Some((self.x, self.y, self.w, self.h)),
                self.frameless,
                RuntimeStyle::ALLOY,
                Some((self.state.clone(), self.label.clone())),
            );
            #[cfg(target_os = "linux")]
            crate::app::install_linux_window_properties_override(&wd);
            window_create_top_level(Some(&mut wd));
            tracing::info!(
                label = %self.label,
                elapsed_us = t0.elapsed().as_micros() as u64,
                "[create-window] window_create_top_level returned"
            );
        }
    }
}

pub fn post_create_window(
    state: &Arc<AppState>,
    url: &str,
    label: &str,
    x: i32, y: i32, w: i32, h: i32,
    frameless: bool,
) {
    let mut task = CreateWindowTask::new(
        state.clone(), url.to_string(), label.to_string(),
        x, y, w, h, frameless,
    );
    post_task(ThreadId::UI, Some(&mut task));
}

// ── DevTools (toggle) ─────────────────────────────────────────────────────

wrap_task! {
    pub struct ShowDevToolsTask {
        state: Arc<AppState>,
        label: String,
    }

    impl Task {
        fn execute(&self) {
            // Phase H.2.b — reducer-aware lookup with fallback.
            let browser = match self.state.get_browser(&self.label) {
                Some(b) => b,
                None => {
                    tracing::warn!("[devtools] browser '{}' not found", self.label);
                    return;
                }
            };

            match browser.host() {
                Some(host) => {
                    // In CEF Views mode, window_info is ignored by show_dev_tools().
                    // CEF routes the DevTools popup through on_popup_browser_view_created
                    // in AgentMuxBrowserViewDelegate, which creates a native window for it.
                    if host.has_dev_tools() != 0 {
                        host.close_dev_tools();
                    } else {
                        host.show_dev_tools(None, None, None, None);
                    }
                }
                None => {
                    tracing::warn!("[devtools] no browser host for '{}'", self.label);
                }
            }
        }
    }
}

pub fn post_show_dev_tools(state: &Arc<AppState>, label: &str) {
    let mut task = ShowDevToolsTask::new(state.clone(), label.to_string());
    post_task(ThreadId::UI, Some(&mut task));
}

// ── DevTools — Inspect Element at coordinates ─────────────────────────────

wrap_task! {
    pub struct InspectElementAtTask {
        state: Arc<AppState>,
        label: String,
        x: i32,
        y: i32,
    }

    impl Task {
        fn execute(&self) {
            let browser = match self.state.get_browser(&self.label) {
                Some(b) => b,
                None => {
                    tracing::warn!("[devtools] inspect-at: browser '{}' not found", self.label);
                    return;
                }
            };

            match browser.host() {
                Some(host) => {
                    // The 4th arg to show_dev_tools is `inspect_element_at: Option<CefPoint>`
                    // in window-relative coords. CEF opens DevTools (creating it if not
                    // already open) and selects the element at that point, equivalent to
                    // Chrome's right-click → Inspect Element flow.
                    let point = Point { x: self.x, y: self.y };
                    host.show_dev_tools(None, None, None, Some(&point));
                }
                None => {
                    tracing::warn!("[devtools] inspect-at: no browser host for '{}'", self.label);
                }
            }
        }
    }
}

pub fn post_inspect_element_at(state: &Arc<AppState>, label: &str, x: i32, y: i32) {
    let mut task = InspectElementAtTask::new(state.clone(), label.to_string(), x, y);
    post_task(ThreadId::UI, Some(&mut task));
}

// ── Main-focus reclaim ────────────────────────────────────────────────────
//
// Reclaim keyboard focus for the main browser when the user clicks a
// main-DOM input (address bar, etc). Runs on the CEF UI thread because:
//   - host.set_focus / browser_view_get_for_browser require the UI thread
//   - walking the HWND tree via EnumChildWindows is safer post-setup when
//     Chromium has published all of its render widgets
//
// On Windows, after the Chromium-level focus flip we also walk the Views
// window for the Chrome_RenderWidgetHostHWND and Win32-SetFocus it — without
// that explicit Win32 SetFocus, keyboard events keep routing to whichever
// pane HWND currently holds Win32 focus even though Chromium "thinks" main
// is focused. Observed on v0.33.264: host.set_focus(1) on main left pane
// keystrokes arriving at the pane HWND for >2 seconds.

wrap_task! {
    pub struct MainFocusReclaimTask {
        state: Arc<AppState>,
        label: String,
    }

    impl Task {
        fn execute(&self) {
            // Phase H.2.b — reducer-aware lookup with fallback.
            let mut browser = match self.state.get_browser(&self.label) {
                Some(b) => b,
                None => {
                    tracing::warn!("[main-focus-reclaim] no browser for label={}", self.label);
                    return;
                }
            };

            if let Some(host) = browser.host() {
                host.set_focus(1);
                tracing::info!("[main-focus-reclaim] host.set_focus(1) on label={}", self.label);
            }

            #[cfg(target_os = "windows")]
            {
                let views_top_hwnd = browser_view_get_for_browser(Some(&mut browser))
                    .and_then(|bv| bv.window())
                    .map(|w| w.window_handle().0 as *mut std::ffi::c_void)
                    .filter(|p| !p.is_null());

                // Collect every pane's outer HWND so we can skip render widgets
                // that descend from them. Panes are siblings of main under the
                // Views top-level, so a naive EnumChildWindows would pick up
                // their Chrome_RenderWidgetHostHWND and SetFocus on the wrong
                // target.
                // Phase H.2.b — reducer-aware iteration with fallback.
                let pane_outer_hwnds: Vec<*mut std::ffi::c_void> = self
                    .state
                    .list_browsers()
                    .into_iter()
                    .filter(|(k, _)| k.starts_with("browser-pane-"))
                    .filter_map(|(_, mut b)| {
                        b.host().and_then(|h| {
                            let wh = h.window_handle();
                            if wh.0.is_null() { None } else { Some(wh.0 as *mut std::ffi::c_void) }
                        })
                    })
                    .collect();

                match views_top_hwnd {
                    Some(top_hwnd) => unsafe {
                        let render = find_main_render_widget(top_hwnd, &pane_outer_hwnds);
                        let target = render.unwrap_or(top_hwnd);
                        windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus(target as _);
                        crate::browser_pane::hwnd::record_intentional_focus(target);
                        tracing::info!(
                            "[main-focus-reclaim] Win32 SetFocus target={:p} render_found={} panes_excluded={}",
                            target,
                            render.is_some(),
                            pane_outer_hwnds.len(),
                        );
                    },
                    None => {
                        tracing::warn!(
                            "[main-focus-reclaim] could not resolve Views top-level HWND for label={}",
                            self.label,
                        );
                    }
                }
            }

            // Defocus all live panes at the Chromium level too.
            self.state.browser_panes.defocus_all(&self.state);
        }
    }
}

/// Walk descendants of `root` and return the first Chrome_RenderWidgetHostHWND
/// whose ancestor chain does NOT pass through any of `pane_outer_hwnds`.
/// Panes are siblings of main under the Views top-level, so without this
/// filter the walk would happily pick a pane's render widget.
#[cfg(target_os = "windows")]
unsafe fn find_main_render_widget(
    root: *mut std::ffi::c_void,
    pane_outer_hwnds: &[*mut std::ffi::c_void],
) -> Option<*mut std::ffi::c_void> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumChildWindows, GetClassNameW, GetParent,
    };

    struct Finder<'a> {
        found: *mut std::ffi::c_void,
        panes: &'a [*mut std::ffi::c_void],
    }
    let mut finder = Finder { found: std::ptr::null_mut(), panes: pane_outer_hwnds };

    unsafe extern "system" fn cb(hwnd: *mut std::ffi::c_void, lparam: isize) -> i32 {
        let finder = &mut *(lparam as *mut Finder);
        let mut buf = [0u16; 64];
        let n = GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
        if n > 0 {
            let class = String::from_utf16_lossy(&buf[..n as usize]);
            if class == "Chrome_RenderWidgetHostHWND" {
                // Walk ancestors; if we pass through any pane outer HWND,
                // this widget belongs to a pane, not main.
                let mut descends_from_pane = false;
                let mut cursor = GetParent(hwnd);
                while !cursor.is_null() {
                    if finder.panes.iter().any(|p| *p == cursor) {
                        descends_from_pane = true;
                        break;
                    }
                    cursor = GetParent(cursor);
                }
                if !descends_from_pane {
                    finder.found = hwnd;
                    return 0; // stop
                }
            }
        }
        1
    }

    EnumChildWindows(root, Some(cb), &mut finder as *mut _ as isize);
    if finder.found.is_null() { None } else { Some(finder.found) }
}
