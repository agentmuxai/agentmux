# SPEC: Cross-Version Sharing — Identity / Memory / Forge Definitions

> **Archived 2026-07-12.** Superseded by `docs/specs/SPEC_GLOBAL_IDENTITY_MEMORY_DRONE_2026_06_24.md` (still current, not archived) — that doc's own header states it supersedes this file. Consolidated tracking: issue #2024.

**Status:** Draft
**Date:** 2026-05-19
**Author:** AgentA
**Related:**
- `agentmux-common/src/data_paths.rs` — current per-version layout (`~/.agentmux/versions/<v>/`)
- `agentmux-srv/src/registry/` — established cross-version registry pattern (named-agent history) with one-shot migration from per-version SQLite
- `agentmux-srv/src/backend/storage/migrations.rs` — current `objects.db` schema (bundles, memories, forge defs, accounts, bindings)

---

## 0. TL;DR

Today **all user content lives in per-version SQLite**: identity bundles, identity accounts (OAuth tokens), bindings, memory bundles, forge agent definitions. Install v0.37.0 alongside v0.36.0 and your "Work" identity, "Project Notes" memory, and saved Claude OAuth do not follow you.

That's a UX bug. Bundles + agent definitions are durable user content; users expect them to persist across releases the way browser bookmarks persist across browser versions. Only **per-version runtime state** (in-progress sessions, CEF cache, working directories, logs) should be siloed.

This spec proposes:

1. **Move durable content to `~/.agentmux/shared/store.db`** — identity bundles + accounts + bindings + memory bundles + forge agent definitions.
2. **Keep per-version state per-version** — agent instance rows, working directories, sagas, CEF cache, logs.
3. **One-shot migration** modeled on `registry/migrate.rs` — scan per-version `objects.db`s, dedup, write to shared store, record a marker so it runs once.
4. **Schema evolution discipline** — shared store has its own monotonic version. Forward-compat: newer field readers ignore unknown columns; older version warns + skips unparseable rows.

---

## 1. Current state (what's where today)

### 1.1 Layout

```
~/.agentmux/
├── versions/<v>/
│   ├── data/db/
│   │   ├── objects.db        ← ALL durable content + per-version instance rows
│   │   ├── filestore.db
│   │   ├── sagas.db
│   │   └── launcher-sagas.db
│   ├── cef-cache/
│   ├── logs/
│   ├── config/
│   └── agents/<instance>/    ← per-instance working dirs
└── shared/
    └── registry/             ← shared named-agent history only
```

### 1.2 What's in `objects.db` today

| Table | What | Should-share? |
|---|---|---|
| `db_identity_bundles` | User-named identity containers (Work, Personal, …) | **Yes** — durable user content. |
| `db_identity_accounts` | Provider OAuth/API-key creds (one row per attached account) | **Yes** — re-prompting OAuth on every install is hostile. |
| `db_identity_bindings` | Junction `bundle_id ↔ account_id ↔ provider` | **Yes** — meaningless without bundles+accounts being shared too. |
| `db_memories` | User-named memory bundles (notes/instructions) | **Yes** — durable user content. |
| `db_forge_agents` | Agent CLI definitions (Claude, Codex, OpenClaw…) | **Yes** — definitions are content; today reseeded per version on first launch. |
| `db_agent_instances` | Per-instance launch rows (block_id, identity_id, memory_id, working_directory, …) | **No** — these reference per-version paths + block_ids. Keep per-version; `Continue agent` already federates via the named-agent registry. |
| `db_oauth_sessions` | Transient OAuth-in-flight | **No** — runtime state. |
| `db_skills`, `db_mcp_servers` (if present) | TBD — depend on whether they're user content or per-version | Probably yes. |

The registry already shares **named-agent history** (Continue-agent dropdown). That stays unchanged.

### 1.3 Why this matters now

- Spec §3.1 of the launch-modal hardening (just shipped) made Identity + Memory **required at launch**. Users with zero bundles must create them. Doing that once per install is friction the spec did not anticipate.
- Phase β/γ added the "+ New" affordance specifically to make creation cheap — that helps first-time-per-version creation, but doesn't help "I already have my Work identity in v0.36.0; let me just use it in v0.37.0."
- The Stage 2 reducer refactor + 124 tests make the wire shape stable enough to safely bolt sharing underneath.

---

## 2. Design

### 2.1 New layout

```
~/.agentmux/
├── versions/<v>/data/db/objects.db   ← `db_agent_instances` only (per-version runtime state)
├── versions/<v>/...                  ← unchanged: cef-cache, logs, sagas, agents/
└── shared/
    ├── registry/                     ← existing named-agent history (unchanged)
    ├── store.db                      ← NEW: shared SQLite for durable content
    └── secrets/                      ← OAuth tokens (existing OS-keychain path stays)
        └── …
```

Notes:
- `store.db` is the new home for `db_identity_*`, `db_memories`, `db_forge_agents`.
- OS-keychain references (`SecretRef`) stay where they are — those already aren't IN `objects.db`; only the indirection (`SecretRef` ID strings) live in DB. So sharing the DB shares the indirection, which resolves to the same OS keychain entry from any version. ✓
- `versions/<v>/data/db/objects.db` keeps just `db_agent_instances` (and any other per-version-only tables). Migration drops the shared tables from per-version DBs at the end.

### 2.2 Schema versioning

`shared/store.db` has its own `PRAGMA user_version`. Migrations live in a new `agentmux-srv/src/shared/migrations.rs`. The schema is initially **a copy of the relevant tables from `agentmux-srv/src/backend/storage/migrations.rs`** at the time this spec ships.

Forward-compat rules:
- Newer version reads older `store.db`: missing columns get defaults during read; new columns are added by migration on first launch of the newer version.
- Older version reads newer `store.db`: unknown columns are ignored (`SELECT id, name, …` enumerates expected columns explicitly). Unknown TABLES are ignored.
- Breaking schema changes go through the same RFC #857-style changeset workflow and are flagged in `VERSION_HISTORY.md` — users see a release note about a forced migration.

### 2.3 RPC + reducer impact

- Backend RPCs that read/write these tables (`upsertmemory`, `upsertidentitybundle`, `bundle_identity_bind`, `listidentitybundles`, …) target `store.db` instead of `objects.db`. Wrap `WaveStore::open` with a path resolver that picks `store.db` for shared tables vs `objects.db` for the instance table.
- Frontend reducer slice (`launch-flow-state`) is **unchanged**. The view doesn't care what backing file the data comes from.
- The `identitybundlebindings:changed:<id>` event (already cross-tab-broadcasted via WPS) gains a free win — it now also broadcasts across versions IF they share the same WPS process. They don't (each version has its own srv on a dynamic port), so cross-version reactivity needs a different mechanism.

### 2.4 Cross-version reactivity

Each version runs its own `agentmux-srv` sidecar. They can't subscribe to each other's WPS broker. Options:

1. **SQLite WAL polling** — each srv polls `store.db`'s WAL frames for changes and broadcasts to its own clients. Cheap, ~1s lag, no coordination needed. Recommended for v1.
2. **File watcher on `store.db`** — works but less precise (full-file mtime is the only signal in pure-fs mode).
3. **Named-pipe IPC between sidecars** — overkill; introduces a discovery problem.

V1 ships with **option 1**. Each srv runs a 1-second poll on `store.db`'s SQLite `data_version` PRAGMA; on change, refresh the in-memory cache and emit the relevant `identitybundlebindings:changed:*` / `bundles:changed` events to its renderer.

### 2.5 One-shot migration

Modeled on `registry/migrate.rs`:

1. On srv startup, check `~/.agentmux/shared/.migrated_from_per_version` marker. If present, skip.
2. Otherwise, enumerate `~/.agentmux/versions/*/data/db/objects.db`.
3. For each file, read-only-open and extract: `db_identity_bundles`, `db_identity_accounts`, `db_identity_bindings`, `db_memories`, `db_forge_agents`.
4. For each row, insert into `shared/store.db` with conflict resolution (§2.6).
5. Write the marker.
6. Per-version `objects.db` rows are **not deleted** in v1 — kept as a fallback in case migration is buggy. A follow-up release (e.g. v0.40.0) drops the per-version columns.

Migration is **idempotent + read-only on the source DBs** — same discipline as the named-agent registry migration.

### 2.6 Conflict resolution

Same name, different `id` in two versions (the common case — user created "Work" identity independently in 0.34.0 and 0.37.0):

| Strategy | Pro | Con | v1 choice |
|---|---|---|---|
| Latest `updated_at` wins | Simple | Loses the other's bindings | **No** |
| Merge bundles by name | Preserves both versions' work | Requires deduping bindings underneath | **No** in v1 (complexity) |
| Keep both as `Work` and `Work (0.34.0)` | Lossless | Clutters the dropdown | **Yes** in v1 |

V1 ships the rename-on-conflict strategy. A future "merge identical bundles" UI lives in the Identity pane.

For `db_forge_agents` (seeded agent definitions), name conflicts are common but rows are content-identical — keep the seeded version with the lower `created_at` (oldest install wins; later seeds skipped via `INSERT OR IGNORE` on `(slug, parent_id)`).

### 2.7 Per-version opt-out

Some users may want to test feature-branch builds without polluting their shared bundles. Add `AGENTMUX_USE_LOCAL_STORE=1` env var: when set, the srv falls back to per-version `objects.db` for ALL tables (legacy behavior). Document for developers; not a user-facing setting.

### 2.8 Backward compatibility

Older versions launched after this ships (e.g. user pins 0.35.0 to compare):
- They read `~/.agentmux/versions/0.35.0/data/db/objects.db` — which after migration STILL contains the original rows (we didn't delete them in v1, §2.5).
- So 0.35.0 sees the snapshot from its install date. New bundles created in 0.37.0+ are invisible to 0.35.0. Acceptable.
- Once v0.40.0 drops per-version columns, older versions can't read anything; flagged as a release-notes breaking change.

---

## 3. Implementation phases

### Phase 1 — read path (no migration yet)

1. Add `agentmux-srv/src/shared/store.rs` — opens `~/.agentmux/shared/store.db`, runs migrations.
2. Add `SharedStore` type alongside existing `WaveStore`.
3. Adapt the relevant RPC handlers (`listidentitybundles`, `listmemories`, `listforgeagents`, `listidentitybindings`, …) to read from `SharedStore`.
4. **Without writing yet** — write paths still target `WaveStore` so we can compare in dev.

Wire this behind `AGENTMUX_SHARED_STORE_READS=1` for canary testing.

### Phase 2 — migration

5. Implement `migrate_from_per_version()` in `shared/migrate.rs`. Same shape as `registry/migrate.rs`.
6. Wire into srv startup with the marker file gate.
7. Document in release notes.

### Phase 3 — write path

8. Switch upsert/bind/unbind/upsertmemory/upsertforgeagent RPCs to write to `SharedStore`.
9. Drop the `AGENTMUX_SHARED_STORE_READS` gate (always on).

### Phase 4 — cross-version reactivity

10. SQLite `data_version` polling per §2.4.
11. Emit `identitybundlebindings:changed:*` / `bundles:changed` / `memories:changed` events when poll detects mutation.
12. Frontend launch-flow-state already subscribes — gets cross-version reactivity for free.

### Phase 5 — per-version cleanup (separate release)

13. v0.40.0+ drops the shared tables from per-version `objects.db`. Release-notes-flagged.

---

## 4. Risks + open questions

### 4.1 Risks

- **Lock contention.** Multiple srv processes hitting the same `store.db` need WAL mode + `busy_timeout`. SQLite handles this well in practice but worth a stress test.
- **Account secrets in OS keychain.** Sharing `SecretRef` IDs across versions means each version resolves the same keychain entry. Verify on macOS (Keychain Services) + Linux (libsecret / Secret Service) + Windows (Credential Manager).
- **Migration data loss.** A buggy migration is hostile — non-recoverable. Mitigation: per-version `objects.db` rows are not deleted in v1 (§2.5). Plus a backup of `store.db` is written next to the marker file: `~/.agentmux/shared/store.db.pre-migrate-<v>.backup`.
- **Forge agent re-seeding.** Each new install seeds default agents into its `objects.db`. With sharing, the seed should happen against `store.db` exactly once (gated by `INSERT OR IGNORE` on `slug` + a release-version watermark).

### 4.2 Open questions

1. **Renaming sensitivity.** If user renames a shared identity in 0.37.0, the 0.36.0 instance shows the old name until it polls + reloads. UX-acceptable in v1 (~1s).
2. **Delete semantics.** Deleting a shared bundle removes it from all versions. Add a confirmation dialog noting "this affects all your AgentMux installs". V1 keeps the existing single-confirmation; revisit if support tickets surface.
3. **Multi-user.** This spec assumes one OS user. Multi-user (e.g. company shared workstation) is out of scope.
4. **`db_skills` / `db_mcp_servers` / future content tables.** Need to be classified the same way (durable vs runtime) as they're added. Add to §1.2 table on each new table.

---

## 5. Acceptance criteria

1. After upgrading from 0.37.0 (per-version) to a release shipping Phase 3, identity bundles + memories + accounts created in 0.37.0 appear in the new version without user action.
2. Old per-version `objects.db` rows remain readable by older versions (until Phase 5).
3. Binding an account in version A, reopening version B → version B sees the new binding within 1s.
4. SQLite stress test: two srvs hammering `store.db` for 60s show no lock contention errors.
5. `AGENTMUX_USE_LOCAL_STORE=1` opt-out preserves legacy behavior.
6. Migration marker file is present + correctly idempotent (re-running srv doesn't re-migrate).
7. Backup file (`store.db.pre-migrate-*.backup`) exists after Phase 2 ships.

---

## 6. Out of scope

- **Cloud sync.** Some users want bundles synced across machines. This spec only addresses cross-version on the same machine. Cloud sync is a separate, larger effort with auth + encryption concerns.
- **Encryption at rest** for `store.db`. OAuth tokens live in OS keychain (already encrypted by the OS); the SQLite has only `SecretRef` IDs. If full-DB encryption is wanted later, that's a separate spec.
- **Merge-identical-bundles UI.** The rename-on-conflict (§2.6) is the v1 shipping strategy. A merge UI is a follow-up.
- **Per-tab / per-pane bundle scoping.** Bundles are user-scoped. Per-pane overrides are out of scope.

---

## 7. References

- [`agentmux-common/src/data_paths.rs`](../../agentmux-common/src/data_paths.rs) — current layout
- [`agentmux-srv/src/registry/migrate.rs`](../../agentmux-srv/src/registry/migrate.rs) — model for §2.5
- [`agentmux-srv/src/backend/storage/migrations.rs`](../../agentmux-srv/src/backend/storage/migrations.rs) — schema this spec moves
- SPEC_LAUNCH_MODAL_STATE_MACHINE_2026_05_19.md — the reducer slice this spec sits underneath
