# Spec: Durable Bindings

**Status:** active. Phase 1 landed (#3175). Both §8 decisions are now made —
identity store (item 1); item 2 keeps `db_agents` unpromoted, resolving agent
identity as local-`db_agents`-OR-registry rather than registry-only (revised
from an initial registry-only framing that Codex correctly flagged as
rejecting real, currently-bindable agents — PR #3179), which in turn requires
Phase 5 to add explicit agent-delete cleanup of identity-store refs (no
cross-database `ON DELETE CASCADE`) — see §7 Phase 5 and §8 item 2 for the
full mechanism. Phase 2 in progress: schema (#3180) and the carry migration
(#3181) landed; the application-layer read/write redirect (§5.4) is the
remaining piece.
**Date:** 2026-09-10
**Verified against:** `94d9c6c1c` (code and live on-disk data, not spec prose)
**Follows:** `SPEC_INSTRUCTION_AND_MEMORY_PORTABILITY_2026_09_09.md` §3.4a, which
found the first instance of this and blocked on it
**Related:** #3148 (tracking), #3168 (drafted, blocked by this),
`SPEC_VERSION_ISOLATION_2026_06_01.md` §5 Phase 2 (the scoping this collides
with), `SPEC_IDENTITY_STORE_SPLIT_2026_08_17.md` (the store this proposes to
use), `SPEC_V1_MCP_SKILLS_PRIMITIVES_2026_06_30.md` (the primitives affected)

---

## 1. The requirement

`SPEC_INSTRUCTION_AND_MEMORY_PORTABILITY_2026_09_09.md` §1 states the goal as
"tight control over anything the agent uses as instructions and memory,
including portability" — for every byte reaching an agent, be able to answer
*where did this come from*, *what is in it now*, and *can I move it to another
machine*.

That spec's §3.4a found one thing that fails the test: a bundle's skills and MCP
servers do not survive a version upgrade. Investigating the fix turned up that
§3.4a is not one bug. **It is one shape, appearing six times**, and fixing only
the instance §3.4a names would leave the other five and produce a fix that does
not work end to end — the exact failure reagentx caught on PR #2632 (§3.3).

## 2. What exists today

Verified against `94d9c6c1c` and against live data in `~/.agentmux`. Descriptive
— no proposals in this section.

### 2.1 Three stores, two scopes

| Store | Path | Scope |
|---|---|---|
| Channel store | `<data_dir>/db/objects.db` | **per channel AND per version** |
| Shared store | `~/.agentmux/shared/store.db` | host-global, one file |
| Identity store | `~/.agentmux/shared/identity-store.db` | host-global, one file |

The per-version half is easy to miss. `DataPaths::resolve_internal`
(`agentmux-common/src/data_paths.rs`) puts `data_dir` under
`channels/<ch>/versions/<v>/` for Installed and Portable runtimes, per
`SPEC_VERSION_ISOLATION_2026_06_01.md` §5 Phase 2, and **nothing carries the
previous version's `objects.db` forward**. `config/` and `agents/` are
deliberately hoisted to channel level so settings and agent definitions survive
upgrades; `data/` is not.

Confirmed on disk: `channels/stable/versions/*/data/db/objects.db` is a separate
file per release — five of them on the machine this was written on, plus 25 more
across `local-*` and dev channels.

### 2.2 What lives where

Nine tables are duplicated into all three stores for durability
(`db_bundles`, `db_accounts`, `db_agent_credentials`,
`db_agent_identity_links`, `db_agent_native_memory`,
`db_agent_native_memory_versions`, `db_cron_jobs`, `db_drone_definitions`,
`db_muxbus_credentials`). Agent definitions are durable by a different
mechanism — the global registry under `~/.agentmux/shared/agents`.

These exist **only** in `objects.db`, and so exist only for one channel at one
version:

| Table | What it holds | Durable? |
|---|---|---|
| `db_skills` | the skill catalog | **no** |
| `db_mcp_servers` | the MCP server catalog | **no** |
| `db_bundle_skills_ref` | bundle → skill | **no** |
| `db_bundle_mcp_ref` | bundle → MCP server | **no** |
| `db_agent_skills_ref` | agent → skill | **no** |
| `db_agent_mcp_ref` | agent → MCP server | **no** |
| `db_agent_project_instructions` | observed instruction hashes | no — but see §6 |

Everything those six rows bind is durable. Bundles are host-global. Agents are
host-global. Only the bindings, and the catalogs they point into, are not.

### 2.3 The identity of a skill is not stable

This is the finding that determines the shape of the fix, and it is stronger
than "bindings are lost."

**As verified against `94d9c6c1c`** (the snapshot this section describes),
`m0015_seed_starter_skills` / `m0016_seed_starter_mcp_servers` seeded the
starter catalog per store, minting a fresh random UUID each time. Two versions
of the same channel therefore held the same six skills under six *different*
ids:

```
channels/stable/versions/0.55.10/…/objects.db
  84fe8721-e509-4f20-927f-8e05971ef197 | Systematic Debugging
channels/stable/versions/0.55.32/…/objects.db
  455d8994-9d8c-4e2b-aeb4-d4307afe286d | Systematic Debugging
```

**This is now Phase 1's legacy case, not current behavior.** #3175 (landed,
§7) made the id deterministic — derived from the entry's stable key rather than
random — so any store seeded AFTER that change mints the SAME id as every
other store for the same starter entry. The divergence above describes every
store that seeded BEFORE #3175 shipped — which is still every store that
exists today, since Phase 2's migration (the thing that would reconcile them)
hasn't run yet. A reader designing that migration needs to handle both
populations: legacy stores with divergent ids (name-matched, per §5.3) and any
post-#3175 store (already-converged, no matching needed).

So a ref row carried forward verbatim from 0.55.10 would dangle in 0.55.32: the
skill it names does not exist there, and the skill that *is* "Systematic
Debugging" has an id nothing points at. **Relocating the ref tables alone would
not work.** A binding is only as durable as the identity it references.

Across all 30 stores on this machine: 168 skill rows, **6 distinct names**, 28
copies of each — i.e. every row is a re-seeded starter and nothing is
user-created.

## 3. The gaps

### 3.1 Bindings do not survive an upgrade

`bundle_skill_bind` writes one row into the channel store (`managed_bind_bundle`,
`storage/managed.rs`), consulting the shared store only to confirm the bundle
exists. It never writes back to `db_bundles`' inline columns. The same holds for
`bundle_mcp_bind`, `agent_skill_bind` and `agent_mcp_bind`.

So a skill bound under one channel-and-version is invisible to every other,
including the next release on the same channel. Since #3152/#3153 made ABF
export read the ref tables, this now affects export as well as launch.

### 3.2 A user-created skill or MCP server does not survive at all

Weaker-sounding than §3.1 and worse in practice. The binding at least has a
row somewhere; a skill created in 0.55.10 has no representation in 0.55.32's
catalog. There is no copy, no mirror, and no migration that looks backward.

Latent today: zero user-created skills exist on this machine (§2.3). It stops
being latent the first time anyone uses the feature the Armory advertises.

### 3.3 Fixing one layer does not fix the bug — precedent

This exact mistake has already been made once and caught in review.
`SHARED_STORE_SCHEMA_VERSION` v2's own doc comment records it:

> the link resolves via this store now, but the account row it points to could
> still only exist in the per-channel-isolatable `id_store`, which is empty on a
> fresh channel — the reported bug wasn't actually fixed end to end without this
> (reagentx P0 review on PR #2632)

Agent→account links had been promoted without promoting `db_accounts`. Skills
and MCP servers are the same shape with the same trap, which is why §5 orders
catalogs before refs rather than treating them as independent.

### 3.4 The bundle→component case additionally blocks #3168

`m0030` seeds a store's ref tables from `db_bundles`' inline `skills` /
`mcp_servers` columns at that store's first boot, and nothing else ever does.
Those columns are therefore **the durable representation** of a bundle's
components, and the per-store ref tables are a projection of them — the reverse
of what Phase 0b assumed. Dropping the columns (#3168) removes the seeding
source for every future release for every user.

Once §5 lands, the columns genuinely become redundant and #3168 can proceed.

### 3.5 The agent→bundle binding has the same problem, one layer up

`DefinitionRecordV1` does not carry `memory_id`
(`storage/def_registry_mirror.rs`). The bundle row is global and the agent
record is global, but the binding between them is channel-local: a cross-channel
reopen starts unbound, and #3147's export warning cannot fire for a bundle
exported from a channel other than the one its agent was created in — precisely
the portability case. Already tracked on #3148; named here because it is the
same shape and should be fixed by the same principle.

## 4. The invariant

Generalizing `SPEC_INSTRUCTION_AND_MEMORY_PORTABILITY_2026_09_09.md` §5.3, which
states the same idea for one agent's native memory:

> **A binding, and everything it points at, has exactly one durable home,
> resolvable from a stable id alone, and unaffected by the channel or the
> version under which the binding was made.**

Two clauses, both load-bearing. "And everything it points at" is §3.3's lesson.
"Resolvable from a stable id alone" is what §2.3 currently violates.

Corollary, worth stating because it is the rule that would have prevented this:
**a table in `objects.db` may reference a global row, but no global row may
depend on a table in `objects.db` to be meaningful.** A bundle whose components
live only in one version's `objects.db` is exactly that dependency inverted.

## 5. Design

### 5.1 Decision: promote the catalogs and the bindings to the identity store

Six tables gain a declaration in `run_identity_store_schema` and become
authoritative there: `db_skills`, `db_mcp_servers`, `db_bundle_skills_ref`,
`db_bundle_mcp_ref`, `db_agent_skills_ref`, `db_agent_mcp_ref`.

**They keep their `run_object_schema` declarations.** "Promote" means moving
where the authoritative row lives, not deleting the local table — the same shape
the nine already-durable tables use, all of which are still declared in all
three schemas. This is not redundancy for its own sake: `bootstrap.rs`
deliberately substitutes `wstore` for `identity_store` when the identity store
cannot be resolved, created, or opened, so that "a resolution/open failure
degrades to today's per-channel behavior rather than being fatal." Undeclaring
the tables would convert that documented degraded path into missing-table errors
on every catalog and binding call (Codex, PR #3173). The local copies stop being
written as authoritative; they do not stop existing.

Authoritative, **not** the read-through fallback-mirror pattern `db_accounts`
uses. That pattern exists for one documented reason — Armory's disposable
delete-account testing flow needs genuine per-channel isolation
(`SHARED_STORE_SCHEMA_VERSION` v2). Skills and MCP servers have no such need and
the opposite intent: `ARCHITECTURE_ARMORY_2026_07_20.md` scopes the Armory to
"shared/reusable resources only," and
`SPEC_ARMORY_PHASE5_CONSOLIDATION_AND_SKILL_SEEDING_2026_07_13.md` removed the
per-agent Identities rail for that reason. A skill that is invisible from the
next release is not a reusable resource.

The identity store rather than the shared store because it is the one described
as "permanently global" (`SPEC_IDENTITY_STORE_SPLIT_2026_08_17.md`) and is not
subject to the channel-isolation defaults that
`SPEC_ISOLATED_AUTH_DEFAULT_BY_CHANNEL_2026_08_06.md` applies elsewhere.

### 5.2 Options rejected

**Stable ids for seeded skills.** Derive the starter UUIDs deterministically so
§2.3's divergence disappears. Cheap, and worth doing anyway as a complement —
but it fixes only the six starters. A user-created skill still vanishes on
upgrade, so it does not satisfy §4. Not a solution on its own.

**Carry `objects.db` forward on upgrade.** Directly contradicts
`SPEC_VERSION_ISOLATION_2026_06_01.md` §5 Phase 2, which introduced per-version
data dirs so two concurrent releases could not share SQLite DBs. It would also
carry forward everything, including rows that are legitimately per-version
(`db_background_tasks`, `db_drone_runs`, `db_agent_history`). Rejected: it
un-fixes a shipped fix to work around a scoping error in six tables.

**Leave the inline columns as the durable representation.** i.e. accept §3.4,
close #3168, and keep writing both. This is the status quo and it is what Phase
0b already rejected on its own terms: two places to write one fact, with nothing
keeping them in sync, is the §3.4 divergence that started all of this. It also
does not help §3.1 (binds still do not propagate) or §3.2 (catalogs still do not
survive) — it only stops the bleeding for bundle components specifically.

### 5.3 Migration

Ordering is the whole design, per §3.3: **catalogs first, then refs, then the
column retirement.**

Deduplication is the interesting part, and it is narrower than it first looks.
Rows must be matched on something other than id, since §2.3 establishes ids are
not comparable across stores. **Name is not sufficient, and matching on it would
destroy data** (Codex, PR #3173).

`managed_upsert_unique` (`storage/managed.rs`) enforces name uniqueness as
`name = ?1 AND (is_global = 1 OR id IN (<refs for THIS owner>))` — i.e. **within
an owner**, not globally. There is no `owner_id` column on `db_skills`;
ownership is expressed by `is_global = 0` plus a ref row. So two bundles may each
hold a private skill called `Deploy` with entirely different content, and that is
a valid state today. Collapsing them on name would discard one payload and leave
both owners editing the survivor.

The rule is therefore:

- **Seeded globals** (`is_global = 1`, matching a known starter) dedup across
  stores on name plus `skill_type`. This is the 168 → 6 case, and it is the only
  case where cross-store collapse is safe, because a global skill is by
  definition not owned by anyone.
- **Owner-private rows** (`is_global = 0`) are carried over **without
  deduplication**, keeping one row per (owner, row) pair and minting a fresh id
  where two stores' ids collide. Identical content under two owners stays two
  rows: they are two resources that happen to agree, and merging them creates an
  edit channel between owners that does not exist today.

Ref rows are rewritten to the surviving id as part of the same migration, never
left to dangle.

Doing this now is cheaper than it will ever be: on the machine this was verified
against, every row is a seeded global and nothing is owner-private, so only the
first bullet has any work to do. **That is a statement about one machine, not a
claim that the migration is lossless in general** — the second bullet exists
precisely because other installs will not be so tidy.

### 5.4 Application-layer read/write redirect is not a call-site swap

Discovered while implementing Phase 2's redirect (this section documents the
finding before the code that acts on it, per this repo's own review discipline
— a design this load-bearing should be reviewable on its own).

The naive plan — change every handler's `wstore.skill_get(...)` to
`state.identity_store.skill_get(...)` and stop — is correct for the methods
that touch only `db_skills`/`db_mcp_servers` (`skill_get`, `skill_delete`'s
catalog half is not this simple, see below, but `skill_upsert_unique_global`,
`skill_list_all_raw`, `skill_find_global_by_name_and_type` and their MCP
equivalents genuinely are pure single-table reads/writes with no ref-table
involvement — true zero-touch redirects). It is **wrong** for every method
that joins the catalog table with an agent- or bundle-level ref table in one
SQL statement — `managed_list`, `managed_list_global`,
`managed_list_global_for_agent`, `managed_delete`, `managed_is_accessible_to`,
and `managed_upsert_unique` (`storage/managed.rs`) all do exactly this, e.g.
`managed_list`'s `FROM {table} s WHERE s.is_global = 1 OR s.id IN (SELECT
{refc} FROM {reft} WHERE {key} = ?1)`. `db_agent_skills_ref` and
`db_bundle_skills_ref` are **not** promoted by Phase 2 — only Phase 3/6
promote them, and Phase 6 has a hard prerequisite (Phase 5) not yet built. So
for the whole window between Phase 2 shipping and Phase 6 landing, the ref
table and the catalog table it needs to join against live in two different
physical SQLite files. A single-connection query against either file alone is
wrong: querying `wstore` alone reads a catalog mirror that has stopped being
written (per §5.1, "authoritative, not read-through fallback-mirror" — the
mirror goes stale the moment writes redirect to `identity_store`); querying
`identity_store` alone finds no ref rows at all (they never move there in
Phase 2).

**Resolution:** the affected `managed.rs` methods gain an explicit `catalog:
&Store` parameter, decoupled from `self` (which keeps meaning "the ref-table
owner," i.e. `wstore`, unchanged in this phase). Each becomes two queries
composed in Rust instead of one SQL join: fetch the relevant ref ids from
`self`, fetch the relevant catalog rows from `catalog`, combine and sort in
memory. This is not a new pattern — `managed_bind_bundle` and
`managed_upsert_unique_for_bundle` already take an `id_store: &Store`
parameter for bundle-existence checks, for the identical reason (existence
lives in a different store than the ref row being written). `managed_get`
needs no such change — it is genuinely single-table, so it is called directly
on `identity_store` as receiver, no `managed.rs` signature change at all.

**Codex P1, PR #3182: the local ref tables still FK to the local catalog
table — binding a newly-created row would fail outright, not just read
stale data.** Confirmed against the code, not just the review text:
`Store::configure_and_migrate` sets `PRAGMA foreign_keys=ON` on every
production connection (`storage/store.rs`, the block whose own comment
already warns "without this pragma... cascades silently no-op" — the same
enforcement cuts both ways), and `db_agent_skills_ref`/`db_agent_mcp_ref`/
`db_bundle_skills_ref`/`db_bundle_mcp_ref` all declare `FOREIGN KEY
(skill_id/mcp_id) REFERENCES db_skills(id)/db_mcp_servers(id) ON DELETE
CASCADE` against `run_object_schema` — i.e. `wstore`'s **local** copy of the
catalog table (`storage/migrations.rs` v10/v23 block, lines 717-770).
Once catalog writes redirect to `identity_store` only, `wstore`'s local
`db_skills`/`db_mcp_servers` stops gaining new rows — so `managed_bind_agent`
or `managed_bind_bundle`'s `INSERT INTO {ref table} ... skill_id = ?` for any
row created after the redirect ships has no matching local parent row, and
`PRAGMA foreign_keys=ON` turns that into a hard `FOREIGN KEY constraint
failed` error, not silently stale data. Splitting reads into two queries (the
fix above) does nothing for this — it is a **write**-path constraint. Fixing
one layer without the other was exactly §3.3's lesson repeating itself one
level down.

The bundle-level ref tables already had to solve the identical problem for
`db_bundles` — `db_bundle_skills_ref`/`db_bundle_mcp_ref` deliberately carry
**no** FK to `db_bundles` at all (`migrations.rs`'s own comment on those
tables: "Bundles are authoritatively written through `id_store`... An FK
against [wstore's local copy] would make every real bind fail... Existence is
checked at the application layer instead"). That is precisely the shape this
gap needs, one layer over: a new **Channel**-scoped migration
(`OBJECT_SCHEMA_VERSION` bump) rebuilds all four ref tables — SQLite has no
`ALTER TABLE ... DROP CONSTRAINT`, so each is recreated without its
`skill_id`/`mcp_id` FK (columns, `PRIMARY KEY`, and — for the agent-level
pair — the existing `agent_id → db_agents` FK are all preserved verbatim;
only the catalog-pointing FK is dropped, run with `PRAGMA foreign_keys=OFF`
for the duration of the rebuild, the standard SQLite procedure for this).
Existence moves to the application layer: `managed_bind_agent` gains the same
`catalog: &Store` parameter as the read methods above and checks
`catalog.managed_get::<R>(id).is_some()` before inserting the ref row —
mirroring `managed_bind_bundle`'s existing `id_store` check exactly, just one
store later in the same chain.

**Codex P2, PR #3182: the unique index doesn't match the invariant the
application actually enforces, for skills specifically.**
`skill_upsert_unique_global`'s dup-check is `WHERE name = ?1 AND id <> ?2 AND
is_global = 1` — no `skill_type` filter, i.e. the application's own
contract is "a global skill's name is unique, full stop, regardless of
type" (its error message agrees: "a global {} named '{}' already exists," not
scoped to type). But Phase 2a's unique index,
`idx_ids_skills_global_name_type`, is keyed on `(name, skill_type)`
(`migrations.rs`, `IDENTITY_STORE_SCHEMA_VERSION` v7) — narrower than the
invariant it's meant to back. Verified this is a genuinely new exposure, not
a preexisting one merely mislabeled: `db_skills` was always per-channel
before Phase 2, so the only concurrency `managed_upsert_unique_global` ever
faced was same-process, already fully serialized by `Store`'s own
`Mutex<Connection>` — the missing index was irrelevant because nothing could
race across it. Once the catalog is shared across processes (channels), that
serialization no longer holds, `conn.transaction()` is SQLite's default
DEFERRED behavior (no `TransactionBehavior::Immediate` override anywhere in
`managed.rs`), and a same-name-different-`skill_type` race between two
channels can have both transactions' dup-check SELECTs run before either's
INSERT takes the write lock — both see zero conflicts, both insert, and
`idx_ids_skills_global_name_type` has nothing to say about it since the keys
genuinely differ. (MCP servers don't have this mismatch —
`mcp_server_upsert_unique_global`'s dup-check and
`idx_ids_mcp_servers_global_name` are both name-only already; this bullet is
skills-specific.) Fix: narrow the skill index to `(name) WHERE is_global = 1`
— matching what the application already promises — via an
`IDENTITY_STORE_SCHEMA_VERSION` bump that drops the old index and creates the
new one. Because `m0031`'s own dedup key is `(name, skill_type)` (§5.3), this
migration must defensively handle any store where that carry already
produced two same-name/different-type global rows: collapse to the
earlier-`created_at` row exactly the way `m0031` collapses starters, rewrite
the loser's refs to the survivor, delete the loser — the same
detect-then-converge shape, applied once more, this time scoped `Global`
(cleaning up `identity_store`'s own rows, not carrying from a channel).

**Codex P2, PR #3182: `managed_upsert_unique`'s insert-plus-bind is no longer
atomic — add compensation, not just documentation.** The original "accepted
interim gap" framing here undersold it: the transaction split doesn't only
widen the owner-private name-uniqueness race (still real, still accepted
below, still low-severity for the reasons already given) — it also means a
catalog insert that has already committed to `identity_store` can be
followed by a ref-bind insert into `self` (`wstore`) that fails (lock
contention, disk full, a channel-local constraint), leaving a durable,
unreferenced catalog row that no ordinary list/access/delete path can
discover (they all filter through a ref table or `is_global`, and a stray
private row is neither bound nor global). Fix: `managed_upsert_unique`
performs a **best-effort compensating delete** of the just-inserted catalog
row if the subsequent ref-bind fails — `catalog.managed_delete::<R>(catalog,
row.id())`-equivalent cleanup before propagating the bind error up, so the
RPC's failure and the durable state agree ("nothing was created") instead of
diverging ("something was created but the caller was told it wasn't"). This
does not restore full atomicity (the compensating delete can itself fail,
e.g. on the same disk-full condition that broke the bind) — it converts a
silent, permanent orphan into, at worst, a logged best-effort-cleanup
failure with the same orphan, which is a strict improvement and cheap to add
given the delete path already exists. The narrower race — two concurrent
`skill.upsert` calls for a brand-new name, same owner, same instant, both
passing the dup-check before either inserts — remains accepted as interim:
single-owner, single-submit UI flows are not a realistic multi-writer
scenario, and closing it fully needs either a cross-database saga or
promoting the ref tables early (Phase 3/6), neither justified by this
phase's scope. Flag it again once Phase 6 lands, since promoting the agent
ref tables restores single-connection atomicity for free.

**Cross-channel dangling refs on catalog delete, still accepted as
interim (not part of Codex's PR #3182 findings — carried over unchanged).**
`skill.catalog.delete` purges this channel's own ref rows (`managed_delete`)
before removing the now-global catalog row — but a *different* channel's
`db_agent_skills_ref`/`db_bundle_skills_ref` rows pointing at that same
catalog id are unreachable from the deleting channel's `wstore` and are left
dangling until that other channel's own migration/redirect eventually runs
(Phase 6). A dangling ref today already degrades gracefully — `skill_get`
against a now-missing id returns `None`, and every read path already handles
that — so the failure mode is "a stale bound-skill entry silently
disappears from that other channel's list," not a crash or data corruption.
Full closure is the same Phase 6 dependency as the point above.

## 6. Non-goals

**`db_agent_project_instructions` stays per-store.** It is an observation cache
— a content hash and a timestamp, deliberately never content
(`OBJECT_SCHEMA_VERSION` v33). Losing it on upgrade costs one re-observation at
the next launch and nothing else. It fails §4's letter and not its intent: it
does not *point at* anything, so there is nothing to dangle.

**Genuinely per-version tables stay put.** `db_background_tasks`,
`db_drone_runs`, `db_agent_history`, `db_agent_content`, and the LAN/jekt key
tables are either runtime state or keyed to a running instance.

**No UI change.** The Armory already presents skills and MCP servers as shared
resources; this makes the storage match what the UI already claims.

## 7. Phases

**Phase 1 — stable starter ids. DONE (#3175).** Deterministic UUIDs for
seeded skills and MCP servers. Independently shippable, independently useful,
and it shrinks Phase 2's dedup to the user-created case. Does not satisfy §4
alone (§5.2). A duplicate-trigger/-name manifest entry is now rejected loudly
at seed time rather than silently merged (reagent P2 x2, PR #3175) — the exact
safety net random ids removed by accident.

**Phase 2 — promote the catalogs.** `db_skills` and `db_mcp_servers` to the
identity store, with the dedup migration of §5.3. The largest single step and
the one with real data movement.

**Phase 3 — promote the two BUNDLE ref tables.** `db_bundle_skills_ref` and
`db_bundle_mcp_ref`, only after Phase 2 per §3.3. Ref rows rewritten to surviving
catalog ids. Safe to do now because the validation these bind paths perform —
"does this bundle exist" — reads `db_bundles`, which the identity store already
carries.

**Phase 4 — retire the inline bundle columns.** #3168 rebased; its mechanical
work (schema/SQL/frontend removal, the `m0031` guard shape, the `map_memory_row`
pin test) is already written and reviewed.

**Phase 5 — registry-resolvable agent identity.** `memory_id` into
`DefinitionRecordV1` (§3.5), **and** agent existence resolved through the
registry as an ADDITION to the local `db_agents` check, not a replacement for
it (revised from "resolved through the registry" — see below, Codex P1/P1 x2,
PR #3179).

This is a hard prerequisite of Phase 6, not a nice-to-have, and an earlier
revision of this spec had the two the wrong way round (Codex, PR #3173). The
agent bind path validates against `db_agents` **on the same connection as the
ref table** (`managed_bind_agent`, `storage/managed.rs`), and
`managed_union_bundle_refs` calls `agent_def_get` on that same store to read
`memory_id`. Promoting the agent ref tables before this phase would make every
direct skill/MCP bind error out and every bundle-derived component vanish from
launch.

**Registry-ONLY resolution is wrong, not just incomplete — the registry
deliberately excludes real, bindable agents, and the codebase already
documents why once:**

- Seeded templates (`def.is_seeded != 0`) and template launches never reach
  `agent_def_insert`, so `registry_def_upsert` skips both
  (`def_registry_mirror.rs`). Unnamed instances and continuations
  (`parent_instance_id` non-empty) are excluded from the instance registry the
  same way (`registry_mirror.rs::registry_upsert_if_named`).
- `managed_bind_agent`'s own doc comment already explains why it checks
  `db_agents` and not a registry-shaped table: *"checking the legacy table
  here would reject a bind for any agent that exists ONLY as a `db_agents` row
  (a template launch), even though the FK the INSERT below actually depends on
  would accept it"* (`storage/managed.rs`, written for Phase 3c / #3088).
  Registry-only resolution reintroduces exactly the bug that comment was
  written to prevent — this spec would have reverted a fix already on record
  for the identical reason.

**Revised design:** existence resolves as local-`db_agents`-OR-registry, not
registry-only. A launch-only agent's id is still a genuine, globally-unique
UUID regardless of whether the registry has a JSON file for it, so the bind
still writes its ref row into the (now-identity-store) ref table either way —
only the EXISTENCE CHECK gains a fallback, the ref row's location doesn't
depend on which check passed. This keeps launch-only agents bindable without
promoting `db_agents` itself, honoring §8 item 2's decision.

**That revision creates a second gap, and Codex caught it too:** once the ref
tables live in the identity store, `ON DELETE CASCADE`
(`FOREIGN KEY (agent_id) REFERENCES db_agents(id) ON DELETE CASCADE`,
`migrations.rs` v30) can no longer fire — it's a SQLite-native constraint
scoped to one connection, and `db_agents` and the ref tables are now different
files. `agent_def_delete`/`instance_delete` (`storage/agents.rs`) only ever
delete the local `db_agents` row; neither calls into the identity store today.
Without an explicit cleanup call, a deleted agent's skill/MCP refs become
permanently orphaned — inflating Armory bound counts with rows for agents that
no longer exist.

**Fix, same shape as existing precedent:** `agent_def_delete` already calls
`self.project_instructions_forget(id)` explicitly beside its own `db_agents`
delete, for the identical reason (no FK can express "this table has no
relationship SQLite can enforce, clean it up by hand"). Phase 5 adds the same
kind of explicit call — an identity-store ref cleanup for the deleted agent id
— to both `agent_def_delete` and `instance_delete`.

**Phase 6 — promote the two AGENT ref tables.** `db_agent_skills_ref` and
`db_agent_mcp_ref`, once Phase 5's revised existence check and explicit
delete-cleanup are in place.

**Phase 7 — memory location invariant.** The former portability Phase 4. Shares
§4's invariant but moves files rather than rows, so it keeps its own spec.

## 8. Decisions

1. **Identity store or shared store? Decided: identity store.** §5.1 already
   argued this; recorded here as final rather than open. The shared store's
   symmetry argument (it already holds `db_bundles`) doesn't outweigh the
   identity store being the one store explicitly documented as permanently
   global with no channel-isolation exceptions — every other exception in this
   codebase (`db_accounts`) exists for a specific, named isolation need
   (disposable test accounts) that skills and MCP servers do not share.

2. **Should `db_agents` be promoted, or should agent identity resolve through
   the registry? Decided: registry — as a fallback ADDED to the local
   `db_agents` check, not a replacement for it.** Agents are already durable
   via `~/.agentmux/shared/agents` — promoting `db_agents` too would build a
   second durable home for the exact fact the registry already owns, which is
   the two-sources-of-truth shape this whole spec exists to close, not one to
   reintroduce for agents specifically. The cost (§7 Phase 5 has to change
   `managed_bind_agent`'s validation and `managed_union_bundle_refs`'s
   `memory_id` lookup, not just relocate a table) is accepted as the correct
   trade against reopening the same class of bug for a second row type.

   **Registry-only was tried first and is wrong, not just incomplete** (Codex
   P1/P1, PR #3179): the registry deliberately excludes seeded templates,
   template launches, unnamed instances, and continuations —
   `managed_bind_agent`'s own doc comment already explains why the bind path
   checks `db_agents` rather than a registry-shaped table, for the identical
   reason, written for Phase 3c (#3088). See §7 Phase 5 for the full
   mechanism and the matching delete-cleanup gap it also exposed.

3. ~~What happens to rows in the existing `objects.db` files?~~ **Resolved by
   §5.1.** The local tables keep their declarations and simply stop being
   authoritative, so there is nothing to drop and no window in which a store
   lacks them. This also removes the #3168-shaped trap the question was worried
   about. Noted rather than deleted because the original framing — "promote
   means move" — is the intuitive one and is wrong here.
