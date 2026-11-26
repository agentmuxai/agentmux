// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! What `Desktop` does to a native app's window: mapping a point in a
//! screenshot to the screen, reading a key chord, and (on Windows) sending
//! the input (docs/specs/SPEC_DESKTOP_DRIVING_2026_10_20.md §5.1–5.2).
//!
//! Everything before [`win`] is plain logic, tested on every platform. The
//! rules it enforces are the spec's defence against a stale or invented
//! coordinate landing on the wrong thing: a point is relative to the image the
//! last `look` returned, and it is refused if the window moved or resized
//! since, or the point is outside the window.

use std::collections::HashMap;
use std::sync::Mutex;

/// `type` takes at most this many characters.
pub(crate) const MAX_TEXT_CHARS: usize = 500;
/// `wait` takes at most this long.
pub(crate) const MAX_WAIT_MS: u64 = 5000;
/// A screenshot is scaled to at most this on its long edge.
pub(crate) const MAX_IMAGE_EDGE: u32 = 1568;

/// A window's place on the screen, in physical pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Rect {
    pub(crate) x: i32,
    pub(crate) y: i32,
    pub(crate) w: u32,
    pub(crate) h: u32,
}

/// What the last `look` at a window returned: where the window was, and the
/// size of the image the agent was shown. Clicks are in that image's pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Look {
    pub(crate) rect: Rect,
    pub(crate) image_w: u32,
    pub(crate) image_h: u32,
    /// Which look of this window it was: 1 for the first, one more each time.
    /// Set by [`remember_look`].
    pub(crate) n: u32,
}

static LOOKS: Mutex<Option<HashMap<u32, Look>>> = Mutex::new(None);

/// Remember what `look` showed of window `id`, as the look after the one
/// before it. Returns the look as stored (numbered) and the one it replaced.
pub(crate) fn remember_look(id: u32, look: Look) -> (Look, Option<Look>) {
    let mut g = LOOKS.lock().unwrap_or_else(|e| e.into_inner());
    let map = g.get_or_insert_with(HashMap::new);
    let before = map.get(&id).copied();
    let stored = Look { n: before.map_or(1, |b| b.n.wrapping_add(1).max(1)), ..look };
    map.insert(id, stored);
    (stored, before)
}

/// Whether coordinates the agent says come from look `of` can be used: they
/// can when `of` is the window's latest look. Otherwise the answer says which
/// look is the latest, and whether its size differs from the one the
/// coordinates came from, so the agent knows they are stale.
pub(crate) fn check_of_look(latest: &Look, of: i64) -> Result<(), String> {
    if of == latest.n as i64 {
        return Ok(());
    }
    Err(format!(
        "your coordinates are from look #{of}, but the latest look at this window is #{} ({}×{} pixels); coordinates from an older look can point at the wrong place. Use the latest look's pixels, or look again",
        latest.n, latest.image_w, latest.image_h
    ))
}

/// What a `look` says about how its image relates to the window and to the
/// look before it: the scale (window pixels per image pixel), and a warning
/// when the image's size changed since the last look of the same window.
pub(crate) fn describe_scale(look: &Look, before: Option<&Look>) -> String {
    let scale = if look.image_w == 0 { 1.0 } else { look.rect.w as f64 / look.image_w as f64 };
    let mut out = format!(
        "This is look #{}: the image is {}×{} pixels for a window of {}×{} on the screen (scale {:.2}).",
        look.n, look.image_w, look.image_h, look.rect.w, look.rect.h, scale
    );
    if let Some(b) = before {
        if (b.image_w, b.image_h) != (look.image_w, look.image_h) {
            out.push_str(&format!(
                " The window's size changed since look #{} ({}×{}): coordinates from that look, or any earlier one, are stale.",
                b.n, b.image_w, b.image_h
            ));
        }
    }
    out
}

/// A region of the last look, in that look's pixels, to be shown zoomed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Region {
    pub(crate) x: u32,
    pub(crate) y: u32,
    pub(crate) w: u32,
    pub(crate) h: u32,
}

/// The smallest region `look` zooms into: smaller is not worth an image.
pub(crate) const MIN_REGION: u32 = 8;
/// The most a region is enlarged.
pub(crate) const MAX_ZOOM: f64 = 4.0;

/// A region inside `look`'s image, or why not.
pub(crate) fn check_region(look: &Look, x: i64, y: i64, w: i64, h: i64) -> Result<Region, String> {
    if w < MIN_REGION as i64 || h < MIN_REGION as i64 {
        return Err(format!("the region is {w}×{h}; it must be at least {MIN_REGION}×{MIN_REGION} pixels"));
    }
    // Checked sums: a huge x or y from the caller must read as outside, not overflow.
    let right = x.checked_add(w);
    let bottom = y.checked_add(h);
    if x < 0 || y < 0 || right.is_none_or(|r| r > look.image_w as i64) || bottom.is_none_or(|b| b > look.image_h as i64) {
        return Err(format!(
            "the region ({x}, {y}, {w}×{h}) is not inside look #{}'s image, which is {}×{}",
            look.n, look.image_w, look.image_h
        ));
    }
    Ok(Region { x: x as u32, y: y as u32, w: w as u32, h: h as u32 })
}

/// How much a `w`×`h` region is enlarged: up to [`MAX_ZOOM`], and not past
/// [`MAX_IMAGE_EDGE`] on its long edge. Never below 1.
pub(crate) fn zoom_for(w: u32, h: u32) -> f64 {
    let long = w.max(h).max(1) as f64;
    (MAX_IMAGE_EDGE as f64 / long).clamp(1.0, MAX_ZOOM)
}

/// What the last `look` at window `id` showed, if there was one.
pub(crate) fn last_look(id: u32) -> Option<Look> {
    let g = LOOKS.lock().unwrap_or_else(|e| e.into_inner());
    g.as_ref().and_then(|m| m.get(&id).copied())
}

/// The size a `w`×`h` window is shown at: its long edge at most
/// [`MAX_IMAGE_EDGE`], never enlarged.
pub(crate) fn scaled_size(w: u32, h: u32) -> (u32, u32) {
    let long = w.max(h);
    if long <= MAX_IMAGE_EDGE || long == 0 {
        return (w, h);
    }
    let scale = MAX_IMAGE_EDGE as f64 / long as f64;
    (((w as f64 * scale).round() as u32).max(1), ((h as f64 * scale).round() as u32).max(1))
}

/// The screen point for image point (`x`, `y`) of `look`, given where the
/// window is `now`. Refuses, with a reason the agent can act on, when the
/// window moved or resized since the look or the point is outside the image.
pub(crate) fn to_screen(look: &Look, now: Rect, x: i64, y: i64) -> Result<(i32, i32), String> {
    if now != look.rect {
        return Err("the window moved or resized since you last looked at it; look again".into());
    }
    if look.image_w == 0 || look.image_h == 0 || now.w == 0 || now.h == 0 {
        return Err("the window has no size".into());
    }
    if x < 0 || y < 0 || x >= look.image_w as i64 || y >= look.image_h as i64 {
        return Err(format!(
            "({x}, {y}) is outside the window's image, which is {}×{}; use the pixels of your last look",
            look.image_w, look.image_h
        ));
    }
    // The middle of the image pixel, scaled back to the window's own pixels.
    let sx = now.x as f64 + (x as f64 + 0.5) * now.w as f64 / look.image_w as f64;
    let sy = now.y as f64 + (y as f64 + 0.5) * now.h as f64 / look.image_h as f64;
    Ok((sx.floor() as i32, sy.floor() as i32))
}

/// A screen point as `SendInput` wants it with `MOUSEEVENTF_ABSOLUTE |
/// MOUSEEVENTF_VIRTUALDESK`: 0..=65535 across the whole virtual screen, which
/// starts at (`vx`, `vy`) and is `vw`×`vh` pixels.
pub(crate) fn to_absolute(sx: i32, sy: i32, vx: i32, vy: i32, vw: u32, vh: u32) -> (i32, i32) {
    let norm = |p: i32, origin: i32, size: u32| -> i32 {
        let span = (size.max(2) - 1) as i64;
        (((p as i64 - origin as i64) * 65535) / span).clamp(0, 65535) as i32
    };
    (norm(sx, vx, vw), norm(sy, vy, vh))
}

/// A drag passes through at most this many given points besides its two ends.
pub(crate) const MAX_DRAG_WAYPOINTS: usize = 8;
/// The most mouse moves one drag sends, however far it goes.
pub(crate) const MAX_DRAG_MOVES: usize = 80;
/// Roughly one move per this many pixels, so an app that samples the pointer
/// sees a path, not a jump.
const DRAG_PIXELS_PER_MOVE: f64 = 12.0;
/// Every leg of a drag has at least this many moves.
const DRAG_MIN_MOVES_PER_LEG: usize = 4;

/// The screen points a drag moves through, from `start` to `end` by way of
/// `via`, in order. Each leg is cut into short steps (the last step of a leg
/// lands exactly on its end), and the whole drag is at most
/// [`MAX_DRAG_MOVES`] moves. The first point is `start` itself.
pub(crate) fn drag_path(start: (i32, i32), via: &[(i32, i32)], end: (i32, i32)) -> Vec<(i32, i32)> {
    let mut stops = vec![start];
    stops.extend_from_slice(via);
    stops.push(end);
    let legs = stops.len() - 1;
    // Each leg gets the moves its length asks for, within the drag's total.
    let wanted: Vec<usize> = stops
        .windows(2)
        .map(|w| {
            let d = (((w[1].0 - w[0].0) as f64).powi(2) + ((w[1].1 - w[0].1) as f64).powi(2)).sqrt();
            ((d / DRAG_PIXELS_PER_MOVE).ceil() as usize).max(DRAG_MIN_MOVES_PER_LEG)
        })
        .collect();
    let total: usize = wanted.iter().sum();
    let budget = MAX_DRAG_MOVES.max(legs);
    let mut path = vec![start];
    for (i, w) in stops.windows(2).enumerate() {
        let n = if total <= budget { wanted[i] } else { (wanted[i] * budget / total).max(1) };
        for k in 1..=n {
            let f = k as f64 / n as f64;
            let p = if k == n {
                w[1]
            } else {
                (
                    (w[0].0 as f64 + (w[1].0 - w[0].0) as f64 * f).round() as i32,
                    (w[0].1 as f64 + (w[1].1 - w[0].1) as f64 * f).round() as i32,
                )
            };
            path.push(p);
        }
    }
    path
}

/// What a drag does to the mouse and keyboard, one step at a time, for
/// [`run_drag`] to hand to whatever sends it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) enum DragStep {
    /// Move the pointer to a screen point.
    Move(i32, i32),
    KeyDown(u16),
    KeyUp(u16),
    ButtonDown,
    ButtonUp,
    /// Wait this many milliseconds.
    Pause(u64),
}

/// Pause after the pointer arrives and the modifiers go down, before the press.
const DRAG_SETTLE_MS: u64 = 40;
/// Pause after the press, before the first move: an app that starts a drag on
/// the first motion needs to have seen the press.
const DRAG_PRESS_MS: u64 = 120;
/// Pause between moves.
const DRAG_STEP_MS: u64 = 8;
/// Pause on the last point before the release, so a drop target registers.
const DRAG_DROP_MS: u64 = 150;

/// Hold `modifiers`, press the button at the first point, move through the
/// rest, pause, and let go at the last. `out` does each step. Whatever was
/// pressed is let go again on every exit, including when a step fails: the
/// button first, then the modifiers in reverse, so a failed drag never leaves
/// one held. The first error is the one returned.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn run_drag(
    points: &[(i32, i32)],
    modifiers: &[u16],
    out: &mut dyn FnMut(DragStep) -> Result<(), String>,
) -> Result<(), String> {
    let mut button_down = false;
    let mut modifiers_down = 0usize;
    let result = (|| -> Result<(), String> {
        let (first, rest) = points.split_first().ok_or("a drag needs points")?;
        out(DragStep::Move(first.0, first.1))?;
        for m in modifiers {
            out(DragStep::KeyDown(*m))?;
            modifiers_down += 1;
        }
        out(DragStep::Pause(DRAG_SETTLE_MS))?;
        out(DragStep::ButtonDown)?;
        button_down = true;
        out(DragStep::Pause(DRAG_PRESS_MS))?;
        for p in rest {
            out(DragStep::Move(p.0, p.1))?;
            out(DragStep::Pause(DRAG_STEP_MS))?;
        }
        out(DragStep::Pause(DRAG_DROP_MS))?;
        out(DragStep::ButtonUp)?;
        button_down = false;
        Ok(())
    })();
    // Let go of whatever is still held, whatever happened above. Each is tried
    // even if an earlier one fails.
    if button_down {
        let _ = out(DragStep::ButtonUp);
    }
    for m in modifiers.iter().take(modifiers_down).rev() {
        let _ = out(DragStep::KeyUp(*m));
    }
    result
}

/// The modifiers `drag` may hold while it drags: ctrl, alt and shift, written
/// like a chord without a key (`ctrl+shift`). The Windows key is refused for
/// the same reason `key` refuses it.
pub(crate) fn parse_hold(spec: &str) -> Result<Vec<u16>, String> {
    let mut modifiers = Vec::new();
    for m in spec.split('+').map(str::trim).filter(|p| !p.is_empty()) {
        let code = match m.to_lowercase().as_str() {
            "ctrl" | "control" => VK_CONTROL,
            "alt" | "option" => VK_MENU,
            "shift" => VK_SHIFT,
            "win" | "windows" | "meta" | "cmd" | "command" | "super" => {
                return Err("the Windows/Command key is not allowed: it acts on the operating system, not the app".into())
            }
            other => return Err(format!("unknown key to hold {other:?}; use ctrl, alt or shift")),
        };
        if !modifiers.contains(&code) {
            modifiers.push(code);
        }
    }
    Ok(modifiers)
}

/// A key chord such as `ctrl+shift+s`: modifiers held, then one key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Chord {
    /// Virtual-key codes of the modifiers, in the order written.
    pub(crate) modifiers: Vec<u16>,
    pub(crate) key: u16,
}

const VK_SHIFT: u16 = 0x10;
const VK_CONTROL: u16 = 0x11;
const VK_MENU: u16 = 0x12;

/// The virtual-key code of a key name, or `None`.
fn key_code(name: &str) -> Option<u16> {
    let n = name.to_lowercase();
    if n.chars().count() == 1 {
        let c = n.chars().next().unwrap();
        return match c {
            'a'..='z' => Some(0x41 + (c as u16 - 'a' as u16)),
            '0'..='9' => Some(0x30 + (c as u16 - '0' as u16)),
            _ => None,
        };
    }
    if let Some(num) = n.strip_prefix('f').and_then(|d| d.parse::<u16>().ok()) {
        return (1..=12).contains(&num).then(|| 0x70 + num - 1);
    }
    Some(match n.as_str() {
        "enter" | "return" => 0x0D,
        "tab" => 0x09,
        "esc" | "escape" => 0x1B,
        "space" => 0x20,
        "backspace" => 0x08,
        "delete" | "del" => 0x2E,
        "insert" => 0x2D,
        "home" => 0x24,
        "end" => 0x23,
        "pageup" => 0x21,
        "pagedown" => 0x22,
        "left" => 0x25,
        "up" => 0x26,
        "right" => 0x27,
        "down" => 0x28,
        _ => return None,
    })
}

/// Read a chord. The Windows key is refused: a chord with it acts on the
/// operating system (Run, the Start menu, lock), not on the app the user
/// allowed.
pub(crate) fn parse_keys(spec: &str) -> Result<Chord, String> {
    let parts: Vec<&str> = spec.split('+').map(str::trim).filter(|p| !p.is_empty()).collect();
    let Some((last, mods)) = parts.split_last() else {
        return Err("keys is empty; give a key such as \"enter\" or a chord such as \"ctrl+s\"".into());
    };
    let mut modifiers = Vec::new();
    for m in mods {
        let code = match m.to_lowercase().as_str() {
            "ctrl" | "control" => VK_CONTROL,
            "alt" | "option" => VK_MENU,
            "shift" => VK_SHIFT,
            "win" | "windows" | "meta" | "cmd" | "command" | "super" => {
                return Err("the Windows/Command key is not allowed: it acts on the operating system, not the app".into())
            }
            other => return Err(format!("unknown modifier {other:?}; use ctrl, alt or shift")),
        };
        if !modifiers.contains(&code) {
            modifiers.push(code);
        }
    }
    let key = key_code(last).ok_or_else(|| {
        format!("unknown key {last:?}; use a letter, a digit, f1–f12, or enter, tab, esc, space, backspace, delete, insert, home, end, pageup, pagedown, left, up, right, down")
    })?;
    Ok(Chord { modifiers, key })
}

/// `type`'s text must be non-empty, short, and free of control characters
/// other than newline and tab.
pub(crate) fn check_text(text: &str) -> Result<(), String> {
    if text.is_empty() {
        return Err("text is empty".into());
    }
    let n = text.chars().count();
    if n > MAX_TEXT_CHARS {
        return Err(format!("text is {n} characters; the most one call types is {MAX_TEXT_CHARS}"));
    }
    if text.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
        return Err("text has a control character; use the key action for keys such as escape".into());
    }
    Ok(())
}

#[cfg(windows)]
pub(crate) mod win {
    //! `SendInput` for Windows. The process is made per-monitor DPI aware once,
    //! so the window rectangles `xcap` reports and the points sent here are in
    //! the same physical pixels.

    use std::mem::size_of;
    use std::sync::Once;
    use std::time::Duration;

    use windows_sys::Win32::Foundation::{CloseHandle, BOOL, HWND, LPARAM, POINT, RECT};
    use windows_sys::Win32::System::Threading::{
        AttachThreadInput, GetCurrentThreadId, OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, EnumChildWindows, GetAncestor, GetForegroundWindow, GetSystemMetrics, GetWindowRect,
        GetWindowThreadProcessId, IsIconic, SetForegroundWindow, ShowWindow, WindowFromPoint,
        GA_ROOT, GA_ROOTOWNER, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SW_RESTORE,
    };

    use super::{to_absolute, Chord, DragStep, Rect};

    pub(crate) fn ensure_dpi_aware() {
        static ONCE: Once = Once::new();
        ONCE.call_once(|| unsafe {
            // Fails harmlessly if the process already has an awareness.
            SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        });
    }

    /// The whole window, invisible resize borders included, in physical
    /// pixels: the area `PrintWindow` (what `look` captures) covers, so an
    /// image pixel maps onto it without an offset. `xcap`'s own rectangle is
    /// the visible frame, a few pixels smaller.
    pub(crate) fn window_rect(id: u32) -> Option<Rect> {
        ensure_dpi_aware();
        let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        let ok = unsafe { GetWindowRect(id as usize as HWND, &mut r) };
        if ok == 0 || r.right <= r.left || r.bottom <= r.top {
            return None;
        }
        Some(Rect { x: r.left, y: r.top, w: (r.right - r.left) as u32, h: (r.bottom - r.top) as u32 })
    }

    /// The executable's file name of the process that owns `hwnd`.
    fn window_exe_name(hwnd: HWND) -> Option<String> {
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(hwnd, &mut pid) };
        if pid == 0 {
            return None;
        }
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                return None;
            }
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len);
            CloseHandle(process);
            if ok == 0 || len == 0 {
                return None;
            }
            let path = String::from_utf16_lossy(&buf[..len as usize]);
            path.rsplit(['\\', '/']).next().map(str::to_string)
        }
    }

    /// Restore window `id` if it is minimised, and wait for it to be drawn.
    /// Returns whether it was minimised.
    pub(crate) fn restore_if_minimized(id: u32) -> bool {
        ensure_dpi_aware();
        let hwnd = id as usize as HWND;
        unsafe {
            if IsIconic(hwnd) == 0 {
                return false;
            }
            ShowWindow(hwnd, SW_RESTORE);
            for _ in 0..20 {
                if IsIconic(hwnd) == 0 {
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        }
        // Let it paint before it is captured.
        std::thread::sleep(Duration::from_millis(300));
        true
    }

    /// Try the usual ways to make `hwnd` the foreground window, in turn: ask;
    /// share the input queue of the window that has the focus (Windows lets a
    /// thread joined to it take the focus); and a harmless key press (F24),
    /// which counts as the user's input and lifts the foreground lock.
    unsafe fn try_foreground(hwnd: HWND, attempt: u32) -> bool {
        unsafe {
            if GetForegroundWindow() == hwnd {
                return true;
            }
            SetForegroundWindow(hwnd);
            if GetForegroundWindow() == hwnd {
                return true;
            }
            let fg = GetForegroundWindow();
            let fg_thread = if fg.is_null() { 0 } else { GetWindowThreadProcessId(fg, std::ptr::null_mut()) };
            let me = GetCurrentThreadId();
            let joined = fg_thread != 0 && fg_thread != me && AttachThreadInput(me, fg_thread, 1) != 0;
            BringWindowToTop(hwnd);
            SetForegroundWindow(hwnd);
            if joined {
                AttachThreadInput(me, fg_thread, 0);
            }
            if GetForegroundWindow() == hwnd {
                return true;
            }
            if attempt >= 1 {
                let _ = send(&[key(VK_F24, 0, 0), key(VK_F24, 0, KEYEVENTF_KEYUP)]);
                SetForegroundWindow(hwnd);
            }
            GetForegroundWindow() == hwnd
        }
    }

    /// Whether `hwnd` is the foreground window, after a few tries at bringing
    /// it there (restoring it if minimised). When it can't be, the error names
    /// the window that has the focus.
    pub(crate) fn bring_to_front(id: u32) -> Result<(), String> {
        ensure_dpi_aware();
        let hwnd = id as usize as HWND;
        restore_if_minimized(id);
        for attempt in 0..3u32 {
            if unsafe { try_foreground(hwnd, attempt) } {
                // Give the app a moment to take the focus before input goes to it.
                std::thread::sleep(Duration::from_millis(100));
                if unsafe { GetForegroundWindow() } == hwnd {
                    return Ok(());
                }
            }
            std::thread::sleep(Duration::from_millis(150 * (attempt as u64 + 1)));
        }
        if unsafe { GetForegroundWindow() } == hwnd {
            return Ok(());
        }
        let fg = unsafe { GetForegroundWindow() };
        let holder = if fg.is_null() {
            "no window".to_string()
        } else {
            // The app only, never its title: the title of an app the user hasn't
            // allowed could be sensitive (`apps` hides a blocked window's too).
            window_exe_name(fg).unwrap_or_else(|| "another app".into())
        };
        Err(format!(
            "could not bring the window to the front: {holder} has the focus and Windows would not hand it over. Call again in a few seconds; if it keeps failing, ask the person to click this app's window once"
        ))
    }

    /// The window at screen point (`sx`, `sy`) must be `id` itself, a child
    /// of it, or a window it owns: an open menu, dropdown or dialog of the app
    /// is a top-level window of its own, owned by the app's window
    /// (`GA_ROOTOWNER`). If anything else covers the point, the click would
    /// land there, so it is refused.
    pub(crate) fn point_belongs_to(id: u32, sx: i32, sy: i32) -> Result<(), String> {
        let target = id as usize as HWND;
        unsafe {
            let at = WindowFromPoint(POINT { x: sx, y: sy });
            if at.is_null() {
                return Err("nothing is at that point".into());
            }
            let root = GetAncestor(at, GA_ROOT);
            let owner = GetAncestor(at, GA_ROOTOWNER);
            if root == target || owner == target || at == target {
                Ok(())
            } else {
                Err("another window covers that point; the click would land there".into())
            }
        }
    }

    /// The executable of the app a Windows Store (UWP) frame hosts. Its
    /// top-level window belongs to `ApplicationFrameHost.exe`, which hosts
    /// every such app, so the app itself is the process of a child window
    /// that isn't the frame host's. `None` when there is none, as while the
    /// app is minimised and suspended.
    pub(crate) fn hosted_app_exe(id: u32) -> Option<String> {
        struct Search {
            frame_pid: u32,
            found: u32,
        }
        unsafe extern "system" fn each(child: HWND, lparam: LPARAM) -> BOOL {
            let search = unsafe { &mut *(lparam as *mut Search) };
            let mut pid = 0u32;
            unsafe { GetWindowThreadProcessId(child, &mut pid) };
            if pid != 0 && pid != search.frame_pid {
                search.found = pid;
                return 0;
            }
            1
        }
        let frame = id as usize as HWND;
        let mut search = Search { frame_pid: 0, found: 0 };
        unsafe {
            GetWindowThreadProcessId(frame, &mut search.frame_pid);
            EnumChildWindows(frame, Some(each), &mut search as *mut Search as LPARAM);
        }
        if search.found == 0 {
            return None;
        }
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, search.found);
            if process.is_null() {
                return None;
            }
            let mut buf = [0u16; 1024];
            let mut len = buf.len() as u32;
            let ok = QueryFullProcessImageNameW(process, PROCESS_NAME_WIN32, buf.as_mut_ptr(), &mut len);
            CloseHandle(process);
            (ok != 0 && len > 0).then(|| String::from_utf16_lossy(&buf[..len as usize]))
        }
    }

    fn send(inputs: &[INPUT]) -> Result<(), String> {
        let sent = unsafe { SendInput(inputs.len() as u32, inputs.as_ptr(), size_of::<INPUT>() as i32) };
        if sent as usize == inputs.len() {
            Ok(())
        } else {
            Err(format!("Windows accepted {sent} of {} input events (an elevated window refuses input from here)", inputs.len()))
        }
    }

    fn mouse(dx: i32, dy: i32, data: i32, flags: MOUSE_EVENT_FLAGS) -> INPUT {
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 { mi: MOUSEINPUT { dx, dy, mouseData: data as u32, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
        }
    }

    fn key(vk: u16, unicode: u16, flags: KEYBD_EVENT_FLAGS) -> INPUT {
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: unicode, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
        }
    }

    fn virtual_screen() -> (i32, i32, u32, u32) {
        unsafe {
            (
                GetSystemMetrics(SM_XVIRTUALSCREEN),
                GetSystemMetrics(SM_YVIRTUALSCREEN),
                GetSystemMetrics(SM_CXVIRTUALSCREEN).max(1) as u32,
                GetSystemMetrics(SM_CYVIRTUALSCREEN).max(1) as u32,
            )
        }
    }

    fn move_to(sx: i32, sy: i32) -> INPUT {
        let (vx, vy, vw, vh) = virtual_screen();
        let (ax, ay) = to_absolute(sx, sy, vx, vy, vw, vh);
        mouse(ax, ay, 0, MOUSEEVENTF_MOVE | MOUSEEVENTF_ABSOLUTE | MOUSEEVENTF_VIRTUALDESK)
    }

    fn button_flags(button: &str) -> Result<(MOUSE_EVENT_FLAGS, MOUSE_EVENT_FLAGS), String> {
        match button {
            "left" => Ok((MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP)),
            "right" => Ok((MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP)),
            "middle" => Ok((MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP)),
            other => Err(format!("unknown button {other:?}; use left, right or middle")),
        }
    }

    /// Hold `modifiers`, press `button` at the first point, move through the
    /// rest in short steps and let go at the last (see `run_drag`). It takes
    /// about a second at most, and lets go on every exit path.
    pub(crate) fn drag(points: &[(i32, i32)], button: &str, modifiers: &[u16]) -> Result<(), String> {
        let (down, up) = button_flags(button)?;
        super::run_drag(points, modifiers, &mut |step| match step {
            DragStep::Move(x, y) => send(&[move_to(x, y)]),
            DragStep::KeyDown(vk) => send(&[key(vk, 0, 0)]),
            DragStep::KeyUp(vk) => send(&[key(vk, 0, KEYEVENTF_KEYUP)]),
            DragStep::ButtonDown => send(&[mouse(0, 0, 0, down)]),
            DragStep::ButtonUp => send(&[mouse(0, 0, 0, up)]),
            DragStep::Pause(ms) => {
                std::thread::sleep(Duration::from_millis(ms));
                Ok(())
            }
        })
    }

    /// Click `count` times at a screen point with `button` ("left", "right",
    /// "middle").
    pub(crate) fn click(sx: i32, sy: i32, button: &str, count: u32) -> Result<(), String> {
        let (down, up) = button_flags(button)?;
        let mut events = vec![move_to(sx, sy)];
        for _ in 0..count.clamp(1, 3) {
            events.push(mouse(0, 0, 0, down));
            events.push(mouse(0, 0, 0, up));
        }
        send(&events)
    }

    /// Scroll at a screen point: `dy` notches (positive is up), `dx` notches
    /// (positive is right).
    pub(crate) fn scroll(sx: i32, sy: i32, dx: i32, dy: i32) -> Result<(), String> {
        let mut events = vec![move_to(sx, sy)];
        if dy != 0 {
            events.push(mouse(0, 0, dy.clamp(-50, 50) * 120, MOUSEEVENTF_WHEEL));
        }
        if dx != 0 {
            events.push(mouse(0, 0, dx.clamp(-50, 50) * 120, MOUSEEVENTF_HWHEEL));
        }
        send(&events)
    }

    /// Press a chord: modifiers down, the key, modifiers up.
    pub(crate) fn press(chord: &Chord) -> Result<(), String> {
        let mut events = Vec::new();
        for m in &chord.modifiers {
            events.push(key(*m, 0, 0));
        }
        events.push(key(chord.key, 0, 0));
        events.push(key(chord.key, 0, KEYEVENTF_KEYUP));
        for m in chord.modifiers.iter().rev() {
            events.push(key(*m, 0, KEYEVENTF_KEYUP));
        }
        let result = send(&events);
        if result.is_err() {
            // Do not leave a modifier held down.
            let ups: Vec<INPUT> = chord.modifiers.iter().map(|m| key(*m, 0, KEYEVENTF_KEYUP)).collect();
            let _ = send(&ups);
        }
        result
    }

    /// Type `text` into whatever has the focus. Newline is Enter and tab is
    /// Tab; everything else is sent as the character itself.
    pub(crate) fn type_text(text: &str) -> Result<(), String> {
        let mut events = Vec::new();
        for c in text.chars() {
            match c {
                '\n' => {
                    events.push(key(VK_RETURN, 0, 0));
                    events.push(key(VK_RETURN, 0, KEYEVENTF_KEYUP));
                }
                '\t' => {
                    events.push(key(VK_TAB, 0, 0));
                    events.push(key(VK_TAB, 0, KEYEVENTF_KEYUP));
                }
                _ => {
                    let mut buf = [0u16; 2];
                    for unit in c.encode_utf16(&mut buf) {
                        events.push(key(0, *unit, KEYEVENTF_UNICODE));
                        events.push(key(0, *unit, KEYEVENTF_UNICODE | KEYEVENTF_KEYUP));
                    }
                }
            }
        }
        send(&events)
    }
}

/// Other platforms: the tool answers that driving is Windows-only for now
/// (spec §8, phases 3 and 4).
#[cfg(not(windows))]
pub(crate) mod win {
    use super::{Chord, Rect};

    const NOT_YET: &str = "Desktop driving works on Windows in this version; macOS and Linux (X11) come later";

    pub(crate) fn ensure_dpi_aware() {}
    pub(crate) fn window_rect(_id: u32) -> Option<Rect> {
        None
    }
    pub(crate) fn bring_to_front(_id: u32) -> Result<(), String> {
        Err(NOT_YET.into())
    }
    pub(crate) fn restore_if_minimized(_id: u32) -> bool {
        false
    }
    pub(crate) fn point_belongs_to(_id: u32, _sx: i32, _sy: i32) -> Result<(), String> {
        Err(NOT_YET.into())
    }
    pub(crate) fn hosted_app_exe(_id: u32) -> Option<String> {
        None
    }
    pub(crate) fn click(_sx: i32, _sy: i32, _button: &str, _count: u32) -> Result<(), String> {
        Err(NOT_YET.into())
    }
    pub(crate) fn scroll(_sx: i32, _sy: i32, _dx: i32, _dy: i32) -> Result<(), String> {
        Err(NOT_YET.into())
    }
    pub(crate) fn drag(_points: &[(i32, i32)], _button: &str, _modifiers: &[u16]) -> Result<(), String> {
        Err(NOT_YET.into())
    }
    pub(crate) fn press(_chord: &Chord) -> Result<(), String> {
        Err(NOT_YET.into())
    }
    pub(crate) fn type_text(_text: &str) -> Result<(), String> {
        Err(NOT_YET.into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn look() -> Look {
        Look { rect: Rect { x: 100, y: 50, w: 2000, h: 1000 }, image_w: 1000, image_h: 500, n: 0 }
    }

    #[test]
    fn a_big_window_is_shown_at_most_1568_on_its_long_edge() {
        assert_eq!(scaled_size(3136, 1568), (1568, 784));
        assert_eq!(scaled_size(800, 600), (800, 600));
        assert_eq!(scaled_size(1568, 100), (1568, 100));
    }

    #[test]
    fn an_image_point_maps_to_the_middle_of_its_pixels_on_the_screen() {
        // The image is half the window's size: pixel 0 covers screen x 100..102.
        assert_eq!(to_screen(&look(), look().rect, 0, 0), Ok((101, 51)));
        assert_eq!(to_screen(&look(), look().rect, 500, 250), Ok((1101, 551)));
        assert_eq!(to_screen(&look(), look().rect, 999, 499), Ok((2099, 1049)));
    }

    #[test]
    fn a_moved_or_resized_window_is_refused() {
        let l = look();
        let moved = Rect { x: 101, ..l.rect };
        assert!(to_screen(&l, moved, 10, 10).unwrap_err().contains("look again"));
        let resized = Rect { w: 1999, ..l.rect };
        assert!(to_screen(&l, resized, 10, 10).unwrap_err().contains("look again"));
    }

    #[test]
    fn a_point_outside_the_image_is_refused() {
        let l = look();
        for (x, y) in [(-1, 0), (0, -1), (1000, 0), (0, 500), (5000, 5000)] {
            assert!(to_screen(&l, l.rect, x, y).unwrap_err().contains("outside"), "({x},{y})");
        }
    }

    #[test]
    fn absolute_coordinates_span_the_virtual_screen() {
        assert_eq!(to_absolute(0, 0, 0, 0, 1920, 1080), (0, 0));
        assert_eq!(to_absolute(1919, 1079, 0, 0, 1920, 1080), (65535, 65535));
        // A second monitor to the left makes the origin negative.
        assert_eq!(to_absolute(-1920, 0, -1920, 0, 3840, 1080), (0, 0));
        assert_eq!(to_absolute(1919, 0, -1920, 0, 3840, 1080), (65535, 0));
        // Outside the screen clamps.
        assert_eq!(to_absolute(-50, 5000, 0, 0, 1920, 1080), (0, 65535));
    }

    #[test]
    fn chords_are_read_with_modifiers_first() {
        assert_eq!(parse_keys("ctrl+s"), Ok(Chord { modifiers: vec![VK_CONTROL], key: 0x53 }));
        assert_eq!(
            parse_keys("Ctrl + Shift + F5"),
            Ok(Chord { modifiers: vec![VK_CONTROL, VK_SHIFT], key: 0x74 })
        );
        assert_eq!(parse_keys("enter"), Ok(Chord { modifiers: vec![], key: 0x0D }));
        assert_eq!(parse_keys("alt+left"), Ok(Chord { modifiers: vec![VK_MENU], key: 0x25 }));
        assert_eq!(parse_keys("7"), Ok(Chord { modifiers: vec![], key: 0x37 }));
    }

    #[test]
    fn the_windows_key_and_unknown_keys_are_refused() {
        for spec in ["win+r", "cmd+c", "meta+x", "super+e"] {
            assert!(parse_keys(spec).unwrap_err().contains("not allowed"), "{spec}");
        }
        assert!(parse_keys("").is_err());
        assert!(parse_keys("ctrl+").is_err());
        assert!(parse_keys("f13").is_err());
        assert!(parse_keys("hyper+a").unwrap_err().contains("unknown modifier"));
        assert!(parse_keys("ctrl+banana").unwrap_err().contains("unknown key"));
    }

    #[test]
    fn typed_text_is_limited() {
        assert!(check_text("hello\nworld\t!").is_ok());
        assert!(check_text("").is_err());
        assert!(check_text(&"a".repeat(501)).unwrap_err().contains("501"));
        assert!(check_text(&"a".repeat(500)).is_ok());
        assert!(check_text("a\u{1b}b").unwrap_err().contains("control"));
    }

    #[test]
    fn a_drag_starts_and_ends_exactly_where_it_was_told() {
        let path = drag_path((100, 100), &[], (400, 100));
        assert_eq!(path.first(), Some(&(100, 100)));
        assert_eq!(path.last(), Some(&(400, 100)));
        // 300 px in steps of about 12 px, never a jump.
        assert!(path.len() > 20, "{}", path.len());
        assert!(path.windows(2).all(|w| (w[1].0 - w[0].0).abs() <= 13), "{path:?}");
        // Moves only go forward along a straight leg.
        assert!(path.windows(2).all(|w| w[1].0 >= w[0].0));
    }

    #[test]
    fn a_short_drag_still_moves_in_several_steps() {
        let path = drag_path((10, 10), &[], (14, 10));
        assert!(path.len() >= 5, "{path:?}");
        assert_eq!(path.last(), Some(&(14, 10)));
    }

    #[test]
    fn a_drag_passes_through_each_waypoint_in_order() {
        let path = drag_path((0, 0), &[(100, 0), (100, 100)], (0, 100));
        let at = |p: (i32, i32)| path.iter().position(|q| *q == p).unwrap_or_else(|| panic!("{p:?} not on {path:?}"));
        assert!(at((100, 0)) < at((100, 100)));
        assert!(at((100, 100)) < at((0, 100)));
        assert_eq!(path.last(), Some(&(0, 100)));
    }

    #[test]
    fn a_long_drag_is_capped_but_keeps_every_stop() {
        let via = [(5000, 0), (5000, 5000), (0, 5000), (0, 0), (5000, 5000), (1, 1), (4000, 40), (2, 2)];
        let path = drag_path((0, 0), &via, (9000, 9000));
        assert!(path.len() <= MAX_DRAG_MOVES + 1, "{}", path.len());
        for stop in via {
            assert!(path.contains(&stop), "{stop:?} missing");
        }
        assert_eq!(path.last(), Some(&(9000, 9000)));
    }

    #[test]
    fn a_drag_may_hold_ctrl_alt_and_shift_only() {
        assert_eq!(parse_hold("ctrl"), Ok(vec![VK_CONTROL]));
        assert_eq!(parse_hold("Ctrl + Shift"), Ok(vec![VK_CONTROL, VK_SHIFT]));
        assert_eq!(parse_hold("alt+alt"), Ok(vec![VK_MENU]));
        assert_eq!(parse_hold(""), Ok(vec![]));
        assert!(parse_hold("win").unwrap_err().contains("not allowed"));
        assert!(parse_hold("ctrl+cmd").unwrap_err().contains("not allowed"));
        assert!(parse_hold("ctrl+a").unwrap_err().contains("unknown key to hold"));
    }

    /// Run a drag against a recorder that fails at its `fail_at`th step (counting from 0).
    fn record_drag(points: &[(i32, i32)], mods: &[u16], fail_at: Option<usize>) -> (Result<(), String>, Vec<DragStep>) {
        let mut steps = Vec::new();
        let mut n = 0usize;
        let r = run_drag(points, mods, &mut |s| {
            let this = n;
            n += 1;
            // Only the steps of the drag itself can fail; the cleanup is tried regardless.
            if fail_at == Some(this) {
                return Err("input refused".into());
            }
            steps.push(s);
            Ok(())
        });
        (r, steps)
    }

    #[test]
    fn a_drag_presses_moves_and_lets_go_once() {
        let (r, steps) = record_drag(&[(1, 1), (5, 5), (9, 9)], &[VK_SHIFT], None);
        assert_eq!(r, Ok(()));
        assert_eq!(steps.first(), Some(&DragStep::Move(1, 1)));
        assert_eq!(steps.iter().filter(|s| **s == DragStep::ButtonDown).count(), 1);
        assert_eq!(steps.iter().filter(|s| **s == DragStep::ButtonUp).count(), 1);
        // The release is at the last point and the drop pause comes first.
        let up = steps.iter().position(|s| *s == DragStep::ButtonUp).unwrap();
        assert_eq!(steps[up - 3], DragStep::Move(9, 9));
        assert!(matches!(steps[up - 1], DragStep::Pause(ms) if ms >= 100));
        // The modifier goes down before the press and up after the release.
        let down = steps.iter().position(|s| *s == DragStep::ButtonDown).unwrap();
        assert!(steps.iter().position(|s| *s == DragStep::KeyDown(VK_SHIFT)).unwrap() < down);
        assert!(steps.iter().position(|s| *s == DragStep::KeyUp(VK_SHIFT)).unwrap() > up);
    }

    #[test]
    fn a_drag_that_fails_after_the_press_still_lets_go_of_everything() {
        let points = [(1, 1), (5, 5), (9, 9)];
        let mods = [VK_CONTROL, VK_SHIFT];
        // Fail at each step in turn: after the press, whatever was pressed is released.
        let (_, clean) = record_drag(&points, &mods, None);
        // The last steps are the modifiers coming up; only the drag proper can fail with an error.
        for fail_at in 0..clean.len() - mods.len() {
            let (r, steps) = record_drag(&points, &mods, Some(fail_at));
            assert_eq!(r, Err("input refused".into()), "fail_at {fail_at}");
            let downs = steps.iter().filter(|s| **s == DragStep::ButtonDown).count();
            let ups = steps.iter().filter(|s| **s == DragStep::ButtonUp).count();
            assert_eq!(downs, ups, "a button was left held when step {fail_at} failed: {steps:?}");
            for m in mods {
                let d = steps.iter().filter(|s| **s == DragStep::KeyDown(m)).count();
                let u = steps.iter().filter(|s| **s == DragStep::KeyUp(m)).count();
                assert_eq!(d, u, "{m:#x} was left held when step {fail_at} failed: {steps:?}");
            }
        }
    }

    #[test]
    fn modifiers_are_let_go_in_reverse_after_the_button() {
        let (r, steps) = record_drag(&[(0, 0), (3, 3)], &[VK_CONTROL, VK_SHIFT], Some(7));
        assert!(r.is_err());
        let tail: Vec<DragStep> = steps.iter().rev().take(3).rev().copied().collect();
        assert_eq!(tail, vec![DragStep::ButtonUp, DragStep::KeyUp(VK_SHIFT), DragStep::KeyUp(VK_CONTROL)], "{steps:?}");
    }

    #[test]
    fn a_drag_with_no_points_does_nothing() {
        let (r, steps) = record_drag(&[], &[VK_CONTROL], None);
        assert!(r.is_err());
        assert!(steps.is_empty());
    }

    #[test]
    fn looks_are_remembered_per_window() {
        let (first, before) = remember_look(7001, look());
        assert_eq!((first.n, before), (1, None));
        assert_eq!(last_look(7001), Some(first));
        assert_eq!(last_look(7002), None);
        // The next look of the same window is the second, and knows the first.
        let (second, before) = remember_look(7001, Look { image_h: 600, ..look() });
        assert_eq!(second.n, 2);
        assert_eq!(before, Some(first));
        assert_eq!(last_look(7001), Some(second));
    }

    #[test]
    fn coordinates_from_an_older_look_are_refused_and_say_which_is_latest() {
        let latest = Look { n: 3, ..look() };
        assert_eq!(check_of_look(&latest, 3), Ok(()));
        let e = check_of_look(&latest, 2).unwrap_err();
        assert!(e.contains("look #2") && e.contains("latest look at this window is #3") && e.contains("1000×500"), "{e}");
    }

    #[test]
    fn a_look_says_its_scale_and_when_the_size_changed() {
        let a = Look { n: 1, ..look() };
        let said = describe_scale(&a, None);
        assert!(said.contains("look #1") && said.contains("1000×500") && said.contains("2000×1000") && said.contains("scale 2.00"), "{said}");
        assert!(!said.contains("stale"));
        let b = Look { n: 2, image_h: 540, ..look() };
        let said = describe_scale(&b, Some(&a));
        assert!(said.contains("changed since look #1 (1000×500)") && said.contains("stale"), "{said}");
        assert!(!describe_scale(&b, Some(&Look { n: 1, ..b })).contains("stale"));
    }

    #[test]
    fn a_region_must_be_inside_the_look_and_big_enough() {
        let l = Look { n: 4, ..look() };
        assert_eq!(check_region(&l, 10, 20, 100, 50), Ok(Region { x: 10, y: 20, w: 100, h: 50 }));
        assert_eq!(check_region(&l, 900, 450, 100, 50), Ok(Region { x: 900, y: 450, w: 100, h: 50 }));
        assert!(check_region(&l, 901, 450, 100, 50).unwrap_err().contains("not inside look #4"));
        assert!(check_region(&l, -1, 0, 100, 50).is_err());
        assert!(check_region(&l, 0, 0, 7, 50).unwrap_err().contains("at least 8"));
        // Huge numbers from the caller are outside the image, never an overflow.
        assert!(check_region(&l, i64::MAX, 0, 8, 8).unwrap_err().contains("not inside"));
        assert!(check_region(&l, 0, i64::MAX, 8, 8).unwrap_err().contains("not inside"));
        assert!(check_region(&l, 0, 0, i64::MAX, 8).unwrap_err().contains("not inside"));
        assert!(check_region(&l, i64::MAX, i64::MAX, i64::MAX, i64::MAX).is_err());
    }

    #[test]
    fn a_small_region_is_enlarged_up_to_four_times_and_a_large_one_not_at_all() {
        assert_eq!(zoom_for(100, 50), MAX_ZOOM);
        assert_eq!(zoom_for(1568, 800), 1.0);
        assert_eq!(zoom_for(2000, 1000), 1.0);
        assert!((zoom_for(784, 100) - 2.0).abs() < 1e-9);
    }
}

/// A real drag, with the real mouse: a throwaway window with a slider and a
/// canvas, driven by `win::drag`. It takes the pointer and the screen for a few
/// seconds, so it is ignored by default; run it by hand on a machine nobody is
/// working at:
/// `cargo test -p agentmux-mcp real_drag -- --ignored --nocapture`.
#[cfg(all(test, windows))]
mod real_input {
    use super::*;
    use serde_json::Value;
    use std::path::Path;
    use std::process::{Child, Command};
    use std::time::{Duration, Instant};

    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL, VK_LBUTTON};

    /// A form with a slider (0 to 100) and a white canvas. It writes where they are
    /// (`ready.json`, in physical screen pixels) and what happened to them
    /// (`result.json`, after each press, release and slider change).
    const FORM: &str = r##"
param($dir)
Add-Type -TypeDefinition 'using System.Runtime.InteropServices; public class Dpi { [DllImport("user32.dll")] public static extern bool SetProcessDPIAware(); }'
[Dpi]::SetProcessDPIAware() | Out-Null
Add-Type -AssemblyName System.Windows.Forms
$f = New-Object Windows.Forms.Form
$f.StartPosition = 'Manual'; $f.Location = New-Object Drawing.Point(80, 80); $f.Size = New-Object Drawing.Size(640, 360); $f.TopMost = $true
$tb = New-Object Windows.Forms.TrackBar
$tb.Minimum = 0; $tb.Maximum = 100; $tb.Value = 0; $tb.Location = New-Object Drawing.Point(20, 20); $tb.Size = New-Object Drawing.Size(560, 45)
$p = New-Object Windows.Forms.Panel
$p.Location = New-Object Drawing.Point(20, 100); $p.Size = New-Object Drawing.Size(560, 180); $p.BackColor = [Drawing.Color]::White
$f.Controls.Add($tb); $f.Controls.Add($p)
$s = @{ slider = 0; downs = 0; ups = 0; moves = 0; ctrl_at_down = $false; first = @(0, 0); last = @(0, 0) }
function Save { ($s | ConvertTo-Json -Compress) | Set-Content -Path (Join-Path $dir 'result.json') -Encoding ASCII }
$tb.add_ValueChanged({ $s.slider = $tb.Value; Save })
$p.add_MouseDown({ param($o, $e) $s.downs++; $s.ctrl_at_down = ([Windows.Forms.Control]::ModifierKeys -band [Windows.Forms.Keys]::Control) -ne 0; $s.first = @($e.X, $e.Y); Save })
$p.add_MouseMove({ param($o, $e) if ($s.downs -gt $s.ups) { $s.moves++; $s.last = @($e.X, $e.Y) } })
$p.add_MouseUp({ param($o, $e) $s.ups++; $s.last = @($e.X, $e.Y); Save })
$f.add_Shown({
  $f.Activate()
  $a = $tb.PointToScreen((New-Object Drawing.Point(0, 0))); $b = $p.PointToScreen((New-Object Drawing.Point(0, 0)))
  $r = @{ slider = @($a.X, $a.Y, $tb.Width, $tb.Height); canvas = @($b.X, $b.Y, $p.Width, $p.Height) }
  ($r | ConvertTo-Json -Compress) | Set-Content -Path (Join-Path $dir 'ready.json') -Encoding ASCII
})
[Windows.Forms.Application]::Run($f)
"##;

    struct Form(Child);
    impl Drop for Form {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }

    fn wait_for_json(path: &Path, secs: u64) -> Value {
        let end = Instant::now() + Duration::from_secs(secs);
        while Instant::now() < end {
            if let Ok(text) = std::fs::read_to_string(path) {
                if let Ok(v) = serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) {
                    return v;
                }
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        panic!("{} did not appear", path.display());
    }

    /// What the form recorded, once it has had time to see the last event.
    fn settled_result(dir: &Path) -> Value {
        wait_for_json(&dir.join("result.json"), 5);
        std::thread::sleep(Duration::from_millis(700));
        wait_for_json(&dir.join("result.json"), 5)
    }

    fn rect(v: &Value, key: &str) -> (i32, i32, i32, i32) {
        let a = v[key].as_array().unwrap_or_else(|| panic!("{key} missing in {v}"));
        let n = |i: usize| a[i].as_i64().unwrap() as i32;
        (n(0), n(1), n(2), n(3))
    }

    fn down(vk: u16) -> bool {
        unsafe { GetAsyncKeyState(vk as i32) as u16 & 0x8000 != 0 }
    }

    #[test]
    #[ignore = "takes the real mouse and screen for a few seconds; run by hand"]
    fn real_drag_moves_a_slider_and_draws_on_a_canvas() {
        win::ensure_dpi_aware();
        let dir = std::env::temp_dir().join(format!("am-real-drag-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("form.ps1");
        std::fs::write(&script, FORM).unwrap();
        let child = Command::new("powershell")
            .args(["-NoProfile", "-STA", "-ExecutionPolicy", "Bypass", "-File"])
            .arg(&script)
            .arg(&dir)
            .spawn()
            .expect("start powershell");
        let _form = Form(child);
        let ready = wait_for_json(&dir.join("ready.json"), 30);
        std::thread::sleep(Duration::from_millis(800));

        // The slider: press on its thumb (at the left end) and drag to the right end.
        let (sx, sy, sw, sh) = rect(&ready, "slider");
        let from = (sx + 14, sy + sh / 3);
        let to = (sx + sw - 14, sy + sh / 3);
        win::drag(&drag_path(from, &[], to), "left", &[]).expect("drag the slider");
        let r = settled_result(&dir);
        assert!(r["slider"].as_i64().unwrap() >= 90, "the slider only reached {}: {r}", r["slider"]);
        assert!(!down(VK_LBUTTON as u16), "the left button is still held");

        // The canvas: a drag through a waypoint, holding ctrl.
        let _ = std::fs::remove_file(dir.join("result.json"));
        let (cx, cy, cw, ch) = rect(&ready, "canvas");
        let start = (cx + 30, cy + 30);
        let mid = (cx + cw / 2, cy + ch - 30);
        let end = (cx + cw - 30, cy + 30);
        win::drag(&drag_path(start, &[mid], end), "left", &[0x11]).expect("drag on the canvas");
        let r = settled_result(&dir);
        assert_eq!(r["downs"].as_i64(), Some(1), "{r}");
        assert_eq!(r["ups"].as_i64(), Some(1), "{r}");
        assert!(r["moves"].as_i64().unwrap() >= 20, "{r}");
        assert_eq!(r["ctrl_at_down"].as_bool(), Some(true), "ctrl was not held at the press: {r}");
        let last = (r["last"][0].as_i64().unwrap(), r["last"][1].as_i64().unwrap());
        assert!((last.0 - (cw as i64 - 30)).abs() <= 3 && (last.1 - 30).abs() <= 3, "released at {last:?}: {r}");
        assert!(!down(VK_LBUTTON as u16), "the left button is still held");
        assert!(!down(VK_CONTROL as u16), "ctrl is still held");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
