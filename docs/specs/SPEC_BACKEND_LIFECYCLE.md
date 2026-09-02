# Backend Process Lifecycle — Analysis & Fix Spec

**Date:** 2026-03-16
**Status:** Draft
**Priority:** High — orphaned backends caused 95%+ system CPU on macOS

---

## 1. Problem Statement

When the AgentMux frontend (Tauri app) is closed, force-killed, or crashes, the backend
sidecar (`agentmuxsrv-rs`) can be left running as an orphan. Multiple orphaned backends
accumulate across app upgrades, consuming CPU and ports indefinitely.

**Observed incident:** Three `agentmuxsrv-rs` processes running simultaneously:
- v0.32.8 (current) — from active app
- v0.32.3 (stale) — from previous version
- v0.31.118 (stale, 67+ hours) — overflowed INT32 counters, 87% kernel CPU

The v0.31.118 process had been running for 67 hours with overflowed context-switch
and Mach message counters (2,147,483,647 = INT32_MAX), generating massive kernel
overhead and driving system CPU to 95%+.

**Platform:** macOS only. Windows is immune due to Job Objects (kernel-enforced).

---

## 2. Current Architecture

### Shutdown Mechanisms (3 layers)

| # | Mechanism | File | Platform | Survives SIGKILL? |
|---|-----------|------|----------|-------------------|
| 1 | `child.kill()` on last window close | `src-tauri/src/lib.rs:277-287` | All | No |
| 2 | Job Object (`KILL_ON_JOB_CLOSE`) | `src-tauri/src/sidecar.rs:19-59` | Windows | **Yes** |
| 3 | PPID watchdog (polls every 2s) | `agentmuxsrv-rs/src/main.rs:22-42` | macOS/Linux | Yes |
| 4 | stdin-EOF watcher | `agentmuxsrv-rs/src/main.rs:229-251` | All | Yes (if connected) |

### Why Each Fails on macOS

**Layer 1 — `child.kill()` on close:**
Works for normal window close. Does NOT run if:
- Frontend is `kill -9`'d
- Frontend crashes (panic handler may not reach this code)
- macOS force-quits the app
- DMG is unmounted while app is running

**Layer 2 — Job Object:**
Windows-only. No macOS equivalent exists in the OS.

**Layer 3 — PPID watchdog:**
Checks if `getppid()` changes from the original parent PID. Fails when:
- App launched via `open(1)` / Finder / Spotlight → launchd is the intermediary
- Tauri shell plugin may spawn via `posix_spawn` with process detachment
- If original PPID is already 1 (launchd), the watchdog never triggers because
  reparenting to launchd produces the same PID

**Layer 4 — stdin-EOF watcher:**
The backend reads from stdin and exits on EOF. But:
- **The frontend never connects a pipe to the sidecar's stdin**
- Tauri's `shell.sidecar().spawn()` captures stdout/stderr via `CommandEvent`
  but does not provide a stdin write handle
- The backend's stdin is inherited (likely /dev/null or terminal) — EOF never arrives
- This layer is effectively **dead code**

---

## 3. Research: Best Practices for Process Lifecycle

### 3.1 — Pipe-Based EOF Detection (Most Reliable, Cross-Platform)

The parent creates a pipe, holds the write end, and passes the read end to the child.
When the parent dies for ANY reason (including SIGKILL), the kernel closes the write end
and the child receives EOF.

```
Frontend                         agentmuxsrv-rs
  |                                   |
  |-- creates pipe ------------------>|
  |   holds write-end               reads from pipe (blocking)
  |                                   |
  |   [dies / SIGKILL / crash]        |
  |   kernel closes write-end         |
  |                                   |-- read() returns 0 (EOF)
  |                                   |-- graceful shutdown
```

**Properties:**
- Zero polling overhead (blocked on read)
- Survives SIGKILL — kernel closes file descriptors unconditionally
- Works on macOS, Linux, Windows (named pipes)
- Detection latency: near-instant (kernel event)
- **The single most reliable cross-platform mechanism**

**Why it's not working today:**
Tauri's shell plugin `spawn()` creates pipes for stdout/stderr but does NOT expose
a stdin write handle. The sidecar's stdin is not connected to a pipe from the frontend.

### 3.2 — kqueue with EVFILT_PROC + NOTE_EXIT (macOS-Specific, Event-Driven)

macOS (and all BSDs) support monitoring a specific PID for exit via kqueue:

```c
int kq = kqueue();
struct kevent kev;
EV_SET(&kev, parent_pid, EVFILT_PROC, EV_ADD | EV_ONESHOT, NOTE_EXIT, 0, NULL);
kevent(kq, &kev, 1, NULL, 0, NULL);  // register
// Block on kevent() — fires when parent_pid exits
```

**Properties:**
- Event-driven (no polling, no CPU waste)
- Works even if PPID was already 1 at startup (monitors specific PID, not parent relationship)
- Available via Rust `nix` crate (`nix::sys::event`)
- Detection latency: near-instant

**Race condition:** If the parent dies between `getppid()` and `kevent()` registration,
the event is missed. Fix: register kqueue, then immediately re-check `getppid()`.

**Caveat:** Requires knowing the frontend's PID. Must be passed explicitly as an argument
since `getppid()` may already be 1 on macOS.

### 3.3 — Process Groups (Kill Descendants)

Spawn the sidecar in its own process group. On shutdown, `kill(-pgid, SIGTERM)` kills
the entire group including any grandchild processes (shell sessions, etc.).

```rust
use std::os::unix::process::CommandExt;
Command::new("sidecar")
    .process_group(0)  // PGID = child PID
    .spawn()?;

// To kill entire group:
unsafe { libc::kill(-(child_pid as i32), libc::SIGTERM); }
```

**Properties:**
- Ensures grandchildren die with the sidecar
- Does NOT auto-trigger when parent dies — still needs a detection mechanism
- Useful for cleanup, not detection

**Limitation on macOS:** If a subprocess calls `setsid()` or `setpgid()`, it escapes
the group. macOS also lacks Linux's `PR_SET_CHILD_SUBREAPER`.

### 3.4 — Windows Job Objects (Already Implemented)

```rust
// JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE — kernel kills all processes in the job
// when the last handle to the job object is closed (i.e., frontend dies)
```

Already implemented in `sidecar.rs:19-59`. Kernel-enforced, survives SIGKILL.
**No changes needed for Windows.**

### 3.5 — Startup Orphan Cleanup (Defense in Depth)

On app launch, scan for stale backends and kill them before spawning a new one.
This catches any orphans that slipped through all other mechanisms.

### 3.6 — Heartbeat / Ping-Pong Liveness (Rejected)

Any approach using periodic liveness probing between frontend and backend
(heartbeat file polling, WebSocket ping/pong) causes **unstable operation**
and has been ruled out. This includes both file-based and socket-based variants.

### 3.7 — What NOT to Rely On

| Mechanism | Problem |
|-----------|---------|
| `App::drop` / Tauri cleanup on drop | Doesn't run on SIGKILL or crash |
| `getppid()` polling alone | Fails when original PPID is 1 |
| `PR_SET_PDEATHSIG` (Linux) | Tracks parent **thread**, not process — fires spuriously with Tokio |
| Signal handlers alone | SIGKILL cannot be caught |
| Tauri's built-in sidecar cleanup | Doesn't survive SIGKILL; doesn't kill grandchildren |

### 3.8 — Industry Comparison

**Electron:** Relies on Node.js event loop for cleanup. Orphans child processes ~30% of the
time on Windows when force-killed. No reliable mechanism for renderer-spawned processes.
Known issues: electron/electron#16317, electron/electron#7084.

**VS Code:** Uses a dedicated "watchdog" process + named pipes for lifecycle management.
The watchdog monitors both the main process and extension host, restarting or killing
as needed.

**iTerm2:** Uses `waitpid()` + session-based process groups. When a tab closes, sends
`SIGHUP` to the entire session's process group.

---

## 4. Existing Behavior & The Actual Gap

**What already works:** The backend tracks connected frontends via WebSocket. When the
last frontend disconnects cleanly (normal window close), the backend shuts itself down.
The `child.kill()` in the Tauri close handler is a belt-and-suspenders backup.

**The actual gap:** When a frontend is force-killed (SIGKILL, crash, macOS force-quit),
the WebSocket TCP connection enters a half-open state. The backend doesn't know the
frontend is dead until TCP keepalive expires — which by default on macOS is **~2 hours**
(`net.inet.tcp.keepidle = 7200`). During those 2 hours, the backend thinks a frontend
is still connected and stays alive as an orphan.

This is exactly what happened: the v0.31.118 backend ran for 67+ hours because it
never learned its frontend was gone.

---

## 5. Proposed Solution

### Primary: kqueue Parent Watcher (macOS) + pidfd (Linux)

**Goal:** Replace the broken PPID watchdog with event-driven OS-level monitoring.
When the spawning frontend dies, the backend detects it immediately and shuts down
if no other frontends are connected.

**Frontend changes** (`src-tauri/src/sidecar.rs`):
1. Pass `--parent-pid <PID>` argument when spawning the backend (frontend's own PID)

**Backend changes** (`agentmuxsrv-rs/src/main.rs`):
1. Accept `--parent-pid <PID>` CLI argument
2. Replace `start_ppid_watchdog()` with `start_parent_watcher(parent_pid)`:
   - **macOS:** register kqueue `EVFILT_PROC` + `NOTE_EXIT` on the parent PID.
     Event-driven, zero CPU overhead, fires immediately when the parent exits.
   - **Linux:** use `pidfd_open(parent_pid)` + `poll()` (kernel 5.3+).
     Falls back to existing PPID polling for older kernels.
   - **Windows:** no change needed (Job Objects already handle this).
3. When the parent death event fires:
   - Check if any other frontend WebSocket clients are still connected
   - If 0 connected clients → initiate graceful shutdown
   - If >0 connected clients → log warning, do nothing (other frontends still active)
4. Race condition guard: after kqueue registration, immediately verify parent is alive
   via `kill(parent_pid, 0)`. If already dead, handle immediately.

**Why this is sufficient:**
- Event-driven: zero polling overhead, near-instant detection
- Works even when original PPID is 1 (monitors specific PID, not parent relationship)
- Respects multi-frontend model: only shuts down if no other frontends are connected
- Combined with the existing `child.kill()` on normal close and Windows Job Objects,
  all platforms and shutdown scenarios are covered

### Supplementary: Startup Orphan Cleanup

**Goal:** Safety net that prevents orphan accumulation across version upgrades.

**Frontend changes** (`src-tauri/src/sidecar.rs`):
1. Before spawning a new backend, scan for running `agentmuxsrv-rs` processes
2. Kill any with a different `--instance` version than current (stale versions)
3. Send SIGTERM first, wait 3 seconds, then SIGKILL any survivors
4. Log all cleanup actions

**Implementation:** Shell out to `pgrep -f agentmuxsrv-rs` on macOS/Linux.
Not needed on Windows (Job Objects already handle this).

---

## 6. Implementation Plan

### Phase 1: kqueue Parent Watcher

**Effort:** Medium (2-3 hours)
**Risk:** Low

**Frontend (`src-tauri/src/sidecar.rs`):**
- Add `--parent-pid` argument to the sidecar spawn command (pass `std::process::id()`)

**Backend (`agentmuxsrv-rs/src/main.rs`):**
- Add `--parent-pid <PID>` to clap CLI args
- Replace `start_ppid_watchdog()` with `start_parent_watcher()`:
  - macOS: kqueue `EVFILT_PROC` + `NOTE_EXIT`
  - Linux: `pidfd_open()` + `poll()`, fallback to PPID polling
- On event: check WebSocket client count → shutdown if 0

### Phase 2: Startup Orphan Cleanup

**Effort:** Small (1-2 hours)
**Risk:** Low

**Frontend (`src-tauri/src/sidecar.rs`):**
- Add `cleanup_stale_backends()` called before `spawn_backend()`
- `pgrep -f agentmuxsrv-rs`, parse `--instance` args, kill different versions
- SIGTERM → wait 3s → SIGKILL

---

## 7. Dependency Changes

| Crate | Current | Action |
|-------|---------|--------|
| `nix` | Not used in agentmuxsrv-rs | Add for kqueue (macOS), pidfd (Linux) |
| `libc` | 0.2 (already in deps) | Already available for kill(), getppid() |
| `sysinfo` | 0.34 (already in deps) | Can use for process enumeration in cleanup |

---

## 8. Testing Strategy

### Unit Tests
- Verify kqueue watcher fires when monitored PID exits
- Verify pipe-EOF triggers shutdown token cancellation

### Integration Tests
1. **Normal close:** Start app → close last window → verify backend exits within 2s
2. **SIGKILL frontend:** Start app → `kill -9 <frontend_pid>` → verify backend exits within 5s
3. **Stale orphan cleanup:** Start old backend manually → launch new app → verify old backend is killed
4. **Multiple windows:** Open 3 windows → close 2 → verify backend still running → close last → verify backend exits
5. **Upgrade scenario:** Run v0.32.8 → install v0.32.9 → launch → verify v0.32.8 backend is cleaned up

### Platform Matrix
- macOS arm64 (Apple Silicon) — primary target
- macOS x64 (Intel) — verify kqueue works the same
- Windows x64 — verify Job Objects still work, no regressions
- Linux x64 — verify pidfd/PPID fallback

---

## 9. References

- [macOS kqueue(2)](https://developer.apple.com/library/archive/documentation/System/Conceptual/ManPages_iPhoneOS/man2/kqueue.2.html)
- [PDEATHSIG is Almost Never What You Want — Recall.ai](https://www.recall.ai/blog/pdeathsig-is-almost-never-what-you-want)
- [Tauri Sidecar Lifecycle — Issue #3062](https://github.com/tauri-apps/plugins-workspace/issues/3062)
- [Tauri Kill Process on Exit — Discussion #3273](https://github.com/tauri-apps/tauri/discussions/3273)
- [Waiting for Process Groups on macOS — Julio Merino](https://jmmv.dev/2019/11/wait-for-process-group-darwin.html)
- [process-wrap crate (successor to command-group)](https://github.com/watchexec/process-wrap)
- [kill_tree crate](https://lib.rs/crates/kill_tree)
- [dispatch_source_create(3) — GCD process monitoring](https://keith.github.io/xcode-man-pages/dispatch_source_create.3.html)
