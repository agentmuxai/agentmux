# Jekt Auto-Registration via AGENTMUX_AGENT_ID

**Author:** AgentX
**Date:** 2026-03-12
**Status:** Draft
**Branch:** agentx/jekt-auto-register

---

## Problem

Jekt (terminal injection) currently requires a **manual registration call** before it works:

```bash
POST /wave/reactive/register
{ "agent_id": "AgentX", "block_id": "<block-id>" }
```

But the backend already has all the information it needs at spawn time:
- `block_id` — the block's own identifier, set in `ShellController`
- `tab_id` — known at spawn time
- `AGENTMUX_AGENT_ID` — set in the block's env via `cmd:env` metadata or global settings

The explicit registration call is friction that shouldn't exist. If a pane is launched
with `AGENTMUX_AGENT_ID` set, jekt should work immediately — no extra step.

---

## Current Flow (broken without manual step)

```
1. User/Forge sets cmd:env["AGENTMUX_AGENT_ID"] = "Agent1" on block
2. Block spawns — AGENTMUX_AGENT_ID + AGENTMUX_BLOCKID are set in PTY env
3. ← Manual step required: POST /wave/reactive/register {agent_id, block_id}
4. Jekt via POST /api/bus/inject → ReactiveHandler → blockcontroller::send_input → PTY ✓
```

Without step 3, jekt returns "agent not found" and falls back to MessageBus WS push
(which pushes a bus:message event but never writes to PTY).

---

## Proposed Flow (automatic)

```
1. User/Forge sets cmd:env["AGENTMUX_AGENT_ID"] = "Agent1" on block
2. Block spawns → ShellController detects AGENTMUX_AGENT_ID in env
3. ShellController auto-calls reactive::get_global_handler().register_agent(agent_id, block_id, tab_id)
4. Jekt via POST /api/bus/inject → ReactiveHandler → blockcontroller::send_input → PTY ✓
5. Process exits → ShellController auto-calls reactive_handler.unregister_block(block_id)
```

---

## Implementation

### File: `agentmuxsrv-rs/src/backend/blockcontroller/shell.rs`

#### 1. Capture effective agent_id during env construction

**Change** `has_agent_id: bool` → `agent_id_for_jekt: Option<String>` to capture
the actual value rather than just presence.

Priority order (highest wins, matches existing env injection priority):
1. Block metadata `cmd:env["AGENTMUX_AGENT_ID"]` (highest)
2. Global settings `cmd_env["AGENTMUX_AGENT_ID"]`
3. `WAVEMUX_AGENT_ID` env var (backward compat bridge, lowest)

```rust
// Before (current)
let mut has_agent_id = false;
// ...
if k == "AGENTMUX_AGENT_ID" { has_agent_id = true; }
// ...
if !has_agent_id { c.env("AGENTMUX_AGENT_ID", &wavemux_val); }

// After
let mut agent_id_for_jekt: Option<String> = None;
// ...
if k == "AGENTMUX_AGENT_ID" { agent_id_for_jekt = Some(v.clone()); }
// ...
if agent_id_for_jekt.is_none() {
    if let Ok(val) = std::env::var("WAVEMUX_AGENT_ID") {
        agent_id_for_jekt = Some(val.clone());
        c.env("AGENTMUX_AGENT_ID", &val);
    }
}
```

#### 2. Auto-register after successful spawn

Immediately after `pair.slave.spawn_command(cmd)` succeeds:

```rust
if let Some(ref agent_id) = agent_id_for_jekt {
    match crate::backend::reactive::get_global_handler()
        .register_agent(agent_id, &self.block_id, Some(&self.tab_id))
    {
        Ok(()) => tracing::info!(
            block_id = %self.block_id,
            agent_id = %agent_id,
            "jekt: auto-registered"
        ),
        Err(e) => tracing::warn!(
            block_id = %self.block_id,
            agent_id = %agent_id,
            error = %e,
            "jekt: auto-register failed"
        ),
    }
}
```

#### 3. Auto-unregister on process exit

In the `spawn_blocking` wait task, after `child.wait()` returns:

```rust
tracing::info!(block_id = %block_id_wait, exit_code = exit_code, "process exited");
crate::backend::reactive::get_global_handler().unregister_block(&block_id_wait);
```

`unregister_block` looks up by block_id and removes the agent_id → block_id mapping.
Already exists on `ReactiveHandler`.

---

## Scope

### What changes
- `shell.rs`: ~15 lines changed/added

### What does NOT change
- `/wave/reactive/register` endpoint — still works for manual registration (backward compat,
  useful for non-shell blocks or external processes)
- MessageBus registration — unchanged; agents can still call `bus:register` separately
- Any frontend code
- Any other Rust files

---

## Edge Cases

### Agent ID re-use (same agent_id, different block)
`register_agent()` already handles this — it removes the old block mapping and installs
the new one. If Agent1 is restarted in a new block, the new block wins.

### Multiple blocks with same agent_id
Same as above — last-write wins. This is consistent with existing manual registration behavior.

### Process restart (run_on_start / resync)
If the same block is restarted (e.g. resync command), it re-spawns and hits the same
auto-register path. The mapping is refreshed with the same block_id. No issue.

### Blocks without AGENTMUX_AGENT_ID
`agent_id_for_jekt` stays `None` → no registration attempt. Pure shell panes, code editor
panes, etc. are unaffected.

---

## Testing

```bash
# 1. Open AgentMux, create a terminal pane with cmd:env["AGENTMUX_AGENT_ID"] = "test-agent"
# 2. Immediately jekt it — no /wave/reactive/register call needed:
curl -X POST http://localhost:<port>/api/bus/inject \
  -H 'Content-Type: application/json' \
  -d '{"from": "agentx", "target": "test-agent", "message": "echo hello"}'
# Expected: {"status":"injected","via":"pty","block_id":"...","target":"test-agent"}

# 3. Close the pane, retry jekt:
# Expected: {"status":"injected","via":"messagebus",...} (falls back, agent deregistered)

# 4. Verify agents list:
curl http://localhost:<port>/wave/reactive/agents
# Expected: [] after close, ["test-agent"] while running
```

---

## Open Questions

1. **Shell panes (no cmd:env agent_id)** — should the block's own `AGENTMUX_BLOCKID`
   be jekt-addressable by block_id directly? Currently no — jekt targets are always
   agent_ids. Leave for later.

2. **wsh binary** — should `wsh` support a `register` subcommand so non-shell processes
   (e.g. a Python script) can self-register? Out of scope here; manual HTTP endpoint
   still available.
