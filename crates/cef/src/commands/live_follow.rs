// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// The picture that follows the cursor while a window tab is dragged out of its strip
// (docs/reports/REPORT_LIVE_TEAROFF_CHROME_STYLE_2026_11_04.md §5, "preview-follow"). The drag is the
// untouched HTML5/OLE drag and nothing is torn off until release: a click-through, window-sized picture
// of the source window is moved by the mouse hook while the cursor is below the strip, and on release
// the real window opens at its last place (`landing`). The picture has its own thread and message loop
// so the hook only calls `SetWindowPos` with `SWP_ASYNCWINDOWPOS` (Windows removes a slow hook). It
// never touches the window pool, so a cancelled drag leaves nothing to restore.

use std::sync::Arc;

use crate::state::AppState;

/// Where the picture's top-left goes for a cursor at `cursor`: the window is held at a fixed offset from
/// the cursor, the one the real tear-off would use, so the cursor stays on the tab that was grabbed.
#[cfg(any(target_os = "windows", test))]
pub(crate) fn follow_origin(cursor: (i32, i32), offset: (i32, i32)) -> (i32, i32) {
    (cursor.0 - offset.0, cursor.1 - offset.1)
}

/// Whether the picture is shown for a cursor at `cursor_y`. A window tab's shows below the tear line (the
/// strip's bottom edge plus the tear threshold, the same rule `decideTabRelease` uses) and not over another
/// AgentMux window, where a release merges the tab. A pane has no tear line: its picture is the dragged pane
/// itself, shown from the drag's first move wherever the cursor is, over the layout making room for it as much
/// as over the desktop, so no translucent ghost ever stands in for it (the live reflow spec's "what the user sees").
#[cfg(any(target_os = "windows", test))]
pub(crate) fn picture_wanted(cursor_y: i32, tear_line_y: Option<i32>, over_other_window: bool) -> bool {
    match tear_line_y {
        Some(line) => cursor_y > line && !over_other_window,
        None => true,
    }
}

#[cfg(target_os = "windows")]
pub use win::{begin, end, landing, on_cursor, on_escape, on_session_end, picture_shown};

#[cfg(not(target_os = "windows"))]
pub fn begin(_state: &Arc<AppState>, _args: &serde_json::Value) -> Result<serde_json::Value, String> {
    Ok(serde_json::Value::Null)
}

#[cfg(not(target_os = "windows"))]
pub fn end() -> Result<serde_json::Value, String> {
    Ok(serde_json::Value::Null)
}

/// `begin` on the blocking pool: it decodes the picture and waits for the picture's thread to be ready.
pub async fn begin_blocking(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let (state, args) = (state.clone(), args.clone());
    tokio::task::spawn_blocking(move || begin(&state, &args))
        .await
        .map_err(|e| format!("live_follow_begin join error: {}", e))?
}

#[cfg(target_os = "windows")]
mod win {
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};

    use windows_sys::Win32::Foundation::{GetLastError, ERROR_CLASS_ALREADY_EXISTS, HWND, LPARAM, LRESULT, POINT, SIZE, WPARAM};
    use windows_sys::Win32::Graphics::Gdi::{
        ClientToScreen, CreateCompatibleDC, CreateDIBSection, DeleteDC, DeleteObject, GetDC, ReleaseDC, SelectObject,
        BITMAPINFO, BITMAPINFOHEADER, BLENDFUNCTION,
    };
    use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
    use windows_sys::Win32::UI::HiDpi::GetDpiForWindow;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetMessageW, PostMessageW, PostQuitMessage,
        RegisterClassExW, SetTimer, SetWindowPos, TranslateMessage, UpdateLayeredWindow, HWND_TOP, MSG, SWP_ASYNCWINDOWPOS,
        SWP_HIDEWINDOW, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SWP_SHOWWINDOW, ULW_ALPHA, WM_APP,
        WM_ERASEBKGND, WM_DESTROY, WM_MOUSEACTIVATE, WM_NCHITTEST, WM_TIMER, WNDCLASSEXW, WS_EX_LAYERED, WS_EX_NOACTIVATE,
        WS_EX_TOOLWINDOW, WS_EX_TRANSPARENT, WS_POPUP,
    };

    use crate::state::AppState;

    const CLASS_NAME: &str = "AgentMuxLiveFollow";
    /// Sent to the picture's window: destroy it now.
    const WM_FOLLOW_END: u32 = WM_APP + 0x61;
    /// Sent to the picture's window when the drag is over: destroy it shortly, if nobody has asked sooner.
    const WM_FOLLOW_GRACE: u32 = WM_APP + 0x62;
    const TIMER_ID: usize = 1;
    /// The longest a picture lives if nothing ends it (a drag that never reports its end).
    const FAILSAFE_MS: u32 = 120_000;
    /// How long a picture stays after the drag ends: the real window is promoted at its place meanwhile.
    const GRACE_MS: u32 = 1_500;
    /// The window sizes taken from the page, in physical pixels.
    const MIN_DIM: i32 = 100;
    const MAX_DIM: i32 = 8_000;

    struct Follower {
        hwnd: usize,
        size: (i32, i32),
        /// Cursor to the window's top-left, physical pixels.
        offset: (i32, i32),
        /// Screen y below which a window tab's picture is shown; a pane has none and shows everywhere.
        tear_line_y: Option<i32>,
        shown: bool,
        /// Where the picture was last put (clamped to the work area).
        last: (i32, i32),
        /// `landing` has handed `last` to a promote.
        landed: bool,
    }

    static FOLLOWER: Mutex<Option<Follower>> = Mutex::new(None);
    /// Whether the picture is on screen, readable without the lock the hook thread holds on every move.
    static PICTURE_SHOWN: AtomicBool = AtomicBool::new(false);

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    fn num(args: &serde_json::Value, key: &str) -> Result<f64, String> {
        args.get(key).and_then(|v| v.as_f64()).ok_or_else(|| format!("missing {}", key))
    }

    /// The picture as BGRA at `w` x `h` (scaled if the capture is another size), opaque.
    fn decode_bgra(jpeg: &[u8], w: u32, h: u32) -> Result<Vec<u8>, String> {
        let decoded = image::load_from_memory_with_format(jpeg, image::ImageFormat::Jpeg)
            .map_err(|e| format!("picture does not decode: {}", e))?
            .into_rgba8();
        let rgba = if decoded.width() == w && decoded.height() == h {
            decoded
        } else {
            image::imageops::resize(&decoded, w, h, image::imageops::FilterType::Triangle)
        };
        let mut bgra = rgba.into_raw();
        for px in bgra.chunks_exact_mut(4) {
            px.swap(0, 2);
            px[3] = 255;
        }
        Ok(bgra)
    }

    /// Put the picture of the source window under the cursor's drag. `args`: `sourceWindowLabel`, `picture`
    /// (base64 JPEG), `windowWidth` and `windowHeight` (the window's outer size, DIP), `offsetX` and `offsetY`
    /// (cursor to the window's top-left, DIP), `tearLineY` (the strip's bottom plus the tear threshold,
    /// viewport DIP). A window tab's picture stays hidden until the cursor passes the tear line; a pane's (no
    /// `tearLineY`) shows at the cursor's next move.
    pub fn begin(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
        use base64::Engine;
        let label = args
            .get("sourceWindowLabel")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "missing sourceWindowLabel".to_string())?;
        let picture = args.get("picture").and_then(|v| v.as_str()).ok_or_else(|| "missing picture".to_string())?;
        let jpeg = base64::engine::general_purpose::STANDARD
            .decode(picture)
            .map_err(|e| format!("picture is not base64: {}", e))?;
        let source = unsafe { crate::commands::window::resolve_window_hwnd_strict(state, label) }
            .ok_or_else(|| format!("no window for {}", label))?;
        let (scale, client_top) = unsafe {
            let dpi = GetDpiForWindow(source);
            let mut origin = POINT { x: 0, y: 0 };
            ClientToScreen(source, &mut origin);
            (if dpi == 0 { 1.0 } else { dpi as f64 / 96.0 }, origin.y)
        };
        let px = |dip: f64| (dip * scale).round() as i32;
        let size = (
            px(num(args, "windowWidth")?).clamp(MIN_DIM, MAX_DIM),
            px(num(args, "windowHeight")?).clamp(MIN_DIM, MAX_DIM),
        );
        let offset = (px(num(args, "offsetX")?), px(num(args, "offsetY")?));
        let tear_line_y = args.get("tearLineY").and_then(|v| v.as_f64()).map(|line| client_top + px(line));
        let bgra = decode_bgra(&jpeg, size.0 as u32, size.1 as u32)?;

        end_now();
        let hwnd = spawn_overlay(bgra, size)?;
        *FOLLOWER.lock().map_err(|_| "follower state poisoned".to_string())? = Some(Follower {
            hwnd,
            size,
            offset,
            tear_line_y,
            shown: false,
            last: (0, 0),
            landed: false,
        });
        tracing::info!(
            target: "dnd:tearoff",
            label = %label, width = size.0, height = size.1, tear_line_y = ?tear_line_y,
            "[live-follow] picture ready; it shows with the cursor's next move"
        );
        // The drag ended while the picture was being made: nothing will move it, so it goes.
        if !crate::commands::tear_off_hook::hook_session_active() {
            on_session_end();
        }
        Ok(serde_json::Value::Null)
    }

    /// Drop the picture now (the page's drag is over and its real window is up, or the drag was cancelled).
    pub fn end() -> Result<serde_json::Value, String> {
        end_now();
        Ok(serde_json::Value::Null)
    }

    fn end_now() {
        let taken = FOLLOWER.lock().ok().and_then(|mut g| g.take());
        PICTURE_SHOWN.store(false, Ordering::Release);
        if let Some(f) = taken {
            unsafe { PostMessageW(f.hwnd as HWND, WM_FOLLOW_END, 0, 0) };
        }
    }

    /// Hook thread, on every mouse move of a window-tab drag: show, move or hide the picture. Never
    /// blocks: a contended lock skips the move, and the window calls are asynchronous.
    pub fn on_cursor(x: i32, y: i32, over_other_window: bool) {
        let Ok(mut guard) = FOLLOWER.try_lock() else { return };
        let Some(f) = guard.as_mut() else { return };
        let hwnd = f.hwnd as HWND;
        if !super::picture_wanted(y, f.tear_line_y, over_other_window) {
            if f.shown {
                f.shown = false;
                PICTURE_SHOWN.store(false, Ordering::Release);
                unsafe {
                    SetWindowPos(
                        hwnd,
                        std::ptr::null_mut(),
                        0,
                        0,
                        0,
                        0,
                        SWP_HIDEWINDOW | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS,
                    );
                }
            }
            return;
        }
        let (px, py) = place((x, y), f.offset, f.size);
        f.last = (px, py);
        let (insert_after, flags) = if f.shown {
            (std::ptr::null_mut(), SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS)
        } else {
            (HWND_TOP, SWP_SHOWWINDOW | SWP_NOSIZE | SWP_NOACTIVATE | SWP_ASYNCWINDOWPOS)
        };
        f.shown = true;
        PICTURE_SHOWN.store(true, Ordering::Release);
        unsafe { SetWindowPos(hwnd, insert_after, px, py, 0, 0, flags) };
    }

    /// Where the picture goes for the cursor: at its offset, pulled inside the work area of the monitor it
    /// lands on, as the window promote does, so the real window opens exactly here.
    fn place(cursor: (i32, i32), offset: (i32, i32), size: (i32, i32)) -> (i32, i32) {
        let (ox, oy) = super::follow_origin(cursor, offset);
        match crate::app::get_monitor_work_area_physical(ox, oy) {
            Some((wx, wy, ww, wh)) => {
                let (x, y, _, _) = crate::commands::window_pool::clamp_rect_within(ox, oy, size.0, size.1, wx, wy, ww, wh);
                (x, y)
            }
            None => (ox, oy),
        }
    }

    /// Whether the window picture is on screen now. Asked from the drag's own thread on every cursor move, while
    /// the hook thread holds the follower's lock on every move too, so it reads a flag instead: a contended lock
    /// would answer no and bring the shell's picture back for a frame.
    pub fn picture_shown() -> bool {
        PICTURE_SHOWN.load(Ordering::Acquire)
    }

    /// Where the real window should open: the picture's last place, if the picture is on screen. Handed out
    /// once, so a later tear-off can't reuse it.
    pub fn landing() -> Option<(i32, i32)> {
        let mut guard = FOLLOWER.lock().ok()?;
        let f = guard.as_mut().filter(|f| f.shown && !f.landed)?;
        f.landed = true;
        Some(f.last)
    }

    /// Escape: the tear-off is cancelled, so the picture goes at once.
    pub fn on_escape() {
        end_now();
    }

    /// The drag's hook session is over (release, Escape, or stopped). A hidden picture goes now; a shown one
    /// stays for its grace time, under the window about to be promoted at its place.
    pub fn on_session_end() {
        let state = FOLLOWER.lock().ok().and_then(|g| g.as_ref().map(|f| (f.hwnd, f.shown)));
        match state {
            Some((hwnd, true)) => {
                unsafe { PostMessageW(hwnd as HWND, WM_FOLLOW_GRACE, 0, 0) };
            }
            Some((_, false)) => end_now(),
            None => {}
        }
    }

    fn spawn_overlay(bgra: Vec<u8>, size: (i32, i32)) -> Result<usize, String> {
        let (tx, rx) = std::sync::mpsc::channel::<Result<usize, String>>();
        std::thread::Builder::new()
            .name("live-follow".to_string())
            .spawn(move || unsafe {
                let hwnd = match create_overlay(&bgra, size) {
                    Ok(h) => h,
                    Err(e) => {
                        let _ = tx.send(Err(e));
                        return;
                    }
                };
                drop(bgra);
                let _ = tx.send(Ok(hwnd as usize));
                let mut msg: MSG = std::mem::zeroed();
                while GetMessageW(&mut msg, std::ptr::null_mut(), 0, 0) > 0 {
                    TranslateMessage(&msg);
                    DispatchMessageW(&msg);
                }
                if let Ok(mut g) = FOLLOWER.lock() {
                    if g.as_ref().is_some_and(|f| f.hwnd == hwnd as usize) {
                        *g = None;
                        PICTURE_SHOWN.store(false, Ordering::Release);
                    }
                }
            })
            .map_err(|e| format!("failed to spawn the picture thread: {}", e))?;
        rx.recv().map_err(|e| format!("picture thread ended early: {}", e))?
    }

    /// A hidden, click-through, never-activated popup holding the picture. Click-through (layered and
    /// transparent) keeps it out of hit-testing, so the OS drag and the hook still see the window beneath.
    unsafe fn create_overlay(bgra: &[u8], size: (i32, i32)) -> Result<HWND, String> {
        let (w, h) = size;
        let class = wide(CLASS_NAME);
        let hinst = GetModuleHandleW(std::ptr::null());
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            style: 0,
            lpfnWndProc: Some(overlay_proc),
            cbClsExtra: 0,
            cbWndExtra: 0,
            hInstance: hinst,
            hIcon: std::ptr::null_mut(),
            hCursor: std::ptr::null_mut(),
            hbrBackground: std::ptr::null_mut(),
            lpszMenuName: std::ptr::null(),
            lpszClassName: class.as_ptr(),
            hIconSm: std::ptr::null_mut(),
        };
        if RegisterClassExW(&wc) == 0 && GetLastError() != ERROR_CLASS_ALREADY_EXISTS {
            return Err(format!("RegisterClassExW failed: {}", GetLastError()));
        }
        let hwnd = CreateWindowExW(
            WS_EX_LAYERED | WS_EX_TRANSPARENT | WS_EX_TOOLWINDOW | WS_EX_NOACTIVATE,
            class.as_ptr(),
            class.as_ptr(),
            WS_POPUP,
            0,
            0,
            w,
            h,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            hinst,
            std::ptr::null(),
        );
        if hwnd.is_null() {
            return Err(format!("CreateWindowExW failed: {}", GetLastError()));
        }

        let screen = GetDC(std::ptr::null_mut());
        let mem = CreateCompatibleDC(screen);
        let mut bmi: BITMAPINFO = std::mem::zeroed();
        bmi.bmiHeader = BITMAPINFOHEADER {
            biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
            biWidth: w,
            biHeight: -h,
            biPlanes: 1,
            biBitCount: 32,
            biCompression: 0,
            biSizeImage: 0,
            biXPelsPerMeter: 0,
            biYPelsPerMeter: 0,
            biClrUsed: 0,
            biClrImportant: 0,
        };
        let mut bits: *mut std::ffi::c_void = std::ptr::null_mut();
        let dib = CreateDIBSection(screen, &bmi, 0, &mut bits, std::ptr::null_mut(), 0);
        let mut result = Err("CreateDIBSection failed".to_string());
        if !dib.is_null() && !bits.is_null() {
            std::ptr::copy_nonoverlapping(bgra.as_ptr(), bits as *mut u8, bgra.len().min((w * h * 4) as usize));
            let old = SelectObject(mem, dib as _);
            let dest_size = SIZE { cx: w, cy: h };
            let src = POINT { x: 0, y: 0 };
            let blend = BLENDFUNCTION { BlendOp: 0, BlendFlags: 0, SourceConstantAlpha: 255, AlphaFormat: 0 };
            let ok = UpdateLayeredWindow(hwnd, screen, std::ptr::null(), &dest_size, mem, &src, 0, &blend, ULW_ALPHA);
            SelectObject(mem, old);
            result = if ok != 0 { Ok(()) } else { Err(format!("UpdateLayeredWindow failed: {}", GetLastError())) };
        }
        if !dib.is_null() {
            DeleteObject(dib as _);
        }
        DeleteDC(mem);
        ReleaseDC(std::ptr::null_mut(), screen);
        if let Err(e) = result {
            DestroyWindow(hwnd);
            return Err(e);
        }
        SetTimer(hwnd, TIMER_ID, FAILSAFE_MS, None);
        Ok(hwnd)
    }

    unsafe extern "system" fn overlay_proc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        match msg {
            // -1 is HTTRANSPARENT: the point belongs to whatever is beneath.
            WM_NCHITTEST => -1,
            // MA_NOACTIVATE.
            WM_MOUSEACTIVATE => 3,
            WM_ERASEBKGND => 1,
            WM_FOLLOW_END | WM_TIMER => {
                DestroyWindow(hwnd);
                0
            }
            WM_FOLLOW_GRACE => {
                SetTimer(hwnd, TIMER_ID, GRACE_MS, None);
                0
            }
            WM_DESTROY => {
                PostQuitMessage(0);
                0
            }
            _ => DefWindowProcW(hwnd, msg, wp, lp),
        }
    }

    #[cfg(test)]
    mod tests {
        use super::decode_bgra;

        fn jpeg(w: u32, h: u32, rgb: [u8; 3]) -> Vec<u8> {
            let mut bytes = Vec::new();
            image::RgbImage::from_pixel(w, h, image::Rgb(rgb))
                .write_to(&mut std::io::Cursor::new(&mut bytes), image::ImageFormat::Jpeg)
                .unwrap();
            bytes
        }

        #[test]
        fn a_picture_of_the_size_asked_comes_back_as_opaque_bgra() {
            let out = decode_bgra(&jpeg(16, 8, [200, 30, 30]), 16, 8).unwrap();
            assert_eq!(out.len(), 16 * 8 * 4);
            let (b, g, r, a) = (out[0], out[1], out[2], out[3]);
            assert!(r > 150 && g < 80 && b < 80, "red in BGR order, got {:?}", (b, g, r));
            assert_eq!(a, 255);
        }

        #[test]
        fn a_picture_of_another_size_is_scaled_to_the_window() {
            let out = decode_bgra(&jpeg(8, 8, [10, 10, 10]), 32, 20).unwrap();
            assert_eq!(out.len(), 32 * 20 * 4);
            assert!(out.chunks_exact(4).all(|px| px[3] == 255));
        }

        #[test]
        fn bytes_that_are_not_a_picture_are_refused() {
            assert!(decode_bgra(b"not a jpeg", 4, 4).is_err());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{follow_origin, picture_wanted};

    #[test]
    fn the_picture_keeps_its_offset_from_the_cursor() {
        assert_eq!(follow_origin((900, 500), (120, 30)), (780, 470));
        assert_eq!(follow_origin((10, 10), (120, 30)), (-110, -20));
    }

    #[test]
    fn a_window_tabs_picture_shows_only_below_the_tear_line() {
        assert!(picture_wanted(301, Some(300), false));
        assert!(!picture_wanted(300, Some(300), false));
        assert!(!picture_wanted(120, Some(300), false));
    }

    #[test]
    fn a_window_tabs_picture_stays_hidden_over_another_window() {
        assert!(!picture_wanted(900, Some(300), true));
    }

    #[test]
    fn a_panes_picture_shows_everywhere_from_the_first_move() {
        assert!(picture_wanted(0, None, false));
        assert!(picture_wanted(-5_000, None, false));
        // Over its own window, where the layout makes room, and over another AgentMux window too.
        assert!(picture_wanted(0, None, true));
    }
}
