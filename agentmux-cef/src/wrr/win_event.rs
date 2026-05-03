// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0
//
// Phase B.9.1 — `SetWinEventHook` installation + callback.
//
// One install at startup, one teardown at shutdown. The hook
// runs WINEVENT_OUTOFCONTEXT (callback fires on the calling
// thread of the hooking process — no DLL injection, no extra
// thread). We filter to the host's own PID via the `idProcess`
// parameter so we don't see events from other processes.
//
// The callback fires synchronously per OS event. It:
//   1. Filters out non-window object events (OBJID != OBJID_WINDOW
//      or CHILDID != CHILDID_SELF).
//   2. Reads the HWND's class name; bails if not in
//      `classify::is_app_class`.
//   3. Maps the event ID to a launcher_ipc::report_hwnd_*
//      function and dispatches.
//
// The `report_hwnd_*` functions on the launcher_ipc side are
// non-blocking sync APIs (UnboundedSender → drain task) so this
// callback returns quickly even under burst load.

use std::sync::{Arc, OnceLock};

use windows_sys::Win32::Foundation::{HWND, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    EnumDisplayMonitors, GetMonitorInfoW, HDC, HMONITOR, MONITORINFO,
};
use windows_sys::Win32::UI::Accessibility::{
    SetWinEventHook, UnhookWinEvent, HWINEVENTHOOK,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    GetClassNameW, GetWindowRect, GetWindowTextW, GetWindowThreadProcessId, IsIconic,
    IsWindowVisible, CHILDID_SELF, EVENT_OBJECT_CREATE, EVENT_OBJECT_DESTROY, EVENT_OBJECT_HIDE,
    EVENT_OBJECT_LOCATIONCHANGE, EVENT_OBJECT_SHOW, EVENT_SYSTEM_FOREGROUND,
    EVENT_SYSTEM_MINIMIZEEND, EVENT_SYSTEM_MINIMIZESTART, OBJID_WINDOW, WINEVENT_OUTOFCONTEXT,
};

use crate::launcher_ipc;
use crate::state::AppState;
use crate::wrr::{classify, position_debounce};

/// Phase B.9.1 — handle to host AppState for the static
/// `SetWinEventHook` callback (the OS API takes a fixed-shape
/// `extern "system" fn`, so AppState has to be reachable through
/// a static). Set once by `install_hooks(state)`. The callback
/// reads `pending_window_creations` to supply `label_hint` on
/// `EVENT_OBJECT_CREATE`, eliminating the launcher-side
/// "pending HWND with no matching mirror" race.
fn app_state() -> &'static OnceLock<Arc<AppState>> {
    static S: OnceLock<Arc<AppState>> = OnceLock::new();
    &S
}


/// Phase B.9.1 — handles to the installed hooks. Held in a
/// `OnceLock<Mutex<Option<...>>>` so install_hooks is idempotent
/// (no-op on second call) and uninstall can drop them on shutdown.
fn hook_handles() -> &'static std::sync::Mutex<Vec<HookHandle>> {
    static H: OnceLock<std::sync::Mutex<Vec<HookHandle>>> = OnceLock::new();
    H.get_or_init(|| std::sync::Mutex::new(Vec::new()))
}

#[derive(Debug)]
struct HookHandle(HWINEVENTHOOK);

// SAFETY: HWINEVENTHOOK is a HANDLE (opaque pointer). Send + Sync
// are needed to store it in a static Mutex; the Win32 API doesn't
// document any thread-affinity for the handle itself (Unhook can
// be called from any thread).
unsafe impl Send for HookHandle {}
unsafe impl Sync for HookHandle {}

/// Phase B.9.1 — install the WRR hooks. Idempotent (logs and
/// returns if hooks are already installed). Must be called from a
/// thread that owns a Win32 message pump — the host's CEF UI
/// thread does, so we install from there at startup.
///
/// Also enumerates the current monitor topology and reports it
/// once. WM_DISPLAYCHANGE wiring for mid-session topology updates
/// is a B.9.2 follow-up.
///
/// `state` is stashed in a static `OnceLock` so the static callback
/// can read `pending_window_creations` to supply `label_hint`.
pub fn install_hooks(state: Arc<AppState>) {
    if app_state().set(state).is_err() {
        tracing::debug!("[wrr] install_hooks called twice — already installed (state set)");
    }
    let mut handles = match hook_handles().lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    if !handles.is_empty() {
        tracing::debug!("[wrr] install_hooks called twice — already installed");
        return;
    }

    let pid = std::process::id();

    // Hook range 1: object create / destroy / show / hide.
    // EVENT_OBJECT_CREATE..=EVENT_OBJECT_HIDE happens to be a
    // contiguous range (0x8000..=0x8003); install one hook with
    // bracketing endpoints.
    let h1 = install_one(EVENT_OBJECT_CREATE, EVENT_OBJECT_HIDE, pid);
    if let Some(h) = h1 {
        handles.push(HookHandle(h));
    }

    // Hook range 2: foreground change. Discrete event; installed
    // as a degenerate range (start == end).
    let h2 = install_one(EVENT_SYSTEM_FOREGROUND, EVENT_SYSTEM_FOREGROUND, pid);
    if let Some(h) = h2 {
        handles.push(HookHandle(h));
    }

    // Hook range 3: minimize start / end.
    let h3 = install_one(EVENT_SYSTEM_MINIMIZESTART, EVENT_SYSTEM_MINIMIZEEND, pid);
    if let Some(h) = h3 {
        handles.push(HookHandle(h));
    }

    // Hook range 4: location change (debounced — see
    // position_debounce.rs). Discrete event.
    let h4 = install_one(
        EVENT_OBJECT_LOCATIONCHANGE,
        EVENT_OBJECT_LOCATIONCHANGE,
        pid,
    );
    if let Some(h) = h4 {
        handles.push(HookHandle(h));
    }

    let installed_count = handles.len();
    drop(handles);

    tracing::info!(
        "[wrr] installed {}/{} WinEventHook range(s) for pid={}",
        installed_count, 4, pid
    );

    // Enumerate the initial monitor topology and report it once.
    // Without this, `state.monitors` stays empty in the launcher
    // and `OffMonitor` drift is suppressed (per the design doc).
    let monitors = enumerate_monitors();
    if !monitors.is_empty() {
        launcher_ipc::report_monitor_topology_changed(monitors.clone());
        tracing::info!("[wrr] reported initial monitor topology: {} monitor(s)", monitors.len());
    } else {
        tracing::warn!("[wrr] initial monitor enumeration returned 0 monitors — OffMonitor drift will be suppressed");
    }
}

fn install_one(event_min: u32, event_max: u32, pid: u32) -> Option<HWINEVENTHOOK> {
    unsafe {
        let h = SetWinEventHook(
            event_min,
            event_max,
            std::ptr::null_mut(),
            Some(win_event_callback),
            pid,
            0, // idThread = 0 means all threads in the process
            WINEVENT_OUTOFCONTEXT,
        );
        if h.is_null() {
            tracing::error!(
                "[wrr] SetWinEventHook failed for range {:#x}..={:#x}",
                event_min,
                event_max
            );
            None
        } else {
            Some(h)
        }
    }
}

/// Phase B.9.1 — uninstall the hooks. Called from host shutdown.
pub fn uninstall_hooks() {
    let mut handles = match hook_handles().lock() {
        Ok(g) => g,
        Err(poisoned) => poisoned.into_inner(),
    };
    let mut count = 0;
    for h in handles.drain(..) {
        unsafe {
            if UnhookWinEvent(h.0) != 0 {
                count += 1;
            }
        }
    }
    if count > 0 {
        tracing::info!("[wrr] unhooked {} WinEventHook range(s)", count);
    }
}

/// Phase B.9.1 — hook callback. Runs OUT-OF-CONTEXT (on the
/// hooking thread, posted via the message pump). Must not block
/// for long: we only do quick HWND-property reads and dispatch
/// non-blocking sync IPC reports.
unsafe extern "system" fn win_event_callback(
    _hook: HWINEVENTHOOK,
    event: u32,
    hwnd: HWND,
    id_object: i32,
    id_child: i32,
    _id_event_thread: u32,
    _dwms_event_time: u32,
) {
    // 1. Only OBJID_WINDOW with CHILDID_SELF — events for child
    // controls and accessibility objects are noise.
    if id_object != OBJID_WINDOW || id_child != CHILDID_SELF as i32 {
        return;
    }
    if hwnd.is_null() {
        return;
    }

    // 2. Confirm the HWND belongs to our process (defense-in-
    // depth — `idProcess` filter on the hook should already
    // restrict this, but the OS occasionally bubbles events for
    // ancestor / desktop windows).
    let mut hwnd_pid: u32 = 0;
    GetWindowThreadProcessId(hwnd, &mut hwnd_pid);
    if hwnd_pid != std::process::id() {
        return;
    }

    let raw_hwnd = hwnd as u64;

    // Phase B.9.1 — diagnostic. INFO-level for B.9.1 smoke;
    // dial back to debug! once we've confirmed the chain is
    // wired end-to-end. Volume is bounded by the OBJID_WINDOW
    // + CHILDID_SELF + idProcess filters above; for normal use
    // this fires at ~5-20/sec during user activity.
    tracing::info!(
        target: "wrr",
        "[wrr] callback event=0x{:x} hwnd={:#x}",
        event, raw_hwnd
    );

    // 3. Per-event dispatch. Reads HWND properties as needed.
    match event {
        EVENT_OBJECT_CREATE => {
            let class = read_class_name(hwnd);
            // Cheap filter at the hook so we don't IPC every
            // tooltip / IME / message-only window.
            if !classify::is_app_class(&class) || classify::is_explicitly_excluded(&class) {
                return;
            }
            let title = read_window_text(hwnd);
            // Phase B.9.1 diagnostic — log app-class creates so
            // smoke can correlate with launcher-side reception.
            tracing::info!(
                target: "wrr",
                "[wrr] EVENT_OBJECT_CREATE app-class hwnd={:#x} class={} title={:?}",
                raw_hwnd, class, title
            );
            // PR — drop back-of-queue label_hint optimization.
            //
            // The original B.9.1 design peeked the back of
            // `pending_window_creations` to label this OS-level
            // WM_CREATE event. That assumed at most one create in
            // flight at any time. When users click "open new window"
            // multiple times in succession, multiple pending entries
            // queue up, and back-of-queue returns the LATEST label
            // for EVERY in-flight WM_CREATE — mislabeling every HWND
            // that arrives before its corresponding `on_after_created`.
            //
            // Worse: the launcher's drain-on-WindowOpened fallback in
            // `handle_report_window_opened` (launcher reducer.rs) only
            // drains pending HWNDs whose `label_hint.is_none()`. A
            // wrong hint actively HIJACKS the fallback — the launcher
            // sees `label_hint=Some(WRONG)`, doesn't match any
            // existing mirror, stashes a wrong-labeled pending entry,
            // and the fallback never runs because of the is_none()
            // filter. Result: aliased mirror entries (multiple labels
            // pointing to the same HWND in launcher state, or vice
            // versa), `HwndWithoutBrowser` drift errors, and the
            // user-visible bug "InstancePanel grows but no window
            // appears; closing one window collapses multiple panel
            // entries."
            //
            // Always passing None routes EVERY HWND through the
            // launcher's drain-on-WindowOpened fallback, which matches
            // the most-recent unlinked pending HWND when the
            // authoritative `ReportWindowOpened` arrives from
            // `on_after_created`. Same path pool windows already used.
            //
            // See `docs/retro/wrr-label-hint-race-2026-05-02.md` for
            // full diagnosis from the 0.33.589 smoke session.
            launcher_ipc::report_hwnd_opened(raw_hwnd, class, title, None);

            // Fire synthetic position + visibility events after
            // create so the reducer has a complete state for this
            // HWND right away (LOCATIONCHANGE wouldn't fire on a
            // window that lands at its final position immediately).
            if let Some(rect) = read_window_rect(hwnd) {
                if position_debounce::should_emit(raw_hwnd) {
                    launcher_ipc::report_hwnd_position_changed(raw_hwnd, rect);
                }
            }
            let visible = IsWindowVisible(hwnd) != 0;
            launcher_ipc::report_hwnd_visibility_changed(raw_hwnd, visible);
            let iconic = IsIconic(hwnd) != 0;
            launcher_ipc::report_hwnd_iconic_changed(raw_hwnd, iconic);
        }
        EVENT_OBJECT_DESTROY => {
            // No class-name filter on destroy — the HWND may be
            // mid-teardown so its class is unreliable. The
            // launcher reducer handles "destroy of an unknown
            // HWND" as a no-op via pending_hwnds + windows
            // membership check.
            position_debounce::forget(raw_hwnd);
            launcher_ipc::report_hwnd_destroyed(raw_hwnd);
        }
        EVENT_OBJECT_SHOW => {
            let class = read_class_name(hwnd);
            if !classify::is_app_class(&class) {
                return;
            }
            launcher_ipc::report_hwnd_visibility_changed(raw_hwnd, true);
        }
        EVENT_OBJECT_HIDE => {
            let class = read_class_name(hwnd);
            if !classify::is_app_class(&class) {
                return;
            }
            launcher_ipc::report_hwnd_visibility_changed(raw_hwnd, false);
        }
        EVENT_SYSTEM_FOREGROUND => {
            let class = read_class_name(hwnd);
            if !classify::is_app_class(&class) {
                return;
            }
            launcher_ipc::report_hwnd_foreground_changed(raw_hwnd);
        }
        EVENT_SYSTEM_MINIMIZESTART => {
            let class = read_class_name(hwnd);
            if !classify::is_app_class(&class) {
                return;
            }
            launcher_ipc::report_hwnd_iconic_changed(raw_hwnd, true);
        }
        EVENT_SYSTEM_MINIMIZEEND => {
            let class = read_class_name(hwnd);
            if !classify::is_app_class(&class) {
                return;
            }
            launcher_ipc::report_hwnd_iconic_changed(raw_hwnd, false);
        }
        EVENT_OBJECT_LOCATIONCHANGE => {
            // Heavy event during drags — debounce per HWND.
            if !position_debounce::should_emit(raw_hwnd) {
                return;
            }
            let class = read_class_name(hwnd);
            if !classify::is_app_class(&class) {
                return;
            }
            if let Some(rect) = read_window_rect(hwnd) {
                launcher_ipc::report_hwnd_position_changed(raw_hwnd, rect);
            }
        }
        _ => {}
    }
}

/// Read the Win32 class name into an owned `String`.
unsafe fn read_class_name(hwnd: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = GetClassNameW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
    if n <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..n as usize])
}

/// Read the window's text (title) into an owned `String`. Empty
/// string if no title.
unsafe fn read_window_text(hwnd: HWND) -> String {
    let mut buf = [0u16; 512];
    let n = GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32);
    if n <= 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buf[..n as usize])
}

/// Read the window's screen-coordinate rectangle. `None` if the
/// API call fails (window torn down between our event and the
/// read — common during destroy).
unsafe fn read_window_rect(hwnd: HWND) -> Option<agentmux_common::ipc::Rect> {
    let mut r: RECT = std::mem::zeroed();
    if GetWindowRect(hwnd, &mut r) == 0 {
        return None;
    }
    Some(agentmux_common::ipc::Rect {
        left: r.left,
        top: r.top,
        right: r.right,
        bottom: r.bottom,
    })
}

/// Phase B.9.1 — initial monitor enumeration. Called once from
/// install_hooks. Mid-session topology changes (a follow-up
/// concern for B.9.2) would re-enumerate via WM_DISPLAYCHANGE.
fn enumerate_monitors() -> Vec<agentmux_common::ipc::Rect> {
    use std::cell::RefCell;
    thread_local! {
        // Per-thread accumulator so the callback below can push
        // into it without locking. EnumDisplayMonitors is
        // synchronous — we drain after it returns.
        static MONITORS: RefCell<Vec<agentmux_common::ipc::Rect>> =
            RefCell::new(Vec::new());
    }

    unsafe extern "system" fn enum_proc(
        h: HMONITOR,
        _hdc: HDC,
        _rect: *mut RECT,
        _data: windows_sys::Win32::Foundation::LPARAM,
    ) -> windows_sys::Win32::Foundation::BOOL {
        let mut info: MONITORINFO = std::mem::zeroed();
        info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;
        if GetMonitorInfoW(h, &mut info) != 0 {
            MONITORS.with(|m| {
                m.borrow_mut().push(agentmux_common::ipc::Rect {
                    left: info.rcMonitor.left,
                    top: info.rcMonitor.top,
                    right: info.rcMonitor.right,
                    bottom: info.rcMonitor.bottom,
                });
            });
        }
        1 // continue enumeration
    }

    MONITORS.with(|m| m.borrow_mut().clear());
    unsafe {
        EnumDisplayMonitors(std::ptr::null_mut(), std::ptr::null(), Some(enum_proc), 0);
    }
    MONITORS.with(|m| m.borrow().clone())
}
