# Spec: Messaging App Integration — Microsoft Teams

**Date:** 2026-07-07
**Status:** Draft — design complete, sequencing/priority TBD (see §2)
**Scope:** Rust bridge design for Microsoft Teams, the fifth and last platform in `SPEC_MESSAGING_INTEGRATIONS_PLAN_2026_06_24.md`. The webview pane already ships (`defwidget@teams` in `agentmux-srv/src/config/widgets.json`); this spec covers the background bridge that lets an agent read/send messages through it.

---

## 1. Goal and context

`SPEC_MESSAGING_INTEGRATIONS_PLAN_2026_06_24.md` (§1–§2) lays out the two-layer pattern used for all five messaging integrations: a CEF webview pane showing the real platform UI, plus an invisible background "bridge" process that lets an agent read inbound messages and post replies through the same native surface the user is looking at. Discord shipped first, in pure Rust, as PR #1763 (2026-06-24) — see `agentmux-srv/src/messaging/discord/`. This spec adapts that shape to Teams.

The master plan's own verdict on Teams (§3.5, §8) is blunt: **"Implement Last."** Every other platform in the five (Telegram, Discord, Slack, WhatsApp) can be bridged with nothing more than a bot token or app credentials the user controls personally. Teams cannot — it requires an Azure subscription, an Entra ID (AAD) tenant, and admin-gated app sideloading, none of which exist for a personal Microsoft account. This spec does not relitigate that verdict; it treats it as settled input and designs accordingly (§2).

This is a companion to two sibling specs written concurrently: `SPEC_MESSAGING_INTEGRATION_TELEGRAM_2026_07_07.md` (formalizes a `MessagingBridge` trait as Discord's shape gets generalized to a second platform) and `SPEC_MESSAGING_INTEGRATION_WHATSAPP_2026_07_07.md` (designs the first webhook-receiver bridge, since WhatsApp Cloud API — like Teams — is inbound-via-HTTP-POST rather than inbound-via-outbound-WebSocket like Discord/Telegram/Slack). Both may not exist on disk at the time this is read; where this spec depends on their conclusions, it says so explicitly and gives its own default.

---

## 2. Should we build this, and when

**Recommendation: design now (this document), implement only on demonstrated demand from a user inside an M365 tenant. Do not schedule it as part of the routine Telegram → Slack → WhatsApp → Teams rollout implied by the master plan's phase numbering.**

Reasoning:

1. **The audience that can use this at all is a strict subset of AgentMux's users, and it is not knowable in advance who that is.** Discord, Telegram, Slack, and WhatsApp all work with credentials an individual can generate for themselves in minutes, for free, with no organizational gatekeeper. Teams requires all of the following simultaneously:
   - A Microsoft 365 **organizational** account (personal `outlook.com`/`hotmail.com` MSA accounts cannot install Teams bots — this is a hard platform restriction, not a configuration gap).
   - An Azure subscription (free tier F0 suffices for the bot resource, but still requires a credit card on file in most signup flows).
   - An Entra ID tenant matching the M365 org (usually already exists for a corporate user; does not exist for a personal deployment).
   - A tenant admin willing to enable "Upload custom apps" in Teams Admin Center, or the user personally holding that role — many corporate users do not, and cannot self-serve this.
   - A publicly reachable HTTPS endpoint reachable from Azure's IP ranges (§5).

   A hobbyist, freelancer, or anyone running AgentMux against a personal Microsoft account gets zero value from this integration no matter how well it is built. This is different from WhatsApp's friction (business verification, tunnel setup) or Slack's (workspace admin approval for a *personal* workspace the user usually owns) — those are surmountable by one determined individual. Teams' friction floor includes an org that the individual developer may not control.

2. **The master plan already ranks it last by value/friction (§8 decision matrix) and calls it "Implement Last" explicitly (§3.5, §4 Phase 5).** Nothing has changed since 2026-06-24 that revises that ranking upward. This spec exists so that *if* the call is made to build it, there is a concrete, implementable design ready — not because the ranking should be revisited.

3. **Sequencing recommendation:** keep Teams behind the other three (Telegram, Slack, WhatsApp) indefinitely, gated on one of:
   - A specific user request from someone who confirms they have (a) an M365 org account, (b) Azure access, and (c) sideloading rights or an admin willing to grant them, **or**
   - AgentMux itself moving into a team/enterprise distribution model where a shared Entra app registration could serve many tenants at once (multi-tenant bot registration — see §14 open decision) — at that point the setup burden shifts from "every user configures their own Azure Bot" to "AgentMux ships one, tenant admins consent once," which meaningfully changes the friction calculus and would be worth revisiting this spec's sequencing call.

   Until either condition holds, this spec should sit in `docs/specs/` as a ready-to-build design, not an active roadmap item.

4. **This is not an argument to under-build the design.** Sections 3–14 below are implementation-ready to the same standard as the Discord POC spec and the shipped Discord code. If a developer picks this up because condition (3) above is met, they should be able to work from this document directly.

---

## 3. Setup prerequisites (restated from the master plan, user-facing)

These steps are unavoidably heavy and must be surfaced to the user *before* they enable the Teams bridge in settings — ideally as a checklist gate in the settings UI, not just documentation, so a user without an M365 org finds out in step 1 rather than step 4.

1. **Confirm M365 org account.** Personal Microsoft accounts (MSA) cannot install Teams bots. If the user's Teams sign-in is a personal `outlook.com` address, stop here.
2. **Register an app in Entra ID** (Azure Portal → Entra ID → App registrations → New registration). Note the `Application (client) ID` and the `Directory (tenant) ID`. Create a client secret (Certificates & secrets → New client secret) — this is the `app_password`.
3. **Create an Azure Bot resource** (Azure Portal → Create a resource → "Azure Bot"). Free tier (F0) has no message volume charge. Link it to the Entra app from step 2. Under "Channels," enable the **Microsoft Teams** channel.
4. **Set the messaging endpoint** on the Azure Bot resource to `https://<public-endpoint>/api/messages` (see §5 for what fills in `<public-endpoint>`).
5. **Build and sideload the Teams app package** — a zip containing `manifest.json` (schema version 1.16+, referencing the bot's `Application ID`) plus two icons (192x192 color, 32x32 outline). Upload via Teams client → Apps → "Manage your apps" → "Upload an app" → "Upload a custom app." **This step requires either the user to hold upload rights personally, or a tenant admin to have enabled "Upload custom apps" in Teams Admin Center** (Teams Admin Center → Teams apps → Setup policies). This is the step most likely to silently block a well-intentioned individual user; call it out explicitly in the settings UI, before asking for Azure credentials.
6. **Personal dev/test tenant option:** if the user does not have qualifying M365 access, the free Microsoft 365 Developer Program grants an E5 sandbox tenant (subject to program eligibility and periodic renewal activity requirements) — mention this as the self-serve path for individual developers who want to build/test this integration without a corporate tenant. M365 Business Basic (~$6/user/month) is the paid fallback.

---

## 4. Protocol design (condensed)

Full protocol detail is in the master plan §3.5; this section translates it into what the Rust implementation actually needs to do.

- **Inbound:** Azure Bot Service is a mandatory relay. There is no direct client-to-Teams connection of any kind (unlike Discord's Gateway or Telegram's long-poll, both outbound-only). Azure receives the Teams-side event and re-POSTs a Bot Framework **Activity** object to *our* registered endpoint: `POST /api/messages`. This is a webhook receiver, not a client connection — closer in shape to the WhatsApp Cloud API receiver pattern than to Discord/Telegram/Slack.
- **Activity shape** (subset relevant to text messages):
  ```json
  {
    "type": "message",
    "id": "<activity id>",
    "timestamp": "2026-07-07T12:00:00.000Z",
    "serviceUrl": "https://smba.trafficmanager.net/teams/",
    "channelId": "msteams",
    "from": { "id": "29:1abc...", "name": "User Name", "aadObjectId": "..." },
    "conversation": { "id": "19:abc...@thread.v2", "tenantId": "..." },
    "recipient": { "id": "28:<bot app id>", "name": "AgentMux" },
    "text": "Hello agent"
  }
  ```
- **Every inbound request must be JWT-validated before the body is trusted** (see §11 — non-negotiable). Only after validation does the activity get normalized into `InboundMsg` and injected into the reactive bus, matching the pattern Discord's `handle_dispatch` uses for `MESSAGE_CREATE` in `agentmux-srv/src/messaging/discord/gateway.rs:378-440`.
- **Outbound / proactive messaging:** unlike Discord (fixed `channel_id`) or Telegram (fixed `chat_id`), Teams has no long-lived address you can just POST to ahead of time. The *only* way to message a user is:
  1. Reply directly within an existing turn — `POST {serviceUrl}/v3/conversations/{conversationId}/activities/{activityId}` (a reply to an inbound activity, always available with data from that inbound request), or
  2. Proactively initiate — requires a previously **persisted** `conversationReference` captured from *some* earlier inbound activity from that user, then `POST {serviceUrl}/v3/conversations` (or `.../activities`) using it. This is what makes agent-initiated ("agent speaks first") Teams messages fundamentally different infrastructure from the other four platforms — see §6.
- **Outbound auth:** Bot Framework REST calls need a bearer token obtained via OAuth2 client-credentials grant against `https://login.microsoftonline.com/{tenant_id}/oauth2/v2.0/token` (or the multi-tenant `botframework.com` endpoint depending on app type), using `app_id` + `app_password` as the client credentials, scope `https://api.botframework.com/.default`. Cache the token and refresh before its ~1hr expiry — do not fetch per-message.
- **Rich output:** Adaptive Cards (JSON body under `attachments[].content` with `contentType: "application/vnd.microsoft.card.adaptive"`), following the master plan's §3.5 example. This plays the same role as Discord's `MsgEmbed` / Slack's Block Kit.
- **No Rust SDK exists for Bot Framework.** The master plan confirms this (§3.5: "For Rust: no official SDK — raw HTTP with `reqwest` for outbound + `axum` to receive Bot Framework activities"). Unlike Discord (`twilight-rs` vs `serenity` decision) or Telegram (`teloxide`), there is no framework choice to make here — raw `reqwest` + hand-rolled `serde_json` structs, matching exactly what Discord's `rest.rs`/`types.rs` already do, is not a fallback option, it is the only option. This is, ironically, the one dimension on which Teams is *simpler* to slot into the existing codebase than Slack would be (Slack has an official but Node-only SDK, creating a language-boundary decision Teams doesn't have).

---

## 5. Tunnel / relay: Dev Tunnels vs. Azure App Service

Azure Bot Service must be able to reach the bot's `/api/messages` endpoint over public HTTPS. Two real options (the master plan §3.5 and §5.1 both mention these):

**Option A — Dev Tunnels (Microsoft's tool, free with a GitHub/Microsoft account).**
- Pros: symmetric with how WhatsApp's `TunnelManager` abstraction already needs to exist (§5.1 of the master plan; shared with WhatsApp's Cloudflare Tunnel need) — reuse the same subprocess-lifecycle code, just a different `TunnelProvider` variant. Persistent named tunnels once configured (unlike ngrok's free tier, which reassigns the URL every restart). Native fit for a desktop app: the bridge process runs locally, `devtunnel` is a companion subprocess AgentMux manages, exactly like `cloudflared` for WhatsApp.
- Cons: yet another local subprocess dependency, another point of failure that shows up as "bridge down" without an obvious cause to a non-technical user; devtunnel CLI must be installed and authenticated once per user.

**Option B — Azure App Service (F1 free tier, 60 CPU-min/day).**
- Pros: **eliminates the tunnel dependency entirely in production.** The bot endpoint runs *in* Azure, next to Azure Bot Service — no local subprocess, no NAT traversal, no "is my laptop's tunnel still up" failure mode. This is qualitatively different from every other tunnel option in this entire messaging-integrations project: it is the only "no tunnel" answer for a platform that mandates a public endpoint.
- Cons: this is no longer "AgentMux desktop app talks to messaging API" (the architecture this whole plan is built around) — it is "AgentMux desktop app talks to *an Azure-hosted proxy component* that AgentMux would need to deploy and maintain," and that proxy has to somehow route the received activity back to the user's local AgentMux instance (their agent, their reactive bus) — which reintroduces exactly the "public relay to a private machine" problem the tunnel was solving, just moved one hop over. F1's 60 CPU-min/day free quota is easy to exceed under any real traffic and the app needs to be deployed/updated by *someone* (the user, or AgentMux centrally) — a new piece of infrastructure ownership that doesn't exist for any other platform in this plan.

**Recommendation: Dev Tunnels (Option A).** It keeps the architecture consistent with the rest of the project (local process + `TunnelManager`, matching WhatsApp), keeps the user's message content and the bridge's connection to the reactive bus entirely on their own machine (no new component holding conversation data), and reuses infrastructure this project needs to build anyway for WhatsApp. Azure App Service is worth revisiting only if AgentMux ever moves to centrally hosting relay infrastructure for many users (the same trigger condition as the multi-tenant bot registration idea in §2 point 3) — at that point it stops being "one user's tunnel" and becomes "AgentMux's hosted relay," a materially different product decision out of scope here.

Config field: `messaging:teams:tunnel` — same flat-key convention as WhatsApp's (§8), value `"devtunnel"` (default if Teams is enabled) or `"manual"` (user supplies their own public URL, e.g. they already run one for other purposes).

---

## 6. `conversationReference` persistence — the one genuinely new piece of infrastructure

None of Discord, Telegram, Slack, or WhatsApp need this. Each of those sends to a fixed, pre-configured address (`channel_id`, `chat_id`, `channel_id`, `phone_number_id`+recipient) that the user types into settings once. Teams has no equivalent fixed address for *proactive* sends — the address (`conversationReference`, which bundles `conversation.id`, `serviceUrl`, `bot`, `channelId`, and the user's `from`/`user` identity) only exists after the user has messaged the bot at least once, and it must be captured from that inbound activity and persisted for later reuse. Without it, the Teams bridge can only *reply* within a turn the user started — it can never have the agent speak first (e.g., "build finished," "approval needed") the way the other four platforms can.

### 6.1 Where this lives

The existing storage layer already has exactly this shape of problem solved twice: `db_mcp_servers` (`agentmux-srv/src/backend/storage/mcp_servers.rs`) and `db_muxbus_credentials` (`agentmux-srv/src/backend/storage/migrations.rs:444-454`) are both small, `Store`-backed SQLite tables added by a per-subsystem file that does `impl Store { ... }` against the shared `rusqlite::Connection` in `Store` (`agentmux-srv/src/backend/storage/store.rs`). This is the established pattern for "a new small durable record type that isn't a full StoreObj" — reuse it rather than inventing a KV store, a JSON file on disk, or piggybacking it into `wconfig` settings (settings are for user-editable config, not for opaque runtime state captured from inbound events).

**New file:** `agentmux-srv/src/backend/storage/messaging_conversations.rs`, following the `mcp_servers.rs` template (struct + `impl Store` methods, no new module system needed — it's registered the same way `mod mcp_servers;` is in `storage/mod.rs`).

**New table** (add to the flat `CREATE TABLE IF NOT EXISTS` batch in `migrations.rs`, bump `OBJECT_SCHEMA_VERSION` from 10 to 11):

```sql
CREATE TABLE IF NOT EXISTS db_messaging_conversation_refs (
    id               TEXT PRIMARY KEY,   -- "{platform}:{user_id}", e.g. "teams:29:1abc..."
    platform         TEXT NOT NULL,      -- "teams" (future-proofs for any other proactive-messaging platform)
    external_user_id TEXT NOT NULL,      -- Teams: activity.from.id
    conversation_ref TEXT NOT NULL,      -- full serialized ConversationReference JSON
    display_name     TEXT NOT NULL DEFAULT '',
    created_at       INTEGER NOT NULL,
    updated_at       INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_messaging_conv_refs_platform ON db_messaging_conversation_refs(platform);
```

`conversation_ref` stores the whole Bot Framework `ConversationReference` object as a JSON blob (`conversation`, `serviceUrl`, `bot`, `user`, `channelId`) rather than being split into typed columns — this mirrors how `db_mcp_servers.config` already stores an opaque JSON blob (`mcp_servers.rs:9-11` comment: "the full server object JSON... that gets merged into `.mcp.json`") for the same reason: the shape is platform-defined and evolves outside our control, and we round-trip it rather than re-model it.

### 6.2 Write path

On every inbound activity in the webhook receiver (§7, `webhook.rs`), after JWT validation and before/alongside forwarding to the reactive bus: build a `ConversationReference` from the activity (`TurnContext.getConversationReference()` equivalent — done by hand since there's no SDK, straightforward field copy per Activity schema) and upsert it into `db_messaging_conversation_refs` keyed by `"teams:{from.id}"`. This keeps the stored reference fresh (covers the case where a `serviceUrl` or conversation ID changes, e.g. user moves from a 1:1 chat to a channel).

### 6.3 Read path

`POST /api/messaging/teams/send` (§10) looks up the target user's row by `external_user_id` (or by a friendly `display_name` the user configured, resolved server-side), deserializes `conversation_ref`, and uses it to either reply-in-conversation or call `POST {serviceUrl}/v3/conversations` for a fresh proactive turn. If no row exists yet for the target user, the send fails with a clear error ("no prior Teams message from this user — proactive send requires the user to have messaged the bot at least once") rather than silently no-op'ing — this is a real, expected failure mode users need surfaced (§12).

---

## 7. Rust module layout

Mirrors `agentmux-srv/src/messaging/discord/` (`mod.rs` + `gateway.rs` + `rest.rs` + `types.rs`), adapted for a webhook receiver instead of an outbound Gateway WS — no `gateway.rs` equivalent (no persistent connection to manage, no heartbeat/resume state machine), replaced by `webhook.rs` (a set of axum handler functions, not a background task).

```
agentmux-srv/src/messaging/teams/
├── mod.rs      // Config, Bridge struct, GLOBAL_BRIDGE OnceLock, init_global(), get(), send(), health()
├── webhook.rs  // axum handler(s) for POST /api/messages: JWT validation, activity parsing,
│               // conversationReference upsert, InjectionRequest into reactive bus
├── rest.rs     // outbound: OAuth2 client-credentials token fetch/cache, send-activity POST,
│               // proactive-conversation POST, Adaptive Card builder
└── types.rs    // Activity, ConversationReference, ConversationAccount, ChannelAccount,
                // AdaptiveCard wire types (serde structs), JWKS/JWT claim types
```

### 7.1 `mod.rs` — reconciling with the (not-yet-formalized) `MessagingBridge` trait

As of Discord's ship, no `MessagingBridge` trait exists yet — `DiscordBridge` is a concrete struct with an ad hoc method set (`init_global`, `get`, `send`, `health`; see `agentmux-srv/src/messaging/discord/mod.rs:49-96`). The Telegram spec (`SPEC_MESSAGING_INTEGRATION_TELEGRAM_2026_07_07.md`) is expected to formalize this into a real trait, roughly:

```rust
trait MessagingBridge: Send + Sync {
    fn platform(&self) -> &'static str;
    fn send(&self, msg: OutboundMsg) -> Result<(), String>;  // sync, mpsc-based like Discord's
    fn health(&self) -> BridgeHealth;
}
```

`TeamsBridge` should implement this trait once it lands (reconcile the exact shape with the Telegram spec at implementation time — do not implement Teams first and let the trait be shaped around it, since Teams' `send()` has a materially different failure mode — "no conversationReference for this user" — than the other platforms' "network/rate-limit error," and the trait's `Result<(), String>` error type needs to accommodate that cleanly for all implementers, not just Teams).

Structural difference from Discord's `mod.rs`: Discord's `init_global` spawns a background tokio task (`gateway::run_gateway_loop`) that owns the persistent WS connection and both directions of traffic. Teams has no persistent connection to own — inbound arrives via axum route registered once at server startup (§9), and there is no long-running task loop at all. `TeamsBridge` still holds an `mpsc::UnboundedSender<OutboundMsg>` for symmetry with the trait/other bridges, but the receiving side is a lightweight task that just calls `rest::send_activity` per message — no reconnect/resume state machine needed, no `Session` struct equivalent.

```rust
pub struct TeamsConfig {
    pub app_id: String,
    pub app_password: String,   // client secret, keychain-backed in practice
    pub tenant_id: String,
    pub tunnel: TunnelSetting,  // "devtunnel" | "manual"
}

pub struct TeamsBridge {
    outbound_tx: mpsc::UnboundedSender<OutboundMsg>,
    health: Arc<Mutex<BridgeHealth>>,
    // token cache lives in rest.rs behind a Mutex<Option<CachedToken>>, not here —
    // mirrors keeping wire-protocol state out of the bridge struct in Discord's design.
}

static GLOBAL_BRIDGE: OnceLock<TeamsBridge> = OnceLock::new();

impl TeamsBridge {
    pub fn init_global(config: TeamsConfig, http: reqwest::Client) { /* spawns outbound-send task, not a connection loop */ }
    pub fn get() -> Option<&'static TeamsBridge> { GLOBAL_BRIDGE.get() }
    pub fn send(&self, msg: OutboundMsg) -> Result<(), String> { /* enqueue; async task resolves conversationReference + posts */ }
    pub fn health(&self) -> BridgeHealth { /* Connected once webhook has received ≥1 valid activity; Connecting until then */ }
}
```

Note `health()`'s semantics differ from Discord's meaningfully: Discord's `Connected` means "Gateway WS is currently open." Teams has no persistent connection to be "connected" to — there is nothing to be up or down except the local webhook receiver (which is just always-up as part of the main axum server) and the Dev Tunnel subprocess. Recommend: `Connected` = tunnel is up and endpoint is registered; `Error` = tunnel down or last outbound send failed; `latency_ms`/`reconnect_count` are not meaningful for this platform and should read `None`/`0` rather than being repurposed.

---

## 8. Config schema additions

Following the **actual** flat serde-renamed key convention in `agentmux-srv/src/backend/wconfig/types.rs:300-324` — **not** the master plan's §2.6 `[messaging.teams]` TOML table example, which does not match how config is actually implemented anywhere in this codebase and should be treated as informal/illustrative only, not a schema reference. Discord's real fields (`messaging_discord_enabled`, `messaging_discord_token`, `messaging_discord_channel`, `messaging_discord_target`, `messaging_discord_guild`) are flat `Option<T>`/`bool`/`String` fields on the single settings struct, each with its own `#[serde(rename = "messaging:discord:...")]`. Teams follows the identical shape:

```rust
// -- Teams messaging bridge settings --
// See docs/specs/SPEC_MESSAGING_INTEGRATION_TEAMS_2026_07_07.md.
// NOTE: Teams requires an M365 org + Azure Bot resource + admin-enabled
// sideloading — see §3 of the spec. This is not self-serve for personal
// Microsoft accounts; the settings UI should gate on user confirmation
// before accepting these fields.

/// Master enable for the Teams messaging bridge.
#[serde(rename = "messaging:teams:enabled", default, skip_serializing_if = "is_false")]
pub messaging_teams_enabled: bool,

/// Entra ID application (client) ID for the registered bot app.
#[serde(rename = "messaging:teams:app_id", default, skip_serializing_if = "Option::is_none")]
pub messaging_teams_app_id: Option<String>,

/// Client secret for the Entra ID app. Treat as a secret — do not log.
#[serde(rename = "messaging:teams:app_password", default, skip_serializing_if = "Option::is_none")]
pub messaging_teams_app_password: Option<String>,

/// Entra ID (AAD) tenant ID that the app is registered under.
#[serde(rename = "messaging:teams:tenant_id", default, skip_serializing_if = "Option::is_none")]
pub messaging_teams_tenant_id: Option<String>,

/// Agent ID that receives inbound Teams messages via the reactive bus.
/// Absent → messages are logged but not forwarded to any agent.
#[serde(rename = "messaging:teams:target", default, skip_serializing_if = "Option::is_none")]
pub messaging_teams_target: Option<String>,

/// Tunnel provider for the public /api/messages endpoint. "devtunnel" (default
/// when enabled) or "manual" (user supplies their own public URL below).
#[serde(rename = "messaging:teams:tunnel", default, skip_serializing_if = "String::is_empty")]
pub messaging_teams_tunnel: String,

/// User-supplied public URL when tunnel = "manual". Ignored otherwise.
#[serde(rename = "messaging:teams:manual_url", default, skip_serializing_if = "Option::is_none")]
pub messaging_teams_manual_url: Option<String>,
```

`app_password` (and, for consistency with Discord's `token` field, arguably `app_id`/`tenant_id` too, though those are not secret) should route through the same OS-keychain storage path used for `messaging_discord_token` once that lands — check how Discord's settings UI actually stores the token today (as of the Discord PoC ship it may still be plaintext-in-settings pending a keychain integration pass; if so, Teams should not regress ahead of that fix and should follow whatever Discord ends up doing, not invent a separate path).

---

## 9. Startup wiring

Mirrors the Discord block in `agentmux-srv/src/main.rs:725-753`, with one added step: the tunnel must be up and its URL known *before* the bot's messaging endpoint is meaningfully reachable, so — unlike Discord, which just spawns its gateway task — Teams startup has a real sequencing dependency (same concern flagged for WhatsApp in the task brief).

```rust
// Teams messaging bridge — receives Bot Framework activities via the shared
// axum server at POST /api/messages. Requires a public tunnel (Dev Tunnels
// by default) since Azure Bot Service relays to a public HTTPS endpoint.
// Set messaging:teams:enabled + app_id/app_password/tenant_id in settings.json
// to activate. See docs/specs/SPEC_MESSAGING_INTEGRATION_TEAMS_2026_07_07.md.
{
    let settings = config_watcher.get_settings();
    if settings.messaging_teams_enabled {
        match (
            settings.messaging_teams_app_id.clone(),
            settings.messaging_teams_app_password.clone(),
            settings.messaging_teams_tenant_id.clone(),
        ) {
            (Some(app_id), Some(app_password), Some(tenant_id))
                if !app_id.is_empty() && !app_password.is_empty() =>
            {
                // 1. Ensure the tunnel is up first — the bridge has nothing
                //    useful to do until Azure can reach us. This is a hard
                //    ordering requirement, unlike Discord's fire-and-forget
                //    gateway spawn: an unregistered/stale tunnel URL means
                //    every inbound message from Teams silently never arrives
                //    (Azure retries briefly, then gives up — no local error).
                let tunnel = tunnel_manager::ensure_url(TunnelProvider::from_settings(&settings)).await;
                match tunnel {
                    Ok(public_url) => {
                        messaging::teams::TeamsBridge::init_global(
                            messaging::teams::TeamsConfig {
                                app_id,
                                app_password,
                                tenant_id,
                                target_agent: settings.messaging_teams_target.clone(),
                            },
                            reqwest::Client::new(),
                        );
                        tracing::info!(
                            "teams bridge: initialized, public endpoint {}/api/messages \
                             (must match the Azure Bot resource's messaging endpoint)",
                            public_url
                        );
                    }
                    Err(e) => {
                        tracing::warn!("teams bridge: tunnel setup failed, bridge not started: {e}");
                    }
                }
            }
            _ => {
                tracing::warn!(
                    "teams bridge: enabled but app_id/app_password/tenant_id are not fully set in settings.json"
                );
            }
        }
    }
}
```

The `/api/messages` axum route itself (§10) should be registered **unconditionally** at server startup, same as `/api/messaging/status` — it just returns 503 or is a no-op if `TeamsBridge::get()` is `None`, matching the existing `handle_discord_send` pattern in `messaging_handlers.rs:63-72`. This avoids needing to conditionally register routes based on settings, which the current router doesn't appear to support cleanly.

`tunnel_manager` here refers to the shared `TunnelManager` abstraction from the master plan §5.1 — this spec assumes it gets built as part of (or before) WhatsApp's bridge, since WhatsApp needs it too and is sequenced ahead of Teams regardless of this spec's §2 recommendation. Teams does not introduce the tunnel abstraction; it is the second consumer of it.

---

## 10. HTTP endpoints

Extends `agentmux-srv/src/server/messaging_handlers.rs`'s existing routes (`GET /api/messaging/status`, `POST /api/messaging/discord/send`) with two more:

### 10.1 `POST /api/messaging/teams/send`

Unlike Discord's `handle_discord_send` (stateless: text + channel_id in, POST out), Teams sending must resolve a `conversationReference` first (§6.3):

```rust
#[derive(Deserialize)]
pub(super) struct TeamsSendRequest {
    /// External Teams user ID (activity.from.id) or a resolvable display name.
    pub target_user: String,
    #[serde(default)]
    pub text: String,
    /// Optional Adaptive Card body (JSON), sent as an attachment alongside/instead of text.
    pub adaptive_card: Option<serde_json::Value>,
}

pub(super) async fn handle_teams_send(
    State(state): State<AppState>,
    Json(req): Json<TeamsSendRequest>,
) -> impl IntoResponse {
    let bridge = match TeamsBridge::get() {
        Some(b) => b,
        None => return (StatusCode::SERVICE_UNAVAILABLE,
            Json(json!({"error": "teams bridge not initialized — set messaging:teams:enabled in settings"})))
            .into_response(),
    };

    // Resolve conversationReference — the step with no Discord/Slack/WhatsApp equivalent.
    let conv_ref = match state.store.messaging_conversation_ref("teams", &req.target_user) {
        Ok(Some(r)) => r,
        Ok(None) => return (StatusCode::UNPROCESSABLE_ENTITY,
            Json(json!({"error": format!(
                "no prior Teams message from user '{}' — proactive send requires the user \
                 to have messaged the bot at least once first", req.target_user)})))
            .into_response(),
        Err(e) => return (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({"error": e.to_string()}))).into_response(),
    };

    match bridge.send(OutboundMsg { /* text, embed→adaptive_card mapping, conv_ref carried via reply_to or a Teams-specific extension */ }) {
        Ok(()) => Json(json!({ "ok": true })).into_response(),
        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, Json(json!({ "error": e }))).into_response(),
    }
}
```

Note `OutboundMsg` (`agentmux-srv/src/messaging/mod.rs:33-43`) as it exists today has no field for "which external user" — its `channel_id`/`reply_to` model fits Discord/Slack/Telegram's fixed-channel-address model. Teams needs the resolved `conversationReference` (or at minimum the `target_user` key to re-resolve it) threaded through to `rest.rs`. Cleanest option: resolve the `conversationReference` in the handler (as above) and pass it to the bridge as part of a Teams-specific outbound message type that wraps `OutboundMsg`, rather than stretching the shared `OutboundMsg` struct with a Teams-only field — keep the shared envelope shared, and let each bridge's `send()` accept `OutboundMsg` plus whatever platform-specific addressing it privately needs resolved before calling `send()`. This is a concrete decision the implementer should reconcile with however Telegram/WhatsApp's specs end up shaping `OutboundMsg` — flagged as an open point in §14.

### 10.2 `POST /api/messages` — Bot Framework activity receiver

This is the inbound side; registered as a top-level route (not under `/api/messaging/`, since Azure Bot Service expects exactly `/api/messages` as the path — this is fixed by the Bot Framework contract, not our naming convention, and must match what's configured on the Azure Bot resource in §3 step 4).

```rust
pub(super) async fn handle_bot_framework_activity(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> impl IntoResponse {
    // 1. JWT validation is mandatory and happens before anything else touches
    //    the body — see §11. Reject with 401 on any validation failure.
    let claims = match webhook::validate_jwt(&headers, &state.teams_app_id).await {
        Ok(c) => c,
        Err(e) => {
            tracing::warn!("teams webhook: JWT validation failed: {e}");
            return StatusCode::UNAUTHORIZED.into_response();
        }
    };

    let activity: Activity = match serde_json::from_slice(&body) {
        Ok(a) => a,
        Err(e) => {
            tracing::warn!("teams webhook: activity parse error: {e}");
            return StatusCode::BAD_REQUEST.into_response();
        }
    };

    webhook::handle_activity(activity, claims, &state).await;

    // Bot Framework expects a 200 with an empty body (or 202) to acknowledge
    // receipt — it does not wait for the reply to be sent synchronously.
    StatusCode::OK.into_response()
}
```

This route is registered on the **existing** axum server (same `Router` that serves `/api/messaging/*` and everything else) — not a second HTTP server on a different port. This matches the framing in the task brief and the general shape WhatsApp's webhook receiver needs too (both are "add an axum route," not "stand up a new server").

---

## 11. Security — JWT validation (non-negotiable)

This is the single most important correctness/security requirement in this spec, called out per the task brief as non-negotiable, and worth stating precisely rather than just gesturing at "validate the JWT":

1. Every `POST /api/messages` request carries an `Authorization: Bearer <jwt>` header. **Reject any request missing this header outright — do not process the body.**
2. Fetch (and cache, with reasonable TTL — the keys rotate infrequently) the OpenID configuration at `https://login.botframework.com/v1/.well-known/openidconfiguration`, which points to the JWKS at `https://login.botframework.com/v1/.well-known/keys`. Do not hardcode key material; always resolve via the JWKS endpoint so key rotation doesn't break the bridge.
3. Validate the JWT signature against the matching key in the JWKS (match by `kid` in the JWT header).
4. Validate claims:
   - `iss` (issuer) must be `https://api.botframework.com`.
   - `aud` (audience) must equal our own `app_id` (the Entra app's client ID) — **this is the check that stops another Azure Bot Service tenant/app from being able to inject fake activities into our endpoint.** Do not skip this even though signature validation alone might feel sufficient — the JWKS keys are shared across all Bot Framework apps, so audience is what scopes trust to *this* bot specifically.
   - `exp`/`nbf` (standard expiry window) must be valid at request time.
   - `serviceurl` claim (if present) should match the activity body's `serviceUrl` field — mismatch is a signal of a forged/replayed request.
5. Only after all of the above passes should the activity body be parsed and forwarded to the reactive bus.
6. As a secondary layer (defense in depth, not a substitute for JWT validation): once `target_agent` and any per-tenant/per-user allowlist config exist, apply the same "allowlist-first" principle the master plan states for every platform (§6 point 2) — only forward activities from the configured `tenant_id`; reject and do not respond to messages from other tenants even if the JWT is otherwise valid (relevant if the Entra app is ever registered as multi-tenant, §14).

No Rust crate in this codebase currently does Bot Framework JWT validation — implement with a general JWT library (e.g. `jsonwebtoken` crate, already a reasonable ecosystem-standard choice) doing RS256 verification against the fetched JWKS, structured similarly to how any other JWKS-based verification would be done; there is nothing Teams-specific about the JWT mechanics themselves, only about which issuer/audience/JWKS URL to point at.

---

## 12. Known failure modes / rate limits

From the master plan §3.5, plus failure modes specific to the proactive-messaging design in §6:

| Failure mode | Cause | Mitigation |
|---|---|---|
| Inbound messages never arrive, no local error | Tunnel URL stale/expired, or Azure Bot resource's messaging endpoint out of sync with current tunnel URL | Dev Tunnels should use a **persistent named tunnel** (not ephemeral), configured once; health check should periodically verify the tunnel is still serving the expected URL, not just that the subprocess is alive |
| `POST /api/messaging/teams/send` fails with "no prior message from user" | No `conversationReference` row yet for that user (§6.3) | Expected, not a bug — surface clearly in UI/error message; document that the user must message the bot first before the agent can speak first to them |
| 401 on inbound webhook | JWT validation failure — could be a genuine attack, could be a misconfigured `app_id`/tenant, could be JWKS cache staleness after Microsoft rotates keys | Log the specific claim that failed (issuer/audience/expiry/signature) to distinguish attack from misconfiguration; do not silently drop — surface in bridge health as an error state |
| Rate limiting | Per-bot-per-thread: 7 sends/second, 60/30s, 1800/hour. Global per-app-per-tenant: 50 RPS (master plan §3.5) | Queue and backoff like Discord's `rest.rs` 429 handling; per-thread limits mean a burst to one conversation shouldn't starve others — track per-`conversation.id`, not globally, mirroring Telegram's per-chat vs. global scope distinction (master plan §3.2) |
| Sideloading blocked by tenant policy | Admin has not enabled "Upload custom apps," discovered only at step 5 of setup | Cannot be mitigated in code — this is why §3's setup checklist puts this check early and the settings UI should warn about it before collecting Azure credentials, not after |
| `app_password` (client secret) expiry | Entra client secrets have a mandatory expiry (max ~24 months) | Surface expiry date in bridge health/settings if obtainable from the Entra app metadata; otherwise, document that the user must rotate manually and update settings — bridge should fail loudly (401 from Microsoft's token endpoint) rather than silently stop sending |
| Duplicate/out-of-order activities | Azure may retry delivery on transient failure | Bot Framework activities carry a stable `id` — dedupe on `(conversation.id, activity.id)` before forwarding to the reactive bus, similar in spirit to Telegram's offset-based idempotency requirement (master plan §3.2), though the mechanism differs (dedup set, not offset) since delivery here is push-based, not poll-based |

---

## 13. Implementation checklist — phased, PR-sized

Assuming the §2 gate is met and implementation is greenlit:

1. **PR 1 — Storage + shared `TunnelManager` foundation** (if not already landed by WhatsApp's spec):
   - `agentmux-srv/src/backend/storage/messaging_conversations.rs` + migration bump to `OBJECT_SCHEMA_VERSION = 11` (or whatever it is by then), per §6.1.
   - `TunnelManager` with at least `DevTunnel` and `Manual` providers wired (may already exist from WhatsApp's PR — if so, this PR just adds the `DevTunnel` variant if WhatsApp only built `CloudflareTunnel`).
2. **PR 2 — Teams module skeleton + JWT validation + webhook receiver, no outbound yet:**
   - `agentmux-srv/src/messaging/teams/{mod.rs,webhook.rs,types.rs}`.
   - `POST /api/messages` route wired unconditionally in the server, 503 until configured.
   - JWT validation against Bot Connector JWKS (§11) — this should ship even before outbound works, since an unvalidated receiver must never be exposed even transiently.
   - `conversationReference` capture + upsert into the new table on every valid inbound activity.
   - Success criteria: bot appears reachable to Azure (messaging endpoint healthcheck passes), inbound text messages get logged and a row appears in `db_messaging_conversation_refs`; no reply sent yet.
3. **PR 3 — Outbound send (reply + proactive) + config + startup wiring:**
   - `rest.rs`: OAuth2 client-credentials token fetch/cache, reply-to-activity POST, proactive-conversation POST.
   - `POST /api/messaging/teams/send` per §10.1.
   - Config fields per §8, startup wiring per §9.
   - Success criteria: user messages the bot in Teams, agent (or a manual test POST) replies, reply appears in the real Teams client; a second, unprompted proactive send to the same user also succeeds using the persisted `conversationReference`.
4. **PR 4 — Adaptive Cards + reactive bus integration:**
   - Adaptive Card builder (rendering the shared `AgentOutput`/`MsgEmbed`-equivalent structure the way Discord renders to `MsgEmbed`, once that shared shape is finalized across platforms).
   - Full inbound → reactive bus → agent → outbound round trip via `InjectionRequest`, matching Discord's `gateway.rs:406-439` pattern.
   - Warden widget "Internet" section row for Teams bridge health (`BridgeHealth`, per §5.4 of the master plan) — update `widgets.json`'s Teams pane description away from "(bridge Phase 3)" once this lands.
5. **PR 5 — Settings UI + setup documentation:**
   - Settings pane gating on the §3 checklist (M365 org confirmation, sideloading admin check) before accepting Azure credentials.
   - Full walkthrough doc for Entra app registration, Azure Bot resource creation, manifest build, sideloading.

Each PR should land only when the §2 gating condition (real user demand from an M365-tenant user) is active — do not build ahead of that trigger speculatively, per §2's recommendation.

---

## 14. Open decision points for the implementer

- **Multi-tenant vs. single-tenant Entra app registration.** This spec assumes single-tenant (the user's own Entra app, their own tenant) throughout, matching "every user configures their own Azure Bot" in §2. A multi-tenant app (AgentMux registers one Entra app usable across any consenting tenant) would remove most of steps 2–3 in §3 for end users but shifts significant ownership (consent flow, admin consent UX, a shared app AgentMux must maintain) onto the AgentMux project itself. Flagged in §2 as the condition under which this spec's low-priority sequencing should be revisited — not decided here.
- **`OutboundMsg` extension shape for per-user addressing.** §10.1 flags that Teams needs to thread a resolved `conversationReference` (or a `target_user` key) through to the bridge's `send()`, which the current shared `OutboundMsg` struct has no field for. Whether this becomes a new optional field on the shared struct, a platform-specific wrapper type, or something the `MessagingBridge` trait's `send()` signature accommodates generically (e.g., an `Any`-like extension point) should be decided jointly with however Telegram/WhatsApp's specs evolve `OutboundMsg` — resolve at implementation time, not speculatively here.
- **Token cache invalidation on `app_password` rotation.** If a user rotates their Entra client secret without restarting AgentMux, the cached OAuth2 bearer token remains valid until its own ~1hr expiry, but the *next* token fetch will fail if the settings-file secret was updated without the running process re-reading it. Decide whether `rest.rs`'s token cache should watch the config-reload signal (the existing `config_watcher` mechanism startup already uses) and proactively invalidate, or just let the natural next-fetch failure surface as a bridge health error.
- **Dedup window for retried activities** (§12, last row): an in-memory `HashSet<(conversation_id, activity_id)>` with periodic eviction is sufficient for a single-process desktop app and avoids a third storage table; confirm this is adequate rather than persisting dedup state, since a restart naturally clearing the dedup set is an acceptable (Azure will simply retry-deliver, which is idempotent at the reactive-bus level via `InjectionRequest.request_id`, matching how Discord already passes `msg.id` as `request_id` in `gateway.rs:420`).
- **Whether `health()`'s `Connected` state should also verify the Dev Tunnel subprocess is alive**, not just that the local webhook route exists — since (per §12's first failure mode) the most likely real-world failure is a silently-stale tunnel with no local symptom. Recommend yes — `TeamsBridge::health()` should poll `TunnelManager::status()` rather than reporting `Connected` purely because the axum route was registered at startup.
