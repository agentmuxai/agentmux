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
//
// A pane's picture is also the pane in hand while it is dragged over its own layout. There it takes the
// shape of the rect the armed drop would give the pane (`resize`), and, once the pane stands in that rect on
// screen, its picture is taken again there (`capture`): a pane laid out for the rect, where the first
// picture resized to another shape is a squashed one (the operator, 2026-11-12: "we have the shape but not
// the aspect ratio"). The pixels themselves are live_follow_picture.rs.

use std::sync::Arc;

use crate::state::AppState;

/// Where the picture's top-left goes for a cursor at `cursor`: the window is held at a fixed offset from
/// the cursor, the one the real tear-off would use, so the cursor stays on the tab that was grabbed.
#[cfg(any(target_os = "windows", test))]
pub(crate) fn follow_origin(cursor: (i32, i32), offset: (i32, i32)) -> (i32, i32) {
    (cursor.0 - offset.0, cursor.1 - offset.1)
}

/// The picture's top-left for a cursor, with the whole picture kept inside `area` (a monitor's work area:
/// left, top, width, height): at its offset from the cursor until an edge of the area stops it, as a window
/// stops at the edge of the screen. The cursor itself goes on.
#[cfg(any(target_os = "windows", test))]
pub(crate) fn place_within(cursor: (i32, i32), offset: (i32, i32), size: (i32, i32), area: (i32, i32, i32, i32)) -> (i32, i32) {
    let (ox, oy) = follow_origin(cursor, offset);
    let (x, y, _, _) =
        crate::commands::window_pool::clamp_rect_within(ox, oy, size.0, size.1, area.0, area.1, area.2, area.3);
    (x, y)
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
pub use win::{begin, capture, end, landing, on_cursor, on_escape, on_session_end, picture_shown, resize};

#[cfg(not(target_os = "windows"))]
pub fn begin(_state: &Arc<AppState>, _args: &serde_json::Value) -> Result<serde_json::Value, String> {
    Ok(serde_json::Value::Null)
}

#[cfg(not(target_os = "windows"))]
pub fn end() -> Result<serde_json::Value, String> {
    Ok(serde_json::Value::Null)
}

#[cfg(not(target_os = "windows"))]
pub fn resize(_args: &serde_json::Value) -> Result<serde_json::Value, String> {
    Ok(serde_json::Value::Null)
}

#[cfg(not(target_os = "windows"))]
pub async fn capture(_state: &Arc<AppState>, _args: &serde_json::Value) -> Result<serde_json::Value, String> {
    Ok(serde_json::json!({ "staged": false }))
}

/// The picture's commands by name: `live_follow_begin`, `live_follow_resize`, `live_follow_capture`, and
/// `live_follow_end` for any other.
pub async fn route(cmd: &str, state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    match cmd {
        "live_follow_begin" => begin_blocking(state, args).await,
        "live_follow_resize" => resize_blocking(args).await,
        "live_follow_capture" => capture(state, args).await,
        _ => end(),
    }
}

/// `begin` on the blocking pool: it decodes the picture and waits for the picture's thread to be ready.
pub async fn begin_blocking(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let (state, args) = (state.clone(), args.clone());
    tokio::task::spawn_blocking(move || begin(&state, &args))
        .await
        .map_err(|e| format!("live_follow_begin join error: {}", e))?
}

/// `resize` on the blocking pool: it composes the picture at its new size, a few megabytes at most.
pub async fn resize_blocking(args: &serde_json::Value) -> Result<serde_json::Value, String> {
    let args = args.clone();
    tokio::task::spawn_blocking(move || resize(&args))
        .await
        .map_err(|e| format!("live_follow_resize join error: {}", e))?
}

/// Where the cursor holds a picture of `size`, made from one of `source_size` that was pressed at `grab`: the
/// same spot of the pane, so the grab scales with the picture, and is kept inside it. Always from the original
/// grab, never from a previous size's offset, so the pane's own size has the original alignment back.
#[cfg(any(target_os = "windows", test))]
pub(crate) fn offset_within(grab: (i32, i32), source_size: (i32, i32), size: (i32, i32)) -> (i32, i32) {
    let scaled = |g: i32, from: i32, to: i32| {
        let at = if from > 0 && from != to { (g as i64 * to as i64 / from as i64) as i32 } else { g };
        at.min(to - 1).max(0)
    };
    (scaled(grab.0, source_size.0, size.0), scaled(grab.1, source_size.1, size.1))
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

    use crate::commands::live_follow_picture::{compose, crop_bgra, crop_rect, Landed, LandedPictures, Wash};
    use crate::state::AppState;

    const CLASS_NAME: &str = "AgentMuxLiveFollow";
    /// Sent to the picture's window: destroy it now.
    const WM_FOLLOW_END: u32 = WM_APP + 0x61;
    /// Sent to the picture's window when the drag is over: destroy it shortly, if nobody has asked sooner.
    const WM_FOLLOW_GRACE: u32 = WM_APP + 0x62;
    /// Sent to the picture's window: paint the picture waiting in `PENDING_REPAINT`, at its size.
    const WM_FOLLOW_RESIZE: u32 = WM_APP + 0x63;
    const TIMER_ID: usize = 1;
    /// The longest a picture lives if nothing ends it (a drag that never reports its end).
    const FAILSAFE_MS: u32 = 120_000;
    /// How long a picture stays after the drag ends: the real window is promoted at its place meanwhile.
    const GRACE_MS: u32 = 1_500;
    /// The window sizes taken from the page, in physical pixels.
    const MIN_DIM: i32 = 100;
    const MAX_DIM: i32 = 8_000;
    /// The JPEG quality a pane's window is pictured at for `capture`.
    const CAPTURE_QUALITY: u8 = 80;

    struct Follower {
        hwnd: usize,
        /// The source window's label: a pane's picture is taken again of it (`capture`).
        label: String,
        size: (i32, i32),
        /// Cursor to the window's top-left, physical pixels: `grab` scaled to the current size (`offset_within`).
        offset: (i32, i32),
        /// The grab the pane was pressed with, physical pixels, kept as is across resizes.
        grab: (i32, i32),
        /// DIP to the source window's physical pixels.
        scale: f64,
        /// The picture as first shown, BGRA at `source_size`: another size is resized from it (`compose`).
        source: Arc<Vec<u8>>,
        source_size: (i32, i32),
        /// Screen y below which a window tab's picture is shown; a pane has none and shows everywhere.
        tear_line_y: Option<i32>,
        /// A pane's picture stays on the monitor its window is on: that monitor's work area, physical pixels
        /// (left, top, width, height). Carried onto a second monitor, the whole picture jumped across the
        /// moment its corner passed the edge (the operator, 2026-11-12: "simply not allow the pane to make
        /// out of the monitor agentmux is running on, so it behaves like a typical edge of the screen").
        /// A window tab has none: it is torn off onto whichever monitor it is carried to.
        home: Option<(i32, i32, i32, i32)>,
        /// Where the hook last saw the cursor, screen pixels: a new size is placed around it (`resize`).
        cursor: Option<(i32, i32)>,
        shown: bool,
        /// Where the picture was last put (clamped to the work area).
        last: (i32, i32),
        /// `landing` has handed `last` to a promote.
        landed: bool,
        /// The landing rect whose own picture is on show, or none while it is the first picture resized.
        shows: Option<String>,
        /// The pictures taken of the pane in its landing rects (`capture`): one is staged until the page says
        /// the pane stood still for it (`resize` with `adopt`), then kept for the drag.
        pictures: LandedPictures,
    }

    static FOLLOWER: Mutex<Option<Follower>> = Mutex::new(None);
    /// Whether the picture is on screen, readable without the lock the hook thread holds on every move.
    static PICTURE_SHOWN: AtomicBool = AtomicBool::new(false);
    /// The picture `resize` composed, for the window's thread to paint (`WM_FOLLOW_RESIZE`); a newer one
    /// replaces an unpainted one, and the end of the picture clears it.
    static PENDING_REPAINT: Mutex<Option<Repaint>> = Mutex::new(None);

    struct Repaint {
        bgra: Arc<Vec<u8>>,
        size: (i32, i32),
        /// Where the picture goes at this size, when it is on screen: put there in the same call that paints
        /// it, so it never shows at its new size in its old place.
        at: Option<(i32, i32)>,
    }

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
        let home = if tear_line_y.is_none() { crate::app::window_work_area_physical(source as isize) } else { None };
        let bgra = Arc::new(decode_bgra(&jpeg, size.0 as u32, size.1 as u32)?);

        end_now();
        let hwnd = spawn_overlay(bgra.clone(), size)?;
        *FOLLOWER.lock().map_err(|_| "follower state poisoned".to_string())? = Some(Follower {
            hwnd,
            label: label.to_string(),
            size,
            offset: super::offset_within(offset, size, size),
            grab: offset,
            scale,
            source: bgra,
            source_size: size,
            tear_line_y,
            home,
            cursor: None,
            shown: false,
            last: (0, 0),
            landed: false,
            shows: None,
            pictures: LandedPictures::default(),
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
        if let Ok(mut pending) = PENDING_REPAINT.lock() {
            *pending = None;
        }
        if let Some(f) = taken {
            unsafe { PostMessageW(f.hwnd as HWND, WM_FOLLOW_END, 0, 0) };
        }
    }

    /// The size the page asks for (`windowWidth`, `windowHeight`, DIP), in the picture's physical pixels.
    fn asked_size(args: &serde_json::Value, scale: f64) -> Result<(i32, i32), String> {
        let px = |dip: f64| (dip * scale).round() as i32;
        Ok((
            px(num(args, "windowWidth")?).clamp(MIN_DIM, MAX_DIM),
            px(num(args, "windowHeight")?).clamp(MIN_DIM, MAX_DIM),
        ))
    }

    /// Give the picture another size (`windowWidth`, `windowHeight`, DIP): the shape of the rect the drop
    /// would give the pane while one is armed (`landingKey` names the rect), or the pane's own size again
    /// (pane-live-follow.ts). With a picture kept for that rect (`capture`; `adopt` keeps the one just
    /// taken), it is that picture: the pane as it is laid out there. Without one, the first picture is
    /// resized to the size, squashed where the shape differs, until the rect's own is taken. The cursor keeps
    /// its place on the pane: the picture takes its new size around the cursor, where it last was. A drop
    /// arms while the pointer rests, so no move would bring a picture that changed size around its own corner
    /// back under the pointer. Composed outside the follower's lock, which the hook thread takes on every move.
    pub fn resize(args: &serde_json::Value) -> Result<serde_json::Value, String> {
        let key = args.get("landingKey").and_then(|v| v.as_str());
        let adopt = args.get("adopt").and_then(|v| v.as_bool()).unwrap_or(false);
        let (hwnd, size, kept, source, source_size) = {
            let mut guard = FOLLOWER.lock().map_err(|_| "follower state poisoned".to_string())?;
            let Some(f) = guard.as_mut() else { return Ok(serde_json::Value::Null) };
            let size = asked_size(args, f.scale)?;
            if let Some(key) = key.filter(|_| adopt) {
                f.pictures.adopt(key);
            }
            let kept = key.and_then(|key| f.pictures.kept_for(key, size));
            let shows = kept.as_ref().and(key);
            if f.size == size && f.shows.as_deref() == shows {
                return Ok(serde_json::Value::Null);
            }
            (f.hwnd, size, kept, f.source.clone(), f.source_size)
        };
        let shows = kept.as_ref().and(key).map(str::to_string);
        let bgra = kept.unwrap_or_else(|| Arc::new(compose(&source, source_size, size)));
        let mut guard = FOLLOWER.lock().map_err(|_| "follower state poisoned".to_string())?;
        // The picture ended, or another began, while this one was composed.
        let Some(f) = guard.as_mut().filter(|f| f.hwnd == hwnd) else { return Ok(serde_json::Value::Null) };
        f.size = size;
        f.shows = shows;
        f.offset = super::offset_within(f.grab, f.source_size, size);
        let at = if f.shown { f.cursor.map(|cursor| place(cursor, f.offset, size, f.home)) } else { None };
        if let Some(at) = at {
            f.last = at;
        }
        *PENDING_REPAINT.lock().map_err(|_| "repaint state poisoned".to_string())? = Some(Repaint { bgra, size, at });
        unsafe { PostMessageW(hwnd as HWND, WM_FOLLOW_RESIZE, 0, 0) };
        Ok(serde_json::Value::Null)
    }

    /// Hook thread, on every mouse move of a window-tab drag: show, move or hide the picture. Never
    /// blocks: a contended lock skips the move, and the window calls are asynchronous.
    pub fn on_cursor(x: i32, y: i32, over_other_window: bool) {
        let Ok(mut guard) = FOLLOWER.try_lock() else { return };
        let Some(f) = guard.as_mut() else { return };
        f.cursor = Some((x, y));
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
        let (px, py) = place((x, y), f.offset, f.size, f.home);
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

    /// Where the picture goes for the cursor: at its offset, pulled inside a work area, as the window promote
    /// does, so the real window opens exactly here. The area is `home` for a pane (its window's monitor), and
    /// for a window tab the monitor the picture lands on.
    fn place(cursor: (i32, i32), offset: (i32, i32), size: (i32, i32), home: Option<(i32, i32, i32, i32)>) -> (i32, i32) {
        let (ox, oy) = super::follow_origin(cursor, offset);
        match home.or_else(|| crate::app::get_monitor_work_area_physical(ox, oy)) {
            Some(area) => super::place_within(cursor, offset, size, area),
            None => (ox, oy),
        }
    }

    /// What `capture` takes its picture for, read from the page's request and the follower.
    struct CapturePlan {
        hwnd: usize,
        label: String,
        key: String,
        /// The size the hand shows the pane at for this rect, physical pixels.
        size: (i32, i32),
        /// The pane on screen: left, top, width, height, CSS px of the page.
        rect: (f64, f64, f64, f64),
        /// The page's viewport width, CSS px.
        viewport_width: f64,
        wash: Option<Wash>,
    }

    /// The plan for a request, or none when there is nothing to take: no picture is following, or the rect's
    /// picture is already kept.
    fn capture_plan(args: &serde_json::Value) -> Result<Option<CapturePlan>, String> {
        let key = args.get("landingKey").and_then(|v| v.as_str()).ok_or_else(|| "missing landingKey".to_string())?;
        let rect = args.get("rect").ok_or_else(|| "missing rect".to_string())?;
        let wash = match args.get("wash").filter(|w| !w.is_null()) {
            Some(w) => {
                let channel = |k: &str| num(w, k).map(|v| v.round().clamp(0.0, 255.0) as u8);
                Some(Wash { rgb: (channel("r")?, channel("g")?, channel("b")?), alpha: num(w, "a")? })
            }
            None => None,
        };
        let guard = FOLLOWER.lock().map_err(|_| "follower state poisoned".to_string())?;
        let Some(f) = guard.as_ref() else { return Ok(None) };
        let size = asked_size(args, f.scale)?;
        if f.pictures.kept_for(key, size).is_some() {
            return Ok(None);
        }
        Ok(Some(CapturePlan {
            hwnd: f.hwnd,
            label: f.label.clone(),
            key: key.to_string(),
            size,
            rect: (num(rect, "left")?, num(rect, "top")?, num(rect, "width")?, num(rect, "height")?),
            viewport_width: num(args, "viewportWidth")?,
            wash,
        }))
    }

    /// Take the pane's picture again, as it stands in its landing rect now: `landingKey` (the rect's name),
    /// `rect` (the pane on screen, CSS px), `viewportWidth` (CSS px), `windowWidth` and `windowHeight` (the
    /// size the hand shows it at for this rect, DIP), and `wash` (`r`, `g`, `b`, `a`: the landing zone's fill
    /// over the pane, taken back out of the picture). The window's whole viewport is pictured, as for the
    /// first picture (a clipped capture flickers the window), and the pane is cut out of it here, so the page
    /// does none of the work. The picture is only staged: the page keeps it with its next `resize` (`adopt`)
    /// if the pane stood still while it was taken. Answers `{ staged }`.
    pub async fn capture(state: &Arc<AppState>, args: &serde_json::Value) -> Result<serde_json::Value, String> {
        let Some(plan) = capture_plan(args)? else { return Ok(serde_json::json!({ "staged": false })) };
        let started = std::time::Instant::now();
        let jpeg = crate::browser_api::routes::capture_window_viewport(state, &plan.label, CAPTURE_QUALITY).await?;
        let pictured_ms = started.elapsed().as_millis() as u64;
        let staged = tokio::task::spawn_blocking(move || stage(plan, &jpeg, pictured_ms, started))
            .await
            .map_err(|e| format!("live_follow_capture join error: {}", e))??;
        Ok(serde_json::json!({ "staged": staged }))
    }

    /// Cut the pane out of the window's picture, at the hand's size, and stage it on the follower it was
    /// taken for. False when that follower is gone.
    fn stage(plan: CapturePlan, jpeg_base64: &str, pictured_ms: u64, started: std::time::Instant) -> Result<bool, String> {
        use base64::Engine;
        let jpeg = base64::engine::general_purpose::STANDARD
            .decode(jpeg_base64)
            .map_err(|e| format!("picture is not base64: {}", e))?;
        let window = image::load_from_memory_with_format(&jpeg, image::ImageFormat::Jpeg)
            .map_err(|e| format!("picture does not decode: {}", e))?
            .into_rgb8();
        let crop = crop_rect(plan.rect, plan.viewport_width, (window.width(), window.height()))
            .ok_or_else(|| "the pane is not in the picture".to_string())?;
        let pane = crop_bgra(window.as_raw(), window.width(), crop, plan.wash)
            .ok_or_else(|| "the pane is not in the picture".to_string())?;
        let bgra = Arc::new(compose(&pane, (crop.2 as i32, crop.3 as i32), plan.size));
        let mut guard = FOLLOWER.lock().map_err(|_| "follower state poisoned".to_string())?;
        let Some(f) = guard.as_mut().filter(|f| f.hwnd == plan.hwnd) else { return Ok(false) };
        tracing::info!(
            target: "dnd:tearoff",
            key = %plan.key, width = plan.size.0, height = plan.size.1, pictured_ms,
            total_ms = started.elapsed().as_millis() as u64,
            "[live-follow] the pane was pictured in its landing rect"
        );
        f.pictures.stage(Landed { key: plan.key, bgra, size: plan.size });
        Ok(true)
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

    fn spawn_overlay(bgra: Arc<Vec<u8>>, size: (i32, i32)) -> Result<usize, String> {
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
        if let Err(e) = paint(hwnd, bgra, size, None) {
            DestroyWindow(hwnd);
            return Err(e);
        }
        SetTimer(hwnd, TIMER_ID, FAILSAFE_MS, None);
        Ok(hwnd)
    }

    /// Show `bgra` (`size`) in the layered window, at the size given: the window takes that size with it, and
    /// the place `at` in the same step when one is given (it stays where it is otherwise).
    unsafe fn paint(hwnd: HWND, bgra: &[u8], size: (i32, i32), at: Option<(i32, i32)>) -> Result<(), String> {
        let (w, h) = size;
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
            let dest = at.map(|(x, y)| POINT { x, y });
            let dest_ptr = dest.as_ref().map_or(std::ptr::null(), |p| p as *const POINT);
            let ok = UpdateLayeredWindow(hwnd, screen, dest_ptr, &dest_size, mem, &src, 0, &blend, ULW_ALPHA);
            SelectObject(mem, old);
            result = if ok != 0 { Ok(()) } else { Err(format!("UpdateLayeredWindow failed: {}", GetLastError())) };
        }
        if !dib.is_null() {
            DeleteObject(dib as _);
        }
        DeleteDC(mem);
        ReleaseDC(std::ptr::null_mut(), screen);
        result
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
            WM_FOLLOW_RESIZE => {
                let taken = PENDING_REPAINT.lock().ok().and_then(|mut g| g.take());
                if let Some(repaint) = taken {
                    if let Err(e) = paint(hwnd, repaint.bgra.as_slice(), repaint.size, repaint.at) {
                        tracing::warn!(target: "dnd:tearoff", error = %e, "[live-follow] the picture could not be resized");
                    }
                }
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
    use super::{follow_origin, offset_within, picture_wanted, place_within};

    #[test]
    fn a_panes_picture_stops_whole_at_the_edge_of_its_monitor_while_the_cursor_goes_on() {
        // A 1920 by 1040 work area, with a second monitor to its right. The picture is 600 by 400, held at (100, 20).
        let (home, size, offset) = ((0, 0, 1920, 1040), (600, 400), (100, 20));
        // Well inside: at its offset from the cursor.
        assert_eq!(place_within((900, 500), offset, size, home), (800, 480));
        // Its right edge would pass the monitor's: it stops flush with the edge.
        assert_eq!(place_within((1700, 500), offset, size, home), (1320, 480));
        // The cursor is on the second monitor, corner and all: the picture is still on the first, whole.
        assert_eq!(place_within((2600, 500), offset, size, home), (1320, 480));
        // The other edges stop it the same way, and a monitor that does not start at 0 is its own area.
        assert_eq!(place_within((50, -300), offset, size, home), (0, 0));
        assert_eq!(place_within((900, 2000), offset, size, home), (800, 640));
        assert_eq!(place_within((100, 500), offset, size, (1920, 0, 2560, 1400)), (1920, 480));
    }

    #[test]
    fn the_grab_keeps_its_place_on_a_resized_picture_and_comes_back_whole_at_the_panes_own_size() {
        let grab = (500, 30);
        // Half the size: the cursor is on the same spot of the pane.
        assert_eq!(offset_within(grab, (1000, 800), (500, 400)), (250, 15));
        // From the original grab, not from the smaller picture's offset.
        assert_eq!(offset_within(grab, (1000, 800), (1000, 800)), (500, 30));
        // Kept inside the picture.
        assert_eq!(offset_within((1200, -5), (1000, 800), (500, 400)), (499, 0));
    }

    #[test]
    fn a_resized_picture_is_placed_around_the_cursor_not_around_its_own_corner() {
        // A 1000 by 800 pane pressed near its right end, on a monitor it is well inside.
        let (cursor, grab, own, home) = ((1500, 600), (900, 40), (1000, 800), (0, 0, 3840, 2100));
        let at_full = place_within(cursor, offset_within(grab, own, own), own, home);
        assert_eq!(at_full, (600, 560));
        // At half the size, with the cursor where it was, the corner moves toward the cursor: the spot that
        // was pressed is still under it. Left at its old corner, the picture would be 450 px from the cursor.
        let half = (500, 400);
        let at_half = place_within(cursor, offset_within(grab, own, half), half, home);
        assert_eq!(at_half, (1050, 580));
        assert_eq!((cursor.0 - at_half.0, cursor.1 - at_half.1), (450, 20));
    }

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
