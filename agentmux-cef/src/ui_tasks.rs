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

// macOS: swizzle storage for NativeWidgetMacNSWindow::isMainWindow / isKeyWindow.
// The swizzled implementations check objc_getAssociatedObject on `self` — only the
// specific overlay NSWindow instance is tagged, so pool windows (same class) are
// unaffected and continue returning their real values.
#[cfg(target_os = "macos")]
static ORIG_IS_MAIN_WINDOW: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

#[cfg(target_os = "macos")]
static ORIG_IS_KEY_WINDOW: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

// Stores the main browser's CefBrowserHost so swizzled_nsapp_send_event can call
// set_focus(1) right before dispatching a click, ensuring Blink isn't in a
// defocused state that drops events.
#[cfg(target_os = "macos")]
static MAIN_BROWSER_HOST_FOR_FOCUS: std::sync::Mutex<Option<cef::BrowserHost>> =
    std::sync::Mutex::new(None);

// Unique static address used as the objc_setAssociatedObject key for the overlay tag.
#[cfg(target_os = "macos")]
static PANE_OVERLAY_TAG_KEY: u8 = 0;

// shouldIgnoreMouseEvent: swizzle storage. Original returns YES when the RWHVC's
// window is not key/main, silently dropping mouseDown:. We override to always NO.
#[cfg(target_os = "macos")]
static ORIG_RWHVC_SHOULD_IGNORE: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

// Non-zero while a pane overlay is open. Used by swizzled_nsapp_send_event as the
// gate: if 0 the swizzle is inactive and all events fall through to the original.
#[cfg(target_os = "macos")]
static PANE_LOCAL_W: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

/// Swizzled NSWindow::isMainWindow — returns YES only for the tagged pane overlay.
/// All other NativeWidgetMacNSWindow instances (pool windows, etc.) call through
/// to the original implementation and return their real value.
#[cfg(target_os = "macos")]
extern "C" fn swizzled_is_main_window(
    this: *mut std::ffi::c_void,
    cmd: *const std::ffi::c_void,
) -> u8 {
    extern "C" {
        fn objc_getAssociatedObject(
            obj: *mut std::ffi::c_void,
            key: *const std::ffi::c_void,
        ) -> *mut std::ffi::c_void;
    }
    let key = &PANE_OVERLAY_TAG_KEY as *const u8 as *const std::ffi::c_void;
    let tag = unsafe { objc_getAssociatedObject(this, key) };
    if !tag.is_null() { return 1; }
    let orig = ORIG_IS_MAIN_WINDOW.load(std::sync::atomic::Ordering::SeqCst);
    if orig != 0 {
        let f: extern "C" fn(*mut std::ffi::c_void, *const std::ffi::c_void) -> u8 =
            unsafe { std::mem::transmute(orig) };
        return f(this, cmd);
    }
    0
}

/// Swizzled NSWindow::isKeyWindow — same instance-aware pattern as isMainWindow.
#[cfg(target_os = "macos")]
extern "C" fn swizzled_is_key_window(
    this: *mut std::ffi::c_void,
    cmd: *const std::ffi::c_void,
) -> u8 {
    extern "C" {
        fn objc_getAssociatedObject(
            obj: *mut std::ffi::c_void,
            key: *const std::ffi::c_void,
        ) -> *mut std::ffi::c_void;
    }
    let key = &PANE_OVERLAY_TAG_KEY as *const u8 as *const std::ffi::c_void;
    let tag = unsafe { objc_getAssociatedObject(this, key) };
    if !tag.is_null() { return 1; }
    let orig = ORIG_IS_KEY_WINDOW.load(std::sync::atomic::Ordering::SeqCst);
    if orig != 0 {
        let f: extern "C" fn(*mut std::ffi::c_void, *const std::ffi::c_void) -> u8 =
            unsafe { std::mem::transmute(orig) };
        return f(this, cmd);
    }
    0
}

/// Swizzled `RenderWidgetHostViewCocoa::shouldIgnoreMouseEvent:` — always returns NO.
///
/// Chromium's implementation returns YES when `[self.window isMainWindow]` and
/// `[self.window isKeyWindow]` are both NO, causing mouseDown: to silently drop
/// the event. The main CefNSWindow is neither key nor main while the overlay holds
/// focus. Returning NO unconditionally lets the main RWHVC forward clicks to the
/// main browser renderer even while the overlay is frontmost.
#[cfg(target_os = "macos")]
extern "C" fn swizzled_should_ignore_mouse_event(
    _this: *mut std::ffi::c_void,
    _cmd: *const std::ffi::c_void,
    _event: *mut std::ffi::c_void,
) -> u8 {
    0
}

// Storage for NSApp::sendEvent: original IMP — diagnostic only.
#[cfg(target_os = "macos")]
static ORIG_NSAPP_SEND_EVENT: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

// Cached NSWindow* and RenderWidgetHostViewCocoa* for the main browser window.
// Discovered at leftMouseDown time, reused for drag/up/right-click events so we
// don't re-walk the subview tree on every event. Cleared by clear_pane_swizzle_statics.
// Raw pointer: safe because RWHVC lives for the browser lifetime.
#[cfg(target_os = "macos")]
static MAIN_WIN_PTR:  std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(target_os = "macos")]
static MAIN_RWHVC_PTR: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Called by `detach_browser_pane_view` when a pane closes.
/// Resets all statics that gate the sendEvent: swizzle so it stops intercepting
/// main-window mouse events after the pane is gone.
#[cfg(target_os = "macos")]
pub(crate) fn clear_pane_swizzle_statics() {
    use std::sync::atomic::Ordering::Relaxed;
    PANE_LOCAL_W.store(0, Relaxed);
    MAIN_WIN_PTR.store(0, Relaxed);
    MAIN_RWHVC_PTR.store(0, Relaxed);
    if let Ok(mut guard) = MAIN_BROWSER_HOST_FOR_FOCUS.try_lock() {
        *guard = None;
    }
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn clear_pane_swizzle_statics() {}

/// Swizzled NSApplication::sendEvent: — for mouse events on the main window
/// while the browser pane is open, bypasses NSWindow.sendEvent: and dispatches
/// directly to the main RenderWidgetHostViewCocoa after restoring CEF focus.
///
/// Why direct dispatch:
///   When the pane overlay is frontmost the main window loses key status.
///   NSWindow.sendEvent: has "activate-only" semantics for non-key windows:
///   it activates the window but may not forward the event to the hit view.
///   Directly calling [rwhvc mouseDown:/rightMouseDown:/etc. event] bypasses
///   this — confirmed working ([AMX-DIAG-MD] hasFocus=true fires for every
///   dispatched event).
///
///   set_focus(1) must be called before dispatch so Blink's
///   RenderWidgetHostImpl does not drop the event (CEF calls set_focus(0) on
///   the main browser whenever the overlay gains focus).
///
/// Event types handled via direct dispatch (main window only):
///   1=leftMouseDown, 2=leftMouseUp, 3=rightMouseDown, 4=rightMouseUp,
///   6=leftMouseDragged, 7=rightMouseDragged, 22=scrollWheel
///
///   Drag events (6/7) must be intercepted because NSApp never saw the initial
///   leftMouseDown (we returned early), so its drag-tracking state is absent
///   and subsequent drag events would not be routed to the main RWHVC.
///
///   rightMouseDown/rightMouseUp (3/4) are needed for context menus: without
///   direct dispatch + set_focus(1) Blink silently drops them.
///
/// Overlay clicks (NativeWidgetMacNSWindow, pane area) are forwarded via the
/// original NSApp sendEvent: path so the pane browser handles them normally.
#[cfg(target_os = "macos")]
extern "C" fn swizzled_nsapp_send_event(
    this: *mut std::ffi::c_void,
    cmd: *const std::ffi::c_void,
    event: *mut std::ffi::c_void,
) {
    use std::ffi::c_void;
    extern "C" { fn objc_msgSend(); fn sel_registerName(n: *const i8) -> *const c_void; }
    extern "C" { fn object_getClass(obj: *mut c_void) -> *mut c_void; fn object_getClassName(cls: *mut c_void) -> *const i8; }
    type Id  = *mut c_void;
    type Sel = *const c_void;

    unsafe {
        let get_usize:  extern "C" fn(Id, Sel) -> usize    = std::mem::transmute(objc_msgSend as *const c_void);
        let get_id:     extern "C" fn(Id, Sel) -> Id        = std::mem::transmute(objc_msgSend as *const c_void);
        let get_obj_at: extern "C" fn(Id, Sel, usize) -> Id = std::mem::transmute(objc_msgSend as *const c_void);
        #[repr(C)] #[derive(Copy,Clone)] struct NSPoint { x: f64, y: f64 }
        let get_pt: extern "C" fn(Id, Sel) -> NSPoint       = std::mem::transmute(objc_msgSend as *const c_void);

        let sel_type = sel_registerName(b"type\0".as_ptr() as _);
        let ev_type  = get_usize(event, sel_type);

        // Event types to intercept on the main window.
        // 1=leftMouseDown  2=leftMouseUp    3=rightMouseDown  4=rightMouseUp
        // 6=leftMouseDragged  7=rightMouseDragged  22=scrollWheel
        let is_interceptable = matches!(ev_type, 1|2|3|4|6|7|22);

        if is_interceptable && PANE_LOCAL_W.load(std::sync::atomic::Ordering::Relaxed) > 0 {
            let sel_win = sel_registerName(b"window\0".as_ptr() as _);
            let sel_loc = sel_registerName(b"locationInWindow\0".as_ptr() as _);
            let sel_wn  = sel_registerName(b"windowNumber\0".as_ptr() as _);
            let win = get_id(event, sel_win);
            let loc = get_pt(event, sel_loc);

            if ev_type == 1 {
                let wnum = get_usize(event, sel_wn) as i64;
                tracing::debug!(
                    wnum, win = win as usize, loc_x = loc.x, loc_y = loc.y,
                    "[nsapp-diag] leftMouseDown → wnum={} win={:#x} loc=({:.1},{:.1})",
                    wnum, win as usize, loc.x, loc.y
                );
            }

            if !win.is_null() {
                // Skip overlay (NativeWidgetMacNSWindow = pane window).
                // Let original NSApp sendEvent: deliver pane-area events to the
                // pane browser as normal.
                let win_cls  = object_getClass(win);
                let name_ptr = object_getClassName(win_cls);
                let is_overlay = !name_ptr.is_null() &&
                    std::ffi::CStr::from_ptr(name_ptr).to_str().unwrap_or("").contains("NativeWidgetMacNSWindow");

                if !is_overlay {
                    // Only intercept events for the window that owns the pane.
                    // On the first leftMouseDown MAIN_WIN_PTR is 0 (unknown); we
                    // learn it during the subview walk and store it. Once set,
                    // events for any other window (which has no pane and its own
                    // focus semantics) fall through to the original sendEvent:.
                    let known_pane_win = MAIN_WIN_PTR.load(std::sync::atomic::Ordering::Relaxed);
                    if known_pane_win != 0 && win as usize != known_pane_win {
                        // Different window — pass through without touching focus.
                        let orig = ORIG_NSAPP_SEND_EVENT.load(std::sync::atomic::Ordering::SeqCst);
                        if orig != 0 {
                            let f: extern "C" fn(*mut c_void, *const c_void, *mut c_void) =
                                std::mem::transmute(orig);
                            f(this, cmd, event);
                        }
                        return;
                    }

                    // --- Main window event (window that owns the pane) ---
                    // Find the RWHVC to dispatch to. For leftMouseDown we walk
                    // the subview tree fresh (window may have been recreated).
                    // For all other types we reuse the cached pointer as long as
                    // the window pointer matches, to avoid walking on every drag.
                    let cached_win  = MAIN_WIN_PTR.load(std::sync::atomic::Ordering::Relaxed);
                    let cached_rwhvc = MAIN_RWHVC_PTR.load(std::sync::atomic::Ordering::Relaxed);

                    let rwhvc: Id = if ev_type == 1 || cached_win != win as usize || cached_rwhvc == 0 {
                        // Walk tree.
                        let sel_cv     = sel_registerName(b"contentView\0".as_ptr() as _);
                        let sel_subs   = sel_registerName(b"subviews\0".as_ptr() as _);
                        let sel_count  = sel_registerName(b"count\0".as_ptr() as _);
                        let sel_obj_at = sel_registerName(b"objectAtIndex:\0".as_ptr() as _);

                        let cv = get_id(win, sel_cv);
                        let mut found: Id = std::ptr::null_mut();
                        let mut stack: Vec<Id> = if cv.is_null() { vec![] } else { vec![cv] };
                        'walk: while let Some(view) = stack.pop() {
                            let vcls = object_getClass(view);
                            let vname_ptr = object_getClassName(vcls);
                            if !vname_ptr.is_null() {
                                let vname = std::ffi::CStr::from_ptr(vname_ptr).to_str().unwrap_or("");
                                if vname.contains("RenderWidgetHostViewCocoa") {
                                    found = view;
                                    break 'walk;
                                }
                            }
                            let subs = get_id(view, sel_subs);
                            if !subs.is_null() {
                                let n = get_usize(subs, sel_count);
                                for i in 0..n { stack.push(get_obj_at(subs, sel_obj_at, i)); }
                            }
                        }
                        if !found.is_null() {
                            MAIN_WIN_PTR.store(win as usize, std::sync::atomic::Ordering::Relaxed);
                            MAIN_RWHVC_PTR.store(found as usize, std::sync::atomic::Ordering::Relaxed);
                        }
                        found
                    } else {
                        cached_rwhvc as Id
                    };

                    if !rwhvc.is_null() {
                        // Restore main browser focus so Blink doesn't drop the
                        // event (CEF calls set_focus(0) on the main browser
                        // whenever the overlay gains focus).
                        if let Ok(mut guard) = crate::ui_tasks::MAIN_BROWSER_HOST_FOR_FOCUS.try_lock() {
                            if let Some(ref mut h) = *guard {
                                h.set_focus(1);
                            }
                        }
                        let method_name: &[u8] = match ev_type {
                            1  => b"mouseDown:\0",
                            2  => b"mouseUp:\0",
                            3  => b"rightMouseDown:\0",
                            4  => b"rightMouseUp:\0",
                            6  => b"mouseDragged:\0",
                            7  => b"rightMouseDragged:\0",
                            22 => b"scrollWheel:\0",
                            _  => b"mouseDown:\0",
                        };
                        let sel_m = sel_registerName(method_name.as_ptr() as _);
                        let dispatch_fn: extern "C" fn(Id, Sel, Id) =
                            std::mem::transmute(objc_msgSend as *const c_void);
                        if ev_type == 1 || ev_type == 3 {
                            tracing::debug!(
                                ev_type, loc_x = loc.x, loc_y = loc.y,
                                target = rwhvc as usize,
                                "[browser-pane] direct dispatch: main-win→main RWHVC ev={}",
                                ev_type
                            );
                        }
                        dispatch_fn(rwhvc, sel_m, event);
                        return;
                    }
                }
            }
        }
    }

    let orig = ORIG_NSAPP_SEND_EVENT.load(std::sync::atomic::Ordering::SeqCst);
    if orig != 0 {
        let f: extern "C" fn(*mut c_void, *const c_void, *mut c_void) =
            unsafe { std::mem::transmute(orig) };
        f(this, cmd, event);
    }
}

/// Get the CEF Views Window for a browser label on the UI thread.
fn get_window_on_ui(state: &Arc<AppState>, label: &str) -> Option<Window> {
    // Phase H.2.b — reducer-aware lookup with fallback.
    let mut browser = state.get_browser(label)?;
    let browser_view = browser_view_get_for_browser(Some(&mut browser))?;
    browser_view.window()
}

/// Windows-only: drive the CEF Views `Window` set_bounds() + show() for a
/// promoted pool window — the same path the macOS/Linux promote uses
/// (`PromotePoolWindowTask`). The Windows promote positions the raw HWND via
/// Win32 and never touched the Views `Window`, so the browser's view-hierarchy /
/// compositor visibility never flipped from hidden -> the promoted window painted
/// BLANK despite a valid DOM. This is the macOS-vs-Windows asymmetry. Bounds are
/// DIP (CEF Views space); the Win32 caller converts physical -> DIP via
/// `app::dpi_scale_at`. Must run on the UI thread.
/// See docs/research/RESEARCH_CEF_PREWARM_WINDOW_BLANK_ON_WINDOWS_2026_06_21.md.
// Windows-only: run the macOS-parity CEF Views show() on the UI thread. The
// Windows promote runs on the IPC thread, but CEF Views calls are UI-thread-only,
// so the set_bounds()+show() must be posted here (mirroring the macOS
// PromotePoolWindowTask). The Window was cached at on_window_created because
// browser_view.window() returns None for pool windows post-load on Windows.
#[cfg(target_os = "windows")]
wrap_task! {
    pub struct PromotePoolWindowViewsShowTask {
        state: Arc<AppState>,
        label: String,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    }

    impl Task {
        fn execute(&self) {
            use cef::ImplWindow;
            match crate::commands::window_pool::take_pool_window_view(&self.label) {
                Some(window) => {
                    // set_bounds is DIP (the Win32 promote already positioned the
                    // HWND in physical px; this syncs the Views Window so show()
                    // doesn't jump and performs the real hidden->visible transition).
                    window.set_bounds(Some(&cef::Rect {
                        x: self.x,
                        y: self.y,
                        width: self.width,
                        height: self.height,
                    }));
                    window.show();
                    // Belt-and-suspenders compositor nudge. NOTE: CefBrowserHost
                    // ::WasResized is only load-bearing in windowless/OSR mode; in
                    // CEF windowed mode (our case) it is effectively a no-op. The
                    // ACTUAL fix is the CEF Views window.show() above (the genuine
                    // hidden->visible transition). Kept as a cheap hint in case the
                    // host ever runs OSR; do not rely on it. (plan doc §6.)
                    if let Some(host) =
                        self.state.get_browser(&self.label).and_then(|b| b.host())
                    {
                        host.was_resized();
                    }
                    tracing::info!(
                        target: "pool:new-window",
                        label = %self.label,
                        x = self.x, y = self.y, width = self.width, height = self.height,
                        "[pool] CEF Views set_bounds + show on cached Window (macOS-parity, UI thread)"
                    );
                }
                None => {
                    tracing::warn!(
                        target: "pool:new-window",
                        label = %self.label,
                        "[pool] no cached CEF Views Window at promote show task — fix not applied"
                    );
                }
            }
        }
    }
}

#[cfg(target_os = "windows")]
pub(crate) fn post_promote_pool_window_views_show(
    state: &Arc<AppState>,
    label: &str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) {
    let mut task = PromotePoolWindowViewsShowTask::new(
        state.clone(),
        label.to_string(),
        x,
        y,
        width,
        height,
    );
    post_task(ThreadId::UI, Some(&mut task));
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
            use cef::ImplWindow;
            // CEF Views: close the WINDOW (CefWindow::close), which routes through
            // WindowDelegate::can_close (app.rs) → try_close_browser → on_before_close
            // → host quit cascade. Calling try_close_browser DIRECTLY on a
            // Views-hosted browser tears the Window down WITHOUT firing
            // on_before_close, so the browser is never unregistered and the host
            // never quits — the orphaned-tree regression (Discussion #1680).
            //
            // The historical reason this used try_close_browser — window.close()'s
            // Widget::Close CHECKs !on_call_stack_ and aborts if the widget is
            // already being destroyed (e.g. macOS windowShouldClose racing this
            // queued IPC task) — is handled by the is_closed() guard below.
            if let Some(window) = get_window_on_ui(&self.state, &self.label) {
                if window.is_closed() == 0 {
                    window.close();
                }
                return;
            }
            // Fallback: no CefWindow for this label (non-Views path / pre-init
            // teardown) — close the browser handle directly.
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

// ── Memory-pressure → frontend banner event ────────────────────────────────

wrap_task! {
    pub struct EmitMemoryPressureTask {
        state: Arc<AppState>,
        level: String,
        commit_free_mb: u64,
    }

    impl Task {
        fn execute(&self) {
            let payload = serde_json::json!({
                "level": self.level,
                "commit_free_mb": self.commit_free_mb,
            });
            crate::events::emit_event_to_top_level_windows(
                &self.state,
                "memory-pressure",
                &payload,
            );
        }
    }
}

/// Push a memory-pressure level transition to the frontend banner. Callable
/// from ANY thread (the memory heartbeat runs on a background std::thread); the
/// emit itself (CEF JS execution) must run on the UI thread, so it's wrapped in
/// a posted task. SPEC_MEMORY_PRESSURE_SUPERVISION_2026_06_16 §5.F.
pub fn post_memory_pressure(state: &Arc<AppState>, level: &str, commit_free_mb: u64) {
    let mut task = EmitMemoryPressureTask::new(state.clone(), level.to_string(), commit_free_mb);
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

// ── Pool window promote (macOS / Linux) — Phase 7 ─────────────────────────
//
// Moves a pre-warmed pool window from its off-screen holding position
// (-32000, -32000) to the tear-off destination and emits `pool:promote` so
// the renderer attaches the new workspace. Windows uses its own promote path
// (promote_pool_window cfg(windows)) with Win32 HWND + SetWindowPos + taskbar
// show. Non-Windows uses CEF Views Window::set_bounds() which is the
// cross-platform equivalent and runs correctly on the UI thread on macOS and
// Linux.
#[cfg(not(target_os = "windows"))]
wrap_task! {
    pub struct PromotePoolWindowTask {
        state: Arc<AppState>,
        label: String,
        workspace_id: String,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    }

    impl Task {
        fn execute(&self) {
            let Some(window) = get_window_on_ui(&self.state, &self.label) else {
                tracing::warn!(
                    target: "dnd:tearoff:pool",
                    label = %self.label,
                    "[pool:promote] window not found on UI thread — pool window may have closed"
                );
                return;
            };

            // Pool windows were kept hidden (on_load_end skips show() for
            // window-pool-* labels to avoid focus steal on macOS/Linux). Set
            // the target bounds first so the window appears at the correct
            // position, then show(). The user just performed a drag-to-tear-off
            // so activation is expected and desired here.
            window.set_bounds(Some(&cef::Rect {
                x: self.x,
                y: self.y,
                width: self.width,
                height: self.height,
            }));
            window.show();

            tracing::info!(
                target: "dnd:tearoff:pool",
                label = %self.label,
                x = self.x,
                y = self.y,
                width = self.width,
                height = self.height,
                "[pool:promote] window repositioned + shown via set_bounds + show"
            );

            // Signal the renderer to attach the workspace. The frontend's
            // awaitPoolPromote() listener was installed at pool-spawn time;
            // mark_pool_window_renderer_ready gates queue insertion on it, so
            // the listener is guaranteed to be ready before this event fires.
            crate::events::emit_event_to_window(
                &self.state,
                &self.label,
                "pool:promote",
                &serde_json::json!({ "workspaceId": self.workspace_id }),
            );

            tracing::info!(
                target: "dnd:tearoff:pool",
                label = %self.label,
                workspace_id = %self.workspace_id,
                "[pool:promote] pool:promote event emitted — renderer will attach workspace"
            );
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn post_promote_pool_window(
    state: &Arc<AppState>,
    label: &str,
    workspace_id: &str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) {
    let mut task = PromotePoolWindowTask::new(
        state.clone(),
        label.to_string(),
        workspace_id.to_string(),
        x,
        y,
        width,
        height,
    );
    post_task(ThreadId::UI, Some(&mut task));
}

// ── Promote pool window for new-window (Cmd+N / File → New Window) ────────
//
// Identical mechanical flow to PromotePoolWindowTask (set_bounds + show) but
// emits `pool:new-window` instead of `pool:promote`, carrying no workspaceId.
// The frontend's awaitPoolPromote handles both events; on `pool:new-window` it
// omits workspaceId from the URL so initHostNewWindow creates a fresh workspace
// rather than reattaching an existing one.
#[cfg(not(target_os = "windows"))]
wrap_task! {
    pub struct PromotePoolWindowForNewWindowTask {
        state: Arc<AppState>,
        label: String,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        initial_view: Option<String>,
        initial_meta: Option<String>,
    }

    impl Task {
        fn execute(&self) {
            let Some(window) = get_window_on_ui(&self.state, &self.label) else {
                tracing::warn!(
                    target: "pool:new-window",
                    label = %self.label,
                    "[pool:new-window] window not found on UI thread — pool window may have closed"
                );
                return;
            };

            window.set_bounds(Some(&cef::Rect {
                x: self.x,
                y: self.y,
                width: self.width,
                height: self.height,
            }));
            window.show();

            tracing::info!(
                target: "pool:new-window",
                label = %self.label,
                x = self.x,
                y = self.y,
                width = self.width,
                height = self.height,
                "[pool:new-window] window repositioned + shown via set_bounds + show"
            );

            crate::events::emit_event_to_window(
                &self.state,
                &self.label,
                "pool:new-window",
                &serde_json::json!({
                    "initialView": self.initial_view,
                    "initialMeta": self.initial_meta,
                }),
            );

            tracing::info!(
                target: "pool:new-window",
                label = %self.label,
                "[pool:new-window] pool:new-window emitted — renderer will create fresh workspace"
            );
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn post_promote_pool_window_for_new_window(
    state: &Arc<AppState>,
    label: &str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    initial_view: Option<String>,
    initial_meta: Option<String>,
) {
    let mut task = PromotePoolWindowForNewWindowTask::new(
        state.clone(),
        label.to_string(),
        x,
        y,
        width,
        height,
        initial_view,
        initial_meta,
    );
    post_task(ThreadId::UI, Some(&mut task));
}

// ── Promote pane pool window (macOS / Linux) ──────────────────────────────
//
// Repositions a floating-pool-{uuid} frameless window from its off-screen
// holding position to the drop-target bounds and emits pool:pane-promote so
// the renderer mounts FloatingPaneWorkspace with the given paneId+workspaceId.
#[cfg(not(target_os = "windows"))]
wrap_task! {
    pub struct PromotePanePoolWindowTask {
        state: Arc<AppState>,
        label: String,
        pane_id: String,
        workspace_id: String,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    }

    impl Task {
        fn execute(&self) {
            let Some(window) = get_window_on_ui(&self.state, &self.label) else {
                tracing::warn!(
                    target: "pool:pane",
                    label = %self.label,
                    "[pane-pool] window not found on UI thread — pool window may have closed"
                );
                return;
            };

            window.set_bounds(Some(&cef::Rect {
                x: self.x,
                y: self.y,
                width: self.width,
                height: self.height,
            }));
            window.show();

            tracing::info!(
                target: "pool:pane",
                label = %self.label,
                x = self.x,
                y = self.y,
                width = self.width,
                height = self.height,
                "[pane-pool] window repositioned + shown"
            );

            crate::events::emit_event_to_window(
                &self.state,
                &self.label,
                "pool:pane-promote",
                &serde_json::json!({
                    "paneId": self.pane_id,
                    "workspaceId": self.workspace_id,
                }),
            );

            tracing::info!(
                target: "pool:pane",
                label = %self.label,
                pane_id = %self.pane_id,
                workspace_id = %self.workspace_id,
                "[pane-pool] pool:pane-promote emitted — renderer will mount FloatingPaneWorkspace"
            );
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn post_promote_pane_pool_window(
    state: &Arc<AppState>,
    label: &str,
    pane_id: &str,
    workspace_id: &str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) {
    let mut task = PromotePanePoolWindowTask::new(
        state.clone(),
        label.to_string(),
        pane_id.to_string(),
        workspace_id.to_string(),
        x,
        y,
        width,
        height,
    );
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
                } else if label.starts_with("floating-") {
                    // Floating panes are never valid redock targets — skip
                    // them so a dragged pane hovering over a stacked floater
                    // doesn't ghost the idle floater instead of main.
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
            // An empty label means "reclaim the foreground agentmux window" —
            // used by the pane-destroy focus handoff, which can't know the
            // surviving window's label up front (redock vs. in-window close).
            let label: String = if !self.label.is_empty() {
                self.label.clone()
            } else {
                #[cfg(target_os = "windows")]
                {
                    use windows_sys::Win32::UI::WindowsAndMessaging::GetForegroundWindow;
                    let fg = unsafe { GetForegroundWindow() } as isize;
                    let resolved: Option<String> = if fg != 0 {
                        let map = self.state.window_hwnds.lock();
                        map.iter()
                            .find_map(|(k, &h)| if h == fg { Some(k.clone()) } else { None })
                    } else {
                        None
                    };
                    resolved.unwrap_or_else(|| "main".to_string())
                }
                #[cfg(not(target_os = "windows"))]
                {
                    "main".to_string()
                }
            };

            // Phase H.2.b — reducer-aware lookup with fallback.
            let mut browser = match self.state.get_browser(&label) {
                Some(b) => b,
                None => {
                    tracing::warn!("[main-focus-reclaim] no browser for label={}", label);
                    return;
                }
            };

            if let Some(host) = browser.host() {
                host.set_focus(1);
                tracing::info!("[main-focus-reclaim] host.set_focus(1) on label={}", label);
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
                            label,
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

// ── Mother-window resize after pane tear-off ──────────────────────────────
//
// When a full-height pane is torn off (top-to-bottom column), the mother
// window shrinks by the pane's column width so remaining panes keep their
// absolute pixel sizes.
//
// Spec: docs/specs/SPEC_PANE_TEAROFF_MOTHER_RESIZE_2026_06_20.md

/// Resize the mother window to `new_w_dip` on macOS/Linux via CEF Views
/// `set_bounds`. Width is in CSS/DIP pixels (same coordinate space as the
/// floater args); height is read from the current bounds and preserved.
#[cfg(not(target_os = "windows"))]
wrap_task! {
    pub struct ResizeMotherWindowTask {
        state: Arc<AppState>,
        label: String,
        new_w_dip: i32,
    }

    impl Task {
        fn execute(&self) {
            let Some(window) = get_window_on_ui(&self.state, &self.label) else {
                tracing::warn!(
                    label = %self.label,
                    "[tear-off] ResizeMotherWindowTask: source window not found (already closed?)"
                );
                return;
            };
            let old = window.bounds();
            window.set_bounds(Some(&cef::Rect {
                x: old.x,
                y: old.y,
                width: self.new_w_dip,
                height: old.height,
            }));
            tracing::info!(
                label = %self.label,
                old_w = old.width,
                new_w = self.new_w_dip,
                "[tear-off] mother window resized after pane tear-off"
            );
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn post_resize_mother_window(state: &Arc<AppState>, label: &str, new_w_dip: i32) {
    let mut task = ResizeMotherWindowTask::new(state.clone(), label.to_string(), new_w_dip);
    post_task(ThreadId::UI, Some(&mut task));
}

/// Resize the mother window to `new_w_dip` on Windows via Win32 `SetWindowPos`.
/// `new_w_dip` is in CSS/DIP pixels; this function converts to physical pixels
/// using the source window's monitor DPI before calling `SetWindowPos`.
/// `hwnd` is resolved directly from `source_window_label` in
/// `open_floating_pane_window` (not the cascade-hook fallback `parent_main_hwnd`)
/// so the resize always targets the actual source window.
#[cfg(target_os = "windows")]
wrap_task! {
    pub struct ResizeMotherWindowWin32Task {
        state: Arc<AppState>,
        hwnd: isize,
        new_w_dip: i32,
    }

    impl Task {
        fn execute(&self) {
            use windows_sys::Win32::Foundation::POINT;
            use windows_sys::Win32::Graphics::Gdi::{MonitorFromPoint, MONITOR_DEFAULTTONEAREST};
            use windows_sys::Win32::UI::HiDpi::{GetDpiForMonitor, MDT_EFFECTIVE_DPI};
            use windows_sys::Win32::UI::WindowsAndMessaging::{
                GetWindowRect, SetWindowPos, SWP_NOMOVE, SWP_NOACTIVATE, SWP_NOZORDER,
            };

            unsafe {
                let hwnd = self.hwnd as windows_sys::Win32::Foundation::HWND;
                let mut wr = std::mem::zeroed::<windows_sys::Win32::Foundation::RECT>();
                GetWindowRect(hwnd, &mut wr);
                let current_h = wr.bottom - wr.top;
                let pt = POINT { x: wr.left, y: wr.top };
                let monitor = MonitorFromPoint(pt, MONITOR_DEFAULTTONEAREST);
                let mut dpi_x: u32 = 0;
                let mut dpi_y: u32 = 0;
                let hr = GetDpiForMonitor(monitor, MDT_EFFECTIVE_DPI, &mut dpi_x, &mut dpi_y);
                let dpi_scale = if hr != 0 || dpi_x == 0 { 1.0f32 } else { dpi_x as f32 / 96.0 };
                let new_w_px = (self.new_w_dip as f32 * dpi_scale).round() as i32;
                let ok = SetWindowPos(
                    hwnd,
                    std::ptr::null_mut(),
                    0, 0,
                    new_w_px,
                    current_h,
                    SWP_NOMOVE | SWP_NOZORDER | SWP_NOACTIVATE,
                );
                tracing::info!(
                    hwnd = self.hwnd,
                    new_w_dip = self.new_w_dip,
                    new_w_px,
                    dpi_scale,
                    ok = (ok != 0),
                    "[tear-off] mother window resized after pane tear-off (Win32)"
                );
            }
        }
    }
}

#[cfg(target_os = "windows")]
pub fn post_resize_mother_window_win32(state: &Arc<AppState>, hwnd: isize, new_w_dip: i32) {
    let mut task = ResizeMotherWindowWin32Task::new(state.clone(), hwnd, new_w_dip);
    post_task(ThreadId::UI, Some(&mut task));
}

// ── Deferred overlay bounds + show (non-Windows / CEF Views panes) ──────────
//
// On macOS, add_overlay_view creates the overlay's native NSView asynchronously.
// set_size / set_position / layout() called synchronously after add_overlay_view
// silently no-op because the native layer doesn't exist yet — the readback shows
// oc_w=0, oc_h=0. Without committed bounds the overlay defaults to filling the
// entire parent window, covering the UI with the pane's opaque-black background
// and intercepting all mouse events (the "black screen + UI freeze" bug).
//
// This task runs on the next UI event-loop tick, after CEF has completed native
// view creation, and re-applies the desired bounds before making the overlay
// visible. It also re-issues set_focus(1) on the main browser to handle any
// focus steal that may have occurred during the intervening tick.
//
// On macOS, CefOverlayController::SetSize/SetPosition are permanent no-ops —
// the underlying NativeWidgetMacNSWindow (a CEF-internal popup NSWindow) is
// never resized via the CEF Views API. We detect and size it directly through
// Objective-C: enumerate [NSApplication sharedApplication].windows, identify
// the NativeWidgetMacNSWindow overlay (the last non-key non-main NSWindow),
// and call [overlayWindow setFrame:display:YES] with correct screen coords.
// Pane coords from the frontend are physical pixels; NSWindow setFrame: uses
// points, so we divide by [mainWindow backingScaleFactor] (2.0 on Retina).
// set_visible(1) is called ONLY after the frame is committed.
#[cfg(not(target_os = "windows"))]
wrap_task! {
    pub struct SetPaneBoundsViewsTask {
        state: Arc<AppState>,
        label: String,
        window_label: String,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
        retry: u32,
        // macOS: windowNumber of the overlay NativeWidgetMacNSWindow, captured
        // before set_visible(0) removed it from [NSApp windows]. 0 = unknown.
        overlay_wnum: isize,
    }

    impl Task {
        fn execute(&self) {
            // Guard: if the pane was closed before this task fired, bail.
            // The UI-thread FIFO ordering guarantees detach always runs before
            // this deferred task (close posts its own UI task which precedes ours
            // in the queue when initiated before this task was scheduled).
            let entry = self.state.browser_pane_overlays.lock().get(&self.label).cloned();
            let Some((_, controller)) = entry else {
                tracing::warn!(
                    label = %self.label,
                    "[browser-pane] views: SetPaneBoundsViewsTask: no OverlayController — pane already closed?"
                );
                return;
            };

            // CEF Views sizing — no-ops on macOS, functional on Linux.
            controller.set_size(Some(&Size { width: self.width, height: self.height }));
            controller.set_position(Some(&Point { x: self.x, y: self.y }));
            // Skip window.layout() on macOS: it schedules a deferred CEF Views layout
            // pass that calls NativeWidgetMac::SetBoundsRect(0,0,0,0) AFTER our ObjC
            // setFrame, resetting the overlay to off-screen and re-engaging its event
            // capture. On macOS, we rely entirely on the ObjC NSWindow resize below.
            #[cfg(not(target_os = "macos"))]
            if let Some(window) = self.state.windows.lock().get(&self.window_label).cloned() {
                window.layout();
            }

            let b = controller.bounds();
            tracing::info!(
                label = %self.label,
                req_w = self.width, req_h = self.height,
                got_x = b.x, got_y = b.y, got_w = b.width, got_h = b.height,
                retry = self.retry,
                "[browser-pane] views: SetPaneBoundsViewsTask: CEF bounds (macos)"
            );

            // Linux: CEF Views sizing works; show immediately.
            #[cfg(not(target_os = "macos"))]
            {
                controller.set_visible(1);
                if let Some(main_browser) = self.state.get_browser(&self.window_label) {
                    if let Some(mut host) = main_browser.host() { host.set_focus(1); }
                }
                return;
            }

            // macOS: [NSWindow windowWithWindowNumber:] was removed in macOS 26 Tahoe.
            // After the ObjC block, we also call controller.set_bounds() with DIP
            // coordinates. The root cause of the 0×0 reset is overlay_view_host.cc
            // SetOverlayBounds() doing Intersect(window_view_->bounds()). At creation
            // time the parent CefWindowView bounds may be empty, causing the intersection
            // to produce 0×0. From this deferred task the parent window IS fully laid out,
            // so set_bounds() should succeed and commit the bounds in CEF's Views layer.
            // The ObjC block additionally sets the NSWindow frame directly.
            #[cfg(target_os = "macos")]
            let (mut task_dip_x, mut task_dip_y, mut task_dip_w, mut task_dip_h) = (0i32, 0i32, 0i32, 0i32);
            #[cfg(target_os = "macos")]
            let mut task_main_win:    *mut std::ffi::c_void = std::ptr::null_mut();
            #[cfg(target_os = "macos")]
            let mut task_overlay_win: *mut std::ffi::c_void = std::ptr::null_mut();
            #[cfg(target_os = "macos")]
            let mut task_sel_make_key_front: *const std::ffi::c_void = std::ptr::null();
            #[cfg(target_os = "macos")]
            let mut task_sel_order_front:    *const std::ffi::c_void = std::ptr::null();
            #[cfg(target_os = "macos")]
            let mut task_sel_key_window:     *const std::ffi::c_void = std::ptr::null();
            // Main window height in Cocoa points — used to convert DIP y-from-top to
            // Cocoa y-from-bottom for the hitTest: bounds stored in PANE_LOCAL_Y_BOTTOM.
            #[cfg(target_os = "macos")]
            let mut task_main_h: i32 = 0;

            #[cfg(target_os = "macos")]
            unsafe {
                use std::ffi::c_char;
                type Id  = *mut std::ffi::c_void;
                type Sel = *const std::ffi::c_void;

                extern "C" {
                    fn sel_registerName(name: *const c_char) -> Sel;
                    fn objc_msgSend();
                    fn object_getClassName(obj: Id) -> *const c_char;
                    fn objc_getClass(name: *const c_char) -> Id;
                }

                #[repr(C)] #[derive(Copy,Clone)] struct NSPoint { x: f64, y: f64 }
                #[repr(C)] #[derive(Copy,Clone)] struct NSSize  { w: f64, h: f64 }
                #[repr(C)] #[derive(Copy,Clone)] struct NSRect  { origin: NSPoint, size: NSSize }

                let sel_shared_app    = sel_registerName(b"sharedApplication\0".as_ptr() as _);
                let sel_windows_arr   = sel_registerName(b"windows\0".as_ptr() as _);
                let sel_count         = sel_registerName(b"count\0".as_ptr() as _);
                let sel_obj_at        = sel_registerName(b"objectAtIndex:\0".as_ptr() as _);
                let sel_frame         = sel_registerName(b"frame\0".as_ptr() as _);
                let sel_is_main       = sel_registerName(b"isMainWindow\0".as_ptr() as _);
                let sel_is_key        = sel_registerName(b"isKeyWindow\0".as_ptr() as _);
                let sel_key_window    = sel_registerName(b"keyWindow\0".as_ptr() as _);
                let sel_backing_scale = sel_registerName(b"backingScaleFactor\0".as_ptr() as _);
                let sel_set_frame_d   = sel_registerName(b"setFrame:display:\0".as_ptr() as _);
                let sel_make_key_front = sel_registerName(b"makeKeyAndOrderFront:\0".as_ptr() as _);
                let sel_order_front    = sel_registerName(b"orderFront:\0".as_ptr() as _);
                let sel_win_number     = sel_registerName(b"windowNumber\0".as_ptr() as _);

                let get_id:         extern "C" fn(Id, Sel) -> Id        = std::mem::transmute(objc_msgSend as *const std::ffi::c_void);
                let get_usize:      extern "C" fn(Id, Sel) -> usize     = std::mem::transmute(objc_msgSend as *const std::ffi::c_void);
                let get_isize:      extern "C" fn(Id, Sel) -> isize     = std::mem::transmute(objc_msgSend as *const std::ffi::c_void);
                let get_bool:       extern "C" fn(Id, Sel) -> u8        = std::mem::transmute(objc_msgSend as *const std::ffi::c_void);
                let get_f64:        extern "C" fn(Id, Sel) -> f64       = std::mem::transmute(objc_msgSend as *const std::ffi::c_void);
                let obj_at:         extern "C" fn(Id, Sel, usize) -> Id = std::mem::transmute(objc_msgSend as *const std::ffi::c_void);
                let get_frame:      extern "C" fn(Id, Sel) -> NSRect    = std::mem::transmute(objc_msgSend as *const std::ffi::c_void);
                let set_frame_d:    extern "C" fn(Id, Sel, NSRect, u8)  = std::mem::transmute(objc_msgSend as *const std::ffi::c_void);
                let make_key_front: extern "C" fn(Id, Sel, Id)          = std::mem::transmute(objc_msgSend as *const std::ffi::c_void);
                let order_front:    extern "C" fn(Id, Sel, Id)          = std::mem::transmute(objc_msgSend as *const std::ffi::c_void);

                // Step 1: show the overlay on the first run only.
                // On retry >= 1, the overlay is already visible; we just reaffirm the
                // frame AFTER CEF's deferred Widget layout (triggered by Widget::Show()
                // during retry=0) has had a chance to run and reset the NSWindow bounds
                // to 0×0. The delayed retry fires 50ms later, after all pending layout
                // passes have completed.
                if self.retry == 0 {
                    controller.set_visible(1);
                }

                // Step 2: rescan [NSApp windows] to find the overlay and main window.
                let ns_app_class: Id = objc_getClass(b"NSApplication\0".as_ptr() as _);
                let ns_app: Id       = get_id(ns_app_class, sel_shared_app);
                let all_wins: Id     = get_id(ns_app, sel_windows_arr);
                let win_count: usize = if all_wins.is_null() { 0 }
                                       else { get_usize(all_wins, sel_count) };

                let mut overlay_win: Id = std::ptr::null_mut();
                let mut main_win:    Id = std::ptr::null_mut();
                // Highest windowNumber among NativeWidgetMacNSWindows (for fallback
                // when overlay_wnum is unknown — newest window = highest number).
                let mut highest_native_wnum: isize = 0;
                let mut highest_native_win: Id = std::ptr::null_mut();

                for i in 0..win_count {
                    let win = obj_at(all_wins, sel_obj_at, i);
                    if win.is_null() { continue; }
                    let cls_ptr = object_getClassName(win);
                    let cls = if !cls_ptr.is_null() {
                        std::ffi::CStr::from_ptr(cls_ptr).to_str().unwrap_or("?")
                    } else { "?" };
                    let wn      = get_isize(win, sel_win_number);
                    let is_main = get_bool(win, sel_is_main);
                    let is_key  = get_bool(win, sel_is_key);
                    let fr      = get_frame(win, sel_frame);
                    tracing::info!(
                        i, win_count, class = cls, retry = self.retry,
                        x = fr.origin.x, y = fr.origin.y, w = fr.size.w, h = fr.size.h,
                        is_main, is_key, wn, want_wnum = self.overlay_wnum,
                        "[browser-pane] ObjC task NSApp window"
                    );
                    if cls.contains("CefNSWindow") {
                        // Priority 1: the window reporting isMainWindow=1 is definitive.
                        // Priority 2: any on-screen CefNSWindow (x > -1000) as fallback.
                        // Never use off-screen pool windows (x = -32000) as main_win —
                        // they cause makeKeyAndOrderFront: to fire on the wrong window.
                        // NOTE: the overlay is NativeWidgetMacNSWindow, never CefNSWindow,
                        // so is_main=1 will never be true for the overlay here.
                        let fr_tmp = get_frame(win, sel_frame);
                        if is_main != 0 {
                            main_win = win;
                        } else if main_win.is_null() && fr_tmp.origin.x > -1000.0 {
                            main_win = win;
                        }
                    }
                    if cls.contains("NativeWidgetMacNSWindow") {
                        if self.overlay_wnum > 0 && wn == self.overlay_wnum {
                            overlay_win = win;
                        }
                        if wn > highest_native_wnum {
                            highest_native_wnum = wn;
                            highest_native_win = win;
                        }
                    }
                }

                // Fallback: if wnum-based lookup missed (e.g. overlay_wnum==0 because
                // the pre-hide scan ran before the NSWindow existed), use the newest
                // NativeWidgetMacNSWindow (highest windowNumber = most recently created).
                if overlay_win.is_null() && !highest_native_win.is_null() {
                    overlay_win = highest_native_win;
                    tracing::info!(
                        label = %self.label, retry = self.retry,
                        overlay_wnum = self.overlay_wnum, highest_native_wnum,
                        "[browser-pane] ObjC task: falling back to highest-wnum NativeWidgetMacNSWindow"
                    );
                }

                if overlay_win.is_null() {
                    if self.retry < 5 {
                        tracing::info!(
                            label = %self.label, retry = self.retry,
                            "[browser-pane] ObjC task: no NativeWidgetMacNSWindow found after set_visible(1), retrying"
                        );
                        post_set_pane_bounds_views(
                            &self.state, &self.label, &self.window_label,
                            self.x, self.y, self.width, self.height,
                            self.retry + 1, self.overlay_wnum,
                        );
                    } else {
                        tracing::warn!(label = %self.label,
                            "[browser-pane] ObjC task: no NativeWidgetMacNSWindow found after 5 retries");
                    }
                    return;
                }

                // Step 3: reaffirm the frame (set_visible(1) may have triggered a CEF
                // layout pass that reset the native frame to the pre-creation default).
                let main_frame = if !main_win.is_null() {
                    get_frame(main_win, sel_frame)
                } else {
                    NSRect { origin: NSPoint { x: 0.0, y: 0.0 }, size: NSSize { w: 0.0, h: 0.0 } }
                };
                let scale = if !main_win.is_null() {
                    let s = get_f64(main_win, sel_backing_scale);
                    if s > 0.0 { s } else { 1.0 }
                } else { 1.0 };

                let pane_x = self.x as f64 / scale;
                let pane_y = self.y as f64 / scale;
                let pane_w = self.width  as f64 / scale;
                let pane_h = self.height as f64 / scale;
                let screen_x = main_frame.origin.x + pane_x;
                let screen_y = main_frame.origin.y + main_frame.size.h - pane_y - pane_h;

                let target = NSRect {
                    origin: NSPoint { x: screen_x, y: screen_y },
                    size:   NSSize  { w: pane_w, h: pane_h },
                };

                set_frame_d(overlay_win, sel_set_frame_d, target, 1u8);
                let new_fr = get_frame(overlay_win, sel_frame);

                // Diagnostic A: overlay window level (NSNormalWindowLevel=0,
                //   NSFloatingWindowLevel=3). If can_activate=0 produces a
                //   floating-level panel it sits above the main window at the
                //   OS level even outside its frame — understanding this is key.
                let sel_level = sel_registerName(b"level\0".as_ptr() as _);
                let get_level: extern "C" fn(Id, Sel) -> isize = std::mem::transmute(objc_msgSend as *const std::ffi::c_void);
                let ov_level  = get_level(overlay_win, sel_level);
                let main_level = if !main_win.is_null() { get_level(main_win, sel_level) } else { -1 };

                // Diagnostic B: which window is actually on top at the pane
                // center? [NSWindow windowNumberAtPoint:belowWindowWithWindowNumber:0]
                // returns the window number of the topmost window at that point.
                // If it matches the overlay → overlay is frontmost (clicks reach it).
                // If it matches the main window → main is frontmost (clicks DON'T reach overlay).
                let sel_wnum_at_pt = sel_registerName(b"windowNumberAtPoint:belowWindowWithWindowNumber:\0".as_ptr() as _);
                let wnum_at_fn: extern "C" fn(Id, Sel, NSPoint, isize) -> isize = std::mem::transmute(objc_msgSend as *const std::ffi::c_void);
                let pane_center_screen = NSPoint {
                    x: screen_x + pane_w / 2.0,
                    y: screen_y + pane_h / 2.0,
                };
                let win_cls_id: Id = objc_getClass(b"NSWindow\0".as_ptr() as _);
                let wnum_at_center = wnum_at_fn(win_cls_id, sel_wnum_at_pt, pane_center_screen, 0);
                let ov_wnum  = get_isize(overlay_win, sel_win_number);
                let main_wnum = if !main_win.is_null() { get_isize(main_win, sel_win_number) } else { -1 };
                let frontmost_is_overlay = wnum_at_center == ov_wnum;
                let frontmost_is_main    = wnum_at_center == main_wnum;

                tracing::info!(
                    label = %self.label, retry = self.retry, scale,
                    pane_x, pane_y, pane_w, pane_h, screen_x, screen_y,
                    main_x = main_frame.origin.x, main_y = main_frame.origin.y,
                    main_w = main_frame.size.w, main_h = main_frame.size.h,
                    req_w = self.width, req_h = self.height,
                    got_x = new_fr.origin.x, got_y = new_fr.origin.y,
                    got_w = new_fr.size.w, got_h = new_fr.size.h,
                    ov_level, main_level,
                    wnum_at_center, ov_wnum, main_wnum,
                    frontmost_is_overlay, frontmost_is_main,
                    "[browser-pane] ObjC task overlay setFrame + z-order diag"
                );
                // Export DIP coordinates for the set_bounds call after this block.
                task_dip_x = pane_x as i32;
                task_dip_y = pane_y as i32;
                task_dip_w = pane_w as i32;
                task_dip_h = pane_h as i32;
                // Main window height in Cocoa points (needed for Cocoa y-from-bottom
                // conversion in the hitTest: swizzle bounds check).
                task_main_h = main_frame.size.h as i32;
                // Export window refs for post-set_bounds key restoration.
                task_main_win    = main_win;
                task_overlay_win = overlay_win;
                task_sel_make_key_front = sel_make_key_front;
                task_sel_order_front    = sel_order_front;
                task_sel_key_window     = sel_key_window;
            }

            #[cfg(target_os = "macos")]
            if let Some(main_browser) = self.state.get_browser(&self.window_label) {
                if let Some(mut host) = main_browser.host() { host.set_focus(1); }
            }

            // macOS: call controller.set_bounds() with DIP coordinates computed above.
            // The root cause of the 0×0 reset is CEF's overlay_view_host.cc
            // SetOverlayBounds() doing Intersect(window_view_->bounds()). At creation
            // time the parent CefWindowView may have empty bounds, but by this deferred
            // task the parent window IS fully laid out so the Intersect should preserve
            // our desired rect. This commits the size in CEF's Views layer so subsequent
            // layout passes no longer reset the NSWindow to 0×0.
            #[cfg(target_os = "macos")]
            if task_dip_w > 0 && task_dip_h > 0 {
                use cef::Rect as CefRect;
                let dip = CefRect { x: task_dip_x, y: task_dip_y, width: task_dip_w, height: task_dip_h };
                controller.set_bounds(Some(&dip));
                let b = controller.bounds();
                tracing::info!(
                    label = %self.label, retry = self.retry,
                    dip_x = task_dip_x, dip_y = task_dip_y,
                    dip_w = task_dip_w, dip_h = task_dip_h,
                    readback_x = b.x, readback_y = b.y,
                    readback_w = b.width, readback_h = b.height,
                    "[browser-pane] controller.set_bounds (post-ObjC) and readback"
                );
            }

            // macOS: after set_bounds(), restore key focus to the main CefNSWindow
            // so sidebar clicks always work. Then bring the overlay to the front
            // so the pane is visually on top. With can_activate=0 the overlay is a
            // non-activating NSPanel — clicks reach its content view without being
            // consumed for window activation, so pane clicks should reach Chromium
            // even though the main window remains KEY.
            //
            // Belt-and-suspenders: also inject acceptsFirstMouse:YES into the overlay
            // contentView's class so that even if can_activate ever changes, the first
            // click in the pane is always processed as a content click, not an
            // activation click.
            #[cfg(target_os = "macos")]
            if !task_overlay_win.is_null() {
                unsafe {
                    use std::ffi::c_char;
                    type Id  = *mut std::ffi::c_void;
                    type Sel = *const std::ffi::c_void;
                    extern "C" {
                        fn objc_msgSend();
                        fn sel_registerName(name: *const c_char) -> Sel;
                        fn object_getClass(obj: Id) -> Id;
                        fn class_addMethod(cls: Id, sel: Sel, imp: *const std::ffi::c_void, types: *const c_char) -> u8;
                        fn object_getClassName(obj: Id) -> *const c_char;
                    }
                    let get_id:   extern "C" fn(Id, Sel) -> Id = std::mem::transmute(objc_msgSend as *const std::ffi::c_void);
                    let make_key: extern "C" fn(Id, Sel, Id)   = std::mem::transmute(objc_msgSend as *const std::ffi::c_void);
                    let order_fn: extern "C" fn(Id, Sel, Id)   = std::mem::transmute(objc_msgSend as *const std::ffi::c_void);

                    // Restore main window as key so sidebar clicks work.
                    if !task_main_win.is_null() && !task_sel_make_key_front.is_null() {
                        make_key(task_main_win, task_sel_make_key_front, std::ptr::null_mut());
                    }
                    // Keep overlay frontmost.
                    if !task_sel_order_front.is_null() {
                        order_fn(task_overlay_win, task_sel_order_front, std::ptr::null_mut());
                    }

                    // Swizzle NativeWidgetMacNSWindow::isMainWindow → always returns YES.
                    //
                    // RenderWidgetHostViewCocoa::mouseDown: has an early-exit guard:
                    //   if (!isMainWindow && !isKeyWindow) return;
                    // With can_activate=0 the overlay is neither main nor key (main
                    // window stays key/main so sidebar always works), so RWHVC drops
                    // every click. Making the overlay's class return YES for
                    // isMainWindow bypasses the guard without touching key-window state.
                    // The overlay window class is NativeWidgetMacNSWindow; the main
                    // window is NSKVONotifying_CefNSWindow (a different class), so this
                    // swizzle is scoped only to the overlay.
                    extern "C" {
                        fn class_getInstanceMethod(cls: Id, sel: Sel) -> *mut std::ffi::c_void;
                        fn method_setImplementation(method: *mut std::ffi::c_void, imp: *const std::ffi::c_void) -> *const std::ffi::c_void;
                    }
                    // Scope isMainWindow/isKeyWindow→YES to THIS overlay instance only.
                    //
                    // Strategy: swizzle the class once (class-level), but inside the
                    // swizzled fn check objc_getAssociatedObject(self, KEY). Only the
                    // overlay instance is tagged → only it gets YES. Pool windows (same
                    // class) call through to the original and return their real value.
                    // This avoids object_setClass (which breaks CEF rendering) and
                    // avoids dynamic subclasses (same problem).
                    extern "C" {
                        fn objc_setAssociatedObject(
                            obj: Id,
                            key: *const std::ffi::c_void,
                            value: Id,
                            policy: usize,
                        );
                    }
                    let tag_key = &crate::ui_tasks::PANE_OVERLAY_TAG_KEY as *const u8 as *const std::ffi::c_void;
                    // Associate overlay with itself — non-nil tag; ASSIGN=0 (no retain).
                    objc_setAssociatedObject(task_overlay_win, tag_key, task_overlay_win, 0);

                    static OVERLAY_SWIZZLE_DONE: std::sync::atomic::AtomicBool =
                        std::sync::atomic::AtomicBool::new(false);
                    if !OVERLAY_SWIZZLE_DONE.load(std::sync::atomic::Ordering::SeqCst) {
                        let win_cls = object_getClass(task_overlay_win);
                        if !win_cls.is_null() {
                            let sel_main = sel_registerName(b"isMainWindow\0".as_ptr() as _);
                            let m_main = class_getInstanceMethod(win_cls, sel_main);
                            if !m_main.is_null() {
                                let old = method_setImplementation(
                                    m_main,
                                    crate::ui_tasks::swizzled_is_main_window as *const std::ffi::c_void,
                                );
                                crate::ui_tasks::ORIG_IS_MAIN_WINDOW.store(
                                    old as usize,
                                    std::sync::atomic::Ordering::SeqCst,
                                );
                            }
                            let sel_key = sel_registerName(b"isKeyWindow\0".as_ptr() as _);
                            let m_key = class_getInstanceMethod(win_cls, sel_key);
                            if !m_key.is_null() {
                                let old = method_setImplementation(
                                    m_key,
                                    crate::ui_tasks::swizzled_is_key_window as *const std::ffi::c_void,
                                );
                                crate::ui_tasks::ORIG_IS_KEY_WINDOW.store(
                                    old as usize,
                                    std::sync::atomic::Ordering::SeqCst,
                                );
                            }
                            OVERLAY_SWIZZLE_DONE.store(true, std::sync::atomic::Ordering::SeqCst);
                            let win_cls_name = {
                                let cp = object_getClassName(win_cls);
                                if !cp.is_null() { std::ffi::CStr::from_ptr(cp).to_str().unwrap_or("?") } else { "?" }
                            };
                            tracing::info!(
                                retry = self.retry, win_cls_name,
                                "[browser-pane] instance-aware isMainWindow+isKeyWindow swizzle installed + overlay tagged"
                            );
                        }
                    } else {
                        tracing::info!(retry = self.retry, "[browser-pane] overlay re-tagged (swizzle already installed)");
                    }

                    // BridgedContentView stores acceptsFirstMouse as an INSTANCE
                    // VARIABLE set by initWithStyle:isFrameless:acceptsFirstMouse:.
                    // The overlay is initialized with acceptsFirstMouse=NO, so the
                    // existing implementation returns NO and NSWindow::sendEvent:
                    // drops mouseDown events (it only dispatches if isKeyWindow OR
                    // acceptsFirstMouse:YES). class_addMethod is a no-op when the
                    // method exists; we must use method_setImplementation to forcibly
                    // replace the implementation so it always returns YES regardless
                    // of the ivar. This affects all BridgedContentView instances
                    // (including the main window), which is fine: the main window is
                    // KEY so acceptsFirstMouse: is never consulted for it.
                    extern "C" fn afm_yes(_self: Id, _cmd: Sel, _event: Id) -> u8 { 1 }

                    let sel_cv = sel_registerName(b"contentView\0".as_ptr() as _);
                    let content_view = get_id(task_overlay_win, sel_cv);
                    if !content_view.is_null() {
                        let cls = object_getClass(content_view);
                        let cls_name = if !cls.is_null() {
                            let cp = object_getClassName(cls);
                            if !cp.is_null() { std::ffi::CStr::from_ptr(cp).to_str().unwrap_or("?") } else { "?" }
                        } else { "null" };
                        if !cls.is_null() {
                            let sel_afm = sel_registerName(b"acceptsFirstMouse:\0".as_ptr() as _);
                            let method = class_getInstanceMethod(cls, sel_afm);
                            let old_imp = if !method.is_null() {
                                method_setImplementation(method, afm_yes as *const std::ffi::c_void)
                            } else {
                                // Doesn't exist yet — add it
                                class_addMethod(cls, sel_afm, afm_yes as *const std::ffi::c_void, b"c@:@\0".as_ptr() as _);
                                std::ptr::null()
                            };
                            tracing::info!(
                                retry = self.retry, cls_name,
                                replaced = !old_imp.is_null(),
                                "[browser-pane] acceptsFirstMouse: REPLACED → YES on BridgedContentView"
                            );
                        }

                        // Diagnostic: how many NSView subviews does contentView have,
                        // and what does hitTest: return at the pane center?
                        // macOS routes mouseDown: to the DEEPEST subview found by
                        // hitTest: — that view (not contentView) is where the click lands.
                        // If hitTest returns BridgedContentView itself → RenderWidgetHostViewCocoa
                        // is NOT in the overlay's NSView tree → clicks never reach the renderer.
                        #[repr(C)] #[derive(Copy,Clone)] struct LPt { x: f64, y: f64 }
                        let subviews_sel = sel_registerName(b"subviews\0".as_ptr() as _);
                        let count_sel    = sel_registerName(b"count\0".as_ptr() as _);
                        let subviews = get_id(content_view, subviews_sel);
                        let sub_count: usize = if subviews.is_null() { 0 } else {
                            let get_count: extern "C" fn(Id, Sel) -> usize = std::mem::transmute(objc_msgSend as *const std::ffi::c_void);
                            get_count(subviews, count_sel)
                        };

                        // hitTest at the center of the overlay (in local contentView
                        // coordinates = DIP dimensions / 2).
                        let local_cx = task_dip_w as f64 / 2.0;
                        let local_cy = task_dip_h as f64 / 2.0;
                        let hit_sel  = sel_registerName(b"hitTest:\0".as_ptr() as _);
                        let hit_fn: extern "C" fn(Id, Sel, LPt) -> Id = std::mem::transmute(objc_msgSend as *const std::ffi::c_void);
                        let hit_view = hit_fn(content_view, hit_sel, LPt { x: local_cx, y: local_cy });
                        let hit_cls = if !hit_view.is_null() {
                            let hc = object_getClass(hit_view);
                            if !hc.is_null() {
                                let cp = object_getClassName(hc);
                                if !cp.is_null() { std::ffi::CStr::from_ptr(cp).to_str().unwrap_or("?") } else { "?" }
                            } else { "null-cls" }
                        } else { "nil-no-view" };
                        tracing::info!(
                            retry = self.retry, sub_count, hit_cls, local_cx, local_cy,
                            "[browser-pane] overlay contentView subviews + hitTest diag"
                        );

                        // NSWindow::sendEvent:mouseDown: checks acceptsFirstMouse: on
                        // the DEEPEST view returned by hitTest (hit_view), not on the
                        // contentView. We replaced acceptsFirstMouse: on BridgedContentView
                        // but the target is RenderWidgetHostViewCocoa, which has its own
                        // method. Replace it on the hit_view class as well so the non-key
                        // overlay window dispatches mouseDown: to the renderer.
                        if !hit_view.is_null() {
                            let hit_vcls = object_getClass(hit_view);
                            if !hit_vcls.is_null() {
                                let sel_afm2 = sel_registerName(b"acceptsFirstMouse:\0".as_ptr() as _);
                                // Log the current return value before replacement
                                let afm_pre: extern "C" fn(Id, Sel, Id) -> u8 = std::mem::transmute(objc_msgSend as *const std::ffi::c_void);
                                let afm_before = afm_pre(hit_view, sel_afm2, std::ptr::null_mut());
                                let hit_method = class_getInstanceMethod(hit_vcls, sel_afm2);
                                let hit_old_imp = if !hit_method.is_null() {
                                    method_setImplementation(hit_method, afm_yes as *const std::ffi::c_void)
                                } else {
                                    class_addMethod(hit_vcls, sel_afm2, afm_yes as *const std::ffi::c_void, b"c@:@\0".as_ptr() as _);
                                    std::ptr::null()
                                };
                                let hit_vcls_name = {
                                    let cp = object_getClassName(hit_vcls);
                                    if !cp.is_null() { std::ffi::CStr::from_ptr(cp).to_str().unwrap_or("?") } else { "?" }
                                };
                                tracing::info!(
                                    retry = self.retry, hit_vcls_name, afm_before,
                                    hit_replaced = !hit_old_imp.is_null(),
                                    "[browser-pane] acceptsFirstMouse BEFORE+REPLACED on hit view class"
                                );

                                // Swizzle mouseDown:, mouseUp:, hitTest:, and
                                // shouldIgnoreMouseEvent: on the RenderWidgetHostViewCocoa
                                // CLASS (not on hit_vcls which may be BridgedContentView if
                                // RWHVC hasn't been added to the overlay's view tree yet at
                                // swizzle time). Using objc_getClass guarantees we target the
                                // correct class regardless of hitTest: timing.
                                tracing::info!(
                                    retry = self.retry,
                                    hit_view = hit_view as usize,
                                    "[browser-pane] RWHVC hit_view ptr (compare with SWIZZLE HIT this)"
                                );
                                extern "C" {
                                    fn objc_getClass(name: *const std::ffi::c_char) -> Id;
                                }
                                let rwhvc_cls = objc_getClass(b"RenderWidgetHostViewCocoa\0".as_ptr() as _);
                                if !rwhvc_cls.is_null() {
                                // Mark pane as open so swizzled_nsapp_send_event activates.
                                crate::ui_tasks::PANE_LOCAL_W.store(
                                    task_dip_w, std::sync::atomic::Ordering::SeqCst);
                                tracing::info!(
                                    retry = self.retry,
                                    pw = task_dip_w,
                                    "[browser-pane] sendEvent swizzle activated (PANE_LOCAL_W set)"
                                );

                                // Swizzle shouldIgnoreMouseEvent: → always NO so the main
                                // RWHVC processes clicks even when the overlay is frontmost,
                                // without requiring it to be the key/main window.
                                if ORIG_RWHVC_SHOULD_IGNORE.load(std::sync::atomic::Ordering::SeqCst) == 0 {
                                    let sel_si = sel_registerName(b"shouldIgnoreMouseEvent:\0".as_ptr() as _);
                                    let si_method = class_getInstanceMethod(rwhvc_cls, sel_si);
                                    if !si_method.is_null() {
                                        let old_si = method_setImplementation(
                                            si_method,
                                            crate::ui_tasks::swizzled_should_ignore_mouse_event as *const std::ffi::c_void,
                                        );
                                        crate::ui_tasks::ORIG_RWHVC_SHOULD_IGNORE.store(
                                            old_si as usize,
                                            std::sync::atomic::Ordering::SeqCst,
                                        );
                                        tracing::info!(
                                            retry = self.retry,
                                            "[browser-pane] shouldIgnoreMouseEvent: SWIZZLED → always NO"
                                        );
                                    }
                                }
                                } // rwhvc_cls
                            }
                        }
                    }
                }
            }

            // Store main browser host for use in sendEvent: swizzle (set_focus before dispatch).
            // Also inject a JS mousedown listener into the main browser for diagnostics.
            #[cfg(target_os = "macos")]
            {
                let win_label_for_focus = self.window_label.clone();
                if let Some(main_browser) = self.state.get_browser(&win_label_for_focus) {
                    if let Some(host) = main_browser.host() {
                        // Store host so swizzled_nsapp_send_event can call set_focus(1).
                        *crate::ui_tasks::MAIN_BROWSER_HOST_FOR_FOCUS.lock().unwrap() = Some(host);
                        tracing::info!(
                            retry = self.retry,
                            "[browser-pane] stored main browser host for sendEvent focus restore"
                        );
                    }
                    // Inject mousedown diagnostic: tells us if clicks reach JS at all.
                    if self.retry == 1 {
                        if let Some(frame) = main_browser.main_frame() {
                            let js = "if (!window.__amxMdDiag) { window.__amxMdDiag = true; document.addEventListener('mousedown', function(e) { console.log('[AMX-DIAG-MD] mousedown x=' + e.clientX + ' y=' + e.clientY + ' tgt=' + (e.target ? e.target.tagName : 'null') + ' hasFocus=' + document.hasFocus()); }, true); console.log('[AMX-DIAG-MD] listener installed'); }";
                            let code = CefString::from(js);
                            let url  = CefString::from("");
                            frame.execute_java_script(Some(&code), Some(&url), 0);
                        }
                    }
                }
            }

            // Diagnostic: swizzle NSApp::sendEvent: once to log all leftMouseDown
            // events with their target window number. This lets us see where
            // sidebar clicks actually land (before reaching any RWHVC).
            #[cfg(target_os = "macos")]
            if self.retry == 0 {
                static NSAPP_SWIZZLE_DONE: std::sync::atomic::AtomicBool =
                    std::sync::atomic::AtomicBool::new(false);
                if !NSAPP_SWIZZLE_DONE.load(std::sync::atomic::Ordering::SeqCst) {
                    unsafe {
                        use std::ffi::c_void;
                        extern "C" {
                            fn objc_getClass(name: *const i8) -> *mut c_void;
                            fn sel_registerName(n: *const i8) -> *const c_void;
                            fn class_getInstanceMethod(cls: *mut c_void, sel: *const c_void) -> *mut c_void;
                            fn method_setImplementation(m: *mut c_void, imp: *const c_void) -> *const c_void;
                        }
                        let ns_app_cls = objc_getClass(b"NSApplication\0".as_ptr() as _);
                        if !ns_app_cls.is_null() {
                            let sel_se = sel_registerName(b"sendEvent:\0".as_ptr() as _);
                            let m_se   = class_getInstanceMethod(ns_app_cls, sel_se);
                            if !m_se.is_null() {
                                let old = method_setImplementation(
                                    m_se,
                                    crate::ui_tasks::swizzled_nsapp_send_event as *const c_void,
                                );
                                crate::ui_tasks::ORIG_NSAPP_SEND_EVENT.store(
                                    old as usize,
                                    std::sync::atomic::Ordering::SeqCst,
                                );
                                NSAPP_SWIZZLE_DONE.store(true, std::sync::atomic::Ordering::SeqCst);
                                tracing::info!(
                                    retry = self.retry,
                                    "[browser-pane] NSApp::sendEvent: swizzled for mouseDown diagnostics"
                                );
                            }
                        }
                    }
                }
            }

            // macOS: explicitly focus the pane browser renderer.
            // Link clicks already work (navigation doesn't require renderer focus),
            // but interactive clicks (form inputs, JS handlers, content-editable)
            // require document.hasFocus()=true. When makeKeyAndOrderFront:main
            // makes the main window key, CEF fires windowDidResignKey on the overlay
            // → set_focus(0) on pane browser → renderer becomes "blurred" → clicks
            // not interactive. Calling set_focus(1) on the pane browser directly
            // overrides this, making the renderer accept interactive clicks while the
            // main window retains key status (sidebar keyboard still works).
            // We post a 200ms delayed task to call set_focus(1) AFTER CEF's
            // notification-driven set_focus(0) has fired (it fires asynchronously in
            // a subsequent run-loop cycle after makeKeyAndOrderFront:main).
            #[cfg(target_os = "macos")]
            if self.retry == 1 {
                let label_clone  = self.label.clone();
                let state_clone  = self.state.clone();
                // Delayed task: after can_activate=1 lets the overlay briefly
                // steal key during creation, restore focus to the main window
                // so sidebar keyboard input keeps working. The overlay will
                // naturally re-acquire key when the user clicks into the pane.
                wrap_task! {
                    struct PaneFocusTask { state: Arc<AppState>, label: String }
                    impl Task { fn execute(&self) {
                        // Find the parent window label from the pane's overlay map.
                        let win_label = {
                            let map = self.state.browser_pane_overlays.lock();
                            map.get(&self.label).map(|(wl, _)| wl.clone())
                        };
                        if let Some(win_label) = win_label {
                            if let Some(mut main_browser) = self.state.get_browser(&win_label) {
                                if let Some(mut host) = main_browser.host() {
                                    host.set_focus(1);
                                    tracing::info!(
                                        label = %self.label, win = %win_label,
                                        "[browser-pane] restored focus to main window after pane creation"
                                    );
                                }
                            }
                            // Reschedule: every pane click fires on_set_focus(SYSTEM) which
                            // causes CEF to call set_focus(0) on the main browser, blurring
                            // its renderer and making RenderWidgetHostImpl drop subsequent
                            // mouse events before they reach Blink. Keep calling set_focus(1)
                            // on main every 200ms for as long as the pane is open so the
                            // main renderer stays focused regardless of pane click activity.
                            // The loop exits automatically when the pane closes (overlay map
                            // entry is gone → win_label is None → task does not reschedule).
                            let mut next = PaneFocusTask::new(self.state.clone(), self.label.clone());
                            post_delayed_task(ThreadId::UI, Some(&mut next), 200);
                        }
                    }}
                }
                let mut focus_task = PaneFocusTask::new(state_clone, label_clone);
                post_delayed_task(ThreadId::UI, Some(&mut focus_task), 200);
            }

            // macOS: post a delayed reaffirm on the first run. CEF's Widget::Show()
            // (called inside set_visible(1) above) schedules a deferred Views layout
            // that fires AFTER this task and resets the NSWindow frame to 0×0. The
            // 50ms delayed retry runs after all pending layouts have completed and
            // re-applies the correct frame — at that point it stays permanently.
            #[cfg(target_os = "macos")]
            if self.retry == 0 {
                let mut reaffirm = SetPaneBoundsViewsTask::new(
                    self.state.clone(),
                    self.label.clone(),
                    self.window_label.clone(),
                    self.x, self.y, self.width, self.height,
                    1, // retry=1: skip set_visible(1), just reaffirm frame
                    self.overlay_wnum,
                );
                post_delayed_task(ThreadId::UI, Some(&mut reaffirm), 50);
            }
        }
    }
}

#[cfg(not(target_os = "windows"))]
pub fn post_set_pane_bounds_views(
    state: &Arc<AppState>,
    label: &str,
    window_label: &str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    retry: u32,
    overlay_wnum: isize,
) {
    let mut task = SetPaneBoundsViewsTask::new(
        state.clone(),
        label.to_string(),
        window_label.to_string(),
        x,
        y,
        width,
        height,
        retry,
        overlay_wnum,
    );
    post_task(ThreadId::UI, Some(&mut task));
}
