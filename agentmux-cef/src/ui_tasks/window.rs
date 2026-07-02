// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Top-level window tasks: deferred load_url, close, memory-pressure banner,
// minimize / maximize / focus / move / position / rect, window-at-cursor
// resolution, corrective move, window creation, DevTools, and main-focus
// reclaim. Split out of `ui_tasks.rs` unchanged.

use std::sync::Arc;
use cef::*;
use crate::state::AppState;
use super::get_window_on_ui;

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

// ── Window alpha (macOS uniform whole-window opacity) ────────────────────
// Track 1 of SPEC_TRANSPARENCY_MACOS_LINUX_2026_07_01: the macOS analogue of
// Windows' WS_EX_LAYERED + SetLayeredWindowAttributes(LWA_ALPHA) — a
// WindowServer-level uniform fade of the finished window over the desktop.
// Applied post-render, so it needs zero CEF/renderer cooperation and works on
// stock and patched frameworks alike. Per-pixel ("glass") transparency is the
// separate patched-libcef track and is orthogonal to this.

#[cfg(target_os = "macos")]
wrap_task! {
    pub struct SetWindowAlphaTask {
        state: Arc<AppState>,
        label: String,
        alpha: f64,
    }

    impl Task {
        fn execute(&self) {
            let Some(window) = get_window_on_ui(&self.state, &self.label) else {
                tracing::warn!(label = %self.label, "[opacity] SetWindowAlphaTask: no window for label");
                return;
            };
            let nsview = window.window_handle() as *mut std::ffi::c_void;
            if nsview.is_null() {
                tracing::warn!(label = %self.label, "[opacity] SetWindowAlphaTask: null NSView handle");
                return;
            }
            if unsafe { macos_set_nswindow_alpha(nsview, self.alpha) } {
                tracing::info!(label = %self.label, alpha = self.alpha, "[opacity] applied NSWindow alphaValue");
            } else {
                // Reagent P2 on #1895: don't claim success when the NSView has
                // no NSWindow yet (window not realized) — nothing was applied.
                tracing::warn!(label = %self.label, "[opacity] SetWindowAlphaTask: NSView has no NSWindow; alpha not applied");
                return;
            }

            // Codex P2 on #1895: browser-pane overlays are separate
            // NativeWidgetMacNSWindow instances layered over this window
            // (browser_pane/creation_views.rs) — child NSWindows do NOT
            // inherit the parent's alphaValue, so without this a faded host
            // window keeps fully-opaque pane rectangles floating on top.
            // Resolve each overlay belonging to this window_label via the
            // cached window numbers and fade it to the same alpha.
            // try_lock (matching the overlay-wnum cache writers): missing a
            // beat here only delays an overlay fade until the next opacity
            // event, which beats risking a UI-thread stall.
            let overlay_wnums: Vec<isize> = {
                let (Some(overlays), Some(wnums)) = (
                    self.state.browser_pane_overlays.try_lock(),
                    self.state.browser_pane_overlay_wnums.try_lock(),
                ) else {
                    tracing::warn!(label = %self.label, "[opacity] overlay maps busy; pane overlays not faded this pass");
                    return;
                };
                overlays
                    .iter()
                    .filter(|(_, (window_label, _))| window_label == &self.label)
                    .filter_map(|(pane_label, _)| wnums.get(pane_label).copied())
                    .collect()
            };
            for wnum in overlay_wnums {
                if unsafe { macos_set_window_alpha_by_number(wnum, self.alpha) } {
                    tracing::info!(label = %self.label, wnum, alpha = self.alpha, "[opacity] applied alphaValue to pane overlay window");
                } else {
                    tracing::warn!(label = %self.label, wnum, "[opacity] pane overlay NSWindow not found for wnum");
                }
            }
        }
    }
}

#[cfg(target_os = "macos")]
pub fn post_set_window_alpha(state: &Arc<AppState>, label: &str, alpha: f64) {
    let mut task = SetWindowAlphaTask::new(state.clone(), label.to_string(), alpha);
    post_task(ThreadId::UI, Some(&mut task));
}

/// `[[nsview window] setAlphaValue:alpha]` — raw libobjc FFI, mirroring
/// `ensure_macos_native_window_buttons` in app.rs. `alphaValue` takes CGFloat
/// (f64 on both arm64 and x86_64, passed in a float register, so plain
/// objc_msgSend is correct). AppKit call — must run on the UI/main thread,
/// which SetWindowAlphaTask guarantees. Returns false when the NSView has no
/// NSWindow (nothing applied).
#[cfg(target_os = "macos")]
unsafe fn macos_set_nswindow_alpha(nsview: *mut std::ffi::c_void, alpha: f64) -> bool {
    use std::ffi::{c_char, c_void};
    type Id = *mut c_void;
    type Sel = *const c_void;
    extern "C" {
        fn sel_registerName(name: *const c_char) -> Sel;
        fn objc_msgSend();
    }

    // nswindow = [nsview window]
    let get_window: extern "C" fn(Id, Sel) -> Id =
        std::mem::transmute(objc_msgSend as *const c_void);
    let nswindow = get_window(nsview, sel_registerName(b"window\0".as_ptr() as _));
    if nswindow.is_null() {
        return false;
    }

    // [nswindow setAlphaValue: alpha]
    let set_alpha: extern "C" fn(Id, Sel, f64) =
        std::mem::transmute(objc_msgSend as *const c_void);
    set_alpha(nswindow, sel_registerName(b"setAlphaValue:\0".as_ptr() as _), alpha);
    true
}

/// `[[NSApp windowWithWindowNumber:wnum] setAlphaValue:alpha]` — fade a
/// window resolved by its WindowServer window number (the form the
/// browser-pane overlay cache stores). Returns false when no window matches
/// (overlay already closed / wnum stale). UI/main thread only.
#[cfg(target_os = "macos")]
unsafe fn macos_set_window_alpha_by_number(wnum: isize, alpha: f64) -> bool {
    use std::ffi::{c_char, c_void};
    type Id = *mut c_void;
    type Sel = *const c_void;
    extern "C" {
        fn sel_registerName(name: *const c_char) -> Sel;
        fn objc_getClass(name: *const c_char) -> Id;
        fn objc_msgSend();
    }

    let msg: extern "C" fn(Id, Sel) -> Id = std::mem::transmute(objc_msgSend as *const c_void);
    let nsapp = msg(
        objc_getClass(b"NSApplication\0".as_ptr() as _),
        sel_registerName(b"sharedApplication\0".as_ptr() as _),
    );
    if nsapp.is_null() {
        return false;
    }

    // [nsapp windowWithWindowNumber: wnum]
    let win_by_num: extern "C" fn(Id, Sel, isize) -> Id =
        std::mem::transmute(objc_msgSend as *const c_void);
    let nswindow = win_by_num(
        nsapp,
        sel_registerName(b"windowWithWindowNumber:\0".as_ptr() as _),
        wnum,
    );
    if nswindow.is_null() {
        return false;
    }

    let set_alpha: extern "C" fn(Id, Sel, f64) =
        std::mem::transmute(objc_msgSend as *const c_void);
    set_alpha(nswindow, sel_registerName(b"setAlphaValue:\0".as_ptr() as _), alpha);
    true
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
            // Primary: browser_view.window() — works on Windows and on non-Windows
            // windows that haven't finished loading. On Linux/macOS the Views
            // BrowserView loses its Window reference post-page-load, so
            // browser_view.window() returns None. Fall back to state.windows,
            // which is populated via on_window_created and stays valid for the
            // lifetime of the window (same registry ResolveWindowAtCursorTask uses).
            let pos = if let Some(w) = get_window_on_ui(&self.state, &self.label) {
                let b = w.bounds();
                Some((b.x, b.y))
            } else {
                // Fall back to `state.windows` on Linux/macOS, where the Views
                // BrowserView loses its Window reference post-page-load. That map
                // is `cfg(not(windows))`-only (Windows uses native HWND lookup and
                // never populates it), so on Windows the primary path above is the
                // only source — gate the fallback to keep the Windows build green.
                #[cfg(not(target_os = "windows"))]
                {
                    self.state.windows.lock().get(&self.label).map(|w| {
                        let b = w.bounds();
                        (b.x, b.y)
                    })
                }
                #[cfg(target_os = "windows")]
                {
                    None
                }
            };
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

// ── Get window full rect (DIP) — blocking UI-thread read ─────────────────
//
// Like GetWindowPositionTask but returns (x, y, width, height). Used by the
// macOS / Linux floater edge-resize path (`get_window_rect` IPC) to capture
// the start rect on pointer-down — Windows reads it directly via GetWindowRect.
wrap_task! {
    pub struct GetWindowRectTask {
        state: Arc<AppState>,
        label: String,
        tx: std::sync::mpsc::SyncSender<Option<(i32, i32, i32, i32)>>,
    }

    impl Task {
        fn execute(&self) {
            let rect = get_window_on_ui(&self.state, &self.label).map(|w| {
                let b = w.bounds();
                (b.x, b.y, b.width, b.height)
            });
            let _ = self.tx.try_send(rect);
        }
    }
}

/// Read a CEF Views window's full rect (DIP) from the IPC thread by bouncing
/// through the UI thread. Returns `None` if the window isn't found or the UI
/// thread doesn't answer within the timeout.
pub fn get_window_rect_blocking(state: &Arc<AppState>, label: &str) -> Option<(i32, i32, i32, i32)> {
    let (tx, rx) = std::sync::mpsc::sync_channel::<Option<(i32, i32, i32, i32)>>(1);
    let mut task = GetWindowRectTask::new(state.clone(), label.to_string(), tx);
    post_task(ThreadId::UI, Some(&mut task));
    // 500ms: the UI thread may still be processing set_bounds tasks queued
    // during a prior drag — give it more headroom before treating as failed.
    rx.recv_timeout(std::time::Duration::from_millis(500)).ok().flatten()
}

// ── Set window rect (position + size, DIP) ───────────────────────────────
//
// Non-Windows analogue of the Windows SetWindowPos call in `set_window_rect`.
// Used by the floater edge-resize drag: the frontend captures the start rect on
// pointer-down, computes a new rect per cursor delta + edge, and calls this on
// each move. `set_bounds` is self-contained (no read-modify-write) so concurrent
// in-flight calls are idempotent — last write wins.
wrap_task! {
    pub struct SetWindowRectTask {
        state: Arc<AppState>,
        label: String,
        x: i32,
        y: i32,
        width: i32,
        height: i32,
    }

    impl Task {
        fn execute(&self) {
            if let Some(window) = get_window_on_ui(&self.state, &self.label) {
                window.set_bounds(Some(&cef::Rect {
                    x: self.x,
                    y: self.y,
                    width: self.width,
                    height: self.height,
                }));
            }
        }
    }
}

pub fn post_set_window_rect(state: &Arc<AppState>, label: &str, x: i32, y: i32, width: i32, height: i32) {
    let mut task = SetWindowRectTask::new(state.clone(), label.to_string(), x, y, width, height);
    post_task(ThreadId::UI, Some(&mut task));
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
                //
                // Two sources are combined:
                // 1. Live registered panes from state.list_browsers().
                // 2. Pane outer HWNDs still tracked in BROWSER_PANE_HWND_CONTEXT
                //    — covers the window between BrowserUnregistered and CEF's
                //    on_before_close (deferred teardown), during which the HWND
                //    is still live but the label is gone from state.browsers.
                //    Without this, panes_excluded=0 and find_main_render_widget
                //    picks the pane's render widget → infinite focus storm.
                //
                // Phase H.2.b — reducer-aware iteration with fallback.
                let pane_outer_hwnds: Vec<*mut std::ffi::c_void> = {
                    let mut hwnds: Vec<*mut std::ffi::c_void> = self
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
                    for h in crate::browser_pane::hwnd::pane_outer_hwnds_from_context() {
                        if !hwnds.contains(&h) {
                            hwnds.push(h);
                        }
                    }
                    hwnds
                };

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
