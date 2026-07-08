# Spec: Messaging App Integration — Telegram

**Date:** 2026-07-07
**Status:** Draft — ready to implement
**Scope:** Rust bridge implementation for the Telegram pane (webview already shipped), plus formalization of the `MessagingBridge` trait shared with Discord

---

## 1. Goal / context

The master plan (`docs/specs/SPEC_MESSAGING_INTEGRATIONS_PLAN_2026_06_24.md`) defines the two-layer pane+bridge architecture for embedding messaging apps in AgentMux: a CEF pane shows the real web app, an invisible background bridge lets an agent read/send through the platform's API. That plan's §2–§3 and §5–§6 cover the architecture, protocol research, common abstractions, and security posture in full — this spec does not repeat them, only what's specific to Telegram and to reconciling the plan with what actually shipped.

Discord shipped in PR #1763 (merged 2026-06-24) as the first bridge (`agentmux-srv/src/messaging/discord/`). It validated the pane+bridge split end-to-end but diverged from the plan in three material ways this spec inherits:

1. **No `MessagingBridge` trait exists yet.** `DiscordBridge` is a concrete struct with `init_global`/`get`/`send`/`health`, not a trait impl. §3 below resolves this — Telegram is the platform that formalizes it.
2. **Config is flat serde-renamed keys**, not the nested TOML tables the plan's §2.6 sketches (`[messaging.discord]`). Real keys look like `messaging:discord:enabled`, `messaging:discord:token`. Telegram follows the same convention (§5).
3. **No SDK crates.** Discord hand-rolls the Gateway protocol with `tokio-tungstenite` + `reqwest` + `serde_json` rather than pulling in `twilight-rs`/`serenity`. This spec recommends the same posture for Telegram (§2.5), overriding the plan's `teloxide` recommendation.

The Telegram **pane** already exists: `agentmux-srv/src/config/widgets.json` (`defwidget@telegram`, ~line 151) points at `https://web.telegram.org/` with description `"Telegram Web — real interface, agent-connected (bridge Phase 2)"`. This spec is what makes that description true. Once the bridge lands, drop the `"(bridge Phase 2)"` qualifier (tracked in the checklist, §10).

Why Telegram is next and is the cheapest of the remaining four: no public URL, no webhook, no tunnel, no gateway session/resume state machine — long polling from a single desktop process is the entire receive path, and Telegram's own server enforces single-poller-per-token, which maps naturally onto AgentMux's single-desktop-instance model (see §9).

---

## 2. Protocol design

Full research lives in the master plan §3.2; this section condenses it into exact endpoints/payloads for implementation and calls out where this spec goes further than the plan.

### 2.1 Receiving — long polling

```
GET https://api.telegram.org/bot{TOKEN}/getUpdates
    ?timeout=30
    &offset={last_update_id + 1}
    &allowed_updates=["message","callback_query"]
```

- `timeout=30`: long-poll up to 30s; Telegram returns immediately when an update is available. Use a `reqwest::Client` with a request timeout of at least 35s (30s server-side wait + margin) — do **not** use the client's default short timeout, it will spuriously abort in-flight polls.
- **Offset discipline**: advance the stored offset to `max(update_id) + 1` only *after* every update in the batch has been handed to the injection handler (or otherwise durably processed). If the process crashes mid-batch, the same batch is redelivered on restart — handlers must be idempotent. Idempotency here is achieved the same way Discord achieves it for `MESSAGE_CREATE`: the reactive bus injection call is keyed by `request_id` (Telegram: `update_id` or `message.message_id`), so duplicate injections are a non-issue at the bus layer if it already dedupes, and harmless (a re-injected message) if it doesn't — no additional offset-side de-dup needed.
- Offset is **in-memory only** for v1 (mirrors Discord — no session persistence across restarts either). On restart, omit `offset` on the first call and take the most recent batch; do not attempt to backfill missed messages while offline. This is called out explicitly as an accepted gap, not an oversight (§11).

### 2.2 Sending

```
POST https://api.telegram.org/bot{TOKEN}/sendMessage
{
  "chat_id": 123456789,
  "text": "<b>Agent output</b>",
  "parse_mode": "HTML",
  "reply_markup": { "inline_keyboard": [...] }   // optional
}
```

Use `parse_mode: "HTML"` for all agent-generated output (simpler escaping than MarkdownV2 — only `<`, `>`, `&` require escaping via `&lt;`/`&gt;`/`&amp;`). The `rest.rs` module owns this escaping; never hand raw agent text to `sendMessage` without escaping first, mirroring Discord's `rest::send_message` which JSON-encodes but does not need HTML escaping since Discord uses its own markdown.

### 2.3 Message editing (streaming simulation)

```
POST .../editMessageText
{"chat_id": ..., "message_id": ..., "text": "...", "parse_mode": "HTML"}
```

v1 scope note: `OutboundMsg` (see §4.1) gains an optional `edit_message_id` field so a caller can edit an existing message instead of sending a new one. The poller/sender does not itself implement a streaming-output orchestration loop in v1 — that's an agent-side concern (an agent that wants streaming output sends an initial `OutboundMsg`, receives back the platform message id via the HTTP response, and issues subsequent sends with `edit_message_id` set). See §7 for the response shape change this requires relative to Discord's fire-and-forget `{"ok": true}`.

### 2.4 Inline keyboards and callback queries

```json
{"inline_keyboard": [[
  {"text": "Approve", "callback_data": "approve"},
  {"text": "Cancel", "callback_data": "cancel"}
]]}
```

Inbound `callback_query` updates arrive via the same `getUpdates` poll (included in `allowed_updates`). On receipt:
1. Call `answerCallbackQuery` within **60 seconds** (`POST .../answerCallbackQuery {"callback_query_id": "..."}`) — this clears the client-side button spinner; failing to call it is not fatal but leaves the user's UI in a loading state.
2. Optionally follow with `editMessageReplyMarkup` to update/remove the buttons.
3. Route the callback as an inbound message into the reactive bus the same way a text message is (envelope carries `callback_data` as the message body, prefixed distinctly, e.g. `[Telegram callback @user]: approve`), so the agent can act on it via the same `read_messages` path used for text.

v1 scope: implement the poll-side plumbing for `callback_query` (parse it, answer it, inject it) but do not build a generic "action button" authoring API for agents in this PR — that's follow-on work once there's a concrete agent use case driving the keyboard layout (§11).

### 2.5 Rate limits

| Scope | Limit | Handling |
|---|---|---|
| Single chat | 1 msg/second | Per-chat outbound queue with 1.1s minimum spacing (jittered) |
| Group chat | 20 msg/minute | Per-chat token bucket, refills over 60s |
| Global (all chats) | ~30 msg/second | Global token bucket shared across all outbound sends |
| `429` response | carries `parameters.retry_after` (seconds) and `parameters.scope` (`"chat"` or `"user"`, added Bot API 7.8) | On `chat` scope: pause only that chat's queue for `retry_after` + jitter. On global/unscoped: pause all outbound for `retry_after` + jitter. Never pre-throttle proactively beyond the queue spacing above — react to actual `429`s. |

v1 scope: implement the per-chat 1 msg/s spacing (cheap, prevents the common case) and `429` handling with `retry_after` backoff. Do **not** implement full token-bucket accounting for the 20/min and 30/s tiers in the first PR — a single misbehaving high-volume agent hitting those ceilings is an edge case, not the common path, and Discord's REST client shipped with equivalent minimalism (`rest.rs` logs 429s and returns an error, full header-driven rate limiting deferred). Track this as a fast-follow (§11), not a blocker.

### 2.6 Rust library decision: hand-roll with `reqwest`, do not add `teloxide`

The master plan §3.2 recommends `teloxide` (full Tokio-native Bot API framework). This spec overrides that recommendation. Rationale:

- **Precedent.** Discord shipped by hand-rolling the Gateway protocol with `tokio-tungstenite` + `reqwest` + `serde_json`, explicitly rejecting `twilight-rs`/`serenity` even though the plan recommended them. The codebase now has an established pattern: platform bridges are thin, purpose-built clients over the specific subset of the API AgentMux needs (send message, poll updates, answer callbacks), not full SDK integrations.
- **Telegram's Bot API is simpler than Discord's Gateway.** There's no session/resume/heartbeat state machine to hand-roll — long polling is a plain HTTP GET loop. The amount of code `teloxide` would save here is much smaller than what hand-rolling the Gateway WS protocol would have cost, so the "avoid a heavy dependency" argument that justified skipping `twilight-rs` applies at least as strongly, and the "avoid re-deriving hard protocol logic" argument that might justify pulling in a library is much weaker.
- **Dependency footprint.** `teloxide` pulls in its own async runtime glue, a dispatcher/handler DSL, and a large transitive dependency tree. AgentMux already has `reqwest` (with `rustls-tls`, `json`) in `Cargo.toml` — zero new dependencies needed for a `getUpdates`/`sendMessage` client.
- **Control.** Hand-rolling keeps the bridge's shape (mpsc channel + `Arc<Mutex<BridgeHealth>>` + background tokio task) uniform across platforms, which matters more now that `MessagingBridge` is being formalized (§3) — a `teloxide::Dispatcher`-owned event loop would fight that shape rather than fit it.

**Decision: no new crate.** Implement `messaging/telegram/` with `reqwest` (already a dependency) for both the polling GET and the sending POSTs. No WebSocket library is needed (long polling is plain HTTP).

---

## 3. The trait decision

### 3.1 Problem

`agentmux-srv/src/messaging/mod.rs` currently declares only shared data types (`InboundMsg`, `OutboundMsg`, `MsgEmbed`, `EmbedField`, `BridgeHealth`, `BridgeStatus`) and a `pub mod discord;`. There is no `MessagingBridge` trait, even though the master plan sketched one in §2.5/§5.1. `DiscordBridge` is a concrete struct: `init_global(config, http)`, `get() -> Option<&'static DiscordBridge>`, `send(&self, msg) -> Result<(), String>`, `health(&self) -> BridgeHealth`.

With only one implementation this was the right call — premature abstraction over a single concrete type buys nothing. Telegram is the second real implementation, which is exactly the point at which the abstraction earns its keep: `handle_status` (`agentmux-srv/src/server/messaging_handlers.rs`) currently special-cases Discord (`if let Some(bridge) = DiscordBridge::get() { bridges.push(bridge.health()); }`); with two platforms this either grows a second hand-copied `if let` (fine at 2, ugly at 4-5 once Slack/WhatsApp/Teams land) or the aggregation becomes `for platform in registered_bridges() { ... }` over a trait object.

**Decision: formalize `MessagingBridge` now.** Telegram implements it from day one; `DiscordBridge` is retrofitted to implement it in the same PR (or the PR immediately before Telegram's protocol work — see Phase in §10). This spec makes the recommendation and specifies the mechanical retrofit; per the coordinating plan, Telegram is the platform that executes it.

### 3.2 Trait definition — sync `send`/`health`, not the plan's `async`

The master plan's sketch (§2.5) used `async fn send` and `async fn stop`. This spec deliberately diverges:

```rust
// agentmux-srv/src/messaging/mod.rs

use std::sync::Arc;

/// Common interface implemented by every platform bridge (Discord, Telegram, …).
///
/// Bridges are singletons accessed via `get()`-style static accessors on the
/// concrete type (see `DiscordBridge::get()`, `TelegramBridge::get()`) for the
/// call sites that know their platform statically (e.g. platform-specific HTTP
/// handlers). This trait exists for call sites that need to treat bridges
/// polymorphically — today, only `handle_status`'s aggregation loop.
pub trait MessagingBridge: Send + Sync {
    /// Static platform identifier: "discord", "telegram", etc.
    fn platform(&self) -> &'static str;

    /// Enqueue a message for delivery. Fire-and-forget: pushes onto the
    /// bridge's internal mpsc channel and returns as soon as the send is
    /// queued, not once it's delivered. Matches the existing Discord
    /// contract exactly — see rationale below.
    fn send(&self, msg: OutboundMsg) -> Result<(), String>;

    /// Current connection/health snapshot. Cheap, non-blocking (reads an
    /// `Arc<Mutex<BridgeHealth>>` guarded by a short-lived lock).
    fn health(&self) -> BridgeHealth;
}
```

Note what's *not* in this trait: `start()`/`stop()`/`platform()`-as-constructor. Rationale:

- **`send`/`health` stay sync.** Both existing methods on `DiscordBridge` are already sync and cannot block: `send` pushes onto an `mpsc::UnboundedSender` (non-blocking, returns `Result` immediately based on whether the receiver is still alive), and `health` takes a `Mutex` lock held only long enough to `clone()` a small struct. Making them `async fn` would require every call site (`messaging_handlers.rs` handlers, which are themselves async axum handlers, so this is a low-cost change there) to `.await` a call that never actually awaits anything — pure ceremony. More importantly, retrofitting `DiscordBridge` under an `async fn send` signature would still resolve to the same non-blocking channel push, so the `async` keyword would be lying about the method's actual behavior. Sync signatures document the fire-and-forget contract accurately: the *bridge's* background task is what does the awaiting (the gateway/poller loop `await`s the HTTP send), not the caller.
- **No `start()`/`stop()` in the trait.** Both bridges follow an `init_global(config, ...)` pattern instead: initialize a process-wide singleton once at startup, spawn its background task, and never tear it down for the lifetime of the process (there is no runtime "disable this bridge" flow yet — disabling requires restarting AgentMux with `messaging:discord:enabled = false` / `messaging:telegram:enabled = false`, per the existing `main.rs` wiring). Because `init_global` differs per platform (different `Config` struct shape) it cannot be a trait method with a uniform signature without an ugly `Box<dyn Any>` config parameter. Keep `init_global`/`get()` as inherent (non-trait) associated functions on each concrete bridge type, exactly as `DiscordBridge` already does. If a real stop/reconfigure flow is needed later (tracked as an open question in §11), add `fn stop(&self)` then, once there's a concrete caller and a concrete idea of what "stop" should do to the background task (cooperative cancellation via a `CancellationToken` most likely) — do not speculate on that shape now.
- **`platform()` is included** because it's the one piece `handle_status`'s aggregation actually needs today (to key the JSON status array by platform name) that isn't already derivable from `health().platform` — actually, `BridgeHealth.platform` already carries this, so `platform()` on the trait is redundant with `health().platform`. **Decision: omit `platform()` from the trait entirely**, get it via `health().platform` instead, keeping the trait to exactly the two methods (`send`, `health`) that have real polymorphic call sites today. (Revise the trait sketch above accordingly — final trait is `send` + `health` only.)

Final trait:

```rust
pub trait MessagingBridge: Send + Sync {
    fn send(&self, msg: OutboundMsg) -> Result<(), String>;
    fn health(&self) -> BridgeHealth;
}
```

### 3.3 Discord retrofit — mechanical, no behavior change

In `agentmux-srv/src/messaging/discord/mod.rs`, add:

```rust
impl crate::messaging::MessagingBridge for DiscordBridge {
    fn send(&self, msg: OutboundMsg) -> Result<(), String> {
        DiscordBridge::send(self, msg)
    }
    fn health(&self) -> BridgeHealth {
        DiscordBridge::health(self)
    }
}
```

(The inherent `send`/`health` methods are kept as-is and remain the primary call path for `messaging_handlers::handle_discord_send`, which knows it's talking to Discord specifically and has no reason to go through a trait object. The trait impl exists purely so `&dyn MessagingBridge` can be constructed where polymorphism is needed.) This is a pure additive change — zero risk to existing behavior, no changes to `gateway.rs`, `rest.rs`, `types.rs`, or the inherent method bodies.

### 3.4 Where the trait is actually used

`handle_status` (`agentmux-srv/src/server/messaging_handlers.rs`) becomes:

```rust
pub(super) async fn handle_status(State(_state): State<AppState>) -> impl IntoResponse {
    let mut bridges: Vec<BridgeHealth> = vec![];
    if let Some(b) = DiscordBridge::get() {
        bridges.push((b as &dyn MessagingBridge).health());
    }
    if let Some(b) = TelegramBridge::get() {
        bridges.push((b as &dyn MessagingBridge).health());
    }
    Json(json!({ "bridges": bridges }))
}
```

This is honest about the current state: even with the trait formalized, there is still one `if let` per platform, because each bridge's `get()` is a distinct static accessor on a distinct type (`Option<&'static DiscordBridge>` vs `Option<&'static TelegramBridge>`) — Rust has no ambient registry of "all initialized bridges" without additional machinery (e.g. a `Vec<&'static dyn MessagingBridge>` populated at `init_global` time via a `OnceLock<Mutex<Vec<...>>>` registry). **Recommendation: do not build that registry yet.** At 2 platforms the `if let` chain is more readable than a registry abstraction; revisit when a 3rd platform (Slack) lands and the chain would grow to 3 — that's the point where a shared `register_bridge(&'static dyn MessagingBridge)` call inside each `init_global` starts paying for itself. Note this explicitly as a deferred decision (§11), not an oversight.

---

## 4. Rust module layout

Mirrors `agentmux-srv/src/messaging/discord/` exactly, with `gateway.rs` renamed to `poller.rs` since Telegram has no persistent WebSocket session:

```
agentmux-srv/src/messaging/telegram/
├── mod.rs      // Config, TelegramBridge struct, GLOBAL_BRIDGE OnceLock, init_global/get/send/health
├── poller.rs   // long-polling loop: getUpdates → dispatch → offset advance; outbound mpsc → REST send
├── rest.rs     // sendMessage / editMessageText / answerCallbackQuery / editMessageReplyMarkup
└── types.rs    // Update, Message, CallbackQuery, Chat, User wire types; outbound body structs
```

`agentmux-srv/src/messaging/mod.rs` gains `pub mod telegram;` alongside the existing `pub mod discord;`, plus the `MessagingBridge` trait from §3.2.

### 4.1 `mod.rs`

```rust
mod poller;
pub mod rest;
pub mod types;

use std::sync::{Arc, Mutex, OnceLock};
use tokio::sync::mpsc;
use crate::messaging::{BridgeHealth, OutboundMsg};

#[derive(Debug, Clone)]
pub struct TelegramConfig {
    /// Bot token from @BotFather.
    pub token: String,
    /// Allowlisted chat IDs. Inbound updates from chats not in this list are
    /// silently dropped (no reply, no injection, no log-level above debug).
    pub allowed_chat_ids: Vec<i64>,
    /// Default chat ID for outbound sends when OutboundMsg doesn't override one.
    /// Reuses `OutboundMsg.channel_id` as a stringified chat_id (see §7 note).
    pub default_chat_id: Option<i64>,
    /// Agent ID to inject inbound Telegram messages into via the reactive bus.
    pub target_agent: Option<String>,
}

pub struct TelegramBridge {
    outbound_tx: mpsc::UnboundedSender<OutboundMsg>,
    health: Arc<Mutex<BridgeHealth>>,
}

static GLOBAL_BRIDGE: OnceLock<TelegramBridge> = OnceLock::new();

impl TelegramBridge {
    pub fn init_global(config: TelegramConfig, http: reqwest::Client) {
        let (outbound_tx, outbound_rx) = mpsc::unbounded_channel::<OutboundMsg>();
        let health = Arc::new(Mutex::new(BridgeHealth::connecting("telegram")));
        let bridge = TelegramBridge { outbound_tx, health: health.clone() };
        if GLOBAL_BRIDGE.set(bridge).is_err() {
            return; // already initialized
        }
        tokio::spawn(async move {
            poller::run_poll_loop(config, http, outbound_rx, health).await;
        });
    }

    pub fn get() -> Option<&'static TelegramBridge> { GLOBAL_BRIDGE.get() }

    pub fn send(&self, msg: OutboundMsg) -> Result<(), String> {
        self.outbound_tx.send(msg)
            .map_err(|_| "telegram_bridge: outbound channel closed".to_string())
    }

    pub fn health(&self) -> BridgeHealth {
        self.health.lock().unwrap().clone()
    }
}

impl crate::messaging::MessagingBridge for TelegramBridge {
    fn send(&self, msg: OutboundMsg) -> Result<(), String> { TelegramBridge::send(self, msg) }
    fn health(&self) -> BridgeHealth { TelegramBridge::health(self) }
}
```

Note on reusing `OutboundMsg.channel_id`: Telegram addresses by numeric `chat_id`, not a channel string, but `OutboundMsg` is a cross-platform shared type (`agentmux-srv/src/messaging/mod.rs`) and should not sprout a Telegram-only field. `channel_id: String` is reused to carry the chat id as a decimal string (e.g. `"123456789"`); `poller.rs`/`rest.rs` parse it with `str::parse::<i64>()` and fall back to `config.default_chat_id` when empty, exactly mirroring how Discord treats an empty `channel_id` as "use the bridge's default channel." This keeps `OutboundMsg` platform-agnostic rather than special-casing it per platform — the alternative (a `platform_target: PlatformTarget` enum) is not justified by two platforms and is left as a §11 open question if a third chat-id-shaped platform (e.g. Slack's `channel`) shows the same need.

### 4.2 `poller.rs` — shape mirrors `gateway.rs`'s `run_gateway_loop`/`run_session` split

```rust
pub async fn run_poll_loop(
    config: TelegramConfig,
    http: reqwest::Client,
    mut outbound_rx: mpsc::UnboundedReceiver<OutboundMsg>,
    health: Arc<Mutex<BridgeHealth>>,
) {
    let mut offset: i64 = 0;
    loop {
        tokio::select! {
            // Outbound: agent → Telegram REST (checked opportunistically between polls)
            Some(msg) = outbound_rx.recv() => {
                if let Err(e) = rest::send_or_edit(&http, &config.token, &config, &msg).await {
                    tracing::warn!("telegram_bridge: send failed: {e}");
                }
            }
            // Inbound: long poll
            result = rest::get_updates(&http, &config.token, offset) => {
                match result {
                    Ok(updates) => {
                        { let mut h = health.lock().unwrap();
                          h.status = BridgeStatus::Connected; h.error = None; }
                        for update in &updates {
                            handle_update(update, &config, &health);
                        }
                        if let Some(last) = updates.last() {
                            offset = last.update_id + 1;
                        }
                    }
                    Err(e) => {
                        tracing::warn!("telegram_bridge: getUpdates failed: {e}");
                        let mut h = health.lock().unwrap();
                        h.status = BridgeStatus::Error;
                        h.error = Some(e);
                        h.reconnect_count += 1;
                        drop(h);
                        tokio::time::sleep(Duration::from_secs(5)).await; // backoff before retry
                    }
                }
            }
        }
    }
}
```

This is a plain retry-forever loop (no session/resume state — there is no session), matching Discord's `run_gateway_loop` outer retry shell but without the inner `run_session` complexity, since long polling has no connection to lose beyond a single HTTP request. `handle_update` plays the same role as `gateway.rs`'s `handle_dispatch`: filters by `allowed_chat_ids`, ignores bot-authored messages, builds the `[Telegram @username]: text` envelope, and calls `get_global_handler().inject_message(InjectionRequest { ... source_agent: Some("telegram".into()), request_id: Some(update.update_id.to_string()), delivery_tier: Some("wan".into()), ... })` — same call shape Discord uses in `gateway.rs` line ~414-425.

### 4.3 `rest.rs`

Functions: `get_updates(http, token, offset) -> Result<Vec<Update>, String>`, `send_message`, `edit_message_text`, `answer_callback_query`, `edit_message_reply_markup` — each a thin `reqwest` POST/GET against `https://api.telegram.org/bot{token}/{method}`, parsing Telegram's `{"ok": bool, "result": ..., "error_code": ..., "description": ..., "parameters": {"retry_after": ..., "scope": ...}}` envelope. `send_or_edit` dispatches to `send_message` or `edit_message_text` based on whether `OutboundMsg` carries an `edit_message_id` (per §2.3).

### 4.4 `types.rs`

Wire types: `Update { update_id, message: Option<Message>, callback_query: Option<CallbackQuery> }`, `Message { message_id, chat: Chat, from: Option<User>, text: Option<String>, date: i64 }`, `Chat { id, type_: String }`, `User { id, username: Option<String>, is_bot: bool }`, `CallbackQuery { id, from: User, data: Option<String>, message: Option<Message> }`, plus outbound body structs (`SendMessageBody`, `EditMessageTextBody`, `AnswerCallbackQueryBody`, `InlineKeyboardMarkup`, `InlineKeyboardButton`) — same hand-rolled `serde` struct pattern as `discord/types.rs`.

---

## 5. Config schema additions

`agentmux-srv/src/backend/wconfig/types.rs`, adjacent to the existing `messaging:discord:*` block (~line 300-325), following the same flat-key convention:

```rust
// -- Messaging bridge settings (Telegram) --

/// Master enable for the Telegram messaging bridge.
/// When true, the bridge starts long-polling getUpdates at startup.
#[serde(rename = "messaging:telegram:enabled", default, skip_serializing_if = "is_false")]
pub messaging_telegram_enabled: bool,

/// Telegram bot token from @BotFather. Treat as a secret — do not log.
#[serde(rename = "messaging:telegram:token", default, skip_serializing_if = "Option::is_none")]
pub messaging_telegram_token: Option<String>,

/// Comma-separated allowlist of chat IDs permitted to reach the bridge.
/// Inbound updates from any other chat are silently dropped.
/// Stored as a string (not Vec<i64>) to keep the flat-key/settings.json
/// convention simple — parsed to Vec<i64> at startup wiring time.
#[serde(rename = "messaging:telegram:allowed_chats", default, skip_serializing_if = "String::is_empty")]
pub messaging_telegram_allowed_chats: String,

/// Default chat ID for outbound sends when a request doesn't override one.
#[serde(rename = "messaging:telegram:default_chat", default, skip_serializing_if = "Option::is_none")]
pub messaging_telegram_default_chat: Option<String>,

/// Agent ID that receives inbound Telegram messages via the reactive bus.
/// Absent → messages are logged but not forwarded to any agent.
#[serde(rename = "messaging:telegram:target", default, skip_serializing_if = "Option::is_none")]
pub messaging_telegram_target: Option<String>,
```

Note on `allowed_chats` as a comma-separated string rather than `Vec<i64>`: `messaging_discord_*` fields are all scalar (`bool`/`Option<String>`/`String`) — there's no existing precedent in this struct for a `Vec<T>`-valued settings key, and introducing one raises questions (serde-rename on a list field, how it round-trips through whatever settings-editing UI exists) that are out of scope here. A comma-separated string parsed at the `main.rs` call site (`"123,456,789".split(',').filter_map(|s| s.trim().parse().ok()).collect()`) is the minimal-diff choice consistent with every other field in this struct. If a future platform needs a genuine list-valued setting, standardize the approach there rather than one-offing it for Telegram.

---

## 6. Startup wiring

`agentmux-srv/src/main.rs`, immediately after the existing Discord block (~line 731-754), same shape:

```rust
// Telegram messaging bridge — long-polls getUpdates if configured.
// Set messaging:telegram:enabled + messaging:telegram:token in settings.json to activate.
{
    let settings = config_watcher.get_settings();
    if settings.messaging_telegram_enabled {
        match settings.messaging_telegram_token.clone() {
            Some(token) if !token.is_empty() => {
                let allowed_chat_ids = settings
                    .messaging_telegram_allowed_chats
                    .split(',')
                    .filter_map(|s| s.trim().parse::<i64>().ok())
                    .collect::<Vec<_>>();
                let default_chat_id = settings
                    .messaging_telegram_default_chat
                    .as_deref()
                    .and_then(|s| s.parse::<i64>().ok());
                messaging::telegram::TelegramBridge::init_global(
                    messaging::telegram::TelegramConfig {
                        token,
                        allowed_chat_ids,
                        default_chat_id,
                        target_agent: settings.messaging_telegram_target.clone(),
                    },
                    reqwest::Client::new(),
                );
            }
            _ => {
                tracing::warn!(
                    "telegram bridge: enabled but messaging:telegram:token is not set in settings.json"
                );
            }
        }
    }
}
```

Placed directly after the Discord block so both bridges initialize together and the startup sequence stays easy to scan; no shared "init all messaging bridges" loop is introduced in this PR (that refactor belongs with the bridge-registry idea noted in §3.4/§11, deferred until a 3rd platform makes the duplication actually costly).

---

## 7. HTTP endpoints

`agentmux-srv/src/messaging/mod.rs` (or wherever `MessagingBridge` lands — §3.2) exports the trait; `agentmux-srv/src/server/messaging_handlers.rs` gains:

```rust
use crate::messaging::telegram::TelegramBridge;
use crate::messaging::MessagingBridge;
```

### 7.1 `handle_status` — updated per §3.4

Aggregates both bridges' `health()` into the existing `{"bridges": [...]}` array shape — no response-shape change, just one more entry once Telegram is configured.

### 7.2 `POST /api/messaging/telegram/send`

```rust
/// POST /api/messaging/telegram/send
#[derive(Deserialize)]
pub(super) struct TelegramSendRequest {
    #[serde(default)]
    pub text: String,
    /// Override chat. Empty → use the bridge's default chat (messaging:telegram:default_chat).
    #[serde(default)]
    pub chat_id: String,
    /// If set, edits this existing message instead of sending a new one
    /// (see spec §2.3 — streaming-output simulation).
    pub edit_message_id: Option<i64>,
    /// Optional inline keyboard, one row per Vec entry.
    #[serde(default)]
    pub inline_keyboard: Vec<Vec<TelegramButtonRequest>>,
}

#[derive(Deserialize)]
pub(super) struct TelegramButtonRequest {
    pub text: String,
    pub callback_data: String,
}
```

Handler mirrors `handle_discord_send`'s shape (look up the bridge via `TelegramBridge::get()`, 503 if uninitialized, build `OutboundMsg`, call `bridge.send(msg)`, return `{"ok": true}` on success / 500 with `{"error": ...}` on channel-closed).

**One deliberate deviation from the Discord endpoint's response shape:** because Telegram's edit-for-streaming pattern (§2.3) requires the caller to learn the `message_id` of a message it just sent in order to edit it later, and because `send()` on `MessagingBridge`/`TelegramBridge` is fire-and-forget over an mpsc channel (no synchronous round-trip to Telegram's API, so the handler cannot know the resulting `message_id` at return time), **v1 does not thread `message_id` back through the HTTP response.** A caller wanting to edit a message must independently track `chat_id` + a caller-assigned correlation id, or (simpler, and the recommended path) avoid editing entirely and rely on Telegram's native message grouping for streaming-style updates. Threading a synchronous send-and-return-message-id path through would require either making `send()` async-with-response (breaking the sync contract established in §3.2 and diverging from Discord) or adding a second, separate "synchronous send" API distinct from the fire-and-forget one. Neither is justified by a concrete caller yet — left as an explicit open question in §11, not silently dropped.

`agentmux-srv/src/server/mod.rs` gains one route registration alongside the existing two:

```rust
.route("/api/messaging/telegram/send", post(messaging_handlers::handle_telegram_send))
```

---

## 8. Security considerations specific to Telegram

Builds on the master plan §6 (credentials in keychain — note: today Discord's token is read from plaintext `settings.json`, not OS keychain, despite the plan's §6.1 requirement; this is an existing gap this spec inherits rather than fixes, call it out explicitly rather than silently perpetuating it — see §11):

1. **Allowlist is mandatory, not optional, for Telegram specifically.** Unlike Discord (where a bot is scoped to guilds it's explicitly invited to, so "reachability" is already gated by the invite step), a Telegram bot's username is public and anyone who knows it can start a DM or add it to a group — there is no invite-approval step equivalent to Discord's OAuth flow. `allowed_chat_ids` (§5) is therefore the *only* gate, and `poller.rs`'s `handle_update` must check it before any other processing, silently dropping (not replying to) messages from unlisted chats — replying at all would confirm the bot is alive and invite enumeration, per the master plan §3.2's existing guidance.
2. **`callback_data` is attacker-controllable input** the moment the allowlist is bypassed by a listed-but-compromised chat, or even within an allowed chat by any member of a group chat the bot is in. Treat `callback_data` string content the same as inbound message text: untrusted, never `eval`'d or used to construct file paths/shell commands, only matched against a fixed set of expected action strings the bridge itself generated.
3. **Bot privacy mode.** By default (`/setprivacy` on @BotFather, default state is `enabled`) a bot added to a group only receives messages that mention it or reply to it — not the full group stream. This is a *security-positive* default and this spec does not recommend disabling it (`/setprivacy off`) unless a specific AgentMux use case needs full-group visibility; document this as a user-facing setup note (§10) rather than defaulting to the more permissive mode.
4. **Token in logs.** `rest.rs` must never log the full request URL if the token is embedded in the path (`https://api.telegram.org/bot{TOKEN}/...`) — this is a sharper footgun than Discord's `Authorization: Bot {token}` header, since header values are less likely to be accidentally logged by generic HTTP tracing than URL paths are. Redact the token segment in any `tracing::` call that includes the URL (e.g. log `"telegram_bridge: getUpdates request"` without the URL, or with the token segment replaced by `***`).

---

## 9. Known failure modes / rate limits

(Rate limit table already given in §2.5; this section covers non-rate-limit failure modes, mirroring the "Known failure modes" subsections in both source docs.)

- **409 Conflict on `getUpdates`.** Returned when a second `getUpdates` long-poll is already open for the same token (e.g. AgentMux running twice, or a stray webhook still registered on the same bot). Per the master plan, this is a *feature* for AgentMux's single-desktop-instance model — it prevents split-brain message handling — but the bridge must surface it clearly: on `409`, set `BridgeHealth.status = Error` with `error = Some("telegram: another getUpdates poller is active for this token (409) — check for a duplicate AgentMux instance or a registered webhook")`, back off (same 5s→60s exponential pattern as Discord's reconnect delay), and keep retrying rather than giving up permanently (the conflicting instance may exit).
- **Webhook/polling conflict.** If a webhook was ever registered for this bot token (e.g. by a prior non-AgentMux integration), `getUpdates` fails until `deleteWebhook` is called. v1 does not proactively call `deleteWebhook` at startup — document this as a manual troubleshooting step (§10 setup notes) rather than silently mutating the bot's webhook config on every startup, since AgentMux cannot know whether the user intentionally has a webhook configured elsewhere.
- **Long-poll timeout vs. HTTP client timeout mismatch.** Covered in §2.1 — the `reqwest::Client` used for `get_updates` must have a per-request timeout longer than Telegram's own `timeout=30` parameter, or every poll will spuriously error out right as data would have arrived. Use a dedicated client (or `.timeout()` override on the request builder) distinct from whatever default client `rest.rs`'s send methods use, since sends should time out much faster (a stuck `sendMessage` call blocking the poll loop for 30s+ is worse than a 30s-timeout getUpdates call, given the `tokio::select!` structure in §4.2 processes outbound sends and inbound polls on the same task).
- **Malformed/oversized updates.** As with Discord's `MESSAGE_CREATE` parse failures (`gateway.rs` line ~383-389), a single malformed `Update` in a batch must not abort the whole batch — parse each `Update` independently inside the loop in `poller.rs`, `tracing::warn!` and `continue` past parse failures for that one item, and — critically — still advance the offset past it (an update that will never parse successfully should not permanently wedge the poll loop by being retried forever).

---

## 10. Implementation checklist — phased, PR-sized chunks

Mirrors the Discord POC's phased style, made concrete against this codebase's actual shape.

**PR 1 — Formalize `MessagingBridge` trait + Discord retrofit (small, low-risk, unblocks PR 2-4)**
- Add `MessagingBridge` trait to `agentmux-srv/src/messaging/mod.rs` (§3.2 final form: `send` + `health` only).
- Add `impl MessagingBridge for DiscordBridge` to `discord/mod.rs` (§3.3) — additive, no behavior change.
- Update `handle_status` in `messaging_handlers.rs` to go through `&dyn MessagingBridge` for the health aggregation (§3.4), still one `if let` per platform.
- No new endpoints, no config changes. Verify: existing Discord bridge tests/manual smoke test still pass unmodified.

**PR 2 — Telegram module skeleton + long-poll receive path**
- `agentmux-srv/src/messaging/telegram/{mod,poller,rest,types}.rs` per §4.
- `mod.rs`: `TelegramConfig`, `TelegramBridge`, `init_global`/`get`/`send`/`health`, `impl MessagingBridge`.
- `poller.rs`: `run_poll_loop` — `getUpdates` long polling, offset advance, allowlist filtering, envelope construction, reactive-bus injection (mirrors `gateway.rs::handle_dispatch`'s `MESSAGE_CREATE` handling, §9's malformed-update handling included).
- `rest.rs`: `get_updates` only in this PR (send path is PR 3).
- `types.rs`: `Update`, `Message`, `Chat`, `User` (inbound-only types for this PR; `CallbackQuery` can land here or in PR 4).
- Config: add the 5 `messaging:telegram:*` fields to `wconfig/types.rs` (§5).
- Startup wiring in `main.rs` (§6).
- Success criterion: a message sent to the configured bot in an allowlisted chat is injected into the reactive bus and visible to the target agent via `read_messages`. No outbound path yet (verify via logs/reactive-bus inspection, not a reply).

**PR 3 — Outbound send + HTTP endpoint**
- `rest.rs`: `send_message`, `edit_message_text`, HTML-escaping helper (§2.2).
- `poller.rs`: wire the `outbound_rx` arm of the `tokio::select!` to call `rest::send_or_edit` (§4.2).
- `messaging_handlers.rs`: `TelegramSendRequest`/`handle_telegram_send` (§7.2), route registration in `server/mod.rs`.
- Per-chat 1 msg/s spacing + `429`/`retry_after` handling (§2.5 — v1 scope, not full token-bucket accounting).
- Success criterion: `POST /api/messaging/telegram/send` delivers a message visible in the Telegram pane's live chat; round-trip (user message in → agent reply out) works end-to-end, matching Discord's Phase 2 success criterion.

**PR 4 — Inline keyboards + callback queries**
- `types.rs`: `CallbackQuery`, `InlineKeyboardMarkup`/`InlineKeyboardButton` outbound types.
- `rest.rs`: `answer_callback_query`, `edit_message_reply_markup`.
- `poller.rs`: handle `callback_query` updates — allowlist check, `answerCallbackQuery` within 60s, inject as a distinct envelope into the reactive bus (§2.4).
- `messaging_handlers.rs`: extend `TelegramSendRequest` with `inline_keyboard` (§7.2) if not already added in PR 3.
- Success criterion: an outbound message with an inline keyboard is tappable in the real Telegram client; the tap is delivered to the target agent and the button spinner clears.

**PR 5 — Pane description + polish**
- `agentmux-srv/src/config/widgets.json`: update `defwidget@telegram`'s `description` to drop the `"(bridge Phase 2)"` qualifier, e.g. `"Telegram Web — real interface, agent-connected"` (matches the already-bridge-complete Discord entry's description string, which has no phase qualifier).
- Any settings-UI surface for entering the bot token / allowed chat IDs, if such a UI exists for Discord's equivalent fields (out of scope to design here if it doesn't yet exist for Discord either — check parity at implementation time rather than building a Telegram-only settings UI ahead of Discord getting one).

---

## 11. Open decision points — explicitly not resolved here

1. **Credentials in plaintext `settings.json` vs. OS keychain.** The master plan §6.1 mandates keychain storage; both Discord (shipped) and this Telegram spec store the token as a plaintext settings field (`messaging_discord_token` / `messaging_telegram_token`). This is a pre-existing gap this spec inherits rather than fixes. If/when Discord's token storage is migrated to keychain, Telegram's should follow the same migration in lockstep — do not fix it for one platform and not the other.
2. **Bridge registry for `handle_status`.** Deferred per §3.4 until a 3rd platform (Slack) makes the per-platform `if let` chain in `handle_status` unwieldy enough to justify a `register_bridge()`-based registry.
3. **`OutboundMsg` growing a platform-target enum** instead of overloading `channel_id` as a stringified chat id (§4.1). Revisit if a 3rd platform's addressing scheme doesn't fit the "stringified single identifier" shape either.
4. **Synchronous send-and-get-message-id API** for the streaming-edit pattern (§2.3, §7.2). Not built in v1 because there's no concrete caller yet; if an agent workflow needs it, design it as an additive second entry point rather than changing `send()`'s existing fire-and-forget contract.
5. **Full token-bucket rate limiting** for the 20/min-per-group and ~30/s-global tiers (§2.5). v1 only implements per-chat 1 msg/s spacing and reactive `429` backoff.
6. **Offline backfill.** No attempt to fetch messages missed while AgentMux was not running (offset resets to "most recent" on restart, §2.1). If a use case emerges for catching up on missed messages, that's a deliberate scope addition, not a bug fix.
7. **Proactive `deleteWebhook` call at startup** to avoid the webhook/polling conflict noted in §9. Left as a manual troubleshooting step rather than an automatic mutation of the bot's configuration.
8. **Settings UI** for token/allowlist entry — parity with whatever exists (or doesn't yet) for Discord, not designed fresh here (§10, PR 5).
