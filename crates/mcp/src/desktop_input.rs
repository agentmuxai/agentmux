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
}

static LOOKS: Mutex<Option<HashMap<u32, Look>>> = Mutex::new(None);

/// Remember what `look` showed of window `id`.
pub(crate) fn remember_look(id: u32, look: Look) {
    let mut g = LOOKS.lock().unwrap_or_else(|e| e.into_inner());
    g.get_or_insert_with(HashMap::new).insert(id, look);
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
        OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION,
    };
    use windows_sys::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::*;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        EnumChildWindows, GetAncestor, GetForegroundWindow, GetSystemMetrics, GetWindowRect, GetWindowThreadProcessId,
        IsIconic, SetForegroundWindow, ShowWindow, WindowFromPoint,
        GA_ROOT, GA_ROOTOWNER, SM_CXVIRTUALSCREEN, SM_CYVIRTUALSCREEN, SM_XVIRTUALSCREEN, SM_YVIRTUALSCREEN, SW_RESTORE,
    };

    use super::{to_absolute, Chord, Rect};

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

    /// Whether `hwnd` is the foreground window, after one try at bringing it
    /// there (restoring it if minimised).
    pub(crate) fn bring_to_front(id: u32) -> Result<(), String> {
        ensure_dpi_aware();
        let hwnd = id as usize as HWND;
        unsafe {
            if GetForegroundWindow() == hwnd {
                return Ok(());
            }
            if IsIconic(hwnd) != 0 {
                ShowWindow(hwnd, SW_RESTORE);
            }
            SetForegroundWindow(hwnd);
        }
        std::thread::sleep(Duration::from_millis(150));
        if unsafe { GetForegroundWindow() } == hwnd {
            Ok(())
        } else {
            Err("could not bring the window to the front; another window has the focus".into())
        }
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

    /// Click `count` times at a screen point with `button` ("left", "right",
    /// "middle").
    pub(crate) fn click(sx: i32, sy: i32, button: &str, count: u32) -> Result<(), String> {
        let (down, up) = match button {
            "left" => (MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP),
            "right" => (MOUSEEVENTF_RIGHTDOWN, MOUSEEVENTF_RIGHTUP),
            "middle" => (MOUSEEVENTF_MIDDLEDOWN, MOUSEEVENTF_MIDDLEUP),
            other => return Err(format!("unknown button {other:?}; use left, right or middle")),
        };
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
        Look { rect: Rect { x: 100, y: 50, w: 2000, h: 1000 }, image_w: 1000, image_h: 500 }
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
    fn looks_are_remembered_per_window() {
        remember_look(7001, look());
        assert_eq!(last_look(7001), Some(look()));
        assert_eq!(last_look(7002), None);
    }
}
