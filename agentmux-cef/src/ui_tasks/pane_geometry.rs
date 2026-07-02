// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Deferred pane-overlay bounds + show task (non-Windows / CEF Views panes),
// including the macOS ObjC frame-commit + swizzle-install path. Split out of
// `ui_tasks.rs` unchanged.

// The task in this file is entirely `#[cfg(not(target_os = "windows"))]`, so
// these imports are only needed off Windows.
#[cfg(not(target_os = "windows"))]
use std::sync::Arc;
#[cfg(not(target_os = "windows"))]
use cef::*;
#[cfg(not(target_os = "windows"))]
use crate::state::AppState;
// macOS-only: the ObjC block references this swizzle-storage static by bare
// name; the other swizzle statics/fns are referenced via `crate::ui_tasks::…`.
#[cfg(target_os = "macos")]
use super::ORIG_RWHVC_SHOULD_IGNORE;

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
                        tracing::debug!(
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

                // Cache the resolved overlay wnum so resize tasks can use an exact
                // wnum rather than the highest-wnum fallback (which is ambiguous when
                // ≥2 panes are open on the same window).
                let discovered_wnum = get_usize(overlay_win, sel_win_number) as isize;
                if let Some(mut wmap) = self.state.browser_pane_overlay_wnums.try_lock() {
                    wmap.insert(self.label.clone(), discovered_wnum);
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
                // Export window refs for post-set_bounds key restoration.
                task_main_win    = main_win;
                task_overlay_win = overlay_win;
                task_sel_make_key_front = sel_make_key_front;
                task_sel_order_front    = sel_order_front;
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
                        tracing::debug!(
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
                                tracing::debug!(
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
                                tracing::debug!(
                                    retry = self.retry,
                                    hit_view = hit_view as usize,
                                    "[browser-pane] RWHVC hit_view ptr"
                                );
                                extern "C" {
                                    fn objc_getClass(name: *const std::ffi::c_char) -> Id;
                                }
                                let rwhvc_cls = objc_getClass(b"RenderWidgetHostViewCocoa\0".as_ptr() as _);
                                if !rwhvc_cls.is_null() {
                                // Register this window in PANE_WIN_TO_HOST BEFORE setting
                                // PANE_LOCAL_W so swizzled_nsapp_send_event can route
                                // correctly from the very first click on any window.
                                // PANE_LABEL_TO_WIN allows clear_pane_swizzle_statics to
                                // remove the right entry when this specific pane closes.
                                if !task_main_win.is_null() {
                                    let win_ptr = task_main_win as usize;
                                    if let Some(host) = self.state.get_browser(&self.window_label)
                                        .and_then(|b| b.host())
                                    {
                                        if let Ok(mut m) = crate::ui_tasks::PANE_WIN_TO_HOST.try_lock() {
                                            m.insert(win_ptr, host);
                                        }
                                        if let Ok(mut lm) = crate::ui_tasks::PANE_LABEL_TO_WIN.try_lock() {
                                            lm.insert(self.label.clone(), win_ptr);
                                        }
                                    }
                                }
                                crate::ui_tasks::PANE_LOCAL_W.store(
                                    task_dip_w, std::sync::atomic::Ordering::SeqCst);
                                tracing::info!(
                                    retry = self.retry,
                                    pw = task_dip_w,
                                    main_win = task_main_win as usize,
                                    "[browser-pane] sendEvent swizzle activated"
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

            // Main browser host is now stored into PANE_WIN_TO_HOST inside the
            // rwhvc_cls block above, keyed by the NSWindow pointer for this pane.

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
                        // Restore focus to the main browser once, 200ms after pane creation.
                        // The sendEvent: swizzle calls set_focus(1) before every subsequent
                        // mouse dispatch, so a one-shot restore is sufficient.
                        let win_label = {
                            let map = self.state.browser_pane_overlays.lock();
                            map.get(&self.label).map(|(wl, _)| wl.clone())
                        };
                        if let Some(win_label) = win_label {
                            if let Some(mut main_browser) = self.state.get_browser(&win_label) {
                                if let Some(mut host) = main_browser.host() {
                                    host.set_focus(1);
                                    tracing::debug!(
                                        label = %self.label, win = %win_label,
                                        "[browser-pane] restored focus to main window after pane creation"
                                    );
                                }
                            }
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
