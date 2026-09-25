// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// macOS-specific swizzle storage + swizzled ObjC implementations used by the
// browser-pane overlay path, plus `clear_pane_swizzle_statics` (with a no-op
// non-macOS variant). Split out of `ui_tasks.rs` unchanged.

// macOS: swizzle storage for NativeWidgetMacNSWindow::isMainWindow / isKeyWindow.
// The swizzled implementations check objc_getAssociatedObject on `self` — only the
// specific overlay NSWindow instance is tagged, so pool windows (same class) are
// unaffected and continue returning their real values.
// `set_focus` on cef::BrowserHost is a trait method — the #1891 split dropped
// the old monolith's `use cef::*`, leaving this module without the trait in
// scope. Nothing compiles this file except macOS builds, so no CI caught it
// (main was broken on macOS from #1891 until this line).
#[cfg(target_os = "macos")]
use cef::ImplBrowserHost;

#[cfg(target_os = "macos")]
pub(crate) static ORIG_IS_MAIN_WINDOW: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

#[cfg(target_os = "macos")]
pub(crate) static ORIG_IS_KEY_WINDOW: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

// Per-window browser host map.  Key = NSWindow* (as usize), value = CefBrowserHost
// for that window's main browser.  One entry per open pane overlay; used by
// swizzled_nsapp_send_event both as the routing gate (only intercept windows that
// appear here) and to call set_focus(1) on the correct host before dispatching.
// Updated by SetPaneBoundsViewsTask on every open/re-open; entries are removed
// individually by clear_pane_swizzle_statics when the corresponding pane closes.
#[cfg(target_os = "macos")]
pub(crate) static PANE_WIN_TO_HOST: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<usize, cef::BrowserHost>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

// Maps AgentMux window_label → NSWindow* so clear_pane_swizzle_statics can find
// the right entry in PANE_WIN_TO_HOST without walking ObjC at close time.
#[cfg(target_os = "macos")]
pub(crate) static PANE_LABEL_TO_WIN: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<String, usize>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

// Unique static address used as the objc_setAssociatedObject key for the overlay tag.
#[cfg(target_os = "macos")]
pub(crate) static PANE_OVERLAY_TAG_KEY: u8 = 0;

// shouldIgnoreMouseEvent: swizzle storage. Original returns YES when the RWHVC's
// window is not key/main, silently dropping mouseDown:. We override to always NO.
#[cfg(target_os = "macos")]
pub(crate) static ORIG_RWHVC_SHOULD_IGNORE: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

// Non-zero while a pane overlay is open. Used by swizzled_nsapp_send_event as the
// gate: if 0 the swizzle is inactive and all events fall through to the original.
#[cfg(target_os = "macos")]
pub(crate) static PANE_LOCAL_W: std::sync::atomic::AtomicI32 = std::sync::atomic::AtomicI32::new(0);

// Overlay NSWindow* (the pane's OWN window — not the main window key used by
// PANE_WIN_TO_HOST/PANE_LABEL_TO_WIN above) → (pane label, block_id, weak
// AppState). Populated alongside those two maps by SetPaneBoundsViewsTask.
// Read by swizzled_nsapp_send_event's `is_overlay` branch to detect a click
// landing directly on a pane's rendered body and emit `browser-pane-clicked`
// — the click-to-select signal every in-DOM pane gets for free via ordinary
// click bubbling to blockframe.tsx (see block/block.tsx::handleBlockClick),
// but a browser pane's content is a separate native BrowserView layered on
// top, so no DOM click ever fires for it. Mirrors the Windows
// WM_LBUTTONDOWN → browser-pane-clicked path in browser_pane/hwnd.rs; this
// is the macOS equivalent — Linux still has no click-to-select for browser
// pane bodies, see docs/specs/SPEC_BROWSER_PANE_CLICK_TO_SELECT_2026_07_07.md.
#[cfg(target_os = "macos")]
pub(crate) static PANE_OVERLAY_WIN_TO_BLOCK: std::sync::LazyLock<
    std::sync::Mutex<
        std::collections::HashMap<usize, (String, String, std::sync::Weak<crate::state::AppState>)>,
    >,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// Swizzled NSWindow::isMainWindow — returns YES only for the tagged pane overlay.
/// All other NativeWidgetMacNSWindow instances (pool windows, etc.) call through
/// to the original implementation and return their real value.
#[cfg(target_os = "macos")]
pub(crate) extern "C" fn swizzled_is_main_window(
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
pub(crate) extern "C" fn swizzled_is_key_window(
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

#[cfg(target_os = "macos")]
pub(crate) static ORIG_CAN_BECOME_KEY_WINDOW: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

/// Swizzled NSWindow::canBecomeKeyWindow — same instance-aware pattern as
/// isMainWindow. The overlay is created with `can_activate=0`, so Chromium's
/// own implementation returns NO and the pane can never become the key window:
/// AppKit then delivers every keystroke to the main window, and nothing typed
/// ever reaches the page (confirmed live 2026-09-24 — after a real click on the
/// pane, `canBecomeKeyWindow=0` and `NSApp.keyWindow` stayed the main
/// CefNSWindow). Returning YES for the tagged overlay lets
/// `make_pane_overlay_key` hand it key status on an explicit pane click/focus.
/// See docs/specs/SPEC_MACOS_BROWSER_PANE_KEYBOARD_FOCUS_2026_09_24.md.
#[cfg(target_os = "macos")]
pub(crate) extern "C" fn swizzled_can_become_key_window(
    this: *mut std::ffi::c_void,
    cmd: *const std::ffi::c_void,
) -> u8 {
    if is_tagged_pane_overlay(this) { return 1; }
    let orig = ORIG_CAN_BECOME_KEY_WINDOW.load(std::sync::atomic::Ordering::SeqCst);
    if orig != 0 {
        let f: extern "C" fn(*mut std::ffi::c_void, *const std::ffi::c_void) -> u8 =
            unsafe { std::mem::transmute(orig) };
        return f(this, cmd);
    }
    0
}

/// True if `win` is a browser-pane overlay NSWindow (tagged by
/// SetPaneBoundsViewsTask with PANE_OVERLAY_TAG_KEY).
#[cfg(target_os = "macos")]
pub(crate) fn is_tagged_pane_overlay(win: *mut std::ffi::c_void) -> bool {
    extern "C" {
        fn objc_getAssociatedObject(
            obj: *mut std::ffi::c_void,
            key: *const std::ffi::c_void,
        ) -> *mut std::ffi::c_void;
    }
    if win.is_null() { return false; }
    let key = &PANE_OVERLAY_TAG_KEY as *const u8 as *const std::ffi::c_void;
    !unsafe { objc_getAssociatedObject(win, key) }.is_null()
}

// Tiny ObjC helpers for the key-window handoff below. UI thread only.
#[cfg(target_os = "macos")]
mod keywin_objc {
    use std::ffi::c_void;
    extern "C" {
        fn objc_msgSend();
        fn sel_registerName(n: *const i8) -> *const c_void;
        fn objc_getClass(name: *const i8) -> *mut c_void;
    }
    pub(super) unsafe fn get(obj: *mut c_void, sel: &[u8]) -> *mut c_void {
        if obj.is_null() { return std::ptr::null_mut(); }
        let f: extern "C" fn(*mut c_void, *const c_void) -> *mut c_void =
            std::mem::transmute(objc_msgSend as *const c_void);
        f(obj, sel_registerName(sel.as_ptr() as _))
    }
    pub(super) unsafe fn send0(obj: *mut c_void, sel: &[u8]) {
        if obj.is_null() { return; }
        let f: extern "C" fn(*mut c_void, *const c_void) =
            std::mem::transmute(objc_msgSend as *const c_void);
        f(obj, sel_registerName(sel.as_ptr() as _));
    }
    pub(super) unsafe fn ns_app() -> *mut c_void {
        get(objc_getClass(b"NSApplication\0".as_ptr() as _), b"sharedApplication\0")
    }
    pub(super) unsafe fn key_window() -> *mut c_void {
        get(ns_app(), b"keyWindow\0")
    }
    /// First `RenderWidgetHostViewCocoa` under `win`'s contentView (the page's
    /// input view), or null.
    pub(super) unsafe fn find_rwhvc(win: *mut c_void) -> *mut c_void {
        extern "C" {
            fn object_getClass(obj: *mut c_void) -> *mut c_void;
            fn object_getClassName(cls: *mut c_void) -> *const i8;
        }
        let count: extern "C" fn(*mut c_void, *const c_void) -> usize =
            std::mem::transmute(objc_msgSend as *const c_void);
        let at: extern "C" fn(*mut c_void, *const c_void, usize) -> *mut c_void =
            std::mem::transmute(objc_msgSend as *const c_void);
        let sel_count = sel_registerName(b"count\0".as_ptr() as _);
        let sel_at = sel_registerName(b"objectAtIndex:\0".as_ptr() as _);
        let mut stack = vec![get(win, b"contentView\0")];
        while let Some(view) = stack.pop() {
            if view.is_null() { continue; }
            let name = object_getClassName(object_getClass(view));
            if !name.is_null()
                && std::ffi::CStr::from_ptr(name).to_bytes().windows(25).any(|w| w == b"RenderWidgetHostViewCocoa")
            {
                return view;
            }
            let subs = get(view, b"subviews\0");
            if !subs.is_null() {
                for i in 0..count(subs, sel_count) { stack.push(at(subs, sel_at, i)); }
            }
        }
        std::ptr::null_mut()
    }
    pub(super) unsafe fn make_first_responder(win: *mut c_void, view: *mut c_void) -> bool {
        let f: extern "C" fn(*mut c_void, *const c_void, *mut c_void) -> i8 =
            std::mem::transmute(objc_msgSend as *const c_void);
        f(win, sel_registerName(b"makeFirstResponder:\0".as_ptr() as _), view) != 0
    }
}

/// True if the current key window is a browser-pane overlay (the page holds the
/// keyboard). UI thread only.
#[cfg(target_os = "macos")]
pub(crate) fn key_window_is_pane_overlay() -> bool {
    is_tagged_pane_overlay(unsafe { keywin_objc::key_window() })
}

/// Only the pane overlay that is currently key may keep its page's
/// RenderWidgetHostViewCocoa as first responder. Any other overlay is reset
/// to its own window. Otherwise Chromium's CommandDispatcher, which offers an
/// unhandled key re-sent to the main window to child windows via
/// performKeyEquivalent:, hands it to that background page. The page reports
/// it unhandled, it is re-sent again, and it loops forever (seen live: a key
/// typed into the app after Cmd+L spun between the main window and another
/// pane). UI thread only.
#[cfg(target_os = "macos")]
pub(crate) fn release_inactive_pane_responders() {
    let overlays: Vec<usize> = PANE_OVERLAY_WIN_TO_BLOCK
        .lock()
        .map(|m| m.keys().copied().collect())
        .unwrap_or_default();
    if overlays.is_empty() { return; }
    unsafe {
        extern "C" {
            fn object_getClass(obj: *mut std::ffi::c_void) -> *mut std::ffi::c_void;
            fn object_getClassName(cls: *mut std::ffi::c_void) -> *const i8;
        }
        let key = keywin_objc::key_window() as usize;
        for w in overlays {
            if w == key { continue; }
            let win = w as *mut std::ffi::c_void;
            let fr = keywin_objc::get(win, b"firstResponder\0");
            if fr.is_null() || fr == win { continue; }
            let name = object_getClassName(object_getClass(fr));
            if !name.is_null()
                && std::ffi::CStr::from_ptr(name).to_bytes().windows(25).any(|x| x == b"RenderWidgetHostViewCocoa")
            {
                keywin_objc::make_first_responder(win, std::ptr::null_mut());
            }
        }
    }
}

/// Make `win` (a main/app NSWindow) the key window if a pane overlay currently
/// holds key status — the click-back-into-the-app half of the handoff. No-op
/// when the key window is already something other than a pane overlay, so it
/// never fights ordinary AppKit key changes. UI thread only.
#[cfg(target_os = "macos")]
pub(crate) fn reclaim_key_for_window(win: *mut std::ffi::c_void) {
    unsafe {
        let key = keywin_objc::key_window();
        if !is_tagged_pane_overlay(key) || key == win { return; }
        let target = if !win.is_null() {
            win
        } else {
            // Unknown target: the overlay's parent (the window it is docked
            // in), else the app's main window.
            let parent = keywin_objc::get(key, b"parentWindow\0");
            if !parent.is_null() { parent } else { keywin_objc::get(keywin_objc::ns_app(), b"mainWindow\0") }
        };
        if target.is_null() { return; }
        keywin_objc::send0(target, b"makeKeyWindow\0");
        release_inactive_pane_responders();
        tracing::info!(target = target as usize, "[pane-key] key handed back from pane overlay to app window");
    }
}

// A focus request for a pane whose overlay window isn't registered yet (the
// frontend's claim-on-create can beat SetPaneBoundsViewsTask). Applied by
// `apply_pending_pane_key` once that task has tagged + registered the overlay.
#[cfg(target_os = "macos")]
static PENDING_PANE_KEY: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);

/// Give the pane's overlay window key status so keystrokes reach the page.
/// Called on the UI thread for `browser_pane_focus` (pane click, keyboard pane
/// selection, open-with-focus). If the overlay isn't registered yet, the
/// request is parked and applied when it is. UI thread only.
#[cfg(target_os = "macos")]
pub(crate) fn make_pane_overlay_key(block_id: &str) {
    let entry = PANE_OVERLAY_WIN_TO_BLOCK
        .lock()
        .ok()
        .and_then(|m| {
            m.iter()
                .find(|(_, (_, b, _))| b == block_id)
                .map(|(w, _)| *w)
        });
    match entry {
        Some(w) => {
            if let Ok(mut p) = PENDING_PANE_KEY.lock() { *p = None; }
            let w = w as *mut std::ffi::c_void;
            unsafe {
                if keywin_objc::key_window() != w {
                    keywin_objc::send0(w, b"makeKeyWindow\0");
                }
                // Key status alone leaves the overlay window itself as first
                // responder (seen live), so keyDown never reaches Chromium.
                // Point it at the page's RenderWidgetHostViewCocoa (and the
                // sendEvent: swizzle re-points it per key event, since Views
                // resets it). Deliberately NOT Views request_focus() on the
                // pane's BrowserView: the overlay view belongs to the MAIN
                // widget's views hierarchy, so that made the main window's
                // FocusManager route the main window's own keystrokes into
                // the page (seen live: after Cmd+L, keys typed into the app
                // went to the page and looped through redispatch).
                let rwhvc = keywin_objc::find_rwhvc(w);
                let responder_ok = !rwhvc.is_null()
                    && (keywin_objc::get(w, b"firstResponder\0") == rwhvc
                        || keywin_objc::make_first_responder(w, rwhvc));
                let is_key = keywin_objc::key_window() == w;
                if !is_key {
                    // Registered but not shown yet (a focus request right after
                    // creation, seen live): makeKeyWindow is a silent no-op on a
                    // window that isn't on screen. Park it; the next
                    // SetPaneBoundsViewsTask pass applies it once shown.
                    if let Ok(mut p) = PENDING_PANE_KEY.lock() { *p = Some(block_id.to_string()); }
                }
                tracing::info!(
                    block_id, overlay = w as usize, is_key, responder_ok,
                    "[pane-key] made pane overlay key{}", if is_key { "" } else { " — not key yet, parked" }
                );
            }
        }
        None => {
            if let Ok(mut p) = PENDING_PANE_KEY.lock() { *p = Some(block_id.to_string()); }
            tracing::info!(block_id, "[pane-key] overlay not registered yet — key request parked");
        }
    }
}

/// Apply a parked `make_pane_overlay_key` request if it is for `block_id`.
/// Called by SetPaneBoundsViewsTask right after it registers the overlay.
#[cfg(target_os = "macos")]
pub(crate) fn apply_pending_pane_key(block_id: &str) {
    let pending = PENDING_PANE_KEY.lock().ok().map_or(false, |p| p.as_deref() == Some(block_id));
    if pending {
        make_pane_overlay_key(block_id);
    }
}

/// Drop a parked key request for a pane that is going away / was defocused.
#[cfg(target_os = "macos")]
pub(crate) fn clear_pending_pane_key() {
    if let Ok(mut p) = PENDING_PANE_KEY.lock() { *p = None; }
}

// Identities (pointer + timestamp) of the most recent key events dispatched
// to a pane overlay, to recognise Chromium re-sending the same NSEvent (the
// matching keyUp can land in between, hence a few slots). The timestamp
// guards against a freed event's address being reused by a new one.
#[cfg(target_os = "macos")]
static RECENT_PANE_KEY_EVENTS: std::sync::Mutex<([(usize, u64); 8], usize)> =
    std::sync::Mutex::new(([(0, 0); 8], 0));

/// True if `event` is a key event recently dispatched to a pane overlay coming
/// round again (a Chromium unhandled-key re-send); otherwise records it.
/// UI thread only.
#[cfg(target_os = "macos")]
unsafe fn is_pane_key_redispatch(event: *mut std::ffi::c_void) -> bool {
    use std::ffi::c_void;
    extern "C" { fn objc_msgSend(); fn sel_registerName(n: *const i8) -> *const c_void; }
    let ts: extern "C" fn(*mut c_void, *const c_void) -> f64 =
        std::mem::transmute(objc_msgSend as *const c_void);
    let id = (event as usize, ts(event, sel_registerName(b"timestamp\0".as_ptr() as _)).to_bits());
    let Ok(mut recent) = RECENT_PANE_KEY_EVENTS.lock() else { return false };
    if recent.0.contains(&id) {
        return true;
    }
    let slot = recent.1;
    recent.0[slot] = id;
    recent.1 = (slot + 1) % recent.0.len();
    false
}

#[cfg(target_os = "macos")]
mod key_tasks {
    use cef::*;
    wrap_task! {
        pub struct PaneMakeKeyTask { block_id: String }
        impl Task {
            fn execute(&self) { super::make_pane_overlay_key(&self.block_id); }
        }
    }
}

/// `make_pane_overlay_key` from any thread (IPC handlers run off the UI thread).
#[cfg(target_os = "macos")]
pub(crate) fn post_make_pane_overlay_key(block_id: &str) {
    let mut task = key_tasks::PaneMakeKeyTask::new(block_id.to_string());
    cef::post_task(cef::ThreadId::UI, Some(&mut task));
}

/// Swizzled `RenderWidgetHostViewCocoa::shouldIgnoreMouseEvent:` — always returns NO.
///
/// Chromium's implementation returns YES when `[self.window isMainWindow]` and
/// `[self.window isKeyWindow]` are both NO, causing mouseDown: to silently drop
/// the event. The main CefNSWindow is neither key nor main while the overlay holds
/// focus. Returning NO unconditionally lets the main RWHVC forward clicks to the
/// main browser renderer even while the overlay is frontmost.
#[cfg(target_os = "macos")]
pub(crate) extern "C" fn swizzled_should_ignore_mouse_event(
    _this: *mut std::ffi::c_void,
    _cmd: *const std::ffi::c_void,
    _event: *mut std::ffi::c_void,
) -> u8 {
    0
}

// Storage for NSApp::sendEvent: original IMP — diagnostic only.
#[cfg(target_os = "macos")]
pub(crate) static ORIG_NSAPP_SEND_EVENT: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(0);

// RWHVC walk cache: the last NSWindow* we walked (RWHVC_CACHE_WIN) and the
// RenderWidgetHostViewCocoa* we found (MAIN_RWHVC_PTR).  Reused for drag/up/
// right-click events so we don't re-walk the subview tree on every event.
// NOT used for routing decisions (PANE_WIN_TO_HOST is the gate).
// Cleared per-window by clear_pane_swizzle_statics.
// Raw pointer: safe because RWHVC lives for the browser lifetime.
#[cfg(target_os = "macos")]
static RWHVC_CACHE_WIN: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
#[cfg(target_os = "macos")]
static MAIN_RWHVC_PTR:  std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// Called by `detach_browser_pane_view` each time any pane closes.
///
/// Keyed by *pane* label (unique per overlay), not window label, so two panes
/// that share a window each hold an independent entry.  The PANE_WIN_TO_HOST
/// entry for the window is removed only when no pane labels remain mapped to
/// that window pointer; PANE_LOCAL_W is cleared only when the host map empties.
#[cfg(target_os = "macos")]
pub(crate) fn clear_pane_swizzle_statics(pane_label: &str) {
    use std::sync::atomic::Ordering::Relaxed;
    let win_ptr = PANE_LABEL_TO_WIN.try_lock().ok()
        .and_then(|mut m| m.remove(pane_label));
    if let Some(ptr) = win_ptr {
        // Only evict the host entry if no other pane on the same window remains.
        let still_live = PANE_LABEL_TO_WIN.try_lock()
            .map_or(true, |m| m.values().any(|&w| w == ptr));
        if !still_live {
            if let Ok(mut m) = PANE_WIN_TO_HOST.try_lock() {
                m.remove(&ptr);
                if m.is_empty() {
                    PANE_LOCAL_W.store(0, Relaxed);
                }
            }
            // Evict RWHVC cache if it was for this window.
            if RWHVC_CACHE_WIN.load(Relaxed) == ptr {
                RWHVC_CACHE_WIN.store(0, Relaxed);
                MAIN_RWHVC_PTR.store(0, Relaxed);
            }
        }
    }
    // PANE_OVERLAY_WIN_TO_BLOCK is keyed by the pane's OWN overlay window
    // pointer (unrelated to win_ptr/PANE_WIN_TO_HOST above, which key by the
    // main window) — always independently evict this pane's entry by label.
    if let Ok(mut m) = PANE_OVERLAY_WIN_TO_BLOCK.try_lock() {
        m.retain(|_, (label, _, _)| label != pane_label);
    }
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn clear_pane_swizzle_statics(_pane_label: &str) {}

/// Swizzled NSApplication::sendEvent: — for mouse events on the main window
/// while the browser pane is open, bypasses NSWindow.sendEvent: and dispatches
/// directly to the main RenderWidgetHostViewCocoa after restoring CEF focus.
///
/// Why direct dispatch:
///   When the pane overlay is frontmost the main window loses key status.
///   NSWindow.sendEvent: has "activate-only" semantics for non-key windows:
///   it activates the window but may not forward the event to the hit view.
///   Directly calling [rwhvc mouseDown:/rightMouseDown:/etc. event] bypasses
///   this.
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
pub(crate) extern "C" fn swizzled_nsapp_send_event(
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
                    // Only intercept events for windows that have an open pane.
                    // PANE_WIN_TO_HOST maps win_ptr → CefBrowserHost for each such
                    // window; a missing entry means this window owns no pane and
                    // its events must fall through to the original sendEvent:.
                    let win_usize = win as usize;
                    let in_pane_map = crate::ui_tasks::PANE_WIN_TO_HOST
                        .try_lock()
                        .map_or(false, |m| m.contains_key(&win_usize));
                    if !in_pane_map {
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
                    let cached_win  = RWHVC_CACHE_WIN.load(std::sync::atomic::Ordering::Relaxed);
                    let cached_rwhvc = MAIN_RWHVC_PTR.load(std::sync::atomic::Ordering::Relaxed);

                    let rwhvc: Id = if ev_type == 1 || cached_win != win_usize || cached_rwhvc == 0 {
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
                            RWHVC_CACHE_WIN.store(win_usize, std::sync::atomic::Ordering::Relaxed);
                            MAIN_RWHVC_PTR.store(found as usize, std::sync::atomic::Ordering::Relaxed);
                        }
                        found
                    } else {
                        cached_rwhvc as Id
                    };

                    if !rwhvc.is_null() {
                        // A click on the app's own UI takes the keyboard back
                        // from a pane that holds it (make_pane_overlay_key), so
                        // typing follows the click. Only on button-down; a
                        // no-op unless a pane overlay is currently key.
                        if ev_type == 1 || ev_type == 3 {
                            reclaim_key_for_window(win);
                        }
                        // Restore main browser focus so Blink doesn't drop the
                        // event (CEF calls set_focus(0) on the main browser
                        // whenever the overlay gains focus).
                        if let Ok(mut m) = crate::ui_tasks::PANE_WIN_TO_HOST.try_lock() {
                            if let Some(h) = m.get_mut(&win_usize) {
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
                } else if ev_type == 1 {
                    // A click landed directly on a pane's own overlay window —
                    // the pane-body click that (unlike every in-DOM pane) never
                    // produces a DOM click for blockframe.tsx's selection
                    // handler to bubble-catch. Tap it here and emit
                    // `browser-pane-clicked` so the frontend selects/borders
                    // the pane exactly as the header-click path already does
                    // (browser-model.ts → refocusNode). This does NOT return —
                    // dispatch continues via the original sendEvent: call
                    // below/at function end, unchanged from before this tap
                    // existed.
                    let win_usize = win as usize;
                    let hit = PANE_OVERLAY_WIN_TO_BLOCK
                        .try_lock()
                        .ok()
                        .and_then(|m| m.get(&win_usize).cloned());
                    if let Some((_, block_id, weak_state)) = hit {
                        // The user clicked into the page: give its overlay
                        // key status BEFORE the click is dispatched, so the
                        // keystrokes that follow reach the page instead of
                        // the main window (and the click itself is handled as
                        // a key-window click).
                        make_pane_overlay_key(&block_id);
                        if let Some(state) = weak_state.upgrade() {
                            // Route to the pane's ACTUAL owning window, not
                            // "main" — see the identical fix + rationale in
                            // browser_pane::hwnd's WM_LBUTTONDOWN handler
                            // (reagentx P1 on PR #2597).
                            match state.browser_pane_window_label(&block_id) {
                                Some(window_label) => {
                                    crate::events::emit_event_to_window(
                                        &state,
                                        &window_label,
                                        "browser-pane-clicked",
                                        &serde_json::json!({ "block_id": block_id }),
                                    );
                                }
                                None => {
                                    tracing::warn!(
                                        "[pane-swizzle] leftMouseDown — no owning window label for block_id={}",
                                        block_id
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    // Keyboard events for a pane overlay (10=keyDown 11=keyUp 12=flagsChanged).
    unsafe {
        extern "C" { fn objc_msgSend(); fn sel_registerName(n: *const i8) -> *const c_void; }
        let get_usize: extern "C" fn(*mut c_void, *const c_void) -> usize =
            std::mem::transmute(objc_msgSend as *const c_void);
        let ev_type = get_usize(event, sel_registerName(b"type\0".as_ptr() as _));
        if matches!(ev_type, 10 | 11 | 12) {
            let win = keywin_objc::get(event, b"window\0");
            release_inactive_pane_responders();
            if is_tagged_pane_overlay(win) {
                // A key the page didn't handle comes back here from Chromium
                // (UnhandledKeyboardEventHandler → -[CommandDispatcher
                // redispatchKeyEvent:] → [NSApp sendEvent:], seen live) as the
                // SAME NSEvent. In the pane overlay that re-send is routed to
                // the page again, reported unhandled again, and re-sent
                // forever — a 100% CPU loop on any key a page ignores. The
                // re-send exists to give menu key equivalents a chance, so do
                // exactly that and stop; never route it back into the pane.
                if ev_type != 12 && is_pane_key_redispatch(event) {
                    if ev_type == 10 {
                        let menu = keywin_objc::get(keywin_objc::ns_app(), b"mainMenu\0");
                        if !menu.is_null() {
                            let pke: extern "C" fn(*mut c_void, *const c_void, *mut c_void) -> i8 =
                                std::mem::transmute(objc_msgSend as *const c_void);
                            pke(menu, sel_registerName(b"performKeyEquivalent:\0".as_ptr() as _), event);
                        }
                    }
                    return;
                }
                // Make sure the page's RenderWidgetHostViewCocoa is the first
                // responder before AppKit routes the event. Chromium Views
                // resets the overlay's first responder to the window itself
                // (seen live, right after make_pane_overlay_key), which
                // swallows every keystroke — so re-point it per event.
                let rwhvc = keywin_objc::find_rwhvc(win);
                if !rwhvc.is_null() && keywin_objc::get(win, b"firstResponder\0") != rwhvc {
                    keywin_objc::make_first_responder(win, rwhvc);
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
