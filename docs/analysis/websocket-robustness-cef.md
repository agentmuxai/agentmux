# WebSocket Robustness in CEF

**Date:** 2026-04-02
**Trigger:** UI freeze during testing — WebSocket disconnected, frontend went unresponsive

## Current State

The WebSocket layer (`frontend/app/store/ws.ts`) has reconnect logic from the Tauri era:

### What exists (PR #233, #223)
- `WSControl.reconnect()` — exponential backoff up to 60s, max 20 attempts then gives up
- `WSControl.changeEndpoint()` — reconnects to new port after backend restart
- `backendStatus.ts` — listens for `backend-terminated` / `backend-ready` events
- Ping/pong heartbeat every 5 seconds
- Message queue for offline buffering

### What works
- Reconnects after clean close
- Reconnects after backend restart (new port)
- Detects backend crash via `backend-terminated` event

### What doesn't work (CEF-specific gaps)

#### 1. No `backend-terminated` event in CEF
The `backend-terminated` event is emitted by the Tauri host when the sidecar dies. In CEF, the host (`main.rs`) spawns the sidecar via `std::process::Child` with a Windows Job Object (KILL_ON_JOB_CLOSE). If the sidecar crashes:
- The Job Object kills it
- But there's no event emitted to the frontend
- `backendStatusAtom` stays "running" while WS is dead
- UI becomes unresponsive

**Fix needed:** CEF host should monitor the sidecar process and emit `backend-terminated` event when it exits.

#### 2. Gives up after 20 reconnect attempts
`reconnectTimes > 20` → stops trying. If the sidecar is slow to restart or the port changes, the frontend permanently gives up.

**Fix needed:** Never fully give up — switch to longer intervals (e.g., 60s) but keep trying. Show a reconnecting indicator to the user.

#### 3. No visibility-based reconnect
When the window regains focus after being backgrounded, the WS might be dead but the reconnect timer hasn't fired yet.

**Fix needed:** On `visibilitychange` or `focus`, check WS state and reconnect immediately.

#### 4. WS URL hardcoded at init time
The WS URL is set during `initGlobalWS()` from the backend endpoints. If the sidecar restarts with a different port, only the `backend-ready` event path updates it. If that event is missed (race condition), the WS reconnects to the old port forever.

**Fix needed:** On reconnect failure, re-query the backend endpoints via IPC before retrying.

## What Caused Today's Freeze

The sidecar log shows two WebSocket disconnects at 14:59:18 and 14:59:21. After that, no reconnection — the frontend was frozen.

Likely sequence:
1. Something caused the WebSocket connection to drop (possibly a large terminal write, or the sidecar briefly stalled)
2. The `onclose` handler fired → `reconnect()` called
3. Reconnect may have failed (sidecar port changed, or 20 attempts exhausted from previous disconnects)
4. Frontend stuck with no WS — all RPC calls hang, UI becomes unresponsive

## Priority Fixes

### P0: Emit backend-terminated in CEF host
Monitor sidecar process in a background thread. On exit, emit event to all browsers via JS injection.

### P1: Never give up reconnecting
Change `reconnectTimes > 20` to slower intervals instead of giving up.

### P2: Re-query endpoints on reconnect failure
If WS connect fails, call `get_backend_endpoints` IPC to get current port.

### P3: Frontend reconnect indicator
Show a banner when WS is disconnected so the user knows the UI is stale.

## References
- PR #233: `fix(ws): reconnect to new backend endpoint after restart`
- PR #223: `feat: sidecar modularization + backend crash recovery`
- `frontend/app/store/ws.ts` — WSControl class
- `frontend/app/store/backendStatus.ts` — backend status signals
- `agentmux-cef/src/sidecar.rs` — sidecar spawning
