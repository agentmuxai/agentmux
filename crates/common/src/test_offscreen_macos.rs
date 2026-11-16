// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! macOS half of [`test_offscreen`](crate::test_offscreen): no window of the process is ever ordered in.
//!
//! AppKit moves a window onto a display when it is first shown and clamps every later move, so a
//! window cannot be parked off-screen the way it can on Windows or X11, and hiding one (alpha 0, a
//! veil) is undone by the next code that resets it. A window that was never ordered in is never on a
//! display: it is never in the window server's on-screen list, a move of its frame holds, and its
//! content still runs and draws. (Bare-window probes on starpower, 2026-11-15: `never_ordered_in`
//! and `ordering_blocked`; with the replacements below, none of `orderFront:`,
//! `makeKeyAndOrderFront:`, `orderFrontRegardless`, `setIsVisible:YES`, `orderWindow:relativeTo:`
//! or `addChildWindow:ordered:` on an unordered parent listed a window, and without them five did.)
//!
//! So a test process replaces the ways to put a window on the screen, on `NSWindow` itself and so
//! on every window class that inherits them, with functions that do nothing, and checks that its
//! replacement is the one the runtime now holds. It can say no and not start ([`engage`]). A watchdog
//! is the second line: if a normal window of this process is ever on the on-screen list anyway
//! (a way in not covered here), the process stops at once.

use std::ffi::{c_char, c_void};
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicUsize, Ordering};
use std::time::Duration;

type Id = *mut c_void;
type Sel = *const c_void;
type Class = *mut c_void;
type Method = *mut c_void;

const NS_WINDOW_OUT: isize = 0;

#[link(name = "objc")]
extern "C" {
    fn objc_getClass(name: *const c_char) -> Class;
    fn sel_registerName(name: *const c_char) -> Sel;
    fn class_getInstanceMethod(class: Class, sel: Sel) -> Method;
    fn method_getImplementation(method: Method) -> *const c_void;
    fn method_setImplementation(method: Method, imp: *const c_void) -> *const c_void;
}

extern "C" {
    fn dlopen(path: *const c_char, mode: i32) -> *mut c_void;
}

#[link(name = "CoreGraphics", kind = "framework")]
extern "C" {
    fn CGWindowListCopyWindowInfo(option: u32, relative_to: u32) -> *const c_void;
    static kCGWindowOwnerPID: *const c_void;
    static kCGWindowLayer: *const c_void;
}

#[link(name = "CoreFoundation", kind = "framework")]
extern "C" {
    fn CFArrayGetCount(array: *const c_void) -> isize;
    fn CFArrayGetValueAtIndex(array: *const c_void, index: isize) -> *const c_void;
    fn CFDictionaryGetValue(dict: *const c_void, key: *const c_void) -> *const c_void;
    fn CFNumberGetValue(number: *const c_void, kind: isize, out: *mut c_void) -> u8;
    fn CFRelease(object: *const c_void);
}

/// `kCGWindowListOptionOnScreenOnly`.
const ON_SCREEN_ONLY: u32 = 1;
/// `kCFNumberSInt32Type`.
const SINT32: isize = 3;
const RTLD_LAZY: i32 = 1;

/// How many show requests were dropped, for the log.
static DROPPED: AtomicUsize = AtomicUsize::new(0);
/// The runtime's own `orderWindow:relativeTo:` and `setIsVisible:`, kept to pass an order-out through.
static ORIGINAL_ORDER_WINDOW: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static ORIGINAL_SET_VISIBLE: AtomicPtr<c_void> = AtomicPtr::new(std::ptr::null_mut());
static INSTALLED: AtomicBool = AtomicBool::new(false);

/// `-[NSWindow orderWindow:relativeTo:]`: every request but "out" is dropped.
extern "C" fn order_window(this: Id, sel: Sel, mode: isize, other: isize) {
    if mode == NS_WINDOW_OUT {
        let original = ORIGINAL_ORDER_WINDOW.load(Ordering::Relaxed);
        if !original.is_null() {
            // SAFETY: the pointer is the runtime's implementation of this very selector.
            let original: extern "C" fn(Id, Sel, isize, isize) = unsafe { std::mem::transmute(original) };
            original(this, sel, mode, other);
        }
    } else {
        DROPPED.fetch_add(1, Ordering::Relaxed);
    }
}

/// `-[NSWindow setIsVisible:]`: YES is dropped, NO passes through.
extern "C" fn set_is_visible(this: Id, sel: Sel, visible: bool) {
    if visible {
        DROPPED.fetch_add(1, Ordering::Relaxed);
        return;
    }
    let original = ORIGINAL_SET_VISIBLE.load(Ordering::Relaxed);
    if !original.is_null() {
        // SAFETY: the pointer is the runtime's implementation of this very selector.
        let original: extern "C" fn(Id, Sel, bool) = unsafe { std::mem::transmute(original) };
        original(this, sel, visible);
    }
}

/// A request with one object argument (`orderFront:`, `makeKeyAndOrderFront:`, `deminiaturize:`).
extern "C" fn drop_with_object(_this: Id, _sel: Sel, _arg: Id) {
    DROPPED.fetch_add(1, Ordering::Relaxed);
}

/// A request with no argument (`orderFrontRegardless`, `activate`).
extern "C" fn drop_plain(_this: Id, _sel: Sel) {
    DROPPED.fetch_add(1, Ordering::Relaxed);
}

/// A request with one flag (`activateIgnoringOtherApps:`).
extern "C" fn drop_with_flag(_this: Id, _sel: Sel, _flag: bool) {
    DROPPED.fetch_add(1, Ordering::Relaxed);
}

/// Replaces `selector` (a NUL-terminated name) of `class` with `imp`, keeping the old one in `keep`.
/// A missing method is an error when `required`, and skipped otherwise (an OS that lacks it cannot
/// call it either). The replacement is read back: an install that did not take is an error.
///
/// # Safety
/// `imp` must have the signature the selector is called with.
unsafe fn replace(
    class: Class,
    selector: &[u8],
    imp: *const c_void,
    keep: Option<&AtomicPtr<c_void>>,
    required: bool,
) -> Result<(), String> {
    let name = String::from_utf8_lossy(&selector[..selector.len() - 1]).into_owned();
    let method = class_getInstanceMethod(class, sel_registerName(selector.as_ptr() as *const c_char));
    if method.is_null() {
        return if required { Err(format!("{name} not found")) } else { Ok(()) };
    }
    let old = method_setImplementation(method, imp);
    if let Some(slot) = keep {
        slot.store(old as *mut c_void, Ordering::Relaxed);
    }
    if method_getImplementation(method) != imp {
        return Err(format!("{name} was not replaced"));
    }
    Ok(())
}

/// Replaces every way `NSWindow` and `NSApplication` put a window on the screen. Does nothing the
/// second time. An error says what could not be done; the process must not go on then.
pub fn install() -> Result<(), String> {
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return Ok(());
    }
    let result = unsafe { install_all() };
    if result.is_err() {
        INSTALLED.store(false, Ordering::SeqCst);
    }
    result
}

unsafe fn install_all() -> Result<(), String> {
    let mut window = objc_getClass(b"NSWindow\0".as_ptr() as *const c_char);
    if window.is_null() {
        // A process that never touched AppKit (a test binary) has not loaded it yet.
        dlopen(b"/System/Library/Frameworks/AppKit.framework/AppKit\0".as_ptr() as *const c_char, RTLD_LAZY);
        window = objc_getClass(b"NSWindow\0".as_ptr() as *const c_char);
    }
    let app = objc_getClass(b"NSApplication\0".as_ptr() as *const c_char);
    if window.is_null() || app.is_null() {
        return Err("AppKit's NSWindow or NSApplication is not available".to_string());
    }
    replace(window, b"orderWindow:relativeTo:\0", order_window as *const c_void, Some(&ORIGINAL_ORDER_WINDOW), true)?;
    replace(window, b"setIsVisible:\0", set_is_visible as *const c_void, Some(&ORIGINAL_SET_VISIBLE), true)?;
    replace(window, b"orderFront:\0", drop_with_object as *const c_void, None, true)?;
    replace(window, b"makeKeyAndOrderFront:\0", drop_with_object as *const c_void, None, true)?;
    replace(window, b"orderFrontRegardless\0", drop_plain as *const c_void, None, true)?;
    // A miniaturized window comes back on the screen with this.
    replace(window, b"deminiaturize:\0", drop_with_object as *const c_void, None, false)?;
    // Activating the app takes the keyboard from the person's frontmost app and shows our menu bar.
    replace(app, b"activateIgnoringOtherApps:\0", drop_with_flag as *const c_void, None, false)?;
    replace(app, b"activate\0", drop_plain as *const c_void, None, false)?;
    Ok(())
}

/// A number the window server lists for a window, by key.
unsafe fn number(dict: *const c_void, key: *const c_void) -> Option<i32> {
    let value = CFDictionaryGetValue(dict, key);
    if value.is_null() {
        return None;
    }
    let mut out: i32 = 0;
    (CFNumberGetValue(value, SINT32, &mut out as *mut i32 as *mut c_void) != 0).then_some(out)
}

/// How many normal windows (layer 0: not the menu-bar item, the Dock or a tooltip) of this process
/// the window server lists as on screen. Needs no screen-recording permission.
pub fn windows_on_screen() -> usize {
    let pid = std::process::id() as i32;
    let mut found = 0;
    // SAFETY: the list is owned here and released; each entry is read while the list lives.
    unsafe {
        let list = CGWindowListCopyWindowInfo(ON_SCREEN_ONLY, 0);
        if list.is_null() {
            return 0;
        }
        for i in 0..CFArrayGetCount(list) {
            let entry = CFArrayGetValueAtIndex(list, i);
            if number(entry, kCGWindowOwnerPID) == Some(pid) && number(entry, kCGWindowLayer) == Some(0) {
                found += 1;
            }
        }
        CFRelease(list);
    }
    found
}

/// Stops the process when a normal window of it is ever on the screen.
fn start_watchdog(process: &str) -> Result<(), String> {
    let process = process.to_string();
    std::thread::Builder::new()
        .name("test-offscreen-watchdog".into())
        .spawn(move || loop {
            std::thread::sleep(Duration::from_millis(200));
            let shown = windows_on_screen();
            if shown > 0 {
                let dropped = DROPPED.load(Ordering::Relaxed);
                eprintln!("{process}: test window mode: {shown} window(s) of this process are on the screen ({dropped} show request(s) were blocked); stopping");
                std::process::exit(2);
            }
        })
        .map(|_| ())
        .map_err(|e| format!("the watchdog thread did not start: {e}"))
}

/// Puts this process in "no window is ever ordered in" mode: the replacements first, then the
/// watchdog. An error says why the process must not start.
pub fn engage(process: &str) -> Result<(), String> {
    install()?;
    start_watchdog(process)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn install_takes_and_a_second_call_is_a_no_op() {
        install().expect("the replacements install");
        install().expect("a second install does nothing");
    }

    #[test]
    fn a_process_with_no_windows_lists_none() {
        assert_eq!(windows_on_screen(), 0);
    }
}
