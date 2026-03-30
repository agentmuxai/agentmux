# Benchmark Results: Tauri vs CEF

**Date:** 2026-03-29 20:27 PDT
**Machine:** Area54 — Windows 10 Pro x64 (22H2, 19045)
**Tauri build:** v0.32.106 portable (WebView2 / Edge Chromium)
**CEF build:** v0.32.110 portable (CEF 146 / bundled Chromium)
**Methodology:** 3 runs per build, median reported. 5s UI settle after sidecar detected, 5s cool-down between runs.

---

## Results

| Metric | Tauri (WebView2) | CEF (bundled) | Delta | Winner |
|--------|-----------------|---------------|-------|--------|
| **Disk size** | 30 MB | 365 MB | +335 MB | Tauri |
| **File count** | 4 | 307 | +303 | Tauri |
| **Startup (to sidecar ready)** | 593 ms | 130 ms | -463 ms (4.5x) | CEF |
| **Baseline RSS** | 424 MB | 350 MB | -74 MB | CEF |

## Process Breakdown (baseline, 0 terminals open)

### Tauri

| Process | RSS |
|---------|-----|
| `agentmux.exe` (Tauri host) | 32 MB |
| `agentmuxsrv-rs.x64.exe` (backend) | 28 MB |
| `msedgewebview2.exe` (5 processes) | 364 MB |
| **Total** | **424 MB** |

### CEF

| Process | RSS |
|---------|-----|
| `agentmux-cef.exe` (CEF host + subprocesses) | 320 MB |
| `agentmuxsrv-rs.x64.exe` (backend) | 28 MB |
| **Total** | **350 MB** (reported under single process tree) |

## Analysis

### Startup
CEF is **4.5x faster** (130 ms vs 593 ms to sidecar ready). The ~460 ms
Tauri overhead comes from:
- WebView2 runtime discovery and handshake
- WebView2 UDF (User Data Folder) initialization
- WebView2 broker → renderer process spawn chain
- CEF loads from a local DLL with no system discovery step

### Memory
CEF uses **74 MB less** than Tauri at baseline. This is surprising — CEF bundles its own
Chromium while Tauri shares the system Edge.

Possible explanations:
- WebView2 spawns 5 separate processes (browser, GPU, renderer, crashpad, utility) with
  per-process overhead. CEF uses fewer subprocess types for a single-window app.
- WebView2's shared runtime loads shared state for all WebView2 apps on the system,
  adding overhead that a single-purpose CEF host doesn't carry.
- CEF's alloy-style mode may use a lighter process model than WebView2's full Edge.

### Disk
Tauri wins massively here — 30 MB vs 365 MB. Tauri relies on the system-installed
WebView2 runtime (~150-200 MB, but shared across all apps and pre-installed on Win 10/11).
CEF bundles everything. This is the fundamental trade-off.

## Notes

- Both builds use the same backend sidecar (28 MB RSS, identical across runs)
- Startup times measured to sidecar process appearing (no artificial sleep)
- These are warm-start measurements (not first-boot cold start)
- Memory measurements capture all processes matching each app's pattern
- CEF subprocesses (GPU, renderer) are child processes of `agentmux-cef.exe` and captured
  in its total via tasklist's process tree

## Next Steps

- [ ] Cold start (after reboot) comparison
- [ ] Per-terminal memory scaling (1, 2, 4, 8 terminals)
- [ ] Terminal scroll FPS (seq 100000)
- [ ] Input latency via CDP
- [ ] 4-hour stability soak test
