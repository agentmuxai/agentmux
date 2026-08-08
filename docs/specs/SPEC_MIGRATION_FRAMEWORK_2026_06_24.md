# Migration Framework Spec
**Date:** 2026-06-24  
**Status:** Proposed (implemented — see note below)
**Scope:** agentmux-srv, agentmux-launcher

> **2026-08-07 audit note:** Implemented, foundational — `agentmux-srv/src/migrations/`
> is a fully built, actively-used framework, directly referenced by the later
> `SPEC_MIGRATION_SYSTEM_HARDENING_2026_08_03.md`. Badly stale status. See
> `docs/reports/REPORT_DOCS_AND_DEAD_CODE_CLEANUP_AUDIT_2026_08_07.md`.

---

## Problem

AgentMux currently embeds 17 one-time data migrations directly into the srv startup sequence (`main.rs`). Every launch pays the cost of checking whether each migration has already run. The startup sequence has grown fragile: migrations have ordering dependencies on each other and on startup seeding steps, bugs in those ordering constraints have required multiple revision rounds to resolve, and the accumulation of migrations has made `main.rs` difficult to reason about.

For public release, this pattern becomes untenable:
- A failed migration silently logs a warning and boots anyway — acceptable internally, a potential data corruption incident for end users
- No user-visible feedback during long-running migrations (the app appears frozen)
- No rollback path if a migration partially corrupts the data dir
- No clear contract about which version of the data dir the app expects to find at startup

---

## Goals

1. **Zero migration code in `main.rs`.** The srv starts with a guaranteed clean, fully-migrated data state. No defensive checks, no fallbacks, no ordering constraints.
2. **Automatic for users.** No manual steps. The user launches a new version and it works.
3. **Safe for public release.** Failure is visible, not silent. Data is backed up before destructive steps. A failed migration never bricks the app.
4. **Handles arbitrary upgrade jumps.** A user upgrading from v0.35 → v0.42 gets all intermediate migrations applied in order.
5. **Handles per-channel and global scope.** Some migrations target the shared store; others target per-channel data dirs.

---

## Architecture

### Separation of Concerns

```
agentmux-launcher
  │
  ├─ (splash: "Starting AgentMux...")
  ├─ agentmux-srv migrate          ← new: migration runner, exits 0/1
  │    ├─ reads migration state
  │    ├─ runs pending migrations in order
  │    └─ writes migration state, exits
  │
  └─ agentmux-srv                  ← existing daemon, starts only on exit 0
       └─ main.rs: zero migration code
```

The launcher invokes `agentmux-srv migrate` as a separate process before spawning the daemon. If it exits non-zero, the launcher surfaces an error to the user and does not start the daemon.

### Migration Runner Subcommand

```
agentmux-srv migrate [--data-dir <path>] [--dry-run] [--list]
```

- `--data-dir`: override the target data directory (defaults to the channel's data dir)
- `--dry-run`: print pending migrations without applying them
- `--list`: print all migrations and their status (applied / pending)

### Migration State

A `migrations` table in the shared store (`~/.agentmux/shared/store.db`), plus a per-channel `migrations` table in each channel's `objects.db`, tracks which migrations have been applied:

```sql
CREATE TABLE IF NOT EXISTS db_migrations (
    id          TEXT PRIMARY KEY,   -- e.g. "0001_legacy_data_dir"
    applied_at  TEXT NOT NULL,      -- ISO-8601 UTC
    duration_ms INTEGER NOT NULL,
    scope       TEXT NOT NULL       -- "global" | "channel"
);
```

Global migrations (touching shared store or filesystem layout) record into the shared store. Per-channel migrations record into that channel's objects.db. On first run after adopting this framework, all previously-applied migrations are stamped as completed via a bootstrap step so existing users do not re-run them.

### Migration Definition

Each migration is a Rust struct implementing a `Migration` trait:

```rust
pub trait Migration: Send + Sync {
    fn id(&self) -> &'static str;          // e.g. "0001_legacy_data_dir"
    fn scope(&self) -> MigrationScope;     // Global | Channel
    fn description(&self) -> &'static str;
    fn up(&self, ctx: &MigrationContext) -> Result<(), MigrationError>;
}

pub enum MigrationScope {
    Global,   // runs once against ~/.agentmux/shared/
    Channel,  // runs against the current channel's data dir
}
```

Migrations live in `agentmux-srv/src/migrations/`, one file per migration, numbered sequentially. The registry is a static ordered list — adding a migration is adding a struct to the list and a file to the module. No magic discovery.

```
agentmux-srv/src/migrations/
  mod.rs              ← ordered registry
  m0001_legacy_data_dir.rs
  m0002_block_zones_v1.rs
  m0003_template_sessions_v1.rs
  m0004_registry_from_sqlite.rs
  m0005_registry_source_bases.rs
  m0006_definitions_global.rs
  m0007_agents_consolidate.rs
  m0008_transcript_backfill.rs
  m0009_session_ids.rs
  m0010_shared_store_backfill.rs
  ...
```

### Migration Context

The runner provides each migration with a context object:

```rust
pub struct MigrationContext {
    pub home: PathBuf,             // ~/.agentmux
    pub data_dir: PathBuf,         // current channel data dir
    pub shared_store: Arc<Store>,  // opened read-write
    pub channel_store: Arc<Store>, // opened read-write (channel scope only)
    pub backup_dir: PathBuf,       // pre-migration backup location
}
```

---

## Safety

### Pre-Migration Backup

Before applying any pending migrations, the runner snapshots the shared store and current channel's objects.db into a timestamped backup directory:

```
~/.agentmux/shared/backups/pre-migration-<version>-<timestamp>/
  store.db
  <channel>/objects.db
```

Backups older than 30 days are pruned on successful migration. If the migration run fails, the backup is retained and the error message includes its path so the user can restore manually or support can inspect it.

### Failure Handling

Migrations run inside a SQLite transaction where possible. If a migration returns `Err`, the runner:

1. Rolls back the transaction
2. Leaves the data dir in its pre-migration state (backup is intact)
3. Writes a human-readable error to `~/.agentmux/logs/migration-error.log`
4. Exits non-zero

The launcher, on receiving a non-zero exit, shows a modal to the user:

> **Update failed**  
> AgentMux could not migrate your data to the new version.  
> Your data has not been modified. [Show details] [Report issue]

"Show details" opens `migration-error.log`. The user can continue running the previous version from the prior portable directory (portable model means the old binary is still present).

### Idempotency

Every migration must be safe to re-run if the runner is interrupted mid-flight (e.g. power loss). Migrations use `INSERT OR REPLACE` / `INSERT OR IGNORE` where possible. Migrations that cannot be made idempotent must check their completion state before applying changes.

### Read-Only Sources

Migrations that read from sibling or prior-version `objects.db` files must open them with `Store::open_source_readonly` (SQLITE_OPEN_READ_ONLY). Source DBs are never modified during migration.

---

## Launcher Integration

The launcher passes the channel data dir and version to the migration runner:

```
agentmux-srv migrate --data-dir <channel-data-dir> --app-version <version>
```

The splash screen updates during migration:

- "Starting AgentMux..." — normal launch
- "Updating your data..." — migrations pending (version changed since last launch)
- "Update complete." — migrations applied, transitioning to normal boot

The launcher reads stdout from the migration runner for progress events (newline-delimited JSON):

```json
{"event": "migration_start", "id": "0010_shared_store_backfill", "description": "Migrating identity accounts"}
{"event": "migration_done",  "id": "0010_shared_store_backfill", "duration_ms": 240}
{"event": "complete", "applied": 1, "skipped": 9}
```

---

## Porting Existing Migrations

The 17 existing startup-embedded functions map to migration steps as follows. Functions marked **keep in srv** are not one-time migrations and remain in startup.

| Current function | Migration ID | Scope | Notes |
|---|---|---|---|
| `migrate_legacy_data_dir` | `0001_legacy_data_dir` | Global | filesystem move |
| `migrate_block_zones_v1` | `0002_block_zones_v1` | Channel | |
| `migrate_promote_template_sessions_v1` | `0003_template_sessions_v1` | Channel | |
| `migrate_from_sqlite_once` | `0004_registry_from_sqlite` | Global | already has marker; adopt migration table |
| `backfill_source_bases_once` | `0005_registry_source_bases` | Global | |
| `migrate_definitions_global_once` | `0006_definitions_global` | Global | |
| `run_agents_consolidate` | `0007_agents_consolidate` | Channel | |
| `run_default_bundle_migration` | `0008_default_bundle` | Channel | |
| `backfill_transcripts_once` | `0009_transcript_backfill` | Global | |
| `backfill_session_ids` | `0010_session_ids` | Global | |
| `backfill_shared_store_once` | `0011_shared_store_backfill` | Global | current PR |
| `heal_all_layouts` | — | **keep in srv** | runs every startup, not one-time |
| `scan_orphans` | — | **keep in srv** | runs every startup, not one-time |
| `heal_global_snapshot_source_block_ids` | — | **keep in srv** | idempotent content check, cheap |
| `repair_agent_def_gaps` | — | **keep in srv** | cheap gap-repair, intentionally every startup |
| `cleanup_stale` | — | **keep in srv** | age-based cleanup, intentionally every startup |
| `bootstrap_state_from_wstore` | — | **keep in srv** | not a migration |

---

## Bootstrap: Existing Users

On first run of a version that includes the migration framework, the bootstrap step stamps all previously-applied migrations as completed so existing users are not asked to re-run them. The bootstrap reads the existing marker files (`.migrated_from_sqlite`, `.backfilled_source_bases`, etc.) and the presence of already-migrated data structures to infer which migrations have been applied, then writes the corresponding rows into `db_migrations`.

This is a one-time bootstrap step, itself implemented as migration `0000_bootstrap_migration_state`.

---

## Retiring Old Migrations

Once the minimum supported upgrade path moves past a migration's origin version, the migration step can be removed from the registry. The `db_migrations` row remains as a record. A comment in `mod.rs` documents the removal:

```rust
// m0001_legacy_data_dir: retired 2027-03. Safe to remove for any user
// who has ever run a version >= 0.40.0 (waveterm dir no longer exists).
```

Retiring migrations is what makes the codebase shrink over time rather than accumulate.

---

## Implementation Order

1. **Migration framework** — `Migration` trait, runner binary, `db_migrations` table, launcher integration, backup/restore, progress events. No migrations ported yet. (~1 PR)
2. **Bootstrap** — `0000_bootstrap_migration_state` to stamp existing users. (~0.5 PR, ships with framework)
3. **Port global migrations** — `0004`–`0011`, remove corresponding code from `main.rs`. (~1 PR)
4. **Port channel migrations** — `0001`–`0003`, `0007`–`0008`. (~1 PR)
5. **Launcher progress UI** — splash "Updating..." state, error modal. (~1 PR)

Steps 1–4 can ship without step 5 (launcher shows generic splash during migration). Step 5 is required before public release.
