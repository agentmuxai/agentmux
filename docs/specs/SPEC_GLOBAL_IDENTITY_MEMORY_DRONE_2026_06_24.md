# SPEC — Global Identity, Memory, and Drone Definitions

**Date:** 2026-06-24  
**Status:** Proposed (implemented — see note below)
**Supercedes:** `docs/specs/archive/SPEC_SHARED_BUNDLES_AND_DEFINITIONS_2026_05_19.md` (Draft; agent-definition portion already shipped via #1387–#1396)  
**Related:**
- `SPEC_CROSS_CHANNEL_AGENT_PERSISTENCE_2026-06-13.md` — the agent-persistence ship that set the pattern
- `docs/specs/archive/SPEC_TRUST_CENTER_GLOBAL_BRAIN_2026_06_19.md`
- `agentmux-srv/src/registry/paths.rs` — `resolve_global_shared_root()` and sibling resolvers
- `agentmux-srv/src/backend/storage/migrations.rs` — current `objects.db` schema

> **2026-08-07 audit note:** Implemented — `resolve_global_shared_root()` is
> load-bearing in `paths.rs`, further extended by
> `SPEC_ISOLATED_AUTH_DEFAULT_BY_CHANNEL_2026_08_06.md` (also stale-status).
> See `docs/reports/REPORT_DOCS_AND_DEAD_CODE_CLEANUP_AUDIT_2026_08_07.md`.

---

## 1. Problem

Importing GitHub accounts (agent1–5, agentX, agentY) from AWS Secrets into the running instance stores them in the per-channel/version `objects.db`. When a new version ships, they are gone. The user must re-import every time.

The same is true for:
- Memory bundles (presets — instructions, context files, MCP servers, skills)
- Drone definitions (workflow graphs)
- MuxBus credentials (the cloud subscription token)

This is the last remaining class of durable user content that is not cross-channel. Agents, instances, transcripts, and provider auth are already global.

### 1.1 What's global today (reference)

| Data | Location | Since |
|---|---|---|
| Agent definitions | `shared/agents/definitions/{id}.json` | #1387 |
| Agent instances registry | `shared/agents/registry/{id}.json` | #1388 |
| Conversation transcripts | `shared/agents/transcripts/` | #1391 |
| Provider auth (Claude OAuth, etc.) | `shared/providers/{provider}/` | Legacy |
| MuxBus WS token | `objects.db` → `db_muxbus_credentials` | **Missing** |
| Identity accounts | `objects.db` → `db_identity_accounts` | **Missing** |
| Identity bundles | `objects.db` → `db_identity_bundles` | **Missing** |
| Identity bindings | `objects.db` → `db_identity_bindings` | **Missing** |
| Agent→account links | `objects.db` → `db_agent_identity_links` | **Missing** |
| Memory bundles (presets) | `objects.db` → `db_memory_bundles` | **Missing** |
| Drone definitions | `objects.db` → `db_drone_definitions` | **Missing** |

### 1.2 What stays per-channel/version (intentionally)

| Data | Reason |
|---|---|
| `db_block`, `db_tab`, `db_window`, `db_workspace` | Session/pane layout — legitimately ephemeral |
| `db_agent_instances` | Already global via shared registry |
| `db_drone_runs` | Run history — ephemeral execution records |
| CEF cache, IPC locks, sagas, logs | Runtime state |

---

## 2. Solution

### 2.1 New shared store

Add `~/.agentmux/shared/store.db` — a single shared SQLite for all durable user content that is not already in `shared/agents/`.

```
~/.agentmux/
├── channels/<ch>/versions/<v>/data/db/objects.db   ← session state only (unchanged)
└── shared/
    ├── agents/
    │   ├── definitions/     ← already global
    │   ├── registry/        ← already global
    │   └── transcripts/     ← already global
    └── store.db             ← NEW: identity + memory + drone + muxbus creds
```

`store.db` contains the following tables (identical schema to their `objects.db` counterparts):
- `db_identity_accounts`
- `db_identity_bundles`
- `db_identity_bindings`
- `db_agent_identity_links`
- `db_memory_bundles`
- `db_drone_definitions`
- `db_muxbus_credentials`

### 2.2 Why one shared store.db rather than separate files

Identity accounts form a relational cluster (`account → bundle → binding → agent link`). Unlike agent definitions (one document per agent, no joins), these tables have FK relationships that need atomic multi-row reads. A single SQLite handles this cleanly. Memory bundles and drone definitions are similar — standalone but benefit from the same access pattern.

`store.db` has its own `PRAGMA user_version` for independent schema evolution. It is completely separate from any per-version `objects.db`.

### 2.3 Path resolution

Add to `agentmux-srv/src/registry/paths.rs`:

```rust
/// Resolve `~/.agentmux/shared/store.db` — the global store for identity,
/// memory, and drone definitions. Resolves via the same root as the agent
/// registry so `AGENTMUX_HOME_OVERRIDE` and `AGENTMUX_SHARED_DIR` work.
pub fn resolve_shared_store_path() -> Option<PathBuf> {
    resolve_global_shared_root().map(|h| h.join("store.db"))
}
```

### 2.4 Opening the shared store

In `main.rs`, open `shared_store` alongside `wstore` at startup:

```rust
let shared_store: Option<Arc<SharedStore>> =
    registry::resolve_shared_store_path().and_then(|path| {
        match SharedStore::open(&path) {
            Ok(s) => {
                tracing::info!(path = %path.display(), "shared store opened");
                Some(Arc::new(s))
            }
            Err(e) => {
                tracing::warn!(error = %e, "shared store open failed — identity/memory/drone stay per-channel");
                None
            }
        }
    });
```

`SharedStore` is a thin wrapper around `rusqlite::Connection` with the same WAL + busy-timeout setup as `Store`. Its methods (`identity_upsert`, `memory_upsert`, `drone_def_upsert`, etc.) mirror the identically-named methods on `Store`.

### 2.5 Read/write routing

All RPC handlers that touch the globalized tables route through `shared_store` when available, falling back to `wstore` otherwise:

```rust
fn identity_store(shared: &Option<Arc<SharedStore>>, wstore: &Arc<Store>) -> impl IdentityOps {
    shared.as_deref().map(|s| s as &dyn IdentityOps)
           .unwrap_or(wstore.as_ref())
}
```

This means: **if `shared_store` is None (open failed), behavior is identical to today** — no regression.

### 2.6 Cross-version reactivity

Each version runs its own `agentmux-srv`. They can't share a WPS broker. Use **SQLite `data_version` polling**:

- On startup, spawn a background task that polls `PRAGMA data_version` on `store.db` every 1 second.
- On version increment, re-read changed tables and emit the existing WPS events (`identitybundlebindings:changed`, `bundles:changed`, etc.) to this version's renderer.
- Cost: one SQLite read/sec per running instance. Acceptable.

This is the same approach proposed in `docs/specs/archive/SPEC_SHARED_BUNDLES_AND_DEFINITIONS_2026_05_19.md §2.4`.

---

## 3. Migration

### 3.1 One-shot backfill

On startup, after opening `shared_store`, check for a `~/.agentmux/shared/.identity_memory_migrated` marker. If absent:

1. Enumerate every `channels/*/versions/*/data/db/objects.db` under `~/.agentmux`.
2. For each, open read-only and extract the 7 target tables.
3. Insert into `shared_store` with `INSERT OR IGNORE` (first-seen wins; dedup by `id`).
4. Write the marker.

Source DBs are **never modified** (same discipline as agent-definition backfill). The per-version rows stay in `objects.db` as fallback. A follow-up release (after one stable cycle) drops the shared tables from the per-version schema.

### 3.2 Conflict resolution

`INSERT OR IGNORE` on `id` — first instance seen wins. For identity accounts this is correct: the same account id across versions is the same account. For memory bundles, the same `id` + `name` is the user's same preset. If the user edited a preset in a newer version and the old version row has the same id, the newer version's row should win — resolve by scanning versions in ascending order (`updated_at DESC` tiebreak within the same `id`).

Implementation: sort source DBs by file mtime ascending before scanning; later mtime = more recent data.

### 3.3 Marker gating (idempotent)

Same as `migrate_from_sqlite_once`: write a `.identity_memory_migrated` marker in `shared/` after successful scan. Future startups skip. If the scan partially fails (a locked or corrupt source DB), skip that source, finish the others, still write the marker. A corrupt source DB in an old channel must not block cross-channel access for the healthy ones.

---

## 4. MuxBus credentials

`db_muxbus_credentials` is a global singleton (one row, `id='global'`). Its current home is per-version `objects.db`. The symptom: login to muxbus in one build, launch a new version, and the cloud subscriber stays disconnected.

With `store.db`, `muxbus_load()` and `muxbus_save()` on `Store` are replaced with reads/writes to `shared_store`. The backfill copies the `global` row from whichever source DB has a non-expired `access_token`.

---

## 5. Tables NOT globalized here

### `db_agent_identity_links` (legacy)

This junction (`agent_id → account_id`) predates identity bundles and is kept only for the migration path (`identities.rs` line 13). It should be globalized alongside `db_identity_accounts` (an account has no meaning without the link). However, since `agent_id` references `db_agent_definitions(id)` (now in `shared/agents/definitions/`), the FK must be dropped in `store.db` (cross-DB FK enforcement is impossible in SQLite). The link is enforced at the application layer instead.

### `db_drone_runs`

Run history is ephemeral — records reference block IDs and session IDs that are per-channel. Stays in `objects.db`.

---

## 6. Implementation phases

### Phase 1 — Core plumbing (1 PR)

- `registry/paths.rs`: `resolve_shared_store_path()`
- New `agentmux-srv/src/backend/storage/shared_store.rs`: `SharedStore::open()`, schema identical to the 7 target tables, own `PRAGMA user_version = 1`
- `main.rs`: open `shared_store` after `wstore`; wire into `AppState`
- All RPC handlers for identity/memory/drone/muxbus route through `shared_store` (with `wstore` fallback)
- Tests: `SharedStore` opens in-memory; CRUD round-trips

### Phase 2 — Backfill (1 PR)

- `shared_store::migrate_from_objects_dbs_once(home, shared_store)` — scan, backfill, marker
- Called from `main.rs` after `shared_store` opens
- Tests: backfill from a temp dir with two synthetic `objects.db` files; verify dedup

### Phase 3 — Cross-version polling (1 PR)

- Background task polling `PRAGMA data_version` on `store.db`
- On change: re-read + emit WPS events
- Tests: two in-process `SharedStore` handles on the same file; write on one, verify event emitted on the other

### Phase 4 — Cleanup (1 PR, one stable cycle later)

- Drop `db_identity_accounts`, `db_identity_bundles`, `db_identity_bindings`, `db_memory_bundles`, `db_drone_definitions`, `db_muxbus_credentials` from `objects.db` schema
- Remove `wstore` fallback paths
- Bump `OBJECT_SCHEMA_VERSION`

---

## 7. Non-goals

- Cross-machine sync (out of scope)
- Encrypting `store.db` at rest (the actual secrets are in OS keychain; only metadata/pointers in DB)
- Migrating OS keychain entries (they're already global by construction)
- Moving `db_block` / `db_tab` / `db_window` / `db_workspace` (session layout is correctly per-channel)

---

## 8. Open questions

1. **Drone definitions large?** Drone graphs (nodes + edges JSON) can be a few KB each. SQLite handles this fine — not a concern.
2. **Multiple simultaneous writers?** Two versions writing to `store.db` concurrently is safe — WAL mode + 5s busy timeout (same config as `objects.db`). The only conflict scenario is two instances creating an identity bundle with the same ID simultaneously, which is prevented by `INSERT OR IGNORE` + UUIDs.
3. **`db_agent_skills` and `db_agent_content`?** These are owned by `db_agent_definitions` (FK cascade) and already travel with definitions via the JSON file store. Not needed in `store.db`.
