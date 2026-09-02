# Process Lifecycle v2: OS-Level Parent-Child Binding

## Status: Proposed (replaces backend-lifecycle.md)

## Problem

When the AgentMux frontend exits (normally, crash, or force-kill), backend (`agentmuxsrv-rs`) and WebView2 (`msedgewebview2.exe`) processes are left orphaned.

### Failed Approaches

| Approach | Why it failed |
|----------|---------------|
| WS idle watchdog (v0.31.59) | WS connections drop during React re-renders/tab switches. Backend killed itself during normal use. |
| PID-based kill on close | Only works for normal shutdown. Crash/force-kill bypasses it entirely. |
| `wave-endpoints.json` reuse | Creates backends with no parent handle. HTTP health check adds 10s latency on stale files. Source of most orphan bugs. |
| stdin EOF | Unreliable on Windows — Tauri shell plugin may pass console handles instead of pipes. |
| Tauri's built-in kill-on-drop | Application-level (`App::drop` iterates children and calls `kill()`). Doesn't run on crash/SIGKILL. |

### Root Cause

All failed approaches try to solve process lifecycle at the **application level**. The only reliable solution is at the **OS kernel level**: tie the child process lifetime to the parent process so the kernel enforces cleanup even during crashes.

## Research Findings

### Windows: Job Objects — CONFIRMED RELIABLE

Job Objects with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE` are the industry-standard solution. Used by:
- **Cargo** (Rust's build tool) since 2016 — [PR #2370](https://github.com/rust-lang/cargo/pull/2370)
- **Chromium** for sandboxed renderer processes
- **`process-wrap` crate** — provides `JobObject` wrapper with kill-on-drop
- **`win32job` crate** — safe Rust API for Windows Job Objects

When the last handle to the Job Object closes (which happens automatically when the creating process exits for ANY reason), Windows terminates all assigned processes. This is enforced by the kernel — no application code needs to run.

**Caveat (Cargo PR #5887):** On *normal* exit, Job Object also kills children. Cargo had to [fix this](https://github.com/rust-lang/cargo/pull/5887) because `cargo run` was killing the launched program. For us this is DESIRED behavior — we always want the backend to die with the frontend.

**Tauri shell plugin:** Does NOT expose the raw process handle needed for `AssignProcessToJobObject`. We have two options:
1. Use `windows-sys` to get the handle from the PID after spawn
2. Bypass Tauri's sidecar API and use `std::process::Command` directly

**WebView2:** Spawned via COM, not as direct children. May NOT be in the Job Object. Need to test — if not, WebView2 orphans remain a separate problem.

### Linux: PR_SET_PDEATHSIG — DANGEROUS WITH ASYNC RUST

**Critical finding:** `PR_SET_PDEATHSIG` tracks the **parent thread**, not the parent process.

Two independent sources confirm this is a serious problem:
- **[Kobzol's blog (Feb 2025)](https://kobzol.github.io/rust/2025/02/23/tokio-plus-prctl-equals-nasty-bug.html):** Tokio worker threads that spawn child processes with `PR_SET_PDEATHSIG` cause premature kills. Tokio reaps idle threads after ~10s of inactivity, which the kernel interprets as "parent died" and sends SIGTERM to the child. The fix was to stop using `PR_SET_PDEATHSIG` entirely.
- **[Recall.ai blog](https://www.recall.ai/blog/pdeathsig-is-almost-never-what-you-want):** "PDEATHSIG is almost never what you want." Same thread-vs-process issue with async runtimes. Their fix: remove it entirely.

**For us:** `agentmuxsrv-rs` uses `#[tokio::main]`. If the Tauri sidecar is spawned from a Tokio worker thread (which is possible via `tauri::async_runtime::spawn`), `PR_SET_PDEATHSIG` could fire when that thread is reaped, killing the backend randomly during normal operation. This is exactly what happened with our WS idle watchdog — an unreliable mechanism that kills the backend during normal use.

**Safe alternative for Linux:** Call `PR_SET_PDEATHSIG` from the **main thread only**, before Tokio runtime starts. Our backend calls it at the top of `main()` before `#[tokio::main]`, so the parent thread is the actual main thread. This should be safe, but needs careful testing.

**Safest alternative for Linux:** Use a **PID polling watchdog** in the backend — a dedicated thread that periodically checks `getppid()`. If it returns 1 (reparented to init), the parent died. This is immune to the thread-vs-process problem because it doesn't use `prctl` at all.

```rust
// Safe parent-death detection on Linux (and macOS)
std::thread::spawn(move || {
    let original_ppid = unsafe { libc::getppid() };
    loop {
        std::thread::sleep(std::time::Duration::from_secs(2));
        let current_ppid = unsafe { libc::getppid() };
        if current_ppid != original_ppid {
            eprintln!("parent process died (ppid changed {} -> {}), shutting down",
                original_ppid, current_ppid);
            std::process::exit(0);
        }
    }
});
```

This polls every 2s (negligible overhead) and is 100% reliable — no thread/process confusion, no race conditions, works with any async runtime.

### macOS: kqueue EVFILT_PROC — RELIABLE

`kqueue` with `EVFILT_PROC` + `NOTE_EXIT` watches a specific PID and fires when it exits. This is reliable because it watches by PID, not by thread. Runs in a background thread blocked in kernel with zero CPU overhead.

However, the **ppid polling approach** described above also works on macOS and is simpler. Since we need it for Linux anyway (as a safe alternative to `PR_SET_PDEATHSIG`), using the same approach on both platforms reduces code and testing surface.

## Recommended Solution

### Layered approach (defense in depth)

| Layer | Mechanism | Covers |
|-------|-----------|--------|
| **1. Normal shutdown** | `child.kill()` in Tauri's `CloseRequested` handler | User closes last window |
| **2. OS safety net (Windows)** | Job Object with `KILL_ON_JOB_CLOSE` | Crash, force-kill, OOM — everything |
| **3. OS safety net (Linux/macOS)** | ppid polling in backend (2s interval) | Crash, force-kill, OOM |
| **4. Existing safety net** | stdin EOF watch (already implemented) | Normal parent death on Unix |
| **5. Existing safety net** | Signal handler (already implemented) | Ctrl+C, manual `kill` |

### What to remove

| Remove | Why |
|--------|-----|
| `wave-endpoints.json` reuse path | Root cause of orphans and 10s startup delay |
| PID-based kill in close handler | Job Object handles this (Windows); ppid polling handles this (Unix) |
| `backend_pid` in `AppState` | No longer needed without reuse path |
| Shutdown WS command | OS handles it |
| WS idle watchdog code | Already removed, never re-add |
| Endpoints file write after spawn | No reuse = no file needed |

### What to keep

| Keep | Why |
|------|-----|
| `child.kill()` on last window close | Graceful shutdown path — lets backend flush writes |
| stdin EOF watch | Lightweight, works on Unix, free safety net |
| Signal handler | Development + manual `kill` |
| WS client count + logging | Debugging visibility |
| Endpoints file cleanup on startup | Delete stale files from before this change |

## Implementation Plan

### Step 1: Windows Job Object (frontend)

**File:** `src-tauri/src/sidecar.rs`

After spawning the sidecar, create a Job Object and assign the child process:

```rust
#[cfg(target_os = "windows")]
fn create_job_object_for_child(pid: u32) -> Result<isize, String> {
    use windows_sys::Win32::System::JobObjects::*;
    use windows_sys::Win32::System::Threading::*;
    use windows_sys::Win32::Foundation::*;

    unsafe {
        let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
        if job == 0 {
            return Err("Failed to create job object".into());
        }

        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let ok = SetInformationJobObject(
            job,
            JobObjectExtendedLimitInformation,
            &info as *const _ as *const std::ffi::c_void,
            std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
        );
        if ok == 0 {
            CloseHandle(job);
            return Err("Failed to set job object info".into());
        }

        let process = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, 0, pid);
        if process == 0 {
            CloseHandle(job);
            return Err(format!("Failed to open process {}", pid));
        }

        let ok = AssignProcessToJobObject(job, process);
        CloseHandle(process);
        if ok == 0 {
            CloseHandle(job);
            return Err("Failed to assign process to job".into());
        }

        Ok(job)
    }
}
```

Store the job handle in `AppState`. It MUST NOT be dropped until app exit — dropping it closes the handle, which triggers the kill.

**Dependency:** Add `windows-sys` with features `Win32_System_JobObjects`, `Win32_System_Threading`, `Win32_Foundation`.

### Step 2: ppid polling (backend, Linux + macOS)

**File:** `agentmuxsrv-rs/src/main.rs`

At the top of `main()`, before Tokio runtime:

```rust
#[cfg(any(target_os = "linux", target_os = "macos"))]
{
    let original_ppid = unsafe { libc::getppid() };
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(std::time::Duration::from_secs(2));
            if unsafe { libc::getppid() } != original_ppid {
                eprintln!("parent died (ppid changed), shutting down");
                std::process::exit(0);
            }
        }
    });
}
```

**Dependency:** `libc` crate (add explicit dep if not already direct).

### Step 3: Remove reuse path (frontend)

**File:** `src-tauri/src/sidecar.rs`

Delete: endpoints file read/check (lines 45-112), endpoints file write (lines 326-372).
Delete: `backend_pid` from `state.rs`.
Simplify: close handler in `lib.rs` (remove PID kill branch).

### Step 4: Startup cleanup

**File:** `src-tauri/src/sidecar.rs`

On startup, before spawning, delete any stale `wave-endpoints.json` for our version. Transitional — can be removed once all users have upgraded past the reuse mechanism.

## Testing

1. **Normal close:** Launch, close window. Verify all processes gone within 1s.
2. **Force kill (Windows):** `taskkill /F /IM agentmux.exe`. Verify backend dies immediately via Job Object.
3. **Force kill (Linux/macOS):** `kill -9 <pid>`. Verify backend dies within 2s (ppid poll interval).
4. **Multi-window:** Open 2 windows, close 1. Backend stays. Close last. Backend dies.
5. **Rapid open/close:** 5 times quickly. Zero orphaned processes.
6. **Startup speed:** <1s to first render (no HTTP health check).
7. **WebView2 (Windows):** After close, verify no orphaned `msedgewebview2.exe`.

## Risk Assessment

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| Tauri shell plugin doesn't expose PID for Job Object | Medium | Can't assign to job | Use `sysinfo` crate or `/proc` to find child PID by name |
| WebView2 not killed by Job Object (spawned via COM) | High | Orphaned WebView2 | Separate fix: set WebView2 user data dir outside portable folder |
| `windows-sys` version conflict with Tauri deps | Low | Build failure | Use same major version Tauri depends on |
| ppid polling 2s lag on Unix | Low | Backend lives 0-2s after parent death | Acceptable — stdin EOF covers the fast path |
| Nested job objects on Windows (if Tauri already uses one) | Medium | `AssignProcessToJobObject` fails | Use `JOB_OBJECT_LIMIT_BREAKAWAY_OK` on existing job, or query first |

## Sources

- [Cargo: Use job objects on Windows (PR #2370)](https://github.com/rust-lang/cargo/pull/2370)
- [Cargo: Don't kill children on normal exit (PR #5887)](https://github.com/rust-lang/cargo/pull/5887)
- [Tauri: kill sidecar children on App drop (commit 4bdc406)](https://github.com/tauri-apps/tauri/commit/4bdc406679363f460e39079cb26319c39ab8cac8)
- [Tauri: Sidecar Lifecycle Management Plugin (issue #3062)](https://github.com/tauri-apps/plugins-workspace/issues/3062)
- [Kobzol: Tokio + prctl = nasty bug](https://kobzol.github.io/rust/2025/02/23/tokio-plus-prctl-equals-nasty-bug.html)
- [Recall.ai: PDEATHSIG is almost never what you want](https://www.recall.ai/blog/pdeathsig-is-almost-never-what-you-want)
- [process-wrap crate (Job Object + KillOnDrop)](https://crates.io/crates/process-wrap)
- [win32job crate](https://crates.io/crates/win32job)
- [Tauri: PyInstaller multi-process kill issue (#11686)](https://github.com/tauri-apps/tauri/issues/11686)
