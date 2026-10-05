# SPEC: Agent Bundle Format v0.3, and bundles that bind to any agent

**Status:** active — shipped: Phase A (the v0.3 format), #4346. Remaining: Phase B (bind to any agent, §3)
**Date:** 2026-10-05
**Author:** Camper (agent), at operator request
**Related:** `SPEC_RETIRE_ARMORY_CONNECTORS_AND_KNOWLEDGE_PANES_2026_10_05.md` §4.6–4.7 (the
direction, decided by the operator 2026-10-05), `SPEC_ABF_V0_2_PROVIDER_AWARE_COMPONENTS_AND_NATIVE_MEMORY_2026_08_10.md`
(v0.2), `ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md` (every agent gets its own bundle).

---

## 0. Summary

**Decided (operator, 2026-10-05):** a bundle packages an agent's instructions, MCP servers,
memory and skills; it isn't tied to a model or a harness, so it can be bound to any agent. ABF
is renamed **Agent Bundle Format** and cut as **v0.3**.

Reading the code for this spec turned up that "bind to any agent" is more than a format change.
Today a bundle's `provider` **decides the agent's harness**: wherever AgentMux resolves which
CLI an agent runs, a non-empty bundle `provider` silently wins over the agent's own (§1.2). So
this spec splits the work in two, each its own PR:

- **Phase A, the format.** v0.3 writes `bundle.json` with the `agent-bundle/v0.3` schema;
  `provider` / `model` become an optional `suggestedFor` hint. The importer reads v0.1–v0.3
  forever. No behaviour change at spawn.
- **Phase B, the bundle carries no harness.** The agent owns its provider (read-only after
  creation, as the bundle's is today); new bundles store none; a migration first copies each
  bundle's provider onto its agent so nothing changes CLI (§3).

## 1. What the code does today

### 1.1 The fields

`Bundle.provider` and `Bundle.model` (`backend/storage/bundles.rs`, columns on `db_bundles`,
both `NOT NULL DEFAULT ''`) date from the original `db_memories` table. Despite its name,
`model` holds a **vendor** (`anthropic`, `openai`, `google`, `custom`), never a model id, and
nothing reads it at runtime.

Every agent's own bundle is provisioned with the agent's provider and vendor
(`bundle_provision_for_new_agent`, `storage/agents.rs`; backfill `m0021`). Global, seed and
system bundles leave both empty.

### 1.2 The bundle's provider decides the harness

`Store::resolve_effective_provider_id` (`storage/agents.rs`) returns the bound bundle's
`provider` when it's non-empty, and the agent's own only otherwise. It drives the CLI that runs
(`app_api/agent_open.rs`), the startup-instructions file name (`agent_config.rs`), the
credential gate (`identity/resolver/inject.rs`), clone and fork, and agent export. The frontend
mirrors it in `resolveEffectiveLaunchProvider` (`agent-launch-env.ts`), used by the launch
modal, the picker, install, create-from-template, quick fork and the identity panels.

When the agent's provider and its bundle's disagree (`updateagent` and `agent.define` can change
the agent's), the bundle wins, with no warning. The agent's bound bundle (`memory_id`) can't be
changed after creation.

### 1.3 Per-harness content isn't applied yet

- `instructions_by_provider` (v0.2's per-harness variants) is stored, exported, imported and
  validated, but **never chosen at spawn**; `bundle_import.rs` itself calls that a
  not-yet-built step. The editor's help text says otherwise.
- A bound bundle's instructions reach an agent only as its startup message
  (`startup_bundle_id`); the startup file is built from the agent's own content plus global
  bundles.
- A bundle's MCP servers and skills are merged into the agent's and written in Claude's layout
  (`.mcp.json`, `.claude/…`) whatever the harness; a harness that reads neither gets nothing,
  with no notice.

### 1.4 The file format

The exporter (`backend/bundle_export.rs`) writes `armory.json` with
`$schema: https://docs.agentmux.ai/schemas/armory-bundle/v0.2/bundle.schema.json`,
`provider` / `model` (or `null`), and `"version": "0.1.0"`, the bundle's own content version.
The importer (`bundle_import.rs`) requires `armory.json`, checks only that `$schema` exists,
warns unless `version` is `0.1.x` (mixing the content version up with the format version), and
tells v0.1 from v0.2 by the shape of `components.instructions`. `armory.json` is also
hard-coded in `app_api/bundle/components.rs` and `app_api/bundle/import_for_agent.rs`.

The docs site hosts only the `armory-bundle/v0.1` schemas, so today's v0.2 `$schema` URL is a
dead link.

## 2. Phase A: the v0.3 format

### 2.1 The manifest

| | v0.2 | v0.3 |
|---|---|---|
| Name | Armory Bundle Format | Agent Bundle Format (still ABF, still `.abf`) |
| Manifest file | `armory.json` | `bundle.json` |
| `$schema` | `…/schemas/armory-bundle/v0.2/bundle.schema.json` | `…/schemas/agent-bundle/v0.3/bundle.schema.json` |
| `provider`, `model` | top level, nullable | gone; replaced by `suggestedFor` |
| `suggestedFor` | — | optional `{ "provider": "claude", "vendor": "anthropic" }`, either key optional: who the author had in mind; never enforced |
| `version` | the bundle's content version | unchanged meaning (content version) |
| Components | instructions (+ per-provider variants), context files, MCP servers, skills, native memory | unchanged |
| `requirements.json` | under `armory-bundle/v0.2` | unchanged content, under `agent-bundle/v0.3` |

`model` is renamed `vendor` inside the hint because that's what it has always held (§1.1).

### 2.2 Export

Writes v0.3 only: `bundle.json`, the v0.3 `$schema`, and `suggestedFor` built from the row's
`provider` / `model` (omitted when both are empty).

### 2.3 Import

Reads every version, forever, since users have those files:

- **Manifest:** `bundle.json`, else `armory.json`. Both present: `bundle.json` wins, with a
  warning.
- **Format version from `$schema`**, not `version`: `agent-bundle/v0.3` → v0.3;
  `armory-bundle/v0.2` → v0.2; `armory-bundle/v0.1` → v0.1; missing or unknown → read by shape,
  as today, with a warning. The `version` field is the content version and no longer warns.
- **The hint:** v0.3 `suggestedFor.provider` / `.vendor`, or v0.1–v0.2 top-level `provider` /
  `model`, land in the row's `provider` / `model`, as today.

`components.rs` and `import_for_agent.rs` find the manifest through one shared helper instead of
the hard-coded name.

### 2.4 Schemas (`agentmux-docs`)

Publish `public/schemas/agent-bundle/v0.3/bundle.schema.json` and `requirements.schema.json`,
and the missing `armory-bundle/v0.2/` pair, so every `$schema` URL any AgentMux version has
written resolves.

### 2.5 Tests

Export writes v0.3 (`bundle.json`, schema URL, `suggestedFor`, no top-level `provider`); import
reads a v0.1, a v0.2 and a v0.3 bundle (manifest name, hint mapping, no spurious version
warning); an archive with both manifests; a v0.3 export round-trips through import; the
`components` and `import_for_agent` paths find `bundle.json`.

## 3. Phase B: the bundle carries no harness

A provider-agnostic bundle means the harness lives on the agent alone. That touches three
places: what decides the CLI at spawn, what's written when an agent is created, and the data
already written under the old rule (operator, 2026-10-05: "its provider agnostic, so there may
also need to be migrations").

### 3.1 The agent owns its harness, and keeps it

- **Spawn reads the agent.** `resolve_effective_provider_id` and
  `resolveEffectiveLaunchProvider` return `agent.provider`. They read the bound bundle's only for
  an agent with no provider at all: after the migration (§3.3) there should be none, but it runs
  per channel and best-effort, and for such an agent the bundle's value is what it has always run
  with. Every caller listed in §1.2 (CLI choice, startup file name, credential gate, clone, fork,
  export) follows without its own change.
- **The lock moves from the bundle to the agent.** The bundle's provider was made read-only
  because the agent's could drift (`ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md` §7.4.1).
  That reason still holds once the agent owns it: an agent's sessions, its account links and its
  native memory folder all belong to one harness, so silently switching CLI under them breaks
  resume and sign-in. So `agent.provider` becomes read-only after creation: `updateagent` and
  `agent.define` with `if_exists=update` refuse a different provider with the same
  `FORBIDDEN … readonly once set` error the bundle guard returns today. Running the same setup on
  another harness is a new agent: fork it and pick the provider, which already copies everything
  else. The UI has no provider editor today, so only API callers see the refusal.
- **The bundle's guard goes.** `check_provider_model_immutable` (`app_api/bundle/mod.rs`) and
  its tests are removed: the fields it guards no longer decide anything.

### 3.2 Creating agents and bundles

- **`bundle_provision_for_new_agent` writes no provider or vendor.** An agent's own bundle starts
  as plain content. All six creation paths (new agent, template clone, fork, Claude import, bulk
  import, `agent.define`) go through it, so none needs its own change. Clone and fork already
  take the new agent's provider from the source agent (through the resolver, §3.1).
- **The vendor isn't stored at all.** It's derived from the agent's provider and
  `model_vendor_base_url` (`resolve_effective_vendor`) wherever it's needed.
- **The bundle editor** shows "Suggested for" (provider, vendor) as optional and editable, and
  never blocks Save on it. New bundles created there start with it empty.
- **Export fills the hint from the agent, not the row.** `bundle.export_for_agent` knows the
  agent, so it writes `suggestedFor` from that agent's provider and derived vendor; a plain
  `bundle.export` writes the row's hint if it has one (imported bundles keep theirs). The hint
  then says something true about where the bundle came from without the bundle having to store a
  harness.
- **Import is unchanged:** an imported hint is stored as a hint and never sets an agent's
  provider.

### 3.3 Migrating what's there

One new migration (after `m0034`), idempotent, on the same stores and with the same reach as
`m0021` (local-channel definitions; bundles in `AppState.id_store`'s store):

1. **Make every agent's own provider the one it runs today.** For each definition with a
   `memory_id` whose bound bundle has a non-empty `provider` different from the definition's
   (including an empty definition provider): set the definition's `provider` to the bundle's.
   Log each change (agent, from, to). After this, §3.1's resolver gives every agent exactly the
   CLI the old rule gave it. In practice this touches only agents changed through
   `agent.define` updates and the empty-provider agents `m0021` backfilled as `claude`.
2. **Leave the bundle rows' values in place, as hints.** Clearing them in the same migration
   isn't safe: bundles live in the store every channel shares, definitions live per channel, so a
   channel that migrates later would find the bundle's value already gone and couldn't do step 1.
   The values are harmless once nothing reads them for the harness, and exports of an agent's own
   bundle take the hint from the agent anyway (§3.2). A later clean-up can clear them once every
   channel has run this migration, if it's worth it.
3. **Downgrade stays safe.** An older build still prefers the bundle's provider, which step 2
   kept and step 1 made equal to the agent's, so it runs the same CLI.

Definitions known only from the global registry carry no `memory_id` (`m0021`'s documented
gap), so the old rule never overrode them; they need nothing.

### 3.4 Tests

- Resolvers: the agent's provider decides; the bundle's is read only for an agent with none.
- The agent lock: `updateagent` and `agent.define` update refuse a changed provider, and
  accept an unchanged one.
- Creation: a new agent's bundle has an empty provider and vendor, across the creation paths.
- The migration: a drifted agent takes its bundle's provider; an empty-provider agent takes
  `claude` from its `m0021` bundle; an agent already in step is untouched; a second run changes
  nothing; bundle rows are not modified.
- Export: `export_for_agent` writes `suggestedFor` from the agent; a plain export of an empty-hint
  bundle omits it.
- The editor saves with an empty hint; the removed bundle guard's tests go.

## 4. Compatibility and risks

| Risk | Mitigation |
|---|---|
| An agent runs a different CLI after Phase B | the migration first copies each bound bundle's provider onto its agent (§3.3), so every agent keeps the CLI it runs today |
| A script changes an agent's provider through `agent.define` and is now refused | the error says the provider is read-only once set; fork the agent to run it on another harness (§3.1) |
| A channel that hasn't migrated yet shares bundles with one that has | the migration never clears bundle rows (§3.3 step 2), so an unmigrated channel's old rule still finds them |
| A v0.3 file opened in an older AgentMux | it looks for `armory.json` and refuses the import; the release note says to update. Exporting for older versions isn't offered |
| Third-party tools reading `armory.json` | none known; the schema page documents both |

## 5. Not in this spec

Choosing `instructions_by_provider` at spawn and writing MCP servers and skills in each
harness's own layout. Both are needed for "any agent" to be complete; each is its own design.

Telling the user when a harness can't take a component (MCP servers or skills on a CLI that
reads neither Claude layout) was drafted here and moved out: it isn't specific to bundles. An
agent's own MCP servers and skills are written in Claude's layout too, and a non-Claude agent
drops them the same way, so the notice belongs with the per-harness composition above.

## 6. Decisions

Decided (operator, 2026-10-05: the recommendations below, then Phase B as revised in §3):

1. **Phase A and Phase B as separate PRs**: A is a format change with no
   behaviour change; B changes which CLI some agents run.
2. **`model` becomes `vendor` in the hint**, since that's what it holds.
3. **No "export as v0.2" option**: older versions are a release behind at most.
4. **The agent's provider is read-only after creation** (§3.1): the lock that kept
   sessions and accounts on one harness moves from the bundle to the agent, rather than letting a
   provider change switch CLIs.
5. **The migration keeps bundle rows' values** (§3.3): clearing them is unsafe
   while channels share bundles but migrate separately.
