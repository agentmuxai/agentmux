# White Screen Overnight — 2026-04-08

**Reporter:** User (area54)
**Investigator:** AgentA
**Instances affected:** v0.33.62, v0.33.64, v0.33.73 (all 3 running portable instances)
**Symptom:** All instances showed solid white screen after overnight idle. Processes still running, heartbeats still ticking.

---

## Evidence from Logs

### Smoking gun: `webglcontextlost` in v0.33.64

```
2026-04-08T17:16:30.331Z  [fe] webglcontextlost event received
2026-04-08T17:16:30.408Z  WebSocket client disconnected conn_id=b02380e9...
```

The frontend received a `webglcontextlost` event, then the WebSocket disconnected. No crash, no error, no OOM — the process continued running but the rendering surface was gone.

### Timeline per instance

| Instance | Last real [fe] activity | webglcontextlost? | Heartbeat still running? |
|----------|------------------------|-------------------|-------------------------|
| v0.33.62 | 16:07 (idle timeout save) | Not logged (may have fired before log-pipe was ready, or xterm canvas lost context silently) | Yes, 18:08 |
| v0.33.64 | 10:41 (focus event), then 17:16 (context lost) | **Yes** — 17:16:30 UTC | Yes, 18:08 |
| v0.33.73 | 10:13 (startup + initial render) | Not logged | Yes, 18:08 |

### Memory at time of white screen (all healthy)

| Instance | Working Set | Peak WS | Commit | System Avail |
|----------|------------|---------|--------|-------------|
| v0.33.62 | 158 MB | 213 MB | 203 MB | 19.3 GB |
| v0.33.64 | 170 MB | 177 MB | 225 MB | 19.3 GB |
| v0.33.73 | 148 MB | 148 MB | 152 MB | 19.3 GB |

No OOM. No memory pressure. System load was 39% with 19+ GB available.

### Other errors (benign, unrelated)

All 3 instances logged on startup:
```
[UNHANDLED-REJECTION] Cannot read properties of undefined (reading 'getPlatform')
```
This is a race condition during CEF API initialization — the platform bridge isn't ready when the frontend first tries to call it. Separate bug, not the cause of white screen.

---

## Root Cause Analysis

### What happened

Windows performed a display driver reset overnight (TDR — Timeout Detection and Recovery, or monitor sleep/wake power state change). This invalidated all GPU contexts system-wide.

### Why it caused white screen

PR #311 added `--in-process-gpu` to merge the GPU process into the CEF browser process. This has a known tradeoff documented in the code:

> Tradeoff: GPU driver crash kills the app instead of just restarting the GPU process — acceptable for a local desktop app.
> — `agentmux-cef/src/app.rs:252`

However, the actual behavior is **worse than documented**. A GPU context loss with `--in-process-gpu` doesn't crash the app — it leaves the app in a **zombie rendering state**:
- Process alive (heartbeat ticking)
- Backend alive (websocket was connected until context loss)
- Frontend JavaScript alive (can still log)
- But the rendering surface (WebGL/compositor context) is gone
- Result: solid white screen with no way to recover

### With vs without `--in-process-gpu`

| Scenario | Separate GPU process | In-process GPU |
|----------|---------------------|----------------|
| GPU context lost | GPU process crashes, Chromium restarts it, page re-composits | Rendering surface gone, no recovery path |
| Driver TDR reset | GPU process restarts transparently | White screen zombie state |
| App crash risk | GPU crash isolated | GPU crash can kill entire app |
| Memory overhead | +100GB VA (virtual, not physical) | Eliminated |

### Online research confirms this

- **Electron issue [#11934](https://github.com/electron/electron/issues/11934)**: `webglcontextrestored` event does NOT fire after GPU process respawn — Chromium bug, still open
- **Electron issue [#31625](https://github.com/electron/electron/issues/31625)**: WebGL context lost on screen minimize — same class of bug
- **Electron issue [#8517](https://github.com/electron/electron/issues/8517)**: Context lost when switching displays with dual GPU
- **CEF Forum [thread](https://www.magpcss.org/ceforum/viewtopic.php?f=6&t=17386)**: CEF crash after Windows wake up — GPU context loss after sleep, "Lost UI shared context" errors
- **Chromium design doc on [GPU compositing](https://www.chromium.org/developers/design-documents/gpu-accelerated-compositing-in-chrome/)**: GPU process exists for crash isolation; in-process mode removes that isolation
- **Microsoft TechCommunity [bug report](https://techcommunity.microsoft.com/discussions/windows11/bug-applications-can-crash-after-sleephibernation-if-uses-dedicated-graphics/4024966)**: Applications crash after sleep/hibernation when using dedicated graphics — GPU disabled during sleep, app can't find device on wake
- **Khronos WebGL wiki on [handling context loss](https://khronos.org/webgl/wiki/HandlingContextLost)**: Must call `event.preventDefault()` on `webglcontextlost` and re-init on `webglcontextrestored`

---

## Fix Options

### Option 1: Remove `--in-process-gpu` (recommended)

**Restore the separate GPU process.** Let Chromium's built-in crash recovery handle driver resets.

- **Pro:** Battle-tested recovery path, no code changes needed beyond removing the flag
- **Con:** Adds one process (~100GB virtual address space, ~20-50MB physical RAM)
- **The 100GB VA concern from PR #311 was about PartitionAlloc pools** — this is virtual address space, not physical memory. On a 64-bit system with 128TB VA space, 100GB is 0.07%. It's free.

```rust
// Remove these lines from app.rs:249-255:
// let gpu_key = CefString::from("in-process-gpu");
// cmd.append_switch(Some(&gpu_key));
```

### Option 2: Add `webglcontextlost` handler with page reload

Listen for context loss in the frontend, reload the page to re-establish the rendering surface.

```javascript
// In frontend init:
document.addEventListener('webglcontextlost', (event) => {
    event.preventDefault();
    console.error('[recovery] WebGL context lost — reloading page');
    setTimeout(() => window.location.reload(), 500);
}, true);
```

- **Pro:** Works with or without `--in-process-gpu`, preserves memory savings
- **Con:** User sees a page flash/reload; terminal state may be lost mid-session; `contextrestored` doesn't reliably fire in Chromium ([electron#11934](https://github.com/electron/electron/issues/11934)) so reload is the only reliable recovery
- **Risk:** If the driver is permanently gone (not just a TDR), this creates a reload loop

### Option 3: Both (belt and suspenders)

Remove `--in-process-gpu` AND add the `webglcontextlost` handler. The handler becomes a safety net for edge cases where even the separate GPU process can't recover.

---

## Recommendation

**Option 1 (remove `--in-process-gpu`)** for the immediate fix. The VA overhead is negligible and the separate GPU process is Chromium's designed recovery path.

**Option 2 as a follow-up** regardless — `webglcontextlost` can happen even with a separate GPU process in rare cases, and a reload handler is cheap insurance.

The `--renderer-process-limit=1` and memory heartbeat from PR #311 are still valuable and should be kept.

---

## Secondary Bug: `getPlatform` UNHANDLED-REJECTION

All 3 instances log on startup:
```
[UNHANDLED-REJECTION] Cannot read properties of undefined (reading 'getPlatform')
```

This is a race condition where the frontend calls `getPlatform()` before the CEF API bridge (`window.__AGENTMUX_CEF__` or similar) is injected. Not related to the white screen but should be fixed separately — likely needs a ready-state gate or retry in the CEF API initialization path.
