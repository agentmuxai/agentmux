// Copyright 2026, AgentMux Corp.
// SPDX-License-Identifier: Apache-2.0

/**
 * Background-task-registry adapter — surfaces a durable `db_background_tasks`
 * row (`BackgroundTaskView`, via `useBackgroundTaskRegistry.ts`'s
 * `ListBackgroundTasksCommand`) as a dock row when the transcript itself has
 * no record of it. Closes the gap `useBackgroundTaskRegistry.ts` left open:
 * that hook only ever fed the AGGREGATE `registryAttachedTaskSince` boolean
 * axis, never a rendered ROW — a task that survived a session restart (a
 * fresh session has no transcript history of ever launching it) was
 * invisible in the dock even though `attachedTask` correctly reported it.
 *
 * `BackgroundTaskView.id` mirrors the frontend dock's `node_id` — normally
 * the originating `tool_use_id` (see `background_tasks.rs`'s own module doc
 * comment) — so a task the transcript DOES still know about (same session,
 * no restart) already has a `PinnedActivity` from `tool-adapter.ts`'s
 * `toolActivities`, keyed by that same id. Filtering the registry's rows
 * against the ids already produced by the transcript-derived adapters is
 * enough to avoid double-rendering it; no other join key is needed.
 *
 * report: docs/reports/REPORT_AGENT_PANE_ACTIVITY_DOCK_ARCHITECTURE_ANALYSIS_2026_08_25.md
 * Tier 1 recommendation ("reconcile the durable background-task registry
 * with the dock's own transcript-derived rows").
 */

import type { PinnedActivity } from "./types";
import type { BackgroundTaskView } from "@/app/store/rpc-api";

/** Pure: one registry row as a dock row. No `tool`/`shell`/`subagent` source
 *  field is populated — a registry-only row has no live transcript log to
 *  show, so `ActivityRow`'s expand panel renders nothing for it (same as
 *  any other kind's `Show` gate failing). `canStop: false` mirrors
 *  `tool-adapter.ts`'s own reasoning for an accepted background launch: no
 *  stop path exists for a single detached task today. */
export function backgroundTaskToActivity(task: BackgroundTaskView): PinnedActivity {
    return {
        id: task.id,
        kind: "tool",
        title: task.label,
        status: task.status,
        startedAt: task.started_at_ms,
        endedAt: task.ended_at_ms ?? undefined,
        canStop: false,
    };
}

/** Every registry row NOT already represented among `knownIds` (the ids of
 *  the activities already produced by the transcript-derived adapters this
 *  render pass). Exported separately from `backgroundTaskToActivity` so the
 *  dedup rule is independently testable from the row-shape mapping. */
export function backgroundTaskActivities(
    tasks: readonly BackgroundTaskView[],
    knownIds: ReadonlySet<string>
): PinnedActivity[] {
    return tasks.filter((t) => !knownIds.has(t.id)).map(backgroundTaskToActivity);
}

/** A transcript-derived row still showing `running` takes the registry's
 *  terminal status when the registry knows the task ended. srv now completes
 *  registry rows from the CLI's own `task_notification`/`task_updated` lines
 *  (`background_task_feed.rs`), which arrive even for a task a subagent
 *  launched; the transcript's `<task-notification>` user message never does
 *  for those, so without this their dock row ran forever.
 *  SPEC_BACKGROUND_TASK_STRUCTURED_FEED_AND_SWARM_OWNERSHIP_2026_09_27.md §2.3.
 *  Pure; returns the same array when nothing changes. */
export function applyRegistryOutcomes(
    activities: readonly PinnedActivity[],
    tasks: readonly BackgroundTaskView[]
): PinnedActivity[] {
    const ended = new Map<string, BackgroundTaskView>();
    for (const t of tasks) if (t.status !== "running") ended.set(t.id, t);
    if (ended.size === 0) return activities as PinnedActivity[];
    let changed = false;
    const out = activities.map((a) => {
        const t = a.status === "running" ? ended.get(a.id) : undefined;
        if (!t) return a;
        changed = true;
        return { ...a, status: t.status, endedAt: t.ended_at_ms ?? t.last_seen_ms };
    });
    return changed ? out : (activities as PinnedActivity[]);
}
