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

use std::sync::atomic::Ordering;
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
/// Set true while the grace-period quit watchdog (Step D) is armed, so a
/// flurry of HIDE/DESTROY events in the stuck state spawns one watchdog
/// thread, not one per event. Cleared by the watchdog's UI-thread re-check
/// (whether or not it quits), re-arming future watchdogs.
static WATCHDOG_ARMED: std::sync::atomic::AtomicBool =
    std::sync::atomic::AtomicBool::new(false);
/// How long the quit watchdog waits before trusting the OS signal alone.
/// Long enough for any in-flight UnregisterBrowser dispatch (close_window
/// RPC → CloseWindowTask, or the LOCATIONCHANGE pool-move handler) to land;
/// short enough that a genuinely-stuck reducer doesn't leave the user with
/// an invisible zombie instance for long.
const QUIT_WATCHDOG_GRACE: std::time::Duration = std::time::Duration::from_millis(3000);

/// Bound on how many extra watchdog cycles the "reducer shows a live,
/// non-draining window" desync gets before the code falls back to trusting
/// the OS signal alone (docs/plans/PLAN_WRR_QUIT_WATCHDOG_LAG_RETRY_2026_08_03.md).
/// Worst-case added grace is `WATCHDOG_LAG_RETRIES_MAX * QUIT_WATCHDOG_GRACE`,
/// and only when a live, non-draining window is genuinely still registered —
/// bounded so this stays "quit a few seconds late with a loud log", not a
/// regression toward the invisible-zombie failure mode Step D exists to avoid.
const WATCHDOG_LAG_RETRIES_MAX: u32 = 3;

/// Consecutive re-arm cycles already granted to the `registered > 0 &&
/// !draining` desync flavor (as opposed to the pre-existing `draining &&
/// registered == 0` "post-drain debris" re-arm, which is unbounded by
/// design — see its own comment in `QuitWatchdogRecheckTask::execute`).
/// Reset to 0 whenever the watchdog stands down cleanly (a window is
/// OS-visible again) or ultimately fires (fresh budget for next time).
static WATCHDOG_LAG_RETRY_COUNT: std::sync::atomic::AtomicU32 =
    std::sync::atomic::AtomicU32::new(0);

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

/// Diagnostic-only: enumerate every app-class top-level window belonging to
/// this process — regardless of visibility — and log hwnd/class/title/
/// visible/iconic/rect for each. Called only from the watchdog's anomalous
/// paths (a lag re-arm or the final quit-fire, PLAN_WRR_QUIT_WATCHDOG_LAG_RETRY_2026_08_03.md),
/// never from the per-WINEVENT hot path, so the extra `EnumWindows` pass here
/// doesn't matter. Turns the old "reducer desync, investigate" log line —
/// which told you the *counts* disagreed but nothing about which window the
/// reducer thought was still alive or what state the OS actually saw it in —
/// into something a future investigation can actually use.
unsafe fn diag_dump_app_windows(context: &str) {
    use windows_sys::Win32::Foundation::{BOOL, LPARAM};
    use windows_sys::Win32::System::Threading::GetCurrentProcessId;
    use windows_sys::Win32::UI::WindowsAndMessaging::EnumWindows;

    unsafe extern "system" fn enum_cb(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let pid_target = lparam as u32;
        let mut pid: u32 = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if pid != pid_target {
            return 1;
        }
        let class = read_class_name(hwnd);
        if !classify::is_app_class(&class) {
            return 1;
        }
        let title = read_window_text(hwnd);
        let visible = IsWindowVisible(hwnd) != 0;
        let iconic = IsIconic(hwnd) != 0;
        let rect = read_window_rect(hwnd);
        tracing::warn!(
            target: "wrr",
            "[wrr] diag hwnd={:#x} class={} title={:?} visible={} iconic={} rect={:?}",
            hwnd as u64, class, title, visible, iconic, rect
        );
        1
    }

    let pid = GetCurrentProcessId();
    tracing::warn!(
        target: "wrr",
        "[wrr] diag dump ({}) — enumerating all app-class windows for pid={}",
        context, pid
    );
    EnumWindows(Some(enum_cb), pid as LPARAM);
}

/// Pure decision extracted for unit testing (SPEC_WRR_QUIT_FALSE_POSITIVE_2026_07_08.md).
///
/// Requires BOTH the OS-level `EnumWindows` count (`visible`) AND the reducer's own
/// `count_live_user_windows()` (`registered`) to read zero before quitting. Before this
/// fix, `visible` alone decided — but a synchronous `EnumWindows` pass can transiently
/// misread during window-pool refill/promote churn on the same UI thread (e.g. a
/// promoted pool window not yet moved on-screen), so closing a non-last window could
/// momentarily read `visible == 0` while real windows remained, killing the whole host.
///
/// This AND is only safe because `registered` is now kept accurate for the one case
/// that previously left it permanently stale — the CEF Views main-window
/// recycle-on-close, which never fires `on_before_close` — via the LOCATIONCHANGE
/// pool-move handler above now also dispatching `UnregisterBrowser` (Step A of the
/// same spec). Without that companion fix, ANDing `registered` in here would hang the
/// process open on a recycle-close, regressing #1676.
///
/// Pillar 2 Phase 3 (SPEC_PILLAR2_SANITIZE_THEN_DECIDE §2.2): also requires
/// `draining` — the reducer has DECIDED to quit (`QuitState` left `Running`
/// via `reconcile_quit`'s verdict, consumed at a Phase-2 dispatch site). This
/// demotes WRR from an independent quit authority to the Stage-2 executor of
/// a reducer-made decision: on Windows, parked browsers never fire
/// `on_before_close`, so `client::browser_list` never empties and the
/// on_before_close Stage-2 gate is structurally unreachable — this function
/// is the Windows Stage 2. All-zero counts WITHOUT a drain decision now mean
/// a consumption site was missed: `is_reducer_lagging_os` routes that to the
/// watchdog (quit-late-with-loud-log) instead of quitting silently on WRR's
/// own authority.
fn should_quit_on_last_window(armed: bool, draining: bool, visible: usize, registered: usize) -> bool {
    armed && draining && visible == 0 && registered == 0
}

/// Companion pure decision: the OS says every window is gone but the reducer
/// hasn't finished agreeing — either its count still shows a live window (an
/// UnregisterBrowser dispatch in flight, or a close path that never
/// dispatched), or (Phase 3) the count reads zero but no drain decision was
/// ever consumed (`!draining` — a missed Phase-2 consumption site). Both
/// flavors are the failure mode Step D's watchdog exists to bound.
fn is_reducer_lagging_os(armed: bool, draining: bool, visible: usize, registered: usize) -> bool {
    armed && visible == 0 && (registered > 0 || !draining)
}

/// Pure decision (PLAN_WRR_QUIT_WATCHDOG_LAG_RETRY_2026_08_03.md): should the
/// watchdog grant one more re-arm cycle to the "reducer shows a live,
/// non-draining window" desync flavor, instead of trusting the OS `visible ==
/// 0` snapshot immediately? `registered > 0 && !draining` is the least
/// ambiguous "do NOT quit" signal the reducer can produce — a window nobody
/// has asked to close. `retries_used` is how many re-arms this flavor has
/// already been granted (0 on its first watchdog fire). The
/// `registered == 0 && !draining` flavor (a genuinely missed `request_drain`
/// consumption site — no live window to protect) is deliberately excluded:
/// it has nothing to wait for, so it still fires on the first cycle.
fn should_extend_lag_retry(registered: usize, draining: bool, retries_used: u32) -> bool {
    registered > 0 && !draining && retries_used < WATCHDOG_LAG_RETRIES_MAX
}

/// Step D — bounded fallback so a missed `UnregisterBrowser` path degrades to
/// "quit a few seconds late with a loud log" instead of "hang forever as an
/// invisible zombie instance" (the regression the first cut of this fix caused:
/// requiring reducer agreement coupled the quit to reducer paths that weren't
/// all wired yet). One watchdog at a time; the sleep happens on a throwaway
/// thread and the DECISION + `quit_message_loop()` happen in a UI-posted task
/// (UI-thread-only primitive — off-thread it silently no-ops, v0.33.492).
/// `post_task` is reliable here: the message loop is running normally in this
/// state (nothing has begun tearing down — that's the problem).
pub(crate) fn arm_quit_watchdog(registered: usize) {
    use std::sync::atomic::Ordering::SeqCst;
    if WATCHDOG_ARMED.swap(true, SeqCst) {
        return; // one in flight already
    }
    // Two callers: maybe_quit_on_last_user_window (visible==0 confirmed) and
    // unregister_after_parking_close (unconditional belt-and-suspenders) — so
    // this message must not claim windows are hidden; the re-check decides.
    tracing::warn!(
        target: "wrr",
        "[wrr] arming {}ms quit watchdog (reducer counts {} live) — will re-check visibility on fire",
        QUIT_WATCHDOG_GRACE.as_millis(),
        registered
    );
    let Some(state) = app_state().get().cloned() else {
        WATCHDOG_ARMED.store(false, SeqCst);
        return; // hooks not installed yet — nothing to watch
    };
    std::thread::spawn(move || {
        std::thread::sleep(QUIT_WATCHDOG_GRACE);
        let mut task = QuitWatchdogRecheckTask::new(state);
        cef::post_task(cef::ThreadId::UI, Some(&mut task));
    });
}

// `wrap_task!` is unhygienic — it references `Task`/`WrapTask`/`ImplTask`
// unqualified and calls the `rc::Rc` trait's provided `.add_ref()` (every
// other call site has `use cef::*` + `rc::*`; this module keeps its imports
// explicit, so pull in just what the macro expansion needs).
use cef::rc::Rc as _;
use cef::{ImplTask, Task, WrapTask};

cef::wrap_task! {
    pub struct QuitWatchdogRecheckTask {
        state: Arc<AppState>,
    }

    impl Task {
        fn execute(&self) {
            use std::sync::atomic::Ordering::SeqCst;
            WATCHDOG_ARMED.store(false, SeqCst); // allow future watchdogs either way
            if QUIT_INITIATED.load(SeqCst) {
                return;
            }
            let (registered, draining) = {
                let st = self.state.host_state.lock();
                (
                    crate::reducer::count_live_user_windows(&st),
                    st.quit_state != crate::state::QuitState::Running,
                )
            };
            let visible = unsafe { count_visible_user_windows() };
            if visible != 0 {
                // Once the reducer has DECIDED to drain and counts zero live
                // user windows, a clear-and-forget stand-down here is a
                // permanent-zombie recipe: the drain is monotonic, every
                // close event has already fired (nothing will re-arm), and
                // whatever is "visible" cannot be a live user window (a real
                // minimized/visible user window would still be REGISTERED —
                // minimize never unregisters). Live-caught 2026-07-11: a
                // pool refill that landed after Stage 1 sat transiently
                // visible at the spawn sentinel, the recheck stood down, and
                // the instance hung as a draining corpse. Re-arm and keep
                // watching until the OS agrees; each cycle is one 3s sleep.
                if draining && registered == 0 {
                    tracing::warn!(
                        target: "wrr",
                        "[wrr] quit watchdog: {} window(s) transiently visible while draining with 0 registered — re-arming (post-drain debris, e.g. late pool spawn)",
                        visible
                    );
                    arm_quit_watchdog(registered);
                    return;
                }
                WATCHDOG_LAG_RETRY_COUNT.store(0, SeqCst);
                tracing::info!(
                    target: "wrr",
                    "[wrr] quit watchdog: {} window(s) visible again — stand down",
                    visible
                );
                return;
            }
            // PLAN_WRR_QUIT_WATCHDOG_LAG_RETRY_2026_08_03.md: `registered > 0
            // && !draining` means the reducer still shows a live window that
            // nobody has asked to close — the least ambiguous "do NOT quit"
            // signal it can produce. A single 3s grace period isn't always
            // enough to ride out a burst of window-pool HWND churn (drag/
            // tear-off pool refill) landing in the same window as a pane
            // close, which is exactly what a transient `EnumWindows` misread
            // looks like (SPEC_WRR_QUIT_FALSE_POSITIVE_2026_07_08.md §2).
            // Grant this flavor a few bounded extra cycles before falling
            // back to trusting the OS signal — the `registered == 0`
            // flavor (a genuinely missed request_drain consumption site, no
            // live window to protect) still falls through immediately below.
            let retries_used = WATCHDOG_LAG_RETRY_COUNT.load(SeqCst);
            if should_extend_lag_retry(registered, draining, retries_used) {
                let retries = WATCHDOG_LAG_RETRY_COUNT.fetch_add(1, SeqCst) + 1;
                tracing::warn!(
                    target: "wrr",
                    "[wrr] quit watchdog: 0 visible for {}ms but registered={} draining=false (live window, not draining) — re-arming ({}/{})",
                    QUIT_WATCHDOG_GRACE.as_millis(),
                    registered,
                    retries,
                    WATCHDOG_LAG_RETRIES_MAX
                );
                unsafe { diag_dump_app_windows("lag re-arm") };
                arm_quit_watchdog(registered);
                return;
            }
            WATCHDOG_LAG_RETRY_COUNT.store(0, SeqCst);
            if QUIT_INITIATED.swap(true, SeqCst) {
                return;
            }
            // Trusting the OS signal alone (the pre-Step-B behavior). The
            // state logged here is a live bug report: registered > 0 means
            // some close path failed to dispatch UnregisterBrowser (and the
            // lag-retry budget above is now exhausted); !draining means a
            // request_drain consumption site was missed (Phase 2's sites, or
            // a new close path) — find it.
            unsafe { diag_dump_app_windows("quit fire") };
            tracing::warn!(
                target: "wrr",
                "[wrr] quit watchdog fired: 0 visible for {}ms (retries={}) but reducer disagrees (registered={} draining={}) — quitting on OS signal alone (reducer desync, investigate)",
                QUIT_WATCHDOG_GRACE.as_millis(),
                retries_used,
                registered,
                draining
            );
            cef::quit_message_loop();
        }
    }
}

/// Safe re-entry point for non-WINEVENT callers (Pillar 2 Phase 3): the
/// drain-cascade task re-runs the gate right after `QuitState` flips, so a
/// drain with zero pool inventory (no further OS events will ever arrive)
/// quits deterministically instead of waiting out the watchdog. Safe wrapper:
/// the body only reads OS/window state (`EnumWindows`, class probes) and the
/// host-state lock — the `unsafe` on the inner fn is Win32 FFI, not a caller
/// contract beyond "call it on the UI thread", which `wrap_task!` execution
/// guarantees.
#[cfg(target_os = "windows")]
pub(crate) fn reevaluate_last_window_quit() {
    unsafe { maybe_quit_on_last_user_window() }
}

/// Called from HIDE/DESTROY of an app-class window. If a user window has ever
/// been shown and now zero user-visible windows remain, quit the CEF message
/// loop. Idempotent (QUIT_INITIATED guard).
unsafe fn maybe_quit_on_last_user_window() {
    use std::sync::atomic::Ordering::SeqCst;
    if QUIT_INITIATED.load(SeqCst) {
        return;
    }
    // Gate on whether a user window has EVER been shown — prevents a premature
    // quit when a pool window fails during startup before the main window has
    // loaded. `main` registers in CEF state (count_live_user_windows ≥ 1) at
    // on_after_created time, which is BEFORE the main window is visible.
    // Using `registered > 0` as the armed gate would fire quit when a pool
    // window fails during that gap (main registered but not yet shown, visible=0).
    // `HAD_VISIBLE_USER_WINDOW` is set only on EVENT_OBJECT_SHOW, so it stays
    // false until the page actually loads and the host calls ShowWindow — the
    // correct moment to arm the last-window quit.
    let (registered, draining) = app_state()
        .get()
        .map(|s| {
            // One lock scope so the count and the drain decision are read
            // from the same instant (two separate reads could tear).
            let st = s.host_state.lock();
            let registered = crate::reducer::count_live_user_windows(&st);
            // Phase 3: "the reducer decided to quit" = QuitState left Running
            // (Draining or Quit — both mean a reconcile_quit verdict was
            // consumed at a Phase-2 dispatch site).
            let draining = st.quit_state != crate::state::QuitState::Running;
            (registered, draining)
        })
        .unwrap_or((0, false));
    let armed = HAD_VISIBLE_USER_WINDOW.load(SeqCst);
    if !armed {
        return;
    }
    let visible = count_visible_user_windows();
    tracing::debug!(
        target: "wrr-trace",
        "[wrr] last-window check: registered_user_windows={} os_visible={} draining={}",
        registered, visible, draining
    );
    if !should_quit_on_last_window(armed, draining, visible, registered) {
        if is_reducer_lagging_os(armed, draining, visible, registered) {
            arm_quit_watchdog(registered);
        }
        return;
    }
    if QUIT_INITIATED.swap(true, SeqCst) {
        return; // another event won the race and already initiated quit
    }
    tracing::warn!(
        target: "wrr",
        "[wrr] all user windows hidden/closed (registered=0, 0 visible, draining) — quitting message loop (Stage-2 executor)",
    );
    cef::quit_message_loop();
}

#[cfg(test)]
mod should_quit_tests {
    use super::should_quit_on_last_window;

    #[test]
    fn not_armed_never_quits() {
        assert!(!should_quit_on_last_window(false, true, 0, 0));
    }

    #[test]
    fn quits_only_when_all_three_signals_agree() {
        // OS zero + reducer zero + reducer DECIDED to drain → Stage-2 quit.
        assert!(should_quit_on_last_window(true, true, 0, 0));
    }

    #[test]
    fn all_zero_without_drain_decision_does_not_quit() {
        // Phase 3: counts agree but no reconcile_quit verdict was ever
        // consumed — a missed Phase-2 consumption site. The watchdog handles
        // it (quit-late-with-loud-log); WRR must not quit on its own
        // authority.
        assert!(!should_quit_on_last_window(true, false, 0, 0));
    }

    #[test]
    fn os_transient_zero_with_live_registered_window_does_not_quit() {
        // The false-positive #2043 closed: a transient EnumWindows misread
        // (visible == 0) while the reducer still shows a live window.
        assert!(!should_quit_on_last_window(true, true, 0, 1));
        assert!(!should_quit_on_last_window(true, false, 0, 1));
    }

    #[test]
    fn registered_stale_with_os_confirming_windows_gone_does_not_quit() {
        assert!(!should_quit_on_last_window(true, true, 1, 0));
    }

    #[test]
    fn both_nonzero_does_not_quit() {
        assert!(!should_quit_on_last_window(true, true, 2, 1));
    }

    #[test]
    fn watchdog_arms_when_os_zero_but_reducer_lags() {
        use super::is_reducer_lagging_os;
        // Flavor 1 (Step D's original target): OS says gone, reducer count
        // disagrees.
        assert!(is_reducer_lagging_os(true, true, 0, 1));
        assert!(is_reducer_lagging_os(true, false, 0, 1));
        // Flavor 2 (Phase 3): counts agree at zero but no drain decision was
        // consumed — a missed consumption site; bounded by the same watchdog.
        assert!(is_reducer_lagging_os(true, false, 0, 0));
        // Clean quit state (should_quit handles it instead)…
        assert!(!is_reducer_lagging_os(true, true, 0, 0));
        // …windows still visible…
        assert!(!is_reducer_lagging_os(true, true, 3, 1));
        assert!(!is_reducer_lagging_os(true, false, 3, 1));
        // …or before any user window was ever shown (startup).
        assert!(!is_reducer_lagging_os(false, false, 0, 1));
    }

    #[test]
    fn lag_retry_extends_for_live_nondraining_window_under_budget() {
        use super::{should_extend_lag_retry, WATCHDOG_LAG_RETRIES_MAX};
        // The exact desync this fix targets: a real, non-draining window is
        // still registered — extend as long as the budget isn't exhausted.
        assert!(should_extend_lag_retry(1, false, 0));
        assert!(should_extend_lag_retry(1, false, WATCHDOG_LAG_RETRIES_MAX - 1));
    }

    #[test]
    fn lag_retry_stops_once_budget_exhausted() {
        use super::{should_extend_lag_retry, WATCHDOG_LAG_RETRIES_MAX};
        assert!(!should_extend_lag_retry(1, false, WATCHDOG_LAG_RETRIES_MAX));
        assert!(!should_extend_lag_retry(1, false, WATCHDOG_LAG_RETRIES_MAX + 5));
    }

    #[test]
    fn lag_retry_does_not_extend_once_draining() {
        use super::should_extend_lag_retry;
        // Once the reducer has decided to drain, this is a different flavor
        // (handled by the draining && registered == 0 re-arm above, or a
        // clean quit) — not this desync.
        assert!(!should_extend_lag_retry(1, true, 0));
    }

    #[test]
    fn lag_retry_does_not_extend_with_no_live_window() {
        use super::should_extend_lag_retry;
        // registered == 0 && !draining is a genuinely missed request_drain
        // consumption site — there's no live window to protect, so it
        // should still fire on the first cycle, same as before this fix.
        assert!(!should_extend_lag_retry(0, false, 0));
    }
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
                HAD_VISIBLE_USER_WINDOW.store(true, Ordering::SeqCst);
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
            HAD_VISIBLE_USER_WINDOW.store(true, Ordering::SeqCst);
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
            let class = read_class_name(hwnd);
            if !classify::is_app_class(&class) {
                return;
            }
            let Some(rect) = read_window_rect(hwnd) else {
                return;
            };

            // Gap A — pool-move detection: CEF Views recycle-on-close moves the
            // HWND to x < OFFSCREEN_POOL_THRESHOLD_X immediately after the HIDE.
            // Virtual-desktop switches do NOT move the rect (the window stays at
            // its on-screen coordinates, only its visibility changes), so this
            // check correctly distinguishes a real close from a desktop switch
            // without any debounce heuristic or paired-SHOW cancellation.
            //
            // We bypass the position debounce here (unconditional rect read) so a
            // close that follows a drag within the 50ms debounce window is still
            // detected. LOCATIONCHANGE fires once per WM_WINDOWPOSCHANGED, so a
            // stationary pool window never re-fires — no duplicate-report risk.
            //
            // Ref: docs/retro/retro-window-count-stale-post-1701-2026-06-27.md §Gap A
            //      reagentx P1+P2 on PR #1803.
            // IsIconic guard: minimized windows report (-32000, -32000) from
            // GetWindowRect, which is below OFFSCREEN_POOL_THRESHOLD_X. Skip
            // them — a minimize is not a close.
            if rect.left < OFFSCREEN_POOL_THRESHOLD_X && IsIconic(hwnd) == 0 {
                if let Some(state) = app_state().get() {
                    if let Some(label) = state.label_for_hwnd(hwnd) {
                        // Gate on BrowserKind::TopLevel { is_pool: false } — the
                        // authoritative type flag, not a label prefix. This correctly
                        // handles promoted window-pool-* windows: they keep their
                        // original label but acquire is_pool:false at promotion, so a
                        // user closing one still fires report_window_closed. A naïve
                        // starts_with("window-pool-") check would have suppressed that
                        // report and left the launcher with a stale window count.
                        // Warm-pool HWNDs (is_pool:true), Floaters, Panes, and HWNDs
                        // whose browser hasn't fired OnAfterCreated yet all return
                        // false and are correctly skipped. See reagentx P1 on PR #1827.
                        if state.is_live_top_level_browser(&label) {
                            tracing::debug!(
                                target: "wrr",
                                "[wrr] LOCATIONCHANGE pool-move → report_window_closed label={}",
                                label
                            );
                            crate::launcher_ipc::report_window_closed(label.clone());
                            // SPEC_WRR_QUIT_FALSE_POSITIVE_2026_07_08.md Step A — this is the
                            // one close path where CEF's own `on_before_close` never fires (a
                            // Views recycle-on-close hides/reuses the browser instead of
                            // destroying it), so without this dispatch `count_live_user_windows()`
                            // never learns this window is gone and stays stuck non-zero forever.
                            // Idempotent: `handle_unregister_browser` no-ops on an
                            // already-removed/unknown label.
                            let out = state.host_dispatch(
                                crate::reducer::HostCommand::UnregisterBrowser { label },
                            );
                            // Pillar 2 Phase 2 (sanitize-then-decide §2.4) —
                            // consume the drain verdict this dispatch just
                            // computed instead of discarding it: the recycle
                            // close of the last window flips QuitState and
                            // runs the Stage-1 drain INLINE (this WINEVENT
                            // callback is on the UI thread), so the cascade
                            // has completed before the gate re-run below —
                            // the ordering the reagent P1 on #2082 required.
                            crate::ui_tasks::consume_request_drain(
                                state, &out, "wrr_locationchange_recycle_close",
                            );
                            // Re-evaluate the quit gate NOW: this LOCATIONCHANGE may be
                            // the last event this window ever fires (HIDE preceded the
                            // pool-move), and the gate requires the registered count we
                            // just corrected. Without this, a recycle-close of the last
                            // window would wait on the watchdog instead of quitting
                            // promptly. Same-thread (UI), same fn HIDE/DESTROY call —
                            // idempotent via QUIT_INITIATED.
                            maybe_quit_on_last_user_window();
                        }
                    }
                }
                // Pool-position window: suppress position IPC — the launcher
                // WRR mirror treats negative-x positions as off-monitor noise.
                return;
            }

            // Normal on-screen position reporting, debounced to ~20 Hz.
            if position_debounce::should_emit(raw_hwnd) {
                launcher_ipc::report_hwnd_position_changed(raw_hwnd, rect);
                // Second, independent consumer of the same position stream:
                // debounced srv write-through so a full process-tree
                // restart (launcher included) can still reproject secondary
                // windows at their exact last geometry — see
                // `commands::window::position_persist` for the full
                // rationale. Does not touch the launcher-IPC forward above.
                //
                // IsIconic guard (reagent P1 on PR #2302): a minimize can
                // reach this branch too — the pool-move check above only
                // returns early when `IsIconic(hwnd) == 0`, so a minimized
                // window's LOCATIONCHANGE (reporting GetWindowRect's
                // (-32000,-32000) sentinel, same one the comment above this
                // block already documents) falls through here. That sentinel
                // has a positive width/height (it's a real rect, just
                // off-screen), so `backend_get_window_pos_and_size`'s
                // zero-size guard alone wouldn't catch it — without this
                // check, a full restart would reproject the window
                // off-screen. The launcher-IPC forward above is left as-is
                // (pre-existing behavior, unrelated to this write-through).
                if IsIconic(hwnd) == 0 {
                    if let Some(state) = app_state().get() {
                        crate::commands::window::position_persist::report_position_for_srv_writethrough(
                            state, hwnd, rect,
                        );
                    }
                }
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
