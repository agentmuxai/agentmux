// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Background tasks in the Swarm, grouped under whoever owns them.
 *
 * The source is the durable registry (`db_background_tasks`, listed for the
 * whole fleet by `ListBackgroundTasksCommand` with an empty `blockid`), which
 * srv keeps from the Claude CLI's own task feed: every backgrounded Bash call,
 * its description, its real end status, and — for a call a subagent issued —
 * `owner_tool_use_id`, the Agent call that spawned that subagent. A subagent
 * row carries the same id as `tool_use_id` (from its `meta.json`), so a task
 * renders nested under the subagent that launched it. Everything else — the
 * agent's own tasks, and tasks whose owning subagent has no row (retired, or
 * a workflow member) — renders in the agent's own "Background" bucket.
 *
 * Finished tasks stay visible for the same windows the Activity Dock uses
 * (`RETENTION_MS`), so a task leaves both views at the same moment.
 *
 * SPEC_BACKGROUND_TASK_STRUCTURED_FEED_AND_SWARM_OWNERSHIP_2026_09_27.md §4.
 */

import type { BackgroundTaskView } from "@/app/store/rpc-api";
import { RETENTION_MS } from "@/app/view/agent/activity/types";

/** One agent's background tasks, split by owner. Both lists are newest-first. */
export interface AgentBackgroundTasks {
    /** Keyed by the owning subagent's `tool_use_id`. */
    bySubagent: Map<string, BackgroundTaskView[]>;
    /** The agent's own tasks, plus any whose owner has no subagent row here. */
    own: BackgroundTaskView[];
}

export const NO_BACKGROUND_TASKS: AgentBackgroundTasks = { bySubagent: new Map(), own: [] };

/** Pure: whether a task still shows at `now` — always while running, then for
 *  its status's retention window after it ended. */
export function isBackgroundTaskVisible(task: BackgroundTaskView, now: number): boolean {
    if (task.status === "running") return true;
    const endedAt = task.ended_at_ms ?? task.last_seen_ms;
    return now - endedAt < RETENTION_MS[task.status];
}

/** Pure: `tasks` (the whole fleet) narrowed to `blockId` and split by owner.
 *  `subagentToolUseIds` are the `tool_use_id`s of the subagent rows this agent
 *  actually shows; a task owned by any other subagent falls back to `own`, so
 *  it is never hidden. */
export function groupBackgroundTasks(
    tasks: readonly BackgroundTaskView[],
    blockId: string | null,
    subagentToolUseIds: ReadonlySet<string>
): AgentBackgroundTasks {
    if (!blockId) return NO_BACKGROUND_TASKS;
    const mine = tasks.filter((t) => t.block_id === blockId);
    if (mine.length === 0) return NO_BACKGROUND_TASKS;
    const newestFirst = [...mine].sort((a, b) => b.started_at_ms - a.started_at_ms);
    const bySubagent = new Map<string, BackgroundTaskView[]>();
    const own: BackgroundTaskView[] = [];
    for (const t of newestFirst) {
        const owner = t.owner_tool_use_id;
        if (owner && subagentToolUseIds.has(owner)) {
            const list = bySubagent.get(owner);
            if (list) list.push(t);
            else bySubagent.set(owner, [t]);
        } else {
            own.push(t);
        }
    }
    return { bySubagent, own };
}
