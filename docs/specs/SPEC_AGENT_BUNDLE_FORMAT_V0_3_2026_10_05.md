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
- **Phase B, the binding.** The agent's own provider decides its harness; the bundle's becomes
  a hint, shown but not enforced. That changes one resolver on each side (server and frontend),
  the bundle editor, and an immutability guard.

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

## 3. Phase B: bind to any agent

### 3.1 The agent's provider decides

`resolve_effective_provider_id` and `resolveEffectiveLaunchProvider` return the **agent's own**
provider, and fall back to the bundle's only when the agent has none (agents older than the
provider field; `m0021` set those bundles to `claude`). The two stay in step: one table-driven
test on each side, same cases.

Consequences, all intended:

- Editing an agent's provider now takes effect, instead of being silently overridden.
- A bundle can be bound to (or imported for) an agent on a different harness without changing
  that agent's harness.

### 3.2 The hint in the UI

The bundle editor shows provider and vendor as an optional "Suggested for" hint: editable, not
required, never blocking Save. `check_provider_model_immutable` (`app_api/bundle/mod.rs`) goes:
it guarded a field that no longer decides anything, and the editor's own save path never
applied it.

### 3.3 Saying what a harness can't take

When an agent spawns with a bound bundle whose components its harness doesn't read (MCP servers
or skills for a CLI that reads neither Claude layout), the spawn logs it and the agent pane
shows a one-line notice naming what was left out. Composing per-harness formats is out of scope
here (§5).

### 3.4 Tests

The resolver tables (agent provider wins; empty agent provider falls back to the bundle; no
bundle), the editor's Save with an empty hint, the removed guard's tests, and the notice for a
harness without MCP or skills support.

## 4. Compatibility and risks

| Risk | Mitigation |
|---|---|
| An agent whose bundle's provider differs from its own runs a different CLI after Phase B | that is the fix (§1.2); the release note says so. Every provisioned agent's bundle was created from the agent's own provider, so this only affects agents whose provider was edited later |
| A v0.3 file opened in an older AgentMux | it looks for `armory.json` and refuses the import; the release note says to update. Exporting for older versions isn't offered |
| Third-party tools reading `armory.json` | none known; the schema page documents both |

## 5. Not in this spec

Choosing `instructions_by_provider` at spawn and writing MCP servers and skills in each
harness's own layout. Both are needed for "any agent" to be complete; each is its own design.

## 6. Decisions

1. **Phase A and Phase B as separate PRs** (recommended): A is a format change with no
   behaviour change; B changes which CLI some agents run.
2. **`model` becomes `vendor` in the hint** (recommended), since that's what it holds.
3. **No "export as v0.2" option** (recommended): older versions are a release behind at most.
