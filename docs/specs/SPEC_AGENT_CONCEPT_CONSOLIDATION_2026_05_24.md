> **⚠️ SUPERSEDED — 2026-06-13.** Retained for its design rationale and the inbound code/doc references that cite it. For the current, code-anchored architecture of agent data & cross-channel persistence, see **[ARCHITECTURE_AGENT_DATA_AND_CROSS_CHANNEL_2026_06_13.md](../architecture/ARCHITECTURE_AGENT_DATA_AND_CROSS_CHANNEL_2026_06_13.md)**.

# SPEC: Agent concept consolidation — DRY rethink

**Date:** 2026-05-24
**Author:** AgentA
**Status:** Design analysis — answers a question the two-tier picker spec raised about whether we need this many "agent" types.

---

## The current sprawl

Today the codebase has SEVEN tables / concepts in the "agent" family. The forge → agent rename earlier in the project already collapsed five `db_forge_*` tables into `db_agent_*` ones, but the multiplicity of *concepts* remained.

| Concept | Table / Storage | What it represents |
|---|---|---|
| **Definition** | `db_agent_definitions` (140 rows of seeded + user) | Blueprint: provider, command, env, default auth. `is_seeded` flag. |
| **Instance** | `db_agent_instances` | A spawned runtime entity. References `definition_id`. Carries `instance_name`, `identity_id`, `memory_id`, `working_directory`, `status`, `started_at`, `ended_at`, `block_id`, `session_id`, `parent_instance_id`, `display_hidden`, `github_context`. |
| **Block** | `db_block` rows with `meta.view = "agent"` | UI pane that references an agent via `meta.agentId`. |
| **Session zone** (Option E) | `filestore.db` zone `agent:<defId>:current` | The conversation snapshot + raw stream. **Keyed by definition_id**, NOT instance_id. |
| **Archive zone** | `filestore.db` zone `agent:<defId>:archive:<ts>` | Past conversations of an agent (Option E "+ New session" archives). |
| **Content** | `db_agent_content` | Per-agent content blobs. FK to `db_agent_definitions(id)`. |
| **History** | `db_agent_history` | Per-agent history (separate from session zone). |

**125 source references** to `db_agent_instances` / `AgentInstance` across `agentmux-srv/` and `frontend/` (Rust + TS combined). The instance table is the heaviest of these by a wide margin.

The user's question is correct: this *is* a lot, and Option E made it worse, not better.

---

## What blurred when Option E shipped

Before Option E:
- One **definition** could spawn N **instances** (each with different bindings).
- Each instance owned one **block** (the UI pane it lived in) and one **session zone** keyed by `block_id`.
- Many-to-one fan-in: many instances → one definition. One-to-one fan-out: one instance → one block → one zone.

After Option E:
- The **session zone** is now keyed by `definition_id`, not `block_id`.
- So multiple instances of the same definition now SHARE one zone. The `instances.session_id` field is vestigial.
- `parent_instance_id` (continuation linkage between instances) is also vestigial — Option E's zone is structurally continuous; no need for instance-to-instance chains.

Net: about half of `db_agent_instances` columns are now dead weight under the Option E semantics we actually shipped.

---

## What each concept actually does in production

This is the audit that tells us what to keep.

**Definition** — keep.
- Provider config, cmd template, auth wiring.
- The starting point for any agent.

**Instance** — partial keep, mostly dead.
- `definition_id`: redundant after the merge (the row IS the agent).
- `parent_instance_id`: dead under Option E.
- `block_id`: orthogonal — belongs in `db_block.meta`, not in an instance row.
- `session_id`: dead under Option E.
- `status`, `started_at`, `ended_at`: arguably belong in an audit log, not in the canonical agent row.
- `instance_name`, `identity_id`, `memory_id`, `working_directory`: **THESE are what an "agent" actually is** — the user-given configuration of a named entity. They should live on the agent itself.
- `display_hidden`: UI preference, belongs in `db_block.meta` or a user-prefs table.
- `github_context`: bound to the agent identity, belongs on the agent row.

**Block** — keep.
- Layout, position, view-type, references the agent it's showing.

**Session zone** — keep (Option E).

**Content + History** — keep (separate concerns from agent identity).

---

## The lean model — 3 first-class concepts

### 1. `db_agents` (rename + absorb)

One table. Two flavors via `is_template` flag.

```sql
CREATE TABLE db_agents (
    id                  TEXT PRIMARY KEY,
    name                TEXT NOT NULL,
    icon                TEXT NOT NULL DEFAULT '',
    description         TEXT NOT NULL DEFAULT '',

    -- Template vs user agent
    is_template         INTEGER NOT NULL DEFAULT 0,
    parent_template_id  TEXT NOT NULL DEFAULT '',   -- which template I was cloned from (if any)

    -- Provider/cmd config (was on definition)
    provider_id         TEXT NOT NULL,
    cmd                 TEXT NOT NULL DEFAULT '',
    cmd_args            TEXT NOT NULL DEFAULT '[]',
    cmd_env             TEXT NOT NULL DEFAULT '{}',
    resume_flag         TEXT NOT NULL DEFAULT '',
    auth_config_dir_env TEXT NOT NULL DEFAULT '',

    -- Bindings (was on instance — only relevant when is_template=0)
    identity_id         TEXT NOT NULL DEFAULT '',
    memory_id           TEXT NOT NULL DEFAULT '',
    working_directory   TEXT NOT NULL DEFAULT '',
    github_context      TEXT NOT NULL DEFAULT '',

    -- Provenance
    created_at          INTEGER NOT NULL DEFAULT 0,
    updated_at          INTEGER NOT NULL DEFAULT 0,
    is_seeded           INTEGER NOT NULL DEFAULT 0,  -- managed by manifest seed
    user_hidden         INTEGER NOT NULL DEFAULT 0   -- (future) user hid this template
);
```

This replaces both `db_agent_definitions` AND `db_agent_instances`.

### 2. `db_block` (unchanged structurally)

A block with `meta.view = "agent"` references an agent via `meta.agentId`. Multiple blocks can show the same agent (cross-tab). Block lifecycle is orthogonal to agent lifecycle.

### 3. `filestore.db` zone `agent:<id>:current` + `:archive:*` (Option E, unchanged)

The conversation history. One current zone per agent; many archives per agent.

### Optional 4th: `db_agent_events` (audit log)

If we want lifecycle history (when started, when ended, status changes), put it in an append-only event table. Decoupled from the agent row, so deleting an agent doesn't lose audit. Probably unnecessary for now.

---

## Operational mapping — what the picker spec becomes

The two-tier picker spec from earlier maps cleanly onto this model:

- **My Agents** = `db_agents WHERE is_template = 0 AND user_hidden = 0`
- **Templates** = `db_agents WHERE is_template = 1 AND user_hidden = 0`
- Click a template → `agent_create_from_template { template_id, name, identity_id, memory_id }`:
  - INSERT a new `db_agents` row with `is_template = 0`, `parent_template_id = template_id`, copied cmd/env/auth fields, user-picked name/identity/memory.
  - Returns the new id; frontend immediately opens a block for it.

No new "instance" concept needed. The act of creating a user agent IS the act of cloning the template into a user-owned row.

---

## What `db_agent_instances` rows become

This is the migration. Three patterns map cleanly:

| Existing instance row | Maps to |
|---|---|
| First instance of a definition (definition has 1 instance, no name override) | The instance's bindings get folded into the definition row; the instance row deleted. |
| Renamed instance of a definition (Maks-the-instance from Claude Code-the-definition) | Clone the definition into a new agent row with `is_template = 0`, `parent_template_id = old_defId`, copied bindings. The old definition (Claude Code) flips to `is_template = 1`. Instance row deleted. |
| Multiple unnamed instances of one definition | Each becomes its own user agent row, defaulted name = `{template.name} #N`. |

Session zones are already keyed by definition_id (Option E), so for the "renamed" case the migration must also **move the zone** from `agent:<oldDefId>:current` → `agent:<newAgentId>:current` for the agent that takes over, and clear the source so the template is clean.

For the seeded-template-used-directly case (Q1 Option C from the picker spec): same move-the-zone logic.

---

## Implementation cost — honest accounting

- 125 source references touch `AgentInstance` / `db_agent_instances`. Each one needs to either:
  - Rewrite to use `db_agents` directly, OR
  - Get deleted (it was instance-specific scaffolding that's not needed anymore).
- The forge → agent rename precedent (already in the codebase) collapsed 5 tables; the team has done this kind of refactor before.
- Risk: breaking churn for in-flight features. Recommend doing this between major releases, not before.
- Estimated effort: **4–6 PRs**, ~2500–3500 lines total. Schema migration + Rust rewrites + frontend updates + test churn.

---

## Recommendation

Yes, the rethink is worth it. Three reasons:

1. **The picker spec is cleaner with this model.** Templates vs My Agents is a single `is_template` filter on one table. No JOIN against instances.

2. **Option E already broke the old semantics.** `db_agent_instances` is half-dead in production already. The longer we leave it, the more code is built on top of vestigial state.

3. **The forge rename precedent shows the team CAN do these.** And the lean shape is simpler to explain (and test) than the current sprawl.

### Sequencing proposal

Do this in two phases:

**Phase 1 (next): two-tier picker on the existing schema.**
Per the previous spec (`SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md`). Uses `is_seeded` as the template flag for now; doesn't touch `db_agent_instances`. Ships the user-visible win this week.

**Phase 2 (later, separate cycle): collapse instance into agent.**
Schema migration `db_agent_definitions` + `db_agent_instances` → `db_agents`. Frontend + Rust call-site rewrites. Done across multiple PRs guarded by dual-read (write to old + new, read from new), then drop the old once verified.

This way you get the visible improvement now (Phase 1) without paying the full structural cost; and the structural cleanup happens in a focused cycle once you've decided it's worth the refactor budget.

---

## Open questions

- **Decoupled audit log** (`db_agent_events`): yes / no? Useful for "show me when this agent was last interrupted / errored / paused" without bloating the agent row.
- **`is_template` mutability**: can a user-agent be "promoted" back to template? Probably no; one-way clone.
- **Cross-tab in the lean model**: multiple blocks for one agent share the session zone — naturally handles cross-tab when E3 lands.

---

## The shorter answer

You're right; we don't need this many types. The clean shape is:

- **Agents** (one table; templates and user-owned distinguished by a flag).
- **Blocks** (UI panes pointing at agents).
- **Session zones** (Option E).

Three concepts. Everything else is implementation detail or audit-log noise.

`db_agent_instances` is the artifact we should retire. But it has 125 callers, so retire it deliberately in a focused refactor cycle, not as a side-quest off the picker work.
