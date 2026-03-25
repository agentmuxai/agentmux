# Retro: Why Backend Crash Recovery Was Never Shipped

**Date:** 2026-03-24
**Related:** `OFFLINE_CRASH_ANALYSIS.md`, PR #214, commit `15c9a1a`

---

## What happened

Across v0.32.73–v0.32.79 the backend sidecar (`agentmuxsrv-rs`) crashed multiple times per version, leaving users stuck on a permanent "Offline" state. The fix was known, specced, and partially built — but the second half was never shipped.

---

## The timeline

### March 3, 2026 — Initial commit (v0.31.20)

`sidecar.rs` had a minimal `Terminated` handler:
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
Exit code logged as `warn!`, no pid, no uptime. Frontend shows "Offline" with no context. No restart path.

### March 6, 2026 — WS idle watchdog removed (commit `15c9a1a`)

An idle watchdog that killed the backend when WebSocket client count hit 0 for 30 seconds was found to be too aggressive — WS connections drop briefly during tab switches, multi-window ops, React re-renders. The backend was killing itself unnecessarily.

**Commit message said:**
> "Updated spec with lessons learned and revised approach (heartbeat-based crash recovery instead of WS-based idle detection)."

`specs/backend-lifecycle.md` was updated with the heartbeat-based recovery plan. **The heartbeat-based recovery was never built.**

### March 23, 2026 — PR #214 merged (commit `63ef96b` / `ced3f87`)

`diag(backend): enrich backend-terminated event with pid, uptime, exit-code (v0.32.74)`

The `BACKEND_RESILIENCE_SPEC.md` was created and the diagnostics phase was shipped:
- `Terminated` handler upgraded from `warn!` → `error!` (flushes immediately)
- Payload enriched with `pid`, `uptime_secs`
- `BackendDeathInfo` atom in frontend
- Popover shows died-at, was-up, exit code, signal

The spec explicitly described two phases:

> **PR A — Diagnostics only:** sidecar.rs enrichment + global.ts death atom + BackendStatus popover death info (no restart button yet). Low risk, pure logging/display.
>
> **PR B — Restart + UX:** `restart_backend` command + Restart button + version link suppression. Needs careful testing of WS reconnect.

**PR A was shipped. PR B was never opened.**

### March 23–24, 2026 — Crashes continue across v0.32.73–v0.32.79

The diagnostics from PR #214 confirmed the crashes (exit code `-1073740791` = `0xC0000409` = Windows fast-fail/abort) but gave no insight into the root cause. Users remained stuck on "Offline" with no recovery path.

PR #222 fixed a FileStore cache OOM (merged before v0.32.79) but crashes persisted at shorter uptimes in v0.32.79.

---

## Why PR B never shipped

### 1. "Minimum viable" framing created a stopping point

The spec contained this exact line:
> "Minimum viable — If full restart is too complex right now, ship just items 1 + 4 + 5 (better diagnostics + offline popover info + version link suppression) as a standalone PR. Restart can follow in a second PR."

This was correct tactical advice, but "follow in a second PR" became a permanent deferral. Once PR A merged and the diagnostic info was visible in the popover, the urgency decreased — the problem *looked* addressed.

### 2. The diagnostics obscured the remaining gap

After PR #214, the Offline popover showed real data: pid, uptime, exit code. This felt like progress. But it addressed observability, not recovery. The user still had to close and reopen the entire app to recover. The improvement in *appearance* of the diagnostics reduced the felt pain without reducing the actual disruption.

### 3. The WS reconnect problem was underestimated

The spec flagged this as the hardest part:
> "After restart, the port may change. The `backend-ready` event payload needs to carry the new endpoint and the frontend's RPC client needs to reconnect. This is the hardest part — needs a `reconnectRpcClient(wsEndpoint, authKey)` call in the frontend store."

The `reconnectRpcClient` function doesn't exist. The frontend's RPC client was initialized once at startup with a fixed endpoint baked into the connection. A restart would need new endpoints and a full RPC client re-initialization. This complexity was real and acknowledged — and it became a reason to not start.

### 4. Root cause wasn't resolved alongside diagnostics

The spec noted:
> "Auto-restart on every crash without user action (risky: crash loop)"

This was listed under Non-Goals. Fair for `panic!()` crashes where a bug is looping. But the actual crash type (OOM/abort after 2–11h) is not a crash loop — it's a degradation that happens once after extended use. Auto-restart with a limit (e.g., once, with a 5-second delay) would have been safe and useful here.

### 5. The `0xC0000409` exit code gave a false sense of debuggability

The enriched `backend-terminated` event showed the exit code. It looked informative. But `0xC0000409` (Windows fast-fail) produces **zero stderr output** — Rust's panic hook doesn't run, no backtrace is written, the OS terminates directly. The diagnostics improvements from PR #214, while real, couldn't surface the actual crash callstack for the dominant crash type. The right tool (crash dumps/VEH) was never mentioned in the spec.

---

## What we should have done differently

| Decision | What we did | What would have been better |
|----------|-------------|----------------------------|
| PR split | Two-phase spec, only Phase 1 shipped | Ship Phase 1 + Phase 2 in the same week, not the same PR |
| Spec framing | "Minimum viable" with explicit deferral | "Phase 2 is the actual fix; Phase 1 is prerequisite" |
| Auto-restart scope | Listed as Non-Goal | Should have been in scope for degradation crashes (single restart, 5s delay, with loop protection) |
| Root cause | Investigated via process exit code | Should have also enabled WER local dumps to get the actual callstack |
| Heartbeat recovery | Specced in March 6 commit, never built | Should have been built; the idle watchdog removal left no active recovery mechanism |
| `reconnectRpcClient` complexity | Acknowledged and deferred | Should have been built as the prerequisite; it's the enabling piece for restart |

---

## Current state

- **Permanent "Offline" with no recovery** — user must restart the app
- **No crash dump collection** — 0xC0000409 crashes are completely opaque
- **`restart_backend` command** — specced in `BACKEND_RESILIENCE_SPEC.md`, not implemented
- **`reconnectRpcClient`** — not implemented
- **Heartbeat-based crash recovery** — specced in `specs/backend-lifecycle.md`, not implemented
- **`backendStatusAtom` init** — still `"running"`, should be `"connecting"`
- **Version link** — still active when offline (spec said to suppress it)

All of these are tracked in `BACKEND_RESILIENCE_SPEC.md` (Phase 2 items) and `OFFLINE_CRASH_ANALYSIS.md`.
