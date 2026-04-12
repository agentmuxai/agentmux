# v0.33.91 NDJSON Parse Crash — Analysis

**Date:** 2026-04-12
**Instance:** v0.33.91 running AgentX agent pane
**Symptom:** Frontend crash / unresponsive state during a Write tool call

## What happened

User was running v0.33.91 with an AgentX agent pane open. AgentX was asked to
write an ultra-long sessions plan. While AgentX was streaming the `Write` tool
call (a large markdown file with escaped JSON inside), the frontend logged:

```
[fe] Failed to parse NDJSON line: {"type":"tool_call","toolCallId":"toolu_01MTAWL7HhE15LJ2nqSxBzoq","toolName":"Write","args":"{\"file_path\":\"...\",\"content\":\"# Ultra-Long Session Support Plan\\n\\n...\"}",...
```

The content contained deeply-escaped JSON (JSON-in-JSON-in-string) with thousands
of `\\\"` sequences. The frontend NDJSON parser choked and the pane became
unresponsive.

## Root cause

In `frontend/app/view/agent/useAgentStream.ts:164`:

```typescript
try {
    rawEvent = JSON.parse(trimmed);
} catch {
    // Not valid JSON — skip
    continue;
}
```

The `try/catch` is correct — it should silently skip malformed lines. But the
line being skipped is a real tool call that the UI needs to render. Two problems:

1. **Silent drop** — the tool call event never reaches the translator/parser, so
   the user doesn't see the Write tool execution at all.
2. **Error log is huge** — the error log line itself is the entire tool call
   content (8KB+), which `log-pipe.ts` forwards to the Rust host as a single
   blocking IPC call. This stalls the main thread.

The NDJSON parse is actually failing because **the line is getting split mid-content**.
WebSocket messages have size limits; a 20KB tool_call event may arrive in chunks
but `useAgentStream.ts` treats each chunk as a complete line.

Looking at the stream reader (persistent.rs stdout reader):

```rust
let mut reader = BufReader::new(stdout).lines();
while let Some(line) = reader.next_line().await {
    // publishes line via handle_append_block_file
}
```

This uses tokio's `.lines()` which splits on `\n`. CLI stream-json output uses
newline delimited JSON — one event per line. So the backend IS emitting complete
lines. Which means the split is happening **between** the backend's append and
the frontend's receive.

The WPS blockfile event format:
```typescript
{ fileop: "append", data64: base64(...) }
```

The frontend in `useAgentStream.ts` accumulates partial data:

```typescript
lineBuffer += text;
const lines = lineBuffer.split("\n");
lineBuffer = lines.pop() || ""; // Keep incomplete line
```

This IS correct — it buffers incomplete lines across multiple append events.
But if a single event's `data64` is 100KB+, base64 decoding + string concatenation
on each append takes real time. During heavy streaming, the buffer can fall behind.

**Actual root cause:** The `JSON.parse` error log is so large (the failing line
is the entire tool call content) that forwarding it to the Rust host via the log
pipe causes a main thread stall. The UI appears crashed but is actually blocked
on the `fe_log_structured` IPC call.

## The fix

Two changes to `useAgentStream.ts`:

### 1. Don't log the full line on parse error

```typescript
try {
    rawEvent = JSON.parse(trimmed);
} catch (err) {
    // Could be a partial line mid-stream or genuinely malformed JSON.
    // Don't log the full line — it may be 100KB+ and will stall the main
    // thread forwarding it through the IPC log pipe.
    console.warn(`[useAgentStream] JSON parse failed, len=${trimmed.length}, first 80: ${trimmed.slice(0, 80)}`);
    continue;
}
```

### 2. Handle the case where the line is a complete partial — re-buffer it

Actually re-buffering won't work here because `split('\n')` already consumed the
line. The better fix is to make the line buffer size-limited: if `lineBuffer`
grows beyond 10MB without a newline, assume something is wrong and drop it.

### 3. Verify the backend is emitting complete lines

Add a length check in `persistent.rs`'s stdout reader:

```rust
if line.len() > 1_000_000 {
    tracing::warn!(block_id = %block_id, line_len = line.len(), "huge stdout line");
}
```

## Why it looked like a crash

The instance didn't actually crash — it was main-thread-stalled. The frontend's
console.error forwarding (from `log-pipe.ts`) synchronously sends the log line to
the Rust host over HTTP IPC. A single 100KB log entry takes ~50-200ms, and if
it happens on every retry attempt, the UI freezes.

## Impact

- Large tool calls (Write with long content, Bash with long stdout) could stall
  the agent pane
- The user sees "frozen" behavior with no error visible in the UI
- Recovery: close the pane or restart AgentMux

## Prevention

- [ ] Trim all log messages in `useAgentStream.ts` to <1KB
- [ ] Add size limit on `lineBuffer` (drop if >10MB without newline)
- [ ] Add `tracing::warn` in backend persistent.rs for lines >1MB
- [ ] Consider: async log forwarding in `log-pipe.ts` so large logs don't
      block the main thread even if we don't fix the underlying issue

## Related

- `frontend/log/log-pipe.ts` — synchronous `fe_log_structured` IPC is the
  amplifier. Making it async would mitigate many classes of issues.
- `useAgentStream.ts` already has the console.log debug that was disabled
  in the silky-smooth typing PR — make sure we're not accidentally re-enabling
  large log output.
