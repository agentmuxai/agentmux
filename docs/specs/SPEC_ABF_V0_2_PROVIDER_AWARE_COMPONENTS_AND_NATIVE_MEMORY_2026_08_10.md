# Spec: ABF v0.2 — Provider-Aware Components + Native Memory

**Date:** 2026-08-10
**Status:** proposal — no implementation yet. Revised after Codex review on
PR #2517 found four real implementability gaps in the first draft (all
credited inline below, matching this repo's own `Codex P1, PR #NNNN`
citation convention) — not disputed, all four were correct and are folded
into the design as shipped in this revision.
**Relationship to prior work:** builds on
`docs/specs/REPORT_ARMORY_BUNDLE_STANDARD_RESEARCH_2026_07_16.md` (the
original ABF proposal, §5) and
`docs/specs/SPEC_ABF_V0_1_SINGLE_FILE_AND_IMPORTER_2026_08_01.md` (the
shipped v0.1 exporter/importer, `.abf` packaging decision). Does not
re-litigate what those got right — the composition-not-invention posture,
the credential-requirements-not-secrets design, `.abf` zip packaging all
stand unchanged. This spec narrows in on three concrete gaps found while
auditing the real v0.1 implementation against the goal "ABF should be
everything a user needs to bootstrap an agent," a goal restated directly in
this session's design discussion.

---

## 0. Summary

Three gaps, found by reading the shipped code (`bundle_export.rs`,
`bundle_import.rs`, `agent_config.rs`) against the original spec and against
what a genuinely portable agent bundle needs:

1. **"Provider" already means two unrelated things inside `armory.json`**,
   including in the *original spec's own examples* — a top-level
   harness/model hint (never implemented) and a per-requirement credential
   provider (implemented, shipped). Same field name, disjoint meaning, same
   document.
2. **No component in ABF can vary by harness.** Exactly one instructions
   file, one skill format, ship regardless of which CLI will run the bundle
   — and this isn't only an ABF gap, the live runtime has the identical
   blind spot.
3. **Native memory — the thing an agent autonomously writes and accumulates
   across sessions — is not in ABF at all**, despite AgentMux already having
   a normalized, durable, per-agent table for exactly this data, and despite
   the original spec's manifest being deliberately left open-ended to add it
   later.

None of these require new research into external standards — §3f of the
original report already established there's nothing to converge with on
memory, and no format anywhere versions per-harness content the way this
proposes. Both are AgentMux-native design decisions, same posture as the
original report's own credential-requirements schema.

---

## 1. Problem, precisely

### 1.1 The "provider" naming collision

The original report's own manifest example (§5.2) declares:

```jsonc
"provider": { "preferred": "claude", "model": "claude-sonnet-5" },  // hint, not constraint
```

— a single, bundle-level *harness* hint. Its `accounts/requirements.json`
example (§5.3) separately declares:

```jsonc
{ "id": "gh-main", "provider": "github", "kind": "api-key | oauth", ... }
```

— a per-requirement *credential/account* provider (`"github"`, matching
`db_accounts.provider`). Same key name, two disjoint dimensions, in the same
manifest design.

The shipped code resolved this ambiguity by accident, not decision: the
top-level harness hint was **never implemented** (`bundle_export.rs`'s
actual manifest has exactly `$schema`, `name`, `version`, `description`,
`components`, `metadata` — no `provider` key). The only `"provider"` that
exists in a real, exported `armory.json` today is the credential sense, at
`bundle_export.rs:500`, written into each `accounts/requirements.json`
entry. So the collision is latent, not yet actively confusing anyone — but
it's baked into both the spec text and the schema's implicit contract, and
this spec's own §2.2 is about to give the harness sense real, exported
content for the first time. Landing that under the same key name a
requirement entry already uses for something else would make the collision
real.

This is a *third*, unrelated "provider": AgentMux's own harness registry
(`agentmux-srv/src/backend/providers.rs`, the `ProviderConfig` struct) has
nine canonical IDs — `claude`, `codex`, `gemini`, `qwen`, `kimi`,
`openclaw`, `pi`, `copilot`, `muxcode` — each with its own controller type,
auth directory convention, and (per this session's earlier
harness/model-vendor decoupling work) an independent `base_url_env_var` for
which *model vendor* actually serves it. None of ABF's two existing
"provider" senses refer to this registry at all.

### 1.2 No provider-scoped components — and the runtime has the same gap

`bundle_export.rs` writes exactly one `instructions/AGENTS.md` and one
`skills/<slug>/SKILL.md` per skill, unconditionally. There is no way for a
bundle to say "this instruction file is for Claude Code, this other one is
for Codex." `db_bundles.provider` (a real column) is never read by the
exporter at all.

This isn't unique to ABF. `agent_config.rs`'s `build_config_files()` —
the function that actually materializes instructions into a launched
agent's working directory — takes no `provider` parameter and
unconditionally writes `CLAUDE.md` (`agent_config.rs:54-117`, filename
hardcoded at line 114), regardless of which harness is about to read it.
A Codex or Gemini CLI agent gets a `CLAUDE.md` written to its working
directory today, not the harness's own native convention.

Framed against the original research: ABF's choice of `AGENTS.md` as its
canonical instructions filename (§3c: AGENTS.md is the Agentic AI
Foundation-governed, ~60,000-project-adopted convention, natively read by
Codex, Gemini CLI, GitHub Copilot, and others) is actually the *better*
default of the two — better than the runtime's own CLAUDE.md-always
behavior. But neither layer lets a bundle carry harness-specific variants
where instructions genuinely need to diverge (a Claude-specific
slash-command reference, a Codex approval-mode note, anything that only
makes sense for one controller type).

### 1.3 Native memory is unbundled despite being bundle-able

The original report's §5.5 deferred dynamic memory as an explicit
non-goal, reasoning that §3f found no external standard and no convergent
non-standard to align against — Letta's memory blocks, Zep's knowledge
graph, and LangMem's format-less delegation are mutually incompatible, and
none is worth adopting wholesale. That reasoning is sound and unchanged by
this spec.

It does not, however, block bundling AgentMux's *own* representation.
`docs/specs/SPEC_NATIVE_MEMORY_DURABLE_SYNC_2026_08_07.md` shipped
`db_agent_native_memory(agent_id, filename, content, metadata_type,
size_bytes, updated_at, last_seen_path)` — a durable, per-agent,
per-file mirror of exactly the memory files an agent autonomously writes
(the same mechanism, in fact, that this agent instance uses right now to
persist session memory). This data already exists in a normalized,
queryable shape; nothing needs to be invented to bundle it. `bundle_export.
rs`/`bundle_import.rs` have zero references to `native_memory` or `brain`
today — confirmed by direct grep, not an oversight the original spec
flagged, simply not built yet.

The original manifest's `components` object was deliberately left
open-ended for exactly this ("a future `\"memory\"` component key can be
added without a breaking manifest version bump" — report §5.5). This spec
exercises that reserved extension point.

---

## 2. Design

### 2.1 Resolve the naming collision: rename the credential-provider field

Rename `accounts/requirements.json`'s per-requirement `"provider"` key to
`"credentialProvider"`. This is a breaking field-rename against the v0.1
schema, but v0.1 has shipped with no external bundle producers or consumers
— export/import exist only as internal RPCs with no UI surface reachable
outside this repo as of `SPEC_ABF_IMPORT_UI_PHASE3_2026_08_02.md`'s own
status line. A rename now costs nothing; the same rename after a real
external `.abf` file exists in the wild would require a compatibility
shim. Reserve the bare `"provider"` key exclusively for the harness sense
from this point on — §2.2 gives it real content.

### 2.2 Provider-scoped components

Change `components.instructions` from a flat path array to a structure
keyed by provider ID (AgentMux's own canonical `ProviderConfig.id` values),
with a reserved `"default"` key for provider-agnostic content:

```jsonc
"components": {
  "instructions": {
    "default": ["instructions/AGENTS.md"],
    "claude": ["instructions/claude/CLAUDE.overrides.md"]
  },
  "skills": ["skills/deploy-checklist"],
  "mcpServers": ["mcp/github.server.json"],
  "accounts": "accounts/requirements.json"
}
```

`skills` and `mcpServers` stay flat arrays for v0.2 — no evidence yet that
either needs to diverge per harness, and widening scope to components that
don't need it repeats the mistake this spec is fixing (inventing structure
ahead of a real need). Revisit if that changes.

**Revision note (Codex P1, PR #2517, ×2 findings):** the original draft of
this section put the default+provider merge at *import* time, keyed off
`db_bundles.provider`. Both premises were wrong, for the same underlying
reason: **a bundle is reusable across agents on different providers, and
has no provider of its own** — confirmed by reading
`frontend/app/view/memory/memory-model.ts:100-112`'s `draftToWire()`,
which *deliberately* writes `provider: ""`/`model: ""` on every save,
citing its own doc comment: "provider/model are deprecated on presets
(provider-agnostic, §4.1a). Write empty so the ON CONFLICT update clears
any stale legacy value." This isn't an unused column waiting to be
repurposed (what the original draft assumed) — it's actively, deliberately
scrubbed empty by the shipped UI on every single save, matching this
repo's own documented architecture principle
(`CLAUDE.md`: "a bundle... is the agent's provider-agnostic config
collection... NOT provider/model; those belong to the agent"). Building
§2.2 on top of a column the codebase is actively erasing was a real design
error, not a minor gap. Corrected design follows.

**Storage:** add a new `db_bundles` column, `instructions_by_provider`
(JSON `{provider_id: content}`, same encoding pattern already used for
`context_files`/`mcp_servers` on this exact table). This is deliberately
**not** the deprecated `provider`/`model` columns — those stay exactly as
deprecated as they are today, untouched by this spec. The existing flat
`instructions` column keeps meaning "default," unchanged.

**Authoring:** the Armory bundle editor (`memory-model.ts` /
`MemoryDraft`) needs a new `instructions_by_provider: Array<{provider:
string; content: string}>` field on `MemoryDraft`, mirroring
`context_files`'s existing `Array<{path: string; content: string}>` shape
(`memory-model.ts:37-38`) — same list-add/list-remove editing pattern
already built for that field, with `provider` as a dropdown constrained to
`providers.rs`'s canonical IDs instead of a free-text path. `draftToWire`
serializes it to the new `instructions_by_provider` JSON column the same
way it already serializes `context_files` (`memory-model.ts:112-113`).
This reuses an existing, shipped UI pattern rather than inventing a new
one — the remaining work is wiring a new list section into the bundle
editor's existing form, not designing new interaction patterns.

**Export:** `export_bundle()` writes the flat `instructions` to
`instructions/AGENTS.md` (default, unchanged from v0.1) and one
`instructions/<provider>/AGENTS.md` file per populated key in
`instructions_by_provider`, listed under that provider's key in the
manifest.

**Import:** `parse_bundle_import` stores each variant verbatim into
`instructions_by_provider` — **no merge decision at import time at all.**
This directly resolves the reused-bundle problem: the bundle keeps every
variant it shipped with, undecided, because import time is exactly the
moment it's least knowable which agent(s) will eventually launch against
it.

**Revision note (Codex P1 ×2, PR #2517, second round):** the "launch-time
merge in `build_config_files()`" design above was itself wrong, for a
reason bigger than a missing parameter. Two things I hadn't checked:

1. `build_config_files()`'s actual caller
   (`agent_open.rs:653-707`) assembles its `content_map` from already-
   resolved `AgentContent` rows (`content_type` → content, e.g. `"soul"`,
   `"agentmd"`), not live from `db_bundles`. Whatever flattening happens
   from a bundle into an agent's own content already happened earlier, at
   bundle-attach time — there is no bundle value in scope at launch to
   call `.get(provider)` on in the first place.
2. More fundamentally: `build_config_files()` is **Claude-only**, by
   explicit prior design decision, not an oversight this spec can patch.
   `SPEC_CODEX_PROVIDER_INTEGRATION_2026_08_08.md:438-443` states this
   directly: *"The current `build_config_files()` always creates
   Claude-native files including `CLAUDE.md`... Writing those for a Codex
   definition produces a successful filesystem operation but does not
   configure Codex."* That spec's own §10.2 gives Codex a **completely
   separate** delivery mechanism — compiled `developer_instructions` in an
   AgentMux-owned TOML profile (`$CODEX_HOME/agentmux-<agent-id>.
   config.toml`, loaded via `--profile`), not a file write into the
   working directory at all. Every other provider will need its own
   analogous materializer as it's onboarded. There is no single function
   this spec can extend to deliver a provider-scoped variant to *every*
   provider — that's a per-provider materializer architecture already in
   progress separately, and re-designing it here would mean silently
   redeciding a 400-line spec I hadn't read before proposing this section.

**Corrected scope: ABF stores and exports/imports provider-scoped
instruction data; delivering a selected variant into a running agent is
each provider's own materializer's job, not this spec's.** §2.2 commits to
the storage shape (`instructions_by_provider`), the authoring UI, and
export/import — full stop. Wiring `instructions_by_provider.get(provider)`
into any specific materializer (Claude's `build_config_files()`, Codex's
TOML-profile compiler, or any future provider's own mechanism) is
explicitly **out of scope**, moved to §3's non-goals. Until that wiring
exists, a v0.2 bundle's provider-specific variants are exportable and
importable but not yet consumed by any running agent — an honest, partial
step (the storage/interchange half of the feature) rather than a claim
this spec doesn't back up.

**Backward compatibility:** a v0.1 bundle (or a v0.2 bundle with no
`instructions_by_provider` entries) behaves exactly as today in every
respect — nothing about existing export/import/launch behavior changes
until a materializer is separately built to consume the new data.

### 2.3 Native memory as a new component

Add `components.memory`, pointing at `memory/<filename>` files sourced
directly from `db_agent_native_memory` rows:

```jsonc
"components": {
  ...
  "memory": ["memory/MEMORY.md", "memory/user_role.md"]
}
```

This needs an `agent_id`, not just a `bundle_id` — bundles are reusable
across many agents by design (the same constraint `bundle_export.rs`'s own
doc comment already documents for why `accounts/requirements.json` is
inferred abstractly rather than agent-scoped), but native memory is
inherently per-agent, not per-bundle. Two options:

- **(a) Explicit agent-scoped export.** A new/extended entry point —
  `bundle.export_for_agent(bundle_id, agent_id)` — that pulls the bundle's
  normal components plus a snapshot of that one agent's
  `db_agent_native_memory` rows at export time.
- **(b) Implicit "currently attached agent."** Silently snapshot whichever
  agent the bundle happens to be attached to when export runs.

**Recommend (a).** A bundle is explicitly reusable-across-agents
(`db_bundles` has no agent FK); (b) would silently bind memory to
whichever agent was attached *at that moment*, producing stale or simply
wrong content on a bundle's next export once it's reattached elsewhere,
with no signal to the user that happened. (a) makes the scoping decision
visible in the RPC call itself.

**Revision note (Codex P1, PR #2517, second round): the mirror can be
stale at export time.** `db_agent_native_memory` only gets refreshed by
the Stash Memory tab's own `list`/`read_file`/`write_file` RPCs
(`SPEC_NATIVE_MEMORY_DURABLE_SYNC_2026_08_07.md` §2.2's write-through-on-
read design) — a file Claude wrote autonomously since the tab was last
opened (or never opened at all) sits live on disk but isn't in the mirror
yet. `bundle.export_for_agent` has exactly what those RPCs have — the
agent's own `config_dir`/`working_directory` — so before reading
`db_agent_native_memory` for the snapshot, it must run the same
live-FS-read-then-upsert pass `agent:memory:list` already performs
(`native_memory_handlers.rs` list handler), not read the mirror cold.
Reuses existing logic; doesn't invent new sync behavior.

**Revision note (Codex P1 ×2, PR #2517):** the original draft assumed an
"importing agent" exists during `bundle.import`/`bundle.import.commit` and
that inserting into `db_agent_native_memory` alone was sufficient. Both
were wrong. `bundle_import_commit_impl` creates only a `db_bundles` row —
its request carries no `agent_id`, so there is no valid FK target for a
memory row in the generic import flow at all. Separately,
`SPEC_NATIVE_MEMORY_DURABLE_SYNC_2026_08_07.md` §2.2 (cited earlier in
this same spec, missed on the first pass) is explicit that the durable
table is a **fallback mirror consulted only by the Stash UI's own RPCs**
— the harness process itself reads the live filesystem directly and never
consults this table. Writing only to `db_agent_native_memory` on import
would make imported memory visible in Stash while remaining completely
invisible to the agent that's supposed to have it, until something else
happened to rewrite the live files. Corrected design follows.

**Import requires its own agent-scoped entry point.** Mirroring §2.3's
export-side `bundle.export_for_agent`, add `bundle.import_for_agent
(files_or_zip, agent_id)` — the *only* path that processes a
`components.memory` key. The generic, agent-less `bundle.import` continues
to create only the reusable `db_bundles`/skills/MCP rows as it does today;
if its manifest contains a `memory` component, it's skipped with an
explicit warning in the response ("memory requires an agent-scoped
import"), never silently dropped.

**Import must write through to the live filesystem, not just the mirror.**
`bundle.import_for_agent` writes each memory file through the *same* path
`native_memory_handlers.rs`'s existing `write_file` RPC handler already
uses: resolve `memory_dir_for_cwd(...)` for the target agent, write the
file to that live path, **then** upsert into `db_agent_native_memory` —
reusing that handler's existing dual-write logic directly (line ~647-689)
rather than reimplementing it, so imported memory gets durability and
location-consistency for free, and — critically — is actually on disk
where the harness will read it, not only in the mirror table.

**Revision note (Codex P1, PR #2517, second round): a same-named file at
an existing target silently destroys the agent's own memory.** Nothing in
the original draft stopped `bundle.import_for_agent` from targeting an
agent that already has its own `MEMORY.md` — the reused write-handler
logic overwrites on rename, and the mirror upsert overwrites on conflict,
with no preview or choice. **Scope narrowed for v0.2: `bundle.
import_for_agent` only accepts a target agent with zero existing
`db_agent_native_memory` rows** (checked before any write; a non-empty
target is rejected outright with a clear error, not partially imported).
This is the simplest rule that can't destroy existing memory, at the cost
of not supporting "merge memory into an agent that already has some" —
real skip/rename/replace semantics are a legitimate follow-up if that
turns out to matter, flagged explicitly rather than designed here
(matches this repo's own "explicit follow-up, not guessed at" convention
— see `SPEC_NATIVE_MEMORY_DURABLE_SYNC_2026_08_07.md` §4's own non-goals
list for the precedent).

### 2.4 Manifest version

`$schema` becomes `.../v0.2/bundle.schema.json`; `compatibility.agentmux`
minimum bumped to whatever release ships this. Per
`SPEC_ABF_V0_1_SINGLE_FILE_AND_IMPORTER_2026_08_01.md` §4.3.2, `$schema`/
`version` are read but not a hard gate — same tolerance extends to v0.2:
an importer accepts both v0.1 (flat instructions array, `provider` meaning
credential-provider) and v0.2 (keyed instructions object,
`credentialProvider`) manifests without erroring.

---

## 3. Non-goals

- **Wiring `instructions_by_provider` into any provider's launch-time
  materializer** — including Claude's own `build_config_files()`. §2.2's
  second revision narrowed this deliberately: `build_config_files()` is
  Claude-only by prior design, doesn't currently receive bundle content at
  all (its `content_map` comes from pre-resolved `AgentContent` rows, not
  live `db_bundles` reads — `agent_open.rs:653-707`), and
  `SPEC_CODEX_PROVIDER_INTEGRATION_2026_08_08.md` §10.2 gives Codex a
  structurally different delivery mechanism entirely (a compiled
  `developer_instructions` TOML profile, not a file write). This spec
  ships the storage/authoring/export/import half of provider-scoped
  instructions; actually delivering a selected variant into a running
  agent is separate, per-provider materializer work, tracked as follow-up
  rather than designed here.
- **Any change to credential/account handling.** `SecretRef`-style
  declare-don't-bundle stays exactly as designed; §2.1's rename is
  cosmetic (field name only), not a design change.
- **OCI distribution.** Still deferred per the original report's §5.4;
  nothing here blocks or accelerates it.
- **Cross-harness instruction translation.** Provider-scoping (§2.2) lets a
  bundle *declare* per-provider content; auto-converting a Claude-specific
  instruction into Codex's dialect is not attempted and not proposed.
- **Skill/MCP-server provider-scoping.** Deferred per §2.2 until a real
  need is observed, not speculated ahead of one.

---

## 4. Rollout

- **Migration required** (revised from the original "no backfill needed"
  claim, which only held under the incorrect import-time-merge design):
  add the `instructions_by_provider` JSON column to `db_bundles`
  (`NOT NULL DEFAULT '{}'`, matching the empty-object-sentinel convention
  `context_files`/`mcp_servers` already use on this table). Every existing
  row gets the default empty object — behaviorally identical to today
  until an author actually adds a variant through the new UI affordance
  (§2.2).
- `db_bundles.provider`/`.model` stay exactly as deprecated as they are
  today (`memory-model.ts::draftToWire` continues zeroing them on every
  save) — this spec does not touch, read, or resurrect either column.
- `db_agent_native_memory` already exists and is already populated
  organically per `SPEC_NATIVE_MEMORY_DURABLE_SYNC_2026_08_07.md`'s
  write-through-on-read design, so `bundle.export_for_agent` (§2.3) has
  data to export from day one for any agent whose Stash Memory tab has
  been opened at least once — this part of the original rollout claim
  still holds; only the instructions-column claim needed correcting.

## 5. Test plan

- Unit: `parse_bundle_import` accepts both the v0.1 flat-array
  `instructions` shape and the v0.2 keyed-object shape; a v0.1 array
  imports identically to an equivalent v0.2 `{"default": [...]}` object,
  with `instructions_by_provider` left at its default empty object in
  both cases (no merge decision made at import time — §2.2).
- Unit: `export_bundle()` emits one `instructions/<provider>/AGENTS.md`
  file per populated `instructions_by_provider` key, alongside the
  existing default `instructions/AGENTS.md`; a bundle with no variants
  exports identically to a v0.1 bundle (regression check against existing
  v0.1 export tests).
- Unit: `credentialProvider` round-trips through export → import
  unchanged; a v0.1 bundle still carrying the old `provider` key in
  `accounts/requirements.json` is still accepted on import (read as
  `credentialProvider`'s predecessor name, not rejected).
- Unit: `bundle.export_for_agent` — memory files present for an agent with
  populated `db_agent_native_memory` rows; empty `components.memory`
  omitted entirely (not an empty array) for an agent with none, matching
  the existing omit-empty-components convention already used for
  `skills`/`mcpServers`/`accounts`.
- Unit: `bundle.import` (the generic, agent-less path) returns an explicit
  warning and skips a `memory` component if one is present in the
  manifest, rather than silently dropping or erroring.
- Unit: `bundle.import_for_agent` — an imported memory file exists at the
  target agent's live `memory_dir_for_cwd(...)` path AND in
  `db_agent_native_memory` after import (both halves of the dual-write,
  not just the mirror).
- Unit: `bundle.import_for_agent` rejects a target agent that already has
  at least one `db_agent_native_memory` row, with no partial write of any
  imported file (all-or-nothing on the pre-check, not per-file).
- Unit: `bundle.export_for_agent` refreshes the mirror from the live
  filesystem before snapshotting — a file present on disk but not yet in
  `db_agent_native_memory` (simulating "written since the Memory tab was
  last opened") is included in the export.
- Integration: full export → import round trip for a bundle with
  provider-scoped instructions AND memory components, landing in a fresh
  target instance: export via `bundle.export_for_agent` from a source
  agent, import via `bundle.import_for_agent` into a fresh (empty-memory)
  target agent, and confirm (a) `armory.json`'s `components.instructions`
  carries both the default and provider-keyed entries and (b) the target
  agent's Stash Memory tab shows the imported memory files. Does **not**
  cover launch-time materialization of the provider-scoped variant — that
  consumption path doesn't exist yet per §3's non-goals.
