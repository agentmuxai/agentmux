# Spec: Messaging App Integration — Slack

**Date:** 2026-07-07
**Status:** Draft — ready to implement
**Scope:** Rust implementation of a Slack bridge (Socket Mode + Web API) in `agentmux-srv`, mirroring the shipped Discord bridge. Pane layer (webview) already exists.

---

## 1. Goal / context

AgentMux embeds messaging apps as two layers: a **pane** (a `browser` widget pointed at the real web app) and a **bridge** (a background process that lets an agent read/send messages through that same app). The full cross-platform architecture, decision matrix, and original per-platform protocol research live in `docs/specs/SPEC_MESSAGING_INTEGRATIONS_PLAN_2026_06_24.md` §1–§3.3 — read that first; this spec does not re-derive the Slack protocol from scratch, it translates the plan's §3.3 research into the actual Rust module shape this repo uses.

The Slack **pane** already shipped: `agentmux-srv/src/config/widgets.json` lines 136–150 define `defwidget@slack` as a `browser` widget pointed at `https://app.slack.com/`, description `"Slack — real interface, agent-connected (bridge Phase 2)"`. This spec is that "Phase 2" — the bridge.

The Discord bridge shipped in PR #1763 (merged 2026-06-24) as pure Rust inside `agentmux-srv`, using hand-rolled `tokio-tungstenite` + `reqwest`, no SDK crate, no separate process. That is the load-bearing precedent for this spec (see §3).

Reference implementation to mirror throughout: `agentmux-srv/src/messaging/discord/{mod,gateway,rest,types}.rs`.

---

## 2. Protocol design (condensed from the master plan, Slack-specific)

### 2.1 Connection lifecycle overview

Slack Socket Mode has one structural difference from Discord Gateway that shapes the whole module: Discord's Gateway URL (`wss://gateway.discord.gg`) is a fixed, well-known constant. Slack's Socket Mode URL is **ephemeral** — a fresh, single-use WS URL must be requested via a REST call before every connection attempt (initial connect *and* every reconnect). There is no equivalent of Discord's RESUME; every new connection is effectively a full re-identify at the transport level, though Slack's *application*-level state (subscriptions, event delivery) is stateless per-connection so this costs nothing but an extra round trip.

```
POST https://slack.com/api/apps.connections.open
Authorization: Bearer xapp-{app_token}
→ 200 {"ok": true, "url": "wss://wss-primary.slack.com/link/?ticket=...&app_id=..."}
```

Connect to the returned URL immediately — tickets are single-use and short-lived (treat as "use within seconds", never cache/reuse). On any connect failure, retry `apps.connections.open` with backoff (see §9).

### 2.2 Event envelope + the 3-second ACK

Once connected, Slack pushes JSON frames. The two relevant envelope types:

```json
// Hello (first frame after connect)
{"type": "hello", "num_connections": 1, "connection_info": {"app_id": "A..."}}

// Events API envelope (the actual payload we care about)
{
  "envelope_id": "95869...",
  "type": "events_api",
  "accepts_response_payload": false,
  "payload": {
    "event": {"type": "message", "channel": "C...", "user": "U...", "text": "...", "ts": "..."}
  }
}
```

**Hard correctness requirement:** every envelope carrying an `envelope_id` must be ACKed within 3 seconds by sending `{"envelope_id": "<id>"}` back on the **same socket it arrived on**. Missing the ACK does not just risk a retry — Slack will eventually stop delivering events over the connection and (per Slack's own docs) this failure mode is silent from the client's point of view. The ACK must happen unconditionally, before any business logic that could fail or block — i.e. read the frame, ACK immediately, *then* parse/route the payload. This mirrors Discord's HEARTBEAT_ACK handling in spirit (bounded-latency protocol-level response required to keep the connection alive) but the deadline here is much tighter (3s vs. Discord's ~41s heartbeat interval) and is per-event rather than per-interval.

### 2.3 Reconnect-on-warning — the trickiest part

Slack proactively recycles Socket Mode connections roughly hourly and signals this ahead of time:

```json
{"type": "disconnect", "reason": "warning"}
```

This arrives **~10 seconds before** Slack will actually close the socket. The naive approach — wait for the close, then reconnect — creates a message-loss window: events that Slack routes to the about-to-die connection between the warning and the close, or during the reconnect gap, can be missed if the new connection isn't ready in time.

**Correct pattern (make-before-break):**
1. On receiving `{"type": "disconnect", "reason": "warning"}`, immediately call `apps.connections.open` again to get a *second* ephemeral URL.
2. Open a second WebSocket to that URL in parallel, **while keeping the first connection alive and still ACKing events on it**.
3. Once the second socket reaches `hello`, treat it as the active connection for all *new* inbound processing; keep draining/ACKing any remaining frames on the old socket until it closes on its own (Slack closes it after the warning period) or a `link_disabled`-equivalent signal appears.
4. Discard the old socket handle once closed.

This is a two-socket, cutover-not-teardown-then-standup design — structurally different from Discord's RESUME (which is teardown-then-reconnect-with-replay). Implement it as an explicit state machine with (at most) two live `WebSocketStream` handles rather than trying to force it into the same single-connection loop Discord uses. See §4.2 for the concrete task shape.

**On unexpected close** (no prior warning — network blip, etc.): exponential backoff starting at 1s, doubling, capped at 60s, matching Discord's existing backoff constants (`RECONNECT_DELAY_SECS` / `MAX_RECONNECT_DELAY_SECS` in `gateway.rs`) for consistency. Each retry re-fetches a fresh URL via `apps.connections.open` (never retry the same stale URL).

**Proactive dead-connection heuristic:** Slack has a documented failure mode where a Socket Mode connection reports itself open but silently stops delivering events (no close frame, no warning). Mitigate with a timer: if no frame of any kind (including Slack's periodic pings, if any) has been received for **5 minutes**, force-close and reconnect via the same cutover-or-backoff path. Track `last_event_at` the same way `gateway.rs` does for Discord's health struct, and drive the timer off that field.

### 2.4 Outbound — Web API

```
POST https://slack.com/api/chat.postMessage
Authorization: Bearer xoxb-{bot_token}
Content-Type: application/json
{"channel": "C...", "text": "fallback text", "blocks": [ ... ]}
```

Always send `text` even when `blocks` is present — it's the fallback used for push notifications and accessibility surfaces. Slack's REST response is HTTP 200 even on many logical failures; the real success signal is the `"ok": true` field in the JSON body — check it explicitly, don't rely on status code alone (this is a real divergence from Discord's REST, which uses HTTP status faithfully).

### 2.5 Slash command deferred response

```
1. Slash command interaction arrives as a distinct envelope type ("slash_commands") — ACK the envelope within 3s same as any other event.
2. If synchronous work fits in that window, return payload immediately: {"envelope_id": "...", "payload": {"text": "...", "response_type": "ephemeral"}}
3. Otherwise ACK with just {"envelope_id": "..."} (empty ack) and do the work async, then:
   POST {response_url} {"text": "result", "response_type": "in_channel", "blocks": [...]}
   → response_url is valid for 5 uses within 30 minutes of the original command.
```

Not required for the MVP (see §10) but the module should have an obvious extension point for it (a `response_url` field threaded through the routed command payload).

### 2.6 Rate limits

| Surface | Limit | Notes |
|---|---|---|
| `chat.postMessage` | ~1 msg/s/channel (Special tier, bursts tolerated briefly) | Queue + backoff on 429, don't drop |
| Most read methods (`conversations.*`) | Tier 3, 50+/min | Not used in MVP (no read-side REST calls needed — Socket Mode delivers everything) |
| Socket Mode connections | 10 max concurrently per app | The make-before-break dance in §2.3 briefly uses 2; well under the cap |
| Slash command `response_url` | 5 calls / 30 min window | Enforce client-side if/when slash commands are added |
| `apps.connections.open` | Not separately documented as a hard cap, but treat as expensive — never poll it, only call on initial connect / warning / backoff retry | |

On any `429` from `chat.postMessage`: read `Retry-After` header (seconds), wait, retry once; on repeated 429s apply the same exponential backoff used for reconnects. Never busy-loop retry.

---

## 3. Rust-not-Node correction (override of the master plan)

The master plan (`SPEC_MESSAGING_INTEGRATIONS_PLAN_2026_06_24.md` §3.3, §4 Phase 3) specified Slack as **Electron main-process Node.js**, using `@slack/socket-mode` + `@slack/web-api`, bridged to the Rust core via IPC. That assumption is **overridden** by this spec for two reasons:

1. **This app is not Electron.** AgentMux's desktop host is a CEF-based Rust process (`agentmux-cef`) with `agentmux-srv` as the Rust backend. There is no Electron main process anywhere in this codebase for an IPC bridge to terminate into. The plan's Node.js assumption was written before (or without accounting for) that architectural fact.
2. **Discord already proved the Rust-only pattern in production.** PR #1763 implemented Discord's Gateway WebSocket + REST entirely in `agentmux-srv` using raw `tokio-tungstenite` and `reqwest`, with hand-rolled `serde_json` wire types (`agentmux-srv/src/messaging/discord/`) — not even a Discord SDK crate (`twilight`/`serenity`), despite the plan recommending one. There is no reason Slack's Socket Mode — which is protocol-wise simpler than Discord's Gateway (no bitmask intents, no RESUME/session-replay semantics, no per-shard identify limits) — should regress to a heavier, cross-process Node.js design.

**Decision: implement Slack as pure Rust inside `agentmux-srv`, in a `messaging/slack/` module structurally identical to `messaging/discord/`.** No Node.js process, no Electron, no IPC bridge, no new external process to supervise, package, or crash-recover.

**Crate choice:** raw `tokio-tungstenite` (already a dependency — `agentmux-srv/Cargo.toml` line 58, version `0.24`) for the WebSocket, `reqwest` (already a dependency, line 38) for the Web API. This matches Discord's dependency footprint exactly — zero new crates required for the transport layer.

**Alternative considered and rejected:** `slack-morphism` (unofficial Rust Slack SDK, has Socket Mode + Web API + Block Kit typed builders). It's a reasonable crate and would save some hand-rolled serde work, particularly for Block Kit's deeply nested/variant-heavy JSON shape. Rejected for MVP because (a) it's unofficial and adds a third-party dependency surface for exactly the kind of protocol logic Discord proved is tractable to hand-roll in an afternoon, (b) pulling in its own Block Kit type system creates a second Block Kit representation to reconcile against whatever escape-hatch/raw-JSON path we build (§7), and (c) dependency-footprint consistency with Discord was an explicit design goal from the task that spawned this spec. If Block Kit hand-rolling turns out to be a bigger time sink than expected during implementation, revisit — `slack-morphism` is the fallback, not a rejected-forever option.

---

## 4. Rust module layout

Mirror `agentmux-srv/src/messaging/discord/` exactly:

```
agentmux-srv/src/messaging/slack/
├── mod.rs        — SlackConfig, SlackBridge, GLOBAL_BRIDGE, init_global(), get(), send(), health()
├── socket.rs      — Socket Mode connection lifecycle: apps.connections.open, the hello/events_api/
│                    disconnect-warning state machine, ACK, make-before-break reconnect, dead-conn timer
├── rest.rs        — Web API client: chat.postMessage (+ response_url POST for slash commands, deferred)
└── types.rs       — wire types: envelope structs, event payload structs, Block Kit types, REST body types
```

(Naming note: Discord's equivalent file is `gateway.rs`; Slack's is named `socket.rs` since "Socket Mode" is Slack's own term and avoids implying Gateway-specific semantics like RESUME that don't apply here.)

### 4.1 `mod.rs` — config, bridge, lifecycle

```rust
// agentmux-srv/src/messaging/slack/mod.rs

mod socket;
pub mod rest;
pub mod types;

use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::mpsc;
use crate::messaging::{BridgeHealth, OutboundMsg};

#[derive(Debug, Clone)]
pub struct SlackConfig {
    /// Bot token (`xoxb-...`) — used for chat.postMessage via Web API.
    pub bot_token: String,
    /// App-level token (`xapp-...`) — used only for apps.connections.open.
    pub app_token: String,
    /// Default channel ID — inbound filter + default outbound target.
    pub channel_id: String,
    /// Agent ID to inject inbound Slack messages into via the reactive bus.
    pub target_agent: Option<String>,
}

pub struct SlackBridge {
    outbound_tx: mpsc::UnboundedSender<OutboundMsg>,
    health: Arc<Mutex<BridgeHealth>>,
}

static GLOBAL_BRIDGE: OnceLock<SlackBridge> = OnceLock::new();

impl SlackBridge {
    /// Initialize the global Slack bridge and start the Socket Mode background task.
    /// No-op if already initialized. The first `apps.connections.open` call happens
    /// inside the spawned task, not here — init_global itself does no network I/O
    /// and returns immediately, matching Discord's synchronous, non-blocking init.
    pub fn init_global(config: SlackConfig, http: reqwest::Client) { /* ... */ }

    pub fn get() -> Option<&'static SlackBridge> { GLOBAL_BRIDGE.get() }

    /// Enqueue a message for delivery via chat.postMessage. Sync, fire-and-forget
    /// onto the internal mpsc channel — matches Discord's DiscordBridge::send shape.
    pub fn send(&self, msg: OutboundMsg) -> Result<(), String> { /* ... */ }

    pub fn health(&self) -> BridgeHealth { self.health.lock().unwrap().clone() }

    pub fn platform(&self) -> &'static str { "slack" }
}
```

`init_global`'s spawned task does: `apps.connections.open` (async REST call) → on success, hand the URL to `socket::run_socket_loop(...)` which owns the full state machine from §2.1–§2.3, including subsequent `apps.connections.open` calls for reconnects/warnings. This is the key structural delta from Discord: Discord's `run_gateway_loop` connects directly to a constant URL; Slack's loop must fetch a URL as step zero of every connection attempt, including the very first one. Model this as `run_socket_loop` internally calling a `fetch_ws_url(app_token, http) -> Result<String, String>` helper before each `connect_async`, rather than accepting a URL as a parameter the way Discord's `run_session` does.

### 4.2 `socket.rs` — connection lifecycle sketch

State machine, roughly:

```rust
enum ConnState {
    Single(WsHandle),                    // normal steady-state, one live socket
    Cutover { old: WsHandle, new: WsHandle }, // between warning and old-socket close
}
```

Main loop (`run_socket_loop`) responsibilities:
- Outer retry loop with backoff (mirrors Discord's `run_gateway_loop` outer loop: connect → run_session-equivalent → on error, sleep(delay), double delay capped at 60s, reset delay after a session survives >30s).
- Inner per-connection loop (`run_session`-equivalent) using `tokio::select!` over: inbound WS frames, the outbound `mpsc::UnboundedReceiver<OutboundMsg>` (routes to `rest::post_message`, same as Discord), and — new vs. Discord — a `dead_conn_timer` (`tokio::time::sleep` reset on every received frame; fires after 5 min idle → treat as reconnect-worthy).
- On `{"type": "hello"}`: mark connected, update health.
- On `{"type": "disconnect", "reason": "warning"}`: kick off the make-before-break sequence from §2.3 — this is the one part of the loop that needs to hold two socket handles simultaneously; implement it as a nested async block that opens the second connection, waits for its `hello`, then swaps which handle is "primary" for outbound-relevant bookkeeping (health `last_event_at`, etc.), while a second `tokio::select!` continues draining/ACKing the old socket until it closes.
- On any `events_api` envelope: ACK immediately (send `{"envelope_id": ...}` before doing anything else), then parse `payload.event`, filter to configured channel, skip bot's own messages (Slack sets `bot_id` on bot-authored events — check for its presence the same way Discord checks `author.bot`), and inject into the reactive bus via `get_global_handler().inject_message(...)` exactly as `discord/gateway.rs::handle_dispatch` does for `MESSAGE_CREATE` (same `InjectionRequest` shape, `source_agent: Some("slack")`, envelope text prefixed e.g. `[Slack #channel @user]: text`).

### 4.3 `rest.rs`

```rust
pub async fn open_connection(http: &reqwest::Client, app_token: &str) -> Result<String, String>;
// POST apps.connections.open, Bearer {app_token}, returns the "url" field.

pub async fn post_message(http: &reqwest::Client, bot_token: &str, channel_id: &str, msg: &OutboundMsg) -> Result<(), String>;
// POST chat.postMessage, Bearer {bot_token}. MUST check body.ok, not just HTTP status (see §2.4).

pub async fn post_response_url(http: &reqwest::Client, response_url: &str, body: serde_json::Value) -> Result<(), String>;
// For slash-command deferred replies (future — see §10). Included now as a stub so the
// module shape doesn't need rework when slash commands land.
```

### 4.4 `types.rs`

Wire types needed: `SocketEnvelope` (tagged on `"type"`: `hello` / `events_api` / `disconnect` / `slash_commands`), `EventsApiPayload`, `SlackEvent` (`type`, `channel`, `user`, `text`, `ts`, `bot_id: Option<String>`), `AckFrame { envelope_id: String }`, `OpenConnectionResponse { ok: bool, url: Option<String>, error: Option<String> }`, `PostMessageBody { channel: String, text: String, blocks: Option<Vec<serde_json::Value>> }`, `PostMessageResponse { ok: bool, error: Option<String> }`. Block Kit types: see §7 — recommend representing blocks as `Vec<serde_json::Value>` rather than a fully typed enum tree for MVP (escape hatch, not a hand-rolled Block Kit DSL — see §11).

---

## 5. Config schema additions (`wconfig/types.rs`)

Follow the exact flat, serde-renamed convention at `agentmux-srv/src/backend/wconfig/types.rs` lines 300–324 (Discord's block). Insert a parallel `-- Slack messaging bridge --` section immediately after it:

```rust
    // -- Slack messaging bridge --

    /// Master enable for the Slack messaging bridge.
    /// When true, the bridge opens a Socket Mode connection at startup.
    #[serde(rename = "messaging:slack:enabled", default, skip_serializing_if = "is_false")]
    pub messaging_slack_enabled: bool,

    /// Slack bot token (`xoxb-...`). Used for Web API calls (chat.postMessage).
    /// Treat as a secret — do not log. Obtain from api.slack.com/apps → OAuth & Permissions
    /// → Bot User OAuth Token, after installing the app to the workspace.
    #[serde(rename = "messaging:slack:bot_token", default, skip_serializing_if = "Option::is_none")]
    pub messaging_slack_bot_token: Option<String>,

    /// Slack app-level token (`xapp-...`). Used only for apps.connections.open (Socket Mode).
    /// Treat as a secret — do not log. Obtain from api.slack.com/apps → Basic Information
    /// → App-Level Tokens, scope `connections:write`.
    #[serde(rename = "messaging:slack:app_token", default, skip_serializing_if = "Option::is_none")]
    pub messaging_slack_app_token: Option<String>,

    /// Channel ID to filter inbound messages and use as the default send target.
    #[serde(rename = "messaging:slack:channel", default, skip_serializing_if = "String::is_empty")]
    pub messaging_slack_channel: String,

    /// Agent ID that receives inbound Slack messages via the reactive bus.
    /// Absent → messages are logged but not forwarded to any agent.
    #[serde(rename = "messaging:slack:target", default, skip_serializing_if = "Option::is_none")]
    pub messaging_slack_target: Option<String>,
```

Two token fields, not one — this is the one structural delta from Discord's config block, required by Slack's two-token Socket Mode design (App-Level Token for the WS handshake, Bot Token for REST sends). No `guild`-equivalent field is needed (Slack has no guild-scoping concept analogous to Discord's per-guild slash command registration at MVP scope — see §10, slash commands deferred).

**User-facing setup checklist** (mirrors the plan's §3.3 setup steps, condensed for the settings UI / docs):
1. Create a Slack app at api.slack.com/apps → "From an app manifest" (recommended — faster than manual scope clicking).
2. App Settings → Socket Mode → toggle on → generate an App-Level Token with the `connections:write` scope. Copy the `xapp-...` value.
3. App Settings → Event Subscriptions → enable → subscribe to bot events: `message.channels` (or `message.im` for DMs) and `app_mention`.
4. App Settings → OAuth & Permissions → Bot Token Scopes: add `chat:write`, `channels:read`, `channels:history` (or `im:history`/`im:read`/`im:write` for DM-only use), `app_mentions:read`.
5. Install App to Workspace (OAuth & Permissions → Install to Workspace). Copy the Bot User OAuth Token (`xoxb-...`).
6. Invite the bot to the target channel: `/invite @your-bot-name` in Slack.
7. Enter both tokens + channel ID into AgentMux settings (`messaging:slack:app_token`, `messaging:slack:bot_token`, `messaging:slack:channel`), set `messaging:slack:enabled = true`.

---

## 6. Startup wiring (`main.rs`)

Mirror `agentmux-srv/src/main.rs` lines 731–755 (Discord's wiring block), inserted immediately after it:

```rust
    // Slack messaging bridge — opens a Socket Mode connection if configured.
    // Set messaging:slack:enabled + messaging:slack:bot_token + messaging:slack:app_token
    // in settings.json to activate.
    {
        let settings = config_watcher.get_settings();
        if settings.messaging_slack_enabled {
            match (
                settings.messaging_slack_bot_token.clone(),
                settings.messaging_slack_app_token.clone(),
            ) {
                (Some(bot_token), Some(app_token))
                    if !bot_token.is_empty() && !app_token.is_empty() =>
                {
                    messaging::slack::SlackBridge::init_global(
                        messaging::slack::SlackConfig {
                            bot_token,
                            app_token,
                            channel_id: settings.messaging_slack_channel.clone(),
                            target_agent: settings.messaging_slack_target.clone(),
                        },
                        reqwest::Client::new(),
                    );
                }
                _ => {
                    tracing::warn!(
                        "slack bridge: enabled but messaging:slack:bot_token and/or \
                         messaging:slack:app_token is not set in settings.json"
                    );
                }
            }
        }
    }
```

Note on the async-first-connect concern flagged in the task: `SlackBridge::init_global` itself stays synchronous and non-blocking, identical in shape to Discord's — it only builds the mpsc channel, stores the `SlackBridge` in `GLOBAL_BRIDGE`, and `tokio::spawn`s the background task. The `apps.connections.open` call that Discord doesn't need happens *inside* that spawned task (start of `socket::run_socket_loop`), not in `init_global` or in `main.rs`'s startup path. This keeps `main.rs`'s startup sequence non-blocking regardless of Slack API latency or a transient failure to open the first connection — exactly the same failure-isolation property Discord's wiring already has (a slow/broken Discord Gateway can't hang server startup either).

Add `pub mod slack;` to `agentmux-srv/src/messaging/mod.rs` alongside the existing `pub mod discord;`.

---

## 7. HTTP endpoints

Mirror `agentmux-srv/src/server/messaging_handlers.rs`. Add:

```
POST /api/messaging/slack/send
```

registered in `agentmux-srv/src/server/mod.rs` alongside the existing Discord route (line 330):
```rust
.route("/api/messaging/slack/send", post(messaging_handlers::handle_slack_send))
```

Extend `handle_status` (line 23–29) to also push `SlackBridge::get().map(|b| b.health())` into the `bridges` array, same pattern as the Discord line.

### Design decision: Block Kit vs. simple text

Discord's embed (`MsgEmbed`) and Slack's Block Kit are **not** structurally compatible — Discord embeds are a flat title/description/fields/color/footer record; Block Kit is a heterogeneous array of typed block objects (`section`, `header`, `divider`, `actions`, `rich_text`, ...) with nested `mrkdwn`/`plain_text` text objects and no direct "color" concept (color exists only via the deprecated `attachments` legacy API, not Block Kit proper). Forcing Slack through the shared `MsgEmbed` struct would either drop most of Block Kit's expressiveness or require `MsgEmbed` to grow Slack-specific fields that don't apply to Discord — neither is good.

**Recommendation: dual path on the request body**, simple-text convenience + raw-JSON escape hatch, no forced reuse of `MsgEmbed`:

```rust
#[derive(Deserialize)]
pub(super) struct SlackSendRequest {
    /// Plain text — always sent as the top-level "text" field (required by Slack
    /// as the fallback/notification string even when blocks are present).
    #[serde(default)]
    pub text: String,
    /// Override channel. Empty → use the bridge's default channel.
    #[serde(default)]
    pub channel_id: String,
    /// Optional convenience path: a title + body rendered as a simple two-block
    /// Block Kit message (header + section) — covers the common "agent result"
    /// case without the caller needing to know Block Kit's JSON shape.
    pub title: Option<String>,
    pub body: Option<String>,
    /// Escape hatch: raw Block Kit `blocks` array, passed through verbatim to
    /// chat.postMessage. If present, takes precedence over title/body.
    pub blocks: Option<Vec<serde_json::Value>>,
}
```

Routing logic in the handler: if `blocks` is `Some`, use it as-is (full power, caller's responsibility to produce valid Block Kit JSON). Else if `title`/`body` is `Some`, synthesize a minimal 2-block message (`header` + `section` with `mrkdwn` text) server-side. Else send plain `text` only (no `blocks` key at all — valid Slack Chat API usage). This gives simple callers (e.g. an agent's `send_message` MCP tool call) a low-effort path while not blocking richer use cases behind a hand-rolled typed Block Kit builder that would need to track Slack's fairly large and evolving block-type surface. See §11 for the explicit open decision on whether to later build a typed builder.

`handle_status`/`handle_slack_send` otherwise follow `handle_discord_send`'s shape 1:1: `SlackBridge::get()` → 503 with a clear "not initialized" error if `None`, else build `OutboundMsg` (reusing the existing shared `text`/`channel_id`/`reply_to` fields; `embed` stays `None` for Slack sends since Block Kit doesn't map to `MsgEmbed` — the Block Kit JSON needs its own field, not `OutboundMsg.embed`). **Follow-up note:** this means `OutboundMsg` needs a new `blocks: Option<Vec<serde_json::Value>>` field (or the Slack bridge needs its own outbound message type instead of reusing `OutboundMsg`) — call this out explicitly as an implementation-time decision in §11 rather than resolving it here, since it interacts with whatever shape the `MessagingBridge` trait (§8) ends up taking.

---

## 8. `MessagingBridge` trait compatibility

As of this writing there is no `MessagingBridge` trait in `agentmux-srv/src/messaging/mod.rs` — only the shared types (`InboundMsg`, `OutboundMsg`, `MsgEmbed`, `EmbedField`, `BridgeHealth`, `BridgeStatus`). A sibling spec, `docs/specs/SPEC_MESSAGING_INTEGRATION_TELEGRAM_2026_07_07.md`, is formalizing that trait concurrently with this one as part of adding the Telegram bridge (the next platform after Discord in the roadmap). Do not block Slack implementation on that spec landing first — design `SlackBridge` to be trivially adaptable to it once it exists.

Expected rough shape (per the task brief, reconcile exactly against the Telegram spec at implementation time):

```rust
trait MessagingBridge: Send + Sync {
    fn platform(&self) -> &'static str;
    fn send(&self, msg: OutboundMsg) -> Result<(), String>;   // sync, mpsc fire-and-forget
    fn health(&self) -> BridgeHealth;
}
```

`init_global(config, http_client)` stays a free function/static per module (as sketched in §4.1), not a trait method — Rust traits can't express a polymorphic associated constructor across heterogeneous `Config` types cleanly, and Discord already established the free-function convention. `SlackBridge` as designed in §4.1 already satisfies this shape (`platform()`, `send()`, `health()` all present) — implementing the trait once it lands should be a mechanical `impl MessagingBridge for SlackBridge { ... }` with no restructuring, assuming the trait's `send`/`health` signatures match what's above. If the Telegram spec's trait ends up requiring `async fn start()`/`async fn stop()` (as the original master plan's §2.5 sketch had), reconcile by having `init_global` remain the actual start-and-spawn entry point and add thin `start()`/`stop()` trait methods that delegate to it / signal the background task to exit via a oneshot or `CancellationToken` — not designed further here since it depends on the Telegram spec's final shape.

---

## 9. Security considerations specific to Slack

1. **Two distinct token types, two distinct blast radii.** The App-Level Token (`xapp-...`, scope `connections:write`) can only open Socket Mode connections — it cannot read messages or post on its own; it's not meaningful without an active event subscription setup on the app. The Bot Token (`xoxb-...`) is the one that can actually read/post per its granted scopes. Store both in `settings.json` following the existing plaintext-in-local-config convention already used for the Discord bot token (`messaging:discord:token`) — this repo's established pattern, despite the master plan's original OS-keychain recommendation (§6.1 of the plan), evidently was not followed for Discord either; stay consistent with what actually shipped rather than introducing a keychain path unilaterally for Slack alone. Never log either token (mirror `rest.rs`'s existing discipline of not printing `Authorization` header values).
2. **Scope minimization.** Only request the bot scopes actually used: `chat:write`, `channels:read`, `channels:history` (or `im:*` variants if DM-only), `app_mentions:read`. Do not request `admin.*`, `channels:manage`, or any workspace-wide management scopes — this bridge only needs to read and post in one pre-configured channel.
3. **Channel allowlist, matching Discord's pattern.** Inbound events are filtered to `messaging:slack:channel` only (see §4.2, "filter to configured channel") — same allowlist-first principle as the master plan §6.2 and as Discord's existing `if msg.channel_id != channel_id { return; }` check in `gateway.rs` line 397.
4. **Bot-message loop prevention.** Slack sets `bot_id` on events authored by any bot (including this one) — check for its presence and skip, mirroring Discord's `author.bot` check, to avoid the bridge reacting to its own posted messages.
5. **No secrets in outbound Block Kit payloads.** Same principle as the master plan §6.4 — agent-generated text flowing into `chat.postMessage` should not carry API keys/tokens; out of scope to build automated secret-scanning for MVP, but worth a code comment flagging it as a known gap (matches Discord's current state — no such scanning exists there either).
6. **App-Level Token exposure via Socket Mode ticket.** The ephemeral WS URL returned by `apps.connections.open` embeds a single-use ticket; treat it with the same care as a bearer token in transit (don't log the full URL at `info` level — log only that a connection was opened, at most a truncated/redacted URL at `debug`).

---

## 9b. Known failure modes / rate limits

| Failure mode | Symptom | Mitigation |
|---|---|---|
| Missed 3s ACK deadline | Slack silently reduces/stops event delivery over that connection | ACK before parsing/routing (§2.2); keep the ACK path allocation-light and infallible where possible |
| `disconnect: warning` mishandled as wait-then-reconnect | Message-loss window during the ~10s before forced close | Make-before-break dance (§2.3) — non-negotiable, this is the spec's core correctness requirement |
| Silent connection death (no warning, no close frame) | Bridge shows "connected" but events stop arriving indefinitely | 5-minute no-event force-reconnect timer (§2.3) |
| `apps.connections.open` 429/5xx during backoff retry | Reconnect loop stalls | Same exponential backoff as unexpected-close path; cap at 60s; never busy-loop |
| `chat.postMessage` returns HTTP 200 with `"ok": false` | Message silently "sent" but not delivered (e.g. `channel_not_found`, `not_in_channel`, `invalid_auth`) | Explicitly check `ok` field, surface `error` string in bridge health / logs (§2.4) |
| Rate limit on `chat.postMessage` (~1/s/channel) | 429 with `Retry-After` header | Respect `Retry-After`, single retry, then same backoff family as reconnects (§2.6) |
| Two tokens confused (bot token used for `apps.connections.open`, or app token used for `chat.postMessage`) | Auth failures with confusing errors (`invalid_auth`, `not_allowed_token_type`) | Config field names make the distinction explicit (`bot_token` vs `app_token`); consider a startup sanity check that `bot_token` starts with `xoxb-` and `app_token` starts with `xapp-`, warn (not hard-fail) on mismatch |
| Bot not invited to configured channel | `chat.postMessage` returns `not_in_channel`; inbound events never arrive for that channel | Document as setup step 6 (§5); consider surfacing this specific error string distinctly in bridge health |

---

## 10. Implementation checklist (phased, PR-sized)

**Phase 1 — Foundation + one-way send (1 PR)**
- [ ] `agentmux-srv/src/messaging/slack/types.rs` — envelope, event, REST body/response wire types
- [ ] `agentmux-srv/src/messaging/slack/rest.rs` — `open_connection`, `post_message` (with `ok` field check)
- [ ] `agentmux-srv/src/messaging/slack/mod.rs` — `SlackConfig`, `SlackBridge`, `init_global`/`get`/`send`/`health`
- [ ] `agentmux-srv/src/messaging/slack/socket.rs` — connect via `open_connection`, handle `hello`, ACK `events_api` envelopes (log-only, no routing yet), basic backoff reconnect (no warning-cutover yet — plain reconnect is acceptable for this PR)
- [ ] `pub mod slack;` in `messaging/mod.rs`
- [ ] Config fields in `wconfig/types.rs` (§5)
- [ ] Startup wiring in `main.rs` (§6)
- [ ] `POST /api/messaging/slack/send` handler + route registration (§7, text-only + title/body convenience path; raw `blocks` escape hatch can land same PR or next)
- **Success criteria:** bot appears online in Slack (visible via presence or a manual `chat.postMessage` test through the new endpoint); a message posted via the endpoint appears in the configured channel.

**Phase 2 — Inbound routing + reconnect correctness (1 PR)**
- [ ] Route `events_api` → reactive bus injection (`get_global_handler().inject_message(...)`, mirroring `discord/gateway.rs::handle_dispatch`'s `MESSAGE_CREATE` handling), with channel filter + `bot_id` self-message filter
- [ ] Implement the make-before-break warning-cutover state machine (§2.3, §4.2) — this is the phase where the two-socket logic actually lands
- [ ] 5-minute dead-connection force-reconnect timer
- [ ] Extend `handle_status` to include Slack bridge health
- **Success criteria:** user posts in the configured Slack channel, agent receives it via `read_messages` and can reply; bridge survives an hour of idle connection (crossing at least one Slack-initiated `warning` cycle) without a visible gap in `health().last_event_at`.

**Phase 3 — Block Kit escape hatch + polish (1 PR)**
- [ ] `blocks: Option<Vec<serde_json::Value>>` raw passthrough on the send endpoint if not already done in Phase 1
- [ ] Rate-limit handling: `Retry-After` respect + backoff on `chat.postMessage` 429s
- [ ] Update `widgets.json` line 142 description string from `"Slack — real interface, agent-connected (bridge Phase 2)"` to something reflecting the bridge is now live (e.g. `"Slack — real interface, agent-connected"`, matching Discord's description string exactly)
- **Success criteria:** an agent can post a Block Kit message (e.g. header + section + divider) via the raw `blocks` field and it renders correctly in the Slack client.

**Phase 4 — Slash commands (future, not required for MVP)**
- [ ] `response_url` deferred-response path (§2.5, §4.3 stub)
- [ ] Slash command registration/config (Slack slash commands are configured in the app manifest/dashboard, not registered at runtime like Discord's guild-scoped commands — mostly a docs/setup-checklist addition, not new runtime code)
- Out of scope for this spec's phased delivery; noted for completeness since the master plan's §3.3 covers it.

---

## 11. Open decision points for the implementer

1. **`OutboundMsg.blocks` vs. a separate Slack-specific outbound type.** §7 flags that Block Kit doesn't fit `MsgEmbed`. The cleanest fix is probably adding an `Option<Vec<serde_json::Value>>` field directly to the shared `OutboundMsg` struct in `messaging/mod.rs` (harmless no-op for Discord, `#[serde(skip_serializing_if = "Option::is_none")]`) rather than forking a `SlackOutboundMsg` type — but this should be decided jointly with whatever the Telegram spec's `MessagingBridge` trait work settles on for the shared envelope shape, since a second concurrent spec is touching the same file. Flagging rather than resolving here to avoid a merge conflict in intent.
2. **Full typed Block Kit builder vs. raw JSON forever.** This spec recommends raw `serde_json::Value` blocks (§4.4, §7) for MVP — lowest implementation cost, matches "ready to implement" scope. A typed Block Kit DSL (enum per block type, builder pattern) would be nicer for future agent-facing convenience APIs (e.g. a `send_slack_result(title, fields, actions)` MCP-level helper) but is a genuinely large surface (Slack has ~15 block types and a dozen-plus element/object types as of 2026) and should be scoped as its own follow-up spec if/when there's a concrete driving use case, not spent on speculatively now.
3. **`slack-morphism` re-evaluation.** §3 rejects it for MVP on dependency-footprint-consistency grounds. If hand-rolling the Socket Mode state machine (particularly the make-before-break cutover in §2.3) proves meaningfully harder than Discord's RESUME logic during implementation, it's worth a quick spike comparing hand-rolled vs. `slack-morphism`'s Socket Mode client before committing further engineering time — this spec's recommendation is a starting position, not a irreversible constraint.
4. **Token storage: settings.json plaintext vs. OS keychain.** §9.1 notes this repo's actual Discord precedent stores the token in `settings.json` (not keychain, despite the master plan's original §6.1 recommendation). This spec follows the precedent for consistency, but if there's an in-flight effort elsewhere in the codebase to move Discord's token to keychain storage, Slack's two tokens should move with it in the same change rather than diverging.
5. **Slash commands (§10 Phase 4) and DM support.** Both are explicitly out of scope for the phased plan above (single fixed channel, no interactive commands). If DM support becomes a near-term requirement, the config schema (§5) would need a mode switch (channel-scoped vs. DM-scoped) and the `im:*` scopes noted in the setup checklist (§5, step 4) instead of `channels:*` — worth deciding whether that's a variant of this bridge or a genuinely separate `target` concept before Phase 4 starts.
