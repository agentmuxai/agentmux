# Local MessageBus Architecture

**Status:** Proposal
**Date:** 2026-03-04
**Author:** AgentX

---

## Problem

AgentBus currently routes all inter-agent communication through a cloud Lambda (agentbus.asaf.cc) backed by DynamoDB. Agents sitting in panes on the same machine round-trip to AWS for every message. This creates:

- **High latency** for `inject_terminal` (jekt) — HTTP polling instead of push
- **Cloud dependency** for core local functionality — agents can't communicate offline
- **No pane-level routing** — AgentMux has a WebSocket event bus but doesn't expose it for agent messaging
- **Fragile delivery** — polling means messages sit in DynamoDB until the recipient checks; no guaranteed delivery time

## Proposed Architecture

### Local-First, Cloud-Optional

Build a first-class `MessageBus` module inside agentmuxsrv-rs. All agents on the same machine communicate through the local backend's WebSocket infrastructure. AgentBus cloud becomes an optional bridge for cross-machine scenarios.

```
Local (agentmuxsrv-rs)                Cloud (optional)
┌───────────────────────────┐        ┌──────────────────┐
│  AgentMux Backend         │        │  AgentBus        │
│                           │  sync  │  Lambda/DynamoDB │
│  ┌─────────────────────┐  │◄──────►│  (cross-machine) │
│  │  MessageBus Module  │  │        └──────────────────┘
│  │                     │  │
│  │  - Agent Registry   │  │
│  │  - Route Table      │  │
│  │  - Message Queue    │  │
│  │  - Inject Engine    │  │
│  └──┬──┬──┬──┬─────────┘  │
│     │  │  │  │            │
│   Pane1  Pane2  Pane3     │  ← Already connected via WebSocket
│     │                     │
│   Docker agents           │  ← Connect via localhost TCP
└───────────────────────────┘
```

### Why Inside agentmuxsrv-rs

- WebSocket server already exists — panes are already connected
- Auth infrastructure already exists (AGENTMUX_AUTH_KEY)
- EventBus + WPS Broker already handle pub/sub between panes
- No second server to manage, no additional ports to coordinate
- Version isolation (from the multi-version work) keeps each version's bus independent

## Components

### 1. Agent Registry

Tracks connected agents and their WebSocket connections.

```
AgentRegistry {
    agents: HashMap<AgentId, AgentConnection>
}

AgentConnection {
    id: String,              // "agentx", "agent1", etc.
    ws_connection_id: u64,   // WebSocket connection handle
    pane_id: Option<String>, // AgentMux pane (if applicable)
    registered_at: DateTime,
    last_seen: DateTime,
    capabilities: Vec<String>, // "inject", "message", "broadcast"
}
```

Agent identity comes from the `WAVEMUX_AGENT_ID` environment variable already set per pane. On WebSocket connect, agents send a registration message with their ID.

### 2. Message Router

Routes messages between agents. Supports:

| Operation | Description | Delivery |
|-----------|-------------|----------|
| `send` | Point-to-point message | Async, queued if offline |
| `inject` | Terminal injection (jekt) | Immediate, push via WS |
| `broadcast` | Message all agents | Fan-out to all connections |
| `request` | RPC-style request/response | Correlated via message ID |

```
Message {
    id: Uuid,
    from: AgentId,
    to: AgentId,           // or "*" for broadcast
    type: MessageType,     // Send | Inject | Broadcast | Request
    payload: String,
    priority: Priority,    // Normal | High | Urgent
    timestamp: DateTime,
    ttl: Option<Duration>, // Auto-expire
}
```

### 3. Inject Engine

The core of `inject_terminal` (jekt). Instead of storing a message in DynamoDB and waiting for the target to poll:

1. Sender calls `inject_terminal(target: "agent1", message: "do the thing")`
2. MessageBus looks up Agent1's WebSocket connection
3. Sends a `terminal_inject` event directly over the WebSocket
4. AgentMux frontend receives the event and writes to the terminal's stdin
5. Agent1's Claude Code session sees it as user input

This is push-based, sub-millisecond on localhost. No polling.

### 4. Offline Queue

If a target agent isn't connected, messages are queued in-memory (with optional SQLite persistence). When the agent connects, queued messages are delivered.

```
OfflineQueue {
    queues: HashMap<AgentId, VecDeque<Message>>,
    max_queue_size: usize,  // Per agent, default 1000
    ttl: Duration,          // Auto-expire old messages, default 1h
}
```

### 5. Cloud Bridge (Optional)

AgentBus cloud becomes a sync layer:

- **Outbound**: Messages marked `cloud: true` are forwarded to AgentBus Lambda
- **Inbound**: Cloud messages are pulled and injected into local bus
- **Use cases**: Cross-machine agents, mobile notifications, audit log
- **Disabled by default**: Local bus works without any cloud config

```
CloudBridge {
    enabled: bool,
    agentbus_url: Option<String>,
    agentbus_token: Option<String>,
    sync_interval: Duration,
}
```

## API Surface

### WebSocket Events (Internal)

Sent over existing AgentMux WebSocket connections:

```json
// Agent registration (on connect)
{"type": "bus:register", "agent_id": "agentx", "capabilities": ["inject", "message"]}

// Send message
{"type": "bus:send", "to": "agent1", "payload": "...", "priority": "normal"}

// Inject terminal
{"type": "bus:inject", "to": "agent1", "message": "do the thing", "priority": "urgent"}

// Broadcast
{"type": "bus:broadcast", "payload": "...", "priority": "normal"}

// Incoming message (pushed to recipient)
{"type": "bus:message", "from": "agentx", "payload": "...", "id": "uuid"}

// Incoming injection (pushed to recipient)
{"type": "bus:inject_received", "from": "agentx", "message": "...", "id": "uuid"}
```

### HTTP Endpoints (External)

For container agents and tools that don't have a WebSocket connection:

```
POST /api/bus/register      - Register agent
POST /api/bus/send          - Send message
POST /api/bus/inject        - Inject terminal
POST /api/bus/broadcast     - Broadcast
GET  /api/bus/messages      - Read messages (polling fallback)
GET  /api/bus/agents        - List connected agents
DELETE /api/bus/messages     - Delete messages
```

These use the same auth key as the rest of the backend API.

### MCP Client Changes

`agentbus-client` switches transport:

```
Before: HTTP → Lambda (agentbus.asaf.cc) → DynamoDB
After:  HTTP → localhost:PORT/api/bus/*   (or WebSocket)
```

The MCP tool interface stays identical. Agents don't need to change their tool calls. The client just reads `AGENTBUS_URL` — point it to `http://localhost:{port}` instead of `https://agentbus.asaf.cc`.

## Container Agent Connectivity

Docker container agents (Agent1-5) need to reach the host's agentmuxsrv-rs:

1. **Host networking**: Container uses `host.docker.internal` to reach the host
2. **Port discovery**: Read endpoints from mounted volume or environment variable
3. **Auth**: Same `AGENTMUX_AUTH_KEY` passed as environment variable

Claw already manages container configuration — it would set:
```
AGENTBUS_URL=http://host.docker.internal:{backend_port}
AGENTBUS_AGENT_ID=agent1
AGENTBUS_TOKEN={auth_key}
```

## Migration Path

### Phase 1: Local Bus Module
- Add `backend::messagebus` module to agentmuxsrv-rs
- Implement agent registry, message routing, inject engine
- Add HTTP API endpoints (`/api/bus/*`)
- Add WebSocket event handlers for bus messages

### Phase 2: MCP Client Update
- Update `agentbus-client` to support local URL
- Add auto-discovery: try localhost first, fall back to cloud
- Update claw templates to point MCP config to local backend

### Phase 3: Frontend Integration
- AgentMux panes auto-register on the bus via WebSocket
- Terminal inject writes directly to pane stdin
- Agent status visible in UI (online/offline indicators)

### Phase 4: Cloud Bridge
- Optional sync to AgentBus Lambda for cross-machine use
- Cloud becomes read-through cache / event forwarder
- Local bus is always authoritative

## Backward Compatibility

- Existing `agentbus-client` MCP tools keep working — just change the URL
- Cloud AgentBus stays operational during migration
- Agents that can't reach localhost fall back to cloud automatically
- No breaking changes to agent workflows or CLAUDE.md instructions

## Open Questions

1. **SQLite vs in-memory for offline queue?** SQLite survives backend restarts but adds I/O. In-memory is faster but loses messages on crash. Recommendation: in-memory with optional SQLite persistence flag.

2. **Multi-version bus isolation?** With version-namespaced backends, each version runs its own bus. Should agents on v0.31.26 be able to message agents on v0.31.25? Recommendation: no — version isolation is intentional. Cloud bridge handles cross-version if needed.

3. **Rate limiting?** Should the local bus rate-limit messages? Probably not for local — trust local agents. Cloud bridge should rate-limit outbound.

4. **Message format compatibility?** Should local bus messages be wire-compatible with AgentBus cloud messages? Recommendation: yes, same JSON schema, makes cloud bridge trivial.
