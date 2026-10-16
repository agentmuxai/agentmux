# Getting Started with MuxAPI

MuxAPI, AgentMux's own API (formerly called the App API), lets agents and local tools control a running AgentMux: open
agent and other panes, send messages, read output, arrange tabs and
windows, run shells, and more. It has three faces over one server:

| Surface | Who uses it | Where |
|---|---|---|
| **MCP server** (`agentmux-mcp`) | Agents. It is added to every agent's MCP config automatically, with 23 tools (`Terminal`, `Browser`, `UI`, `Memory`, `Tasks`, `Presence`, `Schedule`, `Pane`, `Stop`, `Widget`, `Identity`, `SendMessage`, `OpenAgent`, …), being consolidated into one tool per family with an `action` ([`SPEC_MUXAPI_TOOL_CONSOLIDATION_2026_10_15.md`](../specs/SPEC_MUXAPI_TOOL_CONSOLIDATION_2026_10_15.md)). | [`crates/mcp/src/tools/`](../../crates/mcp/src/tools) |
| **HTTP** (`/api/v1/…`, `/agentmux/…`) | Shell helpers and scripts. About 100 `/api/v1` routes: panes, agents, tabs, windows, shells, memory. | [`crates/srv/src/server/routes.rs`](../../crates/srv/src/server/routes.rs) |
| **WebSocket RPC** (`/ws`) | The frontend, and tools that want push events. 250 commands, including the intent layer (`agent.open`, `agent.send`, `pane.open`). | [`app-api-extension.md`](../specs/app-api-extension.md) |

This guide shows how to connect and make a first call on each.

## At a glance

- Base URL: `AGENTMUX_LOCAL_URL` (for example `http://127.0.0.1:29700`); the WebSocket is `/ws` on the same port.
- Auth: the `X-AuthKey` header on every HTTP request. The `?authkey=` query parameter is accepted on `/ws` only.
- The auth key is new every time AgentMux starts, so restarting it invalidates every client.
- Scope: loopback by default. See [Security model](#security-model) for the opt-in LAN and paired-device listeners.

## Find the URL and key

AgentMux puts both in the environment of the agents it starts:

| Variable | In agent panes | In terminal panes |
|---|---|---|
| `AGENTMUX_LOCAL_URL` | yes | yes |
| `AGENTMUX_AUTH_KEY` | yes | no |
| `AGENTMUX_BLOCKID`, `AGENTMUX_AGENT_ID` | yes | — |

So run your tool from an agent pane, or have an agent run it. A container
agent gets a URL that points at `host.docker.internal` and a scoped key
that only opens the routes containers may use.

There is no supported way to read the key from outside AgentMux: the host
log used to carry a token that unlocked it, and now redacts it.

## First call over HTTP

```bash
curl -s -H "X-AuthKey: $AGENTMUX_AUTH_KEY" "$AGENTMUX_LOCAL_URL/agentmux/discovery"
```

It returns `host` (version, hostname, channel, the agents on this
instance), `lan` and `wan`. Without the header you get `401` and
`{"error":"unauthorized"}`.

Opening a pane:

```bash
curl -s -X POST -H "X-AuthKey: $AGENTMUX_AUTH_KEY" -H "Content-Type: application/json" \
  -d '{"view":"editor","file":"/path/to/file.md"}' \
  "$AGENTMUX_LOCAL_URL/api/v1/pane/open"
```

## First call over the WebSocket

Send a WSH RPC envelope:

```json
{
  "wscommand": "rpc",
  "message": { "command": "agent.list", "reqid": "demo-1", "data": {} }
}
```

Node 22 or later has `WebSocket` built in:

```javascript
const url = process.env.AGENTMUX_LOCAL_URL.replace(/^http/, "ws")
  + "/ws?authkey=" + process.env.AGENTMUX_AUTH_KEY;
const ws = new WebSocket(url);

ws.onopen = () => ws.send(JSON.stringify({
  wscommand: "rpc",
  message: { command: "agent.list", reqid: "demo-1", data: {} },
}));

ws.onmessage = (e) => {
  const msg = JSON.parse(e.data);
  // Replies arrive as { eventtype: "rpc", data: { resid, data } }.
  if (msg.data?.resid === "demo-1") { console.log(msg.data.data); ws.close(); }
};
```

You should see every agent pane currently open. The socket also carries
events the app publishes, so expect other messages besides your reply.

## Common commands

| Command | What it does |
|---------|--------------|
| `agent.list` | List open agent panes (a good first call) |
| `agent.open` | Open a pane running a registered agent |
| `agent.send` | Send a user message to a running agent |
| `agent.status` | Report an agent's state and session ID |
| `agent.output` | Read accumulated output lines |
| `agent.stop` | Stop an agent's controller process |
| `pane.open` | Open an editor, terminal, browser or other pane, split against a pane, floating, or in a given tab |
| `pane.moveTab` | Move a tab into another pane |

Request and response shapes: [`docs/specs/app-api-extension.md`](../specs/app-api-extension.md).
What is built and what isn't: [`docs/specs/app-api-status.md`](../specs/app-api-status.md).

## From the shell

AgentMux installs small helpers in every terminal and agent shell. They
read the same two variables:

| Command | What it does |
|---|---|
| `muxsh` | Open files, URLs and panes, list panes, run commands, and send to agents ([docs/MUXSH.md](../MUXSH.md)) |
| `muxopen <agent>` | Open an agent's pane, or focus it if it is already open |
| `muxspect` | Inspect a running instance live ([docs/MUXSPECT.md](../MUXSPECT.md)) |
| `muxlog` | Find and follow logs across instances ([docs/MUXLOG.md](../MUXLOG.md)) |

## Security model

- Every listener AgentMux opens at startup binds to loopback.
- Every request needs the instance's auth key, and the key changes on each start.
- **LAN access is opt-in** (`network:lan_discovery`). It adds a listener on each
  network interface, gated by a separate per-process LAN key that opens only the
  routes peers need (message delivery and fleet status), and announces the
  instance over mDNS. Headless mode never enables it.
- **Paired devices** (Settings → Paired devices) use their own TLS listener and
  a per-device token, and can only read agent feeds. The instance key and the
  LAN key don't work there.

## Troubleshooting

- **`connection refused`** — AgentMux isn't running, or it restarted and the
  port changed. Read `AGENTMUX_LOCAL_URL` again from a pane started after the restart.
- **`401 unauthorized`** — the key is missing or stale (AgentMux restarted), or
  you put it in the query string of an HTTP route; only `/ws` reads it there.
- **`403`** — a container agent's scoped key, on a route containers may not use.
- **`unknown command: <name>`** — the RPC name is wrong; check it against
  [`app-api-extension.md`](../specs/app-api-extension.md).
- **`CLI_INSTALLING` on `agent.open`** (`503` over HTTP) — the provider's CLI is
  being installed (first use, or just after an upgrade moved its pin). Retry in a
  minute; the install carries on in the background.
- **`CLI_NOT_AVAILABLE` on `agent.open`** — the provider's CLI could not be
  installed (no `npm`, offline, or a provider with no npm package). The error
  says why; fix that and retry.
