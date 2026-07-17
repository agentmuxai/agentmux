// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Subagent adapter — maps `ActiveSubagent`s (swarm-model.ts, sourced from the
 * shared `subagent-source.ts` singleton) onto `PinnedActivity` for the dock,
 * filtered to one agent pane's own spawns (`parent_block_id`). Phase 2 of the
 * dock — proves the `PinnedActivity` abstraction generalizes beyond shells.
 *
 * `canStop` is always `false`: no subagent-cancel RPC/UI exists anywhere in
 * the app today (confirmed — `AgentStopCommand` targets a pane's own agent
 * process, not a subagent by `agent_id`; the Swarm pane itself has no cancel
 * action either). Wiring a real cancel is its own follow-up, not invented
 * here just because the dock's row chrome has a stop button slot.
 *
 * Grouping: a single Workflow-tool call can spawn dozens of subagents at once
 * (the Swarm pane's own docs cite 45 observed live —
 * REPORT_SWARM_SUBAGENT_HISTORY_FLOOD_2026_07_07.md Finding 4). Left flat,
 * one dock row per `ActiveSubagent` reads as "dozens of subagents" for what
 * was really one Agent-tool call from this pane's perspective — reported
 * live 2026-07-16. Reuses the Swarm pane's own `groupSubagentsByWorkflow`
 * (shared `workflow_id` → one `WorkflowGroup`; same-name loose subagents →
 * one `NameGroup`) instead of re-deriving the grouping rules, so the dock and
 * the Swarm tree always agree on what counts as "one call".
 *
 * Known simplification: a grouped row's expanded view and tail text show
 * only the group's most-recently-active member (`representative`), not every
 * member's transcript — the dock's row chrome (ActivityRow.tsx) renders one
 * `subagent` per row today; teaching it a real multi-member expanded view is
 * a follow-up, not required to fix the row-count explosion this addresses.
 *
 * Spec: docs/specs/SPEC_LONG_RUNNING_SHELL_PINNED_DOCK_2026_06_15.md (§3, §7)
 */

import {
    groupCacheKey,
    groupSubagentsByWorkflow,
    isNameGroup,
    isWorkflowGroup,
    type ActiveSubagent,
    type NameGroup,
    type SwarmChild,
    type WorkflowGroup,
} from "../../swarm/swarm-model";
import type { ActivityStatus, PinnedActivity } from "./types";

function subagentStatusToActivity(s: ActiveSubagent["status"]): ActivityStatus {
    switch (s) {
        case "active": return "running";
        case "completed": return "done";
        // Terminated without a Result line (parent turn ended early) — same
        // bucket as a shell's manual "stopped", not "error" (no failure
        // signal, just cut off). See SPEC_SUBAGENT_LIFECYCLE_RECONCILIATION.
        case "abandoned": return "stopped";
    }
}

export function subagentToActivity(s: ActiveSubagent): PinnedActivity {
    return {
        id: s.agent_id,
        kind: "subagent",
        // `||`, not `??` — an empty-string slug (before the watcher reads the
        // JSONL's first line) must fall through to agent_id, same as
        // swarm-view.tsx's own equivalent fallback chain.
        title: s.display_name || s.slug || s.agent_id,
        status: subagentStatusToActivity(s.status),
        startedAt: s.spawned_at,
        // last_event_at at the moment a subagent completes (or is reconciled
        // to abandoned) is the closest thing to an "ended at" ActiveSubagent
        // exposes (there is no dedicated field). Both are terminal — leaving
        // abandoned out would tick the elapsed timer forever and never let
        // D4 retention (RETENTION_MS.stopped) auto-dismiss the row.
        endedAt: s.status === "completed" || s.status === "abandoned" ? s.last_event_at : undefined,
        canStop: false,
        subagent: s,
    };
}

/** One dock row for a whole `WorkflowGroup`/`NameGroup` batch — see the
 *  module comment's "Known simplification" for why `subagent` is a single
 *  representative member rather than the full group. `id` reuses the Swarm
 *  pane's own `groupCacheKey` (`wf:<id>` / `name:<blockId>:<name>`) so the
 *  dock and the Swarm tree never invent two different identity schemes for
 *  the same group. */
function groupToActivity(g: WorkflowGroup | NameGroup): PinnedActivity {
    const representative = g.subagents[0]; // groupSubagentsByWorkflow sorts newest-first
    return {
        id: groupCacheKey(g),
        kind: "subagent",
        title: `${g.name} (${g.totalCount})`,
        status: g.status === "active" ? "running" : "done",
        startedAt: Math.min(...g.subagents.map((s) => s.spawned_at)),
        endedAt: g.status === "retired" ? g.lastEventAt : undefined,
        canStop: false,
        subagent: representative,
    };
}

function swarmChildToActivity(c: SwarmChild): PinnedActivity {
    if (isWorkflowGroup(c) || isNameGroup(c)) return groupToActivity(c);
    return subagentToActivity(c);
}

/** This pane's own subagents (`parent_block_id === blockId`), grouped by
 *  `workflow_id`/shared name exactly as the Swarm tree groups them, then
 *  mapped to activities — one dock row per Agent-tool call, not per
 *  subagent. */
export function subagentActivities(all: ReadonlyArray<ActiveSubagent>, blockId: string): PinnedActivity[] {
    const mine = all.filter((s) => s.parent_block_id === blockId);
    return groupSubagentsByWorkflow(mine).map(swarmChildToActivity);
}
