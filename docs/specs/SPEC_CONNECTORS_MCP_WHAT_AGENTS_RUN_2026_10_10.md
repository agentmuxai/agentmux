# SPEC: Connectors → MCP servers shows what agents run

**Date:** 2026-10-10
**Status:** active. Phase 0 (one launch path, explicit reach) is PR #4677;
Phases 1–3 (observed servers, showing them, other providers) are next.
**Author:** QTest@narko
**Grounded in:** `docs/reports/REPORT_CONNECTORS_MCP_VS_AGENT_RUNTIME_2026_10_10.md`
(the investigation and history). File references are as of `main` at
`ab3d3e44f`.
**Related:**
- `SPEC_AGENT_MCP_SELF_HEAL_2026_10_10.md` (AgentY): health and repair of
  the `agentmux` server. It shares this spec's Phase 1 observation record,
  §3.
- `SPEC_DURABLE_BINDINGS_2026_09_10.md`: agent binds are not durable until
  its Phase 6.
- `REPORT_MCP_SKILLS_BIND_DOES_NOT_GATE_CONFIG_2026_07_28.md`: the bind gap
  this closes for MCP.

## 1. Decisions (repo owner, 2026-10-10: "follow best recommendations")

- **D1. A catalog server reaches the agents it is bound to, plus every agent
  when its "All agents" switch is on.** It no longer reaches every agent
  just by being in the catalog. This removes the implicit "global means on
  for everyone" rule, which today holds only on the `agent.open` path.
- **D2. Account connectors (claude.ai), account config servers and plugins
  are shown, read-only** (Phase 2).
- **D3. Provider config files are read for server names only, never
  configs** (Phase 3, optional).

## 2. Phase 0: one launch path, explicit reach

### 2.1 Reach

A table of its own beside `db_mcp_servers`: `db_mcp_server_reach (mcp_id,
all_agents, in_containers, updated_at)`. It sits in the identity store
(schema v12), with a copy in `objects.db` (v44) for parity. With no row, both
switches are off. A separate table rather than new columns on
`db_mcp_servers`, so nothing that copies catalog rows changes (the generic
`managed` layer, the m0016/m0031 carries, bundle import), and older builds
never see it. Deleting a server deletes its row.

**`all_agents`** gives the server to every agent.
- It is stored with the catalog in the identity store, which survives an
  upgrade, unlike an agent bind (`db_agent_mcp_ref` lives in the
  per-version `objects.db` until durable bindings Phase 6).
- So "All agents" is the reliable way to keep a server on everywhere.

**`in_containers`** also gives the server to container agents.
- A container agent shares the workdir's `.mcp.json` through its bind mount
  and has `agentmux-mcp` in its image.
- A catalog server is usually a host command: a host path, or `npx` / `uvx`
  that the image may not have. So it is opt in.
- An agent's own private (non-global) servers are always delivered as
  configured: whoever made one made it for that agent.

**Seeding.** The six seeded servers keep both switches off. Agents launched
from the picker never got them, so nothing new starts. Agents opened through
`agent.open` stop getting them unless the operator switches them on. The
changeset says so.

`Store::effective_mcp_servers(catalog, agent_id, container)` is the single
rule: own private rows, plus global rows that are bound or `all_agents`,
and for a container agent only global rows with `in_containers`.

### 2.2 One launch path

- `agent.open` already builds `.mcp.json` with `build_mcp_config_from_refs`
  over `effective_mcp_servers`.
- `writeagentconfig`, the picker path, now overlays the same effective
  servers onto the frontend's `.mcp.json` (the `agentmux` entry plus the
  agent's legacy blob) before signing keys are injected.
- That is `mcp_launch::add_to_files`, which calls
  `mcp_launch::overlay_mcp_servers`.
- Both paths put the rows in with `mcp_launch::insert_server_rows`, which
  never lets a row named `agentmux` replace the built-in entry.
- An older frontend that sends no `agent_id` gets the old content.

### 2.3 Catalog commands and pane

- **`mcp.catalog.set_reach`** `{ id, all_agents, in_containers }`. Global
  rows only, like every catalog command; it publishes `mcp:changed`.
  `mcp.catalog.upsert` does not touch the switches, so editing a server
  keeps them.
- **Connectors → MCP servers.**
  - The badge reads "All agents" when `all_agents` is on, otherwise
    "N agents" (binds).
  - The detail view has the two switches with one line each on what they
    do.
- **The Agent MCP dialog's caveat** ("every global server is currently
  applied to every agent regardless of bind state") is removed. It is false
  now, and was false for picker launches before.

### 2.4 Tests

- Storage: reach rules for private, bound, all-agents and container cases.
- `overlay_mcp_servers`: the reserved key and merge order.
- `writeagentconfig`: a bound server reaches a picker launch; an unbound one
  doesn't.
- The pane: the badge and the switches.

## 3. Phase 1: observed servers (shared with self-heal)

One record per block of the MCP servers the agent's CLI reports, built once
and read by both this spec and `SPEC_AGENT_MCP_SELF_HEAL_2026_10_10.md`
(§4.1 signals A and B).
- **Claude.** `system/init` `mcp_servers` gives a name and status. `tools`
  gives a tool count per `mcp__<server>__` prefix. Later init frames update
  it.
- **Any provider.** The `agentmux-mcp` hello: the first token-bearing
  `agents/self/keys` call per spawn.
- **Published** as an `mcp:observed` event and served by an
  `mcp.observed.list` command. Names and statuses only, never configs.
- **Who builds it** is agreed with AgentY before work starts.
- **Contract test.** The real CLI against the fake API
  (`scripts/cli-contract/`): one stub stdio server reports connected, one
  that exits reports failed.

## 4. Phase 2: show it

Connectors → MCP servers lists every observed server, grouped by source:
AgentMux catalog, built in (`agentmux`), and the agent's account or project
(claude.ai connectors, account config, plugins, a project's own `.mcp.json`
entries).
- Each row shows which agents report it and its status per agent. Agents a
  catalog server is meant to reach but which don't report it show as "not
  connected".
- Non-catalog rows are read-only and say where they come from.
- The agent pane's startup card uses the observed list when there is one.

## 5. Phase 3: every provider; discovery

- Catalog servers and `agentmux-mcp` for Codex (`config.toml`), Gemini
  settings and ACP `mcpServers`.
- Optional name-only discovery in the account's Claude config and Codex
  `config.toml` (D3).
- Durable agent binds: durable bindings Phases 5–6.
