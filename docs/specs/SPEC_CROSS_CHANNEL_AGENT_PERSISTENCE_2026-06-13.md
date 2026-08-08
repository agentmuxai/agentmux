# SPEC — Cross-Channel Agent Persistence

- **Date:** 2026-06-13
- **Status:** Proposed — **high priority** (user-flagged "critical") (implemented — see note below)
- **Owner:** AgentA

> **2026-08-07 audit note:** Implemented, foundational — "cross-channel agent
> persistence" is named directly in current code (`paths.rs`) and is
> load-bearing per `CLAUDE.md`. Badly stale status for a doc flagged
> critical-priority. See
> `docs/reports/REPORT_DOCS_AND_DEAD_CODE_CLEANUP_AUDIT_2026_08_07.md`.
- **Related:** `SPEC_VERSION_ISOLATION_2026_06_01.md` (§5 Phase 2 introduced per-version data), `SPEC_UNIFIED_AGENT_HISTORY_STORE_2026-06-10.md` (transcript store), data-isolation discussion **#1026**, `RESEARCH_PER_VERSION_DATA_ISOLATION_2026_05_24.md`, `scripts/import-agents.sh` (the current manual workaround this spec retires)

---

## 1. Problem

A user's agents (e.g. **Maksi / Mopeo / Spixo**) do **not** follow them across builds. They are trapped in the `objects.db` of the **specific `(channel, version)`** that created them, so:

- Launching a **new version** in the **same channel** boots an empty roster (observed 2026-06-13: a fresh `0.44.2` build on channel `local-agenta-replacechild-diagnos-3f4e60` came up with templates only; the user's three agents had to be copied in by hand).
- Launching a **different channel** (a different branch's local build, `stable` vs `local`, or a `--fresh` data dir) shows a *completely* different/empty roster.
- The only recovery today is `scripts/import-agents.sh`, a brittle manual catalog-and-copy across every `objects.db` on disk. It is the wrong layer (a shell script reaching into SQLite) and silently mis-fired in practice (see its hardening PR).

The user requirement is explicit: **agents must persist across channels** (and, by extension, across versions within a channel). An agent the user created once should be available in every AgentMux build they run, exactly like their provider login already is.

### 1.1 Why it's like this today

`agentmux-common/src/data_paths.rs` version-scopes the mutable runtime dirs:

```
channels/<ch>/versions/<v>/data/   ← objects.db (agents live HERE today)   PER (channel, version)
channels/<ch>/versions/<v>/cef-cache/                                       PER (channel, version)
channels/<ch>/versions/<v>/runtime/  ← ipc-port, lock                       PER (channel, version)
channels/<ch>/config/              ← settings                               CHANNEL-wide
channels/<ch>/agents/              ← file registry (registry/ subdir)       CHANNEL-wide (but unused as source-of-truth)
~/.agentmux/shared/                ← provider homes, transcripts            GLOBAL (cross-channel) ✅
```

Agent **definitions + instances** are written to the per-version `objects.db` (`db_agent_definitions`, `db_agents`, `db_agent_instances`, `db_agent_content`, `db_agent_skills`). The channel-wide `agents/registry/` tree **exists** but in practice holds only a `.migrated_from_sqlite` marker — it is not the authoritative read path. So agents inherit the *narrowest* scope (`channel × version`) when they should inherit the *widest* (global).

## 2. Key insight — most of the work is already done

1. **Transcripts are already global.** Conversation history lives in the provider CLI's home under `~/.agentmux/shared/providers/<provider>/projects/<cwd-slug>/<sid>.jsonl` (cross-channel by construction; `claude --resume` reads it by cwd). The *heavy* data already persists everywhere. Only the **lightweight agent metadata** (definition + the instance row that links an agent to its session/cwd) is trapped per-version.
2. **A cross-process-safe file registry already exists.** `agentmux-srv/src/registry/store.rs` — `Registry` is file-per-agent (`<root>/<uuid>.json`) with a `retired/<uuid>.json` tombstone tree, atomic-rename concurrency safety, an internal merge `Mutex`, and forward-compatible schema validation (`registry/schema.rs`, `MAX_SUPPORTED_SCHEMA`). This is precisely the store a global roster needs.
3. **A SQLite→registry consolidator already exists.** `registry/migrate.rs::migrate_from_sqlite_once(shared_home, registry)` scans `<root>/versions/*/data/db/objects.db`, dedups by `instance_id` (latest `started_at` wins), ORs the `display_hidden` tombstone flag, reads SQLite read-only, and drops a `.migrated_from_sqlite` marker for idempotency.

**The gap is a single architectural decision: the registry is rooted per-channel (`channels/<ch>/agents/registry/`) instead of globally.** Re-root it under `~/.agentmux/shared/` and make it the authoritative read path, and agents persist across channels and versions automatically.

## 3. Goals / Non-goals

**Goals**
- G1 — An agent created in any build is visible in **every** other build (any channel, any version), without a manual import.
- G2 — Its conversation **resumes** in the new build (definition + instance→session/cwd linkage travel with it; transcript is already global).
- G3 — One authoritative, cross-process-safe store; retire `scripts/import-agents.sh` as a normal-path tool.
- G4 — Zero data loss / no regression for existing per-version data (migrated, not abandoned).

**Non-goals**
- N1 — Globalizing **runtime/environment state** (pane layout, window geometry, virtualization, sagas, cef-cache, IPC locks). These are legitimately `(channel, version)`-scoped and stay put.
- N2 — Changing provider auth isolation (already global; unchanged).
- N3 — Cross-*machine* sync. This spec is single-machine, single `~/.agentmux`.

## 4. Proposed design

### 4.1 The data boundary

| Data | Scope | Where |
|---|---|---|
| Agent **definition** (name, icon, provider, type, system prompt/content, skills, working dir, flags) | **GLOBAL** | `~/.agentmux/shared/agents/registry/<uuid>.json` |
| Agent **instance** identity + **session/cwd linkage** (what `My Agents` lists; what `--resume` needs) | **GLOBAL** | same registry record (`instance_id`, `session_id`, `working_directory`, `display_hidden`) |
| Conversation **transcript** | **GLOBAL (already)** | `~/.agentmux/shared/providers/<provider>/projects/…` |
| Provider **auth** | **GLOBAL (already)** | `~/.agentmux/shared/providers/<provider>/` |
| Pane/window/layout/virtualization state, sagas, cef-cache, ipc-port, lock | **PER (channel, version)** | `channels/<ch>/versions/<v>/…` (unchanged) |
| Channel **settings** | **CHANNEL-wide** | `channels/<ch>/config/` (unchanged) |

### 4.2 Re-root the registry

In `data_paths.rs`, introduce a **global** agents root and point the registry at it:

```
// today:  agents_dir = instance_dir.join("agents")              // channels/<ch>/agents
// new:     global_agents_dir = shared_dir.join("agents")         // ~/.agentmux/shared/agents
//          registry root      = global_agents_dir.join("registry")
```

`shared_dir = root.join("shared")` already exists in `DataPaths`. `Registry::open(global_agents_root.join("registry"))` is the only change to construction. `Registry::agents_root()` (parent of root) then resolves to `~/.agentmux/shared/agents`, which is also the natural base for expressing agent working-directory subpaths.

### 4.3 Read path (authoritative source of truth)

- Agent definition/instance **reads** (`agent_def_list`, `instance_list`, `My Agents`) resolve from the **global registry** first.
- During migration (Phase P1), if the registry has no record for an id the running version's local `objects.db` still has, fall back to SQLite (so nothing disappears mid-rollout). After cutover (P3), the registry is the sole source.

### 4.4 Write path

- Definition/instance **writes** go to the **global registry** (atomic-rename upsert) and, during transition, **dual-write** to the local `objects.db` so an older binary on the same machine still sees them. The existing `backend/storage/registry_mirror.rs` already mirrors instances → registry; invert/extend it so the registry is primary and SQLite is the mirror.

### 4.5 Seeding

`backend/agent_seed.rs::auto_seed_on_startup` seeds **templates** when the store is empty and reseeds on manifest change. Point it at the global registry: templates seed **once globally**, not once per `(channel, version)`. `is_seeded=1` template rows stay distinguishable from user agents (`is_seeded=0`), so global de-dup and "trim to my agents" stay trivial.

## 5. Migration (one-time, idempotent, global)

Generalize `migrate_from_sqlite_once`:

1. Scan **every** `~/.agentmux/channels/*/versions/*/data/db/objects.db` (today it scans one channel's `versions/*`). Reuse the existing resilient probe discipline — skip unreadable / pre-agent-schema / locked DBs with a warning, never abort the whole pass (mirror the `import-agents.sh` hardening: a single bad/`*.bak` DB must not poison the run).
2. Merge `is_seeded=0` rows into the global registry: dedup by `id`; on conflict keep the row with the latest `updated_at`/`started_at`; OR the hidden/tombstone flag (any "forget" intent is preserved).
3. Write a **global** `.migrated_from_sqlite` marker under `~/.agentmux/shared/agents/registry/` so it runs at most once.
4. Leave SQLite untouched (read-only) — rollback is just "ignore the registry."

This subsumes `scripts/import-agents.sh`: the same catalog-and-merge, done once in-process at the right layer.

## 6. Edge cases

- **ID / slug collisions across channels.** Different channels may hold same-named agents with different ids (e.g. `Maks` vs `Maksi`, or a re-created `Mopeo`). Dedup by **id** (UUID, globally unique); on same-id conflict, newest-write wins. Same-slug/different-id collisions resolve by the registry's existing collision logic (slug suffixing), surfaced as a one-time merge note, never a silent drop.
- **Tombstones / "forget".** Deletes must be global tombstones (`retired/<uuid>.json`), so deleting an agent in one build doesn't have it resurrected by another build's stale SQLite on next migration. The `display_hidden` OR-merge already models this; the history-store `delete`/`clear` (SPEC_UNIFIED_AGENT_HISTORY_STORE) must write tombstones, not just rows.
- **Provider-auth coupling.** A global agent references a provider; provider homes are already global, so auth travels too. An agent whose provider is logged-out in a given environment simply shows "not authenticated" (existing behavior).
- **`--fresh` semantics.** `--fresh` today mints an isolated channel for a throwaway *runtime*. Decision needed (Open Q1): does `--fresh` still inherit the **global agent roster** (recommended — definitions are global; only runtime is throwaway) or get a truly empty slate? Proposed default: inherit definitions, isolate runtime; add `--fresh-agents` for a clean roster.
- **Concurrent instances.** Multiple builds run in parallel (I1–I6 isolation invariants). The registry's atomic-rename + per-record files make concurrent writes safe without a shared SQLite writer. No new lock contention.
- **Forward/back compat.** Keep the registry's existing rule: never write a higher-schema record into a lower schema, never overwrite an unparseable file (`store.rs` §6). An older binary on the same machine keeps working off SQLite during the dual-write window.

## 7. Rollout phases

- **P0 — Re-root + migrate (read).** Point the registry at `~/.agentmux/shared/agents/registry/`; run the global migration on startup; make `My Agents` read registry-first with SQLite fallback. *No write changes yet.* → agents become **visible** everywhere immediately.
- **P1 — Registry-primary writes (dual-write).** Definition/instance create/update/delete write the registry first, mirror to local SQLite. Tombstones on delete.
- **P2 — Cutover.** Registry is the sole source of truth; SQLite mirror becomes best-effort/diagnostic only.
- **P3 — Retire.** Drop the per-version `db_agent_*` tables from the schema; delete `scripts/import-agents.sh`; remove the SQLite fallback.

Each phase is independently shippable and reversible (drop back to SQLite). P0 alone solves the user-visible problem.

## 8. Open questions

1. **`--fresh` roster:** inherit global definitions (recommended) or empty slate? (See §6.)
2. **Instances vs definitions:** do running *instances* (not just definitions) go fully global, or do we global-ize the *definition + last session linkage* and let each environment own its live instance row? Leaning: definition + session/cwd linkage global; live pane/run-state per-(channel,version).
3. **Channel-scoped opt-out:** is there a real need to *isolate* a channel's agents (e.g. a destructive test channel)? If so, add an explicit `AGENTMUX_AGENTS_SCOPE=channel|global` override; default `global`.
4. **Interaction with the unified history store** (SPEC_UNIFIED_AGENT_HISTORY_STORE_2026-06-10): that spec already globalizes transcripts and adds delete/clear. This spec should share its tombstone model so "clear conversation" and "delete agent" agree.

## 9. Acceptance criteria

- AC1 — Create agent **A** in a build on channel X/version v1. Launch a build on channel **Y**/version **v2** → **A** appears in `My Agents` with no manual step.
- AC2 — Messaging **A** in the Y/v2 build resumes its existing conversation (session linkage travelled).
- AC3 — Deleting **A** in any build keeps it gone in all builds (global tombstone; no resurrection after re-migration).
- AC4 — Templates seed exactly once globally; `is_seeded=0` user agents remain cleanly separable.
- AC5 — Existing users lose nothing: first launch after upgrade migrates every channel's per-version agents into the global roster.

---

## 10. Architecture rationalization

Promoting agents to the global tier is not a bolt-on — it forces (and rewards) a cleanup of the whole data-dir layering. The guiding principle:

> **Scope each datum by its lifetime, not by accident of when it was added.** Three tiers, three lifetimes.

### 10.1 The three-tier model

| Tier | Scope key | Holds | Lifetime / rationale |
|---|---|---|---|
| **Identity** | global — `~/.agentmux/shared/` | agent definitions, conversations/transcripts, provider auth | *the user.* Must **never** fragment across builds. |
| **Preference** | channel — `channels/<ch>/config/` | settings, per-branch feature flags | *how this branch-line behaves.* Differs intentionally between `stable`, `local-<branch>`, `beta`. |
| **Runtime** | `(channel, version)` — `channels/<ch>/versions/<v>/` | `objects.db` (panes/sagas/layout), `cef-cache/`, `runtime/` (ipc-port, lock), `logs/` | *ephemeral execution state.* Regenerable; kept per-version so two concurrent versions never share a SQLite writer or Chromium cache (isolation invariant **I6**). |

### 10.2 The role of a "channel" after this change

A channel does **double duty**, and that is intentional — not cruft:

1. **Preference scope** — `channels/<ch>/config/` (settings for that branch-line).
2. **Runtime namespace** — the `<ch>` segment in `channels/<ch>/versions/<v>/` is what keeps `local-branchA`'s runtime from colliding with `local-branchB`'s at the same version.

What a channel **stops** being is an **identity/content scope**. Today it fragments the agent roster; after this change it never touches identity. This is the single conceptual shift the whole spec turns on.

### 10.3 Cleanup targets (cruft this move exposes)

| # | Cruft | Disposition |
|---|---|---|
| 1 | Channel-scoped `channels/<ch>/agents/registry/` (currently holds only a `.migrated_from_sqlite` marker — never the read-path of record) | Re-root to global `~/.agentmux/shared/agents/registry/`; becomes *the* registry (P0). |
| 2 | Duplicate **channel-root** `channels/<ch>/data/db/objects.db` — Phase-1 legacy, byte-identical to the live `versions/<v>/data/db/objects.db` (verified 2026-06-13) | Stale dead weight. Remove once nothing reads it; confirm no code path still resolves the pre-Phase-2 `data/` path. |
| 3 | Per-version agent tables `db_agent_definitions` / `db_agents` / `db_agent_instances` / `db_agent_content` / `db_agent_skills` | Become vestigial after registry cutover → drop in P3. |
| 4 | `registry/migrate.rs` parameter named `shared_home` that is actually the **channel** dir (`instance_dir`), not global `shared/` | Rename + re-point at the true global root as part of generalizing the migration to scan all channels. |
| 5 | `scripts/import-agents.sh` (manual catalog-and-copy across every `objects.db`) | Obsolete once the in-process global migration ships — delete in P3. |

### 10.4 What this is *not*

Not a flattening. The **version** tier is load-bearing (concurrent-version SQLite/cache safety, I6) and stays. The cleanup is strictly about moving **identity** data out of the runtime tier into the global tier — every other tier keeps its current job, just with clearer boundaries and the legacy duplicates removed.

### 10.5 Optional further rationalization (out of scope for P0–P3, noted for completeness)

`config/` is itself a mix: some settings are true user **preferences** (theme, keybindings) that arguably belong in the **identity** tier (global), while others are genuinely branch-specific (debug/feature flags). A later pass could split settings into *global preferences* + *channel overrides*, shrinking the channel's exclusive domain further. Deliberately deferred — agents are the critical, user-blocking case; settings are not.

---

## 11. Code-map findings & revised P0 plan (verified 2026-06-13)

A full read of the storage layer corrects two premises in §1–§7 and reshapes P0. **The §7 four-phase arc still holds; P0 is what changes.**

### 11.1 Verified current state (corrects §1.1)

- The file **instances** registry is **not unused** and **not per-channel-empty**. It is rooted at `channels/<ch>/agents/registry/` (`registry/paths.rs::resolve_shared_home()` walks `AGENTMUX_DATA_DIR.ancestors().nth(3)` = the **channel** dir, *not* `~/.agentmux`). It is **channel-scoped, version-spanning**.
- It **is the authoritative read** for `listnamedagents` (the "Continue agent" roster) whenever attached at startup, with SQLite `instance_list_named` as fallback (`server/agent_handlers.rs:1530`). It is **not** merely mirrored-to.
- The **definitions** roster — `listagents` → `agent_def_list()` → `db_agents` (`backend/storage/agents.rs:364`) — is **pure SQLite and never consults the registry**. This is the read path that "My Agents" definitions come from.
- Writes: **SQLite is primary, registry is a best-effort mirror** (`backend/storage/registry_mirror.rs`; failures logged, never propagated). Seeding (`agent_seed.rs`) is SQLite-only.
- The one-time migration scans only the **current channel's** `versions/*` (`registry/migrate.rs:66,80`).

### 11.2 The blocker — the record can't stand alone (corrects §4.1/§7)

`registry/schema.rs::NamedAgentRecordV1` is an **instance projection**: `instance_id`, `definition_id` (FK), `identity_id`/`memory_id` (FK), `working_dir` (relative to `<home>/agents/`), timestamps. It carries **neither `session_id` nor any definition body** (name/provider/agent_type/content/skills/is_seeded…). Today the read path *joins* those from the **current channel's** SQLite (`agent_def_list`). Therefore a re-rooted global registry alone would surface rows it **cannot name, cannot describe, and cannot resume** — the join target (current-channel SQLite) won't have other channels' definitions or the session id.

**Conclusion:** true cross-channel persistence needs the record to be self-contained, and the definitions roster to have a global source. Re-rooting is necessary but **not sufficient**.

### 11.3 Design decision — separate global **definitions** registry (normalized)

Two options were considered for carrying definitions globally:
1. **Embed** a definition snapshot inside each instance record. Simple read; suffers snapshot staleness on every definition edit.
2. **Separate global definitions registry** (file-per-definition under `shared/agents/definitions/`), alongside the existing instances registry. Normalized, mirrors the SQLite def/instance split, and naturally gives `agent_def_list` a global source.

**Decision: Option 2.** Definitions and instances are distinct entities (as in SQLite); keep them distinct globally. Instance records gain only `session_id` (the one cross-cutting field they need for resume).

### 11.4 Revised P0 (replaces the single P0 bullet in §7)

- **P0.1 — `session_id` on the instance record.** Bump `NamedAgentRecord` schema to v2 (`MAX_SUPPORTED_SCHEMA` 1→2, `MIN`=1 for back-compat); add `session_id` (+ `status` if cheap); `registry_mirror.rs` populates it. Additive, low-risk. *Enables cross-channel resume.*
- **P0.2 — Global definitions registry.** New file-per-definition store at `~/.agentmux/shared/agents/definitions/`; the def write path (`agent_def_insert/update/delete/set_hidden`) mirrors to it; `agent_def_list` reads global-first with SQLite fallback. *This is what makes cross-channel agents appear in My Agents.*
- **P0.3 — Re-root the instances registry to global** `~/.agentmux/shared/agents/registry/` (change `resolve_shared_home()` to the true `~/.agentmux`, decoupled from `AGENTMUX_DATA_DIR`), and **generalize the migration** to scan `channels/*/versions/*/data/db/objects.db` (+ `dev/*`).
- **P0.4 — Read-path reconstruction.** Cross-channel rows enrich from the **global definitions registry** instead of current-channel SQLite; reconstruct `working_directory` correctly for migrated rows.

### 11.5 Landmines (must-handle)

1. **Per-DB working-dir anchoring in migration.** Legacy `working_directory` values are absolute under each row's **own** `channels/<ch>/agents/…`. Stripping them against the *new global* `agents_root` will mark rows "unmappable" (`migrate.rs:271-276`). Derive each row's channel from its source `objects.db` path and strip against **that** channel's agents dir.
2. **Concurrent multi-process writes.** After re-rooting, **every** srv (one per running channel/version) writes the **same** global registry. Cross-process safety rests on atomic rename (`registry/atomic.rs`) + the `retired/` tombstone tree — verify it holds under simultaneous startup migrations.
3. **Marker relocation.** `.migrated_from_sqlite` moves with the registry root → migration re-runs once into the global location (idempotent via `exists_anywhere`). Expected, acceptable.
4. **Mirror excludes continuation rows** (`registry_mirror.rs:120`) — revisit if the global roster must show the latest continuation.
5. **`AGENTMUX_AGENTS_DIR`** already exists but is channel-scoped (`channels/<ch>/agents`) — do **not** overload it; add an explicit global var (e.g. `AGENTMUX_SHARED_AGENTS_DIR`).

### 11.6 Key files
`agentmux-common/src/data_paths.rs` (paths/env/ensure_dirs) · `agentmux-srv/src/registry/{paths.rs,schema.rs,migrate.rs,store.rs}` · `agentmux-srv/src/backend/storage/{agents.rs,registry_mirror.rs}` · `agentmux-srv/src/server/agent_handlers.rs` · `agentmux-srv/src/main.rs` (startup wiring) · `agentmux-srv/src/backend/agent_seed.rs`.
