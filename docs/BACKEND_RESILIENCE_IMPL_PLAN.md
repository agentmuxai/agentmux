# Backend Resilience Phase 2 — Implementation Plan

**Date:** 2026-03-24
**Branch:** `agenta/backend-resilience-phase2`
**Goal:** Ship restart + better crash observability so long-running sessions recover without app restart.

---

## Scope

| # | Item | File(s) | Priority |
|---|------|---------|----------|
| 1 | `restart_backend` Tauri command | `commands/backend.rs`, `lib.rs` | P0 |
| 2 | WS client reconnect after restart | `store/ws.ts`, `store/global.ts` | P0 |
| 3 | `backendStatusAtom` init fix (`"connecting"`) | `store/global.ts` | P1 |
| 4 | Restart button in Offline popover | `BackendStatus.tsx` | P1 |
| 5 | Version link suppression when offline | `StatusBar.tsx`, `StatusBar.scss` | P2 |
| 6 | WER crash dump registration (Windows) | `sidecar.rs` or `crash.rs` | P2 |
| 7 | Pre-ESTART vs runtime crash distinction in logs | `sidecar.rs` | P2 |

**Out of scope:** auto-restart (crash loop risk), heartbeat watchdog, minidump-writer integration (separate PRs).

---

## 1. `restart_backend` Tauri Command

**File:** `src-tauri/src/commands/backend.rs`

```rust
#[tauri::command]
pub async fn restart_backend(
    app: tauri::AppHandle,
) -> Result<crate::sidecar::BackendSpawnResult, String> {
    tracing::info!("[restart_backend] user-initiated restart");

    // Kill existing sidecar if still alive
    {
        let state = app.state::<crate::state::AppState>();
        let mut sidecar = state.sidecar_child.lock().unwrap();
        if let Some(child) = sidecar.take() {
            let _ = child.kill();
            tracing::info!("[restart_backend] killed stale sidecar");
        }
    }

    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    let result = crate::sidecar::spawn_backend(&app).await?;

    // Update stored endpoints
    {
        let state = app.state::<crate::state::AppState>();
        let mut endpoints = state.backend_endpoints.lock().unwrap();
        endpoints.ws_endpoint = result.ws_endpoint.clone();
        endpoints.web_endpoint = result.web_endpoint.clone();
    }

    // Notify ALL windows (not just main)
    let payload = serde_json::json!({
        "ws": result.ws_endpoint,
        "web": result.web_endpoint,
    });
    for window in app.webview_windows().values() {
        let _ = window.emit("backend-ready", &payload);
    }

    Ok(result)
}
```

Register in `lib.rs`:
```rust
commands::backend::restart_backend,
```

**Notes:**
- 500ms sleep gives the OS time to release the port before re-bind
- Broadcasts to ALL windows — not just main — so secondary windows also reconnect
- Returns `BackendSpawnResult` to confirm new endpoints; frontend ignores the return value

---

## 2. WS Client Reconnect

The `WSControl.baseHostPort` is set at construction and never updated. On restart the port may change.

### 2a. `store/ws.ts` — add `changeEndpoint`

```rust
changeEndpoint(newBaseHostPort: string) {
    this.noReconnect = false;
    this.baseHostPort = newBaseHostPort;
    this.reconnectTimes = 0;
    if (this.wsConn) {
        this.wsConn.close(); // triggers onclose → reconnect() with new baseHostPort
    } else {
        this.connectNow("changeEndpoint");
    }
}
```

Export `changeEndpoint` indirectly via `globalWS`.

### 2b. `store/global.ts` — backend-ready handler

Current:
```typescript
getApi().listen("backend-ready", () => setBackendStatusAtom("running"));
```

New:
```typescript
getApi().listen("backend-ready", (event) => {
    const payload = (event as any)?.payload as { ws?: string; web?: string } | null;
    if (payload?.ws) {
        // Update endpoint globals
        (window as any).__WAVE_SERVER_WS_ENDPOINT__ = payload.ws;
        (window as any).__WAVE_SERVER_WEB_ENDPOINT__ = payload.web;
        // Reconnect WS to new endpoint
        const { globalWS } = await import("./ws"); // lazy import to avoid circular
        globalWS?.changeEndpoint(`ws://${payload.ws}`);
    }
    setBackendStatusAtom("running");
});
```

Wait — `listen` callback can't be async. Use a wrapper:
```typescript
getApi().listen("backend-ready", (event) => {
    const payload = (event as any)?.payload as { ws?: string; web?: string } | null;
    if (payload?.ws) {
        (window as any).__WAVE_SERVER_WS_ENDPOINT__ = payload.ws;
        (window as any).__WAVE_SERVER_WEB_ENDPOINT__ = payload?.web ?? "";
        // globalWS is a module-level var; import ws synchronously (no circular dep)
        import("./ws").then(({ globalWS }) => {
            globalWS?.changeEndpoint(`ws://${payload.ws}`);
        });
    }
    setBackendStatusAtom("running");
});
```

Actually `import("./ws")` from `global.ts` is fine — `ws.ts` doesn't import from `global.ts` (only `wshrpcutil.ts` does, and it imports `getApi` from `global.ts`). Let's verify no circular dep before coding.

Actually safer: export a `reconnectWS(newEndpoint: string)` function from `ws.ts` that calls `globalWS?.changeEndpoint(...)`. Then import it at the top of global.ts:
```typescript
import { reconnectWS } from "./ws";
```

Check circularity: `ws.ts` → no import from `global.ts`. Clean.

---

## 3. `backendStatusAtom` Initial State

**File:** `frontend/app/store/global.ts`, line 102

```typescript
// Before:
export const [backendStatusAtom, setBackendStatusAtom] = createSignal<BackendStatusState>("running");

// After:
export const [backendStatusAtom, setBackendStatusAtom] = createSignal<BackendStatusState>("connecting");
```

The `BackendStatus.tsx` already handles `"connecting"` state (shows `◌` + "Connecting…"). No UI change needed.

---

## 4. Restart Button in BackendStatus.tsx

Add inside the crashed popover section (after the last `<Show when={backendDeathInfoAtom()!.signal != null}>` block):

```tsx
const [restarting, setRestarting] = createSignal(false);

const handleRestart = () => {
    setRestarting(true);
    setBackendStatusAtom("connecting");
    setPopoverOpen(false);
    getApi().restartBackend().catch((e: unknown) => {
        console.error("[BackendStatus] restart failed:", e);
        setBackendStatusAtom("crashed");
    }).finally(() => {
        setRestarting(false);
    });
};
```

Button JSX (inside the crashed `<Show>` block):
```tsx
<div class="status-bar-popover-divider" />
<div class="status-bar-popover-row">
    <button
        class="status-bar-restart-btn"
        disabled={restarting()}
        onClick={handleRestart}
    >
        {restarting() ? "Restarting…" : "Restart Backend"}
    </button>
</div>
```

CSS in `StatusBar.scss`:
```scss
.status-bar-restart-btn {
    width: 100%;
    padding: 4px 8px;
    background: var(--error-color);
    color: #fff;
    border: none;
    border-radius: 3px;
    cursor: pointer;
    font-size: 11px;
    opacity: 0.85;

    &:hover:not(:disabled) { opacity: 1; }
    &:disabled { opacity: 0.4; cursor: default; }
}
```

---

## 5. Version Link Suppression

**File:** `frontend/app/statusbar/StatusBar.tsx`

Import `atoms` if not already imported, then:

```tsx
import { atoms, getApi, windowInstanceNumAtom, windowCountAtom } from "@/store/global";

// In JSX:
<Show when={version}>
    <Show
        when={atoms.backendStatusAtom() !== "crashed"}
        fallback={
            <span class="status-version status-version-offline" title="Backend offline">
                v{version}
            </span>
        }
    >
        <span class="status-version clickable" onClick={handleNewWindow} title="Open New AgentMux Window">
            v{version}
            <Show when={windowCount() > 1}>
                <span class="instance-num"> ({instanceNum()})</span>
            </Show>
        </span>
    </Show>
</Show>
```

**File:** `frontend/app/statusbar/StatusBar.scss`

```scss
.status-version-offline {
    opacity: 0.2;
    cursor: default;
    pointer-events: none;
}
```

---

## 6. WER Crash Dump Registration (Windows)

**File:** `src-tauri/src/crash.rs` (check if exists) or new `src-tauri/src/wer.rs`

On Windows, `0xC0000409` fast-fail crashes produce no stderr output. The only way to get callstack data is Windows Error Reporting local crash dumps. One-time setup at app startup:

```rust
#[cfg(target_os = "windows")]
pub fn register_wer_local_dumps(dump_dir: &std::path::Path) {
    use windows_sys::Win32::System::Registry::*;
    // HKLM\SOFTWARE\Microsoft\Windows\Windows Error Reporting\LocalDumps\agentmuxsrv-rs.exe
    // DumpFolder = dump_dir
    // DumpCount = 10
    // DumpType = 2 (full dump)
    // ... registry write via RegCreateKeyExW / RegSetValueExW
}
```

This is the only mechanism that survives a `__fastfail` — the OS writes the minidump before process termination. Dumps go to `~/.agentmux/crashdumps/`.

**Risk:** Requires `HKLM` write access (admin) on Windows. Alternative: `HKCU` local dumps via `HKCU\SOFTWARE\...` — check if WER supports per-user key.

**Decision:** Include as a best-effort call — log a warning if it fails (non-admin), don't block startup.

---

## 7. Distinguish Pre-ESTART vs Runtime Crash in Logs

In `sidecar.rs`, the event loop `tokio::spawn` starts before ESTART is received. Track whether ESTART was received:

```rust
let mut estart_received = false;

// ... inside the event loop:
CommandEvent::Terminated(status) => {
    if !estart_received {
        tracing::error!("[agentmuxsrv-rs] STARTUP CRASH — terminated before ESTART (pid={} exit_code={:?} uptime_secs={:?})",
            pid, status.code, uptime_secs);
    } else {
        tracing::error!("[agentmuxsrv-rs] RUNTIME CRASH — pid={} exit_code={:?} uptime_secs={:?}",
            pid, status.code, uptime_secs);
    }
    // ...
}
```

And set `estart_received = true` when the ESTART line is parsed. This tells us instantly whether a crash was during startup (DB open failure, port bind failure) vs runtime (OOM/abort after hours).

---

## `AppApi` Type Changes

**File:** `frontend/types/custom.d.ts`

Add to `AppApi`:
```typescript
restartBackend: () => Promise<void>;
```

**File:** `frontend/util/tauri-api.ts`

Add to `buildTauriApi()`:
```typescript
restartBackend: async () => {
    await invoke("restart_backend");
},
```

---

## File Changelist

| File | Change |
|------|--------|
| `src-tauri/src/commands/backend.rs` | Add `restart_backend` command |
| `src-tauri/src/lib.rs` | Register `restart_backend` in invoke handler |
| `src-tauri/src/sidecar.rs` | Track `estart_received` flag; distinguish startup vs runtime crash in log |
| `frontend/types/custom.d.ts` | Add `restartBackend` to `AppApi` |
| `frontend/util/tauri-api.ts` | Implement `restartBackend` in shim |
| `frontend/app/store/ws.ts` | Add `changeEndpoint` method + `reconnectWS` export |
| `frontend/app/store/global.ts` | Fix `"running"` → `"connecting"` initial state; handle WS reconnect on `backend-ready` |
| `frontend/app/statusbar/BackendStatus.tsx` | Add `handleRestart` + Restart button in crashed popover |
| `frontend/app/statusbar/StatusBar.tsx` | Suppress version link when crashed |
| `frontend/app/statusbar/StatusBar.scss` | Add `.status-version-offline`, `.status-bar-restart-btn` |

---

## Testing Checklist

- [ ] Normal startup: status shows `◌ Connecting…` briefly then transitions to `● <uptime>`
- [ ] Kill backend with Task Manager → status shows `● Offline` with death info in popover
- [ ] Click "Restart Backend" → shows `◌ Connecting…` → transitions back to `● <uptime>` with new uptime
- [ ] Terminal panes reconnect after restart (WS endpoint change handled)
- [ ] Version link is non-interactive while Offline, clickable when running
- [ ] Second window: both windows show Offline on crash, both show Running after restart
- [ ] Startup crash (kill before ESTART): log shows "STARTUP CRASH" message
