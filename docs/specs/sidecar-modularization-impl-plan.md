# Sidecar Modularization — Implementation Plan (Phase 1)

**Date:** 2026-03-24
**Branch:** `agenta/sidecar-modularize` (or fold into Phase 2 branch)
**Prereq for:** Phase 2 restart (`restart_backend` needs `binary::build_sidecar_command` to avoid duplicating spawn logic)

---

## What moves, what stays

```
src-tauri/src/
  sidecar.rs              ← keep: ensure_versioned_sidecar, BackendSpawnResult,
                                  cleanup_stale_backends, cleanup_stale_endpoints,
                                  handle_backend_event, spawn_backend (orchestrator)
  sidecar/
    binary.rs             ← NEW: resolve_binary_path, deploy_wsh, build_sidecar_command
    event_loop.rs         ← NEW: run_event_loop (the tokio::spawn block)
```

Everything that's already clean stays in `sidecar.rs`. Only the two pieces that are
(a) shared by the upcoming `respawn_backend` call in Phase 2 or
(b) blocking readability of `spawn_backend`
get extracted.

---

## Module 1 — `sidecar/binary.rs`

**Extracts:** lines 198–305 of `spawn_backend` (auth key read, binary path probe, wsh deploy, env var build, process spawn)

### Public surface

```rust
/// The command receiver and child handle produced by a successful spawn.
pub struct SpawnedSidecar {
    pub rx: tauri_plugin_shell::process::CommandReceiver,
    pub child: tauri_plugin_shell::process::CommandChild,
}

/// Build and spawn the agentmuxsrv-rs process.
///
/// Resolves the binary path (portable → dev → versioned installer copy),
/// deploys the bundled wsh binary, sets all env vars, and calls .spawn().
///
/// Called by both `spawn_backend` (initial launch) and `respawn_backend`
/// (user-initiated restart). Does NOT store state in AppState — caller does that.
pub fn build_sidecar_command(
    app: &tauri::AppHandle,
    auth_key: &str,
    version_data_home: &std::path::Path,
    version_config_home: &std::path::Path,
    config_dir: &std::path::Path,
    version_instance_id: &str,
) -> Result<SpawnedSidecar, String>
```

### What it does internally (same as today, just moved):

1. Resolve binary path: `current_exe() → bin/{name}.x64.exe` (portable) → `target/debug/{name}.exe` (dev) → `ensure_versioned_sidecar()` (installer)
2. Call `deploy_wsh(app_path)`
3. Build the `.args([...]).env(...).spawn()` chain
4. Return `SpawnedSidecar { rx, child }`

### Private helper in same file

```rust
fn deploy_wsh(app_path: &std::path::Path)
```

Moves the wsh deploy block (lines 248–280) verbatim. Keeps `build_sidecar_command` linear.

### What `spawn_backend` looks like after extraction (lines 198–305 collapse to ~10 lines)

```rust
let auth_key = app.state::<crate::state::AppState>().auth_key.lock().unwrap().clone();
tracing::info!("Spawning agentmuxsrv-rs with auth key: {}...", &auth_key[..8]);

let spawned = binary::build_sidecar_command(
    app,
    &auth_key,
    &version_data_home,
    &version_config_home,
    &config_dir,
    &version_instance_id,
)?;
let (mut rx, child) = (spawned.rx, spawned.child);
```

---

## Module 2 — `sidecar/event_loop.rs`

**Extracts:** lines 337–414 of `spawn_backend` (the entire `tokio::spawn` block)

### Why this is the critical extraction

- The `Terminated` arm is where Phase 2 restart hooks in — a restart signal goes out here instead of just `break`
- Once extracted, both `spawn_backend` and `respawn_backend` call `event_loop::run(...)` identically
- The `estart_received` logging flag (from Phase 2 plan) lives naturally here rather than cluttering `spawn_backend`

### Public surface

```rust
/// Drives the CommandEvent stream from a spawned backend process.
///
/// - Relays stderr lines to the host log as `[agentmuxsrv-rs] {line}`
/// - Parses `WAVESRV-ESTART` and sends endpoints on `endpoint_tx` (consumed once)
/// - Parses `WAVESRV-EVENT:` and forwards to frontend via Tauri event
/// - On `Terminated`: logs the crash (startup vs runtime), emits `backend-terminated`
///   to all windows, then returns
///
/// Returns when the backend process terminates.
pub async fn run(
    mut rx: tauri_plugin_shell::process::CommandReceiver,
    app_handle: tauri::AppHandle,
    endpoint_tx: tokio::sync::mpsc::Sender<EStartPayload>,
)
```

```rust
/// Parsed fields from the WAVESRV-ESTART line.
pub struct EStartPayload {
    pub ws: String,
    pub web: String,
    pub version: String,
    pub instance_id: String,
}
```

### Internal structure

```rust
pub async fn run(...) {
    use tauri_plugin_shell::process::CommandEvent;
    let mut estart_received = false;           // ← new: startup vs runtime crash flag

    while let Some(event) = rx.recv().await {
        match event {
            CommandEvent::Stderr(line) => {
                let line = String::from_utf8_lossy(&line);
                for l in line.lines() {
                    if l.starts_with("WAVESRV-ESTART") {
                        let payload = parse_estart(l);
                        tracing::info!("Backend started: ws={} web={} version={} instance={}",
                            payload.ws, payload.web, payload.version, payload.instance_id);
                        estart_received = true;
                        let _ = endpoint_tx.send(payload).await;
                    } else if let Some(event_data) = l.strip_prefix("WAVESRV-EVENT:") {
                        super::handle_backend_event(&app_handle, event_data);
                    } else {
                        tracing::info!("[agentmuxsrv-rs] {}", l);
                    }
                }
            }
            CommandEvent::Stdout(line) => {
                tracing::info!("[agentmuxsrv-rs stdout] {}", String::from_utf8_lossy(&line).trim());
            }
            CommandEvent::Error(err) => {
                tracing::error!("[agentmuxsrv-rs error] {}", err);
            }
            CommandEvent::Terminated(status) => {
                emit_terminated(&app_handle, &status, estart_received);
                break;
            }
            _ => {}
        }
    }
}
```

```rust
fn parse_estart(line: &str) -> EStartPayload { ... }   // pure: no side effects

fn emit_terminated(
    app: &tauri::AppHandle,
    status: &tauri_plugin_shell::process::ExitStatus,
    estart_received: bool,
) {
    let state = app.state::<crate::state::AppState>();
    let pid = state.backend_pid.lock().unwrap().unwrap_or(0);
    let started_at = state.backend_started_at.lock().unwrap().clone();
    let uptime_secs = uptime_from_started_at(started_at.as_deref());

    if estart_received {
        tracing::error!(
            "[agentmuxsrv-rs] RUNTIME CRASH — pid={} exit_code={:?} signal={:?} uptime_secs={:?}",
            pid, status.code, status.signal, uptime_secs
        );
    } else {
        tracing::error!(
            "[agentmuxsrv-rs] STARTUP CRASH — terminated before ready (pid={} exit_code={:?} uptime_secs={:?})",
            pid, status.code, uptime_secs
        );
    }

    let payload = serde_json::json!({
        "code": status.code,
        "signal": status.signal,
        "pid": pid,
        "uptime_secs": uptime_secs,
    });
    for window in app.webview_windows().values() {        // ← all windows, not just main
        let _ = window.emit("backend-terminated", &payload);
    }
}
```

Note: `emit_terminated` already broadcasts to all windows here — this is a **Phase 2 fix** that naturally lands in Phase 1. Currently sidecar.rs only emits to the `"main"` window.

### What `spawn_backend` looks like after extraction (lines 337–414 collapse to ~10 lines)

```rust
let (tx, mut endpoint_rx) = tokio::sync::mpsc::channel::<event_loop::EStartPayload>(1);
let app_handle = app.clone();
tokio::spawn(event_loop::run(rx, app_handle, tx));

let estart = tokio::time::timeout(
    std::time::Duration::from_secs(30),
    endpoint_rx.recv(),
)
.await
.map_err(|_| "Timeout waiting for agentmuxsrv-rs to start (30s)".to_string())?
.ok_or_else(|| "agentmuxsrv-rs channel closed before sending endpoints".to_string())?;

Ok(BackendSpawnResult {
    ws_endpoint: estart.ws,
    web_endpoint: estart.web,
    version: estart.version,
    instance_id: estart.instance_id,
    auth_key,
})
```

---

## Resulting `spawn_backend` (full sketch after both extractions)

~90 lines, 7 clearly labelled steps:

```rust
pub async fn spawn_backend(app: &tauri::AppHandle) -> Result<BackendSpawnResult, String> {
    // 1. Resolve directories
    let data_dir = ...;
    let config_dir = ...;
    let version_instance_id = format!("v{}", env!("CARGO_PKG_VERSION"));

    // 2. Cleanup stale processes/files
    #[cfg(unix)] cleanup_stale_backends(env!("CARGO_PKG_VERSION"));
    cleanup_stale_endpoints(&config_dir);

    // 3. Ensure directory tree
    std::fs::create_dir_all(...)?;  // ×4

    // 4. Spawn the process (binary resolution + wsh deploy + env vars)
    let auth_key = app.state::<AppState>().auth_key.lock().unwrap().clone();
    let spawned = binary::build_sidecar_command(app, &auth_key, ...)?;
    let (mut rx, child) = (spawned.rx, spawned.child);

    // 5. Store PID, child handle, start time
    { let state = app.state::<AppState>(); *state.sidecar_child... = Some(child); ... }

    // 6. Windows Job Object
    #[cfg(target_os = "windows")]
    { match create_job_object_for_child(child_pid) { ... } }

    // 7. Run event loop, wait for ESTART (30s timeout)
    let (tx, mut endpoint_rx) = tokio::sync::mpsc::channel::<event_loop::EStartPayload>(1);
    tokio::spawn(event_loop::run(rx, app.clone(), tx));
    let estart = tokio::time::timeout(Duration::from_secs(30), endpoint_rx.recv())
        .await.map_err(|_| "Timeout...")?
        .ok_or_else(|| "Channel closed...")?;

    Ok(BackendSpawnResult { ws_endpoint: estart.ws, ... })
}
```

---

## Mod declaration in `sidecar.rs`

```rust
// At the top of sidecar.rs — add:
mod binary;
mod event_loop;
```

No `pub mod` needed — these are internal implementation modules, not part of the library's public API. `spawn_backend` and `BackendSpawnResult` remain the only public surface of the sidecar module.

---

## File Changelist

| File | Change |
|------|--------|
| `src-tauri/src/sidecar.rs` | Remove lines 198–305 (binary concern) and 337–414 (event loop); add `mod binary; mod event_loop;` at top |
| `src-tauri/src/sidecar/binary.rs` | **NEW** — `SpawnedSidecar`, `build_sidecar_command`, `deploy_wsh` |
| `src-tauri/src/sidecar/event_loop.rs` | **NEW** — `EStartPayload`, `run`, `parse_estart`, `emit_terminated`, `uptime_from_started_at` |

No changes to `lib.rs`, `state.rs`, or any frontend file.

---

## Why Phase 1 before Phase 2

Phase 2's `restart_backend` command needs to call `spawn_backend`-equivalent logic without re-running stale cleanup and dir creation. After Phase 1:

```rust
// restart_backend (Phase 2) — calls binary::build_sidecar_command directly
let spawned = binary::build_sidecar_command(app, &auth_key, ...)?;
let (tx, mut endpoint_rx) = tokio::sync::mpsc::channel(1);
tokio::spawn(event_loop::run(spawned.rx, app.clone(), tx));
let estart = tokio::time::timeout(Duration::from_secs(30), endpoint_rx.recv())...;
```

Without Phase 1, `restart_backend` would have to either:
- Call `spawn_backend` (runs stale cleanup again — wrong for hot restart)
- Duplicate the 100-line binary + env var block inline

Phase 1 is the prerequisite that makes Phase 2 clean.

---

## Sequencing

```
Phase 1: sidecar modularize
  └─ binary.rs extraction  (~1h)
  └─ event_loop.rs extraction  (~1h)
  └─ cargo check + task dev smoke test

Phase 2: restart + frontend
  └─ restart_backend command using binary::build_sidecar_command
  └─ frontend restart button + WS reconnect
  └─ version link suppression
  └─ bump + PR
```

Both phases can be on the same branch since Phase 1 has no user-visible changes. Or split into separate PRs if desired — Phase 1 is pure refactor, zero risk to ship first.
