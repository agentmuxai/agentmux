# Benchmark Results: Tauri vs CEF

**Date:** 2026-03-29 20:27 PDT
**Machine:** Area54 — Windows 10 Pro x64 (22H2, 19045)
**Tauri build:** v0.32.106 portable (WebView2 / Edge Chromium)
**CEF build:** v0.32.110 portable (CEF 146 / bundled Chromium)
**Methodology:** 3 runs per build, median reported. 5s UI settle after sidecar detected, 5s cool-down between runs.

---

## Results (3 runs per build, median reported)

| Metric | Tauri (WebView2) | CEF (bundled) | Delta | Winner |
|--------|-----------------|---------------|-------|--------|
| **Disk size** | 30 MB | 365 MB | +335 MB | Tauri |
| **Startup (to sidecar)** | 548 ms | 152 ms | -396 ms (3.6x) | CEF |
| **Baseline RSS (1 term)** | 412 MB | 352 MB | -60 MB | CEF |
| **Process count** | 9 | 8 | -1 | CEF |

## Process Breakdown (baseline, 1 default terminal)

### Tauri (9 processes, 412 MB)

| Process | RSS | Count |
|---------|-----|-------|
| `agentmux.exe` (Tauri host) | 32 MB | 1 |
| `agentmuxsrv-rs.x64.exe` (backend + crash monitor) | 28 MB | 2 |
| `msedgewebview2.exe` (browser, GPU, renderer, crashpad, utility, network) | 352 MB | 6 |

### CEF (8 processes, 352 MB)

| Process | RSS | Count |
|---------|-----|-------|
| `agentmux-cef.exe` (host + GPU + renderer + utility + zygote + crashpad) | 326 MB | 6 |
| `agentmuxsrv-rs.x64.exe` (backend + crash monitor) | 28 MB | 2 |

## Raw Data

### Tauri

| Run | Startup | RSS | Processes |
|-----|---------|-----|-----------|
| 1 | 548 ms | 415 MB | 9 |
| 2 | 482 ms | 412 MB | 9 |
| 3 | 587 ms | 412 MB | 9 |

### CEF

| Run | Startup | RSS | Processes |
|-----|---------|-----|-----------|
| 1 | 349 ms | 351 MB | 8 |
| 2 | 144 ms | 352 MB | 8 |
| 3 | 152 ms | 354 MB | 8 |

## Analysis

### Startup
CEF is **3.6x faster** (152 ms vs 548 ms median to sidecar ready).

Tauri overhead comes from:
- WebView2 runtime discovery and version handshake
- WebView2 UDF (User Data Folder) initialization
- WebView2 broker → renderer process spawn chain (6 processes)

CEF loads `libcef.dll` from the local directory with no system discovery step.
CEF run 1 (349 ms) was slower — likely first-time DLL loading into cache.

### Memory
CEF uses **60 MB less** at baseline (352 MB vs 412 MB). Both builds spawn
a similar number of Chromium subprocesses, but:
- WebView2's 6 processes carry per-process overhead from the shared Edge runtime
  (profile data, extensions state, telemetry)
- CEF runs a purpose-built single-app Chromium with no shared runtime state
- The sidecar is identical (28 MB across both builds)

### Disk
Tauri: 30 MB (relies on system WebView2 runtime, ~150-200 MB pre-installed)
CEF: 365 MB (bundles everything, including 251 MB libcef.dll)

With file stripping applied (locales, SwiftShader, WebGPU): CEF drops to ~335 MB.
This is the fundamental trade-off: 12x larger distribution for faster startup
and lower memory.

## Notes

- Warm-start measurements (not cold boot)
- Memory measured 8s after sidecar ready (UI fully loaded)
- All processes matching each build's patterns captured via `tasklist`
- Backend sidecar is identical binary in both builds (28 MB)
- CEF exposes CDP on port 9222 (DevTools available for FPS measurement)

## Next Steps

- [ ] Per-terminal memory scaling (1, 2, 4, 8 terminals)
- [ ] Terminal scroll FPS (seq 100000) via CDP
- [ ] Input latency via CDP
- [ ] Cold start (after reboot)
- [ ] 4-hour stability soak test
