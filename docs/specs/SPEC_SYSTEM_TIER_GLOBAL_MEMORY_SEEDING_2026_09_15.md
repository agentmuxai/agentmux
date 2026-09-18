# Spec: Operator Config seeding (AgentMux-shipped, system-tier Global Memory)

**Status:** implemented — manifest, seeder module, `bundle_upsert_system_ — #3244
with_version`, and the `bootstrap.rs` startup hook are all in place and
tested. §6's open question (what to do with the Claude-only Provider Config
placeholder) remains genuinely unresolved, deliberately, per that section.
**Date:** 2026-09-15
**Related:** `SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md` (the `is_system`
isolation this spec builds on top of, unweakened), `SPEC_GLOBAL_MEMORY_
UNIFY_SYSTEM_AND_ORDINARY_2026_09_15.md` (the single-list Armory UI this
spec's seeded entries appear inside), `SPEC_AGENT_FACING_GLOBAL_MEMORY_API_
2026_09_15.md` (the `db_bundle_versions` audit trail this spec reuses for
its own overwrite-safety check — see §3.3).

## 0. Taxonomy — naming the four memory/config tiers

This codebase has accumulated four distinct tiers with only some of them
named consistently. Fixing that now, so this spec (and future ones) can use
one term per concept instead of paraphrasing:

| Name | Scope | Authored by | Backing storage | Delivered how |
|---|---|---|---|---|
| **Personal Memory** | one agent only | that agent itself | `db_agent_native_memory` | `MemoryWrite`/`MemoryRead` tools |
| **Global Memory** | every agent, this workspace | human (Armory UI) or agent (`GlobalMemory*` tools) — this deployment's own domain/project knowledge | `db_bundles`, `is_global=1, is_system=0` | `format_global_bundle_block` |
| **Operator Config** | every agent, every workspace | AgentMux itself (shipped, versioned manifest) — universal, not domain-specific | `db_bundles`, `is_global=1, is_system=1` | same `format_global_bundle_block`, alongside Global Memory |
| **Provider Config** | one provider (Claude, Codex, ...), every agent using it | AgentMux seeds an empty placeholder only; not composed by AgentMux | `providers/<name>/CLAUDE.md` etc. | the provider's own native file-discovery — outside `format_global_bundle_block` entirely |

**Operator Config is the human name for what the code today only calls the
`is_system=1` tier** — this spec is the seeding mechanism for it. The
distinction from Global Memory is authorship, not mechanism: both are
`is_global=1` rows composed by the same `format_global_bundle_block`, and
both appear in the same unified Armory list (`SPEC_GLOBAL_MEMORY_UNIFY_
SYSTEM_AND_ORDINARY_2026_09_15.md`). Global Memory is what *this
deployment* knows about *its own domain*; Operator Config is what *AgentMux*
knows about *itself*, and is identical across every deployment until a human
edits it locally. Provider Config sits on a different axis entirely — it
isn't part of AgentMux's own composition at all, just a file AgentMux seeds
for a specific underlying CLI tool to discover on its own.

The rest of this spec uses "Operator Config" throughout in place of the
earlier working phrase "operator notes" / "system-tier Global Memory
content" — same concept, now with one settled name.

## 1. Problem, confirmed against current code

AgentMux should ship Operator Config an agent genuinely needs regardless of
what the deployment is used for — how to call the App API, environment
quirks, known gotchas — but today there is **no seeding path for this
content at all**:

- `read_claude_global_config` (`agentmux-srv/src/server/agent_handlers/
  bundle.rs:316-324`, backing `COMMAND_GET_CLAUDE_GLOBAL_CONFIG`) resolves
  `~/.agentmux/shared/providers/claude/CLAUDE.md` and does a **pure
  `fs::read_to_string`** — no template, no default content baked into the
  binary, nothing to fall back on.
- The only write AgentMux ever performs at that path is `seed_claude_md_
  placeholder_if_missing` (`agentmux-srv/src/backend/providers.rs:799-859`,
  called from `prepare_provider_auth_dir` at agent-spawn time,
  `agent_open.rs:445`) — and it writes an **empty, self-disclaiming
  placeholder** ("AgentMux: intentionally empty... use Armory -> Memory ->
  Global instead"), only `if !path.exists()`. It never seeds real content.
- That file is also **Claude-only** (`resolve_shared_claude_provider_dir()`
  → `providers/claude/CLAUDE.md` specifically) and, per its own placeholder
  text, explicitly out of band from Global Memory composition — a Codex or
  Gemini agent never sees it at all, and it is not part of `format_global_
  bundle_block`'s injected content either.

Net effect: today, "how does this agent learn about the App API" has no
answer that survives a fresh install, and the one thing that gestures at an
answer is provider-specific and structurally disconnected from the
mechanism (`is_system` Global Memory) this codebase already built to solve
exactly this kind of problem.

## 2. Design summary

Ship AgentMux's own **Operator Config** as `is_system=1` rows (per
`SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_24.md`), seeded from a compiled-in
manifest and kept in sync on every startup — not a one-time migration. No
new composition or injection pathway is needed: `format_global_bundle_block`
already injects every `is_global=1` row (Operator Config and Global Memory
alike, per the unify spec) into every agent's context, provider-agnostically,
for free.

```
 agentmux-srv/operator-config-seed.json   (compiled in, include_str!)
        │  { version, entries: [{ id, name, instructions }, ...] }
        ▼
 backend::operator_config_seed::auto_seed_on_startup(&store)
        │  called from bootstrap.rs, next to agent_seed's own call
        │  (diff-and-reseed each startup — see §3.2)
        ▼
 db_bundles WHERE is_system=1            (via bundle_upsert_system)
        │
        ▼
 format_global_bundle_block()   ← already existing, unmodified
        │
        ▼
 injected into EVERY agent, every provider, at every launch
```

This deliberately does **not** touch `providers/claude/CLAUDE.md` (Provider
Config) or its placeholder-seeding logic — see §6 for why that's left as an
open question rather than folded in here.

## 3. Mechanics

### 3.1 Manifest

New file `agentmux-srv/operator-config-seed.json`, embedded at compile time
via `include_str!`, mirroring `agent-seed.json`'s own embedding style
exactly (`agent_seed.rs:138`):

```json
{
  "version": 1,
  "entries": [
    {
      "id": "operator-config-app-api",
      "name": "AgentMux Operator Config: App API",
      "instructions": "... App API usage, env vars, gotchas ..."
    }
  ]
}
```

`id` is a manifest-supplied literal string (not a generated UUID) — this
matches `agent_seed.rs`'s own `SeedMemory.id` convention (`agent_seed.rs`'s
one-time `seed_memories()`), not the `Uuid::new_v5` scheme used by `skill_
seed.rs`/`mcp_seed.rs`. A stable literal id is what makes "does this row
already exist" a simple lookup rather than requiring a namespace/derivation
step, and it keeps the manifest human-readable when a future Operator
Config change needs reviewing in a PR diff.

`version` is carried for the same reason `agent-seed.json`'s own top-level
`version` is: informational/loggable at startup, not the comparison this
spec's reseed logic actually keys on (see §3.3 for why content-identity, not
a version number, is the correct check here — this is a deliberate
departure from `agent_seed.rs`'s field, which research confirmed is
similarly present but *not actually read* by its own reseed comparison).

Authoring the actual Operator Config prose (what the App API section says,
which gotchas make the cut) is implementation-time work, out of scope for
this spec.

### 3.2 Where it hooks in

A new `backend::operator_config_seed::auto_seed_on_startup(&store)`,
called from `bootstrap.rs` immediately after the existing `backend::
agent_seed::auto_seed_on_startup(&wstore)` call (`bootstrap.rs:870`) — same
unconditional, every-startup invocation site, no feature flag.

### 3.3 Reseed-vs-skip logic — the crux, and where this spec departs from `agent_seed.rs`

`agent_seed.rs`'s own `reseed_if_needed()` decides "this entry changed" by
comparing a single field (`description`) between the manifest and the
stored row — not a hash, not a version number. That works for agent
definitions because most of an agent's mutable state (`provider`, `shell`,
`auto_start`, ...) is explicitly user-editable and deliberately excluded
from the comparison; `description` is treated as the one AgentMux-owned
field.

Operator Config is different: the **entire value of a seeded row is its
`instructions` text**, and — per the unify spec — a human CAN edit an
Operator Config entry today through the same unified Armory `MemoryEditor`
the UI now uses for both tiers (`upsertsystemmemory`, `bundle.rs:171-241`,
is wired to the UI's save path for `isSystem` rows, just never to any MCP
tool). A field-level "did the description change" check has no equivalent
here — the thing we'd be diffing against is the thing a human might have
legitimately rewritten. Overwriting it blindly on the next AgentMux upgrade
would silently discard a deliberate human edit; never overwriting it would
mean a shipped Operator Config fix never reaches an install that happens to
have touched the row once.

**Proposed check: use `db_bundle_versions`' own `written_by` column (added
in `SPEC_AGENT_FACING_GLOBAL_MEMORY_API_2026_09_15.md` Phase 0) as the
ownership signal**, instead of inventing a new one:

- The seeder writes with a fixed, reserved identity, e.g. `written_by:
  "agentmux-operator-config-seed"` (via a new `Store::bundle_upsert_system_with_
  version`, mirroring `bundle_upsert_with_version`'s existing "one
  transaction, both the `db_bundles` write and the version-row insert"
  pattern — see `bundles.rs`'s `bundle_upsert_with_version` and `bundle_
  versions.rs`'s `bundle_version_insert_tx`).
- On startup, for each manifest entry: look up the row's **latest** `db_
  bundle_versions` entry (`bundle_version_list`, already built, take the
  newest).
  - **No row exists yet** → insert it (first install, or a fresh manifest
    entry added in a later release).
  - **Latest version's `written_by == "agentmux-operator-config-seed"` AND its
    content differs from the manifest's current `instructions`** → safe to
    overwrite: nothing but the seeder has touched this row since it was
    last seeded, so the new manifest content wins. Insert a new version.
  - **Latest version's `written_by == "agentmux-operator-config-seed"` AND content
    already matches** → no-op, exactly like `agent_seed.rs` returning
    `Ok(None)` when nothing changed.
  - **Latest version's `written_by != "agentmux-operator-config-seed"`** (i.e.
    `"armory-ui"`, meaning a human edited it via the Armory pane) → **skip,
    never overwrite**, and log at `info` level that a manifest update was
    withheld because of a local edit. This is the "preserve user overrides"
    guarantee `agent_seed.rs` provides for `user_hidden` etc., translated
    to this table's shape: ownership-by-last-writer instead of a per-field
    allowlist, because here the *entire* value is the thing a human might
    own.

This reuses Phase 0's audit trail as load-bearing infrastructure rather
than adding a second, parallel "did we seed this" marker — the version
table already records exactly the fact this check needs.

### 3.4 Name-collision handling

`bundle_upsert_system` (`bundles.rs:383`) has no uniqueness handling of its
own beyond `id`-keyed upsert; `db_bundles.name` carries a table-wide
`UNIQUE` constraint (confirmed in `SPEC_AGENT_FACING_GLOBAL_MEMORY_API_
2026_09_15.md`'s own implementation — a real constraint hit during that
work, not documented anywhere before it). `agent_seed.rs`'s own one-time
`seed_memories()` already had to handle exactly this for its `bundle_
upsert` calls: catch the constraint violation and warn-and-skip
(`agent_seed.rs:285-298`) rather than aborting the whole seed pass. The new
seeder applies the identical warn-and-skip per entry — a user who happens
to have an unrelated bundle already named e.g. "AgentMux Operator Config:
App API" should not block every other manifest entry from seeding.

### 3.5 Why not the `skill_seed.rs`/`mcp_seed.rs` one-time-migration pattern

That pattern (deterministic `Uuid::new_v5` id, applied once via a numbered
DB migration, never re-applied, deliberately respecting user deletion) is
correct for starter skills/MCP servers: a user can freely delete a starter
skill they don't want, and AgentMux should never resurrect it. Operator
Config is different in kind — it describes AgentMux's own current behavior,
so stale cached content left over from an old release is actively
misleading (a note describing a since-changed App API is worse than no
note). That's why §3.3's ownership-aware reseed-every-startup approach is
required here, not the one-time seed.

## 4. Non-goals

- **Not proposing any change to `is_system` write isolation.** The seeder
  is simply a new *caller* of the already-existing, already-guarded `bundle_
  upsert_system` — it does not touch `bundle_upsert`/`bundle_delete`/
  `bundle_reorder`'s existing refusal to write `is_system=1` rows.
  Nothing here weakens the boundary `SPEC_GLOBAL_MEMORY_SYSTEM_TIER_2026_08_
  24.md` established.
  - Note: `bundle_upsert_system` refuses to convert an existing `is_
    system=0` row into a system one (it checks the existing row's own
    `is_system` first). This means the seeder can only ever create a
    **new** row at a given manifest `id`, or update a row it (or a prior
    seed pass) already created as system-tier — it can never silently
    annex a pre-existing ordinary bundle that happens to reuse the same
    `id` string. Manifest `id`s should be namespaced distinctly (e.g. the
    `operator-config-` prefix used by the shipped manifest, §3.1) to make
    an accidental collision with a user-chosen id vanishingly unlikely in
    the first place.
- **Not writing an `agentmux-mcp` tool or Armory UI affordance for this.**
  The seeded rows appear in the existing unified Global Memory list exactly
  like any other Operator Config entry — no new surface is being added,
  just a new source of rows into an existing one.
- **Not authoring the Operator Config content itself.** What goes in the App
  API / environment / gotchas sections is implementation-time writing work,
  deliberately left out of this design spec.

## 5. Test plan (for implementation time)

- `bundle_upsert_system_with_version`: round-trip write + version insert in
  one transaction, mirroring `bundle_upsert_with_version`'s own test
  module.
- Reseed logic: fresh install (no row) → inserted; unchanged manifest,
  unchanged content, `written_by == seed identity` → no-op, no new version
  row; manifest content changed, `written_by == seed identity` → new
  version inserted, `instructions` updated; manifest content changed but
  latest version's `written_by == "armory-ui"` → skipped, row unchanged,
  confirmed via a regression test analogous to `SPEC_AGENT_FACING_GLOBAL_
  MEMORY_API_2026_09_15.md`'s own hijack-regression tests (this is the
  "don't clobber a human edit" guarantee, and deserves the same explicit
  test-backed treatment that spec gave the hijack-prevention guards).
- Name-collision: a pre-existing ordinary bundle with the same `name` as a
  manifest entry → seed of that one entry is skipped with a logged warning,
  every other manifest entry still seeds normally.

## 6. Open question — not resolved here

What happens to the existing Claude-only `providers/claude/CLAUDE.md` /
`seed_claude_md_placeholder_if_missing` path (Provider Config, per §0) once
real Operator Config content ships? Candidates, **not chosen**:

1. Leave it exactly as-is (empty placeholder, pointing at Global Memory) —
   it already tells a human "look in Armory -> Memory -> Global instead,"
   which becomes literally true once this spec ships. No further change
   needed.
2. Update the placeholder text now that there's real content to point at
   more specifically (name the seeded entry).
3. Deprecate/remove the Claude-specific file and its seeding function
   entirely, on the grounds that Operator Config now fully subsumes its
   purpose and does so provider-agnostically.

## 7. Known limitation — deliberately not fixed in this PR

`bundle_upsert_system_if_changed`'s no-op detection (§3.4 of the original
implementation, `bundle.rs`'s `upsertsystemmemory` handler) compares an
incoming save only against the row's CURRENT live content — not against
whatever version the Armory editor actually had loaded when the operator
opened it. If another process (another AgentMux instance sharing this
store, or the startup reseed itself) writes a new version to the row
between the editor loading it and the operator clicking Save, a
byte-identical-to-what-they-originally-saw (but now stale) submission is
indistinguishable from a real edit: it gets written back, permanently
reverting the row to stale content and marking it `written_by="armory-ui"`
— which then blocks every future manifest-driven correction, per this
whole mechanism's own "never clobber a human edit" rule.

Confirmed independently by both Codex and ReAgent (PR #3244, round 9) as a
real gap, not a false positive. Not fixed here because a real fix needs a
different layer than everything else in this PR: the Armory UI's save
request would need to carry the version/hash it loaded (optimistic
concurrency control), which means frontend changes (`global-bundle-
manager.tsx`/`global-bundle-model.ts`) and an RPC schema change to
`COMMAND_UPSERT_SYSTEM_MEMORY`, not just the backend `Store`-layer work
this PR is scoped to. Flagged here rather than silently dropped — a
follow-up spec should design the actual optimistic-concurrency check
before implementing it, rather than bolting it on ad hoc.

Flagging rather than deciding, consistent with how `SPEC_GLOBAL_MEMORY_
UNIFY_SYSTEM_AND_ORDINARY_2026_09_15.md` left its own visual-distinction
question open rather than guessed.

## 8. Known limitation — cyclic name swaps within one manifest

`seed_from_manifest`'s fixed-point retry loop (§3, PR #3244 round 9)
resolves any name-collision dependency CHAIN of any length — entry C
waiting on a name B is about to release, B waiting on a name A is about to
release, and so on — because each pass that makes at least one entry
succeed unblocks the next. It cannot resolve a genuine CYCLE: two entries
directly swapping display names in the same manifest (A: `X` → `Y`, B: `Y`
→ `X`) can never make progress, since neither can succeed without the
other going first. The loop correctly detects this (a pass with zero
successes) and logs both as permanent collisions rather than looping
forever, but both entries are left on their stale names/content/generation
until the manifest itself stops describing a direct swap.

Not fixed here — a real fix needs either a temporary placeholder name
(rename both to unique scratch names first, then to their final names in a
second pass) or a transaction that plans every rename before applying any
of them, meaningfully more complex than the chain-resolving loop this PR
already has. Deliberately not implemented: AgentMux authors its own
Operator Config manifest and controls every entry's `name`, so a
deliberate two-entry swap is not a realistic authoring pattern — flagging
this as a known edge case is judged sufficient rather than building
cycle-breaking logic for a scenario this codebase is never expected to
actually produce.
