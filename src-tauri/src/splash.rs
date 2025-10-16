// Simple Windows splash screen using embedded BMP resource
// Shows instantly on app launch

#![cfg(windows)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::*;
use windows::Win32::UI::WindowsAndMessaging::*;

// SS_BITMAP constant (0x0000000E)
const SS_BITMAP: WINDOW_STYLE = WINDOW_STYLE(0x0000000E);

static mut SPLASH_HWND: Option<HWND> = None;

/// Show the splash screen in a separate thread
pub fn show() -> Arc<AtomicBool> {
    let should_close = Arc::new(AtomicBool::new(false));
    let should_close_clone = Arc::clone(&should_close);

    thread::spawn(move || {
        if let Err(e) = show_splash_window(should_close_clone) {
            eprintln!("Failed to show splash screen: {}", e);
        }
    });

    should_close
}

/// Close the splash screen
pub fn close(should_close: Arc<AtomicBool>) {
    should_close.store(true, Ordering::Relaxed);

    unsafe {
        if let Some(hwnd) = SPLASH_HWND {
            let _ = PostMessageW(hwnd, WM_CLOSE, WPARAM(0), LPARAM(0));
        }
    }
}

fn show_splash_window(should_close: Arc<AtomicBool>) -> Result<(), Box<dyn std::error::Error>> {
    unsafe {
        let hinstance = GetModuleHandleW(None)?;

        // Load bitmap from embedded resource (ID = 1)
        let hbitmap = LoadBitmapW(hinstance, PCWSTR(1 as *const u16));
        if hbitmap.is_invalid() {
            return Err("Failed to load bitmap from resources".into());
        }

        // Get bitmap dimensions
        let mut bm = BITMAP::default();
        GetObjectW(
            hbitmap,
            std::mem::size_of::<BITMAP>() as i32,
            Some(&mut bm as *mut _ as *mut _),
        );

        let width = bm.bmWidth;
        let height = bm.bmHeight;

        // Get screen dimensions for centering
        let screen_width = GetSystemMetrics(SM_CXSCREEN);
        let screen_height = GetSystemMetrics(SM_CYSCREEN);
        let x = (screen_width - width) / 2;
        let y = (screen_height - height) / 2;

        // Create static window with bitmap
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            w!("STATIC"),
            w!("AgentMux Loading"),
            WS_POPUP | SS_BITMAP,
            x,
            y,
            width,
            height,
            None,
            None,
            hinstance,
            None,
        )?;

        SPLASH_HWND = Some(hwnd);

        // Set the bitmap on the static control (IMAGE_BITMAP = 0)
        SendMessageW(hwnd, STM_SETIMAGE, WPARAM(0), LPARAM(hbitmap.0 as isize));

        // Show window
        ShowWindow(hwnd, SW_SHOW);
        let _ = UpdateWindow(hwnd);

        // Message loop
        let mut msg = MSG::default();
        while !should_close.load(Ordering::Relaxed) {
            if PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                if msg.message == WM_QUIT || msg.message == WM_CLOSE {
                    break;
                }
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            thread::sleep(Duration::from_millis(10));
        }

        DestroyWindow(hwnd)?;
    }

    Ok(())
}
