# Agent Stream Pipeline — Full Analysis

**Date:** 2026-03-16
**Version:** 0.32.8
**Bug:** User sends message, Claude CLI runs and exits code 0, but no response text appears in UI.

---

## Data Flow (Backend → Frontend)

```
1. AgentInputCommand RPC
   → subprocess.rs: spawn_turn() spawns `claude -p --verbose --output-format stream-json`
   → stdin: write user message + \n + close

2. subprocess.rs: stdout reader task (tokio)
   → BufReader::lines() reads NDJSON from subprocess stdout
   → Each line: handle_append_block_file(broker, blockId, "output", line_bytes)

3. shell.rs: handle_append_block_file()
   → Creates WaveEvent { event: "blockfile", scopes: ["block:{blockId}"] }
   → broker.publish(event)

4. WPS Broker → WebSocket → Frontend
   → Event forwarded to all "blockfile" subscribers (frontend registered with allscopes:true)

5. global.ts: blockfile handler
   → Extracts WSFileEventData { zoneid, filename, fileop, data64 }
   → getFileSubject(zoneid, filename).next(fileData)

6. useAgentStream.ts: subscription
   → getFileSubject(blockId, "output").subscribe(...)
   → Decodes base64, splits lines, parses JSON
   → translator.translate(rawEvent) → StreamEvent[]
   → parser.parseLine(event) → DocumentNode[]
   → setDocument(prev => [...prev, ...newNodes])

7. AgentDocumentView.tsx: renders document signal
   → <For each={document()}> → DocumentNodeRenderer
```

---

## Evidence from Backend Logs

### Session at 16:12 (latest with new code)

| Time | Event |
|------|-------|
| 16:12:11.209 | ResolveCli: found claude.exe in PATH |
| 16:12:11.517 | SetMeta cmd=C:\Users\area54\.local\bin\claude.exe |
| 16:12:11.520 | CheckCliAuth started |
| 16:12:16.536 | CheckCliAuth completed (5 sec) |
| 16:12:16.538 | ControllerResync → subprocess controller registered |
| 16:12:21.295 | AgentInput received |
| 16:12:21.295 | working_dir expanded: ~/.claw/... → C:\Users\area54/.claw/... |
| 16:12:21.301 | Subprocess spawned PID=62476, args=[-p, --output-format, stream-json, --verbose] |
| 16:12:22.908 | Captured session_id from system/init |
| 16:12:26.343 | Subprocess exited code=0 |

**Key observation:** Between system/init (16:12:22) and exit (16:12:26), there's a 3.4 second window where Claude should be emitting stream_event lines on stdout. The backend logs `captured session_id` because that's an explicit tracing::info!. But there's NO log of individual stdout lines being published.

---

## Identified Issues

### Issue 1: STDOUT LINES NOT LOGGED
The stdout reader in subprocess.rs reads lines and calls `handle_append_block_file()` but does NOT log the lines at INFO level. We can't see from logs whether the backend is receiving stdout data.

**Action:** Add `tracing::info!` or `tracing::debug!` to the stdout reader loop.

### Issue 2: MISSING `--include-partial-messages` FLAG
From memory: "Correct flags: `claude -p --verbose --output-format stream-json --include-partial-messages`"

Current args: `["-p", "--output-format", "stream-json", "--verbose"]`

Missing: `--include-partial-messages`

This flag might be needed for the CLI to emit incremental stream events during the response. Without it, the CLI may batch all content into the final `assistant` message event, which the translator currently DISCARDS (handleAssistantMessage returns [] because "The assistant message duplicates content from stream_events").

**This is likely the root cause.** Without `--include-partial-messages`:
- The CLI emits system/init, then a single complete assistant message, then result
- The translator receives the `assistant` event but discards it (line 127: `return []`)
- The `result` event is not handled (falls through to "Unknown format - discard")
- User sees nothing

### Issue 3: TRANSLATOR DISCARDS TOP-LEVEL EVENTS
The Claude CLI emits these top-level event types:
- `system` (init, version info) → NOT handled, discarded
- `assistant` (complete message) → DISCARDED by design (line 127)
- `user` (tool results) → handled
- `result` (final stats) → NOT handled, discarded
- `stream_event` (incremental) → handled → only emitted WITH `--include-partial-messages`

Without partial messages, the only substantive events are `assistant` and `result`, both of which are discarded or unhandled.

### Issue 4: STDERR NOW FORWARDED (fixed this session)
Previously stderr was logged at debug level only. Now it's published to frontend as `{"type":"stderr","text":"..."}` events. But this doesn't help with the main issue.

---

## Fix Plan

### Fix A: Add `--include-partial-messages` to provider args
**File:** `frontend/app/view/agent/providers/index.ts`
```typescript
styledArgs: ["--output-format", "stream-json", "--verbose", "--include-partial-messages"],
```

### Fix B: Handle `assistant` events as fallback
**File:** `frontend/app/view/agent/providers/claude-translator.ts`
Currently `handleAssistantMessage()` returns `[]` to avoid duplicates when `--include-partial-messages` is used. Should fall back to extracting text when no stream_events were received.

### Fix C: Handle `result` events
The `result` event contains `{total_cost_usd, is_error, num_turns}`. Should emit a summary node.

### Fix D: Add stdout line logging to backend
**File:** `agentmuxsrv-rs/src/backend/blockcontroller/subprocess.rs`
Add tracing to the stdout reader loop to verify lines are being received.

---

## Priority

**Fix A is the most likely root cause and simplest fix.** Adding `--include-partial-messages` should make the CLI emit incremental stream_event lines that the translator already handles correctly.

Fix B is defense-in-depth: if incremental events aren't available, extract text from the complete assistant message.

---

## Frontend Subscription Chain — Verified OK

1. `global.ts:256` subscribes to `"blockfile"` events (allscopes) ✓
2. Handler routes to `getFileSubject(zoneid, filename)` ✓
3. `useAgentStream.ts:58` subscribes to `getFileSubject(blockId, "output")` ✓
4. Subject key: `{blockId}|output` matches backend's `zoneid|filename` ✓
5. Backend publishes `EVENT_BLOCK_FILE = "blockfile"` with correct scopes ✓

The pipeline is wired correctly. The issue is that **no data flows through it** because the CLI isn't emitting the event types the translator expects.
