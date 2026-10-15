// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! Diagnostics for input that goes somewhere other than where it should, so a
//! "the wheel / typing stopped working" report can be read from the log alone.
//! A thread-local `WH_GETMESSAGE` hook on the UI thread sees every message the
//! thread takes off its queue; it samples the wheel, clicks and key presses (at
//! most one of each kind per `SAMPLE_EVERY_MS`) and posts the check to a UI
//! task, taking no lock itself. A line is
//! logged only when something disagrees, at most once per `LOG_EVERY_MS` per
//! kind. No key codes are read. docs/specs/SPEC_BROWSER_PANE_INPUT_OWNER_2026_10_15.md §3.5.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, OnceLock};

use cef::*;
use windows_sys::Win32::Foundation::{HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, GetAncestor, GetClassNameW, GetForegroundWindow, GetWindowRect, IsWindowVisible,
    SetWindowsHookExW, WindowFromPoint, GA_ROOT, HC_ACTION, MSG, PM_REMOVE, WH_GETMESSAGE, WM_KEYDOWN,
    WM_LBUTTONDOWN, WM_MOUSEHWHEEL, WM_MOUSEWHEEL, WM_SYSKEYDOWN,
};

use crate::state::{AppState, BrowserPaneLifecycle};

use super::machine::{Have, Surface};
use super::registry;

/// At most one sample of each kind per this many ms reaches the check.
const SAMPLE_EVERY_MS: u64 = 250;
/// At most one line of each kind per this many ms; the rest are counted.
const LOG_EVERY_MS: u64 = 5000;

static STATE: OnceLock<Arc<AppState>> = OnceLock::new();
static INSTALLED: AtomicBool = AtomicBool::new(false);

/// What a sample is of, and its own sampling and logging clocks.
#[derive(Clone, Copy)]
enum Kind {
    Wheel,
    Click,
    Key,
}

impl Kind {
    fn name(self) -> &'static str {
        match self {
            Kind::Wheel => "wheel",
            Kind::Click => "click",
            Kind::Key => "keys",
        }
    }
    fn index(self) -> usize {
        self as usize
    }
}

static LAST_SAMPLE: [AtomicU64; 3] = [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];
static LAST_LOG: [AtomicU64; 3] = [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];
static SUPPRESSED: [AtomicU64; 3] = [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];

/// Install the hook on the calling thread, which must be the UI thread. Once.
pub fn install(state: &Arc<AppState>) {
    if INSTALLED.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = STATE.set(state.clone());
    let thread = unsafe { windows_sys::Win32::System::Threading::GetCurrentThreadId() };
    let hook = unsafe { SetWindowsHookExW(WH_GETMESSAGE, Some(message_hook), std::ptr::null_mut(), thread) };
    tracing::info!("[input-watch] hook installed: {}", !hook.is_null());
}

/// Whether a sample of `kind` is due now; takes the slot if so.
fn take_sample(kind: Kind) -> bool {
    let now = registry::monotonic_ms().max(1);
    let last = &LAST_SAMPLE[kind.index()];
    let prev = last.load(Ordering::Relaxed);
    now.saturating_sub(prev) >= SAMPLE_EVERY_MS && last.compare_exchange(prev, now, Ordering::Relaxed, Ordering::Relaxed).is_ok()
}

unsafe extern "system" fn message_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    // Only a message being taken off the queue, not one peeked at and left there.
    if code == HC_ACTION as i32 && wparam == PM_REMOVE as usize && lparam != 0 {
        let msg = &*(lparam as *const MSG);
        match msg.message {
            WM_MOUSEWHEEL | WM_MOUSEHWHEEL if take_sample(Kind::Wheel) => {
                // The wheel's lParam is the pointer in screen coordinates.
                let pt = POINT { x: (msg.lParam & 0xFFFF) as u16 as i16 as i32, y: ((msg.lParam >> 16) & 0xFFFF) as u16 as i16 as i32 };
                post(Sample::Pointer { kind: Kind::Wheel, pt, target: msg.hwnd as isize });
            }
            WM_LBUTTONDOWN if take_sample(Kind::Click) => {
                post(Sample::Pointer { kind: Kind::Click, pt: msg.pt, target: msg.hwnd as isize });
            }
            WM_KEYDOWN | WM_SYSKEYDOWN if take_sample(Kind::Key) => {
                post(Sample::Key { focus: msg.hwnd as isize, foreground: GetForegroundWindow() as isize });
            }
            _ => {}
        }
    }
    CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
}

enum Sample {
    Pointer { kind: Kind, pt: POINT, target: isize },
    Key { focus: isize, foreground: isize },
}

fn post(sample: Sample) {
    let Some(state) = STATE.get() else { return };
    let mut task = CheckTask::new(state.clone(), Arc::new(sample));
    post_task(ThreadId::UI, Some(&mut task));
}

wrap_task! {
    struct CheckTask {
        state: Arc<AppState>,
        sample: Arc<Sample>,
    }
    impl Task {
        fn execute(&self) {
            match *self.sample {
                Sample::Pointer { kind, pt, target } => check_pointer(&self.state, kind, pt, target as HWND),
                Sample::Key { focus, foreground } => check_keys(&self.state, focus as HWND, foreground as HWND),
            }
        }
    }
}

/// A wheel or click at `pt` that Windows gave to `target`: wrong if it was over
/// a visible pane and went elsewhere, or went to a pane it wasn't over.
fn check_pointer(state: &Arc<AppState>, kind: Kind, pt: POINT, target: HWND) {
    let under = pane_at(state, pt);
    let got = crate::browser_pane::hwnd::pane_block_for_hwnd(target);
    if under.is_none() && got.is_none() {
        return;
    }
    if let (Some((under, _)), Some(got)) = (&under, &got) {
        if under == got {
            return;
        }
    }
    let window = unsafe { root_label(state, target) }.or_else(|| under.as_ref().map(|(_, w)| w.clone()));
    let at_point = describe_hwnd(unsafe { WindowFromPoint(pt) });
    let over = under.map_or("no pane".to_string(), |(b, _)| format!("pane:{}", short(&b)));
    let went = got.map_or_else(|| describe_hwnd(target), |b| format!("pane:{}", short(&b)));
    report(
        kind,
        format!(
            "{} over {over} went to {went}; at the point: {at_point}; input owner: {}",
            kind.name(),
            owner_state(window.as_deref()),
        ),
    );
}

/// Keys sent to `focus` (the window with the keyboard): wrong if the input owner of the
/// window in front believes someone else has it, isn't active though the window
/// is in front, or nothing of that window has the keyboard.
fn check_keys(state: &Arc<AppState>, focus: HWND, foreground: HWND) {
    let Some(window) = (unsafe { root_label(state, foreground) }) else { return };
    let surface = if focus.is_null() || unsafe { GetAncestor(focus, GA_ROOT) != GetAncestor(foreground, GA_ROOT) } {
        Surface::Elsewhere
    } else {
        match crate::browser_pane::hwnd::pane_block_for_hwnd(focus) {
            Some(b) => Surface::Pane(b),
            None => Surface::App,
        }
    };
    let Some((active, have, described)) = registry::snapshot(&window) else { return };
    let problem = if !active {
        "the window is in front but its input owner counts it inactive"
    } else if surface == Surface::Elsewhere {
        "no window of it has the keyboard"
    } else if matches!(&have, Have::Owned(s) if *s != surface) {
        "the input owner believes someone else has the keyboard"
    } else {
        return;
    };
    report(Kind::Key, format!("keys in win={window} went to {surface} ({problem}); input owner: {described}"));
}

fn report(kind: Kind, line: String) {
    let now = registry::monotonic_ms().max(1);
    let last = &LAST_LOG[kind.index()];
    if now.saturating_sub(last.load(Ordering::Relaxed)) < LOG_EVERY_MS {
        SUPPRESSED[kind.index()].fetch_add(1, Ordering::Relaxed);
        return;
    }
    last.store(now, Ordering::Relaxed);
    let more = SUPPRESSED[kind.index()].swap(0, Ordering::Relaxed);
    let more = if more > 0 { format!(" (+{more} more since the last line)") } else { String::new() };
    tracing::warn!("[input-watch] {line}{more}");
}

/// The visible, live pane whose window contains `pt`, and its window label.
/// Only panes of the top-level window that is on top at `pt` count: a floating
/// window over the main one, or another app, hides the panes beneath. A window
/// inside the same top-level covering a pane doesn't, so that is still reported.
fn pane_at(state: &Arc<AppState>, pt: POINT) -> Option<(String, String)> {
    let at = unsafe { WindowFromPoint(pt) };
    if at.is_null() {
        return None;
    }
    let top = unsafe { GetAncestor(at, GA_ROOT) };
    let panes: Vec<(String, String, String)> = state
        .host_state
        .lock()
        .browser_panes
        .values()
        .filter(|e| e.lifecycle == BrowserPaneLifecycle::Live)
        .map(|e| (e.block_id.clone(), e.label.clone(), e.window_label.clone()))
        .collect();
    panes.into_iter().find_map(|(block, label, window)| {
        let wrapper = crate::browser_pane::wrapper::peek_wrapper_hwnd(&label)? as HWND;
        let mut r = RECT { left: 0, top: 0, right: 0, bottom: 0 };
        let inside = unsafe {
            GetAncestor(wrapper, GA_ROOT) == top && IsWindowVisible(wrapper) != 0 && GetWindowRect(wrapper, &mut r) != 0
        }
            && pt.x >= r.left
            && pt.x < r.right
            && pt.y >= r.top
            && pt.y < r.bottom;
        inside.then_some((block, window))
    })
}

unsafe fn root_label(state: &Arc<AppState>, hwnd: HWND) -> Option<String> {
    if hwnd.is_null() {
        return None;
    }
    state.label_for_hwnd(GetAncestor(hwnd, GA_ROOT))
}

fn owner_state(window: Option<&str>) -> String {
    match window.and_then(|w| registry::snapshot(w).map(|(_, _, d)| format!("win={w} {d}"))) {
        Some(s) => s,
        None => "unknown window".to_string(),
    }
}

/// A window as `0x… class=… (pane:… | app window | other window)`.
fn describe_hwnd(hwnd: HWND) -> String {
    if hwnd.is_null() {
        return "no window".to_string();
    }
    let mut class = [0u16; 64];
    let n = unsafe { GetClassNameW(hwnd, class.as_mut_ptr(), class.len() as i32) };
    let class = String::from_utf16_lossy(&class[..n.max(0) as usize]);
    let whose = match crate::browser_pane::hwnd::pane_block_for_hwnd(hwnd) {
        Some(b) => format!("pane:{}", short(&b)),
        None => match STATE.get().and_then(|s| unsafe { root_label(s, hwnd) }) {
            Some(w) => format!("win={w}"),
            None => "another app's window".to_string(),
        },
    };
    format!("{hwnd:p} class={class} ({whose})")
}

fn short(block: &str) -> &str {
    block.get(..8).unwrap_or(block)
}
