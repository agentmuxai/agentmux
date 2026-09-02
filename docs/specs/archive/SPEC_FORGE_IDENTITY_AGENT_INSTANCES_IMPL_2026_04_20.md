# Implementation Spec: Forge + Identity + Agent Instances

> **Archived 2026-07-12.** Stale — builds `db_forge_agent_identities`, which exists in current source only as the legacy side of a rename pair (now `db_agent_identity_links`, see `migrations.rs`). The Forge system this describes is gone. Consolidated tracking: issue #2024.

**Date:** 2026-04-20
**Status:** Draft (implementation-ready)
**Companion:** `SPEC_FORGE_IDENTITY_AGENT_INSTANCES_2026_04_20.md` (design / motivation)

---

## Scope

Implements the data-model refinement laid out in the companion spec — moves identity accounts to the DB, splits agent definitions from instances, adds branching, and ships a `.agentbundle` export/import format.

**No migration.** AgentMux is pre-1.0; the `objects.db` rename PR (#478) already established the precedent of dropping data on schema changes. Fresh install creates the new schema cleanly. Existing localStorage identity data is treated as user-must-recreate (one-time hit, dev users only — public users haven't shipped). The spec's "two user situations" section is descoped accordingly.

---

## Phase order

Land in this order so each phase is independently revertible:

1. **Schema** — add tables to migration set, regenerate seed.
2. **Rust types + storage** — `IdentityAccount`, `AgentInstance`, junction row, lineage fields on `ForgeAgent`.
3. **RPC commands** — CRUD for identities + instances; bundle export/import.
4. **Frontend identity model** — swap localStorage backing for DB-backed RPCs.
5. **Frontend agent launch wiring** — create an `AgentInstance` row on launch, update `block_id`/`status`/`session_id`.
6. **Branching UI** — fork action on agent definition; lineage rendering.
7. **Bundle export/import UI** — buttons in the identity/forge panel.

Phases 1-3 are backend-only and can ship as one PR. Phases 4-5 ship together (frontend depends on RPCs landing first). Phases 6-7 are independent UI follow-ups.

---

## Phase 1 — Schema

### Files

- `agentmux-srv/src/backend/storage/migrations.rs` — add migration v6 (or whatever the next slot is; verify when implementing).

### Tables to add

```sql
-- Identity accounts (replaces localStorage)
CREATE TABLE db_identity_accounts (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    provider     TEXT NOT NULL,           -- github | aws | anthropic | custom
    kind         TEXT NOT NULL,           -- pat | role | api_key | env_ref
    display_name TEXT,
    secret_ref   TEXT NOT NULL,           -- JSON: { backend, env_var?, sm_path?, sm_json_path?, plaintext_dev? }
    context      TEXT NOT NULL,           -- JSON: provider-specific (username, scopes, arn, etc.)
    status       TEXT NOT NULL DEFAULT 'unknown',
    created_at   INTEGER NOT NULL,
    updated_at   INTEGER NOT NULL
);

CREATE INDEX idx_identity_accounts_provider ON db_identity_accounts(provider);

-- Junction: agents → identities (replaces ForgeAgent.accounts JSON blob)
CREATE TABLE db_forge_agent_identities (
    agent_id    TEXT NOT NULL REFERENCES db_forge_agents(id) ON DELETE CASCADE,
    account_id  TEXT NOT NULL REFERENCES db_identity_accounts(id) ON DELETE CASCADE,
    provider    TEXT NOT NULL,
    PRIMARY KEY (agent_id, provider)
);

CREATE INDEX idx_forge_agent_identities_account ON db_forge_agent_identities(account_id);

-- Agent instances
CREATE TABLE db_agent_instances (
    id                 TEXT PRIMARY KEY,
    definition_id      TEXT NOT NULL REFERENCES db_forge_agents(id) ON DELETE CASCADE,
    parent_instance_id TEXT REFERENCES db_agent_instances(id) ON DELETE SET NULL,
    block_id           TEXT,
    session_id         TEXT,
    status             TEXT NOT NULL DEFAULT 'running',
    -- enum: running | paused | stopped | crashed | detached
    github_context     TEXT,                            -- JSON
    started_at         INTEGER NOT NULL,
    ended_at           INTEGER,
    created_at         INTEGER NOT NULL
);

CREATE INDEX idx_agent_instances_definition ON db_agent_instances(definition_id);
CREATE INDEX idx_agent_instances_block      ON db_agent_instances(block_id);
CREATE INDEX idx_agent_instances_status     ON db_agent_instances(status);
CREATE INDEX idx_agent_instances_parent     ON db_agent_instances(parent_instance_id);
```

### Changes to existing tables

```sql
ALTER TABLE db_forge_agents ADD COLUMN parent_id TEXT REFERENCES db_forge_agents(id) ON DELETE SET NULL;
ALTER TABLE db_forge_agents ADD COLUMN branch_label TEXT;

-- Drop the now-unused accounts column on db_forge_agents (junction table replaces it)
ALTER TABLE db_forge_agents DROP COLUMN accounts;
```

### Forge seed update

`scripts/gen-seed.js` + regenerated `agentmux-srv/forge-seed.json`:

- Default agents (agentx/y/z) get `parent_id = null`, `branch_label = null`.
- No `accounts` field.
- No identity rows seeded — identities are user-created on first launch (or at all).

---

## Phase 2 — Rust types + storage

### Files

- `agentmux-srv/src/backend/storage/wstore.rs` — add `IdentityAccount`, `AgentInstance`, `ForgeAgentIdentity` structs, CRUD methods (`get_identity_account`, `list_identity_accounts`, `upsert_identity_account`, `delete_identity_account`, etc.).
- `agentmux-srv/src/backend/obj.rs` (or wherever `ForgeAgent` lives) — add `parent_id: Option<String>` and `branch_label: Option<String>` fields; remove `accounts` field.

### Struct shapes

```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct IdentityAccount {
    pub id: String,
    pub name: String,
    pub provider: String,         // "github" | "aws" | "anthropic" | "custom"
    pub kind: String,              // "pat" | "role" | "api_key" | "env_ref"
    pub display_name: Option<String>,
    pub secret_ref: SecretRef,     // serialized as JSON in `secret_ref` column
    pub context: serde_json::Value, // free-form per provider
    pub status: String,            // "unknown" | "ok" | "expired" | "invalid"
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "backend", rename_all = "snake_case")]
pub enum SecretRef {
    Env { env_var: String },
    SecretsManager { sm_path: String, sm_json_path: Option<String> },
    PlaintextDev { plaintext_dev: String }, // dev-mode only; warn on prod
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentInstance {
    pub id: String,
    pub definition_id: String,
    pub parent_instance_id: Option<String>,
    pub block_id: Option<String>,
    pub session_id: Option<String>,
    pub status: InstanceStatus,
    pub github_context: Option<GitHubContext>,
    pub started_at: i64,
    pub ended_at: Option<i64>,
    pub created_at: i64,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum InstanceStatus {
    Running,
    Paused,
    Stopped,
    Crashed,
    Detached,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GitHubContext {
    pub repo: String,                       // "owner/repo"
    pub pr_number: Option<u32>,
    pub branch: Option<String>,
    pub issue_number: Option<u32>,
    pub workflow_run_id: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ForgeAgentIdentity {
    pub agent_id: String,
    pub account_id: String,
    pub provider: String,
}
```

### Notes

- `SecretRef` uses an internally-tagged enum (`#[serde(tag = "backend")]`) so the JSON shape matches what the frontend already half-expects (`{ backend: "env", env_var: "GITHUB_TOKEN" }`).
- `InstanceStatus` as a Rust enum + matching TS literal-union — locks down the spec's loose string list.
- Drop the loose `accounts: Option<Value>` field on `ForgeAgent`; junction table replaces it.

---

## Phase 3 — RPC commands

### Files

- `agentmux-srv/src/backend/rpc_types.rs` — add command constants + payload structs.
- `agentmux-srv/src/server/forge_handlers.rs` (or new `identity_handlers.rs` / `instance_handlers.rs` if it grows large enough).
- `agentmux-srv/src/server/websocket.rs` — register the new handlers.

### Commands

| Command | Payload | Returns |
|---|---|---|
| `listidentityaccounts` | `{ provider?: string }` | `IdentityAccount[]` |
| `getidentityaccount` | `{ id }` | `IdentityAccount` |
| `upsertidentityaccount` | `IdentityAccount` (id optional → server generates UUID) | `IdentityAccount` (with id) |
| `deleteidentityaccount` | `{ id }` | `null` |
| `linkagentidentity` | `{ agent_id, account_id, provider }` | `null` |
| `unlinkagentidentity` | `{ agent_id, provider }` | `null` |
| `listagentidentities` | `{ agent_id }` | `ForgeAgentIdentity[]` (joined with account names for convenience) |
| `listagentinstances` | `{ definition_id?, status? }` | `AgentInstance[]` |
| `createagentinstance` | `{ definition_id, block_id?, parent_instance_id? }` | `AgentInstance` |
| `updateagentinstance` | `{ id, status?, session_id?, block_id?, github_context?, ended_at? }` | `AgentInstance` |
| `deleteagentinstance` | `{ id }` | `null` |
| `forkagentdefinition` | `{ source_id, branch_label }` | `ForgeAgent` (the new branched definition) |
| `exportagentbundle` | `{ agent_id, include_dev_secrets?: bool }` | `{ bundle: <gzip-base64> }` |
| `importagentbundle` | `{ bundle: <gzip-base64> }` | `ForgeAgent` (new agent + identity rows created) |

### Validation rules

- `upsertidentityaccount`: enforce `provider` ∈ allowed set; enforce `kind` consistent with `secret_ref.backend`.
- `linkagentidentity`: must respect the `(agent_id, provider)` PK — replace existing link rather than error.
- `createagentinstance`: must reference an existing definition; `parent_instance_id` if set must reference an existing instance.
- `forkagentdefinition`: copies the source's `content` blobs and skills verbatim, sets `parent_id = source_id`, generates a new id and slug (e.g. `<source_slug>-<branch_label>`).

---

## Phase 4 — Frontend identity model

### Files

- `frontend/app/view/identity/identity-model.ts` — swap `localStorage` reads/writes for `RpcApi.UpsertIdentityAccountCommand` / `ListIdentityAccountsCommand` / etc.
- `frontend/app/view/identity/identity-model.test.ts` — update tests; mock `RpcApi` instead of `localStorage`.
- `frontend/types/gotypes.d.ts` — add TS types matching Rust structs (`IdentityAccount`, `SecretRef`, `AgentInstance`, `InstanceStatus`, `GitHubContext`, `ForgeAgentIdentity`).
- `frontend/app/store/rpc-api.ts` — bind the new commands.

### Notes

- The identity panel's load path becomes async (`await listIdentityAccounts()`) — most call sites are already `async`. Add a loading skeleton if any aren't.
- Drop all `localStorage.getItem("agentmux-identity-accounts")` and friends in one sweep; grep the frontend tree to catch every site.

---

## Phase 5 — Frontend agent launch wiring

### Files

- `frontend/app/view/agent/agent-model.ts` — in `launchAgent`, after the block has its CLI metadata set, call `createAgentInstance({ definition_id: agentId, block_id: this.blockId })` and stash the returned `instance.id` into block meta as `agentInstanceId`.
- Hook controller status changes (`controllerstatus` events) to call `updateAgentInstance` with new `status`.
- On agent stop / pane close, call `updateAgentInstance({ id, status: "stopped", ended_at: now() })`.

### Edge cases

- Pane closed without graceful stop → background reconciler marks orphaned instances as `crashed` after a heartbeat-loss timeout (defer to a follow-up; not required for v1).
- Multiple panes for the same definition → each gets its own instance row. The bus already handles broadcast targeting; surface `instance_id` as an addressable channel.

---

## Phase 6 — Branching UI

### Files

- `frontend/app/view/agent/components/AgentDefinitionMenu.tsx` (or wherever the agent context menu lives) — add a "Fork agent…" action that prompts for `branch_label`, calls `forkAgentDefinition`, then sets the current pane's block to the new definition.
- New `AgentLineageView` component — small tree visualisation showing parent → branched children for the focused definition. Renderable in the forge panel.

### Notes

- Forking does not auto-launch — user picks a pane to launch the new branch into.
- Lineage queries can be O(n) walks of `parent_id` chains; cap depth at 50 to avoid pathological loops.

---

## Phase 7 — Bundle export/import UI

### Files

- `frontend/app/view/identity/IdentityPanel.tsx` (or forge panel) — "Export agent bundle" button → `exportagentbundle` → save via `getApi().saveFile()`. "Import bundle…" → file picker → base64 → `importagentbundle`.
- `frontend/app/view/identity/BundleImportPreview.tsx` — show what the bundle contains BEFORE confirming import. Highlight any identity references that don't resolve in the current install (env var not set, etc.).

### Bundle format

Per design spec; one addition: include a `schema_version: 1` field at the root so we can evolve the format without breaking importers.

```jsonc
{
  "schema_version": 1,
  "exported_at": "2026-04-20T22:00:00Z",
  "agentmux_version": "0.33.300",
  "agent": { /* ForgeAgent + content + skills */ },
  "identities": [ /* IdentityAccount[] minus secret values unless --include-dev-secrets */ ],
  "instance_summary": {
    "total_instances": 0,
    "last_active": null,
    "github_prs_touched": []
  }
}
```

Bundle is gzip-compressed JSON encoded as base64 in the RPC payload (small enough to wire-transfer; large bundles are still <100KB).

---

## Out of scope (defer)

- Migration from existing localStorage identities — we're starting fresh.
- Multi-parent instance lineage (DAG) — current tree-only `parent_instance_id` is enough.
- Cross-machine instance sync — instances are local to the AgentMux DB, by design.
- Bundle signing / verification — bundles are user-trusted artefacts for now; revisit if we ever ship a marketplace.

---

## Test plan

### Phase 1-3 (backend)

- Migration test: empty DB → run migration → all four new tables present, indexes created, foreign keys enforced.
- Round-trip: `upsertidentityaccount` → `getidentityaccount` returns identical struct.
- Cascade delete: deleting a `ForgeAgent` removes its junction rows AND its `AgentInstance` rows.
- `forkagentdefinition`: source's content/skills are deep-copied (modifying the fork doesn't mutate source).
- `exportagentbundle` round-trip: export → import into a clean DB → identical agent reappears.

### Phase 4-5 (frontend)

- Identity panel CRUD: create/edit/delete an account, refresh page, verify persistence.
- Launch agent: row appears in `db_agent_instances` with correct `block_id`/`session_id`.
- Stop agent: row's `status` flips to `stopped` with `ended_at` set.
- Two panes, same definition: two instance rows.

### Phase 6-7 (UI)

- Fork agent → new definition appears in picker with branch_label badge.
- Export bundle → save to file → import into different portable extract → agent reappears.

---

## Affected files (summary)

| File | Phase | Change |
|---|---|---|
| `agentmux-srv/src/backend/storage/migrations.rs` | 1 | Add migration v6 |
| `agentmux-srv/forge-seed.json` | 1 | Drop `accounts` field, add `parent_id`/`branch_label` (null) |
| `scripts/gen-seed.js` | 1 | Match seed schema |
| `agentmux-srv/src/backend/storage/wstore.rs` | 2 | New CRUD methods, struct definitions |
| `agentmux-srv/src/backend/obj.rs` | 2 | New structs (`IdentityAccount`, `AgentInstance`, `ForgeAgentIdentity`); update `ForgeAgent` |
| `agentmux-srv/src/backend/rpc_types.rs` | 3 | Command constants + payload structs |
| `agentmux-srv/src/server/forge_handlers.rs` | 3 | Implement handlers (or split into `identity_handlers.rs` + `instance_handlers.rs`) |
| `agentmux-srv/src/server/websocket.rs` | 3 | Register handlers |
| `frontend/app/view/identity/identity-model.ts` | 4 | Swap localStorage for RPC |
| `frontend/app/view/identity/identity-model.test.ts` | 4 | Update tests |
| `frontend/types/gotypes.d.ts` | 4 | TS types |
| `frontend/app/store/rpc-api.ts` | 4 | Bind new commands |
| `frontend/app/view/agent/agent-model.ts` | 5 | Create/update instance on launch/stop |
| `frontend/app/view/agent/components/AgentDefinitionMenu.tsx` | 6 | Fork action |
| `frontend/app/view/agent/components/AgentLineageView.tsx` | 6 | New component |
| `frontend/app/view/identity/IdentityPanel.tsx` | 7 | Export/import buttons |
| `frontend/app/view/identity/BundleImportPreview.tsx` | 7 | New component |

---

## Rollout

- **PR 1:** Phases 1-3 (backend complete). Merge gates frontend work.
- **PR 2:** Phases 4-5 (frontend identity DB-backing + instance tracking). Bumps version, ships portable.
- **PR 3:** Phase 6 (branching UI).
- **PR 4:** Phase 7 (bundle export/import).

Each PR independently revertible. PR 1 is the riskiest because of the schema drop (`accounts` column); call it out in the PR body and tag `@asaf` for explicit sign-off.
