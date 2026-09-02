# SPEC: v1 — MCP Servers & Skills as first-class primitives

**Date:** 2026-06-30
**Status:** Draft — implementation spec
**Author:** AgentX
**Parent:** `PROPOSAL_COMPOSABLE_AGENT_MODEL_2026_06_30.md` (the composable Armory model — Bundle · Account · Memory · MCP Server · Skill · Brief)
**Related code:** `agentmux-srv/src/backend/storage/{skills.rs,content.rs,agents.rs}`, `backend/agent_config.rs`, `server/app_api.rs` (`write_agent_config_files`, `preset.*`/`memory.*` handlers), `backend/rpc_types.rs`.

---

## 1. Goal & scope

Break **MCP Servers** and **Skills** out of the per-agent inline storage into
**standalone, shareable, referenceable primitives** — the lowest-risk, highest-value
slice of the composable model. After v1: define an MCP server or a skill **once**,
reference it from many agents; manage both in the Armory.

**In scope (v1):** standalone storage for skills + MCP servers, ownership/global
flags, direct **agent→primitive references**, the `skill.*`/`mcp.*` App API, the
config-generation rewrite to read references, migration of existing inline data,
and two Armory tabs.

**Out of scope (later):** the **Bundle** collection (grouping references — v2);
Accounts/Memory/Brief/Policy changes; the `soul`/`agentmd` → Skills content move
(noted in §7 as a follow-on, not required for v1 plumbing).

## 2. Today (verified)

| Thing | Storage | Shape |
|---|---|---|
| **Skill** | `AgentSkill` rows (`agent_skill_insert/list/delete`, `storage/skills.rs`): `{id, agent_id, name, trigger, skill_type, description, content, created_at}` | **per-agent** (keyed by `agent_id`); not shareable |
| **MCP** | `AgentContent` row, `content_type="mcp"` (`db_agent_content`, `storage/content.rs`) | **per-agent** opaque `.mcp.json` blob; not decomposed into individual servers |
| Config gen | `agent_config.rs`: `build_mcp_config` → `.mcp.json` (auto-injects `agentmux-mcp`); skills → CLAUDE.md index + `.claude/commands/<trigger>.md`. Written by `app_api.rs:write_agent_config_files` (`:2299-2405`) at launch | reads the per-agent rows/blob above |

So a skill is glued to one agent, and MCP is an undecomposed blob — neither can be
shared or reviewed as a unit.

## 3. Target

- **`db_skills`** — standalone skill records (not keyed to an agent).
- **`db_mcp_servers`** — standalone MCP server records (one server each, decomposed
  from the blob).
- **Reference tables** — `db_agent_skills_ref` / `db_agent_mcp_ref` linking an agent
  to the skill/MCP IDs it uses (direct binding; Bundle-level grouping is v2).
- **Config gen reads references** — `agent_config.rs` assembles `.mcp.json` +
  skills index + `.claude/commands/*.md` from the *referenced standalone* records.
- **Ownership/global — single source of truth.** Every standalone record carries
  exactly one flag, **`is_global: bool`** (mirroring `Memory` in `memory_bundles.rs`,
  which stores *only* `is_global` — no redundant owner column). **No
  `owner_agent_id` column** (reagent P1 on `:51`): a separate owner string +
  `is_global` can disagree (owner set yet `is_global=1`). Per-agent *ownership for
  editing* is established by the **reference** (an agent may mutate a non-global
  record it references) and enforced exactly as `preset.upsert` does — see §5.

## 4. Storage design

### 4.1 `db_skills` (standalone)
```
id            TEXT PK
name          TEXT
trigger       TEXT          -- slash-command trigger
skill_type    TEXT
description   TEXT
content       TEXT
is_global     INTEGER       -- 0/1; THE single ownership flag (cf. Memory)
created_at    INTEGER
updated_at    INTEGER
```
Reuse the `AgentSkill` field names (`name/trigger/skill_type/description/content`);
drop the hard `agent_id` FK; add only `is_global` (no `owner_agent_id`).

### 4.2 `db_mcp_servers` (standalone — decomposed)
```
id            TEXT PK
name          TEXT          -- the .mcp.json key; UNIQUE per (name, is_global=0 agent scope)
transport     TEXT          -- "stdio" | "url"
config        TEXT (JSON)   -- command/args/env or url/headers
is_global     INTEGER       -- single ownership flag (as above)
created_at    INTEGER
updated_at    INTEGER
-- UNIQUE(name, id) enforced via upsert guard: duplicate names per agent silently
-- collide in build_mcp_config, so mcp.upsert rejects a name already bound to this agent
```
One row = one server (vs. today's whole-`.mcp.json` blob). The auto-injected
`agentmux-mcp` entry stays **synthetic** (added by `build_mcp_config`, never a
user row) — see §6.

### 4.3 References (direct agent binding)
```
db_agent_skills_ref:  agent_id TEXT, skill_id TEXT  (PK pair)
                      FK skill_id  → db_skills(id)      ON DELETE CASCADE
                      FK agent_id  → db_agents(id)      ON DELETE CASCADE
db_agent_mcp_ref:     agent_id TEXT, mcp_id   TEXT  (PK pair)
                      FK mcp_id    → db_mcp_servers(id) ON DELETE CASCADE
                      FK agent_id  → db_agents(id)      ON DELETE CASCADE
```
**FK cascade is required** (reagent P1 on `:86`): deleting an agent *or* a standalone
record must remove its ref rows, so no orphaned references silently survive. Where
SQLite FK enforcement isn't guaranteed (PRAGMA off in some stores), the `delete`
handlers (§5) must also explicitly purge matching ref rows in the same transaction.
An agent's effective set = its referenced records. (Bundle membership unions in at
v2; the resolver shape is identical — just another source of references.)

## 5. App API (`skill.*`, `mcp.*`)

Add command consts in `rpc_types.rs` and handlers in `app_api.rs`, mirroring
`preset.*` exactly (S1 identity check + ownership/global guard):

| Command | Notes |
|---|---|
| `skill.list` / `mcp.list` | list records visible to the agent (own + global) |
| `skill.get` / `mcp.get` | by id |
| `skill.upsert` / `mcp.upsert` | create/update; **both** guards from `preset.upsert` (`app_api.rs` S4a): (a) **strip caller-supplied `is_global`** — force `is_global=false` on every write so an agent can never self-promote a record to global (reagent P1 on `:100`); (b) **reject mutating an existing `is_global` record** ("cannot mutate a global"). |
| `skill.delete` / `mcp.delete` | own, non-global only; also purge the record's ref rows (§4.3) |
| `skill.bind` / `skill.unbind` (and `mcp.*`) | add/remove an agent→record reference |

- **S1** (existing): `ctx.agent_id` non-empty and matches `req.agent_id`.
- **Ownership/global guard** (exactly `preset.upsert`): force `is_global=false` on
  write (strip caller escalation); reject mutating/deleting an existing global; an
  agent may write/delete only a non-global record it references — never a global or
  another agent's.
- Expose the same set over the **REST** routes the App API already mirrors, and as
  **MCP tools** in `agentmux-mcp` (parallel to the memory tools) so agents manage
  their own skills/servers.

## 6. Config-generation changes (`agent_config.rs` / `write_agent_config_files`)

Rewrite the build to read **references**, not per-agent rows:

- **`.mcp.json`** (`build_mcp_config`): for each `db_agent_mcp_ref`, emit the
  referenced `db_mcp_servers` row → `{name: {transport, ...config}}`. **Keep** the
  synthetic `agentmux-mcp` auto-injection (with `AGENTMUX_AGENT_ID`/`BUS_ID`) exactly
  as today — it is not a user record.
- **Skills**: for each `db_agent_skills_ref`, emit the CLAUDE.md `# Available Skills`
  index entry + write `.claude/commands/<trigger>.md`.
- **Fallback (back-compat):** if an agent has **no** refs yet (pre-migration),
  fall back to the legacy `agent_skill_list(agent_id)` + `AgentContent("mcp")` path,
  so unmigrated agents keep working. Remove the fallback after migration completes.

No change to *when* files are written (still `write_agent_config_files` at launch).

## 7. Migration

One-time, idempotent, behind the App API surface:

1. **Skills:** for each existing `AgentSkill` row → create a `db_skills` record
   (`is_global = 0`) + a `db_agent_skills_ref` (agent_id → new skill id); the ref
   establishes ownership. De-dupe identical skills across agents into one global
   record where names+content match (optional; safe default is per-agent copies).
2. **MCP:** parse each agent's `AgentContent("mcp")` `.mcp.json` blob → one
   `db_mcp_servers` row per server entry (`is_global = 0`) + a `db_agent_mcp_ref`
   per server (the ref establishes ownership). **Skip the JSON key `"agentmux"`**
   (the synthetic entry that `build_mcp_config` auto-injects, per `agent_config.rs:277`;
   it has command `"agentmux-mcp"` but is identified by map *key*, not command value,
   to avoid dropping a user-created server that happens to use the same command string).
3. **Read alias / fallback:** keep the legacy tables readable; config-gen falls back
   to them for any agent without refs (until migration is confirmed complete).
4. **Follow-on (not v1 plumbing): `soul`/`agentmd` → Skills.** Per the Brief stance
   (no always-on instruction blob), the `soul`/`agentmd` content currently baked into
   CLAUDE.md migrates into **Skills** (on-demand). This is a content move that *uses*
   the v1 skill primitive; schedule it once v1 storage/API land, so the heavy
   always-on CLAUDE.md can be retired.

## 8. Armory UI

Two new tabs (each: list / create / edit / delete / toggle global / bind-to-agent):
- **MCP Servers** — name, transport, config; "used by N agents".
- **Skills** — name, trigger, type, description, content.

Both deep-linked from an agent's config view ("add MCP server" / "add skill" →
pick existing or create new). Mirrors the existing Presets tab surface.

## 9. Non-goals (v1)

- The **Bundle** collection (grouping references into a named, applyable unit) — v2.
- Accounts / Memory / Brief / Policy primitives — untouched.
- The `db_memory_bundles` → `db_bundles` rename — separate (the proposal's backend
  migration).
- Removing the legacy fallback — done only after migration is verified.

## 10. Test plan

- **Storage:** CRUD + ownership/global guard unit tests for `db_skills` /
  `db_mcp_servers` and the ref tables.
- **App API:** S1 + ownership guard tests for `skill.*` / `mcp.*` (reject global
  edit, reject cross-agent), mirroring the `preset.*` tests.
- **Config gen:** an agent with refs produces the same `.mcp.json` + skill files as
  the legacy inline path (golden-file equivalence); the synthetic `agentmux-mcp`
  entry is present; an agent with no refs falls back cleanly.
- **Migration:** existing `AgentSkill` + `AgentContent("mcp")` → standalone records +
  refs; re-run is idempotent; a migrated agent launches identically (diff the
  generated `.mcp.json` / `.claude/commands/*` before/after = empty).
- **Sharing:** one global skill/server referenced by two agents appears in both;
  editing the global updates both; an agent cannot edit a global (guard).

## 11. Open items

- **De-dup on migration:** copy-per-agent (simple, safe) vs. fold identical
  skills/servers into shared globals (cleaner, riskier). Default: copy; offer a
  later "promote to global" action.
- **Bind granularity:** v1 binds at the **agent** level. Confirm that's enough
  before Bundles (v2) add bundle-level refs.
- **Policy primitive** (hooks/permissions) — tracked in the parent proposal, not v1.
