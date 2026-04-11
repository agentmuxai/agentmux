# Memory Heartbeat Status Report — v0.33.60

**Date:** 2026-04-07
**PR:** #311 (`agenta/memory-heartbeat-inprocess-gpu`)
**Build:** v0.33.60 portable

---

## What Was Added (PR #311)

1. **`--in-process-gpu`** — merges GPU process into browser, eliminates one ~100GB VA reservation
2. **`--renderer-process-limit=1`** — caps renderer subprocess spawns
3. **`memory_heartbeat` module** — logs system + process memory every 20s

## Heartbeat Implementation

**File:** `agentmux-cef/src/memory_heartbeat.rs` (121 lines)

- Spawns background thread named `mem-heartbeat`
- Sleeps 20s between logs (sleeps before first log to avoid startup burst)
- Uses Win32 APIs: `GlobalMemoryStatusEx()` + `GetProcessMemoryInfo()`
- Emits two `tracing::info!` entries per cycle to target `mem_heartbeat`:
  - **System memory:** load%, total/available phys RAM, page file, virtual memory
  - **Process memory:** working set, peak WS, commit, peak commit, page faults

## Current Status: NOT OBSERVABLE IN PORTABLE MODE

### The Problem

The CEF host initializes tracing to write to **stderr only** (`main.rs:58-64`):

```rust
tracing_subscriber::fmt()
    .with_env_filter(...)
    .with_writer(std::io::stderr)
    .init();
```

In portable mode, the launcher spawns `agentmux-cef-0.33.60.exe` as a GUI process. **stderr is not captured anywhere.** The heartbeat thread is running and logging, but the output goes to `/dev/null`.

### Evidence

- No log file exists for v0.33.60 in `~/.agentmux/logs/`
- Last host log: `agentmux-host-v0.33.1.log.2026-03-30`
- No `.log` files in the portable directory
- The sidecar (`agentmux-srv`) writes its own log files to `~/.agentmux/logs/` but the CEF host does not

### Where It DOES Work

- `task dev` — stderr is visible in the terminal
- Any launch where stderr is redirected: `agentmux.exe 2>heartbeat.log`

## Recommendation: Add File Logging to CEF Host

The heartbeat's entire purpose is forensic crash analysis — the data must persist on disk to be useful. Two options:

### Option A: File appender in CEF host (recommended)

Add `tracing-appender` to write to `~/.agentmux/logs/agentmux-host-v{VERSION}.log.{DATE}`:

```rust
use tracing_appender::rolling;
let file_appender = rolling::daily(&log_dir, format!("agentmux-host-v{}.log", version));
let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);

tracing_subscriber::fmt()
    .with_env_filter(...)
    .with_writer(non_blocking)  // file instead of stderr
    .init();
```

This matches what the sidecar already does and would restore the `agentmux-host-v*.log.*` files that stopped appearing after v0.33.1.

### Option B: Launcher captures stderr

Have `agentmux-launcher` redirect the child's stderr to a log file. Simpler but less flexible.

## Other Heartbeat Systems (for context)

The codebase has additional health monitoring unrelated to the memory heartbeat:

| System | Location | Purpose |
|--------|----------|---------|
| Agent Health Monitor | `agentmux-srv/.../health.rs` | Tracks agent subprocess state (Healthy→Degraded→Stalled→Dead) |
| Parent Process Watcher | `agentmux-srv/src/main.rs` | Detects frontend death, shuts down backend |
| Process Watchdog | `agentmux-srv/.../watchdog.rs` | Kills panes exceeding max-runtime/idle limits |
| IPC Health Endpoint | `agentmux-cef/src/ipc.rs` | `GET /health` → `{"status":"ok","version":"..."}` |

These all function independently. Only the memory heartbeat has the stderr-sink issue.

---

**Bottom line:** The heartbeat code is correct and running, but its output is invisible in portable builds. Adding a file appender is a ~10 line fix that makes the forensic data actually available for crash analysis.
