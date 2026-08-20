# Architecture: Armory/Stash Foundation Consolidation (North Star)

**Date:** 2026-08-19
**Status:** proposal — vision/north-star document. Not implemented, not
meant to land as one PR. Intended to be worked incrementally via
follow-up `SPEC_` docs, each scoped to one theme below, sequenced per §4.
**Author:** Agent1 (agent1-06309)
**Related:**
`docs/specs/SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md`
(the memory-versioning spec whose research surfaced most of the findings
below — see §6 for how the two relate),
`docs/specs/ARCHITECTURE_ARMORY_2026_07_20.md`,
`docs/specs/ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md`,
`docs/specs/SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md` (in-flight —
overlaps with §3.2, coordinate before implementing either),
`docs/specs/SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md` (PR #1918,
the Preset→Bundle rename this doc extends to completion).

---

## 0. Summary

Armory/ABF has been actively reshaped across at least six specs since
2026-07-01 (Preset→Bundle rename, Phase 4/5 consolidation, mandatory-ABF
rethink, v0.2 provider-aware components, bundle-as-container v2), and each
pass has left real, working leftovers behind rather than fully completing
the prior rename or pattern. None of this is broken — every piece
individually functions — but the accumulation is now large enough that
"where does X live and what's it called" has a different answer depending
on which era of the codebase you're reading. This doc catalogs the
leftovers found during this session's research and proposes a target
architecture, assuming engineering cost is not the constraint — the
operator's own framing for this exercise. §4 gives a realistic, cost-aware
sequencing for actually getting there.

---

## 1. Motivation: why now

Per the operator: "we are designing as we are building" — Armory/ABF is
still under active, fast-moving construction (`SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md`
is dated two days before this doc). Foundation cleanup is cheap right now,
while comparatively little UI and agent behavior depends on the current
shape, and expensive later, once more does. The alternative — continuing
to layer new features (like `SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md`'s
own versioning proposal) on top of the current fragmentation — compounds
the problem: that spec had to explicitly design *around* the three-way
`memory_id`/`is_global`/`startup_bundle_id` split (§2 below) rather than
resolve it, specifically because resolving it was out of scope for a
feature spec. This doc is that resolution, offered as its own thing.

---

## 2. Current state inventory (verified against code, 2026-08-19)

### 2.1 The "memory" naming overload

The word "memory" currently names three unrelated concepts in the same
codebase:
- **ABF bundles.** Table `db_bundles` (renamed from `db_memory_bundles`,
  `SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md` Phase 4a) — but
  `Store` methods never followed: `bundle_memory_get`, `bundle_memory_list`,
  `bundle_memory_upsert` are still the live method names
  (`ARCHITECTURE_ARMORY_2026_07_20.md` §2, confirmed in
  `agentmux-srv/src/backend/storage/`). The Armory "Bundles" tab's pane
  registration also still uses `view: "memory"` internally
  (`CLAUDE.md`'s "Not widgets" table: "the `viewType` string stays
  `'memory'` as a persisted key").
- **Native memory** — `db_agent_native_memory`, `MemoryList`/`MemoryRead`/
  `MemoryWrite` MCP tools, `agent:memory:*` RPCs. The actual "brain" files.
- **Deprecated `preset.*` aliases** — `bundle.list/get/upsert/delete` are
  the current App-API surface; `preset.*` aliases still exist for backward
  compatibility with the pre-rename name, per
  `ARCHITECTURE_ARMORY_2026_07_20.md` §2.

A reader encountering `bundle_memory_get`, `view: "memory"`, or
`PresetGet` today has no way to know from the name alone whether it
touches bundles or native memory — the collision is not hypothetical, it
already caused this doc's sibling spec to spend a paragraph (§4.3 of
`SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md`) warning
future implementers not to name a *new* native-memory UI surface `"memory"`
because that string is already taken, confusingly, by bundles.

### 2.2 Three unreconciled Agent↔Bundle bindings

Documented in full in `SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md`
§2.5 — summarized here:

| Mechanism | Storage shape | Purpose | Auto-materialized? |
|---|---|---|---|
| `memory_id` | column on `db_agents` | owned ABF identity | No — pull-only |
| `is_global=1` | flag on `db_bundles` | workspace broadcast | Yes — no opt-out |
| `startup_bundle_id` | blob in generic `db_agent_content` | Session Context instructions | Yes — this one only |

Three different storage shapes (column / flag / generic-blob-table key)
for what is conceptually the same question — "which bundle(s) apply to
this agent, and how." Compare to Skills and MCP Servers, which both use
one consistent, correct shape: `is_global` for blanket availability plus
a real `db_agent_{skills,mcp}_ref` join table for explicit per-agent
binding (`ARCHITECTURE_ARMORY_2026_07_20.md` §3, §4). Bundles is the
outlier — the one primitive of the four core Armory entities (Bundles,
Skills, MCP Servers, and Startup) that never got the mature pattern.

### 2.3 `db_agent_content` — an unstructured, growing catch-all

Generic `content_type` (string) → blob key/value table, currently holding
at least `soul`, `agentmd`, `mcp`, `env`, `startup`, and (as of
`AgentStartupModal.tsx`) `startup_bundle_id` — six distinct concepts
sharing one schema-less table, discoverable only by grepping for string
literals, not by reading a schema. `ARCHITECTURE_ARMORY_2026_07_20.md` §5
documents that the one frontend surface generic enough to edit *any* of
these blobs (`frontend/app/view/agent-def/`) is dead code — "not
registered in `block-registry.ts`, no barrel/index file, zero external
imports" — so several of these six concepts have **no authoring UI at
all** today, reachable only via raw RPC calls or the seed manifest.

### 2.4 Two parallel RPC surfaces for the same bundle data

`ARCHITECTURE_ARMORY_2026_07_20.md` §2: UI-facing
(`agentmux-srv/src/server/agent_handlers/memory.rs`) exposes
`listmemories`/`getmemory`/`upsertmemory`/`deletememory`/`reorderglobalbrain`
— bare-word command names, no namespace. App-API
(`agentmux-srv/src/server/app_api/bundle.rs`) exposes
`bundle.list`/`get`/`upsert`/`delete`/`self.get` — dot-namespaced,
matching Skills' and MCP Servers' convention. Both surfaces bottom out in
the identical `Store::bundle_memory_*` methods and the identical
`memories:changed` event. Two names for every operation, no functional
difference, extra surface area to keep in sync.

### 2.5 No versioning/audit history anywhere

Covered exhaustively in `SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md`
§2.8: `db_bundles`, `db_agent_native_memory`, `db_accounts`/secrets, and
skills/MCP config are all plain overwrite-on-write. The only sequential,
dated record of change anywhere in the repo is the schema migration
chain (`m0001`...`m0022`) — schema-scoped, not data-scoped.

### 2.6 Skills' dual storage layers

`ARCHITECTURE_ARMORY_2026_07_20.md` §3: legacy `AgentSkill`/
`db_agent_skills` (per-agent-owned only, `ON DELETE CASCADE` FK to
`db_agent_definitions`) coexists with the v1 `Skill`/`db_skills` +
`db_agent_skills_ref` join-table pattern that Armory's actual UI drives.
The legacy path still exists in the schema; not confirmed dead, not
confirmed load-bearing — flagged for the same reason `preset.*` is: an
incomplete migration left both halves standing.

---

## 3. Proposed target architecture

### 3.1 Naming: retire "memory" for anything but native memory

- Rename `bundle_memory_get`/`bundle_memory_list`/`bundle_memory_upsert`/
  `bundle_memory_delete` → `bundle_get`/`bundle_list`/`bundle_upsert`/
  `bundle_delete` (mechanical rename, `Store` trait + all call sites).
- Change the Bundles pane's registered `view: "memory"` → `view: "bundle"`,
  with a one-time migration for any persisted pane-layout JSON that
  references the old string (same category of concern
  `ARCHITECTURE_ARMORY_2026_07_20.md`'s own corrections section already
  handles for other renames).
- Delete `preset.*` RPC aliases outright — `bundle.*` has been the primary
  surface since Phase 4a; if telemetry shows zero live callers, there's no
  reason to keep carrying the old name forward indefinitely.
- Net effect: "memory" means exactly one thing in this codebase — the
  agent's own native/brain memory — everywhere, no exceptions, no
  qualifiers needed to disambiguate.

### 3.2 One coherent Agent↔Bundle binding model

Replace the three mechanisms in §2.2 with:
- **Identity Bundle** — every agent has exactly one, DB-enforced
  (`db_agents.memory_id` kept as the column, renamed `identity_bundle_id`
  for clarity, `UNIQUE` constraint added). This is the bundle an agent
  "is" — editable, owned, what Stash's own bundle view shows by default.
- **Attached Bundles** — a real `db_agent_bundle_ref(agent_id, bundle_id, role)`
  join table, matching the Skills/MCP pattern exactly. `role` distinguishes
  the two things `is_global` and `startup_bundle_id` used to do
  separately: `role='global'` rows are implied for every agent (no ref row
  needed, same as global skills today) and materialize into CLAUDE.md;
  `role='startup'` rows are the (at most one) bundle whose `instructions`
  populate Session Context. Both auto-materialize at spawn — closing the
  gap `ARCHITECTURE_ARMORY_2026_07_20.md` §2 flagged where `memory_id`
  specifically was pull-only while everything else auto-injects.
- This directly overlaps with `SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md`'s
  in-flight Bundle↔MCP-server / Bundle↔Skill ref-table proposals — **should
  be designed together, not independently**, since both are "give Bundles
  the ref-table treatment" efforts landing in the same window.

### 3.3 Retire `db_agent_content` as a general-purpose store

Each of the six known `content_type` values (§2.3) becomes a real,
typed column or table:
- `soul`, `agentmd`, `env` → likely columns on `db_agents` or
  `db_agent_definitions` directly (small, single-value, always-present).
- `startup` (legacy freeform) → superseded by §3.2's `role='startup'`
  bundle attachment; kept only as read-only fallback data during
  migration, then dropped.
- `mcp` (legacy freeform `.mcp.json` blob) → already superseded in
  practice by `db_mcp_servers` + ref table for agents with any bound
  server (`ARCHITECTURE_ARMORY_2026_07_20.md` §4); formalize the
  fallback-then-drop path the same way as `startup`.
- Net effect: every piece of per-agent config is findable by reading the
  schema, not by grepping for string literals across the frontend.

### 3.4 One RPC surface per domain

Delete the `listmemories`/`getmemory`/`upsertmemory`/`deletememory`/
`reorderglobalbrain` UI-facing surface (§2.4); point the Bundles UI at
`bundle.*` directly, same as every other Armory tab already does. One
naming convention (`<domain>.<verb>`) across Bundles, Skills, MCP Servers,
and native memory alike.

### 3.5 A generic entity-versioning primitive

`SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md` proposes
`db_agent_native_memory_versions` as a bespoke table for one entity type.
In this no-cost-constraint vision, that becomes one instance of a shared
pattern instead:

```sql
CREATE TABLE IF NOT EXISTS db_entity_versions (
    id                 TEXT PRIMARY KEY,
    entity_type        TEXT NOT NULL,   -- 'native_memory' | 'bundle' | 'skill' | 'mcp_server'
    entity_id          TEXT NOT NULL,   -- agent_id+filename composite, or bundle_id, skill_id, etc.
    content_snapshot   TEXT NOT NULL,   -- JSON serialization of the entity's versioned fields
    content_hash       TEXT NOT NULL,
    parent_version_id  TEXT,
    source             TEXT NOT NULL,   -- 'human' | 'agent_inferred' | 'jekt' | 'external_fs_write' | 'revert'
    source_detail      TEXT NOT NULL DEFAULT '{}',
    created_at         INTEGER NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_entity_versions_lookup
    ON db_entity_versions(entity_type, entity_id, created_at);
```

One write-path helper (`record_version(entity_type, entity_id, snapshot,
source)`), called from `bundle.upsert`, `skill.upsert`, `mcp.upsert`, and
`agent:memory:write_file` alike. One history/diff/revert RPC triple,
parameterized by `entity_type`, instead of one per domain. One UI
component (extending the shared component §4.3 of the memory-versioning
spec already proposes) rendering any entity's history, reused across
Armory's Bundles/Skills/MCP/Native-Memory tabs identically. This is the
"wholistic" framing the operator asked for: build the durability/review
primitive once, apply it uniformly, rather than re-solving "how do we
diff and revert a config change" once per entity type as each gets its
own feature request.

### 3.6 Skills: finish the legacy migration

Confirm whether `AgentSkill`/`db_agent_skills` (§2.6) has any live
callers; if not, delete it. If it does, migrate those callers to the v1
`Skill`/`db_skills` + ref-table pattern and then delete it. Either way,
one skill storage model, not two.

---

## 4. Sequencing (cost-aware, despite the no-cost framing of §3)

Realistically these aren't equally cheap or equally risky, and some
depend on others. Recommended order:

1. **§3.1 naming pass** — almost entirely mechanical (rename, no behavior
   change), lowest risk, unblocks nothing else but makes every subsequent
   diff easier to read. Do this first, independent of everything else.
2. **§3.4 RPC surface unification** — mechanical, low risk, same
   rationale as #1. Can happen alongside #1.
3. **§3.2 unified Bundle binding** — the biggest behavior change (real
   data migration, changes materialization semantics for `startup_bundle_id`-bound
   agents). Coordinate with `SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md`
   explicitly before starting, since it's proposing adjacent ref tables
   right now — a combined design avoids two competing ref-table shapes
   landing in the same quarter.
4. **§3.5 generic versioning primitive** — depends on nothing above
   structurally, but is far more valuable done *after* #3.2, since
   otherwise it has to version three inconsistent binding shapes instead
   of one. If memory versioning (the sibling spec) needs to ship before
   this consolidation lands, it can start with its own bespoke table now
   and migrate onto `db_entity_versions` later — the two are not mutually
   blocking (see §6).
5. **§3.3 `db_agent_content` retirement** — depends on #3.2 for the
   `startup`/`startup_bundle_id` piece specifically; the rest (`soul`,
   `agentmd`, `env`) can move independently, any time.
6. **§3.6 skills cleanup** — independent of everything else; do whenever
   convenient, ideally before #3.5 so the generic versioning primitive
   only ever has to model one skills storage shape.

---

## 5. Non-goals

- Not a single PR — this is a multi-quarter program, sequenced in §4.
- Not resolving Warden's own governance gaps (durable jekt audit log,
  `governance.json`, approval queue) — those are cataloged in
  `SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md` §5 as
  Warden's separate charter, unrelated to Armory's foundation.
- Not proposing new user-facing features — every item in §3 is a
  same-behavior structural cleanup (naming, storage shape, one RPC
  instead of two), not new product surface. New surface (e.g. actually
  building the versioning UI) is the sibling memory spec's job.
- Not deciding `SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md`'s open
  questions for it — §3.2 flags the overlap and recommends coordination,
  not a specific resolution.

---

## 6. Relationship to the memory-versioning spec

`SPEC_MEMORY_VERSION_CONTROL_AND_ARMORY_AUDIT_2026_08_19.md` does not need
to block on this doc. It can ship exactly as designed — a bespoke
`db_agent_native_memory_versions` table, `agent_id`-scoped, no Bundle FK —
and later be migrated onto §3.5's generic `db_entity_versions` table as
one more `entity_type` once/if this consolidation is adopted, with
`entity_id` set to the same `agent_id:filename` composite it already uses.
The migration would be additive (copy rows, add a compatibility view or
just repoint the RPCs), not a rewrite of the versioning *design* — the
version-chain semantics (append-only, `parent_version_id`, `source`
tagging, revert-as-new-write) are identical in both. Building memory
versioning now, generically-shaped-later, is explicitly fine — it does
not need to wait on §4's multi-quarter sequencing.

---

## 7. Open questions for the human operator

1. **Sequencing buy-in** — does §4's order match priorities, or should
   e.g. the naming pass (§3.1, cheap, high clarity value) happen
   immediately regardless of the rest?
2. **`SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md` coordination** — should
   that in-flight spec be paused/revised to incorporate §3.2 directly, or
   land first and have §3.2 build on top of whatever ref-table shape it
   ships?
3. **`preset.*` alias deletion** (§3.1) — any known external/scripted
   callers still depending on the old name that would need a deprecation
   window instead of a hard cutover?
4. **Scope of `db_agent_content` retirement** (§3.3) — worth doing all six
   `content_type` values in one pass, or peeling them off individually as
   each gets touched for unrelated reasons?
