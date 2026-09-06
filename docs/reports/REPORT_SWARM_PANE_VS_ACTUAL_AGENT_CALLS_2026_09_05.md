# What an agent actually did vs. what the Swarm pane shows — Manoz session audit

**Status:** historical
**Author:** Posa
**Date:** 2026-09-05
**Method:** Read Manoz's live session transcript directly
(`468e2051-6f50-4621-b705-db0b35d61dd0.jsonl`, 26 MB, under the shared identity
`908412be-…` in channel `local-main-b28b7a-0d9e24f4`), bounded to everything after the
operator's `u there` at `2026-09-06T03:22:53Z` (20:22 local). Cross-referenced against the
Swarm pane's own display path in the current tree at `1cb5b51c0` (v0.55.36).

**Follow-up:** the Agent-tool rows visible alongside this session turned out to be mislabelled
"interrupted" — root cause in `REPORT_SUBAGENT_COMPLETION_NEVER_DETECTED_2026_09_05.md`.

**Scope note:** this audits the *fidelity of the Swarm pane's per-agent activity display*
against a real session. It is an observation report, not a defect report — every behaviour
below is working as designed. Whether the design is sufficient is the open question in §5.

---

## 1. What Manoz actually did

Three operator messages in the window:

| Time (UTC) | Message |
|---|---|
| 03:22:53 | `u there` |
| 03:24:43 | `you are now inside of 55.36` |
| 03:27:26 | `so we want to refine how memory is loaded and managed. There are a couple scenarios: 1) cr…` |

The work is a **read-only investigation of memory loading and compaction**: locating the
startup-payload builder, tracing how `AGENTMUX_MEMORY.md`/`CLAUDE.md` get written at launch,
reading the compaction-boundary handling, and searching for any post-compaction memory
re-injection.

**Tool calls in the window: 26 measurable.**

| Tool | Count |
|---|---|
| `Bash` | 24 |
| `AskUserQuestion` | 1 |
| `Write` | 1 |

Every `Bash` call is a distinct read: `grep -rn "memory_dir_for_agent…"`,
`sed -n '900,930p' agentmux-srv/src/backend/agent_config.rs`,
`cat frontend/app/view/agent/compact-boundary.ts`, and so on — each with its own one-line
`description` field (`"Read the compaction boundary handling"`, `"Find all consumers of
compaction events"`, …).

**Zero `mcp__agentmux__*` calls.** Worth stating plainly, since "agent calls" could be read
as the AgentMux MCP surface: Manoz used none of it in this window. No `SendMessage`, no
`Shell`, no `MemoryRead`, no fleet tools. Everything went through the plain `Bash` tool.

## 2. What the Swarm pane can show for that

Two independent per-agent surfaces, from different mechanisms:

**`currentTool`** — `agentmux-srv/src/backend/reactive/progress_watcher.rs`. A sweep folds
newly-appended transcript lines into per-block state; `current_tool` is
`self.open.first()`, i.e. *the oldest `tool_use` with no matching `tool_result` yet*. Rendered
in `swarm-view.tsx:354-359` as a bolt icon plus **the bare tool name**:

```tsx
<Show when={node.currentTool}>
    <div class="swarm-current-tool">
        <i class="fa-solid fa-bolt swarm-current-tool-icon" />
        <span class="swarm-current-tool-name">{node.currentTool}</span>
    </div>
</Show>
```

**`todoRows`** — same watcher, fed by the agent's own todo-tool calls
(`TaskCreate`/`TaskUpdate` item-at-a-time, or a whole-list call that supersedes them),
capped at `MAX_TODOS = 24`.

## 3. The mapping

| Manoz's actual call | What Swarm can render |
|---|---|
| `Bash` — "Check current repo version and environment/channel context" | `Bash` |
| `Bash` — "Locate memory, startup payload, and compaction modules" | `Bash` |
| `Bash` — "Read the compaction boundary handling" | `Bash` |
| `Bash` — "Find all consumers of compaction events" | `Bash` |
| `Bash` — "Search for any post-compaction memory re-injection" | `Bash` |
| … 19 further distinct `Bash` reads … | `Bash` |
| `AskUserQuestion` — compaction-direction choice | `AskUserQuestion` |
| `Write` | `Write` |

**24 semantically distinct investigative steps collapse to the identical string `Bash`.** The
`description` each call already carries — the thing that would actually distinguish them — is
not part of `AgentProgress` and never reaches the pane. Neither is the command, nor the target
file.

**The todo checklist is empty for this whole window.** Manoz called no todo tool, so
`todos` stays `[]` and `is_empty()` suppresses publication entirely for any tick where
`current_tool` is also `None`. An observer watching the Swarm card would see no task structure
for a ~9-minute multi-step investigation — correctly, since none was ever expressed.

## 4. The sharper finding: most calls are too short to ever be displayed

`SWEEP_INTERVAL_SECS = 3`, and `current_tool` reports whatever is open *at sweep time*. So a
call that opens and closes between two sweeps is never published as `currentTool` at all.

Measured, per call, from `tool_use` to its matching `tool_result` in this window:

| | |
|---|---|
| Calls measured | 26 |
| **Completed in under 3000 ms** | **23 (88%)** |
| 3000 ms or longer | 3 |
| Median | **523 ms** |
| Min / max | 28 ms / 53,223 ms |

Full distribution (ms): `28, 334, 339, 346, 363, 370, 420, 429, 471, 488, 497, 500, 500, 523,
533, 535, 548, 564, 801, 811, 850, 1173, 2211, 4040, 4230, 53223`.

The only reliably-visible call in the entire window is the **`AskUserQuestion` at 53 seconds** —
which is visible precisely because it was blocked waiting on a human. The two 4-second `Bash`
calls would likely catch one sweep each. **The other 23 are all shorter than a single sweep
interval**, so whether any given one is ever seen is down to where the 3s boundary happens to
fall.

This is not a bug — the watcher is a sampler and is documented as one ("The tool this agent is
running right now, or null between tools"). But it means that for a **read-heavy investigation
session**, which is what most agent work looks like, the `currentTool` line is empty or stale
far more often than it is informative. It shows activity best when the agent is *blocked*,
and worst when the agent is working fastest.

## 5. What this implies

Three observations, in order of how confident I am:

1. **Tool *name* alone is near-zero signal for `Bash`-dominated work** (24 of 26 calls here).
   The pane can tell you "Manoz is running Bash" — which, for an agent doing codebase
   investigation, is true almost continuously and says nothing about what it is investigating.
   The per-call `description` already exists in the transcript and is exactly the missing
   field.
2. **Sampling at 3 s under-reports sub-second work by construction.** An 88% miss rate on this
   session is not a tuning problem that a shorter interval fixes cheaply — it is inherent to
   sampling "what is open right now" rather than accumulating "what ran since the last sweep."
   A *last completed tool* (or a small ring of recent ones) would survive between sweeps where
   an instantaneous read cannot.
3. **`activitySummary` is the surface already doing this job.** `swarm-view.tsx` renders the
   Haiku-generated paraphrase directly above `currentTool`, and the code comment there frames
   the literal tool as complementary to it ("The literal call in flight, next to the Haiku
   paraphrase above it"). If the paraphrase is carrying the semantic load, the honest
   framing of the finding is that **`currentTool` is a liveness indicator, not an activity
   description** — and this session suggests it is a fairly weak liveness indicator too.

## 6. What I did not check

- Whether `activitySummary` was actually populated and accurate for this session — I read the
  render path, not its live values. That is the natural follow-up and would change the weight
  of §5.3 substantially.
- Any window other than this one. A session doing long builds or long agent dispatches would
  invert the duration distribution entirely, and the 88% figure should not be generalised
  beyond "a read-heavy investigation session."
- LAN/WAN-tier agents. `ListConversations` reports `remote_fetch_required: true` for those, so
  their transcripts are liveness-only and could not be audited this way.
