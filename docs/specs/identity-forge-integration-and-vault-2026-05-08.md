# Agent Composition — Identity + Memory + Vault (PR-F / PR-G)

**Date:** 2026-05-08
**Author:** AgentA-asaf
**Issue:** #678 (continued from Phase 1 + Phase 2)
**Supersedes:** earlier draft of this file (the per-provider Forge-form picker plan)
**Status:** Proposed — rewrites the model after user feedback

---

## The model

Two first-class concepts. Nothing else.

```
Identity  +  Memory  →  Agent Instance
(creds)      (everything that makes the agent be itself —
              CLI choice, model, soul, instructions, context
              files, MCP servers, skills)
```

There is **no separate "agent definition" entity**. A Memory bundle *is* the agent — its provider choice, its instructions, its context, its tools.

### Composition at launch

The Launch Agent modal has **two dropdowns**:

- **Identity** — pick a credential bundle (or the blank default = ambient creds).
- **Memory** — pick a personality / capability bundle (or the blank default = vanilla CLI with no instructions).

Both dropdowns always have a "blank" option at the top. If the user does nothing, the instance launches with both blank — the equivalent of a plain unconfigured CLI session. No selection blocks the launch.

### Forge becomes Memory; both Identity and Memory are first-class panes

There is no `Forge` concept anywhere — no entity, no tab, no widget. **Forge becomes Memory.** Existing `db_forge_agents` rows migrate into `db_memories`.

**Identity and Memory each become their own standalone pane types** (block views), surfaced by widgets:

- `defwidget@identity` — opens the Identity management pane.
- `defwidget@memory` — opens the Memory management pane.

You **go into** the Identity pane to manage credential bundles and the accounts inside them. You **go into** the Memory pane to manage personality/capability bundles (provider, model, instructions, context files, MCPs). They're not tabs inside the agent pane — they're top-level panes you can open in any tab, split alongside terminals, etc., because they're reusable across instances.

This reverses PR-D's docs claim that "Forge and Identity are not standalone pane types — they're tabs inside the Agent pane." That was true for the previous model. The new model promotes Identity and Memory to first-class panes; the agent pane no longer carries Forge or Identity tabs.

---

## Schema

### New tables

```sql
db_identities (
  id              TEXT PRIMARY KEY,
  name            TEXT NOT NULL UNIQUE,    -- "Work", "Personal", "Demo"
  description     TEXT,
  is_blank        INTEGER NOT NULL DEFAULT 0,  -- 1 for the singleton blank row
  created_at      TEXT NOT NULL,
  updated_at      TEXT NOT NULL
)

db_identity_bindings (
  identity_id     TEXT NOT NULL,
  account_id      TEXT NOT NULL,
  provider        TEXT NOT NULL,
  PRIMARY KEY (identity_id, provider),
  FOREIGN KEY (identity_id) REFERENCES db_identities(id) ON DELETE CASCADE,
  FOREIGN KEY (account_id)  REFERENCES db_identity_accounts(id) ON DELETE CASCADE
)
-- The junction is named `db_identity_bindings` (not `db_identity_accounts`) so it
-- doesn't collide with the v6 individual-credentials table that the agent
-- pane's Identity tab already uses. The FK target for account_id is the
-- existing `db_identity_accounts` table — we don't introduce a new
-- `db_accounts` table.

db_memories (
  id              TEXT PRIMARY KEY,
  name            TEXT NOT NULL UNIQUE,    -- "Claude-coder", "Codex-reviewer"
  description     TEXT,
  is_blank        INTEGER NOT NULL DEFAULT 0,
  provider        TEXT,                    -- "claude" | "codex" | "gemini" | NULL
  model           TEXT,                    -- "claude-sonnet-4-6", etc.
  instructions    TEXT,                    -- system prompt / soul
  context_files   TEXT,                    -- JSON array of {path, content}
  mcp_servers     TEXT,                    -- JSON array of MCP server configs
  skills          TEXT,                    -- JSON array of skill refs
  created_at      TEXT NOT NULL,
  updated_at      TEXT NOT NULL
)
```

The `is_blank` flag identifies the singleton blank row that's seeded on migration. The launch UI always renders the blank row first in each dropdown.

### Modified tables

```sql
-- Replace PR #745's JSON column with single FKs:
ALTER TABLE db_agent_instances ADD COLUMN identity_id TEXT REFERENCES db_identities(id);
ALTER TABLE db_agent_instances ADD COLUMN memory_id   TEXT REFERENCES db_memories(id);
ALTER TABLE db_agent_instances DROP   COLUMN identities;  -- the v7 JSON column

-- Drop the Forge concept entirely:
DROP TABLE db_forge_agent_identities;  -- v6 junction
DROP TABLE db_forge_agents;             -- after migration into db_memories
```

### Migration

New migration v8 runs in this order:

1. Create `db_identities`, `db_identity_bindings`, `db_memories`.
2. Insert the singleton blank Identity (`name = '__blank__'`, `is_blank = 1`).
3. Insert the singleton blank Memory (`name = '__blank__'`, `is_blank = 1`).
4. Migrate existing data:
   - For each row in `db_forge_agents`: insert a corresponding row in `db_memories`. Map `db_forge_agents.provider` → `db_memories.provider`, etc.
   - For each row in `db_forge_agent_identities` for a given `agent_id`: create a per-agent Identity named `<agent_name>-default` and insert the per-provider account rows.
5. Add `identity_id` + `memory_id` columns to `db_agent_instances`. Backfill from the migrated identities/memories.
6. Drop the old JSON column and the Forge tables.

The migration is one-way (no rollback). Run order matters because step 5 reads from rows created in step 4.

---

## UI changes

### Launch Agent modal

Replace PR #745's per-provider `<select>` rows with two single dropdowns:

```
┌────────────────────────────────────────────────┐
│ New Agent Instance                              │
│  Name:         [my-instance______]              │
│  Runtime:      [local | container]              │
│                                                 │
│  Identity:     [▼ — Blank (no creds) —    ]     │
│  Memory:       [▼ — Blank (vanilla CLI) — ]     │
│                                                 │
│  [Advanced ▸]                                   │
│                                                 │
│  [Cancel]              [Launch]                 │
└────────────────────────────────────────────────┘
```

The dropdowns list user-defined Identities/Memories first, then the blank singleton at the bottom (or vice versa — TBD). Clicking the blank option means "no override; spawn vanilla."

### Identity pane (`view: "identity"`)

A first-class block-view pane. Opens via `defwidget@identity` (right-click a pane header → Identity, or click the identity widget icon if pinned, or `pane.open` RPC with `view: "identity"`).

Layout:

- Left rail: list of Identity bundles. `+ New Identity` button at top.
- Right side: detail view for the selected Identity — name, description, accounts table (one row per provider). Add/edit/remove accounts inline.
- Header actions: **Export Vault**, **Import Vault** buttons (PR-G).

### Memory pane (`view: "memory"`)

A first-class block-view pane. Opens via `defwidget@memory`. Same shape as Identity:

- Left rail: list of Memory bundles. `+ New Memory` button at top.
- Right side: detail view — name, description, provider/CLI dropdown, model dropdown, instructions textarea, context-files manager, MCP-servers list, skills list.

### Agent pane

The Forge tab and the Identity tab both disappear from the agent pane's settings panel. The agent pane gets simpler — it focuses on the running agent's stream, tool calls, and turn state. Identity and Memory configuration moves out to the dedicated panes above.

### Hamburger-menu shortcuts (v2)

For convenience, the hamburger menu can grow `Open Identities` and `Open Memories` entries that open the respective panes in a new pane. v1 ships without them; users right-click and pick from the widget menu.

---

## Vault

The vault concern is Identity-side only. Memory bundles don't carry secrets — they're shareable as plain JSON.

### Identity Vault

Two buttons on the Identity tab:

**Export Vault**
- File: `agentmux-vault-<timestamp>.agentmux-vault`.
- Contents: all `db_identities` rows + their `db_identity_bindings` rows + the referenced `db_identity_accounts` rows + resolved `secret_ref` plaintext values.
- Crypto: Argon2id-derived key (params: m=64MB, t=3, p=4 — OWASP 2023 minimums), AES-256-GCM, random 12-byte nonce. Header: magic + version + salt + nonce.
- Passphrase prompt every time. No caching.

**Import Vault**
- File picker → passphrase → decrypt.
- Per-account collision policy: skip-on-conflict default, "merge & overwrite" toggle.
- Returns summary: `{ identities_imported, accounts_imported, skipped, overwritten, errors }`.

### Memory Vault (later)

Memory bundles can be exported/imported as plain JSON (no secrets). Skipping for v1; users can manually copy a JSON dump until then.

### Failure modes

- Wrong passphrase → `BadPassphrase` error, no partial decrypt.
- Corrupted blob → `Corrupted { byte_offset }`.
- Schema version mismatch → `IncompatibleVersion { expected, got }`.

---

## Coordination with PR #745

PR #745 has the right *moment* (instantiation) but the wrong *shape* (per-provider JSON, Phase-1 default fallback through `db_forge_agent_identities`). The new model needs:

- Drop the JSON column, add `identity_id` / `memory_id` FKs on `db_agent_instances`.
- Drop the per-provider picker UI; replace with two dropdowns.
- The resolver still does the secret → env-var work, just keyed off `identity_id` instead of the JSON array.

Two paths:

1. **Re-shape PR #745** — keep the branch, swap the model. Preserves AgentY's resolver work + tests; rewrites the UI + persistence layer.
2. **Close PR #745, open fresh** — clean slate. Loses the resolver work as a separate review unit.

Recommend (1) — re-shape. The resolver + provider-matrix + spawn-injection tests are reusable. We'd ask AgentY to re-shape, or do it ourselves.

---

## PR splitting

| PR | Scope | Stacked on | Independent? |
|---|---|---|---|
| **PR-F.0** | Schema migration v8 + blank singletons + reshape PR #745's resolver to read from new tables | `feat/identity-injection-678` (re-shaped) or main | Yes — backend-only, no UI change |
| **PR-F.1** | Memories management UI (rename Forge tab, CRUD on `db_memories`) | F.0 | Yes |
| **PR-F.2** | Identity management UI updates (named bundles, Account CRUD inside an Identity) | F.0 | Yes |
| **PR-F.3** | Launch modal: replace per-provider rows with two dropdowns | F.0 + F.1 + F.2 | Builds on UI |
| **PR-G** | Vault export/import | F.0 + F.2 | Yes |

Recommended ship order: F.0 → F.2 → F.1 → F.3 → G. F.0 first because it lays the schema; F.2 before F.1 because Identity is the simpler one to validate the named-bundle pattern.

---

## Open questions

1. **Re-shape PR #745, or close it?** I'd vote re-shape.
2. **Memory rename for the existing tab?** "Memories" feels right but "Personalities" / "Brains" / "Loadouts" are alternatives. Confirm Memories.
3. **Hamburger-menu promotion timing**: ship in v1 (this PR cycle) or as a follow-up? I'd vote follow-up — agent-pane tabs work fine for now.
4. **Vault crypto parameters**: Argon2id m=64MB t=3 p=4 — bump if you want stronger? These are OWASP 2023 minimums.
5. **Migration safety**: existing `db_forge_agents` and `db_forge_agent_identities` rows get auto-migrated. Are there any production users we need to gate this for, or is the dev installed-base small enough to ship without a migration flag? (My read: small, ship without flag.)

---

## Out of scope (this cycle)

- Phase 3 OAuth flows (separate PRs per provider).
- AWS multi-secret expansion.
- Cross-machine vault sync (this is local export/import only).
- Memory vault encryption — Memories don't carry secrets.
- Hamburger-menu promotion of Identity/Memory.
- A `Guardrails` concept (folded into Memory; can split out later if security users need it).

---

## Files this will touch

### PR-F.0 (schema + resolver reshape)

- `agentmux-srv/src/backend/storage/migrations.rs` — v8 migration
- `agentmux-srv/src/backend/storage/wstore.rs` — new accessors for db_identities, db_memories, blank singletons
- `agentmux-srv/src/identity/resolver.rs` — read from new tables
- `agentmux-srv/src/identity/mod.rs` — `inject_identity_env` reads `identity_id` instead of JSON
- `agentmux-srv/src/server/app_api.rs` — RPC commands for Identity/Memory CRUD
- `agentmux-srv/src/backend/rpc_types.rs` — new command types
- `frontend/types/gotypes.d.ts` — types

### PR-F.1 (Memory pane)

- `frontend/app/view/memory/memory-model.ts` — new (`MemoryViewModel`)
- `frontend/app/view/memory/memory-view.tsx` — list + detail
- `frontend/app/view/memory/memory.tsx` — barrel
- `frontend/app/view/memory/memory-view.scss`
- `frontend/app/block/block.tsx` — `BlockRegistry.set("memory", MemoryViewModel)`
- `agentmux-srv/src/config/widgets.json` — `defwidget@memory`
- `frontend/app/view/forge/` — directory removed; salvageable components migrated into the new `memory/` module
- `frontend/app/view/agent/components/AgentCardSettingsPanel.tsx` — drop the Forge tab

### PR-F.2 (Identity pane)

- `frontend/app/view/identity/identity-model.ts` — promote to first-class pane (it currently lives only as agent-pane components per PR-E); add Identity entity types (vs. Account-only)
- `frontend/app/view/identity/identity-view.tsx` — left-rail list + right-detail layout, replacing the agent-pane tab content
- `frontend/app/view/identity/identity.tsx` — barrel
- `frontend/app/block/block.tsx` — `BlockRegistry.set("identity", IdentityViewModel)`
- `agentmux-srv/src/config/widgets.json` — `defwidget@identity`
- `frontend/app/view/agent/components/AgentIdentityPanel.tsx` — drop (content moves to the Identity pane)
- `frontend/app/view/agent/components/AgentCardSettingsPanel.tsx` — drop the Identity tab

### PR-F.3 (Launch modal)

- `frontend/app/view/agent/components/AgentLaunchModal.tsx` — replace per-provider rows with two dropdowns
- `frontend/app/view/agent/agent-model.ts` — `LaunchOverrides` shape change

### PR-G (Vault)

- `agentmux-srv/Cargo.toml` — `argon2`, `aes-gcm` deps
- `agentmux-srv/src/identity/vault.rs` — new module
- `agentmux-srv/src/server/app_api.rs` — register vault commands
- `frontend/app/view/agent/components/AgentIdentityPanel.tsx` — Export/Import buttons
