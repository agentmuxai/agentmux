// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Windows-only app-owned wrapper HWND for embedded browser panes.
//!
//! **Why this exists**: `docs/specs/SPEC_BROWSER_PANE_WINDOWS_TEARDOWN_SPIKE_2026_07_03.md`.
//! On Windows, an embedded browser pane's outer HWND was previously CEF's
//! own internally-managed browser window, `set_as_child` directly onto
//! main's top-level (`browser_pane/creation.rs`). Destroying (or
//! `close_browser`-ing) that HWND either failed to release the renderer
//! reliably, or — for `close_browser` — cascaded into tearing down main
//! too (CEF/Alloy conflates a `close_browser` call with the close of any
//! ancestor sharing that WS_CHILD lineage; see the spec's §2 for three
//! prior attempts that hit this).
//!
//! This module inserts one thin, app-owned HWND between main and CEF's
//! browser — mirroring the pattern `floating_pane.rs` already uses
//! successfully for torn-off panes (an app-owned window that embeds CEF as
//! its own `WS_CHILD`). Destroying OUR wrapper via `DestroyWindow` never
//! calls any CEF API; Win32's native `WM_DESTROY` parent→child cascade
//! tears down CEF's child HWND as a side effect, which — per the floater's
//! already-proven behavior — reliably fires CEF's `OnBeforeClose`.
//!
//! Unlike the floater (an unowned top-level `WS_POPUP`), this wrapper is a
//! genuine `WS_CHILD` of whatever window currently hosts the pane (usually
//! main) — it sits exactly where the pane visually lives, not floating
//! independently. Only `WM_SIZE` (resize the sole CEF child to fill) and
//! `WM_DESTROY` (drop our tracking entry) are handled; none of the
//! floater's resize-border hit-testing, no-owner cascade-hook, or drag
//! semantics apply here.

#![cfg(target_os = "windows")]

use std::collections::HashMap;
use std::sync::Mutex;

use cef::Rect;

/// Wrapper HWND per pane label, so `close()`/`resize()` can find the
/// wrapper without threading it through every call site. Populated at
/// creation (`create_wrapper`), removed at `WM_DESTROY` (belt-and-suspenders
/// — the explicit close path also removes it after issuing `DestroyWindow`,
/// since `WM_DESTROY` fires synchronously inside that same call on Win32).
static PANE_WRAPPER_HWNDS: std::sync::LazyLock<Mutex<HashMap<String, isize>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

pub(crate) fn cache_wrapper_hwnd(label: &str, hwnd: isize) {
    PANE_WRAPPER_HWNDS.lock().unwrap().insert(label.to_string(), hwnd);
}

pub(crate) fn take_wrapper_hwnd(label: &str) -> Option<isize> {
    PANE_WRAPPER_HWNDS.lock().unwrap().remove(label)
}

/// Non-destructive lookup — for callers (e.g. `resize()`) that need the
/// wrapper HWND without removing it from the tracking map.
pub(crate) fn peek_wrapper_hwnd(label: &str) -> Option<isize> {
    PANE_WRAPPER_HWNDS.lock().unwrap().get(label).copied()
}

/// Window-class name for pane wrappers, suffixed with the launcher-supplied
/// `AGENTMUX_IPC_HASH` so two parallel AgentMux instances register distinct
/// class atoms (I5 invariant) — same pattern as `floating_pane::floater_class_name`.
fn wrapper_class_name() -> &'static str {
    static NAME: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    NAME.get_or_init(|| match std::env::var("AGENTMUX_IPC_HASH") {
        Ok(h) if !h.is_empty() => format!("AgentMuxPaneWrapper-{}", h),
        _ => "AgentMuxPaneWrapper".to_string(),
    })
}

unsafe extern "system" fn wrapper_wndproc(
    hwnd: *mut std::ffi::c_void,
    msg: u32,
    wparam: usize,
    lparam: isize,
) -> isize {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DefWindowProcW, GetClientRect, GetWindow, SetWindowPos, GW_CHILD, SWP_NOACTIVATE,
        SWP_NOZORDER, WM_DESTROY, WM_SIZE,
    };

    match msg {
        // Resize CEF's browser (our sole child) to fill the wrapper's new
        // client area. A pane wrapper has exactly one child — unlike the
        // floater, which has to walk to the bottom-most direct child to
        // skip its own frontend-header browser — so GW_CHILD (topmost, and
        // here also the only one) is sufficient.
        WM_SIZE => {
            let child = GetWindow(hwnd, GW_CHILD);
            if !child.is_null() {
                let mut rc = std::mem::zeroed::<windows_sys::Win32::Foundation::RECT>();
                if GetClientRect(hwnd, &mut rc) != 0 {
                    SetWindowPos(
                        child,
                        std::ptr::null_mut(),
                        0,
                        0,
                        rc.right - rc.left,
                        rc.bottom - rc.top,
                        SWP_NOZORDER | SWP_NOACTIVATE,
                    );
                }
            }
        }
        // Belt-and-suspenders removal from the tracking map — the explicit
        // close path (`destroy_wrapper`) already removes the entry before
        // calling DestroyWindow, but WM_DESTROY also fires on any other
        // teardown path (e.g. a future parent-window close cascading down),
        // so this keeps PANE_WRAPPER_HWNDS from ever holding a dead HWND.
        WM_DESTROY => {
            let dead = hwnd as isize;
            if let Ok(mut map) = PANE_WRAPPER_HWNDS.lock() {
                map.retain(|_, &mut h| h != dead);
            }
        }
        _ => {}
    }

    DefWindowProcW(hwnd, msg, wparam, lparam)
}

fn register_class_once() {
    use std::ffi::OsStr;
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        RegisterClassExW, CS_HREDRAW, CS_VREDRAW, WNDCLASSEXW,
    };

    static CLASS_REGISTERED: std::sync::Once = std::sync::Once::new();
    CLASS_REGISTERED.call_once(|| unsafe {
        let class_name = wrapper_class_name();
        let mut class_name_utf16: Vec<u16> = OsStr::new(class_name).encode_wide().collect();
        class_name_utf16.push(0);
        let h_instance = GetModuleHandleW(std::ptr::null());
        let wnd_class = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(wrapper_wndproc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: h_instance,
            hIcon: std::ptr::null_mut(),
            hCursor: std::ptr::null_mut(),
            // No background brush — the CEF child fills the client area
            // before any WM_ERASEBKGND would matter for a WS_CHILD (unlike
            // the floater's WS_POPUP, which is briefly visible on its own
            // before CEF paints); NULL avoids an extra GDI object.
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: class_name_utf16.as_ptr(),
            hIconSm: std::ptr::null_mut(),
        };
        let atom = RegisterClassExW(&wnd_class);
        if atom == 0 {
            tracing::error!(
                "[pane-wrapper] RegisterClassExW failed for '{}'; CreateWindowExW will fail",
                class_name,
            );
        }
    });
}

/// Create the wrapper HWND as a `WS_CHILD` of `parent_hwnd` at `rect`
/// (parent-relative coords — the same coordinate space `set_as_child` used
/// to take directly). Raises it to the top of `parent_hwnd`'s Z-order,
/// mirroring what `browser_pane/callbacks.rs::on_after_created_browser_pane`
/// already does for CEF's own HWND today (that code still runs too, now
/// operating one level down — harmless single-child no-op there).
///
/// Returns the wrapper HWND on success. Caches it into `PANE_WRAPPER_HWNDS`
/// under `label` internally before returning — callers must NOT also call
/// `cache_wrapper_hwnd` themselves, or the entry gets redundantly
/// overwritten with the same value.
pub(crate) fn create_wrapper(
    label: &str,
    parent_hwnd: *mut std::ffi::c_void,
    rect: &Rect,
) -> Result<*mut std::ffi::c_void, String> {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, SetWindowPos, ShowWindow, HWND_TOP, SWP_NOACTIVATE, SW_SHOWNOACTIVATE,
        WS_CHILD, WS_VISIBLE,
    };

    register_class_once();
    let class_name = wrapper_class_name();
    let mut class_name_utf16: Vec<u16> = {
        use std::ffi::OsStr;
        use std::os::windows::ffi::OsStrExt;
        OsStr::new(class_name).encode_wide().collect()
    };
    class_name_utf16.push(0);

    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class_name_utf16.as_ptr(),
            std::ptr::null(),
            WS_CHILD | WS_VISIBLE,
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            parent_hwnd,
            std::ptr::null_mut(),
            windows_sys::Win32::System::LibraryLoader::GetModuleHandleW(std::ptr::null()),
            std::ptr::null(),
        )
    };
    if hwnd.is_null() {
        return Err("CreateWindowExW returned null".to_string());
    }

    unsafe {
        // Top of parent's Z-order so the wrapper (and CEF's child inside
        // it) paints above main's own content widget — matches the
        // existing raise-to-top for CEF's HWND in
        // on_after_created_browser_pane, just one level up.
        SetWindowPos(
            hwnd,
            HWND_TOP,
            0, 0, 0, 0,
            0x0001 | 0x0002 | SWP_NOACTIVATE, // SWP_NOSIZE | SWP_NOMOVE | SWP_NOACTIVATE
        );
        ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    }

    cache_wrapper_hwnd(label, hwnd as isize);
    tracing::info!(label, hwnd = ?hwnd, "[pane-wrapper] created");
    Ok(hwnd)
}

/// Resize the wrapper to `rect` (parent-relative). The wrapper's own
/// `WM_SIZE` handler cascades the resize to CEF's child automatically —
/// callers no longer need to `SetWindowPos` CEF's HWND directly.
pub(crate) fn resize_wrapper(wrapper_hwnd: *mut std::ffi::c_void, rect: &Rect) {
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::SetWindowPos(
            wrapper_hwnd,
            std::ptr::null_mut(),
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            0x0010, // SWP_NOACTIVATE
        );
    }
}

/// Destroy the wrapper. Never calls any CEF API — this is a plain Win32
/// `DestroyWindow` on a window we created and own, which is exactly the
/// pattern `floating_pane.rs` already uses successfully: Win32's own
/// `WM_DESTROY` cascade tears down CEF's child HWND as a side effect,
/// which reliably fires CEF's `OnBeforeClose` (per the floater's proven
/// behavior — see this module's doc comment).
///
/// Pure Win32 side effects only — does NOT touch `PANE_WRAPPER_HWNDS`.
/// Callers that need the map cleaned up should call `take_wrapper_hwnd`
/// themselves (typically already have, to get `wrapper_hwnd` in the first
/// place); `wrapper_wndproc`'s own `WM_DESTROY` handler also clears any
/// surviving entry as a backstop.
pub(crate) fn destroy_wrapper_hwnd(wrapper_hwnd: *mut std::ffi::c_void) {
    use windows_sys::Win32::Graphics::Gdi::{InvalidateRect, UpdateWindow};
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        DestroyWindow, GetParent, ShowWindow, SW_HIDE,
    };

    unsafe {
        // Capture the parent BEFORE destroy — GetParent on a destroyed HWND
        // returns null. Same DWM-compositor-caching defense as the old
        // direct-CEF-HWND destroy path (browser_panes.rs::destroy_hwnd):
        // hide first so DWM stops compositing this surface before the
        // window (and its CEF child) actually goes away.
        let parent = GetParent(wrapper_hwnd);
        ShowWindow(wrapper_hwnd, SW_HIDE);
        DestroyWindow(wrapper_hwnd);
        if !parent.is_null() {
            InvalidateRect(parent, std::ptr::null(), 1 /* TRUE erase */);
            UpdateWindow(parent);
        }
    }
    tracing::info!(hwnd = ?wrapper_hwnd, "[pane-wrapper] destroyed");
}
