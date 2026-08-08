# Spec: Messaging App Integration — Discord POC

**Date:** 2026-06-24  
**Status:** Draft (superseded — see note below)
**Scope:** Research + POC design for extending AgentMux integrations to messaging apps, starting with Discord

> **2026-08-07 audit note:** Implemented same day (PR #1763) and substantially
> extended since (`agentmux-srv/src/messaging/discord/{gateway,rest,mod,types}.rs`,
> through 2026-07-29). This POC's "no cloud server" framing no longer matches
> the shipped architecture. See `docs/reports/REPORT_DOCS_AND_DEAD_CODE_CLEANUP_AUDIT_2026_08_07.md`.

---

## 1. Context

AgentMux has two existing integration touchpoints relevant to messaging:

**OpenClaw** (in `integration-vision.md`) is already designed as the external-channel bridge. Its ACP (Agent Communication Protocol) supports Discord, Telegram, Slack, Signal, iMessage, and WhatsApp as named channel types. The OpenClaw widget exists (`widgets.json`) and embeds the OpenClaw gateway as a WebView on `localhost:18789`, but the backend implementation is not yet active.

**OAuth** already has Slack in `oauth_client.rs` (authorization code + PKCE flow for user identity). Discord OAuth is not yet wired up but Slack's presence confirms the pattern is understood.

**Grafana context** (what prompted this spec): the Grafana integration analogy the user cited refers to the iframe-embedding pattern used in the browser widget — externally hosted UIs surfaced in pane tiles. This is Phase 1 of the browser widget; no custom Grafana protocol work was needed. Messaging apps require more than iframe embedding because bidirectional communication (agent sends → user replies → agent receives) requires a protocol bridge, not just a WebView.

The goal: **POC that proves end-to-end bidirectionality between an AgentMux agent and a Discord channel, with no cloud server requirement.**

---

## 2. Stability Assessment of Existing Integration Surfaces

### 2.1 OpenClaw Integration

**Status: Planned but not yet active.**

- OpenClaw widget is defined in `widgets.json` but the gateway process is not shipped or started
- The ACP channel types (including Discord) are specified in `integration-vision.md` but have no implementation code in this repo
- Stability: unknown — depends entirely on OpenClaw's roadmap

**Assessment:** Cannot be relied on for a near-term POC. Worth building the AgentMux side to be OpenClaw-ready (use the same abstraction), but the POC must work without it.

### 2.2 Reactive Message Bus

**Status: Shipping and stable.** 4-tier architecture (local → HTTP loopback → LAN mDNS → cloud relay) is in production. The AgentMux MCP tools (`send_message`, `broadcast`, `list_agents`) are the stable inter-agent communication layer.

**Assessment:** This is the correct internal bus. Discord messages, once received, should flow into the reactive bus so any agent can consume them via `read_messages` — not just the one that opened the Gateway connection.

### 2.3 OAuth (Slack present, Discord absent)

**Status: Slack OAuth is wired. Discord is not.**

Discord OAuth uses the same RFC 8252 + PKCE pattern as Slack. Adding Discord to `oauth_client.rs` is a straightforward config addition. The bot-add flow doesn't require OAuth at all — it's a direct redirect with `scope=bot+applications.commands`.

**Assessment:** Low-friction to add Discord OAuth when needed. Not required for the Gateway-bot POC (bot token stored in keychain is sufficient).

---

## 3. Discord API — Technical Assessment

### 3.1 What to use (and what not to)

| Mechanism | Use for | Don't use for |
|---|---|---|
| **Gateway WebSocket** | Receiving channel messages and slash command events | Anything that can be done via REST |
| **REST API** | Sending messages, registering slash commands | Event delivery |
| **Webhooks** | One-way notification pipelines (CI alerts, agent status) | Bidirectional use cases |

**For an AI agent POC, the minimum viable surface is:** Gateway (receive) + REST (send). Webhooks alone are a dead end for bidirectionality.

### 3.2 Intents and permissions

The **Message Content intent** (`MESSAGE_CONTENT`) is privileged. For a private bot under 100 servers (which a personal agent deployment always will be), it can be enabled directly in the Discord Developer Portal with no review required. This covers 100% of personal/team agent deployments.

Without it: the `content` field on `MESSAGE_CREATE` events is empty. Slash commands are the alternative — they carry user input as structured parameters without needing the intent.

**Recommendation:** Enable `MESSAGE_CONTENT` in Developer Portal for the POC. Build slash commands as the primary interaction pattern for the shipped integration (avoids any review friction at scale).

### 3.3 Gateway stability (known failure modes)

1. **Heartbeat ghost state**: The process is alive but the bot stops receiving events. Requires external health monitoring; discord.js v14 has documented cases where heartbeat fails silently.
2. **Session resumption**: On reconnect, always attempt RESUME with `session_id` + last `seq` using the `resume_gateway_url` from the READY payload. Fallback to re-IDENTIFY with exponential backoff.
3. **App suspend (desktop-specific)**: When the laptop sleeps, the Gateway WS disconnects. The session is usually resumable if under ~60 seconds. If longer, re-IDENTIFY required. Buffer user-visible "reconnecting" state.
4. **Identify rate limit**: 1 IDENTIFY per 5 seconds per shard. Violating → 4008 close code.

**Mitigation**: Let discord.js handle reconnection (it implements resume + re-identify correctly since v14.x). Add health check: if `client.ws.ping` returns −1 or a timeout, force-reconnect. For a desktop app, wrap in a `createEffect` that reconnects when app regains focus after a suspend.

### 3.4 API stability (versioning)

Discord API v10 has been stable since 2022 with no v11 announced. Breaking changes within v10 are infrequent. One notable upcoming break: `PIN_MESSAGES` permission was split from `MANAGE_MESSAGES` in August 2025, effective February 2026. No impact on the POC (we're not pinning messages).

`discord-api-types` publishes versioned sub-paths (`discord-api-types/v10`) that isolate API version changes. Pin to `~0.37.x` in production; upgrade deliberately.

### 3.5 Slash commands vs. message-based commands

**Use slash commands.** Discord's September 2022 policy makes `MESSAGE_CONTENT` privileged and the approval bar for verified bots is high. Slash commands require no privileged intents, have structured input via Discord's interaction payload, and support autocomplete. For a personal agent, the POC can use `MESSAGE_CONTENT` (enabled without review for private bots), but the shipped feature should be slash-command-first.

---

## 4. Comparison: Discord vs. Slack

| Dimension | Discord Gateway | Slack Socket Mode | Verdict for AgentMux |
|---|---|---|---|
| Persistent WS required? | Yes | Yes (for no-server) | Tie |
| HTTP event delivery option? | No (interactions endpoint only, needs public URL) | Yes (HTTP Events API) | Slack wins for serverless |
| Stability risk | Heartbeat ghost state documented | Generally more stable (HTTP option) | Slack slightly more robust |
| Message formatting | Rich embeds + components | Block Kit (more expressive) | Slack richer for structured output |
| Community/bot ecosystem | Larger, gaming-focused | Business-focused | Discord better for personal agent UX |
| Bot setup friction | Very low (Developer Portal) | Moderate (app configuration) | Discord wins |
| User familiarity (dev community) | High | High | Tie |
| OpenClaw support | Planned | Planned | Tie |

**Decision for POC: Discord** — lower setup friction, OpenClaw already names it as a target channel type, and the user explicitly requested it.

**Future Slack support**: The Slack Socket Mode architecture is nearly identical to Discord Gateway from the code's perspective. Once the Discord adapter is working, adding a Slack adapter is a configuration delta, not a rewrite.

---

## 5. POC Architecture

### 5.1 Topology

```
AgentMux Desktop Process
├── DiscordBridge (new service, runs as background Task)
│   ├── GatewayClient (discord.js Client or raw WS)
│   │   ├── Connects to wss://gateway.discord.gg (outbound only — no public URL)
│   │   ├── Listens: MESSAGE_CREATE on configured channel(s)
│   │   ├── Listens: APPLICATION_COMMAND_PERMISSIONS_UPDATE (slash command ACK)
│   │   └── Reconnect/resume: handled by discord.js; health check on focus resume
│   │
│   └── MessageRouter
│       ├── Inbound: Discord message → inject into AgentMux reactive bus
│       │   (POST /agentmux/reactive/send to target agent, or broadcast)
│       └── Outbound: reactive bus message → POST /channels/{id}/messages via REST
│
└── Agent Pane
    ├── Consumes inbound messages via existing MCP read_messages tool
    └── Sends outbound via existing MCP send_message → router → Discord REST
```

**No cloud server, no public URL.** The Gateway WebSocket is a client-side outbound connection. It works behind NAT and corporate firewalls.

### 5.2 Configuration (stored in AgentMux settings)

```json
{
  "discord": {
    "enabled": false,
    "botToken": "<keychain://agentmux/discord-bot-token>",
    "guildId": "...",
    "channelId": "...",
    "agentTarget": "<agent_id to route inbound messages to>",
    "intents": ["Guilds", "GuildMessages", "MessageContent"]
  }
}
```

Bot token stored in OS keychain (`keytar` in Electron; Windows Credential Manager via `keytar` native bindings). Never in plaintext config files.

### 5.3 Agent-to-Discord message flow

1. Agent calls `send_message` with `target: "discord"` (new target type in reactive bus)
2. MessageRouter receives it, enriches with embed (agent name, timestamp, model)
3. `POST https://discord.com/api/v10/channels/{channel_id}/messages`
4. Discord renders the message in the configured channel

### 5.4 Discord-to-agent message flow (with MESSAGE_CONTENT intent)

1. User types in Discord channel: "Hey agent, check the build status"
2. Gateway receives `MESSAGE_CREATE` with full `content` field (intent enabled)
3. MessageRouter filters: only from non-bot users in the configured channel
4. Wraps in AgentMux message envelope: `{ from: "discord:user#1234", content: "..." }`
5. POSTs to `/agentmux/reactive/send` targeting the configured agent_id
6. Agent receives via `read_messages`, responds via `send_message` → back to Discord

### 5.5 Slash command flow (intent-free alternative)

1. Register guild-scoped `/ask <prompt>` slash command on startup
2. User types `/ask prompt:check the build status`
3. Discord sends Interaction over Gateway (`INTERACTION_CREATE`)
4. MessageRouter extracts `prompt` option value (no MESSAGE_CONTENT needed)
5. Same routing as above; respond with `POST /webhooks/{app_id}/{token}` within 3s
6. Use `deferred_channel_message_with_source` if agent response takes >3s

---

## 6. Implementation Plan

### Phase 1: Foundation (POC — 1-2 PRs)

**Deliverables:**
- `agentmux-srv/src/discord/` — new Rust module (or Node.js sidecar if using discord.js)
  - Architecture decision: discord.js runs in Node.js. AgentMux backend is Rust. Options:
    - **Option A (recommended):** Rust sidecar using `serenity` crate (Rust Discord library, actively maintained, async-native). Integrates cleanly with existing Rust backend.
    - **Option B:** Node.js subprocess running discord.js, communicating with main process via stdin/stdout JSON or local HTTP
- `frontend/app/settings/DiscordSettings.tsx` — minimal settings pane: token input (masked), guild ID, channel ID, enable toggle
- Token stored via OS keychain API

**Success criteria:** Bot appears online in Discord, POSTs a "AgentMux connected" message to the configured channel on startup.

### Phase 2: Bidirectional Bridge (1 PR)

**Deliverables:**
- Inbound: `MESSAGE_CREATE` → reactive bus → configured agent
- Outbound: agent `send_message` to `discord` target → REST POST
- Message formatting: plain text + embed with agent identity

**Success criteria:** User types in Discord, agent receives it and can reply; reply appears in Discord.

### Phase 3: Slash Commands (1 PR)

**Deliverables:**
- Guild-scoped `/ask` command registration on startup (auto-register if changed)
- Slash command interaction handler (replaces `MESSAGE_CONTENT` dependency)
- Deferred response pattern (ACK within 3s, follow-up after agent responds)
- `/status` command: returns current agent status inline

**Success criteria:** Full round-trip with no privileged intents required.

### Phase 4: Warden integration (future)

- Discord connection status surface in the Warden widget (Internet section)
- Connection health: ping latency, events/min, reconnect count
- Per-channel routing config (multiple agents, multiple channels)

---

## 7. Rust Library Recommendation: `serenity`

For the Rust-native path (Option A):

```toml
[dependencies]
serenity = { version = "0.12", features = ["client", "gateway", "model", "http"] }
tokio = { version = "1", features = ["full"] }
```

- `serenity` 0.12 targets Discord API v10; maintained and widely used in production Rust bots
- Handles Gateway connection, session resumption, heartbeat scheduling
- `EventHandler` trait: implement `message()` and `interaction_create()` callbacks
- REST via `serenity::http::Http` with automatic rate limit handling

If Node.js sidecar is preferred (Option B), use `discord.js` v14 with TypeScript. The sidecar communicates with the Rust server via a local WebSocket or named pipe.

---

## 8. Security Considerations

- **Bot token = credential**: store in OS keychain only; never log, never serialise to disk
- **Webhook URL rotation**: if a webhook URL is ever leaked, rotate immediately (Developer Portal → channel → Integrations → Webhooks → Regenerate)
- **Channel scoping**: only process messages from the configured guild + channel; reject all others at the router layer
- **Bot permissions**: minimal — `Send Messages`, `Read Message History`, `Use Slash Commands`. No `Administrator`. No `Manage Channels`.
- **User input sanitisation**: Discord message content flows into agent context; treat as untrusted user input (same as web UI input field)
- **No embed-based secrets**: never include API keys or auth tokens in Discord embed fields

---

## 9. Scope Out of POC

- Voice channels (not relevant for agent communication)
- Multi-guild deployment (private bot, single guild)
- Reaction-based interactions (slash commands are the correct pattern)
- Telegram, Slack, Signal, WhatsApp (same architecture, follow-on work)
- OpenClaw ACP channel integration (defer until OpenClaw ships; architecture is compatible)

---

## 10. Decision Points for Implementation

| Decision | Default | Alternative |
|---|---|---|
| Backend language | Rust (`serenity`) | Node.js subprocess (`discord.js`) |
| Initial input method | `MESSAGE_CONTENT` (intent, POC only) | Slash commands only (no intent, production-ready from day 1) |
| Inbound routing | Single configured agent | Broadcast to all agents |
| Message format | Plain text + embed | Embed only |
| Settings surface | Settings pane | Warden widget Discord section |

Recommend: Rust + serenity for the backend (avoids Node.js subprocess); slash commands only from day 1 (avoids privileged intent entirely, works for all deployment sizes).
