# net-fault: make the network refuse, blackhole or flap

A local HTTP/HTTPS (CONNECT) proxy for checking how AgentMux and its agent CLIs
behave when the internet goes away, without touching the machine's real network.
An isolated dev instance and every CLI it starts are pointed at it with
`HTTPS_PROXY`; its mode is switched at run time. It is the fault injection of
[`SPEC_OFFLINE_RESILIENCE_2026_10_20.md`](../../docs/specs/SPEC_OFFLINE_RESILIENCE_2026_10_20.md) §7.1,
and the checklist below is that spec's §7.2.

| File | |
|---|---|
| `proxy.mjs` | the proxy and its control endpoint |
| `ctl.mjs` | switches the mode of a running proxy |
| `modes.mjs` | mode parsing, host rules and flap timing (no sockets) |
| `net-fault.test.mjs` | unit tests, local sockets only (`npx vitest run scripts/net-fault/`) |

Node only, no dependencies.

## Run it

```bash
node scripts/net-fault/proxy.mjs                 # proxy on 127.0.0.1:8899, control on :8898
node scripts/net-fault/proxy.mjs --port 18899 --control-port 18898 --mode refuse
```

Point clients at it:

```bash
export HTTPS_PROXY=http://127.0.0.1:8899 HTTP_PROXY=http://127.0.0.1:8899
export NO_PROXY=127.0.0.1,localhost,::1
```

`NO_PROXY` keeps loopback traffic (srv's own API, Vite, MCP servers) off the
proxy. Add your LAN's address ranges (srv accepts CIDR there) if you test LAN discovery and
want it untouched; otherwise LAN requests go through the proxy and follow its
mode too.

The proxy prints every decision on stdout:

```
21:13:41.902 CONNECT api.anthropic.com:443 [refuse] -> reset
21:14:02.311 now: default=pass api.anthropic.com=refuse
```

Times are local. Redirect stdout to a file if you start it in the background.

## Switch modes

```bash
node scripts/net-fault/ctl.mjs refuse                          # every host
node scripts/net-fault/ctl.mjs refuse --host api.anthropic.com # one host (glob: * and ?)
node scripts/net-fault/ctl.mjs flap:30s/20s                    # 30 s up, 20 s refused, repeat
node scripts/net-fault/ctl.mjs flap:30s/20s:blackhole          # ... or blackholed while down
node scripts/net-fault/ctl.mjs pass
node scripts/net-fault/ctl.mjs status
node scripts/net-fault/ctl.mjs clear                           # drop every --host rule
```

Without `--host` the command sets the default mode; with it, the rule for that
glob (a rule for the same glob is replaced). Rules are tried in the order they
were added; the first match wins; anything unmatched gets the default. So "API
down, everything else up" is `refuse --host api.anthropic.com`, and "everything
down except one service" is `refuse` plus `pass --host <that service's host>`.
To find a service's hosts, run in `pass` and read the log: every CONNECT names
its host.

Set `NET_FAULT_CONTROL=http://127.0.0.1:<control-port>` (or `--control`) when the
control port is not 8898. The same API over HTTP: `GET /status`,
`POST /mode` with `{"mode":"refuse","host":"..."}` (or `?mode=&host=`),
`POST /clear`.

## What each mode looks like to a client

Measured on Windows with curl and Claude Code 2.1.112 (`claude -p`), the API
host set to each mode. `error` is the `api_retry` frame's field; "final" is the
turn's result once retries run out.

| Mode | The proxy | curl | Claude CLI |
|---|---|---|---|
| `pass` | tunnels | normal | normal |
| `refuse`, some hosts | resets (RST) the CONNECT | `Recv failure: Connection was reset` | `error: "unknown"`; final `API Error: Unable to connect to API (ECONNRESET)` |
| `refuse`, every host | stops listening, so the connect itself is refused | `Failed to connect … Could not connect to server` (after 2 s on Windows) | `error: "unknown"`; final `API Error: Unable to connect to API (ConnectionRefused)` |
| `blackhole` | accepts and never answers | hangs until its timeout | `error: "unknown"`; final `Request timed out` |
| `dns` | answers the CONNECT with `502` and the body `getaddrinfo ENOTFOUND <host>` | `CONNECT tunnel failed, response 502` | `error: "server_error"`; final `API Error: 502 getaddrinfo ENOTFOUND <host>` |
| `flap:<up>/<down>` | `pass` for `<up>`, then the down mode (default `refuse`) for `<down>`, repeating, starting up | | |

Limits, said plainly:

- **A real `ECONNREFUSED` needs every host refused.** A proxy that is listening
  can only reset a connection after accepting it, so a per-host `refuse` is an
  `ECONNRESET`. When the default and every rule refuse, the proxy closes its
  listening socket and clients get a true connection refused; it listens again
  as soon as anything passes.
- **`dns` is not a real name-resolution failure.** Through a proxy the client
  never resolves the target; the proxy does. The 502 carries the text a Node
  lookup failure prints, but the Claude CLI reports it as an HTTP 502 from the
  API (`server_error`), not as a network error. For a true `ENOTFOUND`, start the
  client with `HTTPS_PROXY` naming a host that does not resolve
  (`http://net-fault.invalid:8899`); that cannot be switched at run time.
- **A mode change applies to open connections too.** On a switch to `refuse` or
  `dns` the proxy resets the tunnels it holds for the affected hosts; on a
  switch to `blackhole` it stops forwarding them without closing them (a
  stalled connection), and a switch back to `pass` lets them flow again, data
  intact. A CONNECT left unanswered by `blackhole` is completed when its host
  goes back to `pass`, like a retransmitted SYN.
- **Short retries.** `CLAUDE_CODE_MAX_RETRIES=2` and `API_TIMEOUT_MS=5000` in the
  CLI's environment make it give up in seconds instead of minutes, which is
  handy when checking the end of a failed turn rather than the retry row.

## Which clients use the proxy

- **Claude CLI: yes.** Verified: with the API host refused, every request
  showed up in the proxy log as `CONNECT api.anthropic.com:443` and failed as in
  the table. Agent panes inherit srv's environment; srv strips only
  `AGENTMUX_*` variables from it (`crates/srv/src/backend/pane_env.rs`), so
  `HTTPS_PROXY` and `NO_PROXY` set on the dev instance reach every agent CLI it
  starts. Other CLIs (Codex, Gemini, …) were not checked.
- **srv's HTTP clients: yes, read once.** srv uses reqwest 0.12, which reads
  `HTTPS_PROXY`/`HTTP_PROXY`/`ALL_PROXY`/`NO_PROXY` when a client is built, unless
  the builder calls `.no_proxy()`. Two do, and so bypass the proxy:
  `backend/viewer/client.rs` and `backend/widget_net.rs` (a widget's
  `am.net.fetch`). Everything else (MuxBus token refresh, discovery and REST,
  update checks, provider key checks, LAN discovery, …) goes through it.
  Verified: a dev instance started with the variables sent its MuxBus HTTP
  request through the proxy as a CONNECT on startup. Many
  clients are built once and kept, so set the variables before the instance
  starts; switching the proxy's mode afterwards is what changes behaviour.
- **srv's WebSockets: no.** `tokio-tungstenite` connects directly and ignores
  proxy variables. That covers MuxBus's live connection
  (`muxbus/cloud_subscriber.rs`), Slack Socket Mode and the Discord gateway. In
  the proxy's `refuse` mode MuxBus's token refresh fails but an already open
  MuxBus WebSocket stays up, and a reconnect still reaches the internet. "MuxBus
  down" therefore cannot be produced by this proxy alone today.
- **The window (CEF/Chromium): not checked.** On Windows Chromium takes its
  proxy from the system settings, not from these variables. The window talks to
  the local srv, which is not affected.

Proposed dev-only switch for the WebSockets (not part of this change): when
`HTTPS_PROXY` is set and the target is not in `NO_PROXY`, open the TCP stream to
the proxy, send `CONNECT host:443`, wait for the `200`, and hand that stream to
`tokio_tungstenite::client_async_tls_with_config` instead of calling
`connect_async_tls_with_config`. About thirty lines in one helper used by the
three call sites; it would also make MuxBus work behind a real corporate proxy.
Until then, `AGENTMUX_MUXBUS_WS_URL=wss://127.0.0.1:9/` at launch makes the
MuxBus WebSocket fail to connect (connection refused) for the whole session.

## An isolated dev instance behind the proxy

Never point the instance you are working in, or any instance the operator
uses, at the proxy. Use your own `task dev` build with its own data folder
(the launch steps, port check and cleanup are in the `run` skill,
`.claude/skills/run/SKILL.md`):

```bash
# Start the proxy in its own terminal (or in the background with stdout to a file).
node scripts/net-fault/proxy.mjs --port 18899 --control-port 18898
# Then, in the clone, with a scratch folder as the AgentMux home (a Windows
# path on Windows), deleted afterwards:
AGENTMUX_HOME_OVERRIDE="<scratch folder>" \
HTTPS_PROXY=http://127.0.0.1:18899 HTTP_PROXY=http://127.0.0.1:18899 \
NO_PROXY=127.0.0.1,localhost,::1 \
  task dev TITLE="net-fault"
export NET_FAULT_CONTROL=http://127.0.0.1:18898   # for ctl.mjs
```

- `AGENTMUX_HOME_OVERRIDE` replaces `~/.agentmux` for that instance only: it
  starts with no agents, no layout and no MuxBus sign-in, and none of the
  operator's.
- Keep the proxy in `pass` until the window is up and the agent CLIs are
  installed. `task dev`'s own `npm` and the in-app installer of an agent CLI
  inherit the same variables and go through the proxy.
- **A new agent there needs a sign-in before it sends anything.** With none,
  the pane shows "Claude Code needs you to sign in" and the CLI never reaches
  the API, so no fault mode shows anything. An API key added in Connectors →
  Accounts is stored in the OS keychain, which `AGENTMUX_HOME_OVERRIDE` does
  not isolate, so decide with the operator which account the scratch instance
  may use before running the agent scenarios.

When done: stop your own instance's processes by PID (launcher first), stop
the proxy, and delete the scratch folder.

## Scenario checklist (SPEC §7.2)

Status as of this change: PRs A (#5213), B (#5221), C (#5215) and D (#5236) of
the spec's §8 are merged, E (connectivity state, status-bar Offline, held
input, automatic retry) and F (MuxBus resubscribe) are not. **Now** = checkable
today; **E** / **F** = the expected observation needs that PR.

Setup for each: the isolated instance above, three agents in conversation,
the proxy in `pass`.

### 1. `refuse` for 15 min, then `pass`

`ctl.mjs refuse`, send each agent a message, wait, then `ctl.mjs pass`.

| Check | Expect | When |
|---|---|---|
| G1 | Stop (or Esc) on a retrying agent ends the turn within 2 s | Now (C) |
| G2 pane | the working row says the API can't be reached and shows the attempt, of how many, and the delay, with Stop | Now (C) |
| G2 status bar | **Offline** within 10 s, for the model API | E |
| G3 | a message typed into a retrying pane is shown as held, with Send now and Cancel | E |
| G4 | no agent starts a turn that you did not send; `[AgentMux]` notes about MuxBus wait for the next turn | Now (B) |
| G5 | MuxBus shows Offline, not "needs sign-in"; after `pass` it reconnects without a sign-in | Now (A); see the WebSocket caveat above |
| G6 | after `pass`: held messages are sent in order, turns that failed only on the network are retried once, one "back online" note per agent | E |
| G6 MuxBus | every live agent is subscribed again, under every name | F |
| End of a failed turn | the turn ends as a network error with Retry, not an unknown failure | Now (C), but see below |

The final error of a refused turn depends on the CLI build: 2.1.112 ends it
with `Unable to connect to API (ConnectionRefused)` when the connect itself is
refused (the proxy's every-host `refuse`), and `crates/srv/src/agents/failure.rs`
does not match that text today (it matches `connection refused` with a space,
and `econnrefused`). Expect an unknown failure there until it does; the
`ECONNRESET` and `Request timed out` endings are matched.

### 2. `blackhole` for 3 min

`ctl.mjs blackhole`, then `pass`. Same checks as 1, with timeouts instead of
refusals: the retry row appears only after the CLI's request timeout
(`API_TIMEOUT_MS`; set it lower to see it sooner). Open tunnels stall
rather than close. **Now** for G1, G2 pane, G4, G5; the rest as in 1.

### 3. `flap:30s/20s` for 10 min

`ctl.mjs flap:30s/20s`, then `pass`.

| Check | Expect | When |
|---|---|---|
| no needs-reauth | MuxBus never asks for a sign-in | Now (A) |
| no duplicate turns | each message you sent is answered once | Now |
| settles | the status bar ends on Online | E |

### 4. API `refuse`, MuxBus `pass`

`ctl.mjs refuse --host api.anthropic.com`.

| Check | Expect | When |
|---|---|---|
| MuxBus stays connected | delivery status Connected; a review notice still arrives | Now (the WebSocket does not use the proxy, so this holds either way) |
| only the API is Offline | the status bar names the API, not MuxBus | E |

The reverse (MuxBus down, API up) needs the WebSocket switch proposed above,
or the launch-time `AGENTMUX_MUXBUS_WS_URL` workaround.

### 5. Sign out while `refuse`

`ctl.mjs refuse`, wait until MuxBus shows Offline, then Sign out.

| Check | Expect | When |
|---|---|---|
| confirmation | Sign out asks first and says you can't sign in again until you're back online | Now (A) |
| Sign in disabled | Sign in is disabled, with the reason | Now (A) |

### 6. Close the last window during `refuse`, relaunch

| Check | Expect | When |
|---|---|---|
| G8 layout | the workspace (tabs, panes, agents) comes back; agents can be resumed | Now: this is the check itself (spec §3 G8). D (#5236) covers offering a layout another build saved |
| held messages restored | held input survives the restart | E |

### 7. No network at all

Not done with the proxy: disable the network adapter (or unplug), so nothing
reaches anything off the machine.

| Check | Expect | When |
|---|---|---|
| G7 | same-host `Message` send, the Terminal, Stop, panes and layout work as before | Now |
