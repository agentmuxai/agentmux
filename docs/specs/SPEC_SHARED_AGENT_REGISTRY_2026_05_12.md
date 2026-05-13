# Spec: Shared agent registry — cross-version "Continue agent" dropdown

**Status:** Spec (no implementation yet)
**Owner:** AgentA
**Date:** 2026-05-12
**Driving requirement:** "Agents are shared across versions. The Continue dropdown should show every agent I've ever named, no matter which agentmux version I launched it from. And if a future agentmux version doesn't understand an older agent's schema, it should fail soft — not erase or crash."

---

## 1. TL;DR

Today every portable/installed agentmux instance writes the `db_named_agents` table into its **per-version** SQLite database (`~/.agentmux/versions/<v>/data/db/objects.db`). Working directories at `~/.agentmux/agents/<name>-<suffix>/` are already shared across versions, but the metadata row that powers the launch-modal dropdown is not. Result: smoke-build a new version, launch the modal, dropdown is empty even though you have ten named agents on disk.

This spec carves out a **shared, file-per-agent registry** at `~/.agentmux/agents/registry/<instance_id>.json`, with a row-level schema version so:

- **Bulk per-version state** (tabs, blocks, layouts, identities, memories, sagas, filestore) stays in per-version SQLite. No bulk migration headache.
- **Cross-version state** (the "named agents" index — name, definition, working_dir, identity_id, memory_id, timestamps) lives in shared, version-tagged JSON files.
- **Forward compat**: a future agentmux that adds fields the current one doesn't understand stays readable. Unknown rows are skipped + logged, never deleted.
- **Backward compat**: an old agentmux that wrote a schema_version=1 row stays readable forever; we promise never to remove the v1 reader.

The launch modal's `ListNamedAgentsCommand` RPC keeps the same shape — it just sources rows from the file registry instead of the SQLite table. Working-dir reuse, identity/memory binding, and the Phase 4 `--continue` resume hook all stay intact.

Two PRs:

- **PR A — Parallel-write registry.** Add the registry resolver + writer; every `db_named_agents` insert/update/delete also writes/touches/deletes a registry file. RPC reads from SQLite still (no behavior change). Safe to roll back.
- **PR B — Cut over.** RPC reads from registry. One-shot migration on first launch copies every `db_named_agents` row from every version's SQLite into the shared registry. SQLite table is read-only legacy after this.

A future PR C (out of scope for this spec) drops the SQLite table after a release or two of dual-write.

---

## 2. Today's state

### What's per-version

`~/.agentmux/versions/<version>/data/db/`:

- `objects.db` — `db_named_agents`, `db_identities`, `db_memories`, `db_agent_instances`, `db_tabs`, `db_blocks`, …
- `filestore.db` — per-block file dataset
- `sagas.db` — saga state

Schema gated by `agentmux-srv/src/storage/schema.rs` migrations. Every version may add migrations; rolling back forward isn't supported.

### What's already shared

`~/.agentmux/agents/<name>-<suffix>/`:

- Working directory (one per named agent).
- Contains the CLI's project state: `.claude/`, repo checkouts, conversation transcripts, etc.
- Created by `allocate_agent_workdir` (`agentmux-srv/src/spawn/workdir.rs`).
- Filename collisions resolved by `-2`, `-3` suffixes; the registered name is the slugified user-typed name.

### The gap

Two pieces of state for the same logical "named agent" live in different places with different lifetimes:

| Piece | Where | Lifetime |
|---|---|---|
| Working directory | `~/.agentmux/agents/<slug>/` | Shared across versions, persists forever |
| Registry row | `~/.agentmux/versions/<v>/data/db/objects.db :: db_named_agents` | Per-version, dies when the user moves to a new version |

When the user upgrades from 0.33.821 → 0.33.822, the working directories are still there, but the modal can't find them.

---

## 3. Goals

1. **One source of truth for "named agents" across versions.** The launch modal's dropdown lists every agent the user has ever named, regardless of which version created it.
2. **Per-version SQLite stays intact.** No risky bulk migration of objects.db. Versions still own their tabs/blocks/identities/memories independently.
3. **Forward-compatible.** A 0.33.999 file can sit next to a 0.33.822 file. Old version reads what it understands, skips what it doesn't, logs the skip. **No row is ever silently dropped or destructively migrated.**
4. **Concurrency-safe.** Two portables (e.g. 0.33.821 + 0.33.822) can run side-by-side. Both can list, both can append, neither corrupts the other.
5. **Tolerant of partial writes.** If a registry file is corrupt, malformed, or half-written (crash mid-rename), the reader skips it + logs, and the next valid write self-heals.
6. **Migration is one-shot and idempotent.** First launch after PR B reads every per-version `db_named_agents` table, writes registry files, and never re-runs.

### Non-goals

- Sharing identities/memories across versions. Out of scope; separate PR with its own data-shape questions.
- Sharing tabs/blocks/sagas. Per-version stays per-version; that's an explicit design choice.
- Cross-machine sync. Local single-host only.
- Schema evolution beyond additive fields in this spec. Renames/removals get a deprecation-window plan in a follow-up.

---

## 4. Storage layout

### Directory

```
~/.agentmux/
├── agents/                              # already shared, untouched
│   ├── livelog-821-0512a/                # working dirs (existing)
│   ├── moams-05059/
│   ├── ...
│   └── registry/                         # NEW: shared agent registry
│       ├── 7c8e4f12-….json              # one file per named agent
│       ├── 1a9b2d8a-….json
│       ├── ...
│       └── retired/                      # NEW: tombstones
│           └── 3d2e1f4a-….json
```

**Why `agents/registry/` and not `~/.agentmux/registry/` directly:** colocation with working dirs. A backup script that snapshots `~/.agentmux/agents/` captures both halves of "the agent." Easier mental model.

### One file per agent

Path: `~/.agentmux/agents/registry/<instance_id>.json`
Encoding: UTF-8 JSON, 2-space indent (so `git diff` is readable when users version-control their `~/.agentmux/agents`).
Filename is the agent's UUID — globally unique, no slug collision possible.

### Tombstones (soft-delete)

Path: `~/.agentmux/agents/registry/retired/<instance_id>.json`

Retiring an agent moves the file from `registry/` → `registry/retired/` atomically. Keeps a forensic trail without cluttering the dropdown. The "Forget agent" right-click in Phase 3 of the named-agent spec maps to this move.

### Concurrency model

- **Create / update** = atomic write to a temp file in the same dir, then `rename()`. Filesystem guarantees on Windows + macOS + Linux: `rename` over an existing file is atomic. No reader ever sees a partial file.
- **Retire** = atomic `rename(registry/<id>.json, retired/<id>.json)`.
- **Hard delete** = `unlink(retired/<id>.json)`. Out of normal flow; only invoked by an explicit "purge" admin path (not exposed in UI initially).
- **Lock-free list**: readers `readdir + read each file`. A file that disappears mid-list (race with retire) just isn't in the result. A file that's mid-rename either has the old contents or the new contents, never half of either.
- **Two writers, same id**: last write wins. This is acceptable because the only multi-writer scenario is "two portables both update `last_launched_at_ms` for the same instance," and either timestamp is correct enough.

### Why not SQLite at the shared location

Considered. Rejected because:

- SQLite locks are per-database. A shared `registry.db` accessed by multiple portables fights over WAL/SHM files. We'd need careful timeouts + retry logic.
- Schema migrations on a shared DB mean *the newest version writes a schema older versions might not be able to read.* Whoever writes the migration breaks everyone else.
- File-per-agent gives us free per-row schema versioning — exactly what we need for the forward/backward compat story.
- No noticeable perf hit at the relevant scale (hundreds of agents, not millions).

---

## 5. File format

### v1 envelope

```json
{
  "schema_version": 1,
  "data": {
    "instance_id": "7c8e4f12-9a3b-4cba-bd60-1a9e8f7c0001",
    "instance_name": "livelog-821",
    "definition_id": "claude-code",
    "identity_id": "agenta",
    "memory_id": "default",
    "working_dir": "livelog-821-0512a",
    "created_at_ms": 1747049400000,
    "last_launched_at_ms": 1747061640000,
    "created_by_version": "0.33.821",
    "last_launched_by_version": "0.33.821"
  }
}
```

**Field rules:**

| Field | Type | Required (v1) | Notes |
|---|---|---|---|
| `schema_version` | u32 | yes | Envelope-level. Reader's first decision point. |
| `data.instance_id` | UUID string | yes | Must match filename. Mismatch = skip + log. |
| `data.instance_name` | string | yes | User-typed name, not slugified. |
| `data.definition_id` | string | yes | `"claude-code"`, `"gemini"`, … from widgets.json definitions. |
| `data.identity_id` | string \| null | yes | Null = no identity bound. |
| `data.memory_id` | string \| null | yes | Null = no memory bound. |
| `data.working_dir` | string | yes | **Relative** path inside `~/.agentmux/agents/`. Never an absolute path (portable home moves). |
| `data.created_at_ms` | i64 | yes | Unix epoch ms. |
| `data.last_launched_at_ms` | i64 | yes | Updated every launch. Used for dropdown sort order. |
| `data.created_by_version` | string | yes | Diagnostics only. |
| `data.last_launched_by_version` | string | yes | Diagnostics only. |

### Validation contract

A row that fails any of these is **skipped, not destroyed, not migrated, not auto-fixed**:

1. Top-level `schema_version` outside `[MIN_SUPPORTED, MAX_SUPPORTED]`.
2. Filename UUID ≠ `data.instance_id`.
3. Missing required field for the row's declared `schema_version`.
4. `data.working_dir` is absolute, contains `..`, or escapes the agents dir.
5. JSON parse error.

Skip logs the cause and emits a `skipped_registry_file` event into the broker for ops visibility (frontend can surface a one-line "1 agent record could not be loaded" in the Identity/Memory diagnostics pane).

---

## 6. Versioning contract

### Reader

A single resolver in `agentmux-srv/src/registry/schema.rs` declares:

```rust
pub const MIN_SUPPORTED_SCHEMA: u32 = 1;
pub const MAX_SUPPORTED_SCHEMA: u32 = 1;
```

Bumped per release:

- **Add an optional field** (e.g. v2 adds `tags: Vec<String>`): bump `MAX_SUPPORTED_SCHEMA` to 2. Writers populate. Readers default-fill if absent. Old version on the same machine still reads v2 files: parses the envelope, sees `schema_version=2 > MAX_SUPPORTED=1`, skips + logs. The user's dropdown is shorter on the old portable; nothing is lost.
- **Add a required field**: bump major-ish. Writers write v(N+1). Readers of v(N+1) require the field. Old readers skip. Migration of pre-existing v(N) files to v(N+1) is done in the first launch of the new release.
- **Remove or rename a field**: never in v1. If we ever need to, the policy is:
  - One full release cycle of dual-write: writers emit *both* old and new fields.
  - Bump `MIN_SUPPORTED_SCHEMA` in the release after that.
  - Old portables on disk that only understood the pre-change schema continue to find readable rows during the deprecation window.

### Writer

Always writes the latest schema known to this binary. Never writes a partial row "matching" an older schema. Never rewrites an existing higher-schema row into a lower schema.

### "Unknown field tolerance"

Inside `data`, **unknown JSON keys are preserved on round-trip when possible, ignored otherwise**:

- Read: `serde(deny_unknown_fields)` is OFF. Unknown keys are dropped from the deserialized struct.
- Write: writer always writes from its known struct shape. **An old reader doing a round-trip strips unknown fields**. To prevent this, the writer never re-writes a row it didn't conceptually mutate. `touch_last_launched` and similar update paths use **partial JSON merge** (read raw `serde_json::Value`, update only known fields, write back) so unknown fields from newer schemas survive an update by an older binary.

This is the most subtle part of the design and the most important for forward-compat. It's small enough to put in a helper:

```rust
fn merge_known_fields(
    existing: &mut serde_json::Value,
    updates: &serde_json::Value,
) {
    // existing = on-disk JSON parsed as Value (preserves unknown fields)
    // updates = serde_json::to_value(known_struct) where known_struct is
    //           a snapshot the caller built using only the fields it knows
    // result: existing has updates' top-level keys overwritten, all other
    //         keys (potentially from newer schemas) preserved.
    if let (Some(e), Some(u)) = (existing.as_object_mut(), updates.as_object()) {
        for (k, v) in u {
            e.insert(k.clone(), v.clone());
        }
    }
}
```

### Forward compat self-test

Every release adds one test fixture in `agentmux-srv/tests/registry-fixtures/`:

- `v1_minimal.json` — only required v1 fields. Must load.
- `v1_with_future_fields.json` — v1 envelope, but `data` includes a `tags: []` array we haven't shipped. Must load *and* survive round-trip (unknown field preserved).
- `v999_unknown_envelope.json` — `schema_version: 999`. Must be skipped + logged + not deleted.

These run on every PR. Catches the "we accidentally regressed forward-compat" class of bug.

---

## 7. RPC surface

No new RPCs vs the named-agent continuation spec. Internals change:

### `ListNamedAgentsCommand`

```ts
// Frontend signature unchanged from PR #816.
RpcApi.ListNamedAgentsCommand(TabRpcClient, {
  definition_id: "claude-code",
}): Promise<NamedAgentRow[]>
```

Backend (`agentmux-srv/src/server/rpc/named_agents.rs`):

```rust
pub fn list_named_agents(definition_id: &str) -> Vec<NamedAgentRow> {
    let registry = Registry::open(data_paths().home_dir.join("agents/registry"))?;
    let mut rows = registry
        .iter_active()                      // skips retired/, skips invalid
        .filter(|r| r.definition_id == definition_id)
        .collect::<Vec<_>>();
    rows.sort_by(|a, b| b.last_launched_at_ms.cmp(&a.last_launched_at_ms));
    rows
}
```

### `LaunchAgentCommand`

When `overrides.continue_of_instance_id` is set:
1. Read the registry row.
2. Resolve `working_dir` to absolute (`agents_root.join(row.working_dir)`).
3. Reuse identity / memory bindings.
4. After successful spawn, update `last_launched_at_ms` + `last_launched_by_version` via `Registry::touch(instance_id, now)`.

Failure handling:
- Registry file disappears between list and launch → return "Agent no longer exists" + log.
- Working dir disappears → return "Working directory missing" + offer to recreate (out of scope here, separate UX).

### `RetireNamedAgentCommand` (new, for Phase 3 right-click "Forget")

```ts
RpcApi.RetireNamedAgentCommand(TabRpcClient, {
  instance_id: string;
}): Promise<void>
```

Moves `<id>.json` → `retired/<id>.json` atomically. Working directory untouched (deliberate; user keeps the on-disk transcript).

### `ListRetiredNamedAgentsCommand` (out of scope for first cut)

For a future "Trash" view. Spec'd here for completeness, not implemented now.

---

## 8. Migration

### First launch after PR B ships

`agentmux-srv` startup invokes `registry::migrate_from_sqlite_once()`:

```rust
pub fn migrate_from_sqlite_once(data_paths: &DataPaths) -> Result<()> {
    let marker = data_paths.home_dir.join("agents/registry/.migrated_from_sqlite");
    if marker.exists() { return Ok(()); }

    let home = &data_paths.home_dir;
    let versions_root = home.join("versions");
    if !versions_root.is_dir() {
        std::fs::write(&marker, "no versions dir found\n")?;
        return Ok(());
    }

    let mut latest_by_id: HashMap<String, (i64, NamedAgentRow)> = HashMap::new();
    for v_entry in std::fs::read_dir(&versions_root)? {
        let v_dir = v_entry?.path();
        let db_path = v_dir.join("data/db/objects.db");
        if !db_path.exists() { continue; }
        for row in read_named_agents_readonly(&db_path)? {
            let key = row.instance_id.clone();
            let ts = row.last_launched_at_ms;
            match latest_by_id.entry(key) {
                Entry::Occupied(mut e) if e.get().0 < ts => { e.insert((ts, row)); }
                Entry::Vacant(e) => { e.insert((ts, row)); }
                _ => {}
            }
        }
    }

    let registry = Registry::open(home.join("agents/registry"))?;
    for (_, row) in latest_by_id.into_values() {
        if registry.exists(&row.instance_id)? {
            // A user who already ran a newer build has a registry row.
            // Don't clobber it.
            continue;
        }
        registry.create_if_missing(&row)?;
    }
    std::fs::write(&marker, chrono::Utc::now().to_rfc3339())?;
    Ok(())
}
```

**Properties:**
- **Idempotent.** Marker file ensures we run once. Manual delete = re-run.
- **Read-only on SQLite.** Migration never modifies `objects.db`. Old portables keep working unchanged.
- **Conflict resolution:** if the same `instance_id` exists in multiple per-version DBs (because the user launched it under two versions before this migration), the one with the latest `last_launched_at_ms` wins.
- **Don't overwrite a registry file that already exists.** Lets users who upgrade through multiple versions in sequence avoid clobbering newer state.
- **Marker file isn't authoritative**; deleting it just re-runs the merge, which is safe.

### What about the SQLite table after migration?

Stays. PR B keeps `db_named_agents` populated as a parallel write (so a rollback to a pre-registry release still finds rows it wrote itself). PR C, a release or two later, removes the parallel write. Schema migration removes the table.

### Rollback

- Roll back PR B alone → reverts the RPC source. Registry files stay on disk (harmless, ignored by older binary). SQLite table still authoritative.
- Roll back PR A alone → drops the parallel-write code. Registry files stop being updated but don't get deleted. Roll-forward simply re-syncs via the migrator.

---

## 9. Frontend

Zero changes to `AgentLaunchModal.tsx` from PR #816. Same `ListNamedAgentsCommand` shape, same dropdown gating, same `handleContinueSelect` flow. The backend is the only thing that changes shape.

The "1 agent record could not be loaded" footer in the modal (when the broker has emitted `skipped_registry_file` events since the modal opened) is **optional, Phase 2**. First cut is silent skip + log.

---

## 10. Code layout

```
agentmux-srv/src/registry/
├── mod.rs                 // pub use of below
├── schema.rs              // schema_version constants, NamedAgentRow struct, validate()
├── store.rs               // Registry struct: open/create/touch/retire/iter_active
├── atomic.rs              // write_atomic(path, bytes), rename_atomic(from, to)
├── migrate.rs             // migrate_from_sqlite_once + per-version DB readers
└── tests/
    ├── store_tests.rs
    ├── schema_tests.rs    // forward-compat fixtures live here
    └── migrate_tests.rs
```

`server/rpc/named_agents.rs` becomes a thin shim over `registry::store::Registry`.

`storage/named_agents.rs` (the existing SQLite layer) stays as-is in PR A, gets demoted to "writeback to legacy table" in PR B, gets removed in PR C.

---

## 11. Diagnostics

Add three things, all behind the existing diag/perf machinery so they cost nothing in production:

1. **Broker event `named_agent_registry_loaded`** on first list: `{ active_count, retired_count, skipped_count, slowest_file_ms }`. The launch modal listens to it for the optional "could not load N records" footer.
2. **Broker event `named_agent_registry_skipped`** per skip: `{ filename, reason }`. Surface in a diag panel; never in the modal directly (don't blame the user for a forward-compat skip).
3. **`muxlog srv` line per registry write**: `INFO registry: wrote 7c8e…json schema_version=1 fields=10 elapsed_ms=2`. Useful when triaging "why isn't my agent showing up."

No new perf probes — file IO at this scale is negligible compared to broker pub/sub overhead we already instrument.

---

## 12. Test plan

### Unit (Rust)

- **Atomic write**: `write_atomic` followed by a kill -9 emulation (temp file with no rename) leaves the original intact.
- **Forward-compat fixtures**: schema_version=999 fixture is skipped + reported, not deleted.
- **Round-trip preserves unknown fields**: simulate v2 input → v1 binary touches `last_launched_at_ms` → re-read with a v2-aware deserializer → `tags` field still present.
- **Filename / instance_id mismatch**: detected, skipped, logged.
- **Concurrent touch**: two threads update `last_launched_at_ms` for the same id; final value is one of the two timestamps (no merge nonsense).
- **Migration idempotency**: run `migrate_from_sqlite_once` twice. Second call no-ops.

### Integration (Rust + the srv test harness)

- **PR A**: insert a row via the legacy SQLite path; verify a registry file appears with matching contents.
- **PR A**: insert a row via the legacy path while a portable is mid-shutdown (simulate by dropping the connection); registry file is consistent (no half-written).
- **PR B**: cold-launch portable with two simulated per-version DBs (one with 3 rows from version A, one with 2 rows + 1 overlap from version B). Registry has the union with the overlap's newer timestamp winning.

### Replay (frontend)

- New fixture in `frontend/test/fixtures/agent-sessions/`: a launch-modal-open event sequence with the dropdown populated from the broker's `named_agent_registry_loaded` event.
- Replay verifies the dropdown order matches the sort contract.

### Manual smoke

- Build 0.33.823 (PR B). Verify dropdown shows `livelog-821` (created on 0.33.821) without further action.
- Launch from dropdown. Verify working dir is reused, identity/memory pre-locked.
- Switch to portable 0.33.821, launch a *new* agent there. Switch back to 0.33.823. Reopen modal. New agent appears (because the parallel write path during PR A is still live until PR C).

---

## 13. Risks & mitigations

| Risk | Mitigation |
|---|---|
| Two concurrent portables both `touch` the same row — clobbering a field a newer version wrote. | Partial-merge writer (§6) preserves unknown fields. For known fields, last-write-wins on a single `last_launched_at_ms` is fine. |
| A registry file gets corrupted (disk error, antivirus quarantine, user edit). | Skip + log + emit broker event. Don't delete. User can edit JSON by hand to repair. |
| Migration runs on a system with 30+ versions and stalls startup. | Migration is read-only + bounded by total agent count, not version count. ~1ms per row in practice. Worst-case 100 agents × 30 versions = ~3s, behind a marker file so it runs once. |
| User deletes `~/.agentmux/agents/registry/.migrated_from_sqlite` thinking it's junk. | Migration is idempotent; re-runs are harmless (won't overwrite existing registry files). |
| A future agentmux ships a v2 schema and an old portable on the same machine loses access to v2-only rows. | By design. The old portable still launches those agents *if it learns their id from somewhere else*, but the dropdown is shorter. We accept this. The alternative — auto-downgrading v2 → v1 — destroys data. |
| Working-dir collision: two registry rows point to the same `working_dir`. | Detected on launch; latest-modified registry row wins; the other is logged as a stale duplicate. Should never happen in normal flow (UUID-keyed rows). |
| Path traversal in `working_dir`. | Validated on read (§5) — must be relative, no `..`, must resolve inside agents root. |

---

## 14. Out of scope

- Sharing identities or memories across versions. Tracked under data-dir unification (see `reference_data_dir_unification_plan.md`); decided independently.
- Cloud sync of the registry.
- A trash/restore UI for retired agents.
- A registry-level audit log (`who last touched this row when from which version`). The per-row `last_launched_by_version` is enough for v1.

---

## 15. PR sequence

### PR A — Parallel-write registry (no behavior change)

- New `agentmux-srv/src/registry/` module per §10.
- Every existing `db_named_agents` insert/update/delete call site also writes the registry file.
- RPC reads still come from SQLite.
- Migration code added but **not invoked** yet.
- Bump patch. Smoke: confirm registry dir fills up as the user launches agents. Confirm SQLite still authoritative.
- Tests: unit + integration per §12.

### PR B — Cut over reads, run one-shot migration

- `list_named_agents` / `launch_named_agent_continue` / `retire_named_agent` switch source to `Registry`.
- `migrate_from_sqlite_once` runs on srv startup behind the marker file.
- Parallel write to SQLite is **kept** (for one-version rollback safety).
- Add forward-compat test fixtures (§6).
- Bump patch. Smoke: 0.33.821-created agents appear in 0.33.823's dropdown.

### PR C (future, not this spec) — Retire the SQLite table

- Drop parallel write.
- Add a schema migration that deletes `db_named_agents` from objects.db.
- Bump patch. Smoke: existing registry state intact, SQLite clean.

---

## 16. Open questions for review

1. **Encoding choice**: pretty-printed JSON (this spec's choice) vs `bincode` / `cbor` for smaller files? Pretty JSON wins on debuggability and `git diff`-ability; size at this scale (≤1KB per file) is irrelevant.
2. **Filename = UUID vs filename = slug**: UUID guarantees uniqueness but is unreadable. Slug is readable but collides. UUID + `instance_name` inside the file is the safe combo.
3. **Should retire be reversible from the UI** in v1? Currently no — the retired/ subdir exists but no "restore" affordance. Likely OK to defer until users actually ask.
4. **Should the migration also pull from old `~/.waveterm` directories**? Tracked under the AGENTMUX rebrand migration in `CLAUDE.md`; we said no there, and this spec inherits that decision.
