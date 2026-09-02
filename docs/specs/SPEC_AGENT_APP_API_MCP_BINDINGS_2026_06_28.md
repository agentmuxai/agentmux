# SPEC: Agent App API — MCP is the Agent Entry Point

**Date:** 2026-06-28
**Status:** Draft
**Author:** AgentX
**Related:** `agentmux-mcp/src/main.rs`, `agentmux-srv/src/server/app_api.rs`, `agentmux-srv/src/server/mod.rs` (REST router), `agentmux-srv/src/server/websocket.rs`, `agentmux-srv/src/backend/rpc_types.rs`
**Predecessor:** [`specs/archive/SPEC_AGENT_APP_API_IDENTITY_PRESETS_BRAIN_2026_06_27.md`](./archive/SPEC_AGENT_APP_API_IDENTITY_PRESETS_BRAIN_2026_06_27.md) (shipped the `identity.*` / `preset.*` / `memory.*` RPC handlers)

---

## 1. Problem

The predecessor spec added three agent-facing namespaces to the App API —
`identity.*`, `preset.*`, `memory.*` — gated by the **S1 security invariant**
(`ctx.agent_id` must be non-empty and equal the request's `agent_id`). Those
handlers shipped, are registered in `app_api.rs`, and pass an end-to-end smoke
test (see §9).

But there is a transport gap. An agent running inside a pane reaches AgentMux
through exactly one curated surface: the **MCP tools** that `agentmux-mcp`
advertises (`WhoAmI`, `Layout`, `SetName`, `OpenEditor`, `Shell`, `SendMessage`,
…). The new `identity.*` / `preset.*` / `memory.*` commands have **no MCP
binding and no REST route** — they live only on the WebSocket JSON-RPC surface
that the *frontend* uses.

The consequence: the S1-gated, explicitly *agent-facing* API has no agent-facing
door. To exercise it today an agent must:

1. Scrape `AGENTMUX_LOCAL_URL` and the auth key from its environment,
2. Hand-write a WebSocket client,
3. Perform the `bus:register` handshake (which is what stamps `ctx.agent_id`),
4. Learn the `{ eventtype: "rpc", data: <RpcMessage> }` response wrapping,
5. Frame raw `RpcMessage` envelopes.

That is the verification path used in §9 — useful for testing, wrong as a
contract. `AGENTMUX_AUTH_KEY` is also *not* a PTY env var for trust reasons
(`internals/env-vars.md`: "Available In Pane? No"), so an agent doing the raw-WS
dance only works because this build leaks the key into the shell; the security
model intends agents to reach privileged surfaces **only** through `agentmux-mcp`,
which holds the key out-of-band.

## 2. Goal

Establish **MCP as the canonical, and only supported, entry point** for agents to
reach the App API — and make that true for the new namespaces by adding the
bindings.

Concretely:

1. Add MCP tools for the high-value `memory.*` / `preset.*` / `identity.*`
   operations, routed the same way every other MCP tool is: tool → REST endpoint
   on `$AGENTMUX_LOCAL_URL` → app-API handler.
2. Add the REST routes those tools call, and have them stamp the agent identity
   **server-side from a trusted source** so S1 holds without trusting agent
   input.
3. Make the docs (`internals/agent-app-api.md`) state plainly that MCP is the
   entry point and that the raw WebSocket transport is internal/advanced, not the
   agent contract. (Doc audit is a follow-up task — see §10.)

Non-goal: changing the RPC handlers themselves. They are correct and tested. This
spec is purely about the *transport and entry-point* layer in front of them.

## 3. Background — how an MCP tool reaches a handler today

```
Agent CLI (claude / codex / gemini / …)
  └─ spawns agentmux-mcp (stdio server)         ← holds AGENTMUX_AUTH_KEY + AGENTMUX_AGENT_ID + AGENTMUX_BLOCKID
       ├─ advertises a curated TOOLS array (JSON schemas)
       └─ dispatch: match tool name → POST {AGENTMUX_LOCAL_URL}/api/v1/<route>
                                         with header  X-AuthKey: <key>
                                         → agentmux-srv REST handler
                                         → app-API logic
```

Verified in `agentmux-mcp/src/main.rs`: each tool is (a) a JSON-schema entry in
the `TOOLS` array and (b) a `match` arm that builds an `{AGENTMUX_LOCAL_URL}/api/v1/…`
request with the `X-AuthKey` header. The MCP server is the trust boundary: it is
spawned by AgentMux with the auth key and the agent's identity in its *own*
environment; the agent's model output cannot forge either, because the key is not
in the agent's PTY.

This is the lever that makes server-side identity stamping safe (§5).

## 4. The new App-API commands to bind

All shipped in the predecessor spec; names verified against
`agentmux-srv/src/backend/rpc_types.rs` (`COMMAND_*` constants) and the
`register_*` handlers in `app_api.rs`.

| RPC command | Request shape (App API) | Reads/Writes | Sensitivity |
|---|---|---|---|
| `memory.list` | `{ agent_id }` | read | low |
| `memory.read` | `{ agent_id, filename }` | read | low |
| `memory.write` | `{ agent_id, filename, content }` | write (agent-scoped) | low |
| `preset.list` | `{}` | read (shared catalog) | low |
| `preset.get` | `{ id }` or `{ name }` | read | low |
| `preset.self.get` | `{ agent_id }` | read | low |
| `preset.upsert` | full `Memory` object | write (guards: blank/seed/global) | medium |
| `preset.delete` | `{ id }` | write (guards: blank/seed) | medium |
| `identity.self.accounts` | `{ agent_id }` | read (own links) | medium |
| `identity.account.upsert` | `{ agent_id, provider, name, kind, secret, validate, account_id? }` | write + keychain | **high (secret)** |
| `identity.account.validate` | `{ agent_id, account_id }` or `{ provider, secret }` | read + live probe | **high (secret)** |
| `identity.self.unlink` | `{ agent_id, provider }` | write | medium |

Every command except `preset.*` reads/`identity.account.validate` ad-hoc form
carries an `agent_id` that S1 enforces against `ctx.agent_id`.

## 5. Design — server-side identity stamping (the crux)

The S1 invariant requires `ctx.agent_id` to be set and to match the request's
`agent_id`. On the WebSocket path, `bus:register` stamps `ctx.agent_id`
(`websocket.rs`). The REST path has no such handshake, so we must decide how the
new REST endpoints establish `ctx.agent_id`.

**Decision: the REST endpoint derives the agent identity from a trusted channel,
never from agent-supplied JSON.**

Two trusted inputs are available to `agentmux-mcp` and forwarded on every call:

- `AGENTMUX_BLOCKID` — the pane UUID (already used as self-context by `WhoAmI`).
- `AGENTMUX_AGENT_ID` — the agent slug.

The MCP server injects these itself; the agent's model cannot override them. The
REST handler resolves the canonical agent slug and stamps it:

```
POST /api/v1/agent/memory/write
  X-AuthKey: <key>
  { "block_id": "<AGENTMUX_BLOCKID>", "filename": "...", "content": "..." }

  server:
    1. auth_middleware verifies X-AuthKey
    2. resolve slug = instance_for_block(block_id).instance_name   ← trusted, server-side
    3. construct RpcContext { agent_id: slug, .. }
    4. dispatch to the existing app_api handler with request.agent_id = slug
```

Because the slug is derived server-side from `block_id` (or, equivalently, taken
from a dedicated stamped header), the request's `agent_id` and `ctx.agent_id` are
the *same* value by construction — S1 passes, and an agent cannot target another
agent's records. This is **stronger** than the raw-WS path, where the client
supplies `agent_id` in the payload.

> Implementation note: `block_id → slug` resolution mirrors
> `memory_dir_for_agent`'s lookup. Prefer `block_id` over a stamped slug header so
> a single trusted input drives both identity and (for memory) the working-dir /
> `CLAUDE_CONFIG_DIR` resolution.

### 5.1 Secrets over REST

`identity.account.upsert` / `identity.account.validate` carry an API key in the
request body. This is acceptable on the loopback REST channel for the same reason
the existing tools are: `127.0.0.1` only, `X-AuthKey`-gated, key never logged. But
two rules are mandatory:

- The MCP tool description must **not** instruct the agent to paste a raw key
  inline in a way that ends up in conversation transcripts. Prefer a flow where
  the secret is provided out-of-band (env var name reference, or the Armory
  UI) — see §7 open question.
- Responses **never echo the secret back** — only `masked_tail` (already the
  handler's behavior). The REST layer must not add the plaintext to any response
  or log line.

## 6. MCP tool surface to add

Curated, not a 1:1 of every RPC command. First wave by value and safety:

### 6.1 Memory (highest value, lowest risk) — add first

| MCP tool | Routes to | Params |
|---|---|---|
| `MemoryList` | `memory.list` | none (self) |
| `MemoryRead` | `memory.read` | `filename` |
| `MemoryWrite` | `memory.write` | `filename`, `content` |

`agent_id` is *not* a tool parameter — it is stamped server-side (§5). This is the
autonomous fact-storage surface Claude Code already expects; giving agents a
first-class tool for it is the biggest immediate win.

### 6.2 Presets (read-only first)

| MCP tool | Routes to | Params |
|---|---|---|
| `PresetList` | `preset.list` | none |
| `PresetGet` | `preset.get` / `preset.self.get` | `id?` / `name?`; no args → self |

Mutating preset tools (`preset.upsert` / `preset.delete`) are **deferred** —
see §7.

### 6.3 Identity (most sensitive — gated, possibly read-only)

| MCP tool | Routes to | Params | Status |
|---|---|---|---|
| `IdentityAccounts` | `identity.self.accounts` | none (self) | propose |
| `IdentityValidate` | `identity.account.validate` | `account_id` (own) | propose |
| `IdentityUpsert` | `identity.account.upsert` | provider, name, secretRef | **open — see §7** |
| `IdentityUnlink` | `identity.self.unlink` | provider | **open — see §7** |

## 7. Open questions

1. **Do agents get *mutating* identity/preset tools at all, or stay read + memory-write?**
   Options: (a) read-only identity/preset for agents, mutations human-only via
   Armory; (b) full agent CRUD with the S1 guards already implemented.
   Recommendation: ship §6.1 + §6.2 read tools first; gate §6.3 mutations behind
   an explicit per-agent capability flag before exposing.

2. **Secret-passing ergonomics for `IdentityUpsert`.** Inline key in the tool call
   lands in the transcript. Alternatives: reference an env-var name the MCP server
   resolves; or keep credential creation in the Armory UI and let agents
   only *link* / *validate* existing accounts. Recommendation: no inline-secret
   agent tool in the first wave.

3. **REST route shape.** New `/api/v1/agent/{memory,preset,identity}/*` routes vs.
   a single generic `/api/v1/app-api` that takes `{ command, data }` and stamps
   identity. Recommendation: explicit named routes (consistent with existing
   `/api/v1/*`, easier to allow-list per capability later).

## 8. Implementation plan

1. **REST routes** in `agentmux-srv/src/server/mod.rs` (in `authed_routes`):
   `/api/v1/agent/memory/{list,read,write}`, `/api/v1/agent/preset/{list,get}`,
   and (pending §7) identity routes. Each handler resolves the slug from
   `block_id` server-side, builds `RpcContext { agent_id }`, and calls the
   matching `app_api` handler directly (no WS round-trip).
2. **MCP tools** in `agentmux-mcp/src/main.rs`: add the `TOOLS` JSON-schema
   entries and `match` arms (§6.1, §6.2), each forwarding `AGENTMUX_BLOCKID` +
   the call params with `X-AuthKey`.
3. **No handler changes** — `app_api.rs` logic is reused as-is.
4. **Docs audit** (§10).

## 9. Validation (already performed against 0.49.8)

A raw-WS smoke client (`scripts/test-app-api.mjs`) exercised all twelve commands
end to end as agent `AgentX`:

- `identity.self.accounts` empty → `identity.account.upsert` (dummy, `validate:false`)
  created a keychain-backed account with `masked_tail` → re-list showed the link.
- `identity.account.validate` ad-hoc form live-probed Anthropic and correctly
  returned `invalid` (401) for a dummy key.
- `preset.list` returned the three seed presets + blank singleton; `preset.self.get`
  for an unbound agent returned the blank singleton via the two-step fallback.
- `memory.write` → `memory.list` → `memory.read` round-tripped, and the listing
  resolved to the agent's **isolated** `CLAUDE_CONFIG_DIR` memory dir (it showed
  the agent's real memory files), confirming the path-isolation fix.
- S1 negative: a request with a mismatched `agent_id` was rejected with
  `FORBIDDEN: agent_id mismatch`.

The handlers are correct; this spec adds the *supported* way to call them.

## 10. Docs follow-up (separate task)

`agentmux-docs/src/content/docs/internals/agent-app-api.md` currently states that
identity/memory are "driven by the frontend and internal tooling over the
authenticated WebSocket transport." After §8 lands, audit that page to:

- Add `memory.*` / `preset.*` / `identity.*` to the curated MCP-tool section.
- State explicitly: **MCP is the agent entry point**; the raw WebSocket JSON-RPC
  transport is internal/advanced and not the agent contract.
- Add the new `/api/v1/agent/*` routes to the REST table with their MCP
  equivalents.
- Note server-side identity stamping in the Permission boundary section (agents
  cannot target another agent's records because the slug is server-derived).

## 11. Security summary

- **S1 preserved and strengthened:** identity is stamped server-side from
  `block_id`; agents cannot forge `agent_id`.
- **Trust boundary unchanged:** only `agentmux-mcp` holds the auth key; agents
  reach privileged surfaces solely through it. The raw-WS path is not a supported
  agent contract.
- **Secrets:** never echoed (only `masked_tail`); no inline-secret agent tool in
  the first wave (§7.2).
- **Capability gating:** mutating identity/preset tools deferred behind an
  explicit per-agent flag (§7.1).
