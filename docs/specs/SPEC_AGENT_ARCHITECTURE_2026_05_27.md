# SPEC: Agent data-model architecture — consolidation plan & status

**Date:** 2026-05-27
**Author:** AgentA
**Status:** Tracking spec — supersedes `SPEC_AGENT_CONCEPT_CONSOLIDATION_2026_05_24.md` for ongoing planning. The 2026-05-24 spec laid out the design; this one is the live status doc with the per-handler matrix and the migration plan.
**Tracking discussion:** [#1095 — Architecture: agent data-model consolidation — tracking](https://github.com/agentmuxai/agentmux/discussions/1095). All PRs that touch agent data-layer code link there.

> **Staleness note (2026-08-03):** this doc has not been edited since 2026-05-28 (`6584c024`). Phase 3b is only partially shipped and Phase 3c has not happened — `db_agent_definitions`/`db_agent_instances` still exist in the current schema, contradicting this doc's own acceptance criteria. Confirmed stalled, not just unread — see `docs/specs/SPEC_MIGRATION_SYSTEM_HARDENING_2026_08_03.md` §1.4 and `SPEC_DOCS_LIFECYCLE_HARDENING_2026_08_03.md` for the audit that found this. Treat the phase table below as historical intent, not current status, until someone either finishes the migration or formally re-scopes it here.

---

## Why this spec exists

The 2026-05-24 spec laid out the vision (one `db_agents` table, retire instance + definition). Since then, Phase 1 (two-tier picker) and Phase 3a (write-only backfill) have shipped, but the rest of the migration has no live tracking artifact. As of today we have:

- **5 layers of agent storage** — three SQL tables, one JSON registry, one filesystem layout — each authoritative for some subset of agent state.
- **27 mutation sites** across those layers (Rust + dual-write helpers).
- **~14 RPC handlers** that surface agent data to the frontend, only **2 of which** read from the new consolidated table.
- The user-visible **"4 Claudes" bug** in "My Agents" (continuations rendered as separate entries) is a direct consequence of the unfinished read-side migration.

The picker spec said "do this in a focused refactor cycle, not as a side-quest." This is that focused cycle's planning doc.

---

## The 3-concept end state (recap)

From `SPEC_AGENT_CONCEPT_CONSOLIDATION_2026_05_24.md` §"The lean model":

1. **`db_agents`** — one table, two flavors via `is_template` flag.
   - Templates: `is_template = 1` (seeded by manifest; the "Claude Code template", etc.)
   - User agents: `is_template = 0`, `parent_template_id` points back to the template they were cloned from. Carry the user-given bindings (name, identity_id, memory_id, working_directory, github_context).
2. **`db_block`** — UI panes referencing agents via `meta.agentId`. Multiple blocks can show the same agent (cross-tab). Already in this shape; no change.
3. **`filestore.db` zone `agent:<agentId>:current`** + `:archive:*` — Option E session content. Already in this shape; no change.

`db_agent_definitions`, `db_agent_instances`, the JSON registry, and the orphan working dirs all retire.

---

## Today's reality: 5 layers of storage

| # | Layer | Path / Table | Created by | Status | Authoritative for |
|---|---|---|---|---|---|
| 1 | **`db_agents`** | `objects.db` (SQLite) | Phase 3a backfill + dual-write | New (live, mostly empty of reads) | Nothing yet — readers haven't flipped |
| 2 | **`db_agent_definitions`** | `objects.db` | Legacy | Active (live reads + writes) | Templates + user-clones; provider/cmd config |
| 3 | **`db_agent_instances`** | `objects.db` | Legacy | Active (live reads + writes) | Per-launch instance rows; bindings; status; continuation chain |
| 4 | **JSON registry** | `~/.agentmux/agents/registry/*.json` | May-13 SQLite → JSON migration (one-shot) | Live writes; reads are **dead code** | Holds named-agent JSON files for instances not yet retired. Read code exists but is `#[allow(dead_code)]` — fallback never triggers. |
| 5 | **Working dirs** | `~/.agentmux/agents/<slug>/` | Created per agent launch | Active (FS-level) | Per-agent cwd for the CLI. Includes a `.claude/` config dir + `.mcp.json`. Orphans accumulate when corresponding SQL/registry rows are removed but the dir isn't. |

### Why this is a mess in practice

- **(1) and (2/3) disagree.** A user can have 4 rows in `db_agent_instances` for one logical agent (the "4 Claudes" continuation chain) while `db_agents` has 1 consolidated row. Different views, neither authoritative for the UI yet.
- **(4) has stale data.** Maks / DSad / Masa exist as JSON registry files but have **no** corresponding `db_agent_instances` rows. They're invisible to "My Agents" (which reads SQL) and to anyone else (registry reads are dead code). They're disk landfill.
- **(5) has orphans.** 12+ working dirs at `~/.agentmux/agents/` with no row anywhere — leftover from `agent_zones_v1` migration and prior delete operations that cleaned SQL but not the FS.

---

## Per-handler read-site matrix (Phase 3b target)

The inventory below lists every RPC handler that reads from the legacy tables. Each one must be flipped to read from `db_agents` (or have its read removed) before `db_agent_definitions` and `db_agent_instances` can be dropped.

### Already on `db_agents` ✅

| Function | File | RPC handler |
|---|---|---|
| `agent_def_list` | `wstore.rs:698` | `listagents` |
| `agent_def_count` | `wstore.rs:725` | (startup seed check) |
| `agent_def_set_hidden` precondition | `wstore.rs:882` | `agentdefhide` / `agentdefunhide` |
| `agent_def_insert` slug collision | `wstore.rs:793` | `createagent` |

### Still on legacy ⏳ (must flip in Phase 3b)

| Function | File | RPC handler | Returns |
|---|---|---|---|
| `user_clone_defs_for_template` | `wstore.rs:649` | (internal: `template_promote`) | User-clones by parent template |
| `agent_def_get` | `wstore.rs:680` | (internal: many handlers) | Single definition |
| `agent_def_delete_seeded` (capture phase) | `wstore.rs:747` | `reseedagents` | Cascaded instance IDs |
| `instance_list` | `wstore.rs:1837` | `listagentinstances` | All instances by def/status |
| `instance_get` | `wstore.rs:1884` | `getagentinstance` | Single instance |
| `instance_list_named` | `wstore.rs:2025` | `listrecentsessions` | Named instances ("My Agents" / Continue dropdown) |
| `instance_get_by_name` | `wstore.rs:2075` | (launch modal collision detect) | Latest named instance by name |
| `instance_get_active_for_block` | `wstore.rs:2322` | (credential resolver) | Most-recent active instance for block |

**Total: 8 read paths to migrate.** The current "4 Claudes" bug is in `instance_list_named` — it returns continuations unfiltered, where the consolidated `db_agents` view would dedup at the source.

---

## Mutation site matrix (for the 3b migration; freeze this list)

27 write sites across 4 layers. The dual-write helpers (Phase 3a) mirror each legacy mutation into `db_agents`. Phase 3b doesn't change writes; Phase 3c drops the legacy writes once readers have soaked.

### `db_agent_definitions` (5 sites)
| Op | File:line | RPC |
|---|---|---|
| INSERT | `wstore.rs:806` | `createagent` |
| UPDATE (all fields) | `wstore.rs:939` | `updateagent` |
| UPDATE (`user_hidden` only) | `wstore.rs:901` | `agentdefhide` / `agentdefunhide` |
| DELETE | `wstore.rs:992` | `deleteagent` |
| DELETE (bulk seeded) | `wstore.rs:755` | `reseedagents` |

### `db_agent_instances` (6 sites)
| Op | File:line | RPC |
|---|---|---|
| INSERT | `wstore.rs:1906` | `createagentinstance` |
| UPDATE (state fields) | `wstore.rs:2109` | `updateagentinstance` |
| UPDATE (`display_hidden`) | `wstore.rs:1952` | `hidenamedagent` |
| UPDATE (`definition_id`) | `wstore.rs:2161` | (migration escape) |
| UPDATE (`identity_id` backfill) | `wstore.rs:2218` | (startup migration) |
| DELETE | `wstore.rs:2177` | `deleteagentinstance` |

### `db_agents` dual-write mirrors (9 sites)
All in `wstore.rs` (2372–2881). Each fires after the corresponding legacy write; failures log + continue (legacy stays authoritative until Phase 3b).

### JSON registry (7 sites)
| Op | When | RPC |
|---|---|---|
| UPSERT (active/) | Instance create/update if named + not continuation | `createagentinstance`, `updateagentinstance` |
| RETIRE (active/ → retired/) | `display_hidden=1` | `hidenamedagent` |
| UNRETIRE (retired/ → active/) | `display_hidden=0` | `hidenamedagent` |
| HARD DELETE | Instance / def cascade delete | `deleteagentinstance`, `deleteagent` |
| BACKFILL | Startup, marker-gated | (one-shot) |

---

## Phase plan

### Phase 3b — flip readers (next focus)

**Goal:** all 8 legacy read paths replaced by `db_agents` reads. Old tables still written (dual-write); only writes remain on them.

Suggested PR carving (one per row, ~30-80 LOC + tests each):

| Sub-PR | Read path migrated | Risk | Status | Notes |
|---|---|---|---|---|
| 3b.1a | `instance_list_named` picker dedup via CTE on legacy table | Low | ✅ shipped PR #1096 | **Fixes the "4 Claudes" bug.** Reads still come from `db_agent_instances` — see 3b.1b. |
| 3b.1b | `instance_list_named` → `db_agents` (true flip) | Medium | ⏳ pending | Splitting from 3b.1a: the dedup-via-CTE shipped first to fix the user-visible bug; the actual read flip is deferred until per-launch fields (block_id/session_id/status/started_at/ended_at) have a defined story for callers like `listrecentsessions`. |
| 3b.2 | `instance_get_by_name` → `db_agents WHERE is_template=0` | Low | ✅ shipped PR #1110 | One-row lookup. Function had **zero callers** in the live tree, so blast radius nil. COALESCE on `parent_template_id` resolves the folded user-clone case. Bundled the continuation-mirroring dual-write fix + backfill `updated_at` fix. |
| 3b.3a | `instance_list` (no `status` filter case) → `db_agents` | Low–Medium | ✅ shipped (this PR) | Frontend caller (`refreshInstances` in swarm-model.ts) passes `{}` and `instancesAtom` has zero readers in the live tree, so the user-facing surface is empty. `definition_id` filter matches `id = ?` only (the agent's own id) — the legacy "filter by template id" semantics are dropped in the consolidated model since user-clones and template-instances share `parent_template_id` and can't be distinguished without schema changes; no live caller exercises that path. Continuation chains pre-collapse to one row per logical agent. |
| 3b.3b | `instance_list` status-filter case → `db_agents` AFTER `updateagentinstance` refactor | Medium | ⏳ pending | Status is a transient runtime field (no analog on the consolidated row). The status-filter branch currently falls back to a private `instance_list_legacy` helper. Retire it once the `updateagentinstance` "fetch + merge transient fields" pattern is rewritten to take a partial-update API that doesn't need them. |
| 3b.3c | `instance_get` → `db_agents` AFTER `updateagentinstance` refactor | Medium | ⏳ pending | Same blocker as 3b.3b: the handler reads `instance_get` to fetch transient fields (`block_id`, `session_id`, `status`, `started_at`, `ended_at`) before merging. A naive flip would clobber those with empty defaults on write-back. |
| 3b.4 | `instance_get_active_for_block` → block.meta.agentId → `db_agents` | Medium | ✅ shipped (this PR) | Follow `block.meta.agentId` (or legacy `agent:id`) directly. The `status IN ('running', 'paused')` filter that prevented stale-creds bleed across pane-reopens isn't needed anymore — `db_agents` has one row per logical agent and the continuation-mirror dual-write keeps bindings fresh. Resolver tests updated to insert a Block alongside the existing instance fixture. |
| 3b.5 | `agent_def_get`, `user_clone_defs_for_template`, `agent_def_delete_seeded` | Low | ⏳ pending | Internal helpers — straightforward. |

**Correction (2026-05-28):** the original 3b.1 row above conflated two pieces of work — the user-visible dedup fix (CTE on the legacy table) and the actual read flip to `db_agents`. PR #1096 shipped the first; the second became 3b.1b. The "true flip" rows (3b.1b, 3b.2, 3b.3, 3b.4) are the only ones that change which table the SQL `FROM` clause names.

Each sub-PR: read swap + targeted handler test that asserts the new behavior + smoke test against the existing handler's existing tests (no regressions). Use feature flag `AGENTMUX_PHASE_3B=1` if any sub-PR needs to dark-launch.

**Acceptance criteria for Phase 3b complete:**
- [ ] All 8 legacy read paths above use `db_agents`.
- [ ] `listrecentsessions` no longer shows duplicate continuations.
- [ ] Existing integration tests for each handler still pass without modification (except where they assert legacy-table specifics — those get rewritten).
- [ ] Dual-write error rate from `wstore.rs:2372+` is zero across a 7-day soak.

### Phase 3c — drop legacy tables

**Preconditions:** Phase 3b complete + 7-day soak with zero dual-write errors.

**Steps:**
1. Stop writing to `db_agent_definitions` / `db_agent_instances` (delete the 11 mutation sites).
2. Drop the tables via a one-shot migration (marker-gated).
3. Delete the dual-write helpers (`agents_dual_write_*`).
4. `db_agents` becomes the sole agent table.

**Acceptance:** schema migration deletes both tables; all references in code removed; tests green.

### Phase R — registry sunset

**Goal:** retire the JSON registry at `~/.agentmux/agents/registry/`.

**Sub-steps:**
1. **R.1 — Stop writing.** Once Phase 3b ships, named-agent metadata lives in `db_agents`. Remove the 4 registry write sites (`wstore.rs:2247+`).
2. **R.2 — Reconcile + delete.** One-shot reconciliation migration: for each `*.json` file in `registry/active/`, ensure a matching `db_agents` row exists (with `is_template=0`). If missing, INSERT one. Then `rm -rf` the registry directory.
3. **R.3 — Delete the registry module.** `agentmux-srv/src/registry/` removed entirely.

**Acceptance:** registry directory absent on fresh installs and after migration; no Rust references to `registry::` remain.

### Phase O — orphan working-dir cleanup

**Goal:** delete working directories at `~/.agentmux/agents/<slug>/` that no longer have a matching agent row.

**Sub-steps:**
1. **O.1 — Reconciliation migration.** Startup task (marker-gated): list all subdirs of `~/.agentmux/agents/` (excluding `registry/`); for each, check if any `db_agents` row has matching `working_directory`. If not, move to `~/.agentmux/agents/.trash/<timestamp>/<slug>/`. User can manually purge `.trash/`.
2. **O.2 — UX surface.** Future: a Settings → Storage panel showing trash size + a "Empty trash" button.

**Acceptance:** no orphans on a freshly-migrated install; existing orphans moved to `.trash/` with a one-line console.info hint pointing the user at the trash location.

---

## Authoritative-for-what (today vs end state)

| Concern | Today | End state |
|---|---|---|
| List of agents in "My Agents" picker | `db_agent_instances` (deduped at frontend? No — surfaces continuations) | `db_agents WHERE is_template=0 AND user_hidden=0` |
| List of templates in picker | `db_agent_definitions WHERE is_seeded=1` | `db_agents WHERE is_template=1 AND user_hidden=0` |
| Provider/cmd config | `db_agent_definitions` | `db_agents` (template + clone both carry it) |
| User-given name, identity, memory, cwd | `db_agent_instances` | `db_agents` (only on `is_template=0` rows) |
| Continuation history (Maks-from-Claude) | `db_agent_instances.parent_instance_id` chain | Single `db_agents` row; lifecycle events go in optional `db_agent_events` (deferred) |
| Per-launch status (running/paused/ended) | `db_agent_instances.status` | Live runtime state (no SQL persistence needed for transient status) |
| Block → agent reference | `db_block.meta.agentId` | Unchanged |
| Conversation history | `filestore.db` zone `agent:<defId>:current` | `filestore.db` zone `agent:<agentId>:current` — same shape, key changes from def_id to agent_id when 3c lands |

---

## Open questions

- **`db_agent_events` audit log.** The 2026-05-24 spec called it "probably unnecessary for now." Revisit when Phase 3b lands — if any retired handler used `started_at` / `ended_at` non-trivially, may need to capture that.
- **Naming during the transition.** Code currently uses `AgentDefinition` and `AgentInstance` types in both Rust and TypeScript. Phase 3c rename: collapse to `Agent` (Rust struct, TS interface). Track the rename as a follow-up PR after 3c.
- **Templates with non-default cmd_args.** Some seeded templates carry default args; user-clones may override. Confirm `db_agents` schema allows the override (it does — `cmd_args` is per-row).
- **`is_template` mutability.** Per the 2026-05-24 spec: probably no; one-way clone. Keep enforcing at the handler layer.
- **Cross-tab agent sharing.** Multiple blocks for one agent share the session zone. Already in this shape with `agentId`-keyed zones. No work needed.

---

## Acceptance criteria for "consolidation complete"

When all of the following hold:

- [ ] `db_agent_definitions` and `db_agent_instances` tables do not exist (or exist only as a one-tick compat shim about to be dropped).
- [ ] `~/.agentmux/agents/registry/` does not exist after a fresh install or after migration on an existing install.
- [ ] No orphan working dirs under `~/.agentmux/agents/` after the reconciliation migration runs.
- [ ] `db_agents` has one row per logical agent; "My Agents" picker shows one entry per row; no continuation duplicates.
- [ ] All 14 agent-related RPC handlers read from `db_agents`.
- [ ] The TS `AgentDefinition` and `AgentInstance` types are renamed to `Agent` (or removed if duplicated).

---

## References

- `docs/specs/SPEC_AGENT_CONCEPT_CONSOLIDATION_2026_05_24.md` — original design spec.
- `docs/specs/SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md` — Phase 1 (shipped).
- `agentmux-srv/src/backend/storage/agents_consolidate.rs` — Phase 3a backfill code.
- `agentmux-srv/src/backend/storage/store.rs` — all SQL read/write sites.
- `agentmux-srv/src/registry/` — JSON registry module (sunset target).
- Discussion #1095 — long-term tracking thread; link every related PR there.
