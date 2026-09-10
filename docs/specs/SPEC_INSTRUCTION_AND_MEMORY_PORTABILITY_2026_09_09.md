# Spec: Instruction and Memory Portability

**Status:** active — Phases 1, 2 and 3 landed (#3147, #3162, #3163). Phase 0b landed its reconciliation (#3149, #3152, #3153) but is **not closed**: retiring the redundant inline columns turned out to be blocked by a channel-and-version asymmetry (§3.4a), and Phase 4 is untouched. Tracking: #3148.
**Date:** 2026-09-09
**Verified against:** `a86cdee50` (code, not spec prose)
**Follows:** `SPEC_ARMORY_NAMING_CONSOLIDATION_2026_09_09.md` §8, which deferred
these three requirements so naming could land first (Phases 1–2 are on main)
**Related:** #2024 (tracking), `SPEC_ABF_V0_2_PROVIDER_AWARE_COMPONENTS_AND_NATIVE_MEMORY_2026_08_10.md`
(the format this extends), `SPEC_CLAUDE_MD_OWNERSHIP_PROTECTION_2026_08_22.md`
(the ownership rules §2.4 depends on), `SPEC_NATIVE_MEMORY_DURABLE_SYNC_2026_08_07.md`
(the invariant §5.3 extends)

---

## 1. The requirement

> "essentially we want tight control over anything the agent uses as
> instructions and memory including portability"

Three concrete asks came with it, agreed 2026-09-09:

1. **An exported bundle carries the agent's memory.**
2. **Memory is written to one AgentMux-controlled place**, not wherever a
   provider's own path scheme happens to put it.
3. **Files the agent reads as instructions but AgentMux does not own** — the
   repo's own `CLAUDE.md`, `AGENTS.md` and friends — are tracked and readable.
   Tracking only. AgentMux never writes them back.

"Tight control" is doing real work in that sentence. It does not mean AgentMux
owns every file. It means: for every byte that reaches an agent as instructions
or memory, AgentMux can answer *where did this come from*, *what is in it right
now*, and *can I move it to another machine*. Today it can answer all three for
some of those bytes and none of them for the rest. §2 establishes which is
which; §3 names the gaps; §5 closes them.

## 2. What exists today

Verified against `a86cdee50`. This section is descriptive — no proposals in it.

### 2.1 Three export paths, three different payloads

| RPC | Handler | Carries |
|---|---|---|
| `bundle.export` (`commands.rs:397`) | closure in `register_bundle_export`, `app_api/bundle.rs:342` | bundle row + resolved skills. **No agent, so no memory.** |
| `bundle.export_for_agent` (`commands.rs:414`) | `bundle_export_for_agent_impl`, `app_api/bundle.rs:598` | the above, **plus** the agent's native memory (`app_api/bundle.rs:590`) |
| `bundle.export_for_agent_with_history` (`commands.rs:423`) | `bundle_export_for_agent_with_history_impl`, `app_api/bundle.rs:721` | the above, **plus** transcripts under `history/<provider>/<session_id>.jsonl` (`app_api/bundle.rs:797`) |

Memory is spliced by `splice_memory_component` (`app_api/bundle.rs:439`): each
file is written to `memory/<filename>` and the list is registered at
`app_api/bundle.rs:468`. It is a no-op when the agent has no memory
(`app_api/bundle.rs:443`), so an agent-scoped export of an empty-memory agent is
byte-identical to the agent-less one.

**So "export carries memory" is already true for the agent-scoped path.** The
naming spec's §8 recorded this after an earlier draft claimed otherwise. What
remains is not implementation but *discoverability* — see §3.1.

**No frontend calls any export RPC.** `frontend/app/store/rpc-api/bundle.ts`
exposes import only (`bundle.import.preview` at `bundle.ts:103`,
`bundle.import.commit` at `bundle.ts:119`). Export is reachable from the App API
and from agents, not from the Armory UI. Any UI work in §7 starts from zero.

### 2.2 What an ABF actually contains

There is no manifest struct and no component-key constant. The manifest is an
inline `json!` literal at `bundle_export.rs:615`, and `components` is a
`serde_json::Map` built imperatively above it:

| Key | Written at | Content |
|---|---|---|
| `instructions` | `bundle_export.rs:594` | keyed object (`default` + provider keys) since v0.2 |
| `skills` | `bundle_export.rs:597` | directory paths, one per skill |
| `mcpServers` | `bundle_export.rs:600` | one redacted `.server.json` per server |
| `accounts` | `bundle_export.rs:604` | fixed `accounts/requirements.json` |
| `memory` | `app_api/bundle.rs:468` | file list, spliced after the fact |

**Every key is optional and `components` may legitimately be `{}`.** Absence of
a key is therefore indistinguishable from "this bundle had none" — which is
exactly the property §3.1 turns into a bug and §5.1 fixes.

`components.memory` is deliberately not built in `bundle_export.rs`: that module
is agent-unaware by design, and the boundary is documented at
`app_api/bundle.rs:427-438`. §5 preserves that split.

Context files are a **copy, not a reference**. `Bundle.context_files`
(`storage/bundles.rs:52`) stores `[{path, content}]` inline; export writes the
stored `content` verbatim and uses `path` only to derive an archive filename
(`bundle_export.rs:460-480`). Nothing re-reads the original file from disk,
ever. Skills and MCP servers are likewise inlined, not linked
(`bundle_export.rs:510`, `bundle_export.rs:539`).

### 2.3 Memory: on disk, mirrored in SQLite

The live files are the source of truth for current content
(`native_memory_handlers.rs:1003`; the precedence test is
`native_memory_handlers.rs:1773`). `db_agent_native_memory`
(`migrations.rs:791`) is a durable **mirror** so a file written under one
channel stays visible from another; `db_agent_native_memory_versions`
(`migrations.rs:809`) is append-only history and is never read on the
`list`/`read_file` hot path (`storage/agent_native_memory_versions.rs:11-13`).

The location is *derived*, not owned:
`memory_dir_for_cwd(claude_config_dir, working_directory)`
(`native_memory_handlers.rs:65`) produces
`<base>/projects/<sanitized-cwd>/memory`, where `<base>` is `CLAUDE_CONFIG_DIR`
or `~/.agentmux/shared/providers/claude`. `memory_dir_for_agent_by_id`
(`native_memory_handlers.rs:628`) picks that or the blank-workdir variant
(`native_memory_handlers.rs:263`).

`SPEC_NATIVE_MEMORY_DURABLE_SYNC_2026_08_07.md` (Status: implemented, PR #2459)
already fixed the *consequence* of that derivation, stating the invariant as
"no data loss, no silent 'this agent has no memory' when it actually does, and
zero user-visible awareness of `CLAUDE_CONFIG_DIR`/cwd-hash mechanics" (:21-24).
It deliberately left the mechanism open. §5.3 closes the mechanism.

Export reads the **mirror**, not the filesystem (`app_api/bundle.rs:575-589`),
which is only correct because it refreshes the mirror first
(`app_api/bundle.rs:558`). That ordering is load-bearing and undocumented at the
call site.

### 2.4 Project instructions: written, or refused — never tracked

`write_claude_md_respecting_ownership` (`agent_config.rs:1116`) decides
ownership by prefix (`agent_config.rs:1131`): a `CLAUDE.md` starting with
`CLAUDE_MD_MANAGED_MARKER` (`agent_config.rs:904`) is AgentMux's to rewrite. A
foreign one is never overwritten. Instead the generated body goes to
`.claude/AGENTMUX_MEMORY.md` (`agent_config.rs:909`) and an `@`-import line is
offered exactly once per working directory, guarded by an atomic marker file.
Unreadable is treated as foreign, never as absent (`agent_config.rs:1122-1124`).
For non-Claude providers, `write_startup_instructions_respecting_existing`
(`agent_config.rs:1072`) has no import mechanism at all and simply **skips
delivery** on a foreign file (`agent_config.rs:1101-1108`).

**Nothing reads a foreign instruction file for any purpose other than that
branch decision, and nothing records anything about it.** The only persisted
state is `ClaudeMdOwnershipMarker` (`agent_config.rs:934`), a single boolean,
`import_line_offered` (`agent_config.rs:935`) — no hash, no mtime, no path, no
content. No bundle column holds one (`storage/bundles.rs:25-83`).

The near miss is `getclaudeglobalconfig` (`agent_handlers/bundle.rs:236`), which
reads the *provider config dir*'s `CLAUDE.md`, not the agent's working
directory. Its own frontend wrapper says so (`rpc-api/bundle.ts:80-84`), and
`SPEC_SURFACE_CLAUDE_GLOBAL_CONFIG_2026_08_24.md` records that it targeted the
wrong file (:4).

## 3. Gaps

### 3.1 Export asymmetry is silent in the one direction that loses data

Import warns when it drops memory: `MEMORY_COMPONENT_IGNORED_WARNING`
(`bundle_import.rs:432`) is pushed whenever an agent-less import sees
`components.memory` (`bundle_import.rs:909`), and the agent-scoped path filters
it back out (`app_api/bundle.rs:961`). That is the right shape.

Export has no counterpart. `bundle.export` on a bundle bound to an agent with
memory produces a file that looks complete — §2.2 established that a missing
component key is indistinguishable from an empty one — and the operator is told
nothing. The asymmetry is not that one path omits memory; omitting memory from a
bundle detached from any agent is correct. The asymmetry is that **one side
announces the omission and the other does not.**

### 3.2 Project instructions are invisible and unportable

A foreign `CLAUDE.md` can be the majority of what an agent is actually told, and
AgentMux can answer none of §1's three questions about it. It cannot show the
operator what the agent will read, it cannot detect that the file changed under
it, and an export carries no trace of it — so a bundle moved to another machine
silently reconstitutes an agent with different instructions. For non-Claude
providers it is worse: delivery is skipped entirely
(`agent_config.rs:1101-1108`) and nothing surfaces that either.

### 3.3 Memory location is derived from a mutable input

The directory is a function of the working directory
(`native_memory_handlers.rs:65`). Change an agent's working directory and its
memory is, from the resolver's perspective, somewhere else. The durable mirror
(§2.3) is what keeps this from being data loss today — it is a compensating
control for a location that is not owned, not a location guarantee.

### 3.4 Components have two sources of truth; export sees only one

`export_bundle` (`bundle_export.rs:370`) reads the inline JSON columns —
`bundle.context_files` (`bundle_export.rs:449`), `bundle.mcp_servers`
(`bundle_export.rs:526`), and a skills list its caller resolved from
`bundle.skills`. The newer bundle-level ref tables `db_bundle_skills_ref`
(`migrations.rs:742`) and `db_bundle_mcp_ref` (`migrations.rs:749`), described
in the migration as "the new live-resolution path" (`migrations.rs:194-197`),
are read only by `Store::bundle_skill_list` (`storage/skills.rs:495`) and
`Store::bundle_mcp_list` (`storage/mcp_servers.rs:257`), whose only callers are
`app_api/skill.rs:542` and `app_api/mcp.rs:625`.

**Phase 0 answered this, 2026-09-09, and the answer is worse than the question
assumed: there is no sync path, by design, and the divergence runs in both
directions.**

No code path writes both. Binding a skill or server to a bundle
(`skill.catalog.bind_to_bundle`, `mcp.catalog.bind_to_bundle`) reaches
`Store::managed_bind_bundle` (`storage/managed.rs:285`) and issues exactly one
INSERT into the ref table (`storage/managed.rs:298-305`); the handler
(`app_api/skill.rs:513`) never reads or writes the bundle row. Conversely
`bundle.upsert` and the three ABF-import paths write the inline columns
(`storage/bundles.rs:245-246`) and never call `bundle_skill_bind` /
`bundle_mcp_bind`. The split is deliberate and stated in the v23 schema
comment (`migrations.rs:198-199`): the inline columns are "still the .abf
export/import" path while the ref tables are the live-resolution one.

So the two consumers disagree, and each is authoritative for a different thing:

| | ABF export reads | Agent launch reads |
|---|---|---|
| Skills | `bundle.skills` inline (`app_api/bundle.rs:388`) | `effective_skills` → ref table (`app_api/agent_open.rs:793`) |
| MCP servers | `bundle.mcp_servers` inline (`bundle_export.rs:526`) | `effective_mcp_servers` → ref table (`app_api/agent_open.rs:812`) |

Two concrete losses follow:

1. **Bind, then export.** A skill or server added to a bundle in the Armory is
   live at launch and visible in the UI, but the inline column is still `"[]"`,
   so the `.abf` ships with empty `skills/` and `mcp/` directories **and no
   warning**. ABF is advertised as a backup format; this is the same
   silent-incompleteness class the export code already guards against for a
   damaged store (`app_api/bundle.rs:390-395`).
2. **Import, then launch.** `bundle.import` writes inline `mcp_servers` but
   creates no `db_mcp_servers` rows and no ref rows (`app_api/bundle.rs`
   contains zero `mcp_server_upsert*` calls), so imported servers are never
   materialised at spawn — inert at runtime, exactly the state
   `SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md:40-42` describes and the ref
   tables were meant to replace.

That spec kept the inline columns "as-is for this delivery" as an explicit
non-goal (`:91-95`), to be revisited once the ref tables proved out. They have.
Reconciling them is now a prerequisite of §5.4, not a nice-to-have: adding
`components.projectInstructions` to a format that already drops ref-table-bound
skills would ship a second instance of the same bug.

Secondary: `bundle_delete` (`storage/bundles.rs:326`) issues only
`DELETE FROM db_bundles` and there is deliberately no FK from the ref tables
(`migrations.rs:727-741`), so deleting a bundle orphans its ref rows.

### 3.4a The same divergence, along the channel AND version axes

§3.4 found two stores disagreeing about a bundle's components. Phase 0b fixed
that by making `db_bundle_skills_ref` / `db_bundle_mcp_ref` authoritative. That
is true **within one channel-and-version** and false **across** them, which the
original analysis did not draw out:

| Table | Store | Scope |
|---|---|---|
| `db_bundles` + its inline `skills` / `mcp_servers` columns | `~/.agentmux/shared/store.db` | host-global, one file |
| `db_bundle_skills_ref`, `db_bundle_mcp_ref` | `<data_dir>/db/objects.db` | per-channel **and per-version** |

Bundles are shared. Their components are not — and the split is finer than
"per channel". For Installed and Portable runtimes `DataPaths::resolve_internal`
puts `data_dir` under `channels/<ch>/versions/<v>/`
(`agentmux-common/src/data_paths.rs`, `SPEC_VERSION_ISOLATION_2026_06_01.md` §5
Phase 2), and nothing carries the previous version's `objects.db` forward.
Confirmed on disk: `channels/stable/versions/*/data/db/objects.db` is a
separate file per release. So the ordinary upgrade path — one channel, one
user, no dev builds — hits this too. Two consequences:

1. **Binds do not propagate, and do not survive an upgrade.**
   `bundle_skill_bind` writes into the channel store, consulting the shared
   store only to check the bundle exists, and never writes back to the inline
   columns. So a skill bound under one channel-and-version is invisible to
   every other — including the next release on the same channel. Now that
   export reads refs, that is §3.4's divergence reappearing along two more
   axes.

2. **The inline columns are the durable representation.** `m0030` seeds a
   store's ref tables from them at its first boot, and nothing else ever does.
   They are therefore not a redundant duplicate to be deleted; the
   per-channel-per-version refs are a projection of them.

(2) is what blocks retiring the columns: dropping them removes the seeding
source for every store created afterwards — which, given version scoping, means
every future release for every user, not just multi-channel setups. Found by
Codex on PR #3168 (the multi-channel half, then the version boundary) and
confirmed on 26 channels against one shared store.

Mechanism-confirmed but currently latent: on the machine where this was found,
all 68 bundles have empty inline columns and no store has a single ref row, so
nothing is being lost there today. That bounds the urgency — it does not change
the blocker, and it will stop being true the first time anyone binds a skill.

The unblock is to give bundle components a durable home that outlives a channel
and a version. That is not a table move — the ref tables carry foreign keys to
`db_skills` / `db_mcp_servers`, which are scoped the same way — so it reopens
§5.4's "which store owns what" question rather than settling it. Same class as
§5.3's memory-location invariant, and it deserves the same treatment: its own
spec. #3168 is drafted pending that decision.

### 3.5 History files are in the archive but not in the manifest

`export_for_agent_with_history` pushes transcripts into `export.files`
(`app_api/bundle.rs:797`) and reports a count in the RPC response
(`app_api/bundle.rs:819`), but the manifest is never mutated after the memory
splice at `app_api/bundle.rs:590`. A consumer reading `components.*` to learn
what an archive holds does not learn about history at all.

## 4. Target: the Portable Agent Set

The unit that must round-trip is not the bundle. It is everything the agent
consumes:

| Layer | Owner | Portable today | Target |
|---|---|---|---|
| Bundle instructions + context files | AgentMux | yes | yes |
| Skills, MCP servers | AgentMux | inline column only (§3.4) | one source of truth |
| Native memory | agent writes, AgentMux mirrors | agent-scoped export only | + explicit on the agent-less path (§5.1) |
| Session history | provider | archive only, unmanifested (§3.5) | manifested |
| **Project instructions** | **the repo** | **not at all** | **tracked, read-only (§5.2)** |

Round-trip means: export on machine A, import on machine B, and either the agent
is reconstituted with the same inputs or the operator is told precisely which
input could not be carried and why. Silence is the failure mode this spec is
built to remove.

## 5. Design

### 5.1 Symmetry: the agent-less export announces what it dropped

Mirror the import-side constant. When `bundle.export` runs against a bundle that
is bound to an agent which has native memory, push a warning naming
`bundle.export_for_agent` as the path that would carry it. Same shared-constant
discipline as `MEMORY_COMPONENT_IGNORED_WARNING` (`bundle_import.rs:432`), whose
doc comment already explains why the push site and the filter site must not
drift.

Cheap, and it converts §3.1 from a silent loss into an instruction. It does not
change what any path emits, so it is safe to land before the format work.

### 5.2 `components.projectInstructions` — tracked, read-only

**Decided: tracking only.** AgentMux reads, snapshots and exports these files.
It never writes them back, on import or otherwise. This preserves the ownership
protection in `SPEC_CLAUDE_MD_OWNERSHIP_PROTECTION_2026_08_22.md` exactly as it
stands — that spec's whole point is that a foreign file is not ours, and a
portability feature must not become a back door into overwriting one.

Three parts:

**Read.** A resolver that, for a given agent, returns every file the provider
will read as instructions from the working directory.

**That set is not `startup_instructions_filename`.** That field
(`providers.rs:113`) names the single file AgentMux *writes* — the one canonical
target it picked per provider. What a provider *reads* is usually a larger set,
and the gap is already documented in the registry: GitHub Copilot reads
`.github/copilot-instructions.md`, `CLAUDE.md` and `GEMINI.md` in addition to
the `AGENTS.md` that AgentMux chose as its write target
(`providers.rs:501-507`). Resolving only the write target would leave a Copilot
project's actual instructions invisible and unportable — precisely the
round-trip guarantee §4 claims. So the registry needs a second, plural field —
the provider's *native instruction sources* — and the resolver returns the union
of that set with the AgentMux-written target and `.claude/AGENTMUX_MEMORY.md`
where the import-line path is in effect. Populating it per provider is
evidence-gathering in the same shape as
`SPEC_PROVIDER_AWARE_STARTUP_INSTRUCTIONS_2026_08_24.md` §2 already did for the
write targets, and it is a Phase 3 prerequisite. (Codex, PR #3144.)

For each resolved file: path, size, content, a content hash, and whether
AgentMux owns it.

**Ownership is two markers, not one.** `CLAUDE.md` is marked with
`CLAUDE_MD_MANAGED_MARKER` (`agent_config.rs:904`, tested at
`agent_config.rs:1131`); every other startup-instructions file AgentMux writes
is marked with `STARTUP_INSTRUCTIONS_MANAGED_MARKER` (`agent_config.rs:1040`,
tested at `agent_config.rs:1089`). A resolver testing only the first would
report every AgentMux-generated `AGENTS.md`, `GEMINI.md`, `QWEN.md` and
`.pi/APPEND_SYSTEM.md` as `foreign` — inverting the one field §5.2 exists to get
right. The ownership test is therefore a shared helper that knows both marker
classes, reused by the resolver and by both writers rather than reimplemented in
a third place. (Codex, PR #3144.)

**Track.** Persist the hash and the observed-at timestamp per agent, so a
foreign file changing under a running agent is detectable. This is the piece
that has no precedent in the codebase today (§2.4): the existing marker holds
one boolean and nothing else.

**Export.** A new optional `components.projectInstructions` key listing files
written under `instructions/project/<sanitized-path>`, carrying for each entry
the recorded hash and an `owner` discriminator (`agentmux` | `foreign`). On
import the entries are **surfaced, never applied** — the same warning shape as
§5.1, naming what the source machine's agent was reading. An operator who wants
those instructions applies them deliberately; AgentMux does not decide that.

"Surfaced" means *readable*, not merely *announced*: the importer parses the
entries back out (path, hash, owner, content) and returns them on
`bundle.import.preview`. A warning alone would not have been enough, because
`agent.project_instructions` scans the **importing** agent's working directory
and so is structurally unable to show what the source machine held (Codex P1,
PR #3163). Reading is not applying — no import path writes these, and
`bundle.import.commit` has no field that could select them.

The `owner` field is what keeps this honest. A re-import that silently merged a
foreign `CLAUDE.md` into a new machine's repo would be the exact failure the
ownership spec exists to prevent.

### 5.3 Memory location invariant

State it as an invariant, not a path:

> An agent's native memory has exactly one AgentMux-controlled location,
> resolvable from the agent's stable id alone, and unaffected by any change to
> its working directory, its provider, or that provider's own path conventions.

Today's resolver violates the "from the id alone" clause by construction
(`native_memory_handlers.rs:65`). The migration is the interesting part and is
deliberately left to its own spec: the provider's CLI writes into the derived
directory of its own accord, so an AgentMux-owned location needs either a link,
a sync, or provider configuration — and choosing among those needs the same
evidence-gathering the durable-sync spec did. What this spec fixes is that the
invariant was previously implicit; `SPEC_NATIVE_MEMORY_DURABLE_SYNC_2026_08_07.md`
(:21-24) stated durability and location-*consistency* and explicitly left the
mechanism unspecified. Location *ownership* is the successor requirement.

### 5.4 One source of truth for components

Resolve §3.4 before extending the format. Adding `projectInstructions` to a
format that already silently drops ref-table-bound skills would ship a second
instance of the same bug.

**Either option has to close both directions of §3.4, not one.** The two losses
have different shapes and a fix aimed at one leaves the other standing:

- **Ref tables authoritative.** Pointing `export_bundle` at the ref tables fixes
  bind-then-export. It does nothing for import-then-launch: all three import
  paths still write only inline `mcp_servers`, and launch still reads refs, so
  an imported server stays inert. This option is therefore *three* changes, not
  one — export reads refs, **imports create the managed rows and bind them**,
  and existing inline-only data is migrated into refs or it silently disappears
  from every consumer at once. (Codex, PR #3149.)
- **Inline columns authoritative.** The ref tables become a cache, which means a
  sync path on every bind/unbind and a backfill for refs that have no inline
  counterpart today. Launch would have to read through the same resolution, so
  this is the larger change to the hot path.

Recommendation: ref-authoritative, because launch is already the consumer that
matters for correctness at runtime and the ref tables are where the UI writes.
But it is not the cheap option it looks like from the export side alone, and
the migration of existing inline-only data is the part to size first.

### 5.5 Format version debt this inherits

The exporter emits `$schema: .../v0.2/...` (`bundle_export.rs:621`) while
hardcoding `"version": "0.1.0"` (`bundle_export.rs:626`) — those are different
things and the comment at `bundle_export.rs:616-620` says so, but the importer
warns on anything outside `0.1.x`, and `$schema` is never parsed for dispatch.
The naming spec flagged this at its §7.4. **Adding a component key is
backward-compatible in both directions** — old readers ignore an unknown key,
new readers treat absence as "none" — so §5.2 does not force the issue. But
`components.projectInstructions` is the first key whose absence is genuinely
ambiguous (not tracked vs. none found), which is the argument for finally
carrying a real format version. Recommended: bump `$schema` to v0.3 and make the
importer dispatch on it, as a prerequisite of Phase 3, not a side effect.

There is a third piece of the same debt: ABF v0.2 §2.4 also called for a
`compatibility.agentmux` minimum, and the manifest has no `compatibility` field
at all (`bundle_export.rs:615-640`) — found while checking that spec's status
for this one (Codex, PR #3144). A v0.3 bump that adds dispatch should carry it,
since "which AgentMux can read this" is the question a version field exists to
answer and `$schema` alone does not.

## 6. Non-goals

- **Writing project instructions on import.** Explicitly out, permanently — §5.2.
- **Changing what is composed into the startup file, or in what order.** This
  spec adds observation and transport, not composition.
- **Migrating the memory directory.** §5.3 states the invariant; the mechanism
  is its own spec with its own evidence.
- **Mid-session propagation** of bundle or instruction edits to running agents.
- **Retiring the `*memory` RPC aliases** — that is naming-spec Phase 3.
- Session history beyond manifesting what the archive already contains (§3.5).

## 7. Phases

**Phase 0 — DONE (2026-09-09), and it grew.** The investigation found no sync
path and a two-way divergence (§3.4). What it leaves behind is no longer a
question but a fix: reconcile the two stores, or the format grows on top of a
known-lossy base. Still blocks Phase 3.

**Phase 0b — reconcile components (§5.4).** Whichever store wins, the fix has
to close both directions: the ref-authoritative option is export-reads-refs
**plus** imports creating and binding managed rows **plus** a migration of
existing inline-only data, not just the first of those (Codex, PR #3149).
`SPEC_BUNDLE_AS_CONTAINER_V2_2026_08_17.md` §91-95 deferred exactly this
decision and named the trigger — "once the new ref tables have shipped and
proven out" — which has now happened.

**Phase 1 — export symmetry (§5.1).** One constant, one branch, one test. No
format change. Independently shippable and revertible.

**Phase 2 — manifest history (§3.5).** Register the already-written history
files in `components.history`. No new payload, closes a self-description hole.

**Phase 3 — project instructions (§5.2).** Read + track first (useful on its
own: the Armory can finally show an operator what their agent is actually
reading), then export, then the import-side surfacing. Needs Phase 0.

**Phase 4 — memory location (§5.3).** Its own spec. Sequenced last because it is
the only phase that moves data.

Phases 1 and 2 are small enough to land together. Phase 3 is the one with real
design left in it and should not be bundled with them.

## 8. Open decisions

1. **Hash algorithm and storage for §5.2's tracking.** A new table, or a column
   on `db_agents`? Preference: a table keyed `(agent_id, path)`, mirroring the
   shape `db_agent_native_memory` already uses (`migrations.rs:791`), because
   the file set is per-provider and open-ended.
2. **Does tracking poll, or observe at open?** Observing at `agent.open` costs
   nothing and catches the common case. Polling catches edit-during-session and
   costs a watcher. Recommendation: observe at open for Phase 3, revisit only if
   drift-during-session shows up in practice.
3. **Whether §5.5's v0.3 bump lands with Phase 3 or before it.** Recommendation:
   before, so exactly one release carries the dispatch change.
