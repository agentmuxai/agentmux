// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! A test build's window on macOS (see `agentmux_common::test_offscreen`).

use cef::*;

/// A test build (AGENTMUX_TEST_OFFSCREEN) on macOS: AppKit pulls a titled window's frame back onto a
/// display when it is first shown, whatever bounds it was given, so the window is also made fully
/// transparent and deaf to the mouse before it is shown: nothing of it is seen or clicked wherever it
/// lands. UI thread only; any other build, and any other OS, does nothing.
pub(crate) fn veil_test_window(window: &Window) {
    #[cfg(target_os = "macos")]
    if agentmux_common::test_offscreen::active() {
        use std::ffi::{c_char, c_void};
        type Id = *mut c_void;
        type Sel = *const c_void;
        extern "C" {
            fn sel_registerName(name: *const c_char) -> Sel;
            fn objc_msgSend();
        }
        let nsview = window.window_handle() as Id;
        if nsview.is_null() {
            return;
        }
        unsafe {
            let get_window: extern "C" fn(Id, Sel) -> Id = std::mem::transmute(objc_msgSend as *const c_void);
            let nswindow = get_window(nsview, sel_registerName(b"window\0".as_ptr() as _));
            if nswindow.is_null() {
                return;
            }
            let set_alpha: extern "C" fn(Id, Sel, f64) = std::mem::transmute(objc_msgSend as *const c_void);
            set_alpha(nswindow, sel_registerName(b"setAlphaValue:\0".as_ptr() as _), 0.0);
            let set_flag: extern "C" fn(Id, Sel, u8) = std::mem::transmute(objc_msgSend as *const c_void);
            set_flag(nswindow, sel_registerName(b"setIgnoresMouseEvents:\0".as_ptr() as _), 1);
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = window;
}
