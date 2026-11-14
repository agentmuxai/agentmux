// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

//! UI-thread stall watch (Windows): a `[ui-stall]` warning whenever the UI
//! thread takes no message for `STALL_MS` or longer. While it is blocked no
//! click is handled, no navigation starts and every IPC call that needs it
//! waits. `AGENTMUX_UI_STALL_STACKS=1` also logs the call stack it is stuck in.
//! How it works, and how to read a stack: docs/debug/RUNBOOK_UI_THREAD_STALL_WINDOWS.md.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Once;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{HANDLE, LPARAM, LRESULT, WPARAM};
use windows_sys::Win32::System::Threading::{
    GetCurrentThreadId, OpenThread, ResumeThread, SuspendThread, THREAD_GET_CONTEXT, THREAD_QUERY_INFORMATION,
    THREAD_SUSPEND_RESUME,
};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CallNextHookEx, PostThreadMessageW, RegisterWindowMessageW, SetWindowsHookExW, HC_ACTION, MSG, PM_REMOVE,
    WH_GETMESSAGE,
};

/// Shorter than this is not reported: it is what a person starts to call a freeze.
const STALL_MS: u64 = 250;
/// How often the watcher asks the UI thread for a sign of life.
const PING_EVERY: Duration = Duration::from_millis(200);
/// Stop waiting for one answer after this long, so a hung process is not sampled forever.
const GIVE_UP: Duration = Duration::from_secs(20);
/// Stack samples per stall, and the gap between them.
const MAX_SAMPLES: usize = 6;
const SAMPLE_GAP: Duration = Duration::from_millis(300);
/// Frames kept per sample.
const MAX_FRAMES: usize = 48;
/// The longest the UI thread stays stopped for one sample before the guard lets it go.
const SUSPEND_LIMIT_MS: u64 = 150;

/// The id of the last ping the UI thread took off its queue.
static PONG: AtomicU64 = AtomicU64::new(0);
/// The registered window message the watcher posts (0 until `install`).
static PING_MSG: AtomicU64 = AtomicU64::new(0);

/// Start the watch. Call on the UI thread; once.
pub(crate) fn install() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| unsafe {
        let name: Vec<u16> = "AgentMuxUiStallPing\0".encode_utf16().collect();
        let msg = RegisterWindowMessageW(name.as_ptr());
        if msg == 0 {
            tracing::warn!("[ui-stall] could not register the ping message; the watch is off");
            return;
        }
        PING_MSG.store(u64::from(msg), Ordering::SeqCst);
        let ui_thread = GetCurrentThreadId();
        if SetWindowsHookExW(WH_GETMESSAGE, Some(ping_hook), std::ptr::null_mut(), ui_thread).is_null() {
            tracing::warn!("[ui-stall] could not hook the UI thread's message queue; the watch is off");
            return;
        }
        let stacks = std::env::var("AGENTMUX_UI_STALL_STACKS").is_ok_and(|v| v == "1");
        let spawned = std::thread::Builder::new().name("ui-stall-watch".into()).spawn(move || watch(ui_thread, msg, stacks));
        tracing::info!("[ui-stall] watching the UI thread (stacks: {stacks}): {}", spawned.is_ok());
    });
}

/// On the UI thread: a ping taken off the queue is a message pump that runs.
unsafe extern "system" fn ping_hook(code: i32, wparam: WPARAM, lparam: LPARAM) -> LRESULT {
    if code == HC_ACTION as i32 && wparam == PM_REMOVE as usize && lparam != 0 {
        let msg = &*(lparam as *const MSG);
        if u64::from(msg.message) == PING_MSG.load(Ordering::Relaxed) {
            PONG.store(msg.wParam as u64, Ordering::SeqCst);
        }
    }
    CallNextHookEx(std::ptr::null_mut(), code, wparam, lparam)
}

fn watch(ui_thread: u32, msg: u32, stacks: bool) {
    let sampler = stacks.then(|| Sampler::new(ui_thread)).flatten();
    let mut seq: u64 = 0;
    let mut last_logged: Option<Instant> = None;
    loop {
        std::thread::sleep(PING_EVERY);
        seq += 1;
        let sent = Instant::now();
        if unsafe { PostThreadMessageW(ui_thread, msg, seq as WPARAM, 0) } == 0 {
            continue;
        }
        let mut samples: Vec<(u64, String)> = Vec::new();
        let mut last_sample: Option<Instant> = None;
        while PONG.load(Ordering::SeqCst) < seq {
            std::thread::sleep(Duration::from_millis(10));
            let waited = sent.elapsed();
            if waited > GIVE_UP {
                break;
            }
            let due = last_sample.is_none_or(|t| t.elapsed() >= SAMPLE_GAP);
            if let Some(sampler) = sampler.as_ref().filter(|_| waited.as_millis() as u64 >= STALL_MS && due && samples.len() < MAX_SAMPLES) {
                last_sample = Some(Instant::now());
                samples.push((waited.as_millis() as u64, describe(&sampler.sample())));
            }
        }
        let stalled = sent.elapsed().as_millis() as u64;
        if stalled >= STALL_MS && last_logged.is_none_or(|t| t.elapsed() >= Duration::from_millis(500)) {
            last_logged = Some(Instant::now());
            tracing::warn!("[ui-stall] the UI thread took no message for {stalled} ms");
            for (at, frames) in &samples {
                tracing::warn!("[ui-stall] stack at {at} ms: {frames}");
            }
        }
    }
}

/// Reads the UI thread's call stack. The thread is stopped while its stack is
/// walked, and the walk takes the loader's function-table lock: if the stopped
/// thread holds that lock, the walk would wait for it for ever. So a guard
/// thread lets the UI thread go after `SUSPEND_LIMIT_MS` whatever happens, and
/// the walk gives up as soon as it sees that.
struct Sampler {
    thread: HANDLE,
    guard: &'static Guard,
}

struct Guard {
    /// When the stopped thread must run again (`monotonic` ms); 0 while nothing is stopped.
    deadline: AtomicU64,
    /// The guard resumed the thread: the context being walked is stale.
    released: AtomicBool,
}

fn monotonic_ms() -> u64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_millis() as u64 + 1
}

impl Sampler {
    fn new(ui_thread: u32) -> Option<Self> {
        let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME | THREAD_GET_CONTEXT | THREAD_QUERY_INFORMATION, 0, ui_thread) };
        if thread.is_null() {
            return None;
        }
        let guard: &'static Guard = Box::leak(Box::new(Guard { deadline: AtomicU64::new(0), released: AtomicBool::new(false) }));
        let handle = thread as usize;
        std::thread::Builder::new()
            .name("ui-stall-guard".into())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_millis(20));
                let deadline = guard.deadline.load(Ordering::SeqCst);
                if deadline != 0 && monotonic_ms() > deadline && guard.deadline.compare_exchange(deadline, 0, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
                    guard.released.store(true, Ordering::SeqCst);
                    unsafe { ResumeThread(handle as HANDLE) };
                }
            })
            .ok()?;
        Some(Self { thread, guard })
    }

    /// The stack as return addresses, innermost first. Nothing is allocated and
    /// no lock of ours is taken while the thread is stopped.
    fn sample(&self) -> Vec<usize> {
        let mut addrs = [0usize; MAX_FRAMES];
        let mut count = 0usize;
        self.guard.released.store(false, Ordering::SeqCst);
        let deadline = monotonic_ms() + SUSPEND_LIMIT_MS;
        self.guard.deadline.store(deadline, Ordering::SeqCst);
        unsafe {
            if SuspendThread(self.thread) != u32::MAX {
                count = walk(self.thread, &mut addrs, &self.guard.released);
                // Whoever clears the deadline resumes the thread: us here, or the guard.
                if self.guard.deadline.compare_exchange(deadline, 0, Ordering::SeqCst, Ordering::SeqCst).is_ok() {
                    ResumeThread(self.thread);
                }
            } else {
                self.guard.deadline.store(0, Ordering::SeqCst);
            }
        }
        if self.guard.released.load(Ordering::SeqCst) {
            return Vec::new();
        }
        addrs[..count].to_vec()
    }
}

#[cfg(target_arch = "x86_64")]
unsafe fn walk(thread: HANDLE, addrs: &mut [usize; MAX_FRAMES], released: &AtomicBool) -> usize {
    use windows_sys::Win32::System::Diagnostics::Debug::{GetThreadContext, RtlLookupFunctionEntry, RtlVirtualUnwind, CONTEXT};
    const CONTEXT_AMD64: u32 = 0x0010_0000;
    const CONTEXT_CONTROL_AND_INTEGER: u32 = CONTEXT_AMD64 | 0x1 | 0x2;
    const UNW_FLAG_NHANDLER: u32 = 0;

    let mut ctx: CONTEXT = std::mem::zeroed();
    ctx.ContextFlags = CONTEXT_CONTROL_AND_INTEGER;
    if GetThreadContext(thread, &mut ctx) == 0 {
        return 0;
    }
    // A stack is a few megabytes at most: a pointer further away is not one.
    let stack = ctx.Rsp..ctx.Rsp.saturating_add(64 << 20);
    let mut count = 0usize;
    while count < MAX_FRAMES && ctx.Rip != 0 && stack.contains(&ctx.Rsp) {
        addrs[count] = ctx.Rip as usize;
        count += 1;
        let mut image_base: u64 = 0;
        let entry = RtlLookupFunctionEntry(ctx.Rip, &mut image_base, std::ptr::null_mut());
        if released.load(Ordering::SeqCst) {
            break;
        }
        if entry.is_null() {
            // A leaf with no unwind data: its return address is at the top of the stack.
            ctx.Rip = *(ctx.Rsp as *const u64);
            ctx.Rsp += 8;
        } else {
            let mut handler_data: *mut std::ffi::c_void = std::ptr::null_mut();
            let mut establisher: u64 = 0;
            RtlVirtualUnwind(UNW_FLAG_NHANDLER, image_base, ctx.Rip, entry, &mut ctx, &mut handler_data, &mut establisher, std::ptr::null_mut());
        }
    }
    count
}

#[cfg(not(target_arch = "x86_64"))]
unsafe fn walk(_thread: HANDLE, _addrs: &mut [usize; MAX_FRAMES], _released: &AtomicBool) -> usize {
    0
}

/// `module+0xOFFSET` for each frame, innermost first, joined by " < ". The
/// offset is into the module's image, so the frame can be named afterwards
/// from the module's symbol file.
fn describe(frames: &[usize]) -> String {
    if frames.is_empty() {
        return "(no frames)".to_string();
    }
    frames.iter().map(|a| frame_name(*a)).collect::<Vec<_>>().join(" < ")
}

fn frame_name(addr: usize) -> String {
    use windows_sys::Win32::System::LibraryLoader::{
        GetModuleFileNameW, GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS, GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
    };
    unsafe {
        let mut handle = std::ptr::null_mut();
        let flags = GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT;
        if GetModuleHandleExW(flags, addr as *const u16, &mut handle) == 0 {
            return format!("?+{addr:#x}");
        }
        let mut path = [0u16; 260];
        let n = GetModuleFileNameW(handle, path.as_mut_ptr(), path.len() as u32) as usize;
        let full = String::from_utf16_lossy(&path[..n.min(path.len())]);
        let module = full.rsplit(['\\', '/']).next().unwrap_or("?");
        format!("{module}+{:#x}", addr.wrapping_sub(handle as usize))
    }
}
