# SPEC: Mid-Turn Messaging — User Input While Agent is Busy

**Date:** 2026-04-16
**Status:** Draft
**Priority:** High — core UX gap

---

## Problem

When the user sends a message while the agent is mid-turn (subprocess running),
`spawn_turn` rejects it with "subprocess is already running a turn." The message
is lost. The composer gives no feedback. The user thinks the app is broken.

This is the #1 UX gap vs. Claude Code CLI, where you can:
- Type while the agent is working (queued for next turn)
- Press Esc to interrupt (SIGINT → agent stops, you get control)
- Use `/btw` for a side question without interrupting

---

## Current State

```
User sends "check the logs"     → subprocess spawns, turn 1 starts
User sends "also fix the test"  → spawn_turn returns error, message LOST
Agent finishes turn 1            → status → "done", ready for next
                                   but "also fix the test" is gone
```

---

## Design

Three capabilities, layered:

### Layer 1: Message Queue (always-on)

**When:** User sends a message while a turn is in progress.
**Behavior:** Message is queued in the `SubprocessController`. When the current
turn completes (subprocess exits), the queued message is automatically sent as
the next turn via `spawn_turn`.

**Frontend UX:**
- Composer stays enabled during active turns
- Queued messages appear in the document immediately as `user_message` nodes
  (same as today) with a subtle "queued" badge
- Status line shows "Message queued — will send after current turn"
- Multiple messages can be queued — they're concatenated or sent sequentially

**Backend changes (`subprocess.rs`):**

```rust
struct SubprocessControllerInner {
    // ... existing fields ...
    /// Messages queued while a turn is in progress.
    /// Drained sequentially after the current turn exits.
    pending_messages: VecDeque<SubprocessSpawnConfig>,
}
```

When `spawn_turn` is called and `run_lock` is held:
- Instead of returning `Err("already running")`, push to `pending_messages`
- Return `Ok(())` — the message is accepted

In the `process_waiter` task, after the subprocess exits:
- Check `pending_messages`
- If non-empty, pop the first and call `spawn_turn` (re-acquire lock)
- This creates a chain: turn 1 → exit → turn 2 (queued) → exit → ...

**Frontend changes:**
- Remove the composer disable-during-turn logic (if any)
- `AgentInputCommand` handler returns success even when queued
- Add `queued` field to `AgentInputCommand` response so frontend can show badge

### Layer 2: Interrupt (Esc)

**When:** User presses Esc while a turn is in progress.
**Behavior:** Send SIGINT to the running subprocess. Claude CLI handles this
gracefully — it stops the current response, emits a `result` event with
`stop_reason: "interrupted"`, and exits.

**This already works** — `stopAgent()` in `useAgentCommands` calls
`ControllerInputCommand` with `signame: "SIGINT"`. The subprocess controller
handles it in `stop_subprocess`.

**Enhancement:** After interrupt, if there are queued messages, send the first
one automatically. The user interrupted to say something more important — honor
that by processing the queue.

### Layer 3: Side Question (`/btw`)

**When:** User types `/btw <question>` while a turn is in progress.
**Behavior:** The question is answered immediately in a floating overlay without
interrupting the main turn. The answer is ephemeral — not added to session
history.

**Architecture:**

```
Main turn:  subprocess running → stdout streaming → document
/btw:       separate API call → overlay panel → dismissed by user
```

The /btw response does NOT go through the subprocess. It's a direct API call
using the current conversation context (prompt cache reuse = cheap).

**Frontend implementation:**

1. Intercept `/btw <question>` in the composer (before `sendMessage`)
2. Capture the current conversation context (document nodes → messages)
3. Call the Claude API directly via the host API or a dedicated RPC:
   ```typescript
   RpcApi.BtwQueryCommand(TabRpcClient, {
       blockid: blockId,
       question: question,
       // conversation_snapshot is built from current document nodes
   })
   ```
4. Stream response into a dismissible overlay panel (not the main document)
5. Overlay dismissed with Esc/Space/Enter
6. No persistence — gone on session reload

**Backend implementation (`btw_handler.rs`):**

```rust
pub async fn handle_btw_query(
    block_id: &str,
    question: &str,
    wstore: &WaveStore,
    filestore: &FileStore,
) -> Result<String, String> {
    // 1. Read conversation history from filestore
    // 2. Build messages array (same format as the main session)
    // 3. Append the /btw question as a user message
    // 4. Call Anthropic Messages API directly (not via subprocess)
    //    - No tools (read-only)
    //    - Reuse prompt cache from parent session
    //    - max_tokens: 1024 (short answer)
    // 5. Return the response text
}
```

**Cost efficiency:** The /btw call reuses the prompt cache from the main
session. Since the conversation prefix is already cached (5-minute TTL),
the /btw call only pays for:
- Cache read tokens (10% of input cost)
- The question tokens (tiny)
- Output tokens (short answer)

Typical cost: $0.001-0.005 per /btw query.

**Requires:** Direct Anthropic API access from the sidecar, using the same
auth credentials as the subprocess. The auth dir already has the API key.

---

## Implementation Plan

### Phase 1: Message Queue (PR 1)

**Backend:**
1. Add `pending_messages: VecDeque<SubprocessSpawnConfig>` to inner state
2. Modify `spawn_turn`: if locked, push to queue and return Ok
3. Modify process_waiter: after exit, drain queue → spawn next turn
4. Return `queued: true` in AgentInput response when message was queued

**Frontend:**
5. Show "queued" badge on user_message nodes when response indicates queuing
6. Status line: "Message queued — sending after current turn"
7. Keep composer enabled during active turns

**Estimated effort:** 2-3 hours. High impact, low risk.

### Phase 2: Interrupt + Queue Integration (PR 2)

1. After SIGINT interrupt, check pending_messages
2. If queue has messages, auto-send the first one
3. Add "Interrupt and send" UX: if user types while agent is busy and
   presses Ctrl+Enter (not just Enter), interrupt + send immediately

**Estimated effort:** 1-2 hours.

### Phase 3: /btw Side Questions (PR 3-4)

**PR 3 — Backend:**
1. `btw_handler.rs`: read conversation from filestore, call Anthropic API
2. New RPC command: `BtwQueryCommand`
3. Streaming response via WPS event on a `btw` subject

**PR 4 — Frontend:**
1. Intercept `/btw` in composer
2. `BtwOverlay` component: floating panel, dismissible
3. Stream response into overlay
4. No document persistence

**Estimated effort:** 4-6 hours. Requires direct API access from sidecar.

---

## Message Flow Diagrams

### Queue (Phase 1)

```
User: "check logs"    → spawn_turn → subprocess pid=1234
                         turn 1 running...
User: "fix test too"  → spawn_turn → QUEUED (run_lock held)
                         turn 1 still running...
                         subprocess exits
                         process_waiter drains queue
                       → spawn_turn → subprocess pid=5678
                         turn 2 running with "fix test too"
                         subprocess exits
                         queue empty, done
```

### Interrupt + Queue (Phase 2)

```
User: "check logs"    → spawn_turn → subprocess pid=1234
                         turn 1 running...
User: Esc             → SIGINT to pid=1234
                         subprocess exits (interrupted)
User: "fix test too"  → spawn_turn → subprocess pid=5678
                         turn 2 running
```

### /btw (Phase 3)

```
User: "check logs"    → spawn_turn → subprocess pid=1234
                         turn 1 running...
User: "/btw what's    → BtwQueryCommand (separate API call)
       the error?"      → overlay shows answer
                         turn 1 still running (uninterrupted)
                         subprocess exits
                         overlay dismissed by user
```

---

## ACP Considerations

The ACP controller (`acp.rs`) already handles this better than subprocess:
- ACP is a long-running process with stdin/stdout streaming
- Multiple `session/prompt` requests can be sent — the protocol handles
  sequencing natively
- The `pending_prompt` pattern already exists for the first turn

For ACP agents, the queue layer may not be needed — the protocol handles it.
But the /btw layer is still valuable since it's a fundamentally different
interaction pattern (side question vs. queued turn).

---

## Security Considerations

- **/btw** requires direct API access from the sidecar. The sidecar already
  has the auth dir with API credentials. No new credential paths needed.
- **/btw** responses are read-only (no tools). The system prompt explicitly
  says "Answer this side question. Do not modify files or run commands."
- Queued messages go through the same subprocess path as normal messages —
  no new attack surface.

---

## Non-Goals

- **Concurrent turns.** We don't run multiple subprocesses in parallel.
  Queue serializes them.
- **Editing messages mid-turn.** The user can't modify a message that's
  already been sent to the subprocess.
- **Persistent /btw history.** Side questions are ephemeral by design.
  If the user wants a persistent question, they should use the normal
  composer (which queues it).
