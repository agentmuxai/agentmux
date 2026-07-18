// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Subagent adapter — maps `ActiveSubagent`s (swarm-model.ts, sourced from the
 * shared `subagent-source.ts` singleton) onto `PinnedActivity` for the dock,
 * filtered to one agent pane's own spawns (`parent_block_id`). Phase 2 of the
 * dock — proves the `PinnedActivity` abstraction generalizes beyond shells.
 *
 * Groups by `dispatch_id` (collapsing a Workflow dispatch's members into one
 * row, mirroring the Swarm pane's tree view), then by shared `display_name`
 * among the solo leftovers — `NameGroup`, reused directly from
 * swarm-model.ts. Deliberately NOT reusing swarm-model.ts's
 * `buildDispatchChildren`/`WorkflowDispatch` for the dispatch half: that
 * type intentionally drops the member list (SPEC_AGENT_DISPATCH_SUBAGENT_
 * HIERARCHY_2026_07_17 §7 — a Workflow dispatch can have thousands of
 * members, too many to hold in the Swarm pane's frequently-recomputed tree
 * atom), but this adapter needs each member's `spawned_at`/`status` to
 * compute one summary row — and the dock's own scale (pinned activity, not
 * a full tree) makes holding that list here safe. Without grouping at all,
 * the dock would show one row per `ActiveSubagent` instead of one row per
 * Agent-tool-or-Workflow-tool call (a single workflow run can spawn dozens
 * to hundreds at once).
 *
 * `canStop` is always `false`: no subagent-cancel RPC/UI exists anywhere in
 * the app today (confirmed — `AgentStopCommand` targets a pane's own agent
 * process, not a subagent by `agent_id`; the Swarm pane itself has no cancel
 * action either). Wiring a real cancel is its own follow-up, not invented
 * here just because the dock's row chrome has a stop button slot.
 *
 * Spec: docs/specs/SPEC_LONG_RUNNING_SHELL_PINNED_DOCK_2026_06_15.md (§3, §7)
 */

import {
    groupCacheKey,
    isNameGroup,
    type ActiveSubagent,
    type NameGroup,
} from "../../swarm/swarm-model";
import type { ActivityStatus, PinnedActivity } from "./types";

/** One dock row summarizing every member of a shared `dispatch_id` — the
 *  dock-local counterpart of the Swarm pane's `WorkflowDispatch`, but
 *  carrying the member list (see this file's header comment for why). */
interface DispatchGroup {
    kind: "dispatchGroup";
    dispatchId: string;
    name: string;
    subagents: ActiveSubagent[];
    activeCount: number;
    totalCount: number;
    lastEventAt: number;
}

function isDispatchGroup(x: ActiveSubagent | DispatchGroup | NameGroup): x is DispatchGroup {
    return "kind" in x && x.kind === "dispatchGroup";
}

/** Group `subagents` (already filtered to one parent block) by `dispatch_id`
 *  — 2+ members sharing one id (always a Workflow dispatch; a Solo dispatch
 *  is 1:1 with its one member by construction) collapse into a
 *  `DispatchGroup`. Among what's left, 2+ subagents sharing an identical,
 *  non-empty `display_name` collapse into a `NameGroup`. A single subagent
 *  stays a loose, ungrouped row. */
function groupSubagentsForDock(
    subagents: ActiveSubagent[]
): (ActiveSubagent | DispatchGroup | NameGroup)[] {
    const byDispatch = new Map<string, ActiveSubagent[]>();
    for (const s of subagents) {
        const members = byDispatch.get(s.dispatch_id) ?? [];
        members.push(s);
        byDispatch.set(s.dispatch_id, members);
    }

    const loose: ActiveSubagent[] = [];
    const dispatchGroups: DispatchGroup[] = [];
    for (const [dispatchId, members] of byDispatch) {
        if (members.length < 2) {
            loose.push(...members);
            continue;
        }
        const sorted = [...members].sort((a, b) => b.last_event_at - a.last_event_at);
        const activeCount = sorted.filter((m) => m.status === "active").length;
        dispatchGroups.push({
            kind: "dispatchGroup",
            dispatchId,
            name: sorted.find((m) => m.slug)?.slug || dispatchId,
            subagents: sorted,
            activeCount,
            totalCount: sorted.length,
            lastEventAt: sorted[0]?.last_event_at ?? 0,
        });
    }

    const stillLoose: ActiveSubagent[] = [];
    const byName = new Map<string, ActiveSubagent[]>();
    for (const s of loose) {
        if (s.display_name) {
            const members = byName.get(s.display_name) ?? [];
            members.push(s);
            byName.set(s.display_name, members);
        } else {
            stillLoose.push(s);
        }
    }
    const nameGroups: NameGroup[] = [];
    for (const [name, members] of byName) {
        if (members.length < 2) {
            stillLoose.push(...members);
            continue;
        }
        const sorted = [...members].sort((a, b) => b.last_event_at - a.last_event_at);
        const activeCount = sorted.filter((m) => m.status === "active").length;
        nameGroups.push({
            kind: "nameGroup",
            name,
            parentBlockId: sorted[0].parent_block_id,
            subagents: sorted,
            activeCount,
            totalCount: sorted.length,
            status: activeCount > 0 ? "active" : "retired",
            lastEventAt: sorted[0]?.last_event_at ?? 0,
        });
    }

    return [...stillLoose, ...dispatchGroups, ...nameGroups];
}

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

/** A `DispatchGroup`/`NameGroup` → one dock row summarizing every member,
 *  instead of one row per subagent. `startedAt` is the earliest member's
 *  spawn (the group's own lifetime, not just its most-recently-active
 *  member's); `endedAt` only lands once every member is terminal, matching
 *  `subagentToActivity`'s own "no member still running" rule. */
function subagentGroupToActivity(group: DispatchGroup | NameGroup): PinnedActivity {
    const members = group.subagents;
    const anyAbandoned = members.some((m) => m.status === "abandoned");
    const status: ActivityStatus = group.activeCount > 0 ? "running" : anyAbandoned ? "stopped" : "done";
    return {
        id: isDispatchGroup(group) ? group.dispatchId : groupCacheKey(group),
        kind: "subagent",
        title: `${group.name} (${group.totalCount})`,
        status,
        startedAt: Math.min(...members.map((m) => m.spawned_at)),
        endedAt: group.activeCount === 0 ? group.lastEventAt : undefined,
        canStop: false,
        subagentGroup: { members },
    };
}

/** This pane's own subagents (`parent_block_id === blockId`), grouped by
 *  shared `dispatch_id` then by shared `display_name` among what's left,
 *  and mapped to activities. One dock row per Agent-tool-or-Workflow-tool
 *  call, not per individual subagent. */
export function subagentActivities(all: ReadonlyArray<ActiveSubagent>, blockId: string): PinnedActivity[] {
    const mine = all.filter((s) => s.parent_block_id === blockId);
    return groupSubagentsForDock(mine).map((child) =>
        isDispatchGroup(child) || isNameGroup(child) ? subagentGroupToActivity(child) : subagentToActivity(child)
    );
}
