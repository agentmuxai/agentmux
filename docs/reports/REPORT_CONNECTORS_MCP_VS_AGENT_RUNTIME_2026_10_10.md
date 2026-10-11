# Connectors → MCP servers doesn't show what agents actually run

**Status:** active. The plan is `SPEC_CONNECTORS_MCP_WHAT_AGENTS_RUN_2026_10_10.md`; its Phase 0 ships with this report, Phases 1–3 are next.
**Date:** 2026-10-10
**Scope:** the Connectors pane's MCP servers section, how agents get MCP
servers at launch, and what AgentMux can learn about the servers an agent
actually has.

## 1. Summary

The MCP servers section lists AgentMux's MCP catalog and badges each row
"N agents". On a typical install it shows the six seeded servers (git,
fetch, sequential-thinking, memory, playwright, context7), each with
"0 agents", while every agent is plainly using MCP. Three separate gaps
cause this. None of them is a regression; the pane has never been
connected to what agents run.

1. **The badge counts explicit binds only.** `bound_count` counts
   `db_agent_mcp_ref` rows. The seeded servers are global, nobody binds
   them, and on this machine every catalog has six global rows and zero
   refs. So the badge says 0.
2. **The catalog doesn't reach most agents anyway.** An agent launched
   from the picker (the normal path) gets a `.mcp.json` with only the
   built-in `agentmux` server and the agent's legacy MCP blob. The catalog
   servers are added only on the `agent.open` path (App API, the OpenAgent
   tool, layouts). So for a picker-launched agent "0 agents" is accidentally
   true, and the seed spec's "every global server is injected into every
   agent" is false.
3. **What agents really use isn't shown at all.** That is the built-in
   `agentmux` server (every agent has it; it isn't a catalog row) plus
   whatever the provider loads on its own: the Claude account's claude.ai
   connectors, user-scope servers and plugins in the account's
   `CLAUDE_CONFIG_DIR`, a project's own `.mcp.json` entries, and Codex's
   `config.toml`. AgentMux never reads any of it, and never reads the list
   of connected servers Claude reports at startup.

The plan in §5 makes the pane show **observed** servers (what each running
agent reports as connected), joined to the catalog, and makes both launch
paths deliver the catalog the same way.

## 2. How it works today

### 2.1 The pane

- `frontend/app/view/connectors/connectors.tsx` mounts `McpManager`
  (`frontend/app/view/mcp/mcp-manager.tsx`), moved unchanged from the
  Armory (`SPEC_RETIRE_ARMORY_CONNECTORS_AND_KNOWLEDGE_PANES_2026_10_05.md`).
- `McpCatalogModel` (`mcp-model.ts`) calls `mcp.catalog.list`
  (`crates/srv/src/server/app_api/mcp.rs`), which returns global rows with
  `bound_count` (`storage/mcp_servers.rs` `mcp_server_list_global`,
  `storage/managed.rs` `managed_list_global` / `managed_bound_counts`).
- The status pill (`McpStatusPill`, `frontend/app/store/mcp-capabilities.ts`,
  `backend/mcp_probe.rs`) starts its own copy of a catalog server and runs
  `initialize` + `tools/list`. It tests the config. It says nothing about
  whether any agent has the server.

### 2.2 What an agent is launched with

AgentMux writes one MCP file, `.mcp.json` in the agent's working
directory, through `write_mcp_json_respecting_user_servers`
(`backend/agent_config.rs`). That merge keeps the user's own entries and
replaces the ones AgentMux wrote last time (tracked in
`.claude/.agentmux-managed-mcp-servers.json`). Two paths build its content:

| Launch path | Builder | Servers |
|---|---|---|
| Picker, quick fork (`agent-model.ts` → `buildConfigFiles` → `buildMcpConfig` in `agent-config-builder.ts`, written by `writeagentconfig` in `server/editor_handlers.rs`) | frontend | `agentmux` + legacy per-agent blob |
| `agent.open` (`server/app_api/agent_open.rs`) | `build_mcp_config_from_refs` | `agentmux` + legacy blob + bound + **all global** catalog rows (`effective_mcp_servers`) |

No MCP flag is passed (`--mcp-config` / `--strict-mcp-config` appear only
in the ambient side call, `ambient/cli.rs`). Claude Code reads `.mcp.json`
as a project file. Nothing configures its project-server approval
(`enableAllProjectMcpServers` / `enabledMcpjsonServers`); it works because
the agent runs headless.

Per provider:

- **Claude Code** reads `.mcp.json`.
- **Codex** gets no AgentMux MCP (`app_server_controller.rs`: "codex is not
  given agentmux-mcp"); its servers come only from `$CODEX_HOME/config.toml`.
- **Gemini, Qwen, Kimi, Antigravity** have no MCP writer; the `.mcp.json`
  in their working directory is never read.
- **ACP agents** (openclaw, pi, copilot) are sent `"mcpServers": []`
  (`blockcontroller/acp.rs`).
- **Container agents** see the same `.mcp.json` through the bind mount.

### 2.3 What the provider loads on its own

`CLAUDE_CONFIG_DIR` is per account, shared by every agent bound to it
(`~/.agentmux/shared/identities/<account>/claude`,
`identity_auth_dirs.rs`, `data_paths.rs`). The user's own `~/.claude.json`
is therefore isolated away, but that directory's user-scope `mcpServers`,
its plugins, and the account's claude.ai connectors still load. They
appear to the agent as `mcp__claude_ai_*` tools and so on. `CODEX_HOME` is
likewise shared per account. AgentMux writes none of this and reads none
of it.

### 2.4 What AgentMux already receives but drops

- **Claude's `system/init` frame** carries `mcp_servers` (each with a name
  and a status: connected, failed, needs-auth, pending) and `tools` (which
  include every `mcp__<server>__<tool>`).
  - srv reads init only for turn state, the CLI version and the session id
    (`blockcontroller/persistent/stdout_reader.rs`,
    `subprocess/host_spawn.rs`).
  - The translator ignores system frames except `compact_boundary`
    (`agents/translator/claude.rs`).
  - The raw line is still saved to the block's output file.

  This is the one authoritative list of what an agent actually has, for
  every source at once, and it costs nothing to read.
- **agentmux-mcp's startup call.** It calls `GET /agentmux/agents/self/keys`
  with the agent's token at startup (`crates/mcp/src/main.rs`,
  `server/agent_self_keys.rs`). srv records nothing for it, but it means
  "this agent's agentmux server started", for any provider that runs it.
- **The startup-files card** ("MCP servers: agentmux",
  `backend/startup_files.rs` `mcp_item`) lists the key names in
  `cwd/.mcp.json`. Its own doc says it shows what the CLI *should* have
  read, not proof. It misses account servers, plugins and connectors, and
  doesn't show failures.

### 2.5 Evidence on this machine

- Every AgentMux catalog database found holds the same six seeded global
  servers and zero `db_agent_mcp_ref` rows.
- A picker-launched Claude agent's `.mcp.json` contains only `agentmux`.
- That agent's live tool list has the `agentmux` tools and claude.ai
  connector tools (Docs, Gmail, Calendar, Drive), and none of the six
  catalog servers.
- The pane shows the six it doesn't have and neither of the two it does.

## 3. History

- **`SPEC_V1_MCP_SKILLS_PRIMITIVES_2026_06_30.md`** created
  `db_mcp_servers` + `db_agent_mcp_ref` and asked for "used by N agents"
  (§8). Built in #1877, #1943, #1946, #1948, #1962, #2023 (gaps of #1960).
- **`SPEC_ARMORY_MCP_SERVER_DEFAULT_SEED_CATALOG_2026_07_13.md`** (#2147)
  seeded the six servers as global, stating every global server is
  injected into every agent.
- **`REPORT_ARMORY_SKILLS_MARKDOWN_AND_BIND_BUG_2026_07_27.md`**: Bind
  wrote nothing; fixed in #2317.
- **`REPORT_MCP_SKILLS_BIND_DOES_NOT_GATE_CONFIG_2026_07_28.md`** (#2329)
  found bind state has no effect on launch, and recommended backfilling a
  ref for every (agent, global server) pair, then building launch config
  from refs. **Not built**; only a caveat shipped (`AgentMcpModal.tsx`: "every
  global server is currently applied to every agent regardless of bind
  state"). As §2.2 shows, that caveat is itself wrong for picker launches.
- **`SPEC_MCP_INTEGRATION_PARITY_ABLETON_PILOT_2026_07_08.md`** (active):
  probe (#2030) and catalog picker (#2064) shipped; secrets/OAuth
  (Phase C) and tool introspection (Phase D) are open.
- **`SPEC_DURABLE_BINDINGS_2026_09_10.md`** moved the catalog to the
  identity store (#3180–#3183), but `db_agent_mcp_ref` stays in the
  per-version `objects.db`, so explicit binds don't survive an upgrade
  (Phases 5–6 open). That's a second reason binds read 0.
- **`REPORT_CLAUDE_CONFIG_DIR_ISOLATION_EVIDENCE_2026_09_01.md`** checked
  MCP servers and plugins in the account directory and found none on that
  machine. It asked for a re-check once someone has them; not done.
- **`SPEC_CODEX_PROVIDER_INTEGRATION_2026_08_08.md`** §10.4 planned a
  one-way translation of AgentMux servers into Codex TOML; not built.
  **`SPEC_ACP_CLIENT_CONFORMANCE_2026_10_07.md`** leaves agentmux-mcp for
  ACP as a follow-up.

No doc proposes reading what an agent actually connected, or discovering
servers the provider loads itself. No commit was ever reverted in this area.

## 4. What the pane should answer

1. Which MCP servers are my agents using right now, and is each one working?
2. Which agents have a given server, and where does it come from: AgentMux's
   catalog, built in, the agent's account, its project, or a plugin?
3. For the servers AgentMux manages: who gets them, and how do I change that?

Question 1 needs observation (§5, Phase 1). Question 3 needs one launch
path with a clear rule (§5, Phase 0).

## 5. Plan

### Phase 0: one launch path, one rule (small, do first)

- **Build `.mcp.json` in srv for both paths.**
  - `writeagentconfig` builds it with `build_mcp_config_from_refs` and
    `effective_mcp_servers`, as `agent.open` does.
  - The frontend `buildMcpConfig` stops building MCP content.
  - Both builders refuse a user entry named `agentmux`. Today
    `build_mcp_config` and the TS builder let one override it.
- **Decide what "global" means** (decision D1, below). Recommended: global
  means *in the catalog, available to bind*, not *on for every agent*.
  - Launch config comes from binds, as the 07-28 report recommended.
  - Each server gets an explicit "All agents" switch, which is the honest
    form of today's implicit global.
  - With that, `bound_count` (or "All agents") is exactly who gets the
    server.
  - If instead globals stay on for everyone, the picker path starts six
    extra servers for every agent (npx downloads, startup time, failures).
    That change needs its own announcement.
- **Remove or correct the `AgentMcpModal` caveat.**
- **Tests.** One srv unit test per path, asserting the same `.mcp.json`
  for the same agent. A test that a bound server reaches a picker launch.

### Phase 1: observe what each agent has

- **Claude.** In `stdout_reader.rs`, where init is already handled, read
  `mcp_servers` and count `tools` by `mcp__<server>__` prefix.
  - Keep the latest per block, in memory, with its time.
  - Update a server's status on later init frames.
  - Publish an `mcp:observed` event (`{ block_id, agent_id, provider,
    servers: [{ name, status, tool_count }], at }`) and serve the current
    set through a new `mcp.observed.list` command.
  - Names only, never configs: the init frame carries no config, so no
    secret can leak through this.
- **agentmux server, any provider.** Stamp the first token-bearing
  `agents/self/keys` call per agent as "agentmux connected". This covers
  providers without an init frame once they get agentmux-mcp (Phase 3).
- **Codex and Gemini.** Find what their app-server / stream-json output
  reports about MCP servers. Unknown today; investigate before building.
- **Classify each observed name by source.**
  - A catalog row of that name is *catalog*.
  - `agentmux` is *built in*.
  - Names with the claude.ai prefix are *account connector*.
  - Plugin-scoped names are *plugin*.
  - An entry in the agent's `.mcp.json` that AgentMux didn't write is
    *project*.
  - Anything else is *account config*.
  - The exact name shapes for connectors and plugins must be confirmed
    against the pinned CLI before relying on them.
- **Contract test.** Extend `scripts/cli-contract/` (the fake-API harness
  used for the bashwrap hook, no model cost): launch the real CLI with a
  stub stdio MCP server in `.mcp.json` and assert init reports it
  `connected`. A second stub that exits should report `failed`. This pins
  the field against CLI upgrades.

### Phase 2: show it

- **Connectors → MCP servers lists every server any agent has, grouped by
  source:** AgentMux catalog, Built in, From the agent's account or
  project.
  - Each row shows the agents that have it, with a status dot per agent
    (connected, failed, needs sign-in) and tool count.
  - Each row also shows the agents it's bound to but which don't report
    it: bound, but not connected.
  - Catalog rows keep edit, bind and the "All agents" switch. Other rows
    are read-only and say where they come from.
- **The agent pane's startup card uses the observed list** when there is
  one, and falls back to the `.mcp.json` names with its existing "should
  have read" wording.
- **Tower's agent detail can show the same list** later. Not part of this
  plan.

### Phase 3: reach every provider; discovery (each its own spec)

- Deliver catalog servers and agentmux-mcp to Codex (`config.toml`
  translation, Codex spec §10.4), Gemini settings, and ACP `mcpServers`.
- Optional discovery: read the account directory's user-scope `mcpServers`
  and Codex `config.toml`, names only, to list configured servers that no
  running agent reports. Importing one into the catalog needs secrets
  stored as Account references first (parity spec Phase C).
- Durable bindings Phases 5–6, so binds survive upgrades.

## 6. Decisions for the operator

- **D1. What global means.** Recommended: available, opt in per agent, with
  an explicit "All agents" switch. The alternative is on for every agent,
  which makes picker-launched agents start the six seeded servers.
- **D2. Showing account connectors.** Recommended: show them, read-only,
  grouped under the account, since they're on for every agent using that
  account and the user can't see that anywhere today.
- **D3. Reading provider config files (Phase 3 discovery).** Recommended:
  names only, never configs, and only after Phase 1 shows the gap is worth
  it.

## 7. Risks

- `system/init`'s `mcp_servers` is a CLI output field, not a documented
  contract. The Phase 1 contract test catches a rename on upgrade.
- An agent that hasn't started a turn since srv started has no
  observation. The pane says "not seen since AgentMux started" rather than
  guessing.
- Phase 0 changes launch content for picker-launched agents that had a
  legacy MCP blob. The merge order (blob, then refs) is unchanged, and
  `agent.open` already uses it.
