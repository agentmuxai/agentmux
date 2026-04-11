# Memory Heartbeat Spec

**PR:** #311 — `feat(cef): in-process GPU, renderer limit, memory heartbeat`
**Version:** v0.33.62
**Module:** `agentmux-cef/src/memory_heartbeat.rs`

---

## Purpose

Forensic telemetry for diagnosing OOM and virtual address (VA) exhaustion crashes. Logs system-wide and per-process memory stats every 20 seconds to the `mem_heartbeat` tracing target. Designed to provide a timeline leading up to crashes that would otherwise leave no diagnostic trail.

## Background

Each CEF subprocess reserves ~100GB virtual address space (32GB GigaCage + 4x16GB PartitionAlloc pools + 4GB V8 pointer cage). On a 16GB machine with 87GB total VA, 3 subprocesses exhaust available VA. PR #311 reduces subprocess count (`--in-process-gpu`, `--renderer-process-limit=1`) and adds this heartbeat to catch remaining pressure.

## Architecture

```
main.rs:257 → memory_heartbeat::start()
                └── spawns "mem-heartbeat" thread (daemon, no shutdown signal)
                    └── loop { sleep(20s); log_memory_stats(); }
```

- **Thread:** Named `mem-heartbeat`, spawned once at CEF startup, runs for process lifetime
- **Interval:** 20 seconds (hardcoded)
- **Target:** `tracing::info!(target: "mem_heartbeat", ...)`
- **No shutdown:** Thread runs until process exits — no cancellation token needed for a desktop app

## Metrics Logged

### Windows (`cfg(target_os = "windows")`)

**System memory** (via `GlobalMemoryStatusEx`):

| Field | Unit | Description |
|-------|------|-------------|
| `load_pct` | % | Overall memory load percentage |
| `total_phys_gb` | GB | Total physical RAM |
| `avail_phys_gb` | GB | Available physical RAM |
| `total_page_gb` | GB | Total commit limit (RAM + pagefile) |
| `avail_page_gb` | GB | Available commit charge |
| `total_virt_gb` | GB | Total user-mode VA space |
| `avail_virt_gb` | GB | Available user-mode VA space |

**Process memory** (via `GetProcessMemoryInfo`):

| Field | Unit | Description |
|-------|------|-------------|
| `ws_mb` | MB | Current working set (physical pages mapped) |
| `peak_ws_mb` | MB | Peak working set since process start |
| `commit_mb` | MB | Current committed memory (pagefile-backed) |
| `peak_commit_mb` | MB | Peak committed memory |
| `page_faults` | count | Total page faults since start |

### Linux/macOS (`cfg(not(target_os = "windows"))`)

**Process** (from `/proc/self/status`): `VmRSS`, `VmSize`, `VmPeak`
**System** (from `/proc/meminfo`): `MemTotal`, `MemAvailable`

## Log Format

Logs appear in the CEF host log file. Example output:

```
INFO mem_heartbeat: system memory load_pct=42 total_phys_gb=15.9 avail_phys_gb=9.2 total_page_gb=21.5 avail_page_gb=14.1 total_virt_gb=128.0 avail_virt_gb=87.3
INFO mem_heartbeat: process memory ws_mb=312.4 peak_ws_mb=318.7 commit_mb=287.1 peak_commit_mb=290.5 page_faults=82341
```

## Companion Changes (PR #311)

| Switch | Effect | Why |
|--------|--------|-----|
| `--in-process-gpu` | Merges GPU process into browser process | Eliminates one ~100GB VA reservation |
| `--renderer-process-limit=1` | Caps renderer subprocesses to 1 | Prevents DevTools popups spawning additional renderers |

Both switches are appended in `app.rs` via `cmd.append_switch()` during `on_before_command_line_processing`.

## Diagnostic Workflow

When investigating a crash:

1. Find the host log file (see `reference_log_paths.md` for locations)
2. Grep for `mem_heartbeat` entries in the minutes before the crash
3. Look for:
   - `avail_virt_gb` trending toward 0 → VA exhaustion
   - `avail_phys_gb` near 0 with high `load_pct` → physical OOM
   - `ws_mb` growing monotonically → memory leak in browser process
   - `page_faults` spiking → thrashing

## Future Considerations

- Threshold-based warnings (e.g., log at WARN when `avail_virt_gb < 5`)
- Expose metrics to frontend for a sysinfo widget
- Configurable interval via settings
- Child process memory (renderer, utility) — currently only reports browser process
