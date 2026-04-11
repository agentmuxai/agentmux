# AgentMux Logging Stack — Analysis & Consolidation Spec

**Date:** 2026-04-07 | **Version:** 0.33.62
**Problem:** Logs are scattered across two unrelated directories, with version-stamped filenames and date suffixes. Every agent session burns 3-5 tool calls on a scavenger hunt. The spec must work identically across `task dev`, portable, and install modes.

---

## Part 1: Full Stack Analysis

### Architecture

```
┌──────────────────────────────────────────────────────────────┐
│  LAUNCHER (agentmux-launcher/src/main.rs)                    │
│  • No logging today — eprintln! only (proposed: file log)    │
│  • Windows: .status() — STAYS ALIVE for entire app lifetime  │
│  • Unix: exec() — replaces self with CEF host process        │
│  • Spawns CEF host, forwards exit code                       │
└────────────────────────┬─────────────────────────────────────┘
                         │ spawn (.status) / exec
┌────────────────────────▼─────────────────────────────────────┐
│  CEF HOST (agentmux-cef/src/main.rs)                         │
│                                                              │
│  init_logging() @ line 295                                   │
│  ├─ Reads AGENTMUX_DATA_HOME (NOT SET YET at this point)    │
│  ├─ Falls back to ~/.agentmux/logs/                          │
│  ├─ File: agentmux-host-v{VER}.log.{DATE}                   │
│  ├─ Format: JSON (file) + human-readable (stderr)            │
│  └─ Filter: RUST_LOG or "info"                               │
│                                                              │
│  memory_heartbeat::start() @ line 257                        │
│  ├─ Thread "mem-heartbeat", 20s interval                     │
│  ├─ target: "mem_heartbeat"                                  │
│  └─ → CEF host log file                                     │
│                                                              │
│  fe_log_structured (commands/backend.rs:70)                  │
│  ├─ IPC: "fe_log_structured" via HTTP POST /ipc              │
│  ├─ Receives console.log/warn/error/debug/info from frontend │
│  ├─ Prefix: "[fe]"                                           │
│  └─ → CEF host log file                                     │
│                                                              │
│  sidecar::spawn_backend() @ sidecar.rs:23                    │
│  ├─ Sets AGENTMUX_DATA_HOME = {AppData/Roaming}/ai.agentmux │
│  │   .cef.{dev|v{VER}}/                                     │
│  └─ Spawns agentmux-srv with this env                        │
└────────────────────────┬─────────────────────────────────────┘
                         │ spawns
┌────────────────────────▼─────────────────────────────────────┐
│  SIDECAR (agentmux-srv/src/main.rs)                          │
│                                                              │
│  init_logging() @ line 530                                   │
│  ├─ Reads AGENTMUX_DATA_HOME (SET by CEF host)              │
│  ├─ → {AppData/Roaming}/ai.agentmux.cef.{id}/logs/          │
│  ├─ File: agentmuxsrv-v{VER}.log.{DATE}                     │
│  ├─ Format: JSON (file) + human-readable (stderr)            │
│  └─ Filter: RUST_LOG or "agentmuxsrv=info,info"             │
│                                                              │
│  Shell spawn (blockcontroller/shell.rs:480)                  │
│  ├─ Injects: AGENTMUX_VERSION, BLOCKID, TABID, LOCAL_URL    │
│  └─ Does NOT inject any log-related env vars                 │
└────────────────────────┬─────────────────────────────────────┘
                         │ spawns PTY
┌────────────────────────▼─────────────────────────────────────┐
│  SHELL (bash/zsh/pwsh/fish)                                  │
│  • Has AGENTMUX_VERSION but no log paths                     │
│  • Shell integration scripts: OSC sequences only, no logging │
│  • Agent CLIs: own stdout/stderr, not captured               │
└──────────────────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────────────────┐
│  WSH (agentmux-wsh/src/main.rs)                              │
│  • No logging infrastructure (14-line entry point)           │
└──────────────────────────────────────────────────────────────┘

┌──────────────────────────────────────────────────────────────┐
│  FRONTEND (browser context)                                  │
│  • log-pipe.ts:17 — initLogPipe() monkey-patches console.*   │
│  • Fire-and-forget IPC → fe_log_structured → CEF host log    │
│  • Payload: { level, module: "console", message, data }      │
└──────────────────────────────────────────────────────────────┘
```

### The Split: Where Logs Actually Land

**Critical finding:** CEF host and sidecar logs are in completely different directories.

| Component | Log Directory | Why |
|-----------|--------------|-----|
| CEF host | `~/.agentmux/logs/` | `init_logging()` runs BEFORE `AGENTMUX_DATA_HOME` is set; falls back to `~/.agentmux` |
| Sidecar | `{AppData/Roaming}/ai.agentmux.cef.{id}/logs/` | Receives `AGENTMUX_DATA_HOME` from CEF host, points to versioned AppData dir |
| Frontend | Same as CEF host | Piped via IPC |
| Heartbeat | Same as CEF host | Same process |

### Actual File Locations (verified on disk)

```
# CEF host log (+ frontend [fe] + heartbeat)
~/.agentmux/logs/agentmux-host-v0.33.62.log.2026-04-07

# Sidecar log (shell, blocks, RPC, config)
C:\Users\area54\AppData\Roaming\ai.agentmux.cef.v0-33-62\logs\agentmuxsrv-v0.33.62.log.2026-04-07
```

### Per-Mode Behavior

| Mode | AGENTMUX_DEV | AGENTMUX_DATA_HOME (sidecar) | CEF host log dir | Sidecar log dir |
|------|-------------|------------------------------|------------------|-----------------|
| `task dev` | `1` (Taskfile.yml:515) | `{Roaming}/ai.agentmux.cef.dev/` | `~/.agentmux/logs/` | `{Roaming}/ai.agentmux.cef.dev/logs/` |
| Portable | unset | `{Roaming}/ai.agentmux.cef.v{VER}/` | `~/.agentmux/logs/` | `{Roaming}/ai.agentmux.cef.v{VER}/logs/` |
| Install | unset | `{Roaming}/ai.agentmux.cef.v{VER}/` | `~/.agentmux/logs/` | `{Roaming}/ai.agentmux.cef.v{VER}/logs/` |

**Key observation:** The CEF host log location is **always** `~/.agentmux/logs/` regardless of mode, because `init_logging()` at main.rs:299 runs before `AGENTMUX_DATA_HOME` is set. Only the sidecar's location varies.

### Other Issues Found

1. **No log cleanup** — logs accumulate forever. 46+ host log files in `~/.agentmux/logs/`, 100+ versioned AppData dirs
2. **No log rotation** beyond daily rolling — a long session on one date produces a single growing file
3. **Version mismatch possible** — sidecar.rs:30 uses the *CEF host's* `CARGO_PKG_VERSION` for the dir name, but the sidecar binary could be a different version (especially in dev mode where resolve_backend_binary has 4 fallback paths)

---

## Part 2: Consolidation Proposal

### Goal

An agent running in any mode should resolve any log file with one deterministic command — no globs, no version guessing, no directory hunting.

### Option A: Consolidate All Logs to One Directory (recommended)

**Move sidecar logs to `~/.agentmux/logs/`** so all logs land in one place.

The sidecar already has `get_wave_data_dir()` in `base.rs:71` which resolves `AGENTMUX_DATA_HOME` → `~/.agentmux`. The issue is that `AGENTMUX_DATA_HOME` is set by the CEF host to the *versioned* AppData dir. The fix:

**Change in `agentmux-srv/src/main.rs` init_logging():**
```rust
// Always log to ~/.agentmux/logs/ regardless of AGENTMUX_DATA_HOME,
// so all logs land in one discoverable directory.
let log_dir = dirs::home_dir()
    .unwrap_or_default()
    .join(".agentmux")
    .join("logs");
```

This makes the log dir consistent across all modes and both components. `AGENTMUX_DATA_HOME` continues to control the sidecar's *data* directory (db, config, etc.) — just not logs.

**Trade-off:** Sidecar logs are no longer isolated per-version in AppData. But they already have version in the filename (`agentmuxsrv-v0.33.62.log.{DATE}`), so multiple versions can coexist in the same directory.

### Option B: Keep Split, Add Discovery Layer

If consolidation is undesirable, add discovery env vars and pointer files to bridge the split.

### Chosen: Option A + Discovery Layer

Best of both worlds — consolidate for simplicity, add env vars for deterministic access.

---

## Part 3: Implementation

### Change 1: Consolidate sidecar logs (P0)

**File:** `agentmux-srv/src/main.rs` — `init_logging()` (~line 533)

**Before:**
```rust
let log_dir = std::env::var("AGENTMUX_DATA_HOME")
    .map(std::path::PathBuf::from)
    .unwrap_or_else(|_| dirs::home_dir().unwrap_or_default().join(".agentmux"))
    .join("logs");
```

**After:**
```rust
// Always log to ~/.agentmux/logs/ so all logs (host + sidecar) land in one
// discoverable directory. AGENTMUX_DATA_HOME controls the data dir, not logs.
let log_dir = dirs::home_dir()
    .unwrap_or_default()
    .join(".agentmux")
    .join("logs");
```

**Risk:** Low — only changes log output location. Data, config, db all unaffected.
**Verification:** After rebuild, check that `~/.agentmux/logs/agentmuxsrv-v*.log.*` exists.

### Change 2: `AGENTMUX_LOG_DIR` env var (P0)

**File:** `agentmux-srv/src/backend/blockcontroller/shell.rs` (~line 486)

```rust
// Inject log directory so agents can find logs without guessing.
// Always ~/.agentmux/logs/ — matches both host and sidecar after consolidation.
let log_dir = dirs::home_dir()
    .unwrap_or_default()
    .join(".agentmux")
    .join("logs");
c.env("AGENTMUX_LOG_DIR", log_dir.to_string_lossy().as_ref());
```

**Available in shell as:** `$AGENTMUX_LOG_DIR`

### Change 3: Current-log pointer files (P0)

Write a one-line text file containing the current log filename. Pointer files go in the same log dir.

**File:** `agentmux-cef/src/main.rs` — after file appender creation (~line 306)

```rust
// Write pointer to current log file for agent discovery.
// On Windows, symlinks require elevation, so use a plain text pointer.
let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
let current_filename = format!("{}.{}", log_prefix, today);
let _ = std::fs::write(log_dir.join("current-host.path"), &current_filename);
```

**File:** `agentmux-srv/src/main.rs` — after file appender creation (~line 544)

```rust
let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
let current_filename = format!("{}.{}", log_prefix, today);
let _ = std::fs::write(log_dir.join("current-srv.path"), &current_filename);
```

**Result in `~/.agentmux/logs/`:**
```
current-host.path    → contains: "agentmux-host-v0.33.62.log.2026-04-07"
current-srv.path     → contains: "agentmuxsrv-v0.33.62.log.2026-04-07"
```

### Change 4: Heartbeat refreshes pointers (P1)

Piggyback on the existing 20s heartbeat loop to update the pointer file on date change.

**File:** `agentmux-cef/src/memory_heartbeat.rs` — in the loop body

```rust
// Refresh log pointer in case of midnight rollover.
// Cheap: one write every 20s, only changes on date boundary.
let today = chrono::Utc::now().format("%Y-%m-%d").to_string();
let current_filename = format!("agentmux-host-v{}.log.{}", env!("CARGO_PKG_VERSION"), today);
let log_dir = dirs::home_dir()
    .unwrap_or_default()
    .join(".agentmux")
    .join("logs");
let _ = std::fs::write(log_dir.join("current-host.path"), &current_filename);
```

### Change 5: Launcher logging (P1)

The launcher has zero logging today — only `eprintln!` for fatal errors. On Windows it stays alive for the **entire app lifetime** (`.status()` blocks until CEF host exits), so it's an invisible process that could mask startup failures, DLL resolution issues, or child process crashes.

The launcher is intentionally minimal (no `tracing`, no `chrono`, no `dirs` crate — just `std` + `windows-sys`). Adding full tracing would bloat a 325 KB binary. Instead, use lightweight `std::fs` append-logging to the same consolidated log directory.

**File:** `agentmux-launcher/src/main.rs`

```rust
use std::io::Write;

/// Append a timestamped line to ~/.agentmux/logs/agentmux-launcher.log
/// Best-effort — silently no-ops if the log dir doesn't exist yet.
fn log(msg: &str) {
    let log_dir = dirs_fallback_home().join(".agentmux").join("logs");
    let _ = std::fs::create_dir_all(&log_dir);
    let path = log_dir.join("agentmux-launcher.log");
    if let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    {
        // No chrono dep — use SystemTime for a rough timestamp
        let secs = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let _ = writeln!(f, "[{}] v{} {}", secs, env!("CARGO_PKG_VERSION"), msg);
    }
}

/// Home dir without the `dirs` crate (keep launcher zero-dep beyond windows-sys)
fn dirs_fallback_home() -> std::path::PathBuf {
    std::env::var("USERPROFILE")                          // Windows
        .or_else(|_| std::env::var("HOME"))               // Unix
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("."))
}
```

**Log points to add in `main()`:**

```rust
fn main() {
    let exe_path = std::env::current_exe().expect("cannot resolve exe path");
    let exe_dir = exe_path.parent().expect("exe has no parent directory");
    let runtime_dir = exe_dir.join("runtime");

    log(&format!("starting — exe={} runtime={}", exe_path.display(), runtime_dir.display()));

    // ... DLL search path setup ...
    log("SetDllDirectoryW done");

    let real_exe = find_cef_binary(&runtime_dir);
    log(&format!("resolved CEF binary: {}", real_exe.display()));

    if !real_exe.exists() {
        log(&format!("FATAL: CEF binary not found at {}", real_exe.display()));
        eprintln!("AgentMux runtime not found...");
        std::process::exit(1);
    }

    let args: Vec<String> = std::env::args().skip(1).collect();
    log(&format!("spawning CEF host with {} args", args.len()));

    // Windows: .status() blocks — launcher stays alive
    let status = std::process::Command::new(&real_exe)
        .args(&args)
        .status();

    match status {
        Ok(s) => {
            let code = s.code().unwrap_or(1);
            log(&format!("CEF host exited with code {}", code));
            std::process::exit(code);
        }
        Err(e) => {
            log(&format!("FATAL: failed to spawn CEF host: {}", e));
            eprintln!("Failed to launch AgentMux: {}", e);
            std::process::exit(1);
        }
    }
}
```

**Log file:** `~/.agentmux/logs/agentmux-launcher.log` (append-only, no rotation — file stays tiny since launcher only logs ~5 lines per session)

**Format:** `[unix_epoch_secs] v0.33.62 message` — no deps needed, human-readable enough for diagnostics.

**Why not tracing?** The launcher's job is to add zero overhead. Adding `tracing` + `tracing-subscriber` + `tracing-appender` + `chrono` would increase compile time and binary size for a component that logs 5 lines per lifetime. `std::fs::OpenOptions::append` is sufficient.

**Cross-mode behavior:**
- `task dev`: No launcher involved (runs `agentmux-cef` directly from `dist/cef-dev/`)
- Portable: Launcher runs, logs to `~/.agentmux/logs/agentmux-launcher.log`
- Install: Same as portable

**Pointer file:** `current-launcher.path` is unnecessary — there's only one log file (no version/date suffix). Agents can access it directly: `cat "$AGENTMUX_LOG_DIR/agentmux-launcher.log"`.

### Change 6: 7-day log retention (P2)

Add a cleanup pass on startup in both `init_logging()` functions.

```rust
// Delete log files older than 7 days to prevent unbounded growth.
if let Ok(entries) = std::fs::read_dir(&log_dir) {
    let cutoff = std::time::SystemTime::now()
        - std::time::Duration::from_secs(7 * 86400);
    for entry in entries.flatten() {
        let path = entry.path();
        // Only touch log files, not pointer files
        if !path.to_string_lossy().contains(".log.") {
            continue;
        }
        if let Ok(meta) = entry.metadata() {
            if let Ok(modified) = meta.modified() {
                if modified < cutoff {
                    let _ = std::fs::remove_file(&path);
                }
            }
        }
    }
}
```

### Change 7: `muxlog` shell helper (P2)

**Files:** `agentmux-srv/src/backend/shellintegration/{bash.sh,zsh.sh}`

```bash
muxlog() {
  local target="${1:-host}"
  local ptr="$AGENTMUX_LOG_DIR/current-${target}.path"
  if [ ! -f "$ptr" ]; then
    echo "Unknown log target '$target'. Available:" >&2
    ls "$AGENTMUX_LOG_DIR"/current-*.path 2>/dev/null \
      | sed 's|.*/current-||;s|\.path||' >&2
    return 1
  fi
  local logfile="$AGENTMUX_LOG_DIR/$(cat "$ptr")"
  shift 2>/dev/null
  case "${1:-tail}" in
    tail) tail -f "$logfile" ;;
    cat)  cat "$logfile" ;;
    *)    grep "$1" "$logfile" ;;
  esac
}
```

**File:** `agentmux-srv/src/backend/shellintegration/pwsh.ps1`

```powershell
function muxlog {
  param([string]$Target = "host", [string]$Action = "tail")
  $ptr = "$env:AGENTMUX_LOG_DIR\current-$Target.path"
  if (-not (Test-Path $ptr)) {
    Write-Error "Unknown log target '$Target'"
    return
  }
  $logfile = "$env:AGENTMUX_LOG_DIR\$(Get-Content $ptr)"
  switch ($Action) {
    "tail" { Get-Content $logfile -Wait -Tail 50 }
    "cat"  { Get-Content $logfile }
    default { Select-String $Action $logfile }
  }
}
```

---

## Part 4: Cross-Mode Compatibility Matrix

After all changes, the behavior should be identical across modes:

| | `task dev` | Portable | Install |
|-|-----------|----------|---------|
| `$AGENTMUX_LOG_DIR` | `~/.agentmux/logs` | `~/.agentmux/logs` | `~/.agentmux/logs` |
| Host log | `agentmux-host-v{V}.log.{D}` | same | same |
| Sidecar log | `agentmuxsrv-v{V}.log.{D}` | same | same |
| Launcher log | N/A (no launcher) | `agentmux-launcher.log` | `agentmux-launcher.log` |
| `current-host.path` | points to host log | same | same |
| `current-srv.path` | points to srv log | same | same |
| `muxlog host` | tails host log | same | same |
| `muxlog srv` | tails srv log | same | same |
| Launcher log | N/A | `cat "$AGENTMUX_LOG_DIR/agentmux-launcher.log"` | same |

**Multi-instance safety:** Multiple versions running simultaneously write to the same `~/.agentmux/logs/` directory but with different version-stamped filenames. Pointer files reflect whichever instance updated them last — acceptable since the agent is running inside a specific instance and `$AGENTMUX_VERSION` can disambiguate if needed.

**Dev-mode edge case:** `AGENTMUX_DEV=1` no longer affects log location (it still affects CEF data dir for caches, cookies, etc.). This is intentional — dev logs belong with all other logs for easy discovery.

---

## Part 5: After Implementation — CLAUDE.md Update

```markdown
### Log Access (zero lookup)

All logs land in `$AGENTMUX_LOG_DIR` (`~/.agentmux/logs/`). Pointer files resolve the current filename.

| What | Command |
|------|---------|
| Current host log path | `cat "$AGENTMUX_LOG_DIR/current-host.path"` |
| Current sidecar log path | `cat "$AGENTMUX_LOG_DIR/current-srv.path"` |
| Tail host log | `muxlog host` or `tail -f "$AGENTMUX_LOG_DIR/$(cat "$AGENTMUX_LOG_DIR/current-host.path")"` |
| Tail sidecar log | `muxlog srv` |
| Frontend logs | `muxlog host '[fe]'` |
| Memory heartbeat | `muxlog host mem_heartbeat` |
| Dump full host log | `muxlog host cat` |
| Launcher log (portable/install only) | `cat "$AGENTMUX_LOG_DIR/agentmux-launcher.log"` |

Works identically across `task dev`, portable, and install builds.
Never glob for log files. Never guess paths. Use the pointer files.
```

---

## Summary of Changes

| # | What | Files | Lines | Priority |
|---|------|-------|-------|----------|
| 1 | Consolidate sidecar logs to `~/.agentmux/logs/` | `agentmux-srv/src/main.rs:533` | 3 | P0 |
| 2 | Inject `AGENTMUX_LOG_DIR` into shells | `shell.rs:486` | 5 | P0 |
| 3 | Write `current-host.path` pointer | `agentmux-cef/src/main.rs:306` | 3 | P0 |
| 4 | Write `current-srv.path` pointer | `agentmux-srv/src/main.rs:544` | 3 | P0 |
| 5 | Launcher file logging | `agentmux-launcher/src/main.rs` | ~35 | P1 |
| 6 | Heartbeat refreshes host pointer | `memory_heartbeat.rs:16` | 5 | P1 |
| 7 | 7-day log retention on startup | Both `main.rs` init_logging | 15 | P2 |
| 8 | `muxlog` shell helper | `shellintegration/{bash,zsh,pwsh,fish}` | ~20/script | P2 |
| 9 | CLAUDE.md documentation | `CLAUDE.md` | 15 | P2 |

**Total P0 effort:** ~14 lines of Rust across 3 files.
**Total P1 effort:** ~40 lines (launcher logging + heartbeat pointer refresh).
