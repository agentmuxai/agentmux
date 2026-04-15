# Spec: Agent host context — machine binding, agentbus addressing, status bar integration, service attribution

**Status:** Draft
**Date:** 2026-04-14
**Scope:** `ForgeAgent` host identity, agentbus cross-host routing, `LanInstance` agent
advertisement, `HostPopover` agent listing, agent pane host badge,
tool/service interaction attribution
**Related:** `SPEC_AGENT_IDENTITY_RESTRUCTURE_2026_04_14.md`,
`SPEC_CONSOLIDATE_FORGE_IDENTITY_INTO_AGENT_2026_04_13.md`

---

## 1. Problem

AgentMux already has the infrastructure for cross-host agent communication:

- `agent_bus_id` is a global bus address used by `agentbus-client` to route messages
  across machines. `AGENTMUX_AGENT_BUS_ID` is injected into every MCP server env
  at launch time; `AGENTMUX_LOCAL_URL` lets the agentbus-client prefer direct PTY
  delivery when source and target are on the same machine, falling back to the cloud
  agentbus for cross-host delivery.
- `LanInstance` already has an `agents: Vec<String>` field reserved for advertising
  which agents are active on each LAN peer — but it is never populated (always
  `Vec::new()` in `handle_event`).
- The `HostPopover` in the bottom-right status bar shows the machine hostname, OS,
  IP, and LAN peers — but has no concept of *which ForgeAgent represents this machine*
  or *which agents are reachable on each peer via the bus*.

Because these systems are disconnected:

- You can see LAN peers but not which agents are active on them.
- When an agent makes a tool call or service call, there is no indication of which
  host it ran on — critical when agents on multiple machines share a pane view.
- The `HostPopover` is purely machine-level; it says nothing about the agent network
  topology that's actually in use.

---

## 2. What the user asked for

> "Agents also need to maintain concept of their host. It is managed also in the
> bottom right of agentmux. Whenever the agent interacts with a service, it should
> also say the host it is on."

> "agentbus is a good point, since we want to support messages between agentmux
> instances running on different hosts"

Three concrete asks:

1. **Agents know their host** — the host ForgeAgent is the identity anchor for the
   machine it runs on; container agents carry that context.
2. **Bottom-right integration** — the `HostPopover` surfaces which agent is the
   host for the local machine, and shows which agents are active on each LAN peer
   (pulling from the already-existing `LanInstance.agents` field once populated).
3. **Service interaction attribution** — whenever an agent makes a tool call or
   service interaction, the host context (agent + machine) appears alongside it.

---

## 3. The agentbus and cross-host routing

Understanding this is prerequisite to the data model.

### 3.1 Three-tier delivery — no cloud required on a LAN

```
Agent A (machine 1)                        Agent B (machine 2)
  │                                             │
  │ agentbus-client                             │ agentbus-client
  │   AGENTMUX_AGENT_BUS_ID=agenta              │   AGENTMUX_AGENT_BUS_ID=agentb
  │   AGENTMUX_LOCAL_URL=http://127.x:port      │   AGENTMUX_LOCAL_URL=http://127.x:port
  │                                             │
  ├─ 1. Same machine?                           │
  │      → POST AGENTMUX_LOCAL_URL (loopback)   │
  │                                             │
  ├─ 2. LAN peer? (agentb in LanInstance.agents)│
  │      → POST http://<peer.address>:<peer.port>│  ◄── direct, no cloud
  │                                             │
  └─ 3. Not on LAN?                             │
         → POST cloud agentbus ─────────────────►
                                                └── POST AGENTMUX_LOCAL_URL
```

**Tier 1 (loopback):** already implemented via `AGENTMUX_LOCAL_URL`.

**Tier 2 (LAN direct):** the agentbus-client needs a local routing table mapping
`agent_bus_id → http://<peer.address>:<peer.port>`. This table is built from
`LanInstance.agents` once it is populated (§5). No cloud hop, no latency, no
dependency on external connectivity. This is the primary delivery path for
multi-machine developer setups.

**Tier 3 (cloud):** fallback for agents not on the local network — off-site machines,
cloud VMs, mobile. Not required for local-only deployments.

### 3.2 `agent_bus_id` — a global address, not a local key

`agent_bus_id` is the **network address** of an agent in the bus. Other agents on
any machine use it to route messages. It is:
- Set once at creation (typically = `agent.slug`)
- Unique across all AgentMux instances that may communicate
- Injected as `AGENTMUX_AGENT_BUS_ID` into every MCP server env at launch
- Used by the agentbus-client to look up Tier 1/2/3 delivery (in that order)

`AGENTMUX_LOCAL_URL` is the **inbound delivery endpoint** for the local instance —
all three tiers ultimately POST to the target machine's `AGENTMUX_LOCAL_URL`.

### 3.3 `LanInstance.agents` is the LAN routing table

`LanInstance.agents: Vec<String>` already exists in the struct but is always
`Vec::new()`. Once populated with `agent_bus_id`s, it becomes the data the
agentbus-client reads to answer: "is target bus ID reachable on a LAN peer, and
if so what is `peer.address:peer.port`?"

This makes §5 (LAN advertisement) load-bearing for routing, not just display.

### 3.4 Host agents anchor the bus on a machine

A **host agent** (`agent_type: "host"`) is both:
- The **identity** of a person+machine combination (AgentX = the agent that runs
  on this dev workstation)
- The **LAN entry point** for that machine: when other agents look up a bus ID in
  the LAN routing table, they find it under the peer that advertised it — the host
  machine.

Container agents (`agent_type: "container"`) have their own `agent_bus_id` and are
also advertised in `LanInstance.agents`. Their *display context* (which machine they
ran on) is tracked by `host_agent_slug` (§4.1), but their routing is independent.

### 3.5 What changes this spec proposes

The spec does **not** change the agentbus wire protocol. It adds:

1. A new `host_agent_slug` field on ForgeAgent — the display/identity link between
   a container agent and its host agent (separate from the routing key).
2. Populating `LanInstance.agents` with the `agent_bus_id`s of running agents so
   LAN peers can see which agents are reachable on each machine.
3. UI surfaces that expose this topology (HostPopover, agent pane badges, tool attribution).

---

## 4. Data model changes

### 4.1 New field: `host_agent_slug`

Add `host_agent_slug: string` to `ForgeAgent` (v5 SQLite migration, additive
`ALTER TABLE ADD COLUMN`).

| Agent type | `host_agent_slug` value |
|---|---|
| `host` | `self.slug` — the agent is its own host |
| `container` | slug of the host agent whose machine it runs on (e.g. `"agentx"`) |
| `standalone` | `""` — treated as local, no explicit host |

**Why separate from `agent_bus_id`?**
`agent_bus_id` is the global bus routing key — a network address. It must be stable
and unique across all connected AgentMux instances. `host_agent_slug` is the
*machine-identity display* — which human-readable agent represents the machine.
Renaming an agent (slug → name change) must not break bus routing. And in the
future, one machine could have multiple host agents with different `agent_bus_id`s
(one per provider) but the same `host_agent_slug` pointing to the primary host.

### 4.2 Seeded agents — `host_agent_slug` defaults

In `forge-seed.json`, container agents should declare `host_agent_slug` explicitly.
For existing seeded agents, the migration backfills:
- `agent_type == "host"` → `host_agent_slug = slug`
- `agent_type == "container"` → `host_agent_slug = ""` (users/seed update to fill in)

### 4.3 `HostInfo` extension

Extend `HostInfo` (returned by `get_host_info` CEF command) with:

```rust
pub struct HostInfo {
    // ...existing fields...
    pub agent_slug: String,   // local host agent slug, "" if unresolved
    pub agent_name: String,   // display name
    pub agent_icon: String,   // emoji
    pub agent_bus_ids: Vec<String>, // all bus_ids of agents currently running locally
}
```

`agent_bus_ids` is used to populate `LanInstance.agents` on advertisement (§5).

---

## 5. LAN discovery — populate `agents` (the LAN routing table)

`LanInstance.agents: Vec<String>` exists but is always `Vec::new()`. Populating
it is what enables Tier 2 (LAN direct) delivery in §3.1 — without it, the
agentbus-client cannot know which LAN peer hosts a given `agent_bus_id`.

### 5.1 What to advertise

On startup (and whenever the agent list changes), the backend builds the list of
`agent_bus_id`s for **all** agents configured in Forge (host and container) and
injects them into the mDNS advertisement as an additional property:

```
agents = "agentx,agenty,agent1,agent2"   // comma-separated bus IDs
```

The mDNS properties dict supports arbitrary string k/v pairs. mDNS has a ~1300
byte TXT record limit; with typical 6-10 char bus IDs and a handful of agents this
is not a concern in practice. If it ever is, truncate to host agents only.

When a peer is resolved in `handle_event`, parse `agents` and populate
`LanInstance.agents`.

### 5.2 Dynamic updates

When the user creates or deletes an agent, the mDNS service is re-registered with
the updated `agents` property. The backend event bus already fires `forgeagents:changed`
on Forge mutations — hook into this in `LanDiscovery` to trigger re-advertisement.

### 5.3 Why both host and container agents

Container agents have their own globally unique `agent_bus_id`. If Agent1 (container)
is running on machine 1 and Agent B (on machine 2) wants to message it directly,
Agent B needs to find `agent1` in a LAN peer's `agents` list to learn the delivery
address. If only host agents are advertised, Tier 2 delivery is unavailable for
container-to-container cross-machine messaging.

Display consequence: the HostPopover peer list will show all agents on a peer,
grouped by type (hosts prominent, containers indented).

### 5.4 agentbus-client LAN routing table

The frontend exposes `LanInstance[]` via `lanInstancesAtom`. The agentbus-client
(running as an MCP server process) currently reads only `AGENTMUX_LOCAL_URL` and
`AGENTMUX_AGENT_BUS_ID`. To enable Tier 2 delivery, add a new env var:

```
AGENTMUX_LAN_PEERS=agentx=http://192.168.1.42:8765,agenty=http://192.168.1.42:8765,agent1=http://192.168.1.55:8765
```

Format: `bus_id=delivery_url` pairs, comma-separated. Built from `lanInstancesAtom`
at launch time and re-injected when `lanInstances` changes (requires controller
restart or live env update — implementation detail TBD).

The agentbus-client lookup order becomes:
1. `bus_id == AGENTMUX_AGENT_BUS_ID` (self) → local in-process
2. `bus_id` in `AGENTMUX_LAN_PEERS` → POST to peer's URL directly
3. Otherwise → POST to cloud agentbus

---

## 6. Local host agent resolution

On startup, the backend resolves the **local host agent** — the ForgeAgent that
"owns" this AgentMux instance:

1. Read all ForgeAgents where `agent_type == "host"`.
2. If exactly one: use it.
3. If multiple: prefer the one where `environment` matches current platform
   (`windows` / `linux` / `macos`). Tie-break: lowest `created_at`.
4. If none: slug = `""`.

Exposed via new backend command `GetLocalHostAgent` →
`{ slug, name, icon, agent_bus_id }`.

Frontend: `localHostAgentAtom` (reactive, populated once on mount in `app.tsx`).

---

## 7. Status bar — HostPopover enhancements

### 7.1 Host agent badge in trigger

When the local host agent is resolved, the trigger gains an icon prefix:

```
[🔴] dev-workstation
```

Tooltip: `AgentX on dev-workstation · Click for details`.

### 7.2 Popover — host agent section (top)

```
┌─ THIS HOST ───────────────────────────┐
│  🔴  AgentX                           │
│      agentx · Claude Code · bus: agentx │
└───────────────────────────────────────┘
  OS       Windows 10
  IP       192.168.1.42
  ...
```

Fields shown:
- Icon + display name
- Slug (monospace, dimmed) · Provider label · bus ID (monospace, dimmed)

If no host agent resolved:
```
  No host agent configured
  Set one in Forge ▶
```
(link opens the forge widget)

### 7.3 Popover — LAN peers with agent column

Each LAN peer row gains a right-side agent column showing the `agents` bus IDs,
resolved to display names where local ForgeAgent records match:

```
◆  peer-hostname.local    🟡 AgentY          v0.33.160
◆  server01.local         🔵 Agent3          v0.33.155
◆  laptop.local           ?  (2 agents)      v0.33.140
```

- Icon + name if a ForgeAgent with that `agent_bus_id` exists locally.
- `(N agents)` if the peer advertises multiple agents not in local Forge.
- `?` placeholder if `agents` is empty (older peer version or not yet resolved).

Clicking a peer row expands it inline to show all advertised bus IDs.

---

## 8. Agent pane — host context

### 8.1 Block meta at launch

`agent-model.ts` stores in block meta at launch:

```ts
agentHostSlug:   string   // host_agent_slug, or agent.slug if agent_type=="host"
agentHostName:   string   // display name of host agent
agentHostIcon:   string   // emoji
agentHostBusId:  string   // agent_bus_id of the host agent
agentHostMachine: string  // OS hostname of the host (from getApi().getHostName())
```

Derivation priority:
1. `agent.host_agent_slug` (explicit)
2. If `agent.agent_type === "host"`: `agent.slug`
3. Fallback: `localHostAgentAtom()?.slug`

### 8.2 Header host badge (container agents only)

```
╔═ Agent1 ═══════════════════════════ [🔴 AgentX] ═╗
```

Small pill in the pane-frame header, right-aligned. Only shown when
`agentMode === "container"` — host agents don't need to say they're on themselves.

Clicking the badge opens the HostPopover.

### 8.3 Footer host line

In the agent footer status strip (already has `ConnectionStatus`), add:

```
🔴 agentx · dev-workstation
```

Same visibility condition: container agents only.

---

## 9. Tool / service interaction attribution

### 9.1 Host footer on tool blocks

`ToolBlock.tsx` renders a host attribution line below the tool result:

```
┌─ bash ──────────────────────────────────────────┐
│  $ docker ps                                     │
│  CONTAINER ID   IMAGE   ...                      │
├──────────────────────────────────────────────────┤
│  🔴 AgentX · dev-workstation                     │  ← host footer
└──────────────────────────────────────────────────┘
```

**Visibility rules:**
- Container agents: **always shown** (their Docker runtime is on the host machine,
  so "which machine ran this" matters).
- Host agents: **shown on hover** only (avoids noise when everything is local).

The footer reads `agentHostIcon`, `agentHostName`, `agentHostMachine` from
`nodeModel.blockAtom()?.meta`.

### 9.2 Cross-host attribution (future)

When AgentMux gains the ability to send tool calls to a remote agent (via the
agentbus), the `agentHostMachine` in the footer will naturally show the remote
machine's hostname — no additional changes to `ToolBlock` needed. The block meta
carries the origin at launch time; a remote-execution flow would write a different
`agentHostMachine` into the meta.

---

## 10. Implementation steps

Recommended shipping order — each step is independently mergeable after A–C land.

| Step | What | Files | Est. |
|---|---|---|---|
| **A** | `host_agent_slug` column (v5 migration), `ForgeAgent` struct, `gotypes.d.ts`, backfill host agents | `migrations.rs`, `wstore.rs`, `gotypes.d.ts` | ~1h |
| **B** | `GetLocalHostAgent` backend command; extend `HostInfo` with `agent_slug/name/icon/agent_bus_ids` | `forge_handlers.rs`, `server.rs` | ~1h |
| **C** | `localHostAgentAtom` in `global.ts`; block meta gains `agentHost*` fields at launch in `agent-model.ts` | `global.ts`, `agent-model.ts` | ~1h |
| **D** | LAN: populate `LanInstance.agents` on advertisement; re-advertise on `forgeagents:changed`; inject `AGENTMUX_LAN_PEERS` env at agent launch | `lan_discovery.rs`, `agent-model.ts` | ~1.5h |
| **E** | `HostPopover`: host agent section + LAN peer agent column (hosts + containers grouped) | `HostPopover.tsx`, `StatusBar.scss` | ~1h |
| **F** | Agent pane header badge + footer host line (container agents) | `AgentHeader.tsx` or `agent-view.tsx`, `agent-view.scss` | ~45m |
| **G** | `ToolBlock` host attribution footer (always/hover rules) | `ToolBlock.tsx`, `agent-view.scss` | ~1h |

**Recommended PR grouping:**
- PR 1: A + B + C (foundation — backend + frontend atom + launch meta)
- PR 2: D + E (LAN agent advertisement + HostPopover)
- PR 3: F + G (agent pane + tool attribution)

---

## 11. Non-scope

- **Remote tool execution** (sending a tool call to an agent on another machine via
  the agentbus) — the attribution in §9.2 is a placeholder. Actual remote execution
  is a separate, larger feature.
- **Host agent auto-assignment UI** — when a user creates a container agent,
  there's no picker yet for "which host does this run on?" Default: blank; seed
  manifest sets it explicitly. Deferred to a future Forge form update.
- **Cloud agentbus protocol changes** — the cloud path (Tier 3) is unchanged.
  `agent_bus_id` semantics are unchanged. Cloud is now explicitly the fallback for
  non-LAN peers, not the primary path.
- **agentbus-client implementation** — `AGENTMUX_LAN_PEERS` is specified here as the
  interface contract; the agentbus-client internals (how it reads the env, how it
  selects Tier 2 vs Tier 3) are implementation details outside this spec's scope.
- **Live `AGENTMUX_LAN_PEERS` refresh** — when LAN peers join/leave mid-session,
  updating already-running agent processes requires either a controller restart or
  a side-channel. Mechanism TBD; static injection at launch is the v1 behaviour.
- **Multi-instance same-machine bus deduplication** — two AgentMux instances on the
  same machine advertising the same `agent_bus_id` is an existing concern, not
  addressed here.
