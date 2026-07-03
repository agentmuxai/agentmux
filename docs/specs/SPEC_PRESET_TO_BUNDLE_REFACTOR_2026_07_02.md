# Spec: Preset → Bundle internal refactor (Composable Agent Model, Phases 2–4)

**Date:** 2026-07-02
**Author:** Agent1
**Governing decision:** `specs/PROPOSAL_COMPOSABLE_AGENT_MODEL_2026_06_30.md` (PR #1861, **merged**). Product owner chose **Bundle** as the collection name; "preset" is retired; the `_bundles` storage collision is removed (not worked around).
**This spec implements Phases 2–4** of that proposal's sequence. Phase 1 (break out **MCP Servers + Skills** as first-class primitives) already shipped (spec #1865, impl #1877 — the `mcp.*`/`skill.*` App API).

> **Terminology (from the proposal, canonical):** the six primitives are **Account, Memory, MCP Server, Skill, Brief**, plus **Bundle** = the optional named *collection* of references. "Preset" → **Bundle**. "Identity" is a *derived view* over an agent's bound Accounts, **not** a stored object. After this refactor, the word "bundle" means exactly one thing: the collection.

---

## Guiding constraints

- **Reference, don't copy.** A Bundle stores primitive **IDs**, not inline JSON.
- **Compatibility window.** Keep `preset.*` / `db_memory_bundles` as **read aliases for one release**; new writes go to Bundle names.
- **No user-visible regression.** The Trust Center IA gains no new required step; direct bindings remain the base, Bundle is optional sugar.
- **Contract-test discipline.** Renaming App API commands changes the rpc-contract surface (`test/contract/rpc-contract.test.ts`) — every phase updates the baselines in lockstep (this is the guard that caught the mcp/skill drift in #1896).

---

## Phase 2 — "Preset" → "Bundle" (concept + UI + App API)

The cheap, high-value, mostly-reversible layer. No storage migration yet — Bundle is introduced as the concept/surface name over the existing store.

### 2.1 App API command rename (`preset.*` → `bundle.*`)

`agentmux-srv/src/backend/rpc_types/commands.rs` (lines 292–296):
| Old | New |
|-----|-----|
| `COMMAND_PRESET_LIST = "preset.list"` | `COMMAND_BUNDLE_LIST = "bundle.list"` |
| `COMMAND_PRESET_GET = "preset.get"` | `COMMAND_BUNDLE_GET = "bundle.get"` |
| `COMMAND_PRESET_UPSERT = "preset.upsert"` | `COMMAND_BUNDLE_UPSERT = "bundle.upsert"` |
| `COMMAND_PRESET_DELETE = "preset.delete"` | `COMMAND_BUNDLE_DELETE = "bundle.delete"` |
| `COMMAND_PRESET_SELF_GET = "preset.self.get"` | `COMMAND_BUNDLE_SELF_GET = "bundle.self.get"` |

- Handlers live in `agentmux-srv/src/server/app_api/preset.rs` → rename file to `bundle.rs`, rename `register_preset_*` → `register_bundle_*`, update `app_api/mod.rs` registration.
- **Alias for one release:** register BOTH the new `bundle.*` handler and the old `preset.*` string pointing at the same closure (or a thin forwarder). Frontend switches to `bundle.*`; `preset.*` stays wired so any external caller / in-flight pane keeps working. Remove the alias next release.
- The handlers still call `id_store.bundle_memory_*` under the hood in Phase 2 (storage rename is Phase 4) — that's fine; the wire name is what users see.

### 2.2 Frontend command bindings + types

- `frontend/app/store/rpc-api/` — rename the `Preset*Command` bindings to `Bundle*Command` targeting `bundle.*`. (After the rpc-api domain split, these live in the appropriate domain file — `identity.ts`/`memory.ts`; grep `preset` under `rpc-api/`.)
- Types: `frontend/app/view/bundle-summary.tsx` already speaks "bundle" for the panel but its `kind` union is `"Identity" | "Preset"` → `"Identity" | "Bundle"` (and per Phase 3, "Identity" as a *kind* goes away — see §3). Title map line 41 `"Presets"` → `"Bundles"`.

### 2.3 User-facing strings (Preset → Bundle)

| File | Change |
|------|--------|
| `frontend/app/view/trust/trust-view.tsx` (line 17) | tab `{ id: "memories", label: "Presets" }` → `label: "Bundles"` (keep internal id `"memories"` in Phase 2; retire in Phase 4). Icon `sliders` → consider `layer-group`/`box` |
| `frontend/app/view/memory/memory-model.ts` (131, 178) | `viewText … "Presets"` → `"Bundles"`; frame-title fallback `"Presets"` → `"Bundles"` |
| `frontend/app/view/agent/components/AgentLaunchModal.tsx` (811, 892, 920) | "Preset" label/aria → "Bundle" |
| `frontend/app/view/agent/components/AgentNewMemoryModal.tsx` (92) | `New Preset` → `New Bundle` (consider renaming the modal component too) |
| `AgentLaunchModal.integration.test.tsx` (176) | `findByLabelText("Preset")` → `"Bundle"` (test asserts the label) |

### 2.4 Contract test + persisted view key

- `test/contract/rpc-contract.test.ts` baselines: move the five `preset.*` entries → `bundle.*` in `KNOWN_REGISTERED_UNDECLARED` (once frontend binds `bundle.*`, they'll be *declared* and drop out — verify direction). Keep `preset.*` aliases acknowledged if they remain registered.
- Persisted pane `view: "memory"` key stays (Phase 2) — the view still resolves; the label change is cosmetic. (Renaming the view key is optional Phase 4 cleanup, gated by a `block.tsx` shim like `trust`/`forge`.)

**Phase 2 deliverable:** one PR. UI + App API say "Bundle"; storage untouched; `preset.*` aliased. `tsc` + `vitest` (esp. contract + AgentLaunchModal) green.

---

## Phase 3 — Account-direct: collapse the identity-bundle layer

The proposal's "one real refactor" (§3.3, §7). Today: `instance → identity_bundle → binding → account`. Target: `instance/bundle → account` directly, with **≤1 account per provider enforced at resolve time**.

### 3.1 Resolver change (the core)

- `agentmux-srv/src/identity/resolver.rs` — change spawn resolution to read the agent's **directly-bound accounts** (+ accounts referenced by any included Bundle) instead of walking `identity_bundle → bindings`. Enforce "one account per provider" at resolution; a conflict is a surfaced validation error (not a silent pick).
- Reconcile with `specs/SPEC_PER_AGENT_IDENTITY_PROVISIONING_2026_06_30.md` (an Account already carries its own `OAuthConfigDir`, so there's no bundle to provision — log in → Account → bind).

### 3.2 Deprecate the identity-bundle App API + UI

- Commands `listidentitybundles` / `getidentitybundle` / `upsertidentitybundle` / `deleteidentitybundle` / `bindidentityaccount` / `unbindidentityaccount` / `listidentitybindings` (commands.rs 164–170, incl. `COMMAND_DELETE_IDENTITY_BUNDLE` at 167) → mark deprecated; keep read paths for the compat window, stop new writes.
- Trust Center: the **Identities** tab becomes a **derived view** ("what is this agent running as" over bound Accounts) — no stored identity-bundle object. Per the proposal IA (§5) there is **no Identities tab** in the target; fold it into the Accounts + Bundle surfaces. (This is a UI change — confirm the exact interim: hide the tab, or convert to read-only derived view for one release.)

### 3.2b Agent-pane header: consolidate to a single icon (product-owner decision 2026-07-02)

Replace the agent pane's **two** title-bar icons (`brain`/"Agent memory" + `id-card`/"Agent identity") with **one `id-card` icon** that opens a **unified per-agent management modal** — a tabbed surface over all of this agent's bound primitives: **Accounts · Memory · MCP · Skills · Briefs · Bundle** (the Armory, scoped to this agent). Details + rationale in `EXPLAINER_COMPOSABLE_MODEL_AND_AGENT_PANE_2026_07_02.md` §4.

Implementation:
- `frontend/app/view/agent/agent-model.ts` `endIconButtons` (~line 141): replace the two-button array with a single `id-card` button (title "Agent setup"/"Manage agent") whose `click` opens the unified modal.
- Retire `_openMemoryModal` / `_openIdentityModal` (agent-model.ts:36-37, agent-view.tsx wiring) in favor of one `_openAgentSetupModal`, or reuse the Armory's tabbed manager component filtered to the agent's bindings with "add from library / create new" deep-linking to the full Armory.
- Keep the icon as `id-card` (reads as "who/what this agent is + carries"); not `brain`, not `key`.

### 3.3 Data backfill

- Migrate existing `db_identity_bundles` + `db_identity_bindings` rows into **direct account bindings** on the owning agent/Bundle before the tables are dropped in Phase 4. A one-shot migration reads each bundle's bindings and writes equivalent direct bindings.

**Phase 3 deliverable:** one PR (resolver + backfill + UI derived-view). Heaviest behavior change — needs the identity/keychain reconciliation tracked in **issue #1624** ("Reconcile per-agent keychain with the live identity-bundle system") to land compatibly; coordinate.

### 3.4 RESOLVED product decisions (2026-07-02)

These override the corresponding open items in `PROPOSAL_COMPOSABLE_AGENT_MODEL_2026_06_30.md`:

- **§9.1 / derived-Identity UX — RESOLVED:** the agent-pane header consolidates to a **single `id-card` icon** opening a unified per-agent management modal (see §3.2b); the standalone "Identities" concept folds in as the **Accounts** tab. "Identity" remains a derived view only.
- **§3.4 / §9.7 — retire the always-on `CLAUDE.md`? — RESOLVED: NO. Retain `CLAUDE.md`.**
  - **Why:** the Claude CLI **natively auto-loads `CLAUDE.md`** from the agent working directory as its standing project instructions. AgentMux already assembles it from `soul` + `agentmd` + `memory` + skills index (`agentmux-srv/src/backend/agent_config.rs:28,55-96`). Retiring it would break standing-instruction delivery for Claude agents — there is no equivalent always-on channel.
  - **Therefore:** `soul` / `agentmd` / the static `memory` blob **stay in `CLAUDE.md`** (do NOT migrate them into Skills-only). The always-on instruction blob is kept.
  - **Brief** stays defined as *the first message* (kickoff payload), but it is **additive to `CLAUDE.md`**, not a replacement for standing instructions. **Skills** remain on-demand modules, but they are **not** the sole home for instructional content — `CLAUDE.md` remains the standing-instruction home.
  - **Impact on this refactor:** none of the Phase 2–4 storage/naming work depends on retiring `CLAUDE.md`; this decision simply removes the "migrate soul/agentmd → Skills / retire CLAUDE.md" strand from the composable-model rollout. The `agent_config.rs` CLAUDE.md assembly stays as-is.
  - **Note:** `CLAUDE.md` is Claude-specific; other CLIs use their own native context files (e.g. `AGENTS.md`, `GEMINI.md`). "Retain CLAUDE.md" generalizes to "keep writing each provider's native standing-instruction file."
- **§9.6 / Policy primitive — RESOLVED: YES, it's a distinct 7th primitive — but DEFERRED to a follow-on phase (call it Phase 5), NOT part of Phase 3.**
  - **Why first-class:** hooks + `.claude/settings.json` permissions are a *trust decision* (allow/deny tool rules, hook execution) — the same class as Accounts and MCP servers. The model's thesis is that security-sensitive config gets explicit review, not burial in agent config. So Policy earns first-class, reviewable, shareable status in the target model, surfaced in the Armory + the per-agent setup modal.
  - **Why deferred:** it has zero dependency on Phase 3's resolver/identity work, and Phase 3 is already the heaviest phase. Bundling Policy in would expand scope and risk for no sequencing benefit.
  - **Therefore Phase 3 leaves hooks + `.claude/settings.json` writing EXACTLY as today** (no behavior change). Policy is broken out later: a `db`-backed Policy primitive + Armory surface, referenced by Bundles/agents like the other primitives, with an ownership/global guard (§6). Track as a follow-up; not gating Phase 3.

**Phase 3 is now fully specced and unblocked** — all §9 decisions resolved (Policy explicitly deferred).

---

## Phase 4 — Storage migration (drop `_bundles`)

Now that "bundle" means one thing and the identity-bundle layer is gone, rename the tables to shed the misleading suffix.

### 4.1 Table renames

| Old | New | Note |
|-----|-----|------|
| `db_memory_bundles` | **`db_bundles`** | it holds the collections, never memory |
| `db_identity_accounts` | **`db_accounts`** | the Account primitive |
| `db_identity_bundles` | **removed** | collapsed in Phase 3 (data already backfilled) |
| `db_identity_bindings` | **removed / renamed** | replaced by direct account bindings (Phase 3) |

### 4.2 Migration mechanics (reuse the existing pattern)

`agentmux-srv/src/backend/storage/migrations.rs` already has `LEGACY_TABLE_RENAMES` (line 78) + `adopt_legacy_table_names` (rename-on-open) + a paired index-rename step — the v11 `db_identities→db_identity_bundles` / `db_memories→db_memory_bundles` rename used exactly this. **Add new pairs**:
```rust
// Composable-model rename (drop _bundles):
("db_memory_bundles",  "db_bundles"),
("db_identity_accounts","db_accounts"),
```
Plus: rename the associated indexes with their tables — `idx_memory_bundles_is_blank` (line 269) → `idx_bundles_is_blank`, `idx_identity_bundles_is_blank` (line 239, dropped with the table), and the `db_identity_accounts(provider)` index (line 218) → `db_accounts(provider)`. (Note: `is_global` is a *column* on `db_memory_bundles`, not an index — no index to rename there.) Then update every **FK reference**:
- `db_identity_bindings … REFERENCES db_identity_accounts(id)` → `db_accounts(id)` (or removed with the table)
- `agentmux-srv/src/registry/schema.rs:37,39` and `agentmux-srv/src/backend/rpc_types/instance.rs:34,70` — the FK/reference strings to `db_identity_bundles` / `db_memory_bundles`.

### 4.3 Rust symbol renames

- `agentmux-srv/src/backend/storage/memory_bundles.rs` → rename file to `bundles.rs`; `bundle_memory_list/get/upsert/delete/reorder` → `bundle_list/get/upsert/delete/reorder`. Update every caller: `agent_handlers/{identity,memory,session}.rs`, `app_api/bundle.rs` (from Phase 2), `m0011_shared_store_backfill.rs`, `registry/*`.
- Frontend: `IdentityBundle` type + remaining `*IdentityBundleCommand` RPCs removed (identity-bundle layer gone); `.bundle-manager-*` CSS classes in `trust-view.scss` → `.bundle-rail`/`.bundle-tab-bar` (or fold into the Armory rename if that lands first).

### 4.4 Remove the Phase-2 aliases

Drop the `preset.*` command aliases and any `db_memory_bundles` read-alias once this ships (it's the "one release later" removal).

**Phase 4 deliverable:** one PR (storage migration + symbol renames + alias removal). Ship after Phase 3's backfill has been in a release long enough that no `db_identity_bundles` rows remain unmigrated.

---

## Docs to update (across the phases)

- **`CLAUDE.md`** line 29 (`db_memory_bundles, not yet globalized`) → `db_bundles`; line 171 ("Backend names stay `db_memory_bundles` / `bundle_memory_*`") → rewrite to the new names + note "preset" retired in favor of "Bundle". Update the "Not widgets" **Presets** row → **Bundles**.
- `docs/specs/SPEC_MEMORY_IDENTITY_ARCH_2026_06_19.md` — reconcile with the composable model (or supersede).
- Leave historical specs (`SPEC_TRUST_CENTER_*`, older bundle specs) as record.

## Interaction with the Armory rename (`SPEC_RENAME_TRUST_CENTER_TO_ARMORY_2026_07_02.md`)

Orthogonal but touches the same files. If Armory lands first, this refactor operates on `view/armory/` instead of `view/trust/` and the `.bundle-manager-*` cleanup can happen there. Sequence Armory → Phase 2 to avoid double-touching `trust-view.tsx`, or do Phase 2's tab-label change inside the Armory PR. Coordinate to minimize conflicts.

## Sequencing & risk

| Phase | PR | Risk | Gate |
|-------|-----|------|------|
| 2 | Preset→Bundle UI + App API (+ aliases) | **Low** (reversible, aliased) | tsc + vitest (contract, AgentLaunchModal) |
| 3 | Account-direct resolver + identity-bundle collapse + backfill | **High** (behavior + data) | resolver tests; coordinate with #1624; product decision on derived-Identity UX + Policy primitive |
| 4 | Storage `_bundles` drop + symbol renames + alias removal | **Medium** (SQLite migration) | migration test on a populated db; adopt-legacy-on-open verified; contract baselines |

**Do them in order, one release apart where a compat window is promised.** Phase 2 delivers the visible "Preset is now Bundle" immediately; 3–4 are the internal cleanup that makes the name honest end-to-end.

## Open product decisions carried from proposal §9 (must confirm before Phase 3)

1. Derived-Identity UX: hide the Identities tab vs. read-only derived view for one release.
2. **Policy** primitive for hooks + permissions (`.claude/settings.json`) — introduce or defer.
3. Static `memory` content blob (in today's CLAUDE.md) — merge into Brief vs. native Memory.
4. Whether `soul`/`agentmd` → Skills migration rides this refactor or a separate one (proposal §3.4 stance: retire the always-on CLAUDE.md instruction blob).

These affect Phase 3's surface; Phase 2 and Phase 4-storage can proceed without them.
