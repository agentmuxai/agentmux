# Armory Architecture

**Date:** 2026-07-20
**Status:** Reference document
**Scope:** Every current Armory entity — schema, RPC surface, UI, and how each
binds to an agent. Written to give binding-mechanism decisions (starting with
Startup Instructions) a consistent basis instead of being made ad hoc.

---

## 0. What "Armory" is, and what it deliberately isn't

Armory (hamburger menu → Armory, `viewType: "armory"`,
`frontend/app/view/armory/armory-view.tsx`) hosts **shared, reusable resources**:
credentials, reusable instruction blocks, MCP server configs, and skills. Its
tab list is a flat array, not a switch/route registry:

```
frontend/app/view/armory/armory-view.tsx:15-21
const RAIL: { id: ArmorySection; label: string; icon: string }[] = [
    { id: "accounts", label: "Accounts",    icon: "key" },
    { id: "brain",    label: "Memories",    icon: "brain" },
    { id: "skills",   label: "Skills",      icon: "wand-magic-sparkles" },
    { id: "mcp",      label: "MCP Servers", icon: "plug" },
    { id: "memories", label: "Bundles",     icon: "layer-group" },
];
```

All five tab components are always mounted; only visibility toggles via a CSS
class keyed off a `section()` signal (armory-view.tsx:50-69) — instant
switching, no re-fetch, cross-tab consistency via WPS `*:changed` events
instead of remount-on-select. Adding a new Armory entity means: add a `RAIL`
entry + `ArmorySection` literal + an always-mounted `<XManager/>` pane. There is
no deeper per-tab routing indirection than this.

**What deliberately does NOT live in Armory:** per-agent-instance data. Armory
used to have an "Identities" tab (`AgentIdentitiesPanel`) showing which account
each agent was bound to — this was **removed** in Phase 5
(`docs/specs/SPEC_ARMORY_PHASE5_CONSOLIDATION_AND_SKILL_SEEDING_2026_07_13.md`)
specifically to move per-agent data out of Armory. The read-only panel that
used to render there was extracted and relocated to the agent pane's own
`view: "identity"` tab (`AgentIdentityLinksPanel`,
`frontend/app/view/identity/identity-pane-view.tsx`) — Armory kept nothing
per-agent, only the reusable resources themselves. This precedent matters: any
new Armory entity should be a *reusable resource*, with per-agent *bindings* to
it living outside Armory (agent-pane tabs), the same shape all four entities
below already follow.

`CLAUDE.md`'s "Not widgets" table is stale here — it still lists an Armory
"Identities" tab. Corrected as part of this doc's companion PR (see §6).

---

## 1. Accounts

**Table:** `db_accounts` (`agentmux-srv/src/backend/storage/identities.rs`).
`IdentityAccount { id, name, provider, kind, display_name, secret_ref, context,
status }`, `secret_ref` an enum: `Env | SecretsManager | PlaintextDev |
OAuthConfigDir | Keychain`.

**RPC:** `agentmux-srv/src/server/agent_handlers/identity.rs` — account CRUD
(`listidentityaccounts`, `upsertidentityaccount`, etc.) plus
`linkagentidentity`/`unlinkagentidentity`/`listagentidentities`/
`listallagentidentities` for the binding layer (below).

**UI:** Armory "Accounts" tab, `frontend/app/view/accounts/accounts-manager.tsx`.

**Binding mechanism:** a single FK **per provider**, not a list —
`db_agent_identity_links(agent_id, provider) → account_id`, composite-keyed so
each agent has at most one bound account per provider
(`agentmux-srv/src/backend/storage/identities.rs`, `agent_identity_link()`).
Written **only** by the agent-launch flow
(`frontend/app/view/agent/components/AgentLaunchModal.tsx` →
`frontend/app/view/agent/agent-model.ts:611-619` →
`linkagentidentity` RPC) — there is no write path from any Armory or
agent-pane UI; both are read-only views of this table. This is the pattern
the removed Armory "Identities" tab used to show and the agent pane's own
Identity tab (`AgentIdentityLinksPanel`) shows today.

**Materialization at spawn:** `identity/resolver.rs`'s
`resolve_bindings_for_instance` reads `db_agent_identity_links` exclusively (no
bundle/fallback logic) and resolves bound accounts into env vars — e.g. a
bound GitHub account becomes `GITHUB_TOKEN`/`GH_TOKEN` in the spawned process.

**Known inconsistency (fixed alongside this doc, §6):** `AgentDefinition` also
carries a legacy `accounts` JSON blob column
(`agentmux-srv/src/backend/storage/agents.rs:55-64`, doc comment: "An older v6
comment called this deprecated in favour of `db_agent_identity_links`; that
migration never completed"), written by `AgentIdentityPanel.tsx` via
`updateagent`. Session Context's "Assigned Accounts" section
(`buildStartupPayload.ts`) was reading *this* blob instead of
`db_agent_identity_links` — meaning it could show accounts that don't match
what the agent actually launched with.

---

## 2. Bundles (Memories)

**Table:** `db_bundles` (renamed from `db_memory_bundles`;
`agentmux-srv/src/backend/storage/migrations.rs:268-285`). Columns: `id, name,
description, is_blank, is_global, provider, model, instructions,
context_files, mcp_servers, skills, sort_order, created_at, updated_at`. Method
names on `Store` stayed `bundle_memory_*` after the table rename — an
intentional decoupling of Rust API naming from SQL table naming.

**RPC:** two parallel surfaces, both bottoming out in the same
`Store::bundle_memory_*` methods and the same `memories:changed` event:
- UI-facing (`agentmux-srv/src/server/agent_handlers/memory.rs`):
  `listmemories`, `getmemory`, `upsertmemory`, `deletememory`,
  `reorderglobalbrain`.
- App-API/programmatic (`agentmux-srv/src/server/app_api/bundle.rs`):
  `bundle.list/get/upsert/delete/self.get` (+ deprecated `preset.*` aliases),
  with extra guardrails — `bundle.upsert` forbids mutating `id=="blank"`,
  seeded (`"seed-"` prefix), or existing `is_global` bundles, and strips
  caller-supplied `is_global`/`sort_order` escalation.

**UI:** Armory "Bundles" tab, `frontend/app/view/memory/memory-manager.tsx`,
built on the shared `PrimitiveListDetail` primitive
(`frontend/app/element/primitive-list-detail.tsx`) — a flat list-or-detail
stack shared with Skills and MCP Servers (§3, §4). The edit form has **no**
`is_global` toggle; that flag is preserved-but-not-editable from this tab
(promoted/demoted elsewhere, e.g. seeding or the separate "Memories"/brain tab,
`GlobalBrainManager`). `sort_order` is likewise invisible here — owned
exclusively by `reorderglobalbrain`.

**Binding mechanism: the weakest of the four, but not inert.** `is_global=1`
blanket-injects a bundle's `instructions` into **every** agent's CLAUDE.md at
launch — there is no per-agent opt-in/opt-out and no ref/join table for that
path. Separately, a per-agent `db_agent_instances.memory_id` field (populated
by the launch modal's "Memory" bundle picker) selects one non-global bundle,
but it is **pull-only, not push/auto-injected**: it's read by the `bundle.self.get`
App-API RPC (`agentmux-srv/src/server/app_api/mod.rs:579-600`, resolves the
instance's bound bundle for a caller to fetch on demand) and surfaced as a
`memory_name` for display in a couple of agent-handler paths
(`agentmux-srv/src/server/agent_handlers/identity.rs`,
`agentmux-srv/src/server/agent_handlers/session.rs`) — but nothing reads
`memory_id` at `write_agent_config_files()` time to inject that bundle's
`instructions` into CLAUDE.md or Session Context the way `is_global` bundles
get injected automatically. So a non-global bundle bound via `memory_id` is
consumable (an agent/caller can explicitly pull it), just not automatically
materialized into the agent's own context the way every other entity's
binding is.

**Materialization at spawn:** `format_global_brain_block()`
(`agentmux-srv/src/backend/storage/memory_bundles.rs:74-87`) filters to
`is_global` bundles with non-empty `instructions`, formats each as `# [Workspace]
<name>` + body, joins them, and injects the result into the "memory" section
of every agent's generated CLAUDE.md
(`agentmux-srv/src/server/app_api/agent_open.rs:534-548`, also mirrored for
the editor-preview path in `editor_handlers.rs`'s `inject_global_bundles`).

---

## 3. Skills

**Two parallel storage layers coexist** (legacy + v1 primitive — v1 is the
one Armory's UI drives):

- Legacy: `AgentSkill` / `db_agent_skills` — per-agent-owned rows only, no
  global concept, FK `agent_id → db_agent_definitions ON DELETE CASCADE`.
- v1: `Skill` / `db_skills` (`agentmux-srv/src/backend/storage/skills.rs:206-219`)
  — `{ id, name, trigger, skill_type, description, content, is_global,
  created_at, updated_at }`. Binding is a **separate join table**,
  `db_agent_skills_ref(agent_id, skill_id)` composite PK, FK to both
  `db_agent_definitions` and `db_skills`, `ON DELETE CASCADE`.

**RPC:** App-API surface (`agentmux-srv/src/server/app_api/skill.rs`) —
per-agent: `skill.list/get/upsert/delete` + **`skill.bind`/`skill.unbind`**
(`{agent_id, skill_id}`); catalog (window-scoped, no agent context):
`skill.catalog.list/upsert/delete`. `skill.upsert` always creates non-global
rows and forbids mutating a global one; `skill.catalog.upsert` always creates
`is_global: true` rows and cannot promote a private skill. All mutations
publish `skills:changed`.

**UI:** Armory "Skills" tab (`frontend/app/view/skill/skill-manager.tsx`,
`SkillCatalogModel`) plus an agent-setup-modal tab
(`frontend/app/view/agent/components/AgentSkillsModal.tsx`, `AgentSkillModel`)
scoped to one agent — global skills are read-only there (edit/delete blocked
client-side, "managed in the Armory"), only Bind/Unbind exposed. Both use
`PrimitiveListDetail`.

**Binding mechanism — the most complete pattern of the four:** `is_global=1`
makes a skill automatically visible/usable by every agent with **no** ref row
needed (`skill_list`'s `WHERE is_global=1 OR id IN (refs for this agent)`).
Non-global skills require an explicit `db_agent_skills_ref` row, created via
either entry point:
- `AgentSkillsModal` (agent's own setup): creating a new skill there makes a
  private one already implicitly bound to that agent; binding/unbinding a
  global skill toggles the ref row.
- `SkillManager` (Armory catalog): `bindToAgent()` picks any agent from a
  dropdown — but the backend only permits binding **global** skills (or
  already-bound ones) this way; a private skill can't be attached to a second
  agent through this path. Every non-global skill is permanently scoped to
  its creating agent.

**Materialization at spawn:** `write_agent_config_files()`
(`agentmux-srv/src/server/app_api/agent_open.rs:487-615`) resolves
`skill_list(agent_id)` (own refs + globals) into `visible_skills`, then
`build_config_files()` (`agentmux-srv/src/backend/agent_config.rs:28-149`)
(a) appends a `# Available Skills` index into the generated CLAUDE.md, and (b)
writes each skill with a trigger+content as its own slash-command file
`.claude/commands/<trigger>.md`.

---

## 4. MCP Servers

**Table:** `McpServer` / `db_mcp_servers`
(`agentmux-srv/src/backend/storage/mcp_servers.rs:16-26`) — `{ id, name,
transport, config: JSON string, is_global, created_at, updated_at }`. Same
binding join-table shape as Skills: `db_agent_mcp_ref(agent_id, mcp_id)`
composite PK, `ON DELETE CASCADE` on both sides.

**RPC:** App-API (`agentmux-srv/src/server/app_api/mcp.rs`) — per-agent
`mcp.list/get/upsert/delete` + **`mcp.bind`/`mcp.unbind`** + `mcp.probe` (live
MCP handshake check); catalog `mcp.catalog.list/upsert/delete` +
`mcp.catalog.probe`. `mcp.upsert` forbids the reserved name `"agentmux"` and
mutating global rows. All mutations publish `mcp:changed`.

**UI:** Armory "MCP Servers" tab (`frontend/app/view/mcp/mcp-manager.tsx`,
`McpCatalogModel`) — adds a live connectivity status pill
(`McpStatusPill`/`watchMcpCapability`) and a "+ Browse catalog" preset picker
(`McpCatalogPicker`) beyond the base pattern — plus an agent-setup-modal tab
(`AgentMcpModal.tsx`, `AgentMcpModel`). Both `PrimitiveListDetail`.

**Binding mechanism:** identical shape to Skills — `is_global` auto-applies to
every agent with no ref row; non-global servers need an explicit
`db_agent_mcp_ref` row; the same restriction that only global servers can be
bound to a second agent via the catalog picker.

**Materialization at spawn:** also in `write_agent_config_files`
(`agent_open.rs:583-615`). `build_mcp_config_from_refs()`
(`agent_config.rs:320-393`) builds the final `.mcp.json`: always injects a
synthetic `mcpServers.agentmux` entry (the built-in server every agent gets
regardless of bindings), merges the legacy free-form `.mcp.json` blob only
when the agent has no own refs, then merges each bound server's `config` keyed
by name. Unlike Skills, MCP servers are **not** referenced from CLAUDE.md at
all — `.mcp.json` is their only materialization path.

---

## 5. Startup Instructions (not currently an Armory entity)

The "Custom Startup Instructions" section of Session Context
(`frontend/app/view/agent/startup/buildStartupPayload.ts`) is sourced from
`content_type: "startup"` in `db_agent_content` — a generic per-agent-definition
key/value blob table (also holding `soul`/`agentmd`/`mcp`/`env`). This is a
**freeform blob with no binding mechanism and no live authoring UI anywhere**:

- The one frontend surface that would generically edit these blobs
  (`frontend/app/view/agent-def/` — `AgentDefViewModel`, `CONTENT_TABS =
  ["soul","agentmd","mcp","env"]`, notably not including `"startup"`,
  `AgentContentSection.tsx`/`ContentEditor.tsx`) is **orphaned dead code**: not
  registered in `frontend/app/block/block-registry.ts`, no barrel/index file,
  zero external imports anywhere in `frontend/app`.
- Today, setting startup instructions requires a raw RPC call or the seed
  manifest — there is no UI path at all.

### Options for binding Startup Instructions

Two concrete shapes, informed by the comparison above, for a follow-up
decision (not decided in this doc):

**(a) Reuse the Skills/MCP ref-table pattern.** Add `db_agent_startup_ref`
(or extend Bundles with the same shape) — `is_global` + per-agent join table +
`startup.bind`/`startup.unbind`. Matches the most mature, twice-proven
precedent in the codebase and keeps all "attachable resource" entities
structurally consistent. Bigger lift: needs a new table, new RPC surface, and
either a new Armory tab or folding into the Bundles tab with a "use as
startup" toggle per bundle.

**(b) Single FK on `AgentDefinition`.** Add `AgentDefinition.startup_bundle_id`
pointing at one existing Bundle; `buildStartupPayload.ts` fetches that
bundle's `instructions` for the "Startup Instructions" section instead of the
current freeform blob. Simpler: an agent only ever needs *one* startup-
instructions source (unlike skills/MCP, where an agent legitimately wants many
bound at once), so a multi-row ref table would be unused capacity. Smaller
diff: one column + one selector field (e.g. added to the agent's own setup
modal), no new RPC verbs, no new table.

Recommendation is deferred to the follow-up implementation step — this doc's
job is to make sure that decision is made with the full picture above rather
than in isolation.

---

## 6. Corrections tracked alongside this doc

- `CLAUDE.md`'s "Not widgets" table described an Armory "Identities" tab that
  no longer exists (removed in Phase 5, §0 above) — corrected in this same PR
  to describe the current location (agent pane's own Identity tab).
- `buildStartupPayload.ts`'s "Assigned Accounts" section read the stale
  `AgentDefinition.accounts` blob (§1) instead of `db_agent_identity_links` —
  fixed as a separate, standalone PR (#2239, merged) so it could be reviewed
  and verified independently of this doc.
