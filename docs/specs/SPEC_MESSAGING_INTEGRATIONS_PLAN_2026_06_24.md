# Spec: Messaging App Integrations — Unified Plan

**Date:** 2026-06-24  
**Status:** Draft  
**Scope:** Architecture and implementation plan for integrating the top 5 messaging platforms into AgentMux

---

## 1. Goal

Extend AgentMux so that an agent can bidirectionally communicate with a user through the messaging app they already live in — **without changing how the user experiences that app**. The user sees the real, unmodified Discord/Slack/Telegram/WhatsApp/Teams interface inside an AgentMux pane. The agent participates in conversations through that same native interface. No custom chat UI, no reimplemented interface.

The five platforms, in order of global reach:

| Rank | Platform | Users | API Model | Desktop-App Friendly |
|------|----------|-------|-----------|---------------------|
| 1 | **WhatsApp** | 3B+ | Cloud API (webhooks) or unofficial WS | Medium (webhook complexity) |
| 2 | **Telegram** | 900M+ | Bot API (long-polling) | Excellent (no public URL) |
| 3 | **Discord** | 600M+ | Gateway WS + REST | Excellent (no public URL) |
| 4 | **Slack** | 38M DAU | Socket Mode WS + Web API | Good (no public URL) |
| 5 | **Microsoft Teams** | 320M MAU | Azure Bot Service relay | Poor (always needs cloud relay) |

---

## 2. Architecture

### 2.1 Two-layer design: pane + bridge

Each integration has exactly two layers:

**Pane layer** — the user interface. A pre-configured browser widget (CEF pane) pointed at the web version of the messaging app. The user sees and uses the real app, unchanged, inside an AgentMux pane tile. Full drag/drop, layout management, and multi-pane positioning work exactly as they do for any other pane — because it is just the browser widget.

**Bridge layer** — invisible background infrastructure. A Rust or Node.js process connecting to the platform's API (Gateway WebSocket, long-poll, Socket Mode, etc.) so that the agent can read inbound messages and post replies. Agent messages appear naturally inside the native web UI the user is looking at.

```
┌──────────────────────────────────────────┐
│  AgentMux Pane (CEF browser widget)      │
│                                          │
│  ┌──────────────────────────────────┐    │
│  │   discord.com/app                │    │
│  │   (real Discord web UI)          │    │ ← user interacts here natively
│  │   messages appear here naturally │    │
│  └──────────────────────────────────┘    │
└──────────────────────────────────────────┘
              ↕ agent messages appear in-channel
┌──────────────────────────────────────────┐
│  Discord Bridge (background, invisible)  │
│  twilight-rs Gateway WS                  │ ← agent reads/writes here
│  ↕ AgentMux reactive bus                │
│  ↕ Agent (via MCP send_message)         │
└──────────────────────────────────────────┘
```

**What this means in practice:** The user opens a Discord pane in AgentMux. They see their real Discord. The agent is a bot in the same channel. The user types, the bot responds — all visible in the real Discord UI. There is no separate AgentMux chat UI; the messaging app IS the UI.

### 2.2 Pane implementation — widgets.json entries

Each messaging platform is a new entry in `widgets.json` using `"view": "browser"` with a pre-set URL. This is all that is needed on the frontend — no new ViewModel, no new ViewComponent, no new registry entry.

```json
"defwidget@discord": {
    "display:order": 10,
    "icon": "discord",
    "label": "Discord",
    "description": "Discord messaging — real interface, agent-connected",
    "blockdef": { "meta": { "view": "browser", "url": "https://discord.com/app" } }
},
"defwidget@slack": {
    "display:order": 11,
    "icon": "slack",
    "label": "Slack",
    "description": "Slack messaging — real interface, agent-connected",
    "blockdef": { "meta": { "view": "browser", "url": "https://app.slack.com/" } }
},
"defwidget@telegram": {
    "display:order": 12,
    "icon": "telegram",
    "label": "Telegram",
    "description": "Telegram Web — real interface, agent-connected",
    "blockdef": { "meta": { "view": "browser", "url": "https://web.telegram.org/" } }
},
"defwidget@whatsapp": {
    "display:order": 13,
    "icon": "whatsapp",
    "label": "WhatsApp",
    "description": "WhatsApp Web — real interface, agent-connected",
    "blockdef": { "meta": { "view": "browser", "url": "https://web.whatsapp.com/" } }
},
"defwidget@teams": {
    "display:order": 14,
    "icon": "teams",
    "label": "Teams",
    "description": "Microsoft Teams Web — real interface, agent-connected",
    "blockdef": { "meta": { "view": "browser", "url": "https://teams.microsoft.com/" } }
}
```

The icons (`discord`, `slack`, `telegram`, `whatsapp`, `teams`) need to be added to the icon set — or mapped to the closest existing ones temporarily.

### 2.3 Drag/drop and in-app features

Because messaging panes are browser widgets, they inherit everything the browser widget already supports:
- **Drag/drop layout**: pane tiles can be repositioned, split, resized exactly like any other pane
- **Multi-pane**: Discord and an agent pane side-by-side, or Telegram above Slack
- **Pane header**: back/forward/reload nav bar, address bar (so user can also browse docs, etc.)
- **Focus management**: click to focus the CEF pane, keyboard input routes to the web app
- **Full web app features**: all in-app features of the messaging app work — reactions, threads, file upload, search, notifications — because it's running the real web app in CEF, not a stripped-down embed

What does NOT work (CEF sandbox boundary):
- Native desktop notifications from the messaging apps are scoped to the AgentMux process, not the system tray — unless AgentMux forwards them
- Desktop OS integration (drag files from desktop into the pane) may require CEF drag-and-drop config to be enabled

### 2.4 Bridge layer — background infrastructure

Each bridge translates between a platform's wire protocol and the AgentMux reactive bus:

```
External Platform
  │  (Discord Gateway WS / Telegram long-poll / Slack Socket Mode /
  │   WhatsApp Cloud webhook / Teams Bot Framework)
  ▼
Platform Bridge (background, invisible to user)
  │  inbound: platform message → AgentMux reactive bus → agent reads it
  │  outbound: agent send_message → bridge → platform API → appears in CEF pane
  ▼
AgentMux Reactive Bus
  POST /agentmux/reactive/send   ← bridge delivers inbound messages here
  GET  /agentmux/reactive/read   ← bridge reads outbound messages here
  ▼
Agent (reads via MCP read_messages, sends via MCP send_message)
```

**Key design principle:** Bridges are opt-in, per-user configured, and isolated from each other. The existing reactive bus contract is unchanged. The bridge is an adapter on the outside.

### 2.5 Common Bridge Interface

Every bridge implements the same interface regardless of platform:

```rust
trait MessagingBridge: Send + Sync {
    fn platform(&self) -> &'static str;         // "discord", "telegram", etc.
    async fn start(&self) -> Result<()>;         // connect and begin event loop
    async fn stop(&self);                        // clean disconnect
    async fn send(&self, msg: OutboundMsg) -> Result<()>;
    fn health(&self) -> BridgeHealth;           // for Warden widget
}
```

Outbound routing: when an agent calls `send_message` with `target: "messaging:discord"` (or `"messaging:telegram"`, etc.), the reactive bus routes to the configured bridge for that platform. Only one bridge per platform is active at a time.

### 2.6 Configuration Storage

Stored in AgentMux user config under `[messaging.<platform>]`. Credentials (tokens, secrets) are stored in OS keychain — never plaintext. The config block is:

```toml
[messaging.discord]
enabled = true
guild_id = "..."
channel_id = "..."
agent_target = "<agent_id>"     # route inbound messages to this agent
bot_token = "keychain://agentmux/discord-bot-token"

[messaging.telegram]
enabled = false
bot_token = "keychain://agentmux/telegram-bot-token"
allowed_chat_ids = [123456789]  # security allowlist

[messaging.slack]
enabled = false
bot_token = "keychain://agentmux/slack-bot-token"
app_token = "keychain://agentmux/slack-app-token"
channel_id = "C..."

[messaging.whatsapp]
enabled = false
mode = "cloud_api"              # or "bridge" (Baileys-based)
phone_number_id = "..."
access_token = "keychain://agentmux/whatsapp-token"
tunnel = "cloudflare"          # required for cloud_api mode

[messaging.teams]
enabled = false
app_id = "..."
app_password = "keychain://agentmux/teams-app-password"
tenant_id = "..."
tunnel = "devtunnel"
```

### 2.7 Warden Widget — Internet Section

The currently-stub "Internet" section in the Warden widget is the natural home for bridge status. Each enabled bridge gets a row:

```
┌─────────────────────────────────────────────────────┐
│ Internet                  AgentBus cloud relay · opt-in │
│                                                         │
│  discord     ● connected   #agent-channel · 12ms ping  │
│  telegram    ● connected   @agentmux_bot · polling     │
│  slack       ○ disabled    —                            │
│  whatsapp    ⚠ tunnel down  Reconnecting...            │
│  teams       ○ disabled    —                            │
└─────────────────────────────────────────────────────┘
```

---

## 3. Platform Designs

---

### 3.1 Discord

**Maturity: Production-ready.** Discord's Bot API is extremely stable, free, and well-suited to the AgentMux use case.

#### Setup (one-time, user-performed)
1. Create application at discord.com/developers
2. Enable "Message Content" intent in Bot tab (no review needed for private bots <10,000 users)
3. Copy bot token → AgentMux settings (stored in keychain)
4. Invite bot to private server via OAuth2 URL: `?scope=bot+applications.commands&permissions=3072`

#### Receiving (Gateway WebSocket)
- Connect to `wss://gateway.discord.gg/?v=10&encoding=json`
- Identify with intents bitmask: `GUILDS (1<<0) | GUILD_MESSAGES (1<<9) | MESSAGE_CONTENT (1<<15)` = `33281`
- Store `session_id` and `resume_gateway_url` from READY payload
- Track last `s` (sequence) on every dispatch
- On disconnect: RESUME using `resume_gateway_url` + stored `session_id` + `seq`
- OS power wake event (Windows `WM_POWERBROADCAST`) → proactive reconnect before zombie detection

#### Sending (REST)
```
POST https://discord.com/api/v10/channels/{channel_id}/messages
Authorization: Bot {token}
{"content": "...", "embeds": [{...}]}
```
Rate limit: 50 req/s global; parse `X-RateLimit-*` headers dynamically.

#### Slash Commands (intent-free alternative)
Register guild-scoped `/ask <prompt>` command on startup:
```
PUT https://discord.com/api/v10/applications/{app_id}/guilds/{guild_id}/commands
```
For responses >3s: send `type: 5` (deferred) immediately, then PATCH `@original` within 15 min.

#### Rich Output Format
```json
{
  "embeds": [{
    "title": "AgentMux",
    "description": "Task output here",
    "color": 5763719,
    "fields": [
      {"name": "Model", "value": "claude-sonnet-4-6", "inline": true},
      {"name": "Duration", "value": "2.1s", "inline": true}
    ],
    "footer": {"text": "via AgentMux"}
  }]
}
```

#### Rust library
- **`twilight-rs`** (`twilight-gateway` + `twilight-http`): modular, no hidden cache, best for embedding into existing Tokio runtime
- Alternative: `serenity` (batteries-included, less surgical control)

#### Rate Limits Summary
| Limit | Value |
|-------|-------|
| Global REST | 50 req/s |
| Gateway send | 120 events/60s |
| Daily global command updates | 200 |
| Session starts (Identify) | 1,000/24h |

#### Known failure modes
- **Heartbeat ghost**: no ACK → close + RESUME. Detection window: one heartbeat interval (~42s)
- **Wake-from-sleep**: proactive reconnect on OS power resume event
- **4014 close code**: disallowed intent → requires Developer Portal toggle, do not RESUME

---

### 3.2 Telegram

**Maturity: Excellent.** The Bot API is the most developer-friendly of the five. No public URL required. Long polling works perfectly from a desktop app.

#### Setup (one-time, user-performed)
1. Open @BotFather in Telegram, send `/newbot`
2. Set display name + username (must end in `bot`)
3. Copy token → AgentMux settings
4. Optionally: `/setprivacy off` if bot needs to see all group messages; `/setcommands` to register menu

#### Receiving (Long Polling — no public URL needed)
```
GET https://api.telegram.org/bot{TOKEN}/getUpdates?timeout=30&offset={last_id+1}&allowed_updates=["message","callback_query"]
```

**Offset tracking (critical):** After processing a batch, advance offset to `last_update_id + 1`. Never advance before processing — failure before advancing re-delivers the same batch (handlers must be idempotent).

**Single-instance guarantee:** Only one concurrent `getUpdates` per token (Telegram returns 409 Conflict otherwise). This enforces single-desktop-instance operation naturally.

#### Sending
```
POST https://api.telegram.org/bot{TOKEN}/sendMessage
{"chat_id": 123456789, "text": "<b>Agent output</b>", "parse_mode": "HTML", "reply_markup": {...}}
```
Use HTML parse mode for agent-generated content (simpler escaping than MarkdownV2 — only `<`, `>`, `&` need escaping).

#### Rate Limits
| Scope | Limit |
|-------|-------|
| Single chat | 1 message/second |
| Group chat | 20 messages/minute |
| Global | ~30 messages/second |

On `429`: read `parameters.retry_after` and `parameters.scope` (added Bot API 7.8). Per-chat scope → pause only that chat's queue. Global scope → pause all outbound. Always add jitter. Never pre-throttle.

#### Inline Keyboards — Agent Action Buttons
```json
{
  "inline_keyboard": [
    [{"text": "Approve", "callback_data": "approve"},
     {"text": "Cancel", "callback_data": "cancel"}],
    [{"text": "View log", "callback_data": "view_log"}]
  ]
}
```
After user taps: receive `callback_query` update → **must call `answerCallbackQuery`** within 60s (clears button spinner). Then optionally `editMessageReplyMarkup` to update buttons.

#### Message Editing for Streaming Output
Telegram allows bots to edit their own messages. Pattern for long-running tasks:
1. `sendMessage` → get `message_id`
2. `editMessageText` with `message_id` as task progresses
3. Final `editMessageText` with complete output

This simulates streaming without real-time infrastructure.

#### Security Allowlist
`allowed_chat_ids` in config. Every inbound update must have `message.chat.id` in the allowlist. Reject silently; do not reply to unknown chats (reduces enumeration surface).

#### Rust Library
**`teloxide`** (v0.13, Tokio-native, full Bot API 9.1 support). Spawn the dispatcher as a task on the existing Tokio runtime:
```rust
let bot = Bot::new(token);
let dispatcher = Dispatcher::builder(bot, handler).build();
dispatcher.dispatch().await;
```

---

### 3.3 Slack

**Maturity: Good.** Socket Mode makes Slack viable for desktop apps without a public URL, but connections refresh every ~1 hour and can silently stop delivering messages after days.

#### Setup (one-time, user-performed)
1. Create Slack app at api.slack.com/apps → "From an app manifest"
2. Enable Socket Mode → generate App-Level Token (`xapp-`) with `connections:write` scope
3. Enable event subscriptions: `app_mention`, `message.im`
4. Add bot scopes: `chat:write`, `channels:read`, `im:history`, `im:read`, `im:write`, `app_mentions:read`, `commands`
5. Register slash command `/agent`
6. Install to workspace → copy Bot Token (`xoxb-`) → AgentMux settings

Both tokens stored in keychain. Neither expires unless manually revoked.

#### Receiving (Socket Mode WebSocket)
```
POST https://slack.com/api/apps.connections.open
Authorization: Bearer xapp-{token}
→ {"ok": true, "url": "wss://wss.slack.com/link/?ticket=..."}
```
Connect to the returned URL (ephemeral — do not cache). Receive envelope:
```json
{
  "envelope_id": "...",
  "type": "events_api",
  "payload": { "event": { "type": "message", "text": "...", "channel": "..." } }
}
```
**Must ACK within 3 seconds** by sending back `{"envelope_id": "..."}` on the same WebSocket.

#### Reconnect Pattern
- Slack sends `{"type": "disconnect", "reason": "warning"}` ~10s before refresh
- On warning: call `apps.connections.open` immediately for new URL → open second WS in parallel
- On `link_disabled`: close old connection, route acks to new connection
- On unexpected close: exponential backoff (1s → 2s → 4s → capped at 60s)
- **Heartbeat check**: if no event received for >5 minutes on an apparently-open WS, force reconnect (known silent-failure bug)

#### Sending
```
POST https://slack.com/api/chat.postMessage
Authorization: Bearer xoxb-{token}
{"channel": "C...", "text": "fallback", "blocks": [...]}
```
Always include `text` as fallback for notifications. Rate: ~1 msg/s/channel (Special tier).

#### Slash Command Deferred Pattern
```
1. ACK immediately (< 3s): {"envelope_id": "...", "payload": {"text": "Working...", "response_type": "ephemeral"}}
2. Process (any duration)
3. POST to response_url: {"text": "Result", "response_type": "in_channel", "blocks": [...]}
   → Valid for 5 uses within 30 minutes
```

#### Block Kit — Agent Output Template
```json
{
  "blocks": [
    {"type": "header", "text": {"type": "plain_text", "text": "AgentMux Result"}},
    {"type": "section", "text": {"type": "mrkdwn", "text": "*Model:* claude-sonnet-4-6  |  *Duration:* 2.1s"}},
    {"type": "divider"},
    {"type": "rich_text", "elements": [
      {"type": "rich_text_preformatted",
       "elements": [{"type": "text", "text": "output here"}],
       "language": "text"}
    ]},
    {"type": "actions", "elements": [
      {"type": "button", "text": {"type": "plain_text", "text": "Run Again"}, "action_id": "run_again", "style": "primary"}
    ]}
  ]
}
```

#### Node.js Library (Electron Main Process)
`@slack/web-api` + `@slack/socket-mode` — Slack-official, auto-reconnect, typed. Best fit for Electron. IPC bridge to Rust agent core for business logic.

#### Rate Limits Summary
| Method | Limit |
|--------|-------|
| `chat.postMessage` | ~1/s/channel (Special) |
| Most read methods | 50+/min (Tier 3) |
| `conversations.history` | 50+/min (internal apps) |
| Socket Mode connections | 10 max per app |
| Slash command `response_url` | 5 calls / 30 min |

---

### 3.4 WhatsApp

**Maturity: Complex.** The official Cloud API requires business verification and a public HTTPS endpoint (no long-polling). The unofficial path (Baileys) is simpler to set up but carries ToS and ban risk.

WhatsApp has two distinct integration paths. AgentMux should support both, with the user choosing based on their needs.

#### Path A: Official Cloud API

**Requirements:** Meta Business Account, business verification (2-10 days), dedicated phone number (cannot be used with regular WhatsApp app), public HTTPS webhook URL.

**Setup:**
1. Create Meta Business Account at business.facebook.com
2. Add WhatsApp product to a developer app at developers.facebook.com
3. Add + verify a phone number (separate from personal WhatsApp)
4. Create System User token (permanent, never expires): Business Settings → System Users → Generate Token with `whatsapp_business_messaging` + `whatsapp_business_management` scopes
5. Set up webhook (see below)

**Public URL problem — Cloudflare Tunnel (recommended):**

For a desktop app, the webhook needs a public HTTPS URL. Cloudflare Tunnel provides a permanent free named tunnel:

```bash
# One-time setup (user performs)
cloudflared tunnel login
cloudflared tunnel create agentmux-whatsapp
cloudflared tunnel route dns agentmux-whatsapp wa.yourdomain.com
```

AgentMux manages the tunnel lifecycle (`cloudflared tunnel run agentmux-whatsapp`) as a subprocess. The resulting `https://wa.yourdomain.com` is registered once in Meta's dashboard and never changes.

**Alternative tunnels:** ngrok (free tier: random URL on restart, must re-register each session), Hookdeck (stable URL + event queue/replay, free tier 10K events/month — useful since Meta drops events after 7 days of failed delivery).

**Webhook verification:**
```
GET /webhook?hub.mode=subscribe&hub.verify_token={YOUR_TOKEN}&hub.challenge={RANDOM}
→ return hub.challenge plaintext with HTTP 200
```

Every inbound POST: verify `X-Hub-Signature-256` header (HMAC-SHA256 of raw body using app secret, constant-time comparison).

**Sending:**
```
POST https://graph.facebook.com/v25.0/{PHONE_NUMBER_ID}/messages
Authorization: Bearer {SYSTEM_USER_TOKEN}
{
  "messaging_product": "whatsapp",
  "to": "+1...",
  "type": "text",
  "text": {"body": "Agent output here"}
}
```

Or template message when outside the 24-hour window:
```json
{"type": "template", "template": {"name": "agent_update", "language": {"code": "en_US"}, "components": [{"type": "body", "parameters": [{"type": "text", "text": "..."}]}]}}
```

**The 24-hour window rule:**
- Free-form messages: only within 24h of last user message
- After 24h: must use pre-approved template messages
- Design: agent responds promptly; use templates for proactive notifications
- Best pattern: user initiates → agent responds within window → conversation stays open

**Pricing (as of 2026):**
- Service messages (user-initiated, within window): **free**
- Utility templates (within window): **free**
- Utility templates (outside window): $0.004/message (US)
- Marketing templates: $0.025/message (US); banned for US recipients since April 2025
- At ~100 conversations/month: effectively $0-$0.40 in API charges

**Policy note — AI chatbot ban (October 2025):** Meta prohibits using WhatsApp as the delivery channel for a general-purpose AI assistant distributed to others. A developer's private bot serving their own use (responding to their own messages, not distributing a product) is in a gray area — the ban targets "AI providers" distributing to end users. AgentMux should frame this as a personal productivity bridge, not a chatbot product.

**Cloud API rate limits:**
| Limit | Value |
|-------|-------|
| Messaging tier (default) | 1,000 unique users/24h (Tier 1) |
| MPS (default) | 80 messages/second |
| Per-user frequency cap | ~2 marketing/day (cross-business) |
| Per-recipient burst | ~1 msg/6s burst; 45 in 6s then cooldown |

**Rust library:** No official SDK. Use `reqwest` for REST calls. Implement webhook server with `axum` or `warp`. Run webhook receiver inside AgentMux process on a local port; expose via cloudflared.

---

#### Path B: Unofficial Bridge (Baileys)

**Requirements:** Any WhatsApp number (including personal), Node.js, willingness to accept ToS risk.

Baileys v7 implements WhatsApp Web's WebSocket protocol (Noise Protocol + Signal Protocol) directly — no browser, no Puppeteer. The desktop app maintains a linked-device session just like WhatsApp Web.

**Key properties:**
- No public URL needed (outbound WebSocket only)
- No Meta account, no business verification, no templates
- QR code scan or pairing code for initial auth (same as WhatsApp Web)
- Session persists across restarts (saved to disk)

**Ban risk:** Real but manageable for personal use:
- Protocol fingerprinting detection exists; Baileys attempts to mimic legitimate client behavior
- Bots that only respond to incoming messages: <2% ban rate over 12 months
- Bots sending proactive messages to new contacts: 15-30% ban rate
- v7.0.0 introduced improved protocol handling; `Bartender` test infrastructure now catches regressions
- **Never use on a number you can't afford to lose**

**In Electron:** Baileys runs in the Electron main process (pure Node.js). No subprocess needed.

**Recommendation:** Support both paths. Cloud API for users who want the official route; Baileys bridge for personal use with appropriate warning about ToS risk.

```
User config:
  mode = "cloud_api"   → official path, requires business account + tunnel
  mode = "bridge"      → Baileys path, personal number, ToS risk acknowledged
```

---

### 3.5 Microsoft Teams

**Maturity: High friction for personal use.** Teams is enterprise-first by architecture. Every integration path requires an Azure subscription, an Entra ID tenant, a public HTTPS endpoint, and Azure Bot Service as a mandatory relay in the message path. There is no local-only Teams integration.

**Viable for:** Users who already work in a Microsoft 365 organizational tenant (most corporate devs).  
**Not viable for:** Personal use without an M365 org account or Azure.

#### Prerequisites
- Microsoft 365 organizational account (personal MSA accounts cannot use Teams bots)
- Azure subscription (free tier F0 works; no message charges)
- Entra ID (AAD) tenant — same as the M365 org
- A publicly reachable HTTPS endpoint (bot endpoint must be accessible to Azure Bot Service)

#### Setup
1. Register app in Entra ID → get `Application ID` + `Client Secret`
2. Create Azure Bot resource (F0 tier, free) → link Entra app → enable Teams channel
3. Set messaging endpoint: `https://your-endpoint/api/messages`
4. Build app package (manifest.json + two icons) → sideload to Teams

**Sideloading requires an admin** to enable "Upload custom apps" in Teams Admin Center. For personal dev: join Microsoft 365 Developer Program (free E5 sandbox, requires qualifying subscription) or pay $6/month for M365 Business Basic.

#### Public URL requirement — Azure Relay (no ngrok needed in production)

Unlike the other platforms, Teams' Azure Bot Service relay means *your* endpoint only needs to be reachable by Azure's IP ranges. Options:
- **Dev Tunnels** (Microsoft's tool, free with GitHub account): persistent named URLs, integrates with VS Code Teams Toolkit
- **ngrok** (free random URL — must update Azure Bot registration each session; paid for stable URL)
- **Azure App Service F1** (free tier, 60 CPU-min/day): host the bot endpoint in Azure itself → zero tunnel dependency in production

#### Receiving (Azure Bot Service Relay)
All Teams message traffic routes through Azure. Your endpoint receives `POST /api/messages` with Bot Framework Activity objects:
```json
{
  "type": "message",
  "from": {"id": "...", "name": "User Name"},
  "conversation": {"id": "...", "tenantId": "..."},
  "text": "Hello agent",
  "serviceUrl": "https://smba.trafficmanager.net/teams/"
}
```
Validate JWT (Bot Connector JWKS at `https://login.botframework.com/v1/.well-known/keys`).

#### Sending — Proactive Messaging
To message a user without them initiating first:
1. Capture `conversationReference` on first contact → persist
2. Use `adapter.continueConversation(ref, logic)` to send proactively

Or create a new personal conversation:
```
POST https://smba.trafficmanager.net/teams/v3/conversations
{
  "bot": {"id": "APP_ID"},
  "members": [{"id": "AAD_USER_OID"}],
  "tenantId": "TENANT_ID"
}
```

#### Adaptive Cards Output Format
```json
{
  "type": "AdaptiveCard",
  "$schema": "http://adaptivecards.io/schemas/adaptive-card.json",
  "version": "1.4",
  "body": [
    {"type": "TextBlock", "text": "AgentMux Result", "weight": "Bolder", "size": "Medium"},
    {"type": "FactSet", "facts": [
      {"title": "Model", "value": "claude-sonnet-4-6"},
      {"title": "Duration", "value": "2.1s"}
    ]},
    {"type": "CodeBlock", "codeSnippet": "output here", "language": "text"}
  ],
  "actions": [
    {"type": "Action.Execute", "title": "Run Again", "verb": "run_again"}
  ]
}
```

#### SDK
**Teams SDK** (GA for JS and C#, November 2025 rename of Teams AI Library) — absorbs botbuilder (archived Dec 2025), Graph, Adaptive Cards, and Teams JS into one SDK. Python in public preview.

For Rust: no official SDK — raw HTTP with `reqwest` for outbound + `axum` to receive Bot Framework activities.

#### Rate Limits
| Scope | Limit |
|-------|-------|
| Per bot per thread | 7 sends/second, 60/30s, 1800/hour |
| Global per app per tenant | 50 RPS |

#### Verdict: Implement Last
Teams should be the last integration implemented. The mandatory Azure relay, Azure subscription, and admin-gated sideloading make it impractical for users outside enterprise M365 tenants. The value/friction ratio is lowest of the five.

---

## 4. Implementation Roadmap

### Phase 1: Telegram (Weeks 1-2)

**Why first:** Simplest integration (no public URL, no cloud dependency, excellent Rust library, no approval process). Proves the bridge architecture before tackling more complex platforms.

- `agentmux-srv/src/messaging/telegram/` — teloxide bridge
  - Long-polling loop with offset tracking
  - HTML parse mode for agent output
  - Inline keyboard builder for action buttons
  - `answerCallbackQuery` dispatch
  - `editMessageText` for streaming simulation
- Settings UI: token input (masked), allowed chat IDs
- Warden widget: "Internet" section with bridge status rows
- Changeset: `task changeset -- patch "feat: telegram messaging bridge"`

### Phase 2: Discord (Weeks 2-3)

**Why second:** Largest developer community for AgentMux users; clean architecture that validates the bridge interface for WebSocket platforms.

- `agentmux-srv/src/messaging/discord/` — twilight-rs bridge
  - Gateway client (twilight-gateway): identify, heartbeat, resume, reconnect
  - REST client (twilight-http): send messages, deferred interactions
  - OS power event hook for proactive reconnect on wake-from-sleep
  - Slash command registration (guild-scoped) on startup
  - Embed builder for rich agent output
- Settings UI: token, guild ID, channel ID, slash command toggle
- Changeset

### Phase 3: Slack (Weeks 3-4)

**Why third:** Socket Mode architecture is similar to Discord Gateway; code patterns transfer. Main delta is the App-Level Token dance and Block Kit formatting.

- Electron main process: `@slack/socket-mode` + `@slack/web-api`
  - Socket Mode client with auto-reconnect
  - Proactive heartbeat liveness check (silent disconnect mitigation)
  - Block Kit builder for rich output
  - Slash command deferred response via `response_url`
- IPC bridge to Rust reactive bus
- Settings UI: two-token setup (xoxb- and xapp-)
- Changeset

### Phase 4: WhatsApp — Cloud API + Bridge (Weeks 4-6)

**Why fourth:** Most complex setup; two paths to support; tunnel management to build.

- `agentmux-srv/src/messaging/whatsapp/` — Cloud API bridge
  - Webhook HTTP server (axum) on local port
  - `X-Hub-Signature-256` validation
  - Webhook verification GET handler
  - Outbound via Graph API v25.0
  - 24-hour window tracking (per-user state)
  - Template fallback when window expired
- Cloudflare Tunnel subprocess management: start/stop cloudflared, detect URL, register with AgentMux
- Electron main process: Baileys bridge for `mode = "bridge"`
  - QR code surface in Settings UI
  - Session persistence to disk (encrypted)
  - Warning banner: "Unofficial path — account ban risk accepted"
- Settings UI: mode selector, phone number ID or QR, tunnel config
- Changeset

### Phase 5: Microsoft Teams (Weeks 6-8)

**Why last:** Highest setup friction, Azure-dependent, narrowest target audience.

- Bot Framework activity handler (axum endpoint)
- JWT validation against Bot Connector JWKS
- Adaptive Card builder
- Proactive message with stored conversationReference
- Dev Tunnel subprocess management (alternative to Cloudflare for Teams)
- Settings UI: App ID, App Password, Tenant ID, tunnel config
- Documentation: full setup walkthrough (Azure Bot creation, app manifest, sideloading)
- Changeset

---

## 5. Common Abstractions

### 5.1 Tunnel Manager

Three platforms (WhatsApp Cloud API, Slack HTTP fallback, Teams) may need a public HTTPS URL. Rather than handling this per-bridge, a shared `TunnelManager` service manages this:

```rust
pub enum TunnelProvider {
    CloudflareTunnel { name: String, domain: String },
    DevTunnel { name: String },
    NgrokFree,           // ephemeral
    Manual { url: String }, // user provides their own
}

impl TunnelManager {
    pub async fn ensure_url(&self) -> Result<Url>;
    pub fn current_url(&self) -> Option<Url>;
    pub fn status(&self) -> TunnelStatus;
}
```

### 5.2 Message Envelope

All inbound messages from any platform are normalized before entering the reactive bus:

```rust
pub struct InboundMessage {
    pub platform: &'static str,          // "discord", "telegram", etc.
    pub platform_msg_id: String,         // native message ID for reply threading
    pub from_id: String,                 // platform user ID
    pub from_name: String,
    pub text: String,
    pub attachments: Vec<Attachment>,
    pub raw_payload: serde_json::Value,  // original for platform-specific handling
    pub received_at: u64,               // unix ms
}
```

### 5.3 Rich Output Builder

Agent output needs to be rendered differently per platform. A shared `AgentOutput` struct gets rendered by each platform's bridge:

```rust
pub struct AgentOutput {
    pub summary: String,               // plain text fallback
    pub model: Option<String>,
    pub duration_ms: Option<u64>,
    pub body: OutputBody,
    pub actions: Vec<OutputAction>,
}

pub enum OutputBody {
    Text(String),
    Code { language: String, content: String },
    Sections(Vec<Section>),
}

// Each bridge renders AgentOutput to its native format:
// DiscordBridge: → Embed
// TelegramBridge: → HTML message + InlineKeyboard
// SlackBridge: → Block Kit
// WhatsAppBridge: → text (no rich format) or template
// TeamsBridge: → Adaptive Card
```

### 5.4 Bridge Health Reporting

Each bridge reports to the Warden widget via a shared status channel:

```rust
pub struct BridgeHealth {
    pub platform: &'static str,
    pub status: BridgeStatus,           // Connected, Connecting, Disconnected, Error
    pub latency_ms: Option<u32>,        // last ping latency
    pub last_event_at: Option<u64>,     // last received event (ms)
    pub reconnect_count: u32,
    pub error: Option<String>,
}
```

---

## 6. Security Considerations (All Platforms)

1. **All credentials in OS keychain.** Bot tokens, app passwords, system user tokens — never in config files, never in logs.

2. **Allowlist-first.** Every platform: only process messages from explicitly configured sources (channel IDs, chat IDs, team IDs). Reject and do not respond to unknown sources — reduces enumeration, prevents unsolicited agent invocations.

3. **Webhook signature validation.** For any webhook-based platform (WhatsApp, Slack HTTP mode, Teams): always validate the HMAC signature using constant-time comparison. Never trust a webhook without signature validation.

4. **No secrets in platform messages.** Agent output flowing through any messaging bridge should never contain API keys, tokens, or private credentials. The bridge should scan outbound messages for common secret patterns and emit a warning if detected.

5. **Per-platform rate limit handling.** Each bridge implements backoff independently. A rate-limited bridge should queue and retry, not drop messages silently.

6. **WhatsApp ban risk disclosure.** The Baileys path shows a persistent warning in Settings UI. User must acknowledge before enabling.

7. **ToS clarity.** Documentation for each integration notes which Terms of Service govern the integration. Users building on unofficial paths (WhatsApp Baileys, any unofficial WhatsApp gateway) are warned explicitly.

---

## 7. Future: OpenClaw Compatibility

The `integration-vision.md` spec shows OpenClaw as the intended long-term external messaging layer. OpenClaw's ACP channel types (Discord, Telegram, Slack, Signal, iMessage, WhatsApp) overlap exactly with this spec.

When OpenClaw ships, the bridge interface defined here (`MessagingBridge` trait) should be implementable by an `OpenClawBridge` that delegates to OpenClaw's ACP channel API — giving users who have OpenClaw running a single managed gateway instead of per-platform bridges.

Until then, the per-platform bridges built here serve as the non-OpenClaw path and remain useful for users who want local-only, no-dependency messaging with no OpenClaw account required.

---

## 8. Summary: Platform Decision Matrix

| Platform | Setup Friction | Needs Public URL | Desktop Friendly | Rust Library | Priority |
|----------|----------------|-----------------|-----------------|--------------|----------|
| Telegram | Very Low | No | Excellent | `teloxide` | P1 |
| Discord | Low | No | Excellent | `twilight-rs` | P1 |
| Slack | Medium | No (Socket Mode) | Good | `@slack/socket-mode` (Node) | P2 |
| WhatsApp (Cloud) | High | Yes (tunnel) | Medium | Raw HTTP + `axum` | P2 |
| WhatsApp (Baileys) | Low | No | Excellent (with ToS risk) | `@whiskeysockets/baileys` (Node) | P2 |
| Teams | Very High | Yes (relay) | Poor | `Teams SDK` (JS) or raw | P3 |
