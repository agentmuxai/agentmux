# SPEC: A bundle holds instructions, context, skills, Global Memory and Personal Memory — not MCP servers

**Status:** active — §5 step 1 (MCP servers out of bundles) in this PR; steps 2–4 to follow. Decisions D1–D3 taken by the operator (2026-10-07), D4–D7 on the recommendations (operator, 2026-10-08).
**Date:** 2026-10-07
**Author:** agent3 (Agent3@narko), at the operator's request
**Amends:** `SPEC_AGENT_BUNDLE_FORMAT_V0_3_2026_10_05.md` §1 ("a bundle packages an agent's instructions, MCP servers, memory and skills"), `SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md` (bundle-level MCP references), and `SPEC_RENAME_KNOWLEDGE_TO_MEMORY_2026_10_06.md` §3.6 and §5 step 2c (the Launch button adding bundle MCP servers, which this cancels).

## 1. The decision

Operator, 2026-10-07: **a bundle contains instructions, context, skills, Global Memory and Personal Memory.** MCP servers are not part of a bundle; they belong to Connectors and are bound to agents there.

| Part | Today (main `b0e8c6a5`) | After |
|---|---|---|
| Instructions (+ per-provider) | on the bundle; into the startup file at launch | unchanged |
| Context files | stored and exported, **never delivered**, not editable in the UI | delivered at launch (D4) and editable |
| Skills | references to Memory → Skills | unchanged |
| **Global Memory** | not in bundles | **entries scoped to the bundle**: named, ordered sections composed into the startup file of the agents that have the bundle, and only those (D1) |
| **Personal Memory** | not in bundles; only the per-agent `.abf` export carries an agent's memory files | **seed files**: written into an agent's Personal Memory when it gets the bundle, once, never over a file the agent already has (D2) |
| MCP servers | references to Connectors → MCP servers, plus servers private to a bundle; added at `agent.open`, exported in ABF | **removed** (D3). Existing references are dropped; ABF import ignores MCP servers |

## 2. What exists today (the scan)

**MCP servers in bundles — live, not leftover:**
- **UI:** the bundle's detail view renders `BundleMcpSection` (`frontend/app/view/bundle/bundle-manager.tsx:251`, `BundleMcpSection.tsx`, `bundle-mcp-model.ts`). It binds catalog servers and creates servers private to the bundle.
- **RPCs:** `mcp.catalog.list_for_bundle`, `bind_to_bundle`, `unbind_from_bundle` and `upsert_for_bundle` (`server/app_api/mcp.rs`, `rpc_types/mcp.rs`, `frontend/app/store/rpc-api/mcp.ts`, and the generated `McpBundleBindingData`, `McpBundleScopeData`, `CommandMcpCatalogUpsertForBundleData`).
- **Storage:** `db_bundle_mcp_ref` (`storage/mcp_servers.rs`, `managed.rs`), plus the legacy inline `db_bundles.mcp_servers` JSON column. `m0030` backfilled the ref table from that column.
- **Launch:** `effective_mcp_servers` unions the servers of every bundle in the agent's list (`managed_union_bundle_refs`, extended to the list in #4433). That covers `agent.open` only; the Launch button never did it (the gap called 2c).
- **ABF:** export writes `components.mcp_entries` (`bundle_export.rs`, `server/app_api/bundle/export.rs`); import binds them (`components.rs`); validation checks them (`bundle_validate.rs`).

**Memory and context:**
- A bundle stores no memory. A plain export warns `MEMORY_NOT_EXPORTED_WARNING`. `bundle.export_for_agent` adds the agent's native memory files to the `.abf`, and `bundle.import_for_agent` writes them into an agent, but only one with no memory yet (`server/app_api/bundle/import_for_agent.rs`).
- `db_bundles.context_files` is stored, validated and exported. No launch path reads it (`agent_open.rs` and `agent_config.rs` have no reference), and the editor shows it read-only.

## 3. Design

### 3.1 MCP servers out of bundles (D3: drop)

- **UI:** remove `BundleMcpSection`, `bundle-mcp-model.ts` and their tests, plus the detail view's MCP section and its hint text ("MCP servers and skills are managed on the bundle's own detail view…" becomes skills only).
- **RPCs:** remove the four `mcp.catalog.*_for_bundle` / `*_to_bundle` commands, their typed data and the frontend stubs. A caller still using one gets an "unknown method" error.
- **Launch:** `effective_mcp_servers` stops unioning bundle refs. `managed_union_bundle_refs` stays for skills only.
- **Data migration `m0036_drop_bundle_mcp`**, per channel and idempotent:
  - delete every `db_bundle_mcp_ref` row;
  - delete MCP servers private to a bundle (rows only a bundle referenced, `is_global = 0`, with no agent ref);
  - clear `db_bundles.mcp_servers` to `[]`;
  - log each agent that loses servers, by name and server name, so the loss is visible in the srv log.

  The table stays defined (an older build on the same channel expects it) and is no longer written. Dropping the table is a later cleanup. When the identity store can't be opened, the private servers are left in place (unreachable once their refs go) and that is logged, rather than failing the boot.
- **`m0030`** keeps only its skills half. Its MCP half carried the inline column into bundle refs, which `m0036` deletes straight after, and it called store code this removes.
- **Validation** is store-free again: with no MCP servers to check, `bundle.validate` reads only the draft.
- **Export also drops `accounts/requirements.json`.** It was inferred from the servers' `env` keys, so it goes with them, along with the redaction of secrets out of server configs. Import still reads `accounts/requirements.json` from any archive, to match accounts.
- **ABF:** export no longer writes MCP servers. Import reads them from older files and ignores them; the import preview lists them under "Not imported: MCP servers (add them in Connectors)". Validation stops checking them. See D5 for the format version.
- **Agents keep their own MCP servers.** Servers bound to an agent directly (Stash → MCP Servers, `db_agent_mcp_ref`) are untouched. What an agent loses is only what reached it through a bundle, as D3 accepts.

### 3.2 Global Memory entries on a bundle (D1)

- **Storage:** a new identity-store table `db_bundle_memory_entries (id, bundle_id, name, text, position, created_at, updated_at)`, kept with the bundles. It's additive, so there's no schema version bump.
- **At launch** (both paths: `agent.open` and `WriteAgentConfig`): for each bundle in the agent's list, after that bundle's `# [Bundle] <name>` instructions section, one section per entry, `## <entry name>` then its text, in order. Global Memory itself still comes first. The `format_agent_bundle_block` from #4433 is extended.
- **UI:** the bundle's detail view gets a **Global Memory** section: add, edit, remove and reorder entries, saved on each change, like the workspace Global section.
- **Agents:** none of the `GlobalMemory*` tools change. They act on the workspace's Global Memory, not a bundle's.
- **Out of scope for v1:** per-entry version history (D6).

### 3.3 Personal Memory seed files on a bundle (D2)

- **Storage:** a new identity-store table `db_bundle_memory_seeds (bundle_id, filename, content, updated_at)`, with filenames validated like memory files (`validate_memory_filename`).
- **Seeding:** at launch, after the memory record reconciles the agent's folder (`memory_reconcile.rs`), for each bundle in the agent's list and each seed file:
  - write it into the agent's memory record, then its folder, if the agent has no file of that name;
  - record the write in a new per-channel table, `db_agent_memory_seeded (agent_id, bundle_id, filename)`, so a file the agent later deletes or rewrites is never seeded again.

  It writes record-first, through the same path `MemoryWrite` uses, so memory still follows the agent.
- **When a file exists:** it's never overwritten and is marked seeded, so the bundle never touches it later either.
- **UI:** the bundle's detail view gets a **Personal Memory** section to author seed files (name and content), and to copy files in from an existing agent's Personal Memory.
- **ABF:** the manifest's `memory` component carries the bundle's seed files.
  - A plain export includes them, and `MEMORY_NOT_EXPORTED_WARNING` goes away.
  - Import stores them as the bundle's seeds.
  - `export_for_agent` puts the agent's memory files in as seeds.
  - `import_for_agent` becomes "import the bundle, then add it to the agent", and seeding happens at the agent's next launch. This replaces its "the agent must have no memory" rule.

### 3.4 Context files delivered (D4)

The recommendation is to write a bundle's context files into the agent's working directory at launch, at their relative paths:
- never over a file AgentMux didn't write (the same ownership check `CLAUDE.md` uses), and refreshed when AgentMux did write it;
- removed when the bundle leaves the agent's list (tracked in the same manifest the skill files use).

The editor gains add, edit and remove for them.

### 3.5 Docs and copy

- **agentmux-docs:**
  - `bundles.md` (contents table, "An agent's bundles", launch, persistence);
  - `glossary.md` (bundle);
  - `abf.md` (components, version);
  - `connectors.md` (MCP servers bind to agents, not bundles);
  - `first-agent.md`, `config.md` and `settings.md`, which say "MCP servers are configured per-agent in a bundle";
  - `memory.md` (the Bundles section).
- **agentmux:** the README's Bundles line, plus comments describing bundle MCP refs ("composable model v2", `BundleMcpSection`, `managed_union_bundle_refs`). `SPEC_RENAME_KNOWLEDGE_TO_MEMORY_2026_10_06.md` §5 marks 2c cancelled.
- **agentmux-landing:** the site's copy that describes what a bundle carries.

## 4. Decisions

- **D1 (operator):** a bundle's Global Memory is entries scoped to the bundle, composed only into its agents' startup files.
- **D2 (operator):** a bundle's Personal Memory is seed files, written once into an agent's Personal Memory, never over an existing file.
- **D3 (operator):** MCP servers leave bundles. Existing bundle references are dropped (agents lose those servers until rebound in Connectors); ABF import ignores MCP servers.
- **D4 (operator, on the recommendation):** context files are delivered into the working directory at launch, ownership-checked, and editable.
- **D5 (operator, on the recommendation):** ABF **v0.4**: components are instructions (+ per-provider), context files, skills, `memory.global` (entries) and `memory.personal` (seed files), with no `mcpServers`. Import still reads v0.1–v0.3, ignoring MCP servers. Cut in step 2, with the memory components; step 1 stays on v0.3 and stops writing `mcpServers`.
- **D6 (operator, on the recommendation):** bundle Global Memory entries and seed files get no version history in v1; history comes later through `db_bundle_versions`.
- **D7 (operator, on the recommendation):** seed at launch, after the memory record reconciles, when the memory folder is known.

## 5. Delivery

1. **agentmux PR — MCP out of bundles** (§3.1): UI, RPCs, launch, `m0036`, ABF export/import/validate, comments. With tests. This cancels 2c.
2. **agentmux PR — bundle memory** (§3.2–3.3): the two tables, launch composition and seeding, the two editor sections, ABF `memory` (and v0.4 per D5). With tests.
3. **agentmux PR — context files** (§3.4), per D4.
4. **agentmux-docs PR**, then **agentmux-landing PR** (§3.5), after (1)–(3) merge, so the docs match what ships. The ABF schema for v0.4 lives in agentmux-docs (`public/schemas/agent-bundle/`).
