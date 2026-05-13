//! Native pre-splash for Windows: a borderless layered popup showing
//! the app icon while CefInitialize runs (200–600 ms cold start).
//!
//! `spawn_splash(dir_hash)` is called right after the single-instance
//! pipe is claimed — before srv spawn, before CEF init (~10 ms into
//! the launcher process). The returned event name is passed to the
//! CEF host as `AGENTMUX_SPLASH_EVENT`; the host signals it from
//! `on_load_end` to trigger a smooth fade-out.

#![cfg(target_os = "windows")]

use std::thread;
use windows_sys::Win32::Foundation::*;
use windows_sys::Win32::Graphics::Gdi::*;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Threading::*;
use windows_sys::Win32::UI::WindowsAndMessaging::*;

const SPLASH_SIZE: i32 = 200;
const ICON_SIZE: i32 = 80;
/// Dark background COLORREF (0x00BBGGRR)
const BG_COLOR: u32 = 0x001F1A1A;

// HANDLE is a raw pointer; wrap it to cross the thread boundary safely.
struct SendHandle(HANDLE);
unsafe impl Send for SendHandle {}

/// Spawn the pre-splash thread and return the named Win32 event name
/// to pass to the CEF host as `AGENTMUX_SPLASH_EVENT`.
/// Returns `None` if OS calls fail (non-fatal — launcher continues).
pub fn spawn_splash(dir_hash: &str) -> Option<String> {
    let event_name = format!("AgentMuxSplash-{}", dir_hash);
    let nul_name: Vec<u16> = format!("{}\0", event_name)
        .encode_utf16()
        .collect();

    let ev = unsafe {
        CreateEventW(
            std::ptr::null(), // default security
            1,                // manual-reset
            0,                // not signaled
            nul_name.as_ptr(),
        )
    };
    if ev.is_null() {
        crate::log("splash: CreateEventW failed — skipping splash");
        return None;
    }

    let handle = SendHandle(ev);
    thread::spawn(move || unsafe { run_splash(handle.0) });
    Some(event_name)
}

unsafe fn run_splash(dismiss_ev: HANDLE) {
    let class: Vec<u16> = "AgentMuxSplash\0".encode_utf16().collect();
    let hinst = GetModuleHandleW(std::ptr::null());

    // Load the icon embedded by winres in build.rs (resource ID 1).
    // Falls back to null (icon omitted) if not found — not fatal.
    let icon = LoadIconW(hinst, 1usize as *const u16);

    let wc = WNDCLASSEXW {
        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wndproc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: hinst,
        hIcon: icon,
        hCursor: std::ptr::null_mut(),
        hbrBackground: std::ptr::null_mut(), // painted in WM_PAINT
        lpszMenuName: std::ptr::null(),
        lpszClassName: class.as_ptr(),
        hIconSm: std::ptr::null_mut(),
    };
    // Silently tolerate ERROR_CLASS_ALREADY_EXISTS in dev hot-reload.
    RegisterClassExW(&wc);

    let sw = GetSystemMetrics(SM_CXSCREEN);
    let sh = GetSystemMetrics(SM_CYSCREEN);
    let x = (sw - SPLASH_SIZE) / 2;
    let y = (sh - SPLASH_SIZE) / 2;

    let hwnd = CreateWindowExW(
        WS_EX_LAYERED | WS_EX_TOPMOST | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
        class.as_ptr(),
        std::ptr::null(),     // no title
        WS_POPUP,
        x, y, SPLASH_SIZE, SPLASH_SIZE,
        std::ptr::null_mut(), // no parent
        std::ptr::null_mut(), // no menu
        hinst,
        std::ptr::null(),     // no CREATESTRUCT data
    );
    if hwnd.is_null() {
        CloseHandle(dismiss_ev);
        return;
    }

    // Store icon pointer in GWLP_USERDATA for WM_PAINT retrieval.
    SetWindowLongPtrW(hwnd, GWLP_USERDATA, icon as isize);

    // Start fully transparent, paint content, then animate into view.
    SetLayeredWindowAttributes(hwnd, 0, 0, LWA_ALPHA);
    ShowWindow(hwnd, SW_SHOWNOACTIVATE);
    UpdateWindow(hwnd);

    let start = std::time::Instant::now();

    loop {
        // Drain pending messages (handles WM_PAINT repaint requests).
        let mut msg: MSG = std::mem::zeroed();
        while PeekMessageW(&mut msg, std::ptr::null_mut(), 0, 0, PM_REMOVE) != 0 {
            if msg.message == WM_QUIT {
                DestroyWindow(hwnd);
                CloseHandle(dismiss_ev);
                return;
            }
            TranslateMessage(&msg);
            DispatchMessageW(&msg);
        }

        // Non-blocking dismiss check — fires when on_load_end signals.
        if WaitForSingleObject(dismiss_ev, 0) == WAIT_OBJECT_0 {
            fade_out(hwnd);
            break;
        }

        // Alpha envelope: 0→220 over first 200 ms, then sine-pulse 160..220.
        let t = start.elapsed().as_secs_f32();
        let alpha: u8 = if t < 0.2 {
            (t / 0.2 * 220.0) as u8
        } else {
            let pulse = (((t - 0.2) * std::f32::consts::TAU * 1.1).sin() + 1.0) * 0.5;
            (160.0 + pulse * 60.0) as u8
        };
        SetLayeredWindowAttributes(hwnd, 0, alpha, LWA_ALPHA);

        std::thread::sleep(std::time::Duration::from_millis(16)); // ~60 fps
    }

    DestroyWindow(hwnd);
    CloseHandle(dismiss_ev);
}

/// Fade the splash window to transparent over ~160 ms then return.
unsafe fn fade_out(hwnd: HWND) {
    let mut alpha: i32 = 220;
    while alpha > 0 {
        alpha -= 22;
        if alpha < 0 {
            alpha = 0;
        }
        SetLayeredWindowAttributes(hwnd, 0, alpha as u8, LWA_ALPHA);
        std::thread::sleep(std::time::Duration::from_millis(16));
    }
}

unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, w: WPARAM, l: LPARAM) -> LRESULT {
    if msg == WM_PAINT {
        let mut ps: PAINTSTRUCT = std::mem::zeroed();
        let hdc = BeginPaint(hwnd, &mut ps);

        let mut rc: RECT = std::mem::zeroed();
        GetClientRect(hwnd, &mut rc);

        // Fill with dark app background.
        let brush = CreateSolidBrush(BG_COLOR);
        FillRect(hdc, &rc, brush);
        DeleteObject(brush as _);

        // Draw the embedded app icon centered.
        let icon = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as HICON;
        if !icon.is_null() {
            let ix = (SPLASH_SIZE - ICON_SIZE) / 2;
            let iy = (SPLASH_SIZE - ICON_SIZE) / 2;
            DrawIconEx(
                hdc, ix, iy, icon,
                ICON_SIZE, ICON_SIZE,
                0,                      // not animated cursor
                std::ptr::null_mut(),   // no flicker-free brush
                DI_NORMAL,
            );
        }

        EndPaint(hwnd, &ps);
        return 0;
    }
    DefWindowProcW(hwnd, msg, w, l)
}
