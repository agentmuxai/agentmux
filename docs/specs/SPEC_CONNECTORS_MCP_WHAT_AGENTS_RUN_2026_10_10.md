# SPEC: Connectors → MCP servers shows what agents run

**Date:** 2026-10-10
**Status:** active. Phase 0 (one launch path, explicit reach) shipped in
PR #4677. Phase 1 (observed servers) and Phase 2a (In use now) shipped in
PR #4683 (with #4685). Phase 2b's catalog half ("Not loaded by") is PR #4701;
its startup-card half is deferred (§4). Phase 3: ACP agents got the servers in
#4893, Codex `exec` turns get them as `-c` overrides and Gemini turns as a settings
file (§5).
**Author:** QTest@narko
**Grounded in:** [`docs/reports/REPORT_CONNECTORS_MCP_VS_AGENT_RUNTIME_2026_10_10.md`](../reports/REPORT_CONNECTORS_MCP_VS_AGENT_RUNTIME_2026_10_10.md)
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
- **Contract test.** [`scripts/cli-contract/mcp-servers-contract.mjs`](../../scripts/cli-contract/mcp-servers-contract.mjs), in the
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

**Shipped (2b, catalog half).** A catalog server that should be on a running
agent by now, bound to it or switched on for all agents, but isn't in what
that agent's CLI reports, shows "Not loaded by AgentY and Korp. Restart them
to apply." under its catalog row and in its detail.
- **Why it happens.** `.mcp.json` is written when the agent launches, so a
  server bound, or switched on, later reaches it only on its next start.
- **How.** `mcp.observed.list` gives each agent a `missing` list:
  `mcp_launch::missing_for_pane`, the catalog servers AgentMux would launch it
  with now (`effective_mcp_servers`, the container rule included) that it
  doesn't report. Its own private servers are left out, since the pane matches
  by name against catalog rows and a private server can share a catalog
  server's name. A pane that runs no agent definition (quick launch writes no
  `.mcp.json`) has none, and the pane's own `agentMode` decides the container
  rule, as it did at launch. A server the CLI reports as failed is reported, so not missing; one
  AgentMux never writes (the reserved `agentmux` name, a config that isn't
  JSON) is never missing. A test ties that to `insert_server_rows`.
- **Only agents that report.** Panes that haven't sent the CLI's startup
  report, and providers that don't send one (Phase 3), aren't listed.

**Deferred (2b, startup card).** The agent pane's startup card lists the
names in the folder's `.mcp.json`. Using the observed list instead means the
session-start hook, which knows its pane, reading it, but the CLI's startup
report may not have arrived when the hook fires; a card that sometimes shows
the real list and sometimes the file's would be worse than one that always
says what the CLI "should have read" (`startup_files.rs`). Revisit when the
order is pinned down, with a contract test of it.

## 5. Phase 3: every provider; discovery

- **ACP: shipped** (#4893, `blockcontroller/acp_mcp.rs`): the folder's `.mcp.json`
  in `session/new`.
- **Codex: shipped for host `exec` turns** (`subprocess/codex_mcp.rs`). Not in
  `$CODEX_HOME/config.toml`, which is shared by every agent on the account, but
  as `-c mcp_servers.<name>.<key>=<toml>` overrides after `exec`, built each turn
  from the `.mcp.json` in the agent's folder. A secret never goes on the command
  line (other processes can read it): the value of an `env` or `headers` entry,
  which is where a key lives (the agent's signing keys are in `agentmux`'s), is put
  in the environment Codex is launched with and the server asks Codex to forward
  it by NAME (`env_vars`, `env_http_headers`, `bearer_token_env_var`); a `${VAR}`
  that is a whole value is forwarded the same way, from the agent's environment.
  A name that two servers set to different values, or that the agent already has
  under another value, leaves the later server out (the agent's own `AGENTMUX_*`
  value wins for `agentmux`). A server with a variable in its command, an
  argument or its url, or one mixed into other text in an env or header value, is
  left out and named in the log. `agentmux` also forwards the agent's
  `AGENTMUX_*` variables by name.
  Checked against Codex 0.154.0: a real `codex exec` started the server with the
  forwarded and literal variables and nothing else. Not yet: container agents,
  the dormant app-server path (`thread/start` takes a `config` object), and sse
  servers (Codex takes none).
- **Gemini:** a Gemini turn on the host gets a per-agent settings file, written under srv's
  data folder (`gemini-mcp/<block>.json`, temporary file and rename) from the same
  `.mcp.json`, and launched with `GEMINI_CLI_SYSTEM_SETTINGS_PATH` pointing at it. Gemini
  merges its `mcpServers` with the folder's `.gemini/settings.json` and the user's, so
  nothing the person set up is replaced. stdio servers keep `command` (resolved), `args`
  and `env`; http becomes `httpUrl` and sse `url`, with literal headers. Gemini hands a
  server none of the variables named like a key or token unless the server's `env` asks
  for them, and expands `${VAR}` in a server's `env` itself, so a reference stays a
  reference, and `agentmux` asks for every `AGENTMUX_*` variable the agent was launched
  with by reference (no key is copied into the file; a value in `.mcp.json` is replaced by
  the reference). A header is expanded against the stripped environment, so a server with
  a variable in a header, or in a command, argument or url, is left out and named in the
  log. A turn that already has `GEMINI_CLI_SYSTEM_SETTINGS_PATH` set keeps it.
  Checked against Gemini CLI 0.62.0: the system file's servers load beside the project's,
  and a referenced `AGENTMUX_` key reaches the server while an unreferenced one does not.
  Not yet: container agents.

- Optional name-only discovery in the account's Claude config and Codex
  `config.toml` (D3).
- Durable agent binds: durable bindings Phases 5–6.
