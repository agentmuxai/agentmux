# Process Tree Grouping Analysis

**Date:** 2026-04-05
**Issue:** AgentMux processes appear as independent background processes in Task Manager instead of grouped under one app entry.

---

## Findings

### How Task Manager Groups Processes

Task Manager's "Processes" tab has two sections:
- **"Apps"** — processes that own a **visible top-level window**
- **"Background processes"** — everything else

Child processes appear nested under their parent **only if the parent itself is in the "Apps" section** (owns a visible window). If the parent is invisible, its children appear as independent entries.

### The Process Tree

```
agentmux.exe (launcher)                    ← windows_subsystem = "windows", NO visible window
  └─ agentmux-cef-0.33.45.exe (browser)   ← creates CEF windows, SHOULD be an "App"
       ├─ agentmux-cef-0.33.45.exe --type=gpu-process
       ├─ agentmux-cef-0.33.45.exe --type=renderer
       ├─ agentmux-cef-0.33.45.exe --type=utility
       └─ agentmux-srv-0.33.45-windows.x64.exe (backend sidecar)
            ├─ pwsh.exe (terminal pane 1)
            ├─ pwsh.exe (terminal pane 2)
            └─ bash.exe (terminal pane 3)
```

### Evidence: What Controls the Grouping

| File | Line | Code | Effect |
|---|---|---|---|
| `agentmux-launcher/src/main.rs` | 12-15 | `windows_subsystem = "windows"` | Launcher has NO console, NO visible window |
| `agentmux-launcher/src/main.rs` | 56-58 | `Command::new(&real_exe).args(&args).status()` | Spawns CEF as child, no special flags |
| `agentmux-cef/src/main.rs` | 18-22 | `windows_subsystem = "windows"` | CEF host also no console |
| `agentmux-cef/src/sidecar.rs` | 120 | `cmd.creation_flags(0x08000000)` | Backend spawned with `CREATE_NO_WINDOW` |
| `agentmux-cef/src/sidecar.rs` | 134-153 | Job Object `KILL_ON_JOB_CLOSE` | Ties backend lifecycle to CEF, does NOT affect TM grouping |

### The Problem

The launcher (`agentmux.exe`) is the process tree root but has `windows_subsystem = "windows"` and never creates a visible window. It just spawns the CEF host and calls `.status()` to wait.

Task Manager sees:
1. `agentmux.exe` — no window → "Background processes"
2. `agentmux-cef-0.33.45.exe` — has CEF windows → should be "Apps", BUT its parent is a background process
3. Backend + shells — no windows → "Background processes"

The result: everything scatters across "Background processes" instead of grouping under one "Apps" entry.

### When Did This Break?

**This is NOT caused by the v0.33.44 versioned names change.** The launcher architecture was introduced earlier. Before the launcher existed, `agentmux-cef.exe` was run directly — it owned the windows and appeared under "Apps" with all its children grouped.

The launcher added an invisible wrapper that broke the grouping because Task Manager doesn't group children under an invisible parent.

### Previous Behavior (Pre-Launcher)

```
[Apps]
  AgentMux CEF                              ← agentmux-cef.exe owned the windows directly
    ├─ agentmux-cef.exe (gpu)
    ├─ agentmux-cef.exe (renderer)
    └─ agentmux-srv-... (backend)
         ├─ pwsh.exe
         └─ bash.exe
```

### Current Behavior (With Launcher)

```
[Background processes]
  agentmux.exe                              ← invisible launcher
  AgentMux CEF v0.33.45                     ← CEF host, has windows but parent is invisible
  agentmux-cef-0.33.45.exe                  ← GPU subprocess
  agentmux-cef-0.33.45.exe                  ← renderer subprocess  
  agentmux-srv-0.33.45-windows.x64.exe      ← backend
  pwsh.exe                                   ← shell 1
  pwsh.exe                                   ← shell 2
```

---

## Solution Options

### Option A: Launcher Exits After Spawn (Recommended)

Make the launcher exec-replace itself with the CEF host on Windows. Since Windows has no `execvp()`, the launcher can:
1. Spawn the CEF host as a **detached** process (not a child)
2. Exit immediately

The CEF host becomes a top-level process that owns windows → appears under "Apps". The launcher is gone.

**Downside:** Launcher can't monitor exit codes for watchdog/restart logic. But that's Phase 2 anyway.

**Implementation:**
```rust
// agentmux-launcher/src/main.rs
#[cfg(target_os = "windows")]
{
    use std::os::windows::process::CommandExt;
    // DETACHED_PROCESS = 0x00000008
    // CREATE_NEW_PROCESS_GROUP = 0x00000200
    let child = std::process::Command::new(&real_exe)
        .args(&args)
        .creation_flags(0x00000008) // DETACHED_PROCESS
        .spawn();
    match child {
        Ok(_) => std::process::exit(0), // launcher exits, CEF is now top-level
        Err(e) => {
            eprintln!("Failed to launch AgentMux: {}", e);
            std::process::exit(1);
        }
    }
}
```

### Option B: Launcher Creates a Hidden HWND

The launcher creates an invisible window (size 0x0 or off-screen) so Task Manager recognizes it as an "App". Its children then group under it.

**Downside:** More code, hacky, and the launcher's invisible window could interfere with focus/activation.

### Option C: Move DLL Path Setup Into CEF Host

The only reason the launcher exists is to call `SetDllDirectoryW` before the CEF host's load-time dependency on `libcef.dll` is resolved. If we can solve the DLL search path differently, the launcher is unnecessary:
- Use a `.local` file (DLL redirection)
- Use an application manifest with `<probing>` element
- Delay-load `libcef.dll`

**Downside:** Significant rearchitecture, may not be feasible with CEF's load-time linking.

### Option D: Accept Current Behavior

Multiple background process entries is technically correct. The grouping is cosmetic.

---

## Recommendation

**Option A** is the simplest fix. The launcher spawns the CEF host as a detached process and exits. The CEF host becomes the top-level process owner, Task Manager groups everything under it.

The only thing lost is the launcher's ability to monitor the CEF host's exit code (for future watchdog logic). That can be addressed separately when implementing the GPU crash recovery watchdog — the watchdog would be a separate persistent process, not the launcher.
