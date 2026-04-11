# Pane Tear-Off: Root Cause — CEF Shared Renderer Process

**Date:** 2026-03-31
**Status:** ROOT CAUSE CONFIRMED

---

## Root Cause

**CEF Alloy mode shares the renderer process across browser windows created by `browser_host_create_browser`.** This means:

1. **Module-level state is shared** — `savedInitOpts` in `wave.ts` is set by the main window, so the tear-off window's `initWaveWrap` may call `reinitWave()` instead of `initWave()`
2. **`document` may reference the wrong DOM** — if the renderer process is shared, `document.getElementById("main")` returns the main window's element, not the tear-off window's
3. **SolidJS render tree exists only once** — `render(App, elem)` in the main window creates the SolidJS tree; the tear-off window either skips render or renders into the wrong DOM

### Evidence

From trace logging in `initWave`:
- Main window: all 7 trace steps fire, `render()` produces 1 child, `AppInner` mounts ✅
- Tear-off window: "Init Wave" log fires (line 593), then **ZERO trace logs** — not even the very next `sendLog` at line 594

This is impossible if the JS context is truly isolated. The only explanation: the tear-off window either:
- Shares the renderer process and `initWave` operates on stale/wrong state
- OR the tear-off window never actually runs `initWave` — the "Init Wave" log is from the main window reacting to WPS updates

### The `sendLog` routing problem

All CEF browser windows share the same IPC HTTP server. `sendLog` calls go to the same endpoint with no window identifier. We cannot distinguish which window produced which log line. The "Init Wave" at timestamp `.994` might actually be from the main window (re-initializing after the source layout changes from `TearOffBlock`).

---

## Fix: Separate renderer processes per window

### Option A: Use `--renderer-process-per-site` CEF flag

Force each browser to use its own renderer process. Each window gets isolated JS, isolated `document`, isolated module state.

In `app.rs` or wherever CEF settings are configured:
```rust
settings.renderer_process_per_site = true;
// OR use the command-line switch:
// --renderer-process-per-site
```

### Option B: Use CEF `request_context` per browser

Create a new `CefRequestContext` for each browser window in `open_window_at_position`. Each request context gets its own renderer process.

### Option C: Use separate-process window creation

Instead of `browser_host_create_browser` (in-process), spawn a new `agentmux-cef.exe` process for each window (like the portable launcher does). Each process has fully isolated CEF.

### Option D: Frontend-side workaround (quickest)

Make the frontend robust to shared-renderer mode:
- Pass `windowLabel` to `initWave` and use it to scope all DOM operations
- Never rely on module-level singletons — use per-window state maps
- Key SolidJS render trees by window label

This is the most work but doesn't require CEF changes.

---

## Recommended Fix

**Option A** is simplest. One line in CEF settings forces process isolation per site (which in our case = per window since they all load from `localhost:5173`). Each window gets its own V8 isolate, its own `document`, its own module state.

If that flag doesn't isolate per-window (since they're same-origin), try `--site-per-process` or create distinct request contexts (Option B).

---

## Files Involved

| File | Role |
|------|------|
| `agentmux-cef/src/app.rs` | CEF settings — add renderer isolation |
| `agentmux-cef/src/commands/drag.rs:337` | `browser_host_create_browser` — may need `request_context` |
| `frontend/wave.ts:515` | `initWaveWrap` — `savedInitOpts` shared across windows |
| `frontend/wave.ts:585` | `initWave` — `render(App, elem)` targets wrong DOM |
