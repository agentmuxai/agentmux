# AgentMux Memory Retrospective — From v0.12 to v0.33.62

**Date:** 2026-04-07
**Author:** AgentA (Claude Opus 4.6)

---

## The Journey

### Phase 1: Electron Era (v0.12.x — Oct 2025)
- **Stack:** Electron + Go backend + Node.js
- **Memory profile:** Electron's multi-process model (main + renderer + GPU + utility)
- **Binary size:** ~25 MB Go backend alone
- **Pain points:** electron-builder packaging bugs, hardware acceleration crashes on RDP/Sandbox
- **Notable:** v0.14.0 disabled hardware acceleration entirely for Windows Sandbox/RDP compatibility

### Phase 2: Tauri Migration (v0.15.x–v0.27.x — Jan–Feb 2026)
- **Stack:** Tauri (WebView2) + Go backend → transitioning to Rust
- **Memory profile:** WebView2 shared renderer, lighter than Electron
- **Pain points:**
  - `custom-protocol` feature gate bug: webview always loaded production bundle even in dev mode (fixed PR #6)
  - System WebView2 version inconsistency across machines
  - CGO signal faults on macOS (systray crash, v0.27.14)

### Phase 3: Go → 100% Rust (v0.29–v0.31 — Feb 2026)
- **Stack:** Tauri + 100% Rust backend
- **Binary size:** 5.5 MB total (agentmuxsrv 4.4 MB + wsh 1.1 MB) — **78% smaller** than Go
- **Memory:** 3.6x lower baseline than Go backend (measured at v0.29.0)
- **Notable:** Deleted all Go source code. Rewrote wsh in Rust (1.1 MB vs 11 MB, 90% reduction)

### Phase 4: CEF Migration (v0.32.x–v0.33.x — Mar–Apr 2026)
- **Stack:** CEF (Chromium Embedded Framework) + Rust backend
- **Why:** Pinned Chromium version eliminates system WebView2 version lottery
- **Memory profile:** CEF's PartitionAlloc reserves massive virtual address space per subprocess:
  - **32 GB GigaCage** per process
  - **4 x 16 GB pools** per process
  - **4 GB V8 pointer cage** per process
  - Total: **~100 GB VA** per CEF subprocess
  - On a 16 GB machine with 87 GB total user-mode VA, 3 subprocesses left <3 GB free

---

## The Crashes

### OOM #1: FileStore Cache Leak (v0.32.77, fixed v0.32.78 — PR #222)
- **Symptom:** Sidecar crash `0xC0000409` after 4+ hours of multi-agent usage
- **Root cause:** `flush_cache()` was a complete no-op — nothing ever set `dirty=true`, so the cache grew forever. Plus `write_file()` duplicated all data into `data_entries` that was never read.
- **Growth rate:** ~4 MB/hour under multi-agent workloads
- **Fix:** Added LRU eviction with 60s TTL, stopped caching write data
- **Lesson:** WER crash dumps configured (`%LOCALAPPDATA%\CrashDumps\`) for future forensics

### OOM #2: VA Exhaustion from CEF Subprocesses (v0.33.57, mitigated v0.33.58 — PR #311)
- **Symptom:** Potential VA exhaustion on machines with limited address space
- **Root cause:** Each CEF subprocess (browser, GPU, renderer) reserves ~100 GB VA. DevTools popups could spawn additional renderers.
- **Fix (PR #311):**
  - `--in-process-gpu` — merges GPU process into browser, eliminates one ~100 GB reservation
  - `--renderer-process-limit=1` — caps renderer spawns
  - Added `memory_heartbeat` module for forensic telemetry

### Logging Gap: Heartbeat Writing to /dev/null (v0.33.58–v0.33.61)
- **Problem:** CEF host tracing went to stderr only. In portable/GUI mode, stderr is not captured.
- **Effect:** Heartbeat was running but output was lost — defeating its forensic purpose
- **Fix (v0.33.62):** Added `tracing-appender` with rolling daily log file to `~/.agentmux/logs/`

---

## Where We Are Now (v0.33.62)

### Live Heartbeat Data (first ~2 minutes of runtime)

**System:**
| Metric | Value |
|--------|-------|
| Physical RAM | 31.9 GB total, 22.6 GB available |
| Memory load | 29% |
| Page file | 76.5 GB total, 63.9 GB available |
| Virtual address space | 131 TB total, 130.9 TB available |

**CEF Host Process:**
| Metric | First Beat (T+20s) | Latest (T+100s) | Delta |
|--------|---------------------|------------------|-------|
| Working set | 179.6 MB | 168.1 MB | -11.5 MB (settled) |
| Peak working set | 182.2 MB | 182.2 MB | stable |
| Commit | 252.0 MB | 163.6 MB | -88.4 MB (GC/decommit) |
| Peak commit | 255.6 MB | 255.6 MB | stable |
| Page faults | 118,608 | 120,210 | +1,602 (normal) |

### Process Count (with --in-process-gpu)
- **Browser process** (includes GPU) — 1 process
- **Renderer** — 1 process (capped by --renderer-process-limit=1)
- **Total VA reserved:** ~200 GB (2 processes x ~100 GB) vs ~300 GB before (3 processes)
- **VA headroom:** 130.9 TB available — not a concern on this 64-bit machine

### Binary Sizes (current)
| Component | Size |
|-----------|------|
| agentmux-launcher (agentmux.exe) | 325 KB |
| agentmux-cef | ~5 MB |
| agentmux-srv | ~4.4 MB |
| wsh | ~1.1 MB |
| CEF runtime (DLLs + resources) | ~300 MB |
| **Portable ZIP** | **152 MB** |

---

## Timeline Summary

| Version | Date | Stack | Backend Binary | Baseline Memory | Key Event |
|---------|------|-------|----------------|-----------------|-----------|
| v0.12.x | Oct 2025 | Electron + Go | 25 MB | High | Initial fork |
| v0.15.x | Jan 2026 | Tauri + Go | 25 MB | Medium | Agent colors, reactive messaging |
| v0.29.0 | Feb 2026 | Tauri + Rust | 5.5 MB | 3.6x lower than Go | Go deleted, 100% Rust |
| v0.32.78 | Mar 2026 | CEF + Rust | 5.5 MB | Stable (leak fixed) | FileStore OOM fix |
| v0.33.58 | Apr 2026 | CEF + Rust | 5.5 MB | ~180 MB WS | --in-process-gpu, heartbeat added |
| v0.33.62 | Apr 2026 | CEF + Rust | 5.5 MB | ~168 MB WS | File logging, heartbeat visible |

---

## What's Left

1. **Long-running stability:** The FileStore leak is fixed, but the heartbeat is new — need 24+ hour soak tests to confirm no new leaks
2. **Heartbeat alerting:** Currently log-only. Could add threshold-based warnings (e.g., >80% memory load)
3. **VA on 32-bit / low-VA machines:** Not a concern on this 64-bit dev machine (131 TB VA), but `--in-process-gpu` is insurance for constrained environments
4. **CEF multi-window:** Each new window could spawn additional renderer processes — monitor with heartbeat
