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
- **Decision (2026-08-15): do NOT wire this into the `db_agents` dual-write
  path.** Traced `dual_write.rs`'s `agents_dual_write_definition_upsert` —
  it deliberately omits `memory_id` from both its INSERT and its
  `ON CONFLICT DO UPDATE` (only `agents_dual_write_instance_create`/
  `_update`/`_repoint` ever write that column), exactly matching
  `db_agents`' own schema comment ("only meaningful when `is_template=0`").
  Touching that would force resolving "which wins, a definition's default
  or an instance's explicit override" on a table that's mid-refactor
  (Phase 3a dual-write only; Phase 3b/3c — reader flip, legacy table drop —
  haven't happened yet) other in-flight work may depend on. Since Phase 3b
  hasn't happened, `db_agents` isn't read from yet for this purpose anyway
  — leaving it untouched costs nothing today and avoids the risk entirely.
  `db_agent_definitions.memory_id` stands alone, read directly (same as
  every other definition field is read today, pre-Phase-3b). Flagging
  explicitly for whoever does Phase 3b later: this field needs a decision
  then, not now.
- `AgentInstance.memory_id` (launch-time) is unaffected and stays — a
  launched instance can still be pointed at a *different* bundle than its
  definition's default if a user explicitly does that via the launch modal
  (§4's "not proposing to restrict sharing" already covers this); the new
  definition-level field is just the *default* an instance inherits if the
  launch modal doesn't override it.
- New migration `m0021_backfill_agent_bundles.rs` (mirrors `m0020`'s
  pattern): for every existing agent DEFINITION (global registry, not just
  local SQLite — same trap `m0020` already documents) with
  `memory_id in ('', 'blank')`, create a fresh `db_bundles` row
  (empty/default content, `is_blank=false`, `provider='claude'`,
  `model='anthropic'` per §7.3/§7.5, `name` derived from the agent's own
  name for discoverability in Armory's bundle list) and point the
  definition's new `memory_id` at it.

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

1. Fix `bundle-summary.tsx`'s data gap (visibility prerequisite, no
   behavior change, low risk, useful standalone).
2. Backfill migration `m0021` for existing agents.
3. Definition-time provisioning for new agents (`agent.define` +
   template-clone path).
4. (If wanted) delete-cascade or orphan-handling for agent-owned bundles.

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
anthropic" backfills to `provider='claude'`, `model='anthropic'`.

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
5. Spawn-time read-path change (7.4.1) — `agent_open.rs` +
   `agent-model.ts` resolve provider/vendor through the bound bundle
   instead of `AgentDefinition`'s own fields.
6. Bundle export/import field addition (7.4.3) — smallest piece, land last
   once the fields exist and are populated everywhere.
