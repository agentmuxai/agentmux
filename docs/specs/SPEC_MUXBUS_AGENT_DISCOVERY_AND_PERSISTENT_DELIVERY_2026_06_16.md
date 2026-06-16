# MuxBus — Persistent-Agent Delivery & Unified Agent Discovery

**Date:** 2026-06-16
**Status:** Draft → in implementation
**Relates to:** #1470, `SPEC_MUXBUS_DELIVERY_HIERARCHY_2026_06_15.md`, `SPEC_AGENT_CONTROL_PROTOCOL_2026_06_15.md` §6, `SPEC_SHARED_AGENT_REGISTRY_2026_05_12.md`

---

## 1. Context

The muxbus delivery hierarchy (Tier 1 same-sidecar → Tier 2 same-host peer → Tier 3 LAN → Tier 4 cloud) landed in **v0.46.0**. Testing agent↔agent messaging between two **live persistent Claude panes** in the same instance (`Clamk` → `Naki`) surfaced two related gaps:

1. **Addressability (#1470):** `SendMessage → Naki` returns **`agent not found`**. Persistent (stream-json, no-PTY) agents are never registered as muxbus targets, so they can't be reached.
2. **Discoverability:** there is **no unified, agent-accessible listing** of what exists / what's reachable across host, LAN, and WAN. An agent that wants to message a peer has no "directory" to consult — finding a target required filesystem archaeology.

Both trace to one structural fact:

## 2. Root insight — "who exists" ≠ "who's reachable"

AgentMux maintains two *separate* registries that have drifted apart:

| | **Directory** ("who exists") | **Delivery registry** ("who's reachable") |
|---|---|---|
| Backing store | SQLite `db_agent_instances` + shared `~/.agentmux/shared/agents/registry/` | `ReactiveHandler.agent_to_block` (in-memory) + Tier-2 file registry |
| Includes persistent agents? | **Yes** — they appear in "My Agents" | **No** — only PTY/`block_id`-registered agents |
| Surfaces | `listagentinstances` RPC, `MyAgentsList.tsx`, `Registry::list_active` | `/agentmux/reactive/agents`, `/api/bus/agents`, `inject_message` |

`Naki` is in the **directory** (visible in "My Agents") but absent from the **delivery registry** — hence visible-yet-unreachable. This spec closes that gap from both ends: make persistent agents *register* (Part A), and expose a *unified listing* that reconciles directory and reachability (Part B).

## 3. Current-state inventory (evidence)

- **Tier 1 (local):** `ReactiveHandler::list_agents` / `inject_message` (`backend/reactive/handler.rs`), keyed on lowercased agent name → `block_id`. Separate `MessageBus::list_agents` (`backend/messagebus.rs`), no `block_id`.
- **Host catalog:** shared cross-channel registry `Registry::list_active` (`registry/store.rs:150`; records in `registry/schema.rs`). Read-path is live for `listrecentsessions` (`server/agent_handlers.rs:2120`) though frontend RPCs still default to SQLite. SQLite live instances via `Store::instance_list` (`backend/storage/agents.rs:920`).
- **Tier 3 (LAN):** `LanInstance { instance_id, hostname, version, address, port, auth_key, agents: Vec<String>, … }` (`backend/lan_discovery.rs:26`), surfaced by `GET /api/lan-instances` (`server/mod.rs:333`). **Known gap:** `LanInstance.agents` is currently populated empty (`lan_discovery.rs:198`); peers are listed but their agent arrays aren't filled by the resolve path.
- **Tier 4 (cloud):** `CloudSubscriber.agents: HashSet<String>` (`muxbus/cloud_subscriber.rs:83`). Routing only — **no browse endpoint**, and the set has **no public accessor**. Disabled when `MUXBUS_TOKEN` is unset.

## 4. Part A — Persistent-agent delivery (closes #1470)

### 4.1 What already exists (do not rebuild)

The controller-aware **delivery** primitive is already implemented and unit-tested:

- `ReactiveHandler::inject_message` already branches on a `MessageSender` (`handler.rs:262-318`): structured success → return; `Ok(false)` → PTY fallback; `Err` → fail without PTY fallback.
- `MessageSender` wiring → `deliver_agent_message` (`main.rs:866`), which dispatches by controller: **persistent → `persistent.rs::send_user_message`** (`{type:"user"}` NDJSON on live stdin), **ACP → `send_input`**, else PTY (`blockcontroller/mod.rs:305-323`).
- Tests: `backend/reactive/tests.rs:330-442` (structured-delivery / PTY-fallback / structured-failure-no-fallback).

### 4.2 The remaining gap — registration

Persistent and ACP controllers **never call `register_agent`**, so `agent_to_block` has no entry and `inject_message` dies at the lookup (`handler.rs:227-247`) before reaching the delivery branch. Only the PTY shell controller auto-registers (`shell.rs:716-747`, gated on `AGENTMUX_AGENT_ID`).

### 4.3 Fix

Mirror the shell PTY pattern for the persistent (and ACP) controllers:

1. **Register at spawn.** In `persistent.rs::spawn_process`, once the process is running and `stdin_tx` is live (`~:498-505`), resolve the agent id (`config.env_vars["AGENTMUX_AGENT_ID"]`, fallback `agentName` block meta — both equal `agent.name`, `app_api.rs:306/316`) and call `get_global_handler().register_agent(agent_id, &self.block_id, Some(&self.tab_id))` plus the Tier-2 file-registry write (`reactive::registry::write`), exactly as `shell.rs:719-747`.
2. **Unregister at exit.** In the `process_waiter` exit arms (`persistent.rs:680-758`), call `unregister_block(&block_id)` and remove the file-registry entry, mirroring `shell.rs:962-973`.
3. **ACP (optional, same shape):** apply at the ACP controller spawn so ACP agents are addressable too.

**Addressing:** the key is the agent's display **name** (= `AGENTMUX_AGENT_ID` = `agent.name`), lowercased on register (`handler.rs:112`), so a sender addressing `"naki"`/`"Naki"` resolves regardless of controller type.

### 4.4 Acceptance

After PR1, `GET /agentmux/reactive/agents` lists the persistent agent, and `SendMessage → Naki` is **delivered into Naki's pane as a `{type:"user"}` turn** (verified live between two persistent panes).

## 5. Part B — Unified discovery endpoint

### 5.1 Endpoint

`GET /agentmux/discovery` (authed, behind `auth_middleware`; registered next to `/api/lan-instances` in `server/mod.rs`). Agents reach it via `AGENTMUX_LOCAL_URL` + `X-AuthKey: AGENTMUX_AUTH_KEY` — the same pattern as every MCP-backed endpoint.

### 5.2 Response shape

```jsonc
{
  "host": {
    "instance_id": "…",
    "version": "0.46.0",
    "instances": [
      { "instance_id": "…", "name": "…",
        "agents": [ { "name": "naki", "definition_id": "…",
                      "addressable": true, "controller_type": "persistent" } ] }
    ]
  },
  "lan": [ /* Vec<LanInstance> from get_instances() */ ],
  "wan": { "subscribed_agents": [ "…" ] }
}
```

### 5.3 Aggregation

- **Addressable set (authoritative):** `reactive_handler.list_agents()` → every entry has a live `block_id` (Tier-1; the Tier-2 file registry extends this same-host). `addressable = reactive_set.contains(name.to_lowercase())`.
- **Host catalog:** `wstore.shared_agent_registry().list_active()` (fallback `wstore.instance_list(None, None)`).
- **LAN:** `lan_discovery.get_instances()`.
- **WAN:** new `CloudSubscriber::subscribed_agents()` accessor via `get_global_subscriber()` (the one new public method required).

### 5.4 Agent access

- **Option A (default):** no new tool — agents `curl -s -H "X-AuthKey: $AGENTMUX_AUTH_KEY" "$AGENTMUX_LOCAL_URL/agentmux/discovery"` (precedent: `/agentmux/diag/sagas`).
- **Option B (optional):** a first-class `DiscoverAgents` MCP tool in `agentmux-mcp/src/main.rs` mirroring the `SendMessage` arm (GET instead of POST). *(MCP tools are Rust string consts there — `tool-catalog.json` is unrelated bundled-binary metadata.)*

## 6. Work split

| PR | Scope | Branch |
|---|---|---|
| **PR1** | Part A: persistent/ACP registration + unregistration; **closes #1470**; includes this spec | `clamk/muxbus-persistent-delivery` |
| **PR2** | Part B: `GET /agentmux/discovery` + `subscribed_agents()` accessor (+ optional MCP tool) | `clamk/muxbus-agent-discovery` |

PR1 lands first (makes `addressable` real); PR2 builds the listing and shows both persistent agents as `addressable: true`.

## 7. Non-goals

- Unifying the SQLite ↔ shared-registry **read path** (a separate migration; PR2 reads whichever is live).
- Populating `LanInstance.agents` on the advertise/resolve path (tracked gap, `lan_discovery.rs:198`).
- A WAN/cloud **browse** directory (cloud stays routing-only).

## 8. File map

```
backend/reactive/handler.rs      :227 lookup-miss · :262 controller-aware branch · :102/:145/:154 register/unregister
backend/reactive/registry.rs     :86 Tier-2 file write/remove
backend/blockcontroller/persistent.rs :369 spawn_process (REGISTER ~:498) · :680 process_waiter (UNREGISTER)
backend/blockcontroller/shell.rs :716 register pattern · :962 unregister pattern (templates)
backend/blockcontroller/mod.rs   :305 deliver_agent_message · :54 META_KEY_CONTROLLER
main.rs                          :854 input_sender · :866 message_sender wiring
server/mod.rs                    :228 route block (ADD /agentmux/discovery) · :333 handle_lan_instances (template)
backend/reactive (list_agents), registry/store.rs:150, backend/storage/agents.rs:920 (instance_list)
backend/lan_discovery.rs         :26 LanInstance · :473 get_instances
muxbus/cloud_subscriber.rs       :83 agents set (ADD subscribed_agents accessor)
agentmux-mcp/src/main.rs         :58 SEND_MESSAGE_TOOL (template for optional DiscoverAgents)
```

## 9. Open questions

1. `controller_type` in the discovery payload — derive from block meta (`META_KEY_CONTROLLER`) or omit in v1? (Lean: include when cheaply available, else `null`.)
2. ACP registration — in PR1 scope or follow-up? (Lean: include; same shape, low cost.)
3. Should the discovery endpoint dedupe a host instance that *also* appears as a LAN self-entry? (Lean: tag self by `instance_id`.)
