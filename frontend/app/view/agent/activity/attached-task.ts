// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Attached-task liveness — "is at least one agent-declared long-running
 * activity attached to this pane right now," independent of `turnPhase`.
 *
 * Mirrors ActivityDock's own `allActivities` composition (shell + subagent +
 * tool adapters) exactly, per SPEC_ATTACHED_TASK_STATUS_AXIS_2026_08_02.md §4
 * ("the dock's own aggregate is already the system's answer to 'what counts
 * as agent-declared long-running work' — this axis should consume that same
 * signal, not bypass it with a raw process count"). Kept as a standalone pure
 * function (not exported from ActivityDock.tsx itself) so the dispatch call
 * site (agent-view.tsx) doesn't need to mount/subscribe to the dock component
 * to ask "is anything running."
 */

import type { ActiveSubagent } from "../../swarm/swarm-model";
import type { DocumentNode } from "../types";
import { shellActivities } from "./shell-adapter";
import { subagentActivities } from "./subagent-adapter";
import { toolActivities } from "./tool-adapter";

/** Unix ms the EARLIEST currently-running activity started, or null when
 *  nothing is live. The start time (not the observation time) is what
 *  `AttachedTaskObserved` must carry as `at` — `AttachedTaskState.since`
 *  is "when this episode began": a promoted Bash call has already been
 *  running ≥30s when first observed, and a pane reopened over an
 *  already-running shell must not restart the elapsed counter at 0
 *  (reagent P1 on PR #2489). */
export function earliestLiveAttachedStartMs(
    nodes: ReadonlyArray<DocumentNode>,
    allSubagents: ReadonlyArray<ActiveSubagent>,
    blockId: string,
    now: number,
): number | null {
    let earliest: number | null = null;
    const all = [
        ...shellActivities(nodes),
        ...subagentActivities(allSubagents, blockId),
        ...toolActivities(nodes, now),
    ];
    for (const a of all) {
        if (a.status !== "running") continue;
        if (earliest == null || a.startedAt < earliest) earliest = a.startedAt;
    }
    return earliest;
}

export function hasLiveAttachedActivity(
    nodes: ReadonlyArray<DocumentNode>,
    allSubagents: ReadonlyArray<ActiveSubagent>,
    blockId: string,
    now: number,
): boolean {
    return earliestLiveAttachedStartMs(nodes, allSubagents, blockId, now) != null;
}
