# SPEC: Subprocess Integration Tests

**Date:** 2026-04-16
**Status:** Draft

---

## Problem

Four bugs shipped in the subprocess I/O path this session alone:

| Bug | Root Cause | How a test catches it |
|-----|-----------|----------------------|
| Zero stdout on Windows | Missing `CREATE_NO_WINDOW` flag | Spawn test fails: no output received |
| Silent pipe failures | `while let Ok(Some(line))` exits silently on error | Spawn test would log the error path |
| Duplicate messages | `partial:true` assistant events creating extra nodes | Translator test asserts node count |
| Stdin not flushing | `write_all` without `flush()` — data stuck in buffer | Spawn test times out: child never receives input |

All four are **runtime I/O issues** invisible to `cargo check` and `tsc --noEmit`.
Unit tests can't reproduce them because they don't spawn real processes with
real pipes on the target OS. Integration tests that exercise the actual spawn →
stdin → stdout → exit path on Windows would catch all of them.

---

## Test Categories

### 1. Subprocess I/O Smoke Test (Rust, `agentmux-srv`)

**What:** Spawn a real child process, write to stdin, read from stdout, verify
exit code. Exercises the exact Tokio pipe + IOCP path that runs in production.

**Test process:** A minimal script that:
1. Reads one line from stdin
2. Echoes it back to stdout as JSON
3. Exits with code 0

```rust
#[tokio::test]
async fn subprocess_stdin_stdout_roundtrip() {
    // Spawn: node -e "process.stdin.on('data', d => { process.stdout.write(d); process.exit(0); })"
    // Or use a cross-platform Rust helper binary
    //
    // 1. Write "hello\n" to stdin
    // 2. Flush stdin
    // 3. Assert stdout receives "hello\n" within 5s
    // 4. Assert process exits with code 0
}
```

**Variants:**
- `subprocess_stdin_flush_required` — write without flush, verify data still
  arrives (tests that our flush call works; without it, would timeout)
- `subprocess_create_no_window` — Windows-only: verify child process runs
  without allocating a console (check `GetConsoleWindow()` returns NULL in child)
- `subprocess_large_stdin` — write 1MB+ to stdin, verify stdout receives it all
  (tests pipe buffer limits)
- `subprocess_stderr_captured` — child writes to stderr, verify it's logged
- `subprocess_exit_code_nonzero` — child exits with code 1, verify controller
  reports it correctly

**Location:** `agentmux-srv/tests/subprocess_io.rs` (integration test, not unit)

**Dependencies:** Node.js on PATH (already required for CLI providers)

### 2. SubprocessController Integration Test (Rust, `agentmux-srv`)

**What:** Instantiate a real `SubprocessController` with mock broker/filestore,
call `spawn_turn()` with a simple echo process, verify events are published.

```rust
#[tokio::test]
async fn subprocess_controller_publishes_output() {
    let (broker, rx) = mock_broker();
    let ctrl = SubprocessController::new("tab", "block", Some(broker), ...);
    ctrl.spawn_turn(SubprocessSpawnConfig {
        cli_command: "node".into(),
        cli_args: vec!["-e".into(), ECHO_SCRIPT.into()],
        message: r#"{"test": true}"#.into(),
        ..Default::default()
    }).unwrap();

    // Wait for blockfile event on rx
    let event = timeout(Duration::from_secs(5), rx.recv()).await.unwrap();
    assert!(event.data.contains("test"));

    // Wait for controller status → "done"
    let status = ctrl.get_runtime_status();
    assert_eq!(status.shellprocstatus, "done");
}
```

**Variants:**
- `subprocess_controller_session_id_captured` — echo a system/init JSON line,
  verify `session_id()` is populated
- `subprocess_controller_health_transitions` — verify Idle → Healthy → Exited
- `subprocess_controller_concurrent_spawn_blocked` — call `spawn_turn` twice,
  verify second returns error

**Location:** `agentmux-srv/tests/subprocess_controller.rs`

### 3. ACP Controller Integration Test (Rust, `agentmux-srv`)

**What:** Same pattern as #2 but for the ACP controller. Mock ACP server that
speaks JSON-RPC 2.0: responds to `initialize`, `session/create` (returns
sessionId), `session/prompt` (streams `agent_message_chunk` events).

```rust
#[tokio::test]
async fn acp_controller_handshake_and_prompt() {
    // Start mock ACP server (node script or Rust binary)
    // Create AcpController with mock broker
    // Call send_message("hello")
    // Verify:
    //   1. initialize request sent
    //   2. session/create request sent
    //   3. pending_prompt flushed after session/create response
    //   4. agent_message_chunk events published via broker
    //   5. Process exits cleanly after shutdown
}
```

**Location:** `agentmux-srv/tests/acp_controller.rs`

### 4. Claude Translator Dedup Test (TypeScript, vitest)

**What:** Feed the `ClaudeTranslator` a sequence of events that includes
`partial:true` assistant snapshots + streaming deltas + final assistant event.
Assert that the resulting `StreamEvent[]` does not contain duplicate text.

```typescript
describe("ClaudeTranslator dedup", () => {
    it("skips partial:true assistant events", () => {
        const t = new ClaudeTranslator();
        const events = t.translate({
            type: "assistant",
            message: { content: [{ type: "text", text: "hello" }] },
            partial: true,
        });
        expect(events).toHaveLength(0);
    });

    it("skips text blocks in final assistant event", () => {
        const t = new ClaudeTranslator();
        const events = t.translate({
            type: "assistant",
            message: { content: [{ type: "text", text: "hello" }] },
        });
        expect(events).toHaveLength(0);
    });

    it("preserves tool_use blocks in final assistant event", () => {
        const t = new ClaudeTranslator();
        const events = t.translate({
            type: "assistant",
            message: { content: [{ type: "tool_use", id: "t1", name: "Read", input: {} }] },
        });
        expect(events).toHaveLength(1);
        expect(events[0].type).toBe("tool_call");
    });
});
```

**Location:** `frontend/app/view/agent/providers/claude-translator.test.ts`

### 5. Startup Payload Assembly Test (TypeScript, vitest)

**Status:** Already implemented — 17 tests in
`frontend/app/view/agent/startup/buildStartupPayload.test.ts`. Covers identity,
accounts, peers, template expansion, skip sentinel, and peer capping.

---

## Implementation Plan

### Phase 1: TypeScript translator tests (quick win)

1. Create `claude-translator.test.ts` with dedup tests (category 4)
2. Add edge cases: `stream_event` → `assistant` sequence, tool_use dedup
3. Run via `npx vitest`

**Effort:** ~1 hour. Catches the duplicate message bug.

### Phase 2: Rust subprocess smoke tests

1. Create `agentmux-srv/tests/subprocess_io.rs`
2. Write a minimal echo helper (inline node `-e` script)
3. Tests: roundtrip, flush, stderr, exit code, large payload
4. Add `#[cfg(windows)]` tests for CREATE_NO_WINDOW verification
5. Add to CI (requires Node.js on Windows runner)

**Effort:** ~2-3 hours. Catches stdin flush + pipe issues.

### Phase 3: Controller integration tests

1. Create mock broker + filestore helpers in `agentmux-srv/tests/helpers/`
2. `subprocess_controller.rs` — spawn_turn, session capture, health transitions
3. `acp_controller.rs` — handshake, pending_prompt, shutdown

**Effort:** ~4-5 hours. Catches race conditions + protocol issues.

---

## CI Considerations

- Rust integration tests require `node` on PATH (Windows + Linux runners)
- Tests that spawn processes need longer timeouts (10s+) for CI cold starts
- Use `#[ignore]` + `--ignored` flag for slow tests to keep `cargo test` fast
- Windows-specific tests use `#[cfg(target_os = "windows")]`

---

## Test Fixtures

### Echo helper (Node.js, cross-platform)

```javascript
// test-fixtures/echo-stdin.js
process.stdin.setEncoding('utf8');
let buf = '';
process.stdin.on('data', d => { buf += d; });
process.stdin.on('end', () => {
    process.stdout.write(JSON.stringify({ echo: buf.trim() }) + '\n');
    process.exit(0);
});
```

### Mock ACP server (Node.js)

```javascript
// test-fixtures/mock-acp-server.js
const readline = require('readline');
const rl = readline.createInterface({ input: process.stdin });
let sessionId = 'test-session-' + Date.now();

rl.on('line', (line) => {
    const msg = JSON.parse(line);
    if (msg.method === 'initialize') {
        console.log(JSON.stringify({ jsonrpc: '2.0', id: msg.id, result: { capabilities: {} } }));
    } else if (msg.method === 'session/create') {
        console.log(JSON.stringify({ jsonrpc: '2.0', id: msg.id, result: { sessionId } }));
    } else if (msg.method === 'session/prompt') {
        // Stream a response
        console.log(JSON.stringify({ jsonrpc: '2.0', method: 'session/update',
            params: { type: 'agent_message_chunk', content: 'Hello from mock ACP' } }));
        console.log(JSON.stringify({ jsonrpc: '2.0', id: msg.id,
            result: { stopReason: 'endTurn' } }));
    } else if (msg.method === 'shutdown') {
        console.log(JSON.stringify({ jsonrpc: '2.0', id: msg.id, result: {} }));
    }
});
```
