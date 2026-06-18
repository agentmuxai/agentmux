# Architecture — Agent data, channels, and cross-channel persistence

**Date:** 2026-06-13
**Why this doc:** "My Agents" (sessions) is empty in a fresh channel/version even though cross-channel persistence shipped (#1383–#1390). Stepping back to map the whole agent-data architecture, where it's documented, what's actually broken, and the architectural improvements worth making.

**Where it's documented:** the `agentmux-docs` repo is an Astro/typedoc **API site**, not prose architecture. **This is the canonical, code-anchored overview of the agent-data model.** Start here.

## 0. Canonical status, companions, and superseded docs

**This doc is the source of truth (dated 2026-06-13).** It is code-anchored — every claim is checked against the files cited in §7, not against aspirational specs.

**Active companion docs (still current, scoped to a slice this doc summarizes):**
- `docs/specs/SPEC_CROSS_CHANNEL_AGENT_PERSISTENCE_2026-06-13.md` — the live P0→P3 rollout plan for promoting agents to global scope.
- `docs/specs/SPEC_AGENT_ARCHITECTURE_2026_05_27.md` — the live tracker for the **separate** `db_agents` storage-consolidation effort (collapsing the 5 read-sides / 27 write-sites onto one table; Phase 3b/3c/R/O). Related to but distinct from cross-channel.
- `docs/specs/SPEC_UNIFIED_AGENT_HISTORY_STORE_2026-06-10.md` — conversation **transcripts/history** (orthogonal; transcripts are already global).
- `docs/specs/SPEC_NAMED_AGENT_CONTINUATION_2026_05_12.md` — the "continue agent" UX feature spec.
- `docs/specs/SPEC_DATA_CHANNELS_2026_05_24.md` — the **channel** model (Increment A is live; the schema-migration framework + import wizard in B/C are future work, not shipped).
- `docs/analysis/ANALYSIS_MULTI_CLONE_TASK_DEV_ISOLATION_2026-05-26.md` — a real, unimplemented dev-isolation bug (orthogonal).

**Superseded specs — kept in place, banner-marked 2026-06-13.** Each carries a `SUPERSEDED → this doc` header. They are *retained* (not deleted) because live code comments and other kept specs cite them as design rationale — but their architecture has been overtaken; read this doc for the current shape:
- `SPEC_DATA_DIR_UNIFICATION_2026-05-05` — its design shipped as the `RuntimeMode` + `DataPaths` resolver (PR #695); see §2.
- `SPEC_SHARED_AGENT_REGISTRY_2026_05_12` — proposed **one** registry record per agent; the implementation **split** into two stores (`definitions/` + `registry/`); see §3a.
- `SPEC_AGENT_CONCEPT_CONSOLIDATION_2026_05_24` — superseded by the live tracker `SPEC_AGENT_ARCHITECTURE_2026_05_27`.
- `RESEARCH_PER_VERSION_DATA_ISOLATION_2026_05_24` — the research behind the channel model, now established (see §2); its `local-<branch>-<hash>` quick-win is the channel scheme in use.
- `docs/analysis/data-dir-status-2026-05-09` — a point-in-time status check, subsumed by §2.

---

## 1. The two things people call "agents"

| Concept | UI surface | What it is |
|---|---|---|
| **Definition** (template) | **"New from template"** | The reusable config: name, icon, provider, type, system prompt/content, skills, default flags. `is_seeded=1` = built-in template; `is_seeded=0` = user-created. |
| **Instance** (session) | **"My Agents"** | A *running/historical* agent: `instance_id`, `instance_name`, `definition_id`, `session_id`, **`working_directory`**, hidden flag. This is what resumes a conversation. |

The user's symptom is about **instances/sessions** ("My Agents"), not definitions.

---

## 2. On-disk layout (`~/.agentmux/`) and data scope

From `agentmux-common/src/data_paths.rs` + `SPEC_CROSS_CHANNEL §1.1`:

```
~/.agentmux/
├── channels/<channel>/
│   ├── versions/<version>/data/db/objects.db   PER (channel,version)  ← agents live HERE today
│   ├── versions/<version>/{cef-cache,runtime,ipc-port,lock}  PER (channel,version)
│   ├── config/settings.json                    CHANNEL-wide
│   └── agents/registry/                         CHANNEL-wide (exists, ~unused as source of truth)
├── dev/<branch>[/<sub>]/data/db/objects.db     PER dev-branch (the `task dev` layout)
├── agents/<name>/                               GLOBAL  ← every agent's WORKSPACE (cwd) lives here
└── shared/                                       GLOBAL (cross-channel)
    ├── providers/<provider>/…                   auth + transcripts (already global)
    └── agents/
        ├── definitions/<id>.json                global agent DEFINITIONS  (P0.2)
        └── registry/<id>.json                   global agent INSTANCES    (P0.1/P0.3)
```

**Scope matrix** (`SPEC §4.1`):

| Data | Scope | Location |
|---|---|---|
| Definition (template) | **global** (intended) | `shared/agents/definitions/<id>.json` (impl) |
| Instance identity + session/cwd link | **global** (intended) | `shared/agents/registry/<id>.json` |
| Conversation transcript | **global already** | `shared/providers/<provider>/projects/…` (`claude --resume` reads by cwd) |
| Provider auth | **global already** | `shared/providers/<provider>/` |
| **Agent workspace (cwd)** | **global** | **`~/.agentmux/agents/<name>/`** |
| Pane/window/layout, sagas, cef-cache, ipc, lock | **per (channel,version)** | `channels/<ch>/versions/<v>/…` |
| Channel settings | **channel-wide** | `channels/<ch>/config/` |

A **channel** (`SPEC_DATA_CHANNELS`) is a version-spanning data bucket: `stable` (releases), `local-<branch>` (local/portable builds), `dev/<branch>` (`task dev`). The per-`(channel,version)` SQLite `objects.db` is the legacy local store.

**Key fact #1:** the heavy data (transcripts, auth) is *already global*. **Key fact #2:** the agent **workspace** (`working_directory`) is *already global* — `~/.agentmux/agents/<name>`, not per-channel. Only the lightweight definition/instance *metadata* is trapped per-`(channel,version)` in `objects.db`.

---

## 3. The cross-channel design (intended)

Per `SPEC_CROSS_CHANNEL §2–4`: most of the machinery already existed (a cross-process file registry, a SQLite→registry consolidator, global transcripts). **The single decision** was to re-root the registry from per-channel to **global** (`~/.agentmux/shared/agents/`) and make it the authoritative read path, with a one-time backfill of existing per-`(channel,version)` rows.

Rollout phases (`SPEC §7`):
- **P0** — foundation: global stores + re-root + one-shot backfill migration + live write-mirror. *(shipped: #1383–#1390; the fix below is part of it.)*
- **P1** — registry-primary **dual-write** (write registry first, mirror to SQLite, tombstones on delete). *(not started)*
- **P2** — **cutover**: registry is the sole source of truth. *(not started)*
- **P3** — SQLite mirror becomes diagnostic-only. *(not started)*

### 3a. Read / write / backfill paths (P0, as built)
- **Definitions** — read `Store::agent_def_list` (`storage/agents.rs`) merges local SQLite (`db_agents`) with the global `definitions/` store; write-mirror `def_registry_mirror.rs`; one-shot backfill `registry/def_migrate.rs`.
- **Instances** — global registry `registry/store.rs`; live mirror `backend/storage/registry_mirror.rs`; one-shot backfill `registry/migrate.rs::migrate_from_sqlite_once` + `backfill_source_bases_once`.

**Architecture note (divergence from spec):** `SPEC §4.1` puts *both* definition and instance in **one** registry record (`shared/agents/registry/<uuid>.json`). The implementation **split** them into **two** stores — `definitions/` and `registry/` — each with its **own** migration, marker, schema, and live-mirror. That doubled the surface area and is why there are *two* independently-buggy backfills (below).

---

## 4. Current state — what works, what's broken

### ✅ Definitions ("New from template")
The read-merge is correct; the backfill was broken (only 1 of N captured) and is **fixed** in PR #1391 (schema-resilient read, scans `dev/`, recoverable versioned marker). On this machine it recovered the global definitions store 1 → 11. See the definitions deep-dive that lands with PR #1391.

### ❌ Instances ("My Agents") — empty
The instances backfill ran (`dbs_scanned=52, dbs_skipped=0, rows_seen=10`) but wrote **0** records: `records_skipped_unmappable=9`. Root cause is a **path-model mismatch**:

`registry/migrate.rs::row_to_record` (≈L536) makes the instance's `working_directory` *relative* by stripping it under the **source channel's** agents dir (`row.agents_root`), so a reader in another channel can "re-join under its own channel" (the P0.4 `source_agents_base` machinery):
```rust
let rel = abs.strip_prefix(&row.agents_root).ok()?;   // → None ⇒ "unmappable"
```
But the actual `working_directory` values are **global**:
```
Qooma     -> C:\Users\asafe\.agentmux/agents/qooma-0612g
Clamk     -> C:\Users\asafe\.agentmux/agents/clamk-0612a
Naki/CodexPo/GeminiOpp -> …\.agentmux/agents/<name>
```
These are under the **global** `~/.agentmux/agents/`, **not** under any channel's agents dir. So `strip_prefix` fails for every one → all unmappable → 0 instances in the global registry → **My Agents is empty in every channel.** (Mixed separators `…\.agentmux/agents/…` make the prefix match even more fragile.)

In other words: **the design re-introduces per-channel anchoring for a path that is already global.** It's solving a relocation problem that doesn't exist for global workspaces, and breaking on it.

---

## 5. Architecture improvements (prioritized)

### I1 — Align the instance workspace model: workspaces are GLOBAL, store the path as-is *(fixes "My Agents", removes a landmine)*
Agent workspaces live at `~/.agentmux/agents/<name>` (global, version/channel-independent). The instance record should store the **global-absolute** `working_directory` (or a path relative to the **global** `~/.agentmux/agents/` root) and use it directly — **no per-channel strip/re-join, no `source_agents_base`.**
- Concretely: `row_to_record` should anchor on the **global agents root** (`~/.agentmux/agents`), or simply keep the absolute path when it's already under `~/.agentmux/`. A row only becomes "unmappable" if its workspace genuinely can't be resolved — not because it's global.
- This deletes the P0.4 `source_agents_base` reconstruction complexity for the common (global-workspace) case and makes the backfill capture the user's sessions.
- Edge: container/remote agents whose cwd is *not* under `~/.agentmux/agents/` — keep an absolute fallback rather than dropping the row.

### I2 — Finish the cutover (P1→P2→P3) so one-shot scan-and-reconstruct dies
The fragility (two buggy one-shot migrations, marker poisoning, schema drift, path anchoring) is inherent to "SQLite is primary, backfill into the global store." Make the **global registry authoritative** (P1 dual-write → P2 cutover): new writes land in the registry first; SQLite becomes a mirror. Then "retain across channels+versions" is true *by construction*, and backfills shrink to a one-time, read-only import that never has to round-trip data back.

### I3 — Unify definitions + instances into one store/record (per the spec)
`SPEC §4.1` intends a single registry record carrying both. The current two-store split (`definitions/` + `registry/`) is the source of *two* parallel migrations, markers, and schemas — and *both* shipped broken. Consolidating to one store (or at least one shared scan/anchor/marker library) removes a whole class of "fixed one, missed the other" bugs (exactly what happened: #1391 fixed definitions; instances were still broken).

### I4 — Make migrations structurally safe (generalize the #1391 lessons)
Whatever backfills remain must: (a) read **schema-resiliently** (introspect columns / tolerate missing ones — never skip a whole DB on one column); (b) scan **every** root (`channels/*/versions/*` **and** `dev/*`) via one shared enumerator (`migrate.rs::enumerate_sources`); (c) use a **versioned, recoverable** marker (re-run once when logic changes; never finalize a poisoned partial pass). #1391 did this for definitions; apply the same to instances.

### I5 — Observability
Log, at startup, which store served My Agents and the counts (`global=N local=M`), and have each migration log `rows_seen / written / skipped(reason)`. The instances migration already logs `records_skipped_unmappable` — surfacing it (and *why*) would have caught this immediately, the same lesson as the bashwrap-stale-bundle retro.

---

## 6. Recommended sequence
1. **I1** — fix `row_to_record` anchoring (global workspaces) + a recoverable versioned marker on the instance migration, mirroring #1391. Re-run → "My Agents" repopulates from the 10 rows. *(small, high-impact; the user's actual ask)*
2. **I4** — factor a shared schema-resilient enumerator/anchor/marker used by both migrations.
3. **I3** — converge definitions + instances onto one store/record.
4. **I2** — P1/P2 cutover so the global registry is authoritative and backfills become a one-time import.

---

## 7. Key files & specs
- `agentmux-common/src/data_paths.rs` — channel/version/shared path resolution.
- `agentmux-srv/src/registry/paths.rs` — `resolve_global_shared_root` / `resolve_shared_{registry,definitions}_dir`.
- `agentmux-srv/src/registry/store.rs` — the file-per-agent global registry (instances).
- `agentmux-srv/src/registry/migrate.rs` — instance backfill (`migrate_from_sqlite_once`, `row_to_record` ≈L536, `backfill_source_bases_once`) — **the I1 target**.
- `agentmux-srv/src/registry/def_migrate.rs` — definition backfill (**fixed in #1391**).
- `agentmux-srv/src/backend/storage/agents.rs` — `agent_def_list` definition read-merge.
- `agentmux-srv/src/backend/storage/registry_mirror.rs` / `def_registry_mirror.rs` — live mirrors.
- Specs (current — see §0 for the full annotated companion list): `SPEC_CROSS_CHANNEL_AGENT_PERSISTENCE_2026-06-13.md`, `SPEC_DATA_CHANNELS_2026_05_24.md`, `SPEC_AGENT_ARCHITECTURE_2026_05_27.md`, `SPEC_UNIFIED_AGENT_HISTORY_STORE_2026-06-10.md`.
- Analysis: the definitions deep-dive lands with PR #1391 (`docs/analysis/ANALYSIS_CROSS_CHANNEL_AGENT_RETENTION_2026_06_13.md` once that PR merges).

*Written 2026-06-13 by AgentX. Architecture overview + improvement proposals — definitions backfill fixed in #1391; instance backfill (I1) not yet implemented.*
