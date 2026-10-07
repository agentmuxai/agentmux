# Spec: Make AgentMux's ACP client speak ACP v1

**Status:** implemented (#4447)
**Date:** 2026-10-07
**Author:** Camper

## Ask

Copilot, OpenClaw and Pi are ACP harnesses (`controllerType: "acp"`). None of
them can hold a conversation in AgentMux today. Make the ACP client, the
`AcpController` in `crates/srv/src/backend/blockcontroller/acp.rs` and the
paths that start it and send it messages, follow ACP v1, so all three work.

## What happens today

Found while testing Pi through `pi-acp` (#4424) in a `task dev` build, and
checked against the ACP SDK's schema (`@agentclientprotocol/sdk` 0.26.0,
`PROTOCOL_VERSION = 1`, `dist/schema/types.gen.d.ts`):

1. **The UI never starts the ACP controller.** `agent-model.ts` writes
   `controller: isPersistent ? "persistent" : "subprocess"` in both launch
   paths. srv creates an `AcpController` only for `controller: "acp"`
   (`blockcontroller/mod.rs`), so a UI-launched ACP agent runs under the
   per-turn subprocess controller. pi-acp then received the startup message
   as plain text lines and rejected each one ("Failed to parse JSON
   message"). Only `agent.open` (app API) writes `"acp"`.
2. **Messages can't reach an ACP controller.** `run_agent_turn`
   (`agentinput`, reactive delivery, fleet broadcast) and `agent.send`
   handle only the persistent and subprocess controllers, and return
   "controller is not a SubprocessController or PersistentSubprocessController"
   for any other.
3. **The handshake isn't ACP.** It sends `initialize` with `clientInfo`,
   `capabilities`, `workspaceRoots` and no `protocolVersion`; an `initialized`
   notification ACP doesn't have; and `session/create` with only `cwd`. ACP
   v1 is `initialize { protocolVersion, clientCapabilities, clientInfo }`, then
   `session/new { cwd, mcpServers }` (or `session/load`).
4. **The prompt has the wrong shape.** `session/prompt` sends
   `prompt: { type, text }`; ACP v1 requires `prompt: ContentBlock[]`.
5. **Requests from the agent go unanswered.** `session/request_permission`
   (and any `fs/*` or `terminal/*` call) gets no response, so an agent that
   asks before running a tool waits forever.
6. **Errors are invisible.** A JSON-RPC error, such as pi's
   `session/new` → "Authentication required", ends the turn bookkeeping, but
   the ACP translator renders nothing for it.

## Change

### 1. Start the ACP controller (frontend)

Both launch paths in `agent-model.ts` write `controller: "acp"` when
`provider.controllerType === "acp"`, else persistent/subprocess as today.

### 2. Route messages to it (srv)

`run_agent_turn` and `agent.send` gain an `AcpController` branch that calls
its `send_input` with the message (as `input_data`), the same entry point
`agent.open`-started panes already use. Session-id hydration and registration
work as for the other controllers.

### 3. The ACP v1 handshake (srv)

- `initialize`: `protocolVersion: 1`, `clientCapabilities: { fs:
  { readTextFile: false, writeTextFile: false }, terminal: false }`,
  `clientInfo: { name: "AgentMux", version }`. No `initialized`.
- After the `initialize` result (not pipelined, so `agentCapabilities` is
  known): `session/load { sessionId, cwd, mcpServers: [] }` when the agent
  reports `loadSession` and the pane has a session id, else `session/new
  { cwd, mcpServers: [] }`. A failed `session/load` falls back to
  `session/new`.
- A prompt sent before the session exists (the startup message is sent right
  after launch) is queued, joined to any earlier queued text, and sent once
  the session opens; it is never sent with an empty session id.
- A `session/load` the agent refuses is not shown in the pane: the client
  recovers by opening a new session, so it is logged, not rendered as an
  error that ended a turn.

### 4. The prompt shape (srv)

`prompt: [{ "type": "text", "text": message }]`, in both places that send
it.

### 5. Answer the agent's requests (srv)

- `session/request_permission`: AgentMux's default is auto-approve, as for
  every other harness (`--dangerously-skip-permissions`, `--yolo`). Select the
  first option of kind `allow_always`, else `allow_once`; with neither,
  respond `{ outcome: { outcome: "cancelled" } }`. The selected option is
  logged.
- Any other request from the agent (`fs/*`, `terminal/*`, unknown): a JSON-RPC
  error `-32601 Method not found`, so the agent never waits on us. The client
  advertises neither capability, so a conforming agent shouldn't ask.

### 6. Show errors (frontend)

The ACP translator renders a JSON-RPC `error` as `**Error:** <message>`, and,
when it ends a prompt, as a turn end.

## Out of scope

- Passing AgentMux's MCP server (`agentmux-mcp`) to the agent in `mcpServers`,
  which would give ACP agents the AgentMux tools. It's a follow-up.
- **Pi's startup instructions.** Pi 1.0.4 loads `.pi/APPEND_SYSTEM.md` only
  for a trusted project, and RPC mode (pi-acp) treats the default "ask" as
  "never", so AgentMux's generated instructions are skipped. The options each
  have a cost: trusting every project (`defaultProjectTrust: "always"`)
  loads any repo's own `.pi` settings and extensions unasked; a saved trust
  decision for the agent's folder does the same for a repo the agent works
  in; sending the instructions in the session instead changes how every ACP
  agent starts. A decision of its own.
- `session/cancel`, modes, and the other optional methods.

## Tests

- srv: the handshake messages and their order (initialize, then new or load
  after its result); the prompt shape; the permission answer (allow_always,
  then allow_once, then cancelled); `-32601` for any other request.
- srv: `run_agent_turn` delivers to an `AcpController`.
- Frontend: launch writes `controller: "acp"` for an ACP provider; the
  translator shows a JSON-RPC error.
- Manual, in a `task dev` build, done on 2026-10-07:
  - A Pi agent (pi-acp 0.0.34, no Pi login) completes the handshake, and the
    pane shows pi's `session/new` refusal: "Error: Authentication required:
    Configure an API key or log in with an OAuth provider", with AgentMux's
    sign-in card.
  - A full turn against the ACP SDK's example agent
    (`@agentclientprotocol/sdk/dist/examples/agent.js`, which validates every
    message with the SDK's schema): streamed reply, two tool calls with
    results, and a `session/request_permission` answered with the allow
    option, after which the agent reported the change applied; the turn ended.
    A second prompt ran on the same session.
  - No Copilot account exists on the test machine, so Copilot itself was not
    run end to end.

## Seen while testing, not changed

- The example agent reuses the same tool-call ids (`call_1`, `call_2`) every
  turn, and the second turn's tool rows did not render as new rows. Real
  agents (pi-acp, Copilot) use unique ids. The translator already clears its
  own bookkeeping at each turn end; the pane's node map is a separate,
  provider-independent question.
