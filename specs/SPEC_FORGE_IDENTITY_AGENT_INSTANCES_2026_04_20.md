# Spec: Forge + Identity + Agent Instances Refinement
**Date:** 2026-04-20  
**Status:** Draft

---

## Current State (Problems)

| Issue | Where |
|---|---|
| Identity stored in localStorage — not portable, not persistent | `identity-model.ts:116` |
| No agent definition vs instance distinction at data layer | `wstore.rs:399` |
| No branching/lineage — can't fork an agent and track lineage | missing |
| No export of identity/forge as a portable bundle | only session JSONL export exists |
| `accounts` on ForgeAgent is a JSON blob — no relational integrity | `wstore.rs:441` |

---

## Core Concepts

```
AgentDefinition (what an agent IS)
    └── has many ForgeContent blobs (soul, agentmd, mcp, env, hooks, memory)
    └── has many ForgeSkills
    └── references many Identities (via junction table)
    └── may have a parent_definition_id (branched from another)

AgentInstance (a running/historical execution of a definition)
    └── belongs to one AgentDefinition
    └── has a pane/block_id (which pane it's running in)
    └── has session_id (provider session — for Claude Code resume)
    └── has optional github_context (repo, pr, branch — for async GitHub work)
    └── has status: running | paused | stopped | crashed | detached
    └── may have a parent_instance_id (if spawned from another instance)

Identity (an account credential, reusable)
    └── persisted in DB (not localStorage)
    └── has provider: github | aws | anthropic | custom
    └── has secret_ref (env var, secrets manager path, or dev plaintext)
    └── has context (username, scopes, role ARN, etc.)
    └── referenced by many AgentDefinitions
```

---

## Proposed Schema

### `db_identity_accounts` — move from localStorage → DB

```sql
CREATE TABLE db_identity_accounts (
    id           TEXT PRIMARY KEY,   -- UUID
    name         TEXT NOT NULL,      -- user-facing label, e.g. "asaf-github"
    provider     TEXT NOT NULL,      -- github | aws | anthropic | custom
    kind         TEXT NOT NULL,      -- pat | role | api_key | env_ref
    display_name TEXT,
    secret_ref   TEXT NOT NULL,      -- JSON: {backend, env_var?, sm_path?, sm_json_path?}
    context      TEXT NOT NULL,      -- JSON: provider-specific (username, scopes, arn, etc.)
    status       TEXT DEFAULT 'unknown',
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL
);
```

### `db_forge_agent_identities` — junction table (replaces accounts JSON blob)

```sql
CREATE TABLE db_forge_agent_identities (
    agent_id    TEXT NOT NULL REFERENCES db_forge_agents(id) ON DELETE CASCADE,
    account_id  TEXT NOT NULL REFERENCES db_identity_accounts(id) ON DELETE CASCADE,
    provider    TEXT NOT NULL,   -- denormalized for fast lookup
    PRIMARY KEY (agent_id, provider)
    -- only one account per provider per agent
);
```

### `db_forge_agents` — add lineage fields

```sql
ALTER TABLE db_forge_agents ADD COLUMN parent_id TEXT;
-- NULL = root definition; non-null = branched from another definition

ALTER TABLE db_forge_agents ADD COLUMN branch_label TEXT;
-- e.g. "pr-422-review", "experiment-refactor"
```

### `db_agent_instances` — new table

```sql
CREATE TABLE db_agent_instances (
    id                 TEXT PRIMARY KEY,   -- UUID
    definition_id      TEXT NOT NULL REFERENCES db_forge_agents(id),
    parent_instance_id TEXT,               -- NULL if first launch; set if forked from running instance
    block_id           TEXT,               -- pane block (NULL if not currently in a pane)
    session_id         TEXT,               -- provider session ID (e.g. Claude Code --resume target)
    status             TEXT NOT NULL DEFAULT 'running',
    -- running | paused | stopped | crashed | detached
    github_context     TEXT,
    -- JSON: {repo, pr_number?, branch?, issue_number?, workflow_run_id?}
    -- populated when instance is doing async GitHub work
    started_at         INTEGER NOT NULL,
    ended_at           INTEGER,
    created_at         INTEGER NOT NULL
);

CREATE INDEX idx_agent_instances_definition ON db_agent_instances(definition_id);
CREATE INDEX idx_agent_instances_block      ON db_agent_instances(block_id);
CREATE INDEX idx_agent_instances_status     ON db_agent_instances(status);
```

---

## Two User Situations

### New user (no accounts, no definitions)
- First-launch wizard walks through creating one Identity per provider they use
- Then walks through creating first ForgeAgent, auto-linked to those identities
- Seeded built-in agents (AgentX/Y/Z) stay as-is but with `accounts` migrated to junction table

### Existing user (accounts + working identity already set up)
- Migration reads existing `accounts` JSON blob from each ForgeAgent
- Matches each account by provider against any existing `db_identity_accounts` rows
- If an account entry exists in localStorage → imports it into DB on first startup
- Existing sessions/instances get a "legacy" `db_agent_instances` record with `status = stopped` and `session_id` populated from block metadata

---

## Agent Branching & Instance Tracking

```
ForgeAgent(slug="agentx")                                       ← definition, root
    │
    ├─ Instance A  [block:pane-1, running, github_context: PR #422]
    └─ Instance B  [block:pane-3, paused]

ForgeAgent(slug="agentx-pr-455-fork", parent_id="agentx-id")   ← branched definition
    │
    └─ Instance C  [block:pane-2, running, parent_instance_id: Instance A]
```

- **Interagent coordination:** `db_agent_instances.block_id` lets the bus know which pane
  an instance lives in; broadcast messages can target by `definition_id` (all instances of
  this agent) or `instance_id` (specific one)
- **GitHub async:** `github_context` on the instance (not the definition) captures what
  PR/branch/workflow a specific run is operating against — same definition can have multiple
  instances each on different PRs simultaneously

---

## Export Format

A portable `.agentbundle` file (gzipped JSON):

```jsonc
{
  "version": 1,
  "exported_at": "2026-04-20T...",
  "agent": {
    "id": "...",
    "slug": "agentx",
    "name": "AgentX",
    "provider": "claude",
    "description": "...",
    "parent_id": null,
    "branch_label": null,
    "content": {
      "soul": "...",
      "agentmd": "...",
      "mcp": "...",
      "env": "..."
    },
    "skills": [
      { "name": "Startup", "trigger": "startup", "skill_type": "prompt", "content": "..." }
    ]
  },
  "identities": [
    {
      "name": "asaf-github",
      "provider": "github",
      "kind": "pat",
      "secret_ref": { "backend": "env", "env_var": "GITHUB_TOKEN" },
      "context": { "github_username": "AgentA-asaf" }
      // NOTE: actual secret value is never exported — only the reference
    }
  ],
  "instance_summary": {
    "total_instances": 3,
    "last_active": "2026-04-19T...",
    "github_prs_touched": [422, 418]
  }
}
```

**Key design decisions:**
- Secrets are **never** exported — only the reference (`env_var` name or `sm_path`)
- A `--include-dev-secrets` flag can optionally include `plaintext_dev` values for local transfer
- Bundle is self-contained enough to `import` into a fresh AgentMux install and reconstruct
  the agent (minus actual secret values)

---

## Migration Path

1. **DB migration v6:** Create `db_identity_accounts`, `db_forge_agent_identities`,
   `db_agent_instances`; add `parent_id`/`branch_label` to `db_forge_agents`
2. **Data migration:** Read localStorage accounts (via startup RPC from frontend), upsert
   into `db_identity_accounts`, build junction rows from existing `accounts` JSON blob on
   each ForgeAgent
3. **Deprecate** `accounts` column on `db_forge_agents` — zero it out post-migration, remove in v7
4. **Frontend:** Update IdentityViewModel to read/write DB via new RPC instead of localStorage

---

## Affected Files

| File | Change |
|---|---|
| `agentmux-srv/src/backend/storage/wstore.rs` | Add `ForgeAgentIdentity`, `AgentInstance` structs; add `parent_id`/`branch_label` to `ForgeAgent` |
| `agentmux-srv/src/backend/storage/migrations.rs` | Add migration v6 |
| `agentmux-srv/src/backend/rpc_types.rs` | New RPC commands for identity CRUD, instance CRUD, bundle export/import |
| `agentmux-srv/src/server/forge_handlers.rs` | Implement new handlers |
| `frontend/app/view/identity/identity-model.ts` | Replace localStorage with DB-backed RPCs |
| `frontend/app/view/agent/agent-model.ts` | Track instance on launch; support branch |
| `frontend/types/gotypes.d.ts` | Add `AgentInstance`, `IdentityAccount` TS types |
