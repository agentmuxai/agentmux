# Single-Instance Mode with IPC

**Part of:** REACTIVE_UI_SPEC_V2.md
**Version:** 1.0
**Date:** 2025-10-14

---

## Requirement

**Single Instance Principle:** Only one GUI instance of agentmux-desktop should run at a time.

When user runs CLI command while GUI is already running:
- ❌ **Don't** start new GUI instance
- ✅ **Do** send command to existing instance via IPC
- ✅ **Do** focus/show existing GUI window
- ✅ **Do** execute command and update UI
- ✅ **Do** return result to CLI caller's terminal

---

## Architecture

```
┌──────────────────────────────────────────────────────────────┐
│  Terminal: User runs "agentmux agent spawn Agent3"          │
└────────────────┬─────────────────────────────────────────────┘
                 │
                 ↓
┌─────────────────────────────────────────────────────────────┐
│  Check for existing instance (lock file or TCP port)        │
└────┬──────────────────────────────────────┬─────────────────┘
     │                                       │
     │ No instance running                   │ Instance found
     ↓                                       ↓
┌──────────────────────┐          ┌─────────────────────────┐
│  Start new GUI       │          │  Send IPC message to    │
│  Execute command     │          │  running instance       │
│  Show UI             │          │  - Command: spawn Agent3│
└──────────────────────┘          │  - Caller PID           │
                                  └─────────┬───────────────┘
                                           │
                                           ↓
                                  ┌─────────────────────────┐
                                  │  Running GUI Instance   │
                                  │  - Receives IPC message │
                                  │  - Executes command     │
                                  │  - Emits events         │
                                  │  - Updates UI           │
                                  │  - Focuses window       │
                                  │  - Sends result back    │
                                  └─────────┬───────────────┘
                                           │
                                           ↓
                                  ┌─────────────────────────┐
                                  │  CLI process receives   │
                                  │  result and prints to   │
                                  │  terminal, then exits   │
                                  └─────────────────────────┘
```

---

## Implementation Options

### Option A: Tauri Single-Instance Plugin (Recommended)

**Pros:**
- Built into Tauri
- Cross-platform
- Simple API

**Cons:**
- Limited - only sends argv, can't get response back easily

```rust
// Cargo.toml
[dependencies]
tauri-plugin-single-instance = "2.0.0"

// main.rs
use tauri_plugin_single_instance::init as single_instance;

fn main() {
    tauri::Builder::default()
        .plugin(single_instance(|app, argv, cwd| {
            println!("Second instance detected!");
            println!("argv: {:?}", argv);

            // Parse CLI command from argv
            if argv.len() > 1 {
                let command_args = &argv[1..];
                let handle = app.clone();

                tauri::async_runtime::spawn(async move {
                    // Execute command
                    let result = execute_cli_from_args(command_args, handle.clone()).await;

                    // Focus window
                    if let Some(window) = handle.get_window("main") {
                        let _ = window.set_focus();
                        let _ = window.show();
                        let _ = window.unminimize();
                    }

                    // Print result (goes to second instance's stdout)
                    println!("{}", result.output);
                });
            }
        }))
        .setup(|app| {
            // ... existing setup ...
            Ok(())
        })
        .invoke_handler(...)
        .run(...)
}
```

**Problem:** Second instance exits immediately, can't wait for result.

### Option B: HTTP IPC Server (Recommended)

**Pros:**
- Bidirectional communication
- Can wait for response
- Simple request/response model
- Cross-platform

**Cons:**
- Requires HTTP server dependency
- Need to manage port allocation

```rust
// Cargo.toml
[dependencies]
tiny_http = "0.12"

// main.rs - Start HTTP server in setup()
fn setup_ipc_server(app_handle: AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    // Start on random available port
    let server = tiny_http::Server::http("127.0.0.1:0")?;
    let port = server.server_addr().port();

    // Write lock file with port
    write_lock_file(port)?;

    // Spawn server thread
    std::thread::spawn(move || {
        for request in server.incoming_requests() {
            handle_ipc_request(request, app_handle.clone());
        }
    });

    Ok(())
}

fn handle_ipc_request(request: tiny_http::Request, app_handle: AppHandle) {
    // Read command from request body
    let mut body = String::new();
    if let Err(e) = request.as_reader().read_to_string(&mut body) {
        let _ = request.respond(tiny_http::Response::from_string(format!("Error: {}", e)));
        return;
    }

    // Parse command
    let command: IpcCommand = match serde_json::from_str(&body) {
        Ok(cmd) => cmd,
        Err(e) => {
            let _ = request.respond(tiny_http::Response::from_string(format!("Parse error: {}", e)));
            return;
        }
    };

    // Execute command (async)
    let result = tauri::async_runtime::block_on(async {
        execute_ipc_command(command, app_handle.clone()).await
    });

    // Focus window
    if let Some(window) = app_handle.get_window("main") {
        let _ = window.set_focus();
        let _ = window.show();
        let _ = window.unminimize();
    }

    // Send response
    let response_json = serde_json::to_string(&result).unwrap();
    let _ = request.respond(tiny_http::Response::from_string(response_json));
}

// CLI checks lock file and sends HTTP request
fn send_to_existing_instance(command: &str) -> Result<String, String> {
    let lock = read_lock_file()?;

    // Check if process is still running
    if !is_process_running(lock.pid) {
        // Stale lock file, remove it
        remove_lock_file()?;
        return Err("No running instance found (stale lock)".to_string());
    }

    // Send HTTP POST request
    let client = reqwest::blocking::Client::new();
    let response = client
        .post(&format!("http://127.0.0.1:{}/command", lock.ipc_port))
        .json(&command)
        .timeout(std::time::Duration::from_secs(30))
        .send()
        .map_err(|e| format!("IPC error: {}", e))?;

    response.text().map_err(|e| e.to_string())
}
```

### Option C: Named Pipes (Platform-Specific)

**Pros:**
- Fast, efficient
- OS-native

**Cons:**
- Platform-specific code (Unix vs Windows)
- More complex

**Not recommended** due to complexity.

---

## Lock File

### Location

**Unix/macOS:** `~/.agentmux/desktop.lock`
**Windows:** `%LOCALAPPDATA%\agentmux\desktop.lock`

### Format

```json
{
  "pid": 12345,
  "ipc_port": 54321,
  "started_at": "2025-10-14T10:30:00Z",
  "version": "0.3.3"
}
```

### Stale Detection

```rust
fn is_lock_stale(lock: &LockFile) -> bool {
    !is_process_running(lock.pid)
}

#[cfg(unix)]
fn is_process_running(pid: u32) -> bool {
    use nix::sys::signal::{kill, Signal};
    use nix::unistd::Pid;

    match kill(Pid::from_raw(pid as i32), Signal::SIGCONT) {
        Ok(_) => true,
        Err(_) => false,
    }
}

#[cfg(windows)]
fn is_process_running(pid: u32) -> bool {
    use winapi::um::processthreadsapi::{OpenProcess};
    use winapi::um::winnt::PROCESS_QUERY_INFORMATION;

    unsafe {
        let handle = OpenProcess(PROCESS_QUERY_INFORMATION, 0, pid);
        if handle.is_null() {
            false
        } else {
            winapi::um::handleapi::CloseHandle(handle);
            true
        }
    }
}
```

---

## IPC Message Protocol

### Request Format

```typescript
interface IpcCommand {
  command_type: "agent" | "bus" | "logs" | "status";
  action: string;
  args: Record<string, any>;
  caller_pid?: number;
}

// Example
{
  "command_type": "agent",
  "action": "spawn",
  "args": {
    "instance_name": "Agent3"
  },
  "caller_pid": 54321
}
```

### Response Format

```typescript
interface IpcResponse {
  success: boolean;
  output: string;
  data?: any;
  error?: string;
  duration_ms: number;
}

// Success example
{
  "success": true,
  "output": "✓ Agent spawned: Agent3 (PID: 5678)",
  "data": {
    "instance_name": "Agent3",
    "pid": 5678,
    "ws_port": 9001
  },
  "duration_ms": 125
}

// Error example
{
  "success": false,
  "output": "Failed to spawn agent",
  "error": "Port 9001 already in use",
  "duration_ms": 10
}
```

---

## Implementation Flow

### Step 1: Startup Check

```rust
fn main() {
    // Parse CLI args
    let cli = cli::parser::Cli::try_parse();

    // Check for existing instance
    if let Ok(lock) = read_lock_file() {
        if !is_lock_stale(&lock) {
            // Instance running, send IPC
            if let Ok(ref parsed) = cli {
                if let Some(command) = &parsed.command {
                    match send_ipc_command(command, lock.ipc_port) {
                        Ok(response) => {
                            println!("{}", response.output);
                            std::process::exit(if response.success { 0 } else { 1 });
                        }
                        Err(e) => {
                            eprintln!("IPC error: {}", e);
                            std::process::exit(1);
                        }
                    }
                }
            }
        } else {
            // Stale lock, remove it
            let _ = remove_lock_file();
        }
    }

    // No existing instance or stale lock - start new instance
    tauri::Builder::default()
        .setup(|app| {
            // Start IPC server
            setup_ipc_server(app.handle())?;

            // Execute CLI command if provided
            // ... existing hybrid mode code ...

            Ok(())
        })
        .invoke_handler(...)
        .run(...)
}
```

### Step 2: IPC Server Setup

```rust
fn setup_ipc_server(app_handle: AppHandle) -> Result<(), Box<dyn std::error::Error>> {
    use tiny_http::{Server, Response};

    // Start on random port
    let server = Server::http("127.0.0.1:0")?;
    let port = server.server_addr().port();

    // Write lock file
    write_lock_file(LockFile {
        pid: std::process::id(),
        ipc_port: port,
        started_at: chrono::Utc::now(),
        version: env!("CARGO_PKG_VERSION").to_string(),
    })?;

    // Spawn server thread
    std::thread::spawn(move || {
        println!("IPC server listening on port {}", port);

        for request in server.incoming_requests() {
            let handle = app_handle.clone();
            std::thread::spawn(move || {
                handle_ipc_request(request, handle);
            });
        }
    });

    Ok(())
}
```

### Step 3: Command Execution

```rust
async fn execute_ipc_command(command: IpcCommand, app_handle: AppHandle) -> IpcResponse {
    let start = std::time::Instant::now();

    // Convert IPC command to CLI command
    let cli_command = ipc_to_cli_command(command);

    // Execute via existing CLI handler
    let result = cli::handlers::handle_command(
        cli_command,
        cli::output::OutputFormat::Text,
        None,  // State
        Some(app_handle.clone()),
    ).await;

    IpcResponse {
        success: result.success,
        output: result.output,
        data: result.json_output,
        error: if result.success { None } else { Some(result.output.clone()) },
        duration_ms: start.elapsed().as_millis() as u64,
    }
}
```

---

## Implementation Checklist

- [ ] Add `tiny_http` dependency
- [ ] Implement lock file read/write
- [ ] Implement stale lock detection (`is_process_running`)
- [ ] Add IPC server setup in `main.rs setup()`
- [ ] Implement `handle_ipc_request` function
- [ ] Implement `execute_ipc_command` function
- [ ] Add startup check for existing instance
- [ ] Implement `send_ipc_command` for CLI
- [ ] Add window focus/show on IPC request
- [ ] Handle IPC timeouts (30s)
- [ ] Clean up lock file on app exit
- [ ] Test: CLI with GUI running
- [ ] Test: GUI with CLI running
- [ ] Test: Stale lock file removal
- [ ] Test: Multiple rapid CLI commands
- [ ] Test: IPC server crash recovery

---

## Testing Scenarios

### Scenario 1: Normal Flow

```bash
# Terminal 1
agentmux-desktop
# GUI opens, IPC server starts on port 54321

# Terminal 2
agentmux agent spawn Agent3
# Sends HTTP POST to localhost:54321
# GUI receives command, spawns agent, updates UI
# Terminal 2 shows: "✓ Agent spawned: Agent3 (PID: 5678)"
```

### Scenario 2: Stale Lock

```bash
# GUI was running but crashed
cat ~/.agentmux/desktop.lock
# {"pid": 12345, "ipc_port": 54321, ...}

ps -p 12345
# Process not found

agentmux agent list
# Detects stale lock, removes it, starts new instance
```

### Scenario 3: Timeout

```bash
# GUI frozen/unresponsive
agentmux agent spawn Agent3
# Waits 30s
# Returns error: "IPC timeout after 30s"
```

---

## Error Handling

| Error | Handling |
|-------|----------|
| Lock file not found | Start new instance |
| Lock file corrupted | Remove, start new instance |
| Process not running (stale) | Remove lock, start new instance |
| IPC connection refused | Remove lock, start new instance |
| IPC timeout | Return error to user |
| Port already in use | Find new port, update lock |
| Permission denied on lock file | Use temp directory fallback |

---

## References

- Tauri Single Instance: https://tauri.app/v1/guides/features/single-instance/
- tiny_http: https://docs.rs/tiny-http/latest/tiny_http/
- Process detection: https://stackoverflow.com/questions/7854483/how-to-check-if-a-process-id-pid-exists
