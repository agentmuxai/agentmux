# Jekt (inject_terminal): Why It Works for Host Agents but Not Containers

**Date:** 2026-03-22
**Status:** Investigation complete — root cause identified
**Scope:** Why `inject_terminal` delivers instantly for AgentX/AgentY but times out for Agent1-5

---

## Problem Statement

Calling `inject_terminal` (jekt) targeting a host agent (AgentX, AgentY) returns `status: delivered`
in under 1 second. The same call targeting a container agent (Agent1-5) returns:

```
Delivery not confirmed within 15s. Target agent may be offline or unregistered.
```

Live test results:
- Self-jekt to `agenty`: **success / delivered** ✅ — returned immediately
- Jekt to `agent4`: **timeout** ❌ — "Delivery not confirmed within 15s"

---

## Delivery Architecture

`inject_terminal` in `agentbus-client/src/client.ts` has two delivery paths:

### Path A: Local (synchronous, preferred)

Requires `AGENTMUX_LOCAL_URL` in the MCP server's environment.

```
injectTerminalLocal()
  → POST {AGENTMUX_LOCAL_URL}/wave/reactive/inject
  → agentmuxsrv-rs ReactiveHandler
  → agent_to_block lookup
  → InputSender → PTY bytes
  → returns success immediately
```

### Path B: Cloud (async, fallback)

Used when local path fails ("agent not found") or `AGENTMUX_LOCAL_URL` not set.

```
POST {AGENTBUS_URL}/reactive/inject   → DynamoDB pending injection
poll /reactive/status/{id} every 1s  → wait for ACK
  ← receiver: poll /reactive/pending/{agentId} every 5s
  ← receiver: delivers, calls /reactive/ack
  → sender sees "delivered"    (or times out after 15s)
```

---

## Why Host Agents Work

### 1. `AGENTMUX_LOCAL_URL` is in the shell environment

`agentmuxsrv-rs/src/main.rs` calls `std::env::set_var("AGENTMUX_LOCAL_URL", ...)` immediately
after binding the web listener. `shell.rs` re-injects it explicitly into every PTY's env:

```rust
if let Ok(local_url) = std::env::var("AGENTMUX_LOCAL_URL") {
    c.env("AGENTMUX_LOCAL_URL", &local_url);
}
```

The MCP server subprocess inherits this — it is **not** in `.mcp.json`.

Verified in agenty's shell:
```
AGENTMUX_LOCAL_URL = http://127.0.0.1:53968
AGENTMUX_AGENT_ID  = AgentY
AGENTMUX_BLOCKID   = acfb92c4-1af6-41b1-99f0-53661f0b98f4
```

### 2. Agent auto-registers in ReactiveHandler on shell spawn

`shell.rs` detects `AGENTMUX_AGENT_ID` in the pane env and calls:

```rust
reactive::get_global_handler().register_agent(agent_id, &self.block_id, Some(&self.tab_id))
```

This creates the bidirectional mapping `agent_id ↔ block_id` in the in-memory `ReactiveHandler`
and writes to the cross-instance file registry at `{data_dir}/agents/{agent_id}.json`.

### 3. Injection flow (agenty → agenty)

1. Sender: `AGENTMUX_LOCAL_URL` set → calls `injectTerminalLocal("http://127.0.0.1:53968", "agenty", ...)`
2. AgentMux: looks up `agent_to_block["agenty"]` → finds `acfb92c4...`
3. `InputSender` writes `\r{message}\r` to PTY + 3× delayed `\r` at 200ms
4. Returns `{success: true}` → jekt returns `status: delivered` immediately

---

## Why Container Agents Fail

### Problem 1: Agent4 is not registered in ReactiveHandler

Container agents (Agent1-5) run inside Docker. They have no AgentMux terminal pane — there is
no xterm.js block, no `block_id`, and `shell.rs` never runs inside Docker.

When agenty jekets to agent4:
- `injectTerminalLocal()` POSTs to `:53968/wave/reactive/inject` with `target_agent=agent4`
- `ReactiveHandler.inject_message()` calls `agent_to_block.get("agent4")` → **None**
- Checks cross-instance file registry → **not found** (no AgentMux in the container)
- Returns `{success: false, error: "agent not found: agent4"}`
- `injectTerminalLocal()` sees `data.error.includes('not found')` → returns `null`
- Falls back to cloud path

### Problem 2: Port 1717 on the host is not listening

Container `.mcp.json` has `AGENTMUX_LOCAL_URL=http://host.docker.internal:1717`.
Port 1717 on the host is **closed** — the planned fixed-port proxy was never implemented.

```
CLOSED port 1717   ← container's AGENTMUX_LOCAL_URL target
CLOSED port 3100   ← WAVEMUX_REACTIVE_PORT (env var set but nothing listening)
OPEN  port 53968   ← actual AgentMux backend (dynamic, OS-assigned)
```

When agent4's MCP server polls and finds a pending cloud injection:
1. Tries `POST http://host.docker.internal:1717/wave/reactive/inject` → connection refused
2. Exception caught → falls back to `console.error()` (MCP server stderr)
3. **Stderr is NOT fed back as user input in Claude Code** → injection silently lost
4. Still pushes to `deliveredIds` → calls `/reactive/ack` → cloud marks as "delivered"

### Problem 3: Even if port 1717 existed, there is no PTY to inject into

`ReactiveHandler.inject_message()` requires a `block_id` → PTY mapping.
Container agents don't have AgentMux terminal pane blocks — there is no PTY to write to.

### Why the test saw timeout (not "delivered")

The cloud path ACKs when agent4's MCP poller fetches the injection (every 5s). But:
- If agent4's Claude Code session is **not active**: no MCP server → no polling → no ACK → 15s timeout ← **what we observed**
- If agent4's Claude Code session **is active**: MCP server polls, ACKs within 5s, sender sees "delivered" — but Claude still doesn't receive the message as terminal input (only stderr)

---

## Component Map

| File | Role |
|------|------|
| `agentbus/packages/agentbus-client/src/client.ts` | `injectTerminal()`, `injectTerminalLocal()`, `pollAndDeliverInjections()`, `startInjectionPolling()` |
| `agentbus/packages/agentbus-client/src/index.ts` | MCP server startup — starts `startInjectionPolling()` if token set |
| `agentmuxsrv-rs/src/backend/reactive/handler.rs` | `ReactiveHandler` — in-memory `agent_id ↔ block_id` registry, `inject_message()` |
| `agentmuxsrv-rs/src/backend/reactive/registry.rs` | File-based cross-instance registry at `{data_dir}/agents/{agent_id}.json` |
| `agentmuxsrv-rs/src/server/reactive.rs` | HTTP handlers: `/wave/reactive/inject`, `/wave/reactive/register`, etc. |
| `agentmuxsrv-rs/src/backend/blockcontroller/shell.rs` | Auto-registers agent on shell spawn via `AGENTMUX_AGENT_ID` |
| `agentmuxsrv-rs/src/main.rs` | Sets `AGENTMUX_LOCAL_URL` env var after binding web listener |

Relevant specs:
- `specs/agentmux-local-url-injection.md` — why/how `AGENTMUX_LOCAL_URL` is injected into pane env
- `specs/jekt-auto-registration.md` — auto-registration via `AGENTMUX_AGENT_ID` on shell spawn
- `specs/jekt-inject-timing.md` — PTY write timing (`\r` + 3× delayed `\r` at 200ms)

---

## Root Cause Summary

| | Host agents (AgentX/Y) | Container agents (Agent1-5) |
|---|---|---|
| `AGENTMUX_LOCAL_URL` | `:PORT` (real, injected by AgentMux) | `:1717` (not listening) |
| Registered in ReactiveHandler | Yes (auto on shell spawn) | No (no AgentMux pane) |
| PTY block_id mapping | Yes | No |
| jekt local delivery | PTY direct, instant | N/A — no PTY |
| jekt cloud delivery | Works if needed | ACKs but Claude doesn't see it |
| Correct comms tool | `inject_terminal` | `send_message` / `read_messages` |

**jekt is a terminal-pane feature.** It writes bytes to an xterm.js PTY block via the AgentMux
reactive handler. Container agents have no such block — the delivery path does not exist.

---

## Fix Options

### Option 1: Use `send_message` (no code changes — correct tool)

Container agents should use the mailbox path:
- Sender: `send_message(target="agent4", message="...")`
- Receiver: `read_messages()` (polls inbox)

This is what the mailbox exists for. Works today.

### Option 2: Fixed-port bridge on host (implements planned port 1717)

Add a secondary fixed-port listener to `agentmuxsrv-rs` that:
1. Accepts inject/register requests from Docker containers via `host.docker.internal:1717`
2. Registers container agents with a virtual block_id
3. Delivers messages via Docker exec API (`docker exec -i {container} cat`) rather than xterm.js PTY

Requires: fixed-port listener in Rust, Docker socket access from AgentMux, container agent registration on startup.

### Option 3: Cloud path + container stdin write

When container agent's MCP server receives a pending injection:
- Instead of `console.error()`, write to `process.stdin` of the Claude Code process
- This requires understanding how Claude Code reads its terminal input

Not straightforward — Claude Code uses readline/TTY, not raw stdin.

---

## Recommended Action

**Short term:** Document that `inject_terminal` is host-agent only; use `send_message` for containers.

**Medium term:** Option 2 (fixed-port bridge) if reliable container jekt is needed.
