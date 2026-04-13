# Retro: AgentMux Shows Raw Browser Window on Launch

**Date:** 2026-03-31
**Trigger:** Launching v0.33.14 portable build showed a bare web browser instead of the app UI.
**Frequency:** Intermittent — happens on some launches, not all. More common on first run or slow I/O.

## What the user sees

A CEF window opens but instead of the AgentMux UI, it shows what looks like a raw web browser — either a blank page, a spinner that never resolves, or a CEF error page. No title bar, no panes, no widgets.

## Root Cause: Race Condition Between CEF Window and Backend Sidecar

The startup sequence in `agentmux-cef/src/main.rs` is:

```
T=0ms    IPC HTTP server binds to random port        (line 127, synchronous)
T=1ms    Backend sidecar spawned ASYNCHRONOUSLY       (line 134, tokio::spawn)
T=2ms    CEF initialized, browser window created      (line 168+)
T=3ms    Frontend loads from IPC server               (immediate)
T=???ms  Sidecar actually starts, emits backend-ready (variable: 100ms–10s+)
```

The browser window is created **immediately** after the IPC server starts, but the backend sidecar is spawned asynchronously. The frontend loads, calls `get_backend_endpoints` (in `cef-api.ts:39`), and if the sidecar hasn't started yet, falls into a 30-second wait for a `backend-ready` event.

If the sidecar **never starts** (wrong binary path, permission error, AV blocking), the frontend times out and shows nothing useful — just a bare CEF window.

## Why It's Worse in Portable Builds

1. **First launch:** No OS disk cache. Windows Defender scans every new `.exe` on first execution.
2. **Sidecar binary resolution:** `sidecar.rs` searches multiple directories to find `agentmuxsrv-rs.x64.exe` — adds latency.
3. **Slow media:** USB drives, network shares, or HDDs amplify the race window.
4. **Multiple instances:** Job Object creation adds overhead.

## Why It Works in Dev Mode

`task dev` starts Vite first (synchronous), then launches CEF. The frontend connects to `localhost:5173` (Vite HMR) and the backend is started by the dev task separately. The race doesn't exist because Vite is always ready before CEF opens.

## Contributing Code Paths

| File | Lines | Role |
|------|-------|------|
| `agentmux-cef/src/main.rs` | 127 | IPC server starts (sync) |
| `agentmux-cef/src/main.rs` | 134-165 | Sidecar spawn (async — the race) |
| `agentmux-cef/src/app.rs` | 174-200 | URL resolution: portable checks `frontend/index.html`, dev falls back to `localhost:5173` |
| `agentmux-cef/src/app.rs` | 206-210 | IPC port/token appended as query params |
| `agentmux-cef/src/sidecar.rs` | 23-262 | Sidecar spawning + endpoint detection |
| `frontend/util/cef-api.ts` | 35-64 | `setupCefApi()` — tries `get_backend_endpoints`, waits 30s for `backend-ready` |
| `frontend/cef-init.ts` | 20-43 | Detects CEF mode from `ipc_port` query param |

## URL Resolution Can Also Fail

`app.rs` line 191 checks for `frontend/index.html` relative to the exe. In the portable layout, the launcher is in root but the CEF host is in `runtime/`. If the path resolution picks the wrong base directory, it won't find the frontend and falls back to `http://localhost:5173` — which doesn't exist in a portable build. Result: CEF error page.

```rust
let has_frontend = exe_dir
    .as_ref()
    .map(|d| d.join("frontend/index.html").exists())
    .unwrap_or(false);
if has_frontend {
    format!("http://127.0.0.1:{}", self.ipc_port)  // IPC serves frontend
} else {
    "http://localhost:5173".to_string()              // Vite dev fallback — WRONG for portable
}
```

## Fix Options (Ranked)

### 1. Block CEF window creation until sidecar is ready (recommended)

Make the sidecar spawn **synchronous** — wait for `WAVESRV-ESTART` before creating the browser window. The user sees nothing for 0.5–2s (acceptable), then the full UI appears at once.

```rust
// Instead of tokio::spawn, block:
let result = sidecar::spawn_and_wait(&state).await?;
// THEN create browser window
```

### 2. Show a local loading page while waiting

Serve a static `loading.html` from the IPC server that polls `/health` until the backend is ready, then redirects to the real frontend. No race — CEF always has something to show.

### 3. Retry loop in frontend (current approach, fragile)

The current 30s timeout in `cef-api.ts` is a workaround, not a fix. It silently fails on slow machines and gives no feedback to the user.

## Lessons

- **Async sidecar spawn was premature optimization.** The 100-500ms saved by not blocking is invisible to the user, but the race condition creates a terrible first impression.
- **Portable builds are the harshest environment.** Dev mode hides race conditions because everything starts in a controlled order.
- **"Works on my machine" is real here.** Fast SSDs + warm OS cache mask the race. First launch on a clean machine is the true test.
- **The fallback to `localhost:5173` should not exist in release builds.** It's a dev convenience that becomes a silent failure mode in production.
