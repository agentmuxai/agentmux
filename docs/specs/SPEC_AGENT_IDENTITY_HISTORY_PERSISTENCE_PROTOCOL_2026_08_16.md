# Canonical Agent Identity/History Persistence Protocol — Synthesis with Mandatory ABF

**Date:** 2026-08-16
**Author:** Clamk (agent, `~/.agentmux/agents/clamk-0612a`), at operator request
**Status:** Proposal — synthesizes two existing documents into a canonical protocol; not yet implemented or
reviewed.
**Ground truth basis:** `agentmuxai/agentmux` local checkout at `3705f83c3`
(`agent3/bashwrap-persist-cwd-across-calls`), cross-checked against `origin/main` at `72aefad4d`.
**Related:**
`docs/specs/REPORT_AGENT_IDENTITY_HISTORY_FRAGMENTATION_2026_08_16.md` (this agent's own root-cause report,
written earlier the same day — background for §1-2 below),
`docs/specs/ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md` (PR #2587, merged 2026-08-15 — the "every
agent gets its own dedicated, portable bundle" work this doc builds on),
`docs/specs/SPEC_CROSS_CHANNEL_AGENT_PERSISTENCE_2026-06-13.md`,
`docs/specs/SPEC_AGENT_GLOBAL_PORTABILITY_2026-06-16.md`.
**Motivation:** the operator, after reading the root-cause report, asked three questions: (1) does this
indicate the system is mid-migration and needs cleanup, (2) should there be a formal spec canonizing the
correct protocol, and (3) could the just-shipped Mandatory ABF work be the right place to anchor conversation
history, so it's naturally portable. This doc answers all three together, since the answer to (3) changes the
shape of (2).

---

## 1. Yes — this is a mid-migration state, with three overlapping unfinished migrations

The operator's instinct is correct. Three separate, independently-tracked migrations are all in-flight at
once, and the identity/history fragmentation bug traced in the companion report is a symptom of exactly this
overlap, not an isolated defect:

### 1.1 The `db_agent_definitions`/`db_agent_instances` → `db_agents` consolidation is stalled mid-phase

`agentmux-srv/src/backend/storage/migrations.rs`'s own schema-version comment (v4, `OBJECT_SCHEMA_VERSION`)
states: *"db_agents consolidation table (Phase 3a; dual-write only, reads still on
db_agent_definitions / db_agent_instances)."* **This comment is stale and wrong as of 2026-08-15.** PR #2587's
review round 2 (documented in `ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md` §3.1's 2026-08-15 correction)
discovered live, while implementing an unrelated backfill migration, that `agent_def_list()` — the function
most of the codebase already uses to enumerate agents — **already reads from the consolidated `db_agents`
table**, not `db_agent_definitions`. Nobody had updated the schema-version doc comment, or apparently
realized the read-side flip (Phase 3b) had partially happened, until a migration written against the
documented (stale) model broke on its second run. This is a mid-migration system whose own internal
documentation disagrees with its own code about which phase it's in.

### 1.2 The identity/credential store-routing split (`wstore` vs `id_store`) is enforced by convention only, and was violated the same week

`server/mod.rs`'s own doc comment establishes the rule: handlers must capture `id_store` (not `wstore`) "for
any operation that must survive across version upgrades." This is precisely the invariant the companion
report's root cause (PR #2431, `identities_dir()`) and this doc's §2 are both about. Yet PR #2587 — the
Mandatory ABF work itself, merged one day before this report — shipped its first version with exactly this
bug: bundle provisioning at all six agent-creation call sites, and the new `m0021` backfill migration, wrote
into `wstore` (the per-channel store) instead of `id_store` (the shared/global store), making every newly
provisioned per-agent bundle invisible to Armory and to the bundle-summary panel the same PR added. This was
caught and fixed in review (§8 of the ABF doc) — but it demonstrates the "silently routes global data through
a per-channel store" failure mode is not a one-off from 2026-08-06; it is actively recurring in brand-new code
as of 2026-08-15, because there is still no structural guard against it (§2.2 below proposes one).

### 1.3 The history-adapter/identity-isolation split was never reconciled at all

Per the companion report, `ClaudeHistoryAdapter` (the code that finds Claude Code transcripts) has not been
touched to account for PR #2431's `identities_dir()` default change. These two pieces of code were written by
different efforts, at different times, and nothing currently checks that they agree on where history can
live. This is not "a migration in progress" so much as "a migration that was never started" — the read path
simply doesn't know the write path's default changed.

**Conclusion for (1):** yes, this needs cleanup, and it is bigger than the single regression the companion
report root-caused. There are at least three independent seams (definition-table consolidation, store
routing, and history-path/identity-isolation coupling) that all touch the same underlying data and are all
individually incomplete. A canonical protocol (§2) should resolve all three at once, since patching any one in
isolation leaves the other two as latent recurrences of the same bug class.

---

## 2. A canonical protocol (answers "do we want a formal spec")

Propose formalizing the following as an enforced protocol, not a convention — five invariants, each with a
concrete mechanism, targeting the exact failure modes in §1 and the companion report's §2 table (five prior
incidents, all the same class):

### P1 — Every piece of agent data has exactly one declared scope, from a closed taxonomy

| Scope | Contains | Lives | Keyed by |
|---|---|---|---|
| **CREDENTIAL** | OAuth tokens, PATs | Global, unless `isolated_auth_enabled()` (per-channel, for Armory testing only) | `bundle_id` (account) |
| **DEFINITION** | Agent identity: name, provider, `default_memory_id` binding | Always global | `agent_id` |
| **PORTABLE CONFIG (ABF)** | Instructions, context, skills, memory, harness+model | Always global | `memory_id` (bundle id) |
| **CONVERSATION HISTORY** | Provider transcripts (Claude/Codex/etc. session JSONL) | Always global | `agent_id` (§3 — not `memory_id`, not account/bundle UUID) |
| **RUNTIME/EPHEMERAL** | Pane layout, active session pointer, per-launch instance rows | May legitimately be per-channel/per-instance | `channel` + `instance_id` |

This table is new — today, scope is decided ad hoc per call site (§1.2's bug is a direct consequence: nothing
declared which scope a newly-provisioned bundle belonged to, so the six call sites each guessed, and five
guessed wrong). RUNTIME/EPHEMERAL is the *only* row channel isolation is allowed to touch; every regression
in the companion report's §2 table and §1 above is a case of channel-scoping leaking into one of the other
four rows.

### P2 — One resolver per scope, no ad hoc store access

Every read/write for CREDENTIAL/DEFINITION/PORTABLE CONFIG/CONVERSATION HISTORY must go through a named
resolver (`identities_dir()`, an `agent_id`-keyed definition accessor, a `memory_id`-keyed bundle accessor, a
new `agent_history_store(agent_id)` accessor) — never direct `wstore`/`instance_dir` access from a new call
site. This is already the intended pattern (`id_store` vs `wstore`); P2 just says it must be the *only*
pattern, enforced by P3 rather than left to individual PR authors to remember (which is exactly what failed
in §1.2).

### P3 — Each scope's routing is covered by an automated test, not a comment

`agentmux-common/src/data_paths.rs` already has this shape for CREDENTIAL
(`identities_dir_is_shared_on_stable_channel` etc., §1.2 of the companion report) — extend the same pattern to
the other three non-ephemeral scopes: a test asserting DEFINITION reads/writes never vary by channel, a test
asserting PORTABLE CONFIG never lands in a per-channel store (this would have caught §1.2's bug directly), and
a new test asserting CONVERSATION HISTORY resolves to the same location regardless of which channel the
resolving code runs in. Per the companion report §4.5 and the 2026-07-27 retro this is the *fourth-plus* time
this exact recommendation has been made and not shipped — treat it as a blocking prerequisite for this
protocol, not an optional follow-up.

### P4 — Migration-phase state is a single source of truth, not a doc comment

§1.1's stale comment caused a real bug (the `m0021` `UNIQUE constraint failed` loop). Track which phase each
in-flight table consolidation is actually in in one place the code itself can assert against — e.g. a
`#[test]` that queries which table `agent_def_list()` and its siblings actually read from and fails if it
doesn't match the doc comment's claimed phase — rather than trusting the comment to be updated by hand every
time a partial flip happens.

### P5 — Conversation history anchors on `agent_id`, using the ABF binding as its registry (§3)

---

## 3. Should conversation history be persisted in the ABF? — Yes and no; the useful part is the anchor, not the payload

The Mandatory ABF work (2026-08-15) is directly relevant, and the operator's instinct that it "may play into"
this is right — but the useful piece is narrower than "store transcripts inside the bundle."

### 3.1 What ABF now provides that didn't exist this morning

As of PR #2587, every agent **definition** has exactly one dedicated bundle, bound at definition-creation
time (not launch time), written through `id_store` (global, cross-channel, after the §1.2 fix), with the
binding living on `db_agents.default_memory_id`. Concretely: **this is now exactly the "stable name → durable
id, written once, globally, at definition time" registry that the companion report's §4.2 proposed building
from scratch** ("Extend the existing global agent registry record ... with an `identity_bundle_id` field ...
before minting a fresh UUID, look up this global mapping and reuse it"). The ABF work independently built the
infrastructure for that same shape, for a different reason (portable instructions, not history). Reusing it
is strictly cheaper than building a second, parallel global registry.

### 3.2 Why the anchor should be `agent_id`, not `memory_id` (the bundle itself)

It's tempting to key history directly on `memory_id` since that's the id that's now stable and global. Don't
— two properties of ABF make it the wrong direct anchor:

- **Bundles can be shared or rebound.** §4 of the ABF doc explicitly keeps this open ("not proposing to
  restrict sharing... if a user explicitly wants that"). If two agents ever point at the same `memory_id`,
  keying history by bundle id would merge their conversations into one history — wrong, and silently so.
- **Bundles can outlive their agent, or an agent could in principle rebind.** §5 decision 2 of the ABF doc:
  deleting an agent orphans its bundle by default rather than deleting it. History should stay attached to
  the *agent* across a bundle swap or an agent's deletion-and-bundle-orphaning, not silently move or vanish
  because the bundle's lifecycle diverged from the agent's.

So: **use the agent's own durable `agent_id` as the conversation-history anchor** (matching
`SPEC_AGENT_GLOBAL_PORTABILITY`'s original intent), but **use the ABF binding (`default_memory_id`, written
at definition time, globally, via `id_store`) as the mechanism that makes `agent_id` itself discoverable and
stable across a channel change** — i.e., the missing piece from the companion report (§1.3: "a fresh, random
UUID is minted every channel with nothing tying it to the previous one") is fixed not by inventing new plumbing,
but by the fact that a Mandatory-ABF agent's `agent_id` is now already resolvable through the same global,
definition-time-bound registry the bundle uses, instead of through a per-channel, per-launch account UUID
that never had a durable identity to begin with.

### 3.3 Why the transcript payload itself should stay out of the ABF export

ABF's own stated design (§7.1 of the architecture doc) is "the portable unit isn't the agent, it's the ABF" —
explicitly meant to be exported, imported, and **shared** (§4: "not proposing to restrict sharing... a
team-wide instruction set"). A multi-MB-to-GB, single-agent, high-churn, non-reusable conversation transcript
is a different kind of artifact than a reusable instructions/context/skills bundle, and embedding it would:

- Bloat every ordinary ABF export (instructions-sharing use case) with unrelated, often-huge private data.
- Create exactly the ambiguity §3.2 warns about — if bundle B is shared by two agents and the export embeds
  "the" conversation history, whose history is it?
- Conflict with the "orphan, don't delete" bundle-lifecycle default (§5 decision 2) — that default makes
  sense for reusable instructions a user might want to recover later; it's a much more consequential default
  for a full private conversation record, and deserves its own explicit decision rather than inheriting the
  bundle's.

**Recommendation:** keep conversation history in its own store (per the companion report §4.1/§4.3-4.4,
always-global, keyed by `agent_id`), and add a **separate, explicit, opt-in "export with history" action** —
distinct from the default ABF export — for the genuine use case of moving one specific agent's full record to
another machine. The default `bundle.export_for_agent` stays lean and shareable, exactly as designed.

---

## 4. Recommended sequencing

Building on the companion report's §4 (which this section supersedes/refines with the ABF-anchor insight):

1. **Finish P4 first, cheaply**: fix the stale Phase 3a/3b comment in `migrations.rs` to state the actual
   current reality (`agent_def_list()` already reads `db_agents`), and add the doc/code-agreement test P4
   describes. Low-risk, unblocks reasoning about everything else correctly.
2. **P1's taxonomy + P3's tests, applied first to the two known-bad scopes**: add the missing PORTABLE CONFIG
   and DEFINITION routing tests (would have caught §1.2's bug); add the CREDENTIAL vs CONVERSATION HISTORY
   split from the companion report's §4.1 (split `identities_dir()`'s effect on `claude/projects` out from
   its effect on credential material).
3. **Wire `agent_id` → history resolution through the existing ABF/`default_memory_id` binding** (§3.2) —
   this both fixes the companion report's §1.3 (no more freshly-minted, unlinked UUIDs for identity-bound
   agents) and gives CONVERSATION HISTORY its P1-mandated stable key, in one change, reusing infrastructure
   PR #2587 already shipped rather than building a parallel registry.
4. **Fix the read path** (companion report §4.3: `ClaudeHistoryAdapter` scans the per-channel path too) and
   **build the fast index** (§4.4) on top of the now-stable `agent_id` anchor — this is what actually makes
   "Conversation History" fast, per the operator's original ask, and it only needs to be built once the anchor
   from step 3 is stable, not before.
5. **Add the opt-in "export with history" action** (§3.3) as a genuinely new, separate feature once steps 1-4
   give it something stable to read from.
6. **Formalize P2/P3 as a written, reviewable checklist** (not just this doc) that any PR touching
   CREDENTIAL/DEFINITION/PORTABLE CONFIG/CONVERSATION HISTORY routing must satisfy before merge — the actual
   "formal specification that canonizes the proper protocol" the operator asked for, once steps 1-5 have
   proven the taxonomy holds up against real implementation (this doc is the proposal; a leaner, PR-checklist
   version is the artifact worth canonizing after review, per this repo's own pattern of "DECISIONS RESOLVED"
   docs preceding a "PR-sized checklist" — see how `ARCHITECTURE_MANDATORY_ABF_RETHINK_2026_08_14.md` itself
   was structured).

---

## Appendix: what changed since the companion report (same day)

The companion report (`REPORT_AGENT_IDENTITY_HISTORY_FRAGMENTATION_2026_08_16.md`, written earlier today)
proposed a new "stable name → bundle_id mapping" (its §4.2) as new infrastructure to build. This doc's
contribution is realizing that infrastructure was *already built*, one day earlier, for a different purpose
(PR #2587's Mandatory ABF), and reusing it is both cheaper and more consistent with this codebase's existing
"global registry written at definition time" pattern than adding a second one. The companion report's root
cause (§1 there) and proposed fixes 4.1, 4.3, 4.4, 4.5 stand unchanged; only 4.2's mechanism is superseded by
§3.2 above.
