# Performance Comparison Spec: Tauri vs CEF Builds

**Status:** Draft
**Date:** 2026-03-29
**Author:** AgentA

## Overview

This spec defines how to benchmark the two host/shell variants of AgentMux:

| | Tauri Build | CEF Build |
|---|---|---|
| **WebView engine** | System WebView2 (Edge/Chromium) on Windows; WebKitGTK on Linux | Bundled Chromium (CEF 146, libcef.dll ~251 MB) |
| **Frontend** | Same React + xterm.js bundle | Same React + xterm.js bundle |
| **Backend** | Same Rust sidecar (agentmuxsrv-rs) | Same Rust sidecar (agentmuxsrv-rs) |

The only variable under test is the host process and its embedded browser engine. Everything else is held constant.

---

## 1. Metrics to Measure

### 1.1 Startup Performance

| Metric | Definition |
|---|---|
| **Cold start time** | From `CreateProcess` of the exe to the first terminal cursor blink (visible on screen). Measure with the machine freshly rebooted, no prior launch in the session. |
| **Warm start time** | Same measurement, but on the second launch (OS file cache and GPU shader cache are warm). |
| **Sidecar spawn time** | From host exe start to `agentmuxsrv-rs` process appearing in the process list. Should be near-identical between builds; include it to confirm. |
| **Time to first WebSocket** | From exe launch to the first successful WebSocket `open` event logged by the frontend (`performance.mark("ws-connected")`). |
| **Time to first terminal render** | From exe launch to xterm.js firing its first `onRender` callback with visible content. |

### 1.2 Memory Usage

Measure **all** processes that belong to the application (host, renderer, GPU, utility/network, sidecar). Use `tasklist /fi "PID eq ..."` or the scripted approach in Section 3.

| Scenario | How to measure |
|---|---|
| **Baseline (empty window)** | Launch, wait 10 s after UI is idle, snapshot RSS of every process. |
| **Per-terminal scaling** | Open 1, 2, 4, 8 terminals sequentially. Snapshot after each, wait 5 s between opens. Report total RSS and delta per terminal. |
| **1-hour idle** | Open 4 terminals, leave them idle for 60 minutes, snapshot. Compare to the 4-terminal snapshot above. |
| **Sustained output stress** | In one terminal, run `yes \| head -c 50000000` (50 MB of output). Snapshot during and 10 s after completion. |
| **Process breakdown** | For each scenario, report per-process: host, GPU process, each renderer/utility, sidecar. |

### 1.3 Rendering Performance

| Metric | How |
|---|---|
| **Terminal scroll FPS** | Run `seq 1 100000` in a terminal. Measure frame rate using CDP `Performance.metrics` (frameCount delta / time delta) or `requestAnimationFrame` counting in the frontend. |
| **CSS animation FPS** | Trigger a pane resize drag. Count `requestAnimationFrame` callbacks during the drag to compute sustained FPS. |
| **Input latency** | Send a keystroke via CDP `Input.dispatchKeyEvent`, measure time until the echoed character appears in the terminal DOM (`MutationObserver` on the xterm canvas or `onRender`). Report p50 and p99 over 200 keystrokes. |
| **Resize reflow time** | Programmatically resize the window by 200 px in each axis using the OS API (`SetWindowPos` on Windows). Measure time from resize event to xterm.js `onResize` completing (PTY SIGWINCH round-trip). |

### 1.4 GPU / Compositor

| Metric | How |
|---|---|
| **GPU process memory** | RSS of the GPU process (separate from renderer). |
| **GPU acceleration status** | Navigate to `chrome://gpu` (CEF) or query `navigator.gpu` / WebGL context creation. For Tauri/WebView2, use `--enable-features=msEdgeDevToolsWdpProxy` to inspect. |
| **WebGL availability** | Create a `WebGLRenderingContext`; report version string and renderer string. |
| **Compositor frame rate** | CDP `Performance.metrics` → `FrameCount` sampled over 10 s with active terminal output. |

### 1.5 Disk and Distribution

| Metric | How |
|---|---|
| **Installed size on disk** | Uncompressed portable folder size (`du -sh` or PowerShell `(Get-ChildItem -Recurse \| Measure-Object -Property Length -Sum).Sum`). |
| **Download size** | Compressed ZIP artifact size. |
| **First-launch disk writes** | Monitor writes to `~/.agentmux/`, WebView2 UDF / CEF cache dirs. Use `procmon` filtered to the host process, or diff directory sizes before and after first launch. |

### 1.6 Stability

| Metric | How |
|---|---|
| **Process crash count** | Run a 4-hour stress session (8 terminals, periodic `yes` bursts every 15 min). Count unexpected process exits via a watchdog script polling `tasklist` every 5 s. |
| **WebView/browser recovery** | Intentionally kill the renderer process (`taskkill /PID <renderer>`). Measure time to recover or report if the app crashes entirely. |
| **Sidecar crash recovery** | Kill `agentmuxsrv-rs.exe`. Both builds should reconnect. Measure reconnection time. |

---

## 2. Test Methodology

### 2.1 Environment Controls

- **Same machine, same OS, same user session** for all runs.
- Close all non-essential applications. Disable antivirus real-time scanning if possible.
- Disable Windows Update, indexing service, and other background I/O during the test window.
- Both builds must use the **same version** of the frontend bundle and the same sidecar binary.
- Pin the app version (e.g., v0.32.110) and record the exact commit hash.

### 2.2 Repetition and Reporting

- Run each test **5 times**.
- Report the **median** value (not the mean — avoids outlier skew).
- For latency measurements, also report **p95** and **p99**.
- Discard runs where a background process spike is detected (> 10% CPU by a non-test process during the measurement window).

### 2.3 Target Platforms

| Priority | Platform | Notes |
|---|---|---|
| **P0** | Windows 10 x64 (22H2, 19045) | Primary development target. Both Tauri (WebView2) and CEF builds available. |
| **P1** | Linux x64 (Ubuntu 24.04) | Tauri uses WebKitGTK here — rendering pipeline differs significantly. |
| **P2** | macOS arm64 (Sonoma 15) | If builds available. Tauri uses WKWebView. |

### 2.4 Cold Start Protocol

1. Reboot the machine.
2. Log in, wait 60 s for background services to settle.
3. Launch the app under test via `Measure-Command` (see Section 3).
4. Record all timing marks.
5. Close the app, wait 10 s.
6. Launch again (warm start).
7. Record all timing marks.

---

## 3. Tools and Scripts

### 3.1 Startup Timing (Windows)

```powershell
# cold-start-bench.ps1
# Run from an elevated PowerShell prompt after reboot.

param(
    [string]$ExePath,
    [int]$Runs = 5
)

$results = @()

for ($i = 1; $i -le $Runs; $i++) {
    Write-Host "Run $i of $Runs..."

    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $proc = Start-Process -FilePath $ExePath -PassThru

    # Poll for the sidecar process
    $sidecarTime = $null
    while (-not $sidecarTime) {
        $sidecar = Get-Process -Name "agentmuxsrv-rs" -ErrorAction SilentlyContinue
        if ($sidecar) { $sidecarTime = $sw.ElapsedMilliseconds }
        Start-Sleep -Milliseconds 50
    }

    # Wait for the frontend to signal readiness via a marker file.
    # The frontend writes this file at the end of initLogPipe().
    $markerFile = "$env:TEMP\agentmux-bench-ready"
    Remove-Item $markerFile -ErrorAction SilentlyContinue
    while (-not (Test-Path $markerFile)) {
        Start-Sleep -Milliseconds 50
    }
    $totalTime = $sw.ElapsedMilliseconds
    $sw.Stop()

    $results += [PSCustomObject]@{
        Run           = $i
        SidecarMs     = $sidecarTime
        TotalMs       = $totalTime
    }

    Stop-Process -Id $proc.Id -Force -ErrorAction SilentlyContinue
    Start-Sleep -Seconds 5
}

$results | Format-Table -AutoSize
$median = ($results | Sort-Object TotalMs)[[math]::Floor($Runs / 2)]
Write-Host "Median total startup: $($median.TotalMs) ms"
```

> **Frontend marker:** Add a temporary bench hook in `main.tsx` that writes a file or posts a CDP event when the first terminal renders. Remove after benchmarking.

### 3.2 Memory Snapshot (Bash / Git Bash)

```bash
#!/usr/bin/env bash
# mem-snapshot.sh — Capture RSS of all AgentMux processes.
# Usage: ./mem-snapshot.sh [label]

LABEL="${1:-snapshot}"
TIMESTAMP=$(date +%Y%m%d-%H%M%S)
OUTFILE="mem-${LABEL}-${TIMESTAMP}.csv"

echo "process,pid,rss_kb" > "$OUTFILE"

# Match host process (agentmux.exe or agentmux-cef.exe) and all children.
for name in agentmux agentmux-cef agentmuxsrv-rs msedgewebview2 libcef; do
    tasklist //fi "IMAGENAME eq ${name}*" //fo csv //nh 2>/dev/null \
      | grep -i "$name" \
      | while IFS=, read -r img pid session sessnum mem; do
          pid=$(echo "$pid" | tr -d '"')
          mem=$(echo "$mem" | tr -d '"' | tr -d ' K' | tr -d ',')
          echo "${name},${pid},${mem}" >> "$OUTFILE"
        done
done

echo "Saved to $OUTFILE"
cat "$OUTFILE" | column -t -s,
```

### 3.3 Rendering FPS via CDP

```bash
#!/usr/bin/env bash
# fps-bench.sh — Measure frame rate during terminal scroll.
# Requires CDP port 9222 (CEF default) or WebView2 remote debugging.

CDP_PORT="${1:-9222}"
WS_URL=$(curl -s "http://localhost:${CDP_PORT}/json" | python3 -c "
import sys, json
targets = json.load(sys.stdin)
for t in targets:
    if t.get('type') == 'page':
        print(t['webSocketDebuggerUrl'])
        break
")

if [ -z "$WS_URL" ]; then
    echo "ERROR: No CDP target found on port $CDP_PORT"
    exit 1
fi

echo "CDP target: $WS_URL"
echo "Connect with a CDP client and run Performance.getMetrics before and after the scroll test."
echo ""
echo "Manual steps:"
echo "  1. In a terminal pane, prepare: seq 1 100000"
echo "  2. Call Performance.getMetrics -> record FrameCount (T0)"
echo "  3. Execute the seq command"
echo "  4. Wait for completion"
echo "  5. Call Performance.getMetrics -> record FrameCount (T1)"
echo "  6. FPS = (T1.FrameCount - T0.FrameCount) / elapsed_seconds"
```

### 3.4 Input Latency via CDP

```python
#!/usr/bin/env python3
"""
input-latency-bench.py — Measure keypress-to-echo latency via CDP.

Requires: pip install websocket-client

Usage: python input-latency-bench.py [cdp_port]
"""

import json
import time
import sys
import websocket

CDP_PORT = int(sys.argv[1]) if len(sys.argv) > 1 else 9222
NUM_KEYS = 200

# Discover target
import urllib.request
targets = json.loads(urllib.request.urlopen(f"http://localhost:{CDP_PORT}/json").read())
ws_url = next(t["webSocketDebuggerUrl"] for t in targets if t["type"] == "page")

ws = websocket.create_connection(ws_url)
msg_id = 0

def send_cdp(method, params=None):
    global msg_id
    msg_id += 1
    ws.send(json.dumps({"id": msg_id, "method": method, "params": params or {}}))
    while True:
        resp = json.loads(ws.recv())
        if resp.get("id") == msg_id:
            return resp

# Enable DOM mutation tracking
send_cdp("Runtime.enable")

latencies = []
for i in range(NUM_KEYS):
    char = chr(ord('a') + (i % 26))
    t0 = time.perf_counter_ns()
    send_cdp("Input.dispatchKeyEvent", {
        "type": "keyDown",
        "key": char,
        "text": char,
    })
    # Wait for render
    send_cdp("Runtime.evaluate", {
        "expression": "new Promise(r => requestAnimationFrame(() => requestAnimationFrame(r)))",
        "awaitPromise": True,
    })
    t1 = time.perf_counter_ns()
    latencies.append((t1 - t0) / 1e6)  # ms

ws.close()

latencies.sort()
p50 = latencies[len(latencies) // 2]
p95 = latencies[int(len(latencies) * 0.95)]
p99 = latencies[int(len(latencies) * 0.99)]

print(f"Input latency over {NUM_KEYS} keystrokes:")
print(f"  p50: {p50:.2f} ms")
print(f"  p95: {p95:.2f} ms")
print(f"  p99: {p99:.2f} ms")
```

### 3.5 Terminal Scaling Memory Test

```bash
#!/usr/bin/env bash
# terminal-scaling.sh — Open N terminals and snapshot memory after each.
# Requires: agentmux running, mem-snapshot.sh in PATH.

for n in 1 2 4 8; do
    echo "=== Opening terminal $n ==="
    # Use the RPC API to create a new terminal block.
    # This assumes the agentmux CLI or a helper script is available.
    curl -s -X POST "http://localhost:1729/api/create-block" \
        -H "Content-Type: application/json" \
        -d '{"controller":"shell"}' > /dev/null

    sleep 5  # Let the terminal settle
    ./mem-snapshot.sh "terminals-${n}"
done
```

---

## 4. CDP Access Notes

### CEF Build
- CEF exposes CDP on port **9222** by default (configured via `--remote-debugging-port=9222`).
- All standard CDP domains are available: `Performance`, `Runtime`, `Input`, `DOM`, `Page`.

### Tauri / WebView2 Build
- WebView2 supports remote debugging when launched with `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--remote-debugging-port=9223`.
- Set this environment variable before launching the Tauri build.
- Alternatively, use the frontend `window.performance` API and `performance.mark()` / `performance.measure()` for timing (no CDP dependency).

### Frontend Performance Marks (Both Builds)

Add these marks to the frontend startup path for consistent cross-build measurement:

```typescript
// In main.tsx or app initialization
performance.mark("app-script-start");

// After WebSocket connects
performance.mark("ws-connected");

// After first terminal onRender
performance.mark("first-terminal-render");

// Collect
performance.measure("startup-to-ws", "app-script-start", "ws-connected");
performance.measure("startup-to-render", "app-script-start", "first-terminal-render");
```

These marks are readable via CDP `Performance.getEntriesByType("measure")` or directly in the frontend.

---

## 5. Output Format

### Results Table

All results go into a single Markdown table per metric category. Example:

```markdown
### Startup Performance (Windows 10 x64, median of 5 runs)

| Metric                        | Tauri (WebView2) | CEF 146    | Delta      | Winner |
|-------------------------------|------------------|------------|------------|--------|
| Cold start (ms)               | 1,230            | 1,890      | +660 (54%) | Tauri  |
| Warm start (ms)               | 410              | 620        | +210 (51%) | Tauri  |
| Sidecar spawn (ms)            | 180              | 185        | +5 (3%)    | Tie    |
| Time to first WS (ms)        | 320              | 480        | +160 (50%) | Tauri  |
| Time to first render (ms)    | 780              | 1,100      | +320 (41%) | Tauri  |
```

> Values above are **placeholder examples** -- replace with actual measurements.

### Summary File

Produce a final summary file `results/tauri-vs-cef-results-YYYYMMDD.md` containing:

1. **Test environment** (hardware, OS version, driver versions, WebView2 runtime version, CEF version).
2. **All results tables** (startup, memory, rendering, GPU, disk, stability).
3. **Narrative analysis** (2-3 paragraphs interpreting the results).
4. **Recommendation** (which build to ship as default, and under what conditions the other is preferable).

---

## 6. Known Expectations

Based on architecture differences, expected outcomes before measurement:

| Area | Expected Winner | Rationale |
|---|---|---|
| Cold start | Tauri | WebView2 is pre-installed on Windows 10/11; no DLL loading for a bundled Chromium. |
| Warm start | Closer | Both benefit from OS cache; CEF's larger DLLs may still lag slightly. |
| Baseline memory | Tauri | WebView2 shares process infrastructure with Edge; CEF spawns its own process tree. |
| Per-terminal memory | Similar | Both use Chromium renderer internals for xterm.js. |
| Rendering FPS | CEF (slight) | CEF 146 ships a newer Chromium than most WebView2 evergreen versions. May have compositor improvements. |
| Input latency | Similar | Both are Chromium-based. |
| Disk size | Tauri | WebView2 is a system component (0 MB bundled). CEF adds ~251 MB (libcef.dll alone). |
| Stability | Tauri | Mature integration; CEF integration is new and less battle-tested. |
| Linux rendering | CEF | WebKitGTK (Tauri on Linux) has known xterm.js performance issues. CEF provides consistent Chromium on all platforms. |

These are hypotheses. The benchmarks will confirm or refute them.

---

## 7. Checklist Before Running

- [ ] Both builds compiled at the same git commit.
- [ ] Same frontend bundle copied into both builds (verify with `sha256sum` on `dist/` contents).
- [ ] Same `agentmuxsrv-rs` binary used by both (verify with `sha256sum`).
- [ ] WebView2 runtime version recorded (`Get-ItemProperty 'HKLM:\SOFTWARE\...\EdgeUpdate\Clients\{F3017226-...}'`).
- [ ] CEF version recorded (check `libcef.dll` version info or `cef_version.h`).
- [ ] Performance marks added to the frontend startup path (see Section 4).
- [ ] CDP access verified for both builds (see Section 4).
- [ ] `mem-snapshot.sh` tested and producing correct output.
- [ ] Machine rebooted, background processes minimized.
- [ ] Results directory created: `results/tauri-vs-cef-results-YYYYMMDD.md`.
