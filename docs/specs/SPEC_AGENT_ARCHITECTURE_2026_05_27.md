# SPEC: Agent data-model architecture — consolidation plan & status

**Date:** 2026-05-27
**Author:** AgentA
**Status:** active — tracking spec (restamped to the closed enum 2026-09-07; Phase 3b resumed, see the note below). **2026-09-08: the resumed 5-PR database-layer migration (below) is complete** — both legacy tables are dropped as of PR 5. Left `active`, not `implemented`, because this doc's own acceptance criteria describe a broader scope (JSON registry sunset, orphan working-dir cleanup, frontend type rename) that the 5-PR plan never covered and nobody has since picked up — see the acceptance-criteria section for exactly which boxes are and aren't checked. Supersedes `SPEC_AGENT_CONCEPT_CONSOLIDATION_2026_05_24.md` for ongoing planning. The 2026-05-24 spec laid out the design; this one is the live status doc with the per-handler matrix and the migration plan.
**Tracking discussion:** [#1095 — Architecture: agent data-model consolidation — tracking](https://github.com/agentmuxai/agentmux/discussions/1095). All PRs that touch agent data-layer code link there.

> **Staleness note (2026-08-03):** this doc has not been edited since 2026-05-28 (`6584c024`). Phase 3b is only partially shipped and Phase 3c has not happened — `db_agent_definitions`/`db_agent_instances` still exist in the current schema, contradicting this doc's own acceptance criteria. Confirmed stalled, not just unread — see `docs/specs/SPEC_MIGRATION_SYSTEM_HARDENING_2026_08_03.md` §1.4 and `SPEC_DOCS_LIFECYCLE_HARDENING_2026_08_03.md` for the audit that found this. Treat the phase table below as historical intent, not current status, until someone either finishes the migration or formally re-scopes it here.

> **Resumed 2026-09-07** (repo-owner decision: finish, not park — `SPEC_MIGRATION_SYSTEM_HARDENING_2026_08_03.md` Phase 4 option (a)). Inventory of every remaining legacy read/write, FK and caller is in the PR description of the first PR below. The plan grew from four PRs to five once PR 4's own scope turned out too large for one review — the definition flip forced removing the per-boot gap-repair pass in the same PR that stopped writing `db_agent_definitions` (leaving it running would have resurrected every deletion), which is more than "flip reads" alone; the physical `DROP TABLE` is cleaner split into its own PR 5 rather than riding along:
>
> 1. **Launch state on `db_agents`** (schema v29: `session_id`, `status`, `started_at`, `ended_at`; `m0025` backfill; dual-write keeps them current) — the four columns whose absence kept `db_agent_instances` alive (recent-sessions picker, the pane close/reopen continuity write-back, the status-filtered instance list). Shipped 2026-09-07.
> 2. **Instance flip** — every `Store` instance method reads and writes `db_agents` only; the instance-side dual-write helpers and the dead `instance_get_by_name`/`user_clone_defs_for_template` are gone; `m0026` re-keys the named-agent registry from launch ids to agent ids, and `m0027` backfills each row's `working_directory` from its latest legacy launch (Phase 3a stored the definition's configured cwd; the launch held the resolved one, which is what a continuation must reopen). Shipped 2026-09-07. Three behaviour changes it forces, each pinned by a test:
>    - `instance_create` returns the row the launch landed on, whose id can differ from the requested one (a launch of a user agent, or a continuation, folds into that agent's row; only a fresh template launch makes a new one). Callers persist the returned id — the frontend already stores it as the block's `agentInstanceId`.
>    - A launch's **resolved** working directory becomes the agent's workspace (Phase 3a wrote the definition's configured cwd, which made the cross-version registry mirror skip the row).
>    - Reseeding templates no longer deletes agents launched from them, and deleting a template no longer deletes its agents: such a row is the user's agent with its own conversation, exactly as a user clone always was.
>
>    Two things it deliberately leaves for later, both because `db_agent_identity_links` still has its FK on `db_agent_definitions`: an agent that exists only as a `db_agents` row cannot hold account links of its own (`listrecentsessions` falls back to its template's), and `agent_def_delete` still owns definition rows while `instance_delete` owns agent rows.
> 3. **FK re-point** (schema v30, `m0028`): the six tables whose `agent_id` targeted `db_agent_definitions(id)` — `db_agent_content`, `_skills`, `_history`, `_identity_links`, `_skills_ref`, `_mcp_ref` — now target `db_agents(id)` instead. Every existing row already satisfies the new target (a definition's id always has a `db_agents` mirror); what it unlocks is a `db_agents` row with no definition counterpart (a template launch) being able to hold any of these directly, which is the prerequisite for retiring the identity-link template fallback above. Shipped 2026-09-07. **Scoped narrower than originally planned**: does not flip `db_agent_definitions` reads/writes and does not remove the fallback yet — nothing writes an identity link against a launch id today (only the Agent Setup modal writes links, and only for templates/user agents), so removing the fallback now, with no writer ever targeting a launch's own row, would be a regression, not a cleanup. That follower work — giving a launch its own identity override, and then dropping the fallback — is pushed into PR 4 rather than invented as scope here.
> 4. **Definition flip** (Phase 3d): every `agent_def_*` method reads and writes `db_agents` only — `dual_write.rs` (definition-side helpers) is gone, and `agent_def_get` now reads the same unfiltered `db_agents` row `agent_def_list` already did, closing a real inconsistency (an agent visible in "My Agents" could 404 opening its detail view if it existed only as a `db_agents` row). `agent_def_delete`/`instance_delete` are now the same operation reached from two names — neither touches `db_agent_definitions`, and the FK cascade through the six Phase-3c-repointed child tables handles the rest. This forced removing the per-boot gap-repair pass (`Store::repair_agent_def_gaps`, `agents_consolidate::repair_def_gaps`, and its `bootstrap.rs` call site) in the SAME PR, not later: once neither delete path wrote `db_agent_definitions`, a repair pass still reading it would have resurrected every deletion on the next boot, the same class of bug PR 2's round-2 review caught. `agents_consolidate::run_consolidate_migration`/`consolidate_looks_incomplete` (backing frozen `m0007`/`m0000`) are untouched — they run at migration time, before this PR's live code ever executes, so they aren't affected by `db_agent_definitions` going unwritten. `m0025`'s frozen copy of `repair_def_gaps` (a separate, already-frozen snapshot, per the "freeze copies of live logic" doctrine) needed its own inline frozen copy once the live function it had been calling was deleted.
> 5. **Drop** (schema v32, `m0029`, Phase 3e — final): `db_agent_definitions` and `db_agent_instances` are physically dropped from every channel store. `run_object_schema` no longer declares either table — that had to ship in the SAME release as the drop migration, since `CREATE TABLE IF NOT EXISTS` would otherwise silently recreate what the migration just removed on the very next `Store::open()`. `agents_consolidate::consolidate_looks_incomplete`/`run_consolidate_migration` (backing frozen `m0007`) are made tolerant of either table being physically absent — genuinely load-bearing, not defensive-only: a fresh (v32+) channel is stamped `0007` applied via its own pre-existing early-return no-op, and a later `muxspect migrations --verify` doctor pass calls `m0007`'s `verify()` — which reads these — against that channel once it has ordinary `db_agents` data; without the tolerance fix that doctor pass would error on "no such table" for every fresh install. `agents_consolidate.rs` itself, `0007`, and its marker are **not** retired in this PR — the module and the migration both stay, now tolerant, since removing them is a "minimum supported upgrade path" policy call the repo owner hasn't made, not a mechanical follow-on to the drop. `m0004`/`m0006`/`m0009_transcript_backfill` needed no changes: none of the three ever read `db_agent_definitions`/`db_agent_instances` directly. This doc's phase table is updated in each PR — this is the last one; consolidation is complete as of this PR.

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

**2026-09-08 status: the DATABASE-layer criteria (the ones the 5-PR series above actually scoped) are done. The other three were always part of this doc's original, broader vision but were never part of that 5-PR plan and remain open, unscoped follow-up work — nobody has picked them up.** Don't read the checked boxes below as "everything this doc ever described is finished."

- [x] `db_agent_definitions` and `db_agent_instances` tables do not exist. Shipped 2026-09-08 (schema v32, `m0029_drop_legacy_agent_tables`).
- [ ] `~/.agentmux/agents/registry/` does not exist after a fresh install or after migration on an existing install. **Not addressed by the 5-PR series** — the named-agent JSON registry (`agentmux-srv/src/registry/`) is still live and actively relied on (PR 2/3 kept it current, `m0026` re-keyed it from launch ids to agent ids); "sunsetting" it (Phase R below) was never in scope for any of the five PRs.
- [ ] No orphan working dirs under `~/.agentmux/agents/` after the reconciliation migration runs. **Not addressed** (Phase O below) — no reconciliation migration exists.
- [x] `db_agents` has one row per logical agent; "My Agents" picker shows one entry per row; no continuation duplicates. Shipped 2026-09-07 (PR 2, instance flip — continuation chains are pre-collapsed).
- [x] Every agent-related RPC handler reads from `db_agents`. Shipped across PR 2 (instance side, 2026-09-07) and PR 4 (definition side, 2026-09-08) — nothing left reading either legacy table (confirmed by grep across `agentmux-srv/src` outside `migrations/`; `agents_consolidate.rs` is the sole intentional exception, kept for the frozen `m0007` migration's own use).
- [ ] The TS `AgentDefinition`/`AgentInstance` types are renamed to `Agent` (or removed if duplicated). **Not addressed** — this is a frontend/wire-type change the backend-focused 5-PR series never touched; the RPC wire shapes kept their existing field names throughout (see e.g. PR 2's `parent_template_id` → `parent_id` wire-compat note).

---

## References

- `docs/specs/SPEC_AGENT_CONCEPT_CONSOLIDATION_2026_05_24.md` — original design spec.
- `docs/specs/SPEC_AGENT_PICKER_TWO_TIER_2026_05_24.md` — Phase 1 (shipped).
- `agentmux-srv/src/backend/storage/agents_consolidate.rs` — Phase 3a backfill code.
- `agentmux-srv/src/backend/storage/store.rs` — all SQL read/write sites.
- `agentmux-srv/src/registry/` — JSON registry module (sunset target).
- Discussion #1095 — long-term tracking thread; link every related PR there.
