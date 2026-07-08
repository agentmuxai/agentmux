# Spec: Messaging App Integration — WhatsApp

**Date:** 2026-07-07
**Status:** Draft — ready to implement (Cloud API); Path B (Baileys) deferred out of v1, see §2
**Scope:** Rust bridge design for WhatsApp inside `agentmux-srv`, adapting the master messaging plan and the shipped Discord bridge shape to WhatsApp's webhook-based protocol

---

## 1. Goal and context

Extend AgentMux's messaging integrations (see `docs/specs/SPEC_MESSAGING_INTEGRATIONS_PLAN_2026_06_24.md`, the master plan) to WhatsApp. The pane layer already exists — `defwidget@whatsapp` in `agentmux-srv/src/config/widgets.json` (lines ~166-180) points a browser pane at `https://web.whatsapp.com/`, with description `"WhatsApp Web — real interface, agent-connected (bridge Phase 3)"`. This spec designs the bridge layer: the background process that lets an agent read/send WhatsApp messages through the real Meta Cloud API while the user keeps using the native WhatsApp Web UI in the pane, exactly as Discord already does for Discord.com.

Discord shipped in PR #1763 (merged 2026-06-24) as a pure-Rust bridge inside `agentmux-srv` — no Node.js sidecar, no Electron (this app is CEF-hosted Rust; `agentmux-cef` is the host process, there is no Electron main process anywhere in this codebase). That fact drives every architectural call in this spec, and it directly contradicts two assumptions in the master plan (§2.6's config format, and §3.4/§4's implicit Electron-Node sidecar for Baileys). Both are corrected below.

This spec covers **WhatsApp only**. It reuses the master plan's protocol research (webhook verification, signature validation, 24h window, pricing/rate limits) rather than re-deriving it, and translates it into the actual Rust module shape established by `agentmux-srv/src/messaging/discord/`.

---

## 2. The two-path decision: Cloud API vs. Baileys

### 2.1 Recommendation: ship Cloud API only in v1. Do not build Path B (Baileys) in this codebase.

The master plan (§3.4, §4 Phase 4) called for supporting both paths, with Baileys running "in the Electron main process (pure Node.js)." That assumption is now known to be false — there is no Electron process in AgentMux's architecture, and Discord's implementation proves the intended shape for every messaging bridge is a Tokio task inside `agentmux-srv`. Baileys is a Node.js-only library (WhatsApp Web's Noise+Signal protocol implementation); there is no maintained Rust port. This is a real architectural fork, not a config toggle, and the plan ducked it.

Weighing the three options named in the task brief:

**(a) Standalone Node.js subprocess, supervised by `agentmux-srv`, talking over a local Unix socket or loopback HTTP.**
This is the closest match to what the master plan wanted, and it is technically buildable — `agentmux-srv` already spawns and supervises subprocesses elsewhere (see the toolchain manager and persistent shell node specs). But it introduces a Node.js runtime dependency this app has never had before: bundling a Node runtime (or requiring the user to have one — `docs/specs/nodejs-detection-notification.md` shows this project already has to handle "user doesn't have Node" as a real, recurring failure case for *other* features), managing its lifecycle across app restarts/crashes/updates, persisting Baileys' encrypted auth-state blob across sessions, and building a second wire protocol (subprocess IPC) in addition to the Meta webhook protocol. That is a large amount of net-new infrastructure for one platform's unofficial path.

**(b) Cloud API only — drop Baileys from v1.**
The Cloud API is already the "official, no-ban-risk, framework-consistent" path per the master plan's own framing (§3.4 Path A), and it fits the Discord-proven module shape almost exactly (webhook receiver in place of Gateway WS, REST send is REST send either way). It has zero new runtime dependencies. Its downsides — business verification lead time, a public HTTPS endpoint via tunnel, template-message costs outside the 24h window — are real but bounded and already well understood from the plan's research.

**(c) A maintained Rust WhatsApp Web protocol crate.**
I am not confident one exists in a maintained state as of this writing. There have been scattered community experiments reverse-engineering the WhatsApp Web multi-device protocol in Rust, but nothing at the maturity or activity level of Baileys (which itself requires continuous protocol-drift maintenance from a team actively tracking WhatsApp Web releases). Do not assume such a crate exists; do not build on top of one without a concrete, current crates.io link verified at implementation time. Treat this option as closed unless that verification happens.

**Decision: (b).** Ship Cloud API only. The Node-subprocess cost in (a) is disproportionate to the value of an unofficial, ban-risk-bearing path when the official path already covers the core "agent reads/sends WhatsApp through the real UI" use case for the overwhelmingly common case (a developer's own number, moderate volume, personal productivity use). The ban-risk math in the plan itself (§3.4: 15-30% ban rate for bots that send proactive messages, which is exactly what an AI agent bridge does most of the time) combined with Meta's AI-chatbot policy tension (see §10 below) makes Baileys a worse bet than the plan credited it for. If user demand later justifies it, Path B should be revisited as its own spec with the Node-subprocess architecture spelled out in full — it should not be smuggled in as a config variant of this bridge.

**Consequence for `messaging/mod.rs` and the eventual `MessagingBridge` trait:** design the trait (see §5) so a future `WhatsAppBaileysBridge` could implement it later without a rework — the trait itself is protocol-agnostic — but write zero Baileys code now. The `messaging:whatsapp:mode` config key from the master plan is dropped; v1 has exactly one mode.

### 2.2 What this changes vs. the master plan

- Master plan §4 Phase 4 "Deliverables" list a Baileys Electron sidecar with QR code UI and ban-risk warning banner as in-scope for the WhatsApp phase. Out of scope for this spec/v1.
- Master plan §2.6 config shows `mode = "cloud_api" | "bridge"`. Dropped; see §6.
- Master plan §6.6 ("WhatsApp ban risk disclosure... persistent warning in Settings UI, user must acknowledge before enabling") is Path-B-only and therefore not implemented in v1. If Path B ships later, that requirement carries forward unchanged.

---

## 3. Cloud API protocol design (condensed)

This restates the master plan's §3.4 Path A research, condensed to what the implementer needs, translated into this repo's conventions. Full protocol detail (rate limit tables, template examples) is in the master plan; not repeated here except where it drives a design decision.

### 3.1 Webhook verification handshake (one-time, on registration)

Meta calls `GET /webhook/whatsapp?hub.mode=subscribe&hub.verify_token={TOKEN}&hub.challenge={RANDOM}`. The handler must:
1. Compare `hub.verify_token` against the configured `messaging:whatsapp:webhook_verify_token` using constant-time comparison.
2. On match, return HTTP 200 with the raw `hub.challenge` value as the plaintext body (not JSON-wrapped).
3. On mismatch, return 403.

This must succeed before Meta will start delivering inbound webhook POSTs, and Meta re-runs this handshake any time the webhook URL or verify token is changed in the App Dashboard — this is why tunnel sequencing matters (§4).

### 3.2 Inbound signature validation (every POST, non-negotiable)

Every inbound `POST /webhook/whatsapp` carries `X-Hub-Signature-256: sha256=<hex>`, an HMAC-SHA256 of the *raw* request body using the Meta App Secret (`messaging:whatsapp:app_secret`) as key. The handler must:
1. Read the raw body bytes before any JSON deserialization (axum's `Bytes` extractor, not `Json<T>`, so the exact bytes that were signed are available).
2. Compute HMAC-SHA256(app_secret, raw_body).
3. Compare to the header value using a constant-time comparator (e.g. `subtle::ConstantTimeEq`, or `ring::hmac::verify` which is constant-time internally).
4. Reject with 401 on any mismatch or missing header — before touching the payload. No payload parsing, no logging of body content, on a failed check.

This is the one new wire-security primitive this bridge needs that Discord's Gateway-only, outbound-only design never required (Discord never receives unauthenticated inbound traffic from the public internet). See §9 for the exact code shape.

### 3.3 24-hour customer service window

WhatsApp only allows free-form text replies within 24 hours of the user's last inbound message to that phone number. Outside the window, only pre-approved template messages may be sent. The bridge must track, per `from_id` (the user's WhatsApp phone number in the inbound payload), the timestamp of their last inbound message, and on outbound send: if `now - last_inbound_at <= 24h`, send free-form text; else, fall back to a template message (or fail with a clear error surfaced to the agent, if no template is configured — do not silently drop).

State: an in-memory `HashMap<String, u64>` (phone number → last-inbound-unix-ms) inside the bridge, guarded the same way `DiscordBridge`'s health is (`Arc<Mutex<...>>`). Not persisted to disk in v1 — a restart simply means the bridge falls back to templates until the next inbound message re-opens the window, which is an acceptable degradation (matches the plan's "best pattern: user initiates → agent responds within window" framing).

### 3.4 Template fallback

Template name/language are configured per-deployment (`messaging:whatsapp:fallback_template`, `messaging:whatsapp:fallback_template_lang`, default `"en_US"`). If unset and the window has expired, `send()` returns `Err("whatsapp: 24h window expired and no fallback template configured")` rather than attempting delivery — Meta would reject a free-form send outside the window anyway (error code 131047), so this fails fast locally instead of round-tripping to the API first.

### 3.5 Tunnel requirement

Unlike Discord (pure outbound Gateway WS, works behind NAT with zero public exposure), the Cloud API's webhook delivery mechanism requires Meta to reach an HTTPS URL you control. There is no long-polling alternative for Cloud API (that's Baileys' advantage, which is exactly why Path B looked attractive in the plan — see §2). A public tunnel is mandatory whenever `messaging:whatsapp:enabled = true`. See §4.

---

## 4. Tunnel management

### 4.1 Applying the plan's `TunnelManager` (§5.1) to WhatsApp specifically

The master plan's §5.1 sketches a shared `TunnelManager` abstraction because three platforms (WhatsApp Cloud API, Slack HTTP fallback, Teams) may need a public URL. WhatsApp is the first of the three to actually get built, so this spec is where `TunnelManager` needs to go from sketch to real code — but scoped minimally: only the `CloudflareTunnel` provider needs to exist for this PR; `DevTunnel`, `NgrokFree`, and `Manual` variants can stay as enum cases with `todo!()`/unimplemented bodies until Teams or a Slack HTTP-fallback path actually needs them, so this spec doesn't block on unrelated provider work.

```rust
// agentmux-srv/src/messaging/tunnel.rs — new shared module (not whatsapp-specific)

pub enum TunnelProvider {
    CloudflareTunnel { name: String, domain: String },
    DevTunnel { name: String },      // unimplemented until Teams
    NgrokFree,                        // unimplemented until needed
    Manual { url: String },           // user supplies their own — always available, zero-code
}

pub struct TunnelManager {
    provider: TunnelProvider,
    local_port: u16,
    child: Mutex<Option<tokio::process::Child>>,
    status: Mutex<TunnelStatus>,
}

#[derive(Clone, Debug, PartialEq)]
pub enum TunnelStatus {
    Down,
    Starting,
    Up { url: String },
    Error(String),
}

impl TunnelManager {
    /// Starts the tunnel subprocess if not already running, and blocks (with a
    /// timeout, e.g. 30s) until the tunnel reports itself connected — determined
    /// by scraping cloudflared's stdout for its "Registered tunnel connection"
    /// line, since cloudflared has no local status HTTP endpoint by default.
    pub async fn ensure_url(&self) -> Result<String, String>;
    pub fn current_url(&self) -> Option<String>;
    pub fn status(&self) -> TunnelStatus;
}
```

For `Manual { url }` (user runs their own reverse proxy / already has a domain routed to the machine), `ensure_url()` is a no-op that returns the configured URL immediately — this is the fallback path for users who don't want AgentMux managing a `cloudflared` subprocess at all, and it should be the documented "if in doubt" option since it has zero new subprocess-supervision surface.

### 4.2 Cloudflare Tunnel setup (user-performed, one-time)

Per the master plan's §3.4 research, unchanged:
```bash
cloudflared tunnel login
cloudflared tunnel create agentmux-whatsapp
cloudflared tunnel route dns agentmux-whatsapp wa.yourdomain.com
```
AgentMux then manages `cloudflared tunnel run agentmux-whatsapp` as a supervised subprocess (same pattern as other externally-managed subprocesses in this codebase — see the toolchain manager). The resulting `https://wa.yourdomain.com` is registered once in Meta's App Dashboard and does not change across restarts, which is why Cloudflare Tunnel (not the free ngrok tier, which rotates URLs) is the recommended default.

**Security-relevant detail specific to this design, not present in the master plan:** the axum router in `agentmux-srv/src/server/mod.rs` applies `auth_middleware` (`X-AuthKey` header check) to nearly every route via `route_layer`, including the existing `/api/messaging/*` routes (lines ~329-330, confirmed by reading `server/mod.rs`). If the Cloudflare Tunnel ingress is configured to forward the tunnel's entire hostname to the local server's port with no path restriction, **the whole authed API surface becomes reachable from the public internet** (protected only by the same `X-AuthKey` that local callers use — a single static per-instance secret, not designed to withstand internet-facing brute force). The webhook route itself is deliberately unauthenticated (Meta cannot supply `X-AuthKey`; see §5), so this is not a "the webhook is insecure" problem, it's a "don't expose unrelated authenticated routes to the internet by accident" problem.

**Recommendation:** scope the `cloudflared` ingress rule to the webhook path only, using a `config.yml` ingress rule rather than the simpler quick-tunnel default:
```yaml
tunnel: agentmux-whatsapp
credentials-file: /path/to/creds.json
ingress:
  - hostname: wa.yourdomain.com
    path: ^/webhook/whatsapp$
    service: http://localhost:PORT
  - service: http_status:404
```
This keeps the webhook receiver on the same axum router/port as the rest of the app (per §5's recommendation) while ensuring the tunnel itself — not application-layer auth — is the boundary that keeps the authed routes off the public internet. Document this ingress config in the user-facing setup guide; it is not optional.

### 4.3 "Ensure tunnel up before webhook registration" sequencing

Startup wiring (§7) must sequence:
1. Bridge init begins → `TunnelManager::ensure_url().await` (blocks until tunnel reports connected, or times out and surfaces `BridgeStatus::Error`).
2. Compare the resolved tunnel URL + `/webhook/whatsapp` against what's currently registered in Meta's App Dashboard for this phone number (Meta does not expose a "get current webhook URL" read API for Cloud API apps in the way this implies — in practice this comparison is against the last URL AgentMux itself registered, stored locally, not queried from Meta). If it differs from the last-known-registered URL (first run, or the tunnel URL changed), log a clear one-time instruction: **the user must open Meta App Dashboard → WhatsApp → Configuration and paste the callback URL + verify token manually** — Meta's webhook subscription UI does not have a public API for programmatic registration by third-party apps; this is a manual step every time the URL changes, which is exactly why the plan recommends the durable Cloudflare named-tunnel over the free ngrok tier (URL never changes after first setup).
3. Only after the tunnel is confirmed up does the bridge mark itself `BridgeStatus::Connecting` → the webhook route becomes meaningful to Meta as soon as the verification handshake (§3.1) succeeds, which happens on Meta's schedule (whenever the user completes step 2 in the dashboard), not on a fixed timeline the bridge controls.

This differs materially from Discord, where `init_global()` unconditionally spawns the Gateway loop and there's nothing to "wait for" before the bridge can start receiving — WhatsApp's inbound path is inert until a human completes an out-of-band dashboard step, and the bridge's health status should say so explicitly (`BridgeStatus::Connecting` with an `error`-adjacent hint, not a bare "Connecting" that looks like it'll resolve itself).

---

## 5. Rust module layout

Mirrors `agentmux-srv/src/messaging/discord/` (`mod.rs` + `gateway.rs` + `rest.rs` + `types.rs`), with `gateway.rs` (outbound-only Gateway WS client) replaced by `webhook.rs` (inbound HTTP receiver — the genuinely new shape versus Discord, which only ever makes outbound connections).

```
agentmux-srv/src/messaging/whatsapp/
├── mod.rs        // Config, Bridge, GLOBAL_BRIDGE, init_global(), get(), send(), health()
├── webhook.rs     // axum handlers: GET verification handshake, POST inbound receiver + HMAC check
├── rest.rs        // outbound send via Graph API (POST /{phone_number_id}/messages)
└── types.rs       // Graph API + webhook payload wire types (serde structs)

agentmux-srv/src/messaging/
├── mod.rs         // add `pub mod whatsapp;` alongside existing `pub mod discord;`
└── tunnel.rs      // new shared TunnelManager (§4.1) — not whatsapp-specific, lives at this level
```

### 5.1 `mod.rs` — shape

```rust
// agentmux-srv/src/messaging/whatsapp/mod.rs

mod webhook;
pub mod rest;
pub mod types;

use std::collections::HashMap;
use std::sync::{Arc, Mutex, OnceLock};

use crate::messaging::{BridgeHealth, OutboundMsg};
use crate::messaging::tunnel::TunnelManager;

#[derive(Debug, Clone)]
pub struct WhatsAppConfig {
    pub phone_number_id: String,
    pub access_token: String,        // System User token (permanent)
    pub app_secret: String,          // for X-Hub-Signature-256 validation
    pub webhook_verify_token: String,
    pub target_agent: Option<String>,
    pub fallback_template: Option<String>,
    pub fallback_template_lang: String, // default "en_US"
    pub tunnel_domain: String,       // e.g. "wa.yourdomain.com"
    pub tunnel_name: String,         // e.g. "agentmux-whatsapp"
}

pub struct WhatsAppBridge {
    outbound_http: reqwest::Client,
    config: WhatsAppConfig,
    health: Arc<Mutex<BridgeHealth>>,
    /// Per-user (phone number) last-inbound timestamp, for 24h window tracking (§3.3).
    window_state: Arc<Mutex<HashMap<String, u64>>>,
    tunnel: Arc<TunnelManager>,
}

static GLOBAL_BRIDGE: OnceLock<WhatsAppBridge> = OnceLock::new();

impl WhatsAppBridge {
    /// Initialize the global WhatsApp bridge: start the tunnel, then mark the
    /// bridge ready to receive on the webhook routes (already registered on the
    /// main axum router regardless of init state — see §8). No-op if already
    /// initialized. Unlike Discord, this does not spawn a long-lived reconnect
    /// loop; the "connection" is Meta's HTTP calls to us, which are stateless.
    pub fn init_global(config: WhatsAppConfig, http: reqwest::Client) {
        let health = Arc::new(Mutex::new(BridgeHealth::connecting("whatsapp")));
        let tunnel = Arc::new(TunnelManager::new_cloudflare(
            config.tunnel_name.clone(),
            config.tunnel_domain.clone(),
            /* local_port */ crate::server::LOCAL_PORT,
        ));

        let bridge = WhatsAppBridge {
            outbound_http: http,
            config: config.clone(),
            health: health.clone(),
            window_state: Arc::new(Mutex::new(HashMap::new())),
            tunnel: tunnel.clone(),
        };

        if GLOBAL_BRIDGE.set(bridge).is_err() {
            return; // already initialized
        }

        tokio::spawn(async move {
            match tunnel.ensure_url().await {
                Ok(url) => {
                    tracing::info!(
                        "whatsapp_bridge: tunnel up at {url}, webhook callback = {url}/webhook/whatsapp \
                         — verify this is registered in Meta App Dashboard > WhatsApp > Configuration"
                    );
                    let mut h = health.lock().unwrap();
                    h.status = crate::messaging::BridgeStatus::Connected;
                }
                Err(e) => {
                    tracing::error!("whatsapp_bridge: tunnel failed to start: {e}");
                    let mut h = health.lock().unwrap();
                    h.status = crate::messaging::BridgeStatus::Error;
                    h.error = Some(format!("tunnel: {e}"));
                }
            }
        });
    }

    pub fn get() -> Option<&'static WhatsAppBridge> { GLOBAL_BRIDGE.get() }

    /// Send via Graph API. Sync signature (mpsc-free, unlike Discord) because
    /// there's no background loop to hand off to — send is a direct outbound
    /// REST call, so this is `async fn` awaited by the HTTP handler directly.
    /// (This is the one place WhatsApp's shape *simplifies* vs. Discord's
    /// mpsc-channel pattern: Discord needs the channel because REST calls must
    /// interleave with the Gateway loop's single mutable WS handle; WhatsApp
    /// has no equivalent shared mutable resource to serialize against.)
    pub async fn send(&self, msg: OutboundMsg) -> Result<(), String> {
        rest::send_message(&self.outbound_http, &self.config, &self.window_state, &msg).await
    }

    pub fn health(&self) -> BridgeHealth {
        self.health.lock().unwrap().clone()
    }

    /// Called by webhook.rs on each valid inbound message, to update the 24h
    /// window state and inject into the reactive bus.
    pub(crate) fn record_inbound(&self, from_id: &str) {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        self.window_state.lock().unwrap().insert(from_id.to_string(), now);
        let mut h = self.health.lock().unwrap();
        h.last_event_at = Some(now / 1000);
    }
}
```

**Reconciling with the eventual `MessagingBridge` trait:** the task brief specifies the trait shape as `fn platform(&self) -> &'static str`, `fn send(&self, msg: OutboundMsg) -> Result<(), String>` (sync, mpsc-based like Discord's), `fn health(&self) -> BridgeHealth`. WhatsApp's `send()` is naturally `async` (a direct REST call, no mpsc handoff needed — see the comment above). When the trait lands (concurrently with Telegram per `SPEC_MESSAGING_INTEGRATION_TELEGRAM_2026_07_07.md`, if that file exists at implementation time), reconcile by either (a) making the trait's `send` async (`async fn send(&self, msg: OutboundMsg) -> Result<(), String>`, requiring `#[async_trait]` or a Rust edition with native async-in-traits support), or (b) keeping `WhatsAppBridge::send` sync at the trait boundary by wrapping the async REST call in a small internal mpsc + background task purely for trait uniformity, even though it isn't structurally required the way Discord's is. **Recommendation: prefer (a).** Forcing a synchronous trait signature onto a bridge whose native shape is a direct async REST call adds a redundant channel and task for no benefit — Telegram's `teloxide` dispatcher is also fundamentally async, so this reconciliation problem isn't WhatsApp-specific and the trait should just be async from the start. Flag this explicitly for whoever lands the trait.

### 5.2 `webhook.rs` — inbound receiver

```rust
// agentmux-srv/src/messaging/whatsapp/webhook.rs

use axum::{
    body::Bytes,
    extract::{Query, State},
    http::{HeaderMap, StatusCode},
    response::IntoResponse,
};
use hmac::{Hmac, Mac};
use sha2::Sha256;
use subtle::ConstantTimeEq;

use super::WhatsAppBridge;
use super::types::WebhookPayload;
use crate::backend::reactive::handler::get_global_handler;
use crate::backend::reactive::types::InjectionRequest;

/// GET /webhook/whatsapp — Meta's one-time (per-URL-change) verification handshake.
pub async fn handle_verify(Query(params): Query<std::collections::HashMap<String, String>>) -> impl IntoResponse {
    let Some(bridge) = WhatsAppBridge::get() else {
        return (StatusCode::SERVICE_UNAVAILABLE, "bridge not initialized").into_response();
    };
    let mode = params.get("hub.mode").map(String::as_str);
    let token = params.get("hub.verify_token").map(String::as_str).unwrap_or("");
    let challenge = params.get("hub.challenge").cloned().unwrap_or_default();

    let expected = bridge.verify_token();
    let ok = mode == Some("subscribe")
        && token.as_bytes().ct_eq(expected.as_bytes()).unwrap_u8() == 1;

    if ok {
        (StatusCode::OK, challenge).into_response()
    } else {
        (StatusCode::FORBIDDEN, "verification failed").into_response()
    }
}

/// POST /webhook/whatsapp — inbound message delivery. Body MUST be read as raw
/// bytes (not Json<T>) so the exact signed payload is available for HMAC check
/// before any deserialization happens.
pub async fn handle_inbound(headers: HeaderMap, body: Bytes) -> impl IntoResponse {
    let Some(bridge) = WhatsAppBridge::get() else {
        return StatusCode::SERVICE_UNAVAILABLE;
    };

    // §3.2 — signature validation is mandatory and happens before parsing.
    let sig_header = headers
        .get("X-Hub-Signature-256")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("");
    if !verify_signature(bridge.app_secret(), &body, sig_header) {
        tracing::warn!("whatsapp_bridge: rejected inbound webhook — signature mismatch");
        return StatusCode::UNAUTHORIZED;
    }

    let payload: WebhookPayload = match serde_json::from_slice(&body) {
        Ok(p) => p,
        Err(e) => {
            tracing::warn!("whatsapp_bridge: parse error: {e}");
            return StatusCode::BAD_REQUEST;
        }
    };

    for msg in payload.extract_messages() {
        bridge.record_inbound(&msg.from);
        let Some(target) = bridge.target_agent() else { continue };
        let envelope = format!("[WhatsApp {}]: {}", msg.from, msg.text);
        let handler = get_global_handler();
        let req = InjectionRequest {
            target_agent: target.clone(),
            message: envelope,
            source_agent: Some("whatsapp".to_string()),
            request_id: Some(msg.id.clone()),
            priority: None,
            wait_for_idle: false,
            jekt_tier: None,
            delivery_tier: Some("wan".to_string()),
        };
        let _ = handler.inject_message(req);
    }

    StatusCode::OK
}

fn verify_signature(app_secret: &str, body: &[u8], header: &str) -> bool {
    let Some(hex_sig) = header.strip_prefix("sha256=") else { return false };
    let Ok(expected) = hex::decode(hex_sig) else { return false };

    type HmacSha256 = Hmac<Sha256>;
    let Ok(mut mac) = HmacSha256::new_from_slice(app_secret.as_bytes()) else { return false };
    mac.update(body);
    mac.verify_slice(&expected).is_ok() // constant-time internally
}
```

Always return `200 OK` for successfully-processed (or intentionally-ignored, e.g. status-update) webhook events, even if injection into the reactive bus failed — per Meta's delivery semantics, a non-2xx response causes retries and eventually (after repeated failures over ~7 days, per the master plan §3.4) event delivery is dropped entirely; local injection failures should be logged, not surfaced as delivery failures to Meta.

### 5.3 `rest.rs` — outbound send

Mirrors Discord's `rest.rs` shape (single `send_message` function, no client struct), but adds the 24h-window branch (§3.3/§3.4):

```rust
pub async fn send_message(
    http: &reqwest::Client,
    config: &WhatsAppConfig,
    window_state: &Mutex<HashMap<String, u64>>,
    msg: &OutboundMsg,
) -> Result<(), String> {
    let to = &msg.channel_id; // WhatsApp has no channel concept; channel_id carries the recipient phone number
    let within_window = window_state
        .lock()
        .unwrap()
        .get(to)
        .map(|&last| now_ms() - last <= 24 * 60 * 60 * 1000)
        .unwrap_or(false);

    let url = format!(
        "https://graph.facebook.com/v25.0/{}/messages",
        config.phone_number_id
    );

    let body = if within_window {
        types::SendBody::text(to, &msg.text)
    } else {
        let Some(template) = config.fallback_template.as_ref() else {
            return Err("whatsapp: 24h window expired and no fallback template configured".into());
        };
        types::SendBody::template(to, template, &config.fallback_template_lang)
    };

    let resp = http
        .post(&url)
        .bearer_auth(&config.access_token)
        .json(&body)
        .send()
        .await
        .map_err(|e| format!("whatsapp rest: http error: {e}"))?;

    if !resp.status().is_success() {
        let status = resp.status();
        let text = resp.text().await.unwrap_or_default();
        return Err(format!("whatsapp rest: {status}: {text}"));
    }
    Ok(())
}
```

Note `OutboundMsg.channel_id` is repurposed as "recipient phone number in E.164 format" for WhatsApp — there is no channel/room concept on this platform, just 1:1 conversations with phone numbers. This should be called out in `OutboundMsg`'s doc comment in `messaging/mod.rs` when this lands, since the field name is Discord-flavored.

`MsgEmbed` does not map to WhatsApp (no rich embed support in the Cloud API's free-form message types — per the master plan §5.3, `WhatsAppBridge: → text (no rich format) or template`). If `OutboundMsg.embed` is set, flatten it to plain text (`title\n\ndescription\n\nfield: value...`) rather than dropping it silently.

### 5.4 `types.rs`

Hand-rolled serde structs for: `WebhookPayload` (Meta's nested `entry[].changes[].value.messages[]` envelope — deserialize permissively, ignore `statuses` array entries which are delivery receipts, not messages), `SendBody` (text and template variants), Graph API error envelope. No SDK crate, matching Discord's `rest.rs`/`types.rs` precedent of hand-rolled JSON over `reqwest` + `serde_json`.

---

## 6. Config schema additions

Following the flat serde-renamed-key convention in `agentmux-srv/src/backend/wconfig/types.rs` (confirmed at lines ~300-324 for the shipped Discord keys) — **not** the master plan §2.6's `[messaging.whatsapp]` TOML table, which does not match how config is actually stored in this codebase. That correction applies to all five platforms in the plan, not just WhatsApp; this spec fixes it for WhatsApp's fields as the concrete example.

Add to `SettingsType` (or wherever `messaging_discord_*` fields live) in `wconfig/types.rs`:

```rust
// -- WhatsApp Cloud API bridge settings --

/// Master enable for the WhatsApp Cloud API bridge.
#[serde(rename = "messaging:whatsapp:enabled", default, skip_serializing_if = "is_false")]
pub messaging_whatsapp_enabled: bool,

/// WhatsApp Business phone number ID (from Meta App Dashboard > WhatsApp > API Setup).
#[serde(rename = "messaging:whatsapp:phone_number_id", default, skip_serializing_if = "String::is_empty")]
pub messaging_whatsapp_phone_number_id: String,

/// System User access token (permanent). Treat as a secret — do not log.
#[serde(rename = "messaging:whatsapp:access_token", default, skip_serializing_if = "Option::is_none")]
pub messaging_whatsapp_access_token: Option<String>,

/// Meta App Secret, used to validate X-Hub-Signature-256 on inbound webhooks.
/// Treat as a secret — do not log.
#[serde(rename = "messaging:whatsapp:app_secret", default, skip_serializing_if = "Option::is_none")]
pub messaging_whatsapp_app_secret: Option<String>,

/// Verify token used in the GET /webhook/whatsapp handshake. User-chosen,
/// must match what's entered in Meta App Dashboard > WhatsApp > Configuration.
#[serde(rename = "messaging:whatsapp:webhook_verify_token", default, skip_serializing_if = "Option::is_none")]
pub messaging_whatsapp_webhook_verify_token: Option<String>,

/// Agent ID that receives inbound WhatsApp messages via the reactive bus.
#[serde(rename = "messaging:whatsapp:target", default, skip_serializing_if = "Option::is_none")]
pub messaging_whatsapp_target: Option<String>,

/// Template name used for outbound sends outside the 24h customer service window.
#[serde(rename = "messaging:whatsapp:fallback_template", default, skip_serializing_if = "Option::is_none")]
pub messaging_whatsapp_fallback_template: Option<String>,

/// Template language code. Default "en_US" if unset.
#[serde(rename = "messaging:whatsapp:fallback_template_lang", default, skip_serializing_if = "Option::is_none")]
pub messaging_whatsapp_fallback_template_lang: Option<String>,

/// Cloudflare Tunnel name (must already exist — created via `cloudflared tunnel create`).
#[serde(rename = "messaging:whatsapp:tunnel_name", default, skip_serializing_if = "String::is_empty")]
pub messaging_whatsapp_tunnel_name: String,

/// Domain the tunnel is routed to (e.g. "wa.yourdomain.com").
#[serde(rename = "messaging:whatsapp:tunnel_domain", default, skip_serializing_if = "String::is_empty")]
pub messaging_whatsapp_tunnel_domain: String,
```

No `messaging:whatsapp:mode` key (see §2.1 — Cloud API is the only mode). If Path B is ever built as a follow-on spec, it should get its own key namespace (e.g. `messaging:whatsapp_bridge:*`) rather than retrofitting a `mode` switch onto this bridge's config, since the two paths have almost no config fields in common (phone number ID + tokens vs. QR pairing + session file path) and conflating them under one enabled/mode toggle would make both harder to reason about.

Credentials (`access_token`, `app_secret`, `webhook_verify_token`) should go through the same OS-keychain storage path as `messaging:discord:token` once that exists for Discord (per master plan §6.1 — confirm at implementation time whether Discord's token is actually keychain-backed yet or still plaintext in `settings.json`, since the shipped code in `messaging_discord_token: Option<String>` gives no indication either way from the type alone; align WhatsApp with whatever Discord's actual current-state answer is, don't let this spec assume the plan's aspiration is already true).

---

## 7. Startup wiring

Mirrors `agentmux-srv/src/main.rs` lines ~731-755 (Discord wiring), with two structural additions specific to WhatsApp: tunnel-before-webhook sequencing (already encapsulated inside `WhatsAppBridge::init_global`, see §5.1) and a startup log reminding the user to check the Meta Dashboard registration.

```rust
// agentmux-srv/src/main.rs — insert after the existing Discord bridge block (~line 755)

// WhatsApp Cloud API messaging bridge — starts the Cloudflare Tunnel, then
// waits for Meta's webhook verification handshake to complete before it can
// receive anything (unlike Discord, there is no "connect" the bridge itself
// initiates — inbound is passive HTTP delivery gated by tunnel + Meta's own
// dashboard registration state). See docs/specs/SPEC_MESSAGING_INTEGRATION_WHATSAPP_2026_07_07.md.
{
    let settings = config_watcher.get_settings();
    if settings.messaging_whatsapp_enabled {
        match (
            settings.messaging_whatsapp_access_token.clone(),
            settings.messaging_whatsapp_app_secret.clone(),
            settings.messaging_whatsapp_webhook_verify_token.clone(),
        ) {
            (Some(token), Some(secret), Some(verify_token))
                if !token.is_empty() && !secret.is_empty() && !verify_token.is_empty() =>
            {
                messaging::whatsapp::WhatsAppBridge::init_global(
                    messaging::whatsapp::WhatsAppConfig {
                        phone_number_id: settings.messaging_whatsapp_phone_number_id.clone(),
                        access_token: token,
                        app_secret: secret,
                        webhook_verify_token: verify_token,
                        target_agent: settings.messaging_whatsapp_target.clone(),
                        fallback_template: settings.messaging_whatsapp_fallback_template.clone(),
                        fallback_template_lang: settings
                            .messaging_whatsapp_fallback_template_lang
                            .clone()
                            .unwrap_or_else(|| "en_US".to_string()),
                        tunnel_domain: settings.messaging_whatsapp_tunnel_domain.clone(),
                        tunnel_name: settings.messaging_whatsapp_tunnel_name.clone(),
                    },
                    reqwest::Client::new(),
                );
            }
            _ => {
                tracing::warn!(
                    "whatsapp bridge: enabled but one of messaging:whatsapp:{{access_token,app_secret,webhook_verify_token}} is not set in settings.json"
                );
            }
        }
    }
}
```

The tunnel-start-and-wait happens inside the spawned task in `init_global` (§5.1), not inline here — `main.rs`'s startup sequence must not block server boot on a tunnel subprocess (which can take several seconds, or fail entirely if `cloudflared` isn't installed). This matches Discord's fire-and-forget `tokio::spawn` pattern; the difference is purely in what the spawned task does before it can call itself "connected."

---

## 8. HTTP endpoints

### 8.1 `POST /api/messaging/whatsapp/send` — authed, same group as Discord's send endpoint

Added to `agentmux-srv/src/server/messaging_handlers.rs` alongside `handle_discord_send`, and to `agentmux-srv/src/server/mod.rs`'s `authed_routes` (same `route_layer(auth_middleware)` group as `/api/messaging/discord/send`, line ~330):

```rust
.route("/api/messaging/whatsapp/send", post(messaging_handlers::handle_whatsapp_send))
```

Request body: `{ "to": "+1...", "text": "..." }` (no embed field — WhatsApp has no rich format, see §5.3). Also extend `handle_status` (line ~24-29 in `messaging_handlers.rs`) to include `WhatsAppBridge::get().map(|b| b.health())` in the `bridges` array, same pattern as Discord.

### 8.2 `GET /webhook/whatsapp` and `POST /webhook/whatsapp` — unauthenticated, NOT in the authed route group

This is the one deliberate departure from "everything goes through the same authed group as `/api/messaging/*`." These routes must be merged at the same level as `health` in `server/mod.rs` (line ~347, `Router::new().merge(health).merge(authed_routes)...`) — outside `route_layer(auth_middleware)` — because Meta cannot supply the `X-AuthKey` header the middleware requires. Recommended registration:

```rust
// server/mod.rs, alongside the `health` router definition (~line 345)
let webhooks = Router::new()
    .route("/webhook/whatsapp", get(messaging::whatsapp::webhook::handle_verify))
    .route("/webhook/whatsapp", post(messaging::whatsapp::webhook::handle_inbound));

Router::new()
    .merge(health)
    .merge(webhooks)
    .merge(authed_routes)
    .layer(cors)
    .with_state(state)
```

**Recommendation confirmed (per the task brief's investigation prompt): keep this on the same axum router/port as the rest of the API, do not stand up a second HTTP server.** The only reason to prefer a second server would be to physically firewall the webhook surface from the authed API surface, but that's already achieved at the tunnel layer (§4.2's ingress path restriction), which is a cleaner boundary than a second in-process listener — it means the public internet only ever reaches `localhost:PORT` through a `cloudflared` process that itself only forwards one path, regardless of how many routes the axum app internally exposes. A second server would duplicate the axum `AppState`, CORS config, and TLS/graceful-shutdown wiring for no corresponding security gain, since the real gate is the tunnel's ingress rule, not which in-process router handled the request.

This is unauthenticated by AgentMux's own auth scheme by necessity, but not unauthenticated in the security sense — see §9, the HMAC check on every POST *is* the authentication for this endpoint, just via a different mechanism (shared-secret HMAC instead of `X-AuthKey`) suited to a third party (Meta) that AgentMux doesn't control the request format of.

---

## 9. Security

1. **Webhook signature validation is non-negotiable — exact check spelled out in §5.2.** `X-Hub-Signature-256: sha256=<hex>` = `HMAC-SHA256(app_secret, raw_body)`. Must be validated against the *raw* body bytes before any JSON parsing (an axum `Bytes` extractor, not `Json<T>` — deserializing first and re-serializing to check the signature would validate a re-encoded payload, not what Meta actually signed, and is also vulnerable to any subtle round-trip formatting difference). Must use a constant-time comparator (`hmac::Mac::verify_slice`, which is constant-time internally, or explicit `subtle::ConstantTimeEq` if hand-rolling the comparison). Reject with `401` before any further processing — no payload deserialization, no logging of body content — on mismatch or missing header.

2. **Credentials never logged.** `access_token`, `app_secret`, `webhook_verify_token` follow the same "treat as secret" doc-comment convention as `messaging_discord_token` in `wconfig/types.rs`. None of these three should ever appear in a `tracing::info!`/`warn!`/`error!` call — the startup log in §7 deliberately logs only which fields are *missing*, never their values.

3. **Allowlist is implicit, not configured.** Unlike Discord (explicit `channel_id` filter) or the master plan's Telegram design (`allowed_chat_ids`), WhatsApp Cloud API inbound is inherently scoped to the one `phone_number_id` you registered — Meta only delivers messages sent *to* your business number, so there is no equivalent "unknown source" surface to allowlist against at the AgentMux layer. No `allowed_senders` config is needed for v1. (If group-messaging or multi-number support is ever added, revisit this.)

4. **Tunnel ingress scoping is a security control, not just tidiness.** See §4.2 — the `cloudflared` ingress rule must restrict the public hostname to the `/webhook/whatsapp` path only. Document this as a required setup step, not an optional hardening tip; failing to do this exposes the entire authed AgentMux API surface (agent control, pane management, memory read/write) to the internet, gated only by the same static `X-AuthKey` used for trusted local/LAN callers.

5. **No secrets in outbound messages.** Same principle as master plan §6.4: agent-generated WhatsApp message text should be scanned for common secret patterns (API key shapes, token prefixes) before send, consistent with whatever mechanism is (or will be) shared across all bridges — this is not WhatsApp-specific and should reuse Discord's approach once Discord has one; flag as a shared follow-up if it doesn't exist yet.

6. **Ban-risk disclosure UI — not applicable to v1.** The master plan's §6.6 requirement ("Baileys path shows a persistent warning... user must acknowledge before enabling") is scoped to Path B only. Since this spec does not implement Path B (§2.1), no disclosure UI is built here. If Path B is revisited later, that requirement is unchanged and must ship with it — a Path-B spec should not skip it just because Cloud API shipped without an equivalent warning (Cloud API's risk profile, per §10, is a different kind of risk — policy/ToS ambiguity, not account-ban — and deserves its own, milder disclosure; see §10).

---

## 10. Known failure modes, rate limits, policy risk

**Currency note (per the task's correction requirement):** the figures below are restated as "per the master plan, as of 2026-06-24" rather than asserted as current fact, because Meta's developer policy pages and Cloud API pricing are exactly the kind of external, frequently-revised source that can silently drift between spec-writing and implementation. **Re-verify against Meta's live developer docs (developers.facebook.com/docs/whatsapp) immediately before implementation**, not at spec-writing time.

- **Pricing, per the master plan as of 2026-06-24:** service messages (user-initiated, within window) and utility templates within window: free. Utility templates outside window: $0.004/message (US). Marketing templates: $0.025/message (US), banned for US recipients since April 2025 per the plan's research. At ~100 conversations/month, the plan estimates $0-$0.40/month in API charges. **Do not hardcode these figures into user-facing copy without re-checking them; if implementation happens more than a few weeks after this spec, treat every number in this paragraph as unverified.**

- **Policy risk, per the master plan as of 2026-06-24:** Meta's "AI chatbot" policy (dated October 2025 in the plan) prohibits using WhatsApp as the delivery channel for a general-purpose AI assistant distributed to others; a developer's private bot serving only their own use is characterized in the plan as "gray area." **This framing should be re-verified, not assumed** — policy language and enforcement posture are exactly the sort of thing that moves between a plan being written and a feature shipping, and getting this wrong risks the user's WhatsApp Business number (and possibly their underlying personal Meta/Facebook account, depending on how account linkage works) being suspended. AgentMux's user-facing copy for this feature (Settings UI description, setup docs) should frame it explicitly as a personal productivity bridge for the user's own conversations, not as a chatbot product, and should link to Meta's current policy page rather than restating a summary that could be stale by the time a user reads it.

- **Rate limits, per the master plan as of 2026-06-24** (re-verify before relying on these for capacity planning): default messaging tier 1,000 unique users/24h; 80 messages/second; per-recipient burst ~1 msg/6s, 45 in 6s then cooldown. On any 429/rate-limit response from the Graph API, the bridge should log and surface the error to the caller (agent's `send` call fails with a clear message) rather than silently dropping — matching the "queue and retry, not drop silently" principle in master plan §6.5, though a v1 implementation may reasonably just fail fast and let the agent/user retry rather than building a retry queue (see §12 open decision points).

- **Webhook delivery drop after repeated failures:** Meta stops retrying (and drops) undelivered webhook events after approximately 7 days of failed delivery (master plan §3.4, citing this as the reason Hookdeck's event queue/replay feature is useful for the ngrok path). Since this spec recommends Cloudflare Tunnel with a stable URL (§4.2), sustained delivery failure should only happen if `agentmux-srv` itself is down for an extended period or the tunnel subprocess dies without being restarted — worth a health-check / alert consideration at implementation time, out of scope to design fully here.

- **Tunnel subprocess failure modes:** `cloudflared` not installed (`ensure_url()` should fail fast with a clear "install cloudflared" error, not a generic timeout); tunnel credentials expired/revoked; DNS route removed from the Cloudflare dashboard outside AgentMux's control. All three should map to `BridgeStatus::Error` with a human-readable `error` string, surfaced in the Warden widget per master plan §2.7.

- **24h window edge case:** a message sent right at the boundary (e.g. 23h59m after last inbound) may succeed as free-form on the bridge's local clock check but be rejected by Meta's server-side clock if there's clock skew or network latency pushes it just past 24h. Treat Meta's rejection (error code 131047) as authoritative — on that specific error code, retry once as a template send if one is configured, rather than surfacing a raw failure for what is a legitimate race condition rather than a config error.

---

## 11. Implementation checklist — phased PR-sized chunks

**PR 1 — Module skeleton + outbound send only (no webhook, no tunnel).**
- `messaging/whatsapp/mod.rs`, `rest.rs`, `types.rs` (send path only)
- Config fields in `wconfig/types.rs` (§6), minus tunnel fields
- `POST /api/messaging/whatsapp/send` handler + route (§8.1)
- Manual test: send a text message to a pre-verified test number via the Cloud API test credentials Meta provides for free during app review setup (no tunnel needed for outbound-only testing — Meta's test number doesn't require a live webhook to receive a send)
- Success criteria: agent can send a WhatsApp message via the Cloud API test number

**PR 2 — `TunnelManager` (Cloudflare provider only) + tunnel lifecycle.**
- `messaging/tunnel.rs`
- Subprocess supervision: start, detect "connected" from stdout, stop on shutdown
- No WhatsApp-specific code in this PR — this is shared infrastructure per master plan §5.1
- Success criteria: `cloudflared tunnel run` subprocess starts, `TunnelManager::status()` reflects `Up { url }` once connected, process is cleanly terminated on `agentmux-srv` shutdown

**PR 3 — Webhook receiver + signature validation + 24h window tracking.**
- `messaging/whatsapp/webhook.rs` (§5.2)
- `GET /webhook/whatsapp` and `POST /webhook/whatsapp` routes registered outside the authed group (§8.2)
- HMAC-SHA256 validation (§9.1)
- Window-state tracking wired into `rest.rs`'s send path (§3.3, §3.4)
- Success criteria: Meta's verification handshake succeeds against a real tunnel URL; a message sent to the WhatsApp Business number appears injected into the target agent's reactive bus; an agent reply within 24h delivers as free-form text and appears in the user's real WhatsApp app

**PR 4 — Startup wiring + tunnel-before-webhook sequencing + Warden status row.**
- `main.rs` wiring (§7)
- `messaging_handlers.rs::handle_status` extended to include WhatsApp health (§8.1)
- Warden widget "Internet" section row for WhatsApp (master plan §2.7) — tunnel status + last-event timestamp
- `widgets.json`'s `defwidget@whatsapp` description updated to drop "(bridge Phase 3)" once this lands
- Success criteria: full restart cycle — `agentmux-srv` starts, tunnel comes up, webhook is live, Warden shows `Connected`, all without manual intervention beyond the one-time Meta Dashboard URL registration

**PR 5 (optional, follow-on) — Template message builder UI + fallback configuration surface.**
- Settings UI for `fallback_template` / `fallback_template_lang`
- Out of scope for this spec's Rust-side design; noted for completeness since §3.4's fail-fast behavior needs a config surface for users to actually set a template

Each PR should get its own changeset per this repo's convention (`task changeset -- patch "feat: whatsapp ..."`), matching the master plan's phase breakdown style.

---

## 12. Open decision points explicitly left for the implementer

1. **Retry/queue behavior on rate-limit or transient failure.** §10 recommends fail-fast for v1; master plan §6.5 recommends "queue and retry, not drop silently" as a cross-platform principle. This spec does not resolve that tension for WhatsApp specifically — pick one at implementation time based on how it's resolved (if at all) for Discord/Telegram, for consistency across bridges.

2. **Credential storage: keychain vs. `settings.json`.** §6 flags that this spec cannot confirm whether Discord's token is actually keychain-backed today or still plaintext, since the type (`Option<String>`) doesn't encode that. Resolve by inspecting the actual current Discord credential-handling code path at implementation time and match it — don't let WhatsApp's credential handling diverge from whatever Discord actually does today, only from what the master plan aspirationally said it should do.

3. **`MessagingBridge` trait async signature.** §5.1 recommends the trait's `send` be `async fn`, diverging from the task brief's suggested sync/mpsc shape (which was modeled on Discord's specific need for a channel, not a general requirement). Needs to be settled jointly with whoever implements Telegram, since both platforms are naturally async-send and neither needs Discord's mpsc pattern.

4. **`TunnelManager`'s "connected" detection.** §4.1 sketches scraping `cloudflared`'s stdout for a connection-established log line, since `cloudflared` has no local status HTTP endpoint by default in the basic invocation. Verify this against the actual `cloudflared` version/flags AgentMux ends up shipping/requiring — some `cloudflared` configurations do expose a local metrics/health endpoint (`--metrics` flag) which would be a more robust signal than log-scraping; evaluate at implementation time.

5. **Path B (Baileys) revisit trigger.** No formal criteria are set for when/if Path B should be reconsidered. Suggest treating this as user-demand-driven (e.g. a threshold number of feature requests for personal-number support without business verification) rather than pre-committing to a timeline, given the Node-subprocess architecture cost identified in §2.1.

6. **Whether `agentmux-srv` should bundle/require `cloudflared` as a managed download** (like it may already do for other external tool dependencies — check the toolchain manager spec for precedent) versus requiring the user to install it themselves via their OS package manager. Not resolved here; affects the onboarding UX for §4.2's setup steps.
