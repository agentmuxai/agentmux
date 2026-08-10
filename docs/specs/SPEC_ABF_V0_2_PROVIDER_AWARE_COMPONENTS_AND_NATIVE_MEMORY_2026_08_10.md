# Spec: ABF v0.2 — Provider-Aware Components + Native Memory

**Date:** 2026-08-10
**Status:** proposal — no implementation yet.
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

**Import-time merge:** `default` entries load for every agent regardless of
provider; a matching provider-keyed entry appends after `default` (same
precedence idiom already used for structured-field-wins-then-free-form-
extends, e.g. `agent_open.rs`'s auth-env-then-free-form-`env`-blob
ordering). A bundle with no provider-specific entries behaves exactly as
v0.1 bundles do today.

**Export-time behavior:** unchanged by default — `export_bundle()` keeps
writing everything under `default` unless the source `db_bundles.provider`
column (exists today, unused — §1.2) is non-empty, in which case exported
instruction content lands under that provider's key instead of (not
in addition to) `default`. No new UI decision needed for v0.2: this makes
the already-existing-but-ignored column meaningful for the first time,
rather than adding a new one.

**Backward compatibility:** an importer encountering the v0.1 flat-array
shape (`"instructions": ["instructions/AGENTS.md"]`) treats the whole
array as an implicit `default` entry. No v0.1-produced bundle breaks on
a v0.2 importer.

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

**Import behavior:** memory files import into `db_agent_native_memory`
directly, keyed by the *importing* agent's own freshly-created (or
target) `agent_id` — never the exporting agent's ID, which has no meaning
in the importing instance. Mirrors `persist_define_content`'s existing
pattern: commit the agent definition row first, then write content,
logging-but-not-failing on a content-write error so a partial memory
import never blocks agent creation.

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

- **Fixing `agent_config.rs`'s CLAUDE.md-always runtime behavior** (§1.2).
  Real, independently-fixable gap; unrelated to the export/import format
  itself and belongs in its own PR with its own test plan, not bundled into
  a schema change.
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

- No backfill needed for `components.instructions`'s shape change — every
  currently-stored `db_bundles` row exports fresh at request time; there's
  no persisted `armory.json` state to migrate, only the exporter/importer
  code.
- `db_bundles.provider`/`.model` columns already exist and are already
  populated for every bundle (confirmed unused-but-present, §1.2) — §2.2's
  export-time behavior activates immediately for any bundle that already
  has a non-empty `provider`, no new data entry required from users.
- `db_agent_native_memory` already exists and is already populated
  organically per `SPEC_NATIVE_MEMORY_DURABLE_SYNC_2026_08_07.md`'s
  write-through-on-read design — §2.3 has data to export from day one for
  any agent whose Stash Memory tab has been opened at least once.

## 5. Test plan

- Unit: `parse_bundle_import` accepts both the v0.1 flat-array
  `instructions` shape and the v0.2 keyed-object shape; a v0.1 array
  imports identically to an equivalent v0.2 `{"default": [...]}` object.
- Unit: export with `db_bundles.provider` set routes instructions under
  that provider's key, not `default`; export with an empty `provider`
  behaves exactly as today (regression check against existing v0.1 export
  tests).
- Unit: `credentialProvider` round-trips through export → import
  unchanged; a v0.1 bundle still carrying the old `provider` key in
  `accounts/requirements.json` is still accepted on import (read as
  `credentialProvider`'s predecessor name, not rejected).
- Unit: `bundle.export_for_agent` — memory files present for an agent with
  populated `db_agent_native_memory` rows; empty `components.memory`
  omitted entirely (not an empty array) for an agent with none, matching
  the existing omit-empty-components convention already used for
  `skills`/`mcpServers`/`accounts`.
- Integration: full export → import round trip for a bundle with
  provider-scoped instructions AND memory components, landing in a fresh
  target instance, confirming both the merged instructions content at
  launch time and the imported agent's Stash Memory tab show the expected
  content.
