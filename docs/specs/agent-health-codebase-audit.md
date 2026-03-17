# Agent Health/Liveness Detection — Codebase Audit

> **Date:** 2026-03-17
> **Scope:** How AgentMux currently detects agent health, and what's missing.

---

## 1. Subprocess Controller Lifecycle

**File:** `agentmuxsrv-rs/src/backend/blockcontroller/subprocess.rs`

The SubprocessController manages agent CLI processes as **stateless, per-turn subprocess invocations**:

- **State Machine:** `INIT` → (spawn) → `RUNNING` → (process exits) → `DONE` → (new message) → `RUNNING`
- **Per-Turn Model:** Each user message spawns a fresh `claude -p` process with `--resume <session-id>` for continuity
- **Core Fields (SubprocessControllerInner):**
  - `proc_status: String` — Current status (init/running/done)
  - `proc_exit_code: i32` — Exit code from last subprocess
  - `status_version: i32` — Version counter (incremented on each status change)
  - `session_id: Option<String>` — Session ID captured from `system/init` message
  - `current_pid: Option<u32>` — PID of currently running subprocess
  - `kill_tx: Option<tokio::sync::oneshot::Sender<bool>>` — Channel to signal process termination

**Key Methods:**
- `spawn_turn(config)` (Lines 172-454) — Main entry point; spawns subprocess, writes message to stdin, reads NDJSON from stdout
- `stop_subprocess(force)` (Lines 457-469) — Graceful (SIGTERM) or forceful (kill) termination
- `publish_status()` (Lines 154-159) — Publishes BlockControllerRuntimeStatus via WPS broker

**I/O Model (3 concurrent async tasks):**
1. **stdin_writer** (Lines 240-249): Writes user message JSON, closes stdin for EOF
2. **stdout_reader** (Lines 252-339): Reads NDJSON lines from stdout, extracts `session_id` from `system/init`, publishes each line as WPS blockfile event
3. **stderr_reader** (Lines 341-355): Logs stderr to tracing (debug level)
4. **process_waiter** (Lines 362-451): Waits for process exit OR kill signal, captures exit code, updates status to DONE

---

## 2. BlockControllerRuntimeStatus

**File:** `agentmuxsrv-rs/src/backend/blockcontroller/mod.rs` (Lines 116-130)

```rust
pub struct BlockControllerRuntimeStatus {
    pub blockid: String,
    pub version: i32,
    pub shellprocstatus: String,     // "init" | "running" | "done"
    pub shellprocconnname: String,
    pub shellprocexitcode: i32,
}
```

**Status Constants** (mod.rs Lines 30-34):
- `STATUS_INIT = "init"` — Process not started
- `STATUS_RUNNING = "running"` — Process spawned, awaiting exit
- `STATUS_DONE = "done"` — Process exited (check exit code for success/failure)

**Exit codes:** `0` = success, `-1` = spawn/termination error, `>0` = CLI error

---

## 3. Stream Event Handling (Frontend)

### Stream Subscription
**File:** `frontend/app/view/agent/useAgentStream.ts`

1. Subscribes to file subject via `getFileSubject(blockId, "output")`
2. Decodes base64 WPS blockfile events into UTF-8 text
3. Accumulates lines until newline boundary
4. For each complete line: attempts JSON parse, silently skips non-JSON
5. Filters benign stderr warnings ("Fast mode is not available", etc.)

### Provider Translation
**File:** `frontend/app/view/agent/providers/claude-translator.ts`

- Translates Claude `stream_event` wrapper → StreamEvent format
- Handles Anthropic API events: `message_start`, `content_block_start`, `content_block_delta`, `content_block_stop`
- Extracts `is_error` flag from tool_result blocks → `status: "failed" | "success"`

### Stream Parser
**File:** `frontend/app/view/agent/stream-parser.ts`

- Event types: `text`, `thinking`, `tool_call`, `tool_result`, `agent_message`, `user_message`
- **No explicit error event type** — errors only appear in tool_result nodes with `status: "failed"`

---

## 4. Current Error Detection

| What | Where | How |
|------|-------|-----|
| **Spawn failure** | subprocess.rs:208-214 | Exit code = `-1` |
| **Wait failure** | subprocess.rs:366-376 | Exit code = `-1` |
| **JSON parse error** | useAgentStream.ts:93-98 | Silently skipped |
| **Tool execution error** | claude-translator.ts:146-158 | `is_error: true` → `status: "failed"` |
| **Auth failure** | agent-view.tsx:199-216 | `CheckCliAuthCommand` RPC, 30s timeout |
| **CLI not found** | agent-view.tsx:165-190 | `ResolveCliCommand` RPC, 120s timeout |

---

## 5. Critical Gaps — What's NOT Detected

1. **Hung processes** — No timeout on subprocess execution. If `claude -p` hangs, status stays `running` indefinitely. No heartbeat/keep-alive.

2. **API errors (400/429/500)** — No pattern matching on error event types. Cannot distinguish "API rate-limited" from "normal response". Status goes `running` → `done` regardless.

3. **Silent crashes** — If CLI crashes without stdout output, no output captured. Stderr is logged to tracing, not shown to user.

4. **Partial/malformed output** — Incomplete JSON lines silently dropped. No accumulation, no warning. No fallback if stream truncated mid-response.

5. **Session resumption failures** — Session ID extracted and persisted, but no validation that `--resume` succeeded. Session expiry is invisible.

6. **Memory/resource exhaustion** — No monitoring of subprocess memory. No detection of OOM kills. Exit code `-1` is opaque.

7. **Functional deadness** — Agent outputting valid JSON but semantically broken (e.g., gibberish, repeated errors). Only tool_result `is_error` is surfaced; if agent never calls tools, nothing detected.

---

## 6. Status Flow Architecture

```
Backend:
  SubprocessController.process_waiter
    → Updates proc_status → "done"
    → publish_controller_status(BlockControllerRuntimeStatus)
    → WPS Broker → WebSocket

Frontend:
  waveEventSubscribe(eventType="controllerstatus", scope="block:<blockid>")
    → agent-view.tsx:276-292
    → If "running": log "spawned, waiting..."
    → If "done" + exitCode != 0: log error
    → If "done" + exitCode == 0: log "turn complete"

Output:
  SubprocessController.stdout_reader
    → WPS blockfile event("append", data64)
    → useAgentStream → getFileSubject(blockId, "output")
    → decode base64 → parse JSON → translate → DocumentNode
    → SolidJS signal → AgentDocumentView re-renders
```
