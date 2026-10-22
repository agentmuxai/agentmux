# SPEC: Give pre-fix forks their clean agent id, without breaking anything keyed by the old one

**Date:** 2026-10-22
**Status:** active — Phase 0 in #5325, Phase 1 in #5329, Phase 2 in #5342; Phase 3 remains.
**Related:** [`SPEC_OFFLINE_RESILIENCE_2026_10_20.md`](SPEC_OFFLINE_RESILIENCE_2026_10_20.md) §4.8 and plan row F
(this is its "fork slug migration" half; the subscription half shipped in #5238),
[`REPORT_FORK_AGENT_ISOLATION_2026_10_12.md`](../reports/REPORT_FORK_AGENT_ISOLATION_2026_10_12.md) F4 and §5 (why
#4746 fixed new forks and chose not to rewrite existing slugs), `crates/common/src/slug.rs` (`fork_slug`).

## 1. Why

Before #4746, a fork's slug was its parent's slug plus its own label, so every generation chained: `Loap #2` is
`loap-loap-2`, its fork `Loap #3` is `loap-loap-2-loap-3`, and `Naki #2` is `naki-naki-2`. `fork_slug` now gives
`loap-2`, `loap-3`, `naki-2`, but existing forks kept their chained slug. Since #4744 the stored slug is also the
agent's address (`AGENTMUX_AGENT_ID`), so these agents answer to two names: the chained slug (their stable id) and
their display route name (`Loap-3`). Code that assumes one name per agent trips over it; on 2026-10-20 MuxBus
subscribed one name and dropped the other, and messages to the dropped one were lost (#5238 fixed that symptom).

This spec renames the chained slugs to what `fork_slug` gives, keeps the old one working as an alias, and never
lets the old one be reused.

**Affected on the machine this was written on:** three definitions. Every other fork already matches `fork_slug`.

| Definition | Name | Slug today | Slug after |
|---|---|---|---|
| `377e3948…` | Loap #2 | `loap-loap-2` | `loap-2` |
| `a0dfc499…` | Loap #3 | `loap-loap-2-loap-3` | `loap-3` |
| `e4797d71…` | Naki #2 | `naki-naki-2` | `naki-2` |

Their workspace folders (`loap-3-0822g`, `naki-2-09169`) are already clean: a workspace is named from the instance
name, not the slug.

## 2. Goals and non-goals

**Goals**
- G1. After migration each affected agent has one canonical id, the one `fork_slug` gives, everywhere AgentMux
  stores or derives it.
- G2. Nothing addressed to the old id is lost: messages, review notices (PR bodies carry the old
  `agentmux:agent_id`), tasks, schedules.
- G3. The old id is never given to another agent, and no key, credential or grant filed under it outlives it.
- G4. The migration is explicit, previewable, resumable after a crash, verifiable, and reversible.
- G5. A half-done migration can't leave an agent with two canonical ids.

**Non-goals**
- Making the uid the only address (the long-term fix, so a slug is never load-bearing again). §9 lists what this
  spec does toward it.
- Renaming display names, workspaces or Claude project folders. None of them is keyed by the slug.
- Slugs that differ from `fork_slug` for any other reason (a `-N` collision suffix, a root named after a template).
  Only slugs equal to the old chaining rule's output are candidates (§5.1).

## 3. What is keyed by the slug today

Read on `main` at v0.59.38. Scope: **host** = `~/.agentmux/shared`, **channel** = `channels/<ch>/`,
**version** = `channels/<ch>/versions/<v>/data/` (each holds its own `objects.db`).

| # | Item | Key | Scope | Today on rename |
|---|---|---|---|---|
| K1 | `db_agents.slug` | the slug | every version's `objects.db` | nothing writes it after insert |
| K2 | shared definition `definitions/<uid>.json` `slug` | the slug | host | written back from K1 by `registry_def_upsert` on insert or edit |
| K3 | host HMAC message key `db_agent_jekt_keys` | lowercased slug | version | kept under the old name |
| K4 | LAN Ed25519 key `db_agent_lan_keys`, public half in the cross-channel registry | lowercased slug | version, host | kept; peers pin it by name |
| K5 | WAN key (`wan.db` `wan_agent_keys`, `wan_agent_key_publications`), certified over the agent name | lowercased slug | channel, plus the account's key directory | kept and published under the old name |
| K6 | uid-keyed key copies (`*_by_uid`) | uid, `copied_from` = old name | version | fine |
| K7 | per-agent MuxBus credential (`db_agent_credentials`) | `<channel>/` + lowercased name | host `store.db` | provisioned per name the subscriber holds |
| K8 | relay state: subscription, WAN lease, mailbox, ownership | name | cloud | held per name |
| K9 | registration files `data/agents/<name>.json`, cross-channel registry `shared/agents/reactive/<name>/<channel>.json` | name | version, host | old entries linger until TTL |
| K10 | `gh-<slug>` GitHub config folder | `AGENTMUX_AGENT_SLUG` | channel | a new empty folder |
| K11 | bashwrap cwd file `state/bashwrap-cwd/<key>.cwd` | `AGENTMUX_INSTANCE_SLUG`, else the agent id | host | stale file |
| K12 | persisted `cmd:env` on the agent's blocks (`AGENTMUX_AGENT_ID`, `AGENTMUX_AGENT_SLUG`, git email) and the workspace `.mcp.json` | the slug | version, workspace | **wins on respawn** (`identity_env.rs`), so the agent keeps the old id |
| K13 | name-only rows: cron jobs with no `target_uid`, work-queue items with no uid, `db_conversation_trust_grants` | name | host, version | silently stop matching |
| K14 | `db_agent_former_names` | old display and instance names | version | records `name` and `instance_name` changes only, never `slug`; nothing routes by it |
| K15 | outside AgentMux: open PR bodies (`agentmux:agent_id=`), other agents' notes, commit trailers | the slug | GitHub, workspaces | unchanged |

**What reads which copy.** `agent_def_list`, and with it `agent.open` and the MuxBus subscriber, prefers K2.
`agent_def_get`, the spawn env (`AGENTMUX_AGENT_SLUG`), the key server and every SQL lookup use K1. Ownership checks
(`uid_has_slug`, `uid_owns_name`) require the current K1 slug to sign or verify. So changing one copy alone gives the
agent two canonical ids, which is exactly what G5 forbids.

## 4. Approach: expand, migrate, contract

The migration is done the way schema renames are done safely: first teach every reader to accept both names
(expand), then move the data (migrate), and only much later stop accepting the old one (contract). Each step ships
and is verified alone, and none of them depends on every instance being upgraded at the same moment.

### Phase 0, prerequisites (one PR; worth having anyway)

- **P0.1 One reader of the slug.** `agent_def_get`, the spawn env and the SQL lookups resolve the slug the same way
  `agent_def_list` does, so the two copies can't give two answers. A disagreement is logged.
- **P0.2 No stale write-back.** `registry_def_upsert` never replaces K2's slug with a different K1 slug if the K1
  slug is in K2's `former_slugs`. This stops an older, unmigrated copy from undoing the rename.
- **P0.3 The server wins on identity.** On every spawn, `AGENTMUX_AGENT_ID`, `AGENTMUX_AGENT_SLUG`, the git
  identity and `GH_CONFIG_DIR` come from the definition, not from a persisted `cmd:env`, and the workspace
  `.mcp.json` is rewritten from it. Today a persisted value wins.
- **P0.4 Delete covers every name.** Deleting an agent purges and tombstones name-keyed keys under its current slug
  and every former slug, not only former display names.

As built: the rule is "K2's slug when it is non-empty, else K1's" (`storage/agents/canonical_slug.rs`). P0.2 is
enforced in `DefinitionStore::upsert` for every writer: a non-empty K2 slug is never changed by an update, and
`former_slugs` only grows. So S1 must write the slug through its own path, not `upsert`. Every write of a K2 record
holds a cross-process lock on that record (`shared/agents/definitions/locks/<uid>.lock`, an OS file lock taken with a
5 s timeout) across its read, merge and write, so the guard holds between channels and instances too; S1 takes the
same lock through `DefinitionStore::with_record_locked`. A new user agent's slug is picked against every K2 slug and former slug, and published by its mirror, under one host-wide lock (`locks/_slug-allocation.lock`), so two channels can't pick the same one. The pick reads every K2 record strictly: a directory or record whose slug fields can't be read stops the insert, and a newer-schema record whose slug fields read counts as taken. P0.4's delete reads the agent's own K2 record strictly: a
record that exists but can't be read or parsed refuses the delete with nothing changed, while a definition with no
K2 record is deleted as before.

### Phase 1, expand: former slugs are aliases (one PR)

- **Storage.** `former_slugs: [{slug, retired_at}]` in the shared definition, mirrored in a
  `db_agent_former_slugs(agent_uid, slug, retired_at)` table per `objects.db`. A slug is a name, so it's lowercased.
- **Resolution.** Every path that turns a name into an agent also tries former slugs, after live names and the
  current slug, and resolves them to the uid:
  - local delivery, including the miss fallbacks (`handler.rs`, the cross-channel registry lookup);
  - the message resolve endpoint, Tasks and Schedule (`name_resolution.rs`);
  - the transcript and files lookups.

  A message arriving for a former slug is delivered to the agent and logged once per name per hour.
- **Identity is not aliased.** Ownership checks (`uid_has_slug`, `uid_owns_name`), signing and key serving accept the
  current slug only. A former slug can receive, never speak.
- **Reservation.** `resolve_slug_collision` and the fork-name suggester treat every former slug in any definition as
  taken, forever.
- **LAN.** A LAN peer's lookup of a former slug gets the agent's current name and public key, so it can re-pin
  under the new name. Pins stay first-use.
- **MuxBus, for a grace period.** The subscriber also subscribes each former slug, receive-only, for 30 days after
  `retired_at` (§9.1). This covers review notices for PRs opened under the old id. After that the former slug is released on the relay, still reserved locally.

  This deliberately reverses `live_names.rs`'s rule of never holding a former name, but only for a bounded period.
  A relay-side alias, mapping the old name to the new one so nothing needs holding, would be better; it's tracked as
  a separate cloud-side change.

As built:
- **Storage.** `db_agent_former_slugs(agent_uid, slug, retired_at)` is created on every open (additive, no version
  bump) and deleted with the agent's row, so a deleted agent's former slug is free again, as its slug is. Startup and a
  cross-channel backfill copy the shared `former_slugs` into it; a mirror write carries it back, where `upsert` only
  appends. Readers consult both copies (`storage/agents/former_slugs.rs`).
- **Resolution.** One rule, `Store::former_slug_owner`: the name is a former slug of exactly one agent and nobody's
  current slug. The reactive registry asks it only when no block is bound under the name, and only for a delivery
  target: delivery, the files and transcript lookups, the resolve endpoint and the LAN agent lookup opt in
  (`get_delivery_target`, `lookup_delivery_target`), while `get_agent` and `lookup_by_name`, which authenticate and
  act as an agent, never answer a former slug. It delivers by UID (`reactive/former_slug.rs`, logged once per name per
  hour). A name is looked up in full only when it is in an in-memory set of every former slug (re-read at most every
  2 s, cleared by a write through this store), so a sender or target that isn't one costs no store access. `agents_matching_name` falls back to it when nothing
  else matches (from the shared record for an agent with no row here), which covers the resolve endpoint (Tasks, Schedule) and the hold; `agent_defs_for_target` tries it
  after the route name. The miss path looks up the same-host and cross-channel registries under the current slug
  when the former one finds nothing, but forwards the request unchanged, since its signatures cover the target; the
  receiving instance resolves the former slug itself. An entry found that way is never evicted on a failed forward.
- **Identity.** A jekt whose sender claims a former slug is counted unverified (`sig_verified = false`, so it is
  forced sensitive with `ESCALATE=required`), signed or not, whatever its tier. One rule
  (`refuse_former_slug_sender`) holds it: the handler applies it to every message it delivers, so no delivery path can
  skip it, and `verify_jekt_signature` applies it before a request is forwarded. UI automation, which verifies a
  signature and then acts as its signer, refuses a signer named by a former slug. The ownership checks and
  `self/keys` already compared the current slug only.
- **LAN.** `GET /agentmux/reactive/agent?id=<former slug>` answers with `current_name` and the key filed under the
  current name, only when that name is the agent's alone; never a key left under the former slug. A peer that reads
  a valid `current_name` counts a signature claiming the former slug as failed and caches nothing from that answer,
  under either name: it learns the current name's key only from a lookup of that name, pinned as usual.
- **Reservation.** `resolve_slug_collision` also counts this channel's copy of the former slugs (PR 1 added the shared
  ones); a fork number whose name or fork slug is a former slug is skipped; `agent_def_find_or_insert` matches a former
  slug to its agent.
- **MuxBus.** The name directory holds each former slug, ranked below slugs and route names, while its `retired_at`
  is under 30 days old; an unknown time (0) is not held. After that, the next directory leaves it out and the lease
  tick drops the subscription as orphaned.

### Phase 2, migrate: an explicit, journaled operation (one PR)

**Shape.** It's an operation the operator starts, not a boot-time migration, because it needs the affected agents
stopped:
- an srv RPC `agents.migrateForkSlugs {mode: plan|apply|rollback, journal?}`;
- the CLI `agentmux-srv migrate-fork-slugs --plan|--apply|--rollback <journal>`, for development;
- a Settings › Agents card: "3 agents have an outdated id. Update them", with the plan shown.

**1. Plan (dry run, the default).**
- Compute candidates (§5.1) and their targets.
- Check every target is free: in every shared definition's slug and former slugs, every `objects.db` on disk
  (all channels and versions, not only running ones), the cross-channel registry and live registrations. A target
  held only by the same uid, such as its own display binding, counts as free.
- Enumerate every K1–K13 location per agent, with its current value.
- Write the plan as JSON plus a readable summary. Nothing is changed.

**2. Preconditions.** Refuse unless all of these hold:
- the affected agents run nowhere on this host, in any channel (no live registration, pid or block with a turn);
- every running AgentMux instance on the host is on a version with Phases 0 and 1;
- a host-global lock `shared/locks/slug-migration.lock` is taken (one migration at a time).

The UI offers to stop the affected panes, with the usual 15-second override.

**3. Back up.** Before the first write, copy every file it will touch into
`shared/backups/slug-migration-<ts>/`, and record SHA-256 checksums in the journal:
- `objects.db`, `wan.db` and `store.db`, copied with SQLite's online backup (`VACUUM INTO`), never a raw file copy
  of a database in WAL mode;
- the definition JSON;
- the registry entries.

**4. Apply, per agent and per step, with a write-ahead journal.**
- The journal `shared/backups/slug-migration-<ts>/journal.jsonl` records each step's intent before it runs and its
  result after.
- Every step is idempotent (running it twice gives the same state) and recorded as done, so a crash or power loss
  resumes at the first unfinished step.
- Database steps run in one transaction per `objects.db`; file writes go to a temp file and are renamed into place.

The steps:

| Step | What | Notes |
|---|---|---|
| S1 | Shared definition: `slug` = new, `former_slugs` += old | the canonical copy first, so Phase 0 readers follow it |
| S2 | Each `objects.db` on disk: `db_agents.slug`, `db_agent_former_slugs` += old | all channels and versions, including stale ones nothing runs, so none can write the old slug back |
| S3 | Retire name-keyed signing keys (K3, K4, legacy K5 rows) under the old name: delete them, and tombstone the old name | **Rotate, don't carry:** fresh keys are minted under the new name at the next spawn. Signed identity follows the name, and old-name keys can never be inherited (#3633 class) |
| S4 | `wan.db`: mark the old name's key and publications retired; ask the account's key directory to drop the old record where the API allows it | the new name's key is certified and published at the next spawn |
| S5 | `store.db`: keep the old name's MuxBus credential for the grace subscription; the subscriber provisions the new name by itself | removed at contract |
| S6 | Rewrite name-only rows (K13) from the old to the new name, and backfill the uid where it's missing | trust grants, cron jobs with an empty `target_uid`, queue items with no uid. History rows (Kanban `filed_by`, coordination posts) are left as written |
| S7 | Persisted identity (K12): rewrite `AGENTMUX_AGENT_ID`, `AGENTMUX_AGENT_SLUG` and the git email in `cmd:env` on the agent's blocks, in every `objects.db`, including layout snapshots (`session:last_topology`); delete the workspace `.mcp.json` so it's regenerated | belt and braces with P0.3 |
| S8 | Files: move `gh-<old>` to `gh-<new>` per channel (if the target exists, keep both and log); move the bashwrap cwd file; delete stale `data/agents/<old>.json` and `shared/agents/reactive/<old>/` entries | moves, not copies |

**5. Verify.** Re-read every K1–K13 location and assert, per agent:
- the new slug everywhere;
- the old slug only in `former_slugs` and the grace subscription;
- no signing key under the old name;
- resolving the old name and the new name both give the uid.

Any failure stops the run before the next agent and reports exactly what's wrong. The journal makes it resumable or
reversible.

**6. Roll back.**
- **Journal mode:** `--rollback <journal>` undoes the completed steps in reverse order (each step records its
  before-image).
- **Restore mode:** restoring the backup is offered only while nothing has written to those files since; checksums
  and mtimes are compared.
- Keys retired in S3 come back from the backup in either mode.

**7. After.** Relaunching the agents gives them the new `AGENTMUX_AGENT_ID`, fresh keys, a new WAN key publication
and a MuxBus subscription under the new name, plus the receive-only grace subscription under the old one. The report
lists what can't be changed from here:
- open PRs whose body carries the old `agentmux:agent_id` (fix them with `gh-agent pr edit`, or let the grace
  subscription cover them);
- other agents' notes and memory that mention the old name;
- LAN peers that will re-pin.

As built (`crates/srv/src/backend/slug_migration/`):
- **Where it runs.** On the disk, not on the running instance's stores: every `channels/<ch>/versions/<v>/data`,
  `channels/<ch>/data` and `dev/<branch>[/<clone>]/data` on the root, each install's `wan.db`, and the host's
  `store.db` and `identity-store.db`. Another version's `objects.db` is opened raw, never through `Store::open`, so
  it is changed only where a step says and never upgraded. The same code serves the CLI (`agentmux-srv
  migrate-fork-slugs [--plan|--apply|--rollback <journal>|--restore <journal>]`) and the RPC
  `agents.migrateForkSlugs {mode: plan|stop|apply|rollback|restore, journal?, quick?}`. `stop` and `restore` are
  additions: `stop` asks each pane the plan finds running to stop through the agents' 15 s user override
  (`fleet_bulk_stop_with_override`); `restore` is restore mode. `quick` finds the candidates only, for the notice.
- **Plan.** Candidates are read from the shared definitions; a definition with no shared record isn't considered.
  The old rule is matched against the parent's slug and its former slugs, so a parent migrated first (or by an
  earlier, interrupted run) doesn't hide its child. A target is held by:
  - another uid's shared slug, former slug or display route (`agent_route_name` of its name, case-insensitive);
  - a row's slug, former slug, display route or instance route in any `objects.db`;
  - another candidate's target;
  - a live cross-channel registration under it that isn't this uid's;
  - a live per-version registration whose pane (by `agentId`) is another agent's.

  A display route counts because `agent_defs_for_target` prefers a slug: an agent `Loap 2` with the suffixed
  slug `loap-2-2` would lose its messages to an agent renamed `loap-2`. A database that can't be read (corrupt
  or incompatible) blocks the plan, naming the file, and every candidate waits; no target reads as free.

  A refusal suggests the first free `-N`. Locations are listed through each step's own before-image, so the plan
  shows exactly what a run touches. A key row is reported by its presence, never its value.
- **Preconditions.** "Runs nowhere" means no fresh lease whose holder is still alive, and no live registration
  under the agent's slug, former slugs or route name.
- **Version signal.** Nothing on disk said reliably which build a running srv is: the srv lock names a pid only, a
  lease names a version only while it holds an agent, and the registry lists only instances with agents. So each
  srv now writes `<data>/srv-features.json` (pid, version, features) at startup beside `srv.lock`. An instance counts
  as new enough when the stamp's pid is the lock's live pid and it lists `former-slugs-v1`. A live srv in the
  registry with no such stamp counts as too old, as does every build before this one.
- **Lock.** `shared/locks/slug-migration.lock`, an OS file lock held for the whole run or rollback.
- **The target, again, before S1.** The plan comes before the backup, and another channel may create an agent
  with a planned id meanwhile. So before an agent's first write the run takes the host-wide slug allocation lock
  (`DefinitionStore::lock_slug_allocation`, which every agent creation holds while it picks and publishes a slug),
  checks the target again by the plan's rules, and holds the lock through S1 and S2, which publish the new id. A
  target taken since leaves that agent out with nothing written for it (`Skipped` in the journal, a note in the
  report); the others still migrate. The lock isn't held during the backup.
- **Backup.** `shared/backups/slug-migration-<UTC time>/` holds:
  - every database, copied through `VACUUM INTO` (`db_copy::copy_database`);
  - the definitions, workspace `.mcp.json` files and stale registrations, as files.

  Each copy's SHA-256 is in the journal header. S8's moves need no copy. The folder is owner-only on Unix. It holds
  keys and credentials, and the report says so.
- **Journal.** `journal.jsonl` records, in order:
  - a header: the plan, the backups, and the run's `retired_at` in epoch ms;
  - per step, an intent with its before-image, then a result;
  - a verification per agent;
  - a finish, with the fingerprints restore mode compares.

  A resume reuses the recorded before-image and `retired_at`, so a step that stopped halfway finishes the same way.
  An apply resumes the latest unfinished run, after re-checking the preconditions.
- **Steps.**
  - S1 rewrites the record's raw JSON under `with_record_locked` (`LockedRecord::replace_raw`, the one write that
    may change a non-empty slug). The former slug is stored lowercased, with the run's `retired_at`.
  - S2 creates `db_agent_former_slugs` where an older version lacks it.
  - S3 and S4 also clear any key row left under the new name, so the next spawn mints fresh keys instead of
    inheriting one. Their undo restores the old name's rows from the backup copy of the same database, and drops
    rows minted under the new name since.
  - S4 deletes the old name's `wan.db` key and publication rows. The account directory's record stays: no client
    call removes one, and the report says so.
  - S6 rewrites cron jobs (`store.db`), open or claimed queue items (`identity-store.db`) and trust grants (each
    `objects.db`).
  - S7 rewrites `AGENTMUX_AGENT_ID`, `MUXBUS_AGENT_ID`, `AGENTMUX_AGENT_SLUG` and the git name and email in the
    agent's panes and every layout snapshot. A pane is matched by its `agentId`, or without one by the old id
    itself; another agent's is never touched.
  - S7 **retargets** the workspace `.mcp.json` instead of deleting it. The agentmux entry signs as the new id with
    the old keys dropped, the user's other servers stay, and the spawn's realignment writes the new keys. A deleted
    file would leave a respawn, which doesn't regenerate it, without AgentMux's tools.
  - S8 deletes only registrations whose srv is gone; the preconditions guarantee there are no others.
- **Verify.** Re-reads K1, K2, K5, K9, K12, K13 and the key tables. Then it resolves the old and the new name
  through srv's own resolver (`Store::former_slug_owner`, `agent_ids_for_slug_folded`) on each `objects.db`. A
  version too old for that query is checked by its rows only.
- **Both ways back refuse first** while an agent the run touched runs anywhere on the host, under its old id or
  its new one (a relaunch takes the new one), or while an AgentMux too old for former slugs runs. Nothing is
  written, the journal included.
- **Rollback, journal mode.** Undoes every recorded intent, newest first, and is resumable.
  - A file still holding what its step wrote gets the backup's bytes back. One edited since is reverted field by
    field (S1) or left as it is and reported (`.mcp.json`).
  - Database undos put back the recorded fields. S7 also puts back each row's version, when nothing else has written
    the row.
- **Rollback, restore mode.** Needs a finished run, and refuses when any database's rows, or any file's bytes or
  modification time, changed since the finish. It copies each database back through SQLite's online backup and
  moves S8's folders back. A database is compared by its rows, not its file: a WAL checkpoint changes the file
  without changing a row.
- **Surfaces.**
  - Settings › Agents, a new section, holds the card: the plan's line, "Update them", the list with "Show where the
    old ids are stored", what blocks it, the report with the PR searches, and "Undo the update".
  - A running pane is stopped only after a confirmation, and the plan is read again before the run.
  - The startup notice stays until it is closed or acted on. Either turns `agent:outdatedidsnotice` off, and it
    doesn't ask again.
- **Phase 3.** `retired_at` is the run's start: the same for every agent of a run, and kept on a resume.

### Phase 3, contract (after the grace period; one small PR or a timer)

- Release each former slug's MuxBus subscription and credential.
- Keep it reserved and tombstoned locally, forever.
- Drop the "delivered via a former name" path only when no former slug has been used for 90 days. Until then it's
  cheap and harmless.

## 5. Details

### 5.1 Which slugs are candidates

A definition is a candidate only if all of these hold:
1. it has a lineage root (follow `parent_id`);
2. its slug equals the **old** rule's output (parent's slug, `-`, the label's slug);
3. it differs from `fork_slug(root.slug, root.name, branch_label)`.

Anything else is left alone and listed as "not a pre-fix chain". This includes a `-N` collision suffix, a
user-visible custom slug, or a missing parent (the old rule can't be recomputed).

If the target is taken by a different uid, the migration refuses that agent and suggests the next free `-N`, never
picks one silently. The other agents can still migrate.

### 5.2 Order

- Agents are migrated parent first (`loap-loap-2` before `loap-loap-2-loap-3`), although targets don't depend on
  each other, because `fork_slug` uses the root.
- Each agent is a unit: its steps all finish, or it's rolled back, before the next one starts.

### 5.3 Other machines

- Each machine has its own definitions, so each runs its own migration.
- The account's relay sees one install drop the old name's lease and take the new name's.
- A second install holding the same slug for a different agent is unaffected: names are per install and channel,
  under the account's lease rules.

## 6. Testing

- **Planner, unit tests:**
  - candidates by §5.1, including the three real cases, a non-candidate `-N` suffix and a missing parent;
  - collision detection across all `objects.db` on disk;
  - "held only by the same uid" counts as free.
- **Each step, unit tests:** apply → state X; apply again → still X (idempotent); its rollback → the before-image,
  byte-identical where the step is a file write.
- **Crash and resume:** fault injection after every step, both before and after its "done" record. Resume, then
  verify. Run it as a table over all steps.
- **Fixture home:** two channels, each with two versions, holding the three agents. The tests cover:
  - stale `cmd:env`;
  - a name-only cron job and a trust grant;
  - LAN and WAN key rows;
  - a `gh-<slug>` folder.

  Plan, apply, verify; then roll back and compare checksums to the original.
- **Phase 1 resolution:** a message to a former slug is delivered; a message signed as a former slug is refused; a
  new fork can't take a former slug; the LAN lookup of a former slug returns the current name.
- **Phase 0:**
  - a stale K1 can't overwrite K2's slug;
  - a persisted `cmd:env` `AGENTMUX_AGENT_ID` loses to the definition on respawn;
  - delete tombstones former slugs.
- **Live, in an isolated `task dev`** (`AGENTMUX_HOME_OVERRIDE`) with a copy of a real home:
  1. migrate the three agents;
  2. relaunch them;
  3. check a signed host message, a LAN message, a WAN message to and from another install, a MuxBus review notice
     addressed to the old name, `gh-agent auth status`, and a commit's `Agent:` trailer.

  Then do it once more on the real home, with the operator.

## 7. Risks

| Risk | Mitigation |
|---|---|
| An unmigrated copy of the slug writes the old one back | S2 migrates every `objects.db` on disk; P0.2 refuses a write-back of a former slug |
| A running agent keeps the old id until relaunched | preconditions refuse while it runs; P0.3 makes respawn take the definition's id |
| Signed messages fail between the rename and the next spawn | the agent isn't running then; keys are minted fresh at its spawn |
| A review notice for an old PR is lost | grace subscription; the report lists PRs to edit |
| An old portable build of AgentMux started later | it reads the migrated `objects.db` of its own version; a version folder created later starts from the shared definition, which has the new slug |
| A bug in one step | per-agent units, verify before the next agent, journal rollback, backups |
| The migration is run twice, or two instances start it | a host-global lock, and an idempotent planner (a migrated agent is no longer a candidate) |

## 8. Plan

| PR | Scope |
|---|---|
| 1 | Phase 0: one reader of the slug, no stale write-back, the server wins on spawn identity, delete covers former slugs |
| 2 | Phase 1: `former_slugs` storage, resolution, reservation, LAN, the grace subscription |
| 3 | Phase 2: planner, preconditions, backup, journaled steps, verify, rollback; RPC, CLI and the Settings card |
| 4 | Phase 3: contract timer |
| cloud | A relay-side alias, so a former name needs no subscription (separate, optional) |

PRs 1 and 2 are useful even if PR 3 never ships: they remove most of the two-names failure modes for every agent.

## 9. Decisions

Settled by the operator on 2026-10-22 ("proceed with best recommendations"):

1. **Grace period: 30 days after the rename.** srv can't see GitHub, so it doesn't try to extend the period while a
   PR is open. Instead, the migration's report lists every open PR whose body carries the old `agentmux:agent_id`,
   for the agent to update with `gh-agent pr edit`. Phase 1's grace subscription covers the rest.
2. **Keys: rotate.** Identity follows the name: S3 retires the old name's keys and fresh ones are minted under the
   new name at the next spawn. LAN peers re-pin on first use.
3. **Where it runs: the Settings › Agents card**, plus a one-time, dismissible notice pointing to it when candidates
   exist. No prompt at startup; nothing runs without the operator starting it.
4. **Uid as the address: after this lands**, as its own spec.
