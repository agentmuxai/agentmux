# Tear-Off Bug Investigation Status

**Date:** 2026-04-07
**Branch:** `agenta/fix-devtools-secondary` (PR #310)
**Reported symptom:** Torn-off window shows red error text ("like AgentMux cannot log in") for ANY pane type (agent, terminal)
**User says:** "broke when updating in PR" — most recent commit `eb67f4c`

---

## What We Know

### Backend is fine
The sidecar log at `%APPDATA%/ai.agentmux.cef.v0-33-58/logs/agentmuxsrv-v0.33.58.log.2026-04-07` shows:
- `TearOffBlock` calls succeed (3 attempts at 14:09:47–14:10:01)
- New workspace + tab created each time (e.g. `new_ws=25b32a5a`, `new_tab=53e139ab`)
- **But no subsequent `CreateWindow` call from the torn-off window's frontend**

This means the torn-off window's `initHostNewWindow()` either:
1. Never runs (page fails to load)
2. Fails before reaching `WindowService.CreateWindow()`
3. Fails at IPC level (auth, port mismatch)

### eb67f4c changes are backward-compatible
Reviewed the full diff — only adds optional `width/height/grabOffsetX/grabOffsetY` params to `openWindowAtPosition`. All existing callers work without them. No changes to `wave.ts`, `cef-init.ts`, or any init path.

### The red error is `showStartupError()`
Located at `frontend/wave.ts:132`. Shows:
- Title: **"AgentMux failed to start"** (red `#ff6b6b`)
- Body: the stringified error in a `<pre>` block
- Hint: "Press F12 for console details."

This is the only code path that produces red error text in a new window. It fires when `initHostNewWindow()` throws (line 393).

### Missing `CreateWindow` in logs = smoking gun
After TearOffBlock succeeds, the NEW window should call:
1. `ClientService.GetClientData()` → HTTP to sidecar
2. `WindowService.CreateWindow(null, tearOffWsId)` → HTTP to sidecar
3. `WorkspaceService.GetWorkspace(...)` → HTTP to sidecar

None of these appear in the log after the tearoff. The new window is failing before or during its first backend call.

---

## Top Hypotheses (ranked by likelihood)

### 1. IPC token/port not reaching the new window's frontend
The CEF host builds the URL with `?ipc_port=X&ipc_token=Y&windowLabel=Z&workspaceId=W`.
`cef-init.ts` reads these from `window.location.search` and sets `window.__AGENTMUX_IPC_PORT__` etc.
If the URL is malformed or params are missing, `callBackendService()` would fail with 401 Unauthorized or connection refused.

**To verify:** Add logging in `cef-init.ts` before and after reading URL params.

### 2. Stale dev binary (v0.33.27) used during `task dev`
`dist/cef-dev/agentmux-cef.exe` was v0.33.27 (from April 6 00:24). That binary doesn't have the `pending_window_labels` fix from c347c9a (v0.33.55). However, v0.33.58 portable on desktop DOES have it.

**Question for user:** Are you testing with the portable v0.33.58 or with `task dev`?

### 3. `setupCefApi()` not called for the new browser window
Each CEF browser window loads the same `index.html` + JS bundle. The bootstrap calls `setupCefApi()` which reads URL params. If the new window's bootstrap somehow skips this, `window.__AGENTMUX_IPC_PORT__` would be undefined and all IPC would fail.

**To verify:** Check `cef-bootstrap.ts` / `tauri-bootstrap.ts` for any early-exit conditions.

### 4. Window opens but loads wrong URL (no query params)
If `open_window_at_position` in `drag.rs` builds the URL incorrectly (e.g. double `?`), the params would be lost.

**To verify:** Check `resolve_frontend_base_url()` return value and separator logic.

### 5. Race condition: page loads before sidecar processes TearOffBlock response
The new window opens (via `post_create_window`) and starts loading JS immediately, but the source window hasn't yet called `completeCrossDrag`. If the new window's `initHostNewWindow()` calls `CreateWindow(null, wsId)` before the workspace is fully committed to the DB, it could fail.

**Less likely** — the TearOffBlock completes in <1ms and the window takes 200ms+ to load.

---

## Next Steps

1. **Check the bootstrap entry point** — read `cef-bootstrap.ts` to see if `setupCefApi()` is guaranteed to run for secondary windows
2. **Check `resolve_frontend_base_url()`** — verify the URL the torn-off window loads has correct query params
3. **Add diagnostic logging** — in `initHostNewWindow()` catch block, log the actual error message to a file (since the CEF host log may not capture frontend errors for secondary windows)
4. **Reproduce with logging** — build with extra `console.log` in `initHostNewWindow` and `setupCefApi`, then tear off a pane

---

## Files Investigated

| File | Status |
|------|--------|
| `frontend/wave.ts` | Read fully — `initHostNewWindow()` and `showStartupError()` understood |
| `frontend/app/drag/CrossWindowDragMonitor.win32.tsx` | Read fully — `performTearOff()` flow traced |
| `agentmux-cef/src/commands/drag.rs` | Read fully — `open_window_at_position()` URL building traced |
| `frontend/types/custom.d.ts` | Diff reviewed — only optional params added |
| `frontend/layout/lib/TileLayout.win32.tsx` | Diff reviewed — `onDragStart` paneRect capture |
| `frontend/util/cef-api.ts` | Noted — needs re-read for `openWindowAtPosition` impl |
| `agentmux-cef/src/main.rs` | Read partially — CEF data dir naming understood |
| `agentmux-srv` sidecar log | Read — TearOffBlock succeeds, no CreateWindow from new window |

## Files NOT Yet Checked

| File | Why it matters |
|------|---------------|
| `frontend/cef-init.ts` | Bootstrap for CEF windows — sets IPC port/token from URL |
| `frontend/cef-bootstrap.ts` | Entry point — does it call `setupCefApi()` for all windows? |
| `agentmux-cef/src/commands/window.rs` | `resolve_frontend_base_url()` — URL correctness |
| `agentmux-cef/src/ui_tasks.rs` | `post_create_window()` — how the new window is actually created |
| `frontend/app/platform/ipc.ts` | `callBackendService()` — error handling for IPC failures |
