# Architecture rethink: making ABF mandatory ("every agent must have an ABF")

**Date:** 2026-08-14
**Status:** DECISIONS RESOLVED, NOT YET IMPLEMENTED. Extended same-day (§7)
with a portability idea: harness + model as readonly ABF fields, so an ABF
becomes the portable unit instead of "the agent." All open questions (§2,
§5, §7.4) resolved as of this revision — §7.5 is the build order.
**Author:** AgentY (agenty-0629j), at operator request
**Related:** `docs/specs/ARCHITECTURE_ARMORY_2026_07_20.md`,
`docs/specs/archive/SPEC_PRESET_TO_BUNDLE_REFACTOR_2026_07_02.md`,
`docs/specs/archive/SPEC_BUNDLE_MANAGEMENT_2026_05_22.md`,
`docs/specs/SPEC_ABF_V0_2_PROVIDER_AWARE_COMPONENTS_AND_NATIVE_MEMORY_2026_08_10.md`,
PR #2505 ("model vendor as a concept distinct from harness", merged
2026-08-10), PR #2558 ("expose model vendor / custom endpoint UI + add
Antigravity harness", merged 2026-08-13). NOT related despite superficially
similar names:
`SPEC_HARNESS_MODEL_DECOUPLING_AND_ANTIGRAVITY_2026_08_09.md` /
`PLAN_HARNESS_MODEL_APPLICATION_INTEGRATION_2026_08_09.md` — these only exist
on an unmerged, stale branch (`feat/harness-model-decoupling-antigravity`),
self-declare "Approved"/"Ready for Execution" without real review, and
propose a hardcoded `--yolo` auto-approve flag that was explicitly rejected
when PR #2505 independently re-derived and shipped the sound part of the
same core idea. Treat as historical, not authoritative.

## The ask

> "First, we want to formalize ABF. Every agent must have an ABF."

Today ABF (bundle) is **optional** per agent — an agent can exist, launch, and
run its whole life bound to nothing. This doc lays out the current
architecture, why "mandatory" doesn't fit cleanly onto it as-is, and the
concrete decisions needed before any code changes.

## 1. Current state (verified against the code, 2026-08-14)

### 1.1 Data model — there is no real link, only a convention

`db_bundles` (`agentmux-srv/src/backend/storage/migrations.rs:327-345`) is a
**standalone table** — no foreign key to an agent at all:

```
db_bundles(id, name UNIQUE, description, is_blank, is_global, provider, model,
  instructions, instructions_by_provider, context_files, mcp_servers, skills,
  sort_order, created_at, updated_at)
```

The link runs the other way, and it's soft: `db_agents.memory_id` and
`db_agent_instances.memory_id` (`migrations.rs:417`, `:356`) are both
`TEXT NOT NULL DEFAULT ''`. Empty string means "unbound" **by convention**,
not by any constraint the database enforces. A hardcoded singleton row
`id='blank'` is seeded at bootstrap and can never be deleted
(`memory_bundles.rs:200-204`) — it's the fallback every resolver eventually
lands on.

**Implication:** "mandatory" can't be bolted on as a `NOT NULL` constraint —
`memory_id` is already `NOT NULL`, it's just allowed to be `''`. Enforcement
has to happen in application logic (validation at write time) and/or by
changing what counts as "having an ABF."

**Correction found during implementation (2026-08-14), more significant
than §1.1's provider/model correction:** the earlier draft of this section
mis-cited `agent_handlers/memory.rs:74` — that line sets a **bundle's own**
`Memory.id` in the legacy `upsertmemory` handler, unrelated to an agent's
FK. The actual FK lives in exactly two places: `db_agent_instances.memory_id`
(a launched instance) and `db_agents.memory_id` — and `db_agents`' own
schema comment says it plainly: *"Bindings (was on instance — only
meaningful when `is_template=0`). For template rows these stay empty."*
**`db_agent_definitions` — the durable "my agent" row users actually
think of as "the agent" — has no `memory_id` column at all.** There is
currently no way to bind a bundle to an agent *definition*, only to a
specific *launch* of one. This means §3 below (definition-time
provisioning) needs a genuine new column, not just enforcement logic on an
existing field — revised in §3.1.

### 1.2 Agent creation → bundle binding happens late, and inconsistently

Three separate creation paths — revised after the correction above, since
none of them actually operate at the definition level for `memory_id`
today (there's no column to write to):

| Path | Where | Binds a bundle to...? |
|---|---|---|
| `agent.define` (bare definition) | `agent_handlers/*` | Nothing — `db_agent_definitions` has no `memory_id` column to set |
| "+ New from template" | `AgentCreateFromTemplateModal.tsx:162-171`, `agent_handlers/template.rs:180` | The created **instance** (`AgentInstance.memory_id`) — auto-picks first non-blank bundle *if one exists*, `canSubmit` doesn't require it |
| Launch modal (ad-hoc/continuation launch) | `AgentLaunchModal.tsx:401`, `agent_handlers/instance.rs:93` | The launched **instance** — `canSubmit` genuinely blocks on `memoryId() !== ""`, picker filters out the blank singleton |

So the one place that actually enforces a real (non-blank) bundle is the
*launch* modal — which fires per-instance, not per-agent-definition. An
agent's `db_agents` row (the durable definition) can be created and live
indefinitely with `memory_id=''`; only the ephemeral instance launch forces a
real pick, and even that can be bypassed via the template-clone path.

### 1.3 No UI surfaces "this agent has no bundle" at all

`frontend/app/view/bundle-summary.tsx` — the Identity/Memory tab in the agent
settings pane was deliberately stripped of CRUD in
`SPEC_BUNDLE_MANAGEMENT_2026_05_22.md` and now just points at Armory. Its own
comment flags a **DATA GAP**: the panel can't even resolve which bundle is
bound to the current agent (`memory_id` lives on the `AgentInstance` row,
unreachable from where that component renders). So today, whether an agent
has a real bundle, the blank singleton, or nothing at all, the user sees the
same generic pointer either way. There's no "your agent has no ABF, fix this"
surface to build enforcement on top of — it has to be built from scratch.

### 1.4 No backfill migration exists yet

Naming convention is `m00NN_<description>.rs`
(`agentmux-srv/src/migrations/`), currently through `m0020`. The closest
precedent for a mandatory-bundle backfill is `m0020_agent_color_backfill.rs`
— idempotent, hash-deterministic, one-time backfill over every existing
agent def. Its own doc comment warns it must attach the **global/shared
registry**, not just local SQLite, or it silently misses agents defined on
other channels — the same trap applies here.

### 1.5 Validation is structural and advisory, not a gate

`bundle_validate.rs` checks internal well-formedness (provider keys resolve,
context-file paths are safe, MCP/skills JSON is sane) but explicitly allows
saving with errors present, and an **empty bundle validates cleanly**
(`empty_bundle_is_valid_with_no_issues` test). There's no existing concept
of "this bundle doesn't count as a real ABF."

## 2. The central open question this doc can't resolve alone

**What does "every agent must have an ABF" actually mean?**

**(a) Weak reading — no agent should be unbound.** Every agent's `memory_id`
must resolve to *some* real bundle row — the shared `blank` singleton
satisfies this. This mostly just closes the `memory_id=''` gap and unifies
the three creation paths to always write *something* resolvable.

**(b) Strong reading — every agent has its own bundle.** The `blank`
singleton doesn't count; agent creation auto-provisions a fresh, per-agent
`db_bundles` row (empty instructions/context to start, but a real row the
agent owns and can edit without affecting anyone else). This is a bigger
change: it turns bundle-per-agent into the default relationship instead of
opt-in, and raises follow-on questions — does deleting an agent delete its
bundle? Can it still be shared/exported? What happens to the now-mostly-
unused `blank` singleton?

Given ABF's own stated identity ("the agent's provider-agnostic config
collection") and that today's `blank` singleton is explicitly a fallback for
*unconfigured* agents, reading (b) fits the stated intent ("every agent must
have an ABF") more literally than (a) — (a) is closer to "every agent must
not be broken," which is already almost true today via the blank fallback.

**Resolved: (b).** Confirmed implicitly — §7's entire premise (harness/model
as *per-ABF* readonly fields) only makes sense if every agent has its own
distinct bundle to carry them on; a shared/optional-bundle world (a) can't
support that extension at all. The operator built directly on top of (b)
without objection across the whole §7 discussion, so treating it as settled
rather than still-open.

## 3. Proposed architecture (assuming reading (b))

### 3.1 Schema (revised per §1.2's correction)

Genuine new schema needed — not just an invariant on an existing field:

- **Add `memory_id TEXT NOT NULL DEFAULT ''` to `db_agent_definitions`.**
  This is the column that doesn't exist today; without it there is nowhere
  to durably bind a bundle to an agent's definition (only to a specific
  launched instance).
- **Relax `db_agents.memory_id`'s current meaning.** It already exists on
  `db_agents` but is documented as "only meaningful when `is_template=0`"
  (i.e. template/definition rows leave it empty by convention, same soft
  rule as the rest of this doc's §1.1 finding). Since `db_agents` is the
  eventual consolidated table (dual-write today per its own migration
  history, `OBJECT_SCHEMA_VERSION` v4 comment), no column addition needed
  there — just start writing definition-level bundle bindings into it too,
  and stop treating template rows' `memory_id` as always-empty.
- **Superseded (2026-08-15, correction — this decision as originally
  written did NOT survive contact with the code, and a P2 finding on PR
  #2587's review caught the doc/code mismatch): the plan below was "do
  NOT wire this into the `db_agents` dual-write path... `db_agents` isn't
  read from yet for this purpose anyway."** That premise was wrong: while
  implementing the backfill migration, `m0021`'s first run kept
  re-processing the same agent on every rerun
  (`UNIQUE constraint failed: db_bundles.name`) because
  `agent_def_list()` — the function `m0021` (and every creation-path
  provisioning site) uses to enumerate/read agents — turned out to
  **already read from the consolidated `db_agents` table, not
  `db_agent_definitions` directly** (an earlier, undocumented partial
  Phase 3b reader-flip). A definition-level field invisible to that table
  is invisible to every consumer of `agent_def_list()`, which is most of
  this codebase.
  **What actually shipped instead:** a SECOND new column,
  `db_agents.default_memory_id` — deliberately a distinct name from the
  existing instance-scoped `db_agents.memory_id`, to avoid exactly the
  "which wins" ambiguity this original decision was trying to dodge.
  `agents_dual_write_definition_upsert` (`dual_write.rs`) DOES write
  `default_memory_id` (both INSERT and `ON CONFLICT DO UPDATE`) — the
  opposite of what this section used to claim. The one true part of the
  original reasoning: `db_agents.memory_id` (singular, instance-scoped)
  is still untouched by this work, so the "which wins" question for
  *that* column genuinely doesn't arise. It's specifically
  `default_memory_id` that's new and dual-written.
- `AgentInstance.memory_id` (launch-time) is unaffected and stays — a
  launched instance can still be pointed at a *different* bundle than its
  definition's default if a user explicitly does that via the launch modal
  (§4's "not proposing to restrict sharing" already covers this); the new
  definition-level field is just the *default* an instance inherits if the
  launch modal doesn't override it.
- New migration `m0021_backfill_agent_bundles.rs` (mirrors `m0020`'s
  pattern): for every existing agent DEFINITION (global registry, not just
  local SQLite — same trap `m0020` already documents) with an empty
  `memory_id`, create a fresh `db_bundles` row (empty/default content,
  `is_blank=false`, `name` derived from the agent's own name for
  discoverability in Armory's bundle list) and point the definition's new
  `memory_id` at it. **Provider/model correction (2026-08-15, P0 finding
  on PR #2587's review):** originally hardcoded `provider='claude'`,
  `model='anthropic'` for every backfilled agent per §7.3's "every agent
  on this instance is Claude Code + OAuth Anthropic." That's true today
  but unsafe to bake into the migration — combined with
  `check_provider_model_immutable`'s enforcement and `agent_open.rs`'s
  spawn-time bundle-provider preference (step 5 below), it would have
  silently and permanently reassigned any non-Claude agent's harness on
  backfill. Fixed to derive `provider` from the DEFINITION's own
  `provider` column (already known, no inference needed) and `model` from
  that provider's first declared `supported_vendors` entry — same
  derivation new-agent provisioning already uses. Falls back to
  `claude`/`anthropic` only for the degenerate case of a definition with
  no provider set at all.

### 3.2 Creation-path unification
- Move the "must have a bundle" decision from launch-time to
  **definition-time**: `agent.define` and the template-clone path both gain
  a step that auto-provisions a bundle (same shape as the migration's
  backfill) if the caller didn't supply one, instead of defaulting to `''`.
- Launch modal's existing enforcement (`memoryId() !== ""`) stays as a
  belt-and-suspenders check, but should now be unreachable in practice since
  definition-time provisioning guarantees a non-empty id.

### 3.3 UI: close the bundle-summary.tsx data gap
- Fix the documented DATA GAP so the Identity/Memory tab can actually
  resolve and show the CURRENT instance's bound bundle (name, provider,
  quick link into Armory to edit it) — this is prerequisite work regardless
  of (a) vs (b), since there's currently no way for a user to *see* whether
  enforcement is even working.
- Once bundle-per-agent is the default, this tab becomes the natural home
  for "this is your agent's own ABF" — closer to a real per-agent identity
  panel than today's generic pointer.

### 3.4 Validation
- Decide whether `bundle.validate`'s "empty bundle is valid" test should
  still be true. Under reading (b), an agent's own freshly-provisioned
  bundle *should* validate empty (it's a legitimate starting state, not an
  error) — validation should keep checking well-formedness, not
  completeness. Don't conflate "has an ABF" (existence) with "ABF is fully
  filled out" (content) — those are different requirements and this doc is
  only about the former.

## 4. What this doc is NOT proposing

- Not proposing agents can no longer *share* a bundle if a user explicitly
  wants that (e.g. a team-wide instruction set) — mandatory-per-agent is
  about the *default*, not a restriction on binding an agent to an
  existing, shared bundle if the user does so on purpose.
- Not proposing to remove the `blank` singleton — it may still be useful as
  an explicit "intentionally minimal" bundle a user can pick, just no
  longer the silent default nobody chose.
- Not proposing schema changes to `db_bundles` itself, ABF v0.2's
  provider-aware fields, or the validator's rules beyond what's noted above.

## 5. Decisions (resolved 2026-08-14)

1. ~~Confirm reading (a) vs (b)~~ — resolved (b), see §2.

2. **Deleting an agent orphans its bundle by default; does not
   delete it.** The whole point of §7's portability reframe is that a
   bundle can outlive the specific agent instance that created it — that's
   what export/import already means. Cascade-deleting the bundle on agent
   delete would work against that: a user who exported a bundle elsewhere
   (or just wants to keep the memory/instructions around to found a new
   agent later) shouldn't lose it because they deleted the original agent.
   Default is non-destructive; an explicit "delete bundle too" option at
   agent-delete time can be added for users who actually want cleanup, but
   destruction should never be the silent default. Orphaned bundles
   clutter the Armory list over time (the real cost noted in the original
   question) — accepted, addressed by 3 below rather than by deleting data.

3. **List normally in Armory, with an "owned by `<agent>`" indicator —
   not hidden.** Hiding an agent's own bundle would make it hard to
   audit ("where did this agent's instructions actually go") and blocks a
   legitimate use case: browsing/editing an agent's bundle directly from
   Armory, or copying one agent's setup as the starting point for another.
   This also composes cleanly with 2 — an orphaned bundle (agent deleted)
   naturally becomes "no owner" instead of needing separate orphan-tracking
   UI, since ownership is just "which agent's `memory_id` currently points
   here," recomputed live rather than stored.

4. **No known cross-agent bundle sharing on this machine to protect
   during backfill** — confirmed by the operator (§7.3: every existing
   agent is Claude Code + OAuth Anthropic, no exotic sharing setups). The
   `m0021` backfill can treat every unbound agent independently without a
   dedup/sharing-preservation pass.

## 6. Suggested next step

Once the above is confirmed, the actual implementation likely splits into
independently landable pieces (matches this repo's incremental-PR
convention seen elsewhere, e.g. the ABF v0.2 spec's own phased rollout):

1. **DONE (2026-08-15).** Fix `bundle-summary.tsx`'s data gap. Turned out
   already half-closed: Armory Phase 5
   (SPEC_ARMORY_PHASE5_CONSOLIDATION_AND_SKILL_SEEDING_2026_07_13.md §1.3)
   had already fixed the `view: "identity"` side by threading
   `meta.agentId` into `IdentityPaneViewModel` and rendering
   `AgentIdentityLinksPanel` directly instead of this panel — the
   `view: "memory"` side (`memory-view.tsx`) was the one spot still stuck
   on the generic pointer-only form, and its own header comment was stale
   (still describing the gap as unresolved). Fixed the identical way:
   added an `agentId` accessor to `MemoryViewModel` reading `meta.agentId`
   (mirrors `IdentityPaneViewModel.agentId` exactly), threaded it into
   `<BundleSummaryPanel agentId={...}/>`, and gave that component an
   optional `agentId` prop — when present, it resolves the agent's own
   dedicated bundle via `AgentDefinition.memory_id` (the definition-level,
   readonly-after-creation field from step 2/§3.1 above — cleaner than
   the instance-level `AgentInstance.memory_id` the original DATA GAP note
   was written against, since it doesn't depend on which specific launch
   is active) and shows its name + provider inline, with a link into
   Armory to edit it. Falls back to the original context-free form when
   `agentId` is absent, unchanged for every other case. Confirmed via a
   research pass that NEITHER `view: "identity"` nor `view: "memory"`
   blocks are created by any live UI flow anymore — both were superseded
   by the agent pane's "Stash" modal (`AgentStashModal.tsx`) — so this
   fix only matters for pre-existing persisted layouts with one still
   open; still worth doing correctly and cheaply given the established
   pattern already existed. 5 new tests
   (`frontend/app/view/bundle-summary.test.tsx`) covering the
   agent-bound/unbound/failed-resolve/absent-agentId/wrong-agent cases.
2. **DONE (2026-08-15).** Backfill migration `m0021` for existing agents.
   Turned out to need a real schema addition first —
   `db_agent_definitions.memory_id` didn't exist at all (only
   instance-level bindings did); see §3.1's revision. Also required a
   SECOND new column, `db_agents.default_memory_id` — discovered live that
   `agent_def_list()` already reads from the consolidated `db_agents`
   table (a partial Phase 3b flip nobody had documented against this
   plan), so a definition-level field invisible to that table wouldn't be
   backfillable at all; used a distinct column name rather than the
   existing instance-scoped `db_agents.memory_id` to avoid a "which wins"
   conflict. Along the way, found and fixed a genuinely pre-existing,
   unrelated test flake: two migration test modules (`m0020`, and now
   `m0021`) were each using their OWN local mutex to serialize
   `AGENTMUX_HOME_OVERRIDE` env-var access instead of the crate-wide
   `test_support::ISOLATED_AUTH_ENV_LOCK` every other consumer already
   used — separate mutexes don't serialize against each other, so they
   raced under parallel test execution. Both switched to the shared lock.
3. **DONE (2026-08-15).** Definition-time provisioning for new agents.
   Scoped wider than the original "`agent.define` + template-clone path"
   wording once the actual creation surface was cataloged — SIX
   production entry points build a fresh `AgentDefinition` and insert it
   (`createagent`, `importagentfromclaw`, `importagents` bulk import,
   `agentdefcreatefromtemplate`, `forkagentdefinition`, `agent.define`),
   and "every agent must have an ABF" doesn't hold if only two of six are
   covered. Added `Store::bundle_provision_for_new_agent` (builds the
   bundle, carrying the agent's own already-known `provider` — unlike
   `m0021`'s backfill, a brand-new agent's harness isn't a guess) and
   `Store::agent_def_provision_and_bind_bundle` (provisions + binds,
   writing the new id back into the caller's `&mut AgentDefinition` so
   RPC responses that serialize the struct directly reflect it, same
   "caller's struct reflects what landed" convention `agent_def_update`
   already follows). Deliberately a POST-insert step, not part of the
   initial `INSERT`: `agent_define_core`'s `agent_def_find_or_insert`
   uses its struct as both the lookup key and the conditional-insert
   payload, so a bundle built up-front would leak (created, never bound)
   on every `if_exists=skip`/`update` call against an already-existing
   name — and `agent.define` is meant to be called repeatedly/
   idempotently. Binding only after a genuinely-fresh insert is confirmed
   avoids that leak (covered by a dedicated test in both the RPC-level
   and `agent_define_core`-level suites). Vendor default derived from
   `crate::backend::providers::get_provider(provider).supported_vendors
   .first()` — the Rust-side equivalent of the frontend's
   `resolveDefaultVendor`/`catalog.ts`, already existed for other
   purposes (the `every_provider_declares_at_least_one_supported_vendor`
   test), just not wired into bundle creation before now.
4. (If wanted) delete-cascade or orphan-handling for agent-owned bundles.
   NOT YET DONE.

## 7. Extension: harness + model as readonly, portable ABF fields

**New idea, raised 2026-08-14, not yet scoped in detail.** Confirmed
genuinely new — not a rediscovery of the stale branch specs (§ "Related"
above), and not proposed anywhere else in the repo (verified by search).
Recorded here because it changes what "unit of portability" means for
everything above.

### 7.0 Correction found during implementation (2026-08-14): this reverses a prior deliberate decision, not "adding new fields"

While implementing §7.4.1-7.4.3 against the actual schema, found that
`db_bundles.provider`/`.model` **already exist as columns** — in both the
object and shared-store schemas (`migrations.rs:333-334`, `:812`ish). §7.1
below was written assuming these needed to be added; they don't. What
actually needs to change is smaller (no new migration/columns) but the
*decision* being made is bigger than "revive vestigial fields": these
columns were deliberately zeroed by a named, dated, reasoned architectural
decision — `docs/specs/archive/SPEC_MEMORY_IDENTITY_ARCH_2026_06_19.md
§4.1a` (2026-06-20): "A preset is the agent's reusable, **provider-agnostic**
capability pack... Provider + model belong to the agent... A preset can then
be paired with an agent of any provider." `frontend/app/view/memory/
memory-model.ts:130-133` is where that decision is enforced today —
`draftToWire` writes `provider: "", model: ""` unconditionally on every
save specifically so "the ON CONFLICT update clears any stale legacy
value."

**This section is reversing that decision, not fixing an oversight.** The
tradeoff §4.1a optimized for — one preset reusable across agents on
*different* providers — is exactly what §7's readonly-per-ABF model gives
up: once provider/model are fixed at creation, an ABF can only ever pair
with one specific harness+model combination, forever. §4.1a's world valued
cross-provider reusability; this doc's world values self-contained
portability. Both are legitimate goals — they're just in real tension, and
this doc is choosing portability over reusability for the reasons in §7.2.
Worth being explicit about this in whatever PR does the implementation,
since it's the kind of reversal a reviewer familiar with the June decision
should be able to see was made on purpose.

Practical effect: **no new schema/migration work for these two fields**
(§7.5 step 2 is now just "stop zeroing them," not "add columns") — but the
Presets/Armory bundle editor needs its Provider/Model fields *back*
(removed by §4.1a's own action item) — as readonly-after-creation fields,
not the freely-editable ones that existed before.

### 7.1 The idea

Reframe: the portable unit isn't "the agent," it's the **ABF**. Today
harness (`provider`, e.g. `claude`/`codex`/`gemini`) and model vendor
(`resolveEffectiveVendor` / `model_vendor_base_url`) live on the *agent*
(`AgentDefinition`) — real, shipped, working (PR #2505/#2558) — but
deliberately absent from the bundle (`db_bundles.provider`/`.model` are
dead/zeroed columns by design, per the ABF v0.2 spec's own finding). The
proposal: **move harness + model onto the ABF itself, as fields set once at
ABF creation and readonly thereafter.** An ABF then carries everything
needed to reconstitute an agent elsewhere — instructions, context, memory,
*and* which harness/model it runs on — so exporting an ABF and importing it
on another machine/instance reproduces the same agent, not just its prompt
content.

This directly builds on §1.1's finding (bundles currently have no such
fields) and reframes §3's "every agent gets its own bundle" — under this
extension, provisioning an agent's bundle also means fixing its harness at
that moment, not leaving it agent-mutable afterward.

### 7.2 Why readonly, not just "settable"

If harness/model stayed editable after ABF creation, two ABFs with
identical instructions/memory could still diverge in what they need to run
(different harness, different auth) — defeating the portability goal
(taking an ABF "elsewhere" should mean it just works, not "works if the
target also has the right harness configured to match, which the ABF
doesn't tell you"). Fixing them at creation makes the ABF **self-describing**
about its own requirements — a receiving instance can check "do I have
`claude` + Anthropic OAuth available?" before even trying to launch it.

### 7.3 Existing-agent backfill is trivial (confirmed by operator)

Every agent on this instance today runs the same harness + auth: **Claude
Code, OAuth Anthropic.** So the backfill migration from §3.1 (`m0021`) needs
no per-agent inference logic — every backfilled bundle gets
`harness='claude'`, `auth_mode='oauth-anthropic'` (or equivalent constant
names) unconditionally. This significantly de-risks the migration piece
that §1.4/§3.1 flagged as needing care (cross-channel registry attach is
still a real concern; picking the *right value* to backfill to is not).

### 7.4 Design decisions (resolved 2026-08-14)

**7.4.1 — ABF becomes sole source of truth; `AgentDefinition` keeps no
duplicate copy.** Decided: since harness/model are readonly-once-set, there
is no scenario where an agent's own copy and its bundle's copy could
legitimately diverge — keeping both is pure duplication with a sync-bug
risk (which field wins if they disagree?) and no upside. Resolve
provider/vendor by reading through the bound bundle at spawn time, not from
a cached field on `AgentDefinition`.

*Grounded implementation shape* (checked against the actual code, not
guessed): `agent.provider` is read at ~10+ call sites within
`agentmux-srv/src/server/app_api/agent_open.rs` alone (CLI path resolution,
launch args, auth dir, container command, output format, meta tags, etc.)
and a similar spread in `frontend/app/view/agent/agent-model.ts`
(`agent.provider` at lines 325/327/343/345/628/747). Rewiring every
individual site is unnecessary churn. Instead: resolve `provider` and
`model_vendor_base_url` **once, early**, via a bundle lookup keyed by
`agent.memory_id` — mirrors the existing `resolve_vendor_env_override`
pattern already in `agent_open.rs:45-55` (pure function, unit-tested,
called once near the top of the spawn flow) — and shadow the two local
values the rest of the function already treats as agent-local data. Net
effect: one new lookup + two shadowed locals per file, not ~10+ call-site
rewrites. `AgentDefinition`'s own `provider`/`model_vendor_base_url` columns
become dead (same fate as `db_bundles.provider`/`.model` today, per the ABF
v0.2 spec) rather than actively read — a later cleanup PR, not this one's
blocker.

**7.4.2 — Readonly enforced on the backend, not just UI.** Decided: UI-only
protects exactly one client (Armory's bundle editor) — not other RPC
callers, not a future UI surface someone adds without knowing the invariant
matters. Since the entire value of this feature is "an ABF's harness/model
can be trusted to describe what it needs," a guarantee that only holds
because of the one UI you happen to know about isn't really a guarantee.

*Grounded implementation shape*: `register_bundle_upsert`
(`agentmux-srv/src/server/app_api/bundle.rs:115-165`) is the single write
path for both create and update (there's no separate `bundle.update` — it's
upsert-shaped) and **already has this exact guard pattern** for other
protected-mutation cases: it rejects mutating a protected id
(`blank`/`seed-*`, line 129-131) and rejects mutating an existing bundle
flagged `is_global` (line 138-140), both by loading the existing row via
`id_store.bundle_memory_get` before accepting the write. The harness/model
readonly guard is the same shape: if `existing.harness`/`existing.model`
are already non-empty and the incoming `memory.harness`/`memory.model`
differ, reject with `FORBIDDEN`. Small, follows established convention,
directly testable (the file already has inline `#[cfg(test)]` coverage for
`resolve_vendor_env_override` to mirror).

**7.4.3 — Export/import mechanics: already exist, scope is smaller than it
looked.** `bundle.export_for_agent` / `bundle.import_for_agent`
(`agentmux-srv/src/server/app_api/bundle.rs`, ~lines 400-710) are real,
shipped RPCs — not a green-field feature this work needs to build. Adding
harness+model to the portable ABF is "add two fields to an existing,
working export/import payload," not "design export/import." Significantly
shrinks this piece's scope — should be re-estimated once the field
addition itself (7.4.1/7.4.2) lands, likely small.

**7.4.4 — Receiving instance lacks the required harness/auth: still open.**
Not resolved — forward-looking given today's install base is 100%
claude+oauth-anthropic (§7.3), so there's no live case to validate against
yet. `bundle.import_for_agent` already has precedent for a pre-import
guard (line ~592: rejects import into an agent that already has native
memory files) — a harness/auth-availability check would live in the same
place, same shape, whenever this becomes a real scenario. Not a blocker for
an initial version scoped to this machine's actual current agents.

### 7.5 Suggested sequencing relative to §6 (revised per §7.0)

**Naming correction:** use the existing column names throughout —
`db_bundles.provider` (not "harness" — same concept, matches
`AgentDefinition.provider`'s existing naming) and `db_bundles.model`. For
`model`'s value: §4.1a's own text notes the actual model checkpoint
(`--model <x>`) rides in `provider_flags`, not a clean single field — and
the shipped vendor-separation work (PR #2505/#2558) already has a distinct,
better-fitting concept for "which backend" (`resolveEffectiveVendor`).
**Decision: `db_bundles.model` stores the resolved *vendor* string** (e.g.
`"anthropic"`, the provider's default per `PROVIDERS[provider].supportedVendors[0]`,
or `"custom"` if a base-URL override applies) — not a specific model
checkpoint. That's the actually-portable "which backend does this need"
information; checkpoint selection stays a runtime/launch-time choice, not
frozen into the ABF. Flagging this as a judgment call, easily revisited if
wrong. So the operator's "harness is claude code, the API is Oauth
anthropic" backfills a claude-provider agent to `provider='claude'`,
`model='anthropic'` — **not** every agent unconditionally regardless of
its own provider; see §6 step 2's 2026-08-15 correction for the P0 bug
that assumption caused when first implemented literally.

With 7.4.1-7.4.3 and 7.0's correction applied, no new migration is needed
for these two fields — they already exist in both schemas. Revised
implementation order:

1. Stop zeroing `provider`/`model` at write time
   (`frontend/app/view/memory/memory-model.ts:130-133`) and restore
   Provider/Model fields to the Presets/Armory bundle editor — but readonly
   once set (not the freely-editable pre-§4.1a fields).
2. `bundle.upsert` readonly guard (7.4.2) — backend enforcement, same PR as
   step 1 ideally since they're the two halves of the same readonly
   guarantee.
3. `m0021` backfill migration — provisions a dedicated bundle per existing
   unbound agent (§6 step 2) AND sets `provider`/`model` on it in the same
   pass (§7.3, using the naming from above) — one migration, not two.
4. Definition-time provisioning for new agents (§6 step 3), setting
   `provider`/`model` at creation from whatever the create flow already
   knows (the provider the user picked to create the agent with).
5. **DONE (2026-08-15), backend half only.** Spawn-time read-path change
   (7.4.1) — `agent_open.rs` resolves the ACTUAL harness for a spawn
   through the bound bundle, not `AgentDefinition.provider` directly.
   Implemented as one pure, unit-tested function
   (`resolve_effective_provider_id`, 4 tests) called once right after the
   agent definition loads, which then overwrites the local `agent.provider`
   copy — every downstream read in the handler (~10 sites: CLI path,
   launch args, auth dir, container command, output format, meta tags,
   the `AgentOpenResult` response) picks up the resolved value for free,
   matching the "shadow, don't rewrite call sites" shape this section
   originally sketched. This also fixes a real (if currently only
   theoretical, per §7.3) correctness gap the sketch didn't call out:
   `AgentDefinition.provider` is NOT actually immutable post-creation —
   `agent.define`'s `if_exists=update` path can still change it via
   `agent_def_update` — while the bundle's own copy IS backend-enforced
   immutable (7.4.2's guard). So the two *can* legitimately diverge after
   an update, and the bundle is the one guaranteed trustworthy; spawn now
   reads that one. Falls back to `agent.provider` when there's no bundle
   to consult (unbound legacy agent, missing row) so a spawn never
   hard-fails on this alone.
   **NOT DONE: the frontend half** (`agent-model.ts`'s pre-flight
   `launchAgentDefinition` — Node.js availability check, CLI dir
   resolution, log lines — still reads `agent.provider` directly, before
   the RPC call reaches the now-correct backend resolution). Deliberately
   deferred: fixing it means either an extra bundle-fetch round-trip on
   every launch, or teaching `agent_def_list()` itself to shadow
   `provider` for every consumer app-wide (Armory, agent settings, the
   picker — a much bigger blast radius than this one spawn path). Low
   real-world impact today since drift requires an explicit
   `agent.define update` with a changed provider on an already-bundled
   agent, which doesn't happen in current usage (§7.3). Flagging as a
   known gap for whoever picks this up next, not silently dropping it.
6. **DONE (2026-08-15).** Bundle export/import field addition (7.4.3) —
   as small as §7.4.3 predicted. `armory.json`'s manifest gained two
   top-level fields, `provider`/`model`, sourced from the bundle's own
   columns at export time (`bundle_export.rs`) — `null` rather than an
   empty string when the source bundle has neither set yet, so an
   older/still-unbound export doesn't come back as a misleadingly-present
   empty value. `ParsedBundleImport` gained matching `provider`/`model`
   fields (default `""` when the manifest predates this or omits them —
   backward-compatible with every existing exported bundle).
   All THREE places that build a fresh `Memory` from a parsed import
   (`bundle.import_for_agent`, `bundle.import.preview`'s underlying
   commit path, and `bundle.import.commit`) now carry `parsed.provider`/
   `.model` through instead of the `String::new()` they'd been hardcoded
   to — treated the same way `description` already is (always carried,
   not gated behind an `include_*` toggle the preview/commit UI has for
   instructions/context/mcp, since provider/model are structural agent
   requirements, not optional content). The existing
   `check_provider_model_immutable` guard (7.4.2) still applies on write,
   so importing a provider/model pair into an ALREADY-bound existing
   bundle id is still rejected if it would change either value — this
   addition only affects what a FRESH bundle (a new uuid, which every
   current import path always creates) gets populated with. 2 new tests
   (export omits-vs-populates the fields correctly; full export ->
   import round trip preserves both onto the new bundle).

## 8. PR #2587 review round 2 (2026-08-15): real store-mismatch bug

Round 1's review (P0 hardcoded provider, doc drift, formatting — see the
§6/§3.1 correction notes above) landed clean. A second automated pass, on
the round-1 fix commit, caught a genuinely serious bug round 1 didn't
introduce but also didn't catch: **every bundle provisioned by this whole
feature — both the runtime creation-path provisioning (§6 step 4) and the
`m0021` backfill (§6 step 2) — was being written into the wrong SQLite
database.**

**The bug:** `AppState` has two `Store` handles —
`wstore` (the per-channel `objects.db`) and `id_store` (the *effective*
identity/memory store: `shared_store` when a shared store is configured,
else `wstore` — see `server/mod.rs:92-95`'s own doc comment: "Handlers
capture this instead of `wstore` for any operation that must survive
across version upgrades"). Every real bundle-read RPC
(`listmemories`/`getmemory`/`upsertmemory`, `server/agent_handlers/
memory.rs`) already reads/writes through `id_store`, not `wstore` — that
convention predates this feature entirely. But
`Store::bundle_provision_for_new_agent`/`agent_def_provision_and_bind_
bundle` were called as `wstore.agent_def_provision_and_bind_bundle(...)`
at all six creation sites, and `m0021` opened only `Store::open(&ctx.
channel_store_path)` and wrote the bundle there too. In any normal
install (shared store resolvable — the common case, not just CI), every
newly-provisioned per-agent bundle landed in a SQLite file nothing else
ever reads: invisible to Armory, to `bundle.get`, and to this same
feature's own new bundle-summary panel (§6 step 1), which would show
"couldn't be loaded" for virtually every freshly created or backfilled
agent. A second, smaller finding on the same review pass: bundle
provisioning ignored `AgentDefinition.model_vendor_base_url` entirely,
so an agent created against a custom vendor endpoint got its bundle's
`model` permanently locked to the provider's bare default instead of
`"custom"` (`resolveEffectiveVendor`'s own rule, which nothing on the
Rust side mirrored before this).

**Fix:**
- `Store::bundle_provision_for_new_agent` is now called explicitly on the
  correct store at every site — never implicitly via `self`/`wstore`.
  `agent_def_provision_and_bind_bundle`'s signature gained an explicit
  `bundle_store: &Store` parameter, separate from `self` (still the
  definition store): `self.agent_def_provision_and_bind_bundle(&id_store,
  &mut agent, now)`. All six runtime creation sites
  (`createagent`/`importagentfromclaw`/`importagents`/template-clone/
  fork/`agent.define`) now thread `state.id_store` alongside `state.
  wstore` into their RPC handler closures and pass it through —
  `agent_define_core`'s signature grew an `id_store: Arc<Store>`
  parameter for this, updated at its one real caller plus the HTTP
  service dispatch path (`service/misc.rs`) and every test call site.
- `m0021` now resolves a `bundle_store` via `Store::open_shared(&ctx.
  shared_store_path)` (falling back to the channel store on failure,
  mirroring `AppState.id_store`'s own degrade-to-`wstore` behavior) and
  writes every backfilled bundle there instead of the channel store —
  `agent_def_set_memory_id_if_empty`'s definition-binding write stays on
  the channel store, unaffected; only the bundle's own row moved.
- Added `Store::resolve_effective_vendor(provider, model_vendor_base_url)`
  — the Rust-side twin of the frontend's `resolveEffectiveVendor`
  (`"custom"` when a non-empty base-URL override is set, else the
  provider's default vendor) — and wired it into both
  `bundle_provision_for_new_agent` and `m0021`'s backfill, so the two
  paths can't drift on this rule again.
- New tests: a Store-level test proving `agent_def_provision_and_bind_
  bundle` writes the bundle into the explicit `bundle_store` param and
  NEVER into `self` (two genuinely separate in-memory stores); an
  `m0021` test proving the backfilled bundle is absent from the channel
  store and present in the shared store; an `m0021` fallback test
  proving the migration still succeeds (writing to the channel store)
  when the shared store path is unusable; `resolve_effective_vendor`
  unit tests (default, override, whitespace-only override, unknown
  provider); an `m0021` test for the custom-vendor-override backfill
  case. Existing RPC-level tests (`agent_handlers/mod.rs`,
  `agent_define.rs`) needed no assertion changes — their test harnesses
  already set `id_store: wstore.clone()` (same underlying store), so
  they couldn't have caught this class of bug in the first place; the
  new Store-level and migration-level tests are what actually exercise
  two genuinely distinct stores.
- Formatting: the round-1 cleanup of the mechanical `memory_id`-field-
  insertion script's stray-blank-line/glued-brace artifact only matched
  the `let x = Struct { ... };` shape (trailing semicolon); missed eight
  sites where the same struct literal was an implicit-return expression
  or function argument (`Struct { ... }` with no semicolon) —
  `agent_open.rs` (×2), `native_memory_handlers.rs`, `identities.rs`,
  `blockcontroller/core.rs`, `m0017_ambient_login_grandfather.rs`,
  `identity/resolver/inject.rs`, `storage/store/tests.rs`. Fixed with a
  second pass handling both shapes; verified with a broader sweep for
  any other field showing the same pattern (none found).
- Added the missing `.changesets/` entry this feature PR should have had
  from the start per CLAUDE.md's mandatory changesets workflow (new
  schema column, new RPC behavior, new UI) — `minor` bump.
