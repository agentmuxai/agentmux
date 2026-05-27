# SPEC: objects.db Schema Flattening + De-Forge Rename

**Status:** Draft — awaiting approval
**Author:** AgentA
**Date:** 2026-05-19
**Supersedes / absorbs:**
- AUDIT_SQLITE_SYSTEMS §8.5 (`PRAGMA user_version` discipline) — folded in, see §8.
- PR #933 (`run_forge_v11_migrations`) — the v11 rename is absorbed into the flat
  schema; v11 is not reverted, it is collapsed along with v1–v10.
**Companion docs:**
- [`AUDIT_SQLITE_SYSTEMS_2026_05_19.md`](./AUDIT_SQLITE_SYSTEMS_2026_05_19.md) — the file inventory this spec builds on.
- [`SPEC_SHARED_BUNDLES_AND_DEFINITIONS_2026_05_19.md`](./SPEC_SHARED_BUNDLES_AND_DEFINITIONS_2026_05_19.md) — proposes a 5th file; unaffected by this spec.

---

## 0. TL;DR

`objects.db` is built by an 11-step incremental migration chain (`run_forge_v1`
… `run_forge_v11`). Because each AgentMux version gets its own data dir
(`~/.agentmux/versions/<v>/data/db/objects.db`), a new version is always born
with a **fresh** `objects.db` and runs the entire chain top-to-bottom in one
shot to build the final schema. The incremental steps never do incremental work
in production — the chain is pure historical accretion.

This spec:

1. **Flattens** the chain into a single `CREATE TABLE` set defining the final
   schema directly.
2. **De-forges** the table + type names — `forge` is dead vocabulary (replaced
   by Memory / Identity / agent-definition).
3. **Drops** four genuinely-dead tables retained only for an abandoned
   downgrade path.
4. Adds a **`PRAGMA user_version` tripwire** to all four SQLite files
   (closes AUDIT §8.5).
5. Keeps **one** tiny idempotent "adopt legacy" step as a safety net for
   dev databases that predate the flatten.

Net deletion: ~1,400 lines of migration code + tests. Net schema change: zero
(the flat schema is effect-equivalent to the post-v11 migrated schema, minus
the four dead tables).

**This abandons the v1→v11 incremental upgrade path.** Per-version data dirs
make that safe — see §3.

---

## 1. Motivation

- **The chain is cruft.** v1–v11 encode a year of schema evolution. In
  production not one incremental step ever runs incrementally: a fresh
  per-version `objects.db` runs all 11 in sequence to assemble the final
  schema. The intermediate states are unreachable.
- **`forge` is dead vocabulary.** The "Forge" feature was superseded by the
  Memory / Identity bundle model. `db_forge_agents` is now simply the agent
  *definition* table; `db_forge_content` / `_skills` / `_history` are the
  agent's content blobs. The word survives only because renaming a live table
  through an incremental chain is painful. Flattening removes that friction —
  see §3.
- **No version discipline.** There is no `PRAGMA user_version`; migrations
  rely entirely on `CREATE TABLE IF NOT EXISTS` + swallowed
  `ALTER TABLE … duplicate-column` errors. AUDIT §8.5 flagged this. PR #933's
  Codex P1 (the downgrade-then-re-upgrade data-stranding hole) was a direct
  symptom: nothing recorded "this DB is already at the new schema."
- **Dead weight on disk.** `db_workflow_definitions` / `db_workflow_runs` (v9)
  and the two `db_v10_migrated_legacy_*` sentinel tables exist only to keep a
  downgrade path open. They are read by nothing outside `migrations.rs`.

---

## 2. Verified current schema

> **Audit method.** An initial delegated sub-agent audit was **discarded** — it
> wrongly flagged the generic object tables and the entire forge
> content/skills/history subsystem as dead (it grepped literal table-name
> strings and missed both the dynamic `format!("db_{}", otype)` table names and
> the `forge_handlers.rs` RPC layer). The inventory below is reconstructed
> directly from the migration DDL in `migrations.rs` and verified against every
> `FROM` / `INTO` / `UPDATE` / `DELETE` / `JOIN` call site.

### 2.1 `objects.db` — LIVE tables (kept)

| Table | Created | Notes |
|---|---|---|
| `db_client`, `db_window`, `db_workspace`, `db_tab`, `db_layout`, `db_block`, `db_temp` | wstore | Generic WaveObj store; accessed via `format!("db_{}", otype)` in `WaveStore::get/set/delete`. **Live** — backs windows/tabs/blocks. |
| `db_forge_agents` | v1 (+cols v2–v6) | Agent **definition** table. `db_agent_instances.definition_id` FKs into it. CRUD in `wstore.rs`, RPC in `forge_handlers.rs`. |
| `db_forge_content` | v2 | Per-agent content blobs (soul, agentmd, mcp, env, memory). RPC: `get/set forgecontent`. |
| `db_forge_skills` | v2 | Reusable agent skills. RPC: `create/update/delete forgeskill`. |
| `db_forge_history` | v2 | Append-only session logs. RPC: `append/list/search forgehistory`. |
| `db_identity_accounts` | v6 | Provider OAuth/API-key accounts. |
| `db_forge_agent_identities` | v6 | Junction: agent ↔ account ↔ provider. v6 doc calls it "deprecated" but `forge_handlers.rs` `agent_identity_link/unlink/list` keep it **live**. |
| `db_agent_instances` | v6 (+cols v7, v8) | One row per running/historical agent execution. |
| `db_identity_bundles` | v7 as `db_identities`, renamed v11 | Named identity bundles. |
| `db_identity_bindings` | v7 | Junction: identity bundle ↔ account ↔ provider. |
| `db_memory_bundles` | v7 as `db_memories`, renamed v11 | Named memory bundles. |
| `db_drone_definitions` | v10 | Drone DAG definitions. |
| `db_drone_runs` | v10 | Drone run history. |

### 2.2 `objects.db` — DEAD tables (dropped)

| Table | Created | Why dead |
|---|---|---|
| `db_workflow_definitions` | v9 | Superseded by `db_drone_definitions` (v10). Referenced only inside `migrations.rs`. |
| `db_workflow_runs` | v9 | Superseded by `db_drone_runs`. Referenced only inside `migrations.rs`. |
| `db_v10_migrated_legacy_defs` | v10 | Sentinel gating the one-time v9→v10 copy. Pointless once v9 tables are gone. |
| `db_v10_migrated_legacy_runs` | v10 | Same. |

Verified: `grep -rn 'db_workflow_\|db_v10_migrated_legacy'` matches **only**
`migrations.rs`.

### 2.3 The other three SQLite files (schema unchanged)

| File | Tables | Migration shape today |
|---|---|---|
| `filestore.db` | `db_wave_file`, `db_file_data` | Single `run_filestore_migrations` DDL — already flat. |
| `sagas.db` | `saga`, `saga_step` | Single `run_saga_log_migrations` DDL — already flat. |
| `launcher-sagas.db` | `launcher_saga`, `launcher_saga_step` | Single `schema::DDL` const — already flat. |

These three have **no chain to flatten**. They receive only the `user_version`
tripwire (§8).

---

## 3. Decision: abandon the incremental upgrade path

Flattening means a build with the flat schema **cannot reconstruct an
arbitrary old-version `objects.db`** — the v1→v10 ladder that would upgrade,
say, a v5-era DB is gone.

This is safe because **data dirs are per-version**
(`~/.agentmux/versions/<v>/data/db/`, confirmed in CLAUDE.md and AUDIT §1). A
released flattened version `V` opens `versions/V/data/db/objects.db`, which is
always created fresh by `V` itself. It never inherits an older version's DB.

The only databases that realistically predate the flatten are **dev
databases** — a developer running `task dev` repeatedly across the
v11→flatten transition reuses one dev data dir. Those DBs are all at the
**post-v11** schema (v11 is already merged). §7 keeps a one-step safety net
for exactly that case.

**Window of safety.** This must land *before* the data-dir-unification work
(see `reference_data_dir_unification_plan`), which would share one DB across
versions and reintroduce a real upgrade requirement. Flatten now.

---

## 4. Non-goal: consolidating the four SQLite files

We will **not** merge `objects.db` / `filestore.db` / `sagas.db` /
`launcher-sagas.db` into one file. The split is principled:

- **`launcher-sagas.db`** is written by the **launcher process** — a separate
  binary with its own lifecycle (it owns Job Object J0 and spawns srv).
  Merging it into a srv-owned file would couple the launcher to srv's schema
  and force cross-process WAL/lock contention. Launcher-side durability must
  survive even if srv never starts.
- **`filestore.db`** is a large, high-churn BLOB store. Co-locating BLOB pages
  with `objects.db`'s small hot metadata pollutes the shared page cache and
  bloats the WAL.
- **`sagas.db`** is ephemeral runtime state (truncated on saga completion).
  Keeping it separate gives recovery isolation — a corrupt saga log can be
  deleted without risking durable user content.

The split mirrors AUDIT §0's three persistence classes. The only consolidation
the existing specs contemplate is the *opposite* direction
(SPEC_SHARED_BUNDLES adds a 5th file).

---

## 5. The flat `objects.db` schema

All DDL below is applied by a single `run_object_schema(conn)` function (new
name — see §6) via `conn.execute_batch`. Every statement is
`CREATE TABLE IF NOT EXISTS` / `CREATE INDEX IF NOT EXISTS`, so re-running on an
already-initialised DB is a no-op. Parent tables precede child tables so FK
targets exist at creation time.

Column order is cosmetic — every read/write site uses an explicit column list,
never `SELECT *` or positional inserts without a column list.

### 5.1 Generic object tables

```sql
-- one per otype: client, window, workspace, tab, layout, block, temp
CREATE TABLE IF NOT EXISTS db_<otype> (
    oid     TEXT PRIMARY KEY,
    version INTEGER NOT NULL DEFAULT 1,
    data    TEXT NOT NULL
);
```

### 5.2 Agent definitions (was `db_forge_agents`)

```sql
CREATE TABLE IF NOT EXISTS db_agent_definitions (
    id                   TEXT PRIMARY KEY,
    slug                 TEXT NOT NULL DEFAULT '',
    name                 TEXT NOT NULL,
    icon                 TEXT NOT NULL DEFAULT '✦',
    provider             TEXT NOT NULL,
    description          TEXT NOT NULL DEFAULT '',
    working_directory    TEXT NOT NULL DEFAULT '',
    shell                TEXT NOT NULL DEFAULT '',
    provider_flags       TEXT NOT NULL DEFAULT '',
    auto_start           INTEGER NOT NULL DEFAULT 0,
    restart_on_crash     INTEGER NOT NULL DEFAULT 0,
    idle_timeout_minutes INTEGER NOT NULL DEFAULT 0,
    agent_type           TEXT NOT NULL DEFAULT 'standalone',
    environment          TEXT NOT NULL DEFAULT '',
    agent_bus_id         TEXT NOT NULL DEFAULT '',
    is_seeded            INTEGER NOT NULL DEFAULT 0,
    accounts             TEXT NOT NULL DEFAULT '',
    parent_id            TEXT NOT NULL DEFAULT '',
    branch_label         TEXT NOT NULL DEFAULT '',
    created_at           INTEGER NOT NULL DEFAULT 0
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_agent_definitions_slug
    ON db_agent_definitions(slug);
```

> **`accounts` is kept** (decision §12-D, revised). A v6 doc comment called it
> "deprecated, superseded by the identity-links junction," and an early draft
> of this spec proposed dropping it. **That was wrong** — Codex review of
> PR #934 verified the column is live: the Agent pane's Identity tab
> (`AgentIdentityPanel`) writes per-provider account assignments into it as a
> JSON blob via `updateforgeagent`, `parseAgentAccounts` reads it back, and
> startup credential resolution depends on it. The deprecation never
> completed. A flatten must be behaviour-preserving, so the column stays.

### 5.3 Agent content / skills / history (was `db_forge_*`)

```sql
CREATE TABLE IF NOT EXISTS db_agent_content (
    agent_id     TEXT NOT NULL,
    content_type TEXT NOT NULL,
    content      TEXT NOT NULL DEFAULT '',
    updated_at   INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (agent_id, content_type),
    FOREIGN KEY (agent_id) REFERENCES db_agent_definitions(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS db_agent_skills (
    id          TEXT PRIMARY KEY,
    agent_id    TEXT NOT NULL,
    name        TEXT NOT NULL,
    trigger     TEXT NOT NULL DEFAULT '',
    skill_type  TEXT NOT NULL DEFAULT 'prompt',
    description TEXT NOT NULL DEFAULT '',
    content     TEXT NOT NULL DEFAULT '',
    created_at  INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (agent_id) REFERENCES db_agent_definitions(id) ON DELETE CASCADE
);

CREATE TABLE IF NOT EXISTS db_agent_history (
    id           INTEGER PRIMARY KEY AUTOINCREMENT,
    agent_id     TEXT NOT NULL,
    session_date TEXT NOT NULL,
    entry        TEXT NOT NULL,
    timestamp    INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (agent_id) REFERENCES db_agent_definitions(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_agent_history_agent_date
    ON db_agent_history(agent_id, session_date);
```

### 5.4 Identity accounts + agent↔account links

```sql
CREATE TABLE IF NOT EXISTS db_identity_accounts (
    id           TEXT PRIMARY KEY,
    name         TEXT NOT NULL,
    provider     TEXT NOT NULL,
    kind         TEXT NOT NULL,
    display_name TEXT NOT NULL DEFAULT '',
    secret_ref   TEXT NOT NULL,
    context      TEXT NOT NULL DEFAULT '{}',
    status       TEXT NOT NULL DEFAULT 'unknown',
    created_at   INTEGER NOT NULL DEFAULT 0,
    updated_at   INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_identity_accounts_provider
    ON db_identity_accounts(provider);

-- was db_forge_agent_identities
CREATE TABLE IF NOT EXISTS db_agent_identity_links (
    agent_id   TEXT NOT NULL,
    account_id TEXT NOT NULL,
    provider   TEXT NOT NULL,
    PRIMARY KEY (agent_id, provider),
    FOREIGN KEY (agent_id)   REFERENCES db_agent_definitions(id) ON DELETE CASCADE,
    FOREIGN KEY (account_id) REFERENCES db_identity_accounts(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_agent_identity_links_account
    ON db_agent_identity_links(account_id);
```

### 5.5 Identity + Memory bundles

```sql
CREATE TABLE IF NOT EXISTS db_identity_bundles (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL UNIQUE,
    description TEXT NOT NULL DEFAULT '',
    is_blank    INTEGER NOT NULL DEFAULT 0,
    created_at  INTEGER NOT NULL DEFAULT 0,
    updated_at  INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_identity_bundles_is_blank
    ON db_identity_bundles(is_blank);

CREATE TABLE IF NOT EXISTS db_identity_bindings (
    identity_id TEXT NOT NULL,
    provider    TEXT NOT NULL,
    account_id  TEXT NOT NULL,
    PRIMARY KEY (identity_id, provider),
    FOREIGN KEY (identity_id) REFERENCES db_identity_bundles(id)  ON DELETE CASCADE,
    FOREIGN KEY (account_id)  REFERENCES db_identity_accounts(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_identity_bindings_account
    ON db_identity_bindings(account_id);

CREATE TABLE IF NOT EXISTS db_memory_bundles (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL UNIQUE,
    description   TEXT NOT NULL DEFAULT '',
    is_blank      INTEGER NOT NULL DEFAULT 0,
    provider      TEXT NOT NULL DEFAULT '',
    model         TEXT NOT NULL DEFAULT '',
    instructions  TEXT NOT NULL DEFAULT '',
    context_files TEXT NOT NULL DEFAULT '[]',
    mcp_servers   TEXT NOT NULL DEFAULT '[]',
    skills        TEXT NOT NULL DEFAULT '[]',
    created_at    INTEGER NOT NULL DEFAULT 0,
    updated_at    INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_memory_bundles_is_blank
    ON db_memory_bundles(is_blank);
```

The two blank singleton rows (`id='blank'`) are seeded after the DDL, exactly
as v7 did, via `INSERT OR IGNORE`.

### 5.6 Agent instances

```sql
CREATE TABLE IF NOT EXISTS db_agent_instances (
    id                 TEXT PRIMARY KEY,
    definition_id      TEXT NOT NULL,
    parent_instance_id TEXT NOT NULL DEFAULT '',
    block_id           TEXT NOT NULL DEFAULT '',
    session_id         TEXT NOT NULL DEFAULT '',
    status             TEXT NOT NULL DEFAULT 'running',
    github_context     TEXT NOT NULL DEFAULT '',
    identity_id        TEXT NOT NULL DEFAULT '',
    memory_id          TEXT NOT NULL DEFAULT '',
    instance_name      TEXT NOT NULL DEFAULT '',
    working_directory  TEXT NOT NULL DEFAULT '',
    display_hidden     INTEGER NOT NULL DEFAULT 0,
    started_at         INTEGER NOT NULL DEFAULT 0,
    ended_at           INTEGER NOT NULL DEFAULT 0,
    created_at         INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY (definition_id) REFERENCES db_agent_definitions(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_agent_instances_definition
    ON db_agent_instances(definition_id);
CREATE INDEX IF NOT EXISTS idx_agent_instances_block
    ON db_agent_instances(block_id);
CREATE INDEX IF NOT EXISTS idx_agent_instances_status
    ON db_agent_instances(status);
CREATE INDEX IF NOT EXISTS idx_agent_instances_parent
    ON db_agent_instances(parent_instance_id);
CREATE INDEX IF NOT EXISTS idx_agent_instances_name_recent
    ON db_agent_instances(instance_name, started_at DESC)
    WHERE display_hidden = 0 AND instance_name != '';
```

> `identity_id` / `memory_id` are intentionally **not** declared as SQL foreign
> keys (they were added by `ALTER TABLE` in v7, which cannot add FK
> constraints; current code treats `''` as the "blank singleton" sentinel
> rather than NULL). The flat schema preserves that — they stay plain `TEXT`
> columns. *(Open decision §12-E: promote to real FKs.)*

### 5.7 Drone

```sql
CREATE TABLE IF NOT EXISTS db_drone_definitions (
    id          TEXT PRIMARY KEY,
    name        TEXT NOT NULL,
    description TEXT NOT NULL DEFAULT '',
    graph       TEXT NOT NULL DEFAULT '{"nodes":[],"edges":[]}',
    viewport    TEXT NOT NULL DEFAULT '{"x":0,"y":0,"zoom":1}',
    created_at  INTEGER NOT NULL DEFAULT 0,
    updated_at  INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX IF NOT EXISTS idx_drone_definitions_updated
    ON db_drone_definitions(updated_at DESC);

CREATE TABLE IF NOT EXISTS db_drone_runs (
    id           TEXT PRIMARY KEY,
    drone_id     TEXT NOT NULL,
    status       TEXT NOT NULL DEFAULT 'running',
    started_at   INTEGER NOT NULL DEFAULT 0,
    ended_at     INTEGER NOT NULL DEFAULT 0,
    block_states TEXT NOT NULL DEFAULT '{}',
    output       TEXT NOT NULL DEFAULT '',
    error        TEXT NOT NULL DEFAULT '',
    FOREIGN KEY (drone_id) REFERENCES db_drone_definitions(id) ON DELETE CASCADE
);
CREATE INDEX IF NOT EXISTS idx_drone_runs_drone_started
    ON db_drone_runs(drone_id, started_at DESC);
CREATE INDEX IF NOT EXISTS idx_drone_runs_status
    ON db_drone_runs(status);
```

---

## 6. De-forge rename mapping

### 6.1 Tables + indexes

| Old | New |
|---|---|
| `db_forge_agents` | `db_agent_definitions` |
| `db_forge_content` | `db_agent_content` |
| `db_forge_skills` | `db_agent_skills` |
| `db_forge_history` | `db_agent_history` |
| `db_forge_agent_identities` | `db_agent_identity_links` |
| `idx_forge_agents_slug` | `idx_agent_definitions_slug` |
| `idx_forge_history_agent_date` | `idx_agent_history_agent_date` |
| `idx_forge_agent_identities_account` | `idx_agent_identity_links_account` |

### 6.2 Rust internals (renamed with the tables)

- Structs: `ForgeAgent` → `AgentDefinition`, `ForgeContent` → `AgentContent`,
  `ForgeSkill` → `AgentSkill`, `ForgeHistory` → `AgentHistory`,
  `ForgeAgentIdentity` → `AgentIdentityLink`.
- `WaveStore` methods: `forge_list` → `agent_def_list`, `forge_insert` →
  `agent_def_insert`, `forge_get_content` → `agent_content_get`, etc.
- Migration entry point: `run_forge_migrations` → `run_object_schema`;
  module-level free function `run_wstore_migrations` folds into it.
- Files: `server/forge_handlers.rs` → `server/agent_handlers.rs`,
  `backend/forge_seed.rs` → `backend/agent_seed.rs`.

### 6.3 RPC command strings — OPEN DECISION (§12-A)

`forge_handlers.rs` registers ~15 wire command strings (`listforgeagents`,
`createforgeagent`, `setforgecontent`, `createforgeskill`,
`appendforgehistory`, `importforgefromclaw`, …). These are a **frontend-coupled
wire contract**. Two options:

- **A1 — keep the wire strings, rename only Rust-side.** Zero frontend change.
  The `forge` vocabulary survives at exactly one boundary (the IPC command
  name). Lowest risk.
- **A2 — rename the wire strings too**, in a coordinated srv+frontend PR.
  Fully removes `forge`. Larger blast radius; must land srv + frontend
  atomically or the IPC breaks.

**Recommendation: A1 for this spec's PR; A2 as an optional follow-up.** The
table/schema rename — the part that matters for storage clarity — does not
depend on the wire rename.

---

## 7. The single "adopt legacy" safety step

To protect dev databases created by pre-flatten builds (see §3), the flat
`run_object_schema` runs **one** idempotent pre-step before the `CREATE`
batch: `adopt_legacy_table_names(conn)`.

It checks `sqlite_master` for each legacy name and, if the legacy table exists
and its new-named counterpart does not, issues `ALTER TABLE … RENAME TO …`
(plus the index drop/recreate). It covers the **post-v11** legacy names:

```
db_forge_agents            → db_agent_definitions
db_forge_content           → db_agent_content
db_forge_skills            → db_agent_skills
db_forge_history           → db_agent_history
db_forge_agent_identities  → db_agent_identity_links
```

`db_identities` / `db_memories` are *already* renamed to `db_identity_bundles`
/ `db_memory_bundles` by the (now-collapsed) v11 logic; if a dev DB somehow
still has the pre-v11 names, the adopt step renames those too — it is the v11
logic, retained as the single surviving rename.

SQLite ≥ 3.25 auto-updates FK references in child tables when a parent is
renamed, so `db_agent_content` / `_skills` / `_history` / `_identity_links` /
`db_agent_instances` keep their cascades intact.

**Both-tables-present case.** If a legacy table *and* its de-forged
counterpart both exist, the adopt step does **not** rename or drop anything —
it logs a loud `warn!` and leaves the legacy table on disk untouched. This is
only reachable on a deliberate downgrade-roundtrip (flat build → a pre-flatten
build that re-creates the legacy name → flat build again). Silently dropping
the legacy table would be data loss — the exact bug class behind PR #933's
Codex P1 — so the legacy table is preserved for manual recovery and is
otherwise unreferenced by current code.

**Pre-v11 column shape.** The adopt step renames whatever it finds; it does
not pre-flight column sets. A database at a v1–v10 intermediate schema (whose
tables lack later columns) gets renamed, after which the first query
referencing a missing column fails loudly with `no such column` — a hard
error, not silent empty state, with data preserved on disk. Such a DB cannot
exist for a released version (per-version dirs) and is vanishingly unlikely on
a dev machine built since v11 merged.

This one step *is* the flattening: eleven migration functions collapse to one
conditional rename block + the flat `CREATE` batch.

---

## 8. `PRAGMA user_version` tripwire (closes AUDIT §8.5)

Applied to **all four** SQLite files. Each `configure_and_migrate` gains a
trailing call:

```rust
fn stamp_and_check_version(conn: &Connection, current: i64, db_label: &str)
    -> Result<(), StoreError>
{
    let found: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    if found > current {
        // DB written by a NEWER build than this one.
        log::warn!(
            "{db_label}: user_version={found} > {current} — database was \
             written by a newer AgentMux build; proceeding read-compatible \
             but new columns/tables from that build are invisible here"
        );
    }
    conn.execute_batch(&format!("PRAGMA user_version = {current};"))?;
    Ok(())
}
```

Schema-version constants (all start at `1` — the flatten resets the counter;
the pre-flatten chain never set `user_version`, so existing files read `0`):

| File | Constant | Value |
|---|---|---|
| `objects.db` | `OBJECT_SCHEMA_VERSION` | 1 |
| `filestore.db` | `FILESTORE_SCHEMA_VERSION` | 1 |
| `sagas.db` | `SAGA_LOG_SCHEMA_VERSION` | 1 |
| `launcher-sagas.db` | `LAUNCHER_SAGA_SCHEMA_VERSION` | 1 |

**Deliberately a tripwire, not a gate.** The idempotent `CREATE … IF NOT
EXISTS` DDL remains the actual schema mechanism. `user_version` only *records*
the version and *warns* on downgrade. Future real schema changes bump the
constant and the warning threshold; gating migrations on the integer is
explicitly out of scope (AUDIT §8.5 deprioritised it, and idempotent DDL
already works).

---

## 9. What gets deleted

- `run_forge_v1_migrations` … `run_forge_v11_migrations` (11 functions).
- `run_wstore_migrations` (folded into `run_object_schema`).
- The `migrate_through_v7` test helper and all `test_forge_v2..v11_*` tests
  (~15 tests) — replaced by flat-schema + adopt-step tests (§11).
- The four dead tables (§2.2) — never created by the flat schema.
- `WSTORE_OTYPES`-driven loop stays (still the cleanest way to emit the 7
  generic tables).

Estimated deletion: ~1,400 lines across `migrations.rs` (prod + tests).

---

## 10. Call-site impact

The de-forge rename touches every `forge`-named SQL string + identifier:

| File | Approx. sites |
|---|---|
| `backend/storage/store.rs` | ~25 SQL strings + ~17 method renames + 5 struct renames |
| `server/forge_handlers.rs` → `agent_handlers.rs` | ~35 method-call sites |
| `backend/forge_seed.rs` → `agent_seed.rs` | ~8 sites |
| `backend/storage/migrations.rs` | replaced wholesale |
| `backend/rpc_types.rs`, `agents/`, `identity/` | struct-name references |

All are compiler-checked Rust — a missed rename is a build error, not a silent
bug. The SQL strings are the only un-typed surface; §11's full-suite run plus
a smoke test cover them.

---

## 11. Test plan

- **Flat-schema creation.** Open a fresh in-memory `WaveStore`; assert all 19
  live tables + every index exist, and the 4 dead tables do **not**.
- **Idempotency.** `run_object_schema` twice on the same connection — no error,
  no duplicate objects.
- **Adopt step — post-v11 DB.** Build a DB with the legacy `db_forge_*` /
  `db_identity_bundles` names + seeded rows; run `run_object_schema`; assert
  tables renamed, row data preserved, FK cascades intact (delete a definition
  → content/skills/history/instances cascade).
- **Adopt step — fresh DB.** No legacy tables → adopt step is a no-op.
- **Adopt step — pre-v11 reject.** Legacy table with a pre-v11 column set →
  loud error, data left on disk.
- **`user_version`.** After init, `PRAGMA user_version` == the constant;
  opening a DB with a higher value logs the warning and still succeeds.
- **Existing CRUD suites.** All `wstore.rs` / `filestore` / saga tests pass
  against renamed methods + tables (the bulk regression surface).
- **Smoke.** Portable build: create an agent definition + identity + memory,
  restart, confirm they persist; `--diag` opens each DB clean.

---

## 12. Risks & open decisions

- **D-A — RPC wire strings.** §6.3. *Recommend A1* (keep wire names this PR).
- **D-B — abandoning the upgrade path.** Mitigated by per-version dirs (§3) +
  the adopt step (§7). Residual risk: a dev DB at a pre-v11 schema → handled
  by a loud error, no data loss. **Must land before data-dir unification.**
- **D-C — one big PR vs. split.** The rename + flatten are hard to separate
  (both rewrite `migrations.rs` + `wstore.rs`). Recommend **one PR**, reviewed
  with the full test suite green. `user_version` could split out but is small
  enough to ride along.
- **D-D — drop the `accounts` column?** §5.2. **Resolved: KEEP.** An early
  draft recommended dropping it as dead weight; Codex review of PR #934
  proved it is live (Identity-tab account assignments + startup credential
  resolution depend on it). A flatten must be behaviour-preserving — the
  column stays.
- **D-E — promote `identity_id` / `memory_id` to real FKs?** §5.6. *Recommend
  no* for this PR — current code uses `''` sentinels, not NULL; a real FK
  needs a NULL migration + sentinel-row rework. Out of scope; note for later.

---

## 13. Suggested PR breakdown

One PR (`agenta/schema-flatten-deforge`):

1. Write the flat `run_object_schema` (DDL §5) + `adopt_legacy_table_names`
   (§7) + `stamp_and_check_version` (§8).
2. Delete v1–v11 + `run_wstore_migrations` + dead-table DDL (§9).
3. Rename tables/indexes/structs/methods/files (§6) — compiler-driven.
4. Apply `user_version` to the other three files' `configure_and_migrate`.
5. Swap the test suite to flat-schema + adopt-step tests (§11).
6. Refresh doc comments + `CLAUDE.md` + AUDIT §8.5 / §2.2 + agentmux-docs
   internals (`persistence.md`).
7. Changeset: `type: patch` — `refactor(storage): flatten objects.db schema,
   retire the "forge" vocabulary, add user_version tripwire`.

Reviewers: reagent + codex. Expect codex scrutiny on the adopt-step
preconditions (the pre-v11-reject branch) — mirror PR #933's case-analysis
style in the doc comment.
```
