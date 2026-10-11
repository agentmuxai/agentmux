# SPEC: Connectors → MCP servers shows what agents run

**Date:** 2026-10-10
**Status:** active. Phase 0 (one launch path, explicit reach) shipped in
PR #4677. Phase 1 (observed servers) is PR #4683. Phase 2a (In use now) is
in this PR; Phase 2b and Phase 3 (other providers) are next.
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
all_agents, in_containers, updated_at)`. It sits in the identity store,
with a copy in `objects.db` (v44) for parity. The identity store's schema
version is **not** bumped. That file is shared by every AgentMux on the
computer, and a build refuses a store stamped with a newer version, so a
bump would lock every older AgentMux out as soon as one newer build starts.
A dev build of this branch, when it still bumped to v12, did exactly that;
the store was set back to v11. An older build ignores this table, so it
needs no lock. With no row, both
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

## 3. Phase 1: observed servers

`backend/mcp_observed.rs` keeps, per pane, the MCP servers the agent's CLI
says it has, from every source.

- **Claude.** The `system/init` frame's `mcp_servers` gives each server's
  name, status (`connected`, `failed`, `needs-auth`, `pending`) and the CLI's
  own source label:
  - `project` (`.mcp.json`, where AgentMux's servers go);
  - `user` (the account's config);
  - `local`;
  - `dynamic` (`--mcp-config`, plugins);
  - `managed`;
  - `enterprise`;
  - `claudeai` (claude.ai connectors, named "claude.ai …").

  The frame's `tools` gives a tool count per server: the CLI names them
  `mcp__<name>__<tool>`, with every character of the name other than letters,
  digits, `_` and `-` made `_`. Each later init frame replaces the record.
- **Where it hooks in.** The init handling in
  `persistent/stdout_reader.rs`, beside self-heal's
  `observe_mcp_frame` (`SPEC_AGENT_MCP_SELF_HEAL_2026_10_10.md`), which
  reads only the `agentmux` entry for its repair ladder. Each reads the same
  frame for its own purpose; neither changes the other.
- **Published** as an `mcp:observed` event (on a change only, since a
  resumed turn's init repeats the list).
- **Served** by `mcp.observed.list`, which names AgentMux's own sources:
  `builtin` for `agentmux`, `catalog` for a catalog server of that name,
  otherwise the CLI's label. It gives each pane's agent id and Tower label,
  and drops panes that no longer exist.
- **Names, statuses and counts only.** The frame carries no config.
- **Contract test.** `scripts/cli-contract/mcp-servers-contract.mjs`, in the
  `cli-hook-contract` workflow, runs the real CLI against the fake API with
  servers in `.mcp.json`, the account config and `--mcp-config`, one of them
  exiting at once. It checks each one's status, source and tool names.
- **Not in Phase 1.** The `agentmux-mcp` hello, for providers without an init
  frame: none of them gets `agentmux-mcp` yet (Phase 3). Live status changes
  mid-session: self-heal's `mcp_status` requests could feed the record later.

## 4. Phase 2: show it

**Shipped (2a).** Connectors → MCP servers opens with **In use now**, above
the catalog (`mcp-in-use.tsx`, `mcp-observed.ts`). It has one row per server
any running agent reports, and refreshes on `mcp:observed` and `mcp:changed`.
- **Order.** AgentMux's own first (built in, catalog), then claude.ai
  connectors, account config, project, local, added at launch, managed.
- **Each row** gives the server's source in words and a summary ("2 agents,
  1 not connected"). Under it is each reporting agent with a status dot:
  connected, failed, needs sign-in, starting.
- **A server two agents get from different sources** is one row per source.
- **The rows are read-only.** The catalog below stays the place to change a
  catalog server.
- **Empty state.** With no report since srv started, the list says so.

**Next (2b).**
- **Bound but not connected.** Agents a catalog server is meant to reach but
  which don't report it, shown as "not connected" on its catalog row. This
  needs the reach rule (§2.1) evaluated per running agent.
- **The agent pane's startup card** uses the observed list when there is one.

## 5. Phase 3: every provider; discovery

- Catalog servers and `agentmux-mcp` for Codex (`config.toml`), Gemini
  settings and ACP `mcpServers`.
- Optional name-only discovery in the account's Claude config and Codex
  `config.toml` (D3).
- Durable agent binds: durable bindings Phases 5–6.
