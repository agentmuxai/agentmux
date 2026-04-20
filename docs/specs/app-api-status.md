# App API — Implementation Status

**Date:** 2026-04-11
**Spec:** `docs/specs/app-api-extension.md`
**Retro:** `docs/analysis/persistent-process-retro-2026-04-10.md`

---

## What's Working (Verified End-to-End)

### Backend RPC Commands (WebSocket)

All Tier 1 commands register on the WSH RPC engine and respond correctly:

| Command | Status | Verified |
|---------|--------|----------|
| `agent.open` | Working | Creates block, sets metadata, writes config files, registers controller, inserts layout node |
| `agent.send` | Working | Sends message via persistent stdin or subprocess spawn |
| `agent.stop` | Working | Stops controller process |
| `agent.status` | Working | Returns agent state, session ID, exit code |
| `agent.list` | Working | Lists all agent panes across tabs |
| `agent.output` | Implemented | Reads broker event history (needs persist > 0 for blockfile events) |
| `pane.open` | Implemented | Creates a block for view `editor`/`term`/`browser`/`sysinfo`/`help`, optionally split-placed against a reference block. MVP: no idempotency, no new-tab placement, no path sandboxing — see `app-api-pane-open.md`. |

### WebSocket Protocol

Commands are sent via the WSH RPC envelope:
```json
{
  "wscommand": "rpc",
  "message": {
    "command": "agent.open",
    "reqid": "unique-id",
    "data": { "agent_id": "agentx" }
  }
}
```

Auth via query param: `ws://127.0.0.1:{WS_PORT}/ws?authkey={AUTH_KEY}`

### Getting Auth Credentials

```bash
# 1. Read IPC token from host log (injected into page URL)
TOKEN=$(grep "ipc_token=" ~/.agentmux/logs/agentmux-host-v*.log.* | tail -1 | sed 's/.*ipc_token=\([^&"]*\).*/\1/')
IPC_PORT=$(grep "IPC HTTP server started" ~/.agentmux/logs/agentmux-host-v*.log.* | tail -1 | sed 's/.*127.0.0.1:\([0-9]*\).*/\1/')

# 2. Get backend auth key via CEF IPC
AUTH=$(curl -s -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"cmd":"get_auth_key","args":{}}' "http://127.0.0.1:$IPC_PORT/ipc" | jq -r .data)

# 3. Get WebSocket port
WS_PORT=$(curl -s -H "Authorization: Bearer $TOKEN" -H "Content-Type: application/json" \
  -d '{"cmd":"get_backend_endpoints","args":{}}' "http://127.0.0.1:$IPC_PORT/ipc" | jq -r '.data.ws | split(":") | last')
```

### Persistent Process Pipeline

The full flow works:
1. `agent.open` → block created, persistent controller registered
2. `agent.send` → CLI spawned with `--input-format stream-json`, stdin message written
3. Claude Code responds, session ID captured, stdout published to WPS blockfile
4. Process stays alive for multi-turn (no `CLAUDE_CODE_EXIT_AFTER_STOP_DELAY`)

---

## Verified Working End-to-End (as of 0.33.91)

All three initial blockers were resolved:

| Issue | Fix | Version |
|-------|-----|---------|
| Pane not visible after `agent.open` | Use `LayoutState.pendingbackendactions` — frontend's layout model picks up insert actions via its reactive `createEffect` (same mechanism as cross-window drag) | 0.33.83 |
| Agent response not rendering | `claude-translator.handleAssistantMessage` now emits text/thinking/tool_use StreamEvents. Previously returned `[]` on the assumption that `stream_event` deltas would arrive first, but in persistent mode they don't | 0.33.84 |
| CLI auto-install | Still a limitation — caller must pre-install via `npm install --prefix ~/.agentmux/<version>/cli/claude @anthropic-ai/claude-code@latest` before calling `agent.open` | — |

**Confirmed working:**
- `agent.open` → block created, layout inserted, pane visible in UI
- `agent.send` → persistent process spawned with correct args, message delivered
- Claude responds → stdout → WPS blockfile → `useAgentStream` → rendered text
- Multi-turn persistent conversation with preserved session ID

## Known Remaining Limitations

### 1. CLI Auto-Install

`agent.open` returns `CLI_NOT_AVAILABLE` if the npm package isn't installed. Workaround: pre-install before calling `agent.open`.

**Fix planned:** Add an `auto_install: bool` field to the `agent.open` request (default `true`) that triggers the same npm install logic used by the frontend's launch flow.

### 2. Slash Commands

Interactive commands (`/login`, `/help`, `/clear`) don't work natively in stream-json mode — the CLI excludes them from the `slash_commands` array in the `system.init` event. Frontend intercepts `/login` and `/clear` in `handleSendMessage`:

- `/login` → calls `runCliLogin` IPC → spawns separate CLI process → captures OAuth URL → displays in URL box with hover-to-copy button (shipped 0.33.89)
- `/clear` → resets document signal (frontend-only, shipped 0.33.88)

Other slash commands (`/cost`, `/compact`, etc.) pass through to the CLI which handles them as synthetic assistant messages (zero tokens).

---

## Files Added

| File | Lines | Purpose |
|------|-------|---------|
| `agentmux-srv/src/backend/providers.rs` | ~270 | Static provider registry (claude, codex, gemini) with 7 unit tests |
| `agentmux-srv/src/backend/agent_config.rs` | ~330 | Config file builder (CLAUDE.md, .mcp.json, skills) with 9 unit tests |
| `agentmux-srv/src/server/app_api.rs` | ~530 | All 6 command handlers |
| `docs/specs/app-api-extension.md` | ~280 | Full API spec (Tiers 1-5) |
| `docs/specs/app-api-status.md` | this file | Implementation status |
| `docs/analysis/persistent-process-retro-2026-04-10.md` | ~150 | Debug retro (11 issues found and fixed) |

## Files Modified

| File | Change |
|------|--------|
| `agentmux-srv/src/backend/rpc_types.rs` | 7 command constants + 13 request/response structs |
| `agentmux-srv/src/backend/mod.rs` | `pub mod providers; pub mod agent_config;` |
| `agentmux-srv/src/server/mod.rs` | `mod app_api;` |
| `agentmux-srv/src/server/websocket.rs` | `register_app_api_handlers()` call |
| `agentmux-srv/src/backend/blockcontroller/persistent.rs` | `controller_type_str()` method |
| `agentmux-srv/src/backend/providers.rs` | `controller_type_str()` impl |

---

## Next Steps

1. **Fix pane visibility** — frontend `layoutaction` event handler (Option A)
2. **Fix agent response rendering** — debug `useAgentStream` with visible pane
3. **CLI auto-install** — integrate npm install into `agent.open`
4. **Tier 2 commands** — `pane.open`, `pane.close`, `pane.list`, `pane.focus`
5. **HTTP REST gateway** — `/api/v1/agent/*` endpoints for external tools
6. **MCP integration** — expose App API as MCP tools in the agentmux MCP server
