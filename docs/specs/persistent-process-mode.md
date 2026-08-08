# Persistent Process Mode for Agent Pane

**Status:** Proposed (implemented — see note below)
**Date:** 2026-04-09

> **2026-08-07 audit note:** Implemented — this became the persistent
> controller (`agentmux-srv/src/backend/blockcontroller/persistent.rs`),
> foundational to how the app runs today (referenced throughout `CLAUDE.md`).
> Badly stale status for long-shipped, load-bearing code. See
> `docs/reports/REPORT_DOCS_AND_DEAD_CODE_CLEANUP_AUDIT_2026_08_07.md`.

## Summary

Replace the per-turn subprocess model with a persistent long-running CLI
process that accepts messages via stdin streaming. This enables sending
new input while the agent is still processing — the key UX gap between
the AgentMux agent pane and the native Claude Code CLI.

## Background

### Current Architecture (per-turn subprocess)

```
Turn 1: spawn claude -p --output-format stream-json "<prompt>"
        → capture session_id → stream output → process exits

Turn 2: spawn claude -p --output-format stream-json --resume <sid> "<prompt>"
        → stream output → process exits
```

Each turn spawns a **fresh process**. Stdin is written once then closed.
No way to send input mid-turn. Designed this way because of
anthropics/claude-code#3187 (`--input-format stream-json` hung on
Windows via WSL/.NET).

### Why Revisit

- Issue #3187 is **closed** (filed against CLI v1.44, closed July 2025).
  Someone confirmed it works. The bug was specific to WSL-via-cmd in
  .NET — not Rust's `Command` API which handles Windows handle
  inheritance correctly.
- Users can't redirect agents mid-task — a core AgentMux value prop.
- Per-turn respawn adds latency: process startup + session reload per
  message.
- The CLI now has mature `--input-format stream-json` support.

## Proposed Architecture

### Persistent process with bidirectional NDJSON

```
Spawn once:
  claude --input-format stream-json \
         --output-format stream-json \
         --verbose \
         --include-partial-messages \
         --dangerously-skip-permissions

Stdin (write NDJSON lines, keep open):
  {"type":"user","message":{"role":"user","content":"fix the bug in auth.ts"}}
  {"type":"user","message":{"role":"user","content":"actually, use a different approach"}}

Stdout (read NDJSON lines):
  {"type":"system","subtype":"init","session_id":"...","version":"..."}
  {"type":"assistant","message":{"role":"assistant","content":[...]}}
  {"type":"result","cost_usd":0.03,"duration_ms":4200,...}
  ... (next turn's events follow after next stdin message)
```

### State Machine

```
INIT ─(first message)─> SPAWNING ─(process alive)─> IDLE
IDLE ─(user message)─> STREAMING ─(result event)─> IDLE
STREAMING ─(user message)─> INTERRUPTED ─(new result)─> IDLE
IDLE ─(kill)─> DONE
any ─(process crash)─> CRASHED ─(auto-restart or user action)─> INIT
```

Key new state: **STREAMING + user input = INTERRUPTED**. The CLI receives
the new message while still processing and handles the interruption.

### Stdin Protocol

Based on Claude CLI `--input-format stream-json` docs, each stdin line is
a JSON object. The message format for a user turn:

```json
{"type":"user","message":{"role":"user","content":"your message here"}}
```

The process does NOT need to be restarted between turns. The session ID
is captured once from the first `system/init` event and remains valid
for the lifetime of the process.

## Implementation Plan

### Phase 1: PersistentSubprocessController (new controller)

Create a new controller alongside the existing `SubprocessController`
(don't modify it — keep it as fallback).

**New file:** `agentmux-srv/src/backend/blockcontroller/persistent.rs`

Key differences from `subprocess.rs`:

| Aspect | SubprocessController | PersistentSubprocessController |
|--------|---------------------|-------------------------------|
| Process lifetime | Per-turn | Per-session (minutes to hours) |
| Stdin | Write once, close | Keep open, write NDJSON lines |
| Resume | `--resume <sid>` flag | Not needed (same process) |
| CLI flags | `-p` (print mode) | `--input-format stream-json` |
| Mid-turn input | Impossible | Write new NDJSON line to stdin |
| Session ID | Captured, used on respawn | Captured once, informational |

**Inner state additions:**
```rust
struct PersistentInner {
    proc_status: String,
    session_id: Option<String>,
    current_pid: Option<u32>,
    kill_tx: Option<oneshot::Sender<bool>>,
    // NEW: handle to write to the running process's stdin
    stdin_tx: Option<mpsc::Sender<String>>,
    // NEW: track whether a turn is in progress
    turn_active: bool,
}
```

**Stdin writer task:**
```rust
// Long-lived task that writes messages to stdin
async fn stdin_writer(
    mut rx: mpsc::Receiver<String>,
    mut stdin: tokio::process::ChildStdin,
) {
    while let Some(msg) = rx.recv().await {
        if let Err(e) = stdin.write_all(msg.as_bytes()).await { break; }
        if let Err(e) = stdin.write_all(b"\n").await { break; }
        if let Err(e) = stdin.flush().await { break; }
    }
    // Channel closed or write error → stdin drops → process gets EOF
}
```

**send_message (replaces spawn_turn):**
```rust
fn send_message(&self, message: String) -> Result<(), String> {
    let inner = self.inner.lock().unwrap();
    let tx = inner.stdin_tx.as_ref()
        .ok_or("process not running")?;
    // Format as stream-json user message
    let json_msg = serde_json::json!({
        "type": "user",
        "message": {
            "role": "user",
            "content": message
        }
    });
    tx.try_send(json_msg.to_string())
        .map_err(|e| format!("stdin send failed: {e}"))
}
```

### Phase 2: WebSocket handler update

Update `websocket.rs` `agentinput` handler to detect controller type
and call `send_message()` instead of `spawn_turn()`:

```rust
// Try persistent controller first
if let Some(persistent) = ctrl.as_any()
    .downcast_ref::<PersistentSubprocessController>()
{
    persistent.send_message(cmd.message)?;
} else if let Some(subprocess) = ctrl.as_any()
    .downcast_ref::<SubprocessController>()
{
    subprocess.spawn_turn(config)?;
}
```

### Phase 3: Frontend — send while streaming

Update `AgentFooter` to allow sending messages while the agent is
streaming (currently the send button may be disabled during streaming).
The `handleSendMessage` in `agent-view.tsx` doesn't need to change —
it already calls `AgentInputCommand` which routes to the backend.

### Phase 4: Runtime args update

The `buildRuntimeArgs` feature (PR #322) currently writes updated
`cmd:args` before each turn. With a persistent process, CLI flags are
set at spawn time and can't change mid-session. Options:

- **Option A:** Apply runtime args only when spawning (not per-turn).
  Changing permission mode/model requires restarting the process.
- **Option B:** Use stdin control messages if the CLI supports them
  (e.g., `{"type":"config","permission_mode":"plan"}`). Unlikely to
  be supported currently.
- **Option C:** Keep the per-turn subprocess as fallback. If the user
  changes runtime settings, kill the persistent process and respawn
  with new flags. Show a brief "restarting with new settings..." message.

**Recommendation:** Option C — restart on settings change. It's rare
that users change settings mid-session, and the restart is fast (~500ms).

## CLI Flags Comparison

| Flag | Per-turn (current) | Persistent (proposed) |
|------|-------------------|----------------------|
| `-p` | Yes (print mode) | No (not needed) |
| `--input-format stream-json` | No | **Yes** |
| `--output-format stream-json` | Yes | Yes |
| `--verbose` | Yes | Yes |
| `--include-partial-messages` | Yes | Yes |
| `--dangerously-skip-permissions` | Yes | Yes |
| `--resume <sid>` | Yes (turn 2+) | No (same process) |

## Risks and Mitigations

### 1. Stdin hang on Windows (original #3187 concern)

**Mitigation:** AgentMux uses Rust's `tokio::process::Command` which
handles Windows handle inheritance correctly (no WSL, no .NET). The
original bug was specific to WSL-via-cmd in .NET.

**Testing:** Before shipping, verify:
- Send 5+ consecutive messages via stdin NDJSON on Windows
- Send a message while a previous turn is still streaming
- Process doesn't hang after any message

### 2. Process crash mid-session

**Mitigation:** Monitor process exit. If the process dies unexpectedly,
set status to CRASHED and either auto-respawn or show a "reconnect"
button. The session ID is captured, so a new process can `--resume` it.

### 3. Memory growth in long-running process

**Mitigation:** Monitor process memory via PID. If it exceeds a
threshold (e.g., 2GB), warn the user or auto-restart with `--resume`.

### 4. Backward compatibility

**Mitigation:** Keep `SubprocessController` as-is. Add
`PersistentSubprocessController` as a new controller type. Toggle via
block metadata `controller: "persistent"` vs `controller: "subprocess"`.
Default to persistent for Claude, keep subprocess for Codex/Gemini
until their stdin streaming support is verified.

## Files to Create

| File | Purpose |
|------|---------|
| `agentmux-srv/src/backend/blockcontroller/persistent.rs` | New controller |

## Files to Modify

| File | Change |
|------|--------|
| `agentmux-srv/src/backend/blockcontroller/mod.rs` | Register new controller type |
| `agentmux-srv/src/server/websocket.rs` | Route agentinput to send_message() |
| `frontend/app/view/agent/agent-model.ts` | Set controller type, update launchArgs |
| `frontend/app/view/agent/providers/index.ts` | Add persistent-mode launchArgs |
| `frontend/app/view/agent/components/AgentFooter.tsx` | Allow send during streaming |

## Testing

1. Launch agent pane → persistent process spawns
2. Send message → output streams, turn completes, process stays alive
3. Send second message → no new process spawn, output streams
4. Send message **while previous turn is streaming** → agent receives
   interrupt, responds to new message
5. Kill process → status shows CRASHED, restart button works
6. Change permission mode → process restarts with new flags
7. Close pane → process is killed cleanly
8. 10+ consecutive turns → no stdin hang, no memory leak

## Migration Path

1. Ship as opt-in (`controller: "persistent"`) behind a setting
2. Test with real users for 1-2 weeks
3. If stable, make it the default for Claude provider
4. Keep `SubprocessController` for Codex/Gemini and as fallback
