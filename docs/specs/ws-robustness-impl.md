# WebSocket Robustness — Implementation Plan

**Date:** 2026-04-02
**Priority:** P0 — UI freezes are unacceptable
**Branch:** `agenta/ws-robustness`

## Changes

### 1. Never give up reconnecting (`ws.ts`)

Replace hard cutoff with infinite retry at slower intervals:

```typescript
// OLD: gives up
if (this.reconnectTimes > 20) {
    dlog("cannot connect, giving up");
    return;
}

// NEW: never give up, slow down
const timeoutArr = [0, 0, 2, 5, 10, 10, 30, 60];
let timeout = 60; // max interval
if (this.reconnectTimes < timeoutArr.length) {
    timeout = timeoutArr[this.reconnectTimes];
}
// Log every 10th attempt after initial burst
if (this.reconnectTimes > 10 && this.reconnectTimes % 10 === 0) {
    console.warn(`[ws] still reconnecting (attempt ${this.reconnectTimes})`);
}
```

### 2. Re-query endpoints on reconnect failure (`ws.ts`)

After 3 failed attempts, re-query the IPC server for current backend endpoints:

```typescript
onclose(event: CloseEvent) {
    // ... existing code ...
    if (this.reconnectTimes > 0 && this.reconnectTimes % 3 === 0) {
        this.refreshEndpoint();
    }
    this.reconnect();
}

private async refreshEndpoint() {
    try {
        const endpoints = await invokeCommand<{ws: string, web: string}>("get_backend_endpoints");
        if (endpoints?.ws) {
            const newBase = `ws://${endpoints.ws}`;
            if (newBase !== this.baseHostPort) {
                console.log(`[ws] endpoint changed: ${this.baseHostPort} → ${newBase}`);
                this.baseHostPort = newBase;
            }
        }
    } catch (e) {
        // IPC might also be down — just keep retrying
    }
}
```

### 3. Visibility-based reconnect (`ws.ts`)

Check WS health on window focus:

```typescript
// In constructor:
document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "visible" && !this.open && !this.opening) {
        console.log("[ws] window visible, attempting reconnect");
        this.reconnectTimes = 0; // reset backoff
        this.connectNow("visibility");
    }
});
```

### 4. Solid logging (`ws.ts`)

Replace `dlog` (debug module, hidden by default) with `console.log`/`console.warn` for critical events:

```typescript
// Always log these (not behind debug flag):
onopen:    console.log("[ws] connected");
onclose:   console.warn("[ws] disconnected", event.code, event.reason);
reconnect: console.log("[ws] reconnecting (attempt ${n})");
giveup:    // REMOVED — never give up
```

### 5. CEF host: monitor sidecar process (`sidecar.rs`)

Spawn a background thread that waits for the sidecar `Child` to exit, then emits `backend-terminated` to all browsers:

```rust
// After spawning sidecar:
let child_pid = child.id();
let state_for_monitor = app_state.clone();
std::thread::spawn(move || {
    let status = child.wait(); // blocks until sidecar exits
    tracing::error!("Sidecar exited: {:?}", status);
    // Emit backend-terminated to all browsers via JS injection
    let browsers = state_for_monitor.browsers.lock().unwrap();
    for (_, browser) in browsers.iter() {
        if let Some(frame) = browser.main_frame() {
            let js = format!(
                "window.dispatchEvent(new CustomEvent('backend-terminated', {{detail: {{pid: {}, code: {:?}}}}}));",
                child_pid, status.as_ref().map(|s| s.code())
            );
            let code = CefString::from(js.as_str());
            let url = CefString::from("");
            frame.execute_java_script(Some(&code), Some(&url), 0);
        }
    }
});
```

### 6. Frontend reconnect indicator (`app.tsx` or overlay)

Show a non-blocking banner when WS is disconnected:

```typescript
// In WSControl:
onclose() → emit custom event "ws-disconnected"
onopen()  → emit custom event "ws-connected"

// In App or a dedicated component:
const [wsConnected, setWsConnected] = createSignal(true);
window.addEventListener("ws-disconnected", () => setWsConnected(false));
window.addEventListener("ws-connected", () => setWsConnected(true));

// Render:
<Show when={!wsConnected()}>
    <div class="ws-reconnecting-banner">Reconnecting...</div>
</Show>
```

## File Changes

| File | Change |
|------|--------|
| `frontend/app/store/ws.ts` | Never give up, refresh endpoint, visibility reconnect, logging |
| `agentmux-cef/src/sidecar.rs` | Monitor thread, emit backend-terminated |
| `frontend/app/app.tsx` or new component | Reconnecting banner |

## Test Plan

- [ ] Kill sidecar → frontend shows "Reconnecting" → sidecar restarts → frontend recovers
- [ ] Background window 5min → foreground → WS reconnects immediately
- [ ] Backend restart → WS reconnects to new port
- [ ] Rapid disconnect/reconnect → no duplicate connections
- [ ] Check logs: every disconnect/reconnect logged to console
