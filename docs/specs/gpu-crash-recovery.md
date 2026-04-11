# GPU Crash Recovery Spec

**Date:** 2026-04-04
**Status:** Draft
**Context:** agentmux-cef v0.33.42 experienced a GPU crash loop (80+ restarts in 6s) on April 4, 2026. No recovery mechanism exists. Full GPU acceleration is enabled with no fallback flags.

---

## Problem

When the CEF GPU process crashes repeatedly, Chromium's internal fallback stack (HARDWARE_GL → SWIFTSHADER → DISPLAY_COMPOSITOR) can exhaust. If all modes fail, Chromium calls `IntentionallyCrashBrowserForUnusableGpuProcess()` — killing the entire application with no user-facing recovery.

**Observed crash sequence (April 4, 2026):**
```
11:01:59 — GPU process starts crash-looping
           ~80 restart attempts in 6 seconds
           Each: kFatalFailure: Failed to create shared context for virtualization
11:02:05 — WER dump: agentmux-cef.exe.24548.dmp
11:11:45 — GPU process stabilizes but degraded:
           Shared memory region create failed on 3241056 bytes
           No displays detected. Waiting for next update.
           Failed to retrieve D3D11 device
11:12:00 — WER dump: agentmux-cef.exe.36392.dmp
```

**Root cause:** GPU driver instability or resource exhaustion. Exit code `0x80000003` (`STATUS_BREAKPOINT`) indicates an internal Chromium assertion or driver-triggered breakpoint.

---

## Chromium's Built-in Behavior (What We Get for Free)

1. **Crash counter with forgiveness:** 1 crash forgiven per 5 minutes. 3 crashes within the forgiveness window triggers fallback to next GPU mode.
2. **Fallback stack on Windows:** `HARDWARE_GL` → `SWIFTSHADER` → `DISPLAY_COMPOSITOR`
3. **Fatal termination:** If the stack is empty, browser process crashes intentionally.
4. **No CEF API for GPU crashes:** There is no `OnGpuProcessCrashed` callback. `OnRenderProcessTerminated` only fires for renderer crashes.

---

## Design

### Layer 1: Persistent GPU Health Tracking

Track GPU crash history across sessions in a config file (`~/.agentmux/<version>/gpu-health.json`):

```json
{
  "gpu_mode": "hardware",
  "crash_count": 0,
  "last_crash_ts": null,
  "fallback_active": false,
  "driver_version": "32.0.15.6590",
  "gpu_vendor": "NVIDIA",
  "gpu_device": "GeForce GTX 1070"
}
```

**On startup:** Read this file before `CefInitialize()`. If `crash_count >= 3` within the last 24 hours, add `--disable-gpu-compositing` to command-line args. If `crash_count >= 6`, add `--disable-gpu`.

**On clean shutdown:** Reset `crash_count` to 0.

**On abnormal exit:** The file retains its last state. Next launch reads the stale `crash_count` and applies the appropriate fallback.

### Layer 2: Watchdog Launcher

A lightweight launcher process (`agentmux-launcher.exe`) that:

1. Spawns `agentmux-cef.exe` as a child process
2. Monitors its exit code
3. On abnormal exit (non-zero, especially `0x80000003` or `0xC0000409`):
   - Increments `crash_count` in `gpu-health.json`
   - Relaunches with escalated GPU flags based on crash count:
     - 1st crash: relaunch normally (Chromium likely already fell back internally)
     - 2nd crash: relaunch with `--disable-gpu-compositing`
     - 3rd crash: relaunch with `--disable-gpu`
     - 4th crash: show error dialog, do not relaunch
4. On clean exit (code 0): exit normally

```
┌─────────────────────┐
│ agentmux-launcher   │
│                     │
│  spawn ──► agentmux-cef.exe
│  wait for exit      │
│                     │
│  exit code != 0?    │
│    ├─ bump crash_count
│    ├─ pick GPU flags │
│    └─ relaunch       │
│                     │
│  exit code == 0?    │
│    └─ exit           │
└─────────────────────┘
```

### Layer 3: In-Process GPU Health Monitor

Inside `agentmux-cef`, monitor for signs of GPU degradation at runtime:

1. **Render heartbeat:** The frontend sends a periodic heartbeat (every 5s) via IPC to the backend. If the backend stops receiving heartbeats for 30s, the renderer is likely frozen due to GPU issues.

2. **Parse CEF log output:** Hook into CEF's log callback (`CefLogSeverity`) and watch for:
   - `"GPU process exited unexpectedly"` — increment an in-memory counter
   - `"Shared memory region create failed"` — flag memory pressure
   - `"No displays detected"` — flag display loss
   - `"kFatalFailure"` — flag context creation failure

3. **User notification:** When GPU degradation is detected, show a non-blocking banner in the UI:
   > "Hardware acceleration encountered an issue and has been reduced. [Restart with full acceleration] [Keep current mode]"

### Layer 4: User-Facing Settings

Add a setting in the app preferences:

```
Hardware Acceleration: [On] [Off] [Auto]
```

- **On:** No GPU-disabling flags (default for fresh install)
- **Off:** Always pass `--disable-gpu --disable-gpu-compositing`
- **Auto:** Use Layer 1 crash tracking to decide at launch

Persisted in `~/.agentmux/<version>/settings.json` as `"gpu_acceleration": "auto" | "on" | "off"`.

---

## Implementation Plan

### Phase 1: Persistent Health File (Low effort, high impact)

**Files:** `agentmux-cef/src/main.rs`

- On startup (before `CefInitialize`): read `gpu-health.json`, apply flags if needed
- On clean shutdown (`CefShutdown` completes): write `crash_count: 0`
- Log the GPU mode decision at startup

### Phase 2: Watchdog Launcher (Medium effort)

**New crate:** `agentmux-launcher/`

- Minimal Rust binary (~100 lines)
- Spawns `agentmux-cef.exe` with appropriate args
- Monitors exit code, updates `gpu-health.json`
- Handles relaunch logic with escalating GPU flags
- Update portable build script to use launcher as entry point

### Phase 3: In-Process Monitor (Medium effort)

**Files:** `agentmux-cef/src/app.rs`, `agentmux-cef/src/client.rs`, frontend IPC

- CEF log hook for GPU-related messages
- Frontend heartbeat timer → backend IPC endpoint
- UI banner component for GPU degradation notification

### Phase 4: User Settings (Low effort, depends on Phase 1)

**Files:** `agentmux-cef/src/main.rs`, frontend settings UI

- Add GPU acceleration toggle to settings
- Read setting on startup, override health-file logic if set to `on` or `off`

---

## CEF Command-Line Flags Reference

| Escalation Level | Flags | Effect |
|---|---|---|
| Normal | (none) | Full GPU acceleration |
| Level 1 | `--disable-gpu-compositing` | Software compositing, GPU still used for WebGL |
| Level 2 | `--disable-gpu` | No GPU process at all, full software rendering |
| Level 3 | `--disable-gpu --disable-software-rasterizer` | No GPU, no SwiftShader (minimal, last resort) |

**Note:** `--use-angle=swiftshader` (SwANGLE) is being deprecated upstream. Do not rely on it as a fallback.

---

## Windows-Specific Considerations

### TDR (Timeout Detection and Recovery)

- Windows resets the GPU driver if it hangs for >2 seconds
- 5+ TDRs within 60 seconds = BSOD (`VIDEO_TDR_TIMEOUT_DETECTED`)
- TDR events are transparent to the app but contribute to Chromium's internal crash counter
- Cannot be detected reliably from user-mode; would need to poll Event Viewer (`System` log, source `Display`)

### Shared Memory Failures

The `Shared memory region create failed on 3241056 bytes` error indicates page file exhaustion or commit limit reached. Mitigation:
- Ensure system page file is set to "System managed" (not a fixed small size)
- Monitor commit charge on long-running instances
- Not actionable from the app side — this is an OS resource limit

### Exit Codes to Watch

| Code | Hex | Meaning |
|---|---|---|
| -2147483645 | 0x80000003 | STATUS_BREAKPOINT (assertion/debugbreak) |
| -1073740791 | 0xC0000409 | STATUS_STACK_BUFFER_OVERRUN (fast-fail) |
| -1073741819 | 0xC0000005 | STATUS_ACCESS_VIOLATION (segfault) |
| -1073741510 | 0xC000013A | STATUS_CONTROL_C_EXIT (Ctrl+C) |

---

## Open Questions

1. **Should the launcher be a separate binary or integrated into agentmux-cef.exe with a `--watchdog` flag?** Separate binary is simpler but adds a file to the portable distribution.

2. **How long should crash history persist?** Current proposal: 24 hours. Too short and repeat offenders aren't caught. Too long and a one-time driver glitch permanently degrades the experience.

3. **Should we collect GPU info (vendor, device, driver version) at startup for diagnostics?** CEF exposes this via the GPU process but only after initialization.

4. **Do we want to detect TDR events proactively?** Polling Event Viewer is expensive and unreliable. Probably not worth it.
