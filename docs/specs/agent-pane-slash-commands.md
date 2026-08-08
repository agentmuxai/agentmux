# Agent Pane Slash Commands

**Status:** Proposed (implemented — see note below)
**Date:** 2026-04-11

> **2026-08-07 audit note:** Implemented (`SlashAutocomplete.tsx`). Status
> field was never updated. See
> `docs/reports/REPORT_DOCS_AND_DEAD_CODE_CLEANUP_AUDIT_2026_08_07.md`.

---

## Key Finding: Only Stream-Compatible Commands Work

**Verified 2026-04-11:** The Claude Code CLI in `--input-format stream-json` mode
splits slash commands into two categories:

### Category 1: Stream-compatible (work natively)

The `system.init` event includes a `slash_commands` array listing commands that
work in stream-json mode. Currently:

```
["update-config", "debug", "simplify", "batch", "loop", "schedule",
 "claude-api", "build", "deploy", "pr", "compact", "context", "cost",
 "heapdump", "init", "review", "security-review", "extra-usage",
 "insights", "team-onboarding"]
```

Sending `{"type":"user","message":{"role":"user","content":"/cost"}}` returns a
synthetic assistant message (`model: "<synthetic>"`, 0 tokens) with the command
output. **No special handling needed for these — they already work in the agent pane.**

### Category 2: Interactive-only (DO NOT work in stream-json)

Commands that require TTY interaction return `"Unknown skill: <name>"`:

- `/login` → `"Unknown skill: login"` ❌
- `/logout` → likely the same
- `/help` → `"Unknown skill: help"` ❌
- `/clear` → likely the same

These commands are **not in the `slash_commands` array** because they need:
- Interactive prompts (`/login` opens browser, waits for callback)
- TTY clearing (`/clear` uses ANSI codes)
- Pager-style output (`/help` uses scrolling)

**The agent pane currently has no way to handle these commands** — typing `/login`
in the input field sends it to the persistent CLI which responds with "Unknown skill".

## Remaining Problem: UI/UX

While slash commands work at the protocol level, the agent pane's UI doesn't:

1. Show autocomplete when typing `/` (list available commands from `system.init`)
2. Style synthetic responses differently (they're not from Claude)
3. Handle `/login` which needs browser interaction (OAuth popup)
4. Handle `/clear` as a frontend-only action

## Solution

### Option A: Frontend interception (recommended)

The agent pane's input handler checks if the message starts with `/` and routes
it differently:

```typescript
// In handleSendMessage or AgentFooter
if (message.startsWith("/")) {
    // Send as CLI command, not user message
    await sendSlashCommand(blockId, message);
} else {
    await RpcApi.AgentInputCommand(TabRpcClient, { blockid, message });
}
```

The `sendSlashCommand` sends a different stdin format that the CLI recognizes
as a command, not a prompt. The Claude Code CLI's `--input-format stream-json`
supports a command message type:

```json
{"type":"command","command":"/login"}
```

**Verification needed:** Check if `--input-format stream-json` actually supports
a `command` type. If not, the CLI may need to be patched, or we use Option B.

### Option B: Raw stdin passthrough

For slash commands, bypass the stream-json wrapper entirely and write the raw
command text to stdin:

```
/login\n
```

The CLI's stdin reader in stream-json mode might reject non-JSON lines. But some
CLIs have a fallback that treats non-JSON lines as raw commands.

**Verification needed:** Test what happens when you write `/login\n` to stdin
while in `--input-format stream-json` mode.

### Option C: Separate CLI invocation

For commands like `/login` that need interactive I/O (browser popup, etc.),
spawn a separate short-lived CLI process:

```bash
claude auth login
```

This runs alongside the persistent process and handles the OAuth flow. The
persistent process doesn't need to know about it — the auth state is shared
via the credentials file.

For commands like `/compact` that affect the running session, send them through
the persistent process's stdin.

### Hybrid approach (recommended)

Classify slash commands into categories:

| Category | Commands | Handling |
|----------|----------|----------|
| **Session** | `/compact`, `/cost`, `/help`, `/context` | Send via stdin stream-json |
| **Auth** | `/login`, `/logout` | Spawn separate CLI process |
| **Config** | `/init`, `/config` | Spawn separate CLI process |
| **Display** | `/clear` | Frontend-only (clear document) |

## Slash Commands Reference

From the Claude Code CLI (`system.init` event → `slash_commands` array):

| Command | Description | Category |
|---------|-------------|----------|
| `/compact` | Compact conversation context | Session |
| `/context` | Show context window usage | Session |
| `/cost` | Show token costs | Session |
| `/help` | Show help | Session |
| `/init` | Initialize project config | Config |
| `/login` | Log in to Claude | Auth |
| `/review` | Review code changes | Session |
| `/security-review` | Security audit | Session |
| `/extra-usage` | Check extra usage | Session |
| `/insights` | Session insights | Session |
| `/debug` | Debug information | Session |
| `/simplify` | Simplify code | Session |

## Implementation Plan

### Phase 1: Frontend slash command routing

1. In `AgentFooter.tsx` or `handleSendMessage`:
   - Check `message.startsWith("/")`
   - Parse command name + args
   - Route based on category

2. For **Session** commands:
   - Try sending as `{"type":"command","command":"/compact"}` via stdin
   - If CLI rejects, fall back to sending as user message (Claude will
     interpret it contextually)

3. For **Auth** commands:
   - Call `getApi().runCliLogin()` (existing IPC command)
   - Show auth overlay in the pane

4. For **Display** commands (`/clear`):
   - Reset the document signal directly
   - Don't send to CLI

### Phase 2: Backend support

1. Add `agent.command` RPC command:
   ```typescript
   { command: "agent.command", data: { block_id: "...", slash_command: "/compact" } }
   ```

2. The handler routes based on category (same as frontend)

3. For session commands, write to persistent process stdin
4. For auth commands, spawn separate process

### Phase 3: Custom commands

Allow users to register custom slash commands via Forge skills:
- Skill with `trigger: "/deploy"` → executes the skill's content
- Listed in the agent pane's autocomplete when typing `/`

## Files to Modify

| File | Change |
|------|--------|
| `frontend/app/view/agent/components/AgentFooter.tsx` | Slash command detection + routing |
| `frontend/app/view/agent/agent-view.tsx` | `handleSendMessage` slash routing |
| `agentmux-srv/src/server/app_api.rs` | `agent.command` handler (Phase 2) |
| `agentmux-srv/src/backend/blockcontroller/persistent.rs` | Raw command passthrough |

## Testing

1. Type `/help` → shows help text (not a prompt to Claude)
2. Type `/cost` → shows token usage
3. Type `/login` → triggers OAuth flow
4. Type `/compact` → compacts conversation
5. Type `/clear` → clears the pane
6. Type `/nonexistent` → shows "unknown command" error
