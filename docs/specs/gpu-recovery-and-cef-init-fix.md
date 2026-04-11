# Implementation Spec: GPU Context Recovery + CEF API Init Fix

**Date:** 2026-04-08
**Author:** AgentA
**Related:** [white-screen-overnight-2026-04-08.md](../analysis/white-screen-overnight-2026-04-08.md)
**Branch:** `agenta/gpu-recovery-cef-init`
**Affects:** `agentmux-cef`, `frontend`

---

## Problem Statement

Three portable instances (v0.33.62, v0.33.64, v0.33.73) went to solid white screen overnight while the machine and monitor stayed on. Processes remained alive (heartbeats ticking), but the rendering surface was gone. Root cause: `webglcontextlost` event fired (confirmed in v0.33.64 logs) while `--in-process-gpu` was active, leaving no recovery path. Separately, all 3 instances log `Cannot read properties of undefined (reading 'getPlatform')` on every startup — a race condition in CEF API initialization.

---

## Change 1: Remove `--in-process-gpu`

### Rationale

With `--in-process-gpu`, GPU context loss leaves the app in a zombie state — process alive but rendering dead. Without it, Chromium runs GPU compositing in a separate process that can crash and restart transparently. The ~100GB virtual address overhead is irrelevant on 64-bit (0.07% of 128TB VA space) and costs ~20-50MB physical RAM.

### File: `agentmux-cef/src/app.rs`

**Remove lines 249–255:**

```rust
// DELETE:
                // Merge GPU process into the browser process. Eliminates one
                // ~100GB virtual address space reservation from PartitionAlloc
                // pools (32GB GigaCage + 4×16GB pools + 4GB V8 pointer cage).
                // Tradeoff: GPU driver crash kills the app instead of just
                // restarting the GPU process — acceptable for a local desktop app.
                let gpu_key = CefString::from("in-process-gpu");
                cmd.append_switch(Some(&gpu_key));
```

**Keep** the `--renderer-process-limit=1` (lines 257–262) — that's still valuable.

### Expected result

- One additional process in Task Manager: `agentmux-cef-0.33.XX.exe` (GPU process)
- GPU context loss → GPU process crashes → Chromium restarts it → page re-composits
- No more zombie white screen state

---

## Change 2: Add `webglcontextlost` recovery handler

### Rationale

Even with a separate GPU process, `webglcontextlost` can occur in rare edge cases (driver TDR, DXGI device removal, GPU resource eviction with multiple instances). The `webglcontextrestored` event is unreliable in Chromium ([electron#11934](https://github.com/electron/electron/issues/11934)). A page reload is the only guaranteed recovery.

### File: `frontend/tauri-bootstrap.ts`

**Add a document-level `webglcontextlost` listener early in `bootstrap()`, before any rendering starts.**

Insert after the `initLogPipe()` call (after line 17, before the static CSS imports):

```typescript
// Recover from GPU context loss (e.g., driver reset, display power state change).
// webglcontextrestored is unreliable in Chromium, so reload the page instead.
// The 'capture: true' flag ensures we see the event even if it fires on a canvas
// deep in the DOM (xterm WebGL addon, etc).
let contextLostReloading = false;
document.addEventListener("webglcontextlost", (event) => {
    event.preventDefault(); // Tell browser we'll handle recovery
    if (contextLostReloading) return; // Prevent reload loop from multiple canvases
    contextLostReloading = true;
    console.error("[recovery] WebGL context lost — reloading page in 1s");
    setTimeout(() => window.location.reload(), 1000);
}, true);
```

### Why at this level (not in termwrap.ts)

`termwrap.ts:456` already handles per-terminal context loss by falling back to the DOM renderer. But that only covers xterm's canvas. If the **compositor context** (CEF's own rendering) is lost, the entire page goes white — no individual canvas handler can fix that. The document-level handler is the safety net for compositor-level context loss.

### Guard against reload loops

The `contextLostReloading` flag prevents multiple simultaneous reloads. If the driver is permanently dead, the page will reload once, hit context loss again, and reload again — but the 1s delay + flag means it won't spin. In practice, if the GPU process restarts (Change 1), the second load will succeed.

If we want extra safety, we can add a sessionStorage counter:

```typescript
const reloadCount = parseInt(sessionStorage.getItem("__ctx_lost_reloads") || "0");
if (reloadCount >= 3) {
    console.error("[recovery] WebGL context lost 3 times — giving up, switching to DOM renderer");
    sessionStorage.removeItem("__ctx_lost_reloads");
    // Force DOM renderer mode globally (set a flag that termwrap reads)
    sessionStorage.setItem("__force_dom_renderer", "1");
    return;
}
sessionStorage.setItem("__ctx_lost_reloads", String(reloadCount + 1));
// Clear the counter after 30s of stable running
setTimeout(() => sessionStorage.removeItem("__ctx_lost_reloads"), 30000);
```

**Recommendation:** Start with the simple version (no counter). Add the counter only if reload loops are observed in practice.

---

## Change 3: Fix `getPlatform` UNHANDLED-REJECTION

### Root cause

`tauri-bootstrap.ts` statically imports `wave.ts` (line 12):
```typescript
import { initBare } from "./wave";
```

`wave.ts` declares at module scope (line 47):
```typescript
let platform: NodeJS.Platform;
```

This is fine — `platform` is assigned later in `initBare()` at line 408:
```typescript
platform = getApi().getPlatform();
```

The error comes from **something else** calling `getApi().getPlatform()` before `setupCefApi()` completes. Since `getApi()` returns `window.api` (which is `undefined` before `setupCefApi()`), it throws `Cannot read properties of undefined (reading 'getPlatform')`.

### The actual culprit

The error fires during the `initCefApi()` → `Promise.all([...invokeCommand()])` batch (cef-api.ts:72-94). While those IPC calls are in-flight, some **other code** (likely a side effect of static imports, or an eager SolidJS reactive subscription) calls `getApi()` before `window.api` is set.

The `unhandledrejection` handler in `tauri-bootstrap.ts:236` catches it but can't prevent it.

### Fix: Guard `getApi()` to return a safe stub before initialization

**File: `frontend/app/store/global.ts`** (line 521)

Replace:
```typescript
export function getApi(): AppApi {
    return window.api;
}
```

With:
```typescript
export function getApi(): AppApi {
    if (!window.api) {
        throw new Error("[getApi] window.api is not initialized yet — called too early in bootstrap");
    }
    return window.api;
}
```

This converts a cryptic `Cannot read properties of undefined (reading 'getPlatform')` into a clear `[getApi] window.api is not initialized yet` error that points to the actual problem.

**However**, this won't prevent the error — it just improves the message. The real fix depends on finding what calls `getApi()` before bootstrap completes.

### Better fix: Defensive guard in `initBare()`

**File: `frontend/wave.ts`** (line 404-408)

Replace:
```typescript
export async function initBare() {
    // window.api is guaranteed to exist here — tauri-bootstrap.ts calls
    // setupTauriApi() or setupCefApi() before calling initBare().
    // Assign deferred module-level values now.
    platform = getApi().getPlatform();
```

With:
```typescript
export async function initBare() {
    // window.api is guaranteed to exist here — tauri-bootstrap.ts calls
    // setupTauriApi() or setupCefApi() before calling initBare().
    // Assign deferred module-level values now.
    if (!window.api) {
        console.error("[initBare] window.api not available — waiting for host API init");
        // This should never happen if bootstrap order is correct, but guard against it
        await new Promise<void>((resolve) => {
            const check = setInterval(() => {
                if (window.api) { clearInterval(check); resolve(); }
            }, 50);
            setTimeout(() => { clearInterval(check); resolve(); }, 5000); // 5s timeout
        });
    }
    platform = getApi().getPlatform();
```

### Root cause hunt: find the early caller

To find the actual code that calls `getApi()` too early, add a temporary diagnostic:

**File: `frontend/app/store/global.ts`** (in `getApi()`)

```typescript
export function getApi(): AppApi {
    if (!window.api) {
        console.error("[getApi] called before window.api exists. Stack:", new Error().stack);
    }
    return window.api;
}
```

Build, launch, check the host log for the stack trace. The stack will show exactly which module/line calls `getApi()` during static import resolution. Fix that caller, then remove the diagnostic.

---

## Files Changed Summary

| File | Change |
|------|--------|
| `agentmux-cef/src/app.rs` | Remove `--in-process-gpu` flag (lines 249-255) |
| `frontend/tauri-bootstrap.ts` | Add `webglcontextlost` document listener (after line 17) |
| `frontend/app/store/global.ts` | Add diagnostic + guard to `getApi()` (line 521) |
| `frontend/wave.ts` | Add defensive `window.api` wait in `initBare()` (line 404) |

---

## Verification Plan

### Pre-build checklist

- [ ] `bump patch -m "fix: remove in-process-gpu, add WebGL recovery, fix CEF init race"`
- [ ] `cargo check -p agentmux-cef` — Rust compiles
- [ ] `npx tsc --noEmit` — TypeScript compiles
- [ ] `npm install --package-lock-only` + commit lock sync

### Build and deploy

- [ ] `task cef:package:portable` — produces ZIP on Desktop
- [ ] Extract to new folder, launch `agentmux.exe`

### Test 1: Startup — no `getPlatform` error

1. Launch portable build
2. Check host log: `~/.agentmux/logs/agentmux-host-v*.log.*`
3. **PASS:** No `UNHANDLED-REJECTION` with `getPlatform` in the log
4. **PASS:** `[fe] ✅ Main application loaded successfully` appears
5. **PASS:** If diagnostic `getApi()` guard fires, log shows stack trace pointing to the early caller

### Test 2: GPU process visible in Task Manager

1. Open Task Manager → Details tab
2. **PASS:** Two `agentmux-cef-0.33.XX.exe` entries visible (browser + GPU process)
3. **PASS:** Previously only one entry existed (in-process GPU was merged)

### Test 3: Memory heartbeat still running

1. Wait 30 seconds after launch
2. Check host log for `mem_heartbeat` entries
3. **PASS:** System memory + process memory entries appear every 20s

### Test 4: WebGL context loss recovery (manual trigger)

1. Open DevTools (if available) or use the Chrome DevTools Protocol
2. In DevTools console, find an xterm canvas and force context loss:
   ```javascript
   const canvas = document.querySelector('canvas');
   const gl = canvas.getContext('webgl2') || canvas.getContext('webgl');
   const ext = gl.getExtension('WEBGL_lose_context');
   ext.loseContext();
   ```
3. **PASS:** Log shows `[recovery] WebGL context lost — reloading page in 1s`
4. **PASS:** Page reloads within ~1s and app is fully functional after reload
5. **PASS:** Terminal state is restored after reload (backend preserves it)

### Test 5: Overnight soak (the real test)

1. Launch 2 portable instances side by side
2. Open at least one terminal pane in each
3. Leave running overnight (8+ hours, machine and monitor stay on)
4. **PASS:** Both instances still rendering correctly in the morning
5. **PASS:** No `webglcontextlost` in logs (GPU process handles recovery transparently)
6. **PASS:** If `webglcontextlost` does fire, page auto-reloaded and app recovered

### Test 6: Multiple rapid context losses (reload loop guard)

1. In DevTools console, trigger `ext.loseContext()` three times in quick succession
2. **PASS:** Only one reload occurs (flag prevents multiple)
3. **PASS:** App recovers and is functional after the single reload

### Regression checks

- [ ] Terminal rendering works (text, colors, scrolling)
- [ ] Per-pane zoom (Ctrl+/-, Ctrl+Scroll) still works
- [ ] Context menu (right-click) works
- [ ] Command palette (Ctrl+P) opens
- [ ] Window close kills all child processes (check Task Manager)
- [ ] Pane splitting works (terminal shows "running", not "Disconnected")
