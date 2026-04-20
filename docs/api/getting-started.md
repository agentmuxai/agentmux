# Getting Started with the AgentMux App API

The App API lets external tools and agents control a running AgentMux
host — open agent panes, send messages, read output, and (soon) open
file/view panes. It speaks WSH RPC over a local WebSocket.

This guide shows how to connect and make your first call.

## At a glance

- Transport: WebSocket on `127.0.0.1:{WS_PORT}/ws`
- Auth: `authkey` query param or `X-AuthKey` header
- Envelope: WSH RPC JSON
- Scope: local only, loopback bind, auth-gated

Full command reference: [`docs/specs/app-api-extension.md`](../specs/app-api-extension.md).
Current implementation status: [`docs/specs/app-api-status.md`](../specs/app-api-status.md).

## Discover connection info

AgentMux doesn't publish WS_PORT or AUTH_KEY as fixed values — both
are per-instance. There are two ways to find them.

### Method 1 — parse the host log (works from any shell)

```bash
LOG=$(ls -t ~/.agentmux/logs/agentmux-host-v*.log.* | head -1)

IPC_TOKEN=$(grep "ipc_token=" "$LOG" | tail -1 \
  | sed 's/.*ipc_token=\([^&"]*\).*/\1/')
IPC_PORT=$(grep "IPC HTTP server started" "$LOG" | tail -1 \
  | sed 's/.*127.0.0.1:\([0-9]*\).*/\1/')

AUTH_KEY=$(curl -s \
  -H "Authorization: Bearer $IPC_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"cmd":"get_auth_key","args":{}}' \
  "http://127.0.0.1:$IPC_PORT/ipc" | jq -r .data)

WS_PORT=$(curl -s \
  -H "Authorization: Bearer $IPC_TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"cmd":"get_backend_endpoints","args":{}}' \
  "http://127.0.0.1:$IPC_PORT/ipc" | jq -r '.data.ws | split(":") | last')

echo "ws://127.0.0.1:$WS_PORT/ws?authkey=$AUTH_KEY"
```

The IPC token is the bearer token CEF injects into the page URL. Any
process able to read the AgentMux log can use it to ask the host for
the backend auth key and WebSocket port.

### Method 2 — from inside an agent pane

Agents launched by AgentMux inherit env vars that identify the
instance. You can read them with the same method above (the log path
is stable), or — once shipped — use the planned `agentmux` CLI
wrapper.

## Send your first command

With `WS_PORT` and `AUTH_KEY` in hand, send a WSH RPC envelope:

```json
{
  "wscommand": "rpc",
  "message": {
    "command": "agent.list",
    "reqid": "demo-1",
    "data": {}
  }
}
```

Minimal Node example:

```javascript
import WebSocket from "ws";

const ws = new WebSocket(`ws://127.0.0.1:${WS_PORT}/ws?authkey=${AUTH_KEY}`);

ws.on("open", () => {
  ws.send(JSON.stringify({
    wscommand: "rpc",
    message: { command: "agent.list", reqid: "demo-1", data: {} },
  }));
});

ws.on("message", (buf) => console.log(buf.toString()));
```

You should see a response listing every agent pane currently open.

## Common commands

| Command | What it does |
|---------|--------------|
| `agent.list` | List open agent panes (good first call to verify the connection) |
| `agent.open` | Open a pane running a registered agent |
| `agent.send` | Send a user message to a running agent |
| `agent.status` | Report an agent's state and session ID |
| `agent.output` | Read accumulated output lines |
| `agent.stop` | Stop an agent's controller process |

Request/response shapes for each are in
[`docs/specs/app-api-extension.md`](../specs/app-api-extension.md).

## Security model

- The WebSocket listens on loopback only.
- Every request requires the backend auth key.
- The auth key is rotated per AgentMux process — restarting the host
  invalidates outstanding clients.
- There is no remote/network surface.

If you're writing an integration that runs on the same machine as
AgentMux, this is the path. If you need cross-machine access, wait
for the planned HTTP REST gateway
([`docs/specs/app-api-extension.md`](../specs/app-api-extension.md)
Tier 3).

## Troubleshooting

- **`connection refused`** — AgentMux isn't running, or the log's
  WS_PORT is stale (the host restarted). Re-run the discovery step.
- **`401 unauthorized`** — the auth key rotated (host restart), or
  you pasted the IPC token instead of the backend auth key. These
  are different values.
- **`unknown command`** — check the command name against
  `app-api-status.md`. Tier 1 is implemented; Tier 2+ may not be yet.
- **`CLI_NOT_AVAILABLE` on `agent.open`** — the target provider's CLI
  isn't installed at the expected path. Install it first; see
  `app-api-status.md` for the exact npm command.

## What's next

- Tier 2 (`pane.open` and friends) — draft in
  [`docs/specs/app-api-pane-open.md`](../specs/app-api-pane-open.md).
- Tier 3 HTTP REST gateway and Tier 4 MCP exposure are planned; see
  the extension spec.
- A shell wrapper (`agentmux` CLI) that hides the discovery dance is
  being discussed.
