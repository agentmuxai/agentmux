# Report: Armory Architecture & Naming Review

**Date:** 2026-07-23
**Author:** Agent2
**Scope:** The app-wide Armory pane (`view:"armory"`) and the per-agent "Agent
setup" modal, across both current implementation and the spec history that
produced it. Triggered by a request to reassess whether the Armory's
architecture should have been handled differently, and whether the "Accounts"
tab should be renamed to "Connectors."

**Bottom line up front:**
- The Armory was built the right way, incrementally, with each phase captured
  in a spec and a real architecture doc (`ARCHITECTURE_ARMORY_2026_07_20.md`)
  written once the shape stabilized. That doc's own stated principle —
  "Armory holds *reusable resources*; per-agent *bindings* to them live
  outside Armory" — is sound and is followed by 4 of 5 primitives.
- The rough edges are real but narrow and traceable to specific decisions,
  not a systemic design failure: one naming inversion left over from a
  label-only rename, hand-duplicated (not shared) model code across two
  scopes for two primitives, one primitive (Startup) that's structurally a
  hole rather than a peer primitive by design, and — the one item worth
  fixing, not just documenting — **a genuine bug**: the per-agent "Accounts"
  tab in Agent setup writes to a database column nothing reads at agent
  launch time.
- **Do not rename "Accounts" to "Connectors."** A draft spec already reserves
  "Connectors" for a different, adjacent concept (preloaded MCP server
  integrations), and industry precedent for this exact UI slot converges on
  "Connections" / "Connected accounts," not "Connectors." See §4.

---

## 1. Current architecture

### 1.1 The app-wide Armory pane

`frontend/app/view/armory/armory-view.tsx` (118 lines) is a thin rail-plus-
visibility shell with **zero business logic**. It mounts five independent,
self-contained manager components simultaneously and toggles visibility via
CSS (`is-hidden`), never remounting — "instant switching, no re-fetch,
cross-tab consistency via WPS `*:changed` events instead of remount-on-select"
(the file's own comment). Each manager owns its own model and RPC calls:

| Rail label | Component | Model | Backend |
|---|---|---|---|
| Accounts | `AccountsManager` | `IdentityViewModel` | `*IdentityAccountCommand` RPCs → `db_accounts` |
| Memories | `GlobalBrainManager` | `GlobalBrainViewModel` | Same `bundle_*` RPCs as Bundles, filtered to `is_global` |
| Skills | `SkillManager` | `SkillCatalogModel` | `skill.catalog.*` App API → `db_skills` |
| MCP Servers | `McpManager` | `McpCatalogModel` | `mcp.catalog.*` App API → `db_mcp_servers` |
| Bundles | `MemoryManager` | `MemoryViewModel` | `listmemories`/`upsertmemory`/etc. → `db_bundles` |

This is a legitimately good pattern: composition over a monolith, each
primitive independently testable and independently owned. Three of the five
(Skills, MCP Servers, Bundles) additionally share a generic
`primitive-list-detail.tsx` layout component, so the "list-or-detail, never
both" interaction is consistent across them without being copy-pasted.

**One real wart:** the rail's own `ArmorySection` identifiers don't match
their labels — `id: "brain"` renders as **"Memories,"** and `id: "memories"`
renders as **"Bundles."** This is direct residue of the Phase 5 rename (§3)
that changed labels but not the underlying string literals, and it actively
misleads anyone reading the code later (`section() === "memories"` finds the
*Bundles* pane, not the tab labeled "Memories").

### 1.2 The per-agent "Agent setup" modal

`AgentSetupModal.tsx` (143 lines) is the per-agent analogue — a tab shell
around five child components, opened from a single "vault" icon that (per its
own comment) replaced two earlier separate icons:

| Tab | Component | Backend relationship to its Armory counterpart |
|---|---|---|
| Accounts | `AgentIdentityModalPanel` | **Different concept, different table** — see §2 |
| Memories | `AgentNativeMemoryModal` | Not related to Bundles at all — reads/writes on-disk `~/.claude/.../memory/*.md` files |
| MCP Servers | `AgentMcpModal` | Same backend table, **duplicated model code** — see §2 |
| Skills | `AgentSkillsModal` | Same backend table, **duplicated model code** — see §2 |
| Startup | `AgentStartupModal` | Not a CRUD primitive — a `<select>` into Bundles, writing one string value |

**"Memories" means three different things** depending on which of these two
modals you're in and which tab: Armory's "Memories" tab (global-brain slice
of `db_bundles`), AgentSetupModal's "Memories" tab (on-disk markdown files —
a completely different backend), and Armory's "Bundles" tab (the primitive
AgentSetupModal's *Startup* tab actually selects from). A user cannot infer
from the label alone which backend a given "Memories" screen is touching.

---

## 2. Two concrete problems worth naming precisely

### 2.1 Duplicated model code, not shared abstraction (MCP Servers, Skills)

`AgentMcpModel` (per-agent) and `McpCatalogModel` (Armory catalog) define an
identical `McpDraft` interface, an identical `emptyMcpDraft()`, an identical
`draftFromServer()`, and near-identical save/cancel/edit logic — hand-copied
between two files, not shared via a common base class parameterized by scope.
Skills repeats the exact same pattern (`AgentSkillModel` vs.
`SkillCatalogModel`), down to the doc comments admitting it: *"Same shape as
AgentMcpModel — see its doc comment for the is_global / bound_to_agent
details, which apply identically here."*

This isn't catastrophic — the RPC/type layer is shared, so behavior stays
consistent — but any fix to draft validation or error handling has to be
applied twice by hand, and nothing enforces that it actually is. This is
exactly the kind of drift a small `createEntityCrudModel(scope: "catalog" |
{ agentId: string })`-style shared factory would close, following the same
principle the report's architecture-source doc already commits to elsewhere
(reusable resource, scoped binding) — just applied to the *model layer*, not
only the data layer.

### 2.2 Agent setup's "Accounts" tab is a dead write path — this is a bug, not a naming issue

This is the one finding from this review that should be tracked as a bug, not
filed away as an architecture note.

- Agent setup → Accounts (`AgentIdentityModalPanel`) writes provider
  assignments into `AgentDefinition.accounts` — a legacy JSON blob column,
  via `UpdateAgentDefinitionCommand`.
- The actual spawn-time credential resolver
  (`identity/resolver.rs::resolve_bindings_for_instance`) reads **only**
  `db_agent_identity_links`, a separate table written **exclusively** by the
  agent-launch flow (`AgentLaunchModal.tsx` → `linkagentidentity` RPC).
- `ARCHITECTURE_ARMORY_2026_07_20.md` §1 documents this explicitly: *"there
  is no write path from any Armory or agent-pane UI"* into
  `db_agent_identity_links` — meaning the modal in question presents fully
  functional-looking CRUD (pick a provider, assign an account, see it save)
  that has no effect on what credentials the agent actually launches with.
- A companion fix (PR #2239, same day as the architecture doc) corrected the
  **read** side of this gap for Session Context's "Assigned Accounts"
  display, but did not touch the **write** side described here — the modal
  still silently no-ops today.

Net: three UI surfaces currently claim to show or edit "this agent's
accounts" (Armory's Accounts catalog, AgentSetupModal's Accounts tab, and the
read-only `AgentIdentityLinksPanel` on the agent pane's own Identity tab),
across two tables, and only one of the three (the read-only one) reflects
reality. **Recommendation:** either wire `AgentIdentityModalPanel` to write
`db_agent_identity_links` for real, or replace it with a read-only view
identical in spirit to `AgentIdentityLinksPanel` (arguably simpler, since that
panel already exists and works) plus a link to the launch flow where the
binding is actually created. Filing this as a separate follow-up is the right
move — it's a functional bug, and fixing it doesn't require or block any
naming/architecture decision below.

---

## 3. History — was this handled well?

The evolution is unusually well-documented for what it is: nine dated specs
plus one retrospective architecture doc, each phase landing the same day
its spec was written, each building visibly on the last (Trust Center →
Armory rename → Preset→Bundle rename + per-agent modal consolidation →
storage rename → Phase 5 primitive consolidation → responsive layout →
architecture reference doc → Startup Instructions binding). That is a
genuinely healthy pattern: rename churn isolated to its own commit, storage
rename isolated to its own phase, and a real "why does this look the way it
looks" document written once the shape had settled rather than upfront
speculation.

The architecture doc's central design principle — *reusable resource in
Armory, scoped binding lives with the agent* — is coherent and is what Skills,
MCP Servers, and (weakly) Bundles all follow. Where it breaks down is not
because the principle is wrong, but because:

1. **Accounts predates the principle** and was never retrofitted to it — it's
   the oldest primitive (from the Trust Center era) and inherited a
   single-FK-per-provider shape rather than the later join-table pattern.
2. **Startup was deliberately built as an exception**, not an oversight — the
   architecture doc explicitly left the choice open, and the implementation
   (same day) chose the cheapest correct option (a string value in the
   generic `db_agent_content` blob table) over building a fifth full ref-table
   primitive for something that only ever needs one value. That's a
   defensible call, but it means Startup is the only AgentSetupModal tab with
   no catalog of its own — worth documenting as an intentional asymmetry, not
   quietly leaving it looking like a missed spot.
3. **Two renames (Preset→Bundle, Trust Center→Armory) didn't fully
   propagate** — the `preset.*` RPC alias was scheduled for removal "in Phase
   4" per its own source comment and is still registered; the Bundles tab's
   own UI copy still says "New Preset," "Edit Preset," and "Delete this
   preset?" in three separate places.

None of this reads as "should have used a different architecture." It reads
as normal follow-through debt from a fast-moving, well-specced sequence of
incremental renames — the kind of thing that's cheap to clean up now and
progressively more annoying to clean up later. A useful, low-risk next step
(not undertaken as part of this report, since it's implementation not
research) would be a single small pass that (a) fixes the `ArmorySection`
id/label mismatch, (b) finishes deleting the `preset.*` alias and the
"Preset" copy, and (c) extracts the MCP/Skills duplicated model logic into
one scope-parameterized shared model — three independent, low-risk, easily
reviewable changes rather than one large refactor.

---

## 4. Naming: should "Accounts" become "Connectors"?

**Recommendation: no.** Two independent reasons converge on the same answer.

### 4.1 "Connectors" is already spoken for, one tab over

A draft spec (`SPEC_ARMORY_PRELOADED_CREATIVE_MCP_CONNECTORS_2026_07_10.md`,
plus `SPEC_ARMORY_MCP_SERVER_DEFAULT_SEED_CATALOG_2026_07_13.md`) already uses
"connector" / `AppConnector` for a different, unshipped concept: preloaded MCP
server integrations for creative apps (Ableton Live, TouchDesigner, Blender),
scoped to the **MCP Servers** tab, not Accounts. It's draft/unshipped, so
there's no live collision today — but renaming Accounts to "Connectors" now
would plant a landmine for whenever that spec ships: two tabs both plausibly
called "Connectors" for two different things (OAuth/API-key credentials vs.
preloaded MCP integrations), one tab apart in the same pane.

No other repo in the ecosystem (agentmux-cloud, dev-tools,
shared-infrastructure) uses "connector" in this sense — the only other hits
are unrelated (SVG/CSS connector *lines* in diagrams, a GitHub bot literally
named `chatgpt-codex-connector`, and third-party references like Claude's own
"Custom Connectors" feature, cited only as a comparison point in a spec).

### 4.2 Industry precedent for this exact UI slot doesn't favor "Connectors" either

Researched how comparable products label the "per-user list of authenticated
OAuth/API-key connections" screen specifically (as opposed to the different
concept of a pre-built integration adapter):

- **Notion** labels it "Connections" (Settings → Connections).
- **Zapier** uses "Connections" in its own SDK/developer terminology, but in
  its consumer-facing Zap builder the actual flow is "**+ Connect a new
  account**" — i.e. "account," not "connector."
- **"Connectors"** as a term is most established in enterprise
  iPaaS/automation platforms (UiPath, MuleSoft) to mean the *pre-built
  adapter/integration component itself* — closer to what AgentMux's own
  draft MCP-Connectors spec means by the word — not the *end-user's specific
  authenticated instance* of a connection, which is exactly what the Armory
  Accounts tab is.
- The most common pattern across consumer and B2B SaaS for "your specific
  login/token tied to your identity" is **"Connected Accounts"** — which
  also happens to be closest to the term already in use here.

### 4.3 Recommendation

Keep **"Accounts."** It's already the established, documented, code-level
term (`AccountsManager`, `db_accounts`, the docs site glossary), it has no
in-code competing usage, and renaming it wouldn't fix an actual naming
problem — Accounts vs. Identities is already a reasonably clear split
conceptually (Accounts = the vault of credentials; Identities = named
pointers into that vault), it's the *implementation* split (§2.2) that's
broken, not the label.

If a rename is wanted anyway for tone/clarity reasons unrelated to this
review, "Connected Accounts" is the only option that both matches
established industry precedent for this exact screen *and* has zero
collision risk against the pending MCP-Connectors spec. "Connectors" and
plain "Connections" should both be avoided — the former for the direct
collision above, the latter because "Connections" reads ambiguously next to
"Identities" (both could describe a link between an agent and a credential).

---

## Sources

- Codebase: `frontend/app/view/armory/`, `frontend/app/view/agent/components/AgentSetupModal.tsx` and siblings, `frontend/app/view/accounts/`, `frontend/app/view/identity/`, `agentmux-srv/src/server/agent_handlers/identity.rs`, `identity/resolver.rs`, `agentmux-srv/src/backend/storage/migrations.rs`
- Specs: `docs/specs/ARCHITECTURE_ARMORY_2026_07_20.md`, `docs/specs/SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md`, `docs/specs/SPEC_ARMORY_PHASE4_STORAGE_RENAME_COMPLETION_2026_07_12.md`, `docs/specs/SPEC_ARMORY_PHASE5_CONSOLIDATION_AND_SKILL_SEEDING_2026_07_13.md`, `docs/specs/SPEC_ARMORY_RESPONSIVE_SINGLE_PANE_LAYOUT_2026_07_15.md`, `docs/specs/SPEC_ARMORY_ACCOUNTS_NO_MODALS_2026_07_16.md`, `docs/specs/archive/SPEC_RENAME_TRUST_CENTER_TO_ARMORY_2026_07_02.md`, `docs/specs/SPEC_ARMORY_PRELOADED_CREATIVE_MCP_CONNECTORS_2026_07_10.md`, `docs/specs/SPEC_ARMORY_MCP_SERVER_DEFAULT_SEED_CATALOG_2026_07_13.md`
- [Notion: Add & manage connections](https://www.notion.com/help/add-and-manage-connections-with-the-api)
- [Zapier: How to integrate Notion with Slack](https://zapier.com/blog/integrate-notion-with-slack/)
- [UiPath Integration Service — Connectors](https://docs.uipath.com/integration-service/automation-cloud/latest/user-guide/connectors)
- [MuleSoft: Difference Between APIs, Connectors and Integration Applications](https://www.mulesoft.com/api/management/difference-between-apis-connectors-integration-applications)
- [Adobe Sign: Integration keys](https://helpx.adobe.com/vn_vi/sign/developer/integration-key.html)
- [Registry Pattern — GeeksforGeeks](https://www.geeksforgeeks.org/system-design/registry-pattern/)
- [Feature-Sliced Design: The UI Architecture That Won't Break Your App](https://feature-sliced.design/blog/ui-architecture-patterns)
