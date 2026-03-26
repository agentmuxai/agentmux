# Analysis: LAN Instance Awareness & Embedded Jekt API

**Date:** 2026-03-26
**Author:** agent2
**Status:** Draft — investigation & analysis

---

## Context

AgentMux instances currently operate in isolation unless connected through AgentBus (a separate cloud-backed service at `agentbus.asaf.cc`). Two related goals:

1. **LAN awareness** — AgentMux instances should discover and be aware of each other on the local network without any external service
2. **Remove AgentBus dependency for local/LAN jekt** — The `inject_terminal` (jekt) capability should work through an API embedded directly in AgentMux for intra-instance and LAN scenarios. AgentBus is **not eliminated** — it remains as the cloud layer for cross-network (WAN) delivery only

---

## Current Architecture

### How Jekt Works Today (3-tier delivery)

```
Agent (e.g., Claude Code)
  │
  ├─ MCP tool: inject_terminal
  │   └─ @a5af/agentbus-client (Node.js stdio MCP server)
  │       │
  │       ├─ Try 1: LOCAL — POST http://127.0.0.1:PORT/wave/reactive/inject
  │       │   └─ ReactiveHandler → blockcontroller::send_input() → PTY write
  │       │   └─ Sub-millisecond, synchronous
  │       │
  │       ├─ Try 2: CROSS-INSTANCE — File registry lookup → HTTP forward
  │       │   └─ {data_dir}/agents/{agent_id}.json → POST remote:PORT/wave/reactive/inject
  │       │
  │       └─ Try 3: CLOUD — POST https://agentbus.asaf.cc/reactive/inject
  │           └─ Lambda + DynamoDB, 15s polling timeout
  │           └─ Remote agent's MCP client polls /reactive/pending/{agent_id}
  │
  └─ MCP tool: send_message (async mailbox, cloud-backed)
```

### What Already Exists in AgentMux Backend

The Rust backend (`agentmuxsrv-rs`) already has **all the primitives** for local messaging:

| Component | Location | Purpose |
|-----------|----------|---------|
| **ReactiveHandler** | `backend/reactive/handler.rs` | Agent→block mapping, PTY injection, rate limiting, audit log |
| **MessageBus** | `backend/messagebus.rs` | Point-to-point send, inject, broadcast, offline queues |
| **Agent Registry** | `backend/reactive/registry.rs` | File-based cross-instance agent discovery |
| **WebSocket bus commands** | `server/websocket.rs` | `bus:register`, `bus:send`, `bus:inject`, `bus:broadcast` |
| **HTTP API** | `server/reactive.rs`, `server/messagebus.rs` | Full REST API for all operations |
| **Cross-instance forwarding** | `server/reactive.rs:32-77` | HTTP forward to remote AgentMux via registry lookup |

### AgentBus Cloud: Scoped Role

AgentBus (`agentbus.asaf.cc`) is **not being removed**. Its role is being **scoped down** to cloud/WAN-only delivery — the third and final layer. It is not needed for:
- **Intra-instance** jekt (same AgentMux process) — handled by ReactiveHandler + MessageBus
- **LAN** jekt (same network, different machines) — handled by mDNS discovery + HTTP forwarding

AgentBus remains the fallback for:
- **Cross-network (WAN)** jekt — agents on different networks with no LAN path
- **Cloud persistence** — audit trail, message history across sessions
- **Disconnected delivery** — agents that are offline and not reachable via LAN

### What Changes for AgentBus

| Before | After |
|--------|-------|
| AgentBus MCP client is the **primary** entry point for all jekt | AgentMux embedded MCP server is primary; AgentBus is cloud-only fallback |
| Every agent needs `AGENTBUS_TOKEN` + `AGENTBUS_URL` | Only needed if cloud/WAN delivery is configured |
| `@a5af/agentbus-client` Node.js process per agent | Not needed for local/LAN — only if cloud layer enabled |
| Cloud Lambda used even for same-machine jekt failures | Cloud only used after local + LAN both fail |

**Pain points being addressed (local/LAN path):**
- Extra Node.js process per agent session (eliminated for local/LAN)
- Cloud Lambda latency and failure modes (bypassed for local/LAN)
- 15-second polling timeout (replaced by direct HTTP on LAN)
- Mandatory token/URL config even for purely local setups

---

## Proposal: Embedded Jekt API (MCP Server in agentmuxsrv-rs)

### Core Idea

Move the MCP server **into the AgentMux backend itself**. Instead of a separate Node.js stdio process, agents connect to AgentMux's own MCP endpoint.

### Option A: AgentMux as MCP Server (SSE transport)

MCP supports [SSE (Server-Sent Events) transport](https://spec.modelcontextprotocol.io/specification/basic/transports/#http-with-sse) — a server exposes an HTTP endpoint that MCP clients connect to.

```
Agent (Claude Code)
  │
  └─ .mcp.json: { "agentmux": { "type": "sse", "url": "http://127.0.0.1:PORT/mcp" } }
      │
      └─ agentmuxsrv-rs handles MCP protocol directly
          ├─ inject_terminal → ReactiveHandler (same process, zero overhead)
          ├─ send_message → MessageBus
          ├─ list_agents → ReactiveHandler + LAN discovery
          └─ broadcast_message → MessageBus
```

**Advantages:**
- No extra process (Node.js MCP server eliminated)
- Zero-copy delivery — MCP tool handler calls ReactiveHandler directly (in-process)
- Auth inherited from AgentMux's existing `X-AuthKey`
- Claude Code, Codex CLI, and Gemini CLI all support SSE MCP servers

**Implementation:**
- New module: `agentmuxsrv-rs/src/server/mcp.rs`
- Implements MCP protocol (tool listing, tool execution, SSE streaming)
- Route: `GET /mcp` (SSE) + `POST /mcp` (tool calls)
- Tools: `inject_terminal`, `send_message`, `read_messages`, `list_agents`, `broadcast_message`
- Rust crate: `rmcp` or hand-roll (MCP protocol is simple JSON-RPC over SSE)

### Option B: Thin stdio wrapper (simpler, incremental)

Keep the stdio MCP pattern but replace `@a5af/agentbus-client` with a minimal wrapper that only talks to the local AgentMux API. No cloud fallback.

```
Agent (Claude Code)
  │
  └─ .mcp.json: { "agentmux": { "type": "stdio", "command": "wsh", "args": ["mcp"] } }
      │
      └─ wsh mcp (Rust binary, already distributed with AgentMux)
          └─ POST http://127.0.0.1:PORT/api/bus/inject (local only)
```

**Advantages:**
- Uses existing `wsh` binary (already installed with AgentMux)
- No npm dependency
- Simpler than full SSE server
- `wsh` already knows `AGENTMUX_LOCAL_URL`

**Disadvantages:**
- Still spawns a child process per agent
- Not as clean as Option A

### Recommendation: Option A (SSE MCP Server)

Option A eliminates an entire process layer. The MCP protocol is straightforward (JSON-RPC 2.0 over SSE), and `agentmuxsrv-rs` already has Axum + tokio — adding SSE endpoints is minimal work.

---

## Proposal: LAN Instance Discovery

### Approach: mDNS/DNS-SD (as designed in `statusbar-network-stats.md`)

Each AgentMux instance advertises itself on the local network via mDNS:

```
Service: _agentmux._tcp.local
TXT records: version=0.31.x, hostname=macbook-pro, agents=agent1,agent2
```

Other instances automatically discover peers. No central server needed.

### Extended for Cross-Instance Jekt

With LAN discovery, jekt delivery becomes a **4-tier cascade** with clear separation of concerns:

```
Jekt delivery priority:
  1. LOCAL PTY     — agent on this instance → direct PTY write (sub-ms)
  2. LOCAL MSGBUS  — agent has WS connection → push via MessageBus (ms)
  3. LAN FORWARD   — agent on LAN peer → HTTP POST to peer's /wave/reactive/inject (low ms)
  4. CLOUD RELAY   — agent unreachable locally/LAN → AgentBus cloud (agentbus.asaf.cc)
```

**Tiers 1-3** are handled entirely by AgentMux (embedded). No external dependencies.
**Tier 4** is AgentBus cloud — only invoked when local and LAN paths both fail. This is the only tier that requires `AGENTBUS_TOKEN`/`AGENTBUS_URL`.

### Layer Responsibilities

| Layer | Handled by | Latency | Dependencies |
|-------|-----------|---------|-------------|
| Intra-instance | AgentMux (ReactiveHandler + MessageBus) | < 1ms | None |
| LAN | AgentMux (mDNS + HTTP forward) | 1-10ms | None |
| Cloud/WAN | AgentBus (`agentbus.asaf.cc`) | 100ms-15s | Lambda, DynamoDB, auth token |

For most users (single machine or local network), AgentBus cloud is never needed. It becomes an opt-in capability for distributed/remote teams.

### Implementation

**Rust crate:** `mdns-sd` (pure Rust, async, well-maintained)

**New module:** `agentmuxsrv-rs/src/backend/lan_discovery.rs`

```rust
pub struct LanDiscovery {
    daemon: ServiceDaemon,
    instances: Arc<RwLock<HashMap<String, LanInstance>>>,
}

pub struct LanInstance {
    pub instance_id: String,     // Unique per agentmux process
    pub hostname: String,
    pub version: String,
    pub address: IpAddr,
    pub port: u16,
    pub agents: Vec<String>,     // Agent IDs registered on that instance
    pub first_seen: SystemTime,
    pub last_seen: SystemTime,
}
```

**Lifecycle:**
1. On startup: register `_agentmux._tcp.local` with TXT records
2. Browse for peers continuously
3. When agent registers/unregisters: update TXT record's `agents` field
4. Broadcast `laninstances` event to frontend on changes
5. On shutdown: gracefully deregister

**Frontend integration:**
- `lanInstancesAtom` in global store
- `LanStatus.tsx` component in status bar: `◆ 3 on LAN`
- Click opens popover: hostname, version, agents, address for each peer

---

## Combined Architecture (Target State)

```
┌───────────────────────────────────────────────────────────────┐
│  Agent Session (Claude Code / Codex / Gemini)                │
│                                                               │
│  .mcp.json → SSE: http://127.0.0.1:PORT/mcp                 │
│  (connects directly to AgentMux backend, no intermediary)    │
└──────────────────────────┬────────────────────────────────────┘
                           │ MCP over SSE
                           ▼
┌──────────────────────────────────────────────────────────────┐
│  AgentMux Backend (agentmuxsrv-rs)                          │
│                                                              │
│  ┌─────────────────────────────────────────────────────────┐ │
│  │ MCP Server (/mcp endpoint)                              │ │
│  │  inject_terminal → ReactiveHandler (in-process)        │ │
│  │  send_message → MessageBus                             │ │
│  │  list_agents → local registry + LAN peers              │ │
│  │  broadcast → MessageBus + LAN fan-out                  │ │
│  └─────────────────────────────────────────────────────────┘ │
│                                                              │
│  ┌──────────────────────┐  ┌──────────────────────────────┐ │
│  │ ReactiveHandler      │  │ MessageBus                   │ │
│  │ agent→block mapping  │  │ offline queues, WS push      │ │
│  │ PTY injection        │  │ point-to-point + broadcast   │ │
│  │ rate limiting        │  └──────────────────────────────┘ │
│  │ audit logging        │                                    │
│  └──────────────────────┘                                    │
│                                                              │
│  ┌──────────────────────────────────────────────────────────┐│
│  │ LAN Discovery (mDNS)                                     ││
│  │  Advertise: _agentmux._tcp.local                         ││
│  │  Browse: discover peers automatically                     ││
│  │  Forward: jekt to agents on peer instances                ││
│  └──────────────────────────────────────────────────────────┘│
│                                                              │
│  ◆─────── mDNS ───────◆─────── mDNS ───────◆               │
│           (LAN)                 (LAN)                        │
└──────────────────────────────────────────────────────────────┘
         ▲                         ▲              │
         │ HTTP forward            │ HTTP forward  │ Tier 4 (cloud fallback)
         ▼                         ▼              ▼
┌──────────────────┐     ┌──────────────────┐   ┌─────────────────────────┐
│ AgentMux Peer A  │     │ AgentMux Peer B  │   │ AgentBus Cloud (opt-in) │
│ (LAN machine)    │     │ (LAN machine)    │   │ agentbus.asaf.cc        │
│ Tier 3: LAN      │     │ Tier 3: LAN      │   │ Lambda + DynamoDB       │
└──────────────────┘     └──────────────────┘   │ WAN/cross-network only  │
                                                 └─────────────────────────┘
```

**What's new (Tiers 1-3: embedded in AgentMux):**
- MCP SSE server built into agentmuxsrv-rs — no external process needed
- mDNS LAN discovery for cross-machine awareness and forwarding
- Zero-dependency local/LAN jekt — works offline, no tokens, no cloud

**What's no longer required for local/LAN:**
- `@a5af/agentbus-client` npm package (Node.js MCP server)
- `AGENTBUS_TOKEN` / `AGENTBUS_URL` env vars
- Cloud Lambda round-trip

**What's preserved (Tier 4: AgentBus cloud):**
- `agentbus.asaf.cc` remains as opt-in cloud relay for WAN delivery
- DynamoDB-backed message persistence for cross-network scenarios
- `@a5af/agentbus-client` can still be configured alongside for cloud features
- All current jekt semantics (PTY injection, timing, audit, rate limiting)

---

## Implementation Phases

### Phase 0: Bug fix (already spec'd)
- Filter phantom "1 connection" in ConnectionStatus

### Phase 1: LAN Discovery (no AgentBus changes yet)
- Add `mdns-sd` crate
- Create `lan_discovery.rs` module
- Advertise + browse on startup
- Frontend: `LanStatus.tsx` in status bar
- API: `GET /api/lan-instances`

### Phase 2: Embedded MCP Server
- Add MCP SSE endpoint to agentmuxsrv-rs (`/mcp`)
- Implement tool handlers (inject, send, list, broadcast, read, delete)
- Update `.mcp.json` template to use SSE transport
- Test with Claude Code, Codex CLI, Gemini CLI

### Phase 3: LAN-based Jekt Forwarding
- When `inject_terminal` target not found locally, query LAN peers
- Forward via HTTP to peer's `/wave/reactive/inject`
- Update `list_agents` to include agents from LAN peers
- Update `broadcast` to fan out to LAN peers

### Phase 4: Cloud Fallback Integration (AgentBus as Tier 4)
- Embedded MCP server's `inject_terminal` adds cloud fallback as final tier
- If local (tier 1-2) and LAN (tier 3) both fail → relay via `agentbus.asaf.cc`
- Cloud config is **opt-in**: only activated if `AGENTBUS_URL` + `AGENTBUS_TOKEN` are set
- No cloud config = tiers 1-3 only (fully self-contained)

### Phase 5: Simplify Default Config
- Default `.mcp.json` uses embedded SSE server only (no `@a5af/agentbus-client`)
- `@a5af/agentbus-client` remains available as optional add-on for users who need cloud relay without AgentMux
- Claw templates updated: cloud env vars only injected when cloud features explicitly enabled
- Document the 3-layer architecture and when each layer is needed

---

## Open Questions

1. **MCP transport**: SSE vs stdio (`wsh mcp`)? SSE is cleaner but requires MCP client support for SSE. Claude Code supports it. Verify Codex CLI and Gemini CLI.

2. **LAN security**: mDNS is unauthenticated. Should cross-instance jekt require a shared secret? Options:
   - No auth (trust the LAN) — simplest, fine for dev environments
   - Shared key derived from AgentMux config — moderate complexity
   - mTLS between instances — overkill for now

3. **Firewall/corporate networks**: mDNS uses UDP:5353. Blocked on some networks. Fallback: manual peer list in config? Or just degrade gracefully (no LAN awareness).

4. **Agent namespace collisions**: Two instances might have agents with the same ID (e.g., both have "agent1"). Need instance-scoped agent IDs or a conflict resolution strategy.

5. **WAN use cases**: Cross-network jekt **is** needed for distributed teams. AgentBus cloud remains as the opt-in Tier 4 relay for this. No separate relay service needed — AgentBus already does this.

---

## Key Files Reference

| Current file | Role |
|-------------|------|
| `agentmuxsrv-rs/src/backend/reactive/handler.rs` | PTY injection engine |
| `agentmuxsrv-rs/src/backend/reactive/registry.rs` | File-based agent registry |
| `agentmuxsrv-rs/src/backend/messagebus.rs` | Local message broker |
| `agentmuxsrv-rs/src/server/websocket.rs` | WS handler + bus commands |
| `agentmuxsrv-rs/src/server/reactive.rs` | HTTP endpoints + cross-instance forward |
| `agentmuxsrv-rs/src/server/messagebus.rs` | MessageBus HTTP endpoints |
| `agentmuxsrv-rs/src/server/mod.rs` | Router, AppState, all routes |
| `agentmuxsrv-rs/src/main.rs` | Startup wiring, AGENTMUX_LOCAL_URL |
| `specs/statusbar-network-stats.md` | Existing LAN discovery spec |
| `specs/jekt-inject-timing.md` | PTY injection timing spec |
| `specs/jekt-auto-registration.md` | Auto-registration spec |
| `specs/agentmux-local-url-injection.md` | Local URL env var spec |
