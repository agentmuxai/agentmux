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
// ── B1: last-user-window teardown trigger (Discussion #1680) ──────────────────
//
// The Views main window does NOT fire `on_before_close` when closed — closing
// HIDES/recycles it (warm pool) rather than destroying the browser, so the
// host's CEF-lifecycle quit never starts and the launcher (gated on host exit
// via host_child.wait → Job Object KILL_ON_JOB_CLOSE) never reaps the tree.
// Instead we trigger teardown off the RELIABLE OS signal: when the last
// user-visible top-level window goes away, quit the message loop → host exits →
// launcher reaps. This callback runs on the CEF UI thread (install_hooks is
// called immediately before run_message_loop on the main thread, and
// WINEVENT_OUTOFCONTEXT callbacks are delivered on the hook-installing thread's
// message pump), so `quit_message_loop()` — the canonical, reliable exit
// (unlike PostThreadMessage(WM_QUIT), which CEF's pump ignores, or post_task,
// which drops during teardown) — is safe to call here.

/// Set true once any user-visible top-level window has been shown — prevents a
/// startup-time `count == 0` (before the first window appears) from quitting.
static HAD_VISIBLE_USER_WINDOW: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
/// Set true once the quit has been initiated, so a flurry of HIDE/DESTROY
/// events can't call `quit_message_loop()` more than once.
static QUIT_INITIATED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);

/// X-coordinate below which a top-level window is an off-screen warm-pool member
/// (created at -32000), not a real user window. Mirrors
/// `commands::window::lifecycle::OFFSCREEN_POOL_THRESHOLD_X`.
const OFFSCREEN_POOL_THRESHOLD_X: i32 = -20000;

/// Count this process's user-visible top-level windows: `IsWindowVisible`,
/// app-class (`Chrome_WidgetWin_*`; floaters use a different class and are
/// excluded), and on-screen (off-screen warm-pool windows excluded). A promoted
/// pool window counts (on-screen + app-class); a hidden/recycled or off-screen
/// pool window does not. Minimized windows remain `IsWindowVisible` and count
/// (minimize ≠ close).
unsafe fn count_visible_user_windows() -> usize {
    use windows_sys::Win32::Foundation::{BOOL, LPARAM, RECT};
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::{EnumWindows, GetWindowRect};

    struct Ctx {
        pid: u32,
        count: usize,
    }
    unsafe extern "system" fn enum_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let ctx = &mut *(lparam as *mut Ctx);
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid != ctx.pid {
            return 1;
        }
        if IsWindowVisible(hwnd) == 0 {
            return 1;
        }
        if !classify::is_app_class(&read_class_name(hwnd)) {
            return 1;
        }
        // A MINIMIZED window's GetWindowRect also reports (-32000, -32000) on
        // Win32, so guard the off-screen pool exclusion with IsIconic: a
        // minimized user window stays counted (minimize ≠ close — else
        // minimizing the last window + any HIDE churn would quit the instance,
        // reagent P1 #1676); only a NON-minimized off-screen window is a
        // warm-pool member.
        if IsIconic(hwnd) == 0 {
            let mut rect: RECT = std::mem::zeroed();
            if GetWindowRect(hwnd, &mut rect) != 0 && rect.left < OFFSCREEN_POOL_THRESHOLD_X {
                return 1; // off-screen warm-pool window — not user-visible
            }
        }
        ctx.count += 1;
        1
    }
    let mut ctx = Ctx {
        pid: GetCurrentProcessId(),
        count: 0,
    };
    EnumWindows(Some(enum_cb), &mut ctx as *mut Ctx as LPARAM);
    ctx.count
}

/// Called from HIDE/DESTROY of an app-class window. If a user window has ever
/// been shown and now zero user-visible windows remain, quit the CEF message
/// loop. Idempotent (QUIT_INITIATED guard).
unsafe fn maybe_quit_on_last_user_window() {
    use std::sync::atomic::Ordering::SeqCst;
    if QUIT_INITIATED.load(SeqCst) {
        return;
    }
    // "Have we ever had a real user window?" — gate primarily on the CEF
    // registry: `main` registers as a `is_pool:false` top-level at startup and
    // STAYS registered through the recycle-on-close (the browser is hidden, not
    // destroyed; `on_before_close` never fires), so this is ≥1 at close time
    // yet 0 during early startup before `main` registers — preventing a
    // premature quit. `HAD_VISIBLE_USER_WINDOW` (armed when a user window is
    // shown/created-visible) is kept as a belt-and-suspenders OR.
    let registered = app_state()
        .get()
        .map(|s| s.count_live_user_windows())
        .unwrap_or(0);
    let armed = registered > 0 || HAD_VISIBLE_USER_WINDOW.load(SeqCst);
    if !armed {
        return;
    }
    let visible = count_visible_user_windows();
    tracing::debug!(
        target: "wrr-trace",
        "[wrr] last-window check: registered_user_windows={} os_visible={}",
        registered, visible
    );
    if visible != 0 {
        return;
    }
    if QUIT_INITIATED.swap(true, SeqCst) {
        return; // another event won the race and already initiated quit
    }
    tracing::warn!(
        target: "wrr",
        "[wrr] all user windows hidden/closed (registered={}, 0 visible) — quitting message loop",
        registered
    );
    cef::quit_message_loop();
}

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
            // B1: a user-visible window appeared — arm the last-window quit.
            if visible {
                HAD_VISIBLE_USER_WINDOW.store(true, std::sync::atomic::Ordering::SeqCst);
            }
        }
        EVENT_OBJECT_DESTROY => {
            // No class-name filter on destroy — the HWND may be
            // mid-teardown so its class is unreliable. The
            // launcher reducer handles "destroy of an unknown
            // HWND" as a no-op via pending_hwnds + windows
            // membership check.
            position_debounce::forget(raw_hwnd);
            launcher_ipc::report_hwnd_destroyed(raw_hwnd);
            // B1: a top-level window was destroyed — if it was the last
            // user-visible one, quit. (No class filter on destroy; the fresh
            // EnumWindows count doesn't depend on this HWND's class.)
            maybe_quit_on_last_user_window();
        }
        EVENT_OBJECT_SHOW => {
            let class = read_class_name(hwnd);
            if !classify::is_app_class(&class) {
                return;
            }
            launcher_ipc::report_hwnd_visibility_changed(raw_hwnd, true);
            // B1: a user-visible window was shown — arm the last-window quit.
            HAD_VISIBLE_USER_WINDOW.store(true, std::sync::atomic::Ordering::SeqCst);
        }
        EVENT_OBJECT_HIDE => {
            let class = read_class_name(hwnd);
            if classify::is_app_class(&class) {
                launcher_ipc::report_hwnd_visibility_changed(raw_hwnd, false);
            }
            // B1: a HIDE may mean the last user window is gone. Do NOT gate this
            // on is_app_class — closing a window fires HIDE for its CHILD render
            // widget (`Chrome_RenderWidgetHostHWND`), not the top-level
            // `Chrome_WidgetWin_` frame (confirmed via Discussion #1680 smoke).
            // `count_visible_user_windows` does its own EnumWindows filtering, so
            // the check is independent of which HWND triggered it. Cheap + the
            // QUIT_INITIATED guard makes it idempotent.
            maybe_quit_on_last_user_window();
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
