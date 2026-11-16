// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! The mouse for a pane window with toasts over it (macOS), by pixel.
//!
//! A pane is its own `NSWindow` above the app's, and macOS sends every mouse event to the topmost window under
//! the cursor, whatever is drawn there (`pane_hole_mask.rs`). A menu or a dialog over a pane is the app's whole
//! attention, so the window then ignores the mouse altogether. A toast is not: the page under and around it
//! stays in use, and the toast's buttons (close, copy, "+N more") have to work. So while only toasts are over a
//! pane, its window ignores the mouse exactly when the cursor is over one of them.
//!
//! A window cannot say which of its pixels take the mouse, so a timer on the main run loop, running only while
//! such a hole is open, reads where the cursor is and sets `ignoresMouseEvents` from it
//! (`overlay_holes::ignore_mouse`). A button held down keeps it as it is, so a drag in the page goes on over a
//! toast and a press on a toast ends on it. The cursor crossing a toast's edge is seen within one tick (about
//! 16 ms), before a person can aim a click at it.
//!
//! Must run on the CEF UI thread (= the AppKit main thread): `track` and `stop` from the clip task, the timer
//! from the main run loop.

#![cfg(target_os = "macos")]

use std::collections::HashMap;
use std::ffi::{c_char, c_void};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

use crate::browser_panes::overlay_holes::{ignore_mouse, point_in_any, LocalRect, MouseRouting};

type Id = *mut c_void;
type Sel = *const c_void;

#[repr(C)]
#[derive(Copy, Clone)]
struct CGPoint {
    x: f64,
    y: f64,
}

extern "C" {
    fn sel_registerName(name: *const c_char) -> Sel;
    fn objc_msgSend();
    fn objc_getClass(name: *const c_char) -> Id;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    static kCFRunLoopCommonModes: *const c_void;
    fn CFAbsoluteTimeGetCurrent() -> f64;
    fn CFRunLoopGetMain() -> *mut c_void;
    fn CFRunLoopTimerCreate(
        allocator: *const c_void,
        fire_date: f64,
        interval: f64,
        flags: u64,
        order: isize,
        callout: extern "C" fn(*mut c_void, *mut c_void),
        context: *mut c_void,
    ) -> *mut c_void;
    fn CFRunLoopAddTimer(run_loop: *mut c_void, timer: *mut c_void, mode: *const c_void);
    fn CFRunLoopTimerInvalidate(timer: *mut c_void);
    fn CFRelease(cf: *const c_void);
}

/// How often the cursor is read while a toast is over a pane.
const TICK_SECONDS: f64 = 1.0 / 60.0;

/// The toast holes of each pane window being tracked, by window number.
static ROUTES: Mutex<Option<HashMap<isize, Vec<LocalRect>>>> = Mutex::new(None);
/// The run loop timer, or 0 when there is none: it exists exactly while a window is tracked.
static TIMER: AtomicUsize = AtomicUsize::new(0);

fn routes() -> std::sync::MutexGuard<'static, Option<HashMap<isize, Vec<LocalRect>>>> {
    ROUTES.lock().unwrap_or_else(|p| p.into_inner())
}

/// Set where the mouse goes for pane window `win` (its `NSWindow`, number `wnum`) while its holes are open.
/// `App`: the whole window ignores it, so a menu or dialog behind gets every event. `Page`: the window takes it.
/// `ByPixel`: it takes it except over a toast, by `holes` (points, origin at the window's bottom-left).
pub fn apply(win: Id, wnum: isize, routing: MouseRouting, holes: Vec<LocalRect>) {
    match routing {
        MouseRouting::ByPixel => {
            set_ignores_mouse(win, false);
            track(wnum, holes);
        }
        MouseRouting::App => {
            stop(wnum);
            set_ignores_mouse(win, true);
        }
        MouseRouting::Page => {
            stop(wnum);
            set_ignores_mouse(win, false);
        }
    }
}

fn set_ignores_mouse(win: Id, ignore: bool) {
    unsafe {
        let sel = sel_registerName(b"setIgnoresMouseEvents:\0".as_ptr() as _);
        let set_bool: extern "C" fn(Id, Sel, u8) = std::mem::transmute(objc_msgSend as *const c_void);
        set_bool(win, sel, ignore as u8);
    }
}

/// Start (or update) routing the mouse by pixel for pane window `wnum`, whose toast holes are `holes`
/// (points, origin at the window's bottom-left). The window's state is set from the cursor now.
pub fn track(wnum: isize, holes: Vec<LocalRect>) {
    routes().get_or_insert_with(Default::default).insert(wnum, holes);
    ensure_timer();
    tick();
}

/// Stop routing the mouse by pixel for `wnum` (its holes cleared, or a menu took the whole window).
/// The caller sets the window's own state; the timer goes with the last window.
pub fn stop(wnum: isize) {
    let none_left = {
        let mut guard = routes();
        match guard.as_mut() {
            Some(m) => {
                m.remove(&wnum);
                m.is_empty()
            }
            None => true,
        }
    };
    if none_left {
        stop_timer();
    }
}

fn ensure_timer() {
    if TIMER.load(Ordering::Acquire) != 0 {
        return;
    }
    unsafe {
        let timer = CFRunLoopTimerCreate(
            std::ptr::null(),
            CFAbsoluteTimeGetCurrent() + TICK_SECONDS,
            TICK_SECONDS,
            0,
            0,
            on_timer,
            std::ptr::null_mut(),
        );
        if timer.is_null() {
            tracing::warn!("[pane-hole-mouse] could not create the timer: a toast over a pane is not click-routed");
            return;
        }
        CFRunLoopAddTimer(CFRunLoopGetMain(), timer, kCFRunLoopCommonModes);
        TIMER.store(timer as usize, Ordering::Release);
    }
}

fn stop_timer() {
    let timer = TIMER.swap(0, Ordering::AcqRel);
    if timer != 0 {
        unsafe {
            CFRunLoopTimerInvalidate(timer as *mut c_void);
            CFRelease(timer as *const c_void);
        }
    }
}

extern "C" fn on_timer(_timer: *mut c_void, _info: *mut c_void) {
    tick();
}

/// One reading of the cursor: set each tracked window to ignore the mouse or not.
fn tick() {
    let tracked: Vec<(isize, Vec<LocalRect>)> = match routes().as_ref() {
        Some(m) if !m.is_empty() => m.iter().map(|(w, h)| (*w, h.clone())).collect(),
        _ => {
            stop_timer();
            return;
        }
    };
    unsafe {
        let sel_mouse_location = sel_registerName(b"mouseLocation\0".as_ptr() as _);
        let sel_pressed = sel_registerName(b"pressedMouseButtons\0".as_ptr() as _);
        let sel_shared_app = sel_registerName(b"sharedApplication\0".as_ptr() as _);
        let sel_window_with_number = sel_registerName(b"windowWithWindowNumber:\0".as_ptr() as _);
        let sel_is_visible = sel_registerName(b"isVisible\0".as_ptr() as _);
        let sel_convert = sel_registerName(b"convertPointFromScreen:\0".as_ptr() as _);
        let sel_ignores = sel_registerName(b"ignoresMouseEvents\0".as_ptr() as _);
        let sel_set_ignores = sel_registerName(b"setIgnoresMouseEvents:\0".as_ptr() as _);

        let get_id: extern "C" fn(Id, Sel) -> Id = std::mem::transmute(objc_msgSend as *const c_void);
        let get_point: extern "C" fn(Id, Sel) -> CGPoint = std::mem::transmute(objc_msgSend as *const c_void);
        let get_usize: extern "C" fn(Id, Sel) -> usize = std::mem::transmute(objc_msgSend as *const c_void);
        let get_bool: extern "C" fn(Id, Sel) -> u8 = std::mem::transmute(objc_msgSend as *const c_void);
        let window_with_number: extern "C" fn(Id, Sel, isize) -> Id = std::mem::transmute(objc_msgSend as *const c_void);
        let convert: extern "C" fn(Id, Sel, CGPoint) -> CGPoint = std::mem::transmute(objc_msgSend as *const c_void);
        let set_bool: extern "C" fn(Id, Sel, u8) = std::mem::transmute(objc_msgSend as *const c_void);

        let ns_event = objc_getClass(b"NSEvent\0".as_ptr() as _);
        let ns_app = get_id(objc_getClass(b"NSApplication\0".as_ptr() as _), sel_shared_app);
        let mouse = get_point(ns_event, sel_mouse_location);
        let button_down = get_usize(ns_event, sel_pressed) != 0;

        for (wnum, holes) in tracked {
            let win = window_with_number(ns_app, sel_window_with_number, wnum);
            if win.is_null() {
                // The window is gone: nothing left to route.
                stop(wnum);
                continue;
            }
            if get_bool(win, sel_is_visible) == 0 {
                continue;
            }
            let local = convert(win, sel_convert, mouse);
            let inside = point_in_any(&holes, (local.x, local.y));
            let ignoring = get_bool(win, sel_ignores) != 0;
            let want = ignore_mouse(inside, button_down, ignoring);
            if want != ignoring {
                set_bool(win, sel_set_ignores, want as u8);
            }
        }
    }
}
