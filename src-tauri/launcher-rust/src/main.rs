// AgentMux Launcher - Shows splash immediately and launches main executable
// Minimal Rust binary (~500KB vs 19MB main app)

#![windows_subsystem = "windows"]

use std::path::PathBuf;
use std::process::{Child, Command};
use std::thread;
use std::time::{Duration, Instant};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::*;
use windows::Win32::Graphics::Gdi::*;
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::*;

const SPLASH_WIDTH: i32 = 400;
const SPLASH_HEIGHT: i32 = 400;
const MAIN_EXE_NAME: &str = "agentmux-main.exe";
const SPLASH_SIGNAL_FILE: &str = "agentmux_ready.signal";
const TIMEOUT_MS: u64 = 10000;

static mut SPLASH_HWND: Option<HWND> = None;

fn main() {
    unsafe {
        // Show splash immediately
        if let Err(e) = show_splash() {
            show_error(&format!("Failed to show splash: {}", e));
            return;
        }

        // Launch main executable
        let mut child = match launch_main_executable() {
            Ok(c) => c,
            Err(e) => {
                show_error(&format!("Failed to launch main executable: {}", e));
                close_splash();
                return;
            }
        };

        // Wait for main to signal ready or timeout
        wait_for_main_ready(&mut child);

        // Close splash
        close_splash();
    }
}

fn show_splash() -> Result<(), Box<dyn std::error::Error>> {
    unsafe {
        let hinstance = GetModuleHandleW(None)?;

        // Get path to splash.bmp
        let mut exe_path = std::env::current_exe()?;
        exe_path.pop();
        let bmp_path = exe_path.join("splash.bmp");

        // Load bitmap from file
        let bmp_path_wide: Vec<u16> = bmp_path
            .to_str()
            .ok_or("Invalid path")?
            .encode_utf16()
            .chain(Some(0))
            .collect();

        let hbitmap = LoadImageW(
            None,
            PCWSTR(bmp_path_wide.as_ptr()),
            IMAGE_BITMAP,
            0,
            0,
            LR_LOADFROMFILE,
        )?;

        // Get screen dimensions
        let screen_width = GetSystemMetrics(SM_CXSCREEN);
        let screen_height = GetSystemMetrics(SM_CYSCREEN);
        let x = (screen_width - SPLASH_WIDTH) / 2;
        let y = (screen_height - SPLASH_HEIGHT) / 2;

        // Create splash window
        let hwnd = CreateWindowExW(
            WS_EX_TOPMOST | WS_EX_TOOLWINDOW,
            w!("STATIC"),
            w!("AgentMux Loading"),
            WS_POPUP | WINDOW_STYLE(0x0000000E), // SS_BITMAP
            x,
            y,
            SPLASH_WIDTH,
            SPLASH_HEIGHT,
            None,
            None,
            hinstance,
            None,
        )?;

        SPLASH_HWND = Some(hwnd);

        // Set bitmap
        SendMessageW(hwnd, STM_SETIMAGE, WPARAM(0), LPARAM(hbitmap.0 as isize));

        // Show window
        ShowWindow(hwnd, SW_SHOW);
        let _ = UpdateWindow(hwnd);

        Ok(())
    }
}

fn close_splash() {
    unsafe {
        if let Some(hwnd) = SPLASH_HWND {
            DestroyWindow(hwnd).ok();
            SPLASH_HWND = None;
        }
    }
}

fn launch_main_executable() -> Result<Child, Box<dyn std::error::Error>> {
    let mut exe_path = std::env::current_exe()?;
    exe_path.pop();
    let main_exe = exe_path.join(MAIN_EXE_NAME);

    if !main_exe.exists() {
        return Err(format!("Main executable not found: {:?}", main_exe).into());
    }

    let child = Command::new(&main_exe).spawn()?;

    Ok(child)
}

fn wait_for_main_ready(child: &mut Child) {
    let start_time = Instant::now();

    loop {
        // Process Windows messages
        unsafe {
            let mut msg = MSG::default();
            while PeekMessageW(&mut msg, None, 0, 0, PM_REMOVE).as_bool() {
                TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
        }

        // Check if main is ready (via named mutex)
        if check_main_ready() {
            break;
        }

        // Check timeout
        if start_time.elapsed().as_millis() > TIMEOUT_MS as u128 {
            break;
        }

        // Check if main process crashed
        match child.try_wait() {
            Ok(Some(status)) => {
                show_error(&format!("AgentMux crashed during startup (exit code: {:?})", status.code()));
                break;
            }
            _ => {}
        }

        thread::sleep(Duration::from_millis(50));
    }
}

fn check_main_ready() -> bool {
    use std::env;
    use std::path::PathBuf;

    let temp_dir = env::temp_dir();
    let signal_file = temp_dir.join(SPLASH_SIGNAL_FILE);

    if signal_file.exists() {
        // Clean up signal file
        let _ = std::fs::remove_file(&signal_file);
        true
    } else {
        false
    }
}

fn show_error(message: &str) {
    unsafe {
        let title: Vec<u16> = "AgentMux Launcher Error"
            .encode_utf16()
            .chain(Some(0))
            .collect();
        let msg: Vec<u16> = message.encode_utf16().chain(Some(0)).collect();

        MessageBoxW(
            None,
            PCWSTR(msg.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_ICONERROR,
        );
    }
}
