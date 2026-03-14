# Spec: Frontend Log Pipe

**Goal:** Route `console.log/warn/error` from the frontend (WebView) to a log file on disk, so backend tooling and developers can inspect UI behavior without opening DevTools.

**Status:** Implemented.

---

## Problem

Debugging UI bugs (e.g., focus border not tracking, reactivity failures) requires opening DevTools in the running instance and manually reproducing the issue. This is especially painful when:
- The bug is intermittent or timing-sensitive
- The instance is a production portable build (no DevTools habit)
- Another agent/developer needs to inspect logs from a running instance they don't control

---

## Design

### Architecture

```
console.log("x")
       │
       ▼
 monkey-patch layer (frontend/log/log-pipe.ts, runs once at startup)
       │
       ├──► original console.log("x")   (DevTools still works)
       │
       └──► Tauri invoke("fe_log_structured", {level, module, message, data})
                    │
                    ▼
             Existing Rust command (src-tauri/src/commands/backend.rs)
                    │
                    ▼
             tracing::info/warn/error → ~/.agentmux/logs/agentmux-host-v*.log
```

Single direction. No acknowledgment, no batching. Fire-and-forget Tauri invoke.
Uses the **existing** `fe_log_structured` command — no new Rust code needed.

### Frontend: `frontend/log/log-pipe.ts` (NEW)

Monkey-patches `console.log/warn/error/debug/info`. Each call:
1. Forwards to the original console method (DevTools unaffected)
2. Serializes args to a string
3. Fires `invoke("fe_log_structured", ...)` — catch-and-swallow on failure

Called from `frontend/tauri-bootstrap.ts` at the very top, before any other imports run.

### Log File Location

Logs go to the existing host log at `~/.agentmux/logs/agentmux-host-v<VERSION>.log`.

Frontend messages are tagged with `module: "console"` and prefixed `[fe]` by the Rust handler:
```
{"timestamp":"...","level":"INFO","module":"console","message":"[fe] focus changed to block abc-123"}
```

To tail:
```bash
tail -f ~/.agentmux/logs/agentmux-host-v*.log | grep '\[fe\]'
```

---

## File Changes

| File | Change |
|------|--------|
| `frontend/log/log-pipe.ts` (NEW) | Console monkey-patch, fires Tauri invoke |
| `frontend/tauri-bootstrap.ts` | Import + call `initLogPipe()` at startup |

No Rust changes — reuses existing `fe_log_structured` command.

---

## What This Enables

1. `tail -f ~/.agentmux/logs/agentmux-host-v*.log | grep '\[fe\]'` while reproducing a bug
2. Post-mortem: user shares the log file from `~/.agentmux/logs/`
3. Add targeted `console.log` statements to debug focus/reactivity issues, read output without DevTools
4. All frontend logs co-located with backend logs in a single timeline

---

## Out of Scope

- Separate frontend.log file (unnecessary — unified log is better)
- Structured/queryable logs (overkill for a dev tool)
- Log levels / filtering at the backend (just grep the file)
- Shipping logs to a remote service
