# SPEC: Bundle-as-container v2 (GH issue #2024, item 3)

## Background

`specs/PROPOSAL_COMPOSABLE_AGENT_MODEL_2026_06_30.md` describes a target
where a Bundle is an *optional named collection of references* to
independently-owned primitives (Account, Memory, MCP Server, Skill,
Brief) — "reference, don't copy." `specs/SPEC_V1_MCP_SKILLS_PRIMITIVES_2026_06_30.md`
shipped v1 of that: MCP servers and Skills became standalone,
referenceable rows (`db_mcp_servers`/`db_skills`) with **agent-level**
ref tables (`db_agent_mcp_ref`/`db_agent_skills_ref`). Its own §9
explicitly deferred the **bundle-level** equivalent to "v2" — that's
this doc.

Issue #2024 has three items; item 1 (fold Identities into Accounts) is
shipped and closed. Item 2 (fold Brain tab into Bundles) is explicitly
**not decided** — the issue's own text says it needs a separate product
call, not implementation, and this doc does not touch it. **Item 3**
(bundle-as-container) is "genuinely new work, no shortcuts available" —
this doc scopes and sequences that work.

## Current state (verified against live code — see full research report
in the implementing session; summarized here)

- `db_bundles.mcp_servers` is a JSON array of **full inline server
  config objects** — zero connection to `db_mcp_servers`.
- `db_bundles.skills` is a JSON array of **skill id strings** —
  semi-referential by convention only, no FK, no ref table, no
  referential-integrity check.
- `db_bundles` has **no account relationship at all**, not even inline
  JSON — the `.abf` export's `accounts/requirements.json` is a
  *heuristic guess* inferred from MCP `env` key names, never a real
  link (`db_agent_identity_links` is agent-scoped, PK'd
  `(agent_id, provider)`, no `bundle_id`).
- **Decisive finding**: `write_agent_config_files` (`agent_open.rs`,
  the sole config-materialization path at spawn) resolves MCP servers
  via `wstore.mcp_server_list(&agent.id)` and skills via
  `wstore.effective_skills(&agent.id)` — both purely **agent**-ref-
  scoped (+ globals). `agent.memory_id` (the bound bundle) is **never
  read** by either call. A bundle's `mcp_servers`/`skills` columns are
  write-only round-trip data for `.abf` export/import today — inert at
  runtime.
- The Bundle editor UI (`memory-manager.tsx`) exposes only Name /
  Description / Provider / Model vendor / Instructions — its own
  hint text says MCP/skills/context-files "will be editable here in a
  follow-up."
- "Memory" (native/learned, the proposal's actual Memory primitive) has
  **no standalone referenceable row** — `db_agent_native_memory` is
  PK'd `(agent_id, filename)`, agent-owned by construction. Unlike
  Accounts/MCP/Skills, there's no existing ref-table pattern to mirror;
  building this would be inventing a new primitive from scratch, not
  extending one.

## Scope decisions for this delivery

**In scope (v2a — this delivery plan):**
- Bundle ↔ MCP Server references (mirrors the existing, fully-shipped
  agent-level pattern exactly).
- Bundle ↔ Skill references (same).
- The launch-time wiring gap (§ above) — without this, new ref tables
  would be exactly as inert as today's JSON blobs.
- Bundle editor UI to add/remove MCP servers and skills, reusing the
  existing `AgentMcpModal`/`AgentSkillsModal` read-only-list-with-bind-
  toggle pattern, scoped to `bundle_id`.

**Explicitly deferred, not attempted in this delivery — flagging back
rather than assuming:**
- **Bundle ↔ Account.** Unlike MCP/Skills, there's no existing
  agent-level ref-table pattern to clone here that fits — the open
  question is whether a bundle-level account ref should be a **real
  FK to `db_accounts.id`** (a live credential — meaningfully higher
  trust/security weight than referencing a stateless MCP config or
  skill prompt; a bundle is reusable across many agents, so a live
  credential ref would mean "every agent using this bundle can use
  this specific account," a much bigger blast-radius decision) or an
  **abstract requirement** ("this bundle needs a `claude`-provider
  account," no live binding — closer to what `accounts/requirements.json`
  already infers heuristically today). This is a real security/product
  decision, not an implementation detail — deferring rather than
  picking one silently.
- **Bundle ↔ Memory (native/learned).** No standalone/referenceable
  memory primitive exists yet at all — building one is materially
  larger scope than "add a bundle-level ref table," and the parent
  proposal itself (§9) still lists this as an open, unresolved
  question ("merge into Brief vs. migrate into native Memory store").
  Out of scope here; native memory stays agent-owned as today.
- Renaming the `Memory` Rust struct to `Bundle` (pre-existing naming
  debt, `bundle_export.rs`'s own doc comment flags it) — cosmetic,
  decoupled from this work, not renaming mid-feature to keep diffs
  reviewable.
- Deprecating/migrating the existing inline `db_bundles.mcp_servers`/
  `.skills` JSON columns — kept as-is for this delivery (still used by
  `.abf` export/import's on-disk layout, which has its own real
  `mcp/*.server.json`/`skills/*/SKILL.md` files independent of any
  live ref table). Revisit once the new ref tables have shipped and
  proven out.

## Delivery plan

1. **Backend: `db_bundle_mcp_ref` + `db_bundle_skills_ref` schema +
   Store methods.** New migration (next available `m00NN`), new ref
   tables mirroring `db_agent_mcp_ref`/`db_agent_skills_ref` exactly
   (`ON DELETE CASCADE` both directions + explicit purge-on-delete in
   Store methods, matching the v1 spec's note that SQLite FK
   enforcement isn't guaranteed everywhere). Store methods mirror
   `mcp_server_list`/`skill_list`'s shape but keyed by `bundle_id`:
   `bundle_mcp_list/bind/unbind/is_accessible_to`,
   `bundle_skill_list/bind/unbind/is_accessible_to`.
2. **Backend: `mcp.catalog.bind_to_bundle`/`unbind_from_bundle`/
   `list_for_bundle`, same for `skill.catalog.*`.** Follows the
   existing `bind_to_agent`/`list_for_agent` RPC naming convention
   exactly (no new verb scheme invented).
3. **Backend: launch-time wiring.** `write_agent_config_files` (via
   `build_mcp_config_from_refs` and `effective_skills`, or a bundle-
   aware wrapper around them) must resolve `agent.memory_id` → bundle
   → the bundle's referenced MCP/skills, and union that into the
   existing agent-ref ∪ global resolution. This is the step that makes
   the feature real rather than cosmetic — sequenced right after the
   ref tables exist so nothing ships "connected but inert."
4. **Frontend: Bundle editor MCP/Skills tabs.** Extend
   `memory-manager.tsx`'s bundle detail view with the two new sections,
   reusing `AgentMcpModal`/`AgentSkillsModal`'s pattern scoped to
   `bundle_id` instead of `agent_id`.

Each step lands as its own PR, same TDD discipline as the rest of this
session's work (read the actual code, write a regression test, verify
it fails against the unfixed code, run the full relevant suite,
changeset, PR referencing #2024 noting which delivery-plan step it is).

## Open questions for the human operator (not resolved by this doc)

1. Bundle ↔ Account: real FK to a live credential, or an abstract
   per-provider requirement? (recommendation: requirement — lower
   blast radius, matches what export already infers; but this is a
   security-relevant call, flagging rather than deciding)
2. Is Memory (native/learned) wanted as a bundle-referenceable
   primitive at all, given no such standalone concept exists today —
   or should the proposal's open §9 question (merge into Brief vs.
   promote to a real primitive) be resolved first, elsewhere?

Proceeding with steps 1-4 (MCP + Skills only) now; will not start on
Accounts or Memory without an answer to the above.
