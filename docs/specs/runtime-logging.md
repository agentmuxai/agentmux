# Spec: Runtime Logging Infrastructure Rewrite

## Status: Draft
## Author: AgentX
## Date: 2026-03-07

---

## Problem

AgentMux has minimal, inconsistent logging that makes debugging production issues nearly impossible. When a user clicks a provider button and gets a blank screen, there is no way to trace what happened — the backend logs only to stderr (ephemeral), the Tauri host writes daily rolling files but the backend sidecar doesn't, and the frontend uses `console.log` which is invisible outside DevTools.

### Current State Audit

| Layer | Logging | Destination | Persistent? | Structured? |
|-------|---------|-------------|-------------|-------------|
| **Tauri host** (`src-tauri/`) | `tracing` with `tracing_appender` | Daily rolling file + stderr | Yes (rolling) | Semi (fmt layer) |
| **Backend sidecar** (`agentmuxsrv-rs/`) | `tracing_subscriber::fmt` | stderr only | No | Semi (fmt layer) |
| **Frontend** (`frontend/`) | `console.log/error/warn` | Browser DevTools only | No | No |
| **Shell controller** | 4 `tracing::debug/warn` calls | stderr (via backend) | No | No |
| **WebSocket RPC** | 13 `tracing::info/warn/debug` calls | stderr (via backend) | No | No |
| **Crash handler** | Panic hook → crash log file | `{log_dir}/crash-{timestamp}.log` | Yes | No |

**Total tracing statements across entire backend: ~59** — far too few for a multi-process desktop app.

### Key Gaps

1. **Backend sidecar has no file logging** — all output goes to stderr, which is only captured if the Tauri host pipes it (and currently it doesn't persist stderr from the sidecar).
2. **No structured logging** — no JSON output, no correlation IDs, no request tracing.
3. **No log levels per module** — `info` globally, no way to enable debug for `shell` without enabling debug for everything.
4. **Frontend logging is fire-and-forget** — `console.log("[agent]")` disappears when DevTools closes.
5. **No block/session correlation** — when debugging "blank screen on click", there's no way to trace a single block's lifecycle from button click → SetMeta → ControllerResync → PTY spawn → first output.
6. **No log rotation on backend** — Tauri host has daily rolling, backend has nothing.
7. **No startup diagnostics** — no single place that logs system state, config, versions, paths on launch.

---

## Goals

1. **Every user-visible action should produce a traceable log chain** from frontend → RPC → backend → PTY/process.
2. **Persistent file logging** on all layers (Tauri host, backend sidecar, frontend).
3. **Structured JSON logs** for machine parsing (log aggregation, crash reports).
4. **Per-module log levels** configurable at runtime without restart.
5. **Block lifecycle tracing** — every block operation tagged with `block_id` for filtering.
6. **Startup diagnostics** — log system state, config, versions, paths on every launch.
7. **Log rotation** — bounded disk usage with configurable retention.
8. **Zero-cost when off** — debug/trace level logging should have no measurable impact when disabled.

---

## Design

### Log Directory Structure

```
~/.agentmux/logs/
  agentmux-host.log          # Tauri host (rolling daily)
  agentmux-host.log.2026-03-06
  agentmuxsrv.log            # Backend sidecar (rolling daily)
  agentmuxsrv.log.2026-03-06
  frontend.log               # Frontend logs bridged via Tauri command
  frontend.log.2026-03-06
  crash-20260307-041523.log   # Crash reports (existing)
```

All logs go under a single directory: `{AGENTMUX_DATA_HOME}/logs/` (typically `~/.agentmux/logs/`). This replaces the current Tauri `app_log_dir()` which uses a platform-specific location that's hard to find.

### Layer 1: Backend Sidecar (`agentmuxsrv-rs`)

This is the most critical layer — it handles PTY spawning, RPC routing, WebSocket connections, and block state management.

#### Changes to `main.rs`

Replace the current stderr-only tracing with a dual-output subscriber:

```rust
fn init_logging(data_dir: &Path) -> tracing_appender::non_blocking::WorkerGuard {
    use tracing_subscriber::{fmt, layer::SubscriberExt, EnvFilter};

    let log_dir = data_dir.join("logs");
    let _ = std::fs::create_dir_all(&log_dir);

    // Rolling daily log file
    let file_appender = tracing_appender::rolling::daily(&log_dir, "agentmuxsrv.log");
    let (non_blocking_file, guard) = tracing_appender::non_blocking(file_appender);

    // JSON structured logging to file, human-readable to stderr
    let subscriber = tracing_subscriber::registry()
        .with(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("agentmuxsrv=info,warn")),
        )
        .with(
            fmt::layer()
                .json()
                .with_writer(non_blocking_file)
                .with_target(true)
                .with_thread_ids(true)
                .with_span_events(fmt::format::FmtSpan::CLOSE),
        )
        .with(
            fmt::layer()
                .with_writer(std::io::stderr)
                .with_ansi(true),
        );

    tracing::subscriber::set_global_default(subscriber).ok();
    guard // Caller must hold this to ensure logs are flushed
}
```

#### Structured Spans for Block Lifecycle

Add `tracing::instrument` spans to key functions in `shell.rs`:

```rust
#[tracing::instrument(skip(self), fields(block_id = %self.block_id, controller = "shell"))]
pub async fn run(&self, ...) -> Result<(), String> {
    // Existing code — now all nested tracing calls inherit block_id
}
```

Key events to log in the shell controller:

| Event | Level | Fields |
|-------|-------|--------|
| Block start requested | `info` | `block_id`, `controller`, `cmd`, `cmd_args`, `interactive` |
| PTY opened | `info` | `block_id`, `rows`, `cols` |
| Command resolved | `info` | `block_id`, `cmd_str`, `effective_path` |
| `cmd:env` injected | `debug` | `block_id`, env key-value pairs |
| Spawn success | `info` | `block_id`, `pid`, `shell_type` |
| Spawn failure | `error` | `block_id`, `error`, `cmd_str`, `working_dir` |
| PTY first output | `debug` | `block_id`, `bytes` |
| PTY read error | `warn` | `block_id`, `error` |
| Process exited | `info` | `block_id`, `exit_code`, `runtime_secs` |
| Block stopped | `info` | `block_id`, `reason` |

#### WebSocket/RPC Logging

Add spans to `websocket.rs`:

```rust
#[tracing::instrument(skip_all, fields(command = %cmd_name))]
fn handle_rpc_command(...) { ... }
```

Key events:

| Event | Level | Fields |
|-------|-------|--------|
| WS client connected | `info` | `client_id`, `remote_addr` |
| WS client disconnected | `info` | `client_id`, `reason` |
| RPC command received | `debug` | `command`, `block_id` (if applicable) |
| RPC command completed | `debug` | `command`, `duration_ms` |
| RPC command failed | `warn` | `command`, `error` |
| SetMeta applied | `info` | `oref`, changed keys |
| ControllerResync | `info` | `block_id`, `forcerestart` |
| EventBus broadcast | `trace` | `event_type`, `scopes` |

### Layer 2: Tauri Host (`src-tauri`)

#### Changes to `lib.rs`

Update `init_logging` to:
1. Use `{data_dir}/logs/` instead of `app_log_dir()`
2. Add JSON file layer (matching backend format)
3. Log startup diagnostics

```rust
fn init_logging(handle: &tauri::AppHandle) -> std::path::PathBuf {
    // Use agentmux data dir for consistency with backend
    let log_dir = wavebase::get_wave_data_dir().join("logs");
    let _ = std::fs::create_dir_all(&log_dir);

    let file_appender = tracing_appender::rolling::daily(&log_dir, "agentmux-host.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    std::mem::forget(_guard);

    let subscriber = tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env()
            .unwrap_or_else(|_| EnvFilter::new("agentmux=info,warn")))
        .with(fmt::layer().json().with_writer(non_blocking))
        .with(fmt::layer().with_writer(std::io::stderr));

    tracing::subscriber::set_global_default(subscriber).ok();

    // Startup diagnostics
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        os = std::env::consts::OS,
        arch = std::env::consts::ARCH,
        data_dir = %log_dir.parent().unwrap_or(&log_dir).display(),
        "AgentMux host starting"
    );

    log_dir
}
```

### Layer 3: Frontend → File Bridge

Frontend `console.log` calls are ephemeral. Add a bridge that sends important frontend logs to the Tauri host for persistence.

#### New Tauri Command: `fe_log_structured`

```rust
#[tauri::command]
pub fn fe_log_structured(level: String, module: String, message: String, data: Option<serde_json::Value>) {
    match level.as_str() {
        "error" => tracing::error!(module = %module, data = ?data, "[fe] {}", message),
        "warn"  => tracing::warn!(module = %module, data = ?data, "[fe] {}", message),
        "info"  => tracing::info!(module = %module, data = ?data, "[fe] {}", message),
        "debug" => tracing::debug!(module = %module, data = ?data, "[fe] {}", message),
        _       => tracing::trace!(module = %module, data = ?data, "[fe] {}", message),
    }
}
```

#### Frontend Logger Utility

```typescript
// frontend/app/util/logger.ts

const log = (level: string, module: string, message: string, data?: Record<string, any>) => {
    // Always log to console for DevTools
    const consoleFn = level === "error" ? console.error
        : level === "warn" ? console.warn
        : console.log;
    consoleFn(`[${module}] ${message}`, data || "");

    // Bridge to Tauri host for file persistence
    try {
        getApi().feLogStructured(level, module, message, data ?? null);
    } catch {
        // Silently ignore if Tauri bridge not ready
    }
};

export const Logger = {
    error: (module: string, msg: string, data?: Record<string, any>) => log("error", module, msg, data),
    warn:  (module: string, msg: string, data?: Record<string, any>) => log("warn", module, msg, data),
    info:  (module: string, msg: string, data?: Record<string, any>) => log("info", module, msg, data),
    debug: (module: string, msg: string, data?: Record<string, any>) => log("debug", module, msg, data),
};
```

#### Usage in Agent Model

Replace scattered `console.log("[agent]")` calls:

```typescript
// Before
console.log(`[agent] Detecting ${provider.id} CLI...`);
console.error("[agent] CLI detection failed:", e);

// After
Logger.info("agent", `Detecting ${provider.id} CLI`);
Logger.error("agent", "CLI detection failed", { provider: provider.id, error: String(e) });
```

### Layer 4: Startup Diagnostics

On every launch, log a structured diagnostic block:

```
[agentmuxsrv startup]
  version: 0.31.67
  platform: windows / x86_64
  data_dir: C:\Users\asafe\.agentmux
  log_dir: C:\Users\asafe\.agentmux\logs
  db_path: C:\Users\asafe\.agentmux\db\wave.db
  config_path: C:\Users\asafe\AppData\Roaming\AgentMux\settings.json
  shell: C:\Program Files\PowerShell\7\pwsh.exe
  wsh_binary: C:\Users\asafe\.agentmux\bin\wsh.exe
  env.AGENTMUX_DATA_HOME: (not set)
  env.AGENTMUX_CONFIG_HOME: (not set)
  env.RUST_LOG: (not set)
  first_launch: false
  ws_endpoint: 127.0.0.1:1739
  web_endpoint: 127.0.0.1:1740
```

### Log Rotation & Retention

| Setting | Default | Configurable? |
|---------|---------|---------------|
| Rotation | Daily | No (fixed) |
| Max file size | Unlimited (daily roll handles it) | Future |
| Retention | 7 days | Yes (`settings.json`) |
| Compression | None | Future |

Add a cleanup task on startup that deletes log files older than the retention period:

```rust
fn cleanup_old_logs(log_dir: &Path, retention_days: u64) {
    let cutoff = std::time::SystemTime::now()
        - std::time::Duration::from_secs(retention_days * 86400);

    if let Ok(entries) = std::fs::read_dir(log_dir) {
        for entry in entries.flatten() {
            if let Ok(metadata) = entry.metadata() {
                if let Ok(modified) = metadata.modified() {
                    if modified < cutoff {
                        let _ = std::fs::remove_file(entry.path());
                        tracing::info!("Removed old log: {}", entry.path().display());
                    }
                }
            }
        }
    }
}
```

### Runtime Log Level Control

Allow changing log levels without restart via settings.json:

```json
{
  "logging": {
    "level": "info",
    "modules": {
      "agentmuxsrv::backend::blockcontroller": "debug",
      "agentmuxsrv::server::websocket": "debug"
    },
    "retention_days": 7
  }
}
```

The config watcher (already exists) reloads this and updates the `EnvFilter` via `tracing_subscriber::reload::Layer`.

---

## Implementation Plan

### Phase 1: Backend File Logging (Critical)

1. Add `tracing-appender` dependency to `agentmuxsrv-rs/Cargo.toml`
2. Rewrite `main.rs` logging init with dual output (JSON file + stderr)
3. Add startup diagnostics block
4. Add log retention cleanup on startup
5. Add 15+ `tracing::info/debug` calls to `shell.rs` covering the block lifecycle
6. Add 10+ `tracing::info/debug` calls to `websocket.rs` covering RPC commands

### Phase 2: Frontend Log Bridge

7. Add `fe_log_structured` Tauri command
8. Create `frontend/app/util/logger.ts` utility
9. Replace all `console.log("[agent]")` calls in agent view with `Logger.*`
10. Replace key `console.log/error` calls across the frontend

### Phase 3: Tauri Host Alignment

11. Move Tauri host logs to `{data_dir}/logs/` (same dir as backend)
12. Switch to JSON file layer
13. Add startup diagnostics

### Phase 4: Runtime Control (Future)

14. Add `logging` section to settings.json schema
15. Integrate `tracing_subscriber::reload` for dynamic level changes
16. Add log viewer UI in app (read log files, filter by module/level)

---

## Log Format

### JSON (file output)

```json
{"timestamp":"2026-03-07T04:15:23.456Z","level":"INFO","target":"agentmuxsrv::backend::blockcontroller::shell","span":{"block_id":"blk-abc123","controller":"shell"},"fields":{"message":"Spawning command","cmd":"codex","effective_path":"C:\\Users\\asafe\\.agentmux\\cli\\codex\\node_modules\\.bin","interactive":true}}
```

### Human-readable (stderr)

```
2026-03-07T04:15:23.456Z  INFO shell{block_id=blk-abc123}: Spawning command cmd="codex" interactive=true
```

---

## Debugging the Blank Screen Issue (Example)

With this logging in place, the blank screen scenario would produce:

```
INFO  [fe] agent: Provider button clicked {provider: "codex"}
INFO  [fe] agent: Detecting codex CLI
INFO  [fe] agent: CLI found at C:\Users\asafe\.agentmux\cli\codex\node_modules\.bin\codex.cmd
INFO  [fe] agent: Launching codex {cmd: "codex", binDir: "...\.bin"}
INFO  [fe] rpc: SetMetaCommand {oref: "block:blk-abc", keys: ["view","controller","cmd","cmd:args","cmd:interactive","cmd:runonstart","cmd:env"]}
INFO  [ws] SetMeta applied {oref: "block:blk-abc", keys: 7}
INFO  [ws] ControllerResync {block_id: "blk-abc", forcerestart: true}
INFO  [shell] Block start {block_id: "blk-abc", cmd: "codex", args: [], interactive: true}
INFO  [shell] cmd:env injected {block_id: "blk-abc", PATH_prepend: "C:\Users\asafe\.agentmux\cli\codex\node_modules\.bin"}
INFO  [shell] Resolved command: CommandBuilder::new("codex") {block_id: "blk-abc"}
ERROR [shell] Spawn failed {block_id: "blk-abc", error: "The system cannot find the file specified. (os error 2)", cmd: "codex"}
```

This immediately reveals that "codex" (bare name) isn't resolving even though PATH was prepended — because `CommandBuilder::new()` on Windows doesn't search PATH, it expects an absolute path or a filename on the system PATH at process start time. The prepended PATH only takes effect for child processes, not for `CreateProcessW` lookup.

---

## Files Modified

| File | Change |
|------|--------|
| `agentmuxsrv-rs/Cargo.toml` | Add `tracing-appender` dependency |
| `agentmuxsrv-rs/src/main.rs` | Rewrite logging init, add startup diagnostics |
| `agentmuxsrv-rs/src/backend/blockcontroller/shell.rs` | Add 15+ tracing calls with block_id spans |
| `agentmuxsrv-rs/src/server/websocket.rs` | Add 10+ tracing calls for RPC lifecycle |
| `src-tauri/src/lib.rs` | Move log dir, add JSON layer, startup diagnostics |
| `src-tauri/src/commands/window.rs` | Add `fe_log_structured` command |
| `frontend/app/util/logger.ts` | New: frontend logging utility |
| `frontend/app/view/agent/agent-model.ts` | Replace console.log with Logger |
| `frontend/app/view/agent/init-monitor.ts` | Replace console.log with Logger |

---

## Out of Scope

- Remote log shipping (telemetry) — future consideration
- Log viewer UI — Phase 4
- Metrics/tracing integration (OpenTelemetry) — future
- Log compression — future
