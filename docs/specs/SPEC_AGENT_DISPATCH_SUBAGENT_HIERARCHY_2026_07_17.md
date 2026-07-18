# SPEC: two-level dispatch/member schema for subagents and workflows

**Date:** 2026-07-17
**Status:** Proposed — design only, no implementation yet.
**Ask (verbatim):** "the subagents are actually subsubagents .. We want to keep
track of the Agent and Workflow tool calls. each should produce one type of
SubAgent entity, which then has SubSubAgents, unless you can think of a better
name, research best practices, write spec to file"

---

## 1. Problem

`agentmux-srv/src/backend/subagent_watcher.rs` currently has one member-level
entity (`SubagentInfo`) and one container-level entity (`WorkflowInfo`) that
was bolted on after the fact — and the container only exists for Workflow-tool
runs. A solo Task-tool call (the common case) has **no container record at
all**: it's just a bare `SubagentInfo` with `workflow_id: None`, floating at
the top level next to Workflow-tool member agents.

This conflation is visible in three places:

1. **The RPC namespace itself.** `subagent.ListWorkflows`
   (`agentmux-srv/src/server/service/misc.rs:51-54`) lives under the
   `"subagent"` RPC namespace as a sibling of `subagent.ListActive` — a
   workflow *run* is being modeled as a kind of subagent-list query, when
   conceptually it's the container one level up.
2. **The frontend had to invent the missing entity itself.**
   `frontend/app/view/swarm/swarm-model.ts:59-72` defines `WorkflowGroup`, a
   client-only wrapper, with the comment *"backend has no separate
   workflow-name concept."* Because this wrapper has no backend-issued stable
   ID, the frontend then needs `shallowEqualGroup` / `stabilizeGroupIdentity`
   / `pruneGroupIdentityCache` (`swarm-model.ts:165-218`) purely to keep
   SolidJS's `<For>` from remounting (and collapsing expand-state on) a
   *synthesized* group every time a fresh `ListActive()` poll returns a new
   array. A solo Task-tool dispatch gets none of this — it has no group
   wrapper, stable or otherwise, because the backend gives it no dispatch-
   level identity to hang one on.
3. **The crash this session traced to it.** `docs/retro/retro-subagent-backfill-storm-oom-2026-07-17.md`
   (this session, same day) root-caused a launcher-killing OOM crash to an
   unbounded historical replay of *raw JSONL files* on every srv restart —
   1,030+ replayed files in ~10s. The fix (PR #2205) caps by file count
   because file count is the only unit the current schema has. A dispatch-
   level entity would make "replay the N most recent **dispatches**" the
   natural cap instead — a more meaningful unit than raw file count (one
   Workflow dispatch can span dozens of files; one solo dispatch is one
   file). Noted as a follow-up in §7, not required by this spec.

## 2. Research: how other systems name this exact shape

The shape here — one "thing that got kicked off" containing N "things that
actually ran" — is a solved problem elsewhere. Every mainstream system gives
the **container its own distinct noun**, not a doubled prefix on the child's
name:

| System | Container (one per trigger) | Member (one per unit of work) |
|---|---|---|
| GitHub Actions | Workflow **Run** | **Job** → **Step** |
| Apache Airflow | **DAG Run** | **Task Instance** |
| Temporal | **Workflow Execution** | **Activity Execution** / Child Workflow Execution |
| Kubernetes | **Job** | **Pod** |
| OpenTelemetry | Trace | Span (parent/child spans reuse the *same* noun at every depth, distinguished by a parent-pointer, not a prefix) |
| Google ADK (multi-agent) | root agent | **sub-agent** (reused recursively at every depth — same pattern as OpenTelemetry, not stacked) |
| Swarm-pattern frameworks | Lead | **Worker** / member |

Two consistent lessons, independent of domain:

1. **When there are exactly two levels, give each its own noun** (Run/Job,
   DAGRun/TaskInstance, WorkflowExecution/ActivityExecution). Nobody names
   the member "RunJob" or "DAGRunTaskInstance."
2. **When a system is recursive** (a sub-agent that can itself have
   sub-agents, arbitrary depth), the convention flips: reuse the *same* noun
   at every level and distinguish by a parent pointer, not by stacking
   prefixes per depth. AgentMux's shape is **not** recursive today — Claude
   Code's own file layout is exactly two levels deep (see §4) — so lesson 1
   applies, not lesson 2. If Task-tool calls made *from inside* a Workflow
   member ever became visible as their own nested dispatch, lesson 2's
   "same noun, parent pointer" pattern is the one to reach for then, not a
   third stacked prefix.

**Conclusion: "SubSubAgent" is the one pattern no reference system actually
uses.** It's a plausible first guess (and the reasoning behind it — "the
member is a sub-thing of a sub-thing" — is completely correct), but every
precedent gives the container its own name instead of doubling the child's.

## 3. Recommendation

**Primary recommendation — two entities, two distinct nouns:**

- **`AgentDispatch`** *(new)* — one per Agent-tool (Task tool) call **or**
  Workflow-tool call from a parent turn. Represents "what got kicked off."
- **`SubAgent`** *(renamed in place from today's `SubagentInfo`)* — one per
  actually-spawned Claude Code agent instance. Represents "what ran."

A solo Task-tool call is an `AgentDispatch` with exactly one `SubAgent`. A
Workflow-tool call is an `AgentDispatch` with however many `SubAgent`s the
run has spawned so far.

**Why keep "SubAgent" at the member level instead of promoting it to the
container** (the reverse of the literal ask): the member-level vocabulary is
already load-bearing across the app — `subagent:spawned`/`subagent:activity`/
`subagent:completed` WS events, the `subagent_watcher.rs` module name, the
`subagent.*` RPC namespace, the "Swarm pane," and four existing spec/report
docs (§4) all mean *member agent* when they say "subagent" today. Keeping
that meaning and only adding a new container noun is a strict subset of the
work the literal `SubAgent`→`SubSubAgent` proposal would require (which
renames *everything* at the member level to free up "SubAgent" for the
container, then invents the awkward doubled name anyway).

**Alternative — the literal ask, fairly stated:** keep `SubAgent` as the
container and use `SubSubAgent` for the member. Structurally identical to the
primary recommendation, so the runtime behavior is the same either way — this
is purely a naming choice. Cost: every existing "subagent = member" reference
(WS event names, RPC namespace, module name, 4 doc titles/bodies) means the
*opposite* thing afterward and needs renaming or an explicit note that the
term shifted meaning. If there's a strong preference for `SubAgent` to always
mean "the dispatch," this is the one to pick — call it out and I'll write the
migration against this shape instead.

Other container-noun candidates considered, if neither of the above lands:
`AgentCall`, `AgentRun`, `SubAgentGroup`, `SubAgentBatch`, `Dispatch`. Any of
these slot into the "primary recommendation" shape unchanged — only the noun
differs.

## 4. Current state (exact citations)

All of the following is **in-memory only** — confirmed no DB schema exists
for this data (`agentmux-srv/src/backend/storage/migrations.rs:102-117,546,1012-1054`'s
`db_workflow_definitions`/`db_workflow_runs` are a dead, unrelated legacy
Drone-DAG feature). Everything here is rebuilt every process start by
scanning JSONL files on disk. **This means the migration in §6 has no data
to move — it's a pure code change**, which is unusually cheap for a schema
redesign.

**File layout** (`subagent_watcher.rs:7-14`, Claude Code's own convention,
not AgentMux's to change):
```
<claude-config>/projects/<ws>/subagents/agent-<id>.jsonl                        — solo dispatch
<claude-config>/projects/<ws>/subagents/workflows/<run-id>/agent-<id>.jsonl     — workflow member
<claude-config>/projects/<ws>/subagents/workflows/<run-id>/journal.jsonl        — workflow run journal
```
This maps directly onto the two-level model: a bare `agent-*.jsonl` is a
solo `AgentDispatch` of exactly one `SubAgent`; a `workflows/<run-id>/`
directory is a Workflow-kind `AgentDispatch` identified by `run-id`,
containing its member `SubAgent`s.

**Backend types** (`subagent_watcher.rs`):
- `SubagentInfo` (L39-57): `agent_id, slug, jsonl_path, parent_agent,
  parent_block_id, session_id, spawned_at, last_event_at, status,
  event_count, model, workflow_id: Option<String>, display_name`
- `SubagentStatus` (L64-77): `Active | Completed | Abandoned` — the
  `Abandoned` variant's doc comment cites
  `docs/specs/SPEC_SUBAGENT_LIFECYCLE_RECONCILIATION_2026_07_12.md`, which
  **does not exist on disk** (confirmed by search) — a dangling reference
  worth fixing alongside this redesign.
- `WorkflowInfo` (L100-113): `workflow_id, parent_agent, parent_block_id,
  session_id, agents_total, agents_done, status, last_event_at`
- `WorkflowStatus`: `Running | Completed`

**RPC surface** (`agentmux-srv/src/server/service/misc.rs:47-88`, all under
the `"subagent"` namespace): `ListActive → Vec<SubagentInfo>`,
`ListWorkflows → Vec<WorkflowInfo>`, `GetHistory`, `GetInfo`, `GenerateName`
(`agentmux-srv/src/server/app_api/session.rs:195-279`, keyed on
`agent_id`/`jsonl_path`/`parent_block_id`).

**WS events** (emitted only from `subagent_watcher.rs`): `subagent:spawned`,
`subagent:activity`, `subagent:completed`, `workflow:updated`.

**Frontend** — two parallel implementations:
- Current: `frontend/app/view/swarm/swarm-model.ts` — `ActiveSubagent`,
  `WorkflowGroup` (client-synthesized, §1), `SwarmChild = ActiveSubagent |
  WorkflowGroup`, `groupSubagentsByWorkflow` (L96-126), `buildTree()`
  (L583-618). Identity-preservation hacks at L128-218 (see §1).
- Legacy: `frontend/app/view/subagent/subagent-model.ts` — separate,
  simpler types, **not workflow-aware at all** (no `workflow_id`/
  `display_name`). Still present; §6 asks what to do with it.

**Doc trail** (chronological, all in `docs/specs/`): `swarm-redesign-active-retired-2026-05-03.md`
→ `SPEC_SWARM_TREE_REDESIGN_2026_06_19.md` → `SPEC_SWARM_LIVE_FEED_BINDINGS_2026_07_05.md`
(states, pre-`WorkflowInfo`: *"No workflow tracking... Workflow tool runs
exist only as files"* — the doc that led to `WorkflowInfo` being bolted on)
→ `SPEC_SWARM_LIVE_FEED_UI_2026_07_05.md` → `REPORT_SWARM_SUBAGENT_HISTORY_FLOOD_2026_07_07.md`
(root cause: state reconstructed from raw filesystem truth with no scoping —
the same root-cause shape as this session's OOM retro) →
`REPORT_SWARM_SUBAGENT_DETAIL_UX_ANALYSIS_2026_07_07.md` (→ `display_name`/
`GenerateName`).

## 5. Proposed data model

```rust
/// One per Agent-tool (Task tool) call or Workflow-tool call — the unit of
/// dispatch from a parent turn. "What got kicked off," not "what ran": a
/// Solo dispatch always has exactly one SubAgent; a Workflow dispatch has
/// however many the run has spawned so far (may still be growing).
pub struct AgentDispatch {
    /// Stable identity. Workflow kind: the run-id Claude Code already uses
    /// for `subagents/workflows/<run-id>/` — stable across srv restarts,
    /// unlike an agent_id. Solo kind: `format!("solo:{agent_id}")` — a solo
    /// dispatch is 1:1 with its one member, so no new ID-minting is needed.
    pub dispatch_id: String,
    pub kind: DispatchKind,          // Solo | Workflow
    pub parent_agent: String,
    pub parent_block_id: String,
    pub session_id: String,
    pub spawned_at: u64,
    pub last_event_at: u64,
    pub status: DispatchStatus,      // Running | Completed | Abandoned
    pub member_count: usize,
    pub members_done: usize,
}

pub enum DispatchKind { Solo, Workflow }

/// Status aggregation rule: Completed iff every member is Completed;
/// Abandoned if the dispatch's parent turn ended with any member not
/// Completed (mirrors SubAgentStatus::Abandoned's existing reconciliation
/// rule, applied one level up); else Running.
pub enum DispatchStatus { Running, Completed, Abandoned }

/// One spawned Claude Code agent instance. Unchanged in spirit from today's
/// SubagentInfo — only workflow_id: Option<String> becomes a mandatory
/// dispatch_id, present for solo members too (today they have no container
/// reference at all).
pub struct SubAgent {
    pub agent_id: String,
    pub slug: String,
    pub jsonl_path: String,
    pub dispatch_id: String,         // was: workflow_id: Option<String>
    pub parent_agent: String,
    pub parent_block_id: String,
    pub session_id: String,
    pub spawned_at: u64,
    pub last_event_at: u64,
    pub status: SubAgentStatus,      // Active | Completed | Abandoned — unchanged
    pub event_count: usize,
    pub model: Option<String>,
    pub display_name: Option<String>,
    /// New (§9.2): the JSONL transcript's own `"parentUuid"` field, parsed
    /// verbatim. `None` in every real transcript checked so far (228/228) —
    /// nested subagent spawning is permitted for unrestricted-tool subagent
    /// types but has never actually been observed on this machine. Captured
    /// defensively since we already read this line for other fields; if
    /// ever `Some`, this SubAgent is a grandchild of another SubAgent, not
    /// a direct member of its nominal `dispatch_id` — the frontend should
    /// attribute/nest it accordingly rather than showing it as a flat peer.
    pub spawned_from_agent_id: Option<String>,
}
```

**RPC surface** (naming here is a detail, not load-bearing — bikeshed freely):
- `subagent.ListDispatches → Vec<AgentDispatch>` — replaces
  `subagent.ListWorkflows`, but now also lists Solo dispatches. This is the
  RPC that lets the frontend delete `WorkflowGroup`/`groupSubagentsByWorkflow`
  entirely (§6) — every dispatch, solo or workflow, is now a real
  backend-issued list item with a stable ID.
- `subagent.ListActive → Vec<SubAgent>` — renamed from `SubagentInfo`, same
  shape plus `dispatch_id`.
- `subagent.GetHistory`, `subagent.GetInfo`, `subagent.GenerateName` —
  unchanged, still keyed on `agent_id`.
- Optional: `subagent.GetDispatch(dispatch_id) → AgentDispatch` for a
  detail-view fetch; not required for an MVP cutover.

**WS events:**
- `subagent:spawned` / `subagent:activity` / `subagent:completed` — unchanged
  wire shape, `workflow_id` field replaced by `dispatch_id`.
- `workflow:updated` → **`dispatch:updated`** — now fires for both kinds, not
  just Workflow. This is what removes the frontend's need to synthesize a
  group locally: Solo dispatches get the same "the container changed, here's
  its stable ID" signal Workflow dispatches already get.

## 6. Frontend implications

- `swarm-model.ts`'s `WorkflowGroup`, `groupSubagentsByWorkflow`,
  `shallowEqualGroup`/`stabilizeGroupIdentity`/`pruneGroupIdentityCache`
  (L59-72, 96-126, 165-218) become **deletable**, not just simplified: once
  `subagent.ListDispatches` returns a real, backend-stable `dispatch_id` for
  every dispatch (solo included), the exact same reference-preservation
  technique already used for `ActiveSubagent`
  (`mergeSubagentsPreservingIdentity`, L128-163) applies uniformly one level
  up too — no bespoke synthesized-group version needed.
- `buildTree()` (L583-618) simplifies from "one real level + one synthesized
  level" to two real levels (`AgentDispatch` → `SubAgent`).
- `frontend/app/view/subagent/subagent-model.ts` (the older, non-workflow-
  aware standalone pane) is a decision point this spec surfaces but doesn't
  resolve: retire it in favor of the Swarm pane exclusively, or bring it onto
  the new two-level model too. Flagging rather than deciding since it's a
  product-surface call, not a schema one.

## 7. Swarm-pane rendering rule: one row per AgentDispatch, members concatenated

**Confirmed pattern, not just plausible:** a session typically has a handful
of `AgentDispatch`es (single digits), but a Workflow-kind dispatch can have
hundreds to low-thousands of members — this session's own crash investigation
found workflow `wf_ff1c5825-522` with **1,030+ member subagent files** in one
run (`docs/retro/retro-subagent-backfill-storm-oom-2026-07-17.md`). Workflow
patterns are *designed* to fan out this way (loop-until-dry, judge panels,
multi-modal sweeps) — a single dispatch legitimately produces far more
members than a human would ever want listed as individual rows.

**Current behavior falls short at this scale.** `WorkflowGroupRow`
(`frontend/app/view/swarm/swarm-view.tsx:315-354`) already collapses the
*top-level* row — one row per `workflow_id`, not one per member, matching
`REPORT_SWARM_SUBAGENT_HISTORY_FLOOD_2026_07_07.md`'s "group, not truncate"
approach. But its expand state (L345-350) still renders one `<SubagentRow>`
per member via `<For each={group.subagents}>` — fine at the scale the
surrounding code comment anchors to (`"observed live: 45"`,
`swarm-model.ts:66`), but this session's real corpus is ~20x past that:
expanding a 1,030-member dispatch today would mount 1,030 `SubagentRow`
components, each with its own state/handlers/detail-expand affordance.

**New rule: a Workflow-kind `AgentDispatch` never renders one row per
member, at any expand depth.** Expanding it shows a single **concatenated
activity feed** instead of N nested rows — every member's events merged into
one chronological, scrollable stream, each entry tagged with which member it
came from (agent_id/slug/display_name prefix), the same shape as `docker
compose logs` interleaving multiple services with a per-line service tag, or
a CI matrix job's combined-log view. No per-member expand/collapse
affordance, no per-member row mount cost, regardless of whether the dispatch
has 3 members or 3,000.

- **Solo-kind `AgentDispatch`** (`member_count == 1`) needs no wrapper row at
  all — render its one member directly, identical to today's un-grouped
  `SubagentRow`. Not a behavior change from today, just confirmed under the
  new schema: the "concatenate" rule only has teeth once there's more than
  one member to concatenate.
- **`NameGroup`'s role shifts, not disappears.** Today it groups *loose*
  (non-workflow) subagents sharing a generated `display_name`
  (`swarm-model.ts:84-119`) — e.g. repeated "review this file" calls across
  many files. Under the new schema that becomes grouping multiple *separate
  solo* `AgentDispatch`es sharing a generated name — same real, common
  pattern, just operating one level up (across dispatches, not across raw
  members within one dispatch).
- **Backend event-coalescing follow-on — do this in the same pass, not as a
  separate later fix.** Today every member, however many, broadcasts its own
  `subagent:spawned`/`subagent:activity`/`subagent:completed` WS event
  individually — the exact mechanism behind the 1,030-event-in-10-seconds
  broadcast storm in the OOM retro. If the product-level view for a large
  Workflow dispatch is a concatenated feed rather than N individually-
  tracked rows, there is no remaining product reason to broadcast N granular
  per-member events for a large dispatch. The backend can batch new events
  across a dispatch's members into a single throttled `dispatch:activity`
  broadcast (e.g. coalesced every 250ms–1s while a dispatch has pending
  events) instead of one WS message per member-event. This shrinks the
  blast radius of the exact failure mode that caused the crash — not just
  the render cost — so it belongs in the same implementation pass as the
  `AgentDispatch` schema itself, not filed as a later follow-up.
- **Open question added to §9**: is the concatenated feed's ordering a
  strict chronological interleave across all members, or grouped-by-member-
  then-chronological-within? And — mirroring PR #2205's `BACKFILL_MAX_FILES`
  precedent — should the concatenated feed itself be capped/paginated rather
  than rendering unbounded per-dispatch history inline, given a dispatch can
  now legitimately have thousands of events behind it?

## 8. Migration plan

No data to migrate (§4) — this is a code-only change, phaseable and each
phase independently shippable:

1. **Backend types**: add `AgentDispatch`/`DispatchKind`/`DispatchStatus`;
   rename `SubagentInfo`→`SubAgent`, `workflow_id: Option<String>`→
   `dispatch_id: String`, add `spawned_from_agent_id: Option<String>` parsed
   from the transcript's `parentUuid` (§9.2). `WorkflowInfo`/`WorkflowStatus`
   are subsumed (every existing `WorkflowInfo` becomes a
   `DispatchKind::Workflow` `AgentDispatch`).
2. **RPC + WS**: add `ListDispatches`, retire `ListWorkflows`; add
   `dispatch:updated`, retire `workflow:updated`; thread `dispatch_id`
   through the three `subagent:*` event payloads.
3. **Frontend**: point `swarm-model.ts` at `ListDispatches` +
   `dispatch:updated`; delete the synthesized-group identity code (§6);
   update `swarm-view.tsx`'s render tree to the two real levels.
4. **Legacy pane decision** (§6) — separate follow-up, not blocking 1-3.
5. **Optional follow-up, not required by this spec**: revisit PR #2205's
   `BACKFILL_MAX_FILES` cap (currently caps raw JSONL file count on cold
   backfill) to cap by dispatch recency instead, now that `dispatch_id`
   exists as a natural grouping key — a more meaningful unit than raw file
   count for the same reason `AgentDispatch` is more meaningful than a flat
   file list generally.
6. **Fix the dangling doc reference**: `SubAgentStatus::Abandoned`'s comment
   cites a spec that doesn't exist (§4) — either this document supersedes
   that citation, or a short lifecycle-reconciliation note should be written
   to actually back it.

## 9. Open questions

1. **Naming, final call: RESOLVED — `AgentDispatch` + `SubAgent`** (§3's
   primary recommendation). Decided 2026-07-17.
2. **Depth: RESOLVED — two levels, empirically confirmed, not just assumed.**
   Structurally, nesting *is* possible: `general-purpose` and `claude`
   (catch-all) subagent types have unrestricted tool access (`Tools: *`),
   which includes the Agent tool itself — a subagent of one of those types
   could call it and spawn a grandchild. `Explore`/`Plan` explicitly exclude
   the Agent tool, blocking this for those types. The CLI has a field for
   exactly this case: every subagent transcript's first JSONL line carries
   `"parentUuid"`, presumably recording which turn's UUID the subagent's
   conversation forked from. **Checked every real subagent transcript on
   this machine — 228 files across 8+ distinct project sessions, spanning
   weeks of actual usage, including sessions that used unrestricted-tool
   subagent types — `parentUuid` is `null` in every single one.** Nested
   spawning has never been observed to actually happen here, despite being
   permitted. Two levels is empirically sufficient today.
   `subagent_watcher.rs` currently does not parse `parentUuid` at all, so if
   nesting ever did occur, AgentMux would have no way to notice — a
   grandchild would land as an unexplained flat sibling file. Since we're
   already reading this file's first line for other fields, capture
   `parentUuid` from day one anyway (cheap, defensive) rather than adding it
   only after the first real occurrence with no historical baseline to
   validate the two-level assumption against — see §5's `SubAgent.
   spawned_from_agent_id` field and §8 step 1.
3. **`DispatchStatus::Abandoned` semantics**: proposed aggregation rule in
   §5 (`Completed` iff all members `Completed`) needs a decision on the edge
   case of a Workflow dispatch with **zero** members yet (just started,
   before its first member file appears) — `Running`, presumably, but worth
   confirming explicitly in the implementation spec.
4. **Concatenated-feed ordering and cap** (§7): strict chronological
   interleave across all members, or grouped-by-member-then-chronological-
   within? And should the feed itself be capped/paginated (mirroring PR
   #2205's `BACKFILL_MAX_FILES` precedent) rather than rendering unbounded
   per-dispatch history inline, now that a dispatch can legitimately have
   thousands of events behind it?
5. **`dispatch:activity` coalescing window** (§7): 250ms–1s was floated as a
   plausible throttle range, not measured — needs a real number, likely
   tuned against how "live" the concatenated feed needs to feel for an
   actively-running large dispatch versus how much broadcast volume it saves.

## 10. Sources

- `agentmux-srv/src/backend/subagent_watcher.rs` (current types, file-layout
  doc comment, `Abandoned` dangling citation)
- `agentmux-srv/src/server/service/misc.rs:47-88`, `agentmux-srv/src/server/app_api/session.rs:195-279`
  (RPC surface)
- `frontend/app/view/swarm/swarm-model.ts`, `frontend/app/view/swarm/swarm-view.tsx:315-354` (current per-member expand rendering), `frontend/app/view/subagent/subagent-model.ts`
- `docs/retro/retro-subagent-backfill-storm-oom-2026-07-17.md` (this session — the crash that motivated re-examining this schema)
- `docs/specs/SPEC_SWARM_LIVE_FEED_BINDINGS_2026_07_05.md`, `REPORT_SWARM_SUBAGENT_HISTORY_FLOOD_2026_07_07.md`, `REPORT_SWARM_SUBAGENT_DETAIL_UX_ANALYSIS_2026_07_07.md`
- GitHub Actions docs (Workflow Run → Job → Step): https://runs-on.com/github-actions/jobs-and-steps/
- Apache Airflow docs (DAG Run → Task Instance): https://www.sparkcodehub.com/airflow/task-management/task-instances
- Temporal docs (Workflow Execution → Activity/Child-Workflow Execution): https://docs.temporal.io/child-workflows
- OpenTelemetry span parent-child model: https://opentelemetry.io/docs/concepts/signals/traces/
- Google ADK sub-agent delegation model (recursive, same-noun-per-level): search summary, 2026 multi-agent framework comparisons (dev.to/medium roundups, April 2026)
- Swarm-pattern Lead/Worker terminology: https://www.ai21.com/glossary/foundational-llm/agent-swarm/, https://docs.agent-swarm.dev/docs
