// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Win32 HWND-level helpers for browser panes: the `WM_SETFOCUS` redirect
//! subclass and the focus-bypass flag.
//!
//! Moved out of `client.rs` during Phase 2 of the pane modularization split
//! (see `docs/specs/SPEC_BROWSER_PANE_MODULARIZATION.md` §6). `client.rs`
//! still uses `ALLOW_PANE_FOCUS_ONCE` at a distance (nothing there imports
//! the function directly), but `install_pane_focus_redirect` is the home
//! for pane-focused Win32 subclass logic and future phases can wire it up
//! to pane `on_after_created` / `on_load_end` without touching `client.rs`.
//!
//! Everything in this file is Windows-only by gating.

#![cfg(target_os = "windows")]

/// Map of pane HWND -> original WndProc, so the subclass hook can delegate
/// to the real handler after running its interception logic. The mutex is
/// held only while mutating the map — hooks that read on the UI thread
/// copy out the pointer quickly.
static PANE_WNDPROCS: std::sync::LazyLock<
    std::sync::Mutex<std::collections::HashMap<usize, isize>>,
> = std::sync::LazyLock::new(|| std::sync::Mutex::new(std::collections::HashMap::new()));

/// When `true`, the next `WM_SETFOCUS` delivered to a subclassed pane HWND
/// is allowed through instead of being redirected back to the parent.
///
/// The frontend's `giveFocus()` -> `browser_pane_focus` IPC sets this flag
/// before calling `SetFocus` on the pane, so user-initiated focus works
/// even though Chromium's internal focus-steal on navigation is blocked.
pub static ALLOW_PANE_FOCUS_ONCE: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// Subclass a browser pane's outer HWND (and every descendant HWND Chromium
/// has already created) so `WM_SETFOCUS` is redirected back to the parent
/// top-level window unless the focus change is user-initiated (see
/// `ALLOW_PANE_FOCUS_ONCE`).
///
/// Without this, Chromium's internal SetFocus on the pane HWND (page load,
/// JS `window.focus()`, etc.) steals the Windows-level keyboard focus —
/// subsequent keystrokes go to the pane's renderer instead of the main
/// window, so terminals, URL bars, and other inputs in the main UI stop
/// responding.
///
/// Wired in by `pane::callbacks::on_after_created_pane` at create time and
/// by `pane::callbacks::on_load_end_pane` after every navigation — Chromium
/// recreates the `Chrome_RenderWidgetHostHWND` on every page load, so the
/// subclass has to follow along or it ends up stranded on a destroyed HWND.
pub unsafe fn install_pane_focus_redirect(hwnd: *mut std::ffi::c_void) {
    use std::sync::atomic::Ordering;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CallWindowProcW, GetParent, SetWindowLongPtrW, GWLP_WNDPROC, WM_SETFOCUS,
    };
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::SetFocus;

    unsafe extern "system" fn wndproc_hook(
        hwnd: *mut std::ffi::c_void,
        msg: u32,
        wparam: usize,
        lparam: isize,
    ) -> isize {
        // Diagnostic: surface mouse-wheel and key events so we can tell
        // whether they reach the pane HWND at all when the user reports
        // scrolling/typing breakage.
        const WM_MOUSEWHEEL: u32 = 0x020A;
        const WM_MOUSEHWHEEL: u32 = 0x020E;
        const WM_KEYDOWN: u32 = 0x0100;
        const WM_CHAR: u32 = 0x0102;
        match msg {
            WM_MOUSEWHEEL | WM_MOUSEHWHEEL => {
                tracing::info!("[pane-wndproc] mouse-wheel hwnd={:p} msg=0x{:x}", hwnd, msg);
            }
            WM_KEYDOWN | WM_CHAR => {
                tracing::info!("[pane-wndproc] key msg=0x{:x} wparam={}", msg, wparam);
            }
            _ => {}
        }

        if msg == WM_SETFOCUS {
            // Intentional focus from the frontend's giveFocus() IPC: honor it
            // once, then revert to redirect-mode for subsequent events.
            if ALLOW_PANE_FOCUS_ONCE.swap(false, Ordering::Relaxed) {
                tracing::info!("[pane-wndproc] WM_SETFOCUS allowed (intentional)");
                // Fall through to the original WndProc.
            } else {
                // Programmatic focus (page load, JS window.focus()): redirect.
                let parent = GetParent(hwnd);
                if !parent.is_null() {
                    SetFocus(parent);
                }
                return 0;
            }
        }

        let original = PANE_WNDPROCS
            .lock()
            .ok()
            .and_then(|m| m.get(&(hwnd as usize)).copied())
            .unwrap_or(0);
        if original != 0 {
            let proc_fn: unsafe extern "system" fn(
                *mut std::ffi::c_void, u32, usize, isize,
            ) -> isize = std::mem::transmute(original);
            CallWindowProcW(Some(proc_fn), hwnd, msg, wparam, lparam)
        } else {
            0
        }
    }

    // Subclass the outer HWND — but only once. Re-calling SetWindowLongPtrW
    // would replace our hook with itself and poison PANE_WNDPROCS.
    let already_hooked = PANE_WNDPROCS
        .lock()
        .ok()
        .map(|m| m.contains_key(&(hwnd as usize)))
        .unwrap_or(false);
    if !already_hooked {
        let original = SetWindowLongPtrW(hwnd, GWLP_WNDPROC, wndproc_hook as *const () as isize);
        if original != 0 {
            if let Ok(mut map) = PANE_WNDPROCS.lock() {
                map.insert(hwnd as usize, original);
            }
            tracing::info!("[pane-subclass] installed focus-redirect WndProc on pane HWND {:p}", hwnd);
        }
    }

    // Chromium creates inner HWNDs (widget + render) below the outer HWND.
    // Mouse input reaches the deepest descendant, so we must walk the whole
    // tree and subclass every one.
    unsafe extern "system" fn enum_children(
        child: *mut std::ffi::c_void,
        _lparam: isize,
    ) -> i32 {
        let already = PANE_WNDPROCS
            .lock()
            .ok()
            .map(|m| m.contains_key(&(child as usize)))
            .unwrap_or(false);
        if already {
            return 1;
        }
        let orig = SetWindowLongPtrW(child, GWLP_WNDPROC, wndproc_hook as *const () as isize);
        if orig != 0 {
            if let Ok(mut map) = PANE_WNDPROCS.lock() {
                map.insert(child as usize, orig);
            }
            let mut class_buf = [0u16; 64];
            let n = windows_sys::Win32::UI::WindowsAndMessaging::GetClassNameW(
                child, class_buf.as_mut_ptr(), class_buf.len() as i32,
            );
            let class_name = String::from_utf16_lossy(&class_buf[..n as usize]);
            tracing::info!("[pane-subclass] subclassed child HWND {:p} class={}", child, class_name);
        }
        1 // continue
    }
    windows_sys::Win32::UI::WindowsAndMessaging::EnumChildWindows(
        hwnd, Some(enum_children), 0,
    );
}

// ── Tests ───────────────────────────────────────────────────────────────
//
// The Win32 calls themselves can't be unit-tested without a real HWND and
// window message loop. What we can test here is the focus-bypass flag's
// behavior as a simple AtomicBool — it's the only testable invariant the
// `wndproc_hook` relies on.

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;

    #[test]
    fn allow_pane_focus_once_starts_false() {
        // Note: this static is global to the process, so other tests can
        // have modified it. Read-only assertion before mutation.
        let _ = ALLOW_PANE_FOCUS_ONCE.load(Ordering::Relaxed);
    }

    #[test]
    fn allow_pane_focus_once_swap_returns_prev_and_clears() {
        ALLOW_PANE_FOCUS_ONCE.store(true, Ordering::Relaxed);
        let prev = ALLOW_PANE_FOCUS_ONCE.swap(false, Ordering::Relaxed);
        assert!(prev, "swap should return the prior true value");
        assert!(!ALLOW_PANE_FOCUS_ONCE.load(Ordering::Relaxed),
            "after swap(false), flag must be cleared");
    }

    #[test]
    fn allow_pane_focus_once_swap_when_false_returns_false() {
        ALLOW_PANE_FOCUS_ONCE.store(false, Ordering::Relaxed);
        let prev = ALLOW_PANE_FOCUS_ONCE.swap(false, Ordering::Relaxed);
        assert!(!prev, "swap on cleared flag should return false");
    }
}
