# Spec: Outgoing Jekt Echo — Make Muxbus Messages Visible on the Sender's Side

**Date:** 2026-07-10
**Author:** AgentY
**Type:** Implementation-ready fix. No code shipped yet.
**Purpose:** Close the gap between `docs/specs/SPEC_JEKT_SECURITY_AND_VISIBILITY_2026_07_01.md` §3.2 (spec'd) and what actually shipped in v0.52.0 (only the incoming half) — a human watching the *sending* agent's pane currently sees nothing distinctive when that agent calls `SendMessage`, only a bare tool-result string. This spec makes the sender's pane show the same `JektBubble` treatment the recipient's pane already gets.

---

## 1. Problem, verified against current code (2026-07-10)

`SPEC_JEKT_SECURITY_AND_VISIBILITY_2026_07_01.md` §3.2 calls for:

> When an agent calls `SendMessage`, the host echoes the outgoing jekt to the agent's own pane output ... This gives the human operator a visible record of what the agent sent and whether delivery was confirmed.

**What actually shipped (v0.52.0, `feat(jekt): render [JEKT:...] markers as JektBubble in the agent pane`):**

- **Incoming, fully working.** `agentmux-srv/src/backend/reactive/handler.rs`'s `handle_reactive_inject` wraps the message via `wrap_jekt_message(...)` (`sanitize.rs:197-230`) and injects the wrapped block as the recipient's next input. The frontend's `tryParseJekt` (`frontend/app/view/agent/stream-parser.ts:547-590`), wired into `userMessageToNode`, detects the `[JEKT:...]...[/JEKT]` block and renders a `JektBubble` (`frontend/app/view/agent/components/JektBubble.tsx`).
- **Outgoing, never implemented.** `agentmux-mcp/src/main.rs`'s `SendMessage` handler (line 869) POSTs to `/agentmux/reactive/inject` and, on success, returns the plain string `Ok(format!("Message sent to {to}"))` as the tool result — not a JEKT-formatted block. This return value flows through `toolResultToNode` (`stream-parser.ts:429-465`), which has **no jekt-detection at all** — it always builds a generic collapsed `ToolBlock`. `tryParseJekt` is only ever called from `userMessageToNode` (`stream-parser.ts:524`), not from the tool-result path.

The frontend is already half-ready for this: `tryParseJekt`'s own comment says outgoing direction is "handled here so JektBubble is ready when it is [emitted]" (`stream-parser.ts:566-567`) — confirming this was a known, deliberately-deferred gap, not an oversight. It was just never fed a producer.

**Net effect matching the user report:** the human sees jekt bubbles on the receiving agent's pane, but the sending agent's pane shows nothing but a bland collapsed tool-result row reading "Message sent to X" — no sender-visible record of the message, tier, or delivery status.

---

## 2. Design

Reuse the existing `wrap_jekt_message` formatting rather than inventing a second marker format — the frontend parser is format-coupled to it (`JEKT_BLOCK_RE`, `parseJektTagFields`), so anything else would need its own parser branch for no benefit.

### 2.1 A caught bug in the naive version of this design

The obvious first cut — have the server return the *exact same* wrapped string it already computed for the recipient, and have `SendMessage` return that as the tool result — is wrong. `wrap_jekt_message`'s trailer line is:

```
Reply: bus:inject to {from}
```

`{from}` is the *sender* (`source_agent`). That line is correct advice when the recipient reads it ("reply to whoever sent this"). But if the identical string is echoed back into the **sender's own pane**, the sender would see "Reply: bus:inject to {themselves}" — nonsensical, and actively confusing for the human watching that pane. This has to be a distinct trailer, not a verbatim reuse.

### 2.2 `wrap_jekt_message` gains a trailer variant

`agentmux-srv/src/backend/reactive/sanitize.rs`:

```rust
pub enum JektTrailer {
    /// Recipient-facing: "Reply: bus:inject to {from}".
    ReplyHint,
    /// Sender-facing echo: "Status: delivered".
    DeliveredStatus,
}

pub fn wrap_jekt_message(
    msg: &str,
    source_agent: Option<&str>,
    target_agent: &str,
    effective_tier: &str,
    delivery_tier: &str,
    msg_id: &str,
    priority: &str,
    trailer: JektTrailer,
) -> String {
    // ... unchanged body ...
    let trailer_line = match trailer {
        JektTrailer::ReplyHint => format!("Reply: bus:inject to {from}"),
        JektTrailer::DeliveredStatus => "Status: delivered".to_string(),
    };
    // ... use trailer_line where reply_hint was used ...
}
```

The one existing call site (`handler.rs:278-286`, building the recipient's injected message) passes `JektTrailer::ReplyHint` — behavior-preserving, zero change to what's already shipped and working. A `#[derive(Clone, Copy)]` enum over a bare `bool` (`is_echo: bool`) — self-documenting at call sites, and room for a third variant later (e.g. `Failed` for a delivery failure) without another signature change.

Also add `STATUS=delivered` to the structured tag itself for the echo variant (spec §3.2's own example includes it) — the frontend's `JektMessageNode` type doesn't currently have a `status` field, and this spec doesn't require adding one (§2.4 covers what the frontend actually needs); it's fine for `STATUS=delivered` to live only in the human-readable trailer for now, not the structured tag, keeping the parser's required-field set (`FROM`/`TO`) unchanged. Revisit if a future "pending/failed" state needs the frontend to branch on it.

### 2.3 `InjectionResponse` carries the echo string

`agentmux-srv/src/backend/reactive/types.rs`:

```rust
pub struct InjectionResponse {
    pub success: bool,
    pub request_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub block_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub timestamp: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub echo: Option<String>,   // NEW — the JEKT block for the sender's own pane
}
```

`handle_reactive_inject` (`handler.rs`) builds a second wrap call alongside the existing one, only on the success path (no `echo` on failure — see §2.5):

```rust
let echo = wrap_jekt_message(
    &sanitized, req.source_agent.as_deref(), &req.target_agent,
    effective_tier, delivery_tier, &request_id, priority,
    JektTrailer::DeliveredStatus,
);
```

Set `response.echo = Some(echo)` on the success path only.

### 2.4 `agentmux-mcp`'s `SendMessage` returns the echo, not a bare string

`agentmux-mcp/src/main.rs`, `"SendMessage"` arm (currently line 869-930): after the existing success check, use `result.get("echo")` if present, falling back to today's string if an older server doesn't send it (forward/backward compat across mismatched host/mcp-tool versions during a rollout):

```rust
if result.get("success").and_then(|v| v.as_bool()) == Some(true) {
    match result.get("echo").and_then(|v| v.as_str()) {
        Some(echo) => Ok(echo.to_string()),
        None => Ok(format!("Message sent to {to}")), // older server, no echo field
    }
} else {
    // unchanged failure path
}
```

### 2.5 Frontend: detect a jekt block inside a tool result

`stream-parser.ts`'s `toolResultToNode` (429-465) needs the same jekt-sniffing `tryParseJekt` already does for user messages, but tool results carry the text in `event.result`, not `event.message`, and only for the `SendMessage` tool specifically (no other tool should have its output speculatively parsed as a jekt block, even if it happened to match the regex by coincidence — narrow the check by tool name, not just content shape):

```ts
private toolResultToNode(event: ToolResultEvent): DocumentNode {
    const toolCall = this.pendingToolCalls.get(event.id);
    const params = toolCall?.params || {};
    const toolName = (event.tool && event.tool !== "Unknown") ? event.tool : (toolCall?.tool || "Unknown");
    this.pendingToolCalls.delete(event.id);

    if (toolName === "SendMessage" && typeof event.result === "string") {
        const jekt = this.tryParseJektText(event.result, event.timestamp);
        if (jekt) return jekt;
    }
    // ... existing generic ToolBlock path, unchanged ...
}
```

This requires factoring `tryParseJekt`'s body (547-590) into a `tryParseJektText(text: string, timestamp?: number): JektMessageNode | null` that both `userMessageToNode` and `toolResultToNode` call — a pure refactor, no behavior change to the existing incoming path.

**Direction detection already works unmodified.** `tryParseJektText`'s existing direction logic (`stream-parser.ts:569-573`) — `FROM === currentAgentId && TO !== currentAgentId → outgoing` — already does the right thing here: the echo's `FROM` is the sending agent itself (this pane's `currentAgentId`), so it's naturally classified `outgoing` without any new logic. This is exactly the case the original comment said the direction logic was "ready" for.

### 2.6 Failure case — no echo, don't fabricate one

On delivery failure (`result.success !== true`), `SendMessage` keeps returning today's plain error string (`"Message delivery failed: {err}"`) — not a jekt block. A failed send was never delivered, so there's no "this is what got sent" record to show; inventing a fake jekt bubble for a failure would misrepresent something that didn't happen. The existing collapsed `ToolBlock` (red border, ✗ icon) is already the correct treatment for a failed tool call — this spec doesn't change that path.

---

## 3. Data flow, before / after

**Before:**
```
Agent calls SendMessage → agentmux-mcp POSTs /agentmux/reactive/inject
  → recipient's pane: [JEKT:FROM=... TO=...] injected → JektBubble ✓
  → sender's pane: tool_result "Message sent to X" → generic ToolBlock ✗ (the gap)
```

**After:**
```
Agent calls SendMessage → agentmux-mcp POSTs /agentmux/reactive/inject
  → recipient's pane: [JEKT:FROM=... TO=... Reply: bus:inject to <sender>] injected → JektBubble ✓ (unchanged)
  → response.echo = [JEKT:FROM=... TO=... Status: delivered] (new, sender-appropriate trailer)
  → agentmux-mcp returns response.echo as the SendMessage tool result
  → sender's pane: tool_result matches JEKT_BLOCK_RE (SendMessage-gated) → JektBubble, direction=outgoing ✓
```

---

## 4. Scope / non-goals

- **No new wire format.** Same `[JEKT:...]...[/JEKT]` structured tag and regex; only the human-readable trailer line differs by variant.
- **No `db_agent_mcp_ref`/muxbus protocol changes.** This is purely: (a) the server also returns the block it already builds, via one new response field; (b) the MCP tool relays it instead of a canned string; (c) the frontend recognizes it in one additional, narrowly-gated code path.
- **STATUS field kept out of the structured tag for now** (§2.2) — avoids widening `JektMessageNode`'s required fields for a status this spec doesn't need machine-readable yet.
- **Delivery-failure bubbles are explicitly out of scope** (§2.6) — the existing failed-ToolBlock treatment already covers it correctly; don't build a second failure-representation.
- **Tier-2/3/4 delivery (LAN/WAN)** — `handle_reactive_inject` already computes `delivery_tier` for all tiers today; this spec's echo reuses whatever tier it resolved, so cross-tier sends get an accurate echo with no extra plumbing.

## 5. Test plan (for the implementing PR)

- Rust: unit test on `wrap_jekt_message` asserting the two trailer variants produce the expected trailer line and that `ReplyHint`'s existing output is byte-for-byt identical to pre-change output (regression guard on the one already-shipped call site).
- Rust: `handle_reactive_inject` test asserting `response.echo` is `Some` on success and `None` on every failure branch (rate limit, invalid agent, agent not found).
- Frontend: `stream-parser.test.ts` (or wherever `tryParseJekt`'s existing incoming-path tests live) gets a new case: a `tool_result` event for tool `"SendMessage"` whose `result` is a well-formed `[JEKT:FROM=self TO=other ...]` block parses to a `JektMessageNode` with `direction: "outgoing"`; a `tool_result` for any *other* tool containing incidentally jekt-shaped text does **not** get parsed as a jekt (confirms the tool-name gate in §2.5).
- Manual: two agents, `SendMessage` A→B — confirm A's own pane shows an outgoing `JektBubble` (right-aligned per §3.3 of the governing spec, paper-plane icon) alongside B's existing incoming bubble.

## 6. References

- `docs/specs/SPEC_JEKT_SECURITY_AND_VISIBILITY_2026_07_01.md` — governing spec, §3.2 is what this implements.
- `agentmux-srv/src/backend/reactive/handler.rs`, `sanitize.rs`, `types.rs` — current incoming-only implementation.
- `agentmux-mcp/src/main.rs:869-930` — `SendMessage` tool handler.
- `frontend/app/view/agent/stream-parser.ts:429-465, 524, 547-590` — `toolResultToNode`, `userMessageToNode`, `tryParseJekt`.
- `frontend/app/view/agent/components/JektBubble.tsx` — rendering, already direction-aware.
- `VERSION_HISTORY.md` 0.52.0 — `feat(jekt): render [JEKT:...] markers as JektBubble in the agent pane` (the shipped incoming-only half).
