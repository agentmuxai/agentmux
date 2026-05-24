# SPEC: Two-tier agent picker — "My Agents" + "Templates"

**Date:** 2026-05-24
**Author:** AgentA
**Status:** Design analysis — needs answers to the two decision points before implementation.

---

## The vision

The current picker treats every agent definition uniformly. Click any card and you get the same flow. After Option E shipped, "Recent Sessions" sits at the top — but it lists *sessions* (blocks), not agents.

The new model splits the picker into two semantically distinct tiers:

1. **My Agents** (top) — your user-created agents. Each row IS an agent (Maks, etc.), not a session. The row shows the agent's state: when it was last active, how many messages, what it's working on. Click = continue. This subsumes today's "Recent Sessions" surface.

2. **Templates** (bottom) — the seeded provider templates (Claude Code, Codex CLI, Cursor, etc.). Click = **create a new user agent** from this template. The template itself doesn't carry conversation state; it's a starting blueprint.

The split mirrors a model that already exists in the schema: `db_agent_definitions.is_seeded INTEGER NOT NULL DEFAULT 0` (see `agentmux-srv/src/backend/storage/migrations.rs:131`). Today's `agent_seed.rs` flips it to `1` for manifest entries; user-created definitions stay at `0`. The picker just doesn't use the distinction yet.

---

## Current state of the code

### Schema (relevant columns)

`db_agent_definitions`:
- `id` TEXT PRIMARY KEY
- `slug` TEXT
- `name` TEXT
- `icon` TEXT
- `description` TEXT
- `provider_id` TEXT
- `is_seeded` INTEGER NOT NULL DEFAULT 0  ← the split signal
- `created_at` INTEGER
- ... (auth, cmd, env)

### Seed flow

`agentmux-srv/src/backend/agent_seed.rs` reads an embedded manifest on first launch (and on manifest version bumps), inserts entries with `is_seeded = 1`, and on re-seed REMOVES seeded entries no longer in the manifest (`is_seeded == 1 && !manifest_ids.contains(id)`).

User-created entries are not touched by seeding.

### Picker (current)

`frontend/app/view/agent/components/AgentPicker.tsx:619-625`:
- `<RecentSessionsList>` at top (generic, lists sessions across all agents).
- `<For each={agents()}>` below — `agents()` is `ListNamedAgentsCommand` output (or similar), unfiltered.
- Click on an `AgentCard` → `handleSelect(agent, evt)` → Option E logic: if `:current` zone non-empty, auto-continue; otherwise open the launch modal.

There's no top-level distinction between user-created and seeded definitions.

---

## Target state

### Layout

```
┌──────────────────────────────────────────┐
│  My Agents                               │
│  ┌────────────────────────────────────┐  │
│  │ 🤖 Maks                            │  │
│  │    last active: 4 hours ago        │  │
│  │    169 messages, working on the    │  │
│  │    agent-pane state machine        │  │
│  └────────────────────────────────────┘  │
│  ┌────────────────────────────────────┐  │
│  │ ✦ AgentY                            │  │
│  │    last active: 12 minutes ago     │  │
│  │    42 messages, new                │  │
│  └────────────────────────────────────┘  │
│                                          │
│  + New agent from template               │
│  ┌────────────────────────────────────┐  │
│  │ Claude Code                        │  │
│  │ Codex CLI                          │  │
│  │ Cursor                             │  │
│  │ ...                                │  │
│  └────────────────────────────────────┘  │
└──────────────────────────────────────────┘
```

### Semantics

**My Agents** entries:
- Listed from `db_agent_definitions WHERE is_seeded = 0`.
- Each row shows: identity icon + name + last-active relative time + node count + preview of last user message (lifted from `agent:<defId>:current` zone).
- Click = `launchAgentDefinition(agent, ...)` with the agent's own bindings (no modal). Same as today's Option E auto-continue for these.
- Each row has a hover-revealed "+ New session" pill (current behavior).
- Empty state: "You haven't created any agents yet. Pick a template below to get started."

**Templates** entries:
- Listed from `db_agent_definitions WHERE is_seeded = 1`.
- Each row is a compact template card: icon + name + description.
- Click = **create a new user agent** from this template. Flow:
  1. Open a "Name your agent" modal (with sensible default like "{Template} #2").
  2. Pick identity + memory.
  3. Submit → backend `CreateUserAgent` RPC clones the template's metadata into a new `db_agent_definitions` row with `is_seeded = 0` and the user-picked name.
  4. Launch the new agent in a pane (which IS the agent's first session).
- Template rows do NOT auto-continue and do NOT show session state. They're pure starting points.

### RPCs needed

Mostly already exist — just need to filter and add one new write path.

- `agent_def_list` (existing): take an optional `is_seeded` filter parameter.
- `agent_session_read` (E1): used as today.
- New: `agent_def_create_from_template { template_id, name, identity_id, memory_id }` → returns the new `definition_id`. Internally inserts a new `db_agent_definitions` row with `is_seeded = 0`, copies the template's cmd/env/auth fields, returns the id so the frontend can immediately launch it.

### Frontend split

`AgentPicker.tsx`:
- Two `<For>` loops over partitioned `agents()` output (memoized).
- `MyAgentsList` component (replaces today's `RecentSessionsList` + the upper part of the agent list).
- `TemplatesList` component (the lower section).
- Click handlers diverge: my-agent click → `launchAgentDefinition`; template click → `openCreateFromTemplateModal`.

`AgentCard` likely gets two variants (or a `variant?: "agent" | "template"` prop). Today's `hasCurrentSession` + `+New session` pill only apply to the agent variant.

---

## Resolved design decisions

### Q1: Can a seeded definition ever have its own session? — **Decision: Option C, IN PHASE 1**

A seeded definition (template) NEVER carries a session zone in the new model. The moment a user spawns a session on a seeded definition, that definition is cloned to a new `is_seeded=0` row with a sensible default name, and the session zone moves to the new row. The template card stays clean as a starting point.

**Why this matters for Phase 1 scope:** under Option E, session zones are keyed by `definition_id`. Today's reality is that users HAVE been clicking seeded definitions directly — Maks's conversation is at `agent:claude:current` where `claude` is a seeded template. If Phase 1 ships the two-tier picker WITHOUT the auto-promote migration:

- "claude" appears under Templates.
- Clicking "claude" → opens launch modal → spawns a new instance → reads `agent:claude:current` → **sees Maks's conversation** because that's where Option E stored it.
- Two users (or two intents) would be appending to the same session zone, indistinguishable from each other.

So Phase 1 MUST include a one-shot **seeded-def-to-user-agent migration** in addition to the picker UI rewiring. The migration:

1. For each `agent:<defId>:current` zone where `<defId>.is_seeded = 1`:
   - Look up the most recently active `db_agent_instances` row with this `definition_id`. Use its `instance_name` as the new agent's name (e.g. "Maks") — fall back to `{template.name}` if instance has no name.
   - INSERT a new `db_agent_definitions` row: copy `provider_id`, `cmd`, `cmd_args`, `cmd_env`, `auth_config_dir_env` from the template; set `is_seeded = 0`, generated id, the chosen name.
   - Move the filestore zone: rename `agent:<oldDefId>:current` → `agent:<newDefId>:current`. Also move any `agent:<oldDefId>:archive:*` zones to `agent:<newDefId>:archive:*`.
   - Update `db_agent_instances.definition_id` for any rows referencing the old template to point at the new user-agent row (preserves the existing reattach flow).
2. Gate the migration with a marker file `<data_dir>/migration_template_promote_v1.flag`. One-shot, idempotent on second run.

Templates are clean post-migration. The `agent:<seeded_template_id>:*` namespace is empty. Future clicks on a template are unambiguous "create new" actions.

Why Option C and not A or B:
- **A** (templates never have sessions) is theoretically clean but loses Maks's data (or requires the user to manually archive him first — bad UX).
- **B** (templates show "(in use)" badge with continue button) keeps the conceptual conflation we're trying to remove. The whole point of the split is "templates are factories, not entities".
- **C** preserves data, matches user intuition (the act of using a template makes a copy that's yours), and lets templates be conceptually pure.

### Q2: Can a user delete a seeded template? — **Decision: Option Y (hide, not delete), DEFERRED to follow-up PR**

Decision locked: users can **hide** seeded templates but never **delete** them. Hide-state is persisted in a new `db_agent_definitions.user_hidden INTEGER NOT NULL DEFAULT 0` column. Manifest re-sync MUST reset hide-state for any newly-added templates (so new templates surface once even if the user previously hid a same-named one).

Why hide-not-delete:
- Seeded templates are manifest-managed; user delete would conflict with manifest re-sync (deleted templates would just reappear on next seed).
- Hide is a per-user preference; doesn't affect global state; survives manifest updates for existing templates.
- Reset on new-add prevents the user from accidentally hiding the entire list and being unable to recover.

Why deferred to a follow-up PR:
- Not needed for Phase 1 correctness — the picker works fine without it.
- Adds a column migration + a small UX surface (right-click → Hide, with an "unhide" affordance in settings).
- Better to ship Phase 1's main win first, then layer this on.

The decision is locked so when we do implement, there's no re-discussion: hide, not delete; new templates auto-unhide on first appearance; per-user preference column on `db_agent_definitions`.

---

## Migration

Minimal. The `is_seeded` column already exists with correct values:
- All existing seeded definitions have `is_seeded = 1`.
- All user-created definitions (if any) have `is_seeded = 0`.

If we adopt Q1 Option C, also need a one-shot migration on startup:
- For each seeded definition WHERE an `agent:<id>:current` zone is non-empty:
  - Clone the definition with `is_seeded = 0` and a default name (`{template.name} #1`).
  - Move the zone from `agent:<oldId>:current` to `agent:<newId>:current`.
- Log the migrations so users know what was renamed.

---

## Implementation order

**Phase 1 (this PR)** — picker rewiring + REQUIRED auto-promote migration:

1. **Backend**:
   - `ListAgentDefinitions` gains optional `is_seeded: Option<i64>` filter parameter.
   - New `agent_def_create_from_template { template_id, name, identity_id, memory_id }` RPC — clones a template into a user agent with `is_seeded=0`.
   - One-shot migration on startup: promote any seeded definition with a non-empty session zone to a new user-agent definition (per Q1 Decision C). Marker-file gated.
2. **Frontend**:
   - `RecentSessionsList` → `MyAgentsList` (rename + relabel; data source stays `ListNamedAgentsCommand`, returns `NamedAgentRow[]`).
   - Filter the bottom card list to `is_seeded === 1` only.
   - Template click → "Name your agent" modal → `agent_def_create_from_template` RPC → launch.
   - My-agent click → existing `handleReattach` (unchanged).
3. **Tests** + spec doc rides with code per `feedback_no_doc_only_prs`.

Estimated: 600–900 lines, single PR.

**Phase 2 (later PR, follow-up)** — hide templates (Q2 Decision Y):

- Schema migration: add `db_agent_definitions.user_hidden INTEGER NOT NULL DEFAULT 0`.
- UI: right-click on a template card → Hide. Settings panel → "Show hidden templates" toggle to unhide.
- Manifest re-sync logic: any NEW template id auto-resets `user_hidden = 0` so newly-added templates always surface once.

Estimated: 200–400 lines.

**Phase 3 (separate cycle)** — the full `db_agents` consolidation (see `SPEC_AGENT_CONCEPT_CONSOLIDATION_2026_05_24.md`). Out of scope for this picker spec.

---

## What disappears

- `RecentSessionsList` becomes redundant — "My Agents" subsumes it. Each user agent has exactly one current session by definition (Option E semantics).
- The `listrecentsessions` RPC stays useful as a cross-agent "archived sessions" view (e.g., for a future "Restore archived session" flow). The picker no longer calls it.

## What stays the same

- Modifier-key escape hatch on My Agents click (force the launch modal). Useful for changing identity/memory at launch.
- The "+ New session" pill on each My Agents row (archives current, opens modal).
- Option E's agent zone storage and migration — unchanged.

---

## Recommendation

Ship in the order above with **Q1 = C** and **Q2 = Y deferred to a later PR**. Land the visible UI win first (My Agents top, Templates bottom); polish (hide templates) becomes a follow-up after the model proves out.

Need your answers on Q1 and Q2 before implementation starts.
