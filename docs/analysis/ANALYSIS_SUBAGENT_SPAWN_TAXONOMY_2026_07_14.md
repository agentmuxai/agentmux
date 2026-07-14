# Subagent spawn taxonomy: every way a subagent can come into existence

**Date:** 2026-07-14
**Status:** Reference doc — informs Swarm pane data-model design, not itself a spec
**Scope:** Every distinct shape a Claude Code CLI subagent-spawn can take, as
observed in AgentMux's own code (`agentmux-srv/src/backend/subagent_watcher.rs`,
`frontend/app/view/swarm/swarm-model.ts`) and as documented/confirmed by
Anthropic's official Claude Code docs (`code.claude.com/docs/en/*`) and
CHANGELOG.
**Related:** `docs/specs/SPEC_SUBAGENT_LIFECYCLE_RECONCILIATION_2026_07_12.md`,
`docs/specs/REPORT_SWARM_SUBAGENT_HISTORY_FLOOD_2026_07_07.md`,
`docs/specs/SPEC_DRONE_CANVAS_NODE_EDITOR_2026_06_05.md`, memory
`agentmux-swarm-duplicate-subagent-groups.md`.

---

## 0. Direct answer

> "Can subagents be launched without a workflow, and without a batch?"

**Yes.** A subagent can be dispatched entirely solo — one `Task`/`Agent`-tool
call, no sibling calls in the same turn, no `subagents/workflows/<run-id>/`
directory anywhere on disk. That's the baseline case (Shape A, §2.1 below).

But the question bundles two independent axes that need separating:

- **"Batch"** — did the CLI dispatch *multiple* subagents together?
- **"Workflow"** — did that dispatch go through Anthropic's own scripted
  **Dynamic workflows** feature, or was it ad-hoc/turn-by-turn?

Crossing those two axes gives four real cells, plus a fifth axis (recursion
depth) that's orthogonal to both:

| | Solo (1 subagent) | Batch (2+ subagents) |
|---|---|---|
| **Ad-hoc (turn-by-turn)** | Shape A — loose singleton | Shape B — ad-hoc parallel batch (shared `slug`, no workflow dir) |
| **Dynamic workflow (scripted)** | Shape C, 1-agent run | Shape C — workflow-tool batch (`subagents/workflows/<run-id>/`) |

...and independently, **any** cell above can additionally be nested one or
more levels deep (§2.4) — a subagent spawned under any of the four cells can
itself dispatch further subagents, confirmed officially supported up to 5
levels as of Claude Code v2.1.172+.

So: yes to "no workflow," yes to "no batch," and those are two separate yeses,
not one.

---

## 1. Terminology: "workflow" means three things — but two of the three may be the same thing

This codebase's biggest self-inflicted confusion risk. Resolve all three
before writing anything that says "workflow" out loud.

**Meaning 1 — Claude Code CLI's own official "Dynamic workflows" feature.**
Confirmed via Anthropic's primary docs (`code.claude.com/docs/en/workflows`,
`code.claude.com/docs/en/agents`), requires CLI v2.1.154+: *"A dynamic
workflow is a JavaScript script that orchestrates subagents at scale. Claude
writes the script for the task you describe, and a runtime executes it in the
background."* Scripts use `agent()`/`parallel()`/`pipeline()` primitives
(concurrency cap ~16, 1,000-agent total ceiling per run), and — this is the
load-bearing detail — *"Every run writes its script to a file under your
session's directory in `~/.claude/projects/`."* This is the exact mechanism
the `deep-research` workflow used to produce the research this doc is built
on, and the exact mechanism this Claude session's own `Workflow` tool wraps.

**Meaning 2 — AgentMux's own pipeline/canvas feature.** RFC'd internally as
"Workflows" (GitHub issues #753, #832) but shipped as **Drone** specifically
to avoid colliding with Meaning 1. Explicit quote,
`SPEC_DRONE_CANVAS_NODE_EDITOR_2026_06_05.md:15`: *"'Workflows' is unrelated —
that term now refers strictly to Claude's own workflows feature."* Evidenced
by the legacy `db_workflow_definitions`/`db_workflow_runs` tables, now dropped
and migrated into `db_drone_*`.

**Meaning 3 — `GitHubContext.workflow_run_id`** (`agents.rs:193`) — GitHub
Actions CI run correlation. Unrelated to subagents entirely, but the string
"workflow" shows up in logs/code near agent context regardless, so it's a
real grep-confusion risk.

**The open question this doc cannot fully close without a live trace:**
`subagent_watcher.rs`'s `parse_workflow_id()` derives `Some(<id>)` from a
`subagents/workflows/<run-id>/` directory segment it finds on disk — pure
filesystem observation, AgentMux never creates this structure itself. Given
Meaning 1's confirmed behavior (*"every run writes its script to a file under
your session's directory"*) and the fact that this session's own `Workflow`
tool calls emit a `journal.jsonl` with *"one `{"type":"result",...}` line per
completed agent"* — a structural match to what `subagent_watcher.rs` already
parses — **the best-supported hypothesis is that Meaning 1 and the
`subagents/workflows/<run-id>/` directory shape are the same thing**: what
`subagent_watcher.rs` calls a "workflow batch" is very likely literally
Anthropic's Dynamic Workflows feature's own on-disk output, not a separate,
coincidentally-named convention. This was assumed-but-unconfirmed prior to
this doc; it is now well-supported by primary sources but still not
byte-for-byte verified against a live Dynamic-workflow run's directory tree.
**Recommended follow-up:** trigger a `Workflow()` call from within an
AgentMux-hosted session and confirm the resulting `subagents/workflows/<id>/`
directory's `journal.jsonl` shape matches what `subagent_watcher.rs` expects,
closing this out definitively.

---

## 2. The five real shapes

### 2.1 Shape A — Loose / ad-hoc solo subagent

One `Task`/`Agent`-tool call (renamed from `Task` to `Agent` in CLI v2.1.63;
`Task(...)` still works as an alias), dispatched turn-by-turn with no siblings
in the same turn. On disk: `subagents/agent-<id>.jsonl` directly —
`parse_workflow_id()` returns `None`. `workflow_id: Option<String> = None` in
`SubagentInfo`. This is the base case; nothing else needs to be true for a
subagent to exist.

Anthropic's own docs (`code.claude.com/docs/en/agents`) explicitly contrast
this mode against Dynamic workflows in a comparison table: ad-hoc/turn-by-turn
delegation vs. *"Dynamic workflows — a script that runs many subagents and
cross-checks their results, for work too big to coordinate one turn at a time
or that needs more than a single pass."* Ad-hoc dispatch has no official name
of its own beyond "the Task/Agent tool used directly" — it's the default,
undocumented-because-it's-the-baseline mode.

### 2.2 Shape B — Ad-hoc parallel batch (no workflow dir, shared slug)

Multiple `Task`/`Agent`-tool calls issued within the **same turn**, still with
no `subagents/workflows/<id>/` directory — each subagent gets its own loose
`subagents/agent-<id>.jsonl` file (still Shape A structurally on disk) — but
the CLI-generated `slug` (a human-readable per-batch codename, read once from
the first JSONL line, `subagent_watcher.rs:1223`) is **identical across every
member of the batch**.

This is **not an officially-named Anthropic concept.** No primary source in
this doc's research confirms a first-party term for "several ad-hoc Task
calls issued in the same turn." (Note: `/batch` — see §3 below — is a
*different*, officially documented mechanism and should not be conflated with
this.) This shape is an **empirical AgentMux-side finding**, established via
live traces this session: the historical "flood" reports (`Mazs`: 13
subagents, one slug; `Loap`: 45 events, mostly one slug) and the duplicate-
label bug just fixed this session (`Lzop`: 17 `agent_id`s, one slug, ~50ms
spawn window — PR #2149). Because it's unofficial and only observationally
confirmed, **it should not be assumed stable across CLI versions** — the slug
generation/sharing behavior could change without a changelog entry, since it
isn't a documented contract.

Currently `subagent_watcher.rs`/`swarm-model.ts` do **not** structurally group
this shape. It falls into the general "loose" bucket, and is only
coincidentally caught by the frontend's `NameGroup` heuristic (grouping loose
subagents that share a Haiku-generated `display_name`) — a mechanism designed
for something else entirely (cosmetic name-collision grouping) and not
guaranteed to catch every batch member, since `display_name` resolution is
per-subagent, lazy, and not derived from the shared `slug` at all.

### 2.3 Shape C — Dynamic-workflow batch

`subagents/workflows/<run-id>/agent-<id>.jsonl` + a `journal.jsonl` in the
same `<run-id>/` directory. `parse_workflow_id()` returns `Some(<run-id>)`.
This is the shape produced by (per the current best hypothesis in §1)
Anthropic's own Dynamic Workflows feature — a script using `agent()`,
`parallel()`, `pipeline()`. `WorkflowGroup` in `swarm-model.ts` is keyed on
this `workflow_id`, with no minimum-member gate (a workflow of exactly 1
`agent()` call still produces this directory shape, so "workflow" and
"batch-of-2+" are not synonymous either — a 1-member workflow run is
structurally Shape C, not Shape A, even though it superficially looks solo).

A **stray** file sitting two segments deep directly in `subagents/workflows/`
(not inside a further `<run-id>/` subdirectory) is explicitly excluded from
Shape C by the existing test suite
(`workflow_id_none_for_stray_file_in_workflows_dir`) — treated as malformed/
incomplete rather than a real workflow member.

### 2.4 Recursive / nested subagents — confirmed real, NOT tracked by AgentMux

**This is the most important actionable finding in this doc.** Prior
internal-only investigation (this session, before this doc) flagged
recursive/grandchild subagent spawning as "unconfirmed/undesigned — inferred
from reading the algorithm, not backed by any test, comment, or live trace."
External research **upgrades this from unconfirmed to confirmed-and-shipping**:

- CHANGELOG (`github.com/anthropics/claude-code/blob/main/CHANGELOG.md`),
  verified via direct `curl` of the raw file, `## 2.1.172`: *"Sub-agents can
  now spawn their own sub-agents (up to 5 levels deep)."*
- `## 2.1.181`: *"forked subagents now count toward the depth cap"* — i.e.
  the cap isn't just "5 calls deep," it accounts for forked/resumed subagent
  chains too.
- `## 2.1.208`: *"Fixed foreground subagents spawning unbounded nested
  chains; they now respect the same 5-level depth limit as background..."* —
  confirms the 5-level cap applies uniformly to both foreground and
  background subagents, and that this was itself a bug fix (foreground
  subagents briefly had *no* cap before this release).
- Anthropic's own subagent docs (`code.claude.com/docs/en/sub-agents`):
  *"As of Claude Code v2.1.172, a subagent can spawn its own subagents...
  Depth is counted as [distance from the top-level turn]."*

**AgentMux has zero code path for this.** `watch_agent()` is only ever
installed for a top-level, AgentMux-registered agent — never re-installed for
a subagent's own discovered `agent_id`. No `parent_subagent_id` field exists
anywhere in `SubagentInfo` or the frontend model. `SPEC_SUBAGENT_LIFECYCLE_
RECONCILIATION_2026_07_12.md`'s §5 core assumption ("a subagent's lifetime is
bounded by its parent's own turn") is written entirely in terms of top-level-
pane parentage and never contemplates a subagent-of-a-subagent.

Practical consequence: if a top-level subagent (in any of Shapes A/B/C) uses
its own delegated Task/Agent-tool call, **AgentMux currently has no way to
observe, attribute, or display that grandchild subagent at all** — it's
either silently invisible (if nothing walks that far into the filesystem
tree) or, worse, could be misattributed as a sibling of its parent if the
scanning logic doesn't distinguish directory depth carefully. This has not
been observed as a live bug yet (no confirmed live trace exists either way in
this environment), but it is now a **known, real gap against a shipping CLI
capability**, not a hypothetical.

### 2.5 Abandoned — a status overlay, not a spawn shape

`SubagentStatus::Abandoned` is not a fifth way to spawn — it's a
runtime-computed liveness overlay on top of Shapes A/B/C, set only by
`reconcile_stale_subagents()`, called only from `scan_session_subagents()` on
pane-reopen. Explicitly documented as reopen-time-only, not real-time, per
`SPEC_SUBAGENT_LIFECYCLE_RECONCILIATION_2026_07_12.md` §9 Open Question 1.
Listed here only to make explicit that it doesn't belong in the same
conceptual bucket as A/B/C/nesting — don't design a sixth "spawn shape" box
for it.

---

## 3. Adjacent mechanisms (mentioned for completeness, not currently relevant to AgentMux's swarm model)

- **`/batch` skill** — officially documented
  (`code.claude.com/docs/en/agents`): *"a skill that has Claude split one
  large change into 5 to 30 worktree-isolated subagents that each open a pull
  request. It's a packaged use of subagents and worktrees, not a separate
  coordination style."* Distinct from Shape B/C: worktree-isolated, one PR
  per subagent, invoked as a named skill rather than an ambient batch. Not
  currently observed or specially handled anywhere in AgentMux — would
  presumably show up as either Shape B or Shape C on disk depending on how
  `/batch` itself dispatches internally (not confirmed by this research).
- **"Background agents"** (`background: true` in agent definitions, `Ctrl+F`
  to kill, introduced v2.1.49 per one GitHub-issue-sourced claim) — **lower
  confidence**: sourced from a single blog/issue reference, not among the
  25 adversarially-verified claims in this research pass. Worth a follow-up
  look if background-agent-shaped activity is ever seen live, but not
  asserted as confirmed here.
  and one bug source: `github.com/anthropics/claude-code/issues/64898`.
- **Hooks cannot spawn subagents** — per one GitHub-issue source (also
  below the adversarial-verification bar used elsewhere in this doc): hooks
  can return `additionalContext`, a permission decision, or a block, but
  cannot dispatch a Task/Agent-tool call themselves. Listed for completeness
  as a confirmed *non*-mechanism, worth re-checking if hook-triggered
  subagent activity is ever reported.

---

## 4. Risk callout: the on-disk JSONL format is explicitly undocumented-as-stable

Anthropic's own docs (`code.claude.com/docs/en/sessions`), directly
contradicting the assumption underlying `subagent_watcher.rs`'s entire
approach: *"The entry format is internal to Claude Code and changes between
versions, so scripts that parse these files directly can break on any
release. To build on session data, use `/export` or the script interfaces
instead."* `subagent_watcher.rs` does exactly the thing this warns against —
raw JSONL parsing of `subagents/agent-<id>.jsonl` and
`subagents/workflows/<run-id>/journal.jsonl`. This isn't a defect (there's no
alternative — AgentMux has to observe subagents somehow, and it's explicitly
100%-observational by design, never triggering spawns itself), but it means
**any CLI upgrade is a standing risk of silently breaking Swarm pane
data**, not just a one-time integration cost. Worth a lightweight canary (a
smoke test against a real subagent JSONL sample, re-run on CLI version bumps)
rather than only reactive bug reports when it breaks.

---

## 5. Summary table

| Shape | Filesystem signature | `workflow_id` | Batched? | Officially named? | AgentMux support |
|---|---|---|---|---|---|
| A — loose/solo | `subagents/agent-<id>.jsonl` | `None` | No | No (baseline/default mode) | Full |
| B — ad-hoc parallel batch | Same as A, ×N, shared `slug` | `None` | Yes | **No** — empirical only | None (coincidental `NameGroup` catch only) |
| C — dynamic-workflow batch | `subagents/workflows/<run-id>/…` + `journal.jsonl` | `Some(<run-id>)` | Usually (1-member workflows exist too) | **Yes** — Anthropic's "Dynamic workflows" | Full (`WorkflowGroup`) |
| D — recursive/nested | (same as A/B/C, one level deeper in spawn ancestry) | inherits parent's | orthogonal axis | **Yes** — confirmed, depth ≤ 5 | **None** — no `parent_subagent_id`, no re-`watch_agent()` |
| Abandoned | n/a — status overlay only | n/a | n/a | n/a | Reopen-time only, not real-time |

---

## 6. Implications for Swarm pane design (not a spec — flagging for the next design pass)

- Treat "batch" (B/C) and "nesting depth" (D) as **orthogonal axes**, not one
  taxonomy. A batch member can itself be nested; nesting can occur inside a
  solo dispatch too.
- Shape B needs its own grouping key, distinct from both `WorkflowGroup`
  (`workflow_id`) and `NameGroup` (Haiku `display_name`) — likely the shared
  `slug` scoped to same-parent-same-turn, now that it's understood as a
  real, load-bearing signal rather than noise. This directly extends the
  user's own proposal earlier this session ("show the slug as a Batch at the
  top level") — correct for Shape B specifically, where it's the *only*
  grouping signal available, even though it would be wrong as a *general*
  batching key for Shape C (workflow_id is authoritative there).
- Shape D (nesting) needs a `parent_subagent_id` field and a
  `watch_agent()`-equivalent re-install step per discovered subagent before
  the Swarm pane can claim "easy visibility into these," per the user's own
  stated purpose for the pane — right now a grandchild subagent is a blind
  spot, not merely an edge case.
