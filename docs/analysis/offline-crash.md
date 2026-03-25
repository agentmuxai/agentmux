# AgentMux "Offline" State — Root Cause Analysis & Spec

**Date:** 2026-03-24
**Versions affected:** v0.32.73 through v0.32.79 (all portable desktop builds)
**Symptom:** Status bar shows red dot + "Offline" text, app becomes unresponsive

---

## 1. What "Offline" Means (UI Data Flow)

```
agentmuxsrv-rs exits
  → CommandEvent::Terminated in sidecar.rs:388
    → window.emit("backend-terminated", {code, signal, pid, uptime_secs})  [sidecar.rs:402]
      → global.ts:235 listen("backend-terminated", ...)
        → setBackendDeathInfoAtom({...})
        → setBackendStatusAtom("crashed")                                  [global.ts:244]
          → BackendStatus.tsx:131 shows "Offline"                          [BackendStatus.tsx:131-133]
```

- **No auto-recovery.** Once `"crashed"`, the atom stays crashed forever. There is no reconnect loop, no restart trigger from the frontend, no watchdog timer.
- `backendStatusAtom` initializes as `"running"` (not `"connecting"`), so there's no "connecting" state before first connection.
- `backendDeathInfoAtom` stores: `code`, `signal`, `pid`, `uptime_secs`, `died_at` (ISO timestamp set client-side at receipt).

---

## 2. Crash Evidence (Log Analysis)

### 2.1 Confirmed Crash Events

| Version | Crash Time (UTC) | Exit Code | Uptime | Notes |
|---------|-----------------|-----------|--------|-------|
| v0.32.73 | 2026-03-23T05:21 | `1` | unknown | Type B – startup/DB failure |
| v0.32.73 | 2026-03-23T11:30 | `-1073740791` | unknown | Type A – abort |
| v0.32.75 | 2026-03-24T03:40 | `-1073740791` | unknown | Type A – abort |
| v0.32.76 | 2026-03-23T21:23:45 | `1` | 5314 s (~88 min) | Type B – deliberate exit |
| v0.32.77 | 2026-03-24T01:32:52 | `-1073740791` | 15002 s (~4.2 h) | Type A – abort |
| v0.32.77 | 2026-03-24T15:20:54 | `-1073740791` | 41922 s (~11.6 h) | Type A – abort, preceded by 22-min hang |
| v0.32.79 | 2026-03-24T18:53:15 | `-1073740791` | 9173 s (~2.5 h) | Type A – abort |
| v0.32.79 | 2026-03-24T23:36:59 | `1` | 35 s | Type B – possible corrupted restart |

### 2.2 Exit Code Meaning

**Exit code `-1073740791` = `0xC0000409` = `STATUS_STACK_BUFFER_OVERRUN` (Windows fast-fail)**

On Windows, this exit code is produced when:
- Rust's `panic!()` propagates to `std::rt::rust_panic_without_abort()` → `core::panicking::panic_fmt()` → abort
- `std::process::abort()` is called explicitly
- The Windows security cookie (`/GS`) detects stack corruption
- `__fastfail(FAST_FAIL_FATAL_APP_EXIT)` is invoked by any mechanism

In Rust, when a panic occurs in a non-`UnwindSafe` context (e.g., tokio thread pool), or when `panic = "abort"` is implicitly active (or the panic hook calls abort), the result on Windows is always `0xC0000409`.

**Exit code `1`**

Deliberate `std::process::exit(1)` call. In `agentmuxsrv-rs/src/main.rs`, these occur for:
- Config load failure (line 234)
- Data dir creation failure (lines 260, 264)
- DB open failure (lines 280, 284, 290)
- Any other fatal startup error

### 2.3 Pre-Crash Patterns (Sidecar Logs)

**v0.32.77 — 11.6-hour session crash (Type A)**
- Sidecar log last entry: `2026-03-24T14:58:08` (22 minutes before crash at 15:20:54)
- Prior to silence: `UpdateObjectMeta` latency degrading: `0.17ms → 0.51ms → 0.78ms → 0.66ms`
- 22-minute gap with zero log output → process was alive but frozen (hang, deadlock, or OOM thrashing)
- Crash is abrupt with no final log entry

**v0.32.76 — 88-minute session crash (Type B)**
- Sidecar log last entry: `2026-03-23T21:23:45.016222`
- Host log crash: `2026-03-23T21:23:45.829540` (same second — immediate exit after last log entry)
- No error logged before exit
- Pattern: `UpdateObjectMeta` calls with 0.77ms spike just before, then immediate exit

**v0.32.79 — 35-second session crash (Type B)**
- Started at 23:36:24, crashed at 23:36:59
- Sidecar log shows: WebSocket connect, ControllerResync, PTY opened (pwsh), then heavy `controllerinput` storm
- Last log entry: 23:36:45.513 (14 seconds before crash)
- Exit code 1 — likely a startup-path failure triggered by DB or state corruption from the prior crash (18:53, Type A)
- No error message in log

---

## 3. Root Causes

### Root Cause A: Rust Panic / Abort in Long-Running Sessions (Primary)

The dominant crash type (0xC0000409) affects sessions of 2.5–11.6 hours. Evidence points to memory exhaustion or an unhandled error in a Tokio async task:

**Leading hypothesis: OOM / memory leak in FileStore or in-memory caches**
- v0.32.77 shows a 22-minute pre-crash hang consistent with memory pressure (allocator thrashing, swap)
- `UpdateObjectMeta` latency degrades before crash (0.17ms → 0.78ms) — consistent with lock contention under memory pressure
- PR #222 (v0.32.78) fixed a FileStore Cache OOM bug. v0.32.77 is pre-fix, v0.32.79 is post-fix.
- v0.32.79 still crashes at 2.5h with 0xC0000409 → either the fix was incomplete or there is a second leak path

**Secondary hypothesis: Unhandled panic in Tokio thread**
- `panic!()` in AI stream handlers (`openai.rs:339`, `anthropic.rs:358`) with `"expected error event"` — if these are hit in an uncaught context they can abort the process
- `catch_panic()` in `panichandler.rs` uses `catch_unwind` — but only covers closures wrapped with it, not all async tasks

### Root Cause B: Deliberate `process::exit(1)` on Startup (Secondary)

Exit code 1 crashes at startup (35s, etc.) are hitting a fatal startup path in `main.rs`. The most likely trigger is a corrupted or locked DB file after a prior Type A crash:
- Type A crash aborts without proper DB shutdown → SQLite WAL may be in inconsistent state
- Next launch fails to open `wave.db` or `filestore.db` → `process::exit(1)`
- v0.32.79's 35s crash (exit 1) immediately followed the 2.5h Type A crash — fits this pattern exactly

---

## 4. Observability Gaps

### Gap 1: abort() produces no stderr — stderr capture is insufficient for 0xC0000409

`CommandEvent::Stderr` IS handled in `sidecar.rs:346`. Non-ESTART lines are forwarded to the host log as `[agentmuxsrv-rs] {l}`. This is correct.

**However, all 0xC0000409 crashes use `__fastfail` (Windows fast-fail/abort).** This bypasses the Rust panic hook entirely — no backtrace is written to stderr, no output of any kind is produced. The OS terminates the process directly. There is literally nothing for the Stderr arm to capture.

Only a `panic!()` with unwind strategy (exit code 101) would produce a traceable stderr backtrace. The dominant crash type (abort = 0xC0000409) is invisible to any stderr-based approach.

**What would actually help:** Windows Error Reporting (WER) crash dumps, or a Vectored Exception Handler (VEH) registered in the sidecar to capture the structured exception callstack before `__fastfail` kills the process.

### Gap 2: No auto-restart on backend death

The frontend has no watchdog. After `"backend-terminated"`:
- UI shows "Offline" permanently
- No retry, no reconnect, no user prompt to restart
- User must manually close and reopen the app

### Gap 3: `backendStatusAtom` initializes as `"running"`

```typescript
// global.ts:102
export const [backendStatusAtom, setBackendStatusAtom] = createSignal<BackendStatusState>("running");
```

The initial state is `"running"`, not `"connecting"`. The `"backend-ready"` event sets it to `"running"` on first connection. If the backend dies before the frontend fully loads, the UI may show a confusing state (green dot that immediately becomes "Offline").

### Gap 4: No post-crash DB integrity check

After a Type A (abort) crash, the DB files may be corrupted. There is no pre-launch integrity check or WAL recovery step. This causes the next launch to fail with exit code 1 (Type B chained failure).

---

## 5. Affected Code Locations

| File | Line(s) | Issue |
|------|---------|-------|
| `src-tauri/src/sidecar.rs` | 388–413 | `CommandEvent::Stderr` not handled → panic messages lost |
| `src-tauri/src/sidecar.rs` | 401–408 | `backend-terminated` emitted but no restart triggered |
| `frontend/app/store/global.ts` | 102 | Initial state `"running"` should be `"connecting"` |
| `frontend/app/store/global.ts` | 235–247 | No retry/restart on `"backend-terminated"` |
| `agentmuxsrv-rs/src/main.rs` | 232–291 | `process::exit(1)` paths with no DB recovery attempt |
| `agentmuxsrv-rs/src/backend/ai/openai.rs` | 339 | `panic!("expected error event")` in stream handler |
| `agentmuxsrv-rs/src/backend/ai/anthropic.rs` | 358 | `panic!("expected error event")` in stream handler |

---

## 6. Recommended Fixes (Priority Order)

### P0 — Windows crash dump collection in the sidecar

`CommandEvent::Stderr` is already captured. The problem is that `0xC0000409` (fast-fail/abort) produces NO stderr output — it bypasses all hooks. The only mechanism that can capture the callstack for these crashes is a Windows Vectored Exception Handler (VEH) or WER crash dumps.

Immediate workaround: enable WER local dumps for a diagnosis session.
Real fix: integrate `minidump-writer` or a VEH in `agentmuxsrv-rs` to write `~/.agentmux/crashdumps/*.dmp` before the process exits.

### P1 — Fix initial backendStatusAtom state

```typescript
// global.ts:102
export const [backendStatusAtom, setBackendStatusAtom] = createSignal<BackendStatusState>("connecting");
```

Set to `"connecting"` on init; `"backend-ready"` event sets it to `"running"`.

### P2 — Add backend restart / reconnect mechanism

On `"backend-terminated"`, instead of permanent "Offline":
1. Show "Offline" immediately (current behavior ✓)
2. After 2s delay, call a Rust command to respawn the backend sidecar
3. If respawn succeeds, emit `"backend-ready"` → frontend transitions back to `"running"`
4. Limit to N restart attempts (e.g., 3) before giving up

### P3 — DB integrity check on startup

In `main.rs`, before opening `wave.db` and `filestore.db`:
1. Run `PRAGMA integrity_check` on SQLite
2. If corrupt, rename the DB (backup) and start fresh rather than `process::exit(1)`

### P4 — Replace stream panic! with error returns

```rust
// openai.rs:339 and anthropic.rs:358
// Replace:
_ => panic!("expected error event"),
// With:
_ => return Err(anyhow::anyhow!("unexpected event type in error stream")),
```

### P5 — Memory leak investigation

Profile memory usage over 2+ hours with:
```bash
# In a long-running session, poll RSS periodically
while true; do
  ps -o pid,rss,vsz -p $(pgrep agentmuxsrv-rs) 2>/dev/null
  sleep 60
done >> /tmp/agentmux-rss.log
```

Focus on `FileStore`, `WaveStore`, and the `UpdateObjectMeta` call path — this is the most active path visible in sidecar logs and shows latency degradation before crashes.

### Gap 5: WebSocket reconnect is orthogonal to backend process death

`frontend/app/store/ws.ts` has a WS reconnect loop (exponential backoff `[0,0,2,5,10,10,30,60]s`, max 20 retries). This handles transient network blips. However, when the backend **process** dies, there is nothing to reconnect to. The `"backend-terminated"` Tauri event fires in parallel, permanently setting the atom to `"crashed"` — so the WS reconnect loop becomes moot. The two paths (WS drop vs. process death) are not coordinated.

### Gap 6: [`BACKEND_RESILIENCE_SPEC`](../specs/backend-resilience.md) exists but is unimplemented

[`BACKEND_RESILIENCE_SPEC`](../specs/backend-resilience.md) is a draft spec for:
- `"connecting"` as an initial state (currently: `"running"` is the initial)
- A "Restart Backend" button in the popover
- A `restart_backend` Tauri command

None of this is implemented. The spec was written but the implementation did not follow.

---

## 7. Summary

| # | Finding | Severity |
|---|---------|----------|
| 1 | Sidecar stderr not captured — panic stack traces are lost | Critical |
| 2 | No auto-restart on backend death — permanent "Offline" | High |
| 3 | 0xC0000409 crashes in long sessions (2–12h) = Rust abort | High |
| 4 | Pre-crash UpdateObjectMeta latency spike (4-5×) before hang | High |
| 5 | 22-min pre-crash hang in v0.32.77 → likely OOM/deadlock | High |
| 6 | Exit code 1 at 35s (v0.32.79) = chained failure after prior abort | Medium |
| 7 | `panic!()` calls in AI stream handlers — unconditional abort risk | Medium |
| 8 | `backendStatusAtom` starts `"running"` not `"connecting"` | Low |
