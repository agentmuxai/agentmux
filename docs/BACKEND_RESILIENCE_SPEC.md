# Backend Resilience: Death Diagnostics & Restart

**Status:** Draft
**Scope:** `src-tauri/src/sidecar.rs`, `frontend/app/statusbar/BackendStatus.tsx`, `frontend/app/statusbar/StatusBar.tsx`, `frontend/app/store/global.ts`, new Tauri command

---

## Problem

When `agentmuxsrv-rs` dies unexpectedly, the user sees "Offline" in the status bar dot but:

1. **No diagnostic info** — exit code, signal, uptime at death, and last-known state are not surfaced anywhere visible
2. **No recovery path** — user must close and reopen the entire AgentMux window to get a new backend
3. **Version link still active** — clicking the version when offline tries to open a new window but the backend is gone; pointless and misleading
4. **Root cause is opaque** — the sidecar log just stops; even the Tauri host log misses the terminated event if the Tauri process itself died

### What we observed in the wild (v0.32.73 incident)
- Backend sidecar log stopped at `06:46:15` with no error, no exit code, no `Terminated` log entry
- No host log for v0.32.73 at all — Tauri app died before flushing
- The `CommandEvent::Terminated` handler in `sidecar.rs:388` exists but its `tracing::warn!` only reaches the host log — which may not exist yet or may not flush before death
- `backend_started_at` is stored in `AppState` but never included in the terminated event payload

---

## Goals

1. **Richer death diagnostics** — when the backend dies, log and emit: exit code, signal, PID, uptime in seconds, last-known version
2. **Backend restart** — add a `restart_backend` Tauri command; frontend "Offline" state calls it
3. **Status bar cleanup when offline** — replace the version string's `onClick` (open new window) with nothing; suppress version link entirely when backend is crashed

---

## Non-Goals

- Auto-restart on every crash without user action (risky: crash loop)
- Persisting terminal state across restart (complex; out of scope)
- Heartbeat / watchdog timer (nice-to-have, separate PR)

---

## Design

### 1. Sidecar.rs — Richer `Terminated` Event

**File:** `src-tauri/src/sidecar.rs`

Current `Terminated` branch (line ~388):
```rust
CommandEvent::Terminated(status) => {
    tracing::warn!("[agentmuxsrv-rs] terminated with status: {:?}", status);
    if let Some(window) = app_handle.get_webview_window("main") {
        let _ = window.emit("backend-terminated", serde_json::json!({
            "code": status.code,
            "signal": status.signal,
        }));
    }
    break;
}
```

**Change:** Enrich the payload with uptime and PID, and upgrade log level to `error!`:
```rust
CommandEvent::Terminated(status) => {
    let state = app_handle.state::<crate::state::AppState>();
    let pid = state.backend_pid.lock().unwrap().unwrap_or(0);
    let started_at = state.backend_started_at.lock().unwrap().clone();
    let uptime_secs: Option<i64> = started_at.as_deref().and_then(|s| {
        chrono::DateTime::parse_from_rfc3339(s).ok().map(|t| {
            (chrono::Utc::now() - t.with_timezone(&chrono::Utc)).num_seconds()
        })
    });

    tracing::error!(
        "[agentmuxsrv-rs] TERMINATED — pid={} exit_code={:?} signal={:?} uptime_secs={:?}",
        pid, status.code, status.signal, uptime_secs
    );

    if let Some(window) = app_handle.get_webview_window("main") {
        let _ = window.emit("backend-terminated", serde_json::json!({
            "code": status.code,
            "signal": status.signal,
            "pid": pid,
            "uptime_secs": uptime_secs,
        }));
    }
    break;
}
```

**Why `error!` not `warn!`:** tracing level `error` is always flushed to disk immediately by `tracing-appender`'s non-blocking writer; `warn`/`info` may buffer. This maximises chance of capture before Tauri process exits.

---

### 2. New Tauri Command: `restart_backend`

**New file (or add to):** `src-tauri/src/commands/backend.rs`

```rust
#[tauri::command]
pub async fn restart_backend(app: tauri::AppHandle) -> Result<crate::sidecar::BackendSpawnResult, String> {
    tracing::info!("[restart_backend] user-requested backend restart");

    // Kill existing sidecar if still alive
    {
        let state = app.state::<crate::state::AppState>();
        let mut sidecar = state.sidecar_child.lock().unwrap();
        if let Some(child) = sidecar.take() {
            let _ = child.kill();
            tracing::info!("[restart_backend] killed stale sidecar");
        }
    }

    // Small delay to let the OS release the port
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Spawn fresh
    let result = crate::sidecar::spawn_backend(&app).await?;

    // Notify frontend of new endpoints
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.emit("backend-ready", serde_json::json!({
            "ws_endpoint": result.ws_endpoint,
            "web_endpoint": result.web_endpoint,
            "auth_key": result.auth_key,
            "instance_id": result.instance_id,
            "version": result.version,
        }));
    }

    Ok(result)
}
```

Register in `lib.rs` handler list:
```rust
.invoke_handler(tauri::generate_handler![
    // ... existing commands ...
    commands::backend::restart_backend,
])
```

---

### 3. Frontend — `BackendStatusState` + Richer Death Info

**File:** `frontend/app/store/global.ts`

Add a `BackendDeathInfo` type and store last death payload:

```typescript
type BackendStatusState = "connecting" | "running" | "crashed";

export interface BackendDeathInfo {
    code: number | null;
    signal: number | null;
    pid: number;
    uptime_secs: number | null;
    died_at: string; // ISO timestamp set by frontend at receipt
}

export const [backendDeathInfoAtom, setBackendDeathInfoAtom] = createSignal<BackendDeathInfo | null>(null);
```

Update the listener:
```typescript
getApi().listen("backend-terminated", (event) => {
    const payload = event.payload as Partial<BackendDeathInfo>;
    setBackendDeathInfoAtom({
        code: payload.code ?? null,
        signal: payload.signal ?? null,
        pid: payload.pid ?? 0,
        uptime_secs: payload.uptime_secs ?? null,
        died_at: new Date().toISOString(),
    });
    setBackendStatusAtom("crashed");
});
```

---

### 4. Frontend — `BackendStatus.tsx` Offline State

**When crashed, show:**
- Red dot (already done)
- "Offline" label (already done)
- Subtitle line in popover: exit code, signal, uptime at death, time of death
- **"Restart Backend" button** in the popover (calls `restart_backend` Tauri command, sets status back to `"connecting"`)

**Popover additions (crashed state):**
```tsx
<Show when={backendStatus() === "crashed" && backendDeathInfo() != null}>
    <div class="status-bar-popover-row">
        <span class="status-bar-popover-label">Died at</span>
        <span>{new Date(backendDeathInfo()!.died_at).toLocaleTimeString()}</span>
    </div>
    <Show when={backendDeathInfo()!.uptime_secs != null}>
        <div class="status-bar-popover-row">
            <span class="status-bar-popover-label">Was up</span>
            <span>{formatUptime(backendDeathInfo()!.uptime_secs!)}</span>
        </div>
    </Show>
    <Show when={backendDeathInfo()!.code != null}>
        <div class="status-bar-popover-row">
            <span class="status-bar-popover-label">Exit code</span>
            <span class="status-bar-popover-mono">{backendDeathInfo()!.code}</span>
        </div>
    </Show>
    <Show when={backendDeathInfo()!.signal != null}>
        <div class="status-bar-popover-row">
            <span class="status-bar-popover-label">Signal</span>
            <span class="status-bar-popover-mono">{backendDeathInfo()!.signal}</span>
        </div>
    </Show>
    <div class="status-bar-popover-row">
        <button class="status-bar-restart-btn" onClick={handleRestart}>
            Restart Backend
        </button>
    </div>
</Show>
```

`handleRestart`:
```typescript
const [restarting, setRestarting] = createSignal(false);

const handleRestart = async () => {
    setRestarting(true);
    setBackendStatusAtom("connecting");
    setPopoverOpen(false);
    try {
        await getApi().invoke("restart_backend");
        // backend-ready event will set status to "running"
    } catch (e) {
        console.error("[BackendStatus] restart failed:", e);
        setBackendStatusAtom("crashed");
    } finally {
        setRestarting(false);
    }
};
```

---

### 5. Frontend — `StatusBar.tsx` Version Link When Offline

**File:** `frontend/app/statusbar/StatusBar.tsx`

Current: version is always a clickable link that opens a new window.

**Change:** when backend is crashed, suppress the `onClick` and style it as non-interactive:

```tsx
<Show when={version}>
    <Show
        when={backendStatus() !== "crashed"}
        fallback={
            <span class="status-version status-version-offline" title="Backend offline">
                v{version}
            </span>
        }
    >
        <span
            class="status-version clickable"
            onClick={handleNewWindow}
            title="Open New AgentMux Window"
        >
            v{version}
            <Show when={windowCount() > 1}>
                <span class="instance-num"> ({instanceNum()})</span>
            </Show>
        </span>
    </Show>
</Show>
```

CSS addition in `StatusBar.scss`:
```scss
.status-version-offline {
    opacity: 0.4;
    cursor: default;
    pointer-events: none;
}
```

---

## Open Questions

1. **Auto-restart on first crash?** — Could restart automatically on first `Terminated` with code 0 or null signal (clean exit), but require user action for non-zero exit codes (crash). Avoids crash loops while recovering from accidental kills.

2. **State reconnect after restart** — The frontend's WS connection uses the original `ws_endpoint` from startup. After restart, the port may change. The `backend-ready` event payload needs to carry the new endpoint and the frontend's RPC client needs to reconnect. This is the hardest part — needs a `reconnectRpcClient(wsEndpoint, authKey)` call in the frontend store.

3. **Multiple windows** — If two AgentMux windows are open, only the `main` webview gets `backend-terminated` and `backend-ready`. Other windows need to be notified too. Consider broadcasting to all webview windows in the Tauri command.

4. **Minimum viable** — If full restart is too complex right now, ship just items 1 + 4 + 5 (better diagnostics + offline popover info + version link suppression) as a standalone PR. Restart can follow in a second PR.

---

## File Changelist

| File | Change |
|------|--------|
| `src-tauri/src/sidecar.rs` | Enrich `Terminated` payload: pid, uptime_secs; upgrade to `error!` |
| `src-tauri/src/commands/backend.rs` | New file — `restart_backend` command |
| `src-tauri/src/lib.rs` | Register `restart_backend` in invoke handler |
| `frontend/app/store/global.ts` | Add `BackendDeathInfo` type + atom; enrich `backend-terminated` listener |
| `frontend/app/statusbar/BackendStatus.tsx` | Offline popover: show death info + Restart button |
| `frontend/app/statusbar/StatusBar.tsx` | Suppress version link click when crashed |
| `frontend/app/statusbar/StatusBar.scss` | `.status-version-offline` style |

---

## Suggested PR Split

- **PR A — Diagnostics only:** sidecar.rs enrichment + global.ts death atom + BackendStatus popover death info (no restart button yet). Low risk, pure logging/display.
- **PR B — Restart + UX:** `restart_backend` command + Restart button + version link suppression. Needs careful testing of WS reconnect.
