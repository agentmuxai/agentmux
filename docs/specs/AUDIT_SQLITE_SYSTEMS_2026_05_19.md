# AUDIT: SQLite Systems in AgentMux

**Status:** Reference document, current as of `main` 2026-05-19
**Author:** AgentA
**Companion specs:**
- [`docs/specs/archive/SPEC_SHARED_BUNDLES_AND_DEFINITIONS_2026_05_19.md`](./archive/SPEC_SHARED_BUNDLES_AND_DEFINITIONS_2026_05_19.md) — proposes moving durable content to `~/.agentmux/shared/`. Read this audit first; the spec assumes its inventory.
- agentmux-docs internals: [`data-layout.md`](https://github.com/agentmuxai/agentmux-docs/blob/main/src/content/docs/internals/data-layout.md), [`persistence.md`](https://github.com/agentmuxai/agentmux-docs/blob/main/src/content/docs/internals/persistence.md). Both are partially stale (see §10).

> **Staleness note (2026-08-03):** "current as of 2026-05-19" is no longer accurate. `~/.agentmux/shared/store.db` (described here as proposed) is now a real, versioned, actively-written store (schema v1-v4: identity/memory/cron/credentials — see `agentmux-srv/src/backend/storage/migrations.rs`). Treat this doc's inventory as a snapshot of intent at the time, not the current schema.

---

## 0. TL;DR

AgentMux today opens **four physical SQLite files + a fifth cross-version "registry" file + an unbounded number of `:memory:` instances in tests**. They split across two processes (srv + launcher) and three persistence categories (durable user content, runtime saga state, ephemeral test fixtures). One additional store, `shared/store.db`, is proposed by SPEC_SHARED_BUNDLES.

This document inventories every SQLite touchpoint: writer, schema, lifecycle, who opens it, where the path is resolved.

---

## 1. Files at a glance

| File | Writer | Holds | Path | Lifecycle |
|---|---|---|---|---|
| `objects.db` | `agentmux-srv` | App-domain state + bundles + agents (mixed durable + runtime) | `<version-data-dir>/db/objects.db` | Lifetime of the user's install per version |
| `filestore.db` | `agentmux-srv` | Per-block content blobs (agent configs, snapshots, file-pane content) | `<version-data-dir>/db/filestore.db` | Same; rebuildable but referenced by `objects.db` |
| `sagas.db` | `agentmux-srv` | srv-side saga coordinator durability | `<version-data-dir>/db/sagas.db` | Per-version; truncated on saga completion |
| `launcher-sagas.db` | `agentmux-launcher` | Launcher-side saga state (LSD spec) | `<version-data-dir>/db/launcher-sagas.db` (legacy installs migrate from the pre-2026-05-19 root location on first launch) | Per-version |
| `registry/store` (NB: filesystem, not SQLite — flat file registry) | `agentmux-srv` migrator + `agentmux-launcher` reader | Cross-version named-agent history (Continue dropdown) | `~/.agentmux/shared/registry/` | One-shot migrated from per-version `objects.db` snapshots |
| `shared/store.db` (proposed) | `agentmux-srv` | Cross-version bundles + accounts + memories + agent definitions | `~/.agentmux/shared/store.db` | One-shot migrated from per-version `objects.db` snapshots (mirrors registry pattern) |

`registry/` is **filesystem-backed**, not SQLite — clarified in §5. SPEC_SHARED_BUNDLES proposes a real SQLite file in the same `shared/` tree.

---

## 2. `objects.db` — the kitchen sink

### 2.1 Opening + configuration

- Opened in `agentmux-srv/src/main.rs:316` via `WaveStore::open(&db_dir.join("objects.db"))`.
- `db_dir = get_wave_data_dir().join("db")` per `agentmux-srv/src/backend/base.rs:132`.
- `get_wave_data_dir()` reads `AGENTMUX_DATA_DIR` env var (launcher-injected) and falls back to `~/.agentmux`.
- PRAGMAs applied per-connection in `WaveStore::configure_and_migrate` (`wstore.rs:56-72`):
  - `journal_mode=WAL`
  - `busy_timeout=5000`
  - `foreign_keys=ON` (required — `db_identity_bindings` cascades on `db_identity_bundles` deletion; missed cascades caused issues pre-v6)
  - `synchronous=NORMAL`
  - `cache_size=-8000`
  - `mmap_size=268435456`
  - `temp_store=MEMORY`

### 2.2 Schema (current, after `run_object_schema`)

> **Updated 2026-05-19** — the v1→v11 incremental migration chain was
> flattened into a single `run_object_schema` and the `forge` table
> vocabulary retired. See
> [`SPEC_SCHEMA_FLATTENING_2026_05_19.md`](./SPEC_SCHEMA_FLATTENING_2026_05_19.md).
> Table names below reflect the post-flatten schema.

The schema is now **one flat `CREATE TABLE IF NOT EXISTS` batch** in
`run_object_schema`, plus a `user_version` tripwire (`stamp_and_check_version`,
see §8.5). A single `adopt_legacy_table_names` step renames any pre-flatten
tables found on a dev database.

#### 2.2.a Generic object tables

One row-per-otype with `(oid, version, data TEXT)`:

```
db_client, db_window, db_workspace, db_tab, db_layout, db_block, db_temp
```

These are the reducer's app-domain state. JSON blobs in `data`. Writes funnel through `wcore` from HTTP/WS RPC; reads are `SELECT * FROM db_<otype> WHERE oid = ?`.

#### 2.2.b Saga durability — NOT in objects.db

The `saga` + `saga_step` schema lives in `sagas.db`, owned by `SagaLog` (`sagas/log.rs`). The initial draft of this audit incorrectly claimed `run_saga_log_migrations` also runs inside `WaveStore::configure_and_migrate`; verification on `main` (2026-05-19) confirms it does **not**. `objects.db` has no saga tables; `sagas.db` is the only home. The launcher's `launcher-sagas.db` is unambiguously its own file. See §4 for both schemas.

#### 2.2.c Agent definitions

| Table | Purpose |
|---|---|
| `db_agent_definitions` (was `db_forge_agents`) | Agent CLI definitions (slug, provider, working_directory, shell, …) — seeded from defaults on first launch |
| `db_agent_content` (was `db_forge_content`) | Generated config blobs per agent (soul/agentmd/mcp/env/memory) |
| `db_agent_skills` (was `db_forge_skills`) | Skill catalog per agent |
| `db_agent_history` (was `db_forge_history`) | Per-agent launch history (legacy; named-agent registry now owns this) |

#### 2.2.d Identity + bundle system

| Table | Purpose |
|---|---|
| `db_identity_accounts` | Provider OAuth/API-key accounts (one row per attached account) |
| `db_agent_identity_links` (was `db_forge_agent_identities`) | Junction: agent ↔ account |
| `db_agent_instances` | Per-instance launch rows (block_id, identity_id, memory_id, working_directory, parent_instance_id) |
| `db_identity_bundles` | User-named identity bundles (Work, Personal, …) |
| `db_identity_bindings` | Junction: identity bundle ↔ account ↔ provider |
| `db_memory_bundles` | User-named memory bundles (notes/instructions) |

#### 2.2.e Drone

| Table | Purpose |
|---|---|
| `db_drone_definitions` | User-defined drone graphs |
| `db_drone_runs` | Drone execution history |

The pre-flatten `db_workflow_definitions` / `db_workflow_runs` (and the
`db_v10_migrated_legacy_*` sentinel tables) were dropped by the flatten —
their data had already been copied into `db_drone_*`.

### 2.3 Indexes summary

Identity / agent path indexes:
```
idx_identity_accounts_provider
idx_agent_identity_links_account
idx_agent_instances_definition
idx_agent_instances_block
idx_agent_instances_status
idx_agent_instances_parent
idx_agent_instances_name_recent
idx_identities_is_blank
idx_identity_bindings_account
idx_memories_is_blank
```

Drone / workflow:
```
idx_drone_definitions_updated
idx_drone_runs_drone_started
idx_drone_runs_status
idx_workflow_definitions_updated
idx_workflow_runs_workflow_started
idx_workflow_runs_status
```

---

## 3. `filestore.db` — content blobs

### 3.1 Opening

- `agentmux-srv/src/main.rs:393`: `FileStore::open(&db_dir.join("filestore.db"))`.
- Same PRAGMA discipline as `objects.db` (`filestore/core.rs:67`):
  - `journal_mode=WAL`
  - `busy_timeout=5000`
- Same `db_dir` resolution.

### 3.2 Schema (`run_filestore_migrations`)

```sql
db_wave_file  (zoneid, name, size, createdts, modts, opts, meta)   -- meta
db_file_data  (zoneid, name, partidx, data BLOB)                    -- bytes
```

Composite primary keys on `(zoneid, name)` + `(zoneid, name, partidx)`. Chunked storage; partidx allows large files split across rows. In-memory LRU cache layer (`filestore/cache.rs`) sits on top.

### 3.3 Lifecycle

- Referenced by hash from `objects.db`.
- Deleting `filestore.db` without `objects.db` → dangling references (file pane reports missing blobs).
- Recoverable: `filestore.db` can be deleted independently if user accepts lost snapshots.

---

## 4. `sagas.db` and `launcher-sagas.db`

### 4.1 srv `sagas.db`

- Opened at `agentmux-srv/src/main.rs:403`: `SagaLog::open(&db_dir.join("sagas.db"))`.
- `SagaLog::configure_and_migrate` (`sagas/log.rs:138`) is the only caller of `run_saga_log_migrations`. So the DDL lives exclusively in `sagas.db` — `objects.db` has no `saga` / `saga_step` tables.

### 4.2 launcher `launcher-sagas.db`

- Opened at `agentmux-launcher/src/main.rs:354`: `LauncherSagaLog::open(&saga_log_path)`.
- Diag-cli reads from `agentmux-launcher/src/diag.rs:728`.
- Path: `paths.data_dir.join("launcher-sagas.db")` — `data_dir` here is the **launcher's resolved data dir**, which is the same `~/.agentmux/versions/<v>/` tree as srv. Note this file lives at the **root of the version data dir**, not inside `db/` (cf. srv's `objects.db` etc.).

#### Schema (`launcher/src/saga/log/schema.rs`)

```sql
launcher_saga (saga_id, name, state, started_at, ended_at, input_json, failure_reason)
launcher_saga_step (saga_id, step_index, name, state, cmd_json, target, started_at, ended_at, output_json, failure_reason)

idx_launcher_saga_state ON launcher_saga(state)
idx_launcher_saga_step_state ON launcher_saga_step(saga_id, state)
```

Two deltas vs srv's `saga` table:

1. **`target` column** on `launcher_saga_step` — launcher sagas dispatch to multiple peers (self / host / srv); srv sagas always target srv.
2. **`failed_compensation` saga state** — launcher doesn't auto-compensate; recovery marks unresolved sagas as `failed_compensation` for operator review. srv has `compensated` terminal state instead.

Timestamps are **RFC3339 TEXT** in the launcher (vs INTEGER epoch ms in srv) — easier to grep in shells.

Schema lifecycle policy (per LSD spec §5 risk #2): only additive `ALTER TABLE` in future migration versions. No in-place rewrites.

---

## 5. `shared/registry/` — cross-version named-agent history

**Not SQLite.** Despite living alongside the DBs and being called "the registry," `agentmux-srv/src/registry/store.rs` implements this as a **flat file store** in `~/.agentmux/shared/registry/`. Migration from per-version `objects.db` is a one-shot read-only scan (`registry/migrate.rs:85` enumerates every `versions/*/data/db/objects.db`).

What gets shared via the registry today:

| Source table in per-version `objects.db` | Shared via registry | Purpose |
|---|---|---|
| `db_agent_instances` (named rows only) | Yes — `NamedAgentRecord` | Continue-agent dropdown population. Cross-version dedup on `instance_id` with latest `started_at` winning. |

Marker file: `~/.agentmux/shared/registry/.migrated_from_sqlite`. Idempotent: present → skip migration.

---

## 6. In-memory SQLite (tests + identity resolver)

Roughly 20+ test files open `WaveStore::open_in_memory()` or `Connection::open_in_memory()`. Notable:

| Site | Purpose |
|---|---|
| `agentmux-srv/src/backend/storage/migrations.rs:905-1612` | Unit tests for every migration version |
| `agentmux-srv/src/backend/storage/store.rs:2238-3060` | Multiple test fixtures |
| `agentmux-srv/src/identity/resolver.rs:242` | Stub for non-binding-coverage code paths |
| `agentmux-srv/src/backend/blockcontroller/session_recovery.rs:134` | Session-recovery test fixture |
| `agentmux-srv/src/backend/session_archive.rs:479` | Session archive test fixture |
| `agentmux-srv/src/backend/wcore/mod.rs:215` | wcore-internal test fixture |

These don't touch disk; they're safe to ignore for cross-version migration concerns.

---

## 7. Cargo dependency posture

Both `agentmux-srv` and `agentmux-launcher` use **`rusqlite = "0.31"` with `bundled` feature** — locked in lockstep so the saga log schema stays compatible. Pinning comment in `agentmux-launcher/Cargo.toml` reminds future bumpers to upgrade both.

No `sqlx`, no `diesel`, no async DB layer. All SQLite access is synchronous behind `Mutex<Connection>` wrappers (`WaveStore.conn: Mutex<Connection>`, `FileStore.conn: Mutex<Connection>`).

---

## 8. Open inconsistencies + clean-up items

1. ~~**Schema naming drift.** v7 creates a table called `db_identities`, but the rest of the code and docs call it `db_identity_bundles`.~~ **Fixed** — first by the v11 rename migration (PR #933), then fully absorbed by the schema flatten: `run_object_schema` defines `db_identity_bundles` / `db_memory_bundles` (and the de-forged `db_agent_*` tables) directly. The legacy-name drift no longer exists in the source.

2. ~~`saga` tables in two files.~~ **Retracted** — the initial audit miscounted; `run_saga_log_migrations` is only invoked from `SagaLog::configure_and_migrate`, so `objects.db` has no saga tables. See §2.2.b.

3. **`launcher-sagas.db` lives in the wrong directory.** ~~Srv puts its DBs in `<data-dir>/db/`; launcher puts `launcher-sagas.db` directly in `<data-dir>/`.~~ **Fixed** in PR #932 — `agentmux-launcher::data_dir::launcher_saga_log_path` performs a one-shot rename of any pre-existing file from `<data-dir>/launcher-sagas.db` into `<data-dir>/db/launcher-sagas.db`. Idempotent + safe to call repeatedly. `main.rs` uses this writing variant; `diag.rs` uses the read-only sibling `launcher_saga_log_path_read_only` so `--diag sagas` stays passive.

4. ~~**`db_workflow_*` vs `db_drone_*` after the rename.**~~ **Fixed** by the schema flatten — `db_workflow_definitions` / `db_workflow_runs` are no longer created, and `adopt_legacy_table_names` drops them (plus the `db_v10_migrated_legacy_*` sentinels) from any dev DB that still has them. Their data had already been copied into `db_drone_*`.

5. ~~**No `PRAGMA user_version` discipline.**~~ **Fixed** — `stamp_and_check_version` stamps `user_version` on all four SQLite files (`objects.db`, `filestore.db`, `sagas.db`, `launcher-sagas.db`) and logs a loud warning when a file reports a version newer than the running build (downgrade tripwire). Deliberately a tripwire, not a migration gate — the idempotent DDL remains the schema mechanism. See `SPEC_SCHEMA_FLATTENING_2026_05_19.md` §8.

6. **Foreign keys ON only set in `WaveStore`.** `FileStore`'s connection doesn't set `foreign_keys=ON` (it has no FKs to enforce so probably fine, but worth documenting the gap).

---

## 9. Path resolution summary

```
~/.agentmux/
├── versions/<version>/      ← installed + portable, resolved via DataPaths::resolve(version, mode)
│   ├── data/
│   │   ├── db/
│   │   │   ├── objects.db          ← srv (mixed durable + runtime)
│   │   │   ├── filestore.db        ← srv
│   │   │   ├── sagas.db            ← srv (saga + saga_step only)
│   │   │   └── launcher-sagas.db   ← launcher (auto-migrated from <data-dir>/launcher-sagas.db on first launch ≥2026-05-19)
│   │   └── launcher-events.log     ← launcher JSONL (Layer-1 reducer log)
│   ├── logs/
│   ├── cef-cache/
│   ├── agents/
│   ├── config/
│   └── runtime/
├── dev/<branch>/            ← `task dev` only — same shape under here
└── shared/
    ├── registry/            ← cross-version named-agent registry (flat files; NOT SQLite)
    └── store.db             ← proposed by SPEC_SHARED_BUNDLES (does NOT exist yet)
```

Notes:
- `DataPaths` in `agentmux-common` is the single resolver. Host + srv + launcher all read paths from launcher-injected env vars (`AGENTMUX_DATA_DIR`, `AGENTMUX_LOG_DIR`, etc.).
- Two same-version instances share the same `versions/<v>/` tree — that's the multi-instance isolation model documented in CLAUDE.md and `docs/internals/data-layout.md`.

---

## 10. Doc accuracy

agentmux-docs internals currently has:

- [`internals/data-layout.md`](https://github.com/agentmuxai/agentmux-docs/blob/main/src/content/docs/internals/data-layout.md) — accurate on the per-version tree structure. Missing: registry layout under `shared/`, the planned `shared/store.db`, the `launcher-sagas.db` filename quirk.
- [`internals/persistence.md`](https://github.com/agentmuxai/agentmux-docs/blob/main/src/content/docs/internals/persistence.md) — accurate on the four-file model. Missing:
  - The full bundle table catalog (`db_identity_bundles`, `db_identity_bindings`, `db_memory_bundles`, `db_identity_accounts`)
  - `db_agent_*` table family (`db_agent_definitions`, `db_agent_content`, `db_agent_skills`, `db_agent_history`, `db_agent_identity_links`)
  - `db_drone_*` (workflows → drone rename)
  - The dual-DB `saga` table presence
  - The named-agent registry sharing layer

**Recommended docs follow-up PR** (separate from any spec implementation):

1. Refresh `persistence.md` with a complete table catalog mirroring §2.2 of this audit.
2. Add a new page `internals/cross-version-sharing.md` documenting the registry pattern and (once shipped) the `shared/store.db` model.
3. Update `data-layout.md` to call out `shared/` more prominently.
4. Cross-link from the user-facing `multi-instance.md` page so users understand what's shared vs not.

Per `feedback_docs_refresh_after_features.md`: docs go in their own repo PR after the implementation lands, not bundled with the code change.

---

## 11. Next-step audit work

Items worth a deeper look once this initial pass is in:

1. **Concurrent access discipline.** All connections are `Mutex<Connection>`-wrapped. With multiple workers calling `wstore.bundle_*`, contention is the bottleneck. Measure latencies under load.
2. **WAL checkpoint policy.** None set explicitly. WAL files can grow unbounded under heavy write. SQLite default auto-checkpoint at 1000 pages is acceptable but worth confirming nothing has overridden it.
3. **Backup story.** No mention of `sqlite3_backup_init()` use. Crash-consistent backups today are "stop srv, copy files." Worth specifying a hot-backup approach if support tickets surface around it.
4. **Cross-version saga compensation.** If launcher v0.36 starts a saga and srv v0.37 reads `launcher-sagas.db`, schema compat is asserted only by lockstep `rusqlite = 0.31` — not by an actual user_version check. Could surface as silent drift on a future bump.
