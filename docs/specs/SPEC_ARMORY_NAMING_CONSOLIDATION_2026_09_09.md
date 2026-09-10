# Spec: Armory Naming Consolidation — Bundle vs Memory

**Status:** proposed — needs the §9 decisions before Phase 1 starts
**Date:** 2026-09-09
**Verified against:** `b65df28c2` (code, not spec prose)
**Supersedes the naming proposal in:** `docs/status/STATUS_ARMORY_MEMORY_CONSOLIDATION_2026_09_09.md` §4, which was wrong — see §2.4
**Related:** #2024 (tracking), `ARCHITECTURE_ARMORY_FOUNDATION_CONSOLIDATION_2026_08_19.md` §3.1/§3.4

---

## 1. Why this comes first

Two architectural questions are currently open and blocked on vocabulary:

- The agent-less `bundle.export` omits `components.memory` while the agent-scoped
  `bundle.export_for_agent` includes it — whether that asymmetry is the intended
  design or a gap is unclear from the code alone.
- Curated memory is structurally empty for every agent, and while the Armory UI
  shows the empty state, nothing at launch tells the operator the agent received
  no `# Memory` section.

Neither can be discussed precisely while one word denotes six different things.
This spec fixes the vocabulary so those two can be reasoned about; it deliberately
does **not** attempt to solve either.

## 2. The problem, stated precisely

### 2.1 Six live terms for overlapping concepts

`Memory` · `Bundle` · `ABF` · `Brain` · `Preset` · `Global Memory`

### 2.2 The exhibit

`agentmux-srv/src/server/app_api/mod.rs:757` — four vocabularies in eight lines:

```rust
pub(crate) async fn bundle_list_impl(state: &AppState) -> Result<Value, String> {
    let memories = state.id_store.bundle_memory_list()...;   // storage: bundle_memory_*
    let bundles: Vec<_> = memories.iter().map(...).collect(); // memories -> bundles
    Ok(json!({ "bundles": bundles, "presets": bundles }))     // emitted under both keys
}
```

A `bundle_*` function calls `bundle_memory_*` storage, binds it to `memories`,
maps it to `bundles`, and serialises it as both `bundles` and `presets`.

### 2.3 What each term actually refers to today

| Term | Real referent | Verdict |
|---|---|---|
| Rust `struct Memory` (`memory_bundles.rs:25`) | a `db_bundles` row — a **Bundle** | misnamed |
| "Global Memory" (UI) | `db_bundles` rows with `is_global=1` — **Bundles with a scope flag** | misnamed |
| "Personal Memory" (UI) | `db_agent_native_memory` | **correct** |
| `listmemories`/`getmemory`/`upsertmemory`/`deletememory`/… | operate on **Bundles** | misnamed |
| "Brain" (`format_global_brain_block`, `reorderglobalbrain`, `global-brain-*`) | global **Bundles** | misnamed |
| "Brain" (MCP schema gloss, `tool_schemas.rs:606/618/640`) | **native memory** | same word, opposite referent |
| "Preset" (`presets` JSON key, `PresetList`/`PresetGet` MCP tools, `/api/v1/agent/preset/list`) | **Bundles** | retired in name only; still on the wire |
| `memory_id` columns (349 tokens) | hold a **bundle id** | misnamed |

### 2.4 Correction to the earlier proposal

The status doc proposed "Global Memory → **Instructions**". That is wrong: a
`db_bundles` row carries `instructions`, `instructions_by_provider`,
`context_files`, `mcp_servers`, `skills`, `provider`, `model` — naming the
container after one of seven payload fields would be a new inaccuracy. Global
bundles are **Bundles with a scope flag**, not a distinct kind of thing.

### 2.5 The same table is surfaced under two tabs

Armory's rail is `Accounts | Memory | Skills | MCP Servers | ABF`
(`armory-model.ts:36-42`). `db_bundles` appears in **two** of them:

- **ABF** tab → `MemoryManager` (`view/memory/memory-manager.tsx`)
- **Memory → Global** → `GlobalBrainManager` (`view/brain/global-brain-manager.tsx`)

Both read `db_bundles` through the same `listmemories`/`upsertmemory` RPCs and
both subscribe to `memories:changed`. Same table, two tabs, three component
names, zero shared vocabulary.

### 2.6 Two duplicate RPC surfaces over one storage layer

`bundle.list/get/upsert/delete` and `listmemories/getmemory/upsertmemory/deletememory`
both terminate in `Store::bundle_memory_*`. This is therefore a **consolidation**,
not merely a rename — and the target names are already occupied by the surface we
want to keep.

## 3. Target vocabulary

Exactly two nouns for these concepts, plus one for the format:

- **Bundle** — a `db_bundles` row: the agent's instructions + context files + MCP
  refs + skill refs + provider/model. Scope is expressed by **flags**, not by a
  different noun: a bundle is *global* (applies to every agent), *system*
  (AgentMux-owned, highest priority, implies global), or *agent-scoped*.
- **Memory** — `db_agent_native_memory`: what an agent autonomously writes about
  itself via `MemoryWrite`. Only this is memory.
- **ABF (Armory Bundle Format)** — the portable *serialisation* of a Bundle. A
  format, not a UI surface and not a synonym for Bundle.

A third category the agent consumes, which AgentMux must be able to **read** but
does not own:

- **Project Instructions** — a project's own git-tracked instruction files that
  the CLI reads natively: the root `CLAUDE.md`, `AGENTS.md`, `GEMINI.md`, etc.
  These live in the repo, are versioned in git, and are hand-edited by humans.
  AgentMux **tracks** them (read, snapshot, version, include in export) and never
  writes them. `SPEC_CLAUDE_MD_OWNERSHIP_PROTECTION_2026_08_22.md` already
  established the "never overwrite" half; the "track" half is new scope,
  deferred to the follow-on spec in §8.

This category is why `db_bundles` is empty in practice: the operator's real
instructions live here, the CLI reads them directly, and the bundle system is
bypassed. It is not a bug in bundles — it is a second source that was never
brought under control.

Retired entirely: **Brain**, **Preset**, **Global Memory** (as a noun for bundles).

The distinction is worth preserving because it is behavioural, not cosmetic:
**bundles are composed into the agent's startup file at spawn and are frozen for
that session; memory is read live by the CLI.** A single collapsed name would
hide that.

## 4. What we are NOT renaming, and why

This section is normative. Each of these is already correct under §3.

| Surface | Why it stays |
|---|---|
| MCP tools `MemoryList/Read/Write/History/Diff/Revert` (`agentmux-mcp/src/tool_schemas.rs:596-664`) | native-memory-only; verified none touch `db_bundles`. Agent-facing — renaming breaks running agents for zero gain. |
| WS RPC `agent:memory:*` (`commands.rs:561-568`), App API `memory.list/read/write` (`:430-432`) | native memory. Correct. |
| Tables `db_agent_native_memory`, `db_agent_native_memory_versions` | correct. |
| Rust `NativeMemory*` types, `native_memory_handlers.rs`, `agent_native_memory*.rs` | correct. |
| Frontend `NativeMemoryManager`, `view/native-memory/*`, `AgentNativeMemoryModal`, `native-memory-history-model.ts` | correct. **Note:** an earlier draft wrongly flagged `NativeMemoryManager` as misnamed. It is not. |
| `db_bundles` table name | already correct (renamed in Phase 4a). |
| **ABF `components.*` keys** | `components.memory` is *correct* under §3 — it carries native memory. No key changes are needed, which is fortunate: §7.3 shows key renames would be silently lossy. |
| **Persisted format markers** — the `# Memory` section heading in generated `CLAUDE.md` (`agent_config.rs` `BUNDLE_SECTION_HEADING`, `frontend/app/view/agent/agent-config-builder.ts`, and the injector in `editor_handlers.rs` that searches for it) | Written into files on disk and matched later; emitter and injector must agree byte-for-byte. Learned the hard way: #3133's word-boundary replace changed only the Rust side and inverted global/personal precedence (Codex P1). Change all three together, or not at all. |
| **Wire and persisted key strings** — legacy RPC aliases `listmemories`/`getmemory`/`upsertmemory`/`deletememory`/`upsertsystemmemory`/`deletesystemmemory`, the `memories:changed` WPS event, `db_agents.memory_id`/`default_memory_id`, `viewType: "memory"`, `armory:section` value `"memory"`, meta key `armory:memory:subsection` | Compatibility surface. Aliases retire in Phase 3 behind the contract-test baseline; the persisted keys never rename (§9 open decision for `viewType`). |
| **User-visible strings** — Armory rail labels, `bundles.rs` guard error messages, the `providers.rs` placeholder text that names a UI path | Wording moves with the UI in Phase 4, so that what the text says matches what the user can click. Phase 1–3 are identifiers only. **Rule for every rename commit:** grep the diff for `"` and backtick lines and review each hit; a `\bMemory\b` replace does not know it is inside a string. |

## 5. Phases

Ordered by blast radius, cheapest and safest first. Each phase is independently
shippable and independently revertible.

### Phase 1 — internal names only (no wire, no schema, no UI text)

Zero contract risk. Nothing user-visible, nothing persisted.

**Rust** (`agentmux-srv`):
- `backend/storage/memory_bundles.rs` → `bundles.rs`; module `memory_bundles` → `bundles` (24 path references across 10 files)
- `struct Memory` → `struct Bundle` (159 bare-token occurrences across 31 files); both re-export paths (`storage/mod.rs:48`, `storage/store.rs:752`)
- the 9 `Store::bundle_memory_*` methods → `Store::bundle_*` (148 occurrences across 13 files)

**Mandatory file splits** — these hold both concepts and must be split, not renamed:
- `backend/rpc_types/memory.rs` → `rpc_types/bundle.rs` + `rpc_types/native_memory.rs`
  (19 `NativeMemory*` types live alongside the bundle types — verified)
- `frontend/app/store/rpc-api/memory.ts` → `rpc-api/bundle.ts` (merging into the existing one) + `rpc-api/native-memory.ts`

**Plain rename, NOT a split:**
- `server/agent_handlers/memory.rs` → `agent_handlers/bundle.rs`. This file
  registers only the bundle CRUD/system commands (plus the Claude config reader)
  — zero native-memory references. Native-memory RPCs already live in
  `server/native_memory_handlers.rs`, which stays untouched. Splitting this file
  would create an empty or duplicate owner. (Codex P2 on #3131.)

**Frontend** (names only, labels deferred to Phase 4):
- `MemoryViewModel` → `BundleViewModel`; `MemoryManager` → `BundleManager`; `MemoryDraft`/`draftFromMemory`/`draftToWire` → `Bundle*`
- `AgentNewMemoryModalPanel` → `AgentNewBundleModalPanel` (its stylesheet is *already* `AgentNewBundleModal.scss` — a half-finished rename)
- `gotypes.d.ts:481` `type Memory` → `type Bundle`

**Generated bindings caveat:** renaming `CommandDeleteMemoryData`/`DeleteMemoryResult` regenerates `frontend/types/rpc/*.ts`. The old files must be `git rm`'d — `scripts/check-rpc-bindings.sh` uses `git status --porcelain -uall` and will not flag a stale leftover.

### Phase 2 — retire "brain"

238 occurrences across 27 files, but only **two live Rust callers** of
`format_global_brain_block` (`app_api/agent_open.rs:778`, `editor_handlers.rs:85`),
so the backend is cheap; the bulk (~185) is the frontend `view/brain/` directory.

- `format_global_brain_block` → `format_global_bundle_block`
- `view/brain/global-brain-model.ts` → folded into the bundle model (see Phase 4)
- `global-brain.scss` (47 class names) → renamed with the component

Once bundles stop using "brain", the MCP schema gloss ("your own native memory
(brain) files") becomes unambiguous on its own and needs no change. Retiring the
term on the bundle side is what resolves the collision.

`reorderglobalbrain` is a wire name and belongs to Phase 3, not here.

### Phase 3 — RPC consolidation (wire-breaking)

Implements `ARCHITECTURE_ARMORY_FOUNDATION_CONSOLIDATION_2026_08_19.md` §3.4.

Retire all seven bundle-operating `*memory` commands in favour of the existing
`bundle.*` surface:

| Retire | Replacement |
|---|---|
| `listmemories` | `bundle.list` |
| `getmemory` | `bundle.get` |
| `upsertmemory` | `bundle.upsert` |
| `deletememory` | `bundle.delete` |
| `reorderglobalbrain` | `bundle.reorder` *(new)* |
| `upsertsystemmemory` | `bundle.upsert_system` *(new)* |
| `deletesystemmemory` | `bundle.delete_system` *(new)* |

Also: event `memories:changed` → `bundles:changed` (22 occurrences), and the
`presets` alias key in `bundle_list_impl` — but **only after** the REST route
`/api/v1/agent/preset/list` and the `PresetList`/`PresetGet` MCP tools are dealt
with, since those are agent-facing and read `presets` (see §7.2).

Only 2 of the 7 are currently `register_typed`/CI-gated; the other 5 are
hand-maintained in `gotypes.d.ts`. Migrating them to `register_typed` requires
`Bundle` to derive `ts_rs::TS` — which is the actual reason they were skipped
(`agent_handlers/memory.rs:453-454`). **Recommendation:** derive `TS` on `Bundle`
as part of this phase so all seven land typed and gated rather than perpetuating
the split.

`test/contract/rpc-contract.test.ts` currently baselines only `bundle.*` and
`memory.*` — the seven retiring commands are absent from it, so it will not catch
their removal. Add them to the baseline *before* removing them.

### Phase 4 — UI consolidation (user-visible)

Resolve §2.5: one table, one tab.

- Fold **Memory → Global** into the **Bundles** tab as a scope filter
  (`all | global | system | agent-scoped`), replacing `GlobalBrainManager`.
- Rail becomes `Accounts | Bundles | Memory | Skills | MCP Servers`, where
  **Memory** is now native memory only (today's "Personal") and **Bundles**
  replaces today's "ABF" label.
- ABF remains the name of the import/export format, surfaced on the Bundles tab's
  import/export affordances — not as a tab name.
- `MemorySubsection = "global" | "personal"` is persisted as block meta
  (`armory-model.ts:50`, `:94`). Migrate it the same way `LegacyArmorySection`
  already handles the retired `"native_memory"` value (`armory-model.ts:29`) —
  precedent exists in the same file.
- `MemoryViewModel.viewType = "memory"` is also a persisted key; CLAUDE.md
  documents it as deliberately frozen. Either keep the wire value `"memory"` with
  a renamed type, or migrate with the same legacy-value pattern. **Decision
  needed (§9.3).**
- `armory-view.test.tsx:116-130` asserts the exact rail labels, including negative
  assertions that "Global Memory"/"Personal Memory" are absent. Update with the
  labels.

### Deferred — DB column renames (explicitly out of scope)

At schema v32, `db_agent_definitions` and `db_agent_instances` no longer exist —
they were folded into `db_agents` (`migrations.rs:550`). The live columns holding
bundle ids are **`db_agents.memory_id`** (`:578`) and **`db_agents.default_memory_id`**
(`:639`) — 349 `memory_id` tokens across the crate. The `ALTER TABLE
db_agent_definitions ADD COLUMN memory_id` at `:999` is a legacy adoption entry
for the retired table, not a live schema. (An earlier draft named the retired
tables; corrected per Codex P2 on #3131.)

Deferred deliberately: `migrations.rs` has **no `RENAME COLUMN` precedent** —
column changes are done by `ADD COLUMN` + backfill (`:991-1057`). Renaming would
require add-column + backfill + read-both-for-a-release, or a table rebuild. The
names are invisible to users and the payoff is small relative to the risk. Revisit
only if a table rebuild is happening for another reason.

## 6. Migration mechanics

No table rename is required (`db_bundles` is already correct). For reference, the
established mechanism — should a future phase need it — is
`migrations.rs:365-380`:

```rust
const LEGACY_TABLE_RENAMES: &[(&str, &str)] = &[
    ("db_memories", "db_memory_bundles"),   // :371
    ("db_memory_bundles", "db_bundles"),    // :379  <- Phase 4a precedent
];
```

Three properties of that precedent matter and must be preserved by anything that
follows it:

1. **Ordering is load-bearing.** Entries chain within one pass, so an ancient DB
   migrates `db_memories → db_memory_bundles → db_bundles` in a single run.
2. **Indexes must be dropped separately** (`LEGACY_INDEX_DROPS`, `:386-398`) —
   `ALTER TABLE … RENAME` keeps index names, which then collide with the flat
   `CREATE INDEX` DDL. Both the plain and shared-store (`idx_ss_*`) variants must
   be listed.
3. **Forward-only.** Old names remain as adoption entries permanently; there is
   no down-migration.

## 7. Risks

### 7.1 Churn volume
~150 `bundle_memory_*` + ~159 `Memory` + ~238 brain + ~184 frontend `Memory`
tokens. Mostly mechanical, but large enough that Phase 1 should be one
reviewable PR per language side rather than a single sweep.

### 7.2 Agent-facing `preset` surface
`PresetList`/`PresetGet` MCP tools and `/api/v1/agent/preset/list` still speak
"preset" and read the `presets` key that `bundle_list_impl` emits for exactly
that reason (`mod.rs:757` comment says so). Dropping the alias without retiring
those first breaks running agents. Phase 3 must sequence this explicitly.

### 7.3 ABF keys are silently lossy
Import performs **no unknown-key warning and no version negotiation** —
`$schema` is never parsed for dispatch, and `version` is only checked against
`0.1.x`. Shape detection is structural. Renaming any `components.*` key would
silently drop data from existing exported bundles. §4 keeps all keys unchanged,
which sidesteps this entirely — but it constrains any future format work.

### 7.4 Pre-existing exporter/importer version inconsistency
The exporter emits `$schema: …/v0.2/…` while hardcoding `version: "0.1.0"`, and
the importer warns on anything not `0.1.x`. Not caused by this spec, but any
future version bump will trip it. Noted for whoever does ABF v0.3.

## 8. Non-goals — deferred to a follow-on portability spec

The following are real requirements, agreed 2026-09-09, and deliberately kept
out of this spec so that naming lands first. They belong together in one
follow-on spec ("instruction and memory portability"):

- **Export includes Memory — already implemented for the agent-scoped path.**
  `bundle.export_for_agent` reads the agent's native-memory files and splices
  `components.memory` plus the referenced files into the manifest
  (`splice_memory_component`, `app_api/bundle.rs:439-468`; tested at `:2761`),
  and `bundle.import_for_agent` consumes it. Only the agent-less `bundle.export`
  omits memory, which is correct by design — a bundle detached from any agent has
  no memory to carry. An earlier draft of this spec claimed export never emits
  memory; that was wrong (Codex P2 on #3131). The remaining question for the
  follow-on spec is narrower: whether the agent-less path should *warn* that
  memory was not included, mirroring the import side's existing
  `MEMORY_COMPONENT_IGNORED_WARNING`.
- **Track Project Instructions.** Read, snapshot, version, and export the repo's
  own `CLAUDE.md`/`AGENTS.md`/etc. Read-only — never overwrite (§3).
- **Memory location invariant.** Native memory must be written to one
  AgentMux-controlled location per agent. Extends the durability +
  location-consistency invariant from `SPEC_NATIVE_MEMORY_DURABLE_SYNC_2026_08_07.md`.

The unifying goal: tight control over everything an agent consumes as
instructions or memory, including portability.

Also out of scope here:

- Fixing empty curated memory / adding a scaffold or emptiness indicator
  (though §3's Project Instructions finding reframes what "empty" means).
- Mid-session propagation of bundle edits to running agents.
- The post-compaction digest decision.
- Any change to what is *composed* into the startup file, or in what order.

This spec changes names and surfaces only. Behaviour is intended to be
bit-identical, and Phase 1–3 should produce no diff in `AGENTMUX_MEMORY.md` output
for any existing agent.

## 9. Decisions needed before Phase 1

1. **Tab label** — is the Bundles tab called **"Bundles"** (honest, matches the
   noun) or does it keep **"ABF"** (format-as-brand)? §5 assumes "Bundles" with
   ABF reserved for import/export.
2. **Phase 3 scope** — retire the `*memory` commands outright, or keep them as
   deprecated aliases for one release? Outright is cleaner; aliases are safer if
   anything outside this repo speaks that surface.
3. **`viewType = "memory"`** — migrate the persisted value, or freeze the wire
   value and rename only the type?
4. **Sequencing** — Phases 1+2 are safe and could land immediately. Phase 3 (wire)
   and Phase 4 (UI) are larger. Land 1+2 first and re-evaluate, or commit to all
   four up front?
