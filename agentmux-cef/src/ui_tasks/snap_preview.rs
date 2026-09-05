// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Translucent snap-preview overlay for the drag-to-top maximize gesture —
//! `docs/specs/SPEC_WINDOW_SNAP_MAXIMIZE_2026_09_04.md` §2.2.
//!
//! A borderless, click-through, non-activating layered window that paints a
//! single translucent rectangle over the area the dragged window would
//! occupy if released. This is the same shape Windows' own Snap Assist
//! preview takes, and it has to be a separate OS window rather than
//! something the renderer draws: the preview covers the FULL work area
//! while the dragged window itself is small and following the cursor, so
//! there is no in-window surface that could render it.
//!
//! Lifecycle is owned entirely by the move loop in [`super::drag`] — show on
//! entering the snap zone, hide on leaving it, on Esc-cancel, and
//! unconditionally when the loop exits (belt-and-braces: an overlay that
//! outlives its drag is a stuck always-on-top rectangle over the user's
//! screen, so every exit path clears it).
//!
//! Windows-only: the gesture itself is Windows-only (see the spec's platform
//! scope), so there is no cross-platform abstraction to justify here.

#![cfg(target_os = "windows")]

use std::sync::Mutex;

use windows_sys::Win32::Foundation::{COLORREF, HWND};

/// The one preview window, created lazily on first use and then reused for
/// the rest of the process's life (hidden between drags rather than
/// destroyed — creating a window inside a modal drag loop on every zone
/// entry would be both slow and a needless failure point mid-gesture).
static PREVIEW_HWND: Mutex<usize> = Mutex::new(0);

/// Accent-ish blue at ~25% alpha. Deliberately a flat translucent fill with
/// no border: `SetLayeredWindowAttributes` gives whole-window alpha for free
/// via the class background brush, so this needs no `WM_PAINT` handler and
/// no GDI resources beyond the brush.
const PREVIEW_FILL: COLORREF = 0x00C0_7020; // BGR: a muted blue
const PREVIEW_ALPHA: u8 = 64; // 0..255

/// Show (creating if needed) the preview at the given PHYSICAL-pixel rect.
///
/// Best-effort: any failure to create or position the overlay is logged and
/// swallowed. The preview is a visual affordance — losing it must never
/// break the drag or the snap itself, both of which work fine without it.
pub(crate) fn show(x: i32, y: i32, width: i32, height: i32) {
    unsafe {
        let hwnd = match ensure_window() {
            Some(h) => h,
            None => return,
        };
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            SetWindowPos, HWND_TOPMOST, SWP_NOACTIVATE, SWP_SHOWWINDOW,
        };
        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            x,
            y,
            width,
            height,
            // NOACTIVATE is essential: activating the overlay mid-drag would
            // steal focus from the window being dragged and can drop the
            // capture the move loop depends on.
            SWP_NOACTIVATE | SWP_SHOWWINDOW,
        );
    }
}

/// Hide the preview. Safe to call when it was never shown or never created.
pub(crate) fn hide() {
    unsafe {
        let hwnd = *PREVIEW_HWND.lock().unwrap_or_else(|e| e.into_inner()) as HWND;
        if hwnd.is_null() {
            return;
        }
        use windows_sys::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
        ShowWindow(hwnd, SW_HIDE);
    }
}

/// Create the overlay window on first use; return the cached HWND after.
unsafe fn ensure_window() -> Option<HWND> {
    use windows_sys::Win32::Graphics::Gdi::CreateSolidBrush;
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, IsWindow, RegisterClassW, SetLayeredWindowAttributes,
        LWA_ALPHA, WNDCLASSW, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
        WS_EX_TOPMOST, WS_EX_TRANSPARENT, WS_POPUP,
    };

    let mut guard = PREVIEW_HWND.lock().unwrap_or_else(|e| e.into_inner());
    let existing = *guard as HWND;
    if !existing.is_null() && IsWindow(existing) != 0 {
        return Some(existing);
    }

    // UTF-16, NUL-terminated — Win32 string convention.
    let class_name: Vec<u16> = "AgentMuxSnapPreview\0".encode_utf16().collect();
    let hinstance = GetModuleHandleW(std::ptr::null());

    // RegisterClassW is idempotent-by-failure here: a second call with the
    // same name fails harmlessly (ERROR_CLASS_ALREADY_EXISTS) and
    // CreateWindowExW below still succeeds against the first registration.
    // Only reachable a second time if the window was destroyed externally.
    let wc = WNDCLASSW {
        style: 0,
        lpfnWndProc: Some(DefWindowProcW),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinstance,
        hIcon: std::ptr::null_mut(),
        hCursor: std::ptr::null_mut(),
        hbrBackground: CreateSolidBrush(PREVIEW_FILL),
        lpszMenuName: std::ptr::null(),
        lpszClassName: class_name.as_ptr(),
    };
    RegisterClassW(&wc);

    let hwnd = CreateWindowExW(
        // LAYERED   — enables the whole-window alpha below.
        // TRANSPARENT + NOACTIVATE — click-through and never takes focus, so
        //             the overlay cannot intercept the drag's own mouse
        //             input or steal activation mid-gesture.
        // TOOLWINDOW — keeps it out of the taskbar and Alt-Tab.
        // TOPMOST    — must paint above the dragged window itself.
        WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_NOACTIVATE | WS_EX_TOOLWINDOW | WS_EX_TOPMOST,
        class_name.as_ptr(),
        std::ptr::null(),
        WS_POPUP,
        0,
        0,
        0,
        0,
        std::ptr::null_mut(),
        std::ptr::null_mut(),
        hinstance,
        std::ptr::null(),
    );
    if hwnd.is_null() {
        tracing::warn!("[snap-preview] CreateWindowExW failed — snap will work without a preview");
        return None;
    }
    SetLayeredWindowAttributes(hwnd, 0, PREVIEW_ALPHA, LWA_ALPHA);
    *guard = hwnd as usize;
    tracing::info!("[snap-preview] created overlay window {:p}", hwnd);
    Some(hwnd)
}
