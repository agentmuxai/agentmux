# Single Instance + New Window on Re-launch

**Date:** 2026-04-02
**Status:** Spec
**Problem:** Double-clicking agentmux.exe when already running opens Chrome (CEF singleton) instead of a new window

## Current Behavior

1. User launches `agentmux.exe` → app opens normally
2. User double-clicks `agentmux.exe` again → CEF detects the lockfile → delegates to "existing browser session" → Chrome opens at google.com

## Desired Behavior

1. User launches `agentmux.exe` → app opens normally
2. User double-clicks `agentmux.exe` again → existing instance opens a new window (same as clicking the version button)

## How Other Apps Do This

### Electron
- Uses `app.requestSingleInstanceLock()`
- Second instance sends argv to first via IPC (named pipe on Windows)
- First instance receives `second-instance` event → opens new window
- Second instance exits

### VS Code
- Uses a named mutex + socket file
- Second launch detects mutex → sends "open" command via socket → exits

### Spotify (CEF-based)
- Uses a named mutex to detect existing instance
- Second launch sends command via localhost HTTP

## Proposed Approach

### Detection: Named Mutex (Windows)

```rust
// In main.rs, before CEF init:
let mutex_name = format!("AgentMux-CEF-v{}", version_slug);
let mutex = CreateMutexW(null(), FALSE, wide_string(&mutex_name));
if GetLastError() == ERROR_ALREADY_EXISTS {
    // Another instance is running — send "new window" command and exit
    send_new_window_request(ipc_port);
    std::process::exit(0);
}
```

### Communication: Localhost HTTP

The IPC server is already running on a random port. The second instance needs to know which port. Options:

**Option A: Port file**
- First instance writes port to `~/.agentmux/cef-ipc-port` (or version-specific path)
- Second instance reads it, sends `POST /ipc` with `{cmd: "open_new_window"}`

**Option B: Named pipe**
- First instance creates `\\.\pipe\AgentMux-v{version}`
- Second instance connects, sends "new-window" → first instance handles

**Option C: Registry key**
- First instance writes port to `HKCU\Software\AgentMux\ipc_port`
- Second instance reads and sends HTTP request

**Recommendation: Option A (port file)** — simplest, cross-platform, no Windows-specific APIs beyond the mutex.

## Implementation Plan

### Step 1: Write IPC port to file on startup

**File:** `agentmux-cef/src/main.rs`

After IPC server starts:
```rust
let port_file = data_dir.join("ipc-port");
std::fs::write(&port_file, ipc_port.to_string()).ok();
// Clean up on exit:
// (handled by Drop or shutdown hook)
```

### Step 2: Check for existing instance before CEF init

**File:** `agentmux-cef/src/main.rs`

Before `cef::initialize()`:
```rust
#[cfg(target_os = "windows")]
{
    let mutex_name: Vec<u16> = format!("AgentMux-CEF-{}\0", version_slug)
        .encode_utf16().collect();
    unsafe {
        let mutex = windows_sys::Win32::System::Threading::CreateMutexW(
            std::ptr::null(), 0, mutex_name.as_ptr(),
        );
        if windows_sys::Win32::Foundation::GetLastError() == 183 { // ERROR_ALREADY_EXISTS
            // Read port file, send new-window request, exit
            if let Ok(port_str) = std::fs::read_to_string(&port_file) {
                if let Ok(port) = port_str.trim().parse::<u16>() {
                    let _ = reqwest::blocking::Client::new()
                        .post(format!("http://127.0.0.1:{}/ipc", port))
                        .json(&serde_json::json!({"cmd": "open_new_window"}))
                        .send();
                }
            }
            std::process::exit(0);
        }
    }
}
```

Note: Using `reqwest` adds a dependency. Alternative: raw TCP/HTTP with `std::net::TcpStream`.

### Step 3: Clean up port file on shutdown

**File:** `agentmux-cef/src/main.rs`

After `run_message_loop()` returns:
```rust
let _ = std::fs::remove_file(&port_file);
```

### Step 4: Auth token for IPC

The IPC server requires `Authorization: Bearer {token}`. The second instance needs the token. Options:
- Write token to port file too: `port:token` format
- Or use a separate token file
- Or skip auth for `open_new_window` specifically (risky)

**Recommendation:** Write `port:token` to the port file:
```
58234:a47a4f18-1234-5678-abcd-ef0123456789
```

### Step 5: Handle lockfile cleanup

Currently we remove the lockfile on startup. With mutex detection, the second instance exits before touching CEF — no lockfile conflict. The first instance's lockfile is valid.

Remove the lockfile cleanup code? No — keep it as a safety net for crashes where the mutex is released but lockfile remains.

## Edge Cases

| Case | Handling |
|------|----------|
| Port file stale (old instance crashed) | HTTP request fails → fall through to normal launch |
| Mutex released but port file exists | Normal launch (mutex check passes) |
| Multiple versions running | Each version has its own mutex name + port file |
| IPC token mismatch | Request rejected → fall through to normal launch |
| Firewall blocks localhost | Unlikely but possible → fall through to normal launch |

## Files Changed

1. `agentmux-cef/src/main.rs` — mutex check, port file write/read/cleanup
2. `agentmux-cef/src/ipc.rs` — possibly skip auth for `open_new_window` from second instance
3. No frontend changes needed

## Dependencies

- No new crate dependencies if using raw `std::net::TcpStream` for HTTP
- Or add `ureq` (lightweight HTTP client) for cleaner code

## Complexity

Low-medium. ~50 lines in main.rs. The IPC server and `open_new_window` command already exist.

## Test Plan

- [ ] First launch: works normally, port file created
- [ ] Second launch (same version): opens new window in first instance, second exits
- [ ] Kill first instance → second launch: starts fresh (mutex released)
- [ ] Crash first instance → second launch: port file stale, HTTP fails, starts fresh
- [ ] Different versions: each runs independently
