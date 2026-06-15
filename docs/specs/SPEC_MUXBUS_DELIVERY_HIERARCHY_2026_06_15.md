# MuxBus Delivery Hierarchy

**Date:** 2026-06-15  
**Status:** Draft  
**Authors:** naki

---

## 1. Problem Statement

AgentMux agents need to send messages to each other across three scopes:

1. **Same host** — agent in pane A → agent in pane B, same machine
2. **Same LAN** — agent on laptop → agent on desktop, same network
3. **WAN** — agent on one machine → agent on another over the internet

These three scopes have radically different latency, cost, and trust profiles. Additionally:

- **Cloud → local delivery**: reagent notifications from the cloud (e.g., a PR review event) must be able to push a message _down_ into a locally running agent pane — the agent doesn't poll, the sidecar delivers it.
- **Promote on context**: when the intended recipient isn't reachable at a lower tier, the message should automatically escalate — not fail.

The current system has all the mechanical pieces but they are not unified into a single delivery stack.

---

## 2. Existing Infrastructure Inventory

### Tier 1 — Same sidecar instance (already working)

- `ReactiveHandler` (`backend/reactive/handler.rs`) holds `agent_id → block_id` mappings in memory.
- Injection: write `\r message\r` + 3 delayed `\r` directly to the PTY block.
- Rate limit: 10 req/sec per target, token bucket.
- Latency: <1ms.

### Tier 2 — Same host, different sidecar instances (already working)

- `AgentEntry` registry: `{data_dir}/agents/{agent_id}.json` — written on register, cleaned up on unregister.
- Each entry contains: `local_url` (http://127.0.0.1:PORT), `auth_key` (per-launch UUID), `block_id`, `pid`.
- Cross-instance forward: HTTP POST to peer's `/agentmux/reactive/inject` with `X-AuthKey`.
- Stale entries (>4h) cleaned at startup.
- Latency: 1–5ms loopback.

### Tier 3 — LAN peers (spec exists, not built)

- `specs/lan-awareness-and-embedded-jekt-api.md` proposes mDNS (`_agentmux._tcp.local`).
- `backend/lan_discovery/` exists for host presence (peer list for UI), but doesn't carry agent registry.
- **Gap**: no way to find which LAN host has agent B running.

### Tier 4 — Cloud / WAN (partially working)

- `muxbus.agentmux.ai` — Fastify on Lambda, DynamoDB.
- REST API: `POST /reactive/inject` stores injection, `GET /reactive/pending/{id}` polls for delivery.
- **Current model is pull-only**: agent must poll to find pending injections — no push path to locally running panes.
- Auth: Cognito PKCE (MUXBUS_TOKEN) for desktop users; `client_credentials` for M2M agents.
- Latency: 50–300ms.

### Client (muxbus-client npm package)

Already implements **local-first** in `injectTerminal()`:
1. Try `AGENTMUX_LOCAL_URL/wave/reactive/inject` (5s timeout).
2. If unavailable or "not found" → fall through to cloud REST.
3. Cloud: stores injection, polls `/reactive/status/{id}` every 1s for up to 15s.

**Core gap**: polling (step 3) means cloud→local delivery has 0–5s latency and requires the agent to be running the muxbus client with a polling loop.

---

## 3. Model: NATS Leaf Node (adapted)

The right mental model is [NATS Leaf Nodes](https://docs.nats.io/running-a-nats-service/configuration/leafnodes):

- Each **AgentMux sidecar** is a **leaf node** — it knows about the agents running on its host.
- **MuxBus cloud** is the **core cluster** — it routes messages between leaf nodes it can't deliver locally.
- **Subscriptions propagate upward**: when a sidecar starts, it tells the cloud "I have agents X, Y, Z — push anything addressed to them here".
- **Messages flow downward**: cloud holds a message for agent X, pushes it to the leaf node (sidecar) that registered X, sidecar delivers locally.

This eliminates polling for cloud→local delivery and makes cloud purely a relay-of-last-resort for WAN.

---

## 4. Delivery Tiers (Unified Model)

```
Sender (agent A)
  │
  ▼
[muxbus-client / MuxBus SDK]
  │
  ├── Tier 1: Same-sidecar in-memory (ReactiveHandler)
  │     Condition: recipient registered in local handler
  │     Transport: direct PTY write
  │     Latency: <1ms
  │     Auth: none (in-process)
  │
  ├── Tier 2: Same-host, peer sidecar (file registry lookup → HTTP)
  │     Condition: {data_dir}/agents/{agent_id}.json exists, local_url ≠ sender
  │     Transport: HTTP POST to peer's /agentmux/reactive/inject
  │     Latency: 1–5ms (loopback)
  │     Auth: entry.auth_key in X-AuthKey header
  │
  ├── Tier 3: LAN peer (mDNS discovery → HTTP) [TO BUILD]
  │     Condition: tier 1+2 failed, mDNS lookup finds host with agent B
  │     Transport: HTTP POST to peer's /agentmux/reactive/inject on LAN IP
  │     Latency: 5–30ms
  │     Auth: auth_key embedded in mDNS TXT record (or negotiated)
  │
  └── Tier 4: Cloud relay (WebSocket subscription) [NEEDS UPGRADE]
        Condition: all local/LAN tiers failed (or recipient is WAN-only)
        Transport: WebSocket to muxbus.agentmux.ai; cloud pushes to recipient's leaf node
        Latency: 50–300ms
        Auth: MUXBUS_TOKEN (Cognito JWT)
```

### Tier Promotion Logic

```
for tier in [1, 2, 3, 4]:
    result = try_deliver(tier, message)
    if result.success:
        return result
    if result.error == "not_found":
        continue           # recipient not at this tier, try next
    if result.error == "timeout":
        mark_tier_unhealthy(tier, 30s)
        continue           # tier temporarily down, skip
    if result.error == "auth_failed":
        break              # don't promote auth failures to cloud
return delivery_failed
```

**No parallel fan-out**: waterfall in order, not scatter-gather. Reasons:
- Same-host delivery is the common case and is <5ms — no benefit to concurrent cloud attempt.
- Parallel would risk double-delivery to agents running locally AND through cloud.
- Cloud tier has auth cost (JWT fetch); waste if agent is local.

**Exception**: cloud-push path (incoming cloud → local) is always delivered at Tier 1 or 2 — the sidecar never re-promotes a cloud message.

---

## 5. Cloud Push Subscription (Replace Polling)

### Current (polling-based — bad)

```
Agent running in pane:
  every 5s → GET /reactive/pending/{agent_id}
           → if any: deliver locally via AGENTMUX_LOCAL_URL
           → POST /reactive/ack

Problem: 0–5s latency for cloud→local; requires agent to run polling loop;
         N agents × 1 poll/5s = N×12 req/min against cloud.
```

### Target (WebSocket subscription — good)

```
Sidecar startup:
  → Open WebSocket to wss://muxbus.agentmux.ai/ws (with MUXBUS_TOKEN)
  → Send: { type: "subscribe", agents: ["agent-a", "agent-b", ...] }

Agent registers in sidecar:
  → Sidecar sends: { type: "subscribe:add", agents: ["new-agent"] }

Cloud receives message for "new-agent":
  → Finds sidecar WebSocket subscribed for "new-agent"
  → Pushes: { type: "inject", target: "new-agent", message: "...", id: "uuid" }
  → Sidecar delivers via Tier 1 (ReactiveHandler)
  → Sidecar sends: { type: "ack", id: "uuid" }

Benefits:
  - Zero polling — cloud push latency is network RTT only (<100ms)
  - One persistent WS per sidecar instance (not per agent)
  - Cloud maintains per-sidecar subscription map (DynamoDB or in-memory)
  - Sidecar automatically subscribes new agents as they register
```

### Sidecar WebSocket Manager (new component)

Lives in `agentmux-srv/src/muxbus/cloud_subscriber.rs`:

```rust
pub struct CloudSubscriber {
    token: Arc<Mutex<Option<String>>>,     // refreshed from DB
    ws: Arc<Mutex<Option<WebSocket>>>,
    subscribed_agents: Arc<Mutex<HashSet<String>>>,
    reactive_handler: &'static ReactiveHandler,
    wstore: Arc<Store>,
}

impl CloudSubscriber {
    pub async fn run(&self) {
        loop {
            match self.connect_and_subscribe().await {
                Ok(()) => {}   // clean disconnect
                Err(e) => {
                    tracing::warn!("cloud_subscriber: {e}, reconnecting in 5s");
                    tokio::time::sleep(Duration::from_secs(5)).await;
                }
            }
        }
    }

    async fn connect_and_subscribe(&self) -> Result<(), Error> {
        let token = self.load_token().await?;
        let ws = connect_wss("wss://muxbus.agentmux.ai/ws", &token).await?;
        // send subscribe for all known agents
        // then loop on incoming messages, deliver via ReactiveHandler
    }

    pub fn add_agent(&self, agent_id: &str) {
        // called by reactive/register handler
        // sends { type: "subscribe:add", agents: [agent_id] } if WS is open
    }

    pub fn remove_agent(&self, agent_id: &str) {
        // called by reactive/unregister handler
        // sends { type: "subscribe:remove", agents: [agent_id] } if WS is open
    }
}
```

**No MUXBUS_TOKEN = no cloud subscription**: if the user hasn't connected via `muxbus.login`, the sidecar doesn't open a cloud WS. Cloud→local delivery degrades gracefully to "unavailable" — local/LAN delivery is unaffected.

---

## 6. LAN Tier (Phase 3 build plan)

### Discovery: mDNS TXT records

Each sidecar advertises:
```
Service:  _agentmux._tcp.local
Host:     {hostname}.local
Port:     {web_port}
TXT:
  auth_key={auth_key}          # for forwarding requests
  instance={instance_id}       # dedup across channels
  version={version}
```

On startup: `lan_discovery` already binds mDNS. Extend it to include `auth_key` in TXT.

### Agent discovery across LAN

When tier 2 fails (no local file registry entry), before trying cloud:
1. Broadcast a DNS-SD query for `_agentmux._tcp.local`.
2. For each discovered peer, try `GET {peer_url}/agentmux/reactive/agent?id={agent_id}`.
3. First 200 response: forward inject to that peer.
4. Cache the peer URL in a short-lived in-memory map (`agent_id → peer_url`, TTL 60s).

This avoids broadcasting a full inject to all LAN peers. The agent lookup is a cheap idempotent GET.

### Auth

Peer's `auth_key` is in the mDNS TXT record. Use it as `X-AuthKey` when forwarding. This is safe since LAN traffic is not internet-exposed; same trust model as the existing tier 2 loopback forwarding.

---

## 7. Addressing

All tiers use the same logical address: `agent_id` (string, case-insensitive).

`agent_id` is:
- The agent's `name` field from `db_agent_definitions` (user-visible name like "claude", "codex")
- Or a fallback to `AGENT_NAME` env var (set by the launcher)
- Case-normalized to lowercase at delivery time

**No host:port in addresses** — location is resolved at delivery time, not embedded in the address. This allows agents to move between hosts without re-wiring senders.

---

## 8. Deduplication

Each message carries a `message_id: UUID`. Sidecars keep a **ring buffer of 200 recently-seen IDs** (per scope: local + cloud). On receive:
- If `message_id` in ring buffer: drop silently, ACK to sender.
- If not: deliver, add to ring buffer.

This handles:
- Cloud retrying after ACK timeout.
- Concurrent delivery attempts via tier 3 and tier 4 racing.

TTL on seen IDs: 5 minutes. Ring buffer evicts by age, not count.

---

## 9. Build Sequence

### Phase 1 — Unify existing tiers (low effort, high value)

Already works mechanically. Just needs wiring:

1. **Inject `AGENTMUX_AGENT_ID` into every spawn env** — muxbus-client reads this (preferred over the `AGENT_NAME` fallback) to know its own identity. Injected by the `websocket.rs` spawn path (next to `AGENTMUX_AUTH_KEY`), sourced from the block's `agentName` meta.
2. **Bundle muxbus-client** with agentmux-mcp package — currently agents must separately install it. Include in the tools bundle so every agent pane gets it.
3. **Fix muxbus-client local URL path** — client hits `/wave/reactive/inject` but sidecar exposes `/agentmux/reactive/inject`. Align these.

### Phase 2 — Cloud push subscription (replace polling)

Medium effort. Highest impact for reagent notifications:

1. Add `cloud_subscriber.rs` to `agentmux-srv/src/muxbus/`.
2. Wire into `ReactiveHandler::register_agent()` / `unregister_agent()` — subscriber tracks active agents.
3. Add WebSocket endpoint to MuxBus cloud server (`wss://muxbus.agentmux.ai/ws`).
4. Cloud server maintains `agent_id → [sidecar_ws]` map (DynamoDB TTL per subscription, heartbeat refresh).
5. On `POST /reactive/inject`: check subscription map first; if sidecar WS open, push and wait for ACK; only fall back to DynamoDB queue if no subscriber.
6. CloudSubscriber only runs when `MUXBUS_TOKEN` is present (user connected via `muxbus.login`).

### Phase 3 — LAN tier (mDNS agent lookup)

Low/medium effort:

1. Add `auth_key` to existing `lan_discovery` mDNS TXT record.
2. Add `GET /agentmux/reactive/agent?id=` endpoint (already exists via `/agentmux/reactive/agent`).
3. In `handle_reactive_inject()` — after tier 2 file registry miss, before tier 4 cloud: query mDNS for peers, try agent lookup, forward if found.
4. Add in-memory LAN peer cache (`HashMap<agent_id, (url, timestamp)>`, TTL 60s).

### Phase 4 — WAN promote via cloud relay

Already works at the API level. Needs:
1. muxbus-client to set message_id and pass it through all tiers.
2. Deduplication ring buffer on receive.
3. Cloud server WebSocket push (from Phase 2) handles the actual delivery.

---

## 10. What NOT to Build

- **Persistent message store on sidecar** — in-memory is fine. Lost on restart; users accept this. DynamoDB in cloud is the durable layer for WAN messages.
- **Message ordering guarantees** — best-effort FIFO. Cross-tier causal ordering is a CRDT/vector-clock problem; not worth it for CLI agent message passing.
- **Per-agent auth keys** — the per-sidecar auth_key is sufficient. Per-agent would require key distribution infra.
- **Direct WebSocket between sidecars on LAN** — HTTP forwarding (Tier 2/3) is simpler and the latency difference (5ms vs 2ms) doesn't matter for agent messaging.

---

## 11. Environment Variables (Unified Reference)

| Variable | Set By | Used By | Tier |
|----------|--------|---------|------|
| `AGENTMUX_LOCAL_URL` | sidecar (main.rs) | muxbus-client, bashwrap, mcp | Tier 1/2 |
| `AGENTMUX_AUTH_KEY` | sidecar (websocket.rs spawn) | muxbus-client, bashwrap | Tier 2 (forwarding) |
| `AGENTMUX_AGENT_ID` | sidecar (websocket.rs spawn, from `agentName` meta) | muxbus-client (self-identification; falls back to `AGENT_NAME`) | all |
| `MUXBUS_TOKEN` | sidecar (websocket.rs spawn, from db_muxbus_credentials) | muxbus-client (cloud auth), CloudSubscriber | Tier 4 |
| `MUXBUS_COGNITO_DOMAIN` | sidecar (websocket.rs spawn) | muxbus-client (token refresh) | Tier 4 |
| `MUXBUS_URL` | agent config / default | muxbus-client | Tier 4 |
| `MUXBUS_AGENT_ID` | agent config / AGENT_NAME fallback | muxbus-client | all |

---

## 12. Open Questions

1. **mDNS on Windows**: Windows Firewall prompts on mDNS bind. The existing `lan_discovery` already handles the opt-in toggle for this. Tier 3 should respect the same `network_lan_discovery` setting.

2. **Cloud WS reconnect during token expiry**: CloudSubscriber holds a token reference and refreshes from `db_muxbus_credentials`. If token expires mid-connection, the WS must close and re-open with a fresh token. Need backoff to avoid rapid reconnect loop.

3. **Multiple sidecar instances on same host (multi-channel)**: Tier 2 file registry handles this correctly today. Tier 3 (LAN) must de-dup — mDNS may return multiple instances of the same host, each at different ports. Agent lookup (`GET /reactive/agent`) is idempotent so querying both is safe.

4. **Container/SSH/WSL panes**: `AGENTMUX_LOCAL_URL` points to `127.0.0.1` which is unreachable from inside a container. For container panes, the sidecar should inject the Docker bridge IP or host-gateway instead. Out of scope for this spec.
