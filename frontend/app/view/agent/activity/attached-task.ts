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

export function hasLiveAttachedActivity(
    nodes: ReadonlyArray<DocumentNode>,
    allSubagents: ReadonlyArray<ActiveSubagent>,
    blockId: string,
    now: number,
): boolean {
    return (
        shellActivities(nodes).some((a) => a.status === "running") ||
        subagentActivities(allSubagents, blockId).some((a) => a.status === "running") ||
        toolActivities(nodes, now).some((a) => a.status === "running")
    );
}
